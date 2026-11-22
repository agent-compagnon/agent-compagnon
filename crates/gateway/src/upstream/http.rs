//! `HttpUpstream`: a lazily connected MCP client over Streamable HTTP, compatible with both protocol generations.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use mcp_proxy_core::config::{Config, SessionId};
use mcp_proxy_core::session::SessionGate;
use rmcp::handler::client::progress::ProgressDispatcher;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ClientRequest,
    Implementation, ProgressNotificationParam, ProtocolVersion, ServerNotification, ServerResult, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, Peer, PeerRequestOptions, RoleClient, RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt};
use tokio::sync::{Mutex, broadcast};
use url::Url;

use super::http_client::{InjectingHttpClient, InjectionLog};
use super::{ProgressUpdate, Upstream, UpstreamCall, UpstreamError};
use crate::convert::definition_from_tool;
use crate::registry::{InstanceId, ToolDefinition};

/// Name and version announced to servers.
pub(crate) fn implementation() -> Implementation {
    Implementation::new("mcp-proxy", env!("CARGO_PKG_VERSION"))
}

/// How to reach one instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUpstreamConfig {
    pub instance: InstanceId,
    pub url: Url,
    /// `None`: the server needs no credential.
    pub session: Option<SessionId>,
    /// Without an active session, `list_tools` fails with `SessionRequired` instead of trying anonymously.
    pub list_requires_auth: bool,
    /// Bounds each request, and the connection.
    pub timeout: Duration,
    /// First delay before a new connection attempt after a failure; doubles at each failure up to `max_backoff`.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl HttpUpstreamConfig {
    pub const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
    pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(60);

    pub fn new(instance: InstanceId, url: Url, session: Option<SessionId>, timeout: Duration) -> Self {
        Self {
            instance,
            url,
            session,
            list_requires_auth: true,
            timeout,
            initial_backoff: Self::DEFAULT_INITIAL_BACKOFF,
            max_backoff: Self::DEFAULT_MAX_BACKOFF,
        }
    }

    /// From the configuration: instance URL, session of the MCP, `list_requires_auth`, `upstream_timeout_secs`.
    /// `None` for an unknown instance.
    pub fn for_instance(config: &Config, instance: &InstanceId) -> Option<Self> {
        let mcp = config.mcp(&instance.mcp)?;
        let url = mcp.instances.get(&instance.env)?.clone();
        let session = config.session_for(&instance.mcp, &instance.env).map(|s| s.id.clone());
        let mut target = Self::new(
            instance.clone(),
            url,
            session,
            Duration::from_secs(config.gateway.upstream_timeout_secs),
        );
        target.list_requires_auth = mcp.list_requires_auth;
        Some(target)
    }
}

/// Receives the server's notifications: progress, and changes of the list of tools (legacy generation).
#[derive(Clone)]
struct UpstreamHandler {
    progress: ProgressDispatcher,
    changes: broadcast::Sender<()>,
}

impl ClientHandler for UpstreamHandler {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(ClientCapabilities::default(), implementation())
    }

    async fn on_progress(&self, params: ProgressNotificationParam, _: NotificationContext<RoleClient>) {
        self.progress.handle_notification(params).await;
    }

    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.changes.send(());
    }
}

#[derive(Default)]
struct Connection {
    running: Option<RunningService<RoleClient, UpstreamHandler>>,
    failures: u32,
    retry_at: Option<Instant>,
}

pub struct HttpUpstream {
    config: HttpUpstreamConfig,
    gate: Arc<dyn SessionGate>,
    handler: UpstreamHandler,
    connection: Mutex<Connection>,
    /// Shared by every transport of this upstream, so that a rejection is seen whichever connection hit it.
    log: Arc<InjectionLog>,
}

impl fmt::Debug for HttpUpstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpUpstream")
            .field("instance", &self.config.instance)
            .field("url", &self.config.url.as_str())
            .finish_non_exhaustive()
    }
}

impl HttpUpstream {
    pub fn new(config: HttpUpstreamConfig, gate: Arc<dyn SessionGate>) -> Self {
        let (changes, _) = broadcast::channel(16);
        Self {
            config,
            gate,
            handler: UpstreamHandler {
                progress: ProgressDispatcher::new(),
                changes,
            },
            connection: Mutex::new(Connection::default()),
            log: Arc::default(),
        }
    }

    pub fn config(&self) -> &HttpUpstreamConfig {
        &self.config
    }

    /// The peer of the current connection, connecting first if there is none or it was closed. After a failure, no
    /// new attempt is made before the backoff delay.
    async fn peer(&self) -> Result<Peer<RoleClient>, UpstreamError> {
        let mut connection = self.connection.lock().await;
        if let Some(running) = &connection.running {
            if !running.is_closed() {
                return Ok(running.peer().clone());
            }
            connection.running = None;
        }
        if let Some(retry_at) = connection.retry_at {
            let now = Instant::now();
            if now < retry_at {
                return Err(UpstreamError::Unavailable {
                    retry_in: retry_at - now,
                });
            }
        }
        match self.connect().await {
            Ok(running) => {
                let peer = running.peer().clone();
                connection.running = Some(running);
                connection.failures = 0;
                connection.retry_at = None;
                self.watch_list_changes(&peer);
                Ok(peer)
            }
            Err(error) => {
                let factor = 2u32.saturating_pow(connection.failures.min(16));
                let delay = self
                    .config
                    .initial_backoff
                    .saturating_mul(factor)
                    .min(self.config.max_backoff);
                connection.failures = connection.failures.saturating_add(1);
                connection.retry_at = Some(Instant::now() + delay);
                Err(error)
            }
        }
    }

    async fn connect(&self) -> Result<RunningService<RoleClient, UpstreamHandler>, UpstreamError> {
        let before = self.log.rejections();
        let mut client =
            InjectingHttpClient::new(Arc::clone(&self.gate), self.config.session.clone(), self.config.timeout)
                .map_err(|_| UpstreamError::Connect("client HTTP : initialisation impossible".to_owned()))?;
        client.share_log(Arc::clone(&self.log));
        let transport = StreamableHttpClientTransport::with_client(
            client,
            StreamableHttpClientTransportConfig::with_uri(self.config.url.as_str()),
        );
        let lifecycle = ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::LATEST_WITH_INITIALIZE),
        };
        let started = tokio::time::timeout(
            self.config.timeout,
            self.handler.clone().serve_with_lifecycle(transport, lifecycle),
        )
        .await;
        match started {
            Ok(Ok(running)) => Ok(running),
            Ok(Err(error)) => Err(self.failure(before, || UpstreamError::Connect(error.to_string()))),
            Err(_) => Err(UpstreamError::Timeout(self.config.timeout)),
        }
    }

    /// On a 2026-07-28 connection, list changes come through `subscriptions/listen`; on a legacy one, through
    /// `UpstreamHandler::on_tool_list_changed`.
    fn watch_list_changes(&self, peer: &Peer<RoleClient>) {
        let modern = peer
            .peer_info()
            .is_some_and(|info| info.protocol_version.as_str() >= ProtocolVersion::V_2026_07_28.as_str());
        if !modern {
            return;
        }
        let peer = peer.clone();
        let changes = self.handler.changes.clone();
        tokio::spawn(async move {
            let filter = SubscriptionFilter::builder().tools_list_changed().build();
            let Ok(mut subscription) = peer.listen(filter).await else {
                return;
            };
            while let Ok(Some(notification)) = subscription.next().await {
                if matches!(notification, ServerNotification::ToolListChangedNotification(_)) {
                    let _ = changes.send(());
                }
            }
        });
    }

    /// The error to report: a rejection or an injection refusal seen during the exchange wins over what the SDK
    /// reported.
    fn failure(&self, rejections_before: u64, otherwise: impl FnOnce() -> UpstreamError) -> UpstreamError {
        if let Some(refusal) = self.log.take_refusal() {
            return UpstreamError::Session(refusal);
        }
        if self.log.rejections() != rejections_before {
            return UpstreamError::AuthRejected;
        }
        otherwise()
    }

    async fn service_error(&self, rejections_before: u64, error: ServiceError) -> UpstreamError {
        let closed = matches!(error, ServiceError::TransportClosed | ServiceError::TransportSend(_));
        let mapped = self.failure(rejections_before, || match error {
            ServiceError::McpError(data) => UpstreamError::Remote {
                code: data.code.0,
                message: data.message.into_owned(),
            },
            ServiceError::Timeout { timeout } => UpstreamError::Timeout(timeout),
            ServiceError::Cancelled { .. } => UpstreamError::Cancelled,
            other => UpstreamError::Protocol(other.to_string()),
        });
        if closed {
            self.connection.lock().await.running = None;
        }
        mapped
    }

    fn require_session(&self) -> Result<(), UpstreamError> {
        match &self.config.session {
            Some(session) if self.config.list_requires_auth && !self.gate.is_active(session) => {
                Err(UpstreamError::SessionRequired(session.clone()))
            }
            _ => Ok(()),
        }
    }
}

#[async_trait]
impl Upstream for HttpUpstream {
    fn instance(&self) -> &InstanceId {
        &self.config.instance
    }

    async fn list_tools(&self) -> Result<Vec<ToolDefinition>, UpstreamError> {
        self.require_session()?;
        let before = self.log.rejections();
        let peer = self.peer().await?;
        let tools = match tokio::time::timeout(self.config.timeout, peer.list_all_tools()).await {
            Ok(Ok(tools)) => tools,
            Ok(Err(error)) => return Err(self.service_error(before, error).await),
            Err(_) => return Err(UpstreamError::Timeout(self.config.timeout)),
        };
        tools
            .iter()
            .map(|tool| {
                definition_from_tool(tool)
                    .map_err(|_| UpstreamError::Protocol("définition d'outil illisible".to_owned()))
            })
            .collect()
    }

    async fn call_tool(&self, call: UpstreamCall) -> Result<CallToolResult, UpstreamError> {
        let before = self.log.rejections();
        let peer = self.peer().await?;
        let mut params = CallToolRequestParams::new(call.tool.as_str().to_owned());
        params.arguments = call.arguments;
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let handle = match peer
            .send_request_with_option(request, PeerRequestOptions::with_timeout(self.config.timeout))
            .await
        {
            Ok(handle) => handle,
            Err(error) => return Err(self.service_error(before, error).await),
        };
        // Progress sent before this subscription is registered is lost: the token is only known once the request
        // is queued.
        let relay = match call.progress {
            Some(sink) => {
                let mut subscriber = self.handler.progress.subscribe(handle.progress_token.clone()).await;
                Some(tokio::spawn(async move {
                    while let Some(progress) = subscriber.next().await {
                        let update = ProgressUpdate {
                            progress: progress.progress,
                            total: progress.total,
                            message: progress.message,
                        };
                        if sink.send(update).await.is_err() {
                            break;
                        }
                    }
                }))
            }
            None => None,
        };
        let response = handle.await_response().await;
        if let Some(relay) = relay {
            relay.abort();
        }
        match response {
            Ok(ServerResult::CallToolResult(result)) => Ok(result),
            Ok(_) => Err(UpstreamError::Protocol(
                "réponse inattendue (résultat incomplet ou tâche, non gérés)".to_owned(),
            )),
            Err(error) => Err(self.service_error(before, error).await),
        }
    }

    fn changes(&self) -> broadcast::Receiver<()> {
        self.handler.changes.subscribe()
    }
}
