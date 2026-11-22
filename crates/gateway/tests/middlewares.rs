//! The call chain, end to end: LLM client → `GatewayServer` → `CallChain` → `HttpUpstream` → fake MCP server.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use common::{Stack, TOKEN, builder, connect, exposed, params, session, stack, text_of};
use mcp_proxy_core::config::{Channel, Decision, EnvId, McpId, RateLimit, Rule, ToolName};
use mcp_proxy_core::confirm::ConfirmationOutcome;
use mcp_proxy_core::filter::FIELD_MARKER;
use mcp_proxy_core::secrets::Credential;
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_core::trace::CallStatus;
use mcp_proxy_gateway::server::STAGES;
use mcp_proxy_testkit::Generation;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientConfig, ElicitRequestParams,
    ElicitResult, ElicitationAction, Implementation, InputRequest, InputRequiredResult,
};
use rmcp::service::{RequestContext, RoleClient, RunningService};
use rmcp::{ClientHandler, ErrorData};
use secrecy::SecretString;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
enum Answer {
    Accept,
    Decline,
    Cancel,
    /// Answers with a JSON-RPC error.
    Error,
    /// Never answers.
    Hang,
}

/// An LLM client that answers elicitation requests from a script and records their messages.
#[derive(Clone)]
struct Client {
    name: String,
    elicitation: bool,
    answers: Arc<Mutex<VecDeque<Answer>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Client {
    fn new(elicitation: bool) -> Self {
        Self {
            name: "client-de-test".to_owned(),
            elicitation,
            answers: Arc::default(),
            asked: Arc::default(),
        }
    }

    fn answer(&self, answers: impl IntoIterator<Item = Answer>) {
        self.answers.lock().unwrap().extend(answers);
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        let capabilities = if self.elicitation {
            ClientCapabilities::builder().enable_elicitation().build()
        } else {
            ClientCapabilities::default()
        };
        ClientConfig::new(capabilities, Implementation::new(self.name.clone(), "4.2.0"))
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        if let ElicitRequestParams::FormElicitationParams { message, .. } = request {
            self.asked.lock().unwrap().push(message);
        }
        let answer = self.answers.lock().unwrap().pop_front();
        let action = match answer {
            Some(Answer::Accept) => ElicitationAction::Accept,
            Some(Answer::Cancel) => ElicitationAction::Cancel,
            Some(Answer::Error) => return Err(ErrorData::internal_error("panne du client", None)),
            Some(Answer::Hang) => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                ElicitationAction::Cancel
            }
            Some(Answer::Decline) | None => ElicitationAction::Decline,
        };
        Ok(ElicitResult::new(action))
    }
}

const GENERATIONS: [Generation; 2] = [Generation::Legacy, Generation::Modern];

fn dev() -> EnvId {
    EnvId::new("dev").unwrap()
}

fn kibana() -> McpId {
    McpId::new("kibana-logs").unwrap()
}

/// A user hardening rule on one tool of `dev/kibana-logs`.
fn harden(settings: &mut UserSettings, tool: &str, decision: Decision) {
    settings
        .hardening
        .env
        .entry(dev())
        .or_default()
        .mcp
        .entry(kibana())
        .or_default()
        .tools
        .insert(
            ToolName::new(tool).unwrap(),
            Rule {
                decision,
                locked: false,
            },
        );
}

fn trusted(config: &mut mcp_proxy_core::config::Config) {
    config.gateway.elicitation_clients = vec!["client-de-test".to_owned()];
}

fn rate_limit(per_minute: u32, burst: u32) -> impl FnOnce(&mut mcp_proxy_core::config::Config) {
    move |config| {
        let kibana = config.mcp.iter_mut().find(|m| m.id.as_str() == "kibana-logs").unwrap();
        kibana.rate_limit = Some(RateLimit { per_minute, burst });
    }
}

fn timeouts(budget: u64, confirmation: u64, auth: u64) -> impl FnOnce(&mut mcp_proxy_core::config::Config) {
    move |config| {
        config.gateway.call_budget_secs = budget;
        config.gateway.confirmation_timeout_secs = confirmation;
        config.gateway.auth_timeout_secs = auth;
    }
}

async fn call(client: &RunningService<RoleClient, Client>, tool: &str, arguments: Value) -> CallToolResult {
    client.call_tool(params(&exposed(tool), arguments)).await.unwrap()
}

fn assert_refused(result: &CallToolResult, expected: &str) {
    assert_eq!(result.is_error, Some(true), "{}", text_of(result));
    assert!(text_of(result).contains(expected), "{}", text_of(result));
}

/// The text of a result made of one text block.
fn only_text(result: &CallToolResult) -> String {
    let content = serde_json::to_value(&result.content).unwrap();
    assert_eq!(content.as_array().map(Vec::len), Some(1), "{content}");
    content[0]["text"].as_str().unwrap().to_owned()
}

fn last_status(stack: &Stack) -> CallStatus {
    stack.trace.last().unwrap().status
}

// Exigence : EX-007-13
#[tokio::test(flavor = "multi_thread")]
async fn the_stages_run_in_the_specified_order() {
    let stack = stack(Generation::Both).await;
    let chain = stack.chain.as_ref().unwrap();
    assert_eq!(chain.stage_names(), STAGES);
    assert_eq!(
        STAGES,
        [
            "resolution",
            "suspension",
            "policy",
            "arguments",
            "rate_limit",
            "confirmation",
            "session",
            "upstream"
        ]
    );
}

// Exigence : EX-007-01, EX-007-13
#[tokio::test(flavor = "multi_thread")]
async fn a_denied_call_stops_at_the_policy() {
    let stack = builder(Generation::Both)
        .config(rate_limit(60, 1))
        .settings(|s| harden(s, "delete_index", Decision::Deny))
        .build()
        .await;
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Client::new(true), generation).await;
        // Arguments that would be blocked, so that the scan is shown not to run.
        let result = call(&client, "delete_index", json!({ "index": TOKEN })).await;
        assert_refused(
            &result,
            "MCP Proxy : appel interdit par la politique (DEV/kibana-logs/delete_index). Raison : refusé",
        );
        assert!(!text_of(&result).contains(TOKEN));
        assert_eq!(last_status(&stack), CallStatus::Denied);
    }
    assert_eq!(stack.upstream_calls(), 0);
    assert!(stack.broker.requests().is_empty());
    assert_eq!(stack.gate.ensure_count(), 0);
    // No token was taken by the refused calls: the one token of the burst is still there.
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    let result = call(&client, "search_logs", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
}

// Exigence : EX-007-01
#[tokio::test(flavor = "multi_thread")]
async fn a_confirmation_lets_through_only_an_explicit_approval() {
    let stack = builder(Generation::Both)
        .config(timeouts(10, 1, 10))
        .settings(|s| harden(s, "list_indices", Decision::Confirm(Channel::Notification)))
        .build()
        .await;
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    let arguments = json!({ "pattern": "app-*", "password": "hunter2" });

    stack.broker.script([ConfirmationOutcome::Approved]);
    let result = call(&client, "list_indices", arguments.clone()).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(stack.upstream_calls(), 1);
    let request = stack.broker.requests().pop().unwrap();
    assert_eq!(request.path, "DEV / kibana-logs / list_indices");
    assert_eq!(request.client, "client-de-test");
    assert_eq!(request.arguments["pattern"], "app-*");
    assert_eq!(request.arguments["password"], FIELD_MARKER);
    assert!(
        request.explanation.starts_with("confirmation (notification)"),
        "{}",
        request.explanation
    );
    let ensures = stack.gate.ensure_count();

    for (outcome, expected, status) in [
        (
            Some(ConfirmationOutcome::Refused),
            "MCP Proxy : appel refusé par l'utilisateur.",
            CallStatus::Refused,
        ),
        (
            Some(ConfirmationOutcome::TimedOut),
            "MCP Proxy : confirmation non reçue dans le délai (1 s), appel refusé.",
            CallStatus::Timeout,
        ),
        (
            Some(ConfirmationOutcome::Unavailable),
            "MCP Proxy : confirmation impossible",
            CallStatus::Refused,
        ),
        // No answer at all: the chain gives up by itself at the end of the delay.
        (
            None,
            "MCP Proxy : confirmation non reçue dans le délai (1 s), appel refusé.",
            CallStatus::Timeout,
        ),
    ] {
        match outcome {
            Some(outcome) => stack.broker.script([outcome]),
            None => stack.broker.stay_silent(),
        }
        let started = Instant::now();
        let result = call(&client, "list_indices", arguments.clone()).await;
        assert_refused(&result, expected);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(last_status(&stack), status);
    }
    // Refused calls neither connect nor call.
    assert_eq!(stack.gate.ensure_count(), ensures);
    assert_eq!(stack.upstream_calls(), 1);
    // An exhausted script refuses (INV-5).
    assert_refused(
        &call(&client, "list_indices", arguments).await,
        "MCP Proxy : appel refusé par l'utilisateur.",
    );
}

// Exigence : EX-007-02
#[tokio::test(flavor = "multi_thread")]
async fn a_known_secret_in_the_arguments_blocks_the_call() {
    let stack = stack(Generation::Both).await;
    let hex: String = TOKEN.bytes().map(|b| format!("{b:02X}")).collect();
    let cases = [
        json!({ "index": TOKEN }),
        json!({ "query": format!("message:{}", STANDARD.encode(TOKEN)) }),
        json!({ "query": URL_SAFE_NO_PAD.encode(TOKEN) }),
        json!({ "filters": [{ "value": hex }] }),
        json!({ TOKEN: "clé" }),
        json!({ "nested": { "deeper": [1, 2, { "q": format!("Bearer {TOKEN}") }] } }),
    ];
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Client::new(false), generation).await;
        for arguments in &cases {
            // delete_index asks for a confirmation: a blocked call must not ask.
            for tool in ["search_logs", "delete_index"] {
                let result = call(&client, tool, arguments.clone()).await;
                assert_eq!(result.is_error, Some(true));
                assert_eq!(
                    only_text(&result),
                    "MCP Proxy : appel bloqué, les arguments contiennent un secret connu.",
                    "{arguments}"
                );
                let record = stack.trace.last().unwrap();
                assert_eq!(record.status, CallStatus::Blocked);
                assert!(!format!("{record:?}").contains(TOKEN), "{record:?}");
            }
        }
    }
    assert_eq!(stack.upstream_calls(), 0);
    assert!(stack.broker.requests().is_empty());
    assert_eq!(stack.gate.ensure_count(), 0);
}

// Exigence : EX-007-03, EX-007-13
#[tokio::test(flavor = "multi_thread")]
async fn the_rate_limit_is_reached_then_restored() {
    // 10 tokens a second, 2 at most.
    let stack = builder(Generation::Both).config(rate_limit(600, 2)).build().await;
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Modern).await;
    for _ in 0..2 {
        let result = call(&client, "search_logs", json!({})).await;
        assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    }
    let result = call(&client, "search_logs", json!({})).await;
    assert_refused(
        &result,
        "MCP Proxy : limite de débit atteinte pour Kibana Logs (DEV). Réessayez dans 1 s.",
    );
    assert_eq!(last_status(&stack), CallStatus::RateLimited);
    assert_eq!(stack.upstream_calls(), 2);
    let ensures = stack.gate.ensure_count();
    // A call blocked above the rate limit takes no token either.
    let blocked = call(&client, "search_logs", json!({ "q": TOKEN })).await;
    assert_refused(&blocked, "appel bloqué");
    assert_eq!(stack.gate.ensure_count(), ensures);

    tokio::time::sleep(Duration::from_millis(150)).await;
    let result = call(&client, "search_logs", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(stack.upstream_calls(), 3);
}

// Exigence : EX-007-04
#[tokio::test(flavor = "multi_thread")]
async fn the_header_goes_only_to_the_origin_of_the_session() {
    let stack = stack(Generation::Both).await;
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    let result = call(&client, "echo_headers", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let received = stack.server.requests().into_iter().find(|r| r.tool.is_some()).unwrap();
    assert_eq!(
        received.headers.get("authorization").unwrap(),
        &format!("Bearer {TOKEN}")
    );

    let stack = builder(Generation::Both).upstream_outside_origins().build().await;
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    let result = call(&client, "echo_headers", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(!text_of(&result).contains(TOKEN));
    let elsewhere = stack.elsewhere.as_ref().unwrap();
    assert!(
        elsewhere
            .requests()
            .iter()
            .all(|r| r.headers.get("authorization").is_none()),
        "{:?}",
        elsewhere.requests()
    );
    assert_eq!(last_status(&stack), CallStatus::Error);
}

// Exigence : EX-007-05
#[tokio::test(flavor = "multi_thread")]
async fn a_secret_sent_back_by_the_server_is_masked_and_reported() {
    let stack = stack(Generation::Both).await;
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Client::new(false), generation).await;
        for tool in ["echo_headers", "fail"] {
            let result = call(&client, tool, json!({})).await;
            assert!(!text_of(&result).contains(TOKEN), "{}", text_of(&result));
            let record = stack.trace.last().unwrap();
            assert_eq!(record.status, CallStatus::LeakMasked, "{tool}");
            assert!(record.filter_report.exact_hits > 0);
            assert!(!record.response.to_string().contains(TOKEN));
        }
        let clean = call(&client, "search_logs", json!({})).await;
        assert_ne!(clean.is_error, Some(true));
        assert_eq!(last_status(&stack), CallStatus::Ok);
    }
}

fn wrong_token(stack: &Stack) {
    stack
        .gate
        .store()
        .put(
            &session(),
            Credential::Bearer {
                token: SecretString::from("jeton-perime-0000"),
            },
        )
        .unwrap();
}

// Exigence : EX-007-06
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_credential_is_renewed_and_the_call_replayed_once() {
    for generation in GENERATIONS {
        let stack = stack(generation).await;
        let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
        wrong_token(&stack);
        stack.gate.logins([TOKEN]);
        let result = call(&client, "search_logs", json!({ "query": "error" })).await;
        assert_ne!(result.is_error, Some(true), "{generation:?}: {}", text_of(&result));
        assert_eq!(stack.gate.rejected(), vec![session()]);
        assert_eq!(stack.gate.ensure_count(), 2, "session stage, then the replay");
        assert_eq!(stack.upstream_calls(), 2);
        assert_eq!(last_status(&stack), CallStatus::Ok);

        // Refused again after the new login: an error, no loop.
        wrong_token(&stack);
        stack.gate.logins(["jeton-toujours-faux"]);
        let result = call(&client, "search_logs", json!({ "query": "error" })).await;
        assert_refused(
            &result,
            "identifiant refusé par le serveur, même après une nouvelle connexion",
        );
        // The upstream may see more than one 401 per call (reconnection); what matters is one login, one replay.
        assert!(stack.gate.rejected().len() >= 3);
        assert_eq!(stack.gate.ensure_count(), 4);
        // The replay may already be refused when reconnecting, before its `tools/call`.
        assert!((3..=4).contains(&stack.upstream_calls()), "{}", stack.upstream_calls());
        assert_eq!(last_status(&stack), CallStatus::AuthRequired);
    }
}

// Exigence : EX-007-07
#[tokio::test(flavor = "multi_thread")]
async fn a_login_beyond_the_budget_asks_to_retry_then_the_next_call_succeeds() {
    let stack = builder(Generation::Both).config(timeouts(1, 1, 1)).build().await;
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    // A login nobody completes: the chain answers at the end of the budget, whatever the gate does.
    stack.gate.hang();
    let started = Instant::now();
    let result = call(&client, "search_logs", json!({})).await;
    assert_refused(
        &result,
        "MCP Proxy : connexion à « SSO DEV » requise. La fenêtre de connexion est ouverte sur le poste de \
         l'utilisateur. Réessayez une fois la connexion faite.",
    );
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(last_status(&stack), CallStatus::AuthRequired);
    assert_eq!(stack.upstream_calls(), 0);

    let result = call(&client, "search_logs", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
}

// Exigence : EX-007-07
#[tokio::test(flavor = "multi_thread")]
async fn confirmation_and_login_share_one_budget() {
    let stack = builder(Generation::Both)
        .config(timeouts(2, 2, 2))
        .settings(|s| harden(s, "list_indices", Decision::Confirm(Channel::Notification)))
        .build()
        .await;
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    stack
        .broker
        .answer_after(Duration::from_millis(1500), ConfirmationOutcome::Approved);
    stack.gate.hang();
    let started = Instant::now();
    let result = call(&client, "list_indices", json!({})).await;
    let elapsed = started.elapsed();
    assert_refused(&result, "MCP Proxy : connexion à « SSO DEV » requise.");
    assert!(
        elapsed >= Duration::from_millis(1900) && elapsed < Duration::from_millis(2900),
        "{elapsed:?}"
    );
}

// Exigence : EX-007-08
#[tokio::test(flavor = "multi_thread")]
async fn legacy_elicitation_accepted_declined_or_unanswered() {
    let stack = builder(Generation::Both)
        .config(trusted)
        .config(timeouts(10, 1, 10))
        .build()
        .await;
    let confirmer = Client::new(true);
    let (client, _connection) = connect(&stack.gateway, confirmer.clone(), Generation::Legacy).await;
    let arguments = json!({ "index": "app-2026.11", "password": "hunter2" });

    confirmer.answer([Answer::Accept]);
    let result = call(&client, "delete_index", arguments.clone()).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(stack.upstream_calls(), 1);
    let asked = confirmer.asked().pop().unwrap();
    assert!(asked.contains("DEV / kibana-logs / delete_index"), "{asked}");
    assert!(asked.contains("« client-de-test »"), "{asked}");
    assert!(asked.contains("app-2026.11") && asked.contains(FIELD_MARKER), "{asked}");
    assert!(!asked.contains("hunter2"), "{asked}");

    for (answer, expected, status) in [
        (
            Answer::Decline,
            "MCP Proxy : appel refusé par l'utilisateur.",
            CallStatus::Refused,
        ),
        (
            Answer::Cancel,
            "MCP Proxy : appel refusé par l'utilisateur.",
            CallStatus::Refused,
        ),
        // An error answer refuses too: only an acceptance lets the call through.
        (
            Answer::Error,
            "MCP Proxy : appel refusé par l'utilisateur.",
            CallStatus::Refused,
        ),
        (
            Answer::Hang,
            "MCP Proxy : confirmation non reçue dans le délai (1 s), appel refusé.",
            CallStatus::Timeout,
        ),
    ] {
        confirmer.answer([answer]);
        let started = Instant::now();
        let result = call(&client, "delete_index", arguments.clone()).await;
        assert_refused(&result, expected);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(last_status(&stack), status);
    }
    assert_eq!(confirmer.asked().len(), 5);
    assert_eq!(stack.upstream_calls(), 1);
    assert!(
        stack.broker.requests().is_empty(),
        "the chat was used, not the notification"
    );
}

async fn first_round(
    client: &RunningService<RoleClient, Client>,
    arguments: &Value,
) -> (CallToolRequestParams, InputRequiredResult) {
    let request = params(&exposed("delete_index"), arguments.clone());
    match client.call_tool_once(request.clone()).await.unwrap() {
        CallToolResponse::InputRequired(result) => (request, result),
        other => panic!("{other:?}"),
    }
}

fn second_round(mut request: CallToolRequestParams, state: &str, action: ElicitationAction) -> CallToolRequestParams {
    request.request_state = Some(state.to_owned());
    request.input_responses = Some(
        [(
            "confirmation".to_owned(),
            serde_json::to_value(ElicitResult::new(action)).unwrap(),
        )]
        .into(),
    );
    request
}

async fn complete(client: &RunningService<RoleClient, Client>, request: CallToolRequestParams) -> CallToolResult {
    match client.call_tool_once(request).await.unwrap() {
        CallToolResponse::Complete(result) => result,
        other => panic!("{other:?}"),
    }
}

// Exigence : EX-007-09
#[tokio::test(flavor = "multi_thread")]
async fn modern_elicitation_round_trip_and_forged_states() {
    let stack = builder(Generation::Both)
        .config(trusted)
        .config(timeouts(10, 1, 10))
        .build()
        .await;
    let confirmer = Client::new(true);
    let (client, _connection) = connect(&stack.gateway, confirmer.clone(), Generation::Modern).await;
    let arguments = json!({ "index": "app-2026.11", "password": "hunter2" });

    // Complete round trip, driven by the SDK through the client's handler.
    confirmer.answer([Answer::Accept]);
    let result = call(&client, "delete_index", arguments.clone()).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(stack.upstream_calls(), 1);
    let asked = confirmer.asked().pop().unwrap();
    assert!(
        asked.contains("DEV / kibana-logs / delete_index") && asked.contains(FIELD_MARKER),
        "{asked}"
    );
    assert!(!asked.contains("hunter2"), "{asked}");
    confirmer.answer([Answer::Decline]);
    assert_refused(
        &call(&client, "delete_index", arguments.clone()).await,
        "MCP Proxy : appel refusé par l'utilisateur.",
    );

    // The first round asks and does nothing else.
    let ensures = stack.gate.ensure_count();
    let (request, input) = first_round(&client, &arguments).await;
    assert_eq!(last_status(&stack), CallStatus::InputRequired);
    assert_eq!(stack.gate.ensure_count(), ensures, "no login before the answer");
    let requests = input.input_requests.as_ref().unwrap();
    assert!(matches!(
        requests.get("confirmation"),
        Some(InputRequest::Elicitation(_))
    ));
    assert!(!serde_json::to_string(&input).unwrap().contains("hunter2"));
    let state = input.request_state.clone().unwrap();
    // Accepted, then the very same state again: replay refused.
    let result = complete(
        &client,
        second_round(request.clone(), &state, ElicitationAction::Accept),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(stack.upstream_calls(), 2);
    let replayed = complete(
        &client,
        second_round(request.clone(), &state, ElicitationAction::Accept),
    )
    .await;
    assert_refused(&replayed, "MCP Proxy : confirmation invalide ou expirée, appel refusé.");

    // Tampered state.
    let (request, input) = first_round(&client, &arguments).await;
    let mut tampered = input.request_state.unwrap().into_bytes();
    tampered[10] = if tampered[10] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap();
    let result = complete(&client, second_round(request, &tampered, ElicitationAction::Accept)).await;
    assert_refused(&result, "MCP Proxy : confirmation invalide ou expirée, appel refusé.");

    // Arguments changed between the two rounds.
    let (request, input) = first_round(&client, &arguments).await;
    let mut changed = second_round(request, &input.request_state.unwrap(), ElicitationAction::Accept);
    changed.arguments = json!({ "index": "app-*", "password": "hunter2" }).as_object().cloned();
    assert_refused(
        &complete(&client, changed).await,
        "MCP Proxy : confirmation invalide ou expirée, appel refusé.",
    );

    // Another connection.
    let (request, input) = first_round(&client, &arguments).await;
    let (other, _other_connection) = connect(&stack.gateway, Client::new(true), Generation::Modern).await;
    let result = complete(
        &other,
        second_round(request, &input.request_state.unwrap(), ElicitationAction::Accept),
    )
    .await;
    assert_refused(&result, "MCP Proxy : confirmation invalide ou expirée, appel refusé.");

    // A state with no answer, or an answer with no state.
    let (request, input) = first_round(&client, &arguments).await;
    let mut no_answer = second_round(
        request.clone(),
        &input.request_state.unwrap(),
        ElicitationAction::Accept,
    );
    no_answer.input_responses = None;
    assert_refused(&complete(&client, no_answer).await, "confirmation invalide ou expirée");
    let mut no_state = second_round(request, "x", ElicitationAction::Accept);
    no_state.request_state = None;
    assert_refused(&complete(&client, no_state).await, "confirmation invalide ou expirée");

    // Expired.
    let (request, input) = first_round(&client, &arguments).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let result = complete(
        &client,
        second_round(request, &input.request_state.unwrap(), ElicitationAction::Accept),
    )
    .await;
    assert_refused(
        &result,
        "MCP Proxy : confirmation non reçue dans le délai (1 s), appel refusé.",
    );

    assert_eq!(stack.upstream_calls(), 2);
    assert!(stack.broker.requests().is_empty());
}

// Exigence : EX-007-10
#[tokio::test(flavor = "multi_thread")]
async fn the_notification_is_used_unless_the_client_is_trusted_and_the_policy_allows_the_chat() {
    // Not listed in `elicitation_clients` (empty in the embedded configuration).
    let stack = stack(Generation::Both).await;
    for generation in GENERATIONS {
        let confirmer = Client::new(true);
        let (client, _connection) = connect(&stack.gateway, confirmer.clone(), generation).await;
        stack.broker.script([ConfirmationOutcome::Approved]);
        let result = call(&client, "delete_index", json!({ "index": "app-1" })).await;
        assert_ne!(result.is_error, Some(true), "{generation:?}: {}", text_of(&result));
        assert!(confirmer.asked().is_empty(), "{generation:?}");
    }
    assert_eq!(stack.broker.requests().len(), 2);

    // Listed, but the client does not announce elicitation.
    let stack = builder(Generation::Both).config(trusted).build().await;
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Client::new(false), generation).await;
        stack.broker.script([ConfirmationOutcome::Approved]);
        let result = call(&client, "delete_index", json!({ "index": "app-1" })).await;
        assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    }
    assert_eq!(stack.broker.requests().len(), 2);

    // Listed and announced, but the policy asks for the notification: never the chat.
    let stack = builder(Generation::Both)
        .config(trusted)
        .settings(|s| harden(s, "delete_index", Decision::Confirm(Channel::Notification)))
        .build()
        .await;
    for generation in GENERATIONS {
        let confirmer = Client::new(true);
        confirmer.answer([Answer::Accept]);
        let (client, _connection) = connect(&stack.gateway, confirmer.clone(), generation).await;
        stack.broker.script([ConfirmationOutcome::Refused]);
        let result = call(&client, "delete_index", json!({ "index": "app-1" })).await;
        assert_refused(&result, "MCP Proxy : appel refusé par l'utilisateur.");
        assert!(confirmer.asked().is_empty(), "{generation:?}");
    }
    assert_eq!(stack.broker.requests().len(), 2);
    assert_eq!(stack.upstream_calls(), 0);
}

// Exigence : EX-007-12
#[tokio::test(flavor = "multi_thread")]
async fn a_suspended_gateway_refuses_every_call() {
    let stack = stack(Generation::Both).await;
    stack.settings.send_modify(|s| s.gateway_suspended = true);
    for generation in GENERATIONS {
        let (client, _connection) = connect(&stack.gateway, Client::new(true), generation).await;
        for tool in ["search_logs", "delete_index"] {
            let result = call(&client, tool, json!({ "q": TOKEN })).await;
            assert_eq!(
                only_text(&result),
                "MCP Proxy : passerelle suspendue par l'utilisateur."
            );
            let record = stack.trace.last().unwrap();
            assert_eq!(record.status, CallStatus::Suspended);
            assert!(record.policy.is_none(), "no policy resolved");
        }
        // Resolution comes first.
        let result = client
            .call_tool(params("prod_kibana_search_logs", json!({})))
            .await
            .unwrap();
        assert_refused(&result, "inconnu ou non exposé");
    }
    assert_eq!(stack.upstream_calls(), 0);
    assert!(stack.broker.requests().is_empty());
    assert_eq!(stack.gate.ensure_count(), 0);

    stack.settings.send_modify(|s| s.gateway_suspended = false);
    let (client, _connection) = connect(&stack.gateway, Client::new(false), Generation::Legacy).await;
    let result = call(&client, "search_logs", json!({})).await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
}

// Exigence : EX-007-11
#[tokio::test(flavor = "multi_thread")]
async fn every_call_leaves_a_filtered_record_with_its_status() {
    let stack = builder(Generation::Both)
        .config(rate_limit(1, 5))
        .config(timeouts(10, 1, 10))
        .settings(|s| harden(s, "get_log_entry", Decision::Deny))
        .build()
        .await;
    // A client whose very name carries the secret.
    let mut named = Client::new(false);
    named.name = format!("client {TOKEN}");
    let (client, _connection) = connect(&stack.gateway, named, Generation::Legacy).await;

    let _ = client
        .call_tool(params("prod_kibana_search_logs", json!({})))
        .await
        .unwrap();
    call(&client, "get_log_entry", json!({ "id": 1 })).await;
    call(&client, "search_logs", json!({ "q": TOKEN })).await;
    stack.broker.script([ConfirmationOutcome::Refused]);
    call(&client, "delete_index", json!({ "index": "a" })).await;
    stack.broker.script([ConfirmationOutcome::TimedOut]);
    call(&client, "delete_index", json!({ "index": "b" })).await;
    stack
        .gate
        .script([Err(mcp_proxy_core::session::SessionError::LoginPending(session()))]);
    call(&client, "search_logs", json!({ "password": "hunter2" })).await;
    call(&client, "echo_headers", json!({})).await;
    call(&client, "search_logs", json!({})).await;
    call(&client, "search_logs", json!({})).await;
    stack.settings.send_modify(|s| s.gateway_suspended = true);
    call(&client, "search_logs", json!({})).await;

    let records = stack.trace.records();
    let statuses: Vec<CallStatus> = records.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        [
            CallStatus::Denied,
            CallStatus::Denied,
            CallStatus::Blocked,
            CallStatus::Refused,
            CallStatus::Timeout,
            CallStatus::AuthRequired,
            CallStatus::LeakMasked,
            CallStatus::Ok,
            CallStatus::RateLimited,
            CallStatus::Suspended,
        ]
    );
    let ids: Vec<u64> = records.iter().map(|r| r.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    for record in &records {
        let text = format!("{record:?}");
        assert!(!text.contains(TOKEN), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!record.client.contains(TOKEN) && record.client.starts_with("client "));
    }
    let denied = &records[1];
    assert_eq!(denied.env, Some(dev()));
    assert_eq!(denied.mcp, Some(kibana()));
    assert_eq!(denied.session.as_ref(), Some(&session()));
    assert_eq!(denied.policy.as_ref().unwrap().decision, Decision::Deny);
    assert!(denied.policy.as_ref().unwrap().explanation.starts_with("refusé"));
    assert_eq!(denied.request, json!({ "id": 1 }));
    assert_eq!(records[0].env, None);
    assert_eq!(records[5].request["password"], FIELD_MARKER);
}
