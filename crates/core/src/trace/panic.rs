//! Panic hook: where the panic happened and its message, filtered, in the journal and the log file. Nothing else:
//! no backtrace, no data of the call being served, and nothing on the standard error stream (in bridge mode it is
//! read by the LLM client).

use std::any::Any;
use std::panic::Location;

use super::journal::Journal;

/// Longest panic message kept, in characters.
const MAX_MESSAGE_CHARS: usize = 1000;

/// Replaces the panic hook of the process. Once, at start-up (spec 011); the default hook is not called.
pub fn install_panic_hook(journal: Journal) {
    std::panic::set_hook(Box::new(move |info| {
        let (location, message) = describe(info.location(), info.payload());
        journal.record_panic(&location, &message);
        // The process may be about to end: get the line to the disk first.
        let _ = journal.flush();
    }));
}

/// `file:line:column` and the text of the payload, before filtering.
pub(crate) fn describe(location: Option<&Location<'_>>, payload: &(dyn Any + Send)) -> (String, String) {
    let location = location.map_or_else(
        || "emplacement inconnu".to_owned(),
        |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
    );
    let message = if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "(message non textuel)".to_owned()
    };
    let message = match message.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((end, _)) => format!("{}…", &message[..end]),
        None => message,
    };
    (location, message)
}
