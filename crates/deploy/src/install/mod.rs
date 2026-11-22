//! Installation at first launch (spec 019): one exe, no administrator rights, everything under `HKCU` and
//! `%LOCALAPPDATA%`. The decision ([`InstallPlan`]) is a pure function of what the launch finds; the operations go
//! through [`crate::ops::SystemOps`]; the window's answers through [`InstallDialog`].

mod autostart;
mod dialog;
mod paths;
mod plan;
mod run;

#[cfg(test)]
mod tests;

pub use autostart::{
    AutostartError, RUN_KEY, RUN_VALUE, autostart_enabled, is_enabled, mirrored, run_command, set_autostart,
};
pub use dialog::{
    Action, Answer, INSTALL_COMMAND, INSTALL_WINDOW, InstallDialog, MalformedAnswer, Proposal, parse_answer,
};
pub use paths::{BACKUP_NAME, EXE_NAME, InstallPaths, LOCATION_LABEL, SHORTCUT_NAME, STAGED_NAME, same_location};
pub use plan::{InstallPlan, Situation, Start};
pub use run::{AUTOSTART_ARG, InstallError, Leftover, NO_INSTALL_ARG, Step, delegate, install, run_here, update};
