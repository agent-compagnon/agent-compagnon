//! What installing, updating and uninstalling do to the system, behind one trait: files, the Start menu shortcut,
//! the values of `HKCU` (`Run`, the entry « Applications installées », the AUMID, the exclusion from error reports),
//! the running instance and new processes. The application implements it for Windows (`winreg`, a safe shortcut
//! writer, the authenticated control channel of the pipe, `std::process`, `self_replace`); tests use
//! [`crate::fake::FakeSystem`]. The file operations have default bodies over `std::fs`, so that an implementation
//! only has to provide the rest.
//!
//! Errors carry an error kind and a system code, never a path: they are shown to the user and written to logs.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::uninstall::UninstallEntry;
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

/// What a path is, read without following a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    File,
    Dir,
    /// A symbolic link or a junction: removed itself, never followed.
    Link,
    Other,
}

/// What removing a value or a key of ours did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Removed,
    Absent,
    /// Something is there under our name, but not what MCP Proxy writes: left as it is.
    NotOurs,
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

    /// The whole content of a file.
    fn read(&self, path: &Path) -> Result<Vec<u8>, OpError> {
        Ok(std::fs::read(path)?)
    }

    /// The entries of a directory, in no particular order.
    fn entries(&self, dir: &Path) -> Result<Vec<PathBuf>, OpError> {
        std::fs::read_dir(dir)?
            .map(|entry| Ok(entry?.path()))
            .collect::<Result<_, std::io::Error>>()
            .map_err(OpError::from)
    }

    /// A directory and everything in it.
    fn remove_dir_all(&self, path: &Path) -> Result<(), OpError> {
        Ok(std::fs::remove_dir_all(path)?)
    }

    /// What `path` is, without following a link (under Windows, a junction is a link too).
    fn kind(&self, path: &Path) -> Result<PathKind, OpError> {
        let file_type = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(PathKind::Missing),
            Err(error) => return Err(error.into()),
        };
        Ok(if file_type.is_symlink() {
            PathKind::Link
        } else if file_type.is_dir() {
            PathKind::Dir
        } else if file_type.is_file() {
            PathKind::File
        } else {
            PathKind::Other
        })
    }

    /// The link itself, never what it points at: under Windows, a link to a folder (or a junction) goes as a
    /// folder does.
    fn remove_link(&self, path: &Path) -> Result<(), OpError> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt as _;
            if std::fs::symlink_metadata(path)?.file_type().is_symlink_dir() {
                return Ok(std::fs::remove_dir(path)?);
            }
        }
        Ok(std::fs::remove_file(path)?)
    }

    /// The size of a file, in bytes.
    fn file_len(&self, path: &Path) -> Result<u64, OpError> {
        Ok(std::fs::metadata(path)?.len())
    }

    /// Writes the shortcut whole or not at all (temporary file then rename), replacing an existing one.
    fn write_shortcut(&self, shortcut: &Shortcut) -> Result<(), OpError>;

    /// The `MCP Proxy` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, if any.
    fn run_entry(&self) -> Result<Option<String>, OpError>;

    fn set_run_entry(&self, command: &str) -> Result<(), OpError>;

    /// Succeeds when the value is already absent.
    fn remove_run_entry(&self) -> Result<(), OpError>;

    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy`, if any.
    fn uninstall_entry(&self) -> Result<Option<UninstallEntry>, OpError>;

    /// Creates or rewrites that key with exactly these values.
    fn set_uninstall_entry(&self, entry: &UninstallEntry) -> Result<(), OpError>;

    /// That key, which holds values only: a key with subkeys is not ours.
    fn remove_uninstall_entry(&self) -> Result<Removal, OpError>;

    /// `HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop` (spec 013), a key of values only.
    fn remove_aumid(&self) -> Result<Removal, OpError>;

    /// The DWORD `1` named `mcp-proxy.exe` under `HKCU\Software\Microsoft\Windows\Windows Error
    /// Reporting\ExcludedApplications` (spec 011); any other value of that name is not ours.
    fn remove_wer_exclusion(&self) -> Result<Removal, OpError>;

    /// Schedules the deletion of `exe`, the image of this very process, for once the process has ended. The file
    /// usually leaves its folder at once; it stays there if it could not be moved out.
    fn delete_running_exe(&self, exe: &Path) -> Result<(), OpError>;

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
