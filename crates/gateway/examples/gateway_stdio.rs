//! The gateway on stdio, with the embedded configuration and the fake MCP servers of the testkit.
//!
//! Try a real MCP client on any OS before the application exists, for instance:
//! `npx @modelcontextprotocol/inspector cargo run -q -p mcp-proxy-gateway --example gateway_stdio`.
//!
//! Profile `developpeur`, its default environments exposed. Each remote MCP is served by one fake server (on
//! `127.0.0.1`) for all its environments, with the tools of its snapshot; every session holds a demonstration token,
//! allowed only towards the fake servers of the MCPs that use it. Stdout carries MCP: diagnostics go to stderr.

#![allow(clippy::print_stderr)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_core::config::{Config, McpKind, Origin, SessionId};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::secrets::{Credential, OriginGuard, SecretStore};
use mcp_proxy_core::session::{SessionGate, StoreSessionGate};
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_gateway::registry::{EmbeddedCatalog, InstanceId, NoCatalogCache, Registry};
use mcp_proxy_gateway::server::{GatewayServer, MinimalPipeline, OutputGate};
use mcp_proxy_gateway::upstream::{HttpUpstream, HttpUpstreamConfig, Upstream, Upstreams, refresh};
use mcp_proxy_testkit::{MockMcpServer, catalog_tools};
use secrecy::SecretString;
use tokio::sync::RwLock;
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
    let pipeline = Arc::new(MinimalPipeline::new(
        Arc::clone(&config),
        Arc::clone(&registry),
        upstreams,
        gate,
        output.clone(),
    ));
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
