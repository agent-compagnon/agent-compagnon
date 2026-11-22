//! Contract between the call chain (spec 007) and the traces (spec 009): one [`CallRecord`] per `tools/call`,
//! refusals included, handed to a [`TraceSink`].
//!
//! Every text and value of a record has been through the output filter before the record is built: a sink never
//! sees raw content, even in memory.

use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::config::{Decision, EnvId, McpId, SessionId};
use crate::filter::FilterReport;

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallStatus {
    /// Answered by the server, nothing found by the blacklist.
    Ok,
    /// Answered by the server (result or error) with a known secret masked by the blacklist.
    LeakMasked,
    /// Error of the server, of the exchange, or of the gateway itself.
    Error,
    /// Refused by the policy, or tool unknown or not exposed.
    Denied,
    /// Refused by the user, or the confirmation could not be obtained.
    Refused,
    /// No answer to the confirmation in time.
    Timeout,
    /// Known secret in the arguments, or filter unavailable to check them.
    Blocked,
    RateLimited,
    /// The session could not be used: login pending, cancelled, impossible, or credential refused twice.
    AuthRequired,
    /// Gateway suspended by the user.
    Suspended,
    /// The client was asked to confirm in the chat (2026-07-28): the call goes on when it answers.
    InputRequired,
    /// Cancelled by the client.
    Cancelled,
}

impl CallStatus {
    /// Key used in the journal and the interface.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::LeakMasked => "leak_masked",
            Self::Error => "error",
            Self::Denied => "denied",
            Self::Refused => "refused",
            Self::Timeout => "timeout",
            Self::Blocked => "blocked",
            Self::RateLimited => "rate_limited",
            Self::AuthRequired => "auth_required",
            Self::Suspended => "suspended",
            Self::InputRequired => "input_required",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The policy decision of a call and its French explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRecord {
    pub decision: Decision,
    pub explanation: String,
}

/// One `tools/call`, as seen by the gateway. Filtered content only.
#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    pub id: u64,
    pub started_at: SystemTime,
    pub duration: Duration,
    /// `clientInfo.name`, filtered.
    pub client: String,
    /// Exposed name asked by the client, filtered.
    pub tool: String,
    /// Known once the exposed name resolved.
    pub env: Option<EnvId>,
    pub mcp: Option<McpId>,
    pub session: Option<SessionId>,
    /// Known once the policy was resolved.
    pub policy: Option<PolicyRecord>,
    pub status: CallStatus,
    /// Arguments, filtered.
    pub request: Value,
    /// What the client received, filtered.
    pub response: Value,
    /// What the filter masked in the response.
    pub filter_report: FilterReport,
}

/// Where call records go. Called once per call, on the call's task: must not block.
pub trait TraceSink: Send + Sync {
    fn record(&self, record: CallRecord);
}

/// Drops every record.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoTrace;

impl TraceSink for NoTrace {
    fn record(&self, _: CallRecord) {}
}
