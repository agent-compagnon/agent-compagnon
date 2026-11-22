//! A fake application interface for the authentication (`FakeAuthUi`), and a fake server that checks Basic
//! identifiers (`FakeBasicServer`).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use mcp_proxy_auth::{
    AuthUi, AuthUiError, BasicInput, BasicPromptSpec, CookiePair, LoginEvent, LoginWindow, LoginWindowSpec,
};
use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::secrets::basic_authorization;
use mcp_proxy_core::session::{AccessReason, SessionInfo};
use secrecy::SecretString;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A cookie of the fake profile, readable by every window.
#[derive(Clone)]
struct FakeCookie {
    name: String,
    value: String,
    expires: Option<SystemTime>,
}

struct WindowRecord {
    spec: LoginWindowSpec,
    events: mpsc::UnboundedSender<LoginEvent>,
    closed: bool,
}

#[derive(Default)]
struct UiState {
    windows: Vec<WindowRecord>,
    /// Events queued for the next windows, one list per window, sent as soon as it opens.
    scripts: VecDeque<Vec<LoginEvent>>,
    cookies: Vec<FakeCookie>,
    prompts: Vec<BasicPromptSpec>,
    notifications: Vec<(SessionId, AccessReason)>,
    cleared: Vec<Vec<String>>,
    /// Windows and Basic prompts open right now, and the most ever open together.
    open: usize,
    max_open: usize,
    /// The next windows fail to open.
    failures: usize,
}

struct Shared {
    state: Mutex<UiState>,
    changed: Notify,
    basic_sender: mpsc::UnboundedSender<Option<BasicInput>>,
    basic_receiver: tokio::sync::Mutex<mpsc::UnboundedReceiver<Option<BasicInput>>>,
}

impl Shared {
    fn update<R>(&self, f: impl FnOnce(&mut UiState) -> R) -> R {
        let result = f(&mut lock(&self.state));
        self.changed.notify_waiters();
        result
    }

    fn opened(&self) {
        self.update(|s| {
            s.open += 1;
            s.max_open = s.max_open.max(s.open);
        });
    }
}

/// Records everything the authenticator asks of the interface. Windows are driven by the test, either by queued
/// scripts (`script_window`) or by a handle to an open window (`window`); Basic answers are queued by
/// `answer_basic`. Clones share the same state.
#[derive(Clone)]
pub struct FakeAuthUi {
    shared: Arc<Shared>,
}

impl Default for FakeAuthUi {
    fn default() -> Self {
        let (basic_sender, basic_receiver) = mpsc::unbounded_channel();
        Self {
            shared: Arc::new(Shared {
                state: Mutex::default(),
                changed: Notify::new(),
                basic_sender,
                basic_receiver: tokio::sync::Mutex::new(basic_receiver),
            }),
        }
    }
}

impl std::fmt::Debug for FakeAuthUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeAuthUi").finish_non_exhaustive()
    }
}

impl FakeAuthUi {
    /// The next window to open receives these events, in order, then waits for more from its handle.
    pub fn script_window(&self, events: Vec<LoginEvent>) {
        self.shared.update(|s| s.scripts.push_back(events));
    }

    /// A cookie of the shared profile, read by `LoginWindow::cookies` whatever the URL.
    pub fn set_cookie(&self, name: &str, value: &str, expires: Option<SystemTime>) {
        self.shared.update(|s| {
            s.cookies.retain(|c| c.name != name);
            s.cookies.push(FakeCookie {
                name: name.to_owned(),
                value: value.to_owned(),
                expires,
            });
        });
    }

    pub fn clear_cookies(&self) {
        self.shared.update(|s| s.cookies.clear());
    }

    /// The next Basic prompt answers this (`None`: cancelled). Answers are used in order.
    pub fn answer_basic(&self, answer: Option<(&str, &str)>) {
        let input = answer.map(|(login, password)| BasicInput {
            login: login.to_owned(),
            password: SecretString::from(password.to_owned()),
        });
        let _ = self.shared.basic_sender.send(input);
    }

    /// The next `count` windows fail to open.
    pub fn fail_windows(&self, count: usize) {
        self.shared.update(|s| s.failures += count);
    }

    /// Waits until window `index` (from 0) has opened, and returns its handle.
    pub async fn window(&self, index: usize) -> FakeWindow {
        loop {
            let notified = self.shared.changed.notified();
            if lock(&self.shared.state).windows.len() > index {
                return FakeWindow {
                    shared: Arc::clone(&self.shared),
                    index,
                };
            }
            notified.await;
        }
    }

    /// Waits until `count` Basic prompts have been shown.
    pub async fn wait_prompts(&self, count: usize) {
        loop {
            let notified = self.shared.changed.notified();
            if lock(&self.shared.state).prompts.len() >= count {
                return;
            }
            notified.await;
        }
    }

    /// Waits until no window nor prompt is open.
    pub async fn wait_all_closed(&self) {
        loop {
            let notified = self.shared.changed.notified();
            if lock(&self.shared.state).open == 0 {
                return;
            }
            notified.await;
        }
    }

    /// Specs of every window opened so far.
    pub fn windows(&self) -> Vec<LoginWindowSpec> {
        lock(&self.shared.state)
            .windows
            .iter()
            .map(|w| w.spec.clone())
            .collect()
    }

    pub fn window_count(&self) -> usize {
        lock(&self.shared.state).windows.len()
    }

    pub fn prompts(&self) -> Vec<BasicPromptSpec> {
        lock(&self.shared.state).prompts.clone()
    }

    pub fn notifications(&self) -> Vec<(SessionId, AccessReason)> {
        lock(&self.shared.state).notifications.clone()
    }

    /// Hosts of every `clear_site_data` call.
    pub fn cleared(&self) -> Vec<Vec<String>> {
        lock(&self.shared.state).cleared.clone()
    }

    pub fn open_now(&self) -> usize {
        lock(&self.shared.state).open
    }

    /// The most windows and prompts ever open at the same time.
    pub fn max_open(&self) -> usize {
        lock(&self.shared.state).max_open
    }
}

/// The test's side of a window opened by `FakeAuthUi`.
pub struct FakeWindow {
    shared: Arc<Shared>,
    index: usize,
}

impl FakeWindow {
    pub fn spec(&self) -> LoginWindowSpec {
        lock(&self.shared.state).windows[self.index].spec.clone()
    }

    pub fn send(&self, event: LoginEvent) {
        let sender = lock(&self.shared.state).windows[self.index].events.clone();
        let _ = sender.send(event);
    }

    pub fn navigate(&self, url: &str) {
        if let Ok(url) = Url::parse(url) {
            self.send(LoginEvent::Navigated { url });
        }
    }

    /// A deposit of the capture script.
    pub fn deposit(&self, url: &str, header: &str, value: &str) {
        if let Ok(url) = Url::parse(url) {
            self.send(LoginEvent::HeaderCaptured {
                url,
                header: header.to_owned(),
                value: SecretString::from(value.to_owned()),
            });
        }
    }

    /// The user closes the window.
    pub fn close_by_user(&self) {
        self.send(LoginEvent::Closed);
    }

    /// Closed by the authenticator (`close` called, or the window dropped).
    pub fn is_closed(&self) -> bool {
        lock(&self.shared.state).windows[self.index].closed
    }

    /// Waits until the authenticator closed the window.
    pub async fn wait_closed(&self) {
        loop {
            let notified = self.shared.changed.notified();
            if self.is_closed() {
                return;
            }
            notified.await;
        }
    }
}

struct FakeLoginWindow {
    shared: Arc<Shared>,
    index: usize,
    events: mpsc::UnboundedReceiver<LoginEvent>,
}

impl FakeLoginWindow {
    fn mark_closed(&self) {
        self.shared.update(|s| {
            let window = &mut s.windows[self.index];
            if !window.closed {
                window.closed = true;
                s.open = s.open.saturating_sub(1);
            }
        });
    }
}

impl Drop for FakeLoginWindow {
    fn drop(&mut self) {
        self.mark_closed();
    }
}

#[async_trait]
impl LoginWindow for FakeLoginWindow {
    async fn next_event(&mut self) -> Option<LoginEvent> {
        self.events.recv().await
    }

    async fn cookies(&self, _url: &Url) -> Result<Vec<CookiePair>, AuthUiError> {
        Ok(lock(&self.shared.state)
            .cookies
            .iter()
            .map(|c| CookiePair {
                name: c.name.clone(),
                value: SecretString::from(c.value.clone()),
                expires: c.expires,
            })
            .collect())
    }

    async fn close(self: Box<Self>) {
        self.mark_closed();
    }
}

#[async_trait]
impl AuthUi for FakeAuthUi {
    async fn login_window(&self, spec: LoginWindowSpec) -> Result<Box<dyn LoginWindow>, AuthUiError> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let index = self.shared.update(|s| {
            if s.failures > 0 {
                s.failures -= 1;
                return None;
            }
            for event in s.scripts.pop_front().unwrap_or_default() {
                let _ = sender.send(event);
            }
            s.windows.push(WindowRecord {
                spec,
                events: sender,
                closed: false,
            });
            s.open += 1;
            s.max_open = s.max_open.max(s.open);
            Some(s.windows.len() - 1)
        });
        let index = index.ok_or(AuthUiError::WindowFailed)?;
        Ok(Box::new(FakeLoginWindow {
            shared: Arc::clone(&self.shared),
            index,
            events: receiver,
        }))
    }

    async fn basic_prompt(&self, spec: BasicPromptSpec) -> Result<Option<BasicInput>, AuthUiError> {
        self.shared.update(|s| s.prompts.push(spec));
        self.shared.opened();
        let answer = self.shared.basic_receiver.lock().await.recv().await;
        self.shared.update(|s| s.open = s.open.saturating_sub(1));
        Ok(answer.flatten())
    }

    async fn notify_login_needed(&self, session: &SessionInfo, reason: &AccessReason) {
        self.shared
            .update(|s| s.notifications.push((session.id.clone(), reason.clone())));
    }

    async fn clear_site_data(&self, hosts: &[String]) -> Result<(), AuthUiError> {
        self.shared.update(|s| s.cleared.push(hosts.to_vec()));
        Ok(())
    }
}

/// An HTTP server whose `/probe` answers 200 to one login and password, 401 to anything else. Stops when dropped.
pub struct FakeBasicServer {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    ct: CancellationToken,
}

#[derive(Clone)]
struct ProbeState {
    expected: Arc<String>,
    requests: Arc<AtomicUsize>,
}

async fn probe(State(state): State<ProbeState>, headers: HeaderMap) -> StatusCode {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let given = headers.get("authorization").and_then(|v| v.to_str().ok());
    if given == Some(state.expected.as_str()) {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    }
}

impl FakeBasicServer {
    pub async fn start(login: &str, password: &str) -> std::io::Result<Self> {
        let expected = basic_authorization(login, &SecretString::from(password.to_owned()))
            .ok()
            .and_then(|v| v.to_str().ok().map(str::to_owned))
            .unwrap_or_default();
        let requests = Arc::new(AtomicUsize::new(0));
        let state = ProbeState {
            expected: Arc::new(expected),
            requests: Arc::clone(&requests),
        };
        let router = Router::new().route("/probe", get(probe)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ct = CancellationToken::new();
        let shutdown = ct.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(Self { addr, requests, ct })
    }

    /// `http://127.0.0.1:<port>/probe`.
    pub fn url(&self) -> String {
        format!("http://{}/probe", self.addr)
    }

    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for FakeBasicServer {
    fn drop(&mut self) {
        self.ct.cancel();
    }
}
