//! An in-memory [`SystemOps`] for tests: files, directories, the shortcut, the `Run` value, the running instance and
//! launched processes, with every operation recorded in order and a failure injectable at any of them. Meant for the
//! tests of this crate and of the specs that reuse the trait (updater, uninstallation).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::ops::{OpError, Shortcut, Stopped, SystemOps};
use crate::version::{Installed, version_in};

/// An operation that changes something, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    CreateDir(PathBuf),
    RemoveDir(PathBuf),
    Copy { from: PathBuf, to: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    RemoveFile(PathBuf),
    WriteShortcut(PathBuf),
    SetRun(String),
    RemoveRun,
    StopInstance,
    Launch { exe: PathBuf, args: Vec<String> },
}

/// The instance that holds the pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Instance {
    #[default]
    NotRunning,
    Running,
    /// Acknowledges but never ends.
    Stuck,
}

/// What the fake system holds. Compared whole by the tests: a rolled back operation leaves it as it was.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct World {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub dirs: BTreeSet<PathBuf>,
    pub run: Option<String>,
    pub instance: Instance,
}

#[derive(Debug, Default)]
struct State {
    world: World,
    log: Vec<Op>,
    /// Indices in `log` of the operations that fail.
    fail_at: BTreeSet<usize>,
    launched: Vec<(PathBuf, Vec<String>)>,
    aliases: BTreeMap<PathBuf, PathBuf>,
}

#[derive(Debug, Default)]
pub struct FakeSystem {
    state: Mutex<State>,
}

fn not_found() -> OpError {
    io::Error::from(io::ErrorKind::NotFound).into()
}

fn kind(kind: io::ErrorKind) -> OpError {
    io::Error::from(kind).into()
}

impl FakeSystem {
    pub fn new(world: World) -> Self {
        Self {
            state: Mutex::new(State {
                world,
                ..State::default()
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn world(&self) -> World {
        self.state().world.clone()
    }

    /// Every operation attempted, failed ones included, in order.
    pub fn log(&self) -> Vec<Op> {
        self.state().log.clone()
    }

    /// The processes started, with their arguments.
    pub fn launched(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.state().launched.clone()
    }

    /// The `index`-th operation (0-based, counted from the creation of the fake) fails with « accès refusé ».
    /// Several may fail.
    pub fn fail_at(&self, index: usize) {
        self.state().fail_at.insert(index);
    }

    /// `alias` resolves to `path`, as a short name or a link would.
    pub fn alias(&self, alias: impl Into<PathBuf>, path: impl Into<PathBuf>) {
        self.state().aliases.insert(alias.into(), path.into());
    }

    /// Records `op`, then applies it unless it is the one that fails.
    fn apply(&self, op: Op, change: impl FnOnce(&mut World) -> Result<(), OpError>) -> Result<(), OpError> {
        let mut state = self.state();
        let index = state.log.len();
        state.log.push(op);
        if state.fail_at.contains(&index) {
            return Err(kind(io::ErrorKind::PermissionDenied));
        }
        change(&mut state.world)
    }
}

impl World {
    fn has_dir(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }

    fn parent_exists(&self, path: &Path) -> bool {
        path.parent().is_some_and(|p| self.has_dir(p))
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path)
    }

    fn is_empty_dir(&self, path: &Path) -> bool {
        let child = |p: &PathBuf| p.parent() == Some(path);
        !self.files.keys().any(child) && !self.dirs.iter().any(child)
    }
}

impl SystemOps for FakeSystem {
    fn exists(&self, path: &Path) -> bool {
        let state = self.state();
        let path = state.aliases.get(path).map_or(path, PathBuf::as_path);
        state.world.exists(path)
    }

    fn canonical(&self, path: &Path) -> Option<PathBuf> {
        let state = self.state();
        let path = state.aliases.get(path).cloned().unwrap_or_else(|| path.to_path_buf());
        state.world.exists(&path).then_some(path)
    }

    fn exe_version(&self, exe: &Path) -> Installed {
        self.state()
            .world
            .files
            .get(exe)
            .map_or(Installed::Unknown, |bytes| version_in(bytes))
    }

    fn create_dir(&self, path: &Path) -> Result<(), OpError> {
        self.apply(Op::CreateDir(path.to_path_buf()), |w| {
            if w.exists(path) {
                return Err(kind(io::ErrorKind::AlreadyExists));
            }
            if !w.parent_exists(path) {
                return Err(not_found());
            }
            w.dirs.insert(path.to_path_buf());
            Ok(())
        })
    }

    fn remove_dir(&self, path: &Path) -> Result<(), OpError> {
        self.apply(Op::RemoveDir(path.to_path_buf()), |w| {
            if !w.has_dir(path) {
                return Err(not_found());
            }
            if !w.is_empty_dir(path) {
                return Err(kind(io::ErrorKind::DirectoryNotEmpty));
            }
            w.dirs.remove(path);
            Ok(())
        })
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), OpError> {
        let op = Op::Copy {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        };
        self.apply(op, |w| {
            let content = w.files.get(from).cloned().ok_or_else(not_found)?;
            if !w.parent_exists(to) {
                return Err(not_found());
            }
            w.files.insert(to.to_path_buf(), content);
            Ok(())
        })
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), OpError> {
        let op = Op::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        };
        self.apply(op, |w| {
            if !w.parent_exists(to) {
                return Err(not_found());
            }
            let content = w.files.remove(from).ok_or_else(not_found)?;
            w.files.insert(to.to_path_buf(), content);
            Ok(())
        })
    }

    fn remove_file(&self, path: &Path) -> Result<(), OpError> {
        self.apply(Op::RemoveFile(path.to_path_buf()), |w| {
            w.files.remove(path).map(|_| ()).ok_or_else(not_found)
        })
    }

    fn write_shortcut(&self, shortcut: &Shortcut) -> Result<(), OpError> {
        self.apply(Op::WriteShortcut(shortcut.path.clone()), |w| {
            if !w.parent_exists(&shortcut.path) || !w.files.contains_key(&shortcut.target) {
                return Err(not_found());
            }
            let content = format!("lnk:{}", shortcut.target.display()).into_bytes();
            w.files.insert(shortcut.path.clone(), content);
            Ok(())
        })
    }

    fn run_entry(&self) -> Result<Option<String>, OpError> {
        Ok(self.state().world.run.clone())
    }

    fn set_run_entry(&self, command: &str) -> Result<(), OpError> {
        self.apply(Op::SetRun(command.to_owned()), |w| {
            w.run = Some(command.to_owned());
            Ok(())
        })
    }

    fn remove_run_entry(&self) -> Result<(), OpError> {
        self.apply(Op::RemoveRun, |w| {
            w.run = None;
            Ok(())
        })
    }

    fn stop_instance(&self) -> Result<Stopped, OpError> {
        let mut outcome = Stopped::NotRunning;
        self.apply(Op::StopInstance, |w| match w.instance {
            Instance::NotRunning => Ok(()),
            Instance::Running => {
                w.instance = Instance::NotRunning;
                outcome = Stopped::Stopped;
                Ok(())
            }
            Instance::Stuck => Err(kind(io::ErrorKind::TimedOut)),
        })?;
        Ok(outcome)
    }

    fn launch(&self, exe: &Path, args: &[&str]) -> Result<(), OpError> {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let op = Op::Launch {
            exe: exe.to_path_buf(),
            args: args.clone(),
        };
        let launched = self.apply(op, |w| {
            if w.files.contains_key(exe) {
                Ok(())
            } else {
                Err(not_found())
            }
        });
        if launched.is_ok() {
            self.state().launched.push((exe.to_path_buf(), args));
        }
        launched
    }
}
