extern crate slab;

use proto::*;
use super::state;

use std::collections::{HashMap, hash_map};

/// Storage for streams
#[derive(Debug)]
pub struct Store {
    slab: slab::Slab<state::Stream>,
    ids: HashMap<StreamId, usize>,
}

pub enum Entry<'a> {
    Occupied(OccupiedEntry<'a>),
    Vacant(VacantEntry<'a>),
}

pub struct OccupiedEntry<'a> {
    ids: hash_map::OccupiedEntry<'a, StreamId, usize>,
    slab: &'a mut slab::Slab<state::Stream>,
}

pub struct VacantEntry<'a> {
    ids: hash_map::VacantEntry<'a, StreamId, usize>,
    slab: &'a mut slab::Slab<state::Stream>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            slab: slab::Slab::new(),
            ids: HashMap::new(),
        }
    }

    pub fn get_mut(&mut self, id: &StreamId) -> Option<&mut state::Stream> {
        if let Some(handle) = self.ids.get(id) {
            Some(&mut self.slab[*handle])
        } else {
            None
        }
    }

    pub fn entry(&mut self, id: StreamId) -> Entry {
        use self::hash_map::Entry::*;

        match self.ids.entry(id) {
            Occupied(e) => {
                Entry::Occupied(OccupiedEntry {
                    ids: e,
                    slab: &mut self.slab,
                })
            }
            Vacant(e) => {
                Entry::Vacant(VacantEntry {
                    ids: e,
                    slab: &mut self.slab,
                })
            }
        }
    }
}

impl<'a> OccupiedEntry<'a> {
    pub fn into_mut(self) -> &'a mut state::Stream {
        &mut self.slab[*self.ids.get()]
    }
}

impl<'a> VacantEntry<'a> {
    pub fn insert(self, value: state::Stream) -> &'a mut state::Stream {
        // Insert the value in the slab
        let handle = self.slab.insert(value);

        // Insert the handle in the ID map
        self.ids.insert(handle);

        &mut self.slab[handle]
    }
}
