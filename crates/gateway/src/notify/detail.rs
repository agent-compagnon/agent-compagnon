//! The detail window of a confirmation: a local page of the bundle (`confirm.html`) that shows one request, already
//! filtered, and answers through its only command. Its label names the request, so that a window can only answer
//! its own.

use std::time::Instant;

use mcp_proxy_auth::webview::js_data;
use mcp_proxy_core::confirm::ConfirmationRequest;
use serde::Serialize;
use serde_json::json;

use super::pending::ConfirmationId;
use super::toast::visible;

/// The command of the detail window: `{ answer: "approve" | "refuse" }`, nothing answered back.
pub const CONFIRM_COMMAND: &str = "answer_confirmation";
const LABEL_PREFIX: &str = "confirm-";

/// Label of the detail window of `id`: `confirm-17`.
pub fn detail_label(id: ConfirmationId) -> String {
    format!("{LABEL_PREFIX}{id}")
}

/// The request a detail window answers, from its label; `None` for any other window.
pub fn confirmation_of_label(label: &str) -> Option<ConfirmationId> {
    let digits = label.strip_prefix(LABEL_PREFIX)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    digits.parse().ok().map(ConfirmationId::from_raw)
}

/// What the detail window shows. Every text is filtered (by the chain, then by the broker) and written with its
/// hidden characters made visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailView {
    #[serde(skip)]
    pub id: ConfirmationId,
    pub path: String,
    pub client: String,
    pub explanation: String,
    /// Every argument, as indented JSON.
    pub arguments: String,
    /// Time left when the window is built, for the countdown; the deadline itself is kept by the broker.
    pub remaining_ms: u64,
}

impl DetailView {
    pub fn new(id: ConfirmationId, request: &ConfirmationRequest) -> Self {
        let remaining = request.deadline.saturating_duration_since(Instant::now());
        let arguments = serde_json::to_string_pretty(&request.arguments)
            .unwrap_or_default()
            .lines()
            .map(visible)
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            id,
            path: visible(&request.path),
            client: if request.client.is_empty() {
                "client LLM inconnu".to_owned()
            } else {
                visible(&request.client)
            },
            explanation: visible(&request.explanation),
            arguments,
            remaining_ms: u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX),
        }
    }

    pub fn title(&self) -> String {
        format!("Confirmation — {}", self.path)
    }
}

/// The initialization script of the detail window: its data as `window.__MCP_PROXY_CONFIRM__`, frozen, written as
/// data only.
pub fn detail_script(view: &DetailView) -> String {
    let data = json!({
        "path": view.path,
        "client": view.client,
        "explanation": view.explanation,
        "arguments": view.arguments,
        "remainingMs": view.remaining_ms,
        "command": CONFIRM_COMMAND,
    });
    format!(
        "Object.defineProperty(window, '__MCP_PROXY_CONFIRM__', {{ value: Object.freeze({}) }});",
        js_data(&data)
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::Duration;

    use mcp_proxy_core::config::{EnvId, McpId, ToolName};
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn labels_name_one_request() {
        let id = ConfirmationId::from_raw(17);
        assert_eq!(detail_label(id), "confirm-17");
        assert_eq!(confirmation_of_label("confirm-17"), Some(id));
        for other in [
            "confirm-",
            "confirm-017",
            "confirm-1a",
            "confirm-+1",
            "basic-1",
            "login-17",
            "main",
            "confirm-1 ",
        ] {
            assert_eq!(confirmation_of_label(other), None, "{other:?}");
        }
    }

    // Exigence : EX-013-03
    #[test]
    fn the_window_shows_everything_as_data() {
        let request = ConfirmationRequest {
            env: EnvId::new("prod").unwrap(),
            mcp: McpId::new("kibana-logs").unwrap(),
            tool: ToolName::new("delete_index").unwrap(),
            path: "PROD / kibana-logs / delete_index".to_owned(),
            client: "Claude\u{202e} Desktop".to_owned(),
            arguments: json!({ "query": "a</script><script>alert(1)</script>", "lines": "un\ndeux", "n": 3 }),
            explanation: "outil destructif en PROD".to_owned(),
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let view = DetailView::new(ConfirmationId::from_raw(3), &request);
        assert_eq!(view.title(), "Confirmation — PROD / kibana-logs / delete_index");
        assert_eq!(view.client, "Claude\\u{202e} Desktop");
        assert!(view.arguments.contains(r#""lines": "un\ndeux""#), "{}", view.arguments);
        assert!(view.remaining_ms > 29_000 && view.remaining_ms <= 30_000);

        let script = detail_script(&view);
        assert!(!script.contains("</script>"));
        assert!(!script.contains('\n'));
        let json = script
            .strip_prefix("Object.defineProperty(window, '__MCP_PROXY_CONFIRM__', { value: Object.freeze(")
            .and_then(|s| s.strip_suffix(") });"))
            .unwrap();
        let data: Value = serde_json::from_str(json).unwrap();
        assert_eq!(data["command"], "answer_confirmation");
        assert_eq!(data["path"], "PROD / kibana-logs / delete_index");
        let arguments: Value = serde_json::from_str(data["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments, request.arguments, "every argument, whole");
    }
}
