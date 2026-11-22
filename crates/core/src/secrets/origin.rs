use std::collections::{BTreeMap, BTreeSet};

use url::Url;

use super::error::SecretError;
use crate::config::{Config, Origin, SessionId};

/// Hosts for which plain `http` is accepted, as for configuration URLs (I1.14).
const LOOPBACK_HOSTS: &[&str] = &["localhost", "127.0.0.1"];

/// Origins (scheme, host, port) each session's secret may be sent to (S7).
#[derive(Debug, Clone, Default)]
pub struct OriginGuard {
    sessions: BTreeMap<SessionId, BTreeSet<Origin>>,
}

impl OriginGuard {
    /// Uses the origins computed by configuration validation: instance URLs of the remote MCPs and `base_url` of the
    /// local modules using the session, plus the Basic `probe_url`.
    pub fn from_config(config: &Config) -> Self {
        Self::new(
            config
                .sessions
                .iter()
                .map(|s| (s.id.clone(), s.allowed_origins.clone())),
        )
    }

    pub fn new(sessions: impl IntoIterator<Item = (SessionId, BTreeSet<Origin>)>) -> Self {
        Self {
            sessions: sessions.into_iter().collect(),
        }
    }

    pub fn knows(&self, session: &SessionId) -> bool {
        self.sessions.contains_key(session)
    }

    /// Accepts `url` only if its origin is declared for `session`, its scheme is `https` (or `http` on a loopback
    /// host) and it carries no user information.
    pub fn authorize(&self, session: &SessionId, url: &Url) -> Result<(), SecretError> {
        let origins = self
            .sessions
            .get(session)
            .ok_or_else(|| SecretError::UnknownSession(session.clone()))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(SecretError::CredentialsInUrl {
                session: session.clone(),
            });
        }
        let secure = match url.scheme() {
            "https" => true,
            "http" => url.host_str().is_some_and(|h| LOOPBACK_HOSTS.contains(&h)),
            _ => false,
        };
        if !secure {
            return Err(SecretError::InsecureScheme {
                session: session.clone(),
            });
        }
        match Origin::of(url) {
            Some(origin) if origins.contains(&origin) => Ok(()),
            origin => Err(SecretError::OriginNotAllowed {
                session: session.clone(),
                origin: origin.map_or_else(|| "(sans origine)".to_owned(), |o| o.to_string()),
            }),
        }
    }
}
