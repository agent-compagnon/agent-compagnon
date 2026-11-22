#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mcp_proxy_testkit::CapturedLogs;
use serde_json::json;

use super::*;
use crate::fake::{FakeSystem, Instance, Op, World};
use crate::ops::{OpError, Shortcut, Stopped, SystemOps};
use crate::version::{Installed, Version, mark_prefix};

const LOCAL: &str = "/home/ana/AppData/Local";
const ROAMING: &str = "/home/ana/AppData/Roaming";
const SOURCE: &str = "/home/ana/Downloads/mcp-proxy.exe";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn paths() -> InstallPaths {
    InstallPaths::new(Path::new(LOCAL), Path::new(ROAMING))
}

/// The bytes of an exe of `version`.
fn exe(version: &str) -> Vec<u8> {
    let mut bytes = b"MZ".to_vec();
    bytes.extend_from_slice(mark_prefix());
    bytes.extend_from_slice(version.as_bytes());
    bytes.push(0x01);
    bytes
}

fn with_ancestors(world: &mut World, dir: &Path) {
    for dir in dir.ancestors() {
        world.dirs.insert(dir.to_path_buf());
    }
}

/// The downloaded exe 0.2.0, the Start menu folder, no installation.
fn fresh() -> World {
    let mut world = World::default();
    with_ancestors(&mut world, Path::new(LOCAL));
    with_ancestors(&mut world, Path::new("/home/ana/Downloads"));
    with_ancestors(&mut world, paths().shortcut.parent().unwrap());
    world.files.insert(PathBuf::from(SOURCE), exe("0.2.0"));
    world
}

/// `fresh`, with 0.1.0 installed, its shortcut and its `Run` value.
fn installed() -> World {
    let paths = paths();
    let mut world = fresh();
    world.dirs.insert(paths.dir.parent().unwrap().to_path_buf());
    world.dirs.insert(paths.dir.clone());
    world.files.insert(paths.exe.clone(), exe("0.1.0"));
    world.files.insert(paths.shortcut.clone(), b"lnk".to_vec());
    world.run = run_command(&paths.exe);
    world
}

fn situation(start: Start, here_is_installed: bool, installed: Option<Installed>) -> Situation {
    Situation {
        start,
        here_is_installed,
        installed,
        own: v("0.2.0"),
    }
}

// Exigence : EX-019-01
#[test]
fn plan_for_every_situation() {
    let known = |text: &str| Some(Installed::Known(v(text)));
    use InstallPlan::{Continue, Delegate, Install, Update};
    use Start::{Autostart, Bridge, InPlace, Interactive};

    assert_eq!(situation(Interactive, false, None).plan(), Install, "no installation");
    assert_eq!(
        situation(Interactive, false, known("0.1.0")).plan(),
        Update { installed: v("0.1.0") },
        "this exe is newer"
    );
    assert_eq!(
        situation(Interactive, false, known("0.2.0-rc.1")).plan(),
        Update {
            installed: v("0.2.0-rc.1")
        },
        "a release after its pre-release"
    );
    assert_eq!(
        situation(Interactive, false, known("0.2.0")).plan(),
        Delegate { autostart: false },
        "identical"
    );
    assert_eq!(
        situation(Interactive, false, known("0.3.0")).plan(),
        Delegate { autostart: false },
        "older"
    );
    assert_eq!(
        situation(Interactive, false, Some(Installed::Unknown)).plan(),
        Delegate { autostart: false },
        "an unreadable version is never replaced"
    );
    assert_eq!(
        situation(Interactive, true, None).plan(),
        Continue,
        "already at its final location"
    );
    assert_eq!(situation(Autostart, true, None).plan(), Continue);

    // `--autostart`: never a dialog.
    assert_eq!(situation(Autostart, false, None).plan(), Continue);
    assert_eq!(
        situation(Autostart, false, known("0.1.0")).plan(),
        Delegate { autostart: true }
    );
    assert_eq!(
        situation(Autostart, false, known("0.2.0")).plan(),
        Delegate { autostart: true }
    );
    assert_eq!(
        situation(Autostart, false, Some(Installed::Unknown)).plan(),
        Delegate { autostart: true }
    );

    // The bridge and a relaunch in place: never a dialog, wherever they run from.
    for start in [Bridge, InPlace] {
        for found in [None, known("0.1.0"), known("0.3.0"), Some(Installed::Unknown)] {
            assert_eq!(
                situation(start, false, found.clone()).plan(),
                Continue,
                "{start:?} {found:?}"
            );
        }
    }
}

// Exigence : EX-019-01
#[test]
fn inspection_finds_the_installation_through_short_names() {
    let paths = paths();
    let ops = FakeSystem::new(installed());
    let own = v("0.2.0");

    let here = Situation::inspect(&ops, &paths, Path::new(SOURCE), Start::Interactive, own.clone());
    assert!(!here.here_is_installed);
    assert_eq!(here.installed, Some(Installed::Known(v("0.1.0"))));
    assert_eq!(here.plan(), InstallPlan::Update { installed: v("0.1.0") });

    // Launched through another spelling of the installed exe (an 8.3 short name).
    let short = PathBuf::from("/home/ANA~1/AppData/Local/Programs/MCPPRO~1/mcp-proxy.exe");
    ops.alias(short.clone(), paths.exe.clone());
    let installed_here = Situation::inspect(&ops, &paths, &short, Start::Interactive, own.clone());
    assert!(installed_here.here_is_installed);
    assert_eq!(installed_here.installed, None);
    assert_eq!(installed_here.plan(), InstallPlan::Continue);

    let none = Situation::inspect(
        &FakeSystem::new(fresh()),
        &paths,
        Path::new(SOURCE),
        Start::Interactive,
        own.clone(),
    );
    assert_eq!(none.installed, None);
    assert_eq!(none.plan(), InstallPlan::Install);

    let bridge = Situation::inspect(&ops, &paths, Path::new(SOURCE), Start::Bridge, own);
    assert_eq!(bridge.plan(), InstallPlan::Continue);
    assert!(ops.log().is_empty(), "inspecting changes nothing");
}

fn installation_ops(paths: &InstallPaths, autostart: bool) -> Vec<Op> {
    let mut ops = vec![
        Op::StopInstance,
        Op::CreateDir(paths.dir.parent().unwrap().to_path_buf()),
        Op::CreateDir(paths.dir.clone()),
        Op::Copy {
            from: PathBuf::from(SOURCE),
            to: paths.staged.clone(),
        },
        Op::Rename {
            from: paths.staged.clone(),
            to: paths.exe.clone(),
        },
        Op::WriteShortcut(paths.shortcut.clone()),
    ];
    if autostart {
        ops.push(Op::SetRun(run_command(&paths.exe).unwrap()));
    }
    ops.push(Op::Launch {
        exe: paths.exe.clone(),
        args: vec![],
    });
    ops
}

// Exigence : EX-019-02
#[test]
fn installation_runs_its_steps_in_order() {
    let paths = paths();
    for autostart in [true, false] {
        let ops = FakeSystem::new(fresh());
        install(&ops, &paths, Path::new(SOURCE), autostart).unwrap();
        assert_eq!(ops.log(), installation_ops(&paths, autostart), "autostart {autostart}");
        let world = ops.world();
        assert_eq!(world.files.get(&paths.exe), Some(&exe("0.2.0")));
        assert!(!world.files.contains_key(&paths.staged), "no temporary copy left");
        assert!(world.files.contains_key(&paths.shortcut));
        assert_eq!(world.run, autostart.then(|| run_command(&paths.exe).unwrap()));
        assert_eq!(ops.launched(), vec![(paths.exe.clone(), vec![])]);
        // The data folder is never touched.
        let data = Path::new(LOCAL).join("MCP Proxy");
        assert!(
            ops.log()
                .iter()
                .all(|op| !format!("{op:?}").contains(&*data.to_string_lossy()))
        );
    }
}

// Exigence : EX-019-02
#[test]
fn a_failed_installation_step_undoes_the_previous_ones() {
    let paths = paths();
    let steps = installation_ops(&paths, true).len();
    for index in 0..steps {
        let ops = FakeSystem::new(fresh());
        ops.fail_at(index);
        let error = install(&ops, &paths, Path::new(SOURCE), true).unwrap_err();
        assert_eq!(ops.world(), fresh(), "failure at step {index}: nothing partial left");
        assert!(ops.launched().is_empty(), "failure at step {index}");
        match (index, &error) {
            (0, InstallError::NotStopped(_)) => assert_eq!(ops.log(), vec![Op::StopInstance]),
            (0, other) => panic!("{other:?}"),
            (_, InstallError::Failed { left, .. }) => assert!(left.is_empty(), "{error}"),
            (_, other) => panic!("{other:?}"),
        }
        assert!(error.to_string().contains("Rien n'a été modifié"), "{error}");
    }
}

// Exigence : EX-019-02
#[test]
fn installation_stops_a_running_instance_first_and_keeps_a_stuck_one_untouched() {
    let paths = paths();
    let mut running = fresh();
    running.instance = Instance::Running;
    let ops = FakeSystem::new(running);
    install(&ops, &paths, Path::new(SOURCE), false).unwrap();
    assert_eq!(ops.log()[0], Op::StopInstance);
    assert_eq!(ops.world().instance, Instance::NotRunning);

    let mut stuck = fresh();
    stuck.instance = Instance::Stuck;
    let ops = FakeSystem::new(stuck.clone());
    let error = install(&ops, &paths, Path::new(SOURCE), false).unwrap_err();
    assert!(matches!(error, InstallError::NotStopped(_)));
    assert!(
        error
            .to_string()
            .starts_with("MCP Proxy, déjà ouvert, n'a pas pu être fermé"),
        "{error}"
    );
    assert_eq!(ops.log(), vec![Op::StopInstance], "nothing attempted after");
    assert_eq!(ops.world(), stuck);
}

// Exigence : EX-019-02
#[test]
fn an_undo_that_fails_is_reported() {
    let paths = paths();
    let ops = FakeSystem::new(fresh());
    let launch = installation_ops(&paths, true).len() - 1;
    ops.fail_at(launch);
    // Undo order: `Run` value, shortcut, exe, temporary copy, folders.
    ops.fail_at(launch + 2);
    let error = install(&ops, &paths, Path::new(SOURCE), true).unwrap_err();
    assert_eq!(
        error,
        InstallError::Failed {
            step: Step::Launch,
            cause: OpError::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            left: vec![Leftover::Shortcut],
        }
    );
    let text = error.to_string();
    assert!(
        text.starts_with("Lancement depuis le dossier d'installation impossible (accès refusé)"),
        "{text}"
    );
    assert!(text.contains("sauf : le raccourci du menu Démarrer"), "{text}");
    let world = ops.world();
    assert!(world.files.contains_key(&paths.shortcut));
    assert!(!world.files.contains_key(&paths.exe) && !world.dirs.contains(&paths.dir));
}

fn update_ops(paths: &InstallPaths) -> Vec<Op> {
    vec![
        Op::StopInstance,
        Op::Copy {
            from: PathBuf::from(SOURCE),
            to: paths.staged.clone(),
        },
        Op::Rename {
            from: paths.exe.clone(),
            to: paths.backup.clone(),
        },
        Op::Rename {
            from: paths.staged.clone(),
            to: paths.exe.clone(),
        },
        Op::Launch {
            exe: paths.exe.clone(),
            args: vec![],
        },
        Op::RemoveFile(paths.backup.clone()),
    ]
}

// Exigence : EX-019-02
#[test]
fn update_stops_replaces_and_relaunches() {
    let paths = paths();
    let mut world = installed();
    world.instance = Instance::Running;
    let ops = FakeSystem::new(world);
    update(&ops, &paths, Path::new(SOURCE)).unwrap();
    assert_eq!(ops.log(), update_ops(&paths));
    let world = ops.world();
    assert_eq!(world.instance, Instance::NotRunning);
    assert_eq!(world.files.get(&paths.exe), Some(&exe("0.2.0")));
    assert!(!world.files.contains_key(&paths.backup) && !world.files.contains_key(&paths.staged));
    assert_eq!(world.run, run_command(&paths.exe), "the `Run` value is left as it was");
    assert_eq!(ops.launched(), vec![(paths.exe.clone(), vec![])]);
}

// Exigence : EX-019-02
#[test]
fn a_failed_update_step_puts_the_installed_exe_back() {
    let paths = paths();
    // Every step but the last: removing the previous exe is best effort.
    let steps = update_ops(&paths).len() - 1;
    for index in 0..steps {
        let ops = FakeSystem::new(installed());
        ops.fail_at(index);
        let error = update(&ops, &paths, Path::new(SOURCE)).unwrap_err();
        assert_eq!(ops.world(), installed(), "failure at step {index}: {error}");
        assert!(ops.launched().is_empty());
    }

    // A bridge still holds the previous exe: it stays aside, the update is done.
    let ops = FakeSystem::new(installed());
    ops.fail_at(steps);
    update(&ops, &paths, Path::new(SOURCE)).unwrap();
    let world = ops.world();
    assert_eq!(world.files.get(&paths.exe), Some(&exe("0.2.0")));
    assert_eq!(world.files.get(&paths.backup), Some(&exe("0.1.0")));

    // The previous exe cannot be put back: the user is told where it is.
    let ops = FakeSystem::new(installed());
    ops.fail_at(4);
    // Undo order: the new exe removed, then the previous one renamed back.
    ops.fail_at(6);
    let error = update(&ops, &paths, Path::new(SOURCE)).unwrap_err();
    assert!(matches!(&error, InstallError::Failed { left, .. } if left == &vec![Leftover::Previous]));
    assert!(error.to_string().contains("mcp-proxy.exe.old"), "{error}");
}

// Exigence : EX-019-02
#[test]
fn leftovers_of_an_interrupted_run_are_cleared_first() {
    let paths = paths();
    let mut world = installed();
    world.files.insert(paths.staged.clone(), b"half".to_vec());
    world.files.insert(paths.backup.clone(), exe("0.0.9"));
    let ops = FakeSystem::new(world);
    update(&ops, &paths, Path::new(SOURCE)).unwrap();
    let world = ops.world();
    assert_eq!(world.files.get(&paths.exe), Some(&exe("0.2.0")));
    assert!(!world.files.contains_key(&paths.staged) && !world.files.contains_key(&paths.backup));
}

// Exigence : EX-019-02
#[test]
fn delegating_and_running_here() {
    let paths = paths();
    let ops = FakeSystem::new(installed());
    delegate(&ops, &paths, false).unwrap();
    delegate(&ops, &paths, true).unwrap();
    run_here(&ops, Path::new(SOURCE)).unwrap();
    assert_eq!(
        ops.launched(),
        vec![
            (paths.exe.clone(), vec![]),
            (paths.exe.clone(), vec!["--autostart".to_owned()]),
            (PathBuf::from(SOURCE), vec!["--no-install".to_owned()]),
        ]
    );
    let none = FakeSystem::new(fresh());
    let error = delegate(&none, &paths, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "MCP Proxy n'a pas pu être lancé (dossier ou fichier introuvable)."
    );
}

// Exigence : EX-019-02
#[test]
fn the_journal_names_steps_never_paths() {
    let paths = paths();
    let (logs, _guard) = CapturedLogs::start();
    let ops = FakeSystem::new(fresh());
    ops.fail_at(3);
    let _ = install(&ops, &paths, Path::new(SOURCE), true);
    let text = logs.text();
    assert!(text.contains("installation : étape en échec, retour arrière"), "{text}");
    assert!(text.contains("copie"), "{text}");
    assert!(!text.contains("ana") && !text.contains("AppData"), "{text}");
}

/// Real files in a temporary folder, the rest in memory: the copy, the renames and the folders as `std::fs` does
/// them.
struct RealFiles {
    run: Mutex<Option<String>>,
    launches: Mutex<Vec<PathBuf>>,
    launch_fails: bool,
}

impl SystemOps for RealFiles {
    fn write_shortcut(&self, shortcut: &Shortcut) -> Result<(), OpError> {
        Ok(std::fs::write(
            &shortcut.path,
            shortcut.target.to_string_lossy().as_bytes(),
        )?)
    }

    fn run_entry(&self) -> Result<Option<String>, OpError> {
        Ok(self.run.lock().unwrap().clone())
    }

    fn set_run_entry(&self, command: &str) -> Result<(), OpError> {
        *self.run.lock().unwrap() = Some(command.to_owned());
        Ok(())
    }

    fn remove_run_entry(&self) -> Result<(), OpError> {
        *self.run.lock().unwrap() = None;
        Ok(())
    }

    fn stop_instance(&self) -> Result<Stopped, OpError> {
        Ok(Stopped::NotRunning)
    }

    fn launch(&self, exe: &Path, _args: &[&str]) -> Result<(), OpError> {
        if self.launch_fails || !exe.is_file() {
            return Err(OpError::Refused("lancement refusé"));
        }
        self.launches.lock().unwrap().push(exe.to_path_buf());
        Ok(())
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-install-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn real(root: &Path) -> (InstallPaths, PathBuf) {
    let local = root.join("Local");
    let roaming = root.join("Roaming");
    let paths = InstallPaths::new(&local, &roaming);
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(paths.shortcut.parent().unwrap()).unwrap();
    let downloads = root.join("Downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let source = downloads.join("mcp-proxy.exe");
    std::fs::write(&source, exe("0.2.0")).unwrap();
    (paths, source)
}

// Exigence : EX-019-02
#[test]
fn installation_on_real_files() {
    let dir = TempDir::new("ok");
    let (paths, source) = real(&dir.0);
    let ops = RealFiles {
        run: Mutex::new(None),
        launches: Mutex::new(Vec::new()),
        launch_fails: false,
    };
    install(&ops, &paths, &source, true).unwrap();
    assert_eq!(std::fs::read(&paths.exe).unwrap(), exe("0.2.0"));
    assert!(!paths.staged.exists());
    assert!(paths.shortcut.is_file());
    assert_eq!(*ops.launches.lock().unwrap(), vec![paths.exe.clone()]);
    assert_eq!(ops.exe_version(&paths.exe), Installed::Known(v("0.2.0")));

    // Then the next launch of the downloaded exe hands over.
    let situation = Situation::inspect(&ops, &paths, &source, Start::Interactive, v("0.2.0"));
    assert_eq!(situation.plan(), InstallPlan::Delegate { autostart: false });
    let installed_here = Situation::inspect(&ops, &paths, &paths.exe, Start::Interactive, v("0.2.0"));
    assert_eq!(installed_here.plan(), InstallPlan::Continue);
}

// Exigence : EX-019-02
#[test]
fn a_failed_installation_on_real_files_leaves_nothing() {
    let dir = TempDir::new("echec");
    let (paths, source) = real(&dir.0);
    let ops = RealFiles {
        run: Mutex::new(None),
        launches: Mutex::new(Vec::new()),
        launch_fails: true,
    };
    let error = install(&ops, &paths, &source, true).unwrap_err();
    assert!(matches!(error, InstallError::Failed { step: Step::Launch, ref left, .. } if left.is_empty()));
    assert!(
        !paths.dir.exists() && !paths.dir.parent().unwrap().exists(),
        "folders removed"
    );
    assert!(!paths.shortcut.exists());
    assert_eq!(*ops.run.lock().unwrap(), None);
    assert!(source.is_file(), "the downloaded exe is untouched");
}

// Exigence : EX-019-12
#[test]
fn the_run_value_names_the_installed_exe_between_quotes() {
    let exe = Path::new(r"C:\Users\Ana Lise\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe");
    let command = run_command(exe).unwrap();
    assert_eq!(
        command,
        r#""C:\Users\Ana Lise\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe" --autostart"#
    );
    assert!(is_enabled(Some(&command), exe));
    assert!(!is_enabled(None, exe));
    assert!(!is_enabled(
        Some(r"C:\Users\Ana Lise\Downloads\mcp-proxy.exe --autostart"),
        exe
    ));
    assert!(
        !is_enabled(Some(&command.replace(" --autostart", "")), exe),
        "without the argument"
    );
    assert_eq!(run_command(Path::new(r#"C:\a"b\mcp-proxy.exe"#)), None);
    assert_eq!(
        (RUN_KEY, RUN_VALUE),
        (r"Software\Microsoft\Windows\CurrentVersion\Run", "MCP Proxy")
    );
}

// Exigence : EX-019-12
#[test]
fn the_icon_menu_changes_the_run_value_from_the_installed_exe_only() {
    let paths = paths();
    let ops = FakeSystem::new(installed());
    assert!(autostart_enabled(&ops, &paths));
    assert_eq!(set_autostart(&ops, &paths, true, false), Ok(false));
    assert_eq!(ops.world().run, None);
    assert!(!autostart_enabled(&ops, &paths));
    assert_eq!(set_autostart(&ops, &paths, true, true), Ok(true));
    assert_eq!(ops.world().run, run_command(&paths.exe));

    // Not installed: never a `Run` value toward a provisional location.
    let ops = FakeSystem::new(fresh());
    assert_eq!(
        set_autostart(&ops, &paths, false, true),
        Err(AutostartError::NotInstalled)
    );
    assert!(ops.log().is_empty());
    assert!(
        AutostartError::NotInstalled
            .to_string()
            .contains("une fois MCP Proxy installé")
    );

    // A foreign value under the same name counts as off, and enabling replaces it.
    let mut world = installed();
    world.run = Some(r#""C:\autre\outil.exe""#.to_owned());
    let ops = FakeSystem::new(world);
    assert!(!autostart_enabled(&ops, &paths));
    assert_eq!(set_autostart(&ops, &paths, true, true), Ok(true));

    // A registry that refuses: the error says so, the state is read again by the caller.
    let ops = FakeSystem::new(installed());
    ops.fail_at(0);
    assert!(matches!(
        set_autostart(&ops, &paths, true, false),
        Err(AutostartError::Registry(_))
    ));
    assert!(autostart_enabled(&ops, &paths));

    // The settings mirror the value.
    assert_eq!(mirrored(false, true), Some(true));
    assert_eq!(mirrored(true, false), Some(false));
    assert_eq!(mirrored(true, true), None);
}

// Exigence : EX-019-02
#[test]
fn installation_without_the_box_removes_a_stale_run_value() {
    let paths = paths();
    let mut world = fresh();
    world.run = run_command(&paths.exe);
    let ops = FakeSystem::new(world);
    install(&ops, &paths, Path::new(SOURCE), false).unwrap();
    assert_eq!(ops.world().run, None);
    assert!(ops.log().contains(&Op::RemoveRun));
}

// Exigence : EX-019-14
#[test]
fn answers_of_the_window() {
    assert_eq!(
        parse_answer(Some(&json!({ "choice": "install", "autostart": true }))),
        Ok(Answer::Install { autostart: true })
    );
    assert_eq!(parse_answer(Some(&json!({ "choice": "update" }))), Ok(Answer::Update));
    assert_eq!(parse_answer(Some(&json!({ "choice": "later" }))), Ok(Answer::Later));
    assert_eq!(
        parse_answer(Some(&json!({ "choice": "continue" }))),
        Ok(Answer::Continue)
    );
    assert_eq!(parse_answer(Some(&json!({ "choice": "quit" }))), Ok(Answer::Quit));
    for bad in [
        json!(null),
        json!("install"),
        json!({ "choice": "install" }),
        json!({ "choice": "install", "autostart": "oui" }),
        json!({ "choice": "install", "autostart": true, "path": "C:\\autre" }),
        json!({ "choice": "update", "source": "x" }),
        json!({ "choice": "uninstall" }),
        json!({}),
    ] {
        assert_eq!(parse_answer(Some(&bad)), Err(MalformedAnswer), "{bad}");
    }
    assert_eq!(parse_answer(None), Err(MalformedAnswer));
}

// Exigence : EX-019-14
#[test]
fn the_window_follows_its_states() {
    let own = v("0.2.0");
    let mut dialog = InstallDialog::new(Proposal::Install { autostart: false }, own.clone());
    assert_eq!(dialog.answer(Answer::Update), None, "not offered");
    assert_eq!(dialog.answer(Answer::Continue), None, "only after a failure");
    assert_eq!(
        dialog.answer(Answer::Install { autostart: true }),
        Some(Action::Install { autostart: true })
    );
    assert!(!dialog.may_close(), "closing would stop the installation halfway");
    assert_eq!(
        dialog.answer(Answer::Install { autostart: true }),
        None,
        "a second click"
    );
    assert_eq!(dialog.answer(Answer::Quit), None, "not while working");
    dialog.failed();
    assert!(dialog.may_close());
    assert_eq!(dialog.answer(Answer::Install { autostart: true }), None);
    assert_eq!(dialog.answer(Answer::Continue), Some(Action::RunHere));
    assert_eq!(dialog.answer(Answer::Quit), None, "finished");

    let mut later = InstallDialog::new(Proposal::Install { autostart: true }, own.clone());
    assert_eq!(later.answer(Answer::Later), Some(Action::RunHere));
    let mut quit = InstallDialog::new(Proposal::Install { autostart: true }, own.clone());
    assert_eq!(quit.answer(Answer::Quit), Some(Action::Quit));

    let mut update = InstallDialog::new(Proposal::Update { installed: v("0.1.0") }, own.clone());
    assert_eq!(update.answer(Answer::Install { autostart: true }), None, "not offered");
    assert_eq!(update.answer(Answer::Update), Some(Action::Update));
    update.failed();
    assert_eq!(update.answer(Answer::Quit), Some(Action::Quit));
    let mut kept = InstallDialog::new(Proposal::Update { installed: v("0.1.0") }, own);
    assert_eq!(
        kept.answer(Answer::Later),
        Some(Action::RunInstalled),
        "the installed version runs"
    );
}

// Exigence : EX-019-14
#[test]
fn the_window_shows_versions_and_no_path() {
    let own = v("0.2.0");
    let install = InstallDialog::new(Proposal::Install { autostart: true }, own.clone());
    assert_eq!(
        install.view(),
        json!({
            "kind": "install",
            "version": "0.2.0",
            "installedVersion": null,
            "autostart": true,
            "location": "%LOCALAPPDATA%\\Programs\\MCP Proxy",
            "command": "answer_install",
        })
    );
    let update = InstallDialog::new(
        Proposal::Update {
            installed: v("0.1.0-rc.1"),
        },
        own,
    );
    assert_eq!(update.view()["kind"], "update");
    assert_eq!(update.view()["installedVersion"], "0.1.0-rc.1");
    assert_eq!((INSTALL_WINDOW, INSTALL_COMMAND), ("install", "answer_install"));
}
