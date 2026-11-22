//! The login windows (spec 012) with Tauri's safe API: no COM call, no `unsafe`. What they decide is in
//! `mcp_proxy_auth::webview` and tested there; this module only wires it to WebView2.
//!
//! Every login window and the keeper share one profile: same `data_directory`, same browser arguments, and
//! `incognito(true)` unless the configuration asks for `ephemeral_disk`. A `jwt_header` window gets the capture
//! script in every frame and, at runtime, a capability for its own label and the remote URLs of its allowed hosts
//! that grants the deposit command only. Labels are never reused: once the window is gone, its capability matches
//! nothing any more (Tauri cannot remove a capability).

use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

use async_trait::async_trait;
use mcp_proxy_auth::webview::{
    BasicPrompts, LoginProfile, LoginWindows, SessionWindows, app_page_allowed, basic_prompt_script, capture_script,
    check_runtime_version, cookie_domain_matches, destination_for_log, navigation_allowed, parse_basic_answer,
    remote_url_patterns,
};
use mcp_proxy_auth::{
    AuthUi, AuthUiError, BasicInput, BasicPromptSpec, CookiePair, LoginEvent, LoginWindow, LoginWindowSpec,
};
use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::config::WebviewConfig;
use mcp_proxy_core::session::{AccessReason, SessionInfo};
use mcp_proxy_gateway::notify::{ToastAction, ToastState, login_toast};
use mcp_proxy_gateway::runtime::Gateway;
use secrecy::SecretString;
use serde_json::Value;
use tauri::ipc::CapabilityBuilder;
use tauri::utils::acl::RemoteUrlPattern;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tokio::sync::{Mutex, mpsc};
use url::Url;

use crate::dialog;
use crate::notify::{ToastEvent, Toaster};

/// Hidden window that keeps the profile alive between logins. It never navigates.
pub const KEEPER: &str = "login-keeper";
const LOGIN_PREFIX: &str = "login";
const BASIC_PREFIX: &str = "basic";
const DEPOSIT_PERMISSION: &str = "allow-deposit-login-header";

/// What the commands of the windows reach, managed by Tauri.
#[derive(Debug, Default)]
pub struct LoginState {
    pub windows: LoginWindows,
    pub prompts: BasicPrompts,
    /// The window of each session, for a click on the login notification.
    pub sessions: SessionWindows,
    next: AtomicU64,
}

impl LoginState {
    fn label(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

/// `AuthUi` with WebView2 windows.
pub struct WebviewAuthUi {
    app: AppHandle,
    state: Arc<LoginState>,
    profile: LoginProfile,
    /// The profile directory could not be emptied at start: no login window, fail closed.
    profile_unusable: Option<String>,
    keeper: Mutex<()>,
    told_too_old: AtomicBool,
    /// `None` for the WebView2 probe, which shows no notification.
    toaster: Option<Arc<Toaster>>,
}

impl WebviewAuthUi {
    /// Manages the state of the commands on `app`. `profile_reset` is the result of emptying `data_directory`.
    pub fn new(
        app: &AppHandle,
        config: &WebviewConfig,
        data_directory: PathBuf,
        profile_reset: Result<(), String>,
    ) -> Self {
        Self::with_toaster(app, config, data_directory, profile_reset, None)
    }

    /// The same, with the login notification (spec 013).
    pub(crate) fn with_toaster(
        app: &AppHandle,
        config: &WebviewConfig,
        data_directory: PathBuf,
        profile_reset: Result<(), String>,
        toaster: Option<Arc<Toaster>>,
    ) -> Self {
        let state = Arc::new(LoginState::default());
        app.manage(Arc::clone(&state));
        Self {
            app: app.clone(),
            state,
            profile: LoginProfile::from_config(config, data_directory),
            profile_unusable: profile_reset.err(),
            keeper: Mutex::new(()),
            told_too_old: AtomicBool::new(false),
            toaster,
        }
    }

    pub fn state(&self) -> &Arc<LoginState> {
        &self.state
    }

    pub fn profile(&self) -> &LoginProfile {
        &self.profile
    }

    /// The InPrivate check: an old runtime ignores `incognito` silently, so the login is refused instead.
    fn check_runtime(&self) -> Result<(), AuthUiError> {
        if !self.profile.requires_inprivate() {
            return Ok(());
        }
        let version = tauri::webview_version().ok();
        check_runtime_version(version.as_deref()).map(|_| ()).inspect_err(|_| {
            tracing::error!(
                version = version.as_deref().unwrap_or("illisible"),
                "runtime WebView2 trop ancien pour une connexion sans trace : connexion refusée"
            );
            if !self.told_too_old.swap(true, Ordering::Relaxed) {
                tokio::task::spawn_blocking(|| {
                    dialog::error(
                        "MCP Proxy",
                        "Le runtime WebView2 est trop ancien pour une connexion sans trace (version 101.0.1210.39 \
                         au moins). Mettez à jour « Microsoft Edge WebView2 Runtime », puis relancez MCP Proxy.",
                    );
                });
            }
        })
    }

    /// The keeper, created at the first login and kept until the application stops.
    async fn ensure_keeper(&self) -> Result<(), AuthUiError> {
        let _creating = self.keeper.lock().await;
        if self.app.get_webview_window(KEEPER).is_some() {
            return Ok(());
        }
        let app = self.app.clone();
        let profile = self.profile.clone();
        let built = tokio::task::spawn_blocking(move || {
            let blank = Url::parse("about:blank").map_err(|_| AuthUiError::WindowFailed)?;
            hardened(
                WebviewWindowBuilder::new(&app, KEEPER, WebviewUrl::External(blank)),
                &profile,
            )
            .title("MCP Proxy")
            .visible(false)
            .focused(false)
            .skip_taskbar(true)
            .on_navigation(|url| {
                tracing::warn!(destination = %destination_for_log(url), "navigation de la fenêtre gardienne bloquée");
                false
            })
            .build()
            .map(|_| ())
            .map_err(|error| {
                tracing::error!(%error, "fenêtre gardienne du profil impossible à créer");
                AuthUiError::WindowFailed
            })
        })
        .await;
        built.unwrap_or(Err(AuthUiError::WindowFailed))
    }

    /// The deposit capability of the window `label`: the deposit command only, for that window and the remote URLs
    /// of its allowed hosts, never for a local page.
    fn add_deposit_capability(&self, label: &str, spec: &LoginWindowSpec) -> Result<(), AuthUiError> {
        let session = spec.session().as_str();
        let patterns = remote_url_patterns(spec.allowed_navigation()).ok_or_else(|| {
            tracing::error!(
                session,
                "hôte autorisé inutilisable dans une capacité : fenêtre non ouverte"
            );
            AuthUiError::WindowFailed
        })?;
        // Checked here: an invalid pattern would make Tauri panic while adding the capability.
        if patterns.iter().any(|p| RemoteUrlPattern::from_str(p).is_err()) {
            tracing::error!(session, "motif d'URL de capacité refusé : fenêtre non ouverte");
            return Err(AuthUiError::WindowFailed);
        }
        let capability = patterns.into_iter().fold(
            CapabilityBuilder::new(format!("deposit-{label}"))
                .local(false)
                .window(label)
                .permission(DEPOSIT_PERMISSION),
            CapabilityBuilder::remote,
        );
        self.app.add_capability(capability).map_err(|error| {
            tracing::error!(session, %error, "capacité de dépôt impossible à ajouter : fenêtre non ouverte");
            AuthUiError::WindowFailed
        })
    }
}

/// What every window of the profile shares: data directory, arguments, InPrivate, no developer tools, no new window,
/// no download.
fn hardened<'a>(
    builder: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
    profile: &LoginProfile,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    builder
        .data_directory(profile.data_directory.clone())
        .additional_browser_args(&profile.browser_args)
        .incognito(profile.incognito)
        .devtools(false)
        .on_new_window(|url, _| {
            tracing::warn!(destination = %destination_for_log(&url), "nouvelle fenêtre refusée");
            NewWindowResponse::Deny
        })
        .on_download(|_, _| {
            tracing::warn!("téléchargement refusé dans une fenêtre de connexion");
            false
        })
}

#[async_trait]
impl AuthUi for WebviewAuthUi {
    async fn login_window(&self, spec: LoginWindowSpec) -> Result<Box<dyn LoginWindow>, AuthUiError> {
        if let Some(reason) = &self.profile_unusable {
            tracing::error!(%reason, "dossier du profil des fenêtres de connexion non vidé : connexion refusée");
            return Err(AuthUiError::WindowFailed);
        }
        self.check_runtime()?;
        self.ensure_keeper().await?;

        let label = self.state.label(LOGIN_PREFIX);
        let spec_session = &spec.session().clone();
        if spec.capture().is_some() {
            self.add_deposit_capability(&label, &spec)?;
        }
        let events = self.state.windows.open(&label, &spec);

        let app = self.app.clone();
        let profile = self.profile.clone();
        let state = Arc::clone(&self.state);
        let window_label = label.clone();
        let built =
            tokio::task::spawn_blocking(move || build_login_window(&app, &window_label, &spec, &profile, state))
                .await
                .unwrap_or(Err(AuthUiError::WindowFailed));
        let window = match built {
            Ok(window) => window,
            Err(error) => {
                self.state.windows.forget(&label);
                return Err(error);
            }
        };
        let _ = window.set_focus();
        self.state.sessions.opened(spec_session, &label);
        Ok(Box::new(TauriLoginWindow {
            window,
            label,
            state: Arc::clone(&self.state),
            events,
        }))
    }

    async fn basic_prompt(&self, spec: BasicPromptSpec) -> Result<Option<BasicInput>, AuthUiError> {
        let label = self.state.label(BASIC_PREFIX);
        let session = spec.session().clone();
        let answer = self.state.prompts.open(&label);
        let app = self.app.clone();
        let state = Arc::clone(&self.state);
        let window_label = label.clone();
        let built = tokio::task::spawn_blocking(move || build_basic_window(&app, &window_label, &spec, state))
            .await
            .unwrap_or(Err(AuthUiError::WindowFailed));
        let window = match built {
            Ok(window) => window,
            Err(error) => {
                self.state.prompts.closed(&label);
                return Err(error);
            }
        };
        self.state.sessions.opened(&session, &label);
        // Closed when the answer arrives, and also if this future is dropped before.
        let _open = BasicWindow {
            window,
            label,
            state: Arc::clone(&self.state),
        };
        Ok(answer.await.unwrap_or(None))
    }

    async fn notify_login_needed(&self, session: &SessionInfo, reason: &AccessReason) {
        tracing::info!(session = session.id.as_str(), "connexion demandée par un appel");
        let Some(toaster) = self.toaster.clone() else {
            return;
        };
        let client = match reason {
            AccessReason::ToolCall { client, .. } => client.as_str(),
            AccessReason::ListTools { .. } => "",
        };
        // Filtered by the chain already; again here, and nothing is shown without the filter.
        let Some(gateway) = self.app.try_state::<Arc<Gateway>>() else {
            return;
        };
        let (Some(client), Some(label)) = (gateway.filter_text(client), gateway.filter_text(&session.label)) else {
            return;
        };
        let app = self.app.clone();
        let id = session.id.clone();
        // The login window opens right after, visible whatever happens to the notification: it is not awaited.
        tauri::async_runtime::spawn_blocking(move || {
            if let ToastState::Unavailable(reason) = toaster.state() {
                tracing::info!(%reason, "notification de connexion non affichée");
                return;
            }
            let content = login_toast(toaster.next_number(), &id, &label, &client);
            let shown = toaster.show(&content, None, move |event| {
                if let ToastEvent::Activated(Some(arguments)) = event
                    && let Some(ToastAction::Login(session)) = ToastAction::parse(&arguments)
                {
                    front_window_of(&app, &session);
                }
            });
            if let Err(reason) = shown {
                tracing::info!(%reason, "notification de connexion non affichée");
            }
        });
    }

    async fn clear_site_data(&self, hosts: &[String]) -> Result<(), AuthUiError> {
        let Some(keeper) = self.app.get_webview_window(KEEPER) else {
            // No login since the start: the profile directory was emptied, nothing to remove.
            return Ok(());
        };
        let hosts = hosts.to_vec();
        tokio::task::spawn_blocking(move || {
            let cookies = keeper.cookies().map_err(|_| AuthUiError::ClearSiteData)?;
            for cookie in cookies {
                if cookie_domain_matches(cookie.domain().unwrap_or_default(), &hosts) {
                    keeper.delete_cookie(cookie).map_err(|_| AuthUiError::ClearSiteData)?;
                }
            }
            Ok(())
        })
        .await
        .unwrap_or(Err(AuthUiError::ClearSiteData))
    }
}

/// A click on the login notification: the login or Basic window of `session` comes to the front, if still open.
fn front_window_of(app: &AppHandle, session: &SessionId) {
    let Some(state) = app.try_state::<Arc<LoginState>>() else {
        return;
    };
    if let Some(window) = state
        .sessions
        .of(session)
        .and_then(|label| app.get_webview_window(&label))
    {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_login_window(
    app: &AppHandle,
    label: &str,
    spec: &LoginWindowSpec,
    profile: &LoginProfile,
    state: Arc<LoginState>,
) -> Result<WebviewWindow, AuthUiError> {
    let session = spec.session().as_str().to_owned();
    let allowed = spec.allowed_navigation().to_vec();
    let loads = Arc::clone(&state);
    let mut builder = hardened(
        WebviewWindowBuilder::new(app, label, WebviewUrl::External(spec.login_url().clone())),
        profile,
    )
    .title(spec.title())
    .inner_size(520.0, 720.0)
    .center()
    .visible(true)
    .focused(true)
    .on_navigation(move |url| {
        let allowed = navigation_allowed(&allowed, url);
        if !allowed {
            tracing::warn!(
                session = session.as_str(),
                destination = %destination_for_log(url),
                "navigation hors des hôtes autorisés bloquée"
            );
        }
        allowed
    })
    .on_page_load(move |window, payload| {
        if payload.event() == PageLoadEvent::Finished {
            loads.windows.navigated(window.label(), payload.url());
        }
    });
    if let Some(capture) = spec.capture() {
        builder = builder.initialization_script_for_all_frames(capture_script(capture));
    }
    let window = builder.build().map_err(|error| {
        tracing::error!(session = spec.session().as_str(), %error, "fenêtre de connexion impossible à ouvrir");
        AuthUiError::WindowFailed
    })?;
    let closed = label.to_owned();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            state.windows.closed(&closed);
            state.sessions.closed(&closed);
        }
    });
    Ok(window)
}

fn build_basic_window(
    app: &AppHandle,
    label: &str,
    spec: &BasicPromptSpec,
    state: Arc<LoginState>,
) -> Result<WebviewWindow, AuthUiError> {
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("basic.html".into()))
        .title(spec.title())
        .inner_size(420.0, 360.0)
        .resizable(false)
        .center()
        .visible(true)
        .focused(true)
        .devtools(false)
        .initialization_script(basic_prompt_script(spec))
        .on_navigation(|url| {
            let allowed = app_page_allowed(url);
            if !allowed {
                tracing::warn!(destination = %destination_for_log(url), "navigation de la fenêtre de saisie bloquée");
            }
            allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()
        .map_err(|error| {
            tracing::error!(session = spec.session().as_str(), %error, "fenêtre de saisie impossible à ouvrir");
            AuthUiError::WindowFailed
        })?;
    let closed = label.to_owned();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            state.prompts.closed(&closed);
            state.sessions.closed(&closed);
        }
    });
    let _ = window.set_focus();
    Ok(window)
}

/// A login window handed to the authenticator. Dropping it closes the window.
struct TauriLoginWindow {
    window: WebviewWindow,
    label: String,
    state: Arc<LoginState>,
    events: mpsc::UnboundedReceiver<LoginEvent>,
}

impl Drop for TauriLoginWindow {
    fn drop(&mut self) {
        self.state.windows.forget(&self.label);
        self.state.sessions.closed(&self.label);
        let _ = self.window.destroy();
    }
}

#[async_trait]
impl LoginWindow for TauriLoginWindow {
    async fn next_event(&mut self) -> Option<LoginEvent> {
        self.events.recv().await
    }

    async fn cookies(&self, url: &Url) -> Result<Vec<CookiePair>, AuthUiError> {
        let window = self.window.clone();
        let url = url.clone();
        // `cookies_for_url` blocks Windows when called from a synchronous command or an event handler.
        let cookies = tokio::task::spawn_blocking(move || window.cookies_for_url(url))
            .await
            .map_err(|_| AuthUiError::Cookies)?
            .map_err(|_| AuthUiError::Cookies)?;
        Ok(cookies
            .into_iter()
            .map(|cookie| CookiePair {
                name: cookie.name().to_owned(),
                value: SecretString::from(cookie.value().to_owned()),
                expires: cookie.expires_datetime().map(SystemTime::from),
            })
            .collect())
    }

    async fn close(self: Box<Self>) {}
}

/// The Basic window while its prompt waits. Dropping it closes the window.
struct BasicWindow {
    window: WebviewWindow,
    label: String,
    state: Arc<LoginState>,
}

impl Drop for BasicWindow {
    fn drop(&mut self) {
        self.state.prompts.closed(&self.label);
        self.state.sessions.closed(&self.label);
        let _ = self.window.destroy();
    }
}

/// The capture script hands a candidate here. Nothing is answered, whatever happens; the value becomes a
/// `SecretString` at once and every check is done by `LoginWindows::deposit`.
#[tauri::command]
pub async fn deposit_login_header(window: WebviewWindow, url: Option<Value>, value: Option<Value>) {
    let value = match value {
        Some(Value::String(value)) => Some(SecretString::from(value)),
        _ => None,
    };
    let url = match url {
        Some(Value::String(url)) => Some(url),
        _ => None,
    };
    let Some(state) = window.try_state::<Arc<LoginState>>() else {
        return;
    };
    let page = window.url().ok();
    state
        .windows
        .deposit(window.label(), page.as_ref(), url.as_deref(), value, SystemTime::now());
}

/// The answer of the Basic window (`null`: « Annuler »). Nothing is answered; an answer of another window, a second
/// answer or a malformed one is ignored.
#[tauri::command]
pub async fn submit_basic_credentials(window: WebviewWindow, answer: Option<Value>) {
    let answer = parse_basic_answer(answer);
    let Some(state) = window.try_state::<Arc<LoginState>>() else {
        return;
    };
    match answer {
        Ok(answer) => {
            if !state.prompts.submit(window.label(), answer) {
                tracing::info!("réponse de saisie Basic ignorée : fenêtre sans saisie en cours");
            }
        }
        Err(error) => tracing::info!(%error, "réponse de saisie Basic ignorée"),
    }
}
