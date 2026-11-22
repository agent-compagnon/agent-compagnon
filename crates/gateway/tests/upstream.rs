//! Server side: `HttpUpstream` against the fake MCP servers of the testkit.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use mcp_proxy_core::config::{EnvId, McpId, SessionId, ToolName};
use mcp_proxy_core::secrets::SecretError;
use mcp_proxy_core::session::{SessionError, SessionGate};
use mcp_proxy_gateway::registry::InstanceId;
use mcp_proxy_gateway::upstream::{HttpUpstream, HttpUpstreamConfig, Upstream, UpstreamCall, UpstreamError};
use mcp_proxy_testkit::{FakeSessionGate, Generation, MockMcpServer, catalog_tools};
use serde_json::json;
use url::Url;

const TOKEN: &str = "jeton-amont-7f3a9c";

fn sid() -> SessionId {
    SessionId::new("sso-dev").unwrap()
}

fn instance() -> InstanceId {
    InstanceId::new(EnvId::new("dev").unwrap(), McpId::new("kibana-logs").unwrap())
}

fn bearer() -> (HeaderName, HeaderValue) {
    (
        HeaderName::from_static("authorization"),
        HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
    )
}

async fn kibana(generation: Generation) -> MockMcpServer {
    let (name, value) = bearer();
    MockMcpServer::builder()
        .generation(generation)
        .tools(catalog_tools("kibana-logs").unwrap())
        .with_test_tools()
        .require_header(name, value)
        .start()
        .await
        .unwrap()
}

fn gate_for(origins: &[&str]) -> Arc<FakeSessionGate> {
    Arc::new(FakeSessionGate::with_bearer(&sid(), TOKEN, origins).unwrap())
}

fn upstream(url: &str, gate: &Arc<FakeSessionGate>) -> HttpUpstream {
    let config = HttpUpstreamConfig::new(
        instance(),
        Url::parse(url).unwrap(),
        Some(sid()),
        Duration::from_secs(10),
    );
    let gate: Arc<dyn SessionGate> = gate.clone();
    HttpUpstream::new(config, gate)
}

fn call(tool: &str, arguments: serde_json::Value) -> UpstreamCall {
    UpstreamCall {
        tool: ToolName::new(tool).unwrap(),
        arguments: arguments.as_object().cloned(),
        progress: None,
    }
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    serde_json::to_string(&result.content).unwrap()
}

// Exigence : EX-006-01
// Exigence : EX-006-02
#[tokio::test(flavor = "multi_thread")]
async fn both_generations_list_then_call_with_the_header_injected() {
    for generation in [Generation::Legacy, Generation::Modern, Generation::Both] {
        let server = kibana(generation).await;
        let gate = gate_for(&[&server.url()]);
        let upstream = upstream(&server.url(), &gate);

        let tools = upstream.list_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"search_logs"), "{generation:?}: {names:?}");
        let result = upstream
            .call_tool(call("search_logs", json!({ "index": "app-*", "query": "error" })))
            .await
            .unwrap();
        assert!(text_of(&result).contains("search_logs"), "{generation:?}");

        let methods = server.mcp_methods();
        match generation {
            Generation::Legacy => {
                assert!(methods.contains(&"initialize".to_owned()), "{methods:?}");
            }
            Generation::Modern => {
                assert!(methods.contains(&"server/discover".to_owned()), "{methods:?}");
                assert!(!methods.contains(&"initialize".to_owned()), "{methods:?}");
            }
            Generation::Both => {
                assert!(methods.contains(&"server/discover".to_owned()), "{methods:?}");
            }
        }
        let (name, value) = bearer();
        for request in server.requests() {
            assert_eq!(
                request.headers.get(&name),
                Some(&value),
                "{generation:?} {:?}",
                request.mcp_method
            );
        }
        assert!(gate.rejected().is_empty());
    }
}

// Exigence : EX-006-02
#[tokio::test(flavor = "multi_thread")]
async fn no_request_leaves_towards_an_origin_outside_the_session() {
    let server = kibana(Generation::Both).await;
    let gate = gate_for(&["https://kibana-mcp.dev.example.internal/"]);
    let upstream = upstream(&server.url(), &gate);

    let error = upstream.list_tools().await.unwrap_err();
    assert!(
        matches!(
            error,
            UpstreamError::Session(SessionError::Injection(SecretError::OriginNotAllowed { .. }))
        ),
        "{error:?}"
    );
    assert!(server.requests().is_empty());
    assert!(!error.to_string().contains(TOKEN));
}

// Exigence : EX-006-03
#[tokio::test(flavor = "multi_thread")]
async fn a_401_with_or_without_challenge_rejects_the_session() {
    for challenge in [true, false] {
        let server = MockMcpServer::builder()
            .tools(catalog_tools("kibana-logs").unwrap())
            .require_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_static("Bearer autre"),
            )
            .www_authenticate(challenge)
            .start()
            .await
            .unwrap();
        let gate = gate_for(&[&server.url()]);
        let upstream = upstream(&server.url(), &gate);

        let error = upstream.list_tools().await.unwrap_err();
        assert_eq!(error, UpstreamError::AuthRejected, "challenge: {challenge}");
        assert_eq!(gate.rejected(), vec![sid()]);
        assert!(!gate.is_active(&sid()));
        // The secret is gone: the next list does not even try.
        tokio::time::sleep(HttpUpstreamConfig::DEFAULT_INITIAL_BACKOFF).await;
        let before = server.requests().len();
        assert_eq!(
            upstream.list_tools().await.unwrap_err(),
            UpstreamError::SessionRequired(sid())
        );
        assert_eq!(server.requests().len(), before);
    }
}

// Exigence : EX-006-03
#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_to_another_origin_is_not_followed() {
    let elsewhere = MockMcpServer::builder()
        .tools(catalog_tools("kibana-logs").unwrap())
        .start()
        .await
        .unwrap();
    let server = MockMcpServer::builder()
        .redirect_to(elsewhere.url())
        .start()
        .await
        .unwrap();
    let gate = gate_for(&[&server.url(), &elsewhere.url()]);
    let upstream = upstream(&server.redirect_url(), &gate);

    let error = upstream.list_tools().await.unwrap_err();
    assert!(matches!(error, UpstreamError::Connect(_)), "{error:?}");
    assert!(elsewhere.requests().is_empty());
    assert!(server.requests().iter().all(|r| r.path == "/redirect"));
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_without_session_is_refused_unless_allowed() {
    let server = MockMcpServer::builder()
        .tools(catalog_tools("kibana-logs").unwrap())
        .start()
        .await
        .unwrap();
    let gate = gate_for(&[&server.url()]);
    gate.mark_rejected(&sid());
    let mut config = HttpUpstreamConfig::new(
        instance(),
        Url::parse(&server.url()).unwrap(),
        Some(sid()),
        Duration::from_secs(10),
    );
    let shared: Arc<dyn SessionGate> = gate.clone();
    let refused = HttpUpstream::new(config.clone(), Arc::clone(&shared));
    assert_eq!(
        refused.list_tools().await.unwrap_err(),
        UpstreamError::SessionRequired(sid())
    );
    assert!(server.requests().is_empty());

    config.list_requires_auth = false;
    let anonymous = HttpUpstream::new(config, shared);
    assert!(!anonymous.list_tools().await.unwrap().is_empty());
    assert!(
        server
            .requests()
            .iter()
            .all(|r| r.headers.get("authorization").is_none())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_are_bounded_by_the_upstream_timeout_and_relay_progress() {
    for generation in [Generation::Legacy, Generation::Modern] {
        let server = kibana(generation).await;
        let gate = gate_for(&[&server.url()]);
        let mut config = HttpUpstreamConfig::new(
            instance(),
            Url::parse(&server.url()).unwrap(),
            Some(sid()),
            Duration::from_secs(10),
        );
        let shared: Arc<dyn SessionGate> = gate.clone();
        let upstream = HttpUpstream::new(config.clone(), Arc::clone(&shared));

        let (sink, mut progress) = tokio::sync::mpsc::channel(8);
        let mut slow = call("slow", json!({ "millis": 300 }));
        slow.progress = Some(sink);
        upstream.call_tool(slow).await.unwrap();
        let first = progress.recv().await.unwrap();
        assert_eq!(first.total, Some(3.0), "{generation:?}");

        config.timeout = Duration::from_millis(400);
        let short = HttpUpstream::new(config, shared);
        let error = short
            .call_tool(call("slow", json!({ "millis": 3000 })))
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Timeout(_)), "{generation:?}: {error:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_changes_are_reported_in_both_generations() {
    for generation in [Generation::Legacy, Generation::Modern] {
        let server = kibana(generation).await;
        let gate = gate_for(&[&server.url()]);
        let upstream = upstream(&server.url(), &gate);
        let mut changes = upstream.changes();
        upstream.list_tools().await.unwrap();
        // The legacy notification needs the GET stream, the 2026-07-28 one the listen subscription.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let mut tools = catalog_tools("kibana-logs").unwrap();
        tools.truncate(1);
        server.set_tools(tools, true).await;
        tokio::time::timeout(Duration::from_secs(5), changes.recv())
            .await
            .unwrap_or_else(|_| panic!("{generation:?}: no change reported"))
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnection_waits_for_a_growing_delay() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    drop(listener);
    let gate = gate_for(&[&url]);
    let mut config = HttpUpstreamConfig::new(
        instance(),
        Url::parse(&url).unwrap(),
        Some(sid()),
        Duration::from_secs(5),
    );
    config.initial_backoff = Duration::from_millis(200);
    let shared: Arc<dyn SessionGate> = gate.clone();
    let upstream = HttpUpstream::new(config, shared);

    assert!(matches!(
        upstream.list_tools().await.unwrap_err(),
        UpstreamError::Connect(_)
    ));
    let UpstreamError::Unavailable { retry_in } = upstream.list_tools().await.unwrap_err() else {
        panic!("expected a backoff");
    };
    assert!(retry_in <= Duration::from_millis(200));
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(matches!(
        upstream.list_tools().await.unwrap_err(),
        UpstreamError::Connect(_)
    ));
    let UpstreamError::Unavailable { retry_in } = upstream.list_tools().await.unwrap_err() else {
        panic!("expected a backoff");
    };
    assert!(retry_in > Duration::from_millis(200), "{retry_in:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn server_errors_are_reported_with_their_text() {
    let server = kibana(Generation::Both).await;
    let gate = gate_for(&[&server.url()]);
    let upstream = upstream(&server.url(), &gate);

    let error = upstream.call_tool(call("fail", json!({}))).await.unwrap_err();
    assert!(
        matches!(&error, UpstreamError::Remote { message, .. } if message.contains("échec simulé")),
        "{error:?}"
    );
    let error = upstream.call_tool(call("http_fail", json!({}))).await.unwrap_err();
    assert!(
        matches!(&error, UpstreamError::Protocol(text) if text.contains("refus du serveur amont")),
        "{error:?}"
    );
}
