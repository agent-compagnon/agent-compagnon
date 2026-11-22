//! What happens to a `tools/call` between the client and the server.
//!
//! `CallPipeline` is the insertion point of the middleware chain (spec 007). This step ships `MinimalPipeline`:
//! resolution, session, upstream call, output filter.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mcp_proxy_core::config::Config;
use mcp_proxy_core::session::{AccessReason, SessionGate};
use rmcp::model::JsonObject;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::ClientIdentity;
use super::output::{Filtered, OutputGate};
use crate::registry::Registry;
use crate::upstream::{ProgressSink, UpstreamCall, UpstreamError, Upstreams};

/// One `tools/call` as received from the client.
#[derive(Debug, Clone)]
pub struct CallRequest {
    pub exposed_name: String,
    pub arguments: Option<JsonObject>,
    pub client: ClientIdentity,
    /// Present when the client gave a progress token.
    pub progress: Option<ProgressSink>,
    /// Cancelled when the client cancels the request or goes away.
    pub cancel: CancellationToken,
}

#[async_trait]
pub trait CallPipeline: Send + Sync {
    /// Always answers: a refusal or an error is a filtered tool result with `is_error = true`.
    async fn call(&self, request: CallRequest) -> Filtered;
}

pub struct MinimalPipeline {
    config: Arc<Config>,
    registry: Arc<RwLock<Registry>>,
    upstreams: Upstreams,
    gate: Arc<dyn SessionGate>,
    output: OutputGate,
}

impl std::fmt::Debug for MinimalPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinimalPipeline")
            .field("upstreams", &self.upstreams)
            .finish_non_exhaustive()
    }
}

impl MinimalPipeline {
    pub fn new(
        config: Arc<Config>,
        registry: Arc<RwLock<Registry>>,
        upstreams: Upstreams,
        gate: Arc<dyn SessionGate>,
        output: OutputGate,
    ) -> Self {
        Self {
            config,
            registry,
            upstreams,
            gate,
            output,
        }
    }

    fn refuse(&self, message: &str) -> Filtered {
        self.output.error(&format!("MCP Proxy : {message}"))
    }
}

#[async_trait]
impl CallPipeline for MinimalPipeline {
    async fn call(&self, request: CallRequest) -> Filtered {
        // Only `resolve` may serve a call: it knows nothing of tools that are not exposed.
        let resolved = {
            let registry = self.registry.read().await;
            registry
                .resolve(&request.exposed_name)
                .map(|entry| (entry.instance.clone(), entry.original_name.clone()))
        };
        let Some((instance, tool)) = resolved else {
            return self.refuse(&format!(
                "outil « {} » inconnu ou non exposé, appel refusé.",
                request.exposed_name
            ));
        };

        if let Some(session) = self.config.session_for(&instance.mcp, &instance.env) {
            let deadline = Instant::now() + Duration::from_secs(self.config.gateway.call_budget_secs);
            let reason = AccessReason::ToolCall {
                env: instance.env.clone(),
                mcp: instance.mcp.clone(),
                tool: tool.clone(),
            };
            if let Err(error) = self.gate.ensure(&session.id, deadline, &reason).await {
                return self.refuse(&error.to_string());
            }
        }

        let Some(upstream) = self.upstreams.get(&instance) else {
            return self.refuse(&format!("aucun serveur disponible pour {instance}."));
        };
        let call = UpstreamCall {
            tool,
            arguments: request.arguments,
            progress: request.progress,
        };
        let outcome = tokio::select! {
            outcome = upstream.call_tool(call) => outcome,
            () = request.cancel.cancelled() => Err(UpstreamError::Cancelled),
        };
        match outcome {
            Ok(result) => self.output.result(&result),
            Err(error) => self.refuse(&format!("{instance} : {error}")),
        }
    }
}
