//! The decision taken at every launch of the application, before the single instance, the pipe and the gateway:
//! install, offer to update the installation, hand over to the installed exe, or go on from here.

use std::path::Path;

use crate::install::paths::{InstallPaths, same_location};
use crate::ops::SystemOps;
use crate::version::{Installed, Version, is_newer};

/// How this exe was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// No argument: the user opened it.
    Interactive,
    /// `--autostart`, from the `Run` value at session opening: no window, no dialog.
    Autostart,
    /// `--no-install`: run from here without offering anything (after a declined or failed installation, or
    /// launched by the bridge).
    InPlace,
    /// `stdio`, launched by an LLM client: never a dialog, wherever it runs from.
    Bridge,
}

/// What the launch finds, gathered by [`Situation::inspect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Situation {
    pub start: Start,
    /// This exe is the installed one.
    pub here_is_installed: bool,
    /// The exe at the installation path, when there is one and it is not this exe.
    pub installed: Option<Installed>,
    pub own: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPlan {
    /// Run the application from here.
    Continue,
    /// No installation: show the installation dialog.
    Install,
    /// This exe is newer than the installed one: offer to update the installation.
    Update { installed: Version },
    /// Launch the installed exe (with `--autostart` if this launch had it) and stop.
    Delegate { autostart: bool },
}

impl Situation {
    /// Reads what the decision needs: where this exe runs from, whether an exe is installed, and its version mark.
    /// The bridge reads nothing.
    pub fn inspect(ops: &dyn SystemOps, paths: &InstallPaths, here: &Path, start: Start, own: Version) -> Self {
        if start == Start::Bridge {
            return Self {
                start,
                here_is_installed: false,
                installed: None,
                own,
            };
        }
        let here = ops.canonical(here);
        let installed_exe = ops.canonical(&paths.exe);
        let here_is_installed = matches!((&here, &installed_exe), (Some(a), Some(b)) if same_location(a, b));
        let installed = match installed_exe {
            Some(exe) if !here_is_installed => Some(ops.exe_version(&exe)),
            _ => None,
        };
        Self {
            start,
            here_is_installed,
            installed,
            own,
        }
    }

    /// The decision, from what was found and nothing else.
    pub fn plan(&self) -> InstallPlan {
        let autostart = self.start == Start::Autostart;
        if matches!(self.start, Start::Bridge | Start::InPlace) || self.here_is_installed {
            return InstallPlan::Continue;
        }
        match &self.installed {
            None if autostart => InstallPlan::Continue,
            None => InstallPlan::Install,
            // An installed exe whose version cannot be read is never replaced: it might be newer.
            Some(Installed::Unknown) => InstallPlan::Delegate { autostart },
            Some(Installed::Known(installed)) if !autostart && is_newer(&self.own, installed) => InstallPlan::Update {
                installed: installed.clone(),
            },
            Some(Installed::Known(_)) => InstallPlan::Delegate { autostart },
        }
    }
}
