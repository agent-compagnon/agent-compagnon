//! Downloads of the updater: anonymous (D11), through a client of their own, never the authenticated client of the
//! sessions. HTTPS with the native TLS of the system (the certificate store of Windows), plain HTTP towards the
//! loopback only (tests, like `update.base_url`); no cookie, no credential, no `Referer`; redirects followed within
//! the same origin only; size and time bounded.
//!
//! Errors name what happened, never the URL nor anything of the answer.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::redirect;
use url::Url;

/// How much of an answer is read, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_bytes: u64,
    /// The whole request, body included.
    pub timeout: Duration,
}

/// Why a download failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// Neither `https` nor `http` towards the loopback.
    Insecure,
    /// A redirect towards another origin (or a chain of more than [`MAX_REDIRECTS`]): not followed.
    Redirected,
    Status(u16),
    TooLarge,
    Timeout,
    Connect,
    /// The connection broke, or the body ended before its announced length.
    Transport,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Insecure => f.write_str("adresse non HTTPS refusée"),
            Self::Redirected => f.write_str("redirection vers une autre origine refusée"),
            Self::Status(status) => write!(f, "réponse HTTP {status}"),
            Self::TooLarge => f.write_str("réponse plus grande que la limite"),
            Self::Timeout => f.write_str("délai dépassé"),
            Self::Connect => f.write_str("Artifactory injoignable"),
            Self::Transport => f.write_str("échange interrompu"),
        }
    }
}

/// An anonymous `GET`. Injected, so that the updater is tested without a network.
#[async_trait]
pub trait Fetch: Send + Sync {
    /// The body of a 2xx answer, at most `limits.max_bytes` long.
    async fn get(&self, url: &Url, limits: Limits) -> Result<Vec<u8>, FetchError>;
}

pub const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest silence while reading an answer.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1")
}

/// `https`, or `http` towards the loopback.
pub fn allowed(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => is_loopback(url),
        _ => false,
    }
}

/// The real client: `reqwest` with native TLS, built for the updater alone.
pub struct AnonymousClient {
    remote: reqwest::Client,
    /// The loopback never goes through a proxy.
    loopback: reqwest::Client,
}

impl fmt::Debug for AnonymousClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnonymousClient").finish_non_exhaustive()
    }
}

/// Follows a redirect only to the origin of the first request, scheme and port included, a few times at most.
fn same_origin_policy() -> redirect::Policy {
    redirect::Policy::custom(|attempt| {
        let same = attempt
            .previous()
            .first()
            .is_some_and(|first| first.origin() == attempt.url().origin());
        if same && attempt.previous().len() <= MAX_REDIRECTS {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

impl AnonymousClient {
    pub fn new() -> Result<Self, FetchError> {
        let builder = || {
            reqwest::Client::builder()
                .tls_backend_native()
                .redirect(same_origin_policy())
                .referer(false)
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
        };
        Ok(Self {
            remote: builder().build().map_err(|_| FetchError::Transport)?,
            loopback: builder().no_proxy().build().map_err(|_| FetchError::Transport)?,
        })
    }
}

fn classify(error: &reqwest::Error) -> FetchError {
    if error.is_timeout() {
        FetchError::Timeout
    } else if error.is_connect() {
        FetchError::Connect
    } else {
        FetchError::Transport
    }
}

#[async_trait]
impl Fetch for AnonymousClient {
    async fn get(&self, url: &Url, limits: Limits) -> Result<Vec<u8>, FetchError> {
        if !allowed(url) {
            return Err(FetchError::Insecure);
        }
        let client = if is_loopback(url) { &self.loopback } else { &self.remote };
        let mut response = client
            .get(url.clone())
            .timeout(limits.timeout)
            .send()
            .await
            .map_err(|e| classify(&e))?;
        let status = response.status();
        if status.is_redirection() {
            return Err(FetchError::Redirected);
        }
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        if !allowed(response.url()) {
            return Err(FetchError::Insecure);
        }
        if response.content_length().is_some_and(|len| len > limits.max_bytes) {
            return Err(FetchError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| classify(&e))? {
            if body.len() as u64 + chunk.len() as u64 > limits.max_bytes {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}
