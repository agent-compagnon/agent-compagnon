//! What the interface shows (`AppSnapshot`) and the state of the notification icon (`AppStatus`), computed from the
//! configuration, the registry, the settings, the sessions and the tool lists. Pure: the application only sends the
//! result to the window (`get_snapshot`, `snapshot-changed`) and to the icon.
//!
//! Nothing secret is read here: sessions are seen through their state, never through the store. The runtime passes
//! the serialized snapshot through the output filter once more before it leaves (defence in depth).

use std::time::{SystemTime, UNIX_EPOCH};

use mcp_proxy_core::config::{Category, Config, McpKind, Profile, SessionScope};
use mcp_proxy_core::session::{AuthKind, SessionInfo, SessionState};
use mcp_proxy_core::settings::UserSettings;
use serde::Serialize;

use crate::registry::{CatalogSource, InstanceId, Registry};
use crate::updates::{InstanceState, InstanceStatus, McpUpdate};

/// The three states of the notification icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AppStatus {
    Neutral,
    /// Orange: a session or a confirmation waits for the user.
    Attention,
    Error,
}

/// What the state of the icon depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusInputs {
    /// Sessions whose login was asked for and is not finished.
    pub sessions_waiting: usize,
    pub confirmations_waiting: usize,
    /// Alerts of level [`AlertLevel::Error`].
    pub errors: usize,
}

impl AppStatus {
    /// Attention first: a waiting call has a deadline, an error stays shown in the window until it is dealt with.
    pub fn of(inputs: &StatusInputs) -> Self {
        if inputs.sessions_waiting > 0 || inputs.confirmations_waiting > 0 {
            Self::Attention
        } else if inputs.errors > 0 {
            Self::Error
        } else {
            Self::Neutral
        }
    }

    /// Tooltip of the icon: `MCP Proxy — …`, every waiting item and error counted.
    pub fn tooltip(inputs: &StatusInputs) -> String {
        let mut parts = Vec::new();
        match inputs.sessions_waiting {
            0 => {}
            1 => parts.push("1 connexion attend".to_owned()),
            n => parts.push(format!("{n} connexions attendent")),
        }
        match inputs.confirmations_waiting {
            0 => {}
            1 => parts.push("1 confirmation attend".to_owned()),
            n => parts.push(format!("{n} confirmations attendent")),
        }
        match inputs.errors {
            0 => {}
            1 => parts.push("1 erreur".to_owned()),
            n => parts.push(format!("{n} erreurs")),
        }
        if parts.is_empty() {
            "MCP Proxy — prêt".to_owned()
        } else {
            format!("MCP Proxy — {}", parts.join(", "))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlertLevel {
    Info,
    Attention,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlertKind {
    /// The settings file was unreadable or invalid: the strictest settings replaced it.
    SettingsReset,
    /// More exposed tools than `exposed_tools_warning_threshold`.
    ToolThreshold,
    /// A catalogue cache file was ignored or could not be written.
    CatalogCache,
    /// Hardening that could not be applied at start (exclusion from Windows error reports).
    StartupHardening,
}

/// A global banner of the interface. French, holds no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub level: AlertLevel,
    pub kind: AlertKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChoice {
    pub id: String,
    pub label: String,
    pub strictest: bool,
    pub default_environments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    /// `None` until the user picks one: the window then asks for it (first launch).
    pub chosen: Option<String>,
    /// The profile in force: the chosen one, or the strictest.
    pub active: Option<String>,
    pub first_launch: bool,
    pub choices: Vec<ProfileChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub sensitive: bool,
    pub exposed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStateKind {
    NeverConnected,
    Active,
    Expired,
    LoginRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateView {
    pub kind: SessionStateKind,
    /// « jamais connectée », « active jusqu'à 17:42 », « expirée », « connexion demandée ».
    pub label: String,
    /// Milliseconds since the Unix epoch: end of an active session, or when the login was asked for.
    pub at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceView {
    pub environment: String,
    pub mcp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: String,
    pub label: String,
    /// `None` for a global session.
    pub environment: Option<String>,
    pub method: String,
    pub state: SessionStateView,
    pub services: Vec<ServiceView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    pub exposed_name: String,
    pub name: String,
    pub title: String,
    pub category: String,
    /// Declared in the configuration.
    pub known: bool,
    pub exposed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    pub at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    pub environment: String,
    pub exposed: bool,
    pub state: String,
    pub state_label: String,
    /// Where the tools come from: `embedded`, `cache`, `live`, or `None` without any list.
    pub source: Option<String>,
    pub refreshed_at: Option<u64>,
    /// French, already filtered by the list watcher.
    pub last_error: Option<String>,
    pub tools: Vec<ToolView>,
    pub update: Option<UpdateView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpView {
    pub id: String,
    pub alias: String,
    pub label: String,
    pub local: bool,
    pub disabled: bool,
    pub instances: Vec<InstanceView>,
}

/// Everything the interface shows. Serialized in camelCase; holds no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub version: String,
    pub status: AppStatus,
    pub tooltip: String,
    pub profile: ProfileView,
    pub environments: Vec<EnvironmentView>,
    pub sessions: Vec<SessionView>,
    pub mcps: Vec<McpView>,
    pub exposed_tools: usize,
    pub exposed_tools_threshold: usize,
    pub gateway_suspended: bool,
    pub pending_confirmations: usize,
    pub alerts: Vec<Alert>,
    /// Update ready to apply (spec 020); always `None` before it.
    pub update: Option<String>,
}

/// What a snapshot is built from.
pub struct SnapshotInputs<'a> {
    pub config: &'a Config,
    pub registry: &'a Registry,
    pub settings: &'a UserSettings,
    pub sessions: &'a [(SessionInfo, SessionState)],
    pub instances: &'a [(InstanceId, InstanceStatus)],
    pub updates: &'a [McpUpdate],
    pub pending_confirmations: usize,
    /// Alerts known outside of these inputs (settings reset, start-up hardening).
    pub alerts: &'a [Alert],
    /// Local offset from UTC, east positive, for the « active jusqu'à HH:MM » labels.
    pub utc_offset: i32,
}

fn millis(at: SystemTime) -> Option<u64> {
    at.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
}

fn category_key(category: Category) -> &'static str {
    match category {
        Category::Read => "read",
        Category::Write => "write",
        Category::Destructive => "destructive",
    }
}

fn source_key(source: CatalogSource) -> &'static str {
    match source {
        CatalogSource::Embedded => "embedded",
        CatalogSource::Cache => "cache",
        CatalogSource::Live => "live",
    }
}

fn state_key(state: InstanceState) -> &'static str {
    match state {
        InstanceState::Catalogue => "catalogue",
        InstanceState::Connected => "connected",
        InstanceState::SessionRequired => "sessionRequired",
        InstanceState::Unreachable => "unreachable",
        InstanceState::Disabled => "disabled",
    }
}

fn method_key(method: AuthKind) -> &'static str {
    match method {
        AuthKind::JwtHeader => "jwtHeader",
        AuthKind::Cookie => "cookie",
        AuthKind::Basic => "basic",
    }
}

fn session_state(state: SessionState, utc_offset: i32) -> SessionStateView {
    let (kind, at) = match state {
        SessionState::NeverConnected => (SessionStateKind::NeverConnected, None),
        SessionState::Active { until } => (SessionStateKind::Active, millis(until)),
        SessionState::Expired => (SessionStateKind::Expired, None),
        SessionState::LoginRequested { since } => (SessionStateKind::LoginRequested, millis(since)),
    };
    SessionStateView {
        kind,
        label: state.label(utc_offset),
        at,
    }
}

impl AppSnapshot {
    pub fn build(inputs: &SnapshotInputs<'_>) -> Self {
        let config = inputs.config;
        let registry = inputs.registry;
        let exposure = registry.exposure();
        let chosen = inputs.settings.chosen_profile(config);

        let profile = ProfileView {
            chosen: chosen.map(|p| p.id.to_string()),
            active: inputs.settings.active_profile(config).map(|p| p.id.to_string()),
            first_launch: chosen.is_none(),
            choices: config
                .profiles
                .values()
                .map(|p| ProfileChoice {
                    id: p.id.to_string(),
                    label: p.label.clone(),
                    strictest: p.strictest,
                    default_environments: p.default_environments.iter().map(ToString::to_string).collect(),
                })
                .collect(),
        };

        let environments = config
            .environments
            .iter()
            .map(|e| EnvironmentView {
                id: e.id.to_string(),
                label: e.label.clone(),
                description: e.description.clone(),
                sensitive: e.sensitive,
                exposed: exposure.environments().contains(&e.id),
            })
            .collect();

        // In the order of the configuration, like environments and MCPs.
        let sessions = config
            .sessions
            .iter()
            .filter_map(|s| inputs.sessions.iter().find(|(info, _)| info.id == s.id))
            .map(|(info, state)| SessionView {
                id: info.id.to_string(),
                label: info.label.clone(),
                environment: match &info.scope {
                    SessionScope::Global => None,
                    SessionScope::Env(env) => Some(env.to_string()),
                },
                method: method_key(info.method).to_owned(),
                state: session_state(*state, inputs.utc_offset),
                services: info
                    .services
                    .iter()
                    .map(|s| ServiceView {
                        environment: s.env.to_string(),
                        mcp: s.mcp.to_string(),
                    })
                    .collect(),
            })
            .collect();

        let mcps = config
            .mcp
            .iter()
            .map(|mcp| McpView {
                id: mcp.id.to_string(),
                alias: mcp.alias.to_string(),
                label: mcp.label.clone(),
                local: matches!(mcp.kind, McpKind::Local { .. }),
                disabled: exposure.disabled_mcp().contains(&mcp.id),
                instances: config
                    .environments
                    .iter()
                    .map(|e| &e.id)
                    .filter(|env| mcp.instances.contains_key(*env))
                    .map(|env| {
                        let instance = InstanceId::new(env.clone(), mcp.id.clone());
                        let status = inputs.instances.iter().find(|(i, _)| *i == instance).map(|(_, s)| s);
                        let exposed = exposure.exposes(&instance);
                        InstanceView {
                            environment: env.to_string(),
                            exposed,
                            state: status.map_or("catalogue", |s| state_key(s.state)).to_owned(),
                            state_label: status
                                .map_or(InstanceState::Catalogue.label(), |s| s.state.label())
                                .to_owned(),
                            source: registry.instance_source(&instance).map(|s| source_key(s).to_owned()),
                            refreshed_at: status.and_then(|s| s.refreshed_at).and_then(millis),
                            last_error: status.and_then(|s| s.last_error.clone()),
                            tools: registry
                                .tools(&instance)
                                .map(|t| ToolView {
                                    exposed_name: t.exposed_name.clone(),
                                    name: t.original_name.to_string(),
                                    title: t.title.clone(),
                                    category: category_key(t.category).to_owned(),
                                    known: t.known,
                                    exposed,
                                })
                                .collect(),
                            update: inputs
                                .updates
                                .iter()
                                .find(|u| u.instance == instance)
                                .map(|u| UpdateView {
                                    added: u.added.iter().map(ToString::to_string).collect(),
                                    modified: u.modified.iter().map(ToString::to_string).collect(),
                                    removed: u.removed.iter().map(ToString::to_string).collect(),
                                    at: millis(u.at),
                                }),
                        }
                    })
                    .collect(),
            })
            .collect();

        let mut alerts = inputs.alerts.to_vec();
        if registry.over_threshold() {
            alerts.push(Alert {
                level: AlertLevel::Attention,
                kind: AlertKind::ToolThreshold,
                message: format!(
                    "{} outils exposés, au-delà du seuil de {} : certains clients LLM en ignorent une partie.",
                    registry.exposed_count(),
                    config.gateway.exposed_tools_warning_threshold
                ),
            });
        }
        alerts.extend(registry.cache_warnings().iter().map(|w| Alert {
            level: AlertLevel::Info,
            kind: AlertKind::CatalogCache,
            message: format!("{} : {}", w.instance, w.reason),
        }));

        let status_inputs = StatusInputs {
            sessions_waiting: inputs
                .sessions
                .iter()
                .filter(|(_, s)| matches!(s, SessionState::LoginRequested { .. }))
                .count(),
            confirmations_waiting: inputs.pending_confirmations,
            errors: alerts.iter().filter(|a| a.level == AlertLevel::Error).count(),
        };

        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            status: AppStatus::of(&status_inputs),
            tooltip: AppStatus::tooltip(&status_inputs),
            profile,
            environments,
            sessions,
            mcps,
            exposed_tools: registry.exposed_count(),
            exposed_tools_threshold: config.gateway.exposed_tools_warning_threshold,
            gateway_suspended: inputs.settings.gateway_suspended,
            pending_confirmations: inputs.pending_confirmations,
            alerts,
            update: None,
        }
    }
}

/// What choosing `profile` does, asked before it applies: the environments it exposes, sensitive ones named. For
/// the « Profil » menu of the icon (spec 014) until the profile selector of the interface (spec 016).
pub fn profile_change_question(config: &Config, profile: &Profile) -> String {
    let label = |id: &mcp_proxy_core::config::EnvId| {
        config
            .environment(id)
            .map_or_else(|| id.to_string(), |e| e.label.clone())
    };
    let exposed: Vec<String> = profile.default_environments.iter().map(label).collect();
    let sensitive: Vec<String> = profile
        .default_environments
        .iter()
        .filter(|id| config.environment(id).is_some_and(|e| e.sensitive))
        .map(label)
        .collect();
    let mut text = format!(
        "Passer au profil « {} » ?\n\nEnvironnements exposés aux clients LLM : {}.",
        profile.label,
        if exposed.is_empty() {
            "aucun".to_owned()
        } else {
            exposed.join(", ")
        }
    );
    if !sensitive.is_empty() {
        text.push_str(&format!(" Environnements sensibles : {}.", sensitive.join(", ")));
    }
    text.push_str(" Vos durcissements sont conservés.");
    text
}

#[cfg(test)]
mod tests {
    use mcp_proxy_core::config::ProfileId;

    use super::*;

    #[test]
    fn the_profile_question_names_what_gets_exposed() {
        let config = crate::modules::embedded_config().unwrap();
        let question = |id: &str| profile_change_question(&config, &config.profiles[&ProfileId::new(id).unwrap()]);
        assert_eq!(
            question("developpeur"),
            "Passer au profil « Développeur » ?\n\nEnvironnements exposés aux clients LLM : DEV, INT, REC. Vos \
             durcissements sont conservés."
        );
        assert!(
            question("support").contains("Environnements sensibles : PPROD, PROD."),
            "{}",
            question("support")
        );
    }

    // Exigence : EX-011-01
    #[test]
    fn status_of_the_icon() {
        let none = StatusInputs::default();
        assert_eq!(AppStatus::of(&none), AppStatus::Neutral);
        assert_eq!(AppStatus::tooltip(&none), "MCP Proxy — prêt");

        let session = StatusInputs {
            sessions_waiting: 1,
            ..none
        };
        assert_eq!(AppStatus::of(&session), AppStatus::Attention);
        assert_eq!(AppStatus::tooltip(&session), "MCP Proxy — 1 connexion attend");

        let confirmations = StatusInputs {
            confirmations_waiting: 2,
            ..none
        };
        assert_eq!(AppStatus::of(&confirmations), AppStatus::Attention);
        assert_eq!(
            AppStatus::tooltip(&confirmations),
            "MCP Proxy — 2 confirmations attendent"
        );

        let error = StatusInputs { errors: 1, ..none };
        assert_eq!(AppStatus::of(&error), AppStatus::Error);
        assert_eq!(AppStatus::tooltip(&error), "MCP Proxy — 1 erreur");

        let all = StatusInputs {
            sessions_waiting: 3,
            confirmations_waiting: 1,
            errors: 2,
        };
        assert_eq!(AppStatus::of(&all), AppStatus::Attention, "a waiting call comes first");
        assert_eq!(
            AppStatus::tooltip(&all),
            "MCP Proxy — 3 connexions attendent, 1 confirmation attend, 2 erreurs"
        );
        assert_eq!(serde_json::to_value(AppStatus::Attention).unwrap(), "attention");
    }
}
