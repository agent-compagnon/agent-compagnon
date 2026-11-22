//! What happens to a `tools/call` between the client and the server.
//!
//! `CallPipeline` is the seam between the MCP server and the call chain (`CallChain`, spec 007).

use async_trait::async_trait;
use rmcp::model::{InputResponses, JsonObject};
use rmcp::service::{Peer, RoleServer};
use tokio_util::sync::CancellationToken;

use super::output::Filtered;
use super::{ClientIdentity, ConnectionId};
use crate::upstream::ProgressSink;

/// One `tools/call` as received from the client. `Debug` leaves out the arguments and what the client brought back:
/// they are not filtered.
#[derive(Clone)]
pub struct CallRequest {
    pub exposed_name: String,
    pub arguments: Option<JsonObject>,
    pub client: ClientIdentity,
    /// The client connection (stream) the request came on.
    pub connection: ConnectionId,
    /// To send requests to the client (legacy elicitation). `None` outside a live connection.
    pub peer: Option<Peer<RoleServer>>,
    /// 2026-07-28: answers to the `input_requests` of a previous `input_required` result.
    pub input_responses: Option<InputResponses>,
    /// 2026-07-28: the `request_state` of a previous `input_required` result, as the client brings it back.
    pub request_state: Option<String>,
    /// Present when the client gave a progress token.
    pub progress: Option<ProgressSink>,
    /// Cancelled when the client cancels the request or goes away.
    pub cancel: CancellationToken,
}

impl std::fmt::Debug for CallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallRequest")
            .field("connection", &self.connection)
            .field("progress", &self.progress.is_some())
            .finish_non_exhaustive()
    }
}

impl CallRequest {
    /// A request with nothing but a name and arguments, outside any live connection.
    pub fn new(exposed_name: impl Into<String>, arguments: Option<JsonObject>, client: ClientIdentity) -> Self {
        Self {
            exposed_name: exposed_name.into(),
            arguments,
            client,
            connection: ConnectionId::next(),
            peer: None,
            input_responses: None,
            request_state: None,
            progress: None,
            cancel: CancellationToken::new(),
        }
    }
}

#[async_trait]
pub trait CallPipeline: Send + Sync {
    /// Always answers: a refusal or an error is a filtered tool result with `is_error = true`.
    async fn call(&self, request: CallRequest) -> Filtered;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn debug_shows_neither_arguments_nor_state() {
        let mut request = CallRequest::new(
            "dev_kibana_search_logs",
            json!({ "query": "argument-secret" }).as_object().cloned(),
            ClientIdentity::default(),
        );
        request.request_state = Some("etat-secret".to_owned());
        let text = format!("{request:?}");
        assert!(
            !text.contains("argument-secret") && !text.contains("etat-secret"),
            "{text}"
        );
    }
}
