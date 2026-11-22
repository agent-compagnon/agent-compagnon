//! Message boxes and the save dialog through `rfd` (no direct Win32 call): usable before Tauri starts and after it
//! failed.

use std::path::PathBuf;

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageLevel};

pub fn error(title: &str, message: &str) {
    let _ = MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(MessageButtons::Ok)
        .show();
}

/// The save dialog of « Exporter un diagnostic », owned by `parent`: a zip archive, `name` proposed, opened in
/// `directory` when given. Blocks until the user chooses: called on a blocking thread. `None`: closed without choice.
pub fn save_archive(
    parent: &tauri::WebviewWindow,
    name: &str,
    directory: Option<PathBuf>,
) -> impl FnOnce() -> Option<PathBuf> + Send + 'static {
    let mut dialog = FileDialog::new()
        .set_parent(parent)
        .set_title("Exporter un diagnostic")
        .set_file_name(name)
        .add_filter("Archive zip", &["zip"]);
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    move || dialog.save_file()
}
