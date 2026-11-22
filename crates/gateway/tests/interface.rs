//! What the main window asks of the gateway (spec 016), on the assembly the application runs: log in and
//! disconnect from a card, choose the profile, resume, show the waiting confirmations, and the client that waits for
//! a login.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::config::{Decision, EnvId, McpId, Rule, ToolName};
use mcp_proxy_core::secrets::{SecretStore, platform_key_wrapper};
use mcp_proxy_core::trace::{EntryKind, JournalFilter, SystemWallClock};
use mcp_proxy_gateway::bridge::{PeerProof, ProtectFailure, Role, prove};
use mcp_proxy_gateway::control::GatewayControl;
use mcp_proxy_gateway::interface::InterfaceError;
use mcp_proxy_gateway::notify::{
    ConfirmationId, ConfirmationPresenter, DetailView, PresentError, ToastContent, ToastState,
};
use mcp_proxy_gateway::runtime::{DataDirs, Gateway, Logging, Persistence, RuntimeDeps};
use mcp_proxy_gateway::view::{AlertKind, AppStatus, SessionStateKind};
use mcp_proxy_testkit::FakeAuthUi;
use rmcp::ServiceExt;
use rmcp::service::{RoleClient, RunningService};
use serde_json::json;
use zeroize::Zeroizing;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-interface-{}-{}",
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

/// Toasts off: every confirmation opens its detail window, which nobody answers. Counts the windows opened.
#[derive(Default)]
struct DetailWindows {
    opened: Mutex<Vec<ConfirmationId>>,
}

#[async_trait]
impl ConfirmationPresenter for DetailWindows {
    async fn toast_state(&self) -> ToastState {
        ToastState::Unavailable("mode « Ne pas déranger »".to_owned())
    }

    async fn show_toast(&self, _id: ConfirmationId, _toast: ToastContent) -> Result<(), PresentError> {
        Err(PresentError("pas de toast".to_owned()))
    }

    async fn open_detail(&self, id: ConfirmationId, _view: DetailView) -> Result<(), PresentError> {
        self.opened.lock().unwrap().push(id);
        Ok(())
    }

    fn withdraw(&self, _id: ConfirmationId) {}
}

struct Started {
    gateway: Arc<Gateway>,
    ui: FakeAuthUi,
    windows: Arc<DetailWindows>,
}

async fn start(dir: &TempDir) -> Started {
    let dirs = DataDirs::under(&dir.0);
    let ui = FakeAuthUi::default();
    let windows = Arc::new(DetailWindows::default());
    let gateway = Gateway::start(RuntimeDeps {
        config: Arc::new(mcp_proxy_gateway::modules::embedded_config().unwrap()),
        persistence: Persistence::sealed(&dirs, platform_key_wrapper()),
        dirs,
        store: SecretStore::for_platform().unwrap(),
        auth_ui: Arc::new(ui.clone()),
        presenter: windows.clone(),
        clock: Arc::new(SystemWallClock),
        logging: Logging::Local,
        utc_offset: 0,
    })
    .await
    .unwrap();
    Started {
        gateway: Arc::new(gateway),
        ui,
        windows,
    }
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

async fn until(mut ready: impl FnMut() -> bool) {
    for _ in 0..500 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition never met");
}

struct SameUser;

impl PeerProof for SameUser {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure> {
        Ok([b"U", data].concat())
    }

    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure> {
        data.strip_prefix(b"U")
            .map(|rest| Zeroizing::new(rest.to_vec()))
            .ok_or(ProtectFailure(-1))
    }
}

async fn client_of(gateway: &Arc<Gateway>) -> RunningService<RoleClient, ()> {
    let (mut bridge_end, app_end) = tokio::io::duplex(1 << 20);
    let served = Arc::clone(gateway);
    tokio::spawn(async move { served.serve_connection(Arc::new(SameUser), app_end).await });
    prove(Arc::new(SameUser), Role::Bridge, &mut bridge_end).await.unwrap();
    ().serve(bridge_end).await.unwrap()
}

fn call(client: &Arc<RunningService<RoleClient, ()>>, name: &str) -> tokio::task::JoinHandle<String> {
    let client = Arc::clone(client);
    let name = name.to_owned();
    tokio::spawn(async move {
        let mut params = rmcp::model::CallToolRequestParams::new(name);
        params.arguments = json!({ "jql": "project = DEMO", "key": "DEMO-1", "body": "texte" })
            .as_object()
            .cloned();
        serde_json::to_string(&client.call_tool(params).await.unwrap().content).unwrap()
    })
}

// Exigence : EX-016-07
#[tokio::test(flavor = "multi_thread")]
async fn a_card_logs_in_then_disconnects_its_session() {
    let dir = TempDir::new();
    let s = start(&dir).await;
    let gateway = &s.gateway;
    let developer = gateway.known_profile("developpeur").unwrap();
    gateway.set_profile(&developer).await.unwrap();

    assert_eq!(gateway.known_session("inconnue"), Err(InterfaceError::UnknownSession));
    assert_eq!(gateway.known_session("../sso-dev"), Err(InterfaceError::UnknownSession));
    let dev = gateway.known_session("sso-dev").unwrap();

    let connecting = {
        let gateway = Arc::clone(gateway);
        let dev = dev.clone();
        tokio::spawn(async move { gateway.connect_session(&dev).await })
    };
    let window = s.ui.window(0).await;
    let requested = gateway.snapshot().await;
    let card = requested.sessions.iter().find(|c| c.id == "sso-dev").unwrap();
    assert_eq!(card.state.kind, SessionStateKind::LoginRequested);
    assert_eq!(card.state.requested_by, None, "asked by the user, no LLM client waits");
    assert!(
        s.ui.notifications().is_empty(),
        "no notification for the user's own request"
    );

    s.ui.set_cookie("SESSION", "cookie-dev", None);
    window.navigate("https://apps.dev.example.internal/home");
    connecting.await.unwrap().unwrap();
    let active = gateway.snapshot().await;
    let card = active.sessions.iter().find(|c| c.id == "sso-dev").unwrap();
    assert_eq!(card.state.kind, SessionStateKind::Active);
    assert!(card.state.at.is_some());
    assert!(card.state.label.starts_with("active jusqu'à "), "{}", card.state.label);

    gateway.disconnect_session(&dev).await;
    let expired = gateway.snapshot().await;
    let card = expired.sessions.iter().find(|c| c.id == "sso-dev").unwrap();
    assert_eq!(card.state.kind, SessionStateKind::Expired);
    assert_eq!(card.state.label, "expirée");
    assert_eq!(card.state.at, None);
    assert_eq!(
        s.ui.cleared(),
        [vec![
            "sso.dev.example.internal".to_owned(),
            "apps.dev.example.internal".to_owned()
        ]]
    );
}

// Exigence : EX-016-06
#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_names_the_client_waiting_for_a_login() {
    let dir = TempDir::new();
    let s = start(&dir).await;
    let gateway = &s.gateway;
    gateway
        .set_profile(&gateway.known_profile("developpeur").unwrap())
        .await
        .unwrap();
    let snapshot = gateway.snapshot().await;
    let developer = snapshot.profile.choices.iter().find(|p| p.id == "developpeur").unwrap();
    assert!(!developer.description.is_empty(), "one sentence per profile");

    // A search in REC needs « SSO Outils »: its Basic input waits for the user.
    let client = Arc::new(client_of(gateway).await);
    let running = call(&client, "rec_jira_search_issues");
    s.ui.wait_prompts(1).await;
    let waiting = gateway.snapshot().await;
    let tools = waiting.sessions.iter().find(|c| c.id == "sso-tools").unwrap();
    assert_eq!(tools.state.kind, SessionStateKind::LoginRequested);
    let client_name = tools.state.requested_by.clone().unwrap();
    assert_eq!(client_name, "rmcp", "clientInfo.name of the client");
    assert_eq!(waiting.status, AppStatus::Attention);
    let others = waiting.sessions.iter().filter(|c| c.id != "sso-tools");
    assert!(others.into_iter().all(|c| c.state.requested_by.is_none()));

    s.ui.answer_basic(None);
    let answer = running.await.unwrap();
    assert!(answer.contains("SSO Outils"), "{answer}");
    let after = gateway.snapshot().await;
    let tools = after.sessions.iter().find(|c| c.id == "sso-tools").unwrap();
    assert_eq!(tools.state.kind, SessionStateKind::NeverConnected);
    assert_eq!(tools.state.requested_by, None);
}

fn harden_dev(settings: &mut mcp_proxy_core::settings::UserSettings) {
    settings
        .hardening
        .env
        .entry(EnvId::new("dev").unwrap())
        .or_default()
        .mcp
        .entry(McpId::new("kibana-logs").unwrap())
        .or_default()
        .tools
        .insert(
            ToolName::new("search_logs").unwrap(),
            Rule {
                decision: Decision::Deny,
                locked: false,
            },
        );
}

// Exigence : EX-016-07
#[tokio::test(flavor = "multi_thread")]
async fn choosing_a_profile_resets_the_exposed_environments_and_keeps_the_hardening() {
    let dir = TempDir::new();
    // A file written by hand: settings reset, banner, the profile must be chosen again.
    let settings = DataDirs::under(&dir.0).settings;
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(settings, b"{ pas du json").unwrap();
    let s = start(&dir).await;
    let gateway = &s.gateway;
    let reset = gateway.snapshot().await;
    assert!(reset.profile.first_launch);
    assert_eq!(reset.alerts[0].kind, AlertKind::SettingsReset);

    assert_eq!(gateway.known_profile("admin"), Err(InterfaceError::UnknownProfile));
    let support = gateway.known_profile("support").unwrap();
    gateway.set_profile(&support).await.unwrap();
    let chosen = gateway.snapshot().await;
    assert_eq!(chosen.profile.chosen.as_deref(), Some("support"));
    assert!(!chosen.profile.first_launch);
    assert!(
        chosen.alerts.iter().all(|a| a.kind != AlertKind::SettingsReset),
        "banner gone"
    );
    let exposed: Vec<&str> = chosen
        .environments
        .iter()
        .filter(|e| e.exposed)
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(exposed, ["rec", "pprod", "prod"]);

    // The user exposes one more environment and hardens a rule, then changes profile.
    gateway
        .change_settings(|settings| {
            settings.exposed_environments.insert(EnvId::new("qua").unwrap());
            harden_dev(settings);
        })
        .await
        .unwrap();
    gateway
        .set_profile(&gateway.known_profile("developpeur").unwrap())
        .await
        .unwrap();
    let changed = gateway.snapshot().await;
    let exposed: Vec<&str> = changed
        .environments
        .iter()
        .filter(|e| e.exposed)
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(exposed, ["dev", "int", "rec"], "back to the defaults of the profile");
    let mut kept = mcp_proxy_core::settings::UserSettings::default();
    harden_dev(&mut kept);
    assert_eq!(gateway.settings().hardening, kept.hardening, "hardening kept");
    assert_eq!(
        events(gateway)
            .into_iter()
            .filter(|e| e.starts_with("profil"))
            .collect::<Vec<_>>(),
        [
            "profil « Support » choisi par l'utilisateur",
            "profil « Développeur » choisi par l'utilisateur"
        ]
    );
    drop(s);

    // Sealed: the choice survives a restart.
    let s = start(&dir).await;
    assert_eq!(
        s.gateway.snapshot().await.profile.chosen.as_deref(),
        Some("developpeur")
    );
}

// Exigence : EX-016-07
#[tokio::test(flavor = "multi_thread")]
async fn see_opens_the_detail_windows_and_answers_nothing_and_resume_lifts_the_suspension() {
    let dir = TempDir::new();
    let s = start(&dir).await;
    let gateway = &s.gateway;
    gateway
        .set_profile(&gateway.known_profile("support").unwrap())
        .await
        .unwrap();
    // Nothing waits: « Voir » opens nothing.
    gateway.show_confirmations().await;
    assert!(s.windows.opened.lock().unwrap().is_empty());

    // A write in PROD waits for its confirmation, asked before any login.
    let client = Arc::new(client_of(gateway).await);
    let running = call(&client, "prod_jira_add_comment");
    let pending = Arc::clone(gateway.confirmations().pending());
    until(|| pending.count() == 1).await;
    let id = pending.waiting()[0];
    assert_eq!(*s.windows.opened.lock().unwrap(), [id], "opened once when asked");

    gateway.show_confirmations().await;
    assert_eq!(*s.windows.opened.lock().unwrap(), [id, id], "« Voir » opens it again");
    assert_eq!(pending.count(), 1, "nothing approved nor refused");
    assert_eq!(gateway.snapshot().await.pending_confirmations, 1);

    // Suspending refuses it; « Reprendre » lifts the suspension.
    gateway.suspend().await.unwrap();
    let refused = running.await.unwrap();
    assert!(refused.contains("passerelle suspendue"), "{refused}");
    assert!(gateway.snapshot().await.gateway_suspended);
    gateway.resume().await.unwrap();
    let resumed = gateway.snapshot().await;
    assert!(!resumed.gateway_suspended);
    assert_eq!(resumed.pending_confirmations, 0);
}
