//! The login windows open right now, and the checks of what their pages hand over.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use mcp_proxy_core::config::SessionId;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::sync::mpsc;
use url::Url;

use super::navigation::navigation_allowed;
use crate::capture::{CaptureRejection, MAX_CANDIDATE_LEN, check_jwt};
use crate::ui::{HeaderCapture, LoginEvent, LoginWindowSpec};

/// Why a deposit was ignored. Logged with the session, never with the value nor the URL; never told to the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DepositRejection {
    #[error("fenêtre appelante autre que celle de la connexion en cours")]
    UnknownWindow,
    #[error("fenêtre sans capture d'en-tête")]
    NoCapture,
    #[error("jeton déjà retenu pour cette fenêtre")]
    AlreadyCaptured,
    #[error("page appelante hors des hôtes autorisés")]
    PageHost,
    #[error("valeur absente ou non textuelle")]
    NoValue,
    #[error("valeur trop longue")]
    TooLong,
    #[error("URL de la requête absente ou illisible")]
    NoUrl,
    #[error("{0}")]
    Capture(#[from] CaptureRejection),
}

struct Entry {
    session: SessionId,
    allowed: Vec<String>,
    capture: Option<HeaderCapture>,
    events: mpsc::UnboundedSender<LoginEvent>,
    captured: bool,
}

/// The login windows open right now, by window label. The authenticator opens one at a time; a label is never
/// reused, so a window that is gone, and any capability attached to its label, stays without effect.
#[derive(Default)]
pub struct LoginWindows {
    entries: Mutex<HashMap<String, Entry>>,
}

impl std::fmt::Debug for LoginWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginWindows")
            .field("open", &self.lock().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl LoginWindows {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers the window `label` for `spec`; its events arrive on the receiver.
    pub fn open(&self, label: &str, spec: &LoginWindowSpec) -> mpsc::UnboundedReceiver<LoginEvent> {
        let (events, receiver) = mpsc::unbounded_channel();
        self.lock().insert(
            label.to_owned(),
            Entry {
                session: spec.session().clone(),
                allowed: spec.allowed_navigation().to_vec(),
                capture: spec.capture().cloned(),
                events,
                captured: false,
            },
        );
        receiver
    }

    pub fn is_open(&self, label: &str) -> bool {
        self.lock().contains_key(label)
    }

    /// A deposit of the capture script, from the window `label` whose top-level page is `page`. Answers nothing,
    /// whatever happens: a refusal is logged without the value nor the URL, and only the first valid token of the
    /// window becomes a [`LoginEvent::HeaderCaptured`].
    pub fn deposit(
        &self,
        label: &str,
        page: Option<&Url>,
        url: Option<&str>,
        value: Option<SecretString>,
        now: SystemTime,
    ) {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(label) else {
            tracing::info!(reason = %DepositRejection::UnknownWindow, "dépôt d'en-tête ignoré");
            return;
        };
        match check(entry, page, url, value, now) {
            Ok(event) => {
                entry.captured = true;
                let _ = entry.events.send(event);
            }
            Err(reason) => tracing::info!(
                session = entry.session.as_str(),
                %reason,
                "dépôt d'en-tête ignoré"
            ),
        }
    }

    /// The top-level page of `label` is now `url`. Ignored if the host is not allowed (the navigation was refused).
    pub fn navigated(&self, label: &str, url: &Url) {
        if let Some(entry) = self.lock().get(label)
            && navigation_allowed(&entry.allowed, url)
        {
            let _ = entry.events.send(LoginEvent::Navigated { url: url.clone() });
        }
    }

    /// The user closed the window: [`LoginEvent::Closed`], and the window is forgotten.
    pub fn closed(&self, label: &str) {
        if let Some(entry) = self.lock().remove(label) {
            let _ = entry.events.send(LoginEvent::Closed);
        }
    }

    /// The application closes the window: forgotten without any event.
    pub fn forget(&self, label: &str) {
        self.lock().remove(label);
    }
}

fn check(
    entry: &Entry,
    page: Option<&Url>,
    url: Option<&str>,
    value: Option<SecretString>,
    now: SystemTime,
) -> Result<LoginEvent, DepositRejection> {
    let capture = entry.capture.as_ref().ok_or(DepositRejection::NoCapture)?;
    if entry.captured {
        return Err(DepositRejection::AlreadyCaptured);
    }
    if !page.is_some_and(|page| navigation_allowed(&entry.allowed, page)) {
        return Err(DepositRejection::PageHost);
    }
    let value = value.ok_or(DepositRejection::NoValue)?;
    if value.expose_secret().len() > MAX_CANDIDATE_LEN {
        return Err(DepositRejection::TooLong);
    }
    let url = url.and_then(|u| Url::parse(u).ok()).ok_or(DepositRejection::NoUrl)?;
    let token = capture.candidate_token(&url, capture.header(), value.expose_secret())?;
    check_jwt(token, now, capture.expectations()).map_err(CaptureRejection::from)?;
    Ok(LoginEvent::HeaderCaptured {
        url,
        header: capture.header().to_owned(),
        value,
    })
}
