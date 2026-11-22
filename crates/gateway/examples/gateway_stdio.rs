//! The gateway on stdio, with the embedded configuration and the fake MCP servers of the testkit.
//!
//! Try a real MCP client on any OS before the application exists, for instance:
//! `npx @modelcontextprotocol/inspector cargo run -q -p mcp-proxy-gateway --example gateway_stdio`.
//!
//! Profile `developpeur`, its default environments exposed. Each remote MCP is served by one fake server (on
//! `127.0.0.1`) for all its environments, with the tools of its snapshot; every session holds a demonstration token,
//! allowed only towards the fake servers of the MCPs that use it. Every call goes through the whole call chain; a
//! confirmation outside the chat is always refused (no notification here). Tool lists are followed by the
//! `ListWatcher` (spec 008). Every call goes to the journal of spec 009, whose log file, with every `tracing` event,
//! is written in `MCP_PROXY_LOG_DIR` (default: `mcp-proxy-gateway-stdio-logs` in the temporary directory). Stdout
//! carries MCP: diagnostics go to stderr.

#![allow(clippy::print_stderr)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_core::config::{Config, McpKind, Origin, SessionId};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::secrets::{Credential, OriginGuard, SecretStore};
use mcp_proxy_core::session::{SessionGate, StoreSessionGate};
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_core::trace::{
    Journal, JournalConfig, LogFileOptions, SystemWallClock, install_global, install_panic_hook, open_log,
};
use mcp_proxy_gateway::journal::follow_tool_changes;
use mcp_proxy_gateway::registry::{EmbeddedCatalog, InstanceId, NoCatalogCache, Registry};
use mcp_proxy_gateway::server::{CallChain, CallChainDeps, GatewayServer, OutputGate};
use mcp_proxy_gateway::updates::{ListWatcher, ListWatcherDeps, UpdateTracker, WatchTiming};
use mcp_proxy_gateway::upstream::{HttpUpstream, HttpUpstreamConfig, Upstream, Upstreams, refresh};
use mcp_proxy_testkit::{FakeConfirmationBroker, MockMcpServer, catalog_tools};
use secrecy::SecretString;
use tokio::sync::{RwLock, watch};
use url::Url;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let config = Arc::new(Config::embedded()?);

    let mut servers = BTreeMap::new();
    for mcp in config.mcp.iter().filter(|m| m.kind == McpKind::Remote) {
        let server = MockMcpServer::builder()
            .tools(catalog_tools(mcp.id.as_str()).unwrap_or_default())
            .start()
            .await?;
        eprintln!("faux serveur {} : {}", mcp.id, server.url());
        servers.insert(mcp.id.clone(), server);
    }

    let mut origins: BTreeMap<SessionId, BTreeSet<Origin>> = BTreeMap::new();
    let mut targets = Vec::new();
    for (mcp_id, server) in &servers {
        let url = Url::parse(&server.url())?;
        let Some(mcp) = config.mcp(mcp_id) else { continue };
        for env in mcp.instances.keys() {
            let instance = InstanceId::new(env.clone(), mcp_id.clone());
            let Some(mut target) = HttpUpstreamConfig::for_instance(&config, &instance) else {
                continue;
            };
            target.url = url.clone();
            if let (Some(session), Some(origin)) = (&target.session, Origin::of(&url)) {
                origins.entry(session.clone()).or_default().insert(origin);
            }
            targets.push(target);
        }
    }

    let store = Arc::new(SecretStore::for_platform()?);
    for session in &config.sessions {
        let token = SecretString::from(format!("jeton-de-demonstration-{}", session.id));
        store.put(&session.id, Credential::Bearer { token })?;
    }
    let guard = Arc::new(OriginGuard::new(origins));
    let gate: Arc<dyn SessionGate> = Arc::new(StoreSessionGate::new(Arc::clone(&store), guard));
    let filter = Arc::new(OutputFilter::new(Arc::clone(&store), &config.filter)?);

    let log_dir = std::env::var_os("MCP_PROXY_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("mcp-proxy-gateway-stdio-logs"));
    let (log, _log_guard) = open_log(
        LogFileOptions::new(&log_dir, &config.trace),
        Arc::new(SystemWallClock),
        Arc::clone(&filter),
    )?;
    install_global(log.clone())?;
    let journal = Journal::new(
        JournalConfig::from_config(&config),
        Arc::clone(&filter),
        Arc::new(SystemWallClock),
        Some(log),
    );
    install_panic_hook(journal.clone());
    eprintln!("journal : {}", log_dir.display());

    let embedded = EmbeddedCatalog::builtin(&config)?;
    let mut registry = Registry::new(
        Arc::clone(&config),
        &embedded,
        Box::new(NoCatalogCache),
        Arc::clone(&filter),
    )?;
    let mut settings = UserSettings::default();
    if let Some(profile) = config.profiles.values().find(|p| p.id.as_str() == "developpeur") {
        settings.choose_profile(profile);
    }
    registry.apply_settings(&settings);
    let _tool_changes = follow_tool_changes(registry.subscribe(), journal.clone());
    let registry = Arc::new(RwLock::new(registry));

    let mut upstreams = Upstreams::default();
    for target in targets {
        let exposed = registry.read().await.exposure().exposes(&target.instance);
        let upstream: Arc<dyn Upstream> = Arc::new(HttpUpstream::new(target, Arc::clone(&gate)));
        if exposed && let Err(error) = refresh(&registry, upstream.as_ref()).await {
            eprintln!("{} : liste non chargée : {error}", upstream.instance());
        }
        upstreams.insert(upstream);
    }
    eprintln!("{} outils exposés", registry.read().await.exposed_count());

    let output = OutputGate::new(filter);
    let (_settings_sender, settings) = watch::channel(settings);
    // Tool lists follow the servers: notifications, subscriptions, periodic reloads, reconnections.
    let _watcher = ListWatcher::spawn(ListWatcherDeps {
        config: Arc::clone(&config),
        registry: Arc::clone(&registry),
        upstreams: upstreams.clone(),
        gate: Arc::clone(&gate),
        settings: settings.clone(),
        output: output.clone(),
        tracker: UpdateTracker::new(&config),
        timing: WatchTiming::from_config(&config),
    });
    let pipeline = Arc::new(CallChain::new(CallChainDeps {
        config: Arc::clone(&config),
        registry: Arc::clone(&registry),
        settings,
        upstreams,
        gate,
        broker: Arc::new(FakeConfirmationBroker::default()),
        output: output.clone(),
        trace: Arc::new(journal),
    })?);
    let gateway = GatewayServer::new(
        registry,
        output,
        pipeline,
        Duration::from_secs(config.gateway.list_ttl_secs),
    );
    gateway
        .run(tokio::io::join(tokio::io::stdin(), tokio::io::stdout()))
        .await;
    Ok(())
}
