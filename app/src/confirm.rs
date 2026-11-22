//! Confirmations outside the chat (spec 013): the toasts and the detail window of a request. What is shown, and
//! how every answer ends the request, is decided in `mcp_proxy_gateway::notify`; this module only shows and passes
//! the answers on.
//!
//! The detail window is a local page of the bundle (`confirm.html`), labelled `confirm-<id>`, with its own
//! capability (`capabilities/confirm.json`) that grants its only command. It receives its request, already
//! filtered, through an initialization script.

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use mcp_proxy_auth::webview::{app_page_allowed, destination_for_log};
use mcp_proxy_gateway::notify::{
    ConfirmationId, ConfirmationPresenter, DetailView, NotificationBroker, PresentError, ToastContent, ToastFollowUp,
    ToastState, confirmation_of_label, confirmation_tag, detail_label, detail_script,
};
use mcp_proxy_gateway::runtime::Gateway;
use serde_json::Value;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::notify::{ToastEvent, Toaster};

/// The broker of the running gateway; `None` before it started or after it stopped.
fn broker(app: &AppHandle) -> Option<Arc<NotificationBroker>> {
    app.try_state::<Arc<Gateway>>().map(|g| Arc::clone(g.confirmations()))
}

/// What a toast reports, passed on to the waiting request.
fn on_toast_event(app: &AppHandle, id: ConfirmationId, event: ToastEvent) {
    let Some(broker) = broker(app) else {
        return;
    };
    match event {
        ToastEvent::Activated(arguments) => {
            if broker.pending().toast_activated(id, arguments.as_deref()) == ToastFollowUp::OpenDetail {
                tauri::async_runtime::spawn(async move { broker.show_detail(id).await });
            }
        }
        ToastEvent::Dismissed(reason) => broker.pending().toast_dismissed(id, reason),
        ToastEvent::Failed => {
            tauri::async_runtime::spawn(async move { broker.toast_failed(id).await });
        }
    }
}

pub struct WindowsPresenter {
    app: AppHandle,
    toaster: Arc<Toaster>,
}

impl WindowsPresenter {
    pub fn new(app: &AppHandle, toaster: Arc<Toaster>) -> Self {
        Self {
            app: app.clone(),
            toaster,
        }
    }
}

fn bring_to_front(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn build_detail_window(app: &AppHandle, id: ConfirmationId, view: &DetailView) -> Result<(), PresentError> {
    let label = detail_label(id);
    if let Some(window) = app.get_webview_window(&label) {
        bring_to_front(&window);
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("confirm.html".into()))
        .title(view.title())
        .inner_size(560.0, 600.0)
        .center()
        .visible(true)
        .focused(true)
        .always_on_top(true)
        .devtools(false)
        .initialization_script(detail_script(view))
        .on_navigation(|url| {
            let allowed = app_page_allowed(url);
            if !allowed {
                tracing::warn!(destination = %destination_for_log(url), "navigation de la fenêtre de confirmation bloquée");
            }
            allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()
        .map_err(|error| {
            tracing::error!(%error, "fenêtre de confirmation impossible à ouvrir");
            PresentError("fenêtre de confirmation impossible à ouvrir".to_owned())
        })?;
    let closed = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed)
            && let Some(broker) = broker(&closed)
        {
            broker.pending().detail_closed(id);
        }
    });
    bring_to_front(&window);
    // Settled while the window was being built: it shows nothing that waits.
    if broker(app).is_none_or(|b| !b.pending().is_waiting(id)) {
        let _ = window.destroy();
    }
    Ok(())
}

#[async_trait]
impl ConfirmationPresenter for WindowsPresenter {
    async fn toast_state(&self) -> ToastState {
        let toaster = Arc::clone(&self.toaster);
        tauri::async_runtime::spawn_blocking(move || toaster.state())
            .await
            .unwrap_or_else(|_| ToastState::Unavailable("état des notifications illisible".to_owned()))
    }

    async fn show_toast(&self, id: ConfirmationId, toast: ToastContent) -> Result<(), PresentError> {
        let toaster = Arc::clone(&self.toaster);
        let app = self.app.clone();
        let expires = toast
            .deadline
            .map(|deadline| SystemTime::now() + deadline.saturating_duration_since(Instant::now()));
        tauri::async_runtime::spawn_blocking(move || {
            toaster.show(&toast, expires, move |event| on_toast_event(&app, id, event))
        })
        .await
        .unwrap_or_else(|_| Err("notification interrompue".to_owned()))
        .map_err(PresentError)
    }

    async fn open_detail(&self, id: ConfirmationId, view: DetailView) -> Result<(), PresentError> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || build_detail_window(&app, id, &view))
            .await
            .unwrap_or_else(|_| Err(PresentError("fenêtre de confirmation interrompue".to_owned())))
    }

    fn withdraw(&self, id: ConfirmationId) {
        let toaster = Arc::clone(&self.toaster);
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            toaster.hide(&confirmation_tag(id));
            if let Some(window) = app.get_webview_window(&detail_label(id)) {
                let _ = window.destroy();
            }
        });
    }
}

/// The answer of the detail window: `"approve"` approves, anything else refuses. Nothing is answered back; a
/// window that is not a detail window, or whose request is over, changes nothing.
#[tauri::command]
pub async fn answer_confirmation(window: WebviewWindow, answer: Option<Value>) {
    let Some(id) = confirmation_of_label(window.label()) else {
        tracing::warn!("réponse de confirmation d'une autre fenêtre ignorée");
        return;
    };
    let Some(broker) = broker(window.app_handle()) else {
        return;
    };
    if !broker.pending().detail_answer(id, answer.as_ref()) {
        tracing::info!(id = id.raw(), "réponse de confirmation ignorée : demande terminée");
    }
}
