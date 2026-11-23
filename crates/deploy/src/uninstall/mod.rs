//! Uninstallation (spec 022): `mcp-proxy.exe --uninstall`, from « Applications installées », removes everything the
//! application put on the machine, without administrator rights.
//!
//! The plan is a pure function ([`plan`]); the operations go through [`crate::ops::SystemOps`], as for the
//! installation; every folder removed recursively is checked first ([`check_owned`]) and walked without following
//! any link. Nothing is removed before the exe is set aside and the running instance stopped ([`prepare`]); then
//! every item is tried and reported ([`remove`]). The entry « Applications installées » is written by the
//! installation, the updates and each start of the installed exe ([`record_entry`]).

mod dialog;
mod entry;
mod guard;
mod plan;
mod run;

#[cfg(test)]
mod tests;

use std::path::Path;

pub use dialog::{
    Action, Answer, DATA_LABEL, MalformedAnswer, UNINSTALL_COMMAND, UNINSTALL_WINDOW, UninstallDialog,
    after_close_message, item_label, parse_answer, report_view,
};
pub use entry::{
    DISPLAY_NAME, PUBLISHER, Recorded, UNINSTALL_ARG, UNINSTALL_KEY, UninstallEntry, command_exe, entry_for,
    estimated_size, record_entry, uninstall_command,
};
pub use guard::{Refusal, TreeRemoval, check_owned, remove_tree};
pub use plan::{
    ClientDecision, DATA_FOLDER, INSTALL_FOLDER, Item, PROGRAMS_FOLDER, UninstallPaths, WINDOW_DATA_FOLDER, decide,
    plan,
};
pub use run::{
    Context, DATA_ATTEMPTS, Done, Outcome, Report, UninstallError, WINDOW_DATA_ATTEMPTS, prepare, remove,
    remove_window_data,
};

use crate::install::{InstallPaths, same_location};
use crate::ops::SystemOps;

/// The key of the AUMID registered by the application (spec 013), checked against the gateway by its tests.
pub const AUMID_KEY: &str = r"Software\Classes\AppUserModelId\McpProxy.Desktop";
/// The key of the exclusion from Windows Error Reporting (spec 011); the value is named after the exe.
pub const WER_KEY: &str = r"Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications";

/// Whether `--uninstall` may run from `here`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// This exe is the installed one.
    Proceed,
    /// No exe is installed.
    NotInstalled,
    /// An exe is installed, but this is another copy.
    Elsewhere,
}

/// Only the installed exe uninstalls: every path it removes is derived from its own location. Paths compared as in
/// the installation (links and short names resolved, case ignored).
pub fn launch(ops: &dyn SystemOps, paths: &InstallPaths, here: &Path) -> Launch {
    let Some(installed) = ops.canonical(&paths.exe) else {
        return Launch::NotInstalled;
    };
    match ops.canonical(here) {
        Some(here) if same_location(&here, &installed) => Launch::Proceed,
        _ => Launch::Elsewhere,
    }
}

impl Launch {
    /// The message box of a launch that does not uninstall. French.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Proceed => None,
            Self::NotInstalled => Some(
                "MCP Proxy n'est pas installé sur ce compte : il n'y a rien à désinstaller. Rien n'a été modifié. Si \
                 vous l'avez utilisé sans l'installer, ses journaux et réglages sont dans %LOCALAPPDATA%\\MCP Proxy : \
                 vous pouvez supprimer ce dossier, MCP Proxy fermé.",
            ),
            Self::Elsewhere => Some(
                "Cet exe n'est pas celui qui est installé : rien n'a été modifié. Pour désinstaller MCP Proxy, ouvrez \
                 les paramètres de Windows, « Applications installées », puis « MCP Proxy », « Désinstaller ».",
            ),
        }
    }
}
