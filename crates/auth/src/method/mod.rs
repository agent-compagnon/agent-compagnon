//! Authentication methods: how a session obtains its credential from the user. A new method implements
//! [`AuthMethod`] and is given to the authenticator for its sessions; neither the gateway nor the core change.

mod basic;
mod cookie;
mod jwt;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use mcp_proxy_core::config::{AuthConfig, Config, SessionId};
use mcp_proxy_core::secrets::Credential;
use mcp_proxy_core::session::SessionInfo;
use mcp_proxy_core::trace::WallClock;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub use self::basic::{BASIC_MAX_ATTEMPTS, BASIC_MAX_TTL, BasicMethod};
pub use self::cookie::CookieMethod;
pub use self::jwt::JwtHeaderMethod;
use crate::probe::BasicProbe;
use crate::ui::{AuthUi, AuthUiError, LoginEvent, LoginWindow};

/// What a login runs with.
pub struct LoginContext<'a> {
    pub session: &'a SessionInfo,
    pub ui: &'a dyn AuthUi,
    pub clock: &'a dyn WallClock,
    /// Set when the user disconnects the session meanwhile: the method closes its window and gives up.
    pub cancel: &'a CancellationToken,
}

/// A credential obtained by a login, and when it stops being valid (before the expiry margin is taken off).
pub struct Obtained {
    pub credential: Credential,
    pub expires_at: SystemTime,
}

impl fmt::Debug for Obtained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Obtained")
            .field("credential", &self.credential)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Why a login ended without a credential. French, never carries a secret: it goes to the log only.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LoginFailure {
    #[error("connexion annulée par l'utilisateur")]
    Cancelled,
    #[error("{0}")]
    Ui(#[from] AuthUiError),
    #[error("identifiants refusés {0} fois : abandon")]
    TooManyAttempts(u32),
    #[error("contrôle des identifiants impossible : {0}")]
    Probe(String),
    #[error("identifiant obtenu inutilisable : {0}")]
    Unusable(String),
}

#[async_trait]
pub trait AuthMethod: Send + Sync {
    /// Hosts whose cookies and site data belong to the session, cleared on `disconnect`. Empty for a method without
    /// a web page.
    fn site_hosts(&self) -> Vec<String>;

    /// Obtains a credential through a visible window. Never tries anything silently.
    async fn login(&self, context: LoginContext<'_>) -> Result<Obtained, LoginFailure>;
}

/// `now + hours`, saturating.
pub(crate) fn after_hours(now: SystemTime, hours: u32) -> SystemTime {
    let span = Duration::from_secs(u64::from(hours) * 3600);
    now.checked_add(span).unwrap_or(now)
}

/// The next event of `window`, or `None` if the login is cancelled meanwhile. A window that is gone counts as
/// closed.
pub(crate) async fn next_event(window: &mut Box<dyn LoginWindow>, cancel: &CancellationToken) -> Option<LoginEvent> {
    tokio::select! {
        event = window.next_event() => Some(event.unwrap_or(LoginEvent::Closed)),
        () = cancel.cancelled() => None,
    }
}

/// The method of each session of `config`. `probe` checks Basic identifiers against `probe_url`.
pub fn methods_from_config(config: &Config, probe: &Arc<dyn BasicProbe>) -> BTreeMap<SessionId, Arc<dyn AuthMethod>> {
    config
        .sessions
        .iter()
        .filter_map(|session| {
            let method: Arc<dyn AuthMethod> = match &session.auth {
                AuthConfig::JwtHeader { .. } => Arc::new(JwtHeaderMethod::from_config(session)?),
                AuthConfig::Cookie { .. } => Arc::new(CookieMethod::from_config(session)?),
                AuthConfig::Basic { .. } => Arc::new(BasicMethod::from_config(session, Arc::clone(probe))?),
            };
            Some((session.id.clone(), method))
        })
        .collect()
}
