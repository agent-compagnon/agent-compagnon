//! Contract between the gateway and session management (spec 006), and the sessions themselves (spec 010): their
//! state, the services they cover and the broadcast of their changes. The on-demand implementation of the gate is
//! `mcp_proxy_auth::OnDemandAuthenticator`.
//!
//! The gateway never holds a credential: it asks a [`SessionGate`] to make a session usable, then to add the
//! authentication headers to each request right before it is sent, after the destination has been checked against
//! the session's origins.

mod registry;
mod state;

pub use registry::{AuthKind, SessionEvent, SessionInfo, SessionRegistry, SessionService};
pub use state::{EXPIRY_MARGIN, SessionChange, SessionState};

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap};
use thiserror::Error;
use url::Url;

use crate::config::{EnvId, McpId, SessionId, ToolName};
use crate::secrets::http::auth_header;
use crate::secrets::{OriginGuard, SecretError, SecretStore, SecretVersion};

/// Why a session is needed, shown to the user when a connection is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessReason {
    ListTools { env: EnvId, mcp: McpId },
    ToolCall { env: EnvId, mcp: McpId, tool: ToolName },
}

/// Why a session could not be used. French, never carries a secret.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The connection is still in progress at the deadline; it goes on, and a later call may succeed.
    #[error(
        "connexion à la session « {0} » requise : la connexion est en cours, réessayez une fois la connexion faite"
    )]
    LoginPending(SessionId),
    #[error("connexion à la session « {0} » annulée par l'utilisateur")]
    Cancelled(SessionId),
    /// No connection can be started (no interface, or the gate cannot log in).
    #[error("connexion à la session « {0} » impossible pour le moment")]
    Unavailable(SessionId),
    /// The login ended without a usable credential: identifiers refused three times, probe unreachable, window
    /// error. The detail is in the log, never here.
    #[error("connexion à la session « {0} » échouée")]
    LoginFailed(SessionId),
    /// The credential could not be injected: unknown session, no secret, destination outside the session's origins.
    #[error("{0}")]
    Injection(#[from] SecretError),
}

#[async_trait]
pub trait SessionGate: Send + Sync {
    /// Makes the session usable before `deadline`, starting a connection if needed.
    async fn ensure(&self, session: &SessionId, deadline: Instant, reason: &AccessReason) -> Result<(), SessionError>;

    /// Whether the session can be used right now, without starting anything.
    fn is_active(&self, session: &SessionId) -> bool;

    /// Adds the authentication headers for `url`, after checking that its origin is one of the session's. On error,
    /// `headers` holds no authentication header. Returns the version of the secret injected.
    fn inject(&self, session: &SessionId, url: &Url, headers: &mut HeaderMap) -> Result<SecretVersion, SessionError>;

    /// The server refused the secret of `version`: if it is still the session's secret, it is wiped and the
    /// session becomes expired. A secret put since (a login completed by a concurrent call) is kept.
    fn mark_rejected(&self, session: &SessionId, version: SecretVersion);
}

/// A gate over the secret store, without any way to log in: a session is usable while its secret is in the store.
/// The on-demand authenticator (spec 010) replaces it; it is used by tests and development tools.
pub struct StoreSessionGate {
    store: Arc<SecretStore>,
    guard: Arc<OriginGuard>,
}

impl StoreSessionGate {
    pub fn new(store: Arc<SecretStore>, guard: Arc<OriginGuard>) -> Self {
        Self { store, guard }
    }
}

impl fmt::Debug for StoreSessionGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreSessionGate")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

/// Removes every header the gate may set, so that nothing set elsewhere is sent along with, or instead of, ours.
fn clear_auth(headers: &mut HeaderMap) {
    headers.remove(AUTHORIZATION);
    headers.remove(COOKIE);
}

#[async_trait]
impl SessionGate for StoreSessionGate {
    async fn ensure(&self, session: &SessionId, _: Instant, _: &AccessReason) -> Result<(), SessionError> {
        if self.store.has(session) {
            Ok(())
        } else {
            Err(SessionError::Unavailable(session.clone()))
        }
    }

    fn is_active(&self, session: &SessionId) -> bool {
        self.store.has(session)
    }

    fn inject(&self, session: &SessionId, url: &Url, headers: &mut HeaderMap) -> Result<SecretVersion, SessionError> {
        inject_from_store(&self.store, &self.guard, session, url, headers)
    }

    fn mark_rejected(&self, session: &SessionId, version: SecretVersion) {
        self.store.wipe_version(session, version);
    }
}

/// Removes any authentication header, checks the destination against the session's origins, then adds the header
/// built from the session's secret. On error, `headers` holds no authentication header.
pub fn inject_from_store(
    store: &SecretStore,
    guard: &OriginGuard,
    session: &SessionId,
    url: &Url,
    headers: &mut HeaderMap,
) -> Result<SecretVersion, SessionError> {
    clear_auth(headers);
    guard.authorize(session, url)?;
    let (header, version) = store.with_exposed_versioned(session, auth_header)?;
    let (name, value) = header?;
    headers.insert(name, value);
    Ok(version)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use reqwest::header::HeaderValue;
    use secrecy::SecretString;

    use super::*;
    use crate::config::Origin;
    use crate::secrets::Credential;

    fn sid(id: &str) -> SessionId {
        SessionId::new(id).unwrap()
    }

    fn gate() -> (StoreSessionGate, Arc<SecretStore>) {
        let store = Arc::new(SecretStore::for_platform().unwrap());
        let origin = Origin::of(&Url::parse("https://api.example.internal/").unwrap()).unwrap();
        let guard = Arc::new(OriginGuard::new([(sid("sso"), BTreeSet::from([origin]))]));
        (StoreSessionGate::new(Arc::clone(&store), guard), store)
    }

    #[tokio::test]
    async fn injects_only_towards_the_session_origins() {
        let (gate, store) = gate();
        let session = sid("sso");
        let reason = AccessReason::ListTools {
            env: EnvId::new("dev").unwrap(),
            mcp: McpId::new("kibana-logs").unwrap(),
        };
        assert_eq!(
            gate.ensure(&session, Instant::now(), &reason).await,
            Err(SessionError::Unavailable(session.clone()))
        );
        assert!(!gate.is_active(&session));
        store
            .put(
                &session,
                Credential::Bearer {
                    token: SecretString::from("tok-123"),
                },
            )
            .unwrap();
        gate.ensure(&session, Instant::now(), &reason).await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("planted=1"));
        let url = Url::parse("https://api.example.internal/mcp").unwrap();
        let version = gate.inject(&session, &url, &mut headers).unwrap();
        assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer tok-123");
        assert!(headers.get(AUTHORIZATION).unwrap().is_sensitive());
        assert!(headers.get(COOKIE).is_none());

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer planted"));
        let other = Url::parse("https://evil.example.internal/mcp").unwrap();
        let error = gate.inject(&session, &other, &mut headers).unwrap_err();
        assert!(matches!(
            error,
            SessionError::Injection(SecretError::OriginNotAllowed { .. })
        ));
        assert!(headers.is_empty());
        assert!(!error.to_string().contains("tok-123"));

        // A secret put since the rejected one is kept.
        let renewed = store
            .put(
                &session,
                Credential::Bearer {
                    token: SecretString::from("tok-456"),
                },
            )
            .unwrap();
        gate.mark_rejected(&session, version);
        assert!(gate.is_active(&session));
        gate.mark_rejected(&session, renewed);
        assert!(!gate.is_active(&session));
    }
}
