//! Deposits that are ignored, and what the log says of them (spec 012). Alone in its test binary: `tracing` caches
//! the interest of a call site process-wide, and a test of another thread without a collector could hide its lines.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{SENTINEL, jwt};
use mcp_proxy_auth::method::{CookieMethod, JwtHeaderMethod};
use mcp_proxy_auth::webview::LoginWindows;
use mcp_proxy_auth::{LoginEvent, LoginWindowSpec};
use mcp_proxy_core::config::{AuthConfig, Config};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;

const NOW: u64 = 1_795_356_187;
const CORP_API: &str = "https://api.example.internal/v1/me";
const PORTAL: &str = "https://portal.example.internal/home";

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(NOW)
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn spec(id: &str) -> LoginWindowSpec {
    let mut config = Config::embedded(&["jira-local"]).unwrap();
    let session = config.sessions.iter_mut().find(|s| s.id.as_str() == id).unwrap();
    if let AuthConfig::JwtHeader { expected_issuer, .. } = &mut session.auth {
        *expected_issuer = Some("https://login.example.internal".to_owned());
        return JwtHeaderMethod::from_config(session).unwrap().window_spec();
    }
    CookieMethod::from_config(session).unwrap().window_spec()
}

fn corp_spec() -> LoginWindowSpec {
    spec("sso-corp")
}

fn dev_spec() -> LoginWindowSpec {
    spec("sso-dev")
}

fn token(claims: Value) -> String {
    jwt(claims)
}

fn valid_token() -> String {
    token(json!({
        "aud": "api://internal",
        "iss": "https://login.example.internal",
        "exp": NOW + 3600,
        "sub": "user",
    }))
}

fn secret(text: &str) -> Option<SecretString> {
    Some(SecretString::from(text.to_owned()))
}

/// Calling window, calling page, request URL, value.
type Case<'a> = (&'a str, Option<Url>, Option<String>, Option<String>);

fn drain(events: &mut UnboundedReceiver<LoginEvent>) -> Vec<LoginEvent> {
    let mut all = Vec::new();
    while let Ok(event) = events.try_recv() {
        all.push(event);
    }
    all
}

/// Collects every log line written on this thread while the guard lives.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Captured {
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(self.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

// Exigence : EX-012-09
#[test]
fn every_other_deposit_is_ignored_and_logged_without_the_value() {
    let logs = Captured::default();
    let _guard = logs.install();
    let windows = LoginWindows::default();
    let spec = corp_spec();
    let mut events = windows.open("login-1", &spec);
    let mut cookie_events = windows.open("login-2", &dev_spec());
    let good = valid_token();
    let bearer = |t: &str| format!("Bearer {t}");
    let claims = |extra: Value| {
        let mut base = json!({"aud": "api://internal", "iss": "https://login.example.internal", "exp": NOW + 3600, "sub": SENTINEL});
        base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        token(base)
    };
    let page = url(PORTAL);
    let sentinel_url = format!("{CORP_API}?access_token={SENTINEL}");
    let too_long = format!("Bearer {}{}", good, "A".repeat(16 * 1024));
    let cases: Vec<Case> = vec![
        // another window, or none of ours
        (
            "login-9",
            Some(page.clone()),
            Some(CORP_API.to_owned()),
            Some(bearer(&good)),
        ),
        (
            "main",
            Some(page.clone()),
            Some(CORP_API.to_owned()),
            Some(bearer(&good)),
        ),
        // a cookie window has no capture
        (
            "login-2",
            Some(page.clone()),
            Some(CORP_API.to_owned()),
            Some(bearer(&good)),
        ),
        // page outside the allowed hosts, or unknown
        (
            "login-1",
            Some(url("https://evil.example/")),
            Some(CORP_API.to_owned()),
            Some(bearer(&good)),
        ),
        (
            "login-1",
            Some(url("http://portal.example.internal/")),
            Some(CORP_API.to_owned()),
            Some(bearer(&good)),
        ),
        ("login-1", None, Some(CORP_API.to_owned()), Some(bearer(&good))),
        // value or URL missing, URL unreadable or outside the pattern
        ("login-1", Some(page.clone()), Some(CORP_API.to_owned()), None),
        ("login-1", Some(page.clone()), None, Some(bearer(&good))),
        (
            "login-1",
            Some(page.clone()),
            Some("/v1/me".to_owned()),
            Some(bearer(&good)),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(format!("https://evil.example/?t={SENTINEL}")),
            Some(bearer(&good)),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some("https://user:pw@api.example.internal/v1".to_owned()),
            Some(bearer(&good)),
        ),
        // scheme
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(format!("Basic {SENTINEL}")),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(good.clone()),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(format!("Bearer  {good}")),
        ),
        // structure, expiry, audience, issuer
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(SENTINEL)),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(&format!("{SENTINEL}.{SENTINEL}.{SENTINEL}"))),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(&claims(json!({"exp": NOW - 1})))),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(&claims(json!({"exp": NOW + 30})))),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(&claims(json!({"aud": "api://other"})))),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(&claims(json!({"iss": "https://evil.example"})))),
        ),
        (
            "login-1",
            Some(page.clone()),
            Some(sentinel_url.clone()),
            Some(bearer(&claims(json!({"nbf": NOW + 600})))),
        ),
        // too long, even with a valid token inside
        ("login-1", Some(page.clone()), Some(CORP_API.to_owned()), Some(too_long)),
    ];
    let count = cases.len();
    for (label, page, request, value) in cases {
        windows.deposit(
            label,
            page.as_ref(),
            request.as_deref(),
            value.as_deref().and_then(secret),
            now(),
        );
    }
    assert!(drain(&mut events).is_empty());
    assert!(drain(&mut cookie_events).is_empty());

    let text = logs.text();
    assert_eq!(text.matches("dépôt d'en-tête ignoré").count(), count, "{text}");
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(!text.contains(&good), "{text}");
    assert!(!text.contains("evil.example/?"), "{text}");
    for reason in [
        "fenêtre appelante autre que celle de la connexion en cours",
        "fenêtre sans capture d'en-tête",
        "page appelante hors des hôtes autorisés",
        "valeur absente ou non textuelle",
        "URL de la requête absente ou illisible",
        "URL hors du motif de capture",
        "schéma autre que celui attendu",
        "valeur trop longue",
        "jeton expiré",
        "audience inattendue",
        "émetteur inattendu",
        "pas encore valide",
        "structure invalide",
    ] {
        assert!(text.contains(reason), "{reason}: {text}");
    }

    // The window is still waiting: a valid deposit is retained afterwards.
    windows.deposit(
        "login-1",
        Some(&url(PORTAL)),
        Some(CORP_API),
        secret(&bearer(&good)),
        now(),
    );
    assert_eq!(drain(&mut events).len(), 1);
}
