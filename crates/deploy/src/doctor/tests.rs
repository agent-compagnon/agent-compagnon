#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use mcp_proxy_testkit::DEV_SIGNING_SEED;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use super::*;
use crate::clients::{ClientFiles, Environment, LocationId};
use crate::fake::{FakeSystem, World};
use crate::install::{InstallPaths, run_command};
use crate::uninstall::entry_for;
use crate::update::{Channel, Source};
use crate::version::{Version, own_version};

// ------------------------------------------------------------------------------------------------------ fakes

/// Answers from a table; every address asked is recorded.
#[derive(Default)]
struct FakeWeb {
    answers: BTreeMap<String, Result<Answer, WebError>>,
    unresolved: Vec<String>,
    asked: Mutex<Vec<String>>,
}

impl FakeWeb {
    fn answer(mut self, url: &str, status: u16, body: &str, date: Option<&str>) -> Self {
        self.answers.insert(
            url.to_owned(),
            Ok(Answer {
                status,
                date: date.map(str::to_owned),
                body: body.as_bytes().to_vec(),
            }),
        );
        self
    }

    fn fail(mut self, url: &str, error: WebError) -> Self {
        self.answers.insert(url.to_owned(), Err(error));
        self
    }

    fn unresolved(mut self, host: &str) -> Self {
        self.unresolved.push(host.to_owned());
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl Web for FakeWeb {
    async fn resolve(&self, host: &str, _port: u16) -> Result<(), WebError> {
        if self.unresolved.iter().any(|h| h == host) {
            Err(WebError::Dns)
        } else {
            Ok(())
        }
    }

    async fn get(&self, url: &Url, _max_body: u64) -> Result<Answer, WebError> {
        self.asked.lock().unwrap().push(url.to_string());
        self.answers
            .get(url.as_str())
            .cloned()
            .unwrap_or(Err(WebError::Connect))
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

/// Never answers: the bound of the diagnosis.
struct Silent;

#[async_trait]
impl InstanceProbe for Silent {
    async fn probe(&self) -> PipeProbe {
        std::future::pending().await
    }
}

async fn findings(check: impl Check) -> Vec<Finding> {
    check.run().await
}

fn statuses(found: &[Finding]) -> Vec<Status> {
    found.iter().map(|f| f.status).collect()
}

fn worst(found: &[Finding]) -> Status {
    found.iter().map(|f| f.status).max().unwrap()
}

/// Every finding of a warning or a failure says what to do; every text is French without a path of the machine.
fn readable(found: &[Finding]) {
    for finding in found {
        assert!(!finding.message.is_empty());
        if finding.status != Status::Ok {
            assert!(finding.fix.as_deref().is_some_and(|f| !f.is_empty()), "{finding:?}");
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-doctor-{}-{}",
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

// ---------------------------------------------------------------------------------------------------- Config

// Exigence : EX-024-01
#[tokio::test]
async fn config_in_its_three_outcomes() {
    let ok = findings(ConfigCheck {
        loaded: Ok(ConfigFacts {
            example_hosts: 0,
            environments: 8,
            mcp: 5,
        }),
    })
    .await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    assert!(ok[0].message.contains("8 environnements, 5 MCP"), "{ok:?}");

    let example = findings(ConfigCheck {
        loaded: Ok(ConfigFacts {
            example_hosts: 3,
            environments: 8,
            mcp: 5,
        }),
    })
    .await;
    assert_eq!(statuses(&example), [Status::Warning]);

    let errors = (0..7).map(|i| format!("erreur {i}")).collect();
    let invalid = findings(ConfigCheck { loaded: Err(errors) }).await;
    assert_eq!(statuses(&invalid), [Status::Failure]);
    assert!(invalid[0].message.contains("erreur 4 ; et 2 autres"), "{invalid:?}");
    for found in [ok, example, invalid] {
        readable(&found);
    }
}

// ------------------------------------------------------------------------------------------------- WebView2

// Exigence : EX-024-01
#[tokio::test]
async fn webview_in_its_three_outcomes() {
    let check = |version: Option<&str>, sufficient, in_memory| WebviewCheck {
        version: version.map(str::to_owned),
        sufficient,
        in_memory,
    };
    let ok = findings(check(Some("131.0.2903.70"), true, true)).await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    assert!(ok[0].message.contains("131.0.2903.70"));
    let old_on_disk = findings(check(Some("100.0.1.1"), false, false)).await;
    assert_eq!(statuses(&old_on_disk), [Status::Warning]);
    let old = findings(check(Some("100.0.1.1"), false, true)).await;
    assert_eq!(statuses(&old), [Status::Failure]);
    let absent = findings(check(None, false, true)).await;
    assert_eq!(statuses(&absent), [Status::Failure]);
    for found in [ok, old_on_disk, old, absent] {
        readable(&found);
    }
}

// --------------------------------------------------------------------------------------------- Installation

fn paths() -> InstallPaths {
    InstallPaths::new(Path::new("/u/Local"), Path::new("/u/Roaming"))
}

fn exe_of(version: &str) -> Vec<u8> {
    format!("MZ\u{1}mcp-proxy:version={version}\u{1}").into_bytes()
}

/// An installation as the exe leaves it: shortcut, AUMID, exclusion, `Run` value and entry.
fn installed_world() -> World {
    let paths = paths();
    let exe = exe_of("0.3.0");
    let mut world = World::default();
    for dir in [
        "/u",
        "/u/Local",
        "/u/Local/Programs",
        "/u/Local/Programs/MCP Proxy",
        "/u/Roaming",
        "/u/Roaming/Microsoft",
        "/u/Roaming/Microsoft/Windows",
        "/u/Roaming/Microsoft/Windows/Start Menu",
        "/u/Roaming/Microsoft/Windows/Start Menu/Programs",
    ] {
        world.dirs.insert(PathBuf::from(dir));
    }
    world.uninstall = entry_for(&paths, Some(&Version::new(0, 3, 0)), exe.len() as u64);
    world.files.insert(paths.exe.clone(), exe);
    world.files.insert(paths.shortcut.clone(), b"lnk".to_vec());
    world.aumid = true;
    world.wer = Some(1);
    world.run = run_command(&paths.exe);
    world
}

async fn installation(world: World) -> (Vec<Finding>, Arc<FakeSystem>) {
    let fake = Arc::new(FakeSystem::new(world));
    let found = InstallationCheck {
        ops: Arc::clone(&fake) as Arc<dyn crate::ops::SystemOps>,
        paths: Some(paths()),
        here: Some(paths().exe),
    }
    .run()
    .await;
    (found, fake)
}

// Exigence : EX-024-01
// Exigence : EX-024-11
#[tokio::test]
async fn installation_in_its_three_outcomes_and_nothing_is_written() {
    let (ok, fake) = installation(installed_world()).await;
    assert_eq!(statuses(&ok), [Status::Ok], "{ok:?}");
    for part in [
        "version 0.3.0",
        "lancé depuis l'installation",
        "raccourci",
        "AUMID",
        "rapports d'erreur",
        "démarrage avec Windows activé",
        "« Applications installées » en version 0.3.0",
    ] {
        assert!(ok[0].message.contains(part), "{part}: {ok:?}");
    }
    assert_eq!(fake.log(), [], "a check only reads");

    // What the previous steps asked to report: a `.old` left, a `Run` value that starts another exe, an entry
    // missing or of another version, the shortcut, the AUMID and the exclusion missing.
    let mut world = installed_world();
    world.files.insert(paths().backup, exe_of("0.2.0"));
    world.files.remove(&paths().shortcut);
    world.aumid = false;
    world.wer = None;
    world.run = Some(r#""C:\Downloads\mcp-proxy.exe" --autostart"#.to_owned());
    world.uninstall = entry_for(&paths(), Some(&Version::new(0, 2, 0)), 10);
    let before = world.clone();
    let (warned, fake) = installation(world).await;
    assert_eq!(worst(&warned), Status::Warning, "{warned:?}");
    let texts: Vec<&str> = warned.iter().map(|f| f.message.as_str()).collect();
    for part in [
        "mcp-proxy.exe.old",
        "raccourci",
        "AUMID",
        "rapports d'erreur",
        "lance un autre programme",
        "affiche la version 0.2.0",
    ] {
        assert!(texts.iter().any(|t| t.contains(part)), "{part}: {texts:?}");
    }
    assert_eq!(fake.log(), []);
    assert_eq!(fake.world(), before, "nothing repaired");

    let mut world = installed_world();
    world.uninstall = None;
    let (missing, _) = installation(world).await;
    assert!(
        missing
            .iter()
            .any(|f| f.status == Status::Warning && f.message.contains("« Applications installées » absente")),
        "an entry missing is a warning, repaired at the next start: {missing:?}"
    );

    let mut world = installed_world();
    world.files.insert(paths().exe, b"MZ without mark".to_vec());
    let (broken, _) = installation(world).await;
    assert_eq!(worst(&broken), Status::Failure, "{broken:?}");
    assert!(
        broken
            .iter()
            .any(|f| f.message.contains("version de l'exe installé illisible"))
    );

    let mut world = installed_world();
    world.read_only.insert(paths().dir);
    let (read_only, _) = installation(world).await;
    assert!(
        read_only
            .iter()
            .any(|f| f.status == Status::Failure && f.message.contains("lecture seule"))
    );

    let (absent, fake) = installation(World::default()).await;
    assert_eq!(statuses(&absent), [Status::Warning]);
    assert!(absent[0].message.contains("n'est pas installé"));
    assert_eq!(fake.log(), []);

    let elsewhere = InstallationCheck {
        ops: Arc::new(FakeSystem::new(installed_world())),
        paths: Some(paths()),
        here: Some(PathBuf::from("/u/Downloads/mcp-proxy.exe")),
    }
    .run()
    .await;
    assert!(elsewhere.iter().any(|f| f.message.contains("n'est pas l'exe installé")));

    let no_folders = InstallationCheck {
        ops: Arc::new(FakeSystem::default()),
        paths: None,
        here: None,
    }
    .run()
    .await;
    assert_eq!(statuses(&no_folders), [Status::Failure]);
    for found in [ok, warned, missing, broken, read_only, absent, elsewhere, no_folders] {
        readable(&found);
    }
}

// -------------------------------------------------------------------------------------------- Notifications

// Exigence : EX-024-01
#[tokio::test]
async fn notifications_in_their_three_outcomes() {
    let check = |setting, mode| NotificationsCheck {
        probe: Arc::new(FixedToasts(Toasts { setting, mode })),
    };
    let ok = findings(check(Some(0), Some(0))).await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    for (setting, mode, words) in [
        (Some(1), Some(0), "désactivées pour MCP Proxy"),
        (Some(2), Some(0), "toutes les applications"),
        (Some(3), Some(0), "stratégie"),
        (Some(0), Some(1), "Ne pas déranger"),
    ] {
        let warned = findings(check(setting, mode)).await;
        assert_eq!(statuses(&warned), [Status::Warning]);
        assert!(warned[0].message.contains(words), "{warned:?}");
        assert!(warned[0].message.contains("fenêtre au premier plan"));
        readable(&warned);
    }
    let disabled = findings(check(Some(1), Some(0))).await;
    assert!(disabled[0].fix.as_deref().unwrap().contains("« Notifications »"));
    let unreadable = findings(check(None, Some(0))).await;
    assert_eq!(statuses(&unreadable), [Status::Failure]);
    readable(&unreadable);
}

// ----------------------------------------------------------------------------------------------------- Appli

// Exigence : EX-024-01
#[tokio::test]
async fn the_application_in_its_three_outcomes_from_both_viewpoints() {
    let check = |probe: PipeProbe, viewpoint| AppCheck {
        probe: Arc::new(FixedInstance(probe)),
        viewpoint,
        installed: Some(Version::new(0, 3, 0)),
    };
    let answered = |v: &str| PipeProbe::Answered {
        version: Some(v.to_owned()),
    };
    let ok = findings(check(answered("0.3.0"), Viewpoint::Command)).await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    assert!(
        ok[0].message.starts_with("instance en cours en version 0.3.0"),
        "{ok:?}"
    );
    let older = findings(check(answered("0.2.0"), Viewpoint::Command)).await;
    assert_eq!(statuses(&older), [Status::Warning]);
    let old_app = findings(check(PipeProbe::Answered { version: None }, Viewpoint::Command)).await;
    assert_eq!(statuses(&old_app), [Status::Ok]);
    let not_running = findings(check(PipeProbe::NotRunning, Viewpoint::Command)).await;
    assert_eq!(statuses(&not_running), [Status::Warning]);
    for probe in [
        PipeProbe::Foreign,
        PipeProbe::Busy,
        PipeProbe::NoUser,
        PipeProbe::Failed("délai dépassé".to_owned()),
    ] {
        let failed = findings(check(probe, Viewpoint::Command)).await;
        assert_eq!(statuses(&failed), [Status::Failure]);
        readable(&failed);
    }

    // From inside: the instance is this one, and its own pipe must answer.
    let inside = findings(check(answered("0.2.0"), Viewpoint::App)).await;
    assert_eq!(statuses(&inside), [Status::Ok]);
    assert!(inside[0].message.starts_with("cette instance en version 0.2.0"));
    let deaf = findings(check(PipeProbe::NotRunning, Viewpoint::App)).await;
    assert_eq!(statuses(&deaf), [Status::Failure]);
    for found in [ok, older, old_app, not_running, inside, deaf] {
        readable(&found);
    }
}

// -------------------------------------------------------------------------------------------------- Réglages

// Exigence : EX-024-01
#[tokio::test]
async fn settings_in_their_three_outcomes() {
    let facts = |seal, profile: Option<&str>, exposed: &[&str], suspended| SettingsCheck {
        facts: SettingsFacts {
            seal,
            profile: profile.map(str::to_owned),
            exposed: exposed.iter().map(|e| (*e).to_owned()).collect(),
            suspended,
            running: false,
        },
    };
    let ok = findings(facts(SealFacts::Valid, Some("Développeur"), &["DEV", "REC"], false)).await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    assert!(
        ok[0]
            .message
            .contains("profil « Développeur », environnements exposés : DEV, REC")
    );
    let first = findings(facts(SealFacts::Missing, None, &[], false)).await;
    assert_eq!(statuses(&first), [Status::Warning]);
    let no_profile = findings(facts(SealFacts::Valid, None, &[], false)).await;
    assert_eq!(statuses(&no_profile), [Status::Ok, Status::Warning]);
    let nothing = findings(facts(SealFacts::Valid, Some("Support"), &[], false)).await;
    assert_eq!(statuses(&nothing), [Status::Ok, Status::Warning]);
    let suspended = findings(facts(SealFacts::Valid, Some("Support"), &["DEV"], true)).await;
    assert_eq!(statuses(&suspended), [Status::Ok, Status::Warning]);
    assert!(suspended[1].message.contains("suspendue"));
    let refused = findings(facts(SealFacts::Refused("sceau invalide".to_owned()), None, &[], false)).await;
    assert_eq!(statuses(&refused), [Status::Failure]);
    assert!(refused[0].message.contains("sceau invalide"));
    assert!(refused[0].message.contains("aucun environnement n'est exposé"));
    // From inside the application, the settings in memory still apply: the file matters at the next start.
    let mut inside = facts(
        SealFacts::Refused("sceau invalide".to_owned()),
        Some("Support"),
        &["DEV"],
        false,
    );
    inside.facts.running = true;
    let inside = inside.run().await;
    assert_eq!(statuses(&inside), [Status::Failure]);
    assert!(inside[0].message.contains("au prochain démarrage"), "{inside:?}");
    for found in [ok, first, no_profile, nothing, suspended, refused] {
        readable(&found);
    }
}

// ----------------------------------------------------------------------------------------------- Artifactory

const LISTING: &str = "https://artifactory.corp/artifactory/api/storage/releases/mcp-proxy";

fn source() -> Source {
    Source {
        base_url: "https://artifactory.corp/artifactory".to_owned(),
        repository: "releases".to_owned(),
        path: "mcp-proxy".to_owned(),
    }
}

fn listing(versions: &[&str]) -> String {
    let children: Vec<String> = versions
        .iter()
        .map(|v| format!(r#"{{"uri": "/{v}", "folder": true}}"#))
        .collect();
    format!(r#"{{"children": [{}]}}"#, children.join(","))
}

fn artifactory(web: FakeWeb, updates: bool) -> (ArtifactoryCheck, Arc<FakeWeb>) {
    let web = Arc::new(web);
    (
        ArtifactoryCheck {
            web: Arc::clone(&web) as Arc<dyn Web>,
            source: Some(source()),
            channel: Channel::Stable,
            current: Version::new(0, 3, 0),
            updates,
            dates: Dates::default(),
        },
        web,
    )
}

// Exigence : EX-024-01
#[tokio::test]
async fn artifactory_in_its_three_outcomes() {
    let (check, web) = artifactory(
        FakeWeb::default().answer(LISTING, 200, &listing(&["0.2.0", "0.3.0", "0.4.0-rc.1"]), None),
        true,
    );
    let ok = check.run().await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    assert!(
        ok[0].message.contains("à jour (dernière version publiée : 0.3.0"),
        "{ok:?}"
    );
    assert_eq!(web.asked(), [LISTING], "only the Storage API, anonymously");

    let (check, _) = artifactory(
        FakeWeb::default().answer(LISTING, 200, &listing(&["0.4.0"]), None),
        true,
    );
    let newer = check.run().await;
    assert_eq!(statuses(&newer), [Status::Ok]);
    assert!(newer[0].message.contains("0.4.0 est disponible et sera installée"));

    let (check, _) = artifactory(
        FakeWeb::default().answer(LISTING, 200, &listing(&["0.4.0"]), None),
        false,
    );
    let without_key = check.run().await;
    assert_eq!(statuses(&without_key), [Status::Warning]);
    let (check, _) = artifactory(FakeWeb::default().answer(LISTING, 404, "", None), true);
    assert_eq!(statuses(&check.run().await), [Status::Warning]);
    let (check, _) = artifactory(FakeWeb::default().answer(LISTING, 200, &listing(&[]), None), true);
    assert_eq!(statuses(&check.run().await), [Status::Warning]);

    let (check, _) = artifactory(FakeWeb::default().answer(LISTING, 401, "", None), true);
    let denied = check.run().await;
    assert_eq!(statuses(&denied), [Status::Failure]);
    let (check, _) = artifactory(FakeWeb::default().answer(LISTING, 200, "<html>", None), true);
    assert_eq!(statuses(&check.run().await), [Status::Failure]);
    let (check, _) = artifactory(FakeWeb::default().fail(LISTING, WebError::Dns), true);
    let unreachable = check.run().await;
    assert_eq!(statuses(&unreachable), [Status::Failure]);
    assert!(unreachable[0].message.contains("artifactory.corp"));
    assert!(!unreachable[0].message.contains("api/storage"), "no full address");
    for found in [ok, newer, without_key, denied, unreachable] {
        readable(&found);
    }
}

// ---------------------------------------------------------------------------------------- Clé de mise à jour

// Exigence : EX-024-01
#[tokio::test]
async fn the_update_key_in_its_three_outcomes() {
    let dev = SigningKey::from_bytes(&DEV_SIGNING_SEED).verifying_key().to_bytes();
    let release = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
    let ok = findings(UpdateKeyCheck {
        slots: [Some(release), None],
    })
    .await;
    assert_eq!(statuses(&ok), [Status::Ok]);
    // Today's build: no key embedded, the updates are disabled.
    let none = findings(UpdateKeyCheck { slots: [None, None] }).await;
    assert_eq!(statuses(&none), [Status::Warning]);
    assert!(none[0].message.contains("ne se met jamais à jour"));
    let development = findings(UpdateKeyCheck {
        slots: [Some(release), Some(dev)],
    })
    .await;
    assert_eq!(statuses(&development), [Status::Failure]);
    assert!(UpdateKeyCheck::usable(&[Some(release), None]));
    assert!(!UpdateKeyCheck::usable(&[Some(dev), None]));
    assert!(!UpdateKeyCheck::usable(&[None, None]));
    for found in [ok, none, development] {
        readable(&found);
    }
}

// ---------------------------------------------------------------------------------------------- Clients LLM

/// The real files, every write counted: a check must make none.
#[derive(Default)]
struct CountingFiles(AtomicUsize);

impl ClientFiles for CountingFiles {
    fn create_new(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
}

fn environment(root: &Path) -> Environment {
    Environment {
        app_data: root.join("Roaming"),
        local_app_data: root.join("Local"),
    }
}

fn write_client(env: &Environment, id: LocationId, text: &str) {
    let location = env.location(id);
    std::fs::create_dir_all(&location.dir).unwrap();
    std::fs::write(&location.file, text).unwrap();
}

fn ours(exe: &Path) -> String {
    format!(
        "{{\"mcpServers\": {{\"mcp-proxy\": {{\"command\": {}, \"args\": [\"stdio\"]}}, \"github\": {{\"env\": \
         {{\"TOKEN\": \"ghp_SENTINELLE\"}}}}}}}}",
        serde_json::Value::String(exe.display().to_string())
    )
}

async fn clients(root: &Path) -> (Vec<Finding>, Arc<CountingFiles>) {
    let files = Arc::new(CountingFiles::default());
    let found = ClientsCheck {
        files: Arc::clone(&files) as Arc<dyn ClientFiles>,
        env: Some(environment(root)),
    }
    .run()
    .await;
    (found, files)
}

// Exigence : EX-024-01
// Exigence : EX-024-11
#[tokio::test]
async fn the_llm_clients_in_their_three_outcomes() {
    let root = TempDir::new();
    let env = environment(&root.0);
    let exe = env.installed_exe();
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"MZ").unwrap();

    let (nothing, _) = clients(&root.0).await;
    assert_eq!(statuses(&nothing), [Status::Warning]);
    assert!(nothing[0].message.contains("aucun client LLM détecté"));

    std::fs::create_dir_all(env.location(LocationId::VsCode).dir).unwrap();
    let (not_configured, _) = clients(&root.0).await;
    assert_eq!(worst(&not_configured), Status::Warning);
    assert!(
        not_configured
            .iter()
            .any(|f| f.message == "VS Code : installé, non configuré")
    );

    write_client(&env, LocationId::ClaudeDesktopPackage, &ours(&exe));
    write_client(&env, LocationId::ClaudeDesktop, "{\"mcpServers\": {}}");
    let (ok, files) = clients(&root.0).await;
    assert_eq!(worst(&ok), Status::Ok, "{ok:?}");
    let texts: Vec<&str> = ok.iter().map(|f| f.message.as_str()).collect();
    assert!(texts.contains(&"Claude Desktop (paquet MSIX) : configuré"), "{texts:?}");
    // Both places of Claude Desktop: which one the packaged application reads (spec 023).
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Claude Desktop : installé, non configuré (sans doute pas lu")),
        "{texts:?}"
    );

    write_client(
        &env,
        LocationId::ClaudeDesktop,
        "{\"mcpServers\": {\"mcp-proxy\": {\"command\": \"C:\\\\autre\\\\mcp-proxy.exe\", \"args\": [\"stdio\"]}}}",
    );
    let (other, _) = clients(&root.0).await;
    assert!(
        other
            .iter()
            .any(|f| f.status == Status::Warning && f.message.contains("autre exe"))
    );

    write_client(&env, LocationId::VsCode, "{\"servers\": {\"a\": {} \"b\": {}}}");
    let (unreadable, _) = clients(&root.0).await;
    assert_eq!(worst(&unreadable), Status::Failure);
    assert!(
        unreadable
            .iter()
            .any(|f| f.message.contains("VS Code : fichier de configuration illisible"))
    );

    assert_eq!(files.0.load(Ordering::SeqCst), 0, "nothing written");
    for found in [nothing, not_configured, ok, other, unreadable] {
        readable(&found);
        for finding in &found {
            let text = format!("{finding:?}");
            assert!(!text.contains("ghp_SENTINELLE") && !text.contains("github"), "{text}");
            assert!(!text.contains(&root.0.display().to_string()), "{text}");
        }
    }
}

// ------------------------------------------------------------------------------------------------------- MCP

fn target(label: &str, url: &str) -> McpTarget {
    McpTarget {
        label: label.to_owned(),
        url: Url::parse(url).unwrap(),
    }
}

// Exigence : EX-024-01
#[tokio::test]
async fn mcp_in_its_three_outcomes() {
    let web = Arc::new(
        FakeWeb::default()
            .answer("https://kibana.rec.corp/", 401, "", None)
            .answer("https://gitlab.rec.corp:8443/", 503, "", None)
            .fail("https://jira.rec.corp/", WebError::Tls)
            .unresolved("vault.rec.corp"),
    );
    let check = McpCheck {
        web: Arc::clone(&web) as Arc<dyn Web>,
        targets: vec![
            target("Kibana Logs (REC)", "https://kibana.rec.corp/mcp?ticket=abc"),
            target("Kibana bis (REC)", "https://kibana.rec.corp/autre"),
            target("GitLab (REC)", "https://gitlab.rec.corp:8443/mcp"),
            target("Jira (REC)", "https://jira.rec.corp/"),
            target("Vault (REC)", "https://vault.rec.corp/mcp"),
        ],
        dates: Dates::default(),
    };
    let found = check.run().await;
    assert_eq!(
        statuses(&found),
        [
            Status::Ok,
            Status::Ok,
            Status::Warning,
            Status::Failure,
            Status::Failure
        ]
    );
    assert_eq!(
        found[0].message,
        "Kibana Logs (REC) (kibana.rec.corp) : nom résolu, connexion sécurisée établie"
    );
    assert!(found[2].message.contains("HTTP 503"));
    assert!(found[3].message.contains("TLS"));
    assert!(found[4].message.contains("nom introuvable"));
    readable(&found);
    // The origins only, each once: never a path nor a query.
    let mut asked = web.asked();
    asked.sort();
    assert_eq!(
        asked,
        [
            "https://gitlab.rec.corp:8443/",
            "https://jira.rec.corp/",
            "https://kibana.rec.corp/",
            "https://vault.rec.corp/"
        ]
    );
    for finding in &found {
        assert!(!finding.message.contains("ticket") && !finding.message.contains("/mcp"));
    }

    let web = Arc::new(FakeWeb::default());
    let nothing = McpCheck {
        web: Arc::clone(&web) as Arc<dyn Web>,
        targets: Vec::new(),
        dates: Dates::default(),
    }
    .run()
    .await;
    assert_eq!(statuses(&nothing), [Status::Ok]);
    assert!(web.asked().is_empty(), "nothing exposed, nothing contacted");
}

/// A server that records each request as received, then answers `answer`.
async fn recording_server(answer: &'static str) -> (Url, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port())).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
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
            let _ = stream.write_all(answer.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (url, seen)
}

// Exigence : EX-024-03
#[tokio::test]
async fn the_mcp_check_sends_no_credential_and_follows_no_redirect() {
    let (elsewhere, elsewhere_seen) = recording_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
    let redirect: &'static str = Box::leak(
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {elsewhere}steal\r\nSet-Cookie: SESSION=abc\r\nDate: Mon, 23 Nov 2026 \
             10:00:00 GMT\r\nContent-Length: 0\r\n\r\n"
        )
        .into_boxed_str(),
    );
    let (server, seen) = recording_server(redirect).await;
    let (challenge, challenge_seen) = recording_server(
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer realm=\"mcp\"\r\nContent-Length: 0\r\n\r\n",
    )
    .await;
    let check = McpCheck {
        web: Arc::new(AnonymousWeb::new().unwrap()),
        targets: vec![
            target("Kibana Logs (REC)", &format!("{server}mcp?ticket=SECRET-TICKET")),
            target("Kibana Logs (DEV)", &format!("{server}autre")),
            target("GitLab (REC)", &format!("{challenge}mcp")),
        ],
        dates: Dates::default(),
    };
    let dates = check.dates.clone();
    let found = check.run().await;
    // Any HTTP answer counts as reachable, a redirect and a 401 included.
    assert_eq!(statuses(&found), [Status::Ok, Status::Ok, Status::Ok], "{found:?}");

    let requests: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .chain(challenge_seen.lock().unwrap().iter())
        .cloned()
        .collect();
    assert_eq!(requests.len(), 2, "one request per origin: {requests:?}");
    for request in &requests {
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("GET / HTTP/1.1\r\n"), "the origin only: {request}");
        for header in ["authorization:", "cookie:", "proxy-authorization:", "referer:"] {
            assert!(!lower.contains(header), "{header} sent: {request}");
        }
        assert!(!request.contains("SECRET-TICKET"));
    }
    assert!(
        elsewhere_seen.lock().unwrap().is_empty(),
        "the redirect was not followed"
    );
    // The `Date` header of an answer measures the clock, without a request of its own.
    assert!(dates.smallest_skew().is_some());

    // An address with credentials is never contacted.
    let refused = AnonymousWeb::new()
        .unwrap()
        .get(
            &Url::parse(&format!("http://user:pass@127.0.0.1:{}/", server.port().unwrap())).unwrap(),
            0,
        )
        .await;
    assert_eq!(refused, Err(WebError::Refused));
    let refused = AnonymousWeb::new()
        .unwrap()
        .get(&Url::parse("http://intranet.corp/").unwrap(), 0)
        .await;
    assert_eq!(refused, Err(WebError::Refused), "plain HTTP towards the loopback only");
}

// Exigence : EX-024-01
#[tokio::test]
async fn a_refused_connection_is_told_apart() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let refused = AnonymousWeb::new()
        .unwrap()
        .get(&Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(), 0)
        .await;
    assert_eq!(refused, Err(WebError::Connect));
    assert_eq!(
        classify_text("error trying to connect: The certificate chain was issued by an authority that is not trusted"),
        WebError::Tls
    );
    assert_eq!(
        classify_text("error:0A000086:SSL routines::certificate verify failed"),
        WebError::Tls
    );
    assert_eq!(
        classify_text("dns error: failed to lookup address information"),
        WebError::Dns
    );
    assert_eq!(
        classify_text("tcp connect error: Connection refused (os error 111)"),
        WebError::Connect
    );
}

// ---------------------------------------------------------------------------------------------------- Horloge

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

// Exigence : EX-024-01
#[test]
fn the_clock_in_its_three_outcomes() {
    // Mon, 23 Nov 2026 10:00:00 GMT.
    let server = 1_795_428_000;
    assert_eq!(http_date("Mon, 23 Nov 2026 10:00:00 GMT"), Some(at(server)));
    assert_eq!(http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(at(784_111_777)));
    for wrong in [
        "Monday, 23-Nov-26 10:00:00 GMT",
        "Mon Nov 23 10:00:00 2026",
        "Mon, 23 Nov 2026 10:00:00 UTC",
        "Mon, 32 Nov 2026 10:00:00 GMT",
        "",
    ] {
        assert_eq!(http_date(wrong), None, "{wrong}");
    }
    let clock = |local: u64| {
        let dates = Dates::default();
        dates.record_at(Some("Mon, 23 Nov 2026 10:00:00 GMT"), at(local));
        ClockCheck { dates }.run()
    };
    assert_eq!(statuses(&clock(server + 3)), [Status::Ok]);
    let ahead = clock(server + 180);
    assert_eq!(statuses(&ahead), [Status::Warning]);
    assert!(ahead[0].message.contains("en avance de 3 min"), "{ahead:?}");
    let behind = clock(server - 7200);
    assert_eq!(statuses(&behind), [Status::Failure]);
    assert!(behind[0].message.contains("en retard de 2 h"), "{behind:?}");
    let unmeasured = ClockCheck {
        dates: Dates::default(),
    }
    .run();
    assert_eq!(statuses(&unmeasured), [Status::Ok]);
    for found in [ahead, behind, unmeasured] {
        readable(&found);
    }

    // The smallest gap wins: one server with a wrong clock does not make this one wrong.
    let dates = Dates::default();
    dates.record_at(Some("Mon, 23 Nov 2026 12:00:00 GMT"), at(server));
    dates.record_at(Some("Mon, 23 Nov 2026 10:00:01 GMT"), at(server));
    dates.record_at(Some("pas une date"), at(server));
    assert_eq!(dates.smallest_skew(), Some(-1));
    assert_eq!(skew_status(60), Status::Ok);
    assert_eq!(skew_status(-61), Status::Warning);
    assert_eq!(skew_status(301), Status::Failure);
}

// ---------------------------------------------------------------------------------------------------- report

fn report(statuses: &[Status]) -> Report {
    Report {
        version: "0.3.0".to_owned(),
        at: "2026-11-23T10:00:00.000Z".to_owned(),
        viewpoint: Viewpoint::Command,
        checks: statuses
            .iter()
            .zip(CheckId::ALL)
            .map(|(status, id)| CheckResult {
                id,
                findings: vec![Finding {
                    status: *status,
                    message: format!("{} {}", id.name(), status.label()),
                    fix: (*status != Status::Ok).then(|| "Faites ceci.".to_owned()),
                }],
            })
            .collect(),
        saved: Saved::No,
    }
}

// Exigence : EX-024-02
#[test]
fn the_exit_code_follows_the_worst_result() {
    assert_eq!(report(&[Status::Ok, Status::Ok]).exit_code(), 0);
    assert_eq!(report(&[Status::Ok, Status::Warning, Status::Ok]).exit_code(), 1);
    assert_eq!(report(&[Status::Warning, Status::Failure, Status::Ok]).exit_code(), 2);
    assert_eq!(report(&[Status::Failure]).exit_code(), 2);
    let mut unsaved = report(&[Status::Ok]);
    unsaved.saved = Saved::Failed("accès refusé".to_owned());
    assert_eq!(
        unsaved.exit_code(),
        2,
        "a report that could not be written is a failure"
    );
    assert_eq!(
        report(&[Status::Warning, Status::Warning, Status::Failure]).summary(),
        "2 avertissements, 1 échec"
    );
    assert_eq!(report(&[Status::Ok]).summary(), "tout va bien");
}

// Exigence : EX-024-02
#[tokio::test(start_paused = true)]
async fn a_check_that_never_ends_is_a_failure_within_the_bound() {
    let checks: Vec<Box<dyn Check>> = vec![
        Box::new(AppCheck {
            probe: Arc::new(Silent),
            viewpoint: Viewpoint::Command,
            installed: None,
        }),
        Box::new(UpdateKeyCheck { slots: [None, None] }),
    ];
    let started = tokio::time::Instant::now();
    let results = run(
        checks,
        Some(ClockCheck {
            dates: Dates::default(),
        }),
    )
    .await;
    assert!(started.elapsed() <= CHECK_TIMEOUT + Duration::from_secs(1));
    let ids: Vec<CheckId> = results.iter().map(|r| r.id).collect();
    assert_eq!(ids, [CheckId::App, CheckId::UpdateKey, CheckId::Clock]);
    assert_eq!(results[0].status(), Status::Failure);
    assert!(results[0].findings[0].message.contains("pas de réponse en 12 s"));
    assert_eq!(results[1].status(), Status::Warning);
    let report = Report {
        checks: results,
        ..report(&[])
    };
    assert_eq!(report.exit_code(), 2);
}

#[test]
fn the_text_and_the_structure_say_the_same() {
    let mut report = report(&[Status::Ok, Status::Warning, Status::Failure]);
    report.saved = Saved::At(r"%LOCALAPPDATA%\MCP Proxy\logs\doctor-20261123-100000.txt".to_owned());
    let text = report.text();
    assert!(text.starts_with("MCP Proxy — auto-diagnostic\nVersion 0.3.0, 2026-11-23T10:00:00.000Z (UTC)"));
    assert!(text.contains("Résultat : 1 avertissement, 1 échec."));
    assert!(text.contains("[avertissement] WebView2\n  ! WebView2 avertissement\n    À faire : Faites ceci.\n"));
    assert!(text.contains("[échec] Installation\n  × Installation échec\n"));
    assert!(text.ends_with("Rapport enregistré dans %LOCALAPPDATA%\\MCP Proxy\\logs\\doctor-20261123-100000.txt\n"));
    let json = report.to_json();
    assert_eq!(json["status"], "failure");
    assert_eq!(json["checks"][1]["id"], "webview");
    assert_eq!(json["checks"][1]["findings"][0]["fix"], "Faites ceci.");
    assert_eq!(json["text"], text);
    assert_eq!(
        json["file"],
        r"%LOCALAPPDATA%\MCP Proxy\logs\doctor-20261123-100000.txt"
    );
    // The same advice is said once in a check.
    let mut repeated = self::report(&[Status::Warning]);
    let again = repeated.checks[0].findings[0].clone();
    repeated.checks[0].findings.push(again);
    assert_eq!(repeated.text().matches("À faire : Faites ceci.").count(), 1);
    assert_eq!(repeated.to_json()["checks"][0]["findings"][1]["fix"], "Faites ceci.");
    let names: Vec<&str> = CheckId::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(
        names,
        [
            "Config",
            "WebView2",
            "Installation",
            "Notifications",
            "Appli",
            "Réglages",
            "Artifactory",
            "Clé de mise à jour",
            "Clients LLM",
            "MCP",
            "Horloge"
        ]
    );
    let _ = own_version();
}

// ---------------------------------------------------------------------------------------------------- output

/// Files in memory: the first names may exist already, a write may fail.
#[derive(Default)]
struct FakeFiles {
    files: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    fail: Option<io::ErrorKind>,
    fail_dir: bool,
}

impl ReportFiles for FakeFiles {
    fn create_dir_all(&self, _dir: &Path) -> io::Result<()> {
        if self.fail_dir {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    }

    fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(kind) = self.fail {
            return Err(io::Error::from(kind));
        }
        let mut files = self.files.lock().unwrap();
        if files.contains_key(path) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        files.insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }
}

const LOGS: &str = "/u/Local/MCP Proxy/logs";
const LABEL: &str = r"%LOCALAPPDATA%\MCP Proxy\logs";

fn identity(text: &str) -> Option<String> {
    Some(text.to_owned())
}

// Exigence : EX-024-08
#[test]
fn the_report_goes_to_a_new_file_of_the_logs_and_to_the_standard_output() {
    let files = FakeFiles::default();
    let mut stdout = Vec::new();
    let published = publish(
        report(&[Status::Ok, Status::Warning]),
        Path::new(LOGS),
        LABEL,
        "20261123-100000",
        &files,
        &mut stdout,
        &identity,
    );
    let path = Path::new(LOGS).join("doctor-20261123-100000.txt");
    assert_eq!(published.file.as_deref(), Some(path.as_path()));
    let written = String::from_utf8(files.files.lock().unwrap()[&path].clone()).unwrap();
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        written,
        "the same text on the standard output"
    );
    assert!(written.ends_with("Rapport enregistré dans %LOCALAPPDATA%\\MCP Proxy\\logs\\doctor-20261123-100000.txt\n"));
    assert_eq!(published.text, written);
    assert_eq!(published.exit_code, 1);

    // The same second again: a new file, never the previous one replaced.
    let mut stdout = Vec::new();
    let again = publish(
        report(&[Status::Ok]),
        Path::new(LOGS),
        LABEL,
        "20261123-100000",
        &files,
        &mut stdout,
        &identity,
    );
    let second = Path::new(LOGS).join("doctor-20261123-100000_2.txt");
    assert_eq!(again.file.as_deref(), Some(second.as_path()));
    assert!(
        String::from_utf8(stdout)
            .unwrap()
            .ends_with("doctor-20261123-100000_2.txt\n")
    );
    assert_eq!(files.files.lock().unwrap().len(), 2);
    assert_eq!(again.exit_code, 0);
    assert!(report_name("x", 2) > report_name("x", 1), "the latest sorts last");
}

// Exigence : EX-024-08
#[test]
fn a_report_that_cannot_be_written_is_a_failure_on_the_standard_output() {
    for files in [
        FakeFiles {
            fail: Some(io::ErrorKind::PermissionDenied),
            ..FakeFiles::default()
        },
        FakeFiles {
            fail_dir: true,
            ..FakeFiles::default()
        },
    ] {
        let mut stdout = Vec::new();
        let published = publish(
            report(&[Status::Ok]),
            Path::new(LOGS),
            LABEL,
            "20261123-100000",
            &files,
            &mut stdout,
            &identity,
        );
        let out = String::from_utf8(stdout).unwrap();
        assert!(out.contains("[ok] Config"), "the report is still given: {out}");
        assert!(
            out.ends_with("[échec] Rapport non enregistré dans le dossier des logs (accès refusé).\n"),
            "{out}"
        );
        assert_eq!(published.exit_code, 2);
        assert_eq!(published.file, None);
        assert!(files.files.lock().unwrap().is_empty());
    }
}

// Exigence : EX-024-04
#[test]
fn nothing_leaves_without_the_filter() {
    let files = FakeFiles::default();
    let mut stdout = Vec::new();
    let published = publish(
        report(&[Status::Ok]),
        Path::new(LOGS),
        LABEL,
        "20261123-100000",
        &files,
        &mut stdout,
        &|_| None,
    );
    assert!(files.files.lock().unwrap().is_empty(), "no file without the filter");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "MCP Proxy — auto-diagnostic\n[échec] Filtre de sortie indisponible : aucun rapport n'est écrit.\n"
    );
    assert_eq!(published.exit_code, 2);

    let masked = |text: &str| Some(text.replace("Config", "[SECRET MASQUÉ]"));
    let filtered = report(&[Status::Warning]).filtered(&masked).unwrap();
    assert_eq!(filtered.checks[0].findings[0].message, "[SECRET MASQUÉ] avertissement");
    assert!(report(&[Status::Warning]).filtered(&|_| None).is_none());
}

#[test]
fn the_installed_paths_used_by_the_tests_are_those_of_the_installation() {
    assert_eq!(paths().exe, Path::new("/u/Local/Programs/MCP Proxy/mcp-proxy.exe"));
}
