//! Validated configuration model. Values of these types only come out of `validate`, so every invariant listed
//! there holds for them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;
use url::Url;

use super::ids::{EnvId, McpAlias, McpId, ProfileId, SessionId, ToolName};
use super::policy::{Category, Decision, PolicyTree};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub call_budget_secs: u64,
    pub confirmation_timeout_secs: u64,
    pub auth_timeout_secs: u64,
    pub upstream_timeout_secs: u64,
    pub slow_call_threshold_secs: u64,
    pub tool_name_max_len: usize,
    pub exposed_tools_warning_threshold: usize,
    pub refresh_interval_secs: u64,
    pub list_ttl_secs: u64,
    /// `clientInfo.name` values trusted to actually display elicitation requests.
    pub elicitation_clients: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceConfig {
    pub ring_capacity: usize,
    pub ring_max_bytes: usize,
    pub log_retention_days: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebviewProfile {
    InMemory,
    EphemeralDisk,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebviewConfig {
    pub profile: WebviewProfile,
    pub os_primary_account_sso: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    pub base_url: String,
    pub repository: String,
    pub path: String,
    pub check_interval_hours: u32,
}

/// `[filter]`: sensitive keys added to the defaults of the output filter. Additions only: a default can never be
/// removed (INV-6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterConfig {
    /// Last words of a key name that make it sensitive (`password`, `token`…), lower-case ASCII letters and digits.
    #[serde(default)]
    pub extra_key_segments: Vec<String>,
    /// Whole key names that are sensitive whatever their separators and case (`api_key`…).
    #[serde(default)]
    pub extra_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: EnvId,
    pub label: String,
    pub description: String,
    pub sensitive: bool,
}

/// Scheme, host and port of a URL, as used to bind a secret to its destinations (S7).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin(String);

impl Origin {
    /// `None` for URLs without a tuple origin (no host).
    pub fn of(url: &Url) -> Option<Self> {
        match url.origin() {
            origin @ url::Origin::Tuple(..) => Some(Self(origin.ascii_serialization())),
            url::Origin::Opaque(_) => None,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A URL prefix, optionally ending with `*` (any suffix). Matching belongs to the auth methods (step 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlPattern {
    pub prefix: Url,
    pub wildcard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionScope {
    Global,
    Env(EnvId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthConfig {
    JwtHeader {
        login_url: Url,
        capture_url_pattern: UrlPattern,
        capture_header: String,
        capture_scheme: String,
        expected_audience: Option<String>,
        expected_issuer: Option<String>,
        allowed_navigation: Vec<String>,
        max_ttl_hours: u32,
    },
    Cookie {
        login_url: Url,
        cookie_url: Url,
        cookie_names: Vec<String>,
        success_url_pattern: UrlPattern,
        allowed_navigation: Vec<String>,
        ttl_hours: u32,
    },
    Basic {
        ttl_hours: u32,
        probe_url: Option<Url>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub label: String,
    pub scope: SessionScope,
    pub auth: AuthConfig,
    /// Every origin a secret of this session may be sent to (S7). Never empty.
    pub allowed_origins: BTreeSet<Origin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpKind {
    Remote,
    Local { module: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRef {
    /// The `scope = "env"` session of the instance's environment.
    Env,
    Global(SessionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub per_minute: u32,
    pub burst: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolConfig {
    /// `None`: the server annotations decide at run time, then `Destructive` (S11).
    pub category: Option<Category>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mcp {
    pub id: McpId,
    pub alias: McpAlias,
    pub label: String,
    pub kind: McpKind,
    pub session: SessionRef,
    pub list_requires_auth: bool,
    pub rate_limit: Option<RateLimit>,
    /// MCP endpoint (`url`) for a remote MCP, `base_url` for a local module.
    pub instances: BTreeMap<EnvId, Url>,
    pub tools: BTreeMap<ToolName, ToolConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub label: String,
    pub strictest: bool,
    pub default_environments: Vec<EnvId>,
    /// `Confirm(Notification)` or `Deny`.
    pub unknown_tools: Decision,
    pub policy: PolicyTree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub schema_version: u32,
    pub gateway: GatewayConfig,
    pub trace: TraceConfig,
    pub webview: WebviewConfig,
    pub update: UpdateConfig,
    pub filter: FilterConfig,
    pub environments: Vec<Environment>,
    pub sessions: Vec<Session>,
    pub mcp: Vec<Mcp>,
    pub policy: PolicyTree,
    pub profiles: BTreeMap<ProfileId, Profile>,
}

impl Config {
    pub fn environment(&self, id: &EnvId) -> Option<&Environment> {
        self.environments.iter().find(|e| &e.id == id)
    }

    pub fn session(&self, id: &SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| &s.id == id)
    }

    pub fn mcp(&self, id: &McpId) -> Option<&Mcp> {
        self.mcp.iter().find(|m| &m.id == id)
    }

    /// The profile used while no valid choice exists. Validation guarantees there is exactly one.
    pub fn strictest_profile(&self) -> Option<&Profile> {
        self.profiles.values().find(|p| p.strictest)
    }

    /// Session used by an MCP in a given environment, or `None` if the MCP has no instance there.
    pub fn session_for(&self, mcp: &McpId, env: &EnvId) -> Option<&Session> {
        let mcp = self.mcp(mcp)?;
        if !mcp.instances.contains_key(env) {
            return None;
        }
        match &mcp.session {
            SessionRef::Global(id) => self.session(id),
            SessionRef::Env => self
                .sessions
                .iter()
                .find(|s| matches!(&s.scope, SessionScope::Env(e) if e == env)),
        }
    }
}
