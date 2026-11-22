//! Start with Windows: the `MCP Proxy` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, the
//! installed exe between quotes followed by `--autostart`.
//!
//! The value is what Windows obeys, so it is the reference: `UserSettings::autostart` mirrors it. Only the exact
//! command of the installed exe counts as enabled; anything else under that name (an old path, a hand-made value)
//! is shown as disabled and replaced when the user enables the option. A `Run` value never names an exe outside the
//! installation folder: the option can only change in the installed exe.

use std::fmt;
use std::path::Path;

use crate::install::paths::InstallPaths;
use crate::install::run::AUTOSTART_ARG;
use crate::ops::{OpError, SystemOps};

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "MCP Proxy";

/// `"C:\…\mcp-proxy.exe" --autostart`; `None` for a path that is not Unicode or holds a quote (Windows forbids
/// quotes in names, so it cannot be the installed exe).
pub fn run_command(exe: &Path) -> Option<String> {
    let exe = exe.to_str().filter(|e| !e.is_empty() && !e.contains('"'))?;
    Some(format!("\"{exe}\" {AUTOSTART_ARG}"))
}

/// The value starts the installed exe at session opening.
pub fn is_enabled(entry: Option<&str>, exe: &Path) -> bool {
    match (entry, run_command(exe)) {
        (Some(entry), Some(expected)) => entry == expected,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutostartError {
    /// This exe is not the installed one: the value would name a provisional location.
    NotInstalled,
    Path,
    Registry(OpError),
}

impl fmt::Display for AutostartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => f.write_str(
                "le démarrage avec Windows ne se règle qu'une fois MCP Proxy installé : lancez-le depuis le menu \
                 Démarrer",
            ),
            Self::Path => f.write_str("chemin d'installation impossible à inscrire au démarrage de Windows"),
            Self::Registry(cause) => write!(f, "registre de Windows inaccessible ({cause})"),
        }
    }
}

/// Whether the option is on, as Windows will apply it. A value that cannot be read counts as off.
pub fn autostart_enabled(ops: &dyn SystemOps, paths: &InstallPaths) -> bool {
    ops.run_entry()
        .ok()
        .flatten()
        .is_some_and(|entry| is_enabled(Some(&entry), &paths.exe))
}

/// The item « Démarrer avec Windows » of the icon menu: writes or removes the value, from the installed exe only.
/// Returns the state now in force.
pub fn set_autostart(
    ops: &dyn SystemOps,
    paths: &InstallPaths,
    here_is_installed: bool,
    enable: bool,
) -> Result<bool, AutostartError> {
    if !here_is_installed {
        return Err(AutostartError::NotInstalled);
    }
    if enable {
        let command = run_command(&paths.exe).ok_or(AutostartError::Path)?;
        ops.set_run_entry(&command).map_err(AutostartError::Registry)?;
    } else {
        ops.remove_run_entry().map_err(AutostartError::Registry)?;
    }
    tracing::info!(enable, "démarrage avec Windows modifié");
    Ok(autostart_enabled(ops, paths))
}

/// What `UserSettings::autostart` must become to mirror the value, if it differs.
pub fn mirrored(settings: bool, key: bool) -> Option<bool> {
    (settings != key).then_some(key)
}
