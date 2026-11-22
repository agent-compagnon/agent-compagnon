use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::header::{
    AUTHORIZATION, COOKIE, HeaderName, HeaderValue, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, SET_COOKIE,
    WWW_AUTHENTICATE,
};
use reqwest::{Method, Request, Response, StatusCode, redirect};
use url::Url;
use zeroize::Zeroizing;

use super::credential::ExposedCredential;
use super::error::SecretError;
use super::origin::OriginGuard;
use super::store::SecretStore;
use crate::config::{Origin, SessionId};
use crate::session::{SessionError, SessionGate, StoreSessionGate};

/// Response headers that could carry or request credentials; never handed back to the caller.
fn stripped_response_headers() -> [HeaderName; 6] {
    [
        SET_COOKIE,
        AUTHORIZATION,
        WWW_AUTHENTICATE,
        PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION,
        HeaderName::from_static("set-cookie2"),
    ]
}

/// HTTP client bound to one session, given to local modules instead of the secret itself.
///
/// Each request is checked against the session's origins (and, if narrowed by `only_towards`, against that one
/// origin), then the credential is injected into the headers by the `SessionGate` right before sending. Nothing
/// gives the caller the injected header, the secret or its version. Redirects are never followed, TLS certificates
/// are always verified (native TLS: the Windows certificate store on Windows), `TRACE` and `CONNECT` are refused (a
/// `TRACE` answer echoes the request headers).
///
/// A 401 or 403 answer marks the secret that was sent rejected (`SessionGate::mark_rejected`) whatever the caller
/// then does with the response, and is counted for the `RejectionCounter` given to whoever built the client.
pub struct AuthenticatedHttpClient {
    session: SessionId,
    gate: Arc<dyn SessionGate>,
    only: Option<Origin>,
    remote: reqwest::Client,
    /// Never goes through a proxy, so that a plain-`http` loopback request is not relayed elsewhere.
    loopback: reqwest::Client,
    timeout: Duration,
    rejections: Arc<AtomicU64>,
}

/// How many answers refused the credential of one `AuthenticatedHttpClient`. Kept by whoever built the client (the
/// gateway), never reachable from the client itself: it tells a 401 or 403 apart from anything the client's user
/// reports.
#[derive(Debug, Clone)]
pub struct RejectionCounter(Arc<AtomicU64>);

impl RejectionCounter {
    pub fn count(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Methods a request may use. `TRACE` would echo the injected header back; `CONNECT` opens a tunnel.
const ALLOWED_METHODS: [Method; 7] = [
    Method::GET,
    Method::HEAD,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
    Method::OPTIONS,
];

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1")
}

/// Builds the authentication header. The returned value is marked sensitive (`Debug` hides it).
pub(crate) fn auth_header(credential: ExposedCredential<'_>) -> Result<(HeaderName, HeaderValue), SecretError> {
    let (name, raw) = match credential {
        ExposedCredential::Bearer { token } => {
            let mut raw = Zeroizing::new(Vec::with_capacity("Bearer ".len() + token.len()));
            raw.extend_from_slice(b"Bearer ");
            raw.extend_from_slice(token.as_bytes());
            (AUTHORIZATION, raw)
        }
        ExposedCredential::Cookies { pairs } => {
            let len = pairs.iter().map(|c| c.name.len() + 1 + c.value.len()).sum::<usize>() + 2 * pairs.len();
            let mut raw = Zeroizing::new(Vec::with_capacity(len));
            for (i, cookie) in pairs.iter().enumerate() {
                if i > 0 {
                    raw.extend_from_slice(b"; ");
                }
                raw.extend_from_slice(cookie.name.as_bytes());
                raw.push(b'=');
                raw.extend_from_slice(cookie.value.as_bytes());
            }
            (COOKIE, raw)
        }
        ExposedCredential::Basic { login, password } => {
            let mut user_pass = Zeroizing::new(Vec::with_capacity(login.len() + 1 + password.len()));
            user_pass.extend_from_slice(login.as_bytes());
            user_pass.push(b':');
            user_pass.extend_from_slice(password.as_bytes());
            let encoded_len = base64::encoded_len(user_pass.len(), true)
                .ok_or(SecretError::InvalidCredential("identifiants Basic trop longs"))?;
            let mut raw = Zeroizing::new(vec![0u8; "Basic ".len() + encoded_len]);
            let (prefix, encoded) = raw.split_at_mut("Basic ".len());
            prefix.copy_from_slice(b"Basic ");
            STANDARD
                .encode_slice(user_pass.as_slice(), encoded)
                .map_err(|_| SecretError::InvalidCredential("identifiants Basic trop longs"))?;
            (AUTHORIZATION, raw)
        }
    };
    let mut value = HeaderValue::from_bytes(&raw)
        .map_err(|_| SecretError::InvalidCredential("valeur impossible à placer dans un en-tête HTTP"))?;
    value.set_sensitive(true);
    Ok((name, value))
}

/// `Authorization: Basic …` for a login and password not stored yet, such as candidate identifiers being checked
/// against a probe URL. Same encoding as the injection, value marked sensitive.
pub fn basic_authorization(login: &str, password: &secrecy::SecretString) -> Result<HeaderValue, SecretError> {
    use secrecy::ExposeSecret as _;
    let credential = super::credential::Credential::Basic {
        login: login.to_owned(),
        password: password.clone(),
    };
    credential.validate()?;
    auth_header(ExposedCredential::Basic {
        login,
        password: password.expose_secret(),
    })
    .map(|(_, value)| value)
}

fn transport_error(error: &reqwest::Error) -> SecretError {
    if error.is_timeout() {
        SecretError::Timeout
    } else if error.is_connect() {
        SecretError::Connect
    } else {
        SecretError::Transport
    }
}

fn injection_error(session: &SessionId, error: SessionError) -> SecretError {
    match error {
        SessionError::Injection(error) => error,
        _ => SecretError::NoSecret(session.clone()),
    }
}

impl AuthenticatedHttpClient {
    /// Over the store directly, without any login: `timeout` bounds every request (connection included); a request
    /// may ask for a shorter one.
    pub fn new(
        session: SessionId,
        store: Arc<SecretStore>,
        guard: Arc<OriginGuard>,
        timeout: Duration,
    ) -> Result<Self, SecretError> {
        if !guard.knows(&session) {
            return Err(SecretError::UnknownSession(session));
        }
        let gate: Arc<dyn SessionGate> = Arc::new(StoreSessionGate::new(store, guard));
        Self::through_gate(session, gate, timeout).map(|(client, _)| client)
    }

    /// Injecting through `gate`, as the gateway does for remote MCPs: the session's expiry and its rejections are
    /// the gate's. The counter of refused answers stays with the caller.
    pub fn through_gate(
        session: SessionId,
        gate: Arc<dyn SessionGate>,
        timeout: Duration,
    ) -> Result<(Self, RejectionCounter), SecretError> {
        let builder = || {
            reqwest::Client::builder()
                .tls_backend_native()
                .redirect(redirect::Policy::none())
                .referer(false)
                .connect_timeout(timeout)
        };
        let rejections = Arc::new(AtomicU64::new(0));
        let client = Self {
            remote: builder().build().map_err(|_| SecretError::ClientInit)?,
            loopback: builder().no_proxy().build().map_err(|_| SecretError::ClientInit)?,
            session,
            gate,
            only: None,
            timeout,
            rejections: Arc::clone(&rejections),
        };
        Ok((client, RejectionCounter(rejections)))
    }

    /// Narrows the destinations to `origin`, on top of the session's origins: a request elsewhere fails before
    /// anything is sent, even towards another origin of the session. Narrowing again replaces the origin only if
    /// none was set: a client can never be widened.
    #[must_use]
    pub fn only_towards(mut self, origin: Origin) -> Self {
        if self.only.is_none() {
            self.only = Some(origin);
        }
        self
    }

    pub fn session(&self) -> &SessionId {
        &self.session
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// A request to fill in and pass to `execute`.
    pub fn request(&self, method: Method, url: Url) -> Request {
        Request::new(method, url)
    }

    fn check(&self, request: &Request) -> Result<(), SecretError> {
        if !ALLOWED_METHODS.contains(request.method()) {
            return Err(SecretError::MethodNotAllowed {
                method: request.method().as_str().to_owned(),
            });
        }
        if let Some(only) = &self.only {
            let origin = Origin::of(request.url());
            if origin.as_ref() != Some(only) {
                return Err(SecretError::OriginNotAllowed {
                    session: self.session.clone(),
                    origin: origin.map_or_else(|| "(sans origine)".to_owned(), |o| o.to_string()),
                });
            }
        }
        Ok(())
    }

    /// Sends `request` with the session's credential. Fails before sending anything if the method is refused, if
    /// the destination is not allowed or if the session has no usable secret. `Authorization` and `Cookie` headers
    /// set by the caller are replaced.
    pub async fn execute(&self, mut request: Request) -> Result<Response, SecretError> {
        self.check(&request)?;
        let url = request.url().clone();
        let version = self
            .gate
            .inject(&self.session, &url, request.headers_mut())
            .map_err(|e| injection_error(&self.session, e))?;
        let timeout = request.timeout().map_or(self.timeout, |t| (*t).min(self.timeout));
        *request.timeout_mut() = Some(timeout);

        let client = if is_loopback(&url) {
            &self.loopback
        } else {
            &self.remote
        };
        let mut response = client.execute(request).await.map_err(|e| transport_error(&e))?;
        if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            self.gate.mark_rejected(&self.session, version);
            self.rejections.fetch_add(1, Ordering::SeqCst);
        }
        let headers = response.headers_mut();
        for name in stripped_response_headers() {
            headers.remove(name);
        }
        Ok(response)
    }
}

impl fmt::Debug for AuthenticatedHttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthenticatedHttpClient")
            .field("session", &self.session.as_str())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}
