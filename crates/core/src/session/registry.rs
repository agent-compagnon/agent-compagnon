//! Every session of the configuration, with its state, the services it covers and the broadcast of its changes.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use tokio::sync::broadcast;

use super::state::{SessionChange, SessionState};
use crate::config::{AuthConfig, Config, EnvId, McpId, SessionId, SessionScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    JwtHeader,
    Cookie,
    Basic,
}

impl AuthKind {
    pub fn of(auth: &AuthConfig) -> Self {
        match auth {
            AuthConfig::JwtHeader { .. } => Self::JwtHeader,
            AuthConfig::Cookie { .. } => Self::Cookie,
            AuthConfig::Basic { .. } => Self::Basic,
        }
    }

    /// French label for the interface.
    pub fn label(self) -> &'static str {
        match self {
            Self::JwtHeader => "jeton JWT",
            Self::Cookie => "cookie de session",
            Self::Basic => "identifiant et mot de passe",
        }
    }
}

/// An instance (environment, MCP) whose calls use a session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionService {
    pub env: EnvId,
    pub mcp: McpId,
}

/// What a session is, from the configuration. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: SessionId,
    pub label: String,
    pub scope: SessionScope,
    pub method: AuthKind,
    /// Every instance that uses the session: one login unlocks all of them.
    pub services: Vec<SessionService>,
}

impl SessionInfo {
    /// Every session of `config`, with the instances that use it.
    pub fn all_from_config(config: &Config) -> Vec<Self> {
        config
            .sessions
            .iter()
            .map(|session| {
                let mut services: Vec<SessionService> = config
                    .mcp
                    .iter()
                    .flat_map(|mcp| {
                        mcp.instances
                            .keys()
                            .filter(|env| config.session_for(&mcp.id, env).is_some_and(|s| s.id == session.id))
                            .map(|env| SessionService {
                                env: env.clone(),
                                mcp: mcp.id.clone(),
                            })
                    })
                    .collect();
                services.sort();
                Self {
                    id: session.id.clone(),
                    label: session.label.clone(),
                    scope: session.scope.clone(),
                    method: AuthKind::of(&session.auth),
                    services,
                }
            })
            .collect()
    }
}

/// One change of state, broadcast to the interface, the notifications and the list watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEvent {
    pub session: SessionId,
    pub state: SessionState,
    pub change: SessionChange,
}

/// State of every session. Only the authenticator changes it; everyone else reads it or subscribes to its changes.
#[derive(Debug)]
pub struct SessionRegistry {
    infos: BTreeMap<SessionId, SessionInfo>,
    states: RwLock<BTreeMap<SessionId, SessionState>>,
    events: broadcast::Sender<SessionEvent>,
}

impl SessionRegistry {
    pub const EVENT_CAPACITY: usize = 64;

    /// Every session starts `NeverConnected`.
    pub fn new(infos: impl IntoIterator<Item = SessionInfo>) -> Self {
        let infos: BTreeMap<SessionId, SessionInfo> = infos.into_iter().map(|i| (i.id.clone(), i)).collect();
        let states = infos
            .keys()
            .map(|id| (id.clone(), SessionState::NeverConnected))
            .collect();
        Self {
            infos,
            states: RwLock::new(states),
            events: broadcast::channel(Self::EVENT_CAPACITY).0,
        }
    }

    pub fn from_config(config: &Config) -> Self {
        Self::new(SessionInfo::all_from_config(config))
    }

    pub fn info(&self, session: &SessionId) -> Option<&SessionInfo> {
        self.infos.get(session)
    }

    pub fn infos(&self) -> impl Iterator<Item = &SessionInfo> {
        self.infos.values()
    }

    pub fn state(&self, session: &SessionId) -> Option<SessionState> {
        self.states
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .copied()
    }

    /// Every session with its current state, in identifier order.
    pub fn snapshot(&self) -> Vec<(SessionInfo, SessionState)> {
        let states = self.states.read().unwrap_or_else(PoisonError::into_inner);
        self.infos
            .values()
            .map(|info| {
                let state = states.get(&info.id).copied().unwrap_or(SessionState::NeverConnected);
                (info.clone(), state)
            })
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Records the new state of `session` and broadcasts it. `false` for an unknown session (nothing changes).
    pub fn set(&self, session: &SessionId, state: SessionState, change: SessionChange) -> bool {
        {
            let mut states = self.states.write().unwrap_or_else(PoisonError::into_inner);
            match states.get_mut(session) {
                Some(current) => *current = state,
                None => return false,
            }
        }
        let _ = self.events.send(SessionEvent {
            session: session.clone(),
            state,
            change,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionChange;

    fn sid(id: &str) -> SessionId {
        SessionId::new(id).unwrap()
    }

    fn service(env: &str, mcp: &str) -> SessionService {
        SessionService {
            env: EnvId::new(env).unwrap(),
            mcp: McpId::new(mcp).unwrap(),
        }
    }

    #[test]
    fn sessions_know_the_services_they_unlock_and_broadcast_their_changes() {
        let config = Config::embedded().unwrap();
        let registry = SessionRegistry::from_config(&config);
        let dev = registry.info(&sid("sso-dev")).unwrap();
        assert_eq!(dev.method, AuthKind::Cookie);
        assert_eq!(
            dev.services,
            [service("dev", "k8s-admin"), service("dev", "kibana-logs")]
        );
        let corp = registry.info(&sid("sso-corp")).unwrap();
        assert_eq!(corp.method.label(), "jeton JWT");
        assert!(corp.services.contains(&service("prod", "gitlab-ops")));
        assert!(corp.services.contains(&service("int", "db-readonly")));
        assert!(
            corp.services
                .iter()
                .all(|s| ["gitlab-ops", "db-readonly"].contains(&s.mcp.as_str()))
        );
        assert_eq!(registry.state(&sid("sso-dev")), Some(SessionState::NeverConnected));
        assert_eq!(registry.snapshot().len(), config.sessions.len());

        let mut events = registry.subscribe();
        assert!(registry.set(&sid("sso-dev"), SessionState::Expired, SessionChange::Rejected));
        assert!(!registry.set(&sid("inconnue"), SessionState::Expired, SessionChange::Rejected));
        assert_eq!(
            events.try_recv().unwrap(),
            SessionEvent {
                session: sid("sso-dev"),
                state: SessionState::Expired,
                change: SessionChange::Rejected,
            }
        );
        assert!(events.try_recv().is_err());
        assert_eq!(registry.state(&sid("sso-dev")), Some(SessionState::Expired));
    }
}
