//! Message boxes through `rfd` (no direct Win32 call): usable before Tauri starts and after it failed.

use rfd::{MessageButtons, MessageDialog, MessageLevel};

pub fn error(title: &str, message: &str) {
    let _ = MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(MessageButtons::Ok)
        .show();
}
