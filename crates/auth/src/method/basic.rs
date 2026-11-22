//! `basic`: login and password typed in a window of the application, checked against `probe_url` if there is one.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::config::{AuthConfig, Session, SessionId};
use mcp_proxy_core::secrets::{Credential, basic_authorization};
use url::Url;

use super::{AuthMethod, LoginContext, LoginFailure, Obtained};
use crate::probe::{BasicProbe, ProbeOutcome};
use crate::ui::{BasicPromptSpec, window_title};

/// Inputs refused by the probe before the login is given up.
pub const BASIC_MAX_ATTEMPTS: u32 = 3;

/// Longest life of Basic identifiers, whatever `ttl_hours` says.
pub const BASIC_MAX_TTL: Duration = Duration::from_secs(24 * 3600);

const REFUSED: &str = "identifiants refusés";
const INVALID: &str = "identifiant ou mot de passe invalide (vide, ou « : » dans l'identifiant)";

pub struct BasicMethod {
    session: SessionId,
    label: String,
    ttl: Duration,
    probe_url: Option<Url>,
    probe: Arc<dyn BasicProbe>,
}

impl std::fmt::Debug for BasicMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicMethod")
            .field("session", &self.session.as_str())
            .field("ttl", &self.ttl)
            .field("probe_url", &self.probe_url.as_ref().map(Url::as_str))
            .finish_non_exhaustive()
    }
}

impl BasicMethod {
    /// `None` if the session does not use `basic`.
    pub fn from_config(session: &Session, probe: Arc<dyn BasicProbe>) -> Option<Self> {
        let AuthConfig::Basic { ttl_hours, probe_url } = &session.auth else {
            return None;
        };
        Some(Self {
            session: session.id.clone(),
            label: session.label.clone(),
            ttl: Duration::from_secs(u64::from(*ttl_hours) * 3600).min(BASIC_MAX_TTL),
            probe_url: probe_url.clone(),
            probe,
        })
    }
}

#[async_trait]
impl AuthMethod for BasicMethod {
    fn site_hosts(&self) -> Vec<String> {
        Vec::new()
    }

    async fn login(&self, context: LoginContext<'_>) -> Result<Obtained, LoginFailure> {
        let mut error = None;
        let mut login = None;
        for attempt in 1..=BASIC_MAX_ATTEMPTS {
            let spec = BasicPromptSpec {
                session: self.session.clone(),
                title: window_title(&self.label),
                login: login.take(),
                error: error.take().map(str::to_owned),
                attempt,
                max_attempts: BASIC_MAX_ATTEMPTS,
            };
            let input = tokio::select! {
                input = context.ui.basic_prompt(spec) => input?,
                () = context.cancel.cancelled() => None,
            };
            let Some(input) = input else {
                return Err(LoginFailure::Cancelled);
            };
            if basic_authorization(&input.login, &input.password).is_err() {
                error = Some(INVALID);
                login = Some(input.login);
                continue;
            }
            if let Some(url) = &self.probe_url {
                let outcome = tokio::select! {
                    outcome = self.probe.check(&self.session, url, &input.login, &input.password) => outcome,
                    () = context.cancel.cancelled() => return Err(LoginFailure::Cancelled),
                };
                match outcome {
                    ProbeOutcome::Accepted => {}
                    ProbeOutcome::Refused => {
                        tracing::info!(
                            session = self.session.as_str(),
                            attempt,
                            "identifiants Basic refusés par l'adresse de contrôle"
                        );
                        error = Some(REFUSED);
                        login = Some(input.login);
                        continue;
                    }
                    ProbeOutcome::Failed(failure) => return Err(LoginFailure::Probe(failure.to_string())),
                }
            }
            let now = context.clock.now();
            return Ok(Obtained {
                credential: Credential::Basic {
                    login: input.login,
                    password: input.password,
                },
                expires_at: now.checked_add(self.ttl).unwrap_or(now),
            });
        }
        Err(LoginFailure::TooManyAttempts(BASIC_MAX_ATTEMPTS))
    }
}
