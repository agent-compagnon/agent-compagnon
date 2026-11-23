//! An in-memory [`SystemOps`] for tests: files, directories, links, the shortcut, the values of the registry, the
//! running instance and launched processes, with every operation recorded in order and a failure injectable at any of them. Meant for the
//! tests of this crate and of the specs that reuse the trait (updater, uninstallation). [`FakeReplacer`] stands for
//! the replacement of the exe in the tests of the updater.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::ops::{OpError, PathKind, Removal, Shortcut, Stopped, SystemOps};
use crate::uninstall::UninstallEntry;
use crate::update::{ExeReplacer, ReplaceError, VerifiedExe};
use crate::version::{Installed, Version, version_in};

/// An operation that changes something, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    CreateDir(PathBuf),
    RemoveDir(PathBuf),
    Copy { from: PathBuf, to: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    RemoveFile(PathBuf),
    RemoveDirAll(PathBuf),
    WriteShortcut(PathBuf),
    SetRun(String),
    RemoveRun,
    RemoveLink(PathBuf),
    SetUninstallEntry(UninstallEntry),
    RemoveUninstallEntry,
    RemoveAumid,
    RemoveWerExclusion,
    DeleteRunningExe(PathBuf),
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
    /// Symbolic links and junctions: never followed, removed themselves.
    pub links: BTreeSet<PathBuf>,
    pub run: Option<String>,
    pub uninstall: Option<UninstallEntry>,
    /// The key of the AUMID exists.
    pub aumid: bool,
    /// The value `mcp-proxy.exe` of the exclusion from error reports.
    pub wer: Option<u32>,
    /// Files and folders with the read-only attribute.
    pub read_only: BTreeSet<PathBuf>,
    /// Exes whose deletion is scheduled for the end of the process.
    pub deleted_at_exit: Vec<PathBuf>,
    /// The exe scheduled for deletion stays in its folder until the end of the process (it could not be moved).
    pub deletes_in_place: bool,
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
    /// Paths held by another program: removing or renaming them fails, every time.
    held: BTreeSet<PathBuf>,
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

    /// `path` is held by another program: removing or renaming it fails with a sharing violation, until the end.
    pub fn hold(&self, path: &Path) {
        self.state().held.insert(path.to_path_buf());
    }

    /// `alias` resolves to `path`, as a short name or a link would.
    pub fn alias(&self, alias: impl Into<PathBuf>, path: impl Into<PathBuf>) {
        self.state().aliases.insert(alias.into(), path.into());
    }

    /// Records `op`, then applies it unless it is the one that fails or it touches a held path.
    fn apply(&self, op: Op, change: impl FnOnce(&mut World) -> Result<(), OpError>) -> Result<(), OpError> {
        let mut state = self.state();
        let index = state.log.len();
        let held = match &op {
            Op::RemoveFile(path) | Op::RemoveDir(path) | Op::RemoveLink(path) | Op::Rename { from: path, .. } => {
                state.held.contains(path)
            }
            Op::RemoveDirAll(path) => state.held.iter().any(|h| h.starts_with(path)),
            _ => false,
        };
        state.log.push(op);
        if state.fail_at.contains(&index) {
            return Err(kind(io::ErrorKind::PermissionDenied));
        }
        if held {
            return Err(OpError::Io {
                kind: io::ErrorKind::Other,
                code: Some(32),
            });
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
        self.files.contains_key(path) || self.dirs.contains(path) || self.links.contains(path)
    }

    fn is_empty_dir(&self, path: &Path) -> bool {
        let child = |p: &PathBuf| p.parent() == Some(path);
        !self.files.keys().any(child) && !self.dirs.iter().any(child) && !self.links.iter().any(child)
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

    fn read(&self, path: &Path) -> Result<Vec<u8>, OpError> {
        self.state().world.files.get(path).cloned().ok_or_else(not_found)
    }

    fn entries(&self, dir: &Path) -> Result<Vec<PathBuf>, OpError> {
        let state = self.state();
        if !state.world.has_dir(dir) {
            return Err(not_found());
        }
        let child = |p: &&PathBuf| p.parent() == Some(dir);
        Ok(state
            .world
            .files
            .keys()
            .filter(child)
            .chain(state.world.dirs.iter().filter(child))
            .chain(state.world.links.iter().filter(child))
            .cloned()
            .collect())
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), OpError> {
        self.apply(Op::RemoveDirAll(path.to_path_buf()), |w| {
            if !w.has_dir(path) {
                return Err(not_found());
            }
            w.files.retain(|p, _| !p.starts_with(path));
            w.dirs.retain(|p| !p.starts_with(path));
            w.links.retain(|p| !p.starts_with(path));
            Ok(())
        })
    }

    fn kind(&self, path: &Path) -> Result<PathKind, OpError> {
        let world = &self.state().world;
        Ok(if world.links.contains(path) {
            PathKind::Link
        } else if world.dirs.contains(path) {
            PathKind::Dir
        } else if world.files.contains_key(path) {
            PathKind::File
        } else {
            PathKind::Missing
        })
    }

    fn remove_link(&self, path: &Path) -> Result<(), OpError> {
        self.apply(Op::RemoveLink(path.to_path_buf()), |w| {
            if w.links.remove(path) { Ok(()) } else { Err(not_found()) }
        })
    }

    fn file_len(&self, path: &Path) -> Result<u64, OpError> {
        let world = &self.state().world;
        world
            .files
            .get(path)
            .map(|bytes| bytes.len() as u64)
            .ok_or_else(not_found)
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

    fn uninstall_entry(&self) -> Result<Option<UninstallEntry>, OpError> {
        Ok(self.state().world.uninstall.clone())
    }

    fn set_uninstall_entry(&self, entry: &UninstallEntry) -> Result<(), OpError> {
        self.apply(Op::SetUninstallEntry(entry.clone()), |w| {
            w.uninstall = Some(entry.clone());
            Ok(())
        })
    }

    fn remove_uninstall_entry(&self) -> Result<Removal, OpError> {
        let mut removal = Removal::Absent;
        self.apply(Op::RemoveUninstallEntry, |w| {
            if w.uninstall.take().is_some() {
                removal = Removal::Removed;
            }
            Ok(())
        })?;
        Ok(removal)
    }

    fn remove_aumid(&self) -> Result<Removal, OpError> {
        let mut removal = Removal::Absent;
        self.apply(Op::RemoveAumid, |w| {
            if std::mem::take(&mut w.aumid) {
                removal = Removal::Removed;
            }
            Ok(())
        })?;
        Ok(removal)
    }

    fn aumid_registered(&self) -> Result<bool, OpError> {
        Ok(self.state().world.aumid)
    }

    fn wer_exclusion(&self) -> Result<Option<u32>, OpError> {
        Ok(self.state().world.wer)
    }

    fn read_only(&self, path: &Path) -> Result<bool, OpError> {
        let state = self.state();
        if !state.world.exists(path) {
            return Err(not_found());
        }
        Ok(state.world.read_only.contains(path))
    }

    fn remove_wer_exclusion(&self) -> Result<Removal, OpError> {
        let mut removal = Removal::Absent;
        self.apply(Op::RemoveWerExclusion, |w| {
            match w.wer {
                Some(1) => {
                    w.wer = None;
                    removal = Removal::Removed;
                }
                Some(_) => removal = Removal::NotOurs,
                None => {}
            }
            Ok(())
        })?;
        Ok(removal)
    }

    fn delete_running_exe(&self, exe: &Path) -> Result<(), OpError> {
        self.apply(Op::DeleteRunningExe(exe.to_path_buf()), |w| {
            if !w.files.contains_key(exe) {
                return Err(not_found());
            }
            if !w.deletes_in_place {
                w.files.remove(exe);
            }
            w.deleted_at_exit.push(exe.to_path_buf());
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

/// An [`ExeReplacer`] that records what it is asked to put in place: the version, and the bytes found at the
/// verified path at the time of the call. Once it succeeds, the installed version is the new one.
#[derive(Debug)]
pub struct FakeReplacer {
    state: Mutex<ReplacerState>,
}

#[derive(Debug)]
struct ReplacerState {
    installed: Installed,
    failure: Option<ReplaceError>,
    calls: Vec<(Version, Vec<u8>)>,
}

impl FakeReplacer {
    pub fn new(installed: Version) -> Self {
        Self {
            state: Mutex::new(ReplacerState {
                installed: Installed::Known(installed),
                failure: None,
                calls: Vec::new(),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, ReplacerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every call, in order: version and bytes of the verified exe.
    pub fn calls(&self) -> Vec<(Version, Vec<u8>)> {
        self.state().calls.clone()
    }

    pub fn set_installed(&self, installed: Installed) {
        self.state().installed = installed;
    }

    /// Every later call fails with `error`.
    pub fn fail_with(&self, error: ReplaceError) {
        self.state().failure = Some(error);
    }
}

impl ExeReplacer for FakeReplacer {
    fn installed(&self) -> Installed {
        self.state().installed.clone()
    }

    fn replace(&self, verified: &VerifiedExe) -> Result<(), ReplaceError> {
        let bytes = std::fs::read(&verified.path).unwrap_or_default();
        let mut state = self.state();
        state.calls.push((verified.version.clone(), bytes));
        if let Some(error) = state.failure.clone() {
            return Err(error);
        }
        state.installed = Installed::Known(verified.version.clone());
        Ok(())
    }
}
