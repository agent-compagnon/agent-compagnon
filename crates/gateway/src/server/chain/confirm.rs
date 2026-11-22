//! Confirmation stage. Only an explicit approval lets the call through (INV-5): a refusal, a cancellation, an error,
//! an expired delay or an unexpected answer all refuse it.
//!
//! Channel: elicitation when the policy asks for it **and** the client announces it **and** its name is listed in
//! `elicitation_clients`; otherwise the `ConfirmationBroker` (Windows notification). A `Notification` policy never
//! uses elicitation. A technical failure of the elicitation falls back to the broker within the time left.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mcp_proxy_core::config::{Channel, Decision};
use mcp_proxy_core::confirm::{ConfirmationOutcome, ConfirmationRequest};
use mcp_proxy_core::trace::CallStatus;
use rmcp::model::{
    ElicitRequest, ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationSchema, InputRequest,
    InputRequests, InputRequiredResult, ProtocolVersion,
};
use rmcp::service::{Peer, RoleServer, ServiceError};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::stages::{cancelled, whole_seconds};
use super::ticket::TicketError;
use super::{CallContext, Halt, Middleware, Refusal, Shared};
use crate::server::ClientIdentity;

/// Key of the one input request of a confirmation asked in the chat (2026-07-28).
pub(crate) const INPUT_KEY: &str = "confirmation";

/// How long the user is given: `confirmation_timeout_secs`, within what is left of the call budget.
#[derive(Debug, Clone, Copy)]
struct Window {
    deadline: Instant,
    shown: Duration,
}

impl Window {
    fn new(budget_deadline: Instant, timeout: Duration) -> Self {
        let now = Instant::now();
        let deadline = budget_deadline.min(now + timeout);
        Self {
            deadline,
            shown: deadline.saturating_duration_since(now),
        }
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Approved,
    Refused,
    TimedOut,
    Unavailable,
    Cancelled,
}

impl From<ConfirmationOutcome> for Verdict {
    fn from(outcome: ConfirmationOutcome) -> Self {
        match outcome {
            ConfirmationOutcome::Approved => Self::Approved,
            ConfirmationOutcome::Refused => Self::Refused,
            ConfirmationOutcome::TimedOut => Self::TimedOut,
            ConfirmationOutcome::Unavailable => Self::Unavailable,
        }
    }
}

fn is_modern(client: &ClientIdentity) -> bool {
    client
        .protocol_version
        .as_deref()
        .is_some_and(|v| v >= ProtocolVersion::V_2026_07_28.as_str())
}

fn refused_by_user() -> Halt {
    Refusal::new(CallStatus::Refused, "appel refusé par l'utilisateur.").into()
}

fn invalid_state() -> Halt {
    Refusal::new(CallStatus::Refused, "confirmation invalide ou expirée, appel refusé.").into()
}

fn timed_out(shown: Duration) -> Halt {
    Refusal::new(
        CallStatus::Timeout,
        format!(
            "confirmation non reçue dans le délai ({} s), appel refusé.",
            whole_seconds(shown)
        ),
    )
    .into()
}

/// What the user reads, in the chat as in the notification. Arguments and client name are already filtered.
fn message(request: &ConfirmationRequest) -> String {
    let arguments = serde_json::to_string_pretty(&request.arguments).unwrap_or_default();
    format!(
        "MCP Proxy : confirmez-vous l'appel {} demandé par « {} » ?\nArguments : {arguments}\nPolitique : {}",
        request.path, request.client, request.explanation
    )
}

fn elicitation_params(message: String) -> ElicitRequestParams {
    ElicitRequestParams::FormElicitationParams {
        meta: None,
        message,
        requested_schema: ElicitationSchema::new(BTreeMap::new()),
    }
}

/// Elicitation failed for a technical reason: the broker is asked instead.
struct Technical;

/// Legacy generation: a request to the client, answered within the window.
async fn ask_legacy(
    shared: &Shared,
    peer: &Peer<RoleServer>,
    request: &ConfirmationRequest,
    window: Window,
    cancel: &CancellationToken,
) -> Result<Verdict, Technical> {
    let Some(text) = shared.output.text(&message(request)) else {
        return Err(Technical);
    };
    let asked = peer.create_elicitation_with_timeout(elicitation_params(text), Some(window.remaining()));
    let answer = tokio::select! {
        answer = tokio::time::timeout_at(tokio::time::Instant::from_std(window.deadline), asked) => answer,
        () = cancel.cancelled() => return Ok(Verdict::Cancelled),
    };
    match answer {
        Err(_) | Ok(Err(ServiceError::Timeout { .. })) => Ok(Verdict::TimedOut),
        Ok(Ok(result)) if result.action == ElicitationAction::Accept => Ok(Verdict::Approved),
        Ok(Ok(_)) => Ok(Verdict::Refused),
        Ok(Err(ServiceError::TransportSend(_) | ServiceError::TransportClosed)) => Err(Technical),
        // The client answered with an error, or with something else than an elicitation result.
        Ok(Err(_)) => Ok(Verdict::Refused),
    }
}

async fn ask_broker(
    shared: &Shared,
    request: ConfirmationRequest,
    window: Window,
    cancel: &CancellationToken,
) -> Verdict {
    let asked = shared.broker.confirm(request);
    tokio::select! {
        outcome = tokio::time::timeout_at(tokio::time::Instant::from_std(window.deadline), asked) => {
            outcome.map_or(Verdict::TimedOut, Verdict::from)
        }
        () = cancel.cancelled() => Verdict::Cancelled,
    }
}

/// 2026-07-28, second round: the state must be the one issued for this very call, and the answer an acceptance.
fn redeem(shared: &Shared, context: &CallContext) -> Result<(), Halt> {
    let Some(state) = context.request_state.as_deref() else {
        return Err(invalid_state());
    };
    let timeout = Duration::from_secs(shared.config.gateway.confirmation_timeout_secs);
    match shared
        .tickets
        .redeem(state, context.connection, &context.exposed_name, context.args.as_ref())
    {
        Ok(()) => {}
        Err(TicketError::Expired) => return Err(timed_out(timeout)),
        Err(TicketError::Invalid) => return Err(invalid_state()),
    }
    let answer = context
        .input_responses
        .as_ref()
        .filter(|responses| responses.len() == 1)
        .and_then(|responses| responses.get(INPUT_KEY))
        .and_then(|value| serde_json::from_value::<ElicitResult>(value.clone()).ok());
    match answer.map(|a| a.action) {
        Some(ElicitationAction::Accept) => Ok(()),
        Some(ElicitationAction::Decline | ElicitationAction::Cancel) => Err(refused_by_user()),
        Some(_) | None => Err(invalid_state()),
    }
}

pub(super) struct Confirmation(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for Confirmation {
    fn name(&self) -> &'static str {
        "confirmation"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        let shared = &self.0;
        let decision = context.policy.as_ref().map(|p| p.decision);
        let brought_back = context.request_state.is_some() || context.input_responses.is_some();
        let channel = match decision {
            Some(Decision::Confirm(channel)) => channel,
            // A state nobody asked for is not trusted.
            Some(_) if brought_back => return Err(invalid_state()),
            Some(_) => return Ok(()),
            None => return Err(Refusal::internal().into()),
        };
        let gateway = &shared.config.gateway;
        let in_chat = channel == Channel::Elicitation
            && context.client.supports_elicitation
            && gateway.elicitation_clients.iter().any(|c| c == &context.client.name);
        let modern = in_chat && is_modern(&context.client);
        if brought_back {
            return if modern {
                redeem(shared, context)
            } else {
                Err(invalid_state())
            };
        }

        let tool = context.resolved()?.clone();
        let window = Window::new(context.deadline, Duration::from_secs(gateway.confirmation_timeout_secs));
        let arguments = Value::Object(context.args.clone().unwrap_or_default());
        let (Some(arguments), Some(client)) = (
            shared.output.value(&arguments),
            shared.output.text(&context.client.name),
        ) else {
            return Err(Refusal::new(
                CallStatus::Refused,
                "confirmation impossible, le filtre est indisponible ; appel refusé.",
            )
            .into());
        };
        let request = ConfirmationRequest {
            env: tool.instance.env.clone(),
            mcp: tool.instance.mcp.clone(),
            tool: tool.tool.clone(),
            path: tool.path(),
            client,
            arguments,
            explanation: context.explanation.clone().unwrap_or_default(),
            deadline: window.deadline,
        };

        let verdict = if modern {
            match shared
                .tickets
                .issue(context.connection, &context.exposed_name, context.args.as_ref())
            {
                Ok(state) => {
                    let elicitation = ElicitRequest::new(elicitation_params(message(&request)));
                    let requests =
                        InputRequests::from([(INPUT_KEY.to_owned(), InputRequest::Elicitation(elicitation))]);
                    return Err(Halt::InputRequired(InputRequiredResult::new(
                        Some(requests),
                        Some(state),
                    )));
                }
                Err(_) => ask_broker(shared, request, window, &context.cancel).await,
            }
        } else if let (true, Some(peer)) = (in_chat, &context.peer) {
            match ask_legacy(shared, peer, &request, window, &context.cancel).await {
                Ok(verdict) => verdict,
                Err(Technical) => ask_broker(shared, request, window, &context.cancel).await,
            }
        } else {
            ask_broker(shared, request, window, &context.cancel).await
        };

        match verdict {
            Verdict::Approved => Ok(()),
            Verdict::Refused => Err(refused_by_user()),
            Verdict::TimedOut => Err(timed_out(window.shown)),
            Verdict::Unavailable => Err(Refusal::new(
                CallStatus::Refused,
                "confirmation impossible, aucun moyen de la demander à l'utilisateur ; appel refusé.",
            )
            .into()),
            Verdict::Cancelled => Err(cancelled()),
        }
    }
}
