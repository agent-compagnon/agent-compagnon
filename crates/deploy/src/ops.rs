//! What installing, updating and uninstalling do to the system, behind one trait: files, the Start menu shortcut,
//! the `Run` value of `HKCU`, the running instance and new processes. The application implements it for Windows
//! (`winreg`, a safe shortcut writer, the authenticated control channel of the pipe, `std::process`); tests use
//! [`crate::fake::FakeSystem`]. The file operations have default bodies over `std::fs`, so that an implementation
//! only has to provide the rest.
//!
//! Errors carry an error kind and a system code, never a path: they are shown to the user and written to logs.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::version::{Installed, installed_version};

/// A failed operation: what the system said, without any path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    Io {
        kind: io::ErrorKind,
        code: Option<i32>,
    },
    /// A fixed French text, for what is not an I/O error (a path the shortcut format cannot hold…).
    Refused(&'static str),
}

/// `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`: the file is held by another program.
const SHARING_CODES: [i32; 2] = [32, 33];

impl OpError {
    pub fn kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io { kind, .. } => Some(*kind),
            Self::Refused(_) => None,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.kind() == Some(io::ErrorKind::NotFound)
    }
}

impl From<io::Error> for OpError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            kind: error.kind(),
            code: error.raw_os_error(),
        }
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, code) = match self {
            Self::Refused(text) => return f.write_str(text),
            Self::Io { kind, code } => (*kind, *code),
        };
        if code.is_some_and(|c| SHARING_CODES.contains(&c)) {
            return f.write_str("fichier utilisé par un autre programme");
        }
        let text = match kind {
            io::ErrorKind::PermissionDenied => "accès refusé",
            io::ErrorKind::NotFound => "dossier ou fichier introuvable",
            io::ErrorKind::AlreadyExists => "un fichier du même nom existe déjà",
            io::ErrorKind::StorageFull => "disque plein",
            io::ErrorKind::ReadOnlyFilesystem => "disque en lecture seule",
            io::ErrorKind::DirectoryNotEmpty => "dossier non vide",
            io::ErrorKind::TimedOut => "délai dépassé",
            _ => {
                return match code {
                    Some(code) => write!(f, "erreur système {code}"),
                    None => write!(f, "erreur d'entrée-sortie ({kind})"),
                };
            }
        };
        f.write_str(text)
    }
}

/// What asking the running instance to stop gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// No instance held the pipe.
    NotRunning,
    /// An instance proved itself, acknowledged, and its process ended within the bound.
    Stopped,
}

/// A Start menu shortcut to write: `MCP Proxy.lnk` pointing at the installed exe, without arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub path: PathBuf,
    pub target: PathBuf,
}

pub trait SystemOps: Send + Sync {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// The path with every link and short name resolved; `None` if it does not exist.
    fn canonical(&self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }

    /// The version mark of an exe, read without executing it.
    fn exe_version(&self, exe: &Path) -> Installed {
        installed_version(exe)
    }

    /// One directory, whose parent exists.
    fn create_dir(&self, path: &Path) -> Result<(), OpError> {
        Ok(std::fs::create_dir(path)?)
    }

    /// An empty directory.
    fn remove_dir(&self, path: &Path) -> Result<(), OpError> {
        Ok(std::fs::remove_dir(path)?)
    }

    /// Replaces `to` if it exists.
    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), OpError> {
        std::fs::copy(from, to)?;
        Ok(())
    }

    /// Replaces `to` if it exists. Under Windows, an exe that runs can be renamed, not replaced nor deleted.
    fn rename(&self, from: &Path, to: &Path) -> Result<(), OpError> {
        Ok(std::fs::rename(from, to)?)
    }

    fn remove_file(&self, path: &Path) -> Result<(), OpError> {
        Ok(std::fs::remove_file(path)?)
    }

    /// Writes the shortcut whole or not at all (temporary file then rename), replacing an existing one.
    fn write_shortcut(&self, shortcut: &Shortcut) -> Result<(), OpError>;

    /// The `MCP Proxy` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, if any.
    fn run_entry(&self) -> Result<Option<String>, OpError>;

    fn set_run_entry(&self, command: &str) -> Result<(), OpError>;

    /// Succeeds when the value is already absent.
    fn remove_run_entry(&self) -> Result<(), OpError>;

    /// Asks the instance that holds the pipe to stop, through the authenticated control channel only (mutual proof
    /// of spec 011, control role), and waits, bounded, for its process to end.
    fn stop_instance(&self) -> Result<Stopped, OpError>;

    /// Starts `exe` detached, with `args`, in the exe's own directory.
    fn launch(&self, exe: &Path, args: &[&str]) -> Result<(), OpError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_happened_without_any_path() {
        let denied = OpError::from(io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(denied.to_string(), "accès refusé");
        let held = OpError::Io {
            kind: io::ErrorKind::Other,
            code: Some(32),
        };
        assert_eq!(held.to_string(), "fichier utilisé par un autre programme");
        let other = OpError::Io {
            kind: io::ErrorKind::Other,
            code: Some(1234),
        };
        assert_eq!(other.to_string(), "erreur système 1234");
        assert!(OpError::from(io::Error::from(io::ErrorKind::NotFound)).is_not_found());
    }
}
