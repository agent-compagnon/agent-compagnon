//! The `request_state` of a confirmation asked in the chat (2026-07-28): signed, not encrypted, and brought back by
//! the client. It binds the connection, the exposed name, the SHA-256 of the canonical arguments, a single-use nonce
//! and an expiry. Everything is checked again when it comes back.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rmcp::model::{JsonObject, RequestStateCodec, RequestStateError, SealOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::ConnectionId;
use crate::registry::canonical_json;

const ASSOCIATED_DATA: &[u8] = b"mcp-proxy.confirmation.v1";
const VERSION: u8 = 1;
/// Pending confirmations kept at most; the oldest is forgotten beyond (its answer will be refused).
const MAX_PENDING: usize = 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ticket {
    v: u8,
    connection: String,
    tool: String,
    args: String,
    nonce: String,
    /// Unix time, milliseconds.
    exp: u64,
}

/// Why a state brought back by a client is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TicketError {
    /// Signature, format, connection, tool, arguments or nonce: the state does not belong to this call.
    Invalid,
    Expired,
}

/// Could not issue a state: the caller falls back to the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IssueError;

pub(crate) struct Tickets {
    codec: RequestStateCodec,
    ttl: Duration,
    pending: Mutex<HashMap<String, Instant>>,
}

impl std::fmt::Debug for Tickets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tickets")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unix_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// SHA-256 of the canonical JSON of the arguments; no arguments and `{}` are the same call.
pub(crate) fn arguments_digest(arguments: Option<&JsonObject>) -> String {
    let value = Value::Object(arguments.cloned().unwrap_or_default());
    hex(&Sha256::digest(canonical_json(&value).as_bytes()))
}

impl Tickets {
    /// A random signing key for this process.
    pub(crate) fn new(ttl: Duration) -> Result<Self, getrandom::Error> {
        // Moved into the codec, which wipes it on drop.
        let mut key = vec![0u8; RequestStateCodec::MIN_KEY_LENGTH];
        getrandom::fill(&mut key)?;
        let codec = RequestStateCodec::new_unchecked(key);
        Ok(Self {
            codec,
            ttl,
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn issue(
        &self,
        connection: ConnectionId,
        tool: &str,
        arguments: Option<&JsonObject>,
    ) -> Result<String, IssueError> {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| IssueError)?;
        let nonce = hex(&nonce);
        let ticket = Ticket {
            v: VERSION,
            connection: connection.to_string(),
            tool: tool.to_owned(),
            args: arguments_digest(arguments),
            nonce: nonce.clone(),
            exp: unix_ms(SystemTime::now() + self.ttl),
        };
        let options = SealOptions::new().associated_data(ASSOCIATED_DATA).ttl(self.ttl);
        let sealed = self.codec.seal_json_with(&ticket, &options).map_err(|_| IssueError)?;
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, expires| *expires > now);
        if pending.len() >= MAX_PENDING
            && let Some(oldest) = pending.iter().min_by_key(|(_, e)| **e).map(|(n, _)| n.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(nonce, now + self.ttl);
        Ok(sealed)
    }

    /// Checks a state brought back with a call. A state with a valid signature burns its nonce whatever else is
    /// wrong with it: it can never be tried twice.
    pub(crate) fn redeem(
        &self,
        state: &str,
        connection: ConnectionId,
        tool: &str,
        arguments: Option<&JsonObject>,
    ) -> Result<(), TicketError> {
        let ticket: Ticket = match self.codec.open_json_with(state, ASSOCIATED_DATA) {
            Ok(ticket) => ticket,
            Err(RequestStateError::Expired) => return Err(TicketError::Expired),
            Err(_) => return Err(TicketError::Invalid),
        };
        let now = Instant::now();
        let nonce_was_pending = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            pending.remove(&ticket.nonce).is_some_and(|expires| expires > now)
        };
        if ticket.v != VERSION || !nonce_was_pending {
            return Err(TicketError::Invalid);
        }
        if ticket.exp <= unix_ms(SystemTime::now()) {
            return Err(TicketError::Expired);
        }
        if ticket.connection != connection.to_string()
            || ticket.tool != tool
            || ticket.args != arguments_digest(arguments)
        {
            return Err(TicketError::Invalid);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn args(value: Value) -> JsonObject {
        value.as_object().cloned().unwrap()
    }

    // Exigence : EX-007-09
    #[test]
    fn a_state_is_bound_to_its_call_and_used_once() {
        let tickets = Tickets::new(Duration::from_secs(30)).unwrap();
        let connection = ConnectionId::next();
        let a = args(json!({ "index": "app-1", "force": true }));
        let reordered = args(json!({ "force": true, "index": "app-1" }));

        let state = tickets.issue(connection, "dev_kibana_delete_index", Some(&a)).unwrap();
        tickets
            .redeem(&state, connection, "dev_kibana_delete_index", Some(&reordered))
            .unwrap();
        assert_eq!(
            tickets.redeem(&state, connection, "dev_kibana_delete_index", Some(&a)),
            Err(TicketError::Invalid),
            "replayed"
        );

        let other_args = args(json!({ "index": "app-2", "force": true }));
        for (redeemed_on, tool, arguments) in [
            (ConnectionId::next(), "dev_kibana_delete_index", Some(&a)),
            (connection, "dev_kibana_search_logs", Some(&a)),
            (connection, "dev_kibana_delete_index", Some(&other_args)),
            (connection, "dev_kibana_delete_index", None),
        ] {
            let state = tickets.issue(connection, "dev_kibana_delete_index", Some(&a)).unwrap();
            assert_eq!(
                tickets.redeem(&state, redeemed_on, tool, arguments),
                Err(TicketError::Invalid),
                "{tool} {arguments:?}"
            );
            // The failed attempt burnt the nonce.
            assert_eq!(
                tickets.redeem(&state, connection, "dev_kibana_delete_index", Some(&a)),
                Err(TicketError::Invalid)
            );
        }

        let state = tickets.issue(connection, "dev_kibana_delete_index", Some(&a)).unwrap();
        let mut tampered = state.clone().into_bytes();
        // Inside the signed body, after `rs1.`.
        tampered[10] = if tampered[10] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert_eq!(
            tickets.redeem(&tampered, connection, "dev_kibana_delete_index", Some(&a)),
            Err(TicketError::Invalid)
        );
        assert_eq!(
            tickets.redeem("rs1.e30.e30", connection, "dev_kibana_delete_index", Some(&a)),
            Err(TicketError::Invalid)
        );
        // Another process (another key) cannot open it.
        let stranger = Tickets::new(Duration::from_secs(30)).unwrap();
        assert_eq!(
            stranger.redeem(&state, connection, "dev_kibana_delete_index", Some(&a)),
            Err(TicketError::Invalid)
        );
        tickets
            .redeem(&state, connection, "dev_kibana_delete_index", Some(&a))
            .unwrap();
    }

    // Exigence : EX-007-09
    #[test]
    fn an_expired_state_is_refused() {
        let tickets = Tickets::new(Duration::from_millis(50)).unwrap();
        let connection = ConnectionId::next();
        let state = tickets.issue(connection, "dev_kibana_delete_index", None).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(
            tickets.redeem(&state, connection, "dev_kibana_delete_index", None),
            Err(TicketError::Expired)
        );
    }
}
