//! A trace sink that keeps every record in memory.

use std::sync::{Mutex, PoisonError};

use mcp_proxy_core::trace::{CallRecord, TraceSink};

#[derive(Debug, Default)]
pub struct MemoryTraceSink {
    records: Mutex<Vec<CallRecord>>,
}

impl MemoryTraceSink {
    pub fn records(&self) -> Vec<CallRecord> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn last(&self) -> Option<CallRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
    }
}

impl TraceSink for MemoryTraceSink {
    fn record(&self, record: CallRecord) {
        self.records.lock().unwrap_or_else(PoisonError::into_inner).push(record);
    }
}
