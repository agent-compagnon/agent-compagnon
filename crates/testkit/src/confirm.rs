//! A confirmation channel with scripted answers.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use mcp_proxy_core::confirm::{ConfirmationBroker, ConfirmationOutcome, ConfirmationRequest};

/// Answers with the scripted outcomes, in order, then refuses (INV-5). Records every request.
#[derive(Debug, Default)]
pub struct FakeConfirmationBroker {
    script: Mutex<VecDeque<ConfirmationOutcome>>,
    requests: Mutex<Vec<ConfirmationRequest>>,
}

impl FakeConfirmationBroker {
    pub fn new(outcomes: impl IntoIterator<Item = ConfirmationOutcome>) -> Self {
        Self {
            script: Mutex::new(outcomes.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<ConfirmationRequest> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[async_trait]
impl ConfirmationBroker for FakeConfirmationBroker {
    async fn confirm(&self, request: ConfirmationRequest) -> ConfirmationOutcome {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ConfirmationOutcome::Refused)
    }
}
