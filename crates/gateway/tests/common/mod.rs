//! A whole gateway on the embedded configuration, with `dev/kibana-logs` served by a fake MCP server, through the
//! call chain.

#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use mcp_proxy_core::config::{Category, Config, EnvId, McpId, ProfileId, SessionId, ToolConfig, ToolName};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::session::SessionGate;
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_gateway::registry::{CatalogCache, EmbeddedCatalog, InstanceId, NoCatalogCache, Registry};
use mcp_proxy_gateway::server::{CallChain, CallChainDeps, CallPipeline, GatewayServer, OutputGate};
use mcp_proxy_gateway::updates::{ListWatcher, ListWatcherDeps, UpdateTracker, WatchTiming};
use mcp_proxy_gateway::upstream::{HttpUpstream, HttpUpstreamConfig, Upstream, Upstreams, refresh};
use mcp_proxy_testkit::{
    FakeConfirmationBroker, FakeSessionGate, Generation, MemoryTraceSink, MockMcpBuilder, MockMcpServer, catalog_tools,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt};
use tokio::sync::{RwLock, watch};
use tokio::task::JoinHandle;
use url::Url;

pub const TOKEN: &str = "jeton-amont-7f3a9c";

/// Test tools of the fake server, declared as reads so that the `developpeur` profile allows them in DEV.
/// `kibana-logs` has no rate limit unless a test sets one.
pub const TEST_TOOLS: [&str; 4] = ["echo_headers", "slow", "fail", "http_fail"];

pub struct Stack {
    pub generation: Generation,
    pub server: MockMcpServer,
    /// The server calls go to, with `upstream_outside_origins`.
    pub elsewhere: Option<MockMcpServer>,
    pub gate: Arc<FakeSessionGate>,
    pub broker: Arc<FakeConfirmationBroker>,
    pub trace: Arc<MemoryTraceSink>,
    pub settings: watch::Sender<UserSettings>,
    pub config: Arc<Config>,
    pub registry: Arc<RwLock<Registry>>,
    pub upstream: Arc<HttpUpstream>,
    pub output: OutputGate,
    pub gateway: GatewayServer,
    /// `None` when the test gave its own pipeline.
    pub chain: Option<Arc<CallChain>>,
}

impl Stack {
    /// Calls that reached the fake server.
    pub fn upstream_calls(&self) -> usize {
        self.server.requests().iter().filter(|r| r.tool.is_some()).count()
    }

    /// Stops the fake server and starts another one at the same address, with `tools` (and the test tools).
    pub async fn restart_server(&mut self, tools: Vec<rmcp::model::Tool>) {
        let addr = self.server.addr();
        self.server.shutdown();
        let mut last = None;
        for _ in 0..50 {
            let builder = MockMcpServer::builder()
                .generation(self.generation)
                .tools(tools.clone())
                .with_test_tools()
                .require_header(
                    HeaderName::from_static("authorization"),
                    HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
                )
                .bind(addr);
            match builder.start().await {
                Ok(server) => {
                    self.server = server;
                    return;
                }
                // The previous server may still hold the port for a moment.
                Err(error) => last = Some(error),
            }
            std::thread::sleep(Duration::from_millis(20));
            tokio::task::yield_now().await;
        }
        panic!("restart on {addr}: {last:?}");
    }

    /// Follows the tool list of `dev/kibana-logs`.
    pub fn watch(&self, timing: WatchTiming) -> ListWatcher {
        let mut upstreams = Upstreams::default();
        let as_dyn: Arc<dyn Upstream> = self.upstream.clone();
        upstreams.insert(as_dyn);
        let gate: Arc<dyn SessionGate> = self.gate.clone();
        ListWatcher::spawn(ListWatcherDeps {
            config: Arc::clone(&self.config),
            registry: Arc::clone(&self.registry),
            upstreams,
            gate,
            settings: self.settings.subscribe(),
            output: self.output.clone(),
            tracker: UpdateTracker::new(&self.config),
            timing,
        })
    }
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

type ConfigEdit = Box<dyn FnOnce(&mut Config)>;
type SettingsEdit = Box<dyn FnOnce(&mut UserSettings)>;
type ServerEdit = Box<dyn Fn(MockMcpBuilder) -> MockMcpBuilder>;

pub struct StackBuilder {
    generation: Generation,
    pipeline: Option<Arc<dyn CallPipeline>>,
    config: Vec<ConfigEdit>,
    settings: Vec<SettingsEdit>,
    /// The upstream talks to this server instead of the one the session's origin points to.
    elsewhere: bool,
    server: Option<ServerEdit>,
    cache: Option<Box<dyn CatalogCache>>,
    /// The registry is filled from the server before the stack is returned.
    initial_refresh: bool,
}

pub fn builder(generation: Generation) -> StackBuilder {
    StackBuilder {
        generation,
        pipeline: None,
        config: Vec::new(),
        settings: Vec::new(),
        elsewhere: false,
        server: None,
        cache: None,
        initial_refresh: true,
    }
}

pub async fn stack(generation: Generation) -> Stack {
    builder(generation).build().await
}

/// `pipeline`: replaces the call chain, for tests that look at what reaches it.
pub async fn stack_with(generation: Generation, pipeline: Option<Arc<dyn CallPipeline>>) -> Stack {
    let mut builder = builder(generation);
    builder.pipeline = pipeline;
    builder.build().await
}

async fn kibana(generation: Generation, edit: Option<&ServerEdit>) -> MockMcpServer {
    let builder = MockMcpServer::builder()
        .generation(generation)
        .tools(catalog_tools("kibana-logs").unwrap())
        .with_test_tools()
        .require_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
        );
    let builder = match edit {
        Some(edit) => edit(builder),
        None => builder,
    };
    builder.start().await.unwrap()
}

impl StackBuilder {
    pub fn config(mut self, edit: impl FnOnce(&mut Config) + 'static) -> Self {
        self.config.push(Box::new(edit));
        self
    }

    pub fn settings(mut self, edit: impl FnOnce(&mut UserSettings) + 'static) -> Self {
        self.settings.push(Box::new(edit));
        self
    }

    /// The upstream of `dev/kibana-logs` points to a second fake server, outside the session's origins.
    pub fn upstream_outside_origins(mut self) -> Self {
        self.elsewhere = true;
        self
    }

    pub fn server(mut self, edit: impl Fn(MockMcpBuilder) -> MockMcpBuilder + 'static) -> Self {
        self.server = Some(Box::new(edit));
        self
    }

    pub fn cache(mut self, cache: impl CatalogCache + 'static) -> Self {
        self.cache = Some(Box::new(cache));
        self
    }

    /// The registry keeps its embedded snapshot and the upstream is not connected.
    pub fn without_initial_refresh(mut self) -> Self {
        self.initial_refresh = false;
        self
    }

    pub async fn build(self) -> Stack {
        let server = kibana(self.generation, self.server.as_ref()).await;
        let gate = Arc::new(FakeSessionGate::with_bearer(&session(), TOKEN, &[&server.url()]).unwrap());
        let mut config = Config::embedded().unwrap();
        let kibana_config = config.mcp.iter_mut().find(|m| m.id.as_str() == "kibana-logs").unwrap();
        for tool in TEST_TOOLS {
            kibana_config.tools.insert(
                ToolName::new(tool).unwrap(),
                ToolConfig {
                    category: Some(Category::Read),
                },
            );
        }
        // No rate limit unless a test sets one: tests make many calls.
        kibana_config.rate_limit = None;
        for edit in self.config {
            edit(&mut config);
        }
        let config = Arc::new(config);
        let filter = Arc::new(OutputFilter::new(Arc::clone(gate.store()), &config.filter).unwrap());
        let embedded = EmbeddedCatalog::builtin(&config).unwrap();
        let mut registry = Registry::new(
            Arc::clone(&config),
            &embedded,
            self.cache.unwrap_or_else(|| Box::new(NoCatalogCache)),
            Arc::clone(&filter),
        )
        .unwrap();
        let mut settings = UserSettings::default();
        settings.choose_profile(&config.profiles[&ProfileId::new("developpeur").unwrap()]);
        settings.exposed_environments = [EnvId::new("dev").unwrap()].into();
        for edit in self.settings {
            edit(&mut settings);
        }
        registry.apply_settings(&settings);
        let registry = Arc::new(RwLock::new(registry));

        let target_server = if self.elsewhere {
            Some(kibana(self.generation, self.server.as_ref()).await)
        } else {
            None
        };
        let mut target = HttpUpstreamConfig::for_instance(&config, &instance("dev")).unwrap();
        target.url = Url::parse(&target_server.as_ref().unwrap_or(&server).url()).unwrap();
        target.timeout = Duration::from_secs(10);
        let shared_gate: Arc<dyn SessionGate> = gate.clone();
        let upstream = Arc::new(HttpUpstream::new(target, Arc::clone(&shared_gate)));
        if target_server.is_some() {
            // The list comes from the server of the session's origin; calls go to the other one.
            let lister = HttpUpstream::new(
                {
                    let mut t = HttpUpstreamConfig::for_instance(&config, &instance("dev")).unwrap();
                    t.url = Url::parse(&server.url()).unwrap();
                    t
                },
                Arc::clone(&shared_gate),
            );
            refresh(&registry, &lister).await.unwrap();
        } else if self.initial_refresh {
            refresh(&registry, upstream.as_ref()).await.unwrap();
        }
        let mut upstreams = Upstreams::default();
        let as_dyn: Arc<dyn Upstream> = upstream.clone();
        upstreams.insert(as_dyn);

        let output = OutputGate::new(filter);
        let broker = Arc::new(FakeConfirmationBroker::default());
        let trace = Arc::new(MemoryTraceSink::default());
        let (settings_sender, settings_receiver) = watch::channel(settings);
        let chain = match self.pipeline {
            Some(_) => None,
            None => Some(Arc::new(
                CallChain::new(CallChainDeps {
                    config: Arc::clone(&config),
                    registry: Arc::clone(&registry),
                    settings: settings_receiver,
                    upstreams,
                    gate: shared_gate,
                    broker: broker.clone(),
                    output: output.clone(),
                    trace: trace.clone(),
                })
                .unwrap(),
            )),
        };
        let pipeline: Arc<dyn CallPipeline> = match (self.pipeline, &chain) {
            (Some(pipeline), _) => pipeline,
            (None, Some(chain)) => chain.clone(),
            (None, None) => unreachable!(),
        };
        let gateway = GatewayServer::new(
            Arc::clone(&registry),
            output.clone(),
            pipeline,
            Duration::from_secs(config.gateway.list_ttl_secs),
        );
        Stack {
            generation: self.generation,
            server,
            elsewhere: target_server,
            gate,
            broker,
            trace,
            settings: settings_sender,
            config,
            registry,
            upstream,
            output,
            gateway,
            chain,
        }
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
