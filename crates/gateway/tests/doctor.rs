//! The self-diagnostic (spec 024) assembled: `mcp-proxy.exe doctor` over a data folder, and « Auto-diagnostic »
//! over the running gateway, with fake probes of the platform (registry, toasts, pipe, network) and the real
//! settings, seal, filter, journal and export. The view the window receives must be `ui/src/api/doctor.example.json`.
//!
//! After a deliberate change of the view: `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p mcp-proxy-gateway --test doctor`,
//! then `npm test` in `ui/`.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use mcp_proxy_auth::UnavailableAuthUi;
use mcp_proxy_core::config::{Config, EMBEDDED_TOML, ProfileId, SessionId};
use mcp_proxy_core::secrets::{Credential, SecretStore, platform_key_wrapper};
use mcp_proxy_core::trace::{EntryKind, JournalFilter, ManualWallClock, WallClock};
use mcp_proxy_deploy::clients::{ClientFiles, Environment, Kind, LocationId};
use mcp_proxy_deploy::doctor::{
    AnonymousWeb, Answer, InstanceProbe, PipeProbe, RealReportFiles, ToastProbe, Toasts, Web, WebError,
};
use mcp_proxy_deploy::fake::{FakeSystem, World};
use mcp_proxy_deploy::install::{InstallPaths, run_command};
use mcp_proxy_deploy::uninstall::entry_for;
use mcp_proxy_deploy::version::own_version;
use mcp_proxy_gateway::bridge::{PeerProof, ProtectFailure, request_ping};
use mcp_proxy_gateway::control::GatewayControl;
use mcp_proxy_gateway::doctor::{DoctorProbes, Save, mcp_targets, run_command_with};
use mcp_proxy_gateway::modules;
use mcp_proxy_gateway::notify::UnavailablePresenter;
use mcp_proxy_gateway::registry::Exposure;
use mcp_proxy_gateway::runtime::{DataDirs, Gateway, Logging, Persistence, RuntimeDeps};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;
use zeroize::Zeroizing;

/// Mon, 23 Nov 2026 10:00:00 UTC.
const NOW_SECS: u64 = 1_795_428_000;
const NOW_HTTP: &str = "Mon, 23 Nov 2026 10:00:00 GMT";
const SENTINEL: &str = "SENTINELLE-doctor-5c1e9a7f3b2d8e40";
const JWT: &str =
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";

// ------------------------------------------------------------------------------------------------------ fakes

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-doctor-gw-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Answers every address with `default`, unless `answers` says otherwise; records what was asked.
struct FakeWeb {
    default: Result<Answer, WebError>,
    answers: BTreeMap<String, Result<Answer, WebError>>,
    asked: Mutex<Vec<String>>,
}

fn answer(status: u16, date: Option<&str>, body: &str) -> Result<Answer, WebError> {
    Ok(Answer {
        status,
        date: date.map(str::to_owned),
        body: body.as_bytes().to_vec(),
    })
}

impl FakeWeb {
    fn new(default: Result<Answer, WebError>) -> Self {
        Self {
            default,
            answers: BTreeMap::new(),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn with(mut self, url: &str, answer: Result<Answer, WebError>) -> Self {
        self.answers.insert(url.to_owned(), answer);
        self
    }
}

#[async_trait]
impl Web for FakeWeb {
    async fn resolve(&self, _host: &str, _port: u16) -> Result<(), WebError> {
        match &self.default {
            Err(WebError::Dns) => Err(WebError::Dns),
            _ => Ok(()),
        }
    }

    async fn get(&self, url: &Url, _max_body: u64) -> Result<Answer, WebError> {
        self.asked.lock().unwrap().push(url.to_string());
        self.answers.get(url.as_str()).unwrap_or(&self.default).clone()
    }
}

struct FixedToasts(Toasts);

impl ToastProbe for FixedToasts {
    fn toasts(&self) -> Toasts {
        self.0
    }
}

struct FixedInstance(PipeProbe);

#[async_trait]
impl InstanceProbe for FixedInstance {
    async fn probe(&self) -> PipeProbe {
        self.0.clone()
    }
}

/// The files of the LLM clients in memory; every write is counted, and refused.
#[derive(Default)]
struct MemFiles {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: BTreeSet<PathBuf>,
    writes: AtomicUsize,
}

impl MemFiles {
    fn file(&mut self, path: PathBuf, text: &str) {
        if let Some(dir) = path.parent() {
            self.dirs.insert(dir.to_path_buf());
        }
        self.files.insert(path, text.as_bytes().to_vec());
    }

    fn refuse(&self) -> io::Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
}

impl ClientFiles for MemFiles {
    fn kind(&self, path: &Path) -> io::Result<Kind> {
        Ok(if self.files.contains_key(path) {
            Kind::File
        } else if self.dirs.contains(path) {
            Kind::Dir
        } else {
            Kind::Missing
        })
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn create_new(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
        self.refuse()
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        self.refuse()
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        self.refuse()
    }
}

struct FakeUser(u8);

impl PeerProof for FakeUser {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure> {
        Ok([&[b'U', self.0][..], data].concat())
    }

    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure> {
        match data {
            [b'U', user, rest @ ..] if *user == self.0 => Ok(Zeroizing::new(rest.to_vec())),
            _ => Err(ProtectFailure(-1)),
        }
    }
}

// ----------------------------------------------------------------------------------------------- assembling

/// The example configuration with the hosts of a company instead of the reserved example names.
fn corporate_config() -> Config {
    let toml = EMBEDDED_TOML.replace("example.internal", "corp.internal");
    Config::from_toml(&toml, &modules::builtin().ids()).unwrap()
}

fn clock() -> ManualWallClock {
    ManualWallClock::new(UNIX_EPOCH + Duration::from_secs(NOW_SECS))
}

async fn start(dir: &Path, config: Config, store: SecretStore) -> Gateway {
    let dirs = DataDirs::under(dir);
    Gateway::start(RuntimeDeps {
        config: Arc::new(config),
        persistence: Persistence::sealed(&dirs, platform_key_wrapper()),
        dirs,
        store,
        auth_ui: Arc::new(UnavailableAuthUi),
        presenter: Arc::new(UnavailablePresenter),
        clock: Arc::new(clock()),
        logging: Logging::Local,
        utc_offset: 0,
        system: "système de test".to_owned(),
    })
    .await
    .unwrap()
}

async fn choose_developer(gateway: &Gateway) {
    gateway
        .set_profile(&ProfileId::new("developpeur").unwrap())
        .await
        .unwrap();
}

/// The folders of a Windows user, as written in the fixture.
fn windows_folders() -> (PathBuf, PathBuf) {
    (
        PathBuf::from(r"C:\Users\exemple\AppData\Roaming"),
        PathBuf::from(r"C:\Users\exemple\AppData\Local"),
    )
}

/// An installation as the installed exe of this version leaves it.
fn installed_world(paths: &InstallPaths) -> World {
    let exe = format!("MZ\u{1}mcp-proxy:version={}\u{1}", own_version()).into_bytes();
    let mut world = World::default();
    world.dirs.insert(paths.dir.clone());
    world.uninstall = entry_for(paths, Some(&own_version()), exe.len() as u64);
    world.files.insert(paths.exe.clone(), exe);
    world.files.insert(paths.shortcut.clone(), b"lnk".to_vec());
    world.aumid = true;
    world.wer = Some(1);
    world.run = run_command(&paths.exe);
    world
}

fn ours(exe: &Path) -> String {
    format!(
        "{{\"mcpServers\": {{\"mcp-proxy\": {{\"command\": {}, \"args\": [\"stdio\"]}}, \"github\": {{\"command\": \
         \"npx\", \"env\": {{\"TOKEN\": \"{SENTINEL}\"}}}}}}}}",
        Value::String(exe.display().to_string())
    )
}

fn release_key() -> [u8; 32] {
    SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()
}

struct Probes {
    ops: Arc<FakeSystem>,
    files: Arc<MemFiles>,
    web: Arc<FakeWeb>,
    probes: DoctorProbes,
}

#[allow(clippy::too_many_arguments)]
fn probes(
    roaming: PathBuf,
    local: PathBuf,
    world: impl FnOnce(&InstallPaths) -> World,
    files: impl FnOnce(&Environment, &mut MemFiles),
    web: FakeWeb,
    toasts: Toasts,
    instance: PipeProbe,
    webview: Option<&str>,
    slots: [Option<[u8; 32]>; 2],
) -> Probes {
    let paths = InstallPaths::new(&local, &roaming);
    let env = Environment {
        app_data: roaming.clone(),
        local_app_data: local.clone(),
    };
    let ops = Arc::new(FakeSystem::new(world(&paths)));
    let mut mem = MemFiles::default();
    mem.file(env.installed_exe(), "MZ");
    files(&env, &mut mem);
    let files = Arc::new(mem);
    let web = Arc::new(web);
    let probes = DoctorProbes {
        ops: Arc::clone(&ops) as _,
        files: Arc::clone(&files) as _,
        toasts: Arc::new(FixedToasts(toasts)),
        instance: Arc::new(FixedInstance(instance)),
        web: Arc::clone(&web) as _,
        webview_version: webview.map(str::to_owned),
        app_data: Some(roaming),
        local_app_data: Some(local),
        here: Some(paths.exe),
        slots,
    };
    Probes {
        ops,
        files,
        web,
        probes,
    }
}

const LISTING: &str = "https://artifactory.corp.internal/artifactory/api/storage/generic-release-local/mcp-proxy";

fn listing(versions: &[&str]) -> String {
    let children: Vec<String> = versions
        .iter()
        .map(|v| format!(r#"{{"uri": "/{v}", "folder": true}}"#))
        .collect();
    format!(r#"{{"children": [{}]}}"#, children.join(","))
}

fn enabled() -> Toasts {
    Toasts {
        setting: Some(0),
        mode: Some(0),
    }
}

fn answered() -> PipeProbe {
    PipeProbe::Answered {
        version: Some(own_version().to_string()),
    }
}

/// Everything in order, from the window.
fn all_well() -> Probes {
    let (roaming, local) = windows_folders();
    probes(
        roaming,
        local,
        installed_world,
        |env, files| {
            files.file(
                env.location(LocationId::ClaudeDesktop).file,
                &ours(&env.installed_exe()),
            )
        },
        FakeWeb::new(answer(401, Some(NOW_HTTP), "")).with(LISTING, answer(200, Some(NOW_HTTP), &listing(&["0.1.0"]))),
        enabled(),
        answered(),
        Some("131.0.2903.70"),
        [Some(release_key()), None],
    )
}

/// What waits for an action: the updates disabled (no key, as in this build), a `.old` left, the `Run` value of
/// another exe, the notifications of the application off, the gateway suspended, a client with another exe, one
/// server in error, the clock three minutes ahead.
fn warnings() -> Probes {
    let (roaming, local) = windows_folders();
    probes(
        roaming,
        local,
        |paths| {
            let mut world = installed_world(paths);
            world.files.insert(paths.backup.clone(), b"MZ".to_vec());
            world.run = Some(r#""C:\Users\exemple\Downloads\mcp-proxy.exe" --autostart"#.to_owned());
            world
        },
        |env, files| {
            files.file(
                env.location(LocationId::ClaudeDesktop).file,
                r#"{"mcpServers": {"mcp-proxy": {"command": "C:\\Users\\exemple\\Downloads\\mcp-proxy.exe", "args": ["stdio"]}}}"#,
            );
            files.dirs.insert(env.location(LocationId::VsCode).dir);
        },
        FakeWeb::new(answer(401, Some("Mon, 23 Nov 2026 09:57:00 GMT"), ""))
            .with(LISTING, answer(200, None, &listing(&["0.1.0", "0.2.0"])))
            .with("https://gitlab-mcp.dev.corp.internal/", answer(503, None, "")),
        Toasts {
            setting: Some(1),
            mode: Some(0),
        },
        answered(),
        Some("131.0.2903.70"),
        [None, None],
    )
}

/// What is broken, with what to do.
fn failures() -> Probes {
    let (roaming, local) = windows_folders();
    let dev = SigningKey::from_bytes(&mcp_proxy_testkit::DEV_SIGNING_SEED)
        .verifying_key()
        .to_bytes();
    probes(
        roaming,
        local,
        |paths| {
            let mut world = installed_world(paths);
            world.files.insert(paths.exe.clone(), b"MZ sans marque".to_vec());
            world
        },
        |env, files| {
            files.file(
                env.location(LocationId::VsCode).file,
                "{\"servers\": {\"a\": {} \"b\": {}}}",
            )
        },
        FakeWeb::new(Err(WebError::Dns)),
        Toasts {
            setting: None,
            mode: None,
        },
        PipeProbe::Foreign,
        None,
        [Some(dev), None],
    )
}

fn statuses(view: &Value) -> Vec<(String, String)> {
    view["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_owned(),
                c["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn status_of<'a>(statuses: &'a [(String, String)], id: &str) -> &'a str {
    &statuses.iter().find(|(c, _)| c == id).unwrap().1
}

fn events(gateway: &Gateway) -> Vec<String> {
    gateway
        .journal()
        .snapshot(&JournalFilter::default())
        .into_iter()
        .filter(|e| e.kind == EntryKind::Event)
        .map(|e| e.summary)
        .collect()
}

// ------------------------------------------------------------------------------------------------ the window

// Exigence : EX-024-01
// Exigence : EX-024-12
#[tokio::test]
async fn the_window_receives_exactly_the_contract() {
    let mut fixture = serde_json::Map::new();
    for (name, probes) in [("ok", all_well()), ("warnings", warnings()), ("failures", failures())] {
        let dir = TempDir::new();
        let gateway = start(&dir.0, corporate_config(), SecretStore::for_platform().unwrap()).await;
        choose_developer(&gateway).await;
        match name {
            "warnings" => gateway.suspend().await.unwrap(),
            "failures" => {
                // A program edits the settings file: its seal no longer holds.
                let settings = DataDirs::under(&dir.0).settings;
                let mut bytes = std::fs::read(&settings).unwrap();
                bytes.extend_from_slice(b" ");
                std::fs::write(&settings, bytes).unwrap();
            }
            _ => {}
        }
        let view = gateway.doctor_view(&probes.probes).await.unwrap();
        assert_eq!(probes.ops.log(), [], "{name}: nothing written to the system");
        assert_eq!(
            probes.files.writes.load(Ordering::SeqCst),
            0,
            "{name}: no client file written"
        );
        let file = view["file"].as_str().unwrap();
        assert_eq!(file, r"%LOCALAPPDATA%\MCP Proxy\logs\doctor-20261123-100000.txt");
        let written = std::fs::read_to_string(DataDirs::under(&dir.0).logs.join("doctor-20261123-100000.txt")).unwrap();
        assert_eq!(
            written,
            view["text"].as_str().unwrap(),
            "the dialog shows the report of the file"
        );
        fixture.insert(name.to_owned(), view);
    }
    let fixture = Value::Object(fixture);

    let ok = statuses(&fixture["ok"]);
    assert!(ok.iter().all(|(_, s)| s == "ok"), "{ok:?}");
    assert_eq!(ok.len(), 11, "every check of the table");
    assert_eq!(fixture["ok"]["status"], "ok");
    let warned = statuses(&fixture["warnings"]);
    for id in [
        "installation",
        "notifications",
        "settings",
        "artifactory",
        "updateKey",
        "clients",
        "mcp",
        "clock",
    ] {
        assert_eq!(status_of(&warned, id), "warning", "{id}: {warned:?}");
    }
    assert_eq!(fixture["warnings"]["status"], "warning");
    let failed = statuses(&fixture["failures"]);
    for id in [
        "webview",
        "installation",
        "notifications",
        "app",
        "settings",
        "artifactory",
        "updateKey",
        "clients",
        "mcp",
    ] {
        assert_eq!(status_of(&failed, id), "failure", "{id}: {failed:?}");
    }
    assert_eq!(fixture["failures"]["status"], "failure");

    let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    assert!(!text.contains(SENTINEL), "nothing of the files of the clients");
    assert!(!text.contains("github"));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/api/doctor.example.json");
    if std::env::var_os("MCP_PROXY_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    } else {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == text,
            "{} is not the view of the self-diagnostic any more: run `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p \
             mcp-proxy-gateway --test doctor`, then align ui/src/api/types.ts until `npm test` passes in ui/",
            path.display()
        );
    }
}

// Exigence : EX-024-04
// Exigence : EX-024-13
#[tokio::test]
async fn no_secret_in_the_report_of_the_window_the_file_and_the_export() {
    let dir = TempDir::new();
    let store = SecretStore::for_platform().unwrap();
    store
        .put(
            &SessionId::new("sso-corp").unwrap(),
            Credential::Bearer {
                token: SecretString::from(SENTINEL),
            },
        )
        .unwrap();
    let gateway = start(&dir.0, corporate_config(), store).await;
    choose_developer(&gateway).await;
    let mut probes = all_well();
    // Every text a probe gives may carry anything: the report filters them.
    probes.probes.instance = Arc::new(FixedInstance(PipeProbe::Failed(format!("canal {SENTINEL} fermé"))));
    probes.probes.webview_version = Some(format!("1.0.0.0 {SENTINEL}"));

    let view = gateway.doctor_view(&probes.probes).await.unwrap();
    let shown = view.to_string();
    assert!(!shown.contains(SENTINEL), "{shown}");
    assert!(shown.contains("[SECRET MASQUÉ]"), "{shown}");
    let file = std::fs::read_to_string(DataDirs::under(&dir.0).logs.join("doctor-20261123-100000.txt")).unwrap();
    assert!(!file.contains(SENTINEL) && file.contains("[SECRET MASQUÉ]"), "{file}");

    // The export: a report of now, filtered again in the archive.
    let report = gateway.diagnose(&probes.probes, Save::No).await.unwrap();
    assert!(report.file.is_none(), "the export writes no report file");
    assert!(report.text.ends_with("Rapport joint à l'export de diagnostic.\n"));
    let archive = gateway
        .export_diagnostic(&dir.0.join("diagnostic.zip"), Some(report.text))
        .await
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
    let mut attached = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("auto-diagnostic.txt").unwrap(), &mut attached).unwrap();
    assert!(attached.starts_with("MCP Proxy — auto-diagnostic\n"), "{attached}");
    assert!(attached.contains("lancé depuis la fenêtre de MCP Proxy"));
    assert!(!attached.contains(SENTINEL));
    let doctor_files = std::fs::read_dir(DataDirs::under(&dir.0).logs)
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("doctor-"))
        .count();
    assert_eq!(doctor_files, 1, "only the report of the dialog");
    assert!(
        !events(&gateway)
            .iter()
            .any(|e| e.contains("diagnostic") && e.contains(SENTINEL))
    );
}

// ----------------------------------------------------------------------------------------------- the command

fn snapshot(root: &Path, skip: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.starts_with(skip) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

fn command_probes(home: &Path) -> Probes {
    probes(
        home.join("Roaming"),
        home.join("Local"),
        installed_world,
        |_, _| {},
        FakeWeb::new(answer(401, Some(NOW_HTTP), ""))
            .with(LISTING, answer(200, None, &listing(&["0.1.0"])))
            .with(
                &LISTING.replace("corp.internal", "example.internal"),
                answer(200, None, &listing(&["0.1.0"])),
            ),
        enabled(),
        PipeProbe::NotRunning,
        Some("131.0.2903.70"),
        [None, None],
    )
}

// Exigence : EX-024-04
// Exigence : EX-024-08
#[tokio::test]
async fn the_command_filters_its_report_with_the_generic_patterns() {
    let home = TempDir::new();
    let mut probes = command_probes(&home.0);
    probes.probes.instance = Arc::new(FixedInstance(PipeProbe::Failed(format!("jeton {JWT}"))));
    let mut stdout = Vec::new();
    let published = run_command_with(
        &probes.probes,
        Arc::new(clock()),
        &RealReportFiles,
        &mut stdout,
        platform_key_wrapper,
        modules::embedded_config(),
    )
    .await;
    let out = String::from_utf8(stdout).unwrap();
    assert!(!out.contains(JWT), "{out}");
    assert!(out.contains("[JWT MASQUÉ]"), "{out}");
    let file = DataDirs::under(&home.0.join("Local"))
        .logs
        .join("doctor-20261123-100000.txt");
    assert_eq!(published.file.as_deref(), Some(file.as_path()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), out);
    assert!(out.contains("lancé par mcp-proxy.exe doctor"));
    assert!(out.ends_with("Rapport enregistré dans %LOCALAPPDATA%\\MCP Proxy\\logs\\doctor-20261123-100000.txt\n"));
    assert_eq!(published.exit_code, 2, "the pipe failed");
}

// Exigence : EX-024-03
// Exigence : EX-024-11
#[tokio::test]
async fn the_command_modifies_nothing_but_its_report() {
    // A first launch: nothing in the data folder. Reading the seal creates no key.
    let home = TempDir::new();
    let probes = command_probes(&home.0);
    let mut stdout = Vec::new();
    let published = run_command_with(
        &probes.probes,
        Arc::new(clock()),
        &RealReportFiles,
        &mut stdout,
        platform_key_wrapper,
        modules::embedded_config(),
    )
    .await;
    let data = DataDirs::under(&home.0.join("Local"));
    assert!(
        !data.settings_key.exists() && !data.settings.exists(),
        "no key, no settings created"
    );
    let created: Vec<PathBuf> = snapshot(&home.0, Path::new("/nonexistent")).into_keys().collect();
    assert_eq!(created, [data.logs.join("doctor-20261123-100000.txt")]);
    let out = String::from_utf8(stdout).unwrap();
    assert!(
        out.contains("[avertissement] Réglages\n  ! aucun réglage enregistré"),
        "{out}"
    );
    assert!(
        out.contains("[avertissement] Config"),
        "the example configuration: {out}"
    );
    assert!(out.contains("[ok] MCP\n  - aucune instance exposée"), "{out}");
    assert!(
        out.contains("[avertissement] Appli\n  ! MCP Proxy n'est pas lancé"),
        "{out}"
    );
    assert!(
        out.contains("[avertissement] Clé de mise à jour\n  ! aucune clé de mise à jour embarquée"),
        "{out}"
    );
    assert_eq!(published.exit_code, 1);
    assert_eq!(probes.ops.log(), []);
    assert_eq!(probes.files.writes.load(Ordering::SeqCst), 0);
    // Only the origin of Artifactory: nothing is exposed before a profile is chosen.
    assert_eq!(
        *probes.web.asked.lock().unwrap(),
        ["https://artifactory.example.internal/artifactory/api/storage/generic-release-local/mcp-proxy"]
    );

    // Settings sealed by the application: read, never rewritten; the exposed instances only are contacted.
    let home = TempDir::new();
    let data = DataDirs::under(&home.0.join("Local"));
    {
        let gateway = start(
            &home.0.join("Local"),
            corporate_config(),
            SecretStore::for_platform().unwrap(),
        )
        .await;
        choose_developer(&gateway).await;
        let _ = gateway.journal().flush();
    }
    let before = snapshot(&home.0, &data.logs);
    assert!(before.contains_key(&data.settings_key) && before.contains_key(&data.settings));
    let probes = command_probes(&home.0);
    let mut stdout = Vec::new();
    run_command_with(
        &probes.probes,
        Arc::new(clock()),
        &RealReportFiles,
        &mut stdout,
        platform_key_wrapper,
        Ok(corporate_config()),
    )
    .await;
    assert_eq!(snapshot(&home.0, &data.logs), before, "settings and key byte for byte");
    let out = String::from_utf8(stdout).unwrap();
    assert!(
        out.contains("sceau des réglages valide ; profil « Développeur »"),
        "{out}"
    );
    let config = corporate_config();
    let settings = {
        let gateway = start(
            &home.0.join("Local"),
            corporate_config(),
            SecretStore::for_platform().unwrap(),
        )
        .await;
        gateway.settings()
    };
    let mut expected: Vec<String> = mcp_targets(&config, &Exposure::from_settings(&settings, &config))
        .iter()
        .map(|t| format!("{}/", t.url.origin().ascii_serialization()))
        .collect();
    expected.push(LISTING.to_owned());
    expected.sort();
    expected.dedup();
    let mut asked = probes.web.asked.lock().unwrap().clone();
    asked.sort();
    assert_eq!(asked, expected, "only the exposed instances and Artifactory");
    assert!(
        !asked.iter().any(|a| a.contains(".prod.") || a.contains(".pra.")),
        "no environment not chosen"
    );
}

// -------------------------------------------------------------------------------------------- pipe, network

// Exigence : EX-024-10
#[tokio::test]
async fn the_ping_of_the_assembled_gateway_has_no_effect() {
    let dir = TempDir::new();
    let gateway = Arc::new(start(&dir.0, corporate_config(), SecretStore::for_platform().unwrap()).await);
    choose_developer(&gateway).await;
    let settings = std::fs::read(DataDirs::under(&dir.0).settings).unwrap();
    let journal = events(&gateway);
    let exit = {
        let gateway = Arc::clone(&gateway);
        tokio::spawn(async move { gateway.exit_requested().await })
    };
    let (client_end, app_end) = tokio::io::duplex(1 << 16);
    let served = {
        let gateway = Arc::clone(&gateway);
        tokio::spawn(async move { gateway.serve_connection(Arc::new(FakeUser(1)), app_end).await })
    };
    let pong = request_ping(Arc::new(FakeUser(1)), client_end).await;
    assert_eq!(pong, Ok(Some(own_version().to_string())));
    // The application closes the channel after the answer, and nothing else happens.
    assert_eq!(served.await.unwrap(), Ok(()));
    tokio::task::yield_now().await;
    assert!(!exit.is_finished(), "no exit asked");
    assert_eq!(events(&gateway), journal, "nothing journaled");
    assert_eq!(std::fs::read(DataDirs::under(&dir.0).settings).unwrap(), settings);
    exit.abort();
}

/// Records every request, answers 401 with a challenge.
async fn recording_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let record = Arc::clone(&record);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                record
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).into_owned());
                let _ = stream
                    .write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer\r\nContent-Length: 0\r\n\r\n")
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (port, seen)
}

// Exigence : EX-024-03
#[tokio::test]
async fn the_mcp_check_of_the_window_never_uses_the_credentials_of_the_sessions() {
    let (port, seen) = recording_server().await;
    let local = format!("http://127.0.0.1:{port}");
    let toml = EMBEDDED_TOML.replace("https://kibana-mcp.dev.example.internal", &local);
    let config = Config::from_toml(&toml, &modules::builtin().ids()).unwrap();
    let store = SecretStore::for_platform().unwrap();
    for session in ["sso-dev", "sso-corp", "sso-tools"] {
        store
            .put(
                &SessionId::new(session).unwrap(),
                Credential::Bearer {
                    token: SecretString::from(SENTINEL),
                },
            )
            .unwrap();
    }
    let dir = TempDir::new();
    let gateway = start(&dir.0, config, store).await;
    choose_developer(&gateway).await;
    let mut probes = all_well();
    probes.probes.web = Arc::new(AnonymousWeb::new().unwrap());
    let view = gateway.doctor_view(&probes.probes).await.unwrap();
    let mcp = view["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "mcp")
        .unwrap()
        .clone();
    let line = mcp["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["message"].as_str().unwrap().contains("127.0.0.1"))
        .unwrap()
        .clone();
    assert_eq!(line["status"], "ok", "a 401 counts as reachable: {line}");

    let doctor: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("GET / "))
        .cloned()
        .collect();
    assert_eq!(doctor.len(), 1, "one request to the origin of the exposed instance");
    let request = doctor[0].to_ascii_lowercase();
    for header in ["authorization:", "cookie:", "proxy-authorization:"] {
        assert!(!request.contains(header), "{header}: {request}");
    }
    assert!(!doctor[0].contains(SENTINEL));
    assert!(!json!(view).to_string().contains(SENTINEL));
}

#[test]
fn the_clock_of_the_tests_is_the_one_of_the_contract() {
    assert_eq!(mcp_proxy_deploy::doctor::http_date(NOW_HTTP), Some(clock().now()));
}
