//! Following the tool lists of the servers (spec 008): reload on a change notification (legacy generation), on the
//! `subscriptions/listen` subscription (2026-07-28), periodically for servers that never notify, and after each
//! connection; install the list in the registry, which tells the clients only when a fingerprint changed.
//!
//! A reload never triggers a login: periodic reloads only concern instances that are connected, or whose list is
//! anonymous. A failed reload keeps the last known list.

mod model;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use mcp_proxy_core::config::Config;
use mcp_proxy_core::session::SessionGate;
use mcp_proxy_core::settings::UserSettings;
use tokio::sync::{RwLock, broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub use self::model::{InstanceState, InstanceStatus, McpUpdate, ReloadOutcome, UpdateEvent, UpdateTracker};
use crate::registry::{InstanceId, Registry};
use crate::server::OutputGate;
use crate::upstream::{Upstream, UpstreamError, Upstreams};

/// Recorded instead of an error message the filter could not check.
const ERROR_UNAVAILABLE: &str = "erreur non affichable (filtre de sortie indisponible)";

/// Timing of the reloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchTiming {
    /// Periodic reload (`refresh_interval_secs`), for servers that never notify.
    pub refresh_interval: Duration,
    /// Notifications and connections closer than this give a single reload.
    pub debounce: Duration,
    /// A list whose `ttl_ms` is shorter than `refresh_interval` is reloaded when it expires, but never more often
    /// than this.
    pub min_refresh: Duration,
}

impl WatchTiming {
    pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);
    pub const DEFAULT_MIN_REFRESH: Duration = Duration::from_secs(30);

    pub fn from_config(config: &Config) -> Self {
        Self {
            refresh_interval: Duration::from_secs(config.gateway.refresh_interval_secs),
            debounce: Self::DEFAULT_DEBOUNCE,
            min_refresh: Self::DEFAULT_MIN_REFRESH,
        }
    }

    /// Delay before the next periodic reload, after a list valid for `ttl`. A `ttl_ms` of 0, which rmcp sends for a
    /// 2026-07-28 server that gave none (« immediately stale »), is no hint: reloading continuously is not an option.
    fn period(&self, ttl: Option<Duration>) -> Duration {
        match ttl {
            Some(ttl) if !ttl.is_zero() => ttl.max(self.min_refresh).min(self.refresh_interval),
            _ => self.refresh_interval,
        }
    }
}

pub struct ListWatcherDeps {
    pub config: Arc<Config>,
    pub registry: Arc<RwLock<Registry>>,
    pub upstreams: Upstreams,
    /// Only asked whether a session is active: never to log in.
    pub gate: Arc<dyn SessionGate>,
    /// Disabled MCPs and exposure are read at each reload.
    pub settings: watch::Receiver<UserSettings>,
    /// Filters the error messages kept for the interface.
    pub output: OutputGate,
    pub tracker: UpdateTracker,
    pub timing: WatchTiming,
}

/// One task per upstream, plus one that follows the disabled MCPs. Everything stops when dropped.
pub struct ListWatcher {
    tracker: UpdateTracker,
    requests: BTreeMap<InstanceId, mpsc::Sender<()>>,
    tasks: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for ListWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListWatcher")
            .field("instances", &self.requests.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ListWatcher {
    fn drop(&mut self) {
        self.tasks.iter().for_each(JoinHandle::abort);
    }
}

struct Context {
    config: Arc<Config>,
    gate: Arc<dyn SessionGate>,
    registry: Arc<RwLock<Registry>>,
    settings: watch::Receiver<UserSettings>,
    output: OutputGate,
    tracker: UpdateTracker,
    timing: WatchTiming,
}

impl ListWatcher {
    /// Starts following every upstream of `deps.upstreams`. Instances that may be reloaded periodically are
    /// reloaded at once.
    pub fn spawn(deps: ListWatcherDeps) -> Self {
        let context = Arc::new(Context {
            config: deps.config,
            gate: deps.gate,
            registry: deps.registry,
            settings: deps.settings,
            output: deps.output,
            tracker: deps.tracker.clone(),
            timing: deps.timing,
        });
        context.follow_settings_once();
        let mut tasks = vec![tokio::spawn(Arc::clone(&context).follow_settings())];
        let mut requests = BTreeMap::new();
        for upstream in deps.upstreams.iter() {
            let (sender, receiver) = mpsc::channel(1);
            requests.insert(upstream.instance().clone(), sender);
            tasks.push(tokio::spawn(Arc::clone(&context).watch(Arc::clone(upstream), receiver)));
        }
        Self {
            tracker: deps.tracker,
            requests,
            tasks,
        }
    }

    pub fn tracker(&self) -> &UpdateTracker {
        &self.tracker
    }

    /// Asks for a reload of `instance` now (interface button), unless its MCP is disabled. `false` for an instance
    /// without upstream. A request made while one is waiting is merged with it.
    pub fn reload(&self, instance: &InstanceId) -> bool {
        match self.requests.get(instance) {
            Some(sender) => {
                let _ = sender.try_send(());
                true
            }
            None => false,
        }
    }
}

/// Waits for the next signal of `receiver`. A closed channel is forgotten and never fires again.
async fn next_signal(receiver: &mut Option<broadcast::Receiver<()>>) {
    loop {
        match receiver {
            None => std::future::pending::<()>().await,
            Some(r) => match r.recv().await {
                Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => return,
                Err(broadcast::error::RecvError::Closed) => *receiver = None,
            },
        }
    }
}

fn drain(receiver: &mut Option<broadcast::Receiver<()>>) {
    if let Some(r) = receiver {
        while matches!(r.try_recv(), Ok(()) | Err(broadcast::error::TryRecvError::Lagged(_))) {}
    }
}

enum Trigger {
    /// A notification or a connection: wait for the burst to end.
    Signal,
    Periodic,
    Requested,
}

impl Context {
    fn follow_settings_once(&self) {
        let disabled = self.settings.borrow().disabled_mcp.clone();
        self.tracker.set_disabled(&disabled);
    }

    async fn follow_settings(self: Arc<Self>) {
        let mut settings = self.settings.clone();
        while settings.changed().await.is_ok() {
            let disabled = settings.borrow_and_update().disabled_mcp.clone();
            self.tracker.set_disabled(&disabled);
        }
    }

    async fn watch(self: Arc<Self>, upstream: Arc<dyn Upstream>, mut requests: mpsc::Receiver<()>) {
        let instance = upstream.instance().clone();
        let mut changes = Some(upstream.changes());
        let mut connections = Some(upstream.connections());
        let mut requests_open = true;
        let mut next = Instant::now();
        loop {
            let trigger = tokio::select! {
                biased;
                () = next_signal(&mut changes) => Trigger::Signal,
                () = next_signal(&mut connections) => Trigger::Signal,
                request = requests.recv(), if requests_open => match request {
                    Some(()) => Trigger::Requested,
                    None => {
                        requests_open = false;
                        continue;
                    }
                },
                () = tokio::time::sleep_until(next) => Trigger::Periodic,
            };
            match trigger {
                Trigger::Signal => {
                    tokio::time::sleep(self.timing.debounce).await;
                    drain(&mut changes);
                    drain(&mut connections);
                }
                Trigger::Periodic if !self.periodic_allowed(&instance, upstream.as_ref()).await => {
                    next = Instant::now() + self.timing.refresh_interval;
                    continue;
                }
                Trigger::Periodic | Trigger::Requested => {}
            }
            self.reload(upstream.as_ref()).await;
            // The list just read is that of the current connection: its own connection signal needs no reload.
            drain(&mut connections);
            next = Instant::now() + self.timing.period(upstream.list_ttl());
        }
    }

    /// Connected to the server, or exposed with a list that needs no login: an anonymous list, or a session already
    /// active. A periodic reload never triggers a login, and does not contact the servers of environments the user
    /// does not expose.
    async fn periodic_allowed(&self, instance: &InstanceId, upstream: &dyn Upstream) -> bool {
        if self.tracker.is_disabled(instance) {
            return false;
        }
        if upstream.is_connected() {
            return true;
        }
        let Some(mcp) = self.config.mcp(&instance.mcp) else {
            return false;
        };
        let no_login = match self.config.session_for(&instance.mcp, &instance.env) {
            None => true,
            Some(session) => !mcp.list_requires_auth || self.gate.is_active(&session.id),
        };
        no_login && self.registry.read().await.exposure().exposes(instance)
    }

    async fn reload(&self, upstream: &dyn Upstream) {
        let instance = upstream.instance();
        if self.tracker.is_disabled(instance) {
            return;
        }
        let failure = match upstream.list_tools().await {
            Ok(tools) => match self.registry.write().await.apply_live_list(instance, tools) {
                Ok(diff) => {
                    self.tracker.record_success(instance, &diff, SystemTime::now());
                    return;
                }
                Err(error) => (InstanceState::Unreachable, format!("liste d'outils refusée : {error}")),
            },
            Err(error) => {
                let state = match error {
                    UpstreamError::SessionRequired(_) | UpstreamError::AuthRejected | UpstreamError::Session(_) => {
                        InstanceState::SessionRequired
                    }
                    _ => InstanceState::Unreachable,
                };
                (state, error.to_string())
            }
        };
        let (state, message) = failure;
        let message = self
            .output
            .text(&message)
            .unwrap_or_else(|| ERROR_UNAVAILABLE.to_owned());
        self.tracker.record_failure(instance, state, message);
    }
}
