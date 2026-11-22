//! A whole gateway on the embedded configuration, with `dev/kibana-logs` served by a fake MCP server.

#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use mcp_proxy_core::config::{Config, EnvId, McpId, ProfileId, SessionId};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::session::SessionGate;
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_gateway::registry::{EmbeddedCatalog, InstanceId, NoCatalogCache, Registry};
use mcp_proxy_gateway::server::{CallPipeline, GatewayServer, MinimalPipeline, OutputGate};
use mcp_proxy_gateway::upstream::{HttpUpstream, HttpUpstreamConfig, Upstream, Upstreams, refresh};
use mcp_proxy_testkit::{FakeSessionGate, Generation, MockMcpServer, catalog_tools};
use rmcp::model::{CallToolRequestParams, CallToolResult, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use url::Url;

pub const TOKEN: &str = "jeton-amont-7f3a9c";

pub struct Stack {
    pub server: MockMcpServer,
    pub gate: Arc<FakeSessionGate>,
    pub config: Arc<Config>,
    pub registry: Arc<RwLock<Registry>>,
    pub upstream: Arc<HttpUpstream>,
    pub output: OutputGate,
    pub gateway: GatewayServer,
}

pub fn instance(env: &str) -> InstanceId {
    InstanceId::new(EnvId::new(env).unwrap(), McpId::new("kibana-logs").unwrap())
}

pub fn session() -> SessionId {
    SessionId::new("sso-dev").unwrap()
}

/// `dev_kibana_<tool>`.
pub fn exposed(tool: &str) -> String {
    format!("dev_kibana_{tool}")
}

pub async fn stack(generation: Generation) -> Stack {
    stack_with(generation, None).await
}

/// `pipeline`: replaces the minimal pipeline, for tests that look at what reaches it.
pub async fn stack_with(generation: Generation, pipeline: Option<Arc<dyn CallPipeline>>) -> Stack {
    let server = MockMcpServer::builder()
        .generation(generation)
        .tools(catalog_tools("kibana-logs").unwrap())
        .with_test_tools()
        .require_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
        )
        .start()
        .await
        .unwrap();
    let gate = Arc::new(FakeSessionGate::with_bearer(&session(), TOKEN, &[&server.url()]).unwrap());
    let config = Arc::new(Config::embedded().unwrap());
    let filter = Arc::new(OutputFilter::new(Arc::clone(gate.store()), &config.filter).unwrap());
    let embedded = EmbeddedCatalog::builtin(&config).unwrap();
    let mut registry = Registry::new(
        Arc::clone(&config),
        &embedded,
        Box::new(NoCatalogCache),
        Arc::clone(&filter),
    )
    .unwrap();
    let mut settings = UserSettings::default();
    settings.choose_profile(&config.profiles[&ProfileId::new("developpeur").unwrap()]);
    settings.exposed_environments = [EnvId::new("dev").unwrap()].into();
    registry.apply_settings(&settings);
    let registry = Arc::new(RwLock::new(registry));

    let mut target = HttpUpstreamConfig::for_instance(&config, &instance("dev")).unwrap();
    target.url = Url::parse(&server.url()).unwrap();
    target.timeout = Duration::from_secs(10);
    let shared_gate: Arc<dyn SessionGate> = gate.clone();
    let upstream = Arc::new(HttpUpstream::new(target, Arc::clone(&shared_gate)));
    refresh(&registry, upstream.as_ref()).await.unwrap();
    let mut upstreams = Upstreams::default();
    let as_dyn: Arc<dyn Upstream> = upstream.clone();
    upstreams.insert(as_dyn);

    let output = OutputGate::new(filter);
    let pipeline = pipeline.unwrap_or_else(|| {
        Arc::new(MinimalPipeline::new(
            Arc::clone(&config),
            Arc::clone(&registry),
            upstreams,
            shared_gate,
            output.clone(),
        ))
    });
    let gateway = GatewayServer::new(
        Arc::clone(&registry),
        output.clone(),
        pipeline,
        Duration::from_secs(config.gateway.list_ttl_secs),
    );
    Stack {
        server,
        gate,
        config,
        registry,
        upstream,
        output,
        gateway,
    }
}

/// An LLM client of the given generation, connected to `gateway` through an in-memory stream. The gateway side
/// runs until the client goes away.
pub async fn connect<H: ClientHandler>(
    gateway: &GatewayServer,
    handler: H,
    generation: Generation,
) -> (RunningService<RoleClient, H>, JoinHandle<()>) {
    let (client_end, server_end) = tokio::io::duplex(1 << 20);
    let lifecycle = match generation {
        Generation::Legacy => ClientLifecycleMode::Initialize,
        Generation::Modern | Generation::Both => ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    };
    // A 2026-07-28 client is served only from its first request after `server/discover`: the server side cannot be
    // awaited before the client goes on.
    let server = tokio::spawn(gateway.clone().run(server_end));
    let client = handler.serve_with_lifecycle(client_end, lifecycle).await.unwrap();
    (client, server)
}

pub fn params(name: &str, arguments: serde_json::Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name.to_owned());
    params.arguments = arguments.as_object().cloned();
    params
}

pub fn text_of(result: &CallToolResult) -> String {
    serde_json::to_string(&result.content).unwrap()
}
