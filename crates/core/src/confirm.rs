//! Contract of the confirmation channel outside the chat (Windows notification, spec 013). Declared here so that the
//! call pipeline (spec 007) and the test doubles share it; the application's broker is
//! `mcp_proxy_gateway::notify::NotificationBroker`.

use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::{EnvId, McpId, ToolName};

/// What the user is asked to approve. Every text and the arguments have already been through the output filter.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmationRequest {
    pub env: EnvId,
    pub mcp: McpId,
    pub tool: ToolName,
    /// `DEV / kibana-logs / delete_index`: environment label, MCP and tool.
    pub path: String,
    /// `clientInfo.name` of the LLM client, filtered.
    pub client: String,
    pub arguments: Value,
    /// French explanation of the policy rule that asks for the confirmation.
    pub explanation: String,
    pub deadline: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationOutcome {
    Approved,
    Refused,
    TimedOut,
    /// The channel could not show the request.
    Unavailable,
}

impl ConfirmationOutcome {
    /// Only an explicit approval lets a call through (INV-5).
    pub fn is_approved(self) -> bool {
        self == Self::Approved
    }
}

#[async_trait]
pub trait ConfirmationBroker: Send + Sync {
    async fn confirm(&self, request: ConfirmationRequest) -> ConfirmationOutcome;
}
