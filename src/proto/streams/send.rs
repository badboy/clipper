use {frame, ConnectionError};
use proto::*;
use super::*;

use error::User::*;

use bytes::Buf;

use std::collections::VecDeque;
use std::marker::PhantomData;

/// Manages state transitions related to outbound frames.
#[derive(Debug)]
pub(super) struct Send<B> {
    /// Maximum number of locally initiated streams
    max_streams: Option<usize>,

    /// Current number of locally initiated streams
    num_streams: usize,

    /// Stream identifier to use for next initialized stream.
    next_stream_id: StreamId,

    /// Initial window size of locally initiated streams
    init_window_sz: WindowSize,

    /// List of streams waiting for outbound connection capacity
    pending_capacity: store::List<B>,

    /// Prioritization layer
    prioritize: Prioritize<B>,
}

impl<B> Send<B> where B: Buf {

    /// Create a new `Send`
    pub fn new<P: Peer>(config: &Config) -> Self {
        let next_stream_id = if P::is_server() {
            2
        } else {
            1
        };

        Send {
            max_streams: config.max_local_initiated,
            num_streams: 0,
            next_stream_id: next_stream_id.into(),
            init_window_sz: config.init_local_window_sz,
            pending_capacity: store::List::new(),
            prioritize: Prioritize::new(config),
        }
    }

    /// Update state reflecting a new, locally opened stream
    ///
    /// Returns the stream state if successful. `None` if refused
    pub fn open<P: Peer>(&mut self)
        -> Result<Stream<B>, ConnectionError>
    {
        try!(self.ensure_can_open::<P>());

        if let Some(max) = self.max_streams {
            if max <= self.num_streams {
                return Err(Rejected.into());
            }
        }

        let ret = Stream::new(self.next_stream_id);

        // Increment the number of locally initiated streams
        self.num_streams += 1;
        self.next_stream_id.increment();

        Ok(ret)
    }

    pub fn send_headers(&mut self,
                        frame: frame::Headers,
                        stream: &mut store::Ptr<B>)
        -> Result<(), ConnectionError>
    {
        // Update the state
        stream.state.send_open(self.init_window_sz, frame.is_end_stream())?;

        // Queue the frame for sending
        self.prioritize.queue_frame(frame.into(), stream);

        Ok(())
    }

    pub fn send_eos(&mut self, stream: &mut Stream<B>)
        -> Result<(), ConnectionError>
    {
        stream.state.send_close()
    }

    pub fn send_data(&mut self,
                     frame: frame::Data<B>,
                     stream: &mut store::Ptr<B>)
        -> Result<(), ConnectionError>
    {
        let sz = frame.payload().remaining();

        if sz > MAX_WINDOW_SIZE as usize {
            // TODO: handle overflow
            unimplemented!();
        }

        let sz = sz as WindowSize;

        // Make borrow checker happy
        loop {
            let unadvertised = stream.unadvertised_send_window;

            match stream.send_flow_control() {
                Some(flow) => {
                    // Ensure that the size fits within the advertised size
                    try!(flow.ensure_window(
                            sz + unadvertised, FlowControlViolation));

                    // Now, claim the window on the stream
                    flow.claim_window(sz, FlowControlViolation)
                        .expect("local connection flow control error");

                    break;
                }
                None => {}
            }

            if stream.state.is_closed() {
                return Err(InactiveStreamId.into())
            } else {
                return Err(UnexpectedFrameType.into())
            }
        }

        if frame.is_end_stream() {
            try!(stream.state.send_close());
        }

        self.prioritize.queue_frame(frame.into(), stream);

        Ok(())
    }

    pub fn poll_complete<T>(&mut self,
                            store: &mut Store<B>,
                            dst: &mut Codec<T, B>)
        -> Poll<(), ConnectionError>
        where T: AsyncWrite,
    {
        self.prioritize.poll_complete(store, dst)
    }

    pub fn recv_connection_window_update(&mut self,
                                         frame: frame::WindowUpdate,
                                         store: &mut Store<B>)
        -> Result<(), ConnectionError>
    {
        self.prioritize.recv_window_update(frame)?;

        // TODO: If there is available connection capacity, release pending
        // streams.
        //
        // Walk each stream pending capacity and see if this change to the
        // connection window can increase the advertised capacity of the stream.

        unimplemented!();
        // Ok(())
    }

    pub fn recv_stream_window_update(&mut self,
                                     frame: frame::WindowUpdate,
                                     stream: &mut store::Ptr<B>)
        -> Result<(), ConnectionError>
    {
        let connection = self.prioritize.available_window();
        let unadvertised = stream.unadvertised_send_window;

        let effective_window_size = {
            let mut flow = match stream.state.send_flow_control() {
                Some(flow) => flow,
                None => return Ok(()),
            };

            debug_assert!(unadvertised == 0 || connection == 0);

            // Expand the full window
            flow.expand_window(frame.size_increment())?;
            flow.effective_window_size()
        };

        if connection < effective_window_size {
            stream.unadvertised_send_window = effective_window_size - connection;

            if !stream.is_pending_send_capacity {
                stream.is_pending_send_capacity = true;
                self.pending_capacity.push::<stream::NextCapacity>(stream);
            }
        }

        if stream.unadvertised_send_window == frame.size_increment() + unadvertised {
            // The entire window update is unadvertised, no need to do anything
            // else
            return Ok(());
        }

        stream.notify_send();

        Ok(())
    }

    pub fn dec_num_streams(&mut self) {
        self.num_streams -= 1;
    }

    /// Returns true if the local actor can initiate a stream with the given ID.
    fn ensure_can_open<P: Peer>(&self) -> Result<(), ConnectionError> {
        if P::is_server() {
            // Servers cannot open streams. PushPromise must first be reserved.
            return Err(UnexpectedFrameType.into());
        }

        // TODO: Handle StreamId overflow

        Ok(())
    }
}
