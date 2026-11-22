//! `FakeSso`, the login site of the WebView2 probe (spec 012).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::SystemTime;

use mcp_proxy_auth::capture::{JwtExpectations, check_jwt};
use mcp_proxy_testkit::{ApiCall, FAKE_SSO_AUDIENCE, FAKE_SSO_COOKIE, FAKE_SSO_ISSUER, FakeSso, SsoToken};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, COOKIE, LOCATION, SET_COOKIE};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

// Exigence : EX-012-01
#[tokio::test]
async fn login_sets_an_httponly_cookie_and_the_page_calls_the_api_with_the_token() {
    let sso = FakeSso::start().await.unwrap();
    let http = client();

    let login = http.get(sso.url("/login/fetch")).send().await.unwrap();
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    assert_eq!(login.headers()[LOCATION], "/app/fetch");
    let cookie = login.headers()[SET_COOKIE].to_str().unwrap().to_owned();
    let value = sso.session_cookie().unwrap();
    assert_eq!(
        cookie,
        format!("{FAKE_SSO_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax")
    );

    // The page calls the API with the token, by fetch or by XMLHttpRequest.
    let fetch_token = sso.token(SsoToken::Fetch);
    let page = http
        .get(sso.url("/app/fetch"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains(&format!("'Authorization': 'Bearer {fetch_token}'")),
        "{page}"
    );
    assert!(page.contains("fetch('/api/fetch'"), "{page}");
    let page = http
        .get(sso.url("/app/xhr"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("setRequestHeader('Authorization', 'Bearer "), "{page}");
    assert!(page.contains(&sso.token(SsoToken::Xhr)), "{page}");

    // The API accepts the token and records the header it was sent.
    let bearer = format!("Bearer {fetch_token}");
    let ok = http
        .get(sso.url("/api/fetch"))
        .header(AUTHORIZATION, &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let refused = http.get(sso.url("/api/fetch")).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let expired = format!("Bearer {}", sso.token(SsoToken::Expired));
    let refused = http
        .get(sso.url("/api/fetch"))
        .header(AUTHORIZATION, &expired)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        sso.api_calls(),
        [
            ApiCall {
                path: "/api/fetch".to_owned(),
                authorization: Some(bearer),
            },
            ApiCall {
                path: "/api/fetch".to_owned(),
                authorization: None,
            },
            ApiCall {
                path: "/api/fetch".to_owned(),
                authorization: Some(expired),
            },
        ]
    );

    // The session cookie is recognised by `/whoami`.
    http.get(sso.url("/whoami"))
        .header(COOKIE, &cookie[..cookie.find(';').unwrap()])
        .send()
        .await
        .unwrap();
    http.get(sso.url("/whoami")).send().await.unwrap();
    assert_eq!(sso.whoami(), [true, false]);
}

// Exigence : EX-012-01
#[tokio::test]
async fn tokens_pass_the_checks_of_the_capture_or_fail_them_on_purpose() {
    let sso = FakeSso::start().await.unwrap();
    let expect = JwtExpectations {
        audience: Some(FAKE_SSO_AUDIENCE.to_owned()),
        issuer: Some(FAKE_SSO_ISSUER.to_owned()),
    };
    let now = SystemTime::now();
    for valid in [SsoToken::Fetch, SsoToken::Xhr, SsoToken::Forged, SsoToken::Final] {
        assert!(check_jwt(&sso.token(valid), now, &expect).is_ok(), "{valid:?}");
    }
    assert!(check_jwt(&sso.token(SsoToken::Expired), now, &expect).is_err());
    assert!(check_jwt(&sso.token(SsoToken::OtherAudience), now, &expect).is_err());
    let all: std::collections::BTreeSet<String> = [
        SsoToken::Fetch,
        SsoToken::Xhr,
        SsoToken::Forged,
        SsoToken::Final,
        SsoToken::Expired,
        SsoToken::OtherAudience,
    ]
    .into_iter()
    .map(|t| sso.token(t))
    .collect();
    assert_eq!(all.len(), 6);
}

// Exigence : EX-012-01
#[tokio::test]
async fn hostile_pages_and_reports_are_served_and_recorded() {
    let sso = FakeSso::start().await.unwrap();
    let http = client();
    let page = |path: &'static str| {
        let http = http.clone();
        let url = sso.url(path);
        async move { http.get(url).send().await.unwrap().text().await.unwrap() }
    };
    let commands = page("/app/commands").await;
    assert!(
        commands.contains("'get_snapshot'") && commands.contains("/report?name="),
        "{commands}"
    );
    let forged = page("/app/forged").await;
    assert!(
        forged.contains("deposit_login_header") && forged.contains("/api/final"),
        "{forged}"
    );
    assert!(forged.contains(&sso.token(SsoToken::Final)), "{forged}");
    let escape = page("/app/escape").await;
    assert!(escape.contains(&format!(
        "http://localhost:{}/escaped",
        sso.origin().rsplit(':').next().unwrap()
    )));

    http.get(sso.url("/report?name=get_snapshot&result=refused"))
        .send()
        .await
        .unwrap();
    assert_eq!(sso.reports(), [("get_snapshot".to_owned(), "refused".to_owned())]);
    assert_eq!(sso.hits("/escaped"), 0);
    http.get(sso.url("/escaped")).send().await.unwrap();
    assert_eq!(sso.hits("/escaped"), 1);
    assert_eq!(
        http.get(sso.url("/app/unknown")).send().await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(sso.host(), "127.0.0.1");
    assert_eq!(sso.api_prefix(), format!("{}/api/", sso.origin()));
}
