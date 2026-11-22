//! Applying the updates of MCP Proxy itself (spec 020, D12): checks one minute after the start then every
//! `check_interval_hours`, and again when the beta channel is switched; a verified update is put in place at once
//! and takes effect at the next start, which the window offers (« Redémarrer pour mettre à jour »). A critical
//! release restarts the application by itself, once nothing is in progress for the LLM clients: no call, no login,
//! no confirmation waiting.
//!
//! The checks themselves are `mcp_proxy_deploy::update`; time is injected ([`Pause`]), so that no test waits. A
//! failure is only written to the journal.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::session::{SessionInfo, SessionState};
use mcp_proxy_deploy::update::{Channel, CheckOutcome, FIRST_CHECK_DELAY, UpdateReady, Updater};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::interface::InterfaceError;
use crate::notify::Information;
use crate::runtime::Gateway;
use crate::server::{CallPipeline, CallRequest, Filtered};

/// Between the notification of a critical release and the automatic restart, at the least: the user reads why the
/// application is going away.
pub const CRITICAL_GRACE: Duration = Duration::from_secs(60);

/// Waits for a duration. [`TokioPause`] in the application; tests release each pause themselves.
#[async_trait]
pub trait Pause: Send + Sync {
    async fn sleep(&self, duration: Duration);
}

pub struct TokioPause;

#[async_trait]
impl Pause for TokioPause {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Starts the installed exe again, then stops this process: the application, never the gateway.
pub trait Restarter: Send + Sync {
    fn restart(&self);
}

/// What the gateway does for its clients: what a critical restart waits for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// Calls between their arrival and their answer, those waiting for a login or a confirmation included.
    pub calls: usize,
    /// Sessions whose login was asked for and is not finished.
    pub logins: usize,
    pub confirmations: usize,
}

impl Activity {
    pub fn is_idle(&self) -> bool {
        *self == Self::default()
    }
}

/// The calls in flight, counted by a guard: a call cancelled by its client counts down as well.
#[derive(Debug, Clone)]
pub struct CallsInFlight(Arc<watch::Sender<usize>>);

impl Default for CallsInFlight {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(0)))
    }
}

/// One call in flight, until dropped.
#[derive(Debug)]
pub struct CallGuard(Arc<watch::Sender<usize>>);

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n = n.saturating_sub(1));
    }
}

impl CallsInFlight {
    pub fn start(&self) -> CallGuard {
        self.0.send_modify(|n| *n += 1);
        CallGuard(Arc::clone(&self.0))
    }

    pub fn count(&self) -> usize {
        *self.0.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<usize> {
        self.0.subscribe()
    }
}

/// The chain of calls, counted.
pub(crate) struct Counted {
    pub(crate) inner: Arc<dyn CallPipeline>,
    pub(crate) calls: CallsInFlight,
}

#[async_trait]
impl CallPipeline for Counted {
    async fn call(&self, request: CallRequest) -> Filtered {
        let _guard = self.calls.start();
        self.inner.call(request).await
    }
}

/// Signals that the activity may have changed. Taken **before** the activity is read, so that no change is missed.
pub struct ActivitySignal {
    changes: broadcast::Receiver<()>,
    calls: watch::Receiver<usize>,
}

impl ActivitySignal {
    pub fn new(changes: broadcast::Receiver<()>, calls: watch::Receiver<usize>) -> Self {
        Self { changes, calls }
    }

    async fn next(&mut self) {
        tokio::select! {
            received = self.changes.recv() => {
                // A lagging receiver was told (IT.2); a closed one will never be again: the calls still are.
                if matches!(received, Err(broadcast::error::RecvError::Closed)) {
                    let _ = self.calls.changed().await;
                }
            }
            _ = self.calls.changed() => {}
        }
    }
}

/// Where the activity is read.
pub trait ActivityProbe: Send + Sync {
    fn activity(&self) -> Activity;
    fn signal(&self) -> ActivitySignal;
}

/// Sessions whose login was asked for and is not finished.
pub fn logins_in(sessions: &[(SessionInfo, SessionState)]) -> usize {
    sessions
        .iter()
        .filter(|(_, state)| matches!(state, SessionState::LoginRequested { .. }))
        .count()
}

/// Resolves once nothing is in progress.
pub async fn wait_until_idle(probe: &dyn ActivityProbe) {
    let mut signal = probe.signal();
    while !probe.activity().is_idle() {
        signal.next().await;
    }
}

/// A critical release: the grace period, then the first moment nothing is in progress, then the restart.
pub async fn restart_when_idle(probe: &dyn ActivityProbe, pause: &dyn Pause, restarter: &dyn Restarter) {
    pause.sleep(CRITICAL_GRACE).await;
    wait_until_idle(probe).await;
    restarter.restart();
}

/// The notification of an update put in place.
pub fn update_information(ready: &UpdateReady) -> Information {
    let version = ready.version.to_string();
    if ready.critical {
        Information::CriticalUpdate { version }
    } else {
        Information::UpdateReady { version }
    }
}

/// What the checks are run with.
pub struct SelfUpdateDeps {
    pub updater: Updater,
    /// `check_interval_hours`.
    pub interval: Duration,
    pub pause: Arc<dyn Pause>,
    pub restarter: Arc<dyn Restarter>,
}

/// The task of the checks; stopped when dropped.
#[derive(Debug)]
pub struct SelfUpdateTask(JoinHandle<()>);

impl Drop for SelfUpdateTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What the gateway keeps of the updater.
#[derive(Default)]
pub(crate) struct SelfUpdateState {
    pub(crate) ready: watch::Sender<Option<UpdateReady>>,
    pub(crate) restarter: Mutex<Option<Arc<dyn Restarter>>>,
}

impl ActivityProbe for Gateway {
    fn activity(&self) -> Activity {
        Activity {
            calls: self.calls_in_flight().count(),
            logins: self.logins_waiting(),
            confirmations: self.confirmations().pending().count(),
        }
    }

    fn signal(&self) -> ActivitySignal {
        ActivitySignal::new(self.changes(), self.calls_in_flight().subscribe())
    }
}

impl Gateway {
    /// The update put in place, if any: what « Redémarrer pour mettre à jour » applies.
    pub fn update_ready(&self) -> Option<UpdateReady> {
        self.self_update().ready.borrow().clone()
    }

    /// Changes of [`Gateway::update_ready`], for the notifications.
    pub fn update_changes(&self) -> watch::Receiver<Option<UpdateReady>> {
        self.self_update().ready.subscribe()
    }

    /// Starts the checks. Without any embedded key, nothing is started: the journal says once that updates are off.
    pub fn start_self_update(self: &Arc<Self>, deps: SelfUpdateDeps) -> Option<SelfUpdateTask> {
        if !deps.updater.has_keys() {
            self.journal().record_event(
                tracing::Level::WARN,
                "mises à jour automatiques désactivées : aucune clé de mise à jour dans cet exe",
            );
            return None;
        }
        *self
            .self_update()
            .restarter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&deps.restarter));
        let gateway = Arc::clone(self);
        Some(SelfUpdateTask(tokio::spawn(
            async move { gateway.run_checks(deps).await },
        )))
    }

    /// « Redémarrer pour mettre à jour »: the installed exe is started again, then this process stops. Every
    /// session is lost with it: they only live in memory.
    pub fn restart_for_update(&self) -> Result<(), InterfaceError> {
        let Some(ready) = self.update_ready() else {
            return Err(InterfaceError::NoUpdateReady);
        };
        let restarter = self
            .self_update()
            .restarter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(InterfaceError::NoUpdateReady)?;
        self.journal().record_event(
            tracing::Level::INFO,
            &format!("redémarrage demandé pour passer à MCP Proxy {}", ready.version),
        );
        restarter.restart();
        Ok(())
    }

    /// « Recevoir les préversions », from the menu of the icon: saved like every setting, and a check follows.
    pub async fn set_beta_channel(&self, enabled: bool) -> Result<(), mcp_proxy_core::settings::SettingsError> {
        self.change_settings(|s| s.beta_channel = enabled).await
    }

    async fn run_checks(self: Arc<Self>, deps: SelfUpdateDeps) {
        let mut settings = self.settings_receiver();
        let mut beta = settings.borrow_and_update().beta_channel;
        let mut delay = FIRST_CHECK_DELAY;
        loop {
            tokio::select! {
                () = deps.pause.sleep(delay) => {}
                () = channel_switched(&mut settings, beta) => {}
            }
            beta = settings.borrow_and_update().beta_channel;
            if let Some(ready) = self.check_once(&deps.updater, Channel::of(beta)).await
                && ready.critical
            {
                self.journal().record_event(
                    tracing::Level::WARN,
                    &format!(
                        "mise à jour critique {} : MCP Proxy redémarrera dès que plus rien n'est en cours",
                        ready.version
                    ),
                );
                restart_when_idle(self.as_ref(), deps.pause.as_ref(), deps.restarter.as_ref()).await;
                self.journal().record_event(
                    tracing::Level::WARN,
                    &format!("redémarrage automatique pour la mise à jour critique {}", ready.version),
                );
                return;
            }
            delay = deps.interval;
        }
    }

    /// One check; the update now in place, if this check put one.
    async fn check_once(&self, updater: &Updater, channel: Channel) -> Option<UpdateReady> {
        let previous = self.update_ready();
        match updater.check(channel, previous.as_ref().map(|r| &r.version)).await {
            Ok(CheckOutcome::UpToDate) => {
                tracing::info!(?channel, "mise à jour : aucune version plus récente");
                None
            }
            Ok(CheckOutcome::Ready(ready)) => {
                // A later release keeps the urgency of a critical one already in place, whose fix it carries.
                let ready = UpdateReady {
                    critical: ready.critical || previous.is_some_and(|p| p.critical),
                    version: ready.version,
                };
                self.journal().record_event(
                    tracing::Level::INFO,
                    &format!(
                        "mise à jour {} prête : elle sera utilisée au prochain démarrage de MCP Proxy",
                        ready.version
                    ),
                );
                self.self_update().ready.send_replace(Some(ready.clone()));
                Some(ready)
            }
            Err(error) => {
                self.journal().record_event(
                    tracing::Level::WARN,
                    &format!("vérification des mises à jour en échec : {error}"),
                );
                None
            }
        }
    }
}

/// Resolves when « Recevoir les préversions » no longer is `current`.
async fn channel_switched(settings: &mut watch::Receiver<mcp_proxy_core::settings::UserSettings>, current: bool) {
    if settings.wait_for(|s| s.beta_channel != current).await.is_err() {
        std::future::pending::<()>().await;
    }
}
