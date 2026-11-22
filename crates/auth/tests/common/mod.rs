//! An `OnDemandAuthenticator` on the embedded configuration, with `FakeAuthUi` and a clock that moves with tokio's.

#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mcp_proxy_auth::{
    AuthenticatorDeps, BasicProbe, HttpProbe, OnDemandAuthenticator, TokioWallClock, methods_from_config,
};
use mcp_proxy_core::config::{AuthConfig, Config, EnvId, McpId, Origin, SessionId, ToolName};
use mcp_proxy_core::secrets::{OriginGuard, SecretStore};
use mcp_proxy_core::session::{AccessReason, SessionEvent, SessionRegistry};
use mcp_proxy_core::trace::WallClock;
use mcp_proxy_testkit::FakeAuthUi;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use url::Url;

/// A value no output may ever contain.
pub const SENTINEL: &str = "S3NT1NEL-4b1d-do-not-print";

pub struct Harness {
    pub ui: FakeAuthUi,
    pub auth: Arc<OnDemandAuthenticator>,
    pub store: Arc<SecretStore>,
    pub registry: Arc<SessionRegistry>,
    pub config: Arc<Config>,
    pub clock: TokioWallClock,
    pub events: broadcast::Receiver<SessionEvent>,
}

impl Harness {
    pub fn now(&self) -> SystemTime {
        self.clock.now()
    }

    pub fn unix_now(&self) -> u64 {
        self.now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    /// Every event broadcast so far and not read yet.
    pub fn drain_events(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }

    /// The header the authenticator injects for `url`, as text.
    pub fn injected(&self, session: &SessionId, url: &str) -> Option<String> {
        use mcp_proxy_core::session::SessionGate as _;
        let mut headers = HeaderMap::new();
        self.auth
            .inject(session, &Url::parse(url).unwrap(), &mut headers)
            .ok()?;
        headers
            .iter()
            .next()
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap()))
    }
}

pub fn harness() -> Harness {
    harness_with(|_| {})
}

/// `probe_url` of `sso-tools` set to `url`, and allowed as one of its origins.
pub fn basic_probe_at(url: String) -> impl FnOnce(&mut Config) {
    move |config| {
        let session = config
            .sessions
            .iter_mut()
            .find(|s| s.id.as_str() == "sso-tools")
            .unwrap();
        let url = Url::parse(&url).unwrap();
        session.allowed_origins.insert(Origin::of(&url).unwrap());
        session.auth = AuthConfig::Basic {
            ttl_hours: 8,
            probe_url: Some(url),
        };
    }
}

pub fn harness_with(edit: impl FnOnce(&mut Config)) -> Harness {
    let mut config = Config::embedded().unwrap();
    edit(&mut config);
    let config = Arc::new(config);
    let store = Arc::new(SecretStore::for_platform().unwrap());
    let guard = Arc::new(OriginGuard::from_config(&config));
    let registry = Arc::new(SessionRegistry::from_config(&config));
    let probe: Arc<dyn BasicProbe> = Arc::new(HttpProbe::new(Arc::clone(&guard), Duration::from_secs(10)).unwrap());
    let ui = FakeAuthUi::default();
    let clock = TokioWallClock::new();
    let events = registry.subscribe();
    let auth = Arc::new(OnDemandAuthenticator::new(AuthenticatorDeps {
        registry: Arc::clone(&registry),
        store: Arc::clone(&store),
        guard,
        ui: Arc::new(ui.clone()),
        methods: methods_from_config(&config, &probe),
        clock: Arc::new(clock),
    }));
    Harness {
        ui,
        auth,
        store,
        registry,
        config,
        clock,
        events,
    }
}

pub fn sid(id: &str) -> SessionId {
    SessionId::new(id).unwrap()
}

pub fn reason() -> AccessReason {
    AccessReason::ToolCall {
        env: EnvId::new("prod").unwrap(),
        mcp: McpId::new("gitlab-ops").unwrap(),
        tool: ToolName::new("list_pipelines").unwrap(),
    }
}

/// `secs` from now on tokio's clock, as the chain computes its deadlines.
pub fn deadline(secs: u64) -> std::time::Instant {
    (tokio::time::Instant::now() + Duration::from_secs(secs)).into_std()
}

fn segment(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap())
}

/// An unsigned-looking JWT with these claims.
pub fn jwt(claims: Value) -> String {
    format!(
        "{}.{}.c2lnbmF0dXJl",
        segment(&json!({ "alg": "RS256", "typ": "JWT" })),
        segment(&claims)
    )
}

/// A token for `sso-corp` (audience `api://internal`) expiring `secs` after `now`.
pub fn corp_token(h: &Harness, secs: u64) -> String {
    jwt(json!({ "aud": "api://internal", "exp": h.unix_now() + secs, "sub": SENTINEL }))
}

pub const CORP_API: &str = "https://api.example.internal/v1/me";
pub const CORP_MCP: &str = "https://gitlab-mcp.example.internal/mcp";
pub const DEV_SUCCESS: &str = "https://apps.dev.example.internal/home";
pub const DEV_MCP: &str = "https://kibana-mcp.dev.example.internal/mcp";

/// Lets the spawned tasks run until they wait on something.
pub async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}
