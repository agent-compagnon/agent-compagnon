//! What the WebView2 login windows decide (spec 012), without any window.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{SENTINEL, jwt};
use mcp_proxy_auth::method::{CookieMethod, JwtHeaderMethod};
use mcp_proxy_auth::webview::{
    BasicPrompts, CAPTURE_SOURCE, DEPOSIT_COMMAND, LoginProfile, LoginWindows, MIN_RUNTIME_VERSION, OS_SSO_FEATURE,
    WRY_DEFAULT_ARGS, app_page_allowed, browser_args, capture_script, check_runtime_version, cookie_domain_matches,
    destination_for_log, js_data, navigation_allowed, parse_basic_answer, remote_url_patterns, reset_profile_dir,
};
use mcp_proxy_auth::{AuthUiError, LoginEvent, LoginWindowSpec};
use mcp_proxy_core::config::{AuthConfig, Config, WebviewConfig, WebviewProfile};
use proptest::prelude::*;
use secrecy::{ExposeSecret as _, SecretString};
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

fn config() -> Config {
    Config::embedded(&["jira-local"]).unwrap()
}

/// The window of `sso-corp` (`jwt_header`), with an expected issuer added to the embedded configuration.
fn corp_spec() -> LoginWindowSpec {
    let mut config = config();
    let session = config
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "sso-corp")
        .unwrap();
    if let AuthConfig::JwtHeader { expected_issuer, .. } = &mut session.auth {
        *expected_issuer = Some("https://login.example.internal".to_owned());
    }
    JwtHeaderMethod::from_config(session).unwrap().window_spec()
}

fn dev_spec() -> LoginWindowSpec {
    let config = config();
    let session = config.sessions.iter().find(|s| s.id.as_str() == "sso-dev").unwrap();
    CookieMethod::from_config(session).unwrap().window_spec()
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

fn drain(events: &mut UnboundedReceiver<LoginEvent>) -> Vec<LoginEvent> {
    let mut all = Vec::new();
    while let Ok(event) = events.try_recv() {
        all.push(event);
    }
    all
}

// --- Navigation and matching ----------------------------------------------------------------------------------

// Exigence : EX-012-02
#[test]
fn navigation_is_limited_to_the_exact_listed_hosts() {
    let allowed = corp_spec().allowed_navigation().to_vec();
    assert_eq!(allowed, ["portal.example.internal", "login.example.internal"]);
    for ok in [
        "https://portal.example.internal/",
        "https://portal.example.internal/a/b?c=d#e",
        "https://login.example.internal:8443/authorize",
        "https://PORTAL.example.internal/",
    ] {
        assert!(navigation_allowed(&allowed, &url(ok)), "{ok}");
    }
    for refused in [
        // subdomain, neighbours, parent
        "https://www.portal.example.internal/",
        "https://portal.example.internal.evil/",
        "https://evilportal.example.internal/",
        "https://example.internal/",
        "https://portal.example.internal./",
        // scheme
        "http://portal.example.internal/",
        "ftp://portal.example.internal/",
        "about:blank",
        "data:text/html,<p>x</p>",
        "javascript:alert(1)",
        "file:///C:/Windows/win.ini",
        // identifiers
        "https://user:pw@portal.example.internal/",
        "https://user@portal.example.internal/",
    ] {
        assert!(!navigation_allowed(&allowed, &url(refused)), "{refused}");
    }
    // `http` only towards the loopback, and only when it is listed.
    let local = vec!["127.0.0.1".to_owned(), "localhost".to_owned()];
    assert!(navigation_allowed(&local, &url("http://127.0.0.1:4310/login")));
    assert!(navigation_allowed(&local, &url("http://localhost:4310/login")));
    assert!(!navigation_allowed(&local, &url("http://127.0.0.2:4310/login")));
    assert!(!navigation_allowed(&allowed, &url("http://127.0.0.1:4310/login")));
    assert!(!navigation_allowed(&[], &url("https://portal.example.internal/")));
}

// Exigence : EX-012-12
#[test]
fn the_basic_window_stays_on_the_pages_of_the_application() {
    for ok in [
        "http://tauri.localhost/basic.html",
        "https://tauri.localhost/basic.html?x=1",
    ] {
        assert!(app_page_allowed(&url(ok)), "{ok}");
    }
    for refused in [
        "http://tauri.localhost.evil/basic.html",
        "http://evil.tauri.localhost/basic.html",
        "http://tauri.localhost:8080/basic.html",
        "http://user@tauri.localhost/basic.html",
        "https://portal.example.internal/",
        "tauri://localhost/basic.html",
        "about:blank",
    ] {
        assert!(!app_page_allowed(&url(refused)), "{refused}");
    }
}

// Exigence : EX-012-02
#[test]
fn capture_matches_url_and_header_of_the_window() {
    let spec = corp_spec();
    let capture = spec.capture().unwrap();
    let value = format!("Bearer {}", valid_token());
    assert!(capture.candidate_token(&url(CORP_API), "authorization", &value).is_ok());
    assert!(capture.candidate_token(&url(CORP_API), "AUTHORIZATION", &value).is_ok());
    assert!(
        capture
            .candidate_token(&url(CORP_API), "X-Authorization", &value)
            .is_err()
    );
    assert!(
        capture
            .candidate_token(&url("https://api.example.internal.evil/v1"), "Authorization", &value)
            .is_err()
    );
    assert!(
        capture
            .candidate_token(&url("https://www.api.example.internal/v1"), "Authorization", &value)
            .is_err()
    );
    assert!(
        capture
            .candidate_token(&url("http://api.example.internal/v1"), "Authorization", &value)
            .is_err()
    );
    // A cookie window captures nothing.
    assert!(dev_spec().capture().is_none());
}

// Exigence : EX-012-02
#[test]
fn logged_destinations_and_cleared_cookies_follow_the_hosts() {
    assert_eq!(
        destination_for_log(&url("https://evil.example/path?access_token=abc#frag")),
        "https://evil.example"
    );
    assert_eq!(
        destination_for_log(&url("http://127.0.0.1:8080/x")),
        "http://127.0.0.1:8080"
    );
    assert_eq!(destination_for_log(&url("data:text/html,secret")), "data:");

    let hosts = vec![
        "sso.dev.example.internal".to_owned(),
        "apps.dev.example.internal".to_owned(),
    ];
    for ok in [
        "sso.dev.example.internal",
        ".sso.dev.example.internal",
        "dev.example.internal",
        ".example.internal",
    ] {
        assert!(cookie_domain_matches(ok, &hosts), "{ok}");
    }
    for refused in [
        "",
        ".",
        "evil.example",
        "sso.dev.example.internal.evil",
        "v.example.internal",
        "x.sso.dev.example.internal",
    ] {
        assert!(!cookie_domain_matches(refused, &hosts), "{refused}");
    }
}

// Exigence : EX-012-13
#[test]
fn deposit_capability_covers_the_listed_hosts_only() {
    let allowed = corp_spec().allowed_navigation().to_vec();
    assert_eq!(
        remote_url_patterns(&allowed).unwrap(),
        ["https://portal.example.internal:*", "https://login.example.internal:*"]
    );
    assert_eq!(
        remote_url_patterns(&["127.0.0.1".to_owned(), "localhost".to_owned()]).unwrap(),
        [
            "https://127.0.0.1:*",
            "http://127.0.0.1:*",
            "https://localhost:*",
            "http://localhost:*",
        ]
    );
    // Nothing that the pattern syntax would interpret: no capability, no window.
    for bad in [
        "*",
        "a*.example",
        "a.example:443",
        "a(b).example",
        "{a}.example",
        "a..example",
        "-a.example",
        "A.example",
        "[::1]",
        "a.example/",
        "",
    ] {
        assert_eq!(remote_url_patterns(&[bad.to_owned()]), None, "{bad}");
        assert_eq!(
            remote_url_patterns(&["ok.example".to_owned(), bad.to_owned()]),
            None,
            "{bad}"
        );
    }
    assert_eq!(remote_url_patterns(&[]), None);
}

// --- Deposits -------------------------------------------------------------------------------------------------

// Exigence : EX-012-09
#[test]
fn a_valid_deposit_of_the_current_window_is_retained_once() {
    let windows = LoginWindows::default();
    let spec = corp_spec();
    let mut events = windows.open("login-1", &spec);
    let value = format!("Bearer {}", valid_token());
    windows.deposit("login-1", Some(&url(PORTAL)), Some(CORP_API), secret(&value), now());
    let mut seen = drain(&mut events);
    assert_eq!(seen.len(), 1);
    match seen.remove(0) {
        LoginEvent::HeaderCaptured {
            url: u,
            header,
            value: v,
        } => {
            assert_eq!(u.as_str(), CORP_API);
            assert_eq!(header, "Authorization");
            assert_eq!(v.expose_secret(), value);
        }
        other => panic!("{other:?}"),
    }
    // Only the first valid token of the window counts.
    let other = format!(
        "Bearer {}",
        token(json!({"aud": "api://internal", "iss": "https://login.example.internal", "exp": NOW + 7200}))
    );
    windows.deposit("login-1", Some(&url(PORTAL)), Some(CORP_API), secret(&other), now());
    assert!(drain(&mut events).is_empty());
}

// Exigence : EX-012-09
#[test]
fn navigations_and_closing_reach_the_window_of_the_session() {
    let windows = LoginWindows::default();
    let mut events = windows.open("login-1", &dev_spec());
    windows.navigated("login-1", &url("https://apps.dev.example.internal/home"));
    windows.navigated("login-1", &url("https://evil.example/home"));
    windows.navigated("login-7", &url("https://apps.dev.example.internal/home"));
    let seen = drain(&mut events);
    assert_eq!(seen.len(), 1);
    assert!(
        matches!(&seen[0], LoginEvent::Navigated { url } if url.as_str() == "https://apps.dev.example.internal/home")
    );

    windows.closed("login-1");
    assert!(matches!(drain(&mut events).as_slice(), [LoginEvent::Closed]));
    assert!(!windows.is_open("login-1"));
    // A closed window cannot deposit any more, even under its old label.
    windows.deposit("login-1", Some(&url(PORTAL)), Some(CORP_API), secret("Bearer x"), now());
    assert!(events.try_recv().is_err());

    // Closed by the application: no event at all.
    let mut events = windows.open("login-3", &corp_spec());
    windows.forget("login-3");
    assert!(drain(&mut events).is_empty());
    assert!(!windows.is_open("login-3"));
}

// --- Basic window ---------------------------------------------------------------------------------------------

// Exigence : EX-012-12
#[tokio::test]
async fn basic_answers_come_from_the_window_of_the_prompt_once() {
    let prompts = BasicPrompts::default();
    let answer = prompts.open("basic-1");
    assert!(!prompts.submit("basic-2", None));
    assert!(!prompts.submit("main", None));
    let input = parse_basic_answer(Some(json!({"login": "alice", "password": SENTINEL}))).unwrap();
    assert!(prompts.submit("basic-1", input));
    let input = answer.await.unwrap().unwrap();
    assert_eq!(input.login, "alice");
    assert_eq!(input.password.expose_secret(), SENTINEL);
    assert!(!format!("{input:?}").contains(SENTINEL));
    // A second answer of the same window is refused.
    assert!(!prompts.submit("basic-1", None));

    let answer = prompts.open("basic-3");
    prompts.closed("basic-3");
    assert!(answer.await.unwrap().is_none());
}

// Exigence : EX-012-12
#[test]
fn basic_answers_are_parsed_strictly() {
    assert!(parse_basic_answer(None).unwrap().is_none());
    assert!(parse_basic_answer(Some(Value::Null)).unwrap().is_none());
    for bad in [
        json!("alice"),
        json!(42),
        json!([]),
        json!({"login": "alice"}),
        json!({"password": "pw"}),
        json!({"login": 1, "password": "pw"}),
        json!({"login": "alice", "password": 1}),
        json!({"login": "alice", "password": "pw", "extra": true}),
    ] {
        assert!(parse_basic_answer(Some(bad.clone())).is_err(), "{bad}");
    }
}

// --- Runtime and profile --------------------------------------------------------------------------------------

// Exigence : EX-012-10
#[test]
fn runtime_version_must_support_inprivate() {
    assert_eq!(MIN_RUNTIME_VERSION, [101, 0, 1210, 39]);
    for ok in [
        "101.0.1210.39",
        "101.0.1210.40",
        "101.0.1211.0",
        "101.1.0.0",
        "131.0.2903.70",
        "121.0.2277.4 beta",
        "1000.0.0.0",
    ] {
        assert!(check_runtime_version(Some(ok)).is_ok(), "{ok}");
    }
    for refused in [
        "101.0.1210.38",
        "101.0.1209.99",
        "100.9999.9999.9999",
        "86.0.622.38",
        "",
        " ",
        "inconnue",
        "131.0.2903",
        "131.0.2903.70.1",
        "131..2903.70",
        "131.0.2903.-70",
        "+131.0.2903.70",
        "131.0.2903.7a",
        "99999999999999999999999.0.0.0",
    ] {
        assert_eq!(
            check_runtime_version(Some(refused)),
            Err(AuthUiError::RuntimeTooOld),
            "{refused:?}"
        );
    }
    assert_eq!(check_runtime_version(None), Err(AuthUiError::RuntimeTooOld));
}

// Exigence : EX-012-10
#[test]
fn browser_arguments_add_os_sso_only_when_asked() {
    assert_eq!(
        WRY_DEFAULT_ARGS,
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"
    );
    assert_eq!(browser_args(false), WRY_DEFAULT_ARGS);
    assert_eq!(
        browser_args(true),
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
         --enable-features=msSingleSignOnOSForPrimaryAccountIsShared"
    );
    assert!(browser_args(true).ends_with(OS_SSO_FEATURE));

    let dir = std::path::PathBuf::from("webview");
    let embedded = config();
    let profile = LoginProfile::from_config(&embedded.webview, dir.clone());
    assert_eq!(
        profile,
        LoginProfile {
            data_directory: dir.clone(),
            browser_args: WRY_DEFAULT_ARGS.to_owned(),
            incognito: true,
        }
    );
    assert!(profile.requires_inprivate());
    let disk = LoginProfile::from_config(
        &WebviewConfig {
            profile: WebviewProfile::EphemeralDisk,
            os_primary_account_sso: true,
        },
        dir,
    );
    assert!(!disk.incognito);
    assert!(!disk.requires_inprivate());
    assert_eq!(disk.browser_args, browser_args(true));
}

// Exigence : EX-012-14
#[test]
fn profile_directory_is_emptied_and_created_again() {
    let base = std::env::temp_dir().join(format!("mcp-proxy-webview-{}", std::process::id()));
    let dir = base.join("webview");
    let _ = std::fs::remove_dir_all(&base);

    reset_profile_dir(&dir).unwrap();
    assert!(dir.is_dir());
    std::fs::create_dir_all(dir.join("EBWebView").join("Default")).unwrap();
    std::fs::write(dir.join("EBWebView").join("Default").join("Cookies"), SENTINEL).unwrap();
    reset_profile_dir(&dir).unwrap();
    assert!(dir.is_dir());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

    // A file in its place is replaced by an empty directory.
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::write(&dir, SENTINEL).unwrap();
    reset_profile_dir(&dir).unwrap();
    assert!(dir.is_dir());

    // A symbolic link is removed, never followed.
    #[cfg(unix)]
    {
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), SENTINEL).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
        reset_profile_dir(&dir).unwrap();
        assert!(outside.join("keep").is_file());
        assert!(dir.is_dir() && !dir.is_symlink());
    }
    std::fs::remove_dir_all(&base).unwrap();
}

// --- Script configuration -------------------------------------------------------------------------------------

/// Printable ASCII, without what HTML or a template literal would interpret.
fn plain(c: char) -> bool {
    matches!(c, ' '..='~') && !"<>&'`".contains(c)
}

fn assert_data_only(text: &str, value: &Value) {
    assert!(text.chars().all(plain), "{text}");
    assert_eq!(&serde_json::from_str::<Value>(text).unwrap(), value);
}

// Exigence : EX-012-15
#[test]
fn configuration_values_are_written_as_data() {
    for nasty in [
        "\"); alert(1); (\"",
        "'); alert(1); ('",
        "`${alert(1)}`",
        "</script><script>alert(1)</script>",
        "<!-- -->",
        "\u{2028}alert(1)\u{2029}",
        "\\\"",
        "\n\r\t\0",
        "é😀",
        "https://api.example.internal/*\"}); window.x = 1; ({\"a\":\"",
    ] {
        let value = json!({ "prefix": nasty, "list": [nasty], nasty: nasty });
        assert_data_only(&js_data(&value), &value);
    }
    assert_eq!(
        js_data(&json!({"a": "<&>'`"})),
        r#"{"a":"\u003c\u0026\u003e\u0027\u0060"}"#
    );
    assert_eq!(js_data(&json!("😀")), r#""\ud83d\ude00""#);
}

// Exigence : EX-012-15
#[test]
fn capture_script_carries_its_configuration_once() {
    const PLACEHOLDER: &str = "__MCP_PROXY_CAPTURE_CONFIG__";
    assert_eq!(CAPTURE_SOURCE.matches(PLACEHOLDER).count(), 1);
    let spec = corp_spec();
    let script = capture_script(spec.capture().unwrap());
    assert!(!script.contains(PLACEHOLDER));
    let config = json!({
        "prefix": "https://api.example.internal/",
        "wildcard": true,
        "header": "authorization",
        "scheme": "Bearer",
        "command": DEPOSIT_COMMAND,
    });
    let expected = CAPTURE_SOURCE.replace(PLACEHOLDER, &js_data(&config));
    assert_eq!(script, expected);
    assert_eq!(DEPOSIT_COMMAND, "deposit_login_header");
    // Nothing of the script logs or stores anything globally.
    assert!(!CAPTURE_SOURCE.contains("console."));
    assert!(!CAPTURE_SOURCE.contains("localStorage") && !CAPTURE_SOURCE.contains("sessionStorage"));
}

proptest! {
    // Exigence : EX-012-15
    #[test]
    fn any_text_is_written_as_data(text in any::<String>()) {
        let value = json!({ "prefix": text.clone(), "header": [text] });
        let written = js_data(&value);
        prop_assert!(written.chars().all(plain));
        prop_assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), value);
    }
}
