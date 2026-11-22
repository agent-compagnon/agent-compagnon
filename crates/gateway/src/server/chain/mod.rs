//! The chain every `tools/call` goes through (spec 007), in this order:
//!
//! 1. resolution of the exposed name (only exposed tools resolve);
//! 2. suspension of the gateway by the user;
//! 3. policy (`Deny` stops here);
//! 4. scan of the arguments for known secrets;
//! 5. rate limit of the instance;
//! 6. confirmation, if the policy asks for one;
//! 7. session, waited for within the call budget;
//! 8. upstream call, replayed once after a refused credential;
//!
//! then, whatever happened above, the output filter and the trace. A stage that halts the call stops every stage
//! after it: a call refused by the policy is never scanned, confirmed, rate limited nor connected.
//!
//! The answer is always built by `OutputGate`, refusals included: nothing leaves without the filter (INV-3).

mod confirm;
mod rate;
mod stages;
mod ticket;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use mcp_proxy_core::config::{Config, RateLimit, SessionId, ToolName};
use mcp_proxy_core::confirm::ConfirmationBroker;
use mcp_proxy_core::filter::SECRET_MARKER;
use mcp_proxy_core::policy::{Resolution, ToolAnnotations};
use mcp_proxy_core::session::SessionGate;
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_core::trace::{CallRecord, CallStatus, PolicyRecord, TraceSink};
use rmcp::model::{CallToolResult, InputRequiredResult, InputResponses, JsonObject};
use rmcp::service::{Peer, RoleServer};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{RwLock, watch};
use tokio_util::sync::CancellationToken;

use self::rate::RateLimiter;
use self::ticket::Tickets;
use super::output::{Filtered, OutputGate};
use super::pipeline::{CallPipeline, CallRequest};
use super::{ClientIdentity, ConnectionId};
use crate::registry::{InstanceId, Registry};
use crate::upstream::{ProgressSink, Upstreams};

/// Names of the stages, in the order they run. The output filter and the trace follow, always.
pub const STAGES: [&str; 8] = [
    "resolution",
    "suspension",
    "policy",
    "arguments",
    "rate_limit",
    "confirmation",
    "session",
    "upstream",
];

/// The tool a call resolved to, with what the next stages need from the configuration.
#[derive(Debug, Clone)]
pub struct ResolvedTool {
    pub instance: InstanceId,
    pub tool: ToolName,
    /// Annotations given by the server, for a declared tool without a category.
    pub annotations: Option<ToolAnnotations>,
    pub env_label: String,
    pub mcp_label: String,
    pub session: Option<SessionId>,
    pub session_label: String,
    pub rate_limit: Option<RateLimit>,
}

impl ResolvedTool {
    /// `DEV/kibana-logs/delete_index`.
    pub fn short_path(&self) -> String {
        format!("{}/{}/{}", self.env_label, self.instance.mcp, self.tool)
    }

    /// `DEV / kibana-logs / delete_index`.
    pub fn path(&self) -> String {
        format!("{} / {} / {}", self.env_label, self.instance.mcp, self.tool)
    }
}

/// One call on its way through the chain. `Debug` leaves out the arguments and what the client brought back: they
/// are not filtered yet.
pub struct CallContext {
    pub call_id: u64,
    pub started_at: SystemTime,
    pub started: Instant,
    /// Start + `call_budget_secs`: the longest the call waits for the user, confirmation and login together.
    pub deadline: Instant,
    pub client: ClientIdentity,
    pub connection: ConnectionId,
    pub exposed_name: String,
    pub args: Option<JsonObject>,
    pub peer: Option<Peer<RoleServer>>,
    pub input_responses: Option<InputResponses>,
    pub request_state: Option<String>,
    pub progress: Option<ProgressSink>,
    pub cancel: CancellationToken,
    /// Set by the resolution.
    pub tool: Option<ResolvedTool>,
    /// Set by the policy.
    pub policy: Option<Resolution>,
    /// French explanation of `policy`.
    pub explanation: Option<String>,
    /// Set by the upstream call.
    pub result: Option<CallToolResult>,
}

impl std::fmt::Debug for CallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallContext")
            .field("call_id", &self.call_id)
            .field("connection", &self.connection)
            .field("tool", &self.tool)
            .field("decision", &self.policy.as_ref().map(|p| p.decision))
            .finish_non_exhaustive()
    }
}

impl CallContext {
    fn new(call_id: u64, request: CallRequest, budget: Duration) -> Self {
        let started = Instant::now();
        Self {
            call_id,
            started_at: SystemTime::now(),
            started,
            deadline: started + budget,
            client: request.client,
            connection: request.connection,
            exposed_name: request.exposed_name,
            args: request.arguments,
            peer: request.peer,
            input_responses: request.input_responses,
            request_state: request.request_state,
            progress: request.progress,
            cancel: request.cancel,
            tool: None,
            policy: None,
            explanation: None,
            result: None,
        }
    }

    /// The resolved tool; a stage that runs before the resolution fails closed.
    pub fn resolved(&self) -> Result<&ResolvedTool, Halt> {
        self.tool.as_ref().ok_or_else(|| Refusal::internal().into())
    }
}

/// Why a call stops before the upstream answer. `message` is French, follows `MCP Proxy : `, and goes through the
/// output filter like everything else; `Debug` leaves it out for that reason.
#[derive(Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: CallStatus,
    pub message: String,
}

impl std::fmt::Debug for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Refusal")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Refusal {
    pub fn new(status: CallStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal() -> Self {
        Self::new(
            CallStatus::Error,
            "erreur interne de la passerelle, appel non transmis.",
        )
    }
}

#[derive(Debug, Clone)]
pub enum Halt {
    Refused(Refusal),
    /// 2026-07-28: the client is asked to confirm in the chat and to call again with the answer.
    InputRequired(InputRequiredResult),
}

impl From<Refusal> for Halt {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// One stage of the chain.
#[async_trait]
pub trait Middleware: Send + Sync {
    fn name(&self) -> &'static str;

    /// `Err` stops the call: no later stage runs.
    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt>;
}

/// What the chain is built from.
pub struct CallChainDeps {
    pub config: Arc<Config>,
    pub registry: Arc<RwLock<Registry>>,
    /// Current user settings: suspension, profile, hardening.
    pub settings: watch::Receiver<UserSettings>,
    pub upstreams: Upstreams,
    pub gate: Arc<dyn SessionGate>,
    /// Confirmation outside the chat (Windows notification).
    pub broker: Arc<dyn ConfirmationBroker>,
    pub output: OutputGate,
    pub trace: Arc<dyn TraceSink>,
}

#[derive(Debug, Error)]
pub enum ChainError {
    #[error("générateur aléatoire indisponible : la clé des confirmations ne peut pas être créée")]
    Random,
}

/// What the stages share.
struct Shared {
    config: Arc<Config>,
    registry: Arc<RwLock<Registry>>,
    settings: watch::Receiver<UserSettings>,
    upstreams: Upstreams,
    gate: Arc<dyn SessionGate>,
    broker: Arc<dyn ConfirmationBroker>,
    output: OutputGate,
    limiter: RateLimiter,
    tickets: Tickets,
}

/// Built once, shared by every connection.
pub struct CallChain {
    stages: Vec<Box<dyn Middleware>>,
    output: OutputGate,
    trace: Arc<dyn TraceSink>,
    budget: Duration,
    next_id: AtomicU64,
}

impl std::fmt::Debug for CallChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallChain")
            .field("stages", &self.stage_names())
            .field("budget", &self.budget)
            .finish_non_exhaustive()
    }
}

impl CallChain {
    /// The stages in the order of [`STAGES`]. Fails only if no key can be drawn for the confirmation states.
    pub fn new(deps: CallChainDeps) -> Result<Self, ChainError> {
        let gateway = &deps.config.gateway;
        let budget = Duration::from_secs(gateway.call_budget_secs);
        let tickets =
            Tickets::new(Duration::from_secs(gateway.confirmation_timeout_secs)).map_err(|_| ChainError::Random)?;
        let shared = Arc::new(Shared {
            config: deps.config,
            registry: deps.registry,
            settings: deps.settings,
            upstreams: deps.upstreams,
            gate: deps.gate,
            broker: deps.broker,
            output: deps.output.clone(),
            limiter: RateLimiter::default(),
            tickets,
        });
        let stages: Vec<Box<dyn Middleware>> = vec![
            Box::new(stages::Resolution(Arc::clone(&shared))),
            Box::new(stages::Suspension(Arc::clone(&shared))),
            Box::new(stages::Policy(Arc::clone(&shared))),
            Box::new(stages::Arguments(Arc::clone(&shared))),
            Box::new(stages::RateLimit(Arc::clone(&shared))),
            Box::new(confirm::Confirmation(Arc::clone(&shared))),
            Box::new(stages::Session(Arc::clone(&shared))),
            Box::new(stages::UpstreamCallStage(shared)),
        ];
        Ok(Self {
            stages,
            output: deps.output,
            trace: deps.trace,
            budget,
            next_id: AtomicU64::new(1),
        })
    }

    pub fn stage_names(&self) -> Vec<&'static str> {
        self.stages.iter().map(|s| s.name()).collect()
    }

    /// Output filter: the only way to an answer.
    fn answer(&self, context: &mut CallContext, outcome: Result<(), Halt>) -> (Filtered, CallStatus) {
        let (filtered, status) = match outcome {
            Ok(()) => match context.result.take() {
                Some(result) => {
                    let filtered = self.output.result(&result);
                    let status = if filtered.is_unavailable() || result.is_error == Some(true) {
                        CallStatus::Error
                    } else {
                        CallStatus::Ok
                    };
                    (filtered, status)
                }
                None => self.refusal(&Refusal::internal()),
            },
            Err(Halt::Refused(refusal)) => self.refusal(&refusal),
            Err(Halt::InputRequired(result)) => {
                let filtered = self.output.input_required(&result);
                let status = if filtered.is_unavailable() {
                    CallStatus::Error
                } else {
                    CallStatus::InputRequired
                };
                (filtered, status)
            }
        };
        let leaked = filtered.report().exact_hits > 0;
        let status = match status {
            CallStatus::Ok | CallStatus::Error if leaked => CallStatus::LeakMasked,
            other => other,
        };
        (filtered, status)
    }

    fn refusal(&self, refusal: &Refusal) -> (Filtered, CallStatus) {
        let filtered = self.output.error(&format!("MCP Proxy : {}", refusal.message));
        (filtered, refusal.status)
    }

    /// Trace: every call, refusals included, with filtered content only.
    fn record(&self, context: &CallContext, filtered: &Filtered, status: CallStatus) {
        let marker = || SECRET_MARKER.to_owned();
        let arguments = Value::Object(context.args.clone().unwrap_or_default());
        let record = CallRecord {
            id: context.call_id,
            started_at: context.started_at,
            duration: context.started.elapsed(),
            client: self.output.text(&context.client.name).unwrap_or_else(marker),
            tool: self.output.text(&context.exposed_name).unwrap_or_else(marker),
            env: context.tool.as_ref().map(|t| t.instance.env.clone()),
            mcp: context.tool.as_ref().map(|t| t.instance.mcp.clone()),
            session: context.tool.as_ref().and_then(|t| t.session.clone()),
            policy: context.policy.as_ref().map(|p| PolicyRecord {
                decision: p.decision,
                explanation: context
                    .explanation
                    .as_deref()
                    .and_then(|e| self.output.text(e))
                    .unwrap_or_default(),
            }),
            status,
            request: self.output.value(&arguments).unwrap_or_else(|| Value::String(marker())),
            response: filtered.to_value(),
            filter_report: filtered.report().clone(),
        };
        self.trace.record(record);
    }
}

#[async_trait]
impl CallPipeline for CallChain {
    async fn call(&self, request: CallRequest) -> Filtered {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut context = CallContext::new(id, request, self.budget);
        let mut outcome = Ok(());
        for stage in &self.stages {
            outcome = stage.handle(&mut context).await;
            if outcome.is_err() {
                break;
            }
        }
        let (filtered, status) = self.answer(&mut context, outcome);
        self.record(&context, &filtered, status);
        filtered
    }
}
