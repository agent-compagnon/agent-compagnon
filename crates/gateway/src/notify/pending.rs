//! The confirmations waiting for the user. A request is settled once, by the first answer that arrives (a button of
//! the toast, the detail window); only an explicit approval lets the call through (INV-5). Everything else refuses:
//! a refusal, the detail window closed, a dismissed toast, an approval the toast did not offer, an unreadable
//! answer, the application stopping. Without any answer, the deadline gives `TimedOut`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use mcp_proxy_core::confirm::{ConfirmationOutcome, ConfirmationRequest};
use serde_json::Value;
use tokio::sync::{oneshot, watch};

use super::toast::{DismissReason, ToastAction};

/// Identifies a waiting confirmation; never reused in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfirmationId(u64);

impl ConfirmationId {
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ConfirmationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Approve,
    Refuse,
}

/// Where an answer comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerSource {
    Toast,
    DetailWindow,
}

/// What the application must do after an activation of a confirmation toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastFollowUp {
    /// The request was settled (or was not waiting any more): nothing to show.
    Nothing,
    /// « Voir le détail » or a click on the body: open the detail window.
    OpenDetail,
}

/// How a request ended, before the deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    Approved,
    Refused,
    /// Neither a toast nor the detail window could be shown.
    Unavailable,
}

struct Entry {
    request: ConfirmationRequest,
    /// The toast carries « Autoriser une fois ».
    toast_approves: bool,
    settle: oneshot::Sender<Resolution>,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<ConfirmationId, Entry>,
    closed: bool,
}

/// The registry of waiting confirmations, shared by the broker, the toasts and the detail windows.
pub struct PendingConfirmations {
    state: Mutex<State>,
    next: AtomicU64,
    count: watch::Sender<usize>,
}

impl fmt::Debug for PendingConfirmations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingConfirmations")
            .field("waiting", &self.lock().entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for PendingConfirmations {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            next: AtomicU64::new(1),
            count: watch::Sender::new(0),
        }
    }
}

/// The application is stopping: no confirmation is asked any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

impl PendingConfirmations {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, state: &State) {
        self.count.send_replace(state.entries.len());
    }

    /// Registers `request`; `toast_approves` tells whether its toast offers « Autoriser une fois ». The request
    /// waits until settled, until the deadline, or until the [`Waiting`] is dropped.
    pub fn register(self: &Arc<Self>, request: ConfirmationRequest, toast_approves: bool) -> Result<Waiting, Closed> {
        let (settle, settled) = oneshot::channel();
        let deadline = request.deadline;
        let id = ConfirmationId(self.next.fetch_add(1, Ordering::Relaxed));
        let mut state = self.lock();
        if state.closed {
            return Err(Closed);
        }
        state.entries.insert(
            id,
            Entry {
                request,
                toast_approves,
                settle,
            },
        );
        self.publish(&state);
        Ok(Waiting {
            id,
            deadline,
            settled: Some(settled),
            pending: Arc::clone(self),
        })
    }

    fn settle(&self, id: ConfirmationId, resolution: Resolution) -> bool {
        let mut state = self.lock();
        let Some(entry) = state.entries.remove(&id) else {
            return false;
        };
        self.publish(&state);
        drop(state);
        let _ = entry.settle.send(resolution);
        true
    }

    /// An answer of the user. Only the first answer of a waiting request counts: `false` for a second answer or
    /// an unknown request, which change nothing. An approval from a toast that did not offer it refuses.
    pub fn answer(&self, id: ConfirmationId, source: AnswerSource, answer: Answer) -> bool {
        let resolution = match answer {
            Answer::Refuse => Resolution::Refused,
            Answer::Approve => {
                let offered = match source {
                    AnswerSource::DetailWindow => true,
                    AnswerSource::Toast => self.lock().entries.get(&id).is_some_and(|e| e.toast_approves),
                };
                if offered {
                    Resolution::Approved
                } else {
                    tracing::warn!(id = id.raw(), "approbation que la notification n'offrait pas : refus");
                    Resolution::Refused
                }
            }
        };
        self.settle(id, resolution)
    }

    /// The toast of `id` was activated with `arguments`. Arguments that name another request, or that this
    /// application did not write, refuse `id`.
    pub fn toast_activated(&self, id: ConfirmationId, arguments: Option<&str>) -> ToastFollowUp {
        match arguments.and_then(ToastAction::parse) {
            Some(ToastAction::Approve(of)) if of == id => {
                self.answer(id, AnswerSource::Toast, Answer::Approve);
                ToastFollowUp::Nothing
            }
            Some(ToastAction::Refuse(of)) if of == id => {
                self.answer(id, AnswerSource::Toast, Answer::Refuse);
                ToastFollowUp::Nothing
            }
            Some(ToastAction::Detail(of)) if of == id && self.is_waiting(id) => ToastFollowUp::OpenDetail,
            Some(ToastAction::Detail(of)) if of == id => ToastFollowUp::Nothing,
            _ => {
                tracing::warn!(
                    id = id.raw(),
                    "activation inattendue d'une notification de confirmation : refus"
                );
                self.answer(id, AnswerSource::Toast, Answer::Refuse);
                ToastFollowUp::Nothing
            }
        }
    }

    /// The toast of `id` went away without a button: dismissed by the user, sent to the notification centre, or
    /// for an unknown reason, it refuses. Hidden by the application: it was already settled.
    pub fn toast_dismissed(&self, id: ConfirmationId, reason: DismissReason) {
        if reason != DismissReason::ApplicationHidden {
            self.answer(id, AnswerSource::Toast, Answer::Refuse);
        }
    }

    /// The answer sent by the page of the detail window: `"approve"` approves, anything else refuses.
    pub fn detail_answer(&self, id: ConfirmationId, answer: Option<&Value>) -> bool {
        let answer = match answer {
            Some(Value::String(answer)) if answer == "approve" => Answer::Approve,
            _ => Answer::Refuse,
        };
        self.answer(id, AnswerSource::DetailWindow, answer)
    }

    /// The detail window of `id` was closed: refused, unless already settled.
    pub fn detail_closed(&self, id: ConfirmationId) {
        self.answer(id, AnswerSource::DetailWindow, Answer::Refuse);
    }

    /// Nothing could show the request: the call is refused as « unavailable ».
    pub fn unavailable(&self, id: ConfirmationId) {
        self.settle(id, Resolution::Unavailable);
    }

    pub fn is_waiting(&self, id: ConfirmationId) -> bool {
        self.lock().entries.contains_key(&id)
    }

    /// The request, already filtered, while it waits: what the detail window shows.
    pub fn request(&self, id: ConfirmationId) -> Option<ConfirmationRequest> {
        self.lock().entries.get(&id).map(|e| e.request.clone())
    }

    pub fn count(&self) -> usize {
        self.lock().entries.len()
    }

    /// The number of waiting requests, on every change.
    pub fn subscribe(&self) -> watch::Receiver<usize> {
        self.count.subscribe()
    }

    /// Every waiting request is refused; later ones are asked as usual. For the suspension of the gateway (spec 015),
    /// which can be lifted.
    pub fn refuse_all(&self) {
        let entries = {
            let mut state = self.lock();
            let entries = std::mem::take(&mut state.entries);
            self.publish(&state);
            entries
        };
        for entry in entries.into_values() {
            let _ = entry.settle.send(Resolution::Refused);
        }
    }

    /// The application stops: every waiting request is refused, and any later one is refused at once.
    pub fn shutdown(&self) {
        self.lock().closed = true;
        self.refuse_all();
    }
}

/// A registered request. Dropping it forgets the request: an answer that arrives later finds nothing.
pub struct Waiting {
    id: ConfirmationId,
    deadline: Instant,
    settled: Option<oneshot::Receiver<Resolution>>,
    pending: Arc<PendingConfirmations>,
}

impl fmt::Debug for Waiting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Waiting").field("id", &self.id).finish_non_exhaustive()
    }
}

impl Waiting {
    pub fn id(&self) -> ConfirmationId {
        self.id
    }

    /// Waits for the first answer, until the deadline of the request.
    pub async fn outcome(mut self) -> ConfirmationOutcome {
        let Some(settled) = self.settled.take() else {
            return ConfirmationOutcome::Refused;
        };
        let deadline = tokio::time::Instant::from_std(self.deadline);
        match tokio::time::timeout_at(deadline, settled).await {
            Ok(Ok(Resolution::Approved)) => ConfirmationOutcome::Approved,
            Ok(Ok(Resolution::Unavailable)) => ConfirmationOutcome::Unavailable,
            Ok(Ok(Resolution::Refused) | Err(_)) => ConfirmationOutcome::Refused,
            Err(_) => ConfirmationOutcome::TimedOut,
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut state = self.pending.lock();
        if state.entries.remove(&self.id).is_some() {
            self.pending.publish(&state);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::Duration;

    use mcp_proxy_core::config::{EnvId, McpId, ToolName};
    use serde_json::json;

    use super::*;

    fn request(secs: u64) -> ConfirmationRequest {
        ConfirmationRequest {
            env: EnvId::new("prod").unwrap(),
            mcp: McpId::new("kibana-logs").unwrap(),
            tool: ToolName::new("delete_index").unwrap(),
            path: "PROD / kibana-logs / delete_index".to_owned(),
            client: "Claude Desktop".to_owned(),
            arguments: json!({ "index": "logs" }),
            explanation: "confirmation demandée".to_owned(),
            deadline: (tokio::time::Instant::now() + Duration::from_secs(secs)).into_std(),
        }
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn approval_and_refusal() {
        let pending = PendingConfirmations::new();
        let counts = pending.subscribe();
        let waiting = pending.register(request(30), true).unwrap();
        let id = waiting.id();
        assert_eq!(pending.count(), 1);
        assert_eq!(*counts.borrow(), 1);
        assert_eq!(pending.request(id).unwrap().path, "PROD / kibana-logs / delete_index");
        assert!(pending.answer(id, AnswerSource::Toast, Answer::Approve));
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Approved);
        assert_eq!(pending.count(), 0);
        assert_eq!(*counts.borrow(), 0);

        let waiting = pending.register(request(30), true).unwrap();
        assert!(pending.answer(waiting.id(), AnswerSource::Toast, Answer::Refuse));
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused);

        let waiting = pending.register(request(30), false).unwrap();
        assert!(pending.detail_answer(waiting.id(), Some(&json!("approve"))));
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Approved);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn deadline_gives_timed_out() {
        let pending = PendingConfirmations::new();
        let waiting = pending.register(request(30), true).unwrap();
        let id = waiting.id();
        let started = tokio::time::Instant::now();
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::TimedOut);
        assert_eq!(started.elapsed(), Duration::from_secs(30));
        assert_eq!(pending.count(), 0, "forgotten at the deadline");
        assert!(!pending.answer(id, AnswerSource::Toast, Answer::Approve), "too late");
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn only_the_first_answer_counts() {
        let pending = PendingConfirmations::new();
        let waiting = pending.register(request(30), true).unwrap();
        let id = waiting.id();
        assert!(pending.answer(id, AnswerSource::Toast, Answer::Refuse));
        assert!(
            !pending.answer(id, AnswerSource::DetailWindow, Answer::Approve),
            "second answer ignored"
        );
        assert!(!pending.detail_answer(id, Some(&json!("approve"))));
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused);

        let waiting = pending.register(request(30), true).unwrap();
        let id = waiting.id();
        assert!(pending.answer(id, AnswerSource::DetailWindow, Answer::Approve));
        pending.detail_closed(id);
        pending.toast_dismissed(id, DismissReason::UserCanceled);
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Approved);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn unknown_requests_change_nothing() {
        let pending = PendingConfirmations::new();
        let unknown = ConfirmationId::from_raw(999);
        assert!(!pending.answer(unknown, AnswerSource::DetailWindow, Answer::Approve));
        assert!(!pending.detail_answer(unknown, Some(&json!("approve"))));
        assert_eq!(
            pending.toast_activated(unknown, Some("confirm:detail:999")),
            ToastFollowUp::Nothing
        );
        pending.detail_closed(unknown);
        pending.unavailable(unknown);
        assert!(pending.request(unknown).is_none());

        // An answer for a request that is gone (dropped by the chain at its own deadline) is ignored too.
        let waiting = pending.register(request(30), true).unwrap();
        let id = waiting.id();
        drop(waiting);
        assert_eq!(pending.count(), 0);
        assert!(!pending.answer(id, AnswerSource::Toast, Answer::Approve));
        let next = pending.register(request(30), true).unwrap();
        assert_ne!(next.id(), id, "identifiers are never reused");
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn shutdown_refuses_everything_waiting() {
        let pending = PendingConfirmations::new();
        let first = pending.register(request(30), true).unwrap();
        let second = pending.register(request(30), false).unwrap();
        assert_eq!(pending.count(), 2);
        pending.shutdown();
        assert_eq!(pending.count(), 0);
        assert_eq!(first.outcome().await, ConfirmationOutcome::Refused);
        assert_eq!(second.outcome().await, ConfirmationOutcome::Refused);
        assert_eq!(pending.register(request(30), true).err(), Some(Closed));
    }

    // Exigence : EX-015-02
    #[tokio::test(start_paused = true)]
    async fn refusing_everything_waiting_can_be_lifted() {
        let pending = PendingConfirmations::new();
        let first = pending.register(request(30), true).unwrap();
        let second = pending.register(request(30), false).unwrap();
        pending.refuse_all();
        assert_eq!(pending.count(), 0);
        assert_eq!(first.outcome().await, ConfirmationOutcome::Refused);
        assert_eq!(second.outcome().await, ConfirmationOutcome::Refused);
        // Unlike a shutdown, the next request is asked and can be approved.
        let next = pending.register(request(30), true).unwrap();
        assert!(pending.answer(next.id(), AnswerSource::Toast, Answer::Approve));
        assert_eq!(next.outcome().await, ConfirmationOutcome::Approved);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn anything_unexpected_refuses() {
        let pending = PendingConfirmations::new();

        // The toast without « Autoriser une fois » cannot approve.
        let waiting = pending.register(request(30), false).unwrap();
        let id = waiting.id();
        assert_eq!(
            pending.toast_activated(id, Some(&format!("confirm:approve:{id}"))),
            ToastFollowUp::Nothing
        );
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused);

        // An activation naming another request, or written by someone else, or without arguments.
        for arguments in [Some("confirm:approve:424242"), Some("autoriser"), None] {
            let waiting = pending.register(request(30), true).unwrap();
            assert_eq!(pending.toast_activated(waiting.id(), arguments), ToastFollowUp::Nothing);
            assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused, "{arguments:?}");
        }

        // A dismissed toast, whatever the reason given by Windows, except a hide by the application.
        for reason in [
            DismissReason::UserCanceled,
            DismissReason::TimedOut,
            DismissReason::Unknown,
        ] {
            let waiting = pending.register(request(30), true).unwrap();
            pending.toast_dismissed(waiting.id(), reason);
            assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused, "{reason:?}");
        }
        let waiting = pending.register(request(30), true).unwrap();
        pending.toast_dismissed(waiting.id(), DismissReason::ApplicationHidden);
        assert!(pending.is_waiting(waiting.id()));

        // The detail window closed, or an answer of its page that is not exactly an approval.
        pending.detail_closed(waiting.id());
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused);
        for answer in [
            None,
            Some(json!(null)),
            Some(json!(true)),
            Some(json!("Approve")),
            Some(json!({})),
        ] {
            let waiting = pending.register(request(30), true).unwrap();
            assert!(pending.detail_answer(waiting.id(), answer.as_ref()));
            assert_eq!(waiting.outcome().await, ConfirmationOutcome::Refused, "{answer:?}");
        }

        // Nothing could show it.
        let waiting = pending.register(request(30), true).unwrap();
        pending.unavailable(waiting.id());
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Unavailable);
    }

    // Exigence : EX-013-01
    #[tokio::test(start_paused = true)]
    async fn detail_is_opened_only_for_a_waiting_request() {
        let pending = PendingConfirmations::new();
        let waiting = pending.register(request(30), false).unwrap();
        let id = waiting.id();
        assert_eq!(
            pending.toast_activated(id, Some(&format!("confirm:detail:{id}"))),
            ToastFollowUp::OpenDetail
        );
        assert!(pending.is_waiting(id), "opening the detail settles nothing");
        assert!(pending.detail_answer(id, Some(&json!("approve"))));
        assert_eq!(
            pending.toast_activated(id, Some(&format!("confirm:detail:{id}"))),
            ToastFollowUp::Nothing
        );
        assert_eq!(waiting.outcome().await, ConfirmationOutcome::Approved);
    }
}
