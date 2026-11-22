//! A trace sink that keeps every record in memory, and can hand each one on to the real sink.

use std::sync::{Arc, Mutex, PoisonError};

use mcp_proxy_core::trace::{CallRecord, TraceSink};

#[derive(Default)]
pub struct MemoryTraceSink {
    records: Mutex<Vec<CallRecord>>,
    next: Option<Arc<dyn TraceSink>>,
}

impl std::fmt::Debug for MemoryTraceSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryTraceSink")
            .field(
                "records",
                &self.records.lock().unwrap_or_else(PoisonError::into_inner).len(),
            )
            .field("forwards", &self.next.is_some())
            .finish()
    }
}

impl MemoryTraceSink {
    /// Keeps a copy of each record, then gives it to `next` (the journal of spec 009).
    pub fn forwarding_to(next: Arc<dyn TraceSink>) -> Self {
        Self {
            records: Mutex::default(),
            next: Some(next),
        }
    }

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
        if let Some(next) = &self.next {
            next.record(record.clone());
        }
        self.records.lock().unwrap_or_else(PoisonError::into_inner).push(record);
    }
}
