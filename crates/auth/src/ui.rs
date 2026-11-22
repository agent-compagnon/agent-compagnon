//! What the authentication needs from the application: login windows, the Basic input window, the login
//! notification and the clearing of site data. Implemented with WebView2 by the application (spec 012), and by
//! `FakeAuthUi` in the testkit.

use std::fmt;
use std::time::SystemTime;

use async_trait::async_trait;
use mcp_proxy_core::config::{SessionId, UrlPattern};
use mcp_proxy_core::session::{AccessReason, SessionInfo};
use secrecy::SecretString;
use thiserror::Error;
use url::Url;

/// What the header capture of a `jwt_header` window looks for. The script injected in the page (spec 012) only
/// deposits candidates that match; every candidate is checked again on the Rust side
/// (`HeaderCapture::candidate_token`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderCapture {
    pub(crate) url_pattern: UrlPattern,
    pub(crate) header: String,
    pub(crate) scheme: String,
}

impl HeaderCapture {
    pub fn url_pattern(&self) -> &UrlPattern {
        &self.url_pattern
    }

    /// Name of the header, as configured (`Authorization`); compared without regard to case.
    pub fn header(&self) -> &str {
        &self.header
    }

    /// Scheme before the token (`Bearer`); compared without regard to case.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
}

/// A login window to open. It is always visible: this type has no way to ask for a hidden window, and it can only be
/// built by this crate's authentication methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginWindowSpec {
    pub(crate) session: SessionId,
    pub(crate) title: String,
    pub(crate) login_url: Url,
    pub(crate) allowed_navigation: Vec<String>,
    pub(crate) capture: Option<HeaderCapture>,
}

impl LoginWindowSpec {
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    /// `Connexion — SSO PROD`.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// First page shown.
    pub fn login_url(&self) -> &Url {
        &self.login_url
    }

    /// The only hosts the window may navigate to (exact host names, lower case).
    pub fn allowed_navigation(&self) -> &[String] {
        &self.allowed_navigation
    }

    /// `Some` for a `jwt_header` session only: the window then injects the capture script and accepts deposits.
    pub fn capture(&self) -> Option<&HeaderCapture> {
        self.capture.as_ref()
    }
}

/// `Connexion — {label}`.
pub(crate) fn window_title(label: &str) -> String {
    format!("Connexion — {label}")
}

/// What happens in a login window.
pub enum LoginEvent {
    /// A request of the page carried `header: value` towards `url` (deposit of the capture script). A candidate
    /// only: nothing about it is trusted until checked.
    HeaderCaptured {
        url: Url,
        header: String,
        value: SecretString,
    },
    /// The top-level page is now `url`.
    Navigated { url: Url },
    /// The user closed the window.
    Closed,
}

impl fmt::Debug for LoginEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderCaptured { url, header, .. } => f
                .debug_struct("HeaderCaptured")
                .field("url", &url.as_str())
                .field("header", header)
                .finish_non_exhaustive(),
            Self::Navigated { url } => f.debug_struct("Navigated").field("url", &url.as_str()).finish(),
            Self::Closed => f.write_str("Closed"),
        }
    }
}

/// A cookie read from the cookie manager of the login windows. The name is not a secret, the value is.
pub struct CookiePair {
    pub name: String,
    pub value: SecretString,
    /// `None` for a session cookie.
    pub expires: Option<SystemTime>,
}

impl fmt::Debug for CookiePair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CookiePair")
            .field("name", &self.name)
            .field("value", &"***")
            .field("expires", &self.expires)
            .finish()
    }
}

/// The Basic input window: login, password, « Se connecter », « Annuler ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicPromptSpec {
    pub(crate) session: SessionId,
    pub(crate) title: String,
    pub(crate) login: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) attempt: u32,
    pub(crate) max_attempts: u32,
}

impl BasicPromptSpec {
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Login of the previous attempt, to fill in again (never the password).
    pub fn login(&self) -> Option<&str> {
        self.login.as_deref()
    }

    /// French message to show above the form, such as « identifiants refusés ».
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 1 for the first attempt.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
}

/// What the user typed in the Basic window. The login is not a secret, the password is.
pub struct BasicInput {
    pub login: String,
    pub password: SecretString,
}

impl fmt::Debug for BasicInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasicInput")
            .field("login", &self.login)
            .field("password", &"***")
            .finish()
    }
}

/// Why the interface could not do what was asked. Fixed French texts: nothing from the page nor from a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AuthUiError {
    #[error("interface de l'appli indisponible")]
    Unavailable,
    #[error("fenêtre de connexion impossible à ouvrir")]
    WindowFailed,
    #[error("runtime WebView2 trop ancien pour une connexion sans trace")]
    RuntimeTooOld,
    #[error("cookies de la fenêtre de connexion illisibles")]
    Cookies,
    #[error("données des sites de la session impossibles à effacer")]
    ClearSiteData,
}

#[async_trait]
pub trait AuthUi: Send + Sync {
    /// Opens a visible login window on `spec.login_url()`.
    async fn login_window(&self, spec: LoginWindowSpec) -> Result<Box<dyn LoginWindow>, AuthUiError>;

    /// Shows the Basic input window and waits for the answer. `None`: the user cancelled or closed it.
    async fn basic_prompt(&self, spec: BasicPromptSpec) -> Result<Option<BasicInput>, AuthUiError>;

    /// Tells the user that a call waits for a login (Windows notification, spec 013). Called right before the window
    /// opens, when the login was asked for by a call.
    async fn notify_login_needed(&self, session: &SessionInfo, reason: &AccessReason);

    /// Removes the cookies and site data of `hosts` from the login windows' profile.
    async fn clear_site_data(&self, hosts: &[String]) -> Result<(), AuthUiError>;
}

#[async_trait]
pub trait LoginWindow: Send {
    /// The next event; `None` once the window is gone, which counts as `Closed`.
    async fn next_event(&mut self) -> Option<LoginEvent>;

    /// Cookies of `url` in the login windows' profile, HttpOnly and Secure included.
    async fn cookies(&self, url: &Url) -> Result<Vec<CookiePair>, AuthUiError>;

    /// Closes the window. Dropping a window without calling `close` must close it too.
    async fn close(self: Box<Self>);
}
