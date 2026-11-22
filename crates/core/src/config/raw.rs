//! Serde mirror of the TOML file. Only the structure is checked here (types, required and unknown keys); every
//! semantic check lives in `validate`, so that all semantic errors can be reported together.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::model::{FilterConfig, GatewayConfig, TraceConfig, UpdateConfig, WebviewConfig};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    pub schema_version: u32,
    pub gateway: GatewayConfig,
    pub trace: TraceConfig,
    pub webview: WebviewConfig,
    pub update: UpdateConfig,
    #[serde(default)]
    pub filter: FilterConfig,
    pub environments: Vec<RawEnvironment>,
    pub sessions: Vec<RawSession>,
    pub mcp: Vec<RawMcp>,
    pub policy: RawPolicyRoot,
    pub profiles: BTreeMap<String, RawProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEnvironment {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSession {
    pub id: String,
    pub label: String,
    pub scope: String,
    pub environment: Option<String>,
    pub auth: RawAuth,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum RawAuth {
    JwtHeader {
        login_url: String,
        capture_url_pattern: String,
        capture_header: String,
        capture_scheme: String,
        expected_audience: Option<String>,
        expected_issuer: Option<String>,
        allowed_navigation: Vec<String>,
        max_ttl_hours: u32,
    },
    Cookie {
        login_url: String,
        cookie_url: String,
        cookie_names: Vec<String>,
        success_url_pattern: String,
        allowed_navigation: Vec<String>,
        ttl_hours: u32,
    },
    Basic {
        ttl_hours: u32,
        probe_url: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMcp {
    pub id: String,
    pub alias: String,
    pub label: String,
    pub kind: String,
    pub module: Option<String>,
    pub session: String,
    #[serde(default)]
    pub list_requires_auth: bool,
    pub rate_limit: Option<RawRateLimit>,
    #[serde(default)]
    pub instances: BTreeMap<String, RawInstance>,
    #[serde(default)]
    pub tools: BTreeMap<String, RawTool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawRateLimit {
    pub per_minute: u32,
    pub burst: u32,
}

/// `url` for a remote MCP, `base_url` for a local module; which one is required depends on `kind`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInstance {
    pub url: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTool {
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawRule {
    pub decision: String,
    pub channel: Option<String>,
    #[serde(default)]
    pub locked: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicyRoot {
    pub default: Option<RawRule>,
    pub read: Option<RawRule>,
    pub write: Option<RawRule>,
    pub destructive: Option<RawRule>,
    #[serde(default)]
    pub env: BTreeMap<String, RawPolicyEnv>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicyEnv {
    pub default: Option<RawRule>,
    pub read: Option<RawRule>,
    pub write: Option<RawRule>,
    pub destructive: Option<RawRule>,
    #[serde(default)]
    pub mcp: BTreeMap<String, RawPolicyMcp>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicyMcp {
    pub default: Option<RawRule>,
    pub read: Option<RawRule>,
    pub write: Option<RawRule>,
    pub destructive: Option<RawRule>,
    #[serde(default)]
    pub tool: BTreeMap<String, RawPolicyTool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPolicyTool {
    pub rule: RawRule,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawProfile {
    pub label: String,
    #[serde(default)]
    pub strictest: bool,
    #[serde(default)]
    pub default_environments: Vec<String>,
    pub unknown_tools: String,
    #[serde(default)]
    pub policy: RawPolicyRoot,
}
