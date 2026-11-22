//! `OnDemandAuthenticator`: shared logins, deadlines, cancellation, expiry, queue of windows, no secret anywhere.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    CORP_API, CORP_MCP, DEV_MCP, DEV_SUCCESS, Harness, SENTINEL, basic_probe_at, corp_token, deadline, harness,
    harness_with, reason, settle, sid,
};
use mcp_proxy_auth::{BasicInput, CookiePair, LoginEvent, OnDemandAuthenticator};
use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::session::{SessionChange, SessionError, SessionGate, SessionState};
use reqwest::header::HeaderMap;
use secrecy::SecretString;
use tokio::task::JoinHandle;
use url::Url;

fn ensure(auth: &Arc<OnDemandAuthenticator>, session: &SessionId, secs: u64) -> JoinHandle<Result<(), SessionError>> {
    let auth = Arc::clone(auth);
    let session = session.clone();
    tokio::spawn(async move { auth.ensure(&session, deadline(secs), &reason()).await })
}

/// Completes the login of `sso-corp` in window `index` with a token valid for `secs`; returns the token.
async fn log_in_corp(h: &Harness, index: usize, secs: u64) -> String {
    let token = corp_token(h, secs);
    h.ui.window(index)
        .await
        .deposit(CORP_API, "Authorization", &format!("Bearer {token}"));
    token
}

fn changes(h: &mut Harness, session: &SessionId) -> Vec<SessionChange> {
    h.drain_events()
        .into_iter()
        .filter(|e| &e.session == session)
        .map(|e| e.change)
        .collect()
}

fn version_injected(h: &Harness, session: &SessionId, url: &str) -> mcp_proxy_core::secrets::SecretVersion {
    let mut headers = HeaderMap::new();
    h.auth.inject(session, &Url::parse(url).unwrap(), &mut headers).unwrap()
}

// Exigence : EX-010-04
#[tokio::test(start_paused = true)]
async fn simultaneous_calls_share_one_window() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let calls: Vec<_> = (0..10).map(|_| ensure(&h.auth, &corp, 50)).collect();
    settle().await;
    assert_eq!(h.ui.window_count(), 1);
    assert_eq!(h.ui.notifications().len(), 1);
    log_in_corp(&h, 0, 3600).await;
    for call in calls {
        assert_eq!(call.await.unwrap(), Ok(()));
    }
    assert_eq!(h.ui.window_count(), 1);
    assert_eq!(h.ui.max_open(), 1);
    assert_eq!(
        changes(&mut h, &corp),
        [SessionChange::LoginRequested, SessionChange::LoggedIn]
    );
    // Active: answered at once, without anything shown.
    h.auth.ensure(&corp, deadline(1), &reason()).await.unwrap();
    assert_eq!(h.ui.window_count(), 1);
    assert_eq!(h.ui.notifications().len(), 1);
}

// Exigence : EX-010-05
#[tokio::test(start_paused = true)]
async fn the_login_goes_on_past_the_deadline_of_the_call() {
    let h = harness();
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 5);
    let window = h.ui.window(0).await;
    let started = tokio::time::Instant::now();
    assert_eq!(call.await.unwrap(), Err(SessionError::LoginPending(corp.clone())));
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert!(!window.is_closed(), "the window stays open");
    assert!(matches!(
        h.registry.state(&corp),
        Some(SessionState::LoginRequested { .. })
    ));

    // A call that gives up its future (as the chain does at its deadline) does not stop the login either.
    let abandoned = ensure(&h.auth, &corp, 50);
    settle().await;
    abandoned.abort();
    settle().await;
    assert!(!window.is_closed());

    log_in_corp(&h, 0, 3600).await;
    window.wait_closed().await;
    h.auth.ensure(&corp, deadline(1), &reason()).await.unwrap();
    assert_eq!(h.ui.window_count(), 1, "the next call passes without a new window");
}

// Exigence : EX-010-06
#[tokio::test(start_paused = true)]
async fn closing_the_window_cancels_and_releases_every_waiting_call() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let first = ensure(&h.auth, &corp, 50);
    let second = ensure(&h.auth, &corp, 50);
    let window = h.ui.window(0).await;
    window.close_by_user();
    assert_eq!(first.await.unwrap(), Err(SessionError::Cancelled(corp.clone())));
    assert_eq!(second.await.unwrap(), Err(SessionError::Cancelled(corp.clone())));
    assert!(window.is_closed());
    assert_eq!(
        h.registry.state(&corp),
        Some(SessionState::NeverConnected),
        "previous state"
    );
    assert_eq!(
        changes(&mut h, &corp),
        [SessionChange::LoginRequested, SessionChange::LoginCancelled]
    );
    assert!(
        SessionError::Cancelled(corp.clone())
            .to_string()
            .contains("annulée par l'utilisateur"),
        "a clear error"
    );

    // A new call opens a new window.
    let call = ensure(&h.auth, &corp, 50);
    log_in_corp(&h, 1, 3600).await;
    assert_eq!(call.await.unwrap(), Ok(()));
}

// Exigence : EX-010-06
#[tokio::test(start_paused = true)]
async fn cancelling_the_basic_input_restores_the_expired_state() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    log_in_corp(&h, 0, 3600).await;
    call.await.unwrap().unwrap();
    let version = version_injected(&h, &corp, CORP_MCP);
    h.auth.mark_rejected(&corp, version);
    assert_eq!(h.registry.state(&corp), Some(SessionState::Expired));
    h.drain_events();

    let tools = sid("sso-tools");
    let auth = Arc::clone(&h.auth);
    let tools_call = tokio::spawn({
        let tools = tools.clone();
        async move { auth.ensure(&tools, deadline(50), &reason()).await }
    });
    h.ui.wait_prompts(1).await;
    h.ui.answer_basic(None);
    assert_eq!(tools_call.await.unwrap(), Err(SessionError::Cancelled(tools.clone())));
    assert_eq!(h.registry.state(&tools), Some(SessionState::NeverConnected));

    // `sso-corp` was expired before its new login was cancelled: it is expired again.
    let call = ensure(&h.auth, &corp, 50);
    h.ui.window(1).await.close_by_user();
    assert_eq!(call.await.unwrap(), Err(SessionError::Cancelled(corp.clone())));
    assert_eq!(h.registry.state(&corp), Some(SessionState::Expired));
}

// Exigence : EX-010-06
#[tokio::test(start_paused = true)]
async fn a_rejected_secret_expires_the_session_until_a_new_login() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    let first = log_in_corp(&h, 0, 3600).await;
    call.await.unwrap().unwrap();
    let version = version_injected(&h, &corp, CORP_MCP);
    h.drain_events();

    h.auth.mark_rejected(&corp, version);
    assert!(!h.auth.is_active(&corp));
    assert!(!h.store.has(&corp), "secret wiped");
    assert_eq!(h.registry.state(&corp), Some(SessionState::Expired));
    assert_eq!(changes(&mut h, &corp), [SessionChange::Rejected]);
    let mut headers = HeaderMap::new();
    assert!(
        h.auth
            .inject(&corp, &Url::parse(CORP_MCP).unwrap(), &mut headers)
            .is_err()
    );
    assert!(headers.is_empty());
    settle().await;
    assert_eq!(h.ui.window_count(), 1, "no new window until a call asks");

    let call = ensure(&h.auth, &corp, 50);
    let second = log_in_corp(&h, 1, 7200).await;
    call.await.unwrap().unwrap();
    assert_ne!(first, second);
    assert_eq!(
        h.injected(&corp, CORP_MCP).unwrap(),
        format!("authorization: Bearer {second}")
    );
}

// Exigence : EX-010-12
#[tokio::test(start_paused = true)]
async fn a_late_refusal_of_an_old_secret_keeps_the_renewed_one() {
    let h = harness();
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    log_in_corp(&h, 0, 3600).await;
    call.await.unwrap().unwrap();
    // Two calls sent the same secret; the first refusal expires it and a new login renews it.
    let old = version_injected(&h, &corp, CORP_MCP);
    h.auth.mark_rejected(&corp, old);
    let call = ensure(&h.auth, &corp, 50);
    let renewed = log_in_corp(&h, 1, 7200).await;
    call.await.unwrap().unwrap();
    // The second refusal, about the old secret, arrives now.
    h.auth.mark_rejected(&corp, old);
    assert!(h.auth.is_active(&corp));
    assert!(matches!(h.registry.state(&corp), Some(SessionState::Active { .. })));
    assert_eq!(
        h.injected(&corp, CORP_MCP).unwrap(),
        format!("authorization: Bearer {renewed}")
    );
}

// Exigence : EX-010-07
#[tokio::test(start_paused = true)]
async fn the_session_expires_at_its_expiry_and_nothing_else_happens() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    log_in_corp(&h, 0, 2 * 3600).await;
    call.await.unwrap().unwrap();
    h.drain_events();

    // Just before the margin: still active.
    tokio::time::sleep(Duration::from_secs(2 * 3600 - 62)).await;
    assert!(h.auth.is_active(&corp));
    // At expiry minus 60 s, the timer wipes the secret on its own.
    tokio::time::sleep(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(h.registry.state(&corp), Some(SessionState::Expired));
    assert!(!h.store.has(&corp));
    assert_eq!(changes(&mut h, &corp), [SessionChange::Expired]);
    assert!(!h.auth.is_active(&corp));

    // No renewal, no window, no notification, however long nothing asks.
    tokio::time::sleep(Duration::from_secs(48 * 3600)).await;
    settle().await;
    assert_eq!(h.ui.window_count(), 1);
    assert_eq!(h.ui.notifications().len(), 1);
    assert!(h.drain_events().is_empty());
}

// Exigence : EX-010-07
#[tokio::test(start_paused = true)]
async fn an_expiry_missed_by_the_timer_is_seen_at_the_next_use() {
    // The expiry is checked against the wall clock at every use, not only by the timer: after a sleep of the
    // computer, the timer may be late. Here the clock moves without the timer task having run yet.
    let h = harness();
    let dev = sid("sso-dev");
    h.ui.set_cookie("SESSION", "c", Some(h.now() + Duration::from_secs(600)));
    let call = ensure(&h.auth, &dev, 50);
    h.ui.window(0).await.navigate(DEV_SUCCESS);
    call.await.unwrap().unwrap();
    tokio::time::advance(Duration::from_secs(600)).await;
    // Whether or not the timer ran yet, the session is not usable.
    assert!(!h.auth.is_active(&dev));
    assert!(h.injected(&dev, DEV_MCP).is_none());
    assert_eq!(h.auth.state(&dev), Some(SessionState::Expired));
}

// Exigence : EX-010-08
#[tokio::test(start_paused = true)]
async fn two_sessions_get_their_windows_one_after_the_other() {
    let h = harness();
    let (corp, dev) = (sid("sso-corp"), sid("sso-dev"));
    let corp_call = ensure(&h.auth, &corp, 50);
    settle().await;
    let dev_call = ensure(&h.auth, &dev, 50);
    settle().await;
    assert_eq!(h.ui.window_count(), 1, "the second window waits");
    assert_eq!(h.ui.window(0).await.spec().session(), &corp);
    assert!(matches!(
        h.registry.state(&dev),
        Some(SessionState::LoginRequested { .. })
    ));
    assert_eq!(h.ui.notifications().len(), 1, "notified when its window opens");

    log_in_corp(&h, 0, 3600).await;
    assert_eq!(corp_call.await.unwrap(), Ok(()));
    let dev_window = h.ui.window(1).await;
    assert_eq!(dev_window.spec().session(), &dev);
    h.ui.set_cookie("SESSION", "cookie-dev", None);
    dev_window.navigate(DEV_SUCCESS);
    assert_eq!(dev_call.await.unwrap(), Ok(()));
    assert_eq!(h.ui.max_open(), 1, "never two windows at once");
    assert_eq!(h.ui.notifications().len(), 2);
}

// Exigence : EX-010-08
#[tokio::test(start_paused = true)]
async fn a_queued_session_whose_call_gave_up_still_gets_its_window() {
    let h = harness();
    let (corp, dev) = (sid("sso-corp"), sid("sso-dev"));
    let corp_call = ensure(&h.auth, &corp, 50);
    settle().await;
    let dev_call = ensure(&h.auth, &dev, 1);
    assert_eq!(dev_call.await.unwrap(), Err(SessionError::LoginPending(dev.clone())));
    log_in_corp(&h, 0, 3600).await;
    corp_call.await.unwrap().unwrap();
    h.ui.set_cookie("SESSION", "cookie-dev", None);
    h.ui.window(1).await.navigate(DEV_SUCCESS);
    h.ui.wait_all_closed().await;
    settle().await;
    assert!(h.auth.is_active(&dev));
}

// Exigence : EX-010-09
#[tokio::test(start_paused = true)]
async fn no_window_is_ever_hidden_nor_opened_without_a_call() {
    let h = harness();
    // Nothing asks: nothing opens, whatever the time.
    tokio::time::sleep(Duration::from_secs(72 * 3600)).await;
    settle().await;
    assert_eq!(h.ui.window_count(), 0);
    assert!(h.ui.prompts().is_empty());
    assert!(h.ui.notifications().is_empty());

    // Cookies already in the profile are not read silently: the window opens and the login waits for the success
    // page.
    let dev = sid("sso-dev");
    h.ui.set_cookie("SESSION", "already-there", None);
    let call = ensure(&h.auth, &dev, 50);
    let window = h.ui.window(0).await;
    settle().await;
    assert!(!call.is_finished());
    window.navigate(DEV_SUCCESS);
    call.await.unwrap().unwrap();

    let corp = sid("sso-corp");
    let call = ensure(&h.auth, &corp, 50);
    log_in_corp(&h, 1, 3600).await;
    call.await.unwrap().unwrap();
    // Every window has a visible title and one notification, which a call asked for.
    for spec in h.ui.windows() {
        assert!(spec.title().starts_with("Connexion — "), "{}", spec.title());
    }
    assert_eq!(h.ui.notifications().len(), h.ui.window_count());
    // A failing window is an error, never a fallback without a window.
    h.auth.disconnect(&corp).await;
    h.ui.fail_windows(1);
    let call = ensure(&h.auth, &corp, 50);
    assert_eq!(call.await.unwrap(), Err(SessionError::LoginFailed(corp.clone())));
    assert!(!h.store.has(&corp));
}

// Exigence : EX-010-13
#[tokio::test(start_paused = true)]
async fn connect_and_disconnect() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let auth = Arc::clone(&h.auth);
    let connect = tokio::spawn({
        let corp = corp.clone();
        async move { auth.connect(&corp).await }
    });
    log_in_corp(&h, 0, 3600).await;
    connect.await.unwrap().unwrap();
    assert!(
        h.ui.notifications().is_empty(),
        "no notification for the user's own request"
    );
    h.auth.connect(&corp).await.unwrap();
    assert_eq!(h.ui.window_count(), 1, "already active");
    h.drain_events();

    h.auth.disconnect(&corp).await;
    assert!(!h.store.has(&corp));
    assert!(!h.auth.is_active(&corp));
    assert_eq!(h.registry.state(&corp), Some(SessionState::Expired));
    assert_eq!(changes(&mut h, &corp), [SessionChange::Disconnected]);
    assert_eq!(
        h.ui.cleared(),
        [vec![
            "portal.example.internal".to_owned(),
            "login.example.internal".to_owned()
        ]]
    );

    // Disconnecting during a login closes its window and releases the waiting calls.
    let call = ensure(&h.auth, &corp, 50);
    let window = h.ui.window(1).await;
    h.auth.disconnect(&corp).await;
    assert_eq!(call.await.unwrap(), Err(SessionError::Cancelled(corp.clone())));
    window.wait_closed().await;
    assert!(!h.store.has(&corp));
    // A token deposited after the disconnection is not kept.
    window.deposit(CORP_API, "Authorization", &format!("Bearer {}", corp_token(&h, 3600)));
    settle().await;
    assert!(!h.store.has(&corp));

    // Basic: no web page, nothing to clear.
    let tools = sid("sso-tools");
    h.auth.disconnect(&tools).await;
    assert_eq!(h.ui.cleared().len(), 2);
    assert_eq!(
        h.registry.state(&tools),
        Some(SessionState::NeverConnected),
        "never connected stays so"
    );

    // A session without a method cannot log in.
    let unknown = sid("inconnue");
    assert_eq!(
        h.auth.ensure(&unknown, deadline(1), &reason()).await,
        Err(SessionError::Unavailable(unknown.clone()))
    );
}

// Exigence : EX-016-06
#[tokio::test(start_paused = true)]
async fn the_client_waiting_for_a_login_is_known_until_the_login_ends() {
    let h = harness();
    let corp = sid("sso-corp");
    assert!(h.auth.login_requesters().is_empty());

    // Asked from the interface only: nobody waits, no requester.
    let auth = Arc::clone(&h.auth);
    let connect = tokio::spawn({
        let corp = corp.clone();
        async move { auth.connect(&corp).await }
    });
    let _window = h.ui.window(0).await;
    assert!(h.auth.login_requesters().is_empty());
    // A call that joins the login makes its client known.
    let call = ensure(&h.auth, &corp, 50);
    settle().await;
    assert_eq!(h.auth.login_requesters(), [(corp.clone(), "Claude Desktop".to_owned())]);
    log_in_corp(&h, 0, 3600).await;
    call.await.unwrap().unwrap();
    connect.await.unwrap().unwrap();
    assert!(h.auth.login_requesters().is_empty(), "the login is over");

    // A tool list names no client.
    let dev = sid("sso-dev");
    let list = {
        let auth = Arc::clone(&h.auth);
        let dev = dev.clone();
        tokio::spawn(async move {
            let reason = mcp_proxy_core::session::AccessReason::ListTools {
                env: mcp_proxy_core::config::EnvId::new("dev").unwrap(),
                mcp: mcp_proxy_core::config::McpId::new("gitlab-ops").unwrap(),
            };
            auth.ensure(&dev, deadline(50), &reason).await
        })
    };
    let window = h.ui.window(1).await;
    assert_eq!(h.auth.login_requesters(), [(dev.clone(), String::new())]);
    window.close_by_user();
    assert_eq!(list.await.unwrap(), Err(SessionError::Cancelled(dev.clone())));
    assert!(h.auth.login_requesters().is_empty());
}

// Exigence : EX-015-04
#[tokio::test(start_paused = true)]
async fn disconnect_all_wipes_every_secret_and_releases_the_waiting_calls() {
    let mut h = harness();
    let corp = sid("sso-corp");
    let dev = sid("sso-dev");
    let int = sid("sso-int");
    let call = ensure(&h.auth, &corp, 50);
    let token = log_in_corp(&h, 0, 3600).await;
    call.await.unwrap().unwrap();
    let call = ensure(&h.auth, &dev, 50);
    let window = h.ui.window(1).await;
    h.ui.set_cookie("SESSION", "cookie-dev-91c3", None);
    window.navigate(DEV_SUCCESS);
    call.await.unwrap().unwrap();
    assert!(h.store.has(&corp) && h.store.has(&dev));
    // A login in progress, with its window open and a call waiting for it.
    let waiting = ensure(&h.auth, &int, 50);
    let login = h.ui.window(2).await;
    h.drain_events();

    h.auth.disconnect_all().await.unwrap();
    assert_eq!(waiting.await.unwrap(), Err(SessionError::Disconnected(int.clone())));
    assert!(
        SessionError::Disconnected(int.clone())
            .to_string()
            .contains("toutes les sessions ont été déconnectées par l'utilisateur")
    );
    login.wait_closed().await;
    for info in h.registry.infos() {
        assert!(!h.store.has(&info.id), "{}", info.id);
        assert!(!h.auth.is_active(&info.id), "{}", info.id);
        assert_eq!(
            h.registry.state(&info.id),
            Some(SessionState::NeverConnected),
            "{}",
            info.id
        );
    }
    let mut disconnected: Vec<SessionId> = h
        .drain_events()
        .into_iter()
        .filter(|e| e.change == SessionChange::Disconnected)
        .map(|e| e.session)
        .collect();
    disconnected.sort();
    assert_eq!(disconnected, [corp.clone(), dev.clone(), int.clone()]);
    assert_eq!(h.ui.cleared_all(), 1);
    assert!(h.injected(&corp, CORP_MCP).is_none());
    // The wiped token stays masked by the filter during its grace period (spec 002).
    let masked = h
        .store
        .with_variants(|variants| variants.contains(&token.as_bytes()))
        .unwrap();
    assert!(masked);

    // A token deposited in the closed window is not kept; the next call asks for a login again.
    login.deposit(CORP_API, "Authorization", &format!("Bearer {}", corp_token(&h, 3600)));
    settle().await;
    assert!(!h.store.has(&int) && !h.store.has(&corp));
    let call = ensure(&h.auth, &dev, 50);
    let again = h.ui.window(3).await;
    // The profile was emptied: the SSO cookie is gone, the user must log in again.
    again.navigate(DEV_SUCCESS);
    settle().await;
    assert!(!h.store.has(&dev));
    assert!(!again.is_closed());
    h.ui.set_cookie("SESSION", "cookie-dev-27aa", None);
    again.navigate(DEV_SUCCESS);
    call.await.unwrap().unwrap();
    assert!(h.auth.is_active(&dev));
}

/// Collects every log line written while the test runs (current-thread runtime: spawned tasks included).
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

// Exigence : EX-010-11
#[tokio::test(start_paused = true)]
async fn no_secret_in_debug_errors_events_or_logs() {
    let logs = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // Connection refused at once: the Basic login fails without leaving the machine.
    let mut h = harness_with(basic_probe_at("http://127.0.0.1:9/probe".to_owned()));
    let mut seen: Vec<String> = Vec::new();
    let corp = sid("sso-corp");
    let rejected = format!("{SENTINEL}.rejected.token");
    let call = ensure(&h.auth, &corp, 50);
    let window = h.ui.window(0).await;
    window.deposit(
        &format!("{CORP_API}?access_token={SENTINEL}"),
        "Authorization",
        &format!("Bearer {rejected}"),
    );
    let token = corp_token(&h, 3600);
    window.deposit(CORP_API, "Authorization", &format!("Bearer {token}"));
    seen.push(format!("{:?}", call.await.unwrap()));

    let dev = sid("sso-dev");
    h.ui.set_cookie("SESSION", SENTINEL, None);
    let call = ensure(&h.auth, &dev, 50);
    h.ui.window(1).await.navigate(DEV_SUCCESS);
    seen.push(format!("{:?}", call.await.unwrap()));

    let tools = sid("sso-tools");
    let call = ensure(&h.auth, &tools, 50);
    h.ui.wait_prompts(1).await;
    h.ui.answer_basic(Some(("jdupont", SENTINEL)));
    // The probe is unreachable: the login fails, which is logged.
    seen.push(format!("{:?}", call.await.unwrap()));
    seen.push(format!("{}", call_error(&h, &tools).await));

    h.auth.mark_rejected(&corp, version_injected(&h, &corp, CORP_MCP));
    h.auth.disconnect(&dev).await;
    seen.push(format!("{:?}", h.auth));
    seen.push(format!("{:?}", h.store));
    seen.push(format!("{:?}", h.registry));
    seen.push(format!("{:?}", h.registry.snapshot()));
    seen.extend(h.drain_events().iter().map(|e| format!("{e:?}")));
    seen.push(format!(
        "{:?}",
        LoginEvent::HeaderCaptured {
            url: Url::parse(CORP_API).unwrap(),
            header: "Authorization".to_owned(),
            value: SecretString::from(format!("Bearer {SENTINEL}")),
        }
    ));
    seen.push(format!(
        "{:?}",
        CookiePair {
            name: "SESSION".to_owned(),
            value: SecretString::from(SENTINEL.to_owned()),
            expires: None,
        }
    ));
    seen.push(format!(
        "{:?}",
        BasicInput {
            login: "jdupont".to_owned(),
            password: SecretString::from(SENTINEL.to_owned()),
        }
    ));
    seen.push(format!("{:?}", h.ui.prompts()));
    seen.push(format!("{:?}", h.ui.windows()));
    let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        logged.contains("valeur candidate de la fenêtre de connexion ignorée"),
        "{logged}"
    );
    assert!(logged.contains("connexion échouée"), "{logged}");
    seen.push(logged);

    for text in &seen {
        assert!(!text.contains(SENTINEL), "{text}");
        assert!(!text.contains(&token), "{text}");
        assert!(!text.contains(&rejected), "{text}");
    }
}

async fn call_error(h: &Harness, session: &SessionId) -> SessionError {
    h.ui.answer_basic(Some(("jdupont", SENTINEL)));
    h.auth.connect(session).await.unwrap_err()
}
