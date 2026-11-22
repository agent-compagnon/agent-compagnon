//! Server side of the gateway: one `Upstream` per instance (environment, MCP).
//!
//! `HttpUpstream` talks to a remote MCP over Streamable HTTP, in either protocol generation, with the credential
//! injected by the `SessionGate` when each request is sent. Local modules (spec 025) will be another implementation.

mod http;
mod http_client;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::config::{Config, McpKind, SessionId, ToolName};
use mcp_proxy_core::session::{SessionError, SessionGate};
use rmcp::model::{CallToolResult, JsonObject};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast, mpsc};

pub use self::http::{HttpUpstream, HttpUpstreamConfig};
pub use self::http_client::InjectingHttpClient;
use crate::registry::{InstanceId, Registry, RegistryError, ToolDefinition, ToolDiff};

/// A progress notification received from a server, not filtered yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressUpdate {
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

/// Where the progress notifications of one call go. Dropped when the call ends.
pub type ProgressSink = mpsc::Sender<ProgressUpdate>;

/// One call to a server, with the tool's original name.
#[derive(Debug, Clone)]
pub struct UpstreamCall {
    pub tool: ToolName,
    pub arguments: Option<JsonObject>,
    pub progress: Option<ProgressSink>,
}

/// Why a server could not answer. French. Texts coming from the server (`Remote`, `Protocol`, `Connect`) may carry
/// anything, the body of an HTTP error included: they must go through the output filter before leaving.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum UpstreamError {
    #[error("connexion à la session « {0} » nécessaire avant de lister les outils de ce serveur")]
    SessionRequired(SessionId),
    #[error("identifiant refusé par le serveur : la session est expirée, une nouvelle connexion est nécessaire")]
    AuthRejected,
    #[error("{0}")]
    Session(#[from] SessionError),
    #[error("le serveur n'a pas répondu dans le délai ({} s)", .0.as_secs())]
    Timeout(Duration),
    #[error("serveur injoignable, nouvel essai possible dans {} s", .retry_in.as_secs().max(1))]
    Unavailable { retry_in: Duration },
    #[error("connexion au serveur impossible : {0}")]
    Connect(String),
    #[error("erreur renvoyée par le serveur : {message}")]
    Remote { code: i32, message: String },
    #[error("échange avec le serveur en échec : {0}")]
    Protocol(String),
    #[error("appel annulé")]
    Cancelled,
}

#[async_trait]
pub trait Upstream: Send + Sync {
    fn instance(&self) -> &InstanceId;

    /// Every page of `tools/list`, reduced to what the registry knows.
    async fn list_tools(&self) -> Result<Vec<ToolDefinition>, UpstreamError>;

    async fn call_tool(&self, call: UpstreamCall) -> Result<CallToolResult, UpstreamError>;

    /// Fires when the server says its list of tools changed.
    fn changes(&self) -> broadcast::Receiver<()>;

    /// Fires after each connection and reconnection: the list may have changed meanwhile. Closed at once for an
    /// upstream without connections.
    fn connections(&self) -> broadcast::Receiver<()> {
        broadcast::channel(1).1
    }

    /// A connection is open now. Asking never opens one.
    fn is_connected(&self) -> bool {
        false
    }

    /// How long the server said its last complete list stays valid (`ttl_ms`), if it said so.
    fn list_ttl(&self) -> Option<Duration> {
        None
    }
}

/// The upstream of each instance that has one.
#[derive(Default, Clone)]
pub struct Upstreams {
    by_instance: BTreeMap<InstanceId, Arc<dyn Upstream>>,
}

impl std::fmt::Debug for Upstreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.by_instance.keys()).finish()
    }
}

impl Upstreams {
    /// An `HttpUpstream` for every instance of every remote MCP of `config`. Local modules come with spec 025.
    pub fn from_config(config: &Config, gate: &Arc<dyn SessionGate>) -> Self {
        let mut upstreams = Self::default();
        for mcp in config.mcp.iter().filter(|m| m.kind == McpKind::Remote) {
            for env in mcp.instances.keys() {
                let instance = InstanceId::new(env.clone(), mcp.id.clone());
                if let Some(target) = HttpUpstreamConfig::for_instance(config, &instance) {
                    upstreams.insert(Arc::new(HttpUpstream::new(target, Arc::clone(gate))));
                }
            }
        }
        upstreams
    }

    pub fn insert(&mut self, upstream: Arc<dyn Upstream>) {
        self.by_instance.insert(upstream.instance().clone(), upstream);
    }

    pub fn get(&self, instance: &InstanceId) -> Option<&Arc<dyn Upstream>> {
        self.by_instance.get(instance)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Upstream>> {
        self.by_instance.values()
    }
}

/// Why a live list could not replace the tools of an instance. The instance keeps its previous list.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RefreshError {
    #[error("{0}")]
    Upstream(#[from] UpstreamError),
    #[error("{0}")]
    Registry(#[from] RegistryError),
}

/// Lists the tools of `upstream` and installs them in the registry as its live list.
pub async fn refresh(registry: &RwLock<Registry>, upstream: &dyn Upstream) -> Result<ToolDiff, RefreshError> {
    let tools = upstream.list_tools().await?;
    Ok(registry.write().await.apply_live_list(upstream.instance(), tools)?)
}
