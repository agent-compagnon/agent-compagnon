//! Message boxes through `rfd` (no direct Win32 call): usable before Tauri starts and after it failed.

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

pub fn error(title: &str, message: &str) {
    let _ = MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(MessageButtons::Ok)
        .show();
}

/// A Yes/No question; anything but « Oui » is a no.
pub fn confirm(title: &str, message: &str) -> bool {
    let answer = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(message)
        .set_buttons(MessageButtons::YesNo)
        .show();
    answer == MessageDialogResult::Yes
}
