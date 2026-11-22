//! The last step before the LLM client: every result, error, progress text and tool list goes through the output
//! filter here (INV-3). If the filter fails, nothing of the content leaves.

use std::sync::Arc;

use mcp_proxy_core::filter::{FilterError, FilterReport, OutputFilter};
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde_json::Value;

use crate::registry::ToolDefinition;

/// Sent instead of a result or an error that could not be filtered. A fixed text: it needs no filtering.
pub const FILTER_UNAVAILABLE: &str = "MCP Proxy : filtre de sortie indisponible, réponse non transmise.";

/// A tool result that went through the output filter. Only `OutputGate` builds one, so the server cannot send
/// anything else.
#[derive(Debug, Clone, PartialEq)]
pub struct Filtered {
    result: CallToolResult,
    report: FilterReport,
}

impl Filtered {
    pub fn result(&self) -> &CallToolResult {
        &self.result
    }

    /// What the filter masked. Empty when the filter was unavailable: the content was replaced as a whole.
    pub fn report(&self) -> &FilterReport {
        &self.report
    }

    pub fn into_result(self) -> CallToolResult {
        self.result
    }

    fn unavailable() -> Self {
        Self {
            result: CallToolResult::error(vec![ContentBlock::text(FILTER_UNAVAILABLE)]),
            report: FilterReport::default(),
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
            Ok(result) => Filtered { result, report },
            Err(_) => Filtered::unavailable(),
        }
    }

    /// A tool error with a French message, filtered: it may quote the server or the client.
    pub fn error(&self, message: &str) -> Filtered {
        match self.filter.filter_text(message) {
            Ok((text, report)) => Filtered {
                result: CallToolResult::error(vec![ContentBlock::text(text.into_owned())]),
                report,
            },
            Err(_) => Filtered::unavailable(),
        }
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
