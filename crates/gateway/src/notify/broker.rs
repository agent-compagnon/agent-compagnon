//! The `ConfirmationBroker` of the application: Windows notification, or the detail window when toasts cannot be
//! seen or the request does not fit in one. The application only shows and passes answers on, through a
//! [`ConfirmationPresenter`]; what is shown and how a request ends is decided here.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_proxy_core::confirm::{ConfirmationBroker, ConfirmationOutcome, ConfirmationRequest};
use thiserror::Error;

use super::detail::DetailView;
use super::pending::{ConfirmationId, PendingConfirmations};
use super::toast::{Presentation, ToastContent, ToastState, confirmation_toast, fits_in_toast, presentation};
use crate::server::OutputGate;

/// Why something could not be shown. A fixed French text for the log, never a value of the request.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct PresentError(pub String);

/// What the application shows. Every method is given texts that are already filtered.
#[async_trait]
pub trait ConfirmationPresenter: Send + Sync {
    /// Whether a toast would be seen right now.
    async fn toast_state(&self) -> ToastState;

    /// Shows the toast of `id`. Its buttons and dismissal are reported to [`PendingConfirmations`]; a toast that
    /// fails once shown is reported with [`NotificationBroker::toast_failed`].
    async fn show_toast(&self, id: ConfirmationId, toast: ToastContent) -> Result<(), PresentError>;

    /// Opens the detail window of `id` in front; closing it must be reported with
    /// [`PendingConfirmations::detail_closed`].
    async fn open_detail(&self, id: ConfirmationId, view: DetailView) -> Result<(), PresentError>;

    /// The request `id` is over, whatever the outcome: remove its toast and close its detail window. Must not
    /// block: called when the wait ends, including when the chain drops it.
    fn withdraw(&self, id: ConfirmationId);
}

/// Shows nothing: every confirmation outside the chat is refused as unavailable. For tests and assemblies
/// without windows.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailablePresenter;

#[async_trait]
impl ConfirmationPresenter for UnavailablePresenter {
    async fn toast_state(&self) -> ToastState {
        ToastState::Unavailable("aucune interface".to_owned())
    }

    async fn show_toast(&self, _id: ConfirmationId, _toast: ToastContent) -> Result<(), PresentError> {
        Err(PresentError("aucune interface".to_owned()))
    }

    async fn open_detail(&self, _id: ConfirmationId, _view: DetailView) -> Result<(), PresentError> {
        Err(PresentError("aucune interface".to_owned()))
    }

    fn withdraw(&self, _id: ConfirmationId) {}
}

pub struct NotificationBroker {
    pending: Arc<PendingConfirmations>,
    presenter: Arc<dyn ConfirmationPresenter>,
    output: OutputGate,
}

impl fmt::Debug for NotificationBroker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotificationBroker")
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

/// Withdraws what was shown for a request when its wait ends, however it ends.
struct Shown<'a> {
    presenter: &'a dyn ConfirmationPresenter,
    id: ConfirmationId,
}

impl Drop for Shown<'_> {
    fn drop(&mut self) {
        self.presenter.withdraw(self.id);
    }
}

impl NotificationBroker {
    /// `output` filters every text once more before it is shown (defence in depth: the chain already did).
    pub fn new(
        pending: Arc<PendingConfirmations>,
        presenter: Arc<dyn ConfirmationPresenter>,
        output: OutputGate,
    ) -> Self {
        Self {
            pending,
            presenter,
            output,
        }
    }

    pub fn pending(&self) -> &Arc<PendingConfirmations> {
        &self.pending
    }

    fn refilter(&self, request: ConfirmationRequest) -> Option<ConfirmationRequest> {
        Some(ConfirmationRequest {
            path: self.output.text(&request.path)?,
            client: self.output.text(&request.client)?,
            arguments: self.output.value(&request.arguments)?,
            explanation: self.output.text(&request.explanation)?,
            ..request
        })
    }

    /// Opens the detail window of `id` if it still waits; if the window cannot be opened, the request ends as
    /// unavailable.
    pub async fn show_detail(&self, id: ConfirmationId) {
        let Some(request) = self.pending.request(id) else {
            return;
        };
        if let Err(error) = self.presenter.open_detail(id, DetailView::new(id, &request)).await {
            tracing::error!(%error, "fenêtre de confirmation impossible à ouvrir : appel refusé");
            self.pending.unavailable(id);
        }
    }

    /// Windows reported that the toast of `id` could not be shown: the detail window replaces it.
    pub async fn toast_failed(&self, id: ConfirmationId) {
        tracing::warn!(
            id = id.raw(),
            "notification de confirmation non affichée : fenêtre de détail"
        );
        self.show_detail(id).await;
    }
}

#[async_trait]
impl ConfirmationBroker for NotificationBroker {
    async fn confirm(&self, request: ConfirmationRequest) -> ConfirmationOutcome {
        let Some(request) = self.refilter(request) else {
            tracing::error!("filtre indisponible : confirmation non affichée");
            return ConfirmationOutcome::Unavailable;
        };
        let fits = fits_in_toast(&request);
        let state = self.presenter.toast_state().await;
        if let ToastState::Unavailable(reason) = &state {
            tracing::info!(%reason, "notifications indisponibles : fenêtre de confirmation");
        }
        let plan = presentation(&state, fits);
        let toast = matches!(plan, Presentation::Toast { approve: true });
        let Ok(waiting) = self.pending.register(request.clone(), toast) else {
            return ConfirmationOutcome::Refused;
        };
        let id = waiting.id();
        let _shown = Shown {
            presenter: self.presenter.as_ref(),
            id,
        };
        match plan {
            Presentation::Toast { approve } => {
                let content = confirmation_toast(id, &request, approve);
                if let Err(error) = self.presenter.show_toast(id, content).await {
                    tracing::warn!(%error, "notification de confirmation impossible : fenêtre de détail");
                    self.show_detail(id).await;
                }
            }
            Presentation::DetailWindow => self.show_detail(id).await,
        }
        waiting.outcome().await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Mutex;
    use std::time::Duration;

    use mcp_proxy_core::config::{EnvId, McpId, ToolName};
    use mcp_proxy_core::filter::OutputFilter;
    use mcp_proxy_core::secrets::{Credential, SecretStore};
    use serde_json::{Value, json};

    use super::*;
    use crate::notify::pending::{Answer, AnswerSource};
    use crate::notify::toast::DismissReason;

    const SENTINEL: &str = "SENTINEL-c0nf1rm-7f3a9b2e";

    #[derive(Debug, Clone, PartialEq)]
    enum Shown {
        Toast(ConfirmationId, ToastContent),
        Detail(ConfirmationId, DetailView),
        Withdrawn(ConfirmationId),
    }

    #[derive(Default)]
    struct FakePresenter {
        state: Mutex<Option<ToastState>>,
        toast_fails: bool,
        detail_fails: bool,
        shown: Mutex<Vec<Shown>>,
    }

    impl FakePresenter {
        fn with_state(state: ToastState) -> Self {
            Self {
                state: Mutex::new(Some(state)),
                ..Self::default()
            }
        }

        fn shown(&self) -> Vec<Shown> {
            self.shown.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ConfirmationPresenter for FakePresenter {
        async fn toast_state(&self) -> ToastState {
            self.state.lock().unwrap().clone().unwrap_or(ToastState::Available)
        }

        async fn show_toast(&self, id: ConfirmationId, toast: ToastContent) -> Result<(), PresentError> {
            if self.toast_fails {
                return Err(PresentError("toast".to_owned()));
            }
            self.shown.lock().unwrap().push(Shown::Toast(id, toast));
            Ok(())
        }

        async fn open_detail(&self, id: ConfirmationId, view: DetailView) -> Result<(), PresentError> {
            if self.detail_fails {
                return Err(PresentError("fenêtre".to_owned()));
            }
            self.shown.lock().unwrap().push(Shown::Detail(id, view));
            Ok(())
        }

        fn withdraw(&self, id: ConfirmationId) {
            self.shown.lock().unwrap().push(Shown::Withdrawn(id));
        }
    }

    fn output_with_secret() -> OutputGate {
        let store = Arc::new(SecretStore::for_platform().unwrap());
        store
            .put(
                &mcp_proxy_core::config::SessionId::new("sso").unwrap(),
                Credential::Bearer {
                    token: SENTINEL.to_owned().into(),
                },
            )
            .unwrap();
        let config = mcp_proxy_core::config::FilterConfig::default();
        OutputGate::new(Arc::new(OutputFilter::new(store, &config).unwrap()))
    }

    fn request(arguments: Value) -> ConfirmationRequest {
        ConfirmationRequest {
            env: EnvId::new("prod").unwrap(),
            mcp: McpId::new("kibana-logs").unwrap(),
            tool: ToolName::new("delete_index").unwrap(),
            path: "PROD / kibana-logs / delete_index".to_owned(),
            client: "Claude Desktop".to_owned(),
            arguments,
            explanation: "outil destructif en PROD".to_owned(),
            deadline: (tokio::time::Instant::now() + Duration::from_secs(30)).into_std(),
        }
    }

    fn new_broker(presenter: Arc<FakePresenter>) -> Arc<NotificationBroker> {
        Arc::new(NotificationBroker::new(
            PendingConfirmations::new(),
            presenter,
            output_with_secret(),
        ))
    }

    fn only(shown: Vec<Shown>) -> Shown {
        assert_eq!(shown.len(), 1, "{shown:?}");
        shown[0].clone()
    }

    /// Runs `confirm` and returns once it has shown something.
    async fn ask(broker: &Arc<NotificationBroker>, arguments: Value) -> tokio::task::JoinHandle<ConfirmationOutcome> {
        let running = {
            let broker = Arc::clone(broker);
            tokio::spawn(async move { broker.confirm(request(arguments)).await })
        };
        while broker.pending().count() == 0 && !running.is_finished() {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        running
    }

    // Exigence : EX-013-01, EX-013-02
    #[tokio::test(start_paused = true)]
    async fn short_request_is_approved_from_its_toast() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        let Shown::Toast(id, toast) = only(presenter.shown()) else {
            panic!("{:?}", presenter.shown());
        };
        assert_eq!(toast.buttons[1].label, "Autoriser une fois");
        assert_eq!(
            broker.pending().toast_activated(id, Some(&toast.buttons[1].action)),
            crate::notify::ToastFollowUp::Nothing
        );
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Approved);
        assert_eq!(presenter.shown().last(), Some(&Shown::Withdrawn(id)));
        assert_eq!(broker.pending().count(), 0);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn ignored_toast_times_out_and_is_withdrawn() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::TimedOut);
        let shown = presenter.shown();
        assert!(
            matches!(shown.as_slice(), [Shown::Toast(..), Shown::Withdrawn(_)]),
            "{shown:?}"
        );
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn dismissed_toast_refuses() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        let Shown::Toast(id, _) = presenter.shown()[0].clone() else {
            panic!()
        };
        broker.pending().toast_dismissed(id, DismissReason::UserCanceled);
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
    }

    // Exigence : EX-013-01, EX-013-02
    #[tokio::test(start_paused = true)]
    async fn long_request_is_approved_only_from_the_detail_window() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let long = "x".repeat(400);
        let running = ask(&broker, json!({ "query": long })).await;
        let Shown::Toast(id, toast) = presenter.shown()[0].clone() else {
            panic!()
        };
        assert_eq!(toast.buttons[1].label, "Voir le détail");
        let follow_up = broker.pending().toast_activated(id, Some(&toast.buttons[1].action));
        assert_eq!(follow_up, crate::notify::ToastFollowUp::OpenDetail);
        broker.show_detail(id).await;
        let Shown::Detail(_, view) = presenter.shown()[1].clone() else {
            panic!()
        };
        assert!(view.arguments.contains(&long), "the window shows everything");
        assert!(broker.pending().detail_answer(id, Some(&json!("approve"))));
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Approved);

        let running = ask(&broker, json!({ "query": "y".repeat(400) })).await;
        let Shown::Toast(id, _) = presenter.shown()[3].clone() else {
            panic!("{:?}", presenter.shown())
        };
        // An approval forged on a toast that did not offer it refuses.
        broker
            .pending()
            .toast_activated(id, Some(&format!("confirm:approve:{id}")));
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn closing_the_detail_window_refuses() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "query": "z".repeat(400) })).await;
        let Shown::Toast(id, _) = presenter.shown()[0].clone() else {
            panic!()
        };
        broker.show_detail(id).await;
        broker.pending().detail_closed(id);
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn without_toasts_the_detail_window_opens_at_once() {
        let state = ToastState::Unavailable("mode « Ne pas déranger » (prioritaires seulement)".to_owned());
        let presenter = Arc::new(FakePresenter::with_state(state));
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        let Shown::Detail(id, _) = only(presenter.shown()) else {
            panic!("{:?}", presenter.shown());
        };
        assert_eq!(broker.pending().count(), 1, "the icon turns orange");
        // No toast was shown: an approval said to come from one refuses.
        broker.pending().answer(id, AnswerSource::Toast, Answer::Approve);
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn a_toast_that_cannot_be_shown_falls_back_to_the_window() {
        let presenter = Arc::new(FakePresenter {
            toast_fails: true,
            ..FakePresenter::default()
        });
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        let Shown::Detail(id, _) = only(presenter.shown()) else {
            panic!("{:?}", presenter.shown());
        };
        assert!(broker.pending().detail_answer(id, Some(&json!("approve"))));
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Approved);

        // Shown, then reported failed by Windows.
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        let Shown::Toast(id, _) = presenter.shown()[0].clone() else {
            panic!()
        };
        broker.toast_failed(id).await;
        assert!(matches!(presenter.shown()[1], Shown::Detail(..)));
        broker.pending().detail_closed(id);
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn nothing_to_show_with_is_unavailable() {
        let presenter = Arc::new(FakePresenter {
            toast_fails: true,
            detail_fails: true,
            ..FakePresenter::default()
        });
        let broker = new_broker(Arc::clone(&presenter));
        let outcome = broker.confirm(request(json!({}))).await;
        assert_eq!(outcome, ConfirmationOutcome::Unavailable);
        assert_eq!(broker.pending().count(), 0);

        let broker = Arc::new(NotificationBroker::new(
            PendingConfirmations::new(),
            Arc::new(UnavailablePresenter),
            output_with_secret(),
        ));
        assert_eq!(
            broker.confirm(request(json!({}))).await,
            ConfirmationOutcome::Unavailable
        );
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn dropped_by_the_chain_or_stopped_application() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        let running = ask(&broker, json!({ "index": "logs" })).await;
        let Shown::Toast(id, _) = presenter.shown()[0].clone() else {
            panic!()
        };
        running.abort();
        let _ = running.await;
        assert_eq!(broker.pending().count(), 0);
        assert_eq!(
            presenter.shown().last(),
            Some(&Shown::Withdrawn(id)),
            "withdrawn when dropped"
        );

        let running = ask(&broker, json!({ "index": "logs" })).await;
        broker.pending().shutdown();
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
        assert_eq!(broker.confirm(request(json!({}))).await, ConfirmationOutcome::Refused);
    }

    // Exigence : EX-013-03
    #[tokio::test(start_paused = true)]
    async fn nothing_secret_is_shown() {
        let presenter = Arc::new(FakePresenter::default());
        let broker = new_broker(Arc::clone(&presenter));
        // The chain filters before; the broker filters again: a sentinel that got through is masked.
        let leaked = json!({ "token": SENTINEL, "note": format!("Bearer {SENTINEL}") });
        let running = ask(&broker, leaked).await;
        let Shown::Toast(id, toast) = presenter.shown()[0].clone() else {
            panic!()
        };
        broker.show_detail(id).await;
        let Shown::Detail(_, view) = presenter.shown()[1].clone() else {
            panic!()
        };
        broker.pending().detail_closed(id);
        assert_eq!(running.await.unwrap(), ConfirmationOutcome::Refused);
        let everything = format!("{toast:?} {view:?} {}", crate::notify::detail_script(&view));
        assert!(!everything.contains(SENTINEL), "{everything}");
        assert!(!everything.contains("c0nf1rm"), "{everything}");
    }
}
