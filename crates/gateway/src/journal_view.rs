//! What the Journal tab shows (spec 018): the entries of the journal of spec 009, serialized in camelCase for the
//! window (`get_journal`, `journal-entry`), and the feed that follows the journal for the application.
//!
//! The entries are already filtered by the journal; the runtime passes the serialized view through the output filter
//! once more before it leaves (defence in depth). The interface adds no filtering of its own: it shows what it gets.
//! The classification of an entry (problem, policy) is the core's (`JournalEntry::is_problem`, `is_policy`), sent
//! with the entry: the window does not recompute it.

use std::time::{SystemTime, UNIX_EPOCH};

use mcp_proxy_core::filter::{HitKind, OutputFilter};
use mcp_proxy_core::trace::{CallDetails, CallStatus, EntryKind, JournalEntry};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::view::{DecisionKind, PolicyView};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKindView {
    Call,
    ToolsChanged,
    Panic,
    Event,
}

impl From<EntryKind> for EntryKindView {
    fn from(kind: EntryKind) -> Self {
        match kind {
            EntryKind::Call => Self::Call,
            EntryKind::ToolsChanged => Self::ToolsChanged,
            EntryKind::Panic => Self::Panic,
            EntryKind::Event => Self::Event,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusView {
    Ok,
    LeakMasked,
    Error,
    Denied,
    Refused,
    Timeout,
    Blocked,
    RateLimited,
    AuthRequired,
    Suspended,
    InputRequired,
    Cancelled,
}

impl From<CallStatus> for StatusView {
    fn from(status: CallStatus) -> Self {
        match status {
            CallStatus::Ok => Self::Ok,
            CallStatus::LeakMasked => Self::LeakMasked,
            CallStatus::Error => Self::Error,
            CallStatus::Denied => Self::Denied,
            CallStatus::Refused => Self::Refused,
            CallStatus::Timeout => Self::Timeout,
            CallStatus::Blocked => Self::Blocked,
            CallStatus::RateLimited => Self::RateLimited,
            CallStatus::AuthRequired => Self::AuthRequired,
            CallStatus::Suspended => Self::Suspended,
            CallStatus::InputRequired => Self::InputRequired,
            CallStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskedKindView {
    KnownSecret,
    Jwt,
    AuthorizationHeader,
    CookieHeader,
    SensitiveField,
    PrivateKey,
    UrlCredentials,
    /// A kind the core added since (`HitKind` is non-exhaustive).
    Other,
}

impl From<HitKind> for MaskedKindView {
    fn from(kind: HitKind) -> Self {
        match kind {
            HitKind::KnownSecret => Self::KnownSecret,
            HitKind::Jwt => Self::Jwt,
            HitKind::AuthorizationHeader => Self::AuthorizationHeader,
            HitKind::CookieHeader => Self::CookieHeader,
            HitKind::SensitiveField => Self::SensitiveField,
            HitKind::PrivateKey => Self::PrivateKey,
            HitKind::UrlCredentials => Self::UrlCredentials,
            _ => Self::Other,
        }
    }
}

/// What the output filter masked in the response: counts and kinds, never a value nor where it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskedView {
    pub exact: u32,
    pub generic: u32,
    pub kinds: Vec<MaskedKindView>,
}

/// The unfolded part of a row: only for a call whose content the core kept (`keeps_content`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailsView {
    /// Identifier of the call, as in the `mcp.call` span and the log file.
    pub call_id: u64,
    pub client: String,
    pub session: Option<String>,
    pub policy: Option<PolicyView>,
    /// Arguments, filtered.
    pub request: Value,
    /// What the client received, filtered.
    pub response: Value,
    pub masked: MaskedView,
}

impl From<&CallDetails> for DetailsView {
    fn from(details: &CallDetails) -> Self {
        Self {
            call_id: details.call_id,
            client: details.client.clone(),
            session: details.session.as_ref().map(ToString::to_string),
            policy: details.policy.as_ref().map(|p| PolicyView {
                decision: p.decision.into(),
                label: p.decision.label().to_owned(),
                explanation: p.explanation.clone(),
            }),
            request: details.request.clone(),
            response: details.response.clone(),
            masked: MaskedView {
                exact: details.filter_report.exact_hits,
                generic: details.filter_report.generic_hits,
                kinds: details.filter_report.kinds.iter().copied().map(Into::into).collect(),
            },
        }
    }
}

/// One row of the Journal tab.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    /// Order of the journal, from 1; every entry has its own, sent or kept.
    pub seq: u64,
    /// Milliseconds since the Unix epoch: start of a call, time of the other entries.
    pub at: u64,
    pub kind: EntryKindView,
    pub status: Option<StatusView>,
    /// French label of the status (`CallStatus::label`).
    pub status_label: Option<String>,
    pub slow: bool,
    pub client: Option<String>,
    /// Identifier of the environment, of the MCP; the window shows their labels from the snapshot.
    pub environment: Option<String>,
    pub mcp: Option<String>,
    pub tool: Option<String>,
    pub duration_ms: Option<u64>,
    pub decision: Option<DecisionKind>,
    pub summary: String,
    /// Filter `Problèmes` of the core.
    pub problem: bool,
    /// Filter `Politiques` of the core.
    pub policy: bool,
    pub details: Option<DetailsView>,
}

fn millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

impl From<&JournalEntry> for EntryView {
    fn from(entry: &JournalEntry) -> Self {
        Self {
            seq: entry.seq,
            at: millis(entry.at),
            kind: entry.kind.into(),
            status: entry.status.map(Into::into),
            status_label: entry.status.map(|s| s.label().to_owned()),
            slow: entry.slow,
            client: entry.client.clone(),
            environment: entry.env.as_ref().map(ToString::to_string),
            mcp: entry.mcp.as_ref().map(ToString::to_string),
            tool: entry.tool.clone(),
            duration_ms: entry.duration.and_then(|d| u64::try_from(d.as_millis()).ok()),
            decision: entry.decision.map(Into::into),
            summary: entry.summary.clone(),
            problem: entry.is_problem(),
            policy: entry.is_policy(),
            details: entry.details.as_ref().map(Into::into),
        }
    }
}

/// Answer of `get_journal`: the entries of the buffer, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalView {
    pub entries: Vec<EntryView>,
}

impl JournalView {
    pub fn of(entries: &[JournalEntry]) -> Self {
        Self {
            entries: entries.iter().map(Into::into).collect(),
        }
    }
}

/// `value` serialized, then through the output filter once more. `None` when either fails: nothing is sent then.
pub fn filtered_json(filter: &OutputFilter, value: &impl Serialize) -> Option<Value> {
    let mut json = serde_json::to_value(value).ok()?;
    filter.filter_json(&mut json).ok()?;
    Some(json)
}

/// What the application sends to the window next.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedItem {
    /// `journal-entry`: one new entry, serialized and filtered.
    Entry(Value),
    /// `journal-resync`: entries were lost on the way (the window fell behind, or one could not be filtered). The
    /// window asks `get_journal` again rather than showing a list with a hole in it.
    Resync,
}

/// Follows the journal for the window: every new entry, or a resynchronisation when some were lost.
pub struct JournalFeed {
    receiver: broadcast::Receiver<JournalEntry>,
    filter: std::sync::Arc<OutputFilter>,
}

impl std::fmt::Debug for JournalFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalFeed").finish_non_exhaustive()
    }
}

impl JournalFeed {
    pub fn new(receiver: broadcast::Receiver<JournalEntry>, filter: std::sync::Arc<OutputFilter>) -> Self {
        Self { receiver, filter }
    }

    /// The next item; `None` once the journal is gone. A receiver that fell behind (`Lagged`) is not an error: it
    /// gives one `Resync`, then the entries that follow.
    pub async fn next(&mut self) -> Option<FeedItem> {
        match self.receiver.recv().await {
            Ok(entry) => {
                Some(filtered_json(&self.filter, &EntryView::from(&entry)).map_or(FeedItem::Resync, FeedItem::Entry))
            }
            Err(RecvError::Lagged(lost)) => {
                tracing::info!(
                    lost,
                    "entrées du journal non transmises à la fenêtre : resynchronisation"
                );
                Some(FeedItem::Resync)
            }
            Err(RecvError::Closed) => None,
        }
    }
}
