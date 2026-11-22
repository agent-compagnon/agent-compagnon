//! The gateway with the real `OnDemandAuthenticator`: LLM client → chain → authenticator (`FakeAuthUi`) → upstream →
//! fake server that requires the `sso-dev` cookie.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{COOKIE, Stack, builder, connect, exposed, params, session, text_of};
use mcp_proxy_core::config::{EnvId, McpId, ToolName};
use mcp_proxy_core::session::{AccessReason, SessionGate, SessionState};
use mcp_proxy_core::trace::CallStatus;
use mcp_proxy_gateway::updates::WatchTiming;
use mcp_proxy_testkit::Generation;
use rmcp::model::CallToolResult;
use rmcp::service::{Peer, RoleClient};
use serde_json::json;

const GENERATIONS: [Generation; 2] = [Generation::Legacy, Generation::Modern];

async fn call(client: &Peer<RoleClient>, tool: &str) -> CallToolResult {
    client
        .call_tool(params(&exposed(tool), json!({ "query": "error" })))
        .await
        .unwrap()
}

fn last_status(stack: &Stack) -> CallStatus {
    stack.trace.records().last().unwrap().status
}

/// `tools/call` requests that reached the server with the right cookie.
fn accepted_calls(stack: &Stack) -> usize {
    stack
        .server
        .requests()
        .iter()
        .filter(|r| r.tool.is_some())
        .filter(|r| {
            r.headers
                .get("cookie")
                .is_some_and(|v| v.to_str().unwrap() == format!("SESSION={COOKIE}"))
        })
        .count()
}

// Exigence : EX-010-10
#[tokio::test(flavor = "multi_thread")]
async fn the_first_call_waits_for_the_login_then_goes_through() {
    for generation in GENERATIONS {
        let stack = builder(generation).on_demand().build().await;
        let on_demand = stack.on_demand.as_ref().unwrap();
        let (client, _connection) = connect(&stack.gateway, (), Generation::Legacy).await;
        assert!(!on_demand.authenticator.is_active(&session()));

        let pending = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .call_tool(params(&exposed("search_logs"), json!({ "query": "error" })))
                    .await
                    .unwrap()
            })
        };
        let window = on_demand.ui.window(0).await;
        assert_eq!(window.spec().session(), &session());
        assert_eq!(window.spec().title(), "Connexion — SSO DEV");
        assert_eq!(
            on_demand.ui.notifications(),
            vec![(
                session(),
                AccessReason::ToolCall {
                    env: EnvId::new("dev").unwrap(),
                    mcp: McpId::new("kibana-logs").unwrap(),
                    tool: ToolName::new("search_logs").unwrap(),
                }
            )]
        );
        assert!(matches!(
            on_demand.registry.state(&session()),
            Some(SessionState::LoginRequested { .. })
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!pending.is_finished(), "{generation:?}: the call is held");
        assert!(stack.server.requests().is_empty(), "nothing sent before the login");

        on_demand.log_in(0, COOKIE).await;
        let result = pending.await.unwrap();
        assert_ne!(result.is_error, Some(true), "{generation:?}: {}", text_of(&result));
        assert_eq!(accepted_calls(&stack), 1);
        assert_eq!(last_status(&stack), CallStatus::Ok);
        assert!(matches!(
            on_demand.registry.state(&session()),
            Some(SessionState::Active { .. })
        ));

        // The next call needs nothing more.
        let result = call(&client, "search_logs").await;
        assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
        assert_eq!(on_demand.ui.window_count(), 1);
    }
}

// Exigence : EX-010-10
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_credential_gives_one_new_login_and_one_replay() {
    for generation in GENERATIONS {
        let stack = builder(generation).on_demand().build().await;
        let on_demand = stack.on_demand.as_ref().unwrap();
        let (client, _connection) = connect(&stack.gateway, (), Generation::Legacy).await;

        let pending = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .call_tool(params(&exposed("search_logs"), json!({ "query": "error" })))
                    .await
                    .unwrap()
            })
        };
        // A cookie the server refuses, then the right one.
        on_demand.log_in(0, "cookie-perime").await;
        on_demand.log_in(1, COOKIE).await;
        let result = pending.await.unwrap();
        assert_ne!(result.is_error, Some(true), "{generation:?}: {}", text_of(&result));
        assert_eq!(on_demand.ui.window_count(), 2, "{generation:?}: one new login");
        assert_eq!(accepted_calls(&stack), 1, "{generation:?}: one replay");
        assert_eq!(last_status(&stack), CallStatus::Ok);

        // Refused again after the new login: an error, no loop, the session is expired.
        on_demand.authenticator.disconnect(&session()).await;
        let pending = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .call_tool(params(&exposed("search_logs"), json!({ "query": "error" })))
                    .await
                    .unwrap()
            })
        };
        on_demand.log_in(2, "cookie-perime").await;
        on_demand.log_in(3, "toujours-perime").await;
        let result = pending.await.unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(
            text_of(&result).contains("identifiant refusé par le serveur, même après une nouvelle connexion"),
            "{generation:?}: {}",
            text_of(&result)
        );
        assert_eq!(on_demand.ui.window_count(), 4, "{generation:?}");
        assert_eq!(last_status(&stack), CallStatus::AuthRequired);
        assert_eq!(on_demand.registry.state(&session()), Some(SessionState::Expired));
        assert_eq!(accepted_calls(&stack), 1);
        // Nothing opens until a call asks again.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(on_demand.ui.window_count(), 4);
    }
}

// Exigence : EX-010-10
#[tokio::test(flavor = "multi_thread")]
async fn a_login_past_the_budget_answers_then_the_next_call_passes() {
    let stack = builder(Generation::Both)
        .on_demand()
        .config(|config| {
            config.gateway.call_budget_secs = 1;
            config.gateway.auth_timeout_secs = 1;
        })
        .build()
        .await;
    let on_demand = stack.on_demand.as_ref().unwrap();
    let (client, _connection) = connect(&stack.gateway, (), Generation::Legacy).await;
    let result = call(&client, "search_logs").await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text_of(&result).contains("connexion à « SSO DEV » requise"),
        "{}",
        text_of(&result)
    );
    assert_eq!(last_status(&stack), CallStatus::AuthRequired);
    let window = on_demand.ui.window(0).await;
    assert!(!window.is_closed(), "the login goes on");

    on_demand.log_in(0, COOKIE).await;
    let result = call(&client, "search_logs").await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(on_demand.ui.window_count(), 1);
}

// Exigence : EX-010-10
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_window_refuses_the_call_clearly() {
    let stack = builder(Generation::Both).on_demand().build().await;
    let on_demand = stack.on_demand.as_ref().unwrap();
    let (client, _connection) = connect(&stack.gateway, (), Generation::Legacy).await;
    let pending = {
        let client = client.clone();
        tokio::spawn(async move { call(&client, "search_logs").await })
    };
    on_demand.ui.window(0).await.close_by_user();
    let result = pending.await.unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(
        text_of(&result).contains("connexion à « SSO DEV » annulée par l'utilisateur, appel refusé."),
        "{}",
        text_of(&result)
    );
    assert_eq!(last_status(&stack), CallStatus::AuthRequired);
    assert!(stack.server.requests().is_empty());
}

// Exigence : EX-010-10
#[tokio::test(flavor = "multi_thread")]
async fn a_login_reloads_the_tools_of_the_instances_using_the_session() {
    let stack = builder(Generation::Both).on_demand().build().await;
    let on_demand = stack.on_demand.as_ref().unwrap();
    let watcher = stack.watch(WatchTiming {
        refresh_interval: Duration::from_secs(3600),
        debounce: Duration::from_millis(50),
        min_refresh: Duration::from_secs(30),
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Periodic reloads never log in: nothing is asked of a session that is not active.
    assert_eq!(stack.server.list_requests(), 0);
    assert!(stack.registry.read().await.resolve(&exposed("echo_headers")).is_none());

    let authenticator = Arc::clone(&on_demand.authenticator);
    let connect = tokio::spawn(async move { authenticator.connect(&session()).await });
    on_demand.log_in(0, COOKIE).await;
    connect.await.unwrap().unwrap();
    for _ in 0..100 {
        if stack.registry.read().await.resolve(&exposed("echo_headers")).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        stack.registry.read().await.resolve(&exposed("echo_headers")).is_some(),
        "the live list replaces the snapshot after the login"
    );
    assert!(stack.server.list_requests() >= 1);
    drop(watcher);
}
