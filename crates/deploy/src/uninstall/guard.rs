//! What may be removed recursively: a folder of ours, checked before anything is touched. A path is accepted only
//! when it is exactly `<parent>\<name>`: an absolute parent of at least two folders, the expected name, no `.` nor
//! `..` anywhere. An empty or relative path, a root, a profile folder or any folder of another name is refused.
//!
//! The walk never follows a link: a link (symbolic link, junction) met inside is removed itself, never what it points
//! at; a folder of ours that is itself a link is left as it is.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::ops::{OpError, PathKind, SystemOps};

/// Why a path is not one of ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    Relative,
    /// A `.` or `..` component, or a parent too short to be a user folder (a root, `C:\Users`…).
    NotNormal,
    /// Not exactly `<parent>\<name>`.
    NotOurs,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "chemin vide",
            Self::Relative => "chemin relatif",
            Self::NotNormal => "chemin qui n'est pas celui d'un dossier de l'utilisateur",
            Self::NotOurs => "dossier qui n'est pas celui de MCP Proxy",
        })
    }
}

/// The least number of folders above one of ours: `C:\Users\<name>\AppData\Local` has four.
const MIN_PARENT_DEPTH: usize = 2;

fn normal(path: &Path) -> bool {
    !path
        .components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
}

/// `path` is exactly `parent` joined with `name`, both well formed.
pub fn check_owned(path: &Path, parent: &Path, name: &str) -> Result<(), Refusal> {
    if path.as_os_str().is_empty() || parent.as_os_str().is_empty() {
        return Err(Refusal::Empty);
    }
    if !path.is_absolute() || !parent.is_absolute() {
        return Err(Refusal::Relative);
    }
    let depth = parent
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if !normal(path) || !normal(parent) || depth < MIN_PARENT_DEPTH {
        return Err(Refusal::NotNormal);
    }
    let simple_name = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\']);
    let ours = simple_name
        && path.file_name().and_then(|n| n.to_str()) == Some(name)
        && path.parent().is_some_and(|p| p.components().eq(parent.components()));
    if ours { Ok(()) } else { Err(Refusal::NotOurs) }
}

/// What removing a folder of ours did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRemoval {
    Removed,
    Absent,
    /// The path is not a plain folder (a link, a file): nothing was touched.
    NotAFolder,
    /// Some entries could not be removed: their paths relative to the folder, and the first cause.
    Partial {
        left: Vec<PathBuf>,
        cause: OpError,
    },
}

fn gone(result: Result<(), OpError>) -> Result<(), OpError> {
    match result {
        Err(error) if !error.is_not_found() => Err(error),
        _ => Ok(()),
    }
}

struct Walk<'a> {
    ops: &'a dyn SystemOps,
    root: &'a Path,
    left: Vec<PathBuf>,
    cause: Option<OpError>,
}

impl Walk<'_> {
    fn failed(&mut self, path: &Path, cause: OpError) {
        let relative = path.strip_prefix(self.root).unwrap_or(path).to_path_buf();
        self.left.push(relative);
        self.cause.get_or_insert(cause);
    }

    /// Empties `dir`, then removes it. Children first; a child that is not inside `dir` is never touched.
    fn remove(&mut self, dir: &Path) {
        let mut entries = match self.ops.entries(dir) {
            Ok(entries) => entries,
            Err(error) if error.is_not_found() => return,
            Err(error) => return self.failed(dir, error),
        };
        entries.sort();
        let before = self.left.len();
        for entry in entries {
            if entry.parent() != Some(dir) {
                continue;
            }
            let result = match self.ops.kind(&entry) {
                Ok(PathKind::Missing) => Ok(()),
                Ok(PathKind::Link) => gone(self.ops.remove_link(&entry)),
                Ok(PathKind::Dir) => {
                    self.remove(&entry);
                    Ok(())
                }
                Ok(PathKind::File | PathKind::Other) => gone(self.ops.remove_file(&entry)),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                self.failed(&entry, error);
            }
        }
        // A child left keeps the folder: it is not reported a second time.
        if let Err(error) = gone(self.ops.remove_dir(dir))
            && self.left.len() == before
        {
            self.failed(dir, error);
        }
    }
}

/// Removes `path`, a folder already checked by [`check_owned`], and everything in it, without following any link.
pub fn remove_tree(ops: &dyn SystemOps, path: &Path) -> Result<TreeRemoval, OpError> {
    match ops.kind(path)? {
        PathKind::Missing => return Ok(TreeRemoval::Absent),
        PathKind::Dir => {}
        PathKind::File | PathKind::Link | PathKind::Other => return Ok(TreeRemoval::NotAFolder),
    }
    let mut walk = Walk {
        ops,
        root: path,
        left: Vec::new(),
        cause: None,
    };
    walk.remove(path);
    Ok(match walk.cause {
        None => TreeRemoval::Removed,
        Some(cause) => TreeRemoval::Partial { left: walk.left, cause },
    })
}
