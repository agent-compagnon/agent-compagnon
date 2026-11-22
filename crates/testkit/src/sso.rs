//! `FakeSso`: a local login site for the WebView2 login windows (spec 012) and their probe. A login page sets an
//! HttpOnly session cookie; application pages call an API with `Authorization: Bearer <jwt>`, by `fetch` or by
//! `XMLHttpRequest`, or try what a hostile page would try (other commands, forged deposits, leaving the allowed
//! hosts). Everything it receives is recorded for the test.

use std::collections::BTreeMap;
use std::hash::{BuildHasher as _, Hasher as _, RandomState};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
use http::{HeaderValue, StatusCode};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Name of the session cookie set by the login page (HttpOnly).
pub const FAKE_SSO_COOKIE: &str = "FAKE_SSO_SESSION";

/// Audience and issuer of every token of `FakeSso`.
pub const FAKE_SSO_AUDIENCE: &str = "api://fake-sso";
pub const FAKE_SSO_ISSUER: &str = "fake-sso";

/// The tokens of `FakeSso`, distinct so that a test knows which one was retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SsoToken {
    /// Sent by `fetch` on `/app/fetch`.
    Fetch,
    /// Sent by `XMLHttpRequest` on `/app/xhr`.
    Xhr,
    /// Valid, but only ever deposited by `/app/forged` with something wrong around it.
    Forged,
    /// Sent last by `/app/forged`, by a regular `fetch`: the only one that may be retained there.
    Final,
    /// Expired an hour ago.
    Expired,
    /// Meant for another audience.
    OtherAudience,
}

const TOKENS: [SsoToken; 6] = [
    SsoToken::Fetch,
    SsoToken::Xhr,
    SsoToken::Forged,
    SsoToken::Final,
    SsoToken::Expired,
    SsoToken::OtherAudience,
];

impl SsoToken {
    fn name(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Xhr => "xhr",
            Self::Forged => "forged",
            Self::Final => "final",
            Self::Expired => "expired",
            Self::OtherAudience => "other-audience",
        }
    }
}

/// A request to `/api/…` or `/other/…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCall {
    pub path: String,
    pub authorization: Option<String>,
}

#[derive(Default)]
struct SsoState {
    cookie: Option<String>,
    calls: Vec<ApiCall>,
    /// `(name, result)` reported by `/app/commands`.
    reports: Vec<(String, String)>,
    /// Requests to `/whoami`: whether they carried the session cookie set by `/login`.
    whoami: Vec<bool>,
    /// Requests per path.
    hits: BTreeMap<String, usize>,
}

fn lock(state: &Mutex<SsoState>) -> MutexGuard<'_, SsoState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn random_hex() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    format!(
        "{:016x}{:016x}",
        hasher.finish(),
        RandomState::new().build_hasher().finish()
    )
}

fn jwt(claims: &serde_json::Value) -> String {
    let segment = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.{}",
        segment(&json!({ "alg": "RS256", "typ": "JWT" })),
        segment(claims),
        URL_SAFE_NO_PAD.encode("fake-sso-signature")
    )
}

fn make_tokens(now: SystemTime) -> BTreeMap<SsoToken, String> {
    let now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    TOKENS
        .into_iter()
        .map(|token| {
            let (exp, aud) = match token {
                SsoToken::Expired => (now.saturating_sub(3600), FAKE_SSO_AUDIENCE),
                SsoToken::OtherAudience => (now + 3600, "api://other"),
                _ => (now + 3600, FAKE_SSO_AUDIENCE),
            };
            let claims = json!({
                "aud": aud,
                "iss": FAKE_SSO_ISSUER,
                "exp": exp,
                "iat": now,
                "sub": "probe",
                "jti": token.name(),
            });
            (token, jwt(&claims))
        })
        .collect()
}

struct Site {
    state: Mutex<SsoState>,
    tokens: BTreeMap<SsoToken, String>,
    port: u16,
}

impl Site {
    fn token(&self, token: SsoToken) -> &str {
        self.tokens.get(&token).map(String::as_str).unwrap_or_default()
    }

    /// The page script of `/app/{mode}`. Tokens are base64url and dots, the port a number: nothing to escape.
    fn app_script(&self, mode: &str) -> Option<String> {
        let bearer = |t: SsoToken| format!("Bearer {}", self.token(t));
        Some(match mode {
            "fetch" => format!(
                "fetch('/api/fetch', {{ headers: {{ 'Authorization': '{}' }} }});",
                bearer(SsoToken::Fetch)
            ),
            "xhr" => format!(
                "var x = new XMLHttpRequest(); x.open('GET', '/api/xhr'); \
                 x.setRequestHeader('Authorization', '{}'); x.send();",
                bearer(SsoToken::Xhr)
            ),
            "commands" => "(async function () {\n\
                   var internals = window.__TAURI_INTERNALS__;\n\
                   function report(name, result) {\n\
                     return fetch('/report?name=' + encodeURIComponent(name) + '&result=' + result);\n\
                   }\n\
                   if (!internals) { await report('ipc', 'absent'); return; }\n\
                   var commands = ['get_snapshot', 'submit_basic_credentials', 'plugin:event|listen'];\n\
                   for (var i = 0; i < commands.length; i++) {\n\
                     try { await internals.invoke(commands[i], {}); await report(commands[i], 'allowed'); }\n\
                     catch (e) { await report(commands[i], 'refused'); }\n\
                   }\n\
                   await report('done', 'yes');\n\
                 })();"
                .to_owned(),
            "forged" => format!(
                "(function () {{\n\
                   var internals = window.__TAURI_INTERNALS__;\n\
                   var origin = location.origin;\n\
                   function forge(url, value) {{\n\
                     try {{ internals.invoke('deposit_login_header', {{ url: url, value: value }}).catch(function () {{}}); }}\n\
                     catch (e) {{}}\n\
                   }}\n\
                   if (internals) {{\n\
                     forge(origin + '/other/forged', '{forged}');\n\
                     forge('https://evil.example/api/forged', '{forged}');\n\
                     forge(origin + '/api/forged', 'Basic {forged_raw}');\n\
                     forge(origin + '/api/forged', '{forged_raw}');\n\
                     forge(origin + '/api/forged', '{expired}');\n\
                     forge(origin + '/api/forged', '{other}');\n\
                     forge(origin + '/api/forged', 'Bearer ' + 'A'.repeat(17000));\n\
                   }}\n\
                   fetch('/other/forged', {{ headers: {{ 'Authorization': '{forged}' }} }});\n\
                   fetch('/api/forged', {{ headers: {{ 'Authorization': 'Basic {forged_raw}' }} }});\n\
                   setTimeout(function () {{\n\
                     fetch('/api/final', {{ headers: {{ 'Authorization': '{final_}' }} }});\n\
                   }}, 1500);\n\
                 }})();",
                forged = bearer(SsoToken::Forged),
                forged_raw = self.token(SsoToken::Forged),
                expired = bearer(SsoToken::Expired),
                other = bearer(SsoToken::OtherAudience),
                final_ = bearer(SsoToken::Final),
            ),
            "escape" => format!(
                "setTimeout(function () {{ location.href = 'http://localhost:{}/escaped'; }}, 300);",
                self.port
            ),
            _ => return None,
        })
    }
}

fn html(title: &str, script: Option<&str>) -> Response {
    let script = script.map(|s| format!("<script>{s}</script>")).unwrap_or_default();
    let body = format!(
        "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body><h1>{title}</h1>{script}</body></html>"
    );
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response
}

fn status(code: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = code;
    response
}

fn query_value(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

async fn handle(State(site): State<Arc<Site>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let header = |name| {
        request
            .headers()
            .get(name)
            .and_then(|v: &HeaderValue| v.to_str().ok())
            .map(str::to_owned)
    };
    let authorization = header(AUTHORIZATION);
    let cookies = header(COOKIE).unwrap_or_default();
    *lock(&site.state).hits.entry(path.clone()).or_default() += 1;

    if let Some(mode) = path.strip_prefix("/login/") {
        if site.app_script(mode).is_none() {
            return status(StatusCode::NOT_FOUND);
        }
        let value = random_hex();
        lock(&site.state).cookie = Some(value.clone());
        let mut response = status(StatusCode::SEE_OTHER);
        let cookie = format!("{FAKE_SSO_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax");
        if let (Ok(cookie), Ok(location)) = (
            HeaderValue::from_str(&cookie),
            HeaderValue::from_str(&format!("/app/{mode}")),
        ) {
            response.headers_mut().insert(SET_COOKIE, cookie);
            response.headers_mut().insert(LOCATION, location);
        }
        return response;
    }
    if let Some(mode) = path.strip_prefix("/app/") {
        return match site.app_script(mode) {
            Some(script) => html("Application FakeSso", Some(&script)),
            None => status(StatusCode::NOT_FOUND),
        };
    }
    if path.starts_with("/api/") || path.starts_with("/other/") {
        let expected = [SsoToken::Fetch, SsoToken::Xhr, SsoToken::Final].map(|t| format!("Bearer {}", site.token(t)));
        let ok = authorization.as_ref().is_some_and(|a| expected.contains(a));
        lock(&site.state).calls.push(ApiCall { path, authorization });
        return if ok {
            html("API", None)
        } else {
            status(StatusCode::UNAUTHORIZED)
        };
    }
    match path.as_str() {
        "/whoami" => {
            let mut state = lock(&site.state);
            let known = state.cookie.clone();
            let carried = known.is_some_and(|value| {
                cookies
                    .split(';')
                    .any(|pair| pair.trim() == format!("{FAKE_SSO_COOKIE}={value}"))
            });
            state.whoami.push(carried);
            html(
                if carried {
                    "Session reconnue"
                } else {
                    "Session inconnue"
                },
                None,
            )
        }
        "/report" => {
            let name = query_value(&query, "name").unwrap_or_default();
            let result = query_value(&query, "result").unwrap_or_default();
            lock(&site.state).reports.push((name, result));
            status(StatusCode::NO_CONTENT)
        }
        "/escaped" => html("Hors liste", None),
        _ => status(StatusCode::NOT_FOUND),
    }
}

/// The fake login site, on `127.0.0.1` with a free port, plain `http` (allowed towards the loopback only). Stops
/// when dropped.
///
/// - `/login/{mode}` sets the HttpOnly cookie [`FAKE_SSO_COOKIE`] and redirects to `/app/{mode}`;
/// - `/app/fetch` and `/app/xhr` call `/api/…` with `Authorization: Bearer <jwt>`;
/// - `/app/commands` tries other commands of the application and reports each result to `/report`;
/// - `/app/forged` deposits wrong values directly, sends requests the capture must not deposit, then, 1.5 s later,
///   calls `/api/final` with [`SsoToken::Final`];
/// - `/app/escape` navigates to `http://localhost:{port}/escaped`, outside `127.0.0.1`;
/// - `/whoami` records whether the session cookie came with the request.
pub struct FakeSso {
    addr: SocketAddr,
    site: Arc<Site>,
    ct: CancellationToken,
}

impl FakeSso {
    pub async fn start() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let site = Arc::new(Site {
            state: Mutex::default(),
            tokens: make_tokens(SystemTime::now()),
            port: addr.port(),
        });
        let router = Router::new().fallback(handle).with_state(Arc::clone(&site));
        let ct = CancellationToken::new();
        let shutdown = ct.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(Self { addr, site, ct })
    }

    /// `http://127.0.0.1:<port>`.
    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The host of every page, for `allowed_navigation`.
    pub fn host(&self) -> &'static str {
        "127.0.0.1"
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin())
    }

    /// Prefix of the capture pattern: `http://127.0.0.1:<port>/api/`.
    pub fn api_prefix(&self) -> String {
        self.url("/api/")
    }

    pub fn token(&self, token: SsoToken) -> String {
        self.site.token(token).to_owned()
    }

    /// The value of the session cookie set by the last `/login/…`.
    pub fn session_cookie(&self) -> Option<String> {
        lock(&self.site.state).cookie.clone()
    }

    pub fn api_calls(&self) -> Vec<ApiCall> {
        lock(&self.site.state).calls.clone()
    }

    pub fn reports(&self) -> Vec<(String, String)> {
        lock(&self.site.state).reports.clone()
    }

    /// For each request to `/whoami`, whether it carried the session cookie.
    pub fn whoami(&self) -> Vec<bool> {
        lock(&self.site.state).whoami.clone()
    }

    /// Requests received on `path`.
    pub fn hits(&self, path: &str) -> usize {
        lock(&self.site.state).hits.get(path).copied().unwrap_or(0)
    }
}

impl Drop for FakeSso {
    fn drop(&mut self) {
        self.ct.cancel();
    }
}
