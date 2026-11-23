//! The resilient bridge (spec 026) between a real MCP client and the real gateway, in both generations of the
//! protocol: a restart of the application is a new gateway, without any secret, behind a new connection.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use common::{COOKIE, Stack, builder, exposed, params, text_of};
use mcp_proxy_gateway::bridge::{
    AppLauncher, ConnectFailure, Connector, PeerProof, ProtectFailure, RelaunchTarget, RelayDeps, ResilientRelay, Role,
    prove,
};
use mcp_proxy_testkit::Generation;
use rmcp::model::{CallToolResult, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::json;
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

struct SameUser;

impl PeerProof for SameUser {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure> {
        Ok([b"U1", data].concat())
    }

    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure> {
        data.strip_prefix(b"U1")
            .map(|rest| Zeroizing::new(rest.to_vec()))
            .ok_or(ProtectFailure(-1))
    }
}

#[derive(Clone, Default)]
struct Instances(Arc<Mutex<VecDeque<DuplexStream>>>);

impl Connector for Instances {
    type Pipe = DuplexStream;

    async fn connect(&mut self) -> Result<DuplexStream, ConnectFailure> {
        self.0.lock().unwrap().pop_front().ok_or(ConnectFailure::Absent)
    }
}

struct NoLaunch;

impl AppLauncher for NoLaunch {
    fn target(&self) -> RelaunchTarget {
        RelaunchTarget::StoppedOnPurpose("mcp-proxy.exe".into())
    }

    fn launch(&self, _exe: &Path) -> Result<(), String> {
        panic!("the application restarts by itself in these tests");
    }
}

/// The application's side of the next connection: the proof, then the gateway of `stack` until it is stopped.
fn instance(instances: &Instances, stack: &Stack) -> JoinHandle<()> {
    let (bridge_end, mut app_end) = tokio::io::duplex(1 << 20);
    instances.0.lock().unwrap().push_back(bridge_end);
    let gateway = stack.gateway.clone();
    tokio::spawn(async move {
        prove(Arc::new(SameUser), Role::Server, &mut app_end).await.unwrap();
        gateway.run(app_end).await;
    })
}

async fn call_logging_in(client: &RunningService<RoleClient, ()>, stack: &Stack) -> CallToolResult {
    let pending = {
        let client = client.peer().clone();
        tokio::spawn(async move {
            client
                .call_tool(params(&exposed("search_logs"), json!({ "query": "error" })))
                .await
                .unwrap()
        })
    };
    stack.on_demand.as_ref().unwrap().log_in(0, COOKIE).await;
    pending.await.unwrap()
}

// Exigence : EX-026-02
// Exigence : EX-026-03
// Exigence : EX-026-15
#[tokio::test]
async fn a_call_after_a_restart_logs_in_again_then_is_served() {
    for generation in [Generation::Legacy, Generation::Modern] {
        let first = builder(Generation::Legacy).on_demand().build().await;
        let instances = Instances::default();
        let running = instance(&instances, &first);
        let (said, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();

        let (input, client_writer) = tokio::io::duplex(1 << 20);
        let (client_reader, output) = tokio::io::duplex(1 << 20);
        let relay = ResilientRelay::new(RelayDeps {
            proof: Arc::new(SameUser),
            input,
            output,
            connector: instances.clone(),
            launcher: Arc::new(NoLaunch),
            diagnostics: Arc::new(move |line: &str| {
                let _ = said.send(line.to_owned());
            }),
        });
        let bridge = tokio::spawn(relay.run());
        let lifecycle = match generation {
            Generation::Legacy => ClientLifecycleMode::Initialize,
            _ => ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        };
        let client = ().serve_with_lifecycle((client_reader, client_writer), lifecycle).await.unwrap();

        let result = call_logging_in(&client, &first).await;
        assert_ne!(result.is_error, Some(true), "{generation:?}: {}", text_of(&result));
        assert_eq!(first.upstream_calls(), 1);

        // The application restarts: its secrets went with it.
        let second = builder(Generation::Legacy).on_demand().build().await;
        let restarted = instance(&instances, &second);
        running.abort();
        while !heard.recv().await.unwrap().contains("reconnecté") {}
        let result = call_logging_in(&client, &second).await;
        assert_ne!(result.is_error, Some(true), "{generation:?}: {}", text_of(&result));
        assert_eq!(
            second.on_demand.as_ref().unwrap().ui.window_count(),
            1,
            "{generation:?}: a new login"
        );
        assert_eq!(second.upstream_calls(), 1, "{generation:?}");
        let tools = client.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|t| t.name == exposed("search_logs")), "{generation:?}");

        client.cancel().await.unwrap();
        assert!(bridge.await.unwrap().is_ok(), "{generation:?}");
        restarted.abort();
    }
}
