#![allow(clippy::unwrap_used, clippy::panic)]

use std::cell::Cell;
use std::path::{Path, PathBuf};

use mcp_proxy_testkit::CapturedLogs;
use serde_json::{Value, json};

use super::*;
use crate::clients::{self, Environment, Inspection, LocationId, RealFiles, State};
use crate::fake::{FakeSystem, Instance, Op, World};
use crate::install::{run_command, update};
use crate::ops::{OpError, SystemOps};
use crate::update::{ExeReplacer, RenameReplacer, VerifiedExe};
use crate::version::{Version, mark_prefix};

const LOCAL: &str = "/home/ana/AppData/Local";
const ROAMING: &str = "/home/ana/AppData/Roaming";
const STAMP: &str = "20261123-101500";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn exe(version: &str) -> Vec<u8> {
    let mut bytes = b"MZ".to_vec();
    bytes.extend_from_slice(mark_prefix());
    bytes.extend_from_slice(version.as_bytes());
    bytes.push(0x01);
    bytes
}

fn paths() -> UninstallPaths {
    UninstallPaths::new(Path::new(LOCAL), Path::new(ROAMING))
}

fn env() -> Environment {
    Environment {
        app_data: PathBuf::from(ROAMING),
        local_app_data: PathBuf::from(LOCAL),
    }
}

fn dir(world: &mut World, dir: &Path) {
    for dir in dir.ancestors() {
        world.dirs.insert(dir.to_path_buf());
    }
}

fn file(world: &mut World, path: &Path, bytes: &[u8]) {
    dir(world, path.parent().unwrap());
    world.files.insert(path.to_path_buf(), bytes.to_vec());
}

/// MCP Proxy 0.2.0 installed and running, with everything the earlier specs put on the machine.
fn installed() -> World {
    let p = paths();
    let mut world = World::default();
    file(&mut world, &p.install.exe, &exe("0.2.0"));
    file(&mut world, &p.install.shortcut, b"lnk");
    world.run = run_command(&p.install.exe);
    world.uninstall = entry_for(&p.install, Some(&v("0.2.0")), exe("0.2.0").len() as u64);
    world.aumid = true;
    world.wer = Some(1);
    for (path, bytes) in [
        ("settings.json", &b"{}"[..]),
        ("settings.key", b"key"),
        ("notification-icon.png", b"png"),
        ("logs/mcp-proxy-2026-11-23.log", b"log"),
        ("cache/catalog/dev.json", b"[]"),
        ("webview/Default/Cookies", b"c"),
        ("update/0.3.0/mcp-proxy.exe", b"x"),
    ] {
        file(&mut world, &p.data.join(path), bytes);
    }
    file(&mut world, &p.window_data.join("EBWebView/Local State"), b"{}");
    world.instance = Instance::Running;
    world
}

/// What nothing of the uninstallation may touch.
fn untouched(world: &World) -> World {
    let p = paths();
    let mut rest = world.clone();
    let ours = |path: &PathBuf| {
        path.starts_with(&p.install.dir)
            || path.starts_with(&p.data)
            || path.starts_with(&p.window_data)
            || *path == p.install.shortcut
    };
    rest.files.retain(|path, _| !ours(path));
    rest.dirs.retain(|path| !ours(path));
    rest.links.retain(|path| !ours(path));
    rest.run = None;
    rest.uninstall = None;
    rest.aumid = false;
    rest.wer = None;
    rest.instance = Instance::NotRunning;
    rest.deleted_at_exit.clear();
    rest
}

struct Run {
    ops: FakeSystem,
    report: Report,
    pauses: usize,
}

fn uninstall(ops: FakeSystem, plan: &[Item]) -> Run {
    let p = paths();
    let pauses = Cell::new(0);
    let pause = || pauses.set(pauses.get() + 1);
    prepare(&ops, &p.install).unwrap();
    let ctx = Context {
        ops: &ops,
        files: &RealFiles,
        env: &env(),
        paths: &p,
        stamp: STAMP,
        pause: &pause,
    };
    let report = remove(&ctx, plan);
    Run {
        ops,
        report,
        pauses: pauses.get(),
    }
}

fn full_plan() -> Vec<Item> {
    plan(&[], &paths().install.exe, true)
}

fn outcome(report: &Report, item: &Item) -> Outcome {
    report
        .items
        .iter()
        .find(|d| d.item == *item)
        .map(|d| d.outcome.clone())
        .unwrap_or_else(|| panic!("{item:?} not in the report"))
}

fn inspection(location: LocationId, state: State) -> Inspection {
    Inspection {
        location,
        state,
        other_servers: 0,
    }
}

// Exigence : EX-022-01
#[test]
fn the_plan_is_complete_and_in_order() {
    let p = paths();
    let summary = |command: &str| clients::EntrySummary {
        command: Some(command.to_owned()),
        other_keys: vec!["args".to_owned(), "env".to_owned()],
    };
    let installed_spelled_otherwise = p.install.exe.to_string_lossy().to_uppercase();
    let found = [
        inspection(LocationId::ClaudeDesktop, State::Configured),
        inspection(
            LocationId::ClaudeDesktopPackage,
            State::OtherCommand(summary(&installed_spelled_otherwise)),
        ),
        inspection(
            LocationId::VsCode,
            State::OtherCommand(summary("/opt/other/mcp-proxy.exe")),
        ),
    ];
    let tail = [
        Item::Autostart,
        Item::Shortcut,
        Item::Aumid,
        Item::ErrorReports,
        Item::Data,
        Item::UninstallEntry,
        Item::Exe,
        Item::InstallFolder,
        Item::WindowData,
    ];
    let client = |location, decision| Item::Client { location, decision };
    let mut expected = vec![
        client(LocationId::ClaudeDesktop, ClientDecision::Remove),
        client(LocationId::ClaudeDesktopPackage, ClientDecision::Remove),
        client(
            LocationId::VsCode,
            ClientDecision::OtherExe(Some("/opt/other/mcp-proxy.exe".to_owned())),
        ),
    ];
    expected.extend(tail.clone());
    assert_eq!(plan(&found, &p.install.exe, true), expected);

    // The box unticked: our entries are left, the others are still named.
    let unticked = plan(&found, &p.install.exe, false);
    assert_eq!(unticked[0], client(LocationId::ClaudeDesktop, ClientDecision::Asked));
    assert_eq!(
        unticked[1],
        client(LocationId::ClaudeDesktopPackage, ClientDecision::Asked)
    );

    // Nothing in the clients: the rest only.
    let none = [
        inspection(LocationId::ClaudeDesktop, State::NotInstalled),
        inspection(LocationId::VsCode, State::NotConfigured { file_exists: true }),
    ];
    assert_eq!(plan(&none, &p.install.exe, true), tail.to_vec());
    let unreadable = [inspection(
        LocationId::VsCode,
        State::Unreadable(clients::Unreadable::Syntax { line: 3 }),
    )];
    assert_eq!(
        plan(&unreadable, &p.install.exe, false)[0],
        client(
            LocationId::VsCode,
            ClientDecision::Unreadable(clients::Unreadable::Syntax { line: 3 })
        )
    );
}

// Exigence : EX-022-01
#[test]
fn everything_goes_and_the_exe_at_the_end_of_the_process() {
    let p = paths();
    let before = installed();
    let run = uninstall(FakeSystem::new(before.clone()), &full_plan());
    let world = run.ops.world();
    assert!(!run.report.failed(), "{:?}", run.report);
    for item in [
        Item::Autostart,
        Item::Shortcut,
        Item::Aumid,
        Item::ErrorReports,
        Item::Data,
        Item::UninstallEntry,
        Item::InstallFolder,
    ] {
        assert_eq!(outcome(&run.report, &item), Outcome::Removed, "{item:?}");
    }
    assert_eq!(outcome(&run.report, &Item::Exe), Outcome::AtClose);
    assert_eq!(outcome(&run.report, &Item::WindowData), Outcome::AtClose);
    assert_eq!(world.deleted_at_exit, vec![p.install.backup.clone()]);
    assert_eq!(world.run, None);
    assert_eq!(world.uninstall, None);
    assert!(!world.aumid && world.wer.is_none());
    assert!(!world.dirs.contains(&p.data) && !world.dirs.contains(&p.install.dir));
    assert!(!world.files.contains_key(&p.install.shortcut));
    assert_eq!(run.pauses, 0);

    // The window closed: its data go too, and nothing else of ours is left.
    let pause = || {};
    let ctx = Context {
        ops: &run.ops,
        files: &RealFiles,
        env: &env(),
        paths: &p,
        stamp: STAMP,
        pause: &pause,
    };
    let done = remove_window_data(&ctx, &run.report).unwrap();
    assert_eq!(done.outcome, Outcome::Removed);
    assert_eq!(after_close_message(&done), None);
    let world = run.ops.world();
    assert_eq!(world, untouched(&before).with_deleted(&p.install.backup));
    // The data folder of the application keeps its parent: `%LOCALAPPDATA%` is the user's.
    assert!(world.dirs.contains(Path::new(LOCAL)) && world.dirs.contains(&p.programs()));
}

trait Deleted {
    fn with_deleted(self, path: &Path) -> Self;
}

impl Deleted for World {
    fn with_deleted(mut self, path: &Path) -> Self {
        self.deleted_at_exit.push(path.to_path_buf());
        self
    }
}

// Exigence : EX-022-01
#[test]
fn what_is_already_gone_is_fine() {
    let p = paths();
    let mut world = World::default();
    file(&mut world, &p.install.exe, &exe("0.2.0"));
    let run = uninstall(FakeSystem::new(world), &full_plan());
    assert!(!run.report.failed(), "{:?}", run.report);
    for item in [
        Item::Autostart,
        Item::Shortcut,
        Item::Aumid,
        Item::ErrorReports,
        Item::Data,
        Item::UninstallEntry,
    ] {
        assert_eq!(outcome(&run.report, &item), Outcome::Absent, "{item:?}");
    }
    assert_eq!(outcome(&run.report, &Item::InstallFolder), Outcome::Removed);
    let view = report_view(&run.report);
    assert_eq!(view["left"], json!([]));
    assert_eq!(
        view["removed"],
        json!(["Dossier d'installation (%LOCALAPPDATA%\\Programs\\MCP Proxy)"])
    );
}

/// Index in the log of the first operation of each item, after the three of `prepare`.
fn removal_ops(world: World) -> Vec<Op> {
    let run = uninstall(FakeSystem::new(world), &full_plan());
    run.ops.log()
}

// Exigence : EX-022-01
#[test]
fn a_failure_does_not_stop_the_next_items_and_keeps_mcp_proxy_installed() {
    let p = paths();
    let all = removal_ops(installed());
    // 0 to 2: `prepare`. Every later operation fails in turn, up to the entry of « Applications installées ».
    let entry = all.iter().position(|op| *op == Op::RemoveUninstallEntry).unwrap();
    for (index, failing) in all.iter().enumerate().take(entry).skip(3) {
        let ops = FakeSystem::new(installed());
        ops.fail_at(index);
        let pauses = Cell::new(0);
        let pause = || pauses.set(pauses.get() + 1);
        prepare(&ops, &p.install).unwrap();
        let ctx = Context {
            ops: &ops,
            files: &RealFiles,
            env: &env(),
            paths: &p,
            stamp: STAMP,
            pause: &pause,
        };
        let report = remove(&ctx, &full_plan());
        let in_data = matches!(failing, Op::RemoveFile(path) | Op::RemoveDir(path) if path.starts_with(&p.data));
        if in_data {
            // A folder of the data is tried again: one failure is not enough.
            assert_eq!(pauses.get(), 1, "failure at {index}");
            assert_eq!(outcome(&report, &Item::Data), Outcome::Removed, "failure at {index}");
            continue;
        }
        assert!(report.failed(), "failure at {index}: {:?}", *failing);
        let failed: Vec<&Done> = report
            .items
            .iter()
            .filter(|d| matches!(d.outcome, Outcome::Failed { .. }))
            .collect();
        assert_eq!(failed.len(), 1, "failure at {index}: {report:?}");
        let world = ops.world();
        // Every later item was tried: the AUMID and the exclusion went unless they were the failing one.
        assert!(
            !world.aumid || *failing == Op::RemoveAumid,
            "failure at {index}: {:?}",
            *failing
        );
        assert!(world.wer.is_none() || *failing == Op::RemoveWerExclusion);
        // MCP Proxy stays installed: the exe is back, its entry and its folder are there.
        assert_eq!(
            world.files.get(&p.install.exe),
            Some(&exe("0.2.0")),
            "failure at {index}"
        );
        assert!(world.uninstall.is_some() && world.deleted_at_exit.is_empty());
        assert!(report.kept_installed());
        let view = report_view(&report);
        assert_eq!(view["keptInstalled"], true);
        assert_eq!(view["atClose"], json!([]));
        assert!(
            view["left"].as_array().unwrap().iter().any(|l| l["failed"] == true),
            "{view}"
        );
        // Nothing to do once the window closes: the window data stay with the installation.
        assert_eq!(remove_window_data(&ctx, &report), None);
    }
}

// Exigence : EX-022-08
#[test]
fn a_held_folder_is_tried_again_then_reported() {
    let p = paths();
    let log = p.data.join("logs/mcp-proxy-2026-11-23.log");
    let first = removal_ops(installed())
        .iter()
        .position(|op| *op == Op::RemoveFile(log.clone()))
        .unwrap();

    // Held for a moment: the second attempt removes it.
    let ops = FakeSystem::new(installed());
    ops.fail_at(first);
    let run = uninstall(ops, &full_plan());
    assert_eq!(run.pauses, 1);
    assert_eq!(outcome(&run.report, &Item::Data), Outcome::Removed);
    assert!(!run.report.failed());

    // Held for good: every attempt, then the report says what stays and why.
    let ops = FakeSystem::new(installed());
    ops.hold(&log);
    let run = uninstall(ops, &full_plan());
    assert_eq!(run.pauses, DATA_ATTEMPTS as usize - 1);
    assert_eq!(
        outcome(&run.report, &Item::Data),
        Outcome::Failed {
            cause: "fichier utilisé par un autre programme".to_owned(),
            left: vec![r"%LOCALAPPDATA%\MCP Proxy\logs\mcp-proxy-2026-11-23.log".to_owned()],
        }
    );
    let world = run.ops.world();
    assert_eq!(
        world.files.keys().filter(|f| f.starts_with(&p.data)).count(),
        1,
        "the rest went"
    );
    let view = report_view(&run.report);
    let left = view["left"].as_array().unwrap();
    assert_eq!(
        left[0]["what"],
        "Journaux, réglages et autres données (%LOCALAPPDATA%\\MCP Proxy)"
    );
    assert_eq!(
        left[0]["why"],
        "non retiré (fichier utilisé par un autre programme) ; reste : %LOCALAPPDATA%\\MCP \
         Proxy\\logs\\mcp-proxy-2026-11-23.log"
    );
    assert_eq!(left[0]["failed"], true);
    assert_eq!(view["keptInstalled"], true);
}

// Exigence : EX-022-01
#[test]
fn what_is_not_ours_is_left_and_named() {
    let p = paths();
    let mut world = installed();
    world.run = Some("\"C:\\Autre\\outil.exe\" --demarrage".to_owned());
    world.wer = Some(0);
    let mut foreign = world.uninstall.clone().unwrap();
    foreign.uninstall_string = "\"/opt/autre/desinstaller.exe\"".to_owned();
    world.uninstall = Some(foreign.clone());
    file(&mut world, &p.install.dir.join("notes.txt"), b"mine");
    file(&mut world, &p.install.staged, b"half");
    file(
        &mut world,
        &p.install.dir.join(".mcp-proxy.abc.__relocated__.exe"),
        b"old",
    );
    let run = uninstall(FakeSystem::new(world), &full_plan());
    assert!(!run.report.failed(), "{:?}", run.report);
    assert!(!run.report.kept_installed());
    let kept = |item: &Item| match outcome(&run.report, item) {
        Outcome::Kept(why) => why,
        other => panic!("{item:?}: {other:?}"),
    };
    assert!(kept(&Item::Autostart).contains("lance un autre programme"));
    assert!(kept(&Item::ErrorReports).contains("n'est pas celle que pose MCP Proxy"));
    assert!(kept(&Item::UninstallEntry).contains("désigne un autre programme"));
    assert!(
        kept(&Item::InstallFolder).contains("« notes.txt »"),
        "{}",
        kept(&Item::InstallFolder)
    );
    assert_eq!(outcome(&run.report, &Item::Exe), Outcome::AtClose);
    let world = run.ops.world();
    assert_eq!(world.run.as_deref(), Some("\"C:\\Autre\\outil.exe\" --demarrage"));
    assert_eq!(world.wer, Some(0));
    assert_eq!(world.uninstall, Some(foreign));
    // Only the files the installation and the updates put there went; the folder stays with the other one.
    let in_folder: Vec<&PathBuf> = world.files.keys().filter(|f| f.starts_with(&p.install.dir)).collect();
    assert_eq!(in_folder, vec![&p.install.dir.join("notes.txt")]);
    assert!(world.dirs.contains(&p.install.dir));
}

// Exigence : EX-022-08
#[test]
fn an_exe_deleted_where_it_is_leaves_its_folder_to_the_user() {
    let p = paths();
    let mut world = installed();
    world.deletes_in_place = true;
    let run = uninstall(FakeSystem::new(world), &full_plan());
    assert_eq!(outcome(&run.report, &Item::Exe), Outcome::AtClose);
    match outcome(&run.report, &Item::InstallFolder) {
        Outcome::Kept(why) => assert!(why.contains("supprimé à la fermeture de cette fenêtre"), "{why}"),
        other => panic!("{other:?}"),
    }
    let world = run.ops.world();
    assert!(
        world.files.contains_key(&p.install.backup),
        "nothing touched while it runs"
    );
    assert_eq!(world.deleted_at_exit, vec![p.install.backup.clone()]);
}

// Exigence : EX-022-08
#[test]
fn the_data_of_the_window_go_once_it_closed_within_a_bound() {
    let p = paths();
    let run = uninstall(FakeSystem::new(installed()), &full_plan());
    let held = p.window_data.join("EBWebView/Local State");
    run.ops.hold(&held);
    let pauses = Cell::new(0);
    let pause = || pauses.set(pauses.get() + 1);
    let ctx = Context {
        ops: &run.ops,
        files: &RealFiles,
        env: &env(),
        paths: &p,
        stamp: STAMP,
        pause: &pause,
    };
    let done = remove_window_data(&ctx, &run.report).unwrap();
    assert_eq!(pauses.get(), WINDOW_DATA_ATTEMPTS as usize - 1, "bounded");
    assert!(matches!(&done.outcome, Outcome::Failed { left, .. } if left == &vec![
        r"%LOCALAPPDATA%\com.mcpproxy.app\EBWebView\Local State".to_owned()
    ]));
    let message = after_close_message(&done).unwrap();
    assert!(message.contains(r"%LOCALAPPDATA%\com.mcpproxy.app"), "{message}");
    assert!(message.contains("supprimer ce dossier à la main"), "{message}");
}

// Exigence : EX-022-07
#[test]
fn the_exe_is_set_aside_before_the_instance_stops() {
    let p = paths();
    let ops = FakeSystem::new(installed());
    prepare(&ops, &p.install).unwrap();
    assert_eq!(
        ops.log(),
        vec![
            Op::RemoveFile(p.install.backup.clone()),
            Op::Rename {
                from: p.install.exe.clone(),
                to: p.install.backup.clone(),
            },
            Op::StopInstance,
        ]
    );
    let world = ops.world();
    assert_eq!(world.instance, Instance::NotRunning);
    assert!(
        !world.files.contains_key(&p.install.exe),
        "an LLM client finds nothing to start"
    );
    assert_eq!(world.files.get(&p.install.backup), Some(&exe("0.2.0")));
}

// Exigence : EX-022-07
#[test]
fn nothing_changes_when_the_instance_does_not_stop_or_an_old_version_runs() {
    let p = paths();
    let mut stuck = installed();
    stuck.instance = Instance::Stuck;
    let ops = FakeSystem::new(stuck.clone());
    let error = prepare(&ops, &p.install).unwrap_err();
    assert!(matches!(error, UninstallError::NotStopped { restored: true, .. }));
    assert_eq!(ops.world(), stuck, "the exe is back, nothing else was touched");
    let text = error.to_string();
    assert!(
        text.contains("Rien n'a été modifié") && !text.contains("renommez"),
        "{text}"
    );

    // The exe cannot even be put back: the user is told how.
    let ops = FakeSystem::new(stuck.clone());
    ops.fail_at(3);
    let error = prepare(&ops, &p.install).unwrap_err();
    assert!(matches!(error, UninstallError::NotStopped { restored: false, .. }));
    assert!(
        error.to_string().contains("renommez-le en « mcp-proxy.exe »"),
        "{error}"
    );

    // A `.old` of an update still run by the bridge of an older version.
    let mut old = installed();
    file(&mut old, &p.install.backup, &exe("0.1.0"));
    let ops = FakeSystem::new(old.clone());
    ops.hold(&p.install.backup);
    let error = prepare(&ops, &p.install).unwrap_err();
    assert!(matches!(error, UninstallError::OldVersionHeld(_)));
    assert_eq!(ops.world(), old);
    assert_eq!(ops.log(), vec![Op::RemoveFile(p.install.backup.clone())]);
    assert!(error.to_string().contains("Fermez vos clients LLM"), "{error}");

    let ops = FakeSystem::new(installed());
    ops.fail_at(1);
    assert!(matches!(prepare(&ops, &p.install), Err(UninstallError::SetAside(_))));
    assert_eq!(ops.world(), installed());
}

// Exigence : EX-022-06
#[test]
fn only_a_folder_of_ours_may_be_removed() {
    let local = Path::new(LOCAL);
    let ok = local.join("MCP Proxy");
    assert_eq!(check_owned(&ok, local, DATA_FOLDER), Ok(()));
    assert_eq!(
        check_owned(&local.join("com.mcpproxy.app"), local, WINDOW_DATA_FOLDER),
        Ok(())
    );
    let refused = [
        ("", local, Refusal::Empty),
        ("relative/MCP Proxy", local, Refusal::Relative),
        ("MCP Proxy", Path::new(""), Refusal::Empty),
        ("/", local, Refusal::NotOurs),
        ("/MCP Proxy", Path::new("/"), Refusal::NotNormal),
        ("/home/MCP Proxy", Path::new("/home"), Refusal::NotNormal),
        ("/home/ana", local, Refusal::NotOurs),
        ("/home/ana/AppData/Local", local, Refusal::NotOurs),
        ("/home/ana/AppData/Local/MCP Proxy2", local, Refusal::NotOurs),
        ("/home/ana/AppData/Local/mcp proxy", local, Refusal::NotOurs),
        ("/home/ana/AppData/Local/MCP Proxy/..", local, Refusal::NotNormal),
        ("/home/ana/AppData/Local/x/MCP Proxy", local, Refusal::NotOurs),
        ("/tmp/MCP Proxy", local, Refusal::NotOurs),
    ];
    for (path, parent, refusal) in refused {
        assert_eq!(
            check_owned(Path::new(path), parent, DATA_FOLDER),
            Err(refusal),
            "{path}"
        );
    }
    assert_eq!(check_owned(&ok, local, ""), Err(Refusal::NotOurs));
    assert_eq!(check_owned(&ok, local, ".."), Err(Refusal::NotOurs));
    assert_eq!(check_owned(&local.join("a/b"), local, "a/b"), Err(Refusal::NotOurs));

    // `%LOCALAPPDATA%` pointing at a root or a profile folder: nothing is removed, the report says why.
    for local in ["/", "/home"] {
        let p = UninstallPaths::new(Path::new(local), Path::new(ROAMING));
        let mut world = World::default();
        file(&mut world, &p.install.exe, &exe("0.2.0"));
        file(&mut world, &p.data.join("settings.json"), b"{}");
        let ops = FakeSystem::new(world);
        prepare(&ops, &p.install).unwrap();
        let pause = || {};
        let ctx = Context {
            ops: &ops,
            files: &RealFiles,
            env: &env(),
            paths: &p,
            stamp: STAMP,
            pause: &pause,
        };
        let report = remove(&ctx, &plan(&[], &p.install.exe, true));
        for item in [Item::Data, Item::InstallFolder] {
            match outcome(&report, &item) {
                Outcome::Kept(why) => assert!(why.contains("rien n'a été supprimé"), "{local} {item:?}: {why}"),
                other => panic!("{local} {item:?}: {other:?}"),
            }
        }
        assert!(ops.world().files.contains_key(&p.data.join("settings.json")));
        assert!(
            !ops.log()
                .iter()
                .any(|op| matches!(op, Op::RemoveDir(_) | Op::RemoveDirAll(_))),
            "{local}"
        );
    }
}

// Exigence : EX-022-06
#[test]
fn a_link_is_removed_itself_never_followed() {
    let p = paths();
    let mut world = installed();
    let link = p.data.join("webview");
    // The fake knows a link only as a link: it is removed as such, nothing under it is listed.
    world.files.retain(|f, _| !f.starts_with(&link));
    world.dirs.retain(|d| !d.starts_with(&link));
    world.links.insert(link.clone());
    let run = uninstall(FakeSystem::new(world), &full_plan());
    assert_eq!(outcome(&run.report, &Item::Data), Outcome::Removed);
    assert!(run.ops.log().contains(&Op::RemoveLink(link)));

    // The data folder itself is a link: left as it is.
    let mut world = installed();
    world.files.retain(|f, _| !f.starts_with(&p.data));
    world.dirs.retain(|d| !d.starts_with(&p.data));
    world.links.insert(p.data.clone());
    let run = uninstall(FakeSystem::new(world), &full_plan());
    assert!(matches!(outcome(&run.report, &Item::Data), Outcome::Kept(why) if why.contains("lien")));
    assert!(run.ops.world().links.contains(&p.data));
}

/// Real files: the default bodies of the trait; the registry in memory, empty.
struct Disk;

impl SystemOps for Disk {
    fn write_shortcut(&self, _: &crate::ops::Shortcut) -> Result<(), OpError> {
        Err(OpError::Refused("hors sujet"))
    }
    fn run_entry(&self) -> Result<Option<String>, OpError> {
        Ok(None)
    }
    fn set_run_entry(&self, _: &str) -> Result<(), OpError> {
        Ok(())
    }
    fn remove_run_entry(&self) -> Result<(), OpError> {
        Ok(())
    }
    fn uninstall_entry(&self) -> Result<Option<UninstallEntry>, OpError> {
        Ok(None)
    }
    fn set_uninstall_entry(&self, _: &UninstallEntry) -> Result<(), OpError> {
        Ok(())
    }
    fn remove_uninstall_entry(&self) -> Result<crate::ops::Removal, OpError> {
        Ok(crate::ops::Removal::Absent)
    }
    fn remove_aumid(&self) -> Result<crate::ops::Removal, OpError> {
        Ok(crate::ops::Removal::Absent)
    }
    fn remove_wer_exclusion(&self) -> Result<crate::ops::Removal, OpError> {
        Ok(crate::ops::Removal::Absent)
    }
    /// Moved out of its folder, as `self_replace` does when it can.
    fn delete_running_exe(&self, exe: &Path) -> Result<(), OpError> {
        Ok(std::fs::remove_file(exe)?)
    }
    fn stop_instance(&self) -> Result<crate::ops::Stopped, OpError> {
        Ok(crate::ops::Stopped::NotRunning)
    }
    fn launch(&self, _: &Path, _: &[&str]) -> Result<(), OpError> {
        Err(OpError::Refused("hors sujet"))
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-uninstall-{}-{name}", std::process::id()));
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

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn on_disk(root: &Path) -> (UninstallPaths, Environment) {
    let local = root.join("Local");
    let roaming = root.join("Roaming");
    let p = UninstallPaths::new(&local, &roaming);
    write(&p.install.exe, &exe("0.2.0"));
    write(&p.data.join("settings.json"), b"{}");
    write(&p.data.join("logs/mcp-proxy-2026-11-23.log"), b"log");
    write(&p.window_data.join("EBWebView/Local State"), b"{}");
    (
        p,
        Environment {
            app_data: roaming,
            local_app_data: local,
        },
    )
}

// Exigence : EX-022-06
#[cfg(unix)]
#[test]
fn on_real_folders_no_link_is_followed_out_of_ours() {
    use std::os::unix::fs::symlink;
    let root = TempDir::new("links");
    let (p, env) = on_disk(&root.0);
    // Outside of everything of ours: a folder and a file a link inside the data points at.
    let outside = root.0.join("Documents");
    write(&outside.join("rapport.txt"), b"precious");
    write(&root.0.join("precious.txt"), b"precious");
    symlink(&outside, p.data.join("webview")).unwrap();
    symlink(root.0.join("precious.txt"), p.data.join("cache.lnk")).unwrap();
    symlink(&outside, p.window_data.join("EBWebView/linked")).unwrap();
    prepare(&Disk, &p.install).unwrap();
    let pause = || {};
    let ctx = Context {
        ops: &Disk,
        files: &RealFiles,
        env: &env,
        paths: &p,
        stamp: STAMP,
        pause: &pause,
    };
    let report = remove(&ctx, &plan(&[], &p.install.exe, true));
    assert!(!report.failed(), "{report:?}");
    assert_eq!(outcome(&report, &Item::Data), Outcome::Removed);
    assert_eq!(remove_window_data(&ctx, &report).unwrap().outcome, Outcome::Removed);
    assert!(!p.data.exists() && !p.window_data.exists() && !p.install.dir.exists());
    assert_eq!(std::fs::read(outside.join("rapport.txt")).unwrap(), b"precious");
    assert_eq!(std::fs::read(root.0.join("precious.txt")).unwrap(), b"precious");
    assert!(
        p.programs().is_dir() && p.local_app_data.is_dir(),
        "their parents are the user's"
    );

    // The data folder is itself a link: neither it nor its target is touched.
    let root = TempDir::new("linked-data");
    let (p, env) = on_disk(&root.0);
    let elsewhere = root.0.join("D/MCP Proxy");
    std::fs::remove_dir_all(&p.data).unwrap();
    write(&elsewhere.join("settings.json"), b"{}");
    symlink(&elsewhere, &p.data).unwrap();
    prepare(&Disk, &p.install).unwrap();
    let ctx = Context {
        ops: &Disk,
        files: &RealFiles,
        env: &env,
        paths: &p,
        stamp: STAMP,
        pause: &pause,
    };
    let report = remove(&ctx, &plan(&[], &p.install.exe, true));
    assert!(matches!(outcome(&report, &Item::Data), Outcome::Kept(_)));
    assert!(p.data.is_symlink() && elsewhere.join("settings.json").is_file());
}

const OTHER_SERVER: &str = r#""github": { "command": "gh", "env": { "TOKEN": "sentinel-token" } }"#;

fn claude_config(exe: &Path) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    {OTHER_SERVER},\n    \"mcp-proxy\": {{ \"command\": \"{}\", \"args\": [\"stdio\"] }}\n  }}\n}}\n",
        exe.display()
    )
}

const VSCODE_OTHER: &str = "{\n  \"servers\": {\n    \"mcp-proxy\": { \"type\": \"stdio\", \"command\": \"/opt/dev/mcp-proxy.exe\", \"args\": [\"stdio\"] }\n  }\n}\n";

// Exigence : EX-022-09
#[test]
fn the_entries_of_the_clients_go_with_a_backup_when_the_box_is_ticked() {
    for ticked in [true, false] {
        let root = TempDir::new(&format!("clients-{ticked}"));
        let (p, env) = on_disk(&root.0);
        let claude = env.location(LocationId::ClaudeDesktop).file;
        let vscode = env.location(LocationId::VsCode).file;
        write(&claude, claude_config(&p.install.exe).as_bytes());
        write(&vscode, VSCODE_OTHER.as_bytes());
        let package = env.location(LocationId::ClaudeDesktopPackage).file;
        write(&package, b"{ \"mcpServers\": { ");
        let found = clients::inspect_all(&RealFiles, &env, Some(&p.install.exe));
        let the_plan = plan(&found, &p.install.exe, ticked);
        prepare(&Disk, &p.install).unwrap();
        let pause = || {};
        let ctx = Context {
            ops: &Disk,
            files: &RealFiles,
            env: &env,
            paths: &p,
            stamp: STAMP,
            pause: &pause,
        };
        let report = remove(&ctx, &the_plan);
        let claude_item = Item::Client {
            location: LocationId::ClaudeDesktop,
            decision: if ticked {
                ClientDecision::Remove
            } else {
                ClientDecision::Asked
            },
        };
        let text = std::fs::read_to_string(&claude).unwrap();
        let backup = claude.with_file_name(format!("claude_desktop_config.json.bak-{STAMP}"));
        if ticked {
            assert_eq!(outcome(&report, &claude_item), Outcome::Removed);
            assert!(!text.contains("mcp-proxy") && text.contains(OTHER_SERVER), "{text}");
            assert_eq!(std::fs::read_to_string(&backup).unwrap(), claude_config(&p.install.exe));
        } else {
            assert!(matches!(outcome(&report, &claude_item), Outcome::Kept(why) if why == "laissée, comme demandé"));
            assert_eq!(text, claude_config(&p.install.exe));
            assert!(!backup.exists());
        }
        // Another exe, an unreadable file: never written, named in the report.
        assert_eq!(std::fs::read_to_string(&vscode).unwrap(), VSCODE_OTHER);
        assert_eq!(std::fs::read(&package).unwrap(), b"{ \"mcpServers\": { ");
        let view = report_view(&report);
        let left: Vec<String> = view["left"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| format!("{} : {}", l["what"].as_str().unwrap(), l["why"].as_str().unwrap()))
            .collect();
        assert!(
            left.iter().any(|l| l.starts_with(
                "Entrée « mcp-proxy » de VS Code : elle lance un autre programme (/opt/dev/mcp-proxy.exe)"
            )),
            "{left:?}"
        );
        assert!(
            left.iter()
                .any(|l| l.starts_with("Entrée « mcp-proxy » de Claude Desktop (paquet MSIX) : fichier illisible")),
            "{left:?}"
        );
        assert!(!view.to_string().contains("sentinel-token"));
        assert!(!report.failed());
    }
}

// Exigence : EX-022-10
#[test]
fn the_window_answers_strictly_one_action_at_a_time() {
    let ok = |value: Value| parse_answer(Some(&value));
    assert_eq!(
        ok(json!({ "choice": "uninstall", "removeClients": true })),
        Ok(Answer::Uninstall { remove_clients: true })
    );
    assert_eq!(
        ok(json!({ "choice": "uninstall", "removeClients": false })),
        Ok(Answer::Uninstall { remove_clients: false })
    );
    assert_eq!(ok(json!({ "choice": "cancel" })), Ok(Answer::Cancel));
    assert_eq!(ok(json!({ "choice": "close" })), Ok(Answer::Close));
    for bad in [
        json!({ "choice": "uninstall" }),
        json!({ "choice": "uninstall", "removeClients": "oui" }),
        json!({ "choice": "uninstall", "removeClients": true, "path": "/" }),
        json!({ "choice": "cancel", "x": 1 }),
        json!({ "choice": "Uninstall", "removeClients": true }),
        json!({ "choice": "install", "autostart": true }),
        json!("uninstall"),
        json!(null),
    ] {
        assert_eq!(ok(bad.clone()), Err(MalformedAnswer), "{bad}");
    }
    assert_eq!(parse_answer(None), Err(MalformedAnswer));

    let mut dialog = UninstallDialog::new(v("0.2.0"), &full_plan());
    assert!(dialog.may_close());
    assert_eq!(dialog.close_answer(), Answer::Cancel);
    assert_eq!(dialog.answer(Answer::Close), None, "nothing to close yet");
    let uninstall = Answer::Uninstall { remove_clients: true };
    assert_eq!(
        dialog.answer(uninstall),
        Some(Action::Uninstall { remove_clients: true })
    );
    assert!(!dialog.may_close(), "closing is refused while it runs");
    assert_eq!(dialog.answer(uninstall), None, "one action at a time");
    assert_eq!(dialog.answer(Answer::Cancel), None);
    dialog.failed();
    assert!(dialog.may_close() && !dialog.removal_ran());
    assert_eq!(
        dialog.answer(Answer::Uninstall { remove_clients: false }),
        Some(Action::Uninstall { remove_clients: false }),
        "tried again after a failure that changed nothing"
    );
    dialog.reported();
    assert!(dialog.removal_ran() && dialog.may_close());
    assert_eq!(dialog.close_answer(), Answer::Close);
    assert_eq!(dialog.answer(uninstall), None, "never twice");
    assert_eq!(dialog.answer(Answer::Close), Some(Action::Quit));
    assert_eq!(dialog.answer(Answer::Close), None);

    let mut cancelled = UninstallDialog::new(v("0.2.0"), &full_plan());
    assert_eq!(cancelled.answer(Answer::Cancel), Some(Action::Quit));
    assert_eq!(cancelled.answer(uninstall), None);
}

// Exigence : EX-022-10
#[test]
fn only_the_installed_exe_uninstalls() {
    let p = paths();
    let ops = FakeSystem::new(installed());
    assert_eq!(launch(&ops, &p.install, &p.install.exe), Launch::Proceed);
    assert_eq!(Launch::Proceed.refusal(), None);
    let short = PathBuf::from("/home/ANA~1/AppData/Local/Programs/MCPPRO~1/mcp-proxy.exe");
    ops.alias(short.clone(), p.install.exe.clone());
    assert_eq!(launch(&ops, &p.install, &short), Launch::Proceed);
    let mut world = installed();
    file(
        &mut world,
        Path::new("/home/ana/Downloads/mcp-proxy.exe"),
        &exe("0.2.0"),
    );
    let ops = FakeSystem::new(world);
    let copy = Path::new("/home/ana/Downloads/mcp-proxy.exe");
    assert_eq!(launch(&ops, &p.install, copy), Launch::Elsewhere);
    assert!(Launch::Elsewhere.refusal().unwrap().contains("Applications installées"));
    let mut none = World::default();
    file(&mut none, copy, &exe("0.2.0"));
    let ops = FakeSystem::new(none);
    assert_eq!(launch(&ops, &p.install, copy), Launch::NotInstalled);
    assert!(Launch::NotInstalled.refusal().unwrap().contains("rien à désinstaller"));
    assert!(ops.log().is_empty(), "deciding changes nothing");
}

// Exigence : EX-022-10
#[test]
fn the_journal_and_the_page_name_no_path_of_the_user() {
    let p = paths();
    let (logs, _guard) = CapturedLogs::start();
    let ops = FakeSystem::new(installed());
    ops.hold(&p.data.join("settings.key"));
    let run = uninstall(ops, &full_plan());
    assert!(run.report.failed());
    let text = logs.text();
    assert!(text.contains("désinstallation : élément traité"), "{text}");
    assert!(text.contains("données") && text.contains("échec"), "{text}");
    assert!(!text.contains("ana") && !text.contains("AppData"), "{text}");

    let dialog = UninstallDialog::new(v("0.2.0"), &full_plan());
    let view = dialog.view().to_string();
    assert!(!view.contains("ana") && !view.contains("AppData"), "{view}");
    assert_eq!(dialog.view()["command"], "answer_uninstall");
    let report = report_view(&run.report).to_string();
    assert!(!report.contains("/home"), "{report}");
}

// Exigence : EX-022-02
#[test]
fn the_entry_says_how_to_uninstall_and_which_version_is_there() {
    let p = paths().install;
    let entry = entry_for(&p, Some(&v("1.4.0-rc.1")), 20 * 1024 * 1024 + 1).unwrap();
    assert_eq!(entry.display_name, "MCP Proxy");
    assert_eq!(entry.display_version.as_deref(), Some("1.4.0-rc.1"));
    assert_eq!(entry.publisher, "MCP Proxy");
    assert_eq!(entry.install_location, p.dir.to_str().unwrap());
    assert_eq!(entry.display_icon, p.exe.to_str().unwrap());
    assert_eq!(entry.uninstall_string, format!("\"{}\" --uninstall", p.exe.display()));
    assert!(entry.no_modify && entry.no_repair);
    assert_eq!(entry.estimated_size, 20 * 1024 + 1);
    assert_eq!(
        (
            estimated_size(0),
            estimated_size(1),
            estimated_size(1024),
            estimated_size(1025)
        ),
        (0, 1, 1, 2)
    );
    assert_eq!(estimated_size(u64::MAX), u32::MAX);
    assert_eq!(command_exe(&entry.uninstall_string), p.exe.to_str().unwrap());
    assert_eq!(command_exe("C:\\a\\b.exe --x"), "C:\\a\\b.exe");
    assert_eq!(uninstall_command(Path::new("C:\\a\"b.exe")), None);
    assert_eq!(
        UNINSTALL_KEY,
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy"
    );
}

// Exigence : EX-022-02
#[test]
fn the_entry_follows_every_update() {
    let p = paths().install;
    let source = Path::new("/home/ana/Downloads/mcp-proxy.exe");

    // Installation window, update (spec 019): the version of the exe put in place.
    let mut world = World::default();
    file(&mut world, &p.exe, &exe("0.1.0"));
    file(&mut world, &p.shortcut, b"lnk");
    file(&mut world, source, &exe("0.2.0"));
    world.uninstall = entry_for(&p, Some(&v("0.1.0")), 1);
    let ops = FakeSystem::new(world);
    update(&ops, &p, source).unwrap();
    let entry = ops.world().uninstall.unwrap();
    assert_eq!(entry.display_version.as_deref(), Some("0.2.0"));
    assert_eq!(entry.estimated_size, estimated_size(exe("0.2.0").len() as u64));

    // Updater (spec 020): the verified version, once in place.
    use sha2::Digest as _;
    let downloaded = PathBuf::from(LOCAL).join("MCP Proxy/update/0.3.0/mcp-proxy.exe");
    let mut world = ops.world();
    file(&mut world, &downloaded, &exe("0.3.0"));
    let ops = std::sync::Arc::new(FakeSystem::new(world));
    let replacer = RenameReplacer::new(ops.clone(), p.clone());
    let verified = VerifiedExe {
        path: downloaded,
        version: v("0.3.0"),
        sha256: sha2::Sha256::digest(exe("0.3.0")).into(),
    };
    replacer.replace(&verified).unwrap();
    assert_eq!(ops.world().uninstall.unwrap().display_version.as_deref(), Some("0.3.0"));

    // At each start of the installed exe: written once, then nothing to do.
    let mut world = World::default();
    file(&mut world, &p.exe, &exe("0.3.0"));
    let ops = FakeSystem::new(world);
    let first = record_entry(&ops, &p, Some(&v("0.3.0"))).unwrap();
    assert!(first.changed && first.previous.is_none());
    let again = record_entry(&ops, &p, Some(&v("0.3.0"))).unwrap();
    assert!(!again.changed);
    assert_eq!(ops.log().len(), 1, "an entry already right is not written again");
}

// Exigence : EX-022-12
#[test]
fn the_window_reads_what_the_application_writes() {
    let p = paths();
    let summary = clients::EntrySummary {
        command: Some(r"C:\Outils\mcp-proxy-dev.exe".to_owned()),
        other_keys: vec!["args".to_owned()],
    };
    let found = [
        inspection(LocationId::ClaudeDesktop, State::Configured),
        inspection(
            LocationId::ClaudeDesktopPackage,
            State::Unreadable(clients::Unreadable::Syntax { line: 12 }),
        ),
        inspection(LocationId::VsCode, State::OtherCommand(summary)),
    ];
    let the_plan = plan(&found, &p.install.exe, true);
    let dialog = UninstallDialog::new(v("0.2.0"), &the_plan);

    let mut complete = uninstall(FakeSystem::new(installed()), &full_plan());
    complete.report.items.insert(
        0,
        Done {
            item: Item::Client {
                location: LocationId::ClaudeDesktop,
                decision: ClientDecision::Remove,
            },
            outcome: Outcome::Removed,
        },
    );
    let mut world = installed();
    file(&mut world, &p.install.dir.join("notes.txt"), b"mine");
    world.wer = Some(0);
    let ops = FakeSystem::new(world);
    ops.hold(&p.data.join("logs/mcp-proxy-2026-11-23.log"));
    let mut partial = uninstall(ops, &full_plan());
    // The clients of the window, as the report names them.
    let clients: Vec<Done> = the_plan
        .iter()
        .filter(|i| matches!(i, Item::Client { .. }))
        .map(|item| Done {
            item: item.clone(),
            outcome: match item {
                Item::Client { decision: ClientDecision::Remove, .. } => Outcome::Removed,
                Item::Client { decision: ClientDecision::OtherExe(_), .. } => {
                    Outcome::Kept(r"elle lance un autre programme (C:\Outils\mcp-proxy-dev.exe) : laissée".to_owned())
                }
                _ => Outcome::Kept(
                    "fichier illisible (JSON invalide à la ligne 12) : regardez à la main s'il contient une entrée « mcp-proxy »"
                        .to_owned(),
                ),
            },
        })
        .collect();
    partial.report.items.splice(0..0, clients);
    let error = UninstallError::NotStopped {
        cause: OpError::Refused("toujours ouvert après 20 s"),
        restored: true,
    };
    let fixture = json!({
        "data": dialog.view(),
        "complete": report_view(&complete.report),
        "withLeftovers": report_view(&partial.report),
        "notStopped": error.to_string(),
    });
    let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/uninstall/uninstall.example.json");
    if std::env::var_os("MCP_PROXY_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    } else {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == text,
            "{} is not what the window receives any more: run `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p \
             mcp-proxy-deploy uninstall`, then align ui/src/uninstall/ until `npm test` passes in ui/",
            path.display()
        );
    }
    assert_eq!(fixture["complete"]["keptInstalled"], false);
    assert_eq!(fixture["withLeftovers"]["keptInstalled"], true);
}

// Exigence : EX-022-06
#[test]
fn the_folders_are_those_of_the_earlier_specs() {
    let p = paths();
    let local = Path::new(LOCAL);
    assert_eq!(p.install.dir, local.join(PROGRAMS_FOLDER).join(INSTALL_FOLDER));
    assert_eq!(p.data, local.join(DATA_FOLDER));
    // Tauri puts the WebView2 data of the windows of the bundle under its identifier.
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/tauri.conf.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["identifier"], WINDOW_DATA_FOLDER);
    assert_eq!(AUMID_KEY, r"Software\Classes\AppUserModelId\McpProxy.Desktop");
}

// Exigence : EX-022-08
#[test]
fn an_exe_that_cannot_be_scheduled_for_deletion_is_named_with_what_to_do() {
    let p = paths();
    let index = removal_ops(installed())
        .iter()
        .position(|op| matches!(op, Op::DeleteRunningExe(_)))
        .unwrap();
    let ops = FakeSystem::new(installed());
    ops.fail_at(index);
    let run = uninstall(ops, &full_plan());
    match outcome(&run.report, &Item::Exe) {
        Outcome::Failed { left, .. } => assert_eq!(
            left,
            vec![
                r"%LOCALAPPDATA%\Programs\MCP Proxy\mcp-proxy.exe.old et son dossier, à supprimer une fois cette fenêtre fermée"
                    .to_owned()
            ]
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(outcome(&run.report, &Item::UninstallEntry), Outcome::Removed);
    assert_eq!(outcome(&run.report, &Item::InstallFolder), Outcome::KeptToRetry);
    let world = run.ops.world();
    assert!(world.files.contains_key(&p.install.backup) && world.deleted_at_exit.is_empty());
    let view = report_view(&run.report);
    assert_eq!(view["atClose"], json!([]));
    assert_eq!(view["left"][0]["failed"], true);
}
