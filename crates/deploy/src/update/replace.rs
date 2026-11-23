//! Putting a verified exe in place, and what a replacement leaves behind.
//!
//! One mechanism for the installation (spec 019) and the updater: the new exe is copied next to the installed one
//! under `mcp-proxy.exe.new`, checked again, the installed exe is renamed `mcp-proxy.exe.old` (Windows lets a running
//! exe be renamed, not replaced: the application itself and every bridge started by an LLM client hold it), then the
//! new one takes its name. The running process keeps its image: the new version takes effect at the next start.
//! `.old` cannot be removed while a process still runs it; it goes at the next start, with the other leftovers. A
//! `.old` still run by a bridge of an older version (since spec 026, a bridge outlives the restarts of the
//! application) is renamed under a name of leftover, so that it never stops an update.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::install::{BACKUP_NAME, InstallPaths, STAGED_NAME};
use crate::ops::{OpError, SystemOps};
use crate::uninstall::record_entry;
use crate::version::{Installed, Version};

/// Where the updater works: the installation, and `%LOCALAPPDATA%\MCP Proxy\update\` for the downloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePaths {
    pub install: InstallPaths,
    pub downloads: PathBuf,
}

impl UpdatePaths {
    /// `data` is `%LOCALAPPDATA%\MCP Proxy`.
    pub fn new(install: InstallPaths, data: &Path) -> Self {
        Self {
            install,
            downloads: data.join("update"),
        }
    }
}

/// An exe that passed every check. Only the updater builds one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerifiedExe {
    /// In `update\<version>\`.
    pub path: PathBuf,
    pub version: Version,
    pub sha256: [u8; 32],
}

/// A step of the replacement, as the journal names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceStep {
    Copy,
    /// The copy is not what was verified.
    CopyAltered,
    /// The previous `.old` is still held, by a bridge of an older version.
    ClearBackup,
    SetAside,
    PutInPlace,
}

/// Why the installed exe was not replaced. Says whether it is still the previous one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceError {
    pub step: ReplaceStep,
    pub cause: Option<OpError>,
    /// The installed exe is the previous one, as before the attempt.
    pub restored: bool,
}

impl fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let step = match self.step {
            ReplaceStep::Copy => "copie dans le dossier d'installation impossible",
            ReplaceStep::CopyAltered => "copie dans le dossier d'installation différente de l'exe vérifié",
            ReplaceStep::ClearBackup => "ancienne version encore utilisée par un client LLM",
            ReplaceStep::SetAside => "mise de côté de la version installée impossible",
            ReplaceStep::PutInPlace => "mise en place de la nouvelle version impossible",
        };
        f.write_str(step)?;
        if let Some(cause) = &self.cause {
            write!(f, " ({cause})")?;
        }
        if !self.restored {
            f.write_str(" ; la version installée est restée sous le nom « mcp-proxy.exe.old »")?;
        }
        Ok(())
    }
}

/// Replaces the installed exe. The real one is [`RenameReplacer`]; tests count the calls of a fake one.
pub trait ExeReplacer: Send + Sync {
    /// The version mark of the installed exe, read without executing it.
    fn installed(&self) -> Installed;

    /// Called only once every check passed.
    fn replace(&self, verified: &VerifiedExe) -> Result<(), ReplaceError>;
}

/// The replacement by renaming, over [`SystemOps`].
pub struct RenameReplacer {
    ops: Arc<dyn SystemOps>,
    paths: InstallPaths,
}

impl fmt::Debug for RenameReplacer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenameReplacer").finish_non_exhaustive()
    }
}

impl RenameReplacer {
    pub fn new(ops: Arc<dyn SystemOps>, paths: InstallPaths) -> Self {
        Self { ops, paths }
    }

    fn remove_if_present(&self, path: &Path) -> Result<(), OpError> {
        match self.ops.remove_file(path) {
            Err(error) if !error.is_not_found() => Err(error),
            _ => Ok(()),
        }
    }
}

fn failed(step: ReplaceStep, cause: Option<OpError>) -> ReplaceError {
    tracing::warn!(step = ?step, cause = ?cause.as_ref().map(ToString::to_string), "mise à jour : remplacement en échec");
    ReplaceError {
        step,
        cause,
        restored: true,
    }
}

impl ExeReplacer for RenameReplacer {
    fn installed(&self) -> Installed {
        self.ops.exe_version(&self.paths.exe)
    }

    fn replace(&self, verified: &VerifiedExe) -> Result<(), ReplaceError> {
        let paths = &self.paths;
        let ops = self.ops.as_ref();
        let drop_staged = || {
            let _ = self.remove_if_present(&paths.staged);
        };
        // A copy left by an interrupted run.
        self.remove_if_present(&paths.staged)
            .map_err(|e| failed(ReplaceStep::Copy, Some(e)))?;
        if let Err(error) = ops.copy_file(&verified.path, &paths.staged) {
            drop_staged();
            return Err(failed(ReplaceStep::Copy, Some(error)));
        }
        let copied = ops
            .read(&paths.staged)
            .map(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)));
        if copied.as_ref().ok() != Some(&verified.sha256) {
            drop_staged();
            return Err(failed(ReplaceStep::CopyAltered, copied.err()));
        }
        if let Err(error) = clear_backup(ops, paths) {
            drop_staged();
            return Err(failed(ReplaceStep::ClearBackup, Some(error)));
        }
        if let Err(error) = ops.rename(&paths.exe, &paths.backup) {
            drop_staged();
            return Err(failed(ReplaceStep::SetAside, Some(error)));
        }
        if let Err(error) = ops.rename(&paths.staged, &paths.exe) {
            let restored = ops.rename(&paths.backup, &paths.exe).is_ok();
            drop_staged();
            let mut error = failed(ReplaceStep::PutInPlace, Some(error));
            error.restored = restored;
            if !restored {
                tracing::error!("mise à jour : version installée non remise en place");
            }
            return Err(error);
        }
        // Held by this very process: goes at the next start.
        let _ = ops.remove_file(&paths.backup);
        tracing::info!(version = %verified.version, "mise à jour : nouvelle version en place, effective au prochain démarrage");
        // Best effort: the exe is in place; the entry is written again at the next start of the installed exe.
        if let Err(error) = record_entry(ops, paths, Some(&verified.version)) {
            tracing::warn!(cause = %error, "mise à jour : entrée « Applications installées » non mise à jour");
        }
        Ok(())
    }
}

/// Highest number tried for the name of a `.old` set aside.
const SET_ASIDE_MAX: u32 = 64;

/// Frees the name `mcp-proxy.exe.old`: removes the previous one, or, when a process still runs it (a bridge of an
/// LLM client, which lives as long as its client), renames it `mcp-proxy.exe.old-<n>.__relocated__.exe`, a name of
/// leftover that the cleanup at start and the uninstallation remove once it is free. Windows lets a running exe be
/// renamed, not removed.
pub fn clear_backup(ops: &dyn SystemOps, paths: &InstallPaths) -> Result<(), OpError> {
    let error = match ops.remove_file(&paths.backup) {
        Err(error) if !error.is_not_found() => error,
        _ => return Ok(()),
    };
    let free = (1..=SET_ASIDE_MAX)
        .map(|n| paths.dir.join(format!("{BACKUP_NAME}-{n}.__relocated__.exe")))
        .find(|candidate| !ops.exists(candidate));
    let Some(aside) = free else { return Err(error) };
    ops.rename(&paths.backup, &aside).map_err(|_| error)?;
    tracing::info!("mise à jour : ancienne version encore utilisée par un client LLM, mise de côté");
    Ok(())
}

/// What the cleanup at start removed, and what it could not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cleanup {
    pub removed: usize,
    pub left: usize,
}

/// A name left by a replacement in the installation folder: `.old`, `.new`, or what `self_replace` would leave.
pub fn is_leftover(name: &str) -> bool {
    name.eq_ignore_ascii_case(BACKUP_NAME)
        || name.eq_ignore_ascii_case(STAGED_NAME)
        || name.to_ascii_lowercase().ends_with(".__relocated__.exe")
        || name.to_ascii_lowercase().ends_with(".__selfdelete__.exe")
}

/// At start: the leftovers of a replacement in the installation folder, and everything in `update\`. Logged as
/// counts, never as paths.
pub fn clean_leftovers(ops: &dyn SystemOps, paths: &UpdatePaths) -> Cleanup {
    let mut cleanup = Cleanup::default();
    let mut count = |result: Result<(), OpError>| match result {
        Ok(()) => cleanup.removed += 1,
        Err(error) if error.is_not_found() => {}
        Err(_) => cleanup.left += 1,
    };
    if let Ok(entries) = ops.entries(&paths.install.dir) {
        for entry in entries {
            let leftover = entry.file_name().and_then(|n| n.to_str()).is_some_and(is_leftover);
            if leftover {
                count(ops.remove_file(&entry));
            }
        }
    }
    if let Ok(entries) = ops.entries(&paths.downloads) {
        for entry in entries {
            let removed = ops.remove_dir_all(&entry).or_else(|_| ops.remove_file(&entry));
            count(removed);
        }
    }
    if cleanup != Cleanup::default() {
        tracing::info!(
            retires = cleanup.removed,
            restes = cleanup.left,
            "mise à jour : restes d'une version précédente nettoyés"
        );
    }
    cleanup
}
