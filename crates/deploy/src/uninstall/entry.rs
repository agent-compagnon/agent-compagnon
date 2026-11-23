//! The entry « Applications installées » of Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP
//! Proxy`, written at the installation, after each update (by the installation window as by the updater) and at
//! each start of the installed exe, so that it always names the version in place.

use std::path::Path;

use crate::install::InstallPaths;
use crate::ops::{OpError, SystemOps};
use crate::version::Version;

pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy";
/// The argument of `UninstallString`.
pub const UNINSTALL_ARG: &str = "--uninstall";
pub const DISPLAY_NAME: &str = "MCP Proxy";
/// No publisher name exists in the project: the product name, the most neutral value (I022.10).
pub const PUBLISHER: &str = "MCP Proxy";

/// The values of the entry, as written and read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallEntry {
    pub display_name: String,
    /// `None` when the version mark of the exe in place cannot be read: the value is then absent.
    pub display_version: Option<String>,
    pub publisher: String,
    pub install_location: String,
    /// The exe itself, whose first icon Windows shows.
    pub display_icon: String,
    /// `"<exe>" --uninstall`
    pub uninstall_string: String,
    pub no_modify: bool,
    pub no_repair: bool,
    /// KiB, as Windows reads `EstimatedSize`.
    pub estimated_size: u32,
}

fn text(path: &Path) -> Option<&str> {
    path.to_str().filter(|p| !p.is_empty() && !p.contains('"'))
}

/// `"C:\…\mcp-proxy.exe" --uninstall`; `None` for a path that is not Unicode or holds a quote.
pub fn uninstall_command(exe: &Path) -> Option<String> {
    Some(format!("\"{}\" {UNINSTALL_ARG}", text(exe)?))
}

/// Bytes to KiB, rounded up.
pub fn estimated_size(bytes: u64) -> u32 {
    u32::try_from(bytes.div_ceil(1024)).unwrap_or(u32::MAX)
}

/// The entry for the installation in `paths`, its exe of `version` and `size` bytes.
pub fn entry_for(paths: &InstallPaths, version: Option<&Version>, size: u64) -> Option<UninstallEntry> {
    Some(UninstallEntry {
        display_name: DISPLAY_NAME.to_owned(),
        display_version: version.map(ToString::to_string),
        publisher: PUBLISHER.to_owned(),
        install_location: text(&paths.dir)?.to_owned(),
        display_icon: text(&paths.exe)?.to_owned(),
        uninstall_string: uninstall_command(&paths.exe)?,
        no_modify: true,
        no_repair: true,
        estimated_size: estimated_size(size),
    })
}

/// The exe a command starts: the first quoted part, or what precedes the first blank.
pub fn command_exe(command: &str) -> &str {
    let command = command.trim_start();
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => command.split_whitespace().next().unwrap_or(command),
    }
}

/// What writing the entry did, and what it replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub previous: Option<UninstallEntry>,
    pub changed: bool,
}

/// Writes the entry of the exe now at `paths.exe`, of `version`, unless it is already exactly so.
pub fn record_entry(ops: &dyn SystemOps, paths: &InstallPaths, version: Option<&Version>) -> Result<Recorded, OpError> {
    let previous = ops.uninstall_entry()?;
    let size = ops.file_len(&paths.exe)?;
    let entry = entry_for(paths, version, size).ok_or(OpError::Refused(
        "chemin d'installation impossible à inscrire dans « Applications installées »",
    ))?;
    if previous.as_ref() == Some(&entry) {
        return Ok(Recorded {
            previous,
            changed: false,
        });
    }
    ops.set_uninstall_entry(&entry)?;
    Ok(Recorded {
        previous,
        changed: true,
    })
}
