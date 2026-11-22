//! Contract between the call chain (spec 007) and the traces (spec 009): one [`CallRecord`] per `tools/call`,
//! refusals included, handed to a [`TraceSink`].
//!
//! Every text and value of a record has been through the output filter before the record is built: a sink never
//! sees raw content, even in memory.
//!
//! Spec 009 adds the sink of the application, [`Journal`]: circular buffer, subscriptions for the interface and
//! log file ([`open_log`]), with the `tracing` subscriber that writes every other event to the same file
//! ([`install_global`]) and the panic hook ([`install_panic_hook`]).

mod clock;
mod journal;
mod logfile;
mod panic;
mod ring;
mod subscriber;
pub(crate) mod time;

#[cfg(test)]
mod tests;

use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::config::{Channel, Decision, EnvId, McpId, SessionId};
use crate::filter::FilterReport;

pub use self::clock::{ManualWallClock, SystemWallClock, WallClock};
pub use self::journal::{
    CallDetails, EntryKind, Journal, JournalConfig, JournalEntry, JournalFilter, MAX_FILE_CONTENT_BYTES, RingUsage,
    StatusFilter, keeps_content, normalize,
};
pub use self::logfile::{
    ClockTime, DEFAULT_MAX_FILE_BYTES, DEFAULT_QUEUE_LINES, FILE_FULL_NOTICE, FileLog, LOG_FILE_PREFIX,
    LOG_FILE_SUFFIX, LineSender, LogFileOptions, LogGuard, RawLog, log_file_name, log_files, open_log,
};
pub use self::panic::install_panic_hook;
pub use self::subscriber::{LoggingError, default_targets, file_subscriber, install_global};
pub use self::time::rfc3339;

/// Key of a policy decision in the log file and the `mcp.call` span.
pub fn decision_key(decision: Decision) -> &'static str {
    match decision {
        Decision::Allow => "allow",
        Decision::Confirm(Channel::Elicitation) => "confirm_elicitation",
        Decision::Confirm(Channel::Notification) => "confirm_notification",
        Decision::Deny => "deny",
    }
}

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
