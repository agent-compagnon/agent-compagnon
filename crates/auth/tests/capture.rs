//! Checks of the values a login window hands over, without any window.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::jwt;
use mcp_proxy_auth::capture::{
    CaptureRejection, JwtExpectations, JwtRejection, MAX_CANDIDATE_LEN, check_jwt, url_matches,
};
use mcp_proxy_auth::method::JwtHeaderMethod;
use mcp_proxy_core::config::UrlPattern;
use mcp_proxy_core::config::{AuthConfig, Config, SessionId};
use serde_json::json;
use url::Url;

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

const NOW: u64 = 1_795_356_187;

fn expect(audience: Option<&str>, issuer: Option<&str>) -> JwtExpectations {
    JwtExpectations {
        audience: audience.map(str::to_owned),
        issuer: issuer.map(str::to_owned),
    }
}

// Exigence : EX-010-02
#[test]
fn malformed_tokens_are_refused() {
    let none = expect(None, None);
    let now = at(NOW);
    for bad in [
        "",
        "abc",
        "a.b",
        "a.b.c.d",
        "..",
        "a..c",
        "e30.e30.",
        "a b.e30.c2ln",
        "e30.e30.c2l=",
        "+/.e30.c2ln",
    ] {
        assert_eq!(check_jwt(bad, now, &none), Err(JwtRejection::Structure), "{bad:?}");
    }
    // Header and payload must be JSON objects; the header names its algorithm.
    let header = |h: &str| format!("{}.{}.c2ln", b64(h), b64(&format!(r#"{{"exp":{}}}"#, NOW + 3600)));
    assert_eq!(check_jwt(&header("[]"), now, &none), Err(JwtRejection::Header));
    assert_eq!(check_jwt(&header("{}"), now, &none), Err(JwtRejection::Header));
    assert_eq!(
        check_jwt(&header(r#"{"alg":1}"#), now, &none),
        Err(JwtRejection::Header)
    );
    assert_eq!(check_jwt(&header("not json"), now, &none), Err(JwtRejection::Header));
    let payload = |p: &str| format!("{}.{}.c2ln", b64(r#"{"alg":"RS256"}"#), b64(p));
    assert_eq!(check_jwt(&payload("not json"), now, &none), Err(JwtRejection::Payload));
    assert_eq!(check_jwt(&payload("42"), now, &none), Err(JwtRejection::Payload));
    assert_eq!(
        check_jwt(&payload(r#"{"exp":"soon"}"#), now, &none),
        Err(JwtRejection::Payload)
    );
    assert_eq!(
        check_jwt(&payload(r#"{"sub":"x"}"#), now, &none),
        Err(JwtRejection::NoExpiry)
    );
}

fn b64(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text)
}

// Exigence : EX-010-02
#[test]
fn expiry_audience_and_issuer_are_checked() {
    let now = at(NOW);
    let corp = expect(Some("api://internal"), Some("https://login.example.internal"));
    let token = |claims| jwt(claims);
    let good = json!({ "exp": NOW + 3600, "aud": "api://internal", "iss": "https://login.example.internal" });
    assert_eq!(check_jwt(&token(good), now, &corp).unwrap().expires_at, at(NOW + 3600));
    // Within the 60 s margin, or past: expired.
    for exp in [NOW - 1, NOW, NOW + 60] {
        let claims = json!({ "exp": exp, "aud": "api://internal", "iss": "https://login.example.internal" });
        assert_eq!(
            check_jwt(&token(claims), now, &corp),
            Err(JwtRejection::Expired),
            "{exp}"
        );
    }
    let claims =
        json!({ "exp": NOW + 3600, "nbf": NOW + 10, "aud": "api://internal", "iss": "https://login.example.internal" });
    assert_eq!(check_jwt(&token(claims), now, &corp), Err(JwtRejection::NotYetValid));
    for aud in [json!("api://other"), json!(["api://other"]), json!(null), json!(42)] {
        let claims = json!({ "exp": NOW + 3600, "aud": aud, "iss": "https://login.example.internal" });
        assert_eq!(check_jwt(&token(claims), now, &corp), Err(JwtRejection::Audience));
    }
    let claims = json!({ "exp": NOW + 3600, "iss": "https://login.example.internal" });
    assert_eq!(
        check_jwt(&token(claims), now, &corp),
        Err(JwtRejection::Audience),
        "absent audience"
    );
    let claims =
        json!({ "exp": NOW + 3600, "aud": ["api://other", "api://internal"], "iss": "https://login.example.internal" });
    assert!(check_jwt(&token(claims), now, &corp).is_ok(), "audience among several");
    let claims = json!({ "exp": NOW + 3600, "aud": "api://internal", "iss": "https://evil.example" });
    assert_eq!(check_jwt(&token(claims), now, &corp), Err(JwtRejection::Issuer));
    let claims = json!({ "exp": NOW + 3600, "aud": "api://internal" });
    assert_eq!(
        check_jwt(&token(claims), now, &corp),
        Err(JwtRejection::Issuer),
        "absent issuer"
    );
    // Nothing expected: neither is looked at.
    let claims = json!({ "exp": NOW + 3600 });
    assert!(check_jwt(&token(claims), now, &expect(None, None)).is_ok());
}

fn pattern(text: &str) -> UrlPattern {
    let (prefix, wildcard) = match text.strip_suffix('*') {
        Some(prefix) => (prefix, true),
        None => (text, false),
    };
    UrlPattern {
        prefix: Url::parse(prefix).unwrap(),
        wildcard,
    }
}

// Exigence : EX-010-02
#[test]
fn url_patterns_match_on_the_canonical_prefix() {
    let api = pattern("https://api.example.internal/*");
    for good in [
        "https://api.example.internal/",
        "https://api.example.internal/v1/me?x=1",
    ] {
        assert!(url_matches(&api, &Url::parse(good).unwrap()), "{good}");
    }
    for bad in [
        "https://api.example.internal.evil/",
        "http://api.example.internal/",
        "https://api.example.internal:8443/",
        "https://user:pw@api.example.internal/",
        "https://other.example.internal/",
    ] {
        assert!(!url_matches(&api, &Url::parse(bad).unwrap()), "{bad}");
    }
    let exact = pattern("https://apps.example.internal/done");
    assert!(url_matches(
        &exact,
        &Url::parse("https://apps.example.internal/done").unwrap()
    ));
    assert!(!url_matches(
        &exact,
        &Url::parse("https://apps.example.internal/done/more").unwrap()
    ));
}

// Exigence : EX-010-02
#[test]
fn a_captured_header_must_match_url_header_and_scheme() {
    let config = Config::embedded(&["jira-local"]).unwrap();
    let session = config.session(&SessionId::new("sso-corp").unwrap()).unwrap();
    assert!(matches!(session.auth, AuthConfig::JwtHeader { .. }));
    let method = JwtHeaderMethod::from_config(session).unwrap();
    let capture = method_capture(&method);
    let url = Url::parse("https://api.example.internal/v1").unwrap();
    assert_eq!(capture.candidate_token(&url, "Authorization", "Bearer abc"), Ok("abc"));
    assert_eq!(capture.candidate_token(&url, "authorization", "bearer abc"), Ok("abc"));
    let outside = Url::parse("https://portal.example.internal/v1").unwrap();
    assert_eq!(
        capture.candidate_token(&outside, "Authorization", "Bearer abc"),
        Err(CaptureRejection::UrlOutsidePattern)
    );
    assert_eq!(
        capture.candidate_token(&url, "X-Token", "Bearer abc"),
        Err(CaptureRejection::WrongHeader)
    );
    for value in ["Basic abc", "Bearer", "Bearer ", "Bearerabc", "Bearer a b", "B"] {
        assert_eq!(
            capture.candidate_token(&url, "Authorization", value),
            Err(CaptureRejection::WrongScheme),
            "{value:?}"
        );
    }
    let long = format!("Bearer {}", "a".repeat(MAX_CANDIDATE_LEN));
    assert_eq!(
        capture.candidate_token(&url, "Authorization", &long),
        Err(CaptureRejection::TooLong)
    );
}

fn method_capture(method: &JwtHeaderMethod) -> mcp_proxy_auth::HeaderCapture {
    method.window_spec().capture().unwrap().clone()
}
