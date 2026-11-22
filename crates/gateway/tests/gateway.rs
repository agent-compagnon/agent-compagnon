//! Client side, end to end: LLM client → `GatewayServer` → `HttpUpstream` → fake MCP server.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{TOKEN, connect, exposed, params, session, stack, stack_with, text_of};
use mcp_proxy_core::filter::{FIELD_MARKER, HEADER_MARKER, SECRET_MARKER};
use mcp_proxy_core::secrets::Credential;
use mcp_proxy_core::session::SessionError;
use mcp_proxy_gateway::server::{CallPipeline, CallRequest, ClientIdentity, Filtered, OutputGate};
use mcp_proxy_gateway::upstream::refresh;
use mcp_proxy_testkit::{Generation, catalog_tools};
use rmcp::ClientHandler;
use rmcp::model::{
    CallToolResult, ClientCapabilities, ClientConfig, ContentBlock, Implementation, ProgressNotificationParam,
    ProtocolVersion, ServerNotification, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, RoleClient};
use secrecy::SecretString;
use serde_json::json;
use tokio::sync::Notify;

/// A client that records what the gateway sends it.
#[derive(Clone, Default)]
struct Recorder {
    list_changed: Arc<Notify>,
    progress: Arc<Mutex<Vec<ProgressNotificationParam>>>,
    elicitation: bool,
}

impl ClientHandler for Recorder {
    fn get_info(&self) -> ClientConfig {
        let mut capabilities = ClientCapabilities::default();
        if self.elicitation {
            capabilities = ClientCapabilities::builder().enable_elicitation().build();
        }
        ClientConfig::new(capabilities, Implementation::new("client-de-test", "4.2.0"))
    }

    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.list_changed.notify_one();
    }

    async fn on_progress(&self, params: ProgressNotificationParam, _: NotificationContext<RoleClient>) {
        self.progress.lock().unwrap().push(params);
    }
}

const GENERATIONS: [Generation; 2] = [Generation::Legacy, Generation::Modern];

// Exigence : EX-006-01
#[tokio::test(flavor = "multi_thread")]
async fn matrix_of_generations_lists_then_calls() {
    for server_generation in GENERATIONS {
        let stack = stack(server_generation).await;
        for client_generation in GENERATIONS {
            let case = format!("client {client_generation:?}, serveur {server_generation:?}");
            let (client, _connection) = connect(&stack.gateway, Recorder::default(), client_generation).await;
            let negotiated = client.peer().peer_info().unwrap().protocol_version.clone();
            match client_generation {
                Generation::Legacy => assert!(negotiated < ProtocolVersion::V_2026_07_28, "{case}: {negotiated:?}"),
                _ => assert_eq!(negotiated, ProtocolVersion::V_2026_07_28, "{case}"),
            }

            let tools = client.peer().list_all_tools().await.unwrap();
            let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
            assert!(names.contains(&exposed("search_logs").as_str()), "{case}: {names:?}");
            assert!(names.iter().all(|n| n.starts_with("dev_")), "{case}: {names:?}");

            let calls_before = stack.server.requests().iter().filter(|r| r.tool.is_some()).count();
            let result = client
                .call_tool(params(
                    &exposed("search_logs"),
                    json!({ "index": "app-*", "query": "error" }),
                ))
                .await
                .unwrap();
            assert_ne!(result.is_error, Some(true), "{case}: {}", text_of(&result));
            assert!(text_of(&result).contains("search_logs"), "{case}");
            let calls: Vec<_> = stack
                .server
                .requests()
                .into_iter()
                .filter(|r| r.tool.is_some())
                .collect();
            assert_eq!(calls.len(), calls_before + 1, "{case}");
            assert_eq!(calls.last().unwrap().tool.as_deref(), Some("search_logs"), "{case}");
            client.cancel().await.unwrap();
        }
        let methods = stack.server.mcp_methods();
        match server_generation {
            Generation::Legacy => assert!(methods.contains(&"initialize".to_owned()), "{methods:?}"),
            _ => assert!(!methods.contains(&"initialize".to_owned()), "{methods:?}"),
        }
    }
}

// Exigence : EX-006-04
#[tokio::test(flavor = "multi_thread")]
async fn unknown_or_unexposed_tools_are_neither_listed_nor_called() {
    let stack = stack(Generation::Both).await;
    // The prod instance exists in the registry (embedded snapshot) but prod is not exposed.
    assert!(stack.registry.read().await.lookup("prod_kibana_search_logs").is_some());
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), generation).await;
        let tools = client.peer().list_all_tools().await.unwrap();
        assert!(tools.iter().all(|t| t.name != "prod_kibana_search_logs"));
        for name in ["prod_kibana_search_logs", "dev_kibana_does_not_exist", "search_logs"] {
            let result = client.call_tool(params(name, json!({}))).await.unwrap();
            assert_eq!(result.is_error, Some(true), "{name}");
            assert!(
                text_of(&result).contains("inconnu ou non exposé"),
                "{name}: {}",
                text_of(&result)
            );
        }
    }
    assert!(stack.server.requests().iter().all(|r| r.tool.is_none()));
}

// Exigence : EX-006-05
// Exigence : EX-006-02
#[tokio::test(flavor = "multi_thread")]
async fn a_token_echoed_by_the_server_is_masked() {
    let stack = stack(Generation::Both).await;
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), generation).await;
        let result = client
            .call_tool(params(&exposed("echo_headers"), json!({})))
            .await
            .unwrap();
        let text = text_of(&result);
        assert!(!text.contains(TOKEN), "{generation:?}: {text}");
        assert!(
            [SECRET_MARKER, HEADER_MARKER, FIELD_MARKER]
                .iter()
                .any(|m| text.contains(m)),
            "{text}"
        );
    }
    // The server did receive it.
    let last = stack
        .server
        .requests()
        .into_iter()
        .rev()
        .find(|r| r.tool.is_some())
        .unwrap();
    assert_eq!(last.headers.get("authorization").unwrap(), &format!("Bearer {TOKEN}"));
}

// Exigence : EX-006-06
#[tokio::test(flavor = "multi_thread")]
async fn upstream_error_bodies_quoting_the_token_are_masked() {
    let stack = stack(Generation::Both).await;
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), generation).await;
        for tool in ["fail", "http_fail"] {
            let result = client.call_tool(params(&exposed(tool), json!({}))).await.unwrap();
            let text = text_of(&result);
            assert_eq!(result.is_error, Some(true), "{tool}");
            assert!(text.contains("MCP Proxy"), "{tool}: {text}");
            assert!(!text.contains(TOKEN), "{generation:?} {tool}: {text}");
        }
    }
}

// Exigence : EX-006-09
#[tokio::test(flavor = "multi_thread")]
async fn lists_are_filtered_again_when_sent() {
    let stack = stack(Generation::Both).await;
    let word = "Searches";
    let before = stack
        .registry
        .read()
        .await
        .resolve(&exposed("search_logs"))
        .unwrap()
        .description
        .clone();
    assert!(before.contains(word));
    // A secret learnt after the registry was filled (a later connection).
    let other = mcp_proxy_core::config::SessionId::new("sso-corp").unwrap();
    stack
        .gate
        .store()
        .put(
            &other,
            Credential::Bearer {
                token: SecretString::from(word.to_owned()),
            },
        )
        .unwrap();
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), generation).await;
        let tools = client.peer().list_all_tools().await.unwrap();
        let search = tools.iter().find(|t| t.name == exposed("search_logs")).unwrap();
        let description = search.description.as_deref().unwrap();
        assert!(!description.contains(word), "{description}");
        assert!(description.contains(SECRET_MARKER), "{description}");
    }
}

// Exigence : EX-006-10
#[tokio::test(flavor = "multi_thread")]
async fn registry_changes_reach_clients_of_both_generations() {
    let stack = stack(Generation::Both).await;
    let legacy = Recorder::default();
    let (_legacy_client, _c1) = connect(&stack.gateway, legacy.clone(), Generation::Legacy).await;
    let (modern_client, _c2) = connect(&stack.gateway, Recorder::default(), Generation::Modern).await;
    let mut subscription = modern_client
        .peer()
        .listen(SubscriptionFilter::builder().tools_list_changed().build())
        .await
        .unwrap();

    let mut tools = catalog_tools("kibana-logs").unwrap();
    tools.truncate(2);
    stack.server.set_tools(tools, false).await;
    refresh(&stack.registry, stack.upstream.as_ref()).await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), legacy.list_changed.notified())
        .await
        .expect("legacy client not told");
    let notification = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("2026-07-28 client not told")
        .unwrap();
    assert!(matches!(
        notification,
        Some(ServerNotification::ToolListChangedNotification(_))
    ));
}

// Exigence : EX-006-11
#[tokio::test(flavor = "multi_thread")]
async fn only_tools_are_announced() {
    let stack = stack(Generation::Both).await;
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), generation).await;
        let capabilities = client.peer().peer_info().unwrap().capabilities.clone();
        assert!(capabilities.tools.is_some());
        assert!(capabilities.resources.is_none());
        assert!(capabilities.prompts.is_none());
        assert!(capabilities.completions.is_none());
        assert!(client.peer().list_resources(None).await.is_err(), "{generation:?}");
        assert!(client.peer().list_prompts(None).await.is_err(), "{generation:?}");
    }
}

// Exigence : EX-006-12
#[tokio::test(flavor = "multi_thread")]
async fn progress_is_relayed_with_filtered_messages() {
    let stack = stack(Generation::Both).await;
    for generation in GENERATIONS {
        let recorder = Recorder::default();
        let (client, _connection) = connect(&stack.gateway, recorder.clone(), generation).await;
        let result = client
            .call_tool(params(&exposed("slow"), json!({ "millis": 300, "echo": true })))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
        let progress = recorder.progress.lock().unwrap().clone();
        assert!(!progress.is_empty(), "{generation:?}: no progress");
        for update in &progress {
            let message = update.message.as_deref().unwrap();
            assert!(message.starts_with("étape"), "{message}");
            assert!(!message.contains(TOKEN), "{generation:?}: {message}");
        }
    }
}

/// Records what reaches the pipeline, and answers with the token itself to check the output gate.
struct Probe {
    output: Mutex<Option<OutputGate>>,
    seen: Mutex<Vec<ClientIdentity>>,
}

#[async_trait]
impl CallPipeline for Probe {
    async fn call(&self, request: CallRequest) -> Filtered {
        self.seen.lock().unwrap().push(request.client);
        let output = self.output.lock().unwrap().clone().unwrap();
        output.result(&CallToolResult::success(vec![ContentBlock::text(format!(
            "Bearer {TOKEN}"
        ))]))
    }
}

// Exigence : EX-006-13
#[tokio::test(flavor = "multi_thread")]
async fn client_identity_is_read_from_each_request() {
    let probe = Arc::new(Probe {
        output: Mutex::new(None),
        seen: Mutex::new(Vec::new()),
    });
    let as_pipeline: Arc<dyn CallPipeline> = probe.clone();
    let stack = stack_with(Generation::Both, Some(as_pipeline)).await;
    *probe.output.lock().unwrap() = Some(stack.output.clone());
    for (generation, elicitation) in [(Generation::Legacy, true), (Generation::Modern, false)] {
        let recorder = Recorder {
            elicitation,
            ..Recorder::default()
        };
        let (client, _connection) = connect(&stack.gateway, recorder, generation).await;
        let result = client
            .call_tool(params(&exposed("search_logs"), json!({})))
            .await
            .unwrap();
        assert!(!text_of(&result).contains(TOKEN));
        let identity = probe.seen.lock().unwrap().pop().unwrap();
        assert_eq!(identity.name, "client-de-test");
        assert_eq!(identity.version, "4.2.0");
        assert_eq!(identity.supports_elicitation, elicitation, "{generation:?}");
        let version = identity.protocol_version.unwrap();
        match generation {
            Generation::Legacy => assert!(version.as_str() < "2026-07-28", "{version}"),
            _ => assert_eq!(version, "2026-07-28"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_errors_are_tool_errors_in_french() {
    let stack = stack(Generation::Both).await;
    stack.gate.script([Err(SessionError::LoginPending(session()))]);
    let (client, _connection) = connect(&stack.gateway, Recorder::default(), Generation::Legacy).await;
    let result = client
        .call_tool(params(&exposed("search_logs"), json!({})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("MCP Proxy : connexion à « SSO DEV » requise"));
    assert!(stack.server.requests().iter().all(|r| r.tool.is_none()));
}
