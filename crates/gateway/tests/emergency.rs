//! Emergency stop (spec 015): the suspension through the call chain, then « Suspendre », « Reprendre » and « Tout
//! déconnecter » through `GatewayControl` on the real assembly.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::{Stack, builder, connect, exposed, params, text_of};
use mcp_proxy_auth::UnavailableAuthUi;
use mcp_proxy_core::config::{Channel, Decision, EnvId, McpId, ProfileId, Rule, SessionId, ToolName};
use mcp_proxy_core::secrets::{Credential, SecretStore, platform_key_wrapper};
use mcp_proxy_core::session::SessionState;
use mcp_proxy_core::trace::{CallStatus, EntryKind, JournalFilter, SystemWallClock};
use mcp_proxy_gateway::bridge::{PeerProof, ProtectFailure, Role, prove};
use mcp_proxy_gateway::control::GatewayControl;
use mcp_proxy_gateway::notify::{
    ConfirmationId, ConfirmationPresenter, DetailView, PresentError, ToastContent, ToastState,
};
use mcp_proxy_gateway::runtime::{DataDirs, Gateway, Logging, Persistence, RuntimeDeps};
use mcp_proxy_gateway::view::AppStatus;
use mcp_proxy_testkit::Generation;
use rmcp::model::{
    CallToolResult, ClientCapabilities, ClientConfig, ElicitRequestParams, ElicitResult, Implementation,
};
use rmcp::service::{RequestContext, RoleClient, RunningService};
use rmcp::{ClientHandler, ErrorData, ServiceExt};
use secrecy::SecretString;
use serde_json::json;
use zeroize::Zeroizing;

const SUSPENDED: &str = "MCP Proxy : passerelle suspendue par l'utilisateur.";

/// An LLM client that announces elicitation and never answers it.
#[derive(Clone, Default)]
struct Silent;

impl ClientHandler for Silent {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::builder().enable_elicitation().build(),
            Implementation::new("client-de-test", "4.2.0"),
        )
    }

    async fn create_elicitation(
        &self,
        _: ElicitRequestParams,
        _: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        std::future::pending().await
    }
}

fn harden(settings: &mut mcp_proxy_core::settings::UserSettings, tool: &str, decision: Decision) {
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
            ToolName::new(tool).unwrap(),
            Rule {
                decision,
                locked: false,
            },
        );
}

fn only_text(result: &CallToolResult) -> String {
    let content = serde_json::to_value(&result.content).unwrap();
    content[0]["text"].as_str().unwrap().to_owned()
}

fn suspend(stack: &Stack) {
    stack.settings.send_modify(|s| s.gateway_suspended = true);
}

/// Waits (real time, bounded) until `ready` holds.
async fn until(mut ready: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ready() {
        assert!(started.elapsed() < Duration::from_secs(10), "never ready");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn spawn_call<H: ClientHandler>(
    client: &Arc<RunningService<RoleClient, H>>,
    tool: &str,
) -> tokio::task::JoinHandle<CallToolResult> {
    let client = Arc::clone(client);
    let name = exposed(tool);
    tokio::spawn(async move { client.call_tool(params(&name, json!({}))).await.unwrap() })
}

// Exigence : EX-015-01, EX-015-03
#[tokio::test(flavor = "multi_thread")]
async fn suspended_every_call_is_refused_without_upstream_call_and_the_list_stays() {
    let stack = builder(Generation::Both).build().await;
    for generation in [Generation::Legacy, Generation::Modern] {
        let (client, _connection) = connect(&stack.gateway, (), generation).await;
        let upstream_before = stack.upstream_calls();
        let ensures_before = stack.gate.ensure_count();
        let before = client.list_all_tools().await.unwrap();
        assert!(!before.is_empty());
        suspend(&stack);
        // `search_logs` is allowed by the developer profile in DEV; `delete_index` would ask for a confirmation.
        for tool in ["search_logs", "list_indices", "delete_index"] {
            let result = client.call_tool(params(&exposed(tool), json!({}))).await.unwrap();
            assert_eq!(result.is_error, Some(true));
            assert_eq!(only_text(&result), SUSPENDED);
            let record = stack.trace.last().unwrap();
            assert_eq!(record.status, CallStatus::Suspended);
            assert_eq!(record.tool, exposed(tool));
        }
        let after = client.list_all_tools().await.unwrap();
        assert_eq!(before, after, "the clients keep their tools");
        assert_eq!(stack.upstream_calls(), upstream_before);
        assert!(stack.broker.requests().is_empty());
        assert_eq!(stack.gate.ensure_count(), ensures_before);

        // Resumed: calls go through again, on the same connection.
        stack.settings.send_modify(|s| s.gateway_suspended = false);
        let result = client
            .call_tool(params(&exposed("search_logs"), json!({})))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
        assert_eq!(stack.trace.last().unwrap().status, CallStatus::Ok);
        assert_eq!(stack.upstream_calls(), upstream_before + 1);
        client.cancel().await.unwrap();
    }
}

// Exigence : EX-015-02
#[tokio::test(flavor = "multi_thread")]
async fn a_call_waiting_for_a_notification_confirmation_is_refused_at_the_suspension() {
    let stack = builder(Generation::Both)
        .settings(|s| harden(s, "list_indices", Decision::Confirm(Channel::Notification)))
        .build()
        .await;
    stack.broker.stay_silent();
    let (client, _connection) = connect(&stack.gateway, (), Generation::Legacy).await;
    let client = Arc::new(client);
    let running = spawn_call(&client, "list_indices").await;
    until(|| !stack.broker.requests().is_empty()).await;
    let suspended_at = Instant::now();
    suspend(&stack);
    let result = running.await.unwrap();
    assert!(
        suspended_at.elapsed() < Duration::from_secs(2),
        "{:?}",
        suspended_at.elapsed()
    );
    assert_eq!(only_text(&result), SUSPENDED);
    assert_eq!(stack.trace.last().unwrap().status, CallStatus::Suspended);
    assert_eq!(stack.upstream_calls(), 0);
    assert_eq!(stack.gate.ensure_count(), 0, "no login after the refusal");
}

// Exigence : EX-015-02
#[tokio::test(flavor = "multi_thread")]
async fn a_call_waiting_for_a_confirmation_in_the_chat_is_refused_at_the_suspension() {
    let stack = builder(Generation::Both)
        .config(|c| c.gateway.elicitation_clients = vec!["client-de-test".to_owned()])
        .build()
        .await;
    let (client, _connection) = connect(&stack.gateway, Silent, Generation::Legacy).await;
    let client = Arc::new(client);
    // Destructive in DEV: confirmation in the chat for a trusted client, which never answers.
    let running = spawn_call(&client, "delete_index").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!running.is_finished());
    let suspended_at = Instant::now();
    suspend(&stack);
    let result = running.await.unwrap();
    assert!(
        suspended_at.elapsed() < Duration::from_secs(2),
        "{:?}",
        suspended_at.elapsed()
    );
    assert_eq!(only_text(&result), SUSPENDED);
    assert_eq!(stack.upstream_calls(), 0);
}

// Exigence : EX-015-02
#[tokio::test(flavor = "multi_thread")]
async fn a_call_waiting_for_a_login_is_refused_at_the_suspension() {
    let stack = builder(Generation::Both).build().await;
    stack.gate.hang();
    let (client, _connection) = connect(&stack.gateway, (), Generation::Modern).await;
    let client = Arc::new(client);
    let running = spawn_call(&client, "search_logs").await;
    until(|| stack.gate.ensure_count() == 1).await;
    let suspended_at = Instant::now();
    suspend(&stack);
    let result = running.await.unwrap();
    assert!(
        suspended_at.elapsed() < Duration::from_secs(2),
        "{:?}",
        suspended_at.elapsed()
    );
    assert_eq!(only_text(&result), SUSPENDED);
    assert_eq!(stack.trace.last().unwrap().status, CallStatus::Suspended);
    assert_eq!(stack.upstream_calls(), 0);
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-emergency-{}-{}",
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

/// Toasts off: every confirmation opens its detail window, which nobody answers.
#[derive(Default)]
struct NobodyAnswers;

#[async_trait]
impl ConfirmationPresenter for NobodyAnswers {
    async fn toast_state(&self) -> ToastState {
        ToastState::Unavailable("mode « Ne pas déranger »".to_owned())
    }

    async fn show_toast(&self, _id: ConfirmationId, _toast: ToastContent) -> Result<(), PresentError> {
        Err(PresentError("pas de toast".to_owned()))
    }

    async fn open_detail(&self, _id: ConfirmationId, _view: DetailView) -> Result<(), PresentError> {
        Ok(())
    }

    fn withdraw(&self, _id: ConfirmationId) {}
}

async fn start(dir: &TempDir, store: SecretStore) -> Arc<Gateway> {
    let dirs = DataDirs::under(&dir.0);
    let gateway = Gateway::start(RuntimeDeps {
        config: Arc::new(mcp_proxy_gateway::modules::embedded_config().unwrap()),
        persistence: Persistence::sealed(&dirs, platform_key_wrapper()),
        dirs,
        store,
        auth_ui: Arc::new(UnavailableAuthUi),
        presenter: Arc::new(NobodyAnswers),
        clock: Arc::new(SystemWallClock),
        logging: Logging::Local,
        utc_offset: 0,
    })
    .await
    .unwrap();
    Arc::new(gateway)
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

type Served = tokio::task::JoinHandle<Result<(), mcp_proxy_gateway::bridge::ProofError>>;

/// A client on an in-memory pipe, proven like the bridge (spec 011).
async fn connect_to(gateway: &Arc<Gateway>) -> (RunningService<RoleClient, ()>, Served) {
    let (mut bridge_end, app_end) = tokio::io::duplex(1 << 20);
    let served = Arc::clone(gateway);
    let server = tokio::spawn(async move { served.serve_connection(Arc::new(SameUser), app_end).await });
    prove(Arc::new(SameUser), Role::Bridge, &mut bridge_end).await.unwrap();
    (().serve(bridge_end).await.unwrap(), server)
}

// Exigence : EX-015-05, EX-015-01, EX-015-02, EX-015-03
#[tokio::test(flavor = "multi_thread")]
async fn the_suspension_is_saved_survives_a_restart_and_only_resume_lifts_it() {
    let dir = TempDir::new();
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    let config = Arc::clone(gateway.config());
    gateway
        .change_settings(|s| {
            s.choose_profile(&config.profiles[&ProfileId::new("support").unwrap()]);
        })
        .await
        .unwrap();
    let tools_before = gateway.snapshot().await.exposed_tools;
    assert!(tools_before > 0);
    let (client, server) = connect_to(&gateway).await;
    let client = Arc::new(client);

    // A write in PROD waits for its confirmation in a window nobody answers.
    let running = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            let mut call = rmcp::model::CallToolRequestParams::new("prod_jira_add_comment".to_owned());
            call.arguments = json!({ "key": "DEMO-1", "body": "texte" }).as_object().cloned();
            client.call_tool(call).await.unwrap()
        })
    };
    let pending = Arc::clone(gateway.confirmations().pending());
    until(|| pending.count() == 1).await;

    gateway.suspend().await.unwrap();
    assert!(gateway.is_suspended());
    let refused = running.await.unwrap();
    assert_eq!(only_text(&refused), SUSPENDED);
    assert_eq!(pending.count(), 0);
    let snapshot = gateway.snapshot().await;
    assert!(snapshot.gateway_suspended);
    assert_eq!(snapshot.status, AppStatus::Suspended);
    assert!(
        snapshot.tooltip.starts_with("MCP Proxy — passerelle suspendue"),
        "{}",
        snapshot.tooltip
    );
    assert_eq!(snapshot.exposed_tools, tools_before, "the tool list does not change");
    // Even a read in REC, which the profile allows.
    let mut read = rmcp::model::CallToolRequestParams::new("rec_jira_search_issues".to_owned());
    read.arguments = json!({ "jql": "project = DEMO" }).as_object().cloned();
    assert_eq!(only_text(&client.call_tool(read.clone()).await.unwrap()), SUSPENDED);
    // Suspending twice changes nothing and logs once.
    gateway.suspend().await.unwrap();
    assert_eq!(events(&gateway), ["passerelle suspendue par l'utilisateur"]);
    Arc::try_unwrap(client).ok().unwrap().cancel().await.unwrap();
    let _ = server.await;
    drop(gateway);

    // Reloaded from the sealed file: still suspended.
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    assert!(gateway.is_suspended());
    let snapshot = gateway.snapshot().await;
    assert_eq!(snapshot.status, AppStatus::Suspended);
    assert_eq!(snapshot.profile.chosen.as_deref(), Some("support"));
    let (client, server) = connect_to(&gateway).await;
    assert_eq!(only_text(&client.call_tool(read.clone()).await.unwrap()), SUSPENDED);

    // « Reprendre »: calls pass the suspension again (this one then needs a login, impossible without windows).
    gateway.resume().await.unwrap();
    let resumed = only_text(&client.call_tool(read).await.unwrap());
    assert_ne!(resumed, SUSPENDED);
    assert!(resumed.contains("connexion à « SSO Outils »"), "{resumed}");
    assert_eq!(events(&gateway), ["passerelle reprise par l'utilisateur"]);
    client.cancel().await.unwrap();
    let _ = server.await;
    drop(gateway);
    let gateway = start(&dir, SecretStore::for_platform().unwrap()).await;
    assert!(!gateway.is_suspended());
    assert_eq!(gateway.snapshot().await.status, AppStatus::Neutral);
}

// Exigence : EX-015-04
#[tokio::test(flavor = "multi_thread")]
async fn disconnect_all_leaves_no_secret_and_every_session_never_connected() {
    let dir = TempDir::new();
    let sentinel = "SENTINELLE-tout-deconnecter-5c1e";
    let store = SecretStore::for_platform().unwrap();
    store
        .put(
            &SessionId::new("sso-corp").unwrap(),
            Credential::Bearer {
                token: SecretString::from(sentinel),
            },
        )
        .unwrap();
    let gateway = start(&dir, store).await;
    gateway.disconnect_all().await.unwrap();
    let snapshot = gateway.snapshot().await;
    assert!(
        snapshot.sessions.iter().all(|s| s.state.label == "jamais connectée"),
        "{:?}",
        snapshot.sessions
    );
    for (info, state) in gateway.authenticator().registry().snapshot() {
        assert_eq!(state, SessionState::NeverConnected);
        assert!(!mcp_proxy_core::session::SessionGate::is_active(
            gateway.authenticator().as_ref(),
            &info.id
        ));
    }
    // The wiped secret is still masked (grace period of its variants, spec 002).
    assert_eq!(gateway.filter_text(sentinel).as_deref(), Some("[SECRET MASQUÉ]"));
    assert_eq!(events(&gateway), ["toutes les sessions déconnectées par l'utilisateur"]);
    assert!(!gateway.is_suspended(), "disconnecting does not suspend");
}
