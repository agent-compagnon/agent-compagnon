//! The journal: the [`TraceSink`] of the application. Each call becomes a [`JournalEntry`], kept in the circular
//! buffer, sent to the subscribers (the interface) and written as one line of the log file.
//!
//! The content of a call (request, response, policy explanation) is kept, sent and written only for a call that did
//! not end normally or that was slow (see [`keeps_content`]). Every text and value is filtered again before it is
//! kept, whatever the caller did: the buffer never holds raw content.

use std::fmt::Write as _;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use tokio::sync::broadcast;

use super::clock::WallClock;
use super::logfile::FileLog;
use super::ring::Ring;
use super::{CallRecord, CallStatus, PolicyRecord, TraceSink, decision_key, time};
use crate::config::{Config, Decision, EnvId, McpId, SessionId, ToolName};
use crate::filter::{FilterReport, OutputFilter, SECRET_MARKER};

const EVENT_CAPACITY: usize = 256;
/// Longest client name, tool name or panic location kept in an entry, in bytes.
const MAX_LABEL_BYTES: usize = 256;
/// Longest panic message kept, in bytes.
const MAX_MESSAGE_BYTES: usize = 4096;
/// Longest request or response written to the log file, in bytes of JSON.
pub const MAX_FILE_CONTENT_BYTES: usize = 64 * 1024;
/// Tool names quoted in a tool list change entry, per kind.
const MAX_LISTED_TOOLS: usize = 20;
/// Accounted for each entry on top of its texts and content.
const ENTRY_OVERHEAD: usize = std::mem::size_of::<JournalEntry>();
const TRUNCATED: &str = "…[tronqué]";
const DETAILS_DROPPED: &str = " (détails trop volumineux pour le tampon, voir le fichier de log)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalConfig {
    /// `trace.ring_capacity`.
    pub ring_capacity: usize,
    /// `trace.ring_max_bytes`.
    pub ring_max_bytes: usize,
    /// `gateway.slow_call_threshold_secs`: a call that lasts at least this long is slow.
    pub slow_threshold: Duration,
}

impl JournalConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            ring_capacity: config.trace.ring_capacity,
            ring_max_bytes: config.trace.ring_max_bytes,
            slow_threshold: Duration::from_secs(config.gateway.slow_call_threshold_secs),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// One `tools/call`.
    Call,
    /// The tool list of an instance changed: new, modified or removed tools.
    ToolsChanged,
    /// A panic of the application.
    Panic,
}

/// What an entry shows when unfolded: only for a call that did not end normally, or a slow one. Filtered.
#[derive(Debug, Clone, PartialEq)]
pub struct CallDetails {
    /// Identifier of the call (its trace).
    pub call_id: u64,
    pub client: String,
    pub session: Option<SessionId>,
    pub policy: Option<PolicyRecord>,
    pub request: Value,
    pub response: Value,
    pub filter_report: FilterReport,
}

/// One line of the journal. Every text is filtered.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    /// Order of the journal, from 1.
    pub seq: u64,
    pub at: SystemTime,
    pub kind: EntryKind,
    pub status: Option<CallStatus>,
    /// The call lasted at least the slow call threshold.
    pub slow: bool,
    pub client: Option<String>,
    pub env: Option<EnvId>,
    pub mcp: Option<McpId>,
    /// Exposed name for a call; `None` for a tool list change.
    pub tool: Option<String>,
    pub duration: Option<Duration>,
    pub decision: Option<Decision>,
    /// One French line, always present.
    pub summary: String,
    pub details: Option<CallDetails>,
}

impl JournalEntry {
    /// Filter `Problèmes`: error, leak masked, blocked, timeout, rate limit, login required, cancelled, slow; panics.
    pub fn is_problem(&self) -> bool {
        self.kind == EntryKind::Panic
            || self.slow
            || self.status.is_some_and(|s| {
                matches!(
                    s,
                    CallStatus::Error
                        | CallStatus::LeakMasked
                        | CallStatus::Blocked
                        | CallStatus::Timeout
                        | CallStatus::RateLimited
                        | CallStatus::AuthRequired
                        | CallStatus::Cancelled
                )
            })
    }

    /// Filter `Politiques`: denied by the policy, refused by the user, no answer to a confirmation, gateway
    /// suspended, confirmation asked in the chat, and every call whose policy asked for a confirmation (accepted
    /// or not).
    pub fn is_policy(&self) -> bool {
        self.kind == EntryKind::Call
            && (matches!(self.decision, Some(Decision::Confirm(_)))
                || self.status.is_some_and(|s| {
                    matches!(
                        s,
                        CallStatus::Denied
                            | CallStatus::Refused
                            | CallStatus::Timeout
                            | CallStatus::Suspended
                            | CallStatus::InputRequired
                    )
                }))
    }
}

/// Whether the content of a call is kept in the buffer and written to the file: every call except those that ended
/// normally (`ok`, `input_required`, `suspended`), unless they were slow.
pub fn keeps_content(status: CallStatus, slow: bool) -> bool {
    slow || !matches!(
        status,
        CallStatus::Ok | CallStatus::InputRequired | CallStatus::Suspended
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    All,
    Problems,
    Policies,
}

/// Selection of [`Journal::snapshot`]. Every criterion set must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalFilter {
    /// Searched, ignoring case and accents, in the summary, client, environment, MCP, tool and status.
    pub text: Option<String>,
    pub env: Option<EnvId>,
    pub status: StatusFilter,
}

impl JournalFilter {
    pub fn matches(&self, entry: &JournalEntry) -> bool {
        let status = match self.status {
            StatusFilter::All => true,
            StatusFilter::Problems => entry.is_problem(),
            StatusFilter::Policies => entry.is_policy(),
        };
        let env = self.env.as_ref().is_none_or(|env| entry.env.as_ref() == Some(env));
        status && env && self.text.as_deref().is_none_or(|text| text_matches(entry, text))
    }
}

fn text_matches(entry: &JournalEntry, text: &str) -> bool {
    let needle = normalize(text.trim());
    if needle.is_empty() {
        return true;
    }
    let fields = [
        Some(entry.summary.as_str()),
        entry.client.as_deref(),
        entry.tool.as_deref(),
        entry.env.as_ref().map(EnvId::as_str),
        entry.mcp.as_ref().map(McpId::as_str),
        entry.status.map(CallStatus::key),
        entry.status.map(CallStatus::label),
    ];
    fields.into_iter().flatten().any(|f| normalize(f).contains(&needle))
}

/// Lower case, without the accents of French and the other Latin-1 letters.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => out.push('a'),
            'ç' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' => out.push('i'),
            'ñ' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            other => out.push(other),
        }
    }
    out
}

/// Size of the buffer, for tests and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingUsage {
    pub entries: usize,
    pub bytes: usize,
}

struct Inner {
    config: JournalConfig,
    filter: Arc<OutputFilter>,
    clock: Arc<dyn WallClock>,
    log: Option<FileLog>,
    ring: Mutex<Ring>,
    events: broadcast::Sender<JournalEntry>,
    next_seq: AtomicU64,
}

/// Shared by the call chain (as its [`TraceSink`]), the list watcher, the panic hook and the interface.
#[derive(Clone)]
pub struct Journal {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal")
            .field("config", &self.inner.config)
            .field("usage", &self.usage())
            .finish_non_exhaustive()
    }
}

impl Journal {
    /// `log`: the log file, if any; without it, the journal only lives in memory.
    pub fn new(
        config: JournalConfig,
        filter: Arc<OutputFilter>,
        clock: Arc<dyn WallClock>,
        log: Option<FileLog>,
    ) -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                config,
                filter,
                clock,
                log,
                ring: Mutex::new(Ring::new(config.ring_capacity, config.ring_max_bytes)),
                events,
                next_seq: AtomicU64::new(1),
            }),
        }
    }

    pub fn filter(&self) -> &Arc<OutputFilter> {
        &self.inner.filter
    }

    pub fn log(&self) -> Option<&FileLog> {
        self.inner.log.as_ref()
    }

    /// Entries of the buffer that match `filter`, oldest first.
    pub fn snapshot(&self, filter: &JournalFilter) -> Vec<JournalEntry> {
        self.ring().iter().filter(|e| filter.matches(e)).cloned().collect()
    }

    /// Every new entry, from now on. A receiver that falls behind loses the oldest ones (`Lagged`).
    pub fn subscribe(&self) -> broadcast::Receiver<JournalEntry> {
        self.inner.events.subscribe()
    }

    pub fn usage(&self) -> RingUsage {
        let ring = self.ring();
        RingUsage {
            entries: ring.len(),
            bytes: ring.bytes(),
        }
    }

    /// Waits until the lines written so far are in the file. Never called on a call's task.
    pub fn flush(&self) -> io::Result<()> {
        self.inner.log.as_ref().map_or(Ok(()), FileLog::flush)
    }

    /// One call of the chain.
    pub fn record_call(&self, record: CallRecord) {
        let slow = record.duration >= self.inner.config.slow_threshold;
        let content = keeps_content(record.status, slow);
        let client = self.label(&record.client);
        let tool = self.label(&record.tool);
        let decision = record.policy.as_ref().map(|p| p.decision);
        let duration_ms = record.duration.as_millis();
        let mut summary = format!("{client} → {tool} : {}", record.status.label());
        if slow {
            summary.push_str(" (lent)");
        }
        let _ = write!(summary, ", {duration_ms} ms");
        // Made of bounded parts: not cut again, so that the status stays visible.
        let summary = self.text(&summary);
        let details = content.then(|| CallDetails {
            call_id: record.id,
            client: client.clone(),
            session: record.session.clone(),
            policy: record.policy.as_ref().map(|p| PolicyRecord {
                decision: p.decision,
                explanation: self.text(&p.explanation),
            }),
            request: self.value(record.request),
            response: self.value(record.response),
            filter_report: record.filter_report.clone(),
        });

        let mut line = format!(
            "{} {} mcp.call call_id={} status={} slow={slow} duration_ms={duration_ms} client={} env={} mcp={} \
             tool={} session={} policy={}",
            time::rfc3339(self.inner.clock.now()),
            if content { "WARN " } else { "INFO " },
            record.id,
            record.status.key(),
            quoted(&client),
            record.env.as_ref().map_or("-", EnvId::as_str),
            record.mcp.as_ref().map_or("-", McpId::as_str),
            quoted(&tool),
            record.session.as_ref().map_or("-", SessionId::as_str),
            decision.map_or("-", decision_key),
        );
        if let Some(details) = &details {
            let explanation = details.policy.as_ref().map_or("", |p| p.explanation.as_str());
            let _ = write!(
                line,
                " explanation={} masked_exact={} masked_generic={} request={} response={}",
                quoted(explanation),
                details.filter_report.exact_hits,
                details.filter_report.generic_hits,
                bounded_json(&details.request),
                bounded_json(&details.response),
            );
        }
        line.push('\n');

        self.push(
            JournalEntry {
                seq: 0,
                at: record.started_at,
                kind: EntryKind::Call,
                status: Some(record.status),
                slow,
                client: Some(client),
                env: record.env,
                mcp: record.mcp,
                tool: Some(tool),
                duration: Some(record.duration),
                decision,
                summary,
                details,
            },
            &line,
        );
    }

    /// The tool list of an instance changed (spec 008): what is new, modified, removed, by original name.
    pub fn record_tool_changes(
        &self,
        env: &EnvId,
        mcp: &McpId,
        added: &[ToolName],
        modified: &[ToolName],
        removed: &[ToolName],
    ) {
        if added.is_empty() && modified.is_empty() && removed.is_empty() {
            return;
        }
        let mut parts = Vec::new();
        for (names, singular, plural) in [
            (added, "nouveau", "nouveaux"),
            (modified, "modifié", "modifiés"),
            (removed, "supprimé", "supprimés"),
        ] {
            if names.is_empty() {
                continue;
            }
            let mut listed: Vec<&str> = names.iter().take(MAX_LISTED_TOOLS).map(ToolName::as_str).collect();
            if names.len() > MAX_LISTED_TOOLS {
                listed.push("…");
            }
            let word = if names.len() == 1 { singular } else { plural };
            parts.push(format!("{} {word} ({})", names.len(), listed.join(", ")));
        }
        let summary = self.text(&format!(
            "{env} / {mcp} : liste d'outils changée, {}",
            parts.join(" ; ")
        ));
        let line = format!(
            "{} INFO  mcp.tools env={env} mcp={mcp} added={} modified={} removed={} summary={}\n",
            time::rfc3339(self.inner.clock.now()),
            added.len(),
            modified.len(),
            removed.len(),
            quoted(&summary),
        );
        self.push(
            JournalEntry {
                seq: 0,
                at: self.inner.clock.now(),
                kind: EntryKind::ToolsChanged,
                status: None,
                slow: false,
                client: None,
                env: Some(env.clone()),
                mcp: Some(mcp.clone()),
                tool: None,
                duration: None,
                decision: None,
                summary,
                details: None,
            },
            &line,
        );
    }

    /// A panic: where, and its message, filtered. Never waits for the buffer: the panic may have happened while it
    /// was held.
    pub fn record_panic(&self, location: &str, message: &str) {
        let location = self.label(location);
        let mut message = self.text(message);
        truncate(&mut message, MAX_MESSAGE_BYTES);
        let now = self.inner.clock.now();
        let line = format!(
            "{} ERROR panic location={} message={}\n",
            time::rfc3339(now),
            quoted(&location),
            quoted(&message),
        );
        if let Some(log) = &self.inner.log {
            log.write_record(&line);
        }
        let entry = JournalEntry {
            seq: self.inner.next_seq.fetch_add(1, Ordering::Relaxed),
            at: now,
            kind: EntryKind::Panic,
            status: None,
            slow: false,
            client: None,
            env: None,
            mcp: None,
            tool: None,
            duration: None,
            decision: None,
            summary: format!("panic à {location} : {message}"),
            details: None,
        };
        if let Ok(mut ring) = self.inner.ring.try_lock() {
            let weight = weight(&entry);
            ring.push(entry.clone(), weight);
        }
        let _ = self.inner.events.send(entry);
    }

    fn push(&self, mut entry: JournalEntry, line: &str) {
        if let Some(log) = &self.inner.log {
            log.write_record(line);
        }
        let mut weight = weight(&entry);
        let mut ring = self.ring();
        if weight > ring.max_bytes() && entry.details.is_some() {
            entry.details = None;
            entry.summary.push_str(DETAILS_DROPPED);
            weight = self::weight(&entry);
        }
        entry.seq = self.inner.next_seq.fetch_add(1, Ordering::Relaxed);
        ring.push(entry.clone(), weight);
        drop(ring);
        // No receiver is not an error: the interface may be closed.
        let _ = self.inner.events.send(entry);
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.inner.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Filtered text; the marker alone if the filter is unavailable.
    fn text(&self, text: &str) -> String {
        match self.inner.filter.filter_text(text) {
            Ok((filtered, _)) => filtered.into_owned(),
            Err(_) => SECRET_MARKER.to_owned(),
        }
    }

    /// Filtered short text, bounded.
    fn label(&self, text: &str) -> String {
        let mut text = self.text(text);
        truncate(&mut text, MAX_LABEL_BYTES);
        text
    }

    /// Filtered value; the filter replaces it by the marker if it is unavailable.
    fn value(&self, mut value: Value) -> Value {
        let _ = self.inner.filter.filter_json(&mut value);
        value
    }
}

impl TraceSink for Journal {
    fn record(&self, record: CallRecord) {
        self.record_call(record);
    }
}

/// Bytes accounted for an entry in the buffer.
fn weight(entry: &JournalEntry) -> usize {
    let texts =
        entry.summary.len() + entry.client.as_ref().map_or(0, String::len) + entry.tool.as_ref().map_or(0, String::len);
    let details = entry.details.as_ref().map_or(0, |d| {
        d.client.len()
            + d.session.as_ref().map_or(0, |s| s.as_str().len())
            + d.policy.as_ref().map_or(0, |p| p.explanation.len())
            + json_len(&d.request)
            + json_len(&d.response)
    });
    ENTRY_OVERHEAD + texts + details
}

struct Counter(usize);

impl io::Write for Counter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0 += data.len();
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_len(value: &Value) -> usize {
    let mut counter = Counter(0);
    let _ = serde_json::to_writer(&mut counter, value);
    counter.0
}

/// A JSON string: quotes, backslashes and line breaks escaped, so that a value cannot start a new log line.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

fn bounded_json(value: &Value) -> String {
    let mut json = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
    if json.len() > MAX_FILE_CONTENT_BYTES {
        truncate(&mut json, MAX_FILE_CONTENT_BYTES);
        json.push_str(TRUNCATED);
    }
    json
}

/// Cuts `text` to at most `max` bytes, on a character boundary.
fn truncate(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

impl CallStatus {
    /// French label, for the journal and the interface.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "réussi",
            Self::LeakMasked => "fuite masquée",
            Self::Error => "erreur",
            Self::Denied => "interdit",
            Self::Refused => "refusé",
            Self::Timeout => "délai dépassé",
            Self::Blocked => "bloqué",
            Self::RateLimited => "limite de débit",
            Self::AuthRequired => "connexion requise",
            Self::Suspended => "passerelle suspendue",
            Self::InputRequired => "confirmation demandée",
            Self::Cancelled => "annulé",
        }
    }
}
