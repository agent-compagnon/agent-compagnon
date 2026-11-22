use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::header::{
    AUTHORIZATION, COOKIE, HeaderName, HeaderValue, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, SET_COOKIE,
    WWW_AUTHENTICATE,
};
use reqwest::{Method, Request, Response, redirect};
use url::Url;
use zeroize::Zeroizing;

use super::credential::ExposedCredential;
use super::error::SecretError;
use super::origin::OriginGuard;
use super::store::SecretStore;
use crate::config::SessionId;

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
/// Each request is checked against the session's origins, then the credential is injected into the headers right
/// before sending. Redirects are never followed and TLS certificates are always verified (native TLS: the Windows
/// certificate store on Windows).
pub struct AuthenticatedHttpClient {
    session: SessionId,
    store: Arc<SecretStore>,
    guard: Arc<OriginGuard>,
    remote: reqwest::Client,
    /// Never goes through a proxy, so that a plain-`http` loopback request is not relayed elsewhere.
    loopback: reqwest::Client,
    timeout: Duration,
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1")
}

/// Builds the authentication header. The returned value is marked sensitive (`Debug` hides it).
fn auth_header(credential: ExposedCredential<'_>) -> Result<(HeaderName, HeaderValue), SecretError> {
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

fn transport_error(error: &reqwest::Error) -> SecretError {
    if error.is_timeout() {
        SecretError::Timeout
    } else if error.is_connect() {
        SecretError::Connect
    } else {
        SecretError::Transport
    }
}

impl AuthenticatedHttpClient {
    /// `timeout` bounds every request (connection included); a request may ask for a shorter one.
    pub fn new(
        session: SessionId,
        store: Arc<SecretStore>,
        guard: Arc<OriginGuard>,
        timeout: Duration,
    ) -> Result<Self, SecretError> {
        if !guard.knows(&session) {
            return Err(SecretError::UnknownSession(session));
        }
        let builder = || {
            reqwest::Client::builder()
                .tls_backend_native()
                .redirect(redirect::Policy::none())
                .referer(false)
                .connect_timeout(timeout)
        };
        Ok(Self {
            remote: builder().build().map_err(|_| SecretError::ClientInit)?,
            loopback: builder().no_proxy().build().map_err(|_| SecretError::ClientInit)?,
            session,
            store,
            guard,
            timeout,
        })
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

    /// Sends `request` with the session's credential. Fails before sending anything if the destination is not one
    /// of the session's origins or if the session has no secret. `Authorization` and `Cookie` headers set by the
    /// caller are replaced.
    pub async fn execute(&self, mut request: Request) -> Result<Response, SecretError> {
        self.guard.authorize(&self.session, request.url())?;
        let (name, value) = self.store.with_exposed(&self.session, auth_header)??;
        let headers = request.headers_mut();
        headers.remove(AUTHORIZATION);
        headers.remove(COOKIE);
        headers.insert(name, value);
        let timeout = request.timeout().map_or(self.timeout, |t| (*t).min(self.timeout));
        *request.timeout_mut() = Some(timeout);

        let client = if is_loopback(request.url()) {
            &self.loopback
        } else {
            &self.remote
        };
        let mut response = client.execute(request).await.map_err(|e| transport_error(&e))?;
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
