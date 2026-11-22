//! The contract between the journal and the Journal tab (spec 018): entries of every kind, serialized as the window
//! receives them from `get_journal`, and the two answers of `export_diagnostic`, must be the file the tests of `ui/`
//! check against `ui/src/api/types.ts`. A field added, renamed or retyped on either side fails one of the two tests.
//! Then the feed the application follows: every new entry, and a resynchronisation when the window fell behind.
//!
//! After a deliberate change of the view: `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p mcp-proxy-gateway --test
//! journal_contract`, then `npm test` in `ui/`.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mcp_proxy_core::config::{Channel, Decision, EnvId, McpId, SessionId, ToolName};
use mcp_proxy_core::filter::{FilterReport, HitKind, OutputFilter};
use mcp_proxy_core::secrets::SecretStore;
use mcp_proxy_core::trace::{
    CallRecord, CallStatus, Journal, JournalConfig, JournalFilter, ManualWallClock, PolicyRecord,
};
use mcp_proxy_gateway::interface::ExportOutcome;
use mcp_proxy_gateway::journal_view::{FeedItem, JournalFeed, JournalView, filtered_json};
use serde_json::{Value, json};

/// Sun, 22 Nov 2026 14:00:00 UTC.
const NOW_SECS: u64 = 1_795_356_000;
const NOW_MS: u64 = NOW_SECS * 1000;

fn at(offset_secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(NOW_SECS + offset_secs)
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/api/journal.example.json")
}

fn filter() -> Arc<OutputFilter> {
    let config = mcp_proxy_gateway::modules::embedded_config().unwrap();
    let store = Arc::new(SecretStore::for_platform().unwrap());
    Arc::new(OutputFilter::new(store, &config.filter).unwrap())
}

fn journal(clock: &ManualWallClock) -> Journal {
    Journal::new(
        JournalConfig {
            ring_capacity: 5000,
            ring_max_bytes: 1 << 20,
            slow_threshold: Duration::from_secs(5),
        },
        filter(),
        Arc::new(clock.clone()),
        None,
    )
}

fn env(id: &str) -> Option<EnvId> {
    Some(EnvId::new(id).unwrap())
}

fn mcp(id: &str) -> Option<McpId> {
    Some(McpId::new(id).unwrap())
}

#[allow(clippy::too_many_arguments)]
fn call(
    id: u64,
    offset: u64,
    millis: u64,
    environment: &str,
    server: &str,
    tool: &str,
    status: CallStatus,
    policy: Option<(Decision, &str)>,
    request: Value,
    response: Value,
) -> CallRecord {
    CallRecord {
        id,
        started_at: at(offset),
        duration: Duration::from_millis(millis),
        client: "Claude Desktop".to_owned(),
        tool: tool.to_owned(),
        env: env(environment),
        mcp: mcp(server),
        session: Some(SessionId::new(format!("sso-{environment}")).unwrap()),
        policy: policy.map(|(decision, explanation)| PolicyRecord {
            decision,
            explanation: explanation.to_owned(),
        }),
        status,
        request,
        response,
        filter_report: FilterReport::default(),
    }
}

/// One entry of each kind and each family of status: what the window must be able to read.
fn example_journal(clock: &ManualWallClock) -> Journal {
    let journal = journal(clock);
    let read = Some((Decision::Allow, "autorisé — règle « read » de la racine"));
    journal.record_call(call(
        41,
        0,
        182,
        "dev",
        "kibana-logs",
        "dev_kibana_search_logs",
        CallStatus::Ok,
        read,
        json!({ "index": "app-*", "query": "level:error" }),
        json!({ "content": [{ "type": "text", "text": "12 entrées" }] }),
    ));
    journal.record_call(call(
        42,
        12,
        2310,
        "rec",
        "gitlab-ops",
        "rec_gitlab_get_job_log",
        CallStatus::Error,
        read,
        json!({ "project": "paiement/api", "job": 88121 }),
        json!({ "isError": true, "content": [{ "type": "text", "text": "HTTP 502 Bad Gateway" }] }),
    ));
    journal.record_call(call(
        43,
        30,
        7400,
        "dev",
        "kibana-logs",
        "dev_kibana_search_logs",
        CallStatus::Ok,
        read,
        json!({ "index": "audit-*", "query": "*" }),
        json!({ "content": [{ "type": "text", "text": "5000 entrées" }] }),
    ));
    journal.record_call(call(
        44,
        45,
        3,
        "prod",
        "kubernetes-admin",
        "prod_k8s_delete_pod",
        CallStatus::Denied,
        Some((
            Decision::Deny,
            "refusé — règle « destructive » de Kubernetes Admin en PROD, verrouillée",
        )),
        json!({ "namespace": "paiement", "pod": "api-7d9f" }),
        json!({ "isError": true, "content": [{ "type": "text", "text": "Appel refusé par la politique." }] }),
    ));
    let mut leak = call(
        45,
        60,
        640,
        "rec",
        "kibana-logs",
        "rec_kibana_get_log_entry",
        CallStatus::LeakMasked,
        read,
        json!({ "id": "a1b2" }),
        json!({ "content": [{ "type": "text", "text": "Authorization: Bearer exemple-sans-valeur-reelle" }] }),
    );
    leak.filter_report = FilterReport {
        exact_hits: 1,
        generic_hits: 1,
        kinds: vec![HitKind::KnownSecret, HitKind::AuthorizationHeader],
    };
    journal.record_call(leak);
    journal.record_call(call(
        46,
        75,
        4100,
        "prod",
        "gitlab-ops",
        "prod_gitlab_cancel_pipeline",
        CallStatus::Ok,
        Some((
            Decision::Confirm(Channel::Notification),
            "confirmation (notification) — règle « write » de PROD, verrouillée",
        )),
        json!({ "project": "paiement/api", "pipeline": 5120 }),
        json!({ "content": [{ "type": "text", "text": "pipeline annulé" }] }),
    ));
    let mut blocked = call(
        47,
        90,
        1,
        "int",
        "jira",
        "int_jira_add_comment",
        CallStatus::Blocked,
        Some((
            Decision::Confirm(Channel::Elicitation),
            "confirmation (elicitation) — règle « write » de la racine",
        )),
        json!({ "issue": "PAY-12", "body": "[SECRET MASQUÉ]" }),
        json!({ "isError": true, "content": [{ "type": "text", "text": "Un secret connu figure dans les arguments." }] }),
    );
    blocked.session = None;
    journal.record_call(blocked);
    journal.record_tool_changes(
        &EnvId::new("dev").unwrap(),
        &McpId::new("gitlab-ops").unwrap(),
        &[ToolName::new("list_runners").unwrap()],
        &[],
        &[ToolName::new("retry_job").unwrap()],
    );
    clock.advance(Duration::from_secs(120));
    journal.record_event(tracing::Level::INFO, "environnement « PROD » exposé par l'utilisateur");
    journal.record_panic("crates/gateway/src/server/mod.rs:120:9", "index hors limites");
    journal
}

fn json_of(journal: &Journal) -> Value {
    let entries = journal.snapshot(&JournalFilter::default());
    filtered_json(journal.filter(), &JournalView::of(&entries)).unwrap()
}

// Exigence : EX-018-10
// Exigence : EX-018-03
#[test]
fn the_journal_view_is_the_contract_of_the_interface() {
    let clock = ManualWallClock::new(at(0));
    let journal = example_journal(&clock);
    let view = json_of(&journal);
    let fixture = json!({
        "getJournal": view,
        "exportDiagnostic": [
            ExportOutcome::Saved {
                path: r"C:\Users\exemple\Downloads\mcp-proxy-diagnostic-20261122-1400.zip".to_owned(),
            },
            ExportOutcome::Cancelled,
        ],
    });
    let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    let path = fixture_path();
    if std::env::var_os("MCP_PROXY_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    } else {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == text,
            "{} is not the journal view any more: run `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p mcp-proxy-gateway \
             --test journal_contract`, then align ui/src/api/types.ts until `npm test` passes in ui/",
            path.display()
        );
    }

    let entries = view["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 10);
    let by_seq = |seq: u64| entries.iter().find(|e| e["seq"] == seq).unwrap();
    // A call that ended normally keeps no content: the row does not unfold.
    let ok = by_seq(1);
    assert_eq!(ok["kind"], "call");
    assert_eq!(ok["status"], "ok");
    assert_eq!(ok["statusLabel"], "réussi");
    assert_eq!(ok["at"], NOW_MS);
    assert_eq!(ok["durationMs"], 182);
    assert_eq!(ok["environment"], "dev");
    assert_eq!(ok["mcp"], "kibana-logs");
    assert_eq!(
        (ok["problem"].as_bool(), ok["policy"].as_bool()),
        (Some(false), Some(false))
    );
    assert!(ok["details"].is_null());
    // An error keeps its trace, client, session, policy, request and response.
    let error = by_seq(2);
    assert_eq!(error["problem"], true);
    assert_eq!(error["details"]["callId"], 42);
    assert_eq!(error["details"]["client"], "Claude Desktop");
    assert_eq!(error["details"]["session"], "sso-rec");
    assert_eq!(error["details"]["policy"]["decision"], "allow");
    assert_eq!(error["details"]["request"]["project"], "paiement/api");
    assert_eq!(error["details"]["response"]["isError"], true);
    // Slow: a problem whose content is kept, though it succeeded.
    let slow = by_seq(3);
    assert_eq!(
        (slow["slow"].as_bool(), slow["problem"].as_bool()),
        (Some(true), Some(true))
    );
    assert!(slow["details"].is_object());
    // Denied by the policy: a policy, not a problem; unfolds with the explanation of the engine.
    let denied = by_seq(4);
    assert_eq!(
        (denied["problem"].as_bool(), denied["policy"].as_bool()),
        (Some(false), Some(true))
    );
    assert_eq!(denied["decision"], "deny");
    assert_eq!(
        denied["details"]["policy"]["explanation"],
        "refusé — règle « destructive » de Kubernetes Admin en PROD, verrouillée"
    );
    // Leak masked: what was masked is counted, never shown.
    let leak = by_seq(5);
    assert_eq!(leak["status"], "leakMasked");
    assert_eq!(
        leak["details"]["masked"],
        json!({ "exact": 1, "generic": 1, "kinds": ["knownSecret", "authorizationHeader"] })
    );
    assert!(!leak.to_string().contains("exemple-sans-valeur-reelle"));
    // A confirmed call that succeeded: a policy, no content.
    let confirmed = by_seq(6);
    assert_eq!(
        (confirmed["policy"].as_bool(), confirmed["details"].is_null()),
        (Some(true), true)
    );
    let blocked = by_seq(7);
    assert_eq!(
        (blocked["status"].as_str(), blocked["problem"].as_bool()),
        (Some("blocked"), Some(true))
    );
    assert!(blocked["details"]["session"].is_null());
    assert_eq!(by_seq(8)["kind"], "toolsChanged");
    assert!(by_seq(8)["status"].is_null());
    assert_eq!(by_seq(9)["kind"], "event");
    assert_eq!(by_seq(9)["at"], NOW_MS + 120_000);
    assert_eq!(
        (by_seq(10)["kind"].as_str(), by_seq(10)["problem"].as_bool()),
        (Some("panic"), Some(true))
    );
}

// Exigence : EX-018-04
#[test]
fn export_outcomes_are_tagged_by_status() {
    assert_eq!(
        serde_json::to_value(ExportOutcome::Saved {
            path: "D:\\diag.zip".to_owned()
        })
        .unwrap(),
        json!({ "status": "saved", "path": "D:\\diag.zip" })
    );
    assert_eq!(
        serde_json::to_value(ExportOutcome::Cancelled).unwrap(),
        json!({ "status": "cancelled" })
    );
}

// Exigence : EX-018-01
// Exigence : EX-018-12
#[tokio::test]
async fn the_feed_sends_each_new_entry_and_resynchronises_after_a_lag() {
    let clock = ManualWallClock::new(at(0));
    let journal = journal(&clock);
    let mut feed = JournalFeed::new(journal.subscribe(), Arc::clone(journal.filter()));

    journal.record_event(tracing::Level::INFO, "MCP « Kibana Logs » coupé par l'utilisateur");
    let Some(FeedItem::Entry(entry)) = feed.next().await else {
        panic!("one entry expected");
    };
    assert_eq!(entry["seq"], 1);
    assert_eq!(entry["kind"], "event");
    assert_eq!(entry["summary"], "MCP « Kibana Logs » coupé par l'utilisateur");

    // The window fell behind: far more entries than the channel holds, none read.
    for n in 0..1000 {
        journal.record_event(tracing::Level::INFO, &format!("événement {n}"));
    }
    assert_eq!(feed.next().await, Some(FeedItem::Resync), "a lag is not an error");
    // Then the entries go on, from the oldest the channel still holds: nothing is lost once `get_journal` is asked
    // again, since the buffer keeps them all.
    let Some(FeedItem::Entry(next)) = feed.next().await else {
        panic!("the feed goes on after a resynchronisation");
    };
    assert!(next["seq"].as_u64().unwrap() > 2);
    assert_eq!(journal.snapshot(&JournalFilter::default()).len(), 1001);

    drop(journal);
    let mut rest = 0;
    while let Some(item) = feed.next().await {
        assert!(matches!(item, FeedItem::Entry(_)));
        rest += 1;
    }
    assert!(
        rest > 0,
        "the entries already sent are still read once the journal is gone"
    );
}
