//! The network of the diagnosis: a client of its own, never the authenticated client of the sessions nor the one of
//! the updater. Anonymous (no cookie store, no credential, no `Referer`, an address with credentials refused), no
//! redirect followed (a redirect is an answer like any other), TLS native and verified, time bounded. HTTPS, or HTTP
//! towards the loopback only (tests).
//!
//! Errors say what happened, never the address nor anything of the answer.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::redirect;
use url::Url;

/// Connection, TLS included.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A whole request, the body read included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// An HTTP answer, whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    /// The `Date` header, as sent.
    pub date: Option<String>,
    /// At most the length asked; empty when none was asked.
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebError {
    /// Neither `https` nor `http` towards the loopback, or an address with credentials.
    Refused,
    /// The name did not resolve.
    Dns,
    /// The TLS handshake failed: certificate refused, protocol…
    Tls,
    /// No connection.
    Connect,
    Timeout,
    TooLarge,
    /// The exchange broke.
    Transport,
}

impl fmt::Display for WebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Refused => "adresse refusée (ni HTTPS, ni sans identifiant)",
            Self::Dns => "nom introuvable (résolution DNS en échec)",
            Self::Tls => "connexion sécurisée (TLS) refusée",
            Self::Connect => "connexion impossible",
            Self::Timeout => "délai dépassé",
            Self::TooLarge => "réponse plus grande que la limite",
            Self::Transport => "échange interrompu",
        })
    }
}

/// What the network checks ask of the network. Injected: the tests answer for it.
#[async_trait]
pub trait Web: Send + Sync {
    /// Resolves `host` to at least one address.
    async fn resolve(&self, host: &str, port: u16) -> Result<(), WebError>;

    /// A `GET` of `url`, anonymous, no redirect followed. The body is read up to `max_body` bytes (more is
    /// [`WebError::TooLarge`]); `0`: not read at all.
    async fn get(&self, url: &Url, max_body: u64) -> Result<Answer, WebError>;
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1")
}

/// `https`, or `http` towards the loopback; never an address that carries credentials.
fn allowed(url: &Url) -> bool {
    let scheme = match url.scheme() {
        "https" => true,
        "http" => is_loopback(url),
        _ => false,
    };
    scheme && url.username().is_empty() && url.password().is_none()
}

/// The kind of a failed request, from what the TLS and connection layers say: the TLS libraries of each platform
/// have their own error types, which only their texts tell apart. The text is never shown.
pub fn classify_text(chain: &str) -> WebError {
    let text = chain.to_lowercase();
    let tls = [
        "certificat",
        "certificate",
        "tls",
        "ssl",
        "handshake",
        "schannel",
        "trust",
    ];
    if tls.iter().any(|word| text.contains(word)) {
        WebError::Tls
    } else if text.contains("dns") || text.contains("resolve") || text.contains("no such host") {
        WebError::Dns
    } else {
        WebError::Connect
    }
}

fn classify(error: &reqwest::Error) -> WebError {
    if error.is_timeout() {
        return WebError::Timeout;
    }
    if error.is_connect() {
        let mut chain = String::new();
        let mut source: Option<&dyn std::error::Error> = Some(error);
        while let Some(current) = source {
            chain.push_str(&current.to_string());
            chain.push('\n');
            source = current.source();
        }
        return classify_text(&chain);
    }
    WebError::Transport
}

/// The real client of the diagnosis.
pub struct AnonymousWeb {
    remote: reqwest::Client,
    /// The loopback never goes through a proxy.
    loopback: reqwest::Client,
}

impl fmt::Debug for AnonymousWeb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnonymousWeb").finish_non_exhaustive()
    }
}

impl AnonymousWeb {
    pub fn new() -> Result<Self, WebError> {
        let builder = || {
            reqwest::Client::builder()
                .tls_backend_native()
                .redirect(redirect::Policy::none())
                .referer(false)
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
        };
        Ok(Self {
            remote: builder().build().map_err(|_| WebError::Transport)?,
            loopback: builder().no_proxy().build().map_err(|_| WebError::Transport)?,
        })
    }
}

#[async_trait]
impl Web for AnonymousWeb {
    async fn resolve(&self, host: &str, port: u16) -> Result<(), WebError> {
        let found = tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| WebError::Timeout)?;
        match found.map(|mut addresses| addresses.next()) {
            Ok(Some(_)) => Ok(()),
            _ => Err(WebError::Dns),
        }
    }

    async fn get(&self, url: &Url, max_body: u64) -> Result<Answer, WebError> {
        if !allowed(url) {
            return Err(WebError::Refused);
        }
        let client = if is_loopback(url) { &self.loopback } else { &self.remote };
        let mut response = client.get(url.clone()).send().await.map_err(|e| classify(&e))?;
        let status = response.status().as_u16();
        let date = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|d| d.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        if max_body > 0 {
            if response.content_length().is_some_and(|len| len > max_body) {
                return Err(WebError::TooLarge);
            }
            while let Some(chunk) = response.chunk().await.map_err(|e| classify(&e))? {
                if body.len() as u64 + chunk.len() as u64 > max_body {
                    return Err(WebError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
        }
        Ok(Answer { status, date, body })
    }
}

/// No client could be built: every network check fails, and says so.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoNetwork;

#[async_trait]
impl Web for NoNetwork {
    async fn resolve(&self, _host: &str, _port: u16) -> Result<(), WebError> {
        Err(WebError::Transport)
    }

    async fn get(&self, _url: &Url, _max_body: u64) -> Result<Answer, WebError> {
        Err(WebError::Transport)
    }
}
