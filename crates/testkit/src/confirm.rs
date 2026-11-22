//! A confirmation channel with scripted answers.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::confirm::{ConfirmationBroker, ConfirmationOutcome, ConfirmationRequest};

#[derive(Debug, Clone, Copy)]
enum Step {
    Answer(ConfirmationOutcome),
    /// Never answers: the caller must give up by itself.
    Silent,
}

/// Answers with the scripted outcomes, in order, then refuses (INV-5). Records every request.
#[derive(Debug, Default)]
pub struct FakeConfirmationBroker {
    script: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ConfirmationRequest>>,
}

impl FakeConfirmationBroker {
    pub fn new(outcomes: impl IntoIterator<Item = ConfirmationOutcome>) -> Self {
        let broker = Self::default();
        broker.script(outcomes);
        broker
    }

    /// Appends answers to the script.
    pub fn script(&self, outcomes: impl IntoIterator<Item = ConfirmationOutcome>) {
        self.lock_script().extend(outcomes.into_iter().map(Step::Answer));
    }

    /// The next request is never answered.
    pub fn stay_silent(&self) {
        self.lock_script().push_back(Step::Silent);
    }

    pub fn requests(&self) -> Vec<ConfirmationRequest> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn lock_script(&self) -> std::sync::MutexGuard<'_, VecDeque<Step>> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl ConfirmationBroker for FakeConfirmationBroker {
    async fn confirm(&self, request: ConfirmationRequest) -> ConfirmationOutcome {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let step = self.lock_script().pop_front();
        match step {
            Some(Step::Answer(outcome)) => outcome,
            Some(Step::Silent) => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                ConfirmationOutcome::Refused
            }
            None => ConfirmationOutcome::Refused,
        }
    }
}
