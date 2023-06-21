// SPDX-FileCopyrightText: 2023 Jade Lovelace
//
// SPDX-License-Identifier: MPL-2.0

//! Writing of pcap files.

use std::{collections::BTreeMap, io, pin::Pin};

use pcap_parser::{
    EnhancedPacketBlock, InterfaceDescriptionBlock, Linktype, OptionCode, PcapNGOption,
    SectionHeaderBlock, ToVec,
};
use tokio::io::AsyncWriteExt;

use crate::Nanos;

pub struct PcapWriter<W> {
    writer: W,

    /// Map between host if_index values and pcapng values.
    if_index_map: BTreeMap<u32, u32>,

    pcap_if_index: u32,
}

/// Because of async being a pain in the neck, just make a nonblocking
/// synchronous Writer and then flush it asynchronously.
#[derive(Default)]
pub struct AsyncWriteHack {
    writer: io::Cursor<Vec<u8>>,
}

impl io::Write for AsyncWriteHack {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.writer, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.writer)
    }
}

impl AsyncWriteHack {
    pub async fn flush_downstream(
        &mut self,
        async_writer: &mut Pin<&mut impl tokio::io::AsyncWrite>,
    ) -> io::Result<()> {
        let writer = self.writer.get_mut();
        AsyncWriteExt::write_all(async_writer, &writer).await?;
        writer.clear();
        Ok(())
    }
}

impl<W: io::Write> PcapWriter<W> {
    pub fn new(writer: W) -> Result<Self, io::Error> {
        let mut w = PcapWriter {
            writer,
            if_index_map: Default::default(),
            pcap_if_index: 0,
        };

        w.write_section_header()?;
        Ok(w)
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    pub fn get(&self) -> &W {
        &self.writer
    }

    fn write_section_header(&mut self) -> Result<(), io::Error> {
        let mut shb = SectionHeaderBlock {
            block_type: 0,
            block_len1: 0,
            bom: 0,
            major_version: 0,
            minor_version: 0,
            section_len: -1i64,
            // for "lol" reasons the lack of endofopt in this block is not
            // fixed up.
            options: vec![PcapNGOption {
                code: OptionCode::EndOfOpt,
                len: 0,
                value: &[],
            }],
            block_len2: 0,
        };

        self.writer.write(&shb.to_vec().unwrap())?;
        Ok(())
    }

    fn pcap_interface_id(&mut self, if_index: u32) -> Result<u32, io::Error> {
        if let Some(pcap_if_index) = self.if_index_map.get(&if_index) {
            return Ok(*pcap_if_index);
        }

        let tsresol = 9u8;
        let tsresol_enc = (tsresol as u32).to_le_bytes();
        let mut idb = InterfaceDescriptionBlock {
            block_type: 0,
            block_len1: 0,
            block_len2: 0,
            linktype: Linktype::ETHERNET,
            reserved: 0,
            snaplen: 262144,
            options: vec![PcapNGOption {
                code: OptionCode::IfTsresol,
                len: 1,
                value: &tsresol_enc,
            }],
            // nanosecond resolution
            if_tsresol: tsresol,
            if_tsoffset: 0,
        };

        self.writer.write_all(&idb.to_vec().unwrap())?;

        let ret = self.pcap_if_index;
        self.if_index_map.insert(if_index, ret);
        self.pcap_if_index += 1;

        Ok(ret)
    }

    pub fn on_packet(&mut self, time: Nanos, if_index: u32, data: &[u8]) -> Result<(), io::Error> {
        let pcap_if_index = self.pcap_interface_id(if_index)?;

        let (ts_high, ts_low) = ((time >> 32) & 0xffff_ffff, time & 0xffff_ffff);

        let mut epb = EnhancedPacketBlock {
            block_type: 0,
            block_len1: 0,
            block_len2: 0,
            if_id: pcap_if_index,
            ts_high: ts_high as u32,
            ts_low: ts_low as u32,
            caplen: data.len() as u32,
            origlen: data.len() as u32,
            data,
            options: Vec::new(),
        };

        self.writer.write_all(&epb.to_vec().unwrap())?;

        Ok(())
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn writer() -> PcapWriter<io::Cursor<Vec<u8>>> {
        PcapWriter::new(io::Cursor::new(Vec::new())).unwrap()
    }
}
