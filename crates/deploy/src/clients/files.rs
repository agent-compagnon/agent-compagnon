//! The files of the clients, behind a trait so that the tests can make any step fail. Writing a file that belongs to
//! another program: a backup first, the new text in a temporary file of the same folder, a last look at the file,
//! then a rename over it. Whatever fails, the file is left as it was and our own files are removed.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::ops::OpError;

use super::{ClientError, MAX_FILE_SIZE};

/// What a path designates, links not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Missing,
    File,
    Dir,
    /// A link, a device…: never read nor replaced.
    Other,
}

pub trait ClientFiles: Send + Sync {
    fn kind(&self, path: &Path) -> io::Result<Kind> {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => Ok(Kind::Other),
            Ok(meta) if meta.is_file() => Ok(Kind::File),
            Ok(meta) if meta.is_dir() => Ok(Kind::Dir),
            Ok(_) => Ok(Kind::Other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Kind::Missing),
            Err(error) => Err(error),
        }
    }

    /// At most `MAX_FILE_SIZE + 1` bytes: more means too large.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(MAX_FILE_SIZE + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// A new file with `bytes`, flushed to the disk; fails if `path` exists.
    fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    /// Replaces `to`.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

/// The real files.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFiles;

impl ClientFiles for RealFiles {}

/// `name` beside `file`.
fn sibling(file: &Path, name: &str) -> PathBuf {
    file.with_file_name(name)
}

fn file_name(file: &Path) -> String {
    file.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The bytes of the file now; `None` when it does not exist.
pub(crate) fn current(files: &dyn ClientFiles, file: &Path) -> Result<Option<Vec<u8>>, ClientError> {
    match files.kind(file).map_err(|e| ClientError::Io(e.into()))? {
        Kind::Missing => Ok(None),
        Kind::File => files.read(file).map(Some).map_err(|e| ClientError::Io(e.into())),
        Kind::Dir | Kind::Other => Err(ClientError::Changed),
    }
}

/// A stamp is digits and dashes (`20261122-140307`): it goes in a file name.
fn valid_stamp(stamp: &str) -> bool {
    !stamp.is_empty() && stamp.len() <= 32 && stamp.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

/// Writes `text` over `file`, which held `original` (`None`: absent). Returns the name of the backup, if any.
pub(crate) fn write(
    files: &dyn ClientFiles,
    file: &Path,
    original: Option<&[u8]>,
    text: &str,
    stamp: &str,
) -> Result<Option<String>, ClientError> {
    if !valid_stamp(stamp) {
        return Err(ClientError::Refused("horodatage de sauvegarde invalide"));
    }
    let name = file_name(file);
    let backup = match original {
        Some(bytes) => Some(backup(files, file, &name, bytes, stamp)?),
        None => None,
    };
    let undo_backup = |files: &dyn ClientFiles| {
        if let Some(path) = &backup {
            let _ = files.remove_file(path);
        }
    };
    let mut random = [0u8; 6];
    if getrandom::fill(&mut random).is_err() {
        undo_backup(files);
        return Err(ClientError::Refused("nom de fichier temporaire impossible à tirer"));
    }
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temp = sibling(file, &format!("{name}.mcp-proxy-{suffix}.tmp"));
    if let Err(error) = files.create_new(&temp, text.as_bytes()) {
        let _ = files.remove_file(&temp);
        undo_backup(files);
        return Err(ClientError::Io(error.into()));
    }
    // The client may have written the file since it was read: what is replaced is what was shown.
    let still = current(files, file);
    if !matches!(&still, Ok(now) if now.as_deref() == original) {
        let _ = files.remove_file(&temp);
        undo_backup(files);
        return Err(match still {
            Err(ClientError::Io(error)) => ClientError::Io(error),
            _ => ClientError::Changed,
        });
    }
    if let Err(error) = files.rename(&temp, file) {
        let _ = files.remove_file(&temp);
        undo_backup(files);
        return Err(ClientError::Io(error.into()));
    }
    Ok(backup.map(|path| file_name(&path)))
}

/// `<file>.bak-<stamp>`, or `-2` to `-9` after it when that name is taken. Never replaces a file.
fn backup(files: &dyn ClientFiles, file: &Path, name: &str, bytes: &[u8], stamp: &str) -> Result<PathBuf, ClientError> {
    for n in 1..=9 {
        let candidate = if n == 1 {
            format!("{name}.bak-{stamp}")
        } else {
            format!("{name}.bak-{stamp}-{n}")
        };
        let path = sibling(file, &candidate);
        match files.create_new(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                let _ = files.remove_file(&path);
                return Err(ClientError::Backup(OpError::from(error)));
            }
        }
    }
    Err(ClientError::Backup(OpError::Refused(
        "trop de sauvegardes portent déjà cet horodatage",
    )))
}
