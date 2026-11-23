//! What the resilient bridge reads of the MCP messages it relays (spec 026): one JSON-RPC message per line, its id,
//! its method, and nothing of its parameters or result, except the id of a cancellation and the protocol version and
//! capabilities of the answer to `initialize`. No content is kept, logged or rendered.

use std::io;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Longest message relayed, newline excluded: four times what rmcp reads from a stream. A longer line is dropped,
/// as rmcp drops it.
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

/// The error answered for a request that the application cannot answer any more.
pub const RESTARTING: &str = "MCP Proxy redémarre, réessayez dans un instant.";
/// JSON-RPC « server error », the range left to implementations.
const RESTARTING_CODE: i64 = -32000;

pub(crate) const INITIALIZE: &str = "initialize";
pub(crate) const INITIALIZED: &str = "notifications/initialized";
const CANCELLED: &str = "notifications/cancelled";

/// What one read gives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// A whole message, its newline included.
    Message(Vec<u8>),
    /// A line longer than [`MAX_MESSAGE`], dropped up to its newline.
    Oversized,
    /// End of the stream. An unterminated last line is dropped: half a message is never relayed.
    End,
}

/// Splits a stream into lines. Cancel-safe: a read cancelled by a `select!` loses no byte, the partial line stays
/// here for the next call.
pub(crate) struct LineReader<R> {
    inner: R,
    buffer: Vec<u8>,
    /// Bytes of `buffer` already searched for a newline.
    scanned: usize,
    discarding: bool,
    chunk: Box<[u8]>,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            scanned: 0,
            discarding: false,
            chunk: vec![0u8; CHUNK].into_boxed_slice(),
        }
    }

    pub(crate) async fn next(&mut self) -> io::Result<Line> {
        loop {
            if let Some(found) = self.buffer[self.scanned..].iter().position(|b| *b == b'\n') {
                let end = self.scanned + found + 1;
                let rest = self.buffer.split_off(end);
                let line = std::mem::replace(&mut self.buffer, rest);
                self.scanned = 0;
                if std::mem::take(&mut self.discarding) {
                    return Ok(Line::Oversized);
                }
                return Ok(Line::Message(line));
            }
            self.scanned = self.buffer.len();
            if self.buffer.len() > MAX_MESSAGE {
                self.discarding = true;
                self.buffer.clear();
                self.scanned = 0;
            }
            let read = self.inner.read(&mut self.chunk).await?;
            if read == 0 {
                self.buffer.clear();
                self.scanned = 0;
                self.discarding = false;
                return Ok(Line::End);
            }
            self.buffer.extend_from_slice(&self.chunk[..read]);
        }
    }
}

/// A JSON-RPC id: a number or a string, compared by its JSON text (`1` and `"1"` differ).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Id(String);

impl Id {
    fn of(value: &Value) -> Option<Self> {
        matches!(value, Value::Number(_) | Value::String(_)).then(|| Self(value.to_string()))
    }

    fn value(&self) -> Value {
        serde_json::from_str(&self.0).unwrap_or(Value::Null)
    }
}

/// What a message is, as far as the bridge cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    Request {
        id: Id,
        method: String,
    },
    Notification {
        method: String,
        cancelled: Option<Id>,
    },
    Response {
        id: Id,
    },
    /// Not JSON-RPC as the bridge knows it: relayed as is, not followed.
    Other,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "present")]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
}

#[derive(Deserialize)]
struct Cancellation {
    params: CancellationParams,
}

#[derive(Deserialize)]
struct CancellationParams {
    #[serde(rename = "requestId")]
    request_id: Value,
}

/// `Some` as soon as the field is there, `null` included.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// The kinds of the messages of a line: one, or one per element of a batch.
pub(crate) fn inspect(line: &[u8]) -> Vec<Kind> {
    let text = line.trim_ascii_start();
    if text.first() == Some(&b'[') {
        return match serde_json::from_slice::<Vec<BatchElement>>(text) {
            Ok(elements) => elements.into_iter().map(|e| e.0).collect(),
            Err(_) => vec![Kind::Other],
        };
    }
    vec![serde_json::from_slice::<Envelope>(text).map_or(Kind::Other, |e| kind(&e, text))]
}

/// One element of a batch, read like a single message.
struct BatchElement(Kind);

impl<'de> Deserialize<'de> for BatchElement {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let kind = serde_json::from_value::<Envelope>(value.clone()).map_or(Kind::Other, |envelope| {
            let text = serde_json::to_vec(&value).unwrap_or_default();
            kind(&envelope, &text)
        });
        Ok(Self(kind))
    }
}

fn kind(envelope: &Envelope, text: &[u8]) -> Kind {
    match (&envelope.method, &envelope.id) {
        (Some(method), Some(id)) => Id::of(id).map_or(Kind::Other, |id| Kind::Request {
            id,
            method: method.clone(),
        }),
        (Some(method), None) => {
            let cancelled = (method == CANCELLED)
                .then(|| serde_json::from_slice::<Cancellation>(text).ok())
                .flatten()
                .and_then(|c| Id::of(&c.params.request_id));
            Kind::Notification {
                method: method.clone(),
                cancelled,
            }
        }
        (None, Some(id)) => Id::of(id).map_or(Kind::Other, |id| Kind::Response { id }),
        (None, None) => Kind::Other,
    }
}

/// What the client learnt from the answer to its `initialize`: a new instance must answer the same.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Handshake {
    protocol_version: Value,
    capabilities: Value,
}

impl Handshake {
    /// `None` for an error answer, or an answer without a protocol version.
    pub(crate) fn of_answer(line: &[u8]) -> Option<Self> {
        let answer: Value = serde_json::from_slice(line).ok()?;
        let result = answer.get("result")?;
        Some(Self {
            protocol_version: result.get("protocolVersion").filter(|v| v.is_string())?.clone(),
            capabilities: result.get("capabilities").cloned().unwrap_or(Value::Null),
        })
    }

    /// The server announced that it signals changes of its tool list.
    pub(crate) fn tools_list_changed(&self) -> bool {
        self.capabilities.pointer("/tools/listChanged") == Some(&Value::Bool(true))
    }
}

fn line_of(value: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(value).unwrap_or_default();
    line.push(b'\n');
    line
}

/// The error the bridge answers itself to a request of the client.
pub(crate) fn restarting(id: &Id) -> Vec<u8> {
    line_of(&json!({
        "jsonrpc": "2.0",
        "id": id.value(),
        "error": { "code": RESTARTING_CODE, "message": RESTARTING },
    }))
}

/// Tells the client that a request of the previous instance will never need its answer.
pub(crate) fn cancelled(id: &Id) -> Vec<u8> {
    line_of(&json!({
        "jsonrpc": "2.0",
        "method": CANCELLED,
        "params": { "requestId": id.value(), "reason": "MCP Proxy a redémarré" },
    }))
}

/// The answer the bridge gives a new instance whose request reuses the id of a request of the previous instance
/// that the client may still answer: the client's answer could otherwise be taken for an answer to this one.
pub(crate) fn refused_to_server(id: &Id) -> Vec<u8> {
    line_of(&json!({
        "jsonrpc": "2.0",
        "id": id.value(),
        "error": {
            "code": RESTARTING_CODE,
            "message": "identifiant encore attendu par le client depuis l'instance précédente",
        },
    }))
}

pub(crate) fn tools_list_changed() -> Vec<u8> {
    line_of(&json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> Id {
        Id(text.to_owned())
    }

    // Exigence : EX-026-06
    #[test]
    fn kinds_of_messages() {
        assert_eq!(
            inspect(br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"x"}}"#),
            vec![Kind::Request {
                id: id("3"),
                method: "tools/call".to_owned()
            }]
        );
        assert_eq!(
            inspect(br#"{"jsonrpc":"2.0","id":"a","result":{}}"#),
            vec![Kind::Response { id: id("\"a\"") }]
        );
        assert_eq!(
            inspect(br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}"#),
            vec![Kind::Notification {
                method: CANCELLED.to_owned(),
                cancelled: Some(id("7"))
            }]
        );
        assert_eq!(
            inspect(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            vec![Kind::Notification {
                method: INITIALIZED.to_owned(),
                cancelled: None
            }]
        );
        // `1` and `"1"` are two ids.
        assert_ne!(Id::of(&json!(1)), Id::of(&json!("1")));
        for other in [
            &b"pas du json"[..],
            b"{}",
            br#"{"id":null,"error":{}}"#,
            br#"{"id":{},"method":"x"}"#,
        ] {
            assert_eq!(inspect(other), vec![Kind::Other], "{}", String::from_utf8_lossy(other));
        }
        let batch = inspect(br#"[{"jsonrpc":"2.0","id":1,"method":"a"},{"jsonrpc":"2.0","method":"b"},5]"#);
        assert_eq!(batch.len(), 3);
        assert!(matches!(&batch[0], Kind::Request { id: i, .. } if *i == id("1")));
        assert!(matches!(&batch[1], Kind::Notification { method, .. } if method == "b"));
        assert_eq!(batch[2], Kind::Other);
    }

    // Exigence : EX-026-02
    #[test]
    fn handshake_of_the_answer_to_initialize() {
        let answer = br#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true}},"serverInfo":{"name":"a","version":"1"}}}"#;
        let handshake = Handshake::of_answer(answer).unwrap();
        assert!(handshake.tools_list_changed());
        let other_server = br#"{"jsonrpc":"2.0","id":0,"result":{"capabilities":{"tools":{"listChanged":true}},"protocolVersion":"2025-06-18","serverInfo":{"name":"b","version":"2"}}}"#;
        assert_eq!(
            Handshake::of_answer(other_server),
            Some(handshake.clone()),
            "server info is not compared"
        );
        let older = br#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{"listChanged":true}}}}"#;
        assert_ne!(Handshake::of_answer(older), Some(handshake));
        assert_eq!(
            Handshake::of_answer(br#"{"jsonrpc":"2.0","id":0,"error":{"code":1,"message":"x"}}"#),
            None
        );
        let quiet = Handshake::of_answer(br#"{"id":0,"result":{"protocolVersion":"v","capabilities":{"tools":{}}}}"#);
        assert!(!quiet.unwrap().tools_list_changed());
    }

    // Exigence : EX-026-04
    #[test]
    fn answers_of_the_bridge_keep_the_id() {
        let error: Value = serde_json::from_slice(&restarting(&id("\"r-1\""))).unwrap();
        assert_eq!(error["id"], "r-1");
        assert_eq!(error["error"]["message"], RESTARTING);
        let error: Value = serde_json::from_slice(&restarting(&id("12"))).unwrap();
        assert_eq!(error["id"], 12);
        let cancel: Value = serde_json::from_slice(&cancelled(&id("4"))).unwrap();
        assert_eq!(cancel["params"]["requestId"], 4);
        for line in [
            restarting(&id("1")),
            cancelled(&id("1")),
            refused_to_server(&id("1")),
            tools_list_changed(),
        ] {
            assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
            assert!(line.ends_with(b"\n"));
        }
    }
}
