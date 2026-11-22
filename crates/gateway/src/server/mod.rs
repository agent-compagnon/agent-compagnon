//! Client side of the gateway: the MCP server an LLM client talks to, over any byte stream.
//!
//! One `GatewayServer` serves both protocol generations on the same stream: `initialize` handshake, or 2026-07-28
//! per-request metadata. Only tools are announced and relayed: resources, prompts, sampling, roots and elicitation of
//! the servers are neither.

mod chain;
mod output;
mod pipeline;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mcp_proxy_core::filter::FilterError;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorData, Implementation, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, ServerCapabilities, ServerConfig, SubscriptionFilter,
};
use rmcp::service::{RequestContext, RoleServer, RunningService, ServerInitializeError, SubscriptionContext};
use rmcp::{ServerHandler, ServiceExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::task::JoinHandle;

pub use self::chain::{
    CallChain, CallChainDeps, CallContext, ChainError, Halt, Middleware, Refusal, ResolvedTool, STAGES,
};
pub use self::output::{FILTER_UNAVAILABLE, Filtered, OutputGate};
pub use self::pipeline::{CallPipeline, CallRequest};
use crate::registry::{Registry, RegistryEvent};
use crate::upstream::ProgressUpdate;

/// Who is calling, read from each request: in 2026-07-28 every request carries it, in the legacy generation it
/// comes from the `initialize` handshake. Every field is chosen by the client: filter before showing it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientIdentity {
    pub name: String,
    pub version: String,
    pub protocol_version: Option<String>,
    pub supports_elicitation: bool,
}

impl ClientIdentity {
    pub fn from_context(context: &RequestContext<RoleServer>) -> Self {
        let info = context.client_info();
        Self {
            name: info.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
            version: info.map(|i| i.version).unwrap_or_default(),
            protocol_version: context.protocol_version().map(|v| v.as_str().to_owned()),
            supports_elicitation: context.client_capabilities().is_some_and(|c| c.elicitation.is_some()),
        }
    }
}

/// One client connection (one stream), unique in the process. A confirmation asked in the chat is only valid on
/// the connection it was asked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

impl ConnectionId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Shared by every connection.
#[derive(Clone)]
pub struct GatewayServer {
    registry: Arc<RwLock<Registry>>,
    output: OutputGate,
    pipeline: Arc<dyn CallPipeline>,
    list_ttl: Duration,
    connection: ConnectionId,
}

impl std::fmt::Debug for GatewayServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayServer")
            .field("list_ttl", &self.list_ttl)
            .finish_non_exhaustive()
    }
}

/// One client connection. Stops relaying list changes when dropped.
pub struct GatewayConnection {
    running: RunningService<RoleServer, GatewayServer>,
    _notifier: AbortOnDrop,
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl GatewayConnection {
    pub fn running(&self) -> &RunningService<RoleServer, GatewayServer> {
        &self.running
    }

    /// Until the client goes away.
    pub async fn waiting(self) {
        let _ = self.running.waiting().await;
    }
}

fn implementation() -> Implementation {
    Implementation::new("mcp-proxy", env!("CARGO_PKG_VERSION"))
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities::builder()
        .enable_tools()
        .enable_tool_list_changed()
        .build()
}

fn filter_failure(_: FilterError) -> ErrorData {
    ErrorData::internal_error("MCP Proxy : filtre de sortie indisponible, liste non transmise.", None)
}

/// The registry changed in a way a client must hear about.
fn tools_may_have_changed(event: &Result<RegistryEvent, broadcast::error::RecvError>) -> bool {
    !matches!(event, Err(broadcast::error::RecvError::Closed))
}

impl GatewayServer {
    /// `list_ttl` is announced with every list (`list_ttl_secs`).
    pub fn new(
        registry: Arc<RwLock<Registry>>,
        output: OutputGate,
        pipeline: Arc<dyn CallPipeline>,
        list_ttl: Duration,
    ) -> Self {
        Self {
            registry,
            output,
            pipeline,
            list_ttl,
            connection: ConnectionId::next(),
        }
    }

    /// Serves one client on `stream` (stdio, named pipe, in-memory duplex…). Legacy clients are told of registry
    /// changes by `notifications/tools/list_changed`; 2026-07-28 clients through `subscriptions/listen`.
    ///
    /// Returns once the lifecycle is known: after `initialize`, or, for a 2026-07-28 client, after its first request
    /// other than `server/discover` (which is answered meanwhile). Do not wait for it before letting the client go on.
    pub async fn serve<S>(mut self, stream: S) -> Result<GatewayConnection, ServerInitializeError>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.connection = ConnectionId::next();
        let mut events = self.registry.read().await.subscribe();
        let running = ServiceExt::serve(self, stream).await?;
        let peer = running.peer().clone();
        let notifier = tokio::spawn(async move {
            loop {
                let event = events.recv().await;
                if !tools_may_have_changed(&event) {
                    return;
                }
                // Only sessions that went through `initialize`: the others listen.
                if peer.peer_info().is_some() && peer.notify_tool_list_changed().await.is_err() {
                    return;
                }
            }
        });
        Ok(GatewayConnection {
            running,
            _notifier: AbortOnDrop(notifier),
        })
    }

    /// `serve`, then serves until the client goes away. Errors of the handshake end the connection.
    pub async fn run<S>(self, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Ok(connection) = self.serve(stream).await {
            connection.waiting().await;
        }
    }

    /// Forwards the progress of one call to the client, under the client's token, with filtered texts.
    fn progress_relay(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Option<(mpsc::Sender<ProgressUpdate>, JoinHandle<()>)> {
        let token = context.meta.get_progress_token()?;
        let peer = context.peer.clone();
        let output = self.output.clone();
        let (sender, mut receiver) = mpsc::channel::<ProgressUpdate>(16);
        let task = tokio::spawn(async move {
            while let Some(update) = receiver.recv().await {
                let mut notification = ProgressNotificationParam::new(token.clone(), update.progress);
                notification.total = update.total;
                notification.message = update.message.as_deref().and_then(|m| output.text(m));
                if peer.notify_progress(notification).await.is_err() {
                    return;
                }
            }
        });
        Some((sender, task))
    }
}

impl ServerHandler for GatewayServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(capabilities()).with_server_info(implementation())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let definitions: Vec<_> = {
            let registry = self.registry.read().await;
            registry
                .exposed_tools()
                .iter()
                .map(|e| e.exposed_definition())
                .collect()
        };
        let tools = self.output.tools(&definitions).map_err(filter_failure)?;
        let ttl_ms = u64::try_from(self.list_ttl.as_millis()).unwrap_or(u64::MAX);
        Ok(ListToolsResult::with_all_items(tools).with_ttl_ms(ttl_ms))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let relay = self.progress_relay(&context);
        let (progress, relay_task) = relay.unzip();
        let call = CallRequest {
            exposed_name: request.name.into_owned(),
            arguments: request.arguments,
            client: ClientIdentity::from_context(&context),
            connection: self.connection,
            peer: Some(context.peer.clone()),
            input_responses: request.input_responses,
            request_state: request.request_state,
            progress,
            cancel: context.ct.clone(),
        };
        let filtered = self.pipeline.call(call).await;
        // The sender is gone with the call: let the last progress notifications out before the result.
        if let Some(task) = relay_task {
            let _ = task.await;
        }
        Ok(filtered.into_response())
    }

    fn accepted_subscription_filter(&self, requested: &SubscriptionFilter) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&capabilities()))
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mut events = self.registry.read().await.subscribe();
        loop {
            tokio::select! {
                () = context.cancelled() => return Ok(()),
                event = events.recv() => {
                    if !tools_may_have_changed(&event) {
                        return Ok(());
                    }
                    if context.sink().notify_tool_list_changed().await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
}
