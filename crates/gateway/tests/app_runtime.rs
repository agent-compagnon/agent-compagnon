//! The assembly the application runs at start (spec 011), under Linux: embedded configuration, real call chain,
//! on-demand authenticator with the provisional interface, settings and catalogue cache in a temporary folder, a
//! client proven on an in-memory pipe.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mcp_proxy_auth::UnavailableAuthUi;
use mcp_proxy_core::config::{Config, EnvId, ProfileId, SessionId};
use mcp_proxy_core::secrets::{Credential, SecretStore, platform_key_wrapper};
use mcp_proxy_core::trace::{EntryKind, JournalFilter, SystemWallClock};
use mcp_proxy_gateway::bridge::{PeerProof, ProtectFailure, Role, prove};
use mcp_proxy_gateway::notify::{
    ConfirmationId, ConfirmationPresenter, DetailView, PresentError, ToastContent, ToastState, UnavailablePresenter,
};
use mcp_proxy_gateway::runtime::{DataDirs, Gateway, Logging, Persistence, RuntimeDeps};
use mcp_proxy_gateway::view::{AlertKind, AppStatus};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use secrecy::SecretString;
use serde_json::{Value, json};
use zeroize::Zeroizing;

const SENTINEL: &str = "SENTINELLE-secrete-7f3a91c2d4e5b6a8";

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

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-app-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config() -> Arc<Config> {
    Arc::new(mcp_proxy_gateway::modules::embedded_config().unwrap())
}

async fn start(dir: &TempDir, store: SecretStore) -> Gateway {
    start_with(dir, store, Arc::new(UnavailablePresenter)).await
}

async fn start_with(dir: &TempDir, store: SecretStore, presenter: Arc<dyn ConfirmationPresenter>) -> Gateway {
    let dirs = DataDirs::under(&dir.0);
    Gateway::start(RuntimeDeps {
        config: config(),
        persistence: Persistence::sealed(&dirs, platform_key_wrapper()),
        dirs,
        store,
        auth_ui: Arc::new(UnavailableAuthUi),
        presenter,
        clock: Arc::new(SystemWallClock),
        logging: Logging::Local,
        utc_offset: 0,
    })
    .await
    .unwrap()
}

fn store_with_sentinel() -> SecretStore {
    let store = SecretStore::for_platform().unwrap();
    store
        .put(
            &SessionId::new("sso-corp").unwrap(),
            Credential::Bearer {
                token: SecretString::from(SENTINEL),
            },
        )
        .unwrap();
    store
}

async fn choose_developer(gateway: &Gateway, extra: &[&str]) {
    let config = gateway.config().clone();
    gateway
        .change_settings(|s| {
            s.choose_profile(&config.profiles[&ProfileId::new("developpeur").unwrap()]);
            for env in extra {
                s.exposed_environments.insert(EnvId::new(*env).unwrap());
            }
        })
        .await
        .unwrap();
}

fn names(value: &Value, path: &str) -> Vec<String> {
    value
        .pointer(path)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect()
}

// Exigence : EX-011-02
#[tokio::test]
async fn snapshot_of_the_example_configuration_without_any_secret() {
    let dir = TempDir::new();
    let gateway = start(&dir, store_with_sentinel()).await;

    let first = gateway.snapshot_json().await.unwrap();
    assert_eq!(first["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(first["status"], "neutral");
    assert_eq!(first["tooltip"], "MCP Proxy — prêt");
    assert_eq!(first["profile"]["firstLaunch"], true);
    assert_eq!(first["profile"]["chosen"], Value::Null);
    assert_eq!(first["profile"]["active"], "lecture-seule");
    assert_eq!(
        names(&first, "/profile/choices"),
        ["developpeur", "lecture-seule", "support"]
    );
    assert_eq!(
        names(&first, "/environments"),
        ["dev", "int", "qua", "rec", "perf", "pprod", "prod", "pra"]
    );
    assert_eq!(first["environments"][6]["sensitive"], true);
    assert!(
        first["environments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["exposed"] == false)
    );
    assert_eq!(
        names(&first, "/sessions"),
        [
            "sso-corp",
            "sso-tools",
            "sso-dev",
            "sso-int",
            "sso-qua",
            "sso-rec",
            "sso-perf",
            "sso-pprod",
            "sso-prod",
            "sso-pra"
        ]
    );
    let corp = &first["sessions"][0];
    assert_eq!(corp["environment"], Value::Null);
    assert_eq!(corp["method"], "jwtHeader");
    assert_eq!(corp["state"]["kind"], "neverConnected");
    assert_eq!(corp["state"]["label"], "jamais connectée");
    assert_eq!(first["sessions"][2]["environment"], "dev");
    assert_eq!(
        names(&first, "/mcps"),
        ["kibana-logs", "gitlab-ops", "k8s-admin", "db-readonly", "jira"]
    );
    let jira = &first["mcps"][4];
    assert_eq!(jira["local"], true);
    assert_eq!(jira["instances"].as_array().unwrap().len(), 2);
    assert_eq!(jira["instances"][0]["environment"], "rec");
    assert_eq!(jira["instances"][0]["source"], "embedded");
    let tools: Vec<&str> = jira["instances"][0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["exposedName"].as_str().unwrap())
        .collect();
    assert_eq!(
        tools,
        ["rec_jira_add_comment", "rec_jira_get_issue", "rec_jira_search_issues"]
    );
    assert_eq!(jira["instances"][0]["tools"][0]["category"], "write");
    assert_eq!(jira["instances"][0]["tools"][0]["exposed"], false);
    assert_eq!(first["exposedTools"], 0);
    assert_eq!(first["exposedToolsThreshold"], 100);
    assert_eq!(first["pendingConfirmations"], 0);
    assert_eq!(first["alerts"], json!([]));
    assert_eq!(first["update"], Value::Null);

    choose_developer(&gateway, &[]).await;
    let chosen = gateway.snapshot_json().await.unwrap();
    assert_eq!(chosen["profile"]["chosen"], "developpeur");
    assert_eq!(chosen["profile"]["firstLaunch"], false);
    let exposed: Vec<&str> = chosen["environments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["exposed"] == true)
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(exposed, ["dev", "int", "rec"]);
    assert!(chosen["exposedTools"].as_u64().unwrap() > 0);

    // Neither the filtered snapshot nor the raw one carries the secret or a key that names one.
    for text in [
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&chosen).unwrap(),
        serde_json::to_string(&gateway.snapshot().await).unwrap(),
    ] {
        assert!(!text.contains(SENTINEL), "secret in the snapshot");
        for key in ["password", "token", "secret", "value", "cookie", "credential"] {
            assert!(!text.contains(&format!("\"{key}\":")), "key {key} in the snapshot");
        }
    }
}

// Exigence : EX-011-02
#[tokio::test]
async fn settings_are_saved_then_reloaded_and_a_reset_is_an_error() {
    let dir = TempDir::new();
    {
        let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
        choose_developer(&gateway, &["prod"]).await;
    }
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    let snapshot = gateway.snapshot().await;
    assert_eq!(snapshot.profile.chosen.as_deref(), Some("developpeur"));
    assert!(snapshot.environments.iter().any(|e| e.id == "prod" && e.exposed));
    drop(gateway);

    // The settings are sealed (spec 014): a file written by hand, even valid, is refused.
    std::fs::write(
        DataDirs::under(&dir.0).settings,
        br#"{"schema_version":1,"profile":"developpeur","exposed_environments":["dev","int","rec"],"disabled_mcp":[],"hardening":{},"autostart":false,"beta_channel":false,"gateway_suspended":false}"#,
    )
    .unwrap();
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    let snapshot = gateway.snapshot().await;
    assert_eq!(snapshot.profile.chosen, None);
    assert_eq!(snapshot.alerts[0].kind, AlertKind::SettingsReset);
    drop(gateway);

    std::fs::write(DataDirs::under(&dir.0).settings, b"{ pas du json").unwrap();
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    let snapshot = gateway.snapshot().await;
    assert_eq!(snapshot.profile.chosen, None);
    assert_eq!(snapshot.alerts.len(), 1);
    assert_eq!(snapshot.alerts[0].kind, AlertKind::SettingsReset);
    assert_eq!(snapshot.status, AppStatus::Error);
    assert_eq!(snapshot.tooltip, "MCP Proxy — 1 erreur");
}

async fn client_on_proven_pipe(
    gateway: Arc<Gateway>,
    client: u8,
) -> (
    Result<rmcp::service::RunningService<rmcp::RoleClient, ()>, ()>,
    tokio::task::JoinHandle<Result<(), mcp_proxy_gateway::bridge::ProofError>>,
) {
    let (mut bridge_end, app_end) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(async move { gateway.serve_connection(Arc::new(FakeUser(1)), app_end).await });
    let proven = prove(Arc::new(FakeUser(client)), Role::Bridge, &mut bridge_end).await;
    let client = match proven {
        Ok(()) => Ok(().serve(bridge_end).await.unwrap()),
        Err(_) => Err(()),
    };
    (client, server)
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    serde_json::to_string(&result.content).unwrap()
}

async fn call(client: &rmcp::service::RunningService<rmcp::RoleClient, ()>, name: &str) -> String {
    let mut params = CallToolRequestParams::new(name.to_owned());
    params.arguments = json!({ "jql": "project = DEMO", "key": "DEMO-1", "body": "texte" })
        .as_object()
        .cloned();
    let result = client.call_tool(params).await.unwrap();
    assert_eq!(result.is_error, Some(true));
    text_of(&result)
}

// Exigence : EX-011-13
// Exigence : EX-011-16
#[tokio::test]
async fn a_proven_client_lists_the_tools_and_the_provisional_interface_fails_closed() {
    let dir = TempDir::new();
    let gateway = Arc::new(start(&dir, SecretStore::for_platform().unwrap()).await);
    choose_developer(&gateway, &["prod"]).await;

    let (client, server) = client_on_proven_pipe(Arc::clone(&gateway), 1).await;
    let client = client.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let listed: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(listed.contains(&"rec_jira_search_issues"), "{listed:?}");
    assert!(listed.contains(&"prod_jira_add_comment"), "{listed:?}");
    assert_eq!(listed.len(), gateway.snapshot().await.exposed_tools);

    // A read call needs `sso-tools`: no login window yet, so no login at all.
    let refused = call(&client, "rec_jira_search_issues").await;
    assert!(
        refused.contains("connexion à « SSO Outils » impossible pour le moment, appel refusé."),
        "{refused}"
    );
    let snapshot = gateway.snapshot().await;
    let tools_session = snapshot.sessions.iter().find(|s| s.id == "sso-tools").unwrap();
    assert_eq!(tools_session.state.label, "jamais connectée");
    assert_eq!(snapshot.status, AppStatus::Neutral);

    // A write in PROD asks for a confirmation by notification: no channel yet, so it is refused.
    let refused = call(&client, "prod_jira_add_comment").await;
    assert!(
        refused.contains("confirmation impossible, aucun moyen de la demander à l'utilisateur ; appel refusé."),
        "{refused}"
    );

    client.cancel().await.unwrap();
    assert_eq!(server.await.unwrap(), Ok(()));
}

// Exigence : EX-011-16
#[tokio::test]
async fn a_client_of_another_user_is_closed_before_any_mcp_byte() {
    let dir = TempDir::new();
    let gateway = Arc::new(start(&dir, SecretStore::for_platform().unwrap()).await);
    let (client, server) = client_on_proven_pipe(Arc::clone(&gateway), 2).await;
    assert!(client.is_err());
    assert!(server.await.unwrap().is_err());
}

/// Toasts off: every confirmation opens its detail window, recorded here.
#[derive(Default)]
struct WindowsOnly {
    opened: Mutex<Vec<DetailView>>,
    withdrawn: Mutex<Vec<ConfirmationId>>,
}

#[async_trait]
impl ConfirmationPresenter for WindowsOnly {
    async fn toast_state(&self) -> ToastState {
        ToastState::Unavailable("mode « Ne pas déranger » (prioritaires seulement)".to_owned())
    }

    async fn show_toast(&self, _id: ConfirmationId, _toast: ToastContent) -> Result<(), PresentError> {
        panic!("no toast while toasts are off");
    }

    async fn open_detail(&self, _id: ConfirmationId, view: DetailView) -> Result<(), PresentError> {
        self.opened.lock().unwrap().push(view);
        Ok(())
    }

    fn withdraw(&self, id: ConfirmationId) {
        self.withdrawn.lock().unwrap().push(id);
    }
}

// Exigence : EX-013-13
#[tokio::test]
async fn a_waiting_confirmation_shows_in_the_snapshot_until_answered() {
    let dir = TempDir::new();
    let presenter = Arc::new(WindowsOnly::default());
    let gateway = Arc::new(start_with(&dir, store_with_sentinel(), presenter.clone()).await);
    choose_developer(&gateway, &["prod"]).await;
    let mut changes = gateway.changes();
    let (client, server) = client_on_proven_pipe(Arc::clone(&gateway), 1).await;
    let client = Arc::new(client.unwrap());

    // A write in PROD asks for a confirmation by notification (a known secret in the arguments would be blocked
    // before: the secrets of a confirmation are tested with the broker).
    let running = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            let mut params = CallToolRequestParams::new("prod_jira_add_comment".to_owned());
            params.arguments = json!({ "key": "DEMO-1", "body": "texte" }).as_object().cloned();
            text_of(&client.call_tool(params).await.unwrap())
        })
    };
    let pending = Arc::clone(gateway.confirmations().pending());
    while pending.count() == 0 {
        assert!(!running.is_finished(), "{}", running.await.unwrap());
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    while changes.try_recv().is_ok() {}
    let snapshot = gateway.snapshot_json().await.unwrap();
    assert_eq!(snapshot["pendingConfirmations"], 1);
    assert_eq!(snapshot["status"], "attention");
    assert_eq!(snapshot["tooltip"], "MCP Proxy — 1 confirmation attend");

    let view = presenter.opened.lock().unwrap()[0].clone();
    assert_eq!(view.path, "PROD / jira / add_comment");
    assert!(view.arguments.contains("DEMO-1"));

    // Closing the window refuses; the icon goes back to neutral and a change is signalled.
    pending.detail_closed(view.id);
    let refused = running.await.unwrap();
    assert!(refused.contains("appel refusé par l'utilisateur."), "{refused}");
    assert_eq!(pending.count(), 0);
    assert_eq!(*presenter.withdrawn.lock().unwrap(), [view.id]);
    let snapshot = gateway.snapshot().await;
    assert_eq!(snapshot.pending_confirmations, 0);
    assert_eq!(snapshot.status, AppStatus::Neutral);
    tokio::time::timeout(std::time::Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();

    Arc::try_unwrap(client).ok().unwrap().cancel().await.unwrap();
    assert_eq!(server.await.unwrap(), Ok(()));
}

/// The softest settings: developer profile, every environment exposed.
async fn expose_everything(gateway: &Gateway) {
    let config = gateway.config().clone();
    gateway
        .change_settings(|s| {
            s.choose_profile(&config.profiles[&ProfileId::new("developpeur").unwrap()]);
            s.exposed_environments = config.environments.iter().map(|e| e.id.clone()).collect();
            s.autostart = true;
        })
        .await
        .unwrap();
}

/// The settings file without its seal line, as a previous version wrote it.
fn without_seal(sealed: &[u8]) -> Vec<u8> {
    sealed[sealed.iter().position(|b| *b == b'\n').unwrap() + 1..].to_vec()
}

// Exigence : EX-014-01, EX-014-03
#[tokio::test]
async fn sealed_settings_survive_a_restart_and_anything_else_falls_back_to_the_strictest() {
    let dir = TempDir::new();
    let dirs = DataDirs::under(&dir.0);
    {
        let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
        expose_everything(&gateway).await;
    }
    let sealed = std::fs::read(&dirs.settings).unwrap();
    assert!(sealed.starts_with(b"MCP-PROXY-SEAL v1 hmac-sha256="));
    assert!(dirs.settings_key.exists());
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    let snapshot = gateway.snapshot().await;
    assert_eq!(snapshot.profile.chosen.as_deref(), Some("developpeur"));
    assert!(!snapshot.alerts.iter().any(|a| a.kind == AlertKind::SettingsReset));
    drop(gateway);

    let mut profile_edited = sealed.clone();
    let at = profile_edited.windows(11).position(|w| w == b"developpeur").unwrap();
    profile_edited[at..at + 7].copy_from_slice(b"support");
    type Damage = Box<dyn Fn()>;
    let cases: Vec<(&str, Damage)> = vec![
        ("one byte changed", {
            let path = dirs.settings.clone();
            let file = profile_edited.clone();
            Box::new(move || std::fs::write(&path, &file).unwrap())
        }),
        ("no seal (previous version)", {
            let path = dirs.settings.clone();
            let file = without_seal(&sealed);
            Box::new(move || std::fs::write(&path, &file).unwrap())
        }),
        ("key removed", {
            let key = dirs.settings_key.clone();
            let path = dirs.settings.clone();
            let file = sealed.clone();
            Box::new(move || {
                std::fs::write(&path, &file).unwrap();
                let _ = std::fs::remove_file(&key);
            })
        }),
    ];
    for (name, damage) in cases {
        damage();
        let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
        let snapshot = gateway.snapshot().await;
        assert_eq!(snapshot.profile.chosen, None, "{name}");
        assert_eq!(snapshot.profile.active.as_deref(), Some("lecture-seule"), "{name}");
        assert!(snapshot.environments.iter().all(|e| !e.exposed), "{name}");
        assert_eq!(snapshot.exposed_tools, 0, "{name}");
        assert!(!gateway.settings().autostart, "{name}");
        assert_eq!(snapshot.alerts.len(), 1, "{name}");
        assert_eq!(snapshot.alerts[0].kind, AlertKind::SettingsReset, "{name}");
        assert!(
            snapshot.alerts[0]
                .message
                .starts_with("Vos réglages n'ont pas pu être vérifiés"),
            "{name}: {}",
            snapshot.alerts[0].message
        );
        assert_eq!(snapshot.status, AppStatus::Error, "{name}");
        let events: Vec<String> = gateway
            .journal()
            .snapshot(&JournalFilter::default())
            .into_iter()
            .filter(|e| e.kind == EntryKind::Event)
            .map(|e| e.summary)
            .collect();
        assert_eq!(events.len(), 1, "{name}: {events:?}");
        assert!(
            events[0].starts_with("réglages remplacés par les plus stricts : réglages refusés"),
            "{events:?}"
        );

        // Choosing the profile again seals the file anew (with a new key if it was lost).
        expose_everything(&gateway).await;
        drop(gateway);
        let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
        let alerts = gateway.snapshot().await.alerts;
        assert!(!alerts.iter().any(|a| a.kind == AlertKind::SettingsReset), "{name}");
    }
}

// Exigence : EX-014-02
#[tokio::test]
async fn an_altered_catalogue_cache_is_ignored_for_the_embedded_snapshot() {
    use mcp_proxy_core::config::{EnvId, McpId};
    use mcp_proxy_core::sealed::Sealer;
    use mcp_proxy_gateway::registry::{
        CachedCatalog, CatalogCache, FileCatalogCache, InstanceId, SealedCatalog, ToolDefinition,
    };

    let dir = TempDir::new();
    let dirs = DataDirs::under(&dir.0);
    let instance = InstanceId::new(EnvId::new("rec").unwrap(), McpId::new("jira").unwrap());
    let filter = Arc::new(
        mcp_proxy_core::filter::OutputFilter::new(Arc::new(SecretStore::for_platform().unwrap()), &config().filter)
            .unwrap(),
    );
    let catalog = CachedCatalog {
        tools: vec![ToolDefinition {
            name: "search_issues".to_owned(),
            title: None,
            description: Some("Cache sealed by the application.".to_owned()),
            input_schema: json!({ "type": "object" }),
            output_schema: None,
            annotations: None,
        }],
    };
    {
        // The same key file as the application's.
        let sealer = Arc::new(Sealer::open(&dirs.settings_key, platform_key_wrapper()));
        let cache = FileCatalogCache::with_seal(&dirs.catalog_cache, filter, Box::new(SealedCatalog(sealer)));
        cache.store(&instance, &catalog).unwrap();
    }
    let path = dirs.catalog_cache.join("jira.rec.json");

    // Sealed by the application: used.
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    let snapshot = gateway.snapshot().await;
    let rec = &snapshot.mcps.iter().find(|m| m.id == "jira").unwrap().instances[0];
    assert_eq!(rec.source.as_deref(), Some("cache"));
    assert_eq!(rec.tools.len(), 1);
    drop(gateway);

    // Edited outside of it (one byte), or written without seal: ignored, back to the embedded snapshot.
    let sealed = std::fs::read(&path).unwrap();
    let mut edited = sealed.clone();
    let at = edited.windows(6).position(|w| w == b"sealed").unwrap();
    edited[at] = b'S';
    for file in [edited, without_seal(&sealed)] {
        std::fs::write(&path, &file).unwrap();
        let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
        let snapshot = gateway.snapshot().await;
        let rec = &snapshot.mcps.iter().find(|m| m.id == "jira").unwrap().instances[0];
        assert_eq!(rec.source.as_deref(), Some("embedded"));
        assert_eq!(rec.tools.len(), 3);
        assert!(
            snapshot
                .alerts
                .iter()
                .any(|a| a.kind == AlertKind::CatalogCache && a.message.contains("scellement invalide")),
            "{:?}",
            snapshot.alerts
        );
    }
}
