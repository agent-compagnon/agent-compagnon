//! Tool list updates, end to end: fake MCP server → `HttpUpstream` → `ListWatcher` → registry → LLM clients.
//!
//! The tokio clock is paused and frozen: it only moves with `tokio::time::advance`, even while the network is
//! awaited, so the periodic reload and the debounce are driven by the test. Waits for the network use real time.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::{Stack, TOKEN, builder, connect, exposed, instance, params, session, text_of};
use mcp_proxy_core::config::{EnvId, McpId, ProfileId};
use mcp_proxy_core::confirm::ConfirmationOutcome;
use mcp_proxy_core::secrets::Credential;
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_gateway::registry::{CacheError, CachedCatalog, CatalogCache, InstanceId, ToolDiff};
use mcp_proxy_gateway::updates::{InstanceState, ListWatcher, ReloadOutcome, UpdateEvent, WatchTiming};
use mcp_proxy_gateway::upstream::Upstream;
use mcp_proxy_testkit::{Generation, catalog_tools};
use rmcp::ClientHandler;
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, ServerNotification, SubscriptionFilter, Tool};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use secrecy::SecretString;
use serde_json::json;
use tokio::sync::{broadcast, oneshot};

const DEBOUNCE: Duration = WatchTiming::DEFAULT_DEBOUNCE;
const INTERVAL: Duration = Duration::from_secs(300);

fn timing() -> WatchTiming {
    WatchTiming {
        refresh_interval: INTERVAL,
        debounce: DEBOUNCE,
        min_refresh: WatchTiming::DEFAULT_MIN_REFRESH,
    }
}

/// While alive, the paused tokio clock does not auto-advance: a running blocking task inhibits it.
struct FrozenClock(Arc<AtomicBool>);

fn freeze() -> FrozenClock {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    tokio::task::spawn_blocking(move || {
        while !flag.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    FrozenClock(stop)
}

impl Drop for FrozenClock {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// `fut`, or `None` after `limit` of real time.
async fn real_timeout<F: Future>(limit: Duration, fut: F) -> Option<F::Output> {
    let (sender, receiver) = oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(limit);
        let _ = sender.send(());
    });
    tokio::select! {
        output = fut => Some(output),
        _ = receiver => None,
    }
}

async fn within<F: Future>(what: &str, fut: F) -> F::Output {
    real_timeout(Duration::from_secs(10), fut)
        .await
        .unwrap_or_else(|| panic!("{what}: nothing after 10 s"))
}

/// Lets the tasks run for `millis` of real time, the tokio clock staying where it is.
async fn settle(millis: u64) {
    real_timeout(Duration::from_millis(millis), std::future::pending::<()>()).await;
}

async fn next_reload(events: &mut broadcast::Receiver<UpdateEvent>) -> ReloadOutcome {
    within("reload", async {
        loop {
            if let Ok(UpdateEvent::Reloaded { outcome, .. }) = events.recv().await {
                return outcome;
            }
        }
    })
    .await
}

/// No reload ended since the last look.
fn no_reload(events: &mut broadcast::Receiver<UpdateEvent>) {
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, UpdateEvent::Reloaded { .. }), "unexpected {event:?}");
    }
}

/// Advances the clock by `step` until a reload ends: for signals whose arrival time the test cannot know.
async fn reload_after_steps(events: &mut broadcast::Receiver<UpdateEvent>, step: Duration) -> ReloadOutcome {
    for _ in 0..20 {
        tokio::time::advance(step).await;
        if let Some(outcome) = real_timeout(Duration::from_millis(300), next_reload(events)).await {
            return outcome;
        }
    }
    panic!("no reload");
}

fn tool(name: &str, description: &str) -> Tool {
    serde_json::from_value(json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": {} }
    }))
    .unwrap()
}

/// The snapshot of `kibana-logs` plus `purge_cache`, declared nowhere in the configuration.
fn with_new_tool() -> Vec<Tool> {
    let mut tools = catalog_tools("kibana-logs").unwrap();
    tools.push(tool("purge_cache", "Purges the cache."));
    tools
}

fn diff(added: &[&str], modified: &[&str], removed: &[&str]) -> ToolDiff {
    let names = |list: &[&str]| {
        list.iter()
            .map(|n| mcp_proxy_core::config::ToolName::new(*n).unwrap())
            .collect()
    };
    ToolDiff {
        added: names(added),
        removed: names(removed),
        modified: names(modified),
    }
}

/// Changes the tools of the server with a notification, until the upstream hears one: the channel the
/// notification goes through (legacy GET stream, 2026-07-28 subscription) may not be open yet.
async fn notify(stack: &Stack, tools: Vec<Tool>) {
    let mut heard = stack.upstream.changes();
    for _ in 0..50 {
        stack.server.set_tools(tools.clone(), true).await;
        if real_timeout(Duration::from_millis(200), heard.recv()).await.is_some() {
            return;
        }
    }
    panic!("the upstream never heard of the change");
}

/// An LLM client that counts the list changes it is told about.
#[derive(Clone, Default)]
struct Recorder {
    list_changed: Arc<AtomicUsize>,
    elicitation: bool,
}

impl ClientHandler for Recorder {
    fn get_info(&self) -> ClientConfig {
        let capabilities = if self.elicitation {
            ClientCapabilities::builder().enable_elicitation().build()
        } else {
            ClientCapabilities::default()
        };
        ClientConfig::new(capabilities, Implementation::new("client-de-test", "4.2.0"))
    }

    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.list_changed.fetch_add(1, Ordering::SeqCst);
    }
}

async fn listed(client: &RunningService<RoleClient, Recorder>) -> Vec<Tool> {
    client.peer().clear_response_cache().await;
    client.peer().list_all_tools().await.unwrap()
}

/// A stack, its watcher, and the end of the first reload (immediate, the upstream being connected).
async fn watched(stack: &Stack) -> (ListWatcher, broadcast::Receiver<UpdateEvent>) {
    let watcher = stack.watch(timing());
    let mut events = watcher.tracker().subscribe();
    assert_eq!(next_reload(&mut events).await, ReloadOutcome::Unchanged);
    (watcher, events)
}

/// A server notification reloads the list once and the clients of both generations are told.
async fn a_notified_change_reaches_the_clients(server_generation: Generation) {
    let _clock = freeze();
    let stack = builder(server_generation).build().await;
    let (watcher, mut events) = watched(&stack).await;

    let legacy = Recorder::default();
    let (legacy_client, _c1) = connect(&stack.gateway, legacy.clone(), Generation::Legacy).await;
    let (modern_client, _c2) = connect(&stack.gateway, Recorder::default(), Generation::Modern).await;
    let mut subscription = modern_client
        .peer()
        .listen(SubscriptionFilter::builder().tools_list_changed().build())
        .await
        .unwrap();

    notify(&stack, with_new_tool()).await;
    let lists = stack.server.list_requests();
    settle(100).await;
    no_reload(&mut events);
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        next_reload(&mut events).await,
        ReloadOutcome::Changed(diff(&["purge_cache"], &[], &[]))
    );
    assert_eq!(stack.server.list_requests(), lists + 1);
    assert_eq!(
        watcher.tracker().state(&instance("dev")),
        Some(InstanceState::Connected)
    );

    let notification = within("2026-07-28 client", subscription.next()).await.unwrap();
    assert!(matches!(
        notification,
        Some(ServerNotification::ToolListChangedNotification(_))
    ));
    within("legacy client", async {
        while legacy.list_changed.load(Ordering::SeqCst) == 0 {
            settle(10).await;
        }
    })
    .await;
    for client in [&legacy_client, &modern_client] {
        let tools = listed(client).await;
        assert!(tools.iter().any(|t| t.name == exposed("purge_cache")));
    }
}

// Exigence : EX-008-01
#[tokio::test(start_paused = true)]
async fn a_legacy_notification_reloads_the_list_and_tells_the_clients() {
    a_notified_change_reaches_the_clients(Generation::Legacy).await;
}

// Exigence : EX-008-02
#[tokio::test(start_paused = true)]
async fn a_2026_07_28_subscription_reloads_the_list_and_tells_the_clients() {
    let _clock = freeze();
    {
        // The subscription is the one the upstream holds open: the server saw a `subscriptions/listen`.
        let stack = builder(Generation::Modern).build().await;
        let (_watcher, _events) = watched(&stack).await;
        within("subscription", async {
            while !stack.server.mcp_methods().contains(&"subscriptions/listen".to_owned()) {
                settle(10).await;
            }
        })
        .await;
        assert!(!stack.server.mcp_methods().contains(&"initialize".to_owned()));
    }
    a_notified_change_reaches_the_clients(Generation::Modern).await;
}

// Exigence : EX-008-03
#[tokio::test(start_paused = true)]
async fn a_silent_server_is_reloaded_periodically() {
    let _clock = freeze();
    for generation in [Generation::Legacy, Generation::Modern] {
        let stack = builder(generation).build().await;
        let (_watcher, mut events) = watched(&stack).await;
        stack.server.set_tools(with_new_tool(), false).await;
        let lists = stack.server.list_requests();

        tokio::time::advance(INTERVAL - Duration::from_secs(1)).await;
        settle(200).await;
        no_reload(&mut events);
        assert_eq!(stack.server.list_requests(), lists, "{generation:?}");

        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            next_reload(&mut events).await,
            ReloadOutcome::Changed(diff(&["purge_cache"], &[], &[])),
            "{generation:?}"
        );
        // A legacy session of the fake server expires after 300 s idle, like the period: the reload may find it
        // expired and list again after a transparent re-initialization.
        assert!(stack.server.list_requests() > lists, "{generation:?}");
        assert!(stack.registry.read().await.resolve(&exposed("purge_cache")).is_some());
    }
}

// Exigence : EX-008-04
#[tokio::test(start_paused = true)]
async fn a_connection_and_a_reconnection_reload_the_list() {
    let _clock = freeze();
    for generation in [Generation::Legacy, Generation::Modern] {
        // Connection: no periodic reload without an active session; the first call connects and reloads.
        let mut stack = builder(generation).without_initial_refresh().build().await;
        stack.server.set_tools(with_new_tool(), false).await;
        stack.gate.store().wipe(&session());
        let watcher = stack.watch(timing());
        let mut events = watcher.tracker().subscribe();
        settle(200).await;
        no_reload(&mut events);
        assert_eq!(stack.server.list_requests(), 0, "{generation:?}");
        assert_eq!(
            watcher.tracker().state(&instance("dev")),
            Some(InstanceState::Catalogue)
        );

        stack
            .gate
            .store()
            .put(
                &session(),
                Credential::Bearer {
                    token: SecretString::from(TOKEN.to_owned()),
                },
            )
            .unwrap();
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), Generation::Legacy).await;
        let result = client
            .call_tool(params(&exposed("search_logs"), json!({})))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
        let ReloadOutcome::Changed(first) = reload_after_steps(&mut events, DEBOUNCE).await else {
            panic!("{generation:?}: the connection did not reload the list");
        };
        assert!(first.added.iter().any(|t| t.as_str() == "purge_cache"), "{first:?}");
        assert_eq!(
            watcher.tracker().state(&instance("dev")),
            Some(InstanceState::Connected)
        );

        // Reconnection: the server restarts with another list and notifies nobody.
        let mut tools = with_new_tool();
        tools.push(tool("rotate_index", "Rotates an index."));
        stack.restart_server(tools).await;
        // Legacy: the next request finds its session expired and initializes again. 2026-07-28: the subscription
        // ended and is opened again after a delay.
        let _ = client
            .call_tool(params(&exposed("search_logs"), json!({})))
            .await
            .unwrap();
        let outcome = reload_after_steps(&mut events, DEBOUNCE).await;
        assert_eq!(
            outcome,
            ReloadOutcome::Changed(diff(&["rotate_index"], &[], &[])),
            "{generation:?}"
        );
    }
}

/// Counts the writes of the cache.
#[derive(Clone, Default)]
struct CountingCache(Arc<AtomicUsize>);

impl CatalogCache for CountingCache {
    fn load(&self, _: &InstanceId) -> Result<Option<CachedCatalog>, CacheError> {
        Ok(None)
    }

    fn store(&self, _: &InstanceId, _: &CachedCatalog) -> Result<(), CacheError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// Exigence : EX-008-05
#[tokio::test(start_paused = true)]
async fn the_same_fingerprint_changes_nothing() {
    let _clock = freeze();
    let cache = CountingCache::default();
    let stack = builder(Generation::Legacy).cache(cache.clone()).build().await;
    // The first live list replaced the embedded snapshot: it was written once.
    assert_eq!(cache.0.load(Ordering::SeqCst), 1);
    let mut registry_events = stack.registry.read().await.subscribe();
    let (watcher, mut events) = watched(&stack).await;

    let legacy = Recorder::default();
    let (_legacy_client, _c1) = connect(&stack.gateway, legacy.clone(), Generation::Legacy).await;
    let (modern_client, _c2) = connect(&stack.gateway, Recorder::default(), Generation::Modern).await;
    let mut subscription = modern_client
        .peer()
        .listen(SubscriptionFilter::builder().tools_list_changed().build())
        .await
        .unwrap();

    // Same tools, in another order: same fingerprint.
    let mut tools = catalog_tools("kibana-logs").unwrap();
    tools.reverse();
    notify(&stack, tools).await;
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(next_reload(&mut events).await, ReloadOutcome::Unchanged);
    tokio::time::advance(INTERVAL).await;
    assert_eq!(next_reload(&mut events).await, ReloadOutcome::Unchanged);

    settle(300).await;
    assert_eq!(cache.0.load(Ordering::SeqCst), 1);
    assert!(registry_events.try_recv().is_err());
    assert_eq!(legacy.list_changed.load(Ordering::SeqCst), 0);
    assert!(
        real_timeout(Duration::from_millis(300), subscription.next())
            .await
            .is_none()
    );
    assert!(watcher.tracker().pending().is_empty());
}

// Exigence : EX-008-06
#[tokio::test(start_paused = true)]
async fn a_new_tool_is_never_allowed_by_default() {
    let _clock = freeze();
    // A client trusted for elicitation: the destructive tools of DEV would be confirmed in the chat, but an unknown
    // tool is raised to the Windows notification by `unknown_tools`.
    let stack = builder(Generation::Both)
        .config(|c| c.gateway.elicitation_clients.push("client-de-test".to_owned()))
        .build()
        .await;
    let (watcher, mut events) = watched(&stack).await;
    let (client, _connection) = connect(
        &stack.gateway,
        Recorder {
            elicitation: true,
            ..Recorder::default()
        },
        Generation::Legacy,
    )
    .await;
    notify(&stack, with_new_tool()).await;
    tokio::time::advance(DEBOUNCE).await;
    next_reload(&mut events).await;

    {
        let registry = stack.registry.read().await;
        let entry = registry.resolve(&exposed("purge_cache")).unwrap();
        assert!(!entry.known);
        assert_eq!(entry.category, mcp_proxy_core::config::Category::Destructive);
    }
    let listed_tool = listed(&client)
        .await
        .into_iter()
        .find(|t| t.name == exposed("purge_cache"))
        .unwrap();
    let hints = listed_tool.annotations.unwrap();
    assert_eq!(hints.destructive_hint, Some(true));
    assert_eq!(hints.read_only_hint, Some(false));
    assert_eq!(
        watcher.tracker().pending_for(&instance("dev")).unwrap().added,
        diff(&["purge_cache"], &[], &[]).added
    );

    // Profile `developpeur`: confirmation by notification; no answer is a refusal.
    let result = client
        .call_tool(params(&exposed("purge_cache"), json!({})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(
        text_of(&result).contains("refusé par l'utilisateur"),
        "{}",
        text_of(&result)
    );
    assert_eq!(stack.broker.requests().len(), 1);
    assert!(stack.broker.requests()[0].path.contains("purge_cache"));
    let reached = |stack: &Stack| {
        stack
            .server
            .requests()
            .iter()
            .filter(|r| r.tool.as_deref() == Some("purge_cache"))
            .count()
    };
    assert_eq!(reached(&stack), 0);
    stack.broker.script([ConfirmationOutcome::Approved]);
    let result = client
        .call_tool(params(&exposed("purge_cache"), json!({})))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(reached(&stack), 1);

    // Profile `lecture-seule`: forbidden, nobody is asked.
    let config = Arc::clone(&stack.config);
    let mut strict = UserSettings::default();
    strict.choose_profile(&config.profiles[&ProfileId::new("lecture-seule").unwrap()]);
    strict.exposed_environments = [EnvId::new("dev").unwrap()].into();
    stack.registry.write().await.apply_settings(&strict);
    stack.settings.send_replace(strict);
    let result = client
        .call_tool(params(&exposed("purge_cache"), json!({})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(
        text_of(&result).contains("interdit par la politique"),
        "{}",
        text_of(&result)
    );
    assert_eq!(stack.broker.requests().len(), 2);
    assert_eq!(reached(&stack), 1);
}

// Exigence : EX-008-07
#[tokio::test(start_paused = true)]
async fn a_changed_description_is_reported_and_keeps_its_policy() {
    let _clock = freeze();
    let stack = builder(Generation::Both).build().await;
    let (watcher, mut events) = watched(&stack).await;
    let before = stack
        .registry
        .read()
        .await
        .resolve(&exposed("search_logs"))
        .cloned()
        .unwrap();

    let mut tools = catalog_tools("kibana-logs").unwrap();
    let search = tools.iter_mut().find(|t| t.name == "search_logs").unwrap();
    search.description = Some("Searches the logs, now with regular expressions.".into());
    notify(&stack, tools).await;
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        next_reload(&mut events).await,
        ReloadOutcome::Changed(diff(&[], &["search_logs"], &[]))
    );
    let update = watcher.tracker().pending_for(&instance("dev")).unwrap();
    assert_eq!(update.modified, diff(&[], &["search_logs"], &[]).modified);
    assert!(update.added.is_empty() && update.removed.is_empty());

    let after = stack
        .registry
        .read()
        .await
        .resolve(&exposed("search_logs"))
        .cloned()
        .unwrap();
    assert!(after.description.contains("regular expressions"));
    assert_ne!(after.fingerprint, before.fingerprint);
    assert_eq!(
        (
            after.known,
            after.category,
            after.category_source,
            after.annotations.clone()
        ),
        (
            before.known,
            before.category,
            before.category_source,
            before.annotations.clone()
        )
    );
    // Still allowed without confirmation.
    let (client, _connection) = connect(&stack.gateway, Recorder::default(), Generation::Modern).await;
    let result = client
        .call_tool(params(&exposed("search_logs"), json!({})))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert!(stack.broker.requests().is_empty());
}

// Exigence : EX-008-08
#[tokio::test(start_paused = true)]
async fn a_removed_tool_leaves_the_list_and_is_refused() {
    let _clock = freeze();
    let stack = builder(Generation::Both).build().await;
    let (watcher, mut events) = watched(&stack).await;
    let mut tools = catalog_tools("kibana-logs").unwrap();
    tools.retain(|t| t.name != "search_logs");
    notify(&stack, tools).await;
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        next_reload(&mut events).await,
        ReloadOutcome::Changed(diff(&[], &[], &["search_logs"]))
    );
    assert_eq!(
        watcher.tracker().pending_for(&instance("dev")).unwrap().removed,
        diff(&[], &[], &["search_logs"]).removed
    );
    for generation in [Generation::Legacy, Generation::Modern] {
        let (client, _connection) = connect(&stack.gateway, Recorder::default(), generation).await;
        assert!(listed(&client).await.iter().all(|t| t.name != exposed("search_logs")));
        let result = client
            .call_tool(params(&exposed("search_logs"), json!({})))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(
            text_of(&result).contains("inconnu ou non exposé"),
            "{}",
            text_of(&result)
        );
    }
    assert!(
        stack
            .server
            .requests()
            .iter()
            .all(|r| r.tool.as_deref() != Some("search_logs"))
    );
}

// Exigence : EX-008-09
#[tokio::test(start_paused = true)]
async fn a_failed_reload_keeps_the_list_and_marks_the_instance_unreachable() {
    let _clock = freeze();
    for generation in [Generation::Legacy, Generation::Modern] {
        let stack = builder(generation).build().await;
        let (watcher, mut events) = watched(&stack).await;
        let legacy = Recorder::default();
        let (_client, _connection) = connect(&stack.gateway, legacy.clone(), Generation::Legacy).await;
        let fingerprint = stack
            .registry
            .read()
            .await
            .instance_fingerprint(&instance("dev"))
            .cloned();

        stack.server.set_tools(with_new_tool(), false).await;
        stack.server.set_unavailable(true);
        assert!(watcher.reload(&instance("dev")));
        assert_eq!(
            next_reload(&mut events).await,
            ReloadOutcome::Failed(InstanceState::Unreachable),
            "{generation:?}"
        );
        let status = watcher.tracker().status(&instance("dev")).unwrap();
        assert_eq!(status.state, InstanceState::Unreachable);
        let message = status.last_error.unwrap();
        assert!(!message.contains(TOKEN), "{message}");
        assert_eq!(
            stack
                .registry
                .read()
                .await
                .instance_fingerprint(&instance("dev"))
                .cloned(),
            fingerprint
        );
        assert!(stack.registry.read().await.resolve(&exposed("search_logs")).is_some());
        settle(200).await;
        assert_eq!(legacy.list_changed.load(Ordering::SeqCst), 0);

        // The next successful reload, periodic here, brings the instance back.
        stack.server.set_unavailable(false);
        tokio::time::advance(INTERVAL).await;
        assert_eq!(
            next_reload(&mut events).await,
            ReloadOutcome::Changed(diff(&["purge_cache"], &[], &[])),
            "{generation:?}"
        );
        let status = watcher.tracker().status(&instance("dev")).unwrap();
        assert_eq!(status.state, InstanceState::Connected);
        assert!(status.last_error.is_none());
    }
}

// Exigence : EX-008-10
#[tokio::test(start_paused = true)]
async fn acknowledge_clears_the_pending_update() {
    let _clock = freeze();
    let stack = builder(Generation::Both).build().await;
    let (watcher, mut events) = watched(&stack).await;
    let tracker = watcher.tracker().clone();
    assert!(tracker.pending().is_empty());
    assert!(!tracker.acknowledge(&instance("dev")));

    notify(&stack, with_new_tool()).await;
    tokio::time::advance(DEBOUNCE).await;
    next_reload(&mut events).await;
    let pending = tracker.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].instance, instance("dev"));

    let mut ui = tracker.subscribe();
    assert!(tracker.acknowledge(&instance("dev")));
    assert_eq!(
        ui.try_recv().unwrap(),
        UpdateEvent::Acknowledged {
            instance: instance("dev")
        }
    );
    assert!(tracker.pending().is_empty());
    assert!(tracker.pending_for(&instance("dev")).is_none());
    assert!(!tracker.acknowledge(&instance("dev")));
    // The tool itself stays, still unknown.
    assert!(
        !stack
            .registry
            .read()
            .await
            .resolve(&exposed("purge_cache"))
            .unwrap()
            .known
    );
}

// Exigence : EX-008-11
#[tokio::test(start_paused = true)]
async fn a_list_is_reloaded_when_its_ttl_expires() {
    let _clock = freeze();
    let stack = builder(Generation::Modern)
        .server(|b| b.list_ttl(Duration::from_secs(60)))
        .build()
        .await;
    let (_watcher, mut events) = watched(&stack).await;
    // rmcp gives the time left on the list, measured on the real clock.
    let ttl = stack.upstream.list_ttl().unwrap();
    assert!(
        ttl > Duration::from_secs(59) && ttl <= Duration::from_secs(60),
        "{ttl:?}"
    );
    let lists = stack.server.list_requests();
    tokio::time::advance(Duration::from_secs(59)).await;
    settle(200).await;
    no_reload(&mut events);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(next_reload(&mut events).await, ReloadOutcome::Unchanged);
    // Read from the server, not from the client cache of rmcp.
    assert_eq!(stack.server.list_requests(), lists + 1);
}

// Exigence : EX-008-12
#[tokio::test(start_paused = true)]
async fn a_burst_of_notifications_gives_one_reload() {
    let _clock = freeze();
    for generation in [Generation::Legacy, Generation::Modern] {
        let stack = builder(generation).build().await;
        let (_watcher, mut events) = watched(&stack).await;
        let lists = stack.server.list_requests();
        notify(&stack, with_new_tool()).await;
        let mut heard = stack.upstream.changes();
        for _ in 0..4 {
            stack.server.set_tools(with_new_tool(), true).await;
            within("notification", heard.recv()).await.unwrap();
        }
        tokio::time::advance(DEBOUNCE).await;
        assert!(matches!(next_reload(&mut events).await, ReloadOutcome::Changed(_)));
        tokio::time::advance(DEBOUNCE * 4).await;
        settle(300).await;
        no_reload(&mut events);
        assert_eq!(stack.server.list_requests(), lists + 1, "{generation:?}");
    }
}

// Exigence : EX-008-13
#[tokio::test(start_paused = true)]
async fn periodic_reloads_never_trigger_a_login() {
    let _clock = freeze();
    // Never connected, no active session: the server is never contacted, nobody is asked to log in.
    let stack = builder(Generation::Both).without_initial_refresh().build().await;
    stack.gate.store().wipe(&session());
    let watcher = stack.watch(timing());
    let mut events = watcher.tracker().subscribe();
    for _ in 0..3 {
        tokio::time::advance(INTERVAL).await;
        settle(100).await;
    }
    no_reload(&mut events);
    assert!(stack.server.requests().is_empty());
    assert_eq!(stack.gate.ensure_count(), 0);
    assert_eq!(
        watcher.tracker().state(&instance("dev")),
        Some(InstanceState::Catalogue)
    );
    drop(watcher);

    // Connected, but the session expired since: the reload stops at `SessionRequired`, without a login.
    let stack = builder(Generation::Both).build().await;
    let (watcher, mut events) = watched(&stack).await;
    stack.gate.store().wipe(&session());
    let requests = stack.server.requests().len();
    tokio::time::advance(INTERVAL).await;
    assert_eq!(
        next_reload(&mut events).await,
        ReloadOutcome::Failed(InstanceState::SessionRequired)
    );
    assert_eq!(stack.server.requests().len(), requests);
    assert_eq!(stack.gate.ensure_count(), 0);
    let status = watcher.tracker().status(&instance("dev")).unwrap();
    assert_eq!(status.state, InstanceState::SessionRequired);
    assert!(status.last_error.unwrap().contains("sso-dev"));

    // An anonymous list of an exposed instance is reloaded even without a session.
    let stack = builder(Generation::Both)
        .config(|c| {
            let kibana = c.mcp.iter_mut().find(|m| m.id.as_str() == "kibana-logs").unwrap();
            kibana.list_requires_auth = false;
        })
        .server(|b| b.anonymous())
        .without_initial_refresh()
        .build()
        .await;
    stack.gate.store().wipe(&session());
    let watcher = stack.watch(timing());
    let mut events = watcher.tracker().subscribe();
    assert!(matches!(next_reload(&mut events).await, ReloadOutcome::Changed(_)));
    assert_eq!(
        watcher.tracker().state(&instance("dev")),
        Some(InstanceState::Connected)
    );
    assert_eq!(stack.server.list_requests(), 1);
    assert!(
        stack
            .server
            .requests()
            .iter()
            .all(|r| !r.headers.contains_key("authorization"))
    );
    assert_eq!(stack.gate.ensure_count(), 0);
}

// Exigence : EX-008-14
#[tokio::test(start_paused = true)]
async fn instance_states_are_exposed_and_disabled_mcps_are_left_alone() {
    let _clock = freeze();
    let stack = builder(Generation::Both).build().await;
    let (watcher, mut events) = watched(&stack).await;
    let tracker = watcher.tracker().clone();
    let statuses = tracker.statuses();
    // Every instance of the configuration, never reloaded ones in `Catalogue`.
    assert_eq!(statuses.len(), stack.registry.read().await.instances().count());
    let prod = InstanceId::new(EnvId::new("prod").unwrap(), McpId::new("kibana-logs").unwrap());
    assert_eq!(tracker.state(&prod), Some(InstanceState::Catalogue));
    assert_eq!(tracker.state(&instance("dev")), Some(InstanceState::Connected));
    assert!(tracker.status(&instance("dev")).unwrap().refreshed_at.is_some());
    assert_eq!(InstanceState::Unreachable.label(), "injoignable");

    let mut ui = tracker.subscribe();
    let mut settings = stack.settings.borrow().clone();
    settings.disabled_mcp.insert(McpId::new("kibana-logs").unwrap());
    stack.settings.send_replace(settings.clone());
    settle(100).await;
    assert_eq!(tracker.state(&instance("dev")), Some(InstanceState::Disabled));
    assert_eq!(tracker.state(&prod), Some(InstanceState::Disabled));
    assert!(matches!(
        ui.try_recv().unwrap(),
        UpdateEvent::StateChanged {
            state: InstanceState::Disabled,
            ..
        }
    ));

    // Neither a notification nor the period reaches a disabled MCP's server.
    let lists = stack.server.list_requests();
    notify(&stack, with_new_tool()).await;
    tokio::time::advance(DEBOUNCE).await;
    tokio::time::advance(INTERVAL).await;
    settle(300).await;
    no_reload(&mut events);
    assert_eq!(stack.server.list_requests(), lists);

    settings.disabled_mcp.clear();
    stack.settings.send_replace(settings);
    settle(100).await;
    assert_eq!(tracker.state(&instance("dev")), Some(InstanceState::Connected));
    assert_eq!(tracker.state(&prod), Some(InstanceState::Catalogue));
}
