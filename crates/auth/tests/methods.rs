//! The three methods through the authenticator, with `FakeAuthUi`.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    CORP_API, CORP_MCP, DEV_MCP, DEV_SUCCESS, Harness, basic_probe_at, corp_token, deadline, harness, harness_with,
    jwt, reason, settle, sid,
};
use mcp_proxy_auth::OnDemandAuthenticator;
use mcp_proxy_core::config::{AuthConfig, SessionId};
use mcp_proxy_core::session::{EXPIRY_MARGIN, SessionError, SessionGate, SessionState};
use mcp_proxy_testkit::FakeBasicServer;
use serde_json::json;
use tokio::task::JoinHandle;

fn ensure(auth: &Arc<OnDemandAuthenticator>, session: &SessionId, secs: u64) -> JoinHandle<Result<(), SessionError>> {
    let auth = Arc::clone(auth);
    let session = session.clone();
    tokio::spawn(async move { auth.ensure(&session, deadline(secs), &reason()).await })
}

fn until(h: &Harness, session: &SessionId) -> std::time::SystemTime {
    match h.registry.state(session) {
        Some(SessionState::Active { until }) => until,
        other => panic!("{other:?}"),
    }
}

// Exigence : EX-010-01
#[tokio::test(start_paused = true)]
async fn jwt_header_nominal() {
    let h = harness();
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    let window = h.ui.window(0).await;
    let spec = window.spec();
    assert_eq!(spec.title(), "Connexion — SSO Entreprise");
    assert_eq!(spec.login_url().as_str(), "https://portal.example.internal/");
    assert_eq!(
        spec.allowed_navigation(),
        ["portal.example.internal", "login.example.internal"]
    );
    let capture = spec.capture().unwrap();
    assert_eq!((capture.header(), capture.scheme()), ("Authorization", "Bearer"));
    assert_eq!(
        h.registry.state(&corp).map(|s| s.label(0)).as_deref(),
        Some("connexion demandée")
    );

    window.navigate("https://login.example.internal/sso");
    let token = corp_token(&h, 3600);
    window.deposit(CORP_API, "Authorization", &format!("Bearer {token}"));
    assert_eq!(call.await.unwrap(), Ok(()));
    assert!(window.is_closed(), "closed once the first valid token is kept");
    assert_eq!(
        h.injected(&corp, CORP_MCP).unwrap(),
        format!("authorization: Bearer {token}")
    );
    // exp sooner than max_ttl_hours: the expiry is exp, margin taken off.
    let exp = std::time::UNIX_EPOCH + Duration::from_secs(h.unix_now() + 3600);
    assert_eq!(until(&h, &corp), exp - EXPIRY_MARGIN);

    // A later token is not looked at: the window is closed.
    window.deposit(CORP_API, "Authorization", &format!("Bearer {}", corp_token(&h, 7200)));
    settle().await;
    assert_eq!(
        h.injected(&corp, CORP_MCP).unwrap(),
        format!("authorization: Bearer {token}")
    );
}

// Exigence : EX-010-01
#[tokio::test(start_paused = true)]
async fn jwt_expiry_is_capped_by_max_ttl() {
    let h = harness_with(|config| {
        let corp = config
            .sessions
            .iter_mut()
            .find(|s| s.id.as_str() == "sso-corp")
            .unwrap();
        if let AuthConfig::JwtHeader { max_ttl_hours, .. } = &mut corp.auth {
            *max_ttl_hours = 2;
        }
    });
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    let window = h.ui.window(0).await;
    window.deposit(
        CORP_API,
        "authorization",
        &format!("bearer {}", corp_token(&h, 10 * 3600)),
    );
    assert_eq!(call.await.unwrap(), Ok(()));
    assert_eq!(
        until(&h, &corp),
        h.now() + Duration::from_secs(2 * 3600) - EXPIRY_MARGIN
    );
}

// Exigence : EX-010-02
#[tokio::test(start_paused = true)]
async fn rejected_jwt_candidates_are_ignored_and_the_login_goes_on() {
    let h = harness_with(|config| {
        let corp = config
            .sessions
            .iter_mut()
            .find(|s| s.id.as_str() == "sso-corp")
            .unwrap();
        if let AuthConfig::JwtHeader { expected_issuer, .. } = &mut corp.auth {
            *expected_issuer = Some("https://login.example.internal".to_owned());
        }
    });
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    let window = h.ui.window(0).await;
    let now = h.unix_now();
    let iss = "https://login.example.internal";
    let bearer = |t: String| format!("Bearer {t}");
    // Malformed, expired, wrong audience, wrong issuer, URL outside the pattern, wrong header, wrong scheme.
    window.deposit(CORP_API, "Authorization", "Bearer not-a-jwt");
    window.deposit(
        CORP_API,
        "Authorization",
        &bearer(jwt(json!({ "aud": "api://internal", "iss": iss, "exp": now - 10 }))),
    );
    window.deposit(
        CORP_API,
        "Authorization",
        &bearer(jwt(json!({ "aud": "api://other", "iss": iss, "exp": now + 3600 }))),
    );
    window.deposit(
        CORP_API,
        "Authorization",
        &bearer(jwt(
            json!({ "aud": "api://internal", "iss": "https://x", "exp": now + 3600 }),
        )),
    );
    let good = jwt(json!({ "aud": "api://internal", "iss": iss, "exp": now + 3600 }));
    window.deposit(
        "https://portal.example.internal/api",
        "Authorization",
        &bearer(good.clone()),
    );
    window.deposit(CORP_API, "X-Auth", &bearer(good.clone()));
    window.deposit(CORP_API, "Authorization", &format!("Token {good}"));
    settle().await;
    assert!(!call.is_finished());
    assert!(!window.is_closed());
    assert!(!h.auth.is_active(&corp));
    assert!(!h.store.has(&corp));
    assert!(matches!(
        h.registry.state(&corp),
        Some(SessionState::LoginRequested { .. })
    ));

    window.deposit(CORP_API, "Authorization", &bearer(good.clone()));
    assert_eq!(call.await.unwrap(), Ok(()));
    assert_eq!(
        h.injected(&corp, CORP_MCP).unwrap(),
        format!("authorization: Bearer {good}")
    );
}

// Exigence : EX-010-01
#[tokio::test(start_paused = true)]
async fn cookie_nominal() {
    let h = harness();
    let dev = sid("sso-dev");
    let call = ensure(&h.auth, &dev, 50);
    let window = h.ui.window(0).await;
    let spec = window.spec();
    assert_eq!(spec.title(), "Connexion — SSO DEV");
    assert!(spec.capture().is_none(), "no capture script for a cookie session");
    h.ui.set_cookie("SESSION", "cookie-dev-42", Some(h.now() + Duration::from_secs(3600)));
    h.ui.set_cookie("OTHER", "not-kept", None);
    // A navigation outside the success pattern reads nothing.
    window.navigate("https://sso.dev.example.internal/login?step=2");
    settle().await;
    assert!(!call.is_finished());
    window.navigate(DEV_SUCCESS);
    assert_eq!(call.await.unwrap(), Ok(()));
    assert!(window.is_closed());
    assert_eq!(h.injected(&dev, DEV_MCP).unwrap(), "cookie: SESSION=cookie-dev-42");
    // The cookie expires before ttl_hours (8 h).
    assert_eq!(until(&h, &dev), h.now() + Duration::from_secs(3600) - EXPIRY_MARGIN);
}

// Exigence : EX-010-01
#[tokio::test(start_paused = true)]
async fn session_cookie_lives_ttl_hours() {
    let h = harness();
    let dev = sid("sso-dev");
    h.ui.set_cookie("SESSION", "cookie-dev-42", None);
    let call = ensure(&h.auth, &dev, 50);
    h.ui.window(0).await.navigate(DEV_SUCCESS);
    assert_eq!(call.await.unwrap(), Ok(()));
    assert_eq!(until(&h, &dev), h.now() + Duration::from_secs(8 * 3600) - EXPIRY_MARGIN);
}

// Exigence : EX-010-03
#[tokio::test(start_paused = true)]
async fn a_missing_or_expiring_cookie_keeps_the_login_waiting() {
    let h = harness();
    let dev = sid("sso-dev");
    let call = ensure(&h.auth, &dev, 50);
    let window = h.ui.window(0).await;
    h.ui.set_cookie("OTHER", "x", None);
    window.navigate(DEV_SUCCESS);
    settle().await;
    assert!(!call.is_finished(), "SESSION missing");
    h.ui.set_cookie("SESSION", "about-to-expire", Some(h.now() + Duration::from_secs(30)));
    window.navigate(DEV_SUCCESS);
    settle().await;
    assert!(!call.is_finished(), "SESSION within the expiry margin");
    assert!(!h.store.has(&dev));
    h.ui.set_cookie("SESSION", "fresh", None);
    window.navigate(DEV_SUCCESS);
    assert_eq!(call.await.unwrap(), Ok(()));
    assert_eq!(h.injected(&dev, DEV_MCP).unwrap(), "cookie: SESSION=fresh");
}

fn without_probe(config: &mut mcp_proxy_core::config::Config) {
    let tools = config
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "sso-tools")
        .unwrap();
    tools.auth = AuthConfig::Basic {
        ttl_hours: 8,
        probe_url: None,
    };
}

// Exigence : EX-010-01
#[tokio::test(flavor = "multi_thread")]
async fn basic_nominal_without_probe() {
    let h = harness_with(without_probe);
    let tools = sid("sso-tools");
    let auth = Arc::clone(&h.auth);
    h.ui.answer_basic(Some(("jdupont", "mot-de-passe")));
    auth.connect(&tools).await.unwrap();
    let prompts = h.ui.prompts();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].title(), "Connexion — SSO Outils");
    assert_eq!((prompts[0].attempt(), prompts[0].max_attempts()), (1, 3));
    assert_eq!(prompts[0].error(), None);
    assert_eq!(h.ui.window_count(), 0, "no web page for Basic");
    assert!(h.ui.notifications().is_empty(), "connect is the user's own request");
    assert_eq!(
        h.injected(&tools, "https://jira.example.internal/rest/api/2/issue")
            .unwrap(),
        "authorization: Basic amR1cG9udDptb3QtZGUtcGFzc2U="
    );
}

/// `probe_url` of `sso-tools` on a fake server that accepts `jdupont` / `bon-mot-de-passe`.
async fn probed() -> (Harness, FakeBasicServer) {
    let server = FakeBasicServer::start("jdupont", "bon-mot-de-passe").await.unwrap();
    let h = harness_with(basic_probe_at(server.url()));
    (h, server)
}

// Exigence : EX-010-03
#[tokio::test(flavor = "multi_thread")]
async fn basic_probe_refused_then_accepted() {
    let (h, server) = probed().await;
    let tools = sid("sso-tools");
    h.ui.answer_basic(Some(("jdupont", "faux")));
    h.ui.answer_basic(Some(("jdupont", "bon-mot-de-passe")));
    let started = std::time::Instant::now();
    let result = h
        .auth
        .ensure(&tools, started + Duration::from_secs(30), &reason())
        .await;
    assert_eq!(result, Ok(()));
    assert_eq!(server.requests(), 2);
    let prompts = h.ui.prompts();
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[1].error(), Some("identifiants refusés"));
    assert_eq!(
        prompts[1].login(),
        Some("jdupont"),
        "the login is filled in again, never the password"
    );
    assert_eq!(prompts[1].attempt(), 2);
    let until = until(&h, &tools);
    let ttl = until.duration_since(std::time::SystemTime::now()).unwrap() + EXPIRY_MARGIN;
    assert!(
        ttl <= Duration::from_secs(8 * 3600) && ttl > Duration::from_secs(8 * 3600 - 60),
        "{ttl:?}"
    );
}

// Exigence : EX-010-03
#[tokio::test(flavor = "multi_thread")]
async fn three_refusals_give_up() {
    let (h, server) = probed().await;
    let tools = sid("sso-tools");
    for _ in 0..3 {
        h.ui.answer_basic(Some(("jdupont", "faux")));
    }
    let result = h.auth.connect(&tools).await;
    assert_eq!(result, Err(SessionError::LoginFailed(tools.clone())));
    assert_eq!(server.requests(), 3);
    assert_eq!(h.ui.prompts().len(), 3);
    assert!(!h.store.has(&tools));
    assert_eq!(
        h.registry.state(&tools),
        Some(SessionState::NeverConnected),
        "previous state restored"
    );
    // A new attempt starts again from one.
    h.ui.answer_basic(Some(("jdupont", "bon-mot-de-passe")));
    h.auth.connect(&tools).await.unwrap();
    assert_eq!(h.ui.prompts()[3].attempt(), 1);
}

// Exigence : EX-010-03
#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_probe_keeps_nothing() {
    let server = FakeBasicServer::start("jdupont", "bon-mot-de-passe").await.unwrap();
    let url = server.url();
    drop(server);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let h = harness_with(basic_probe_at(url));
    let tools = sid("sso-tools");
    h.ui.answer_basic(Some(("jdupont", "bon-mot-de-passe")));
    assert_eq!(
        h.auth.connect(&tools).await,
        Err(SessionError::LoginFailed(tools.clone()))
    );
    assert_eq!(
        h.ui.prompts().len(),
        1,
        "a probe failure is not a refusal: no new input"
    );
    assert!(!h.store.has(&tools));
}

// Exigence : EX-010-01
#[tokio::test(flavor = "multi_thread")]
async fn invalid_basic_input_is_asked_again() {
    let h = harness_with(without_probe);
    let tools = sid("sso-tools");
    h.ui.answer_basic(Some(("jean:dupont", "x")));
    h.ui.answer_basic(Some(("jdupont", "")));
    h.ui.answer_basic(Some(("jdupont", "ok")));
    h.auth.connect(&tools).await.unwrap();
    let prompts = h.ui.prompts();
    assert_eq!(prompts.len(), 3);
    assert!(
        prompts[1]
            .error()
            .unwrap()
            .starts_with("identifiant ou mot de passe invalide")
    );
}
