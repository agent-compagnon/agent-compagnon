//! Installing and updating: every step is recorded, and a step that fails undoes the previous ones in reverse
//! order, so that nothing partial stays behind. The exe is copied under a temporary name in the installation folder,
//! then renamed: the installed exe is either the previous one or the complete new one.
//!
//! The journal names the steps and the system's error kinds, never a path (they hold the user's name).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::install::autostart::run_command;
use crate::install::paths::InstallPaths;
use crate::ops::{OpError, Shortcut, Stopped, SystemOps};
use crate::uninstall::{UninstallEntry, record_entry};
use crate::version::Installed;

/// Argument of a relaunch that must not offer the installation again.
pub const NO_INSTALL_ARG: &str = "--no-install";
/// Argument of the `Run` value: start without a window.
pub const AUTOSTART_ARG: &str = "--autostart";

/// A step of the installation, as the user reads it in a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    StopInstance,
    CreateDir,
    Copy,
    SetAside,
    PutInPlace,
    Shortcut,
    Autostart,
    UninstallEntry,
    Launch,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Self::StopInstance => "Arrêt de MCP Proxy en cours",
            Self::CreateDir => "Création du dossier d'installation",
            Self::Copy => "Copie de l'exe",
            Self::SetAside => "Mise de côté de la version installée",
            Self::PutInPlace => "Mise en place de l'exe",
            Self::Shortcut => "Création du raccourci du menu Démarrer",
            Self::Autostart => "Inscription du démarrage avec Windows",
            Self::UninstallEntry => "Inscription dans « Applications installées »",
            Self::Launch => "Lancement depuis le dossier d'installation",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::StopInstance => "arrêt de l'instance",
            Self::CreateDir => "dossier",
            Self::Copy => "copie",
            Self::SetAside => "mise de côté",
            Self::PutInPlace => "mise en place",
            Self::Shortcut => "raccourci",
            Self::Autostart => "démarrage automatique",
            Self::UninstallEntry => "entrée de désinstallation",
            Self::Launch => "lancement",
        }
    }
}

/// What an undo could not remove or restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leftover {
    Dir,
    Copy,
    /// The previous exe could not be put back: it is still under its temporary name.
    Previous,
    Shortcut,
    Autostart,
    UninstallEntry,
}

impl Leftover {
    fn label(self) -> &'static str {
        match self {
            Self::Dir => "le dossier d'installation",
            Self::Copy => "une copie de l'exe dans le dossier d'installation",
            Self::Previous => {
                "la version installée, restée sous le nom « mcp-proxy.exe.old » : renommez-la en « mcp-proxy.exe »"
            }
            Self::Shortcut => "le raccourci du menu Démarrer",
            Self::Autostart => "l'inscription au démarrage de Windows",
            Self::UninstallEntry => "l'entrée « MCP Proxy » des applications installées",
        }
    }
}

/// Why the installation or the update did not happen. French, shown in the installation window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// The running instance did not stop: nothing was changed.
    NotStopped(OpError),
    /// `step` failed; everything done before was undone, except `left`.
    Failed {
        step: Step,
        cause: OpError,
        left: Vec<Leftover>,
    },
    /// The installed exe, or this one, could not be started.
    Launch(OpError),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStopped(cause) => write!(
                f,
                "MCP Proxy, déjà ouvert, n'a pas pu être fermé ({cause}). Rien n'a été modifié. Quittez MCP Proxy \
                 depuis son icône, puis réessayez."
            ),
            Self::Failed { step, cause, left } if left.is_empty() => write!(
                f,
                "{} impossible ({cause}). Rien n'a été modifié : tout ce qui avait été fait a été retiré.",
                step.label()
            ),
            Self::Failed { step, cause, left } => {
                let left: Vec<&str> = left.iter().map(|l| l.label()).collect();
                write!(
                    f,
                    "{} impossible ({cause}). Ce qui avait été fait a été retiré, sauf : {}.",
                    step.label(),
                    left.join(" ; ")
                )
            }
            Self::Launch(cause) => write!(f, "MCP Proxy n'a pas pu être lancé ({cause})."),
        }
    }
}

/// One thing done, and how to undo it.
#[derive(Debug)]
enum Done {
    Dir(PathBuf),
    Staged(PathBuf),
    /// The installed exe renamed to `aside`.
    SetAside {
        exe: PathBuf,
        aside: PathBuf,
    },
    Placed(PathBuf),
    Shortcut(PathBuf),
    /// The `Run` value before the change.
    Autostart(Option<String>),
    /// The entry « Applications installées » before the change.
    UninstallEntry(Option<UninstallEntry>),
}

/// The steps done so far, undone in reverse order on failure.
struct Transaction<'a> {
    ops: &'a dyn SystemOps,
    done: Vec<Done>,
}

impl<'a> Transaction<'a> {
    fn new(ops: &'a dyn SystemOps) -> Self {
        Self { ops, done: Vec::new() }
    }

    fn step(&mut self, step: Step, result: Result<(), OpError>, done: Option<Done>) -> Result<(), InstallError> {
        match result {
            Ok(()) => {
                tracing::debug!(step = step.name(), "installation : étape faite");
                self.done.extend(done);
                Ok(())
            }
            Err(cause) => {
                tracing::warn!(step = step.name(), cause = %cause, "installation : étape en échec, retour arrière");
                let left = self.undo();
                if !left.is_empty() {
                    tracing::error!(restes = left.len(), "installation : retour arrière incomplet");
                }
                Err(InstallError::Failed { step, cause, left })
            }
        }
    }

    /// Undoes everything, last first. An element already gone counts as undone.
    fn undo(&mut self) -> Vec<Leftover> {
        let ops = self.ops;
        let gone = |result: Result<(), OpError>| match result {
            Err(error) if !error.is_not_found() => Err(error),
            _ => Ok(()),
        };
        let mut left = Vec::new();
        while let Some(done) = self.done.pop() {
            let (result, leftover) = match done {
                Done::Dir(dir) => (gone(ops.remove_dir(&dir)), Leftover::Dir),
                Done::Staged(file) | Done::Placed(file) => (gone(ops.remove_file(&file)), Leftover::Copy),
                Done::SetAside { exe, aside } => (ops.rename(&aside, &exe), Leftover::Previous),
                Done::Shortcut(path) => (gone(ops.remove_file(&path)), Leftover::Shortcut),
                Done::Autostart(Some(previous)) => (ops.set_run_entry(&previous), Leftover::Autostart),
                Done::Autostart(None) => (ops.remove_run_entry(), Leftover::Autostart),
                Done::UninstallEntry(Some(previous)) => (ops.set_uninstall_entry(&previous), Leftover::UninstallEntry),
                Done::UninstallEntry(None) => (ops.remove_uninstall_entry().map(|_| ()), Leftover::UninstallEntry),
            };
            if let Err(error) = result {
                tracing::warn!(cause = %error, "installation : retour arrière d'une étape en échec");
                if !left.contains(&leftover) {
                    left.push(leftover);
                }
            }
        }
        left
    }

    /// Creates `dir` and its missing parents, each recorded.
    fn create_dirs(&mut self, dir: &Path) -> Result<(), InstallError> {
        let missing: Vec<&Path> = dir.ancestors().take_while(|d| !self.ops.exists(d)).collect();
        for dir in missing.into_iter().rev() {
            let result = self.ops.create_dir(dir);
            self.step(Step::CreateDir, result, Some(Done::Dir(dir.to_path_buf())))?;
        }
        Ok(())
    }

    /// Copies `source` under the temporary name, then renames it over `paths.exe`.
    fn put_exe(&mut self, paths: &InstallPaths, source: &Path) -> Result<(), InstallError> {
        // A copy left by an interrupted run is replaced.
        let result = self.ops.copy_file(source, &paths.staged);
        self.step(Step::Copy, result, Some(Done::Staged(paths.staged.clone())))?;
        let result = self.ops.rename(&paths.staged, &paths.exe);
        self.step(Step::PutInPlace, result, Some(Done::Placed(paths.exe.clone())))
    }

    fn shortcut(&mut self, paths: &InstallPaths, replace: bool) -> Result<(), InstallError> {
        let existed = self.ops.exists(&paths.shortcut);
        if existed && !replace {
            return Ok(());
        }
        let shortcut = Shortcut {
            path: paths.shortcut.clone(),
            target: paths.exe.clone(),
        };
        let result = self.ops.write_shortcut(&shortcut);
        let done = (!existed).then(|| Done::Shortcut(paths.shortcut.clone()));
        self.step(Step::Shortcut, result, done)
    }

    /// The `Run` value as wanted: the installed exe with `--autostart`, or none.
    fn autostart(&mut self, paths: &InstallPaths, wanted: bool) -> Result<(), InstallError> {
        let previous = match self.ops.run_entry() {
            Ok(previous) => previous,
            Err(cause) => return self.step(Step::Autostart, Err(cause), None),
        };
        let command = if wanted {
            match run_command(&paths.exe) {
                Some(command) => Some(command),
                None => {
                    let refused = OpError::Refused("chemin d'installation impossible à inscrire");
                    return self.step(Step::Autostart, Err(refused), None);
                }
            }
        } else {
            None
        };
        if previous == command {
            return Ok(());
        }
        let result = match &command {
            Some(command) => self.ops.set_run_entry(command),
            None => self.ops.remove_run_entry(),
        };
        self.step(Step::Autostart, result, Some(Done::Autostart(previous)))
    }

    /// The entry « Applications installées » of the exe now in place, its version read in the file (spec 022).
    fn uninstall_entry(&mut self, paths: &InstallPaths) -> Result<(), InstallError> {
        let version = match self.ops.exe_version(&paths.exe) {
            Installed::Known(version) => Some(version),
            Installed::Unknown => None,
        };
        match record_entry(self.ops, paths, version.as_ref()) {
            Ok(recorded) if !recorded.changed => Ok(()),
            Ok(recorded) => self.step(
                Step::UninstallEntry,
                Ok(()),
                Some(Done::UninstallEntry(recorded.previous)),
            ),
            Err(cause) => self.step(Step::UninstallEntry, Err(cause), None),
        }
    }

    fn launch(&mut self, exe: &Path) -> Result<(), InstallError> {
        let result = self.ops.launch(exe, &[]);
        self.step(Step::Launch, result, None)
    }
}

/// Stops the instance that holds the pipe, whichever exe it runs. Before anything is changed: an instance that does
/// not stop leaves everything as it was.
fn stop_running(ops: &dyn SystemOps) -> Result<(), InstallError> {
    match ops.stop_instance() {
        Ok(Stopped::NotRunning) => Ok(()),
        Ok(Stopped::Stopped) => {
            tracing::info!("installation : MCP Proxy en cours arrêté par le canal de contrôle");
            Ok(())
        }
        Err(cause) => {
            tracing::warn!(cause = %cause, "installation : MCP Proxy en cours ne s'est pas arrêté");
            Err(InstallError::NotStopped(cause))
        }
    }
}

/// A file left by an interrupted run, removed before it is needed; one that cannot be removed only makes the step
/// that needs its name fail.
fn clear_leftover(ops: &dyn SystemOps, path: &Path) {
    if ops.exists(path) {
        let _ = ops.remove_file(path);
    }
}

/// First installation: stop a running instance (one launched from elsewhere), folder, copy, shortcut, `Run` value if
/// `autostart`, entry « Applications installées », then the installed exe is launched. The caller then stops.
pub fn install(ops: &dyn SystemOps, paths: &InstallPaths, source: &Path, autostart: bool) -> Result<(), InstallError> {
    stop_running(ops)?;
    let mut transaction = Transaction::new(ops);
    transaction.create_dirs(&paths.dir)?;
    clear_leftover(ops, &paths.staged);
    transaction.put_exe(paths, source)?;
    transaction.shortcut(paths, true)?;
    transaction.autostart(paths, autostart)?;
    transaction.uninstall_entry(paths)?;
    transaction.launch(&paths.exe)?;
    tracing::info!(autostart, "installation terminée");
    Ok(())
}

/// Update of the installation: stop the running instance, copy, set the installed exe aside (an exe still held by a
/// bridge can be renamed), put the new one in place, its entry « Applications installées », shortcut if missing,
/// launch. The `Run` value is left as it is:
/// it names the same path. The exe set aside is removed once the new one runs, or left for the next update if a
/// bridge still holds it.
pub fn update(ops: &dyn SystemOps, paths: &InstallPaths, source: &Path) -> Result<(), InstallError> {
    stop_running(ops)?;
    let mut transaction = Transaction::new(ops);
    clear_leftover(ops, &paths.staged);
    let result = ops.copy_file(source, &paths.staged);
    transaction.step(Step::Copy, result, Some(Done::Staged(paths.staged.clone())))?;
    clear_leftover(ops, &paths.backup);
    let result = ops.rename(&paths.exe, &paths.backup);
    let aside = Done::SetAside {
        exe: paths.exe.clone(),
        aside: paths.backup.clone(),
    };
    transaction.step(Step::SetAside, result, Some(aside))?;
    let result = ops.rename(&paths.staged, &paths.exe);
    transaction.step(Step::PutInPlace, result, Some(Done::Placed(paths.exe.clone())))?;
    transaction.uninstall_entry(paths)?;
    transaction.shortcut(paths, false)?;
    transaction.launch(&paths.exe)?;
    if let Err(cause) = ops.remove_file(&paths.backup) {
        tracing::info!(cause = %cause, "mise à jour : ancienne version laissée de côté, retirée à la prochaine");
    }
    tracing::info!("mise à jour de l'installation terminée");
    Ok(())
}

/// Hands over to the installed exe: launched with `--autostart` when this launch had it.
pub fn delegate(ops: &dyn SystemOps, paths: &InstallPaths, autostart: bool) -> Result<(), InstallError> {
    let args: &[&str] = if autostart { &[AUTOSTART_ARG] } else { &[] };
    ops.launch(&paths.exe, args).map_err(|cause| {
        tracing::warn!(cause = %cause, "exe installé impossible à lancer");
        InstallError::Launch(cause)
    })
}

/// This exe again, from where it is, without offering the installation.
pub fn run_here(ops: &dyn SystemOps, source: &Path) -> Result<(), InstallError> {
    ops.launch(source, &[NO_INSTALL_ARG]).map_err(InstallError::Launch)
}
