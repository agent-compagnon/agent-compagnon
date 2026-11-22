//! The last step before the LLM client: every result, error, progress text and tool list goes through the output
//! filter here (INV-3). If the filter fails, nothing of the content leaves.

use std::sync::Arc;

use mcp_proxy_core::filter::{FilterError, FilterReport, OutputFilter};
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, InputRequiredResult, Tool};
use serde_json::Value;

use crate::registry::ToolDefinition;

/// Sent instead of a result or an error that could not be filtered. A fixed text: it needs no filtering.
pub const FILTER_UNAVAILABLE: &str = "MCP Proxy : filtre de sortie indisponible, réponse non transmise.";

/// What the client receives for a `tools/call`, after the output filter. Only `OutputGate` builds one, so the server
/// cannot send anything else.
#[derive(Debug, Clone)]
pub struct Filtered {
    body: Body,
    report: FilterReport,
    unavailable: bool,
}

#[derive(Debug, Clone)]
enum Body {
    Complete(CallToolResult),
    /// 2026-07-28: the client must answer `input_requests` and call again with `request_state`.
    InputRequired(InputRequiredResult),
}

impl Filtered {
    /// The tool result, or `None` for an `input_required` answer.
    pub fn result(&self) -> Option<&CallToolResult> {
        match &self.body {
            Body::Complete(result) => Some(result),
            Body::InputRequired(_) => None,
        }
    }

    pub fn input_required(&self) -> Option<&InputRequiredResult> {
        match &self.body {
            Body::Complete(_) => None,
            Body::InputRequired(result) => Some(result),
        }
    }

    /// What the filter masked. Empty when the filter was unavailable: the content was replaced as a whole.
    pub fn report(&self) -> &FilterReport {
        &self.report
    }

    /// The filter failed and the content was replaced by [`FILTER_UNAVAILABLE`].
    pub fn is_unavailable(&self) -> bool {
        self.unavailable
    }

    /// The answer as JSON, for the trace.
    pub fn to_value(&self) -> Value {
        let value = match &self.body {
            Body::Complete(result) => serde_json::to_value(result),
            Body::InputRequired(result) => serde_json::to_value(result),
        };
        value.unwrap_or(Value::Null)
    }

    pub fn into_response(self) -> CallToolResponse {
        match self.body {
            Body::Complete(result) => CallToolResponse::Complete(result),
            Body::InputRequired(result) => CallToolResponse::InputRequired(result),
        }
    }

    fn unavailable() -> Self {
        Self {
            body: Body::Complete(CallToolResult::error(vec![ContentBlock::text(FILTER_UNAVAILABLE)])),
            report: FilterReport::default(),
            unavailable: true,
        }
    }

    fn complete(result: CallToolResult, report: FilterReport) -> Self {
        Self {
            body: Body::Complete(result),
            report,
            unavailable: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutputGate {
    filter: Arc<OutputFilter>,
}

impl OutputGate {
    pub fn new(filter: Arc<OutputFilter>) -> Self {
        Self { filter }
    }

    pub fn filter(&self) -> &Arc<OutputFilter> {
        &self.filter
    }

    /// The whole result, content, structured content and metadata, filtered as JSON.
    pub fn result(&self, result: &CallToolResult) -> Filtered {
        let Ok(mut value) = serde_json::to_value(result) else {
            return Filtered::unavailable();
        };
        let Ok(report) = self.filter.filter_json(&mut value) else {
            return Filtered::unavailable();
        };
        match serde_json::from_value(value) {
            Ok(result) => Filtered::complete(result, report),
            Err(_) => Filtered::unavailable(),
        }
    }

    /// An `input_required` answer, filtered as a whole: request texts and state included. A state changed by the
    /// filter no longer opens, and the call that brings it back is refused.
    pub fn input_required(&self, result: &InputRequiredResult) -> Filtered {
        let Ok(mut value) = serde_json::to_value(result) else {
            return Filtered::unavailable();
        };
        let Ok(report) = self.filter.filter_json(&mut value) else {
            return Filtered::unavailable();
        };
        match serde_json::from_value(value) {
            Ok(result) => Filtered {
                body: Body::InputRequired(result),
                report,
                unavailable: false,
            },
            Err(_) => Filtered::unavailable(),
        }
    }

    /// A tool error with a French message, filtered: it may quote the server or the client.
    pub fn error(&self, message: &str) -> Filtered {
        match self.filter.filter_text(message) {
            Ok((text, report)) => Filtered::complete(
                CallToolResult::error(vec![ContentBlock::text(text.into_owned())]),
                report,
            ),
            Err(_) => Filtered::unavailable(),
        }
    }

    /// Any JSON value (arguments shown to the user or traced), or `None` if the filter is unavailable.
    pub fn value(&self, value: &Value) -> Option<Value> {
        let mut value = value.clone();
        self.filter.filter_json(&mut value).ok().map(|_| value)
    }

    /// A short text (progress message), or `None` if the filter is unavailable.
    pub fn text(&self, text: &str) -> Option<String> {
        self.filter.filter_text(text).ok().map(|(t, _)| t.into_owned())
    }

    /// Definitions as sent to clients, filtered again: the list of known secrets grows after each connection. A
    /// tool whose name the filter would change is left out, since a masked name could not be called.
    pub fn tools(&self, definitions: &[ToolDefinition]) -> Result<Vec<Tool>, FilterError> {
        let mut tools = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let Ok(mut value) = serde_json::to_value(definition) else {
                continue;
            };
            self.filter.filter_json(&mut value)?;
            if value.get("name").and_then(Value::as_str) != Some(definition.name.as_str()) {
                continue;
            }
            if let Ok(tool) = serde_json::from_value(value) {
                tools.push(tool);
            }
        }
        Ok(tools)
    }
}
