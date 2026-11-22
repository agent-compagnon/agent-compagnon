//! Spec 009 through the whole gateway: the chain gives its records to the real journal, which keeps, sends and
//! writes them; the list watcher's changes are journaled; the diagnostic archive is built from the stack's logs.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{Stack, TOKEN, connect, exposed, instance, params, stack};
use mcp_proxy_core::diagnostics::{DiagnosticState, Diagnostics};
use mcp_proxy_core::filter::SECRET_MARKER;
use mcp_proxy_core::secrets::SecretStore;
use mcp_proxy_core::trace::{CallStatus, EntryKind, JournalEntry, JournalFilter, SystemWallClock};
use mcp_proxy_gateway::journal::instance_lines;
use mcp_proxy_gateway::updates::WatchTiming;
use mcp_proxy_testkit::{Generation, catalog_tools};
use rmcp::ClientHandler;
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, Tool};
use rmcp::service::{RoleClient, RunningService};
use serde_json::{Value, json};

#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("client-de-test", "1.0.0"),
        )
    }
}

async fn call(client: &RunningService<RoleClient, Client>, tool: &str, arguments: Value) {
    client.call_tool(params(&exposed(tool), arguments)).await.unwrap();
}

fn variants(store: &SecretStore) -> Vec<Vec<u8>> {
    store
        .with_variants(|v| v.iter().filter(|x| !x.is_empty()).map(|x| x.to_vec()).collect())
        .unwrap()
}

#[track_caller]
fn assert_no_variant(store: &SecretStore, what: &str, haystack: &[u8]) {
    let variants = variants(store);
    assert!(variants.len() > 5, "{} variants only", variants.len());
    for variant in &variants {
        assert!(
            memchr_find(haystack, variant).is_none(),
            "{what} contains a variant: {}",
            String::from_utf8_lossy(variant)
        );
    }
}

fn memchr_find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn unzip(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut content = Vec::new();
            file.read_to_end(&mut content).unwrap();
            (file.name().unwrap().into_owned(), content)
        })
        .collect()
}

fn diagnostics(stack: &Stack) -> Diagnostics {
    Diagnostics {
        version: "0.1.0".to_owned(),
        system: "test".to_owned(),
        config: Arc::clone(&stack.config),
        log_dir: stack.logs.dir.clone(),
        filter: Arc::clone(stack.output.filter()),
        clock: Arc::new(SystemWallClock),
        log: Some(stack.logs.log.clone()),
    }
}

fn entries(stack: &Stack) -> Vec<JournalEntry> {
    stack.journal.snapshot(&JournalFilter::default())
}

// Exigence : EX-009-02, EX-009-03, EX-009-07
#[tokio::test(flavor = "multi_thread")]
async fn a_secret_of_a_real_call_stays_out_of_the_buffer_the_file_and_the_archive() {
    let stack = stack(Generation::Both).await;
    let mut events = stack.journal.subscribe();
    let (client, _connection) = connect(&stack.gateway, Client, Generation::Modern).await;
    // Blocked: the token in the arguments. Leak masked: the server echoes the Authorization header.
    call(&client, "search_logs", json!({ "q": TOKEN })).await;
    call(&client, "echo_headers", json!({})).await;
    call(&client, "fail", json!({})).await;
    call(&client, "search_logs", json!({ "q": "visible-normal-word" })).await;

    let entries = entries(&stack);
    let statuses: Vec<_> = entries.iter().map(|e| e.status).collect();
    assert_eq!(
        statuses,
        [
            Some(CallStatus::Blocked),
            Some(CallStatus::LeakMasked),
            Some(CallStatus::LeakMasked),
            Some(CallStatus::Ok)
        ]
    );
    let store = stack.gate.store();
    assert_no_variant(store, "buffer", format!("{entries:?}").as_bytes());
    let mut sent = Vec::new();
    while let Ok(entry) = events.try_recv() {
        sent.push(entry);
    }
    assert_eq!(sent, entries, "subscribers get the same entries");

    let blocked = entries[0].details.as_ref().unwrap();
    assert_eq!(blocked.request, json!({ "q": SECRET_MARKER }));
    let leaked = entries[1].details.as_ref().unwrap();
    assert!(!leaked.response.to_string().contains(TOKEN), "{}", leaked.response);
    assert!(leaked.response.to_string().contains("MASQUÉ"), "{}", leaked.response);
    assert!(entries[3].details.is_none(), "a normal call keeps no content");

    let file = stack.logs.text();
    let lines: Vec<&str> = file.lines().filter(|l| l.contains(" mcp.call ")).collect();
    assert_eq!(lines.len(), 4, "{file}");
    assert!(
        lines[0].contains("status=blocked") && lines[0].contains("request={"),
        "{}",
        lines[0]
    );
    assert!(
        lines[3].contains("status=ok") && !lines[3].contains("visible-normal-word"),
        "{}",
        lines[3]
    );
    assert_no_variant(store, "log file", file.as_bytes());

    let dir = stack.logs.dir.join("export");
    fs::create_dir_all(&dir).unwrap();
    let archive = diagnostics(&stack).export(&dir, &DiagnosticState::default()).unwrap();
    let files = unzip(&archive);
    assert!(
        files.iter().any(|(n, _)| n.starts_with("logs/mcp-proxy-")),
        "{:?}",
        files.len()
    );
    for (name, content) in &files {
        assert_no_variant(store, name, content);
    }
}

fn tool(name: &str, description: &str) -> Tool {
    serde_json::from_value(json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": {} }
    }))
    .unwrap()
}

// Exigence : EX-009-10, EX-009-07
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_list_change_is_journaled_and_instances_go_to_the_archive() {
    let stack = stack(Generation::Both).await;
    let mut tools = catalog_tools("kibana-logs").unwrap();
    let removed = tools.pop().unwrap().name.to_string();
    let modified = tools[0].name.to_string();
    tools[0] = tool(&modified, "Description changée.");
    tools.push(tool("purge_cache", "Purges the cache."));
    stack.server.set_tools(tools, false).await;

    let mut events = stack.journal.subscribe();
    // The first periodic reload is immediate.
    let watcher = stack.watch(WatchTiming::from_config(&stack.config));
    let entry = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let entry = events.recv().await.unwrap();
            if entry.kind == EntryKind::ToolsChanged {
                return entry;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(entry.env, Some(instance("dev").env));
    assert_eq!(entry.mcp, Some(instance("dev").mcp));
    assert!(entry.summary.contains("1 nouveau (purge_cache)"), "{}", entry.summary);
    assert!(
        entry.summary.contains(&format!("1 modifié ({modified})")),
        "{}",
        entry.summary
    );
    assert!(
        entry.summary.contains(&format!("1 supprimé ({removed})")),
        "{}",
        entry.summary
    );
    assert!(
        stack
            .logs
            .text()
            .contains("mcp.tools env=dev mcp=kibana-logs added=1 modified=1 removed=1")
    );

    let lines = instance_lines(watcher.tracker());
    let dev = lines.iter().find(|l| l.starts_with("dev/kibana-logs : ")).unwrap();
    assert!(
        dev.starts_with("dev/kibana-logs : connecté, dernier rechargement 20"),
        "{dev}"
    );
    let state = DiagnosticState {
        instances: lines,
        ..DiagnosticState::default()
    };
    let archive = diagnostics(&stack)
        .export(&stack.logs.dir.join("archive.zip"), &state)
        .unwrap();
    let files = unzip(&archive);
    let instances = &files.iter().find(|(n, _)| n == "instances.txt").unwrap().1;
    assert!(String::from_utf8_lossy(instances).contains("dev/kibana-logs : connecté"));
}
