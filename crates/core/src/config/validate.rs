//! Semantic validation: turns the raw serde model into the typed `Config`, collecting every error on the way.

use std::collections::{BTreeMap, BTreeSet};

use url::Url;

use super::error::ConfigError;
use super::ids::{EnvId, IdError, McpAlias, McpId, ProfileId, SessionId, ToolName};
use super::model::{
    AuthConfig, Config, Environment, GatewayConfig, Mcp, McpKind, Origin, Profile, RateLimit, Session, SessionRef,
    SessionScope, ToolConfig, TraceConfig, UpdateConfig, UrlPattern,
};
use super::policy::{Category, Channel, Decision, EnvPolicy, McpPolicy, PolicyTree, Rule, RuleSet};
use super::raw::{
    RawAuth, RawConfig, RawEnvironment, RawMcp, RawPolicyEnv, RawPolicyMcp, RawPolicyRoot, RawProfile, RawRule,
    RawSession,
};
use crate::naming::{self, MAX_EXPOSED_NAME_LEN, MIN_EXPOSED_NAME_LEN};

/// Configuration schema understood by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Upper bound of every session lifetime: tokens live one day at most.
const MAX_TTL_HOURS: u32 = 24;

/// Claude Desktop cuts a tool call at about 60 s (S4).
const CALL_BUDGET_LIMIT_SECS: u64 = 60;

/// Reserved `[[mcp]].session` value meaning "the session of the instance's environment".
const ENV_SESSION_REF: &str = "env";

pub fn validate(raw: RawConfig, known_modules: &[&str]) -> Result<Config, Vec<ConfigError>> {
    let mut v = Validator::default();
    let config = v.config(raw, known_modules);
    match config {
        Some(config) if v.errors.is_empty() => Ok(config),
        _ => Err(v.errors),
    }
}

/// Formats a TOML key as a path segment, quoting it when it is not a bare key.
fn key(name: &str) -> String {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        name.to_owned()
    } else {
        format!("{name:?}")
    }
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Selector<'a> {
    Default,
    Category(Category),
    Tool(&'a ToolName, Category),
}

/// A rule flattened out of a tree, for the locked-rule coverage check.
struct FlatRule<'a> {
    env: Option<&'a EnvId>,
    mcp: Option<&'a McpId>,
    selector: Selector<'a>,
    rule: Rule,
    path: String,
}

impl FlatRule<'_> {
    /// Whether `self`, if locked, is a candidate every time `other` is.
    fn covers(&self, other: &FlatRule<'_>) -> bool {
        let node = (self.env.is_none() || self.env == other.env) && (self.mcp.is_none() || self.mcp == other.mcp);
        let selector = match (self.selector, other.selector) {
            (Selector::Default, _) => true,
            (Selector::Category(a), Selector::Category(b)) => a == b,
            (Selector::Category(a), Selector::Tool(_, b)) => a == b,
            (Selector::Tool(a, _), Selector::Tool(b, _)) => a == b,
            _ => false,
        };
        node && selector
    }
}

fn flatten<'a>(
    tree: &'a PolicyTree,
    prefix: &str,
    categories: &BTreeMap<(McpId, ToolName), Category>,
) -> Vec<FlatRule<'a>> {
    fn push_set<'a>(
        out: &mut Vec<FlatRule<'a>>,
        set: &RuleSet,
        env: Option<&'a EnvId>,
        mcp: Option<&'a McpId>,
        path: &str,
    ) {
        let entries = [
            (Selector::Default, set.default, "default"),
            (Selector::Category(Category::Read), set.read, "read"),
            (Selector::Category(Category::Write), set.write, "write"),
            (
                Selector::Category(Category::Destructive),
                set.destructive,
                "destructive",
            ),
        ];
        for (selector, rule, name) in entries {
            if let Some(rule) = rule {
                out.push(FlatRule {
                    env,
                    mcp,
                    selector,
                    rule,
                    path: format!("{path}.{name}"),
                });
            }
        }
    }

    let mut out = Vec::new();
    push_set(&mut out, &tree.rules, None, None, prefix);
    for (env_id, env) in &tree.env {
        let env_path = format!("{prefix}.env.{}", key(env_id.as_str()));
        push_set(&mut out, &env.rules, Some(env_id), None, &env_path);
        for (mcp_id, mcp) in &env.mcp {
            let mcp_path = format!("{env_path}.mcp.{}", key(mcp_id.as_str()));
            push_set(&mut out, &mcp.rules, Some(env_id), Some(mcp_id), &mcp_path);
            for (tool, rule) in &mcp.tools {
                // Tool rules only exist for declared tools, whose category is known.
                if let Some(category) = categories.get(&(mcp_id.clone(), tool.clone())) {
                    out.push(FlatRule {
                        env: Some(env_id),
                        mcp: Some(mcp_id),
                        selector: Selector::Tool(tool, *category),
                        rule: *rule,
                        path: format!("{mcp_path}.tool.{}.rule", key(tool.as_str())),
                    });
                }
            }
        }
    }
    out
}

/// What the policy trees may reference, known once environments and MCPs are validated.
#[derive(Default)]
struct PolicyContext {
    envs: BTreeSet<EnvId>,
    /// MCP → environments where it has an instance.
    instances: BTreeMap<McpId, BTreeSet<EnvId>>,
    categories: BTreeMap<(McpId, ToolName), Category>,
}

#[derive(Default)]
struct Validator {
    errors: Vec<ConfigError>,
}

impl Validator {
    fn err(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ConfigError {
            path: path.into(),
            message: message.into(),
        });
    }

    fn id<T>(&mut self, path: &str, value: &str, make: fn(String) -> Result<T, IdError>) -> Option<T> {
        match make(value.to_owned()) {
            Ok(id) => Some(id),
            Err(e) => {
                self.err(
                    path,
                    format!("identifiant « {} » invalide : {e}", value.escape_default()),
                );
                None
            }
        }
    }

    fn non_empty(&mut self, path: &str, value: &str) {
        if value.trim().is_empty() {
            self.err(path, "valeur vide");
        }
    }

    fn at_least(&mut self, path: &str, value: u64, min: u64) {
        if value < min {
            self.err(path, format!("doit valoir au moins {min}"));
        }
    }

    fn ttl(&mut self, path: &str, hours: u32) {
        if !(1..=MAX_TTL_HOURS).contains(&hours) {
            self.err(path, format!("durée entre 1 et {MAX_TTL_HOURS} heures attendue"));
        }
    }

    /// `https` only, `http` tolerated for `localhost` and `127.0.0.1` (tests). No credentials in URLs.
    fn url(&mut self, path: &str, value: &str) -> Option<Url> {
        let url = match Url::parse(value) {
            Ok(url) => url,
            Err(e) => {
                self.err(path, format!("URL invalide « {value} » : {e}"));
                return None;
            }
        };
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1"));
        let mut ok = true;
        match url.scheme() {
            "https" => {}
            "http" if local => {}
            "http" => {
                self.err(
                    path,
                    format!("« {value} » : https obligatoire (http toléré pour localhost et 127.0.0.1)"),
                );
                ok = false;
            }
            other => {
                self.err(
                    path,
                    format!("« {value} » : schéma « {other} » refusé, https obligatoire"),
                );
                ok = false;
            }
        }
        if url.host_str().is_none_or(str::is_empty) {
            self.err(path, format!("« {value} » : hôte manquant"));
            ok = false;
        }
        if !url.username().is_empty() || url.password().is_some() {
            self.err(path, "identifiants interdits dans une URL");
            ok = false;
        }
        ok.then_some(url)
    }

    /// A URL prefix with an optional trailing `*`. The prefix must be written in canonical form, so that the host is
    /// always terminated (`https://a.example/*` and never `https://a.example*`).
    fn pattern(&mut self, path: &str, value: &str) -> Option<UrlPattern> {
        let (prefix, wildcard) = match value.strip_suffix('*') {
            Some(prefix) => (prefix, true),
            None => (value, false),
        };
        if prefix.contains('*') {
            self.err(path, format!("« {value} » : « * » n'est accepté qu'en fin de motif"));
            return None;
        }
        let url = self.url(path, prefix)?;
        if url.as_str() != prefix {
            self.err(
                path,
                format!(
                    "« {value} » : préfixe non canonique, écrire « {}{} »",
                    url,
                    if wildcard { "*" } else { "" }
                ),
            );
            return None;
        }
        Some(UrlPattern { prefix: url, wildcard })
    }

    fn hosts(&mut self, path: &str, values: &[String]) -> Vec<String> {
        if values.is_empty() {
            self.err(path, "au moins un domaine attendu");
        }
        let mut seen = BTreeSet::new();
        for (i, host) in values.iter().enumerate() {
            let item = format!("{path}[{i}]");
            let valid = !host.is_empty()
                && host
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
                && url::Host::parse(host).is_ok_and(|h| h.to_string() == *host);
            if !valid {
                self.err(
                    item,
                    format!("« {host} » n'est pas un nom d'hôte valide (minuscules, sans schéma ni joker)"),
                );
            } else if !seen.insert(host.as_str()) {
                self.err(item, format!("domaine « {host} » en double"));
            }
        }
        values.to_vec()
    }

    fn host_allowed(&mut self, path: &str, url: &Url, allowed: &[String]) {
        if let Some(host) = url.host_str()
            && !allowed.iter().any(|a| a == host)
        {
            self.err(path, format!("l'hôte « {host} » doit figurer dans allowed_navigation"));
        }
    }

    fn config(&mut self, raw: RawConfig, known_modules: &[&str]) -> Option<Config> {
        if raw.schema_version != SCHEMA_VERSION {
            self.err(
                "schema_version",
                format!(
                    "version {} non prise en charge (attendue : {SCHEMA_VERSION})",
                    raw.schema_version
                ),
            );
        }
        self.gateway(&raw.gateway);
        self.trace(&raw.trace);
        self.update(&raw.update);

        let environments = self.environments(&raw.environments);
        let env_ids: BTreeSet<EnvId> = environments.iter().map(|e| e.id.clone()).collect();
        let sessions = self.sessions(&raw.sessions, &environments);
        let mcps = self.mcps(
            &raw.mcp,
            &env_ids,
            &sessions,
            known_modules,
            raw.gateway.tool_name_max_len,
        );
        let sessions = self.origins(&raw.sessions, sessions, &mcps);

        let ctx = PolicyContext {
            envs: env_ids,
            instances: mcps
                .iter()
                .map(|m| (m.id.clone(), m.instances.keys().cloned().collect()))
                .collect(),
            categories: mcps
                .iter()
                .flat_map(|m| {
                    m.tools
                        .iter()
                        .map(|(name, tool)| ((m.id.clone(), name.clone()), tool.category))
                })
                .collect(),
        };
        let policy = self.policy_tree("policy", &raw.policy, &ctx);
        if raw.policy.default.is_none() {
            self.err("policy.default", "règle racine « default » obligatoire");
        }
        let profiles = self.profiles(&raw.profiles, &ctx);

        let base_flat = flatten(&policy, "policy", &ctx.categories);
        self.locked_coverage(&base_flat, &base_flat);
        for profile in profiles.values() {
            let prefix = format!("profiles.{}.policy", key(profile.id.as_str()));
            let flat = flatten(&profile.policy, &prefix, &ctx.categories);
            self.locked_coverage(&flat, &base_flat);
            self.locked_coverage(&flat, &flat);
        }

        Some(Config {
            schema_version: raw.schema_version,
            gateway: raw.gateway,
            trace: raw.trace,
            webview: raw.webview,
            update: raw.update,
            environments,
            sessions,
            mcp: mcps,
            policy,
            profiles,
        })
    }

    fn gateway(&mut self, g: &GatewayConfig) {
        if !(1..CALL_BUDGET_LIMIT_SECS).contains(&g.call_budget_secs) {
            self.err(
                "gateway.call_budget_secs",
                format!(
                    "doit être compris entre 1 et {} secondes (coupure des clients vers 60 s)",
                    CALL_BUDGET_LIMIT_SECS - 1
                ),
            );
        }
        self.at_least("gateway.confirmation_timeout_secs", g.confirmation_timeout_secs, 1);
        if g.confirmation_timeout_secs > g.call_budget_secs {
            self.err(
                "gateway.confirmation_timeout_secs",
                "ne doit pas dépasser call_budget_secs",
            );
        }
        self.at_least("gateway.auth_timeout_secs", g.auth_timeout_secs, 1);
        if g.auth_timeout_secs > g.call_budget_secs {
            self.err("gateway.auth_timeout_secs", "ne doit pas dépasser call_budget_secs");
        }
        self.at_least("gateway.upstream_timeout_secs", g.upstream_timeout_secs, 1);
        self.at_least("gateway.slow_call_threshold_secs", g.slow_call_threshold_secs, 1);
        if !(MIN_EXPOSED_NAME_LEN..=MAX_EXPOSED_NAME_LEN).contains(&g.tool_name_max_len) {
            self.err(
                "gateway.tool_name_max_len",
                format!("doit être compris entre {MIN_EXPOSED_NAME_LEN} et {MAX_EXPOSED_NAME_LEN}"),
            );
        }
        self.at_least(
            "gateway.exposed_tools_warning_threshold",
            g.exposed_tools_warning_threshold as u64,
            1,
        );
        self.at_least("gateway.refresh_interval_secs", g.refresh_interval_secs, 1);
        self.at_least("gateway.list_ttl_secs", g.list_ttl_secs, 1);
        let mut seen = BTreeSet::new();
        for (i, name) in g.elicitation_clients.iter().enumerate() {
            let path = format!("gateway.elicitation_clients[{i}]");
            self.non_empty(&path, name);
            if !seen.insert(name) {
                self.err(path, format!("client « {name} » en double"));
            }
        }
    }

    fn trace(&mut self, t: &TraceConfig) {
        self.at_least("trace.ring_capacity", t.ring_capacity as u64, 1);
        self.at_least("trace.ring_max_bytes", t.ring_max_bytes as u64, 1);
        self.at_least("trace.log_retention_days", u64::from(t.log_retention_days), 1);
    }

    fn update(&mut self, u: &UpdateConfig) {
        self.url("update.base_url", &u.base_url);
        for (path, value) in [("update.repository", &u.repository), ("update.path", &u.path)] {
            let valid = !value.is_empty()
                && value.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment != "."
                        && segment != ".."
                        && segment
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                });
            if !valid {
                self.err(
                    path,
                    format!("« {value} » : segments [A-Za-z0-9._-] séparés par « / » attendus"),
                );
            }
        }
        self.at_least("update.check_interval_hours", u64::from(u.check_interval_hours), 1);
    }

    fn environments(&mut self, raw: &[RawEnvironment]) -> Vec<Environment> {
        if raw.is_empty() {
            self.err("environments", "au moins un environnement attendu");
        }
        let mut out: Vec<Environment> = Vec::new();
        for (i, env) in raw.iter().enumerate() {
            let path = format!("environments[{i}]");
            self.non_empty(&format!("{path}.label"), &env.label);
            let Some(id) = self.id(&format!("{path}.id"), &env.id, EnvId::new) else {
                continue;
            };
            if out.iter().any(|e| e.id == id) {
                self.err(format!("{path}.id"), format!("environnement « {id} » en double"));
                continue;
            }
            out.push(Environment {
                id,
                label: env.label.clone(),
                description: env.description.clone(),
                sensitive: env.sensitive,
            });
        }
        out
    }

    fn sessions(&mut self, raw: &[RawSession], envs: &[Environment]) -> Vec<Session> {
        let mut out: Vec<Session> = Vec::new();
        // Environment → index of its env-scoped session in `raw`.
        let mut env_sessions: BTreeMap<EnvId, usize> = BTreeMap::new();
        for (i, s) in raw.iter().enumerate() {
            let path = format!("sessions[{i}]");
            self.non_empty(&format!("{path}.label"), &s.label);
            let id = self.id(&format!("{path}.id"), &s.id, SessionId::new);
            let id = match id {
                Some(id) if id.as_str() == ENV_SESSION_REF => {
                    self.err(format!("{path}.id"), "« env » est réservé (voir [[mcp]].session)");
                    None
                }
                Some(id) if out.iter().any(|o| o.id == id) => {
                    self.err(format!("{path}.id"), format!("session « {id} » en double"));
                    None
                }
                other => other,
            };
            let scope = match (s.scope.as_str(), &s.environment) {
                ("global", None) => Some(SessionScope::Global),
                ("global", Some(_)) => {
                    self.err(
                        format!("{path}.environment"),
                        "interdit pour une session scope = \"global\"",
                    );
                    None
                }
                ("env", None) => {
                    self.err(
                        format!("{path}.environment"),
                        "obligatoire pour une session scope = \"env\"",
                    );
                    None
                }
                ("env", Some(env)) => {
                    let env_path = format!("{path}.environment");
                    match EnvId::new(env.clone()) {
                        Ok(env_id) if envs.iter().any(|e| e.id == env_id) => {
                            if let Some(first) = env_sessions.get(&env_id) {
                                self.err(
                                    env_path,
                                    format!("l'environnement « {env_id} » a déjà sa session (sessions[{first}])"),
                                );
                                None
                            } else {
                                env_sessions.insert(env_id.clone(), i);
                                Some(SessionScope::Env(env_id))
                            }
                        }
                        _ => {
                            self.err(env_path, format!("environnement « {env} » inconnu"));
                            None
                        }
                    }
                }
                (other, _) => {
                    self.err(format!("{path}.scope"), format!("« {other} » inconnu (global | env)"));
                    None
                }
            };
            let auth = self.auth(&format!("{path}.auth"), &s.auth);
            if let (Some(id), Some(scope), Some(auth)) = (id, scope, auth) {
                out.push(Session {
                    id,
                    label: s.label.clone(),
                    scope,
                    auth,
                    allowed_origins: BTreeSet::new(),
                });
            }
        }
        for (i, env) in envs.iter().enumerate() {
            if !env_sessions.contains_key(&env.id) {
                self.err(
                    format!("environments[{i}]"),
                    format!(
                        "aucune session scope = \"env\" pour l'environnement « {} » (exactement une attendue)",
                        env.id
                    ),
                );
            }
        }
        if !raw.iter().any(|s| s.scope == "global") {
            self.err("sessions", "au moins une session scope = \"global\" attendue");
        }
        out
    }

    fn auth(&mut self, path: &str, raw: &RawAuth) -> Option<AuthConfig> {
        let before = self.errors.len();
        let auth = match raw {
            RawAuth::JwtHeader {
                login_url,
                capture_url_pattern,
                capture_header,
                capture_scheme,
                expected_audience,
                expected_issuer,
                allowed_navigation,
                max_ttl_hours,
            } => {
                let allowed = self.hosts(&format!("{path}.allowed_navigation"), allowed_navigation);
                let login = self.url(&format!("{path}.login_url"), login_url);
                if let Some(login) = &login {
                    self.host_allowed(&format!("{path}.login_url"), login, &allowed);
                }
                let capture = self.pattern(&format!("{path}.capture_url_pattern"), capture_url_pattern);
                if !is_token(capture_header) {
                    self.err(format!("{path}.capture_header"), "nom d'en-tête HTTP invalide");
                }
                if !is_token(capture_scheme) {
                    self.err(format!("{path}.capture_scheme"), "schéma d'authentification invalide");
                }
                for (name, value) in [
                    ("expected_audience", expected_audience),
                    ("expected_issuer", expected_issuer),
                ] {
                    if let Some(value) = value {
                        self.non_empty(&format!("{path}.{name}"), value);
                    }
                }
                self.ttl(&format!("{path}.max_ttl_hours"), *max_ttl_hours);
                match (login, capture) {
                    (Some(login_url), Some(capture_url_pattern)) => Some(AuthConfig::JwtHeader {
                        login_url,
                        capture_url_pattern,
                        capture_header: capture_header.clone(),
                        capture_scheme: capture_scheme.clone(),
                        expected_audience: expected_audience.clone(),
                        expected_issuer: expected_issuer.clone(),
                        allowed_navigation: allowed,
                        max_ttl_hours: *max_ttl_hours,
                    }),
                    _ => None,
                }
            }
            RawAuth::Cookie {
                login_url,
                cookie_url,
                cookie_names,
                success_url_pattern,
                allowed_navigation,
                ttl_hours,
            } => {
                let allowed = self.hosts(&format!("{path}.allowed_navigation"), allowed_navigation);
                let login = self.url(&format!("{path}.login_url"), login_url);
                if let Some(login) = &login {
                    self.host_allowed(&format!("{path}.login_url"), login, &allowed);
                }
                let cookie = self.url(&format!("{path}.cookie_url"), cookie_url);
                let success = self.pattern(&format!("{path}.success_url_pattern"), success_url_pattern);
                if let Some(success) = &success {
                    self.host_allowed(&format!("{path}.success_url_pattern"), &success.prefix, &allowed);
                }
                if cookie_names.is_empty() {
                    self.err(format!("{path}.cookie_names"), "au moins un nom de cookie attendu");
                }
                let mut seen = BTreeSet::new();
                for (i, name) in cookie_names.iter().enumerate() {
                    if !is_token(name) {
                        self.err(
                            format!("{path}.cookie_names[{i}]"),
                            format!("nom de cookie « {name} » invalide"),
                        );
                    } else if !seen.insert(name) {
                        self.err(
                            format!("{path}.cookie_names[{i}]"),
                            format!("cookie « {name} » en double"),
                        );
                    }
                }
                self.ttl(&format!("{path}.ttl_hours"), *ttl_hours);
                match (login, cookie, success) {
                    (Some(login_url), Some(cookie_url), Some(success_url_pattern)) => Some(AuthConfig::Cookie {
                        login_url,
                        cookie_url,
                        cookie_names: cookie_names.clone(),
                        success_url_pattern,
                        allowed_navigation: allowed,
                        ttl_hours: *ttl_hours,
                    }),
                    _ => None,
                }
            }
            RawAuth::Basic { ttl_hours, probe_url } => {
                self.ttl(&format!("{path}.ttl_hours"), *ttl_hours);
                match probe_url {
                    Some(url) => self
                        .url(&format!("{path}.probe_url"), url)
                        .map(|url| AuthConfig::Basic {
                            ttl_hours: *ttl_hours,
                            probe_url: Some(url),
                        }),
                    None => Some(AuthConfig::Basic {
                        ttl_hours: *ttl_hours,
                        probe_url: None,
                    }),
                }
            }
        };
        if self.errors.len() == before { auth } else { None }
    }

    fn mcps(
        &mut self,
        raw: &[RawMcp],
        envs: &BTreeSet<EnvId>,
        sessions: &[Session],
        known_modules: &[&str],
        max_len: usize,
    ) -> Vec<Mcp> {
        let mut out: Vec<Mcp> = Vec::new();
        // Exposed name → path of the tool that produced it.
        let mut exposed: BTreeMap<String, String> = BTreeMap::new();
        for (i, m) in raw.iter().enumerate() {
            let path = format!("mcp[{i}]");
            let before = self.errors.len();
            self.non_empty(&format!("{path}.label"), &m.label);
            let id = self.id(&format!("{path}.id"), &m.id, McpId::new);
            if let Some(id) = &id
                && out.iter().any(|o| &o.id == id)
            {
                self.err(format!("{path}.id"), format!("MCP « {id} » en double"));
            }
            let alias = self.id(&format!("{path}.alias"), &m.alias, McpAlias::new);
            if let Some(alias) = &alias
                && out.iter().any(|o| &o.alias == alias)
            {
                self.err(
                    format!("{path}.alias"),
                    format!("alias « {alias} » déjà utilisé par un autre MCP"),
                );
            }
            let kind = match (m.kind.as_str(), &m.module) {
                ("remote", None) => Some(McpKind::Remote),
                ("remote", Some(_)) => {
                    self.err(format!("{path}.module"), "interdit pour kind = \"remote\"");
                    None
                }
                ("local", None) => {
                    self.err(format!("{path}.module"), "obligatoire pour kind = \"local\"");
                    None
                }
                ("local", Some(module)) if known_modules.contains(&module.as_str()) => {
                    Some(McpKind::Local { module: module.clone() })
                }
                ("local", Some(module)) => {
                    self.err(
                        format!("{path}.module"),
                        format!(
                            "module local « {module} » inconnu (connus : {})",
                            known_modules.join(", ")
                        ),
                    );
                    None
                }
                (other, _) => {
                    self.err(format!("{path}.kind"), format!("« {other} » inconnu (remote | local)"));
                    None
                }
            };
            let session = if m.session == ENV_SESSION_REF {
                Some(SessionRef::Env)
            } else {
                match sessions.iter().find(|s| s.id.as_str() == m.session) {
                    Some(s) if s.scope == SessionScope::Global => Some(SessionRef::Global(s.id.clone())),
                    Some(_) => {
                        self.err(
                            format!("{path}.session"),
                            format!("« {} » est une session d'environnement : utiliser « env »", m.session),
                        );
                        None
                    }
                    None => {
                        self.err(format!("{path}.session"), format!("session « {} » inconnue", m.session));
                        None
                    }
                }
            };
            let rate_limit = m.rate_limit.as_ref().map(|r| {
                self.at_least(&format!("{path}.rate_limit.per_minute"), u64::from(r.per_minute), 1);
                self.at_least(&format!("{path}.rate_limit.burst"), u64::from(r.burst), 1);
                RateLimit {
                    per_minute: r.per_minute,
                    burst: r.burst,
                }
            });

            if m.instances.is_empty() {
                self.err(format!("{path}.instances"), "au moins une instance attendue");
            }
            let mut instances = BTreeMap::new();
            for (env, instance) in &m.instances {
                let inst_path = format!("{path}.instances.{}", key(env));
                let env_id = EnvId::new(env.clone()).ok().filter(|e| envs.contains(e));
                if env_id.is_none() {
                    self.err(inst_path.clone(), format!("environnement « {env} » inconnu"));
                }
                let (wanted, unwanted, value, extra) = match kind {
                    Some(McpKind::Local { .. }) => ("base_url", "url", &instance.base_url, &instance.url),
                    _ => ("url", "base_url", &instance.url, &instance.base_url),
                };
                if extra.is_some() && kind.is_some() {
                    self.err(
                        format!("{inst_path}.{unwanted}"),
                        format!("interdit pour kind = \"{}\"", m.kind),
                    );
                }
                let url = match value {
                    Some(value) => self.url(&format!("{inst_path}.{wanted}"), value),
                    None => {
                        if kind.is_some() {
                            self.err(format!("{inst_path}.{wanted}"), "obligatoire");
                        }
                        None
                    }
                };
                if let (Some(env_id), Some(url)) = (env_id, url) {
                    instances.insert(env_id, url);
                }
            }

            let mut tools = BTreeMap::new();
            for (name, tool) in &m.tools {
                let tool_path = format!("{path}.tools.{}", key(name));
                let tool_name = self.id(&tool_path, name, ToolName::new);
                let category = self.category(&format!("{tool_path}.category"), &tool.category);
                if let (Some(tool_name), Some(category)) = (tool_name, category) {
                    tools.insert(tool_name, ToolConfig { category });
                }
            }

            if let Some(alias) = &alias {
                for env in instances.keys() {
                    for tool in tools.keys() {
                        // An invalid max length is already reported on [gateway].
                        let Ok(name) = naming::exposed_tool_name(env, alias, tool, max_len) else {
                            continue;
                        };
                        let tool_path = format!("{path}.tools.{}", key(tool.as_str()));
                        if let Some(other) = exposed.get(&name) {
                            self.err(
                                tool_path,
                                format!("nom exposé « {name} » (environnement « {env} ») déjà produit par {other}"),
                            );
                        } else {
                            exposed.insert(name, format!("{tool_path} (environnement « {env} »)"));
                        }
                    }
                }
            }

            if self.errors.len() != before {
                continue;
            }
            if let (Some(id), Some(alias), Some(kind), Some(session)) = (id, alias, kind, session) {
                out.push(Mcp {
                    id,
                    alias,
                    label: m.label.clone(),
                    kind,
                    session,
                    list_requires_auth: m.list_requires_auth,
                    rate_limit,
                    instances,
                    tools,
                });
            }
        }
        out
    }

    /// Fills `allowed_origins` (S7): instance URLs of the MCPs using the session, plus the Basic `probe_url`.
    fn origins(&mut self, raw: &[RawSession], mut sessions: Vec<Session>, mcps: &[Mcp]) -> Vec<Session> {
        for session in &mut sessions {
            for mcp in mcps {
                for (env, url) in &mcp.instances {
                    let uses = match (&mcp.session, &session.scope) {
                        (SessionRef::Global(id), _) => id == &session.id,
                        (SessionRef::Env, SessionScope::Env(e)) => e == env,
                        (SessionRef::Env, SessionScope::Global) => false,
                    };
                    if uses && let Some(origin) = Origin::of(url) {
                        session.allowed_origins.insert(origin);
                    }
                }
            }
            if let AuthConfig::Basic {
                probe_url: Some(url), ..
            } = &session.auth
                && let Some(origin) = Origin::of(url)
            {
                session.allowed_origins.insert(origin);
            }
        }
        for session in &sessions {
            if session.allowed_origins.is_empty() {
                let index = raw.iter().position(|r| r.id == session.id.as_str()).unwrap_or_default();
                self.err(
                    format!("sessions[{index}]"),
                    format!(
                        "aucune origine autorisée calculable pour la session « {} » : aucun MCP ne l'utilise",
                        session.id
                    ),
                );
            }
        }
        sessions
    }

    fn category(&mut self, path: &str, value: &str) -> Option<Category> {
        match value {
            "read" => Some(Category::Read),
            "write" => Some(Category::Write),
            "destructive" => Some(Category::Destructive),
            other => {
                self.err(
                    path,
                    format!("catégorie « {other} » inconnue (read | write | destructive)"),
                );
                None
            }
        }
    }

    fn rule(&mut self, path: &str, raw: &RawRule) -> Option<Rule> {
        let channel = match raw.channel.as_deref() {
            None => None,
            Some("notification") => Some(Channel::Notification),
            Some("elicitation") => Some(Channel::Elicitation),
            Some(other) => {
                self.err(
                    format!("{path}.channel"),
                    format!("canal « {other} » inconnu (notification | elicitation)"),
                );
                return None;
            }
        };
        let decision = match (raw.decision.as_str(), channel) {
            ("allow", None) => Decision::Allow,
            ("deny", None) => Decision::Deny,
            ("confirm", Some(channel)) => Decision::Confirm(channel),
            ("confirm", None) => {
                self.err(format!("{path}.channel"), "obligatoire avec decision = \"confirm\"");
                return None;
            }
            ("allow" | "deny", Some(_)) => {
                self.err(format!("{path}.channel"), "n'a de sens qu'avec decision = \"confirm\"");
                return None;
            }
            (other, _) => {
                self.err(
                    format!("{path}.decision"),
                    format!("décision « {other} » inconnue (allow | confirm | deny)"),
                );
                return None;
            }
        };
        Some(Rule {
            decision,
            locked: raw.locked,
        })
    }

    fn rule_set(
        &mut self,
        path: &str,
        default: &Option<RawRule>,
        read: &Option<RawRule>,
        write: &Option<RawRule>,
        destructive: &Option<RawRule>,
    ) -> RuleSet {
        let mut one =
            |name: &str, raw: &Option<RawRule>| raw.as_ref().and_then(|r| self.rule(&format!("{path}.{name}"), r));
        RuleSet {
            default: one("default", default),
            read: one("read", read),
            write: one("write", write),
            destructive: one("destructive", destructive),
        }
    }

    fn policy_tree(&mut self, prefix: &str, raw: &RawPolicyRoot, ctx: &PolicyContext) -> PolicyTree {
        let mut tree = PolicyTree {
            rules: self.rule_set(prefix, &raw.default, &raw.read, &raw.write, &raw.destructive),
            env: BTreeMap::new(),
        };
        for (env, raw_env) in &raw.env {
            let path = format!("{prefix}.env.{}", key(env));
            match EnvId::new(env.clone()).ok().filter(|e| ctx.envs.contains(e)) {
                Some(env_id) => {
                    let node = self.env_policy(&path, &env_id, raw_env, ctx);
                    tree.env.insert(env_id, node);
                }
                None => self.err(path, format!("environnement « {env} » inconnu")),
            }
        }
        tree
    }

    fn env_policy(&mut self, path: &str, env: &EnvId, raw: &RawPolicyEnv, ctx: &PolicyContext) -> EnvPolicy {
        let mut node = EnvPolicy {
            rules: self.rule_set(path, &raw.default, &raw.read, &raw.write, &raw.destructive),
            mcp: BTreeMap::new(),
        };
        for (mcp, raw_mcp) in &raw.mcp {
            let mcp_path = format!("{path}.mcp.{}", key(mcp));
            let mcp_id = McpId::new(mcp.clone()).ok();
            match mcp_id
                .as_ref()
                .and_then(|id| ctx.instances.get(id).map(|envs| (id, envs)))
            {
                Some((id, envs)) if envs.contains(env) => {
                    let policy = self.mcp_policy(&mcp_path, id, raw_mcp, ctx);
                    node.mcp.insert(id.clone(), policy);
                }
                Some(_) => self.err(mcp_path, format!("le MCP « {mcp} » n'a pas d'instance dans « {env} »")),
                None => self.err(mcp_path, format!("MCP « {mcp} » inconnu")),
            }
        }
        node
    }

    fn mcp_policy(&mut self, path: &str, mcp: &McpId, raw: &RawPolicyMcp, ctx: &PolicyContext) -> McpPolicy {
        let mut node = McpPolicy {
            rules: self.rule_set(path, &raw.default, &raw.read, &raw.write, &raw.destructive),
            tools: BTreeMap::new(),
        };
        for (tool, raw_tool) in &raw.tool {
            let tool_path = format!("{path}.tool.{}", key(tool));
            let declared = ToolName::new(tool.clone())
                .ok()
                .filter(|t| ctx.categories.contains_key(&(mcp.clone(), t.clone())));
            let Some(tool_name) = declared else {
                // A rule on an undeclared tool would most likely be a typo that silently protects nothing.
                self.err(
                    tool_path,
                    format!("outil « {tool} » non déclaré dans [mcp.tools] de « {mcp} »"),
                );
                continue;
            };
            if let Some(rule) = self.rule(&format!("{tool_path}.rule"), &raw_tool.rule) {
                node.tools.insert(tool_name, rule);
            }
        }
        node
    }

    fn profiles(&mut self, raw: &BTreeMap<String, RawProfile>, ctx: &PolicyContext) -> BTreeMap<ProfileId, Profile> {
        let mut out = BTreeMap::new();
        let mut strictest = Vec::new();
        for (name, p) in raw {
            let path = format!("profiles.{}", key(name));
            let before = self.errors.len();
            let id = self.id(&path, name, ProfileId::new);
            self.non_empty(&format!("{path}.label"), &p.label);
            let mut default_environments = Vec::new();
            for (i, env) in p.default_environments.iter().enumerate() {
                let env_path = format!("{path}.default_environments[{i}]");
                match EnvId::new(env.clone()).ok().filter(|e| ctx.envs.contains(e)) {
                    Some(e) if default_environments.contains(&e) => {
                        self.err(env_path, format!("environnement « {env} » en double"));
                    }
                    Some(e) => default_environments.push(e),
                    None => self.err(env_path, format!("environnement « {env} » inconnu")),
                }
            }
            let unknown_tools = match p.unknown_tools.as_str() {
                "confirm" => Some(Decision::Confirm(Channel::Notification)),
                "deny" => Some(Decision::Deny),
                other => {
                    self.err(
                        format!("{path}.unknown_tools"),
                        format!("« {other} » inconnu (confirm | deny)"),
                    );
                    None
                }
            };
            if p.strictest {
                strictest.push(path.clone());
                if unknown_tools == Some(Decision::Confirm(Channel::Notification)) {
                    self.err(
                        format!("{path}.unknown_tools"),
                        "le profil strictest sert de repli : unknown_tools = \"deny\" exigé",
                    );
                }
            }
            let policy = self.policy_tree(&format!("{path}.policy"), &p.policy, ctx);
            if self.errors.len() != before {
                continue;
            }
            if let (Some(id), Some(unknown_tools)) = (id, unknown_tools) {
                out.insert(
                    id.clone(),
                    Profile {
                        id,
                        label: p.label.clone(),
                        strictest: p.strictest,
                        default_environments,
                        unknown_tools,
                        policy,
                    },
                );
            }
        }
        match strictest.len() {
            1 => {}
            0 => self.err(
                "profiles",
                "exactement un profil strictest = true attendu, aucun trouvé",
            ),
            _ => {
                for path in &strictest[1..] {
                    self.err(
                        format!("{path}.strictest"),
                        format!(
                            "exactement un profil strictest = true attendu (déjà : {})",
                            strictest[0]
                        ),
                    );
                }
            }
        }
        out
    }

    /// A rule softer than a locked rule covering it can never win: it is rejected rather than silently ignored.
    fn locked_coverage(&mut self, rules: &[FlatRule<'_>], locked_from: &[FlatRule<'_>]) {
        for rule in rules {
            let floor = locked_from
                .iter()
                .filter(|l| {
                    l.rule.locked && l.path != rule.path && l.covers(rule) && rule.rule.decision < l.rule.decision
                })
                .max_by_key(|l| l.rule.decision);
            if let Some(floor) = floor {
                self.err(
                    rule.path.clone(),
                    format!(
                        "règle plus souple que la règle verrouillée {} qui la couvre : elle serait sans effet",
                        floor.path
                    ),
                );
            }
        }
    }
}
