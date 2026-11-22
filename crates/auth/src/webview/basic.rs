//! The Basic input windows open right now, and the answers of their page.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use secrecy::SecretString;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::ui::BasicInput;

/// What the page of the Basic window sends: `null` (« Annuler »), or `{ "login": …, "password": … }`. The password
/// becomes a [`SecretString`] as soon as it is taken out of the JSON value, without a copy. `Err` for anything
/// else, which is ignored: the window keeps waiting.
pub fn parse_basic_answer(answer: Option<Value>) -> Result<Option<BasicInput>, MalformedAnswer> {
    let mut object = match answer {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(object)) => object,
        Some(_) => return Err(MalformedAnswer),
    };
    let password = match object.remove("password") {
        Some(Value::String(password)) => SecretString::from(password),
        _ => return Err(MalformedAnswer),
    };
    match object.remove("login") {
        Some(Value::String(login)) if object.is_empty() => Ok(Some(BasicInput { login, password })),
        _ => Err(MalformedAnswer),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("réponse de la fenêtre de saisie mal formée")]
pub struct MalformedAnswer;

/// The Basic windows waiting for an answer, by window label (never reused).
#[derive(Default)]
pub struct BasicPrompts {
    waiting: Mutex<HashMap<String, oneshot::Sender<Option<BasicInput>>>>,
}

impl std::fmt::Debug for BasicPrompts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicPrompts")
            .field("open", &self.lock().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl BasicPrompts {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, oneshot::Sender<Option<BasicInput>>>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers the window `label`; its single answer arrives on the receiver.
    pub fn open(&self, label: &str) -> oneshot::Receiver<Option<BasicInput>> {
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(label.to_owned(), sender);
        receiver
    }

    /// The answer of the window `label`. Only the first answer of a registered window counts; `false` otherwise.
    pub fn submit(&self, label: &str, answer: Option<BasicInput>) -> bool {
        match self.lock().remove(label) {
            Some(sender) => sender.send(answer).is_ok(),
            None => false,
        }
    }

    /// The user closed the window: same as « Annuler ».
    pub fn closed(&self, label: &str) {
        self.submit(label, None);
    }
}
