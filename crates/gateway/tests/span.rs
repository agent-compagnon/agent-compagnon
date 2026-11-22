//! The `mcp.call` span of each call.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use common::{TOKEN, connect, exposed, params, stack};
use mcp_proxy_core::filter::SECRET_MARKER;
use mcp_proxy_testkit::{Generation, set_thread_subscriber};
use rmcp::ClientHandler;
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::{RoleClient, RunningService};
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

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

/// Fields of every `mcp.call` span, as recorded.
#[derive(Clone, Default)]
struct Spans {
    by_id: Arc<Mutex<HashMap<u64, usize>>>,
    spans: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
}

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Spans {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _: Context<'_, S>) {
        if attrs.metadata().name() != "mcp.call" {
            return;
        }
        let mut fields = BTreeMap::new();
        attrs.record(&mut Fields(&mut fields));
        let mut spans = self.spans.lock().unwrap();
        spans.push(fields);
        self.by_id.lock().unwrap().insert(id.into_u64(), spans.len() - 1);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _: Context<'_, S>) {
        if let Some(index) = self.by_id.lock().unwrap().get(&id.into_u64()) {
            values.record(&mut Fields(&mut self.spans.lock().unwrap()[*index]));
        }
    }
}

// Exigence : EX-009-09
#[tokio::test(flavor = "current_thread")]
async fn each_call_has_an_mcp_call_span_with_filtered_fields() {
    let spans = Spans::default();
    // Current thread: every task of the test runs under this subscriber.
    let _default = set_thread_subscriber(tracing_subscriber::registry().with(spans.clone()));
    let stack = stack(Generation::Both).await;
    let (client, _connection) = connect(&stack.gateway, Client, Generation::Legacy).await;
    call(&client, "search_logs", json!({ "q": "x" })).await;
    call(&client, "search_logs", json!({ "q": TOKEN })).await;
    client
        .call_tool(params(&format!("dev_kibana_{TOKEN}"), json!({})))
        .await
        .unwrap();

    let recorded = spans.spans.lock().unwrap().clone();
    assert_eq!(recorded.len(), 3, "{recorded:?}");
    let ok = &recorded[0];
    let expect = |span: &BTreeMap<String, String>, field: &str, value: &str| {
        assert_eq!(span.get(field).map(String::as_str), Some(value), "{field}: {span:?}");
    };
    expect(ok, "client", "client-de-test");
    expect(ok, "tool", "dev_kibana_search_logs");
    expect(ok, "env", "dev");
    expect(ok, "mcp", "kibana-logs");
    expect(ok, "session", "sso-dev");
    expect(ok, "policy", "allow");
    expect(ok, "status", "ok");
    assert!(ok["call_id"].parse::<u64>().is_ok(), "{ok:?}");
    assert!(ok["duration_ms"].parse::<u64>().is_ok(), "{ok:?}");
    expect(&recorded[1], "status", "blocked");
    let unknown = &recorded[2];
    expect(unknown, "status", "denied");
    expect(unknown, "tool", &format!("dev_kibana_{SECRET_MARKER}"));
    assert!(!unknown.contains_key("env"), "not resolved: {unknown:?}");
    assert!(!format!("{recorded:?}").contains(TOKEN), "{recorded:?}");
}
