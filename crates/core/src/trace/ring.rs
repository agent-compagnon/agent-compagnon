//! Circular buffer of journal entries, bounded in count and in bytes: the oldest entries go first.

use std::collections::VecDeque;

use super::journal::JournalEntry;

#[derive(Debug)]
pub(crate) struct Ring {
    entries: VecDeque<(usize, JournalEntry)>,
    bytes: usize,
    capacity: usize,
    max_bytes: usize,
}

impl Ring {
    pub(crate) fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            capacity: capacity.max(1),
            max_bytes: max_bytes.max(1),
        }
    }

    pub(crate) fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Adds `entry`, of `weight` bytes, and evicts the oldest entries until both bounds hold. An entry heavier than
    /// the whole buffer is not kept.
    pub(crate) fn push(&mut self, entry: JournalEntry, weight: usize) -> bool {
        if weight > self.max_bytes {
            return false;
        }
        self.entries.push_back((weight, entry));
        self.bytes += weight;
        while self.entries.len() > self.capacity || self.bytes > self.max_bytes {
            match self.entries.pop_front() {
                Some((w, _)) => self.bytes -= w,
                None => break,
            }
        }
        true
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &JournalEntry> {
        self.entries.iter().map(|(_, e)| e)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}
