//! Check of Basic identifiers against the session's `probe_url`, before they are kept.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::secrets::{OriginGuard, basic_authorization};
use reqwest::StatusCode;
use reqwest::header::AUTHORIZATION;
use reqwest::redirect;
use secrecy::SecretString;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// 2xx.
    Accepted,
    /// 401 or 403: « identifiants refusés ».
    Refused,
    /// Anything else: the identifiers could not be checked, and are not kept.
    Failed(ProbeFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeFailure {
    /// `probe_url` is not one of the session's origins, or not `https`.
    Destination,
    /// The identifiers cannot be put in a header.
    Credential,
    Timeout,
    Connect,
    Transport,
    /// Any status other than 2xx, 401 and 403 (redirects are never followed).
    Status(u16),
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Destination => f.write_str("adresse de contrôle hors des origines de la session"),
            Self::Credential => f.write_str("identifiants impossibles à placer dans un en-tête"),
            Self::Timeout => f.write_str("délai dépassé"),
            Self::Connect => f.write_str("serveur de contrôle injoignable"),
            Self::Transport => f.write_str("échange en échec"),
            Self::Status(status) => write!(f, "réponse HTTP {status} inattendue"),
        }
    }
}

#[async_trait]
pub trait BasicProbe: Send + Sync {
    async fn check(&self, session: &SessionId, url: &Url, login: &str, password: &SecretString) -> ProbeOutcome;
}

/// A `GET` on `probe_url` with the candidate identifiers. Same rules as every request that carries a secret: the
/// destination must be one of the session's origins, redirects are never followed, TLS is native and verified, and a
/// loopback request never goes through a proxy.
pub struct HttpProbe {
    guard: Arc<OriginGuard>,
    remote: reqwest::Client,
    loopback: reqwest::Client,
    timeout: Duration,
}

impl fmt::Debug for HttpProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpProbe")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl HttpProbe {
    pub fn new(guard: Arc<OriginGuard>, timeout: Duration) -> Result<Self, reqwest::Error> {
        let builder = || {
            reqwest::Client::builder()
                .tls_backend_native()
                .redirect(redirect::Policy::none())
                .referer(false)
                .connect_timeout(timeout)
                .timeout(timeout)
        };
        Ok(Self {
            guard,
            remote: builder().build()?,
            loopback: builder().no_proxy().build()?,
            timeout,
        })
    }
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1")
}

#[async_trait]
impl BasicProbe for HttpProbe {
    async fn check(&self, session: &SessionId, url: &Url, login: &str, password: &SecretString) -> ProbeOutcome {
        if self.guard.authorize(session, url).is_err() {
            return ProbeOutcome::Failed(ProbeFailure::Destination);
        }
        let Ok(header) = basic_authorization(login, password) else {
            return ProbeOutcome::Failed(ProbeFailure::Credential);
        };
        let client = if is_loopback(url) { &self.loopback } else { &self.remote };
        let response = client.get(url.clone()).header(AUTHORIZATION, header).send().await;
        match response {
            Ok(response) => match response.status() {
                status if status.is_success() => ProbeOutcome::Accepted,
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProbeOutcome::Refused,
                status => ProbeOutcome::Failed(ProbeFailure::Status(status.as_u16())),
            },
            Err(error) if error.is_timeout() => ProbeOutcome::Failed(ProbeFailure::Timeout),
            Err(error) if error.is_connect() => ProbeOutcome::Failed(ProbeFailure::Connect),
            Err(_) => ProbeOutcome::Failed(ProbeFailure::Transport),
        }
    }
}
