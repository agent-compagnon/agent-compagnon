//! `OnDemandAuthenticator`: the `SessionGate` that logs in when a call needs it, and only then.
//!
//! A login runs in its own task, outside the future of the call that asked for it: the chain gives up that future at
//! its deadline (I007.6), and the login must go on so that the next call finds it finished. Every call waiting for a
//! session shares the same login; one window is visible at a time, the logins of other sessions wait their turn.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::secrets::{OriginGuard, SecretError, SecretStore, SecretVersion};
use mcp_proxy_core::session::{
    AccessReason, EXPIRY_MARGIN, SessionChange, SessionError, SessionGate, SessionInfo, SessionRegistry, SessionState,
    inject_from_store,
};
use mcp_proxy_core::trace::WallClock;
use reqwest::header::HeaderMap;
use tokio::sync::{Semaphore, watch};
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::method::{AuthMethod, LoginContext, LoginFailure, Obtained};
use crate::ui::{AuthUi, AuthUiError};

/// What the authenticator is built from.
pub struct AuthenticatorDeps {
    pub registry: Arc<SessionRegistry>,
    /// The store the output filter is built on, so that every secret obtained is masked.
    pub store: Arc<SecretStore>,
    pub guard: Arc<OriginGuard>,
    pub ui: Arc<dyn AuthUi>,
    /// The method of each session that can log in; a session without one answers `Unavailable`.
    pub methods: BTreeMap<SessionId, Arc<dyn AuthMethod>>,
    /// Expiries are wall-clock times; timers run on tokio's clock.
    pub clock: Arc<dyn WallClock>,
}

/// `None` while the login runs.
type Outcome = Option<Result<(), SessionError>>;

struct Login {
    id: u64,
    done: watch::Receiver<Outcome>,
    cancel: CancellationToken,
    /// Restored when the login is cancelled or fails.
    previous: SessionState,
}

struct ActiveSecret {
    version: SecretVersion,
    until: SystemTime,
}

#[derive(Default)]
struct Slot {
    active: Option<ActiveSecret>,
    login: Option<Login>,
    timer: Option<AbortHandle>,
    /// A login taken away by « Tout déconnecter », and what its waiting calls get when it ends.
    interrupted: Option<(u64, SessionError)>,
}

struct Inner {
    registry: Arc<SessionRegistry>,
    store: Arc<SecretStore>,
    guard: Arc<OriginGuard>,
    ui: Arc<dyn AuthUi>,
    methods: BTreeMap<SessionId, Arc<dyn AuthMethod>>,
    clock: Arc<dyn WallClock>,
    /// One visible login window (or Basic prompt) at a time; fair, so sessions are served in order of request.
    window: Arc<Semaphore>,
    slots: Mutex<BTreeMap<SessionId, Slot>>,
    next_login: AtomicU64,
}

/// Logs in on demand, always visibly: never a silent attempt, never a renewal before a call asks for it. When a
/// session expires, its secret is wiped and nothing else happens.
pub struct OnDemandAuthenticator {
    inner: Arc<Inner>,
}

impl fmt::Debug for OnDemandAuthenticator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnDemandAuthenticator")
            .field(
                "sessions",
                &self.inner.methods.keys().map(SessionId::as_str).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl OnDemandAuthenticator {
    pub fn new(deps: AuthenticatorDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                registry: deps.registry,
                store: deps.store,
                guard: deps.guard,
                ui: deps.ui,
                methods: deps.methods,
                clock: deps.clock,
                window: Arc::new(Semaphore::new(1)),
                slots: Mutex::new(BTreeMap::new()),
                next_login: AtomicU64::new(1),
            }),
        }
    }

    pub fn registry(&self) -> &Arc<SessionRegistry> {
        &self.inner.registry
    }

    /// Current state, expiry included.
    pub fn state(&self, session: &SessionId) -> Option<SessionState> {
        self.inner.refresh(session);
        self.inner.registry.state(session)
    }

    /// « Se connecter » button: starts a login, or joins the one in progress, and waits for its end. No notification:
    /// the user asked for it.
    pub async fn connect(&self, session: &SessionId) -> Result<(), SessionError> {
        match self.inner.begin(session, None)? {
            None => Ok(()),
            Some(done) => wait(session, done, None).await,
        }
    }

    /// « Tout déconnecter » (spec 015), without confirmation: every login in progress is stopped (its waiting calls
    /// get `Disconnected`), every secret is wiped (their variants stay masked for their grace period), every session
    /// goes back to « jamais connectée », then the whole profile of the login windows is cleared and its windows
    /// closed. The secrets are gone before the profile is touched: a failure there is returned, logged, and changes
    /// nothing to them.
    pub async fn disconnect_all(&self) -> Result<(), AuthUiError> {
        self.inner.disconnect_all();
        let cleared = self.inner.ui.clear_all_site_data().await;
        if let Err(error) = &cleared {
            tracing::warn!(%error, "tout déconnecter : données de navigation non effacées");
        }
        cleared
    }

    /// « Déconnecter » button: stops a login in progress (its waiting calls get `Cancelled`), wipes the secret and
    /// clears the site data of the session's hosts.
    pub async fn disconnect(&self, session: &SessionId) {
        let hosts = self
            .inner
            .methods
            .get(session)
            .map(|m| m.site_hosts())
            .unwrap_or_default();
        self.inner.disconnect(session);
        if !hosts.is_empty()
            && let Err(error) = self.inner.ui.clear_site_data(&hosts).await
        {
            tracing::warn!(session = session.as_str(), %error, "déconnexion : données des sites non effacées");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Waits for the end of a login, until `deadline` if there is one. The login goes on after the deadline.
async fn wait(
    session: &SessionId,
    mut done: watch::Receiver<Outcome>,
    deadline: Option<Instant>,
) -> Result<(), SessionError> {
    let ended = async {
        loop {
            if let Some(outcome) = done.borrow_and_update().clone() {
                return outcome;
            }
            if done.changed().await.is_err() {
                return Err(SessionError::LoginFailed(session.clone()));
            }
        }
    };
    match deadline {
        None => ended.await,
        Some(deadline) => tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), ended)
            .await
            .unwrap_or_else(|_| Err(SessionError::LoginPending(session.clone()))),
    }
}

impl Inner {
    fn slots(&self) -> MutexGuard<'_, BTreeMap<SessionId, Slot>> {
        lock(&self.slots)
    }

    /// Wipes the secret of `session` if its expiry is reached. Under the slots lock.
    fn expire_if_due(&self, session: &SessionId, slot: &mut Slot) {
        let now = self.clock.now();
        if let Some(active) = &slot.active
            && now >= active.until
        {
            let version = active.version;
            self.deactivate(session, slot, version, SessionChange::Expired);
        }
    }

    /// The secret of `version` stops being the session's: wiped, state `Expired`. Under the slots lock.
    fn deactivate(&self, session: &SessionId, slot: &mut Slot, version: SecretVersion, change: SessionChange) {
        self.store.wipe_version(session, version);
        if slot.active.as_ref().is_some_and(|a| a.version == version) {
            slot.active = None;
            if let Some(timer) = slot.timer.take() {
                timer.abort();
            }
            if slot.login.is_none() {
                self.registry.set(session, SessionState::Expired, change);
            }
        }
    }

    fn refresh(&self, session: &SessionId) {
        let mut slots = self.slots();
        if let Some(slot) = slots.get_mut(session) {
            self.expire_if_due(session, slot);
        }
    }

    fn is_active(&self, session: &SessionId) -> bool {
        let mut slots = self.slots();
        let Some(slot) = slots.get_mut(session) else {
            return false;
        };
        self.expire_if_due(session, slot);
        slot.active.is_some() && self.store.has(session)
    }

    /// `None` if the session is active; otherwise the end of the login in progress, started now if there is none.
    fn begin(
        self: &Arc<Self>,
        session: &SessionId,
        reason: Option<&AccessReason>,
    ) -> Result<Option<watch::Receiver<Outcome>>, SessionError> {
        let (Some(method), Some(info)) = (self.methods.get(session), self.registry.info(session)) else {
            return Err(SessionError::Unavailable(session.clone()));
        };
        let mut slots = self.slots();
        let slot = slots.entry(session.clone()).or_default();
        self.expire_if_due(session, slot);
        if slot.active.is_some() && self.store.has(session) {
            return Ok(None);
        }
        if let Some(login) = &slot.login {
            return Ok(Some(login.done.clone()));
        }
        let id = self.next_login.fetch_add(1, Ordering::Relaxed);
        let (sender, done) = watch::channel(None);
        let cancel = CancellationToken::new();
        let previous = self.registry.state(session).unwrap_or(SessionState::NeverConnected);
        slot.login = Some(Login {
            id,
            done: done.clone(),
            cancel: cancel.clone(),
            previous,
        });
        self.registry.set(
            session,
            SessionState::LoginRequested {
                since: self.clock.now(),
            },
            SessionChange::LoginRequested,
        );
        let finisher = Finisher {
            inner: Arc::clone(self),
            session: session.clone(),
            id,
            sender: Some(sender),
        };
        tokio::spawn(finisher.run(Arc::clone(method), info.clone(), reason.cloned(), cancel));
        Ok(Some(done))
    }

    /// Records how login `id` ended, if it is still the session's login (a disconnection replaces it), and tells the
    /// waiting calls.
    fn finish(
        self: &Arc<Self>,
        session: &SessionId,
        id: u64,
        outcome: Result<Obtained, LoginFailure>,
    ) -> Result<(), SessionError> {
        let mut slots = self.slots();
        let slot = slots.entry(session.clone()).or_default();
        let Some(login) = slot.login.take_if(|l| l.id == id) else {
            // Disconnected meanwhile: whatever the login obtained is dropped.
            if let Some((_, error)) = slot.interrupted.take_if(|(interrupted, _)| *interrupted == id) {
                return Err(error);
            }
            return Err(SessionError::Cancelled(session.clone()));
        };
        let failure = match outcome {
            Ok(obtained) => match self.activate(session, slot, obtained) {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            },
            Err(failure) => failure,
        };
        if failure == LoginFailure::Cancelled {
            self.registry
                .set(session, login.previous, SessionChange::LoginCancelled);
            tracing::info!(session = session.as_str(), "connexion annulée par l'utilisateur");
            Err(SessionError::Cancelled(session.clone()))
        } else if failure == LoginFailure::Ui(AuthUiError::Unavailable) {
            // No interface to log in with: nothing was attempted, the call is told so plainly.
            self.registry.set(session, login.previous, SessionChange::LoginFailed);
            tracing::warn!(
                session = session.as_str(),
                "connexion impossible : interface de connexion indisponible"
            );
            Err(SessionError::Unavailable(session.clone()))
        } else {
            self.registry.set(session, login.previous, SessionChange::LoginFailed);
            tracing::warn!(session = session.as_str(), error = %failure, "connexion échouée");
            Err(SessionError::LoginFailed(session.clone()))
        }
    }

    /// Stores the credential and makes the session active until its expiry, margin taken off.
    fn activate(
        self: &Arc<Self>,
        session: &SessionId,
        slot: &mut Slot,
        obtained: Obtained,
    ) -> Result<(), LoginFailure> {
        let now = self.clock.now();
        let until = obtained.expires_at.checked_sub(EXPIRY_MARGIN).unwrap_or(now);
        if until <= now {
            return Err(LoginFailure::Unusable("déjà expiré".to_owned()));
        }
        let version = self
            .store
            .put(session, obtained.credential)
            .map_err(|e| LoginFailure::Unusable(e.to_string()))?;
        slot.active = Some(ActiveSecret { version, until });
        if let Some(timer) = slot.timer.replace(self.schedule_expiry(session, version, until)) {
            timer.abort();
        }
        self.registry
            .set(session, SessionState::Active { until }, SessionChange::LoggedIn);
        tracing::info!(session = session.as_str(), "connexion réussie");
        Ok(())
    }

    /// Wipes the secret at its expiry. Holds the authenticator weakly: the timer never keeps it alive.
    fn schedule_expiry(
        self: &Arc<Self>,
        session: &SessionId,
        version: SecretVersion,
        until: SystemTime,
    ) -> AbortHandle {
        let weak: Weak<Self> = Arc::downgrade(self);
        let session = session.clone();
        tokio::spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let left = until.duration_since(inner.clock.now());
                drop(inner);
                match left {
                    Ok(left) if !left.is_zero() => tokio::time::sleep(left).await,
                    _ => break,
                }
            }
            if let Some(inner) = weak.upgrade() {
                let mut slots = inner.slots();
                if let Some(slot) = slots.get_mut(&session)
                    && slot.active.as_ref().is_some_and(|a| a.version == version)
                {
                    // The timer is this task: it must not be aborted while it runs.
                    slot.timer = None;
                    inner.deactivate(&session, slot, version, SessionChange::Expired);
                }
            }
        })
        .abort_handle()
    }

    fn disconnect(&self, session: &SessionId) {
        let mut slots = self.slots();
        let slot = slots.entry(session.clone()).or_default();
        let was_connected = slot.active.is_some() || slot.login.is_some();
        if let Some(login) = slot.login.take() {
            login.cancel.cancel();
        }
        slot.active = None;
        if let Some(timer) = slot.timer.take() {
            timer.abort();
        }
        let wiped = self.store.wipe(session);
        let state = self.registry.state(session);
        if was_connected || wiped || state.is_some_and(|s| s != SessionState::NeverConnected) {
            self.registry
                .set(session, SessionState::Expired, SessionChange::Disconnected);
        }
        tracing::info!(session = session.as_str(), "session déconnectée");
    }

    fn disconnect_all(&self) {
        let mut slots = self.slots();
        for info in self.registry.infos() {
            let slot = slots.entry(info.id.clone()).or_default();
            if let Some(login) = slot.login.take() {
                login.cancel.cancel();
                slot.interrupted = Some((login.id, SessionError::Disconnected(info.id.clone())));
            }
            slot.active = None;
            if let Some(timer) = slot.timer.take() {
                timer.abort();
            }
        }
        self.store.wipe_all();
        for info in self.registry.infos() {
            if self.registry.state(&info.id) != Some(SessionState::NeverConnected) {
                self.registry
                    .set(&info.id, SessionState::NeverConnected, SessionChange::Disconnected);
            }
        }
        tracing::info!("toutes les sessions déconnectées");
    }

    fn mark_rejected(&self, session: &SessionId, version: SecretVersion) {
        let mut slots = self.slots();
        let slot = slots.entry(session.clone()).or_default();
        if slot.active.as_ref().is_some_and(|a| a.version == version) {
            tracing::info!(
                session = session.as_str(),
                "identifiant refusé par le serveur : session expirée"
            );
            self.deactivate(session, slot, version, SessionChange::Rejected);
        } else {
            // A secret renewed meanwhile, or already wiped: kept as it is.
            self.store.wipe_version(session, version);
        }
    }
}

/// Runs one login and always reports its end, even if the task is dropped (runtime shut down) or the method
/// panics: no session stays `LoginRequested` for ever.
struct Finisher {
    inner: Arc<Inner>,
    session: SessionId,
    id: u64,
    sender: Option<watch::Sender<Outcome>>,
}

impl Finisher {
    async fn run(
        mut self,
        method: Arc<dyn AuthMethod>,
        info: SessionInfo,
        reason: Option<AccessReason>,
        cancel: CancellationToken,
    ) {
        let outcome = self.login(method.as_ref(), &info, reason.as_ref(), &cancel).await;
        self.end(outcome);
    }

    async fn login(
        &self,
        method: &dyn AuthMethod,
        info: &SessionInfo,
        reason: Option<&AccessReason>,
        cancel: &CancellationToken,
    ) -> Result<Obtained, LoginFailure> {
        let permit = tokio::select! {
            permit = Arc::clone(&self.inner.window).acquire_owned() => permit
                .map_err(|_| LoginFailure::Unusable("file des fenêtres fermée".to_owned()))?,
            () = cancel.cancelled() => return Err(LoginFailure::Cancelled),
        };
        if let Some(reason) = reason {
            self.inner.ui.notify_login_needed(info, reason).await;
        }
        let context = LoginContext {
            session: info,
            ui: self.inner.ui.as_ref(),
            clock: self.inner.clock.as_ref(),
            cancel,
        };
        let outcome = method.login(context).await;
        drop(permit);
        outcome
    }

    fn end(&mut self, outcome: Result<Obtained, LoginFailure>) {
        let result = self.inner.finish(&self.session, self.id, outcome);
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Some(result));
        }
    }
}

impl Drop for Finisher {
    fn drop(&mut self) {
        if self.sender.is_some() {
            self.end(Err(LoginFailure::Unusable("connexion interrompue".to_owned())));
        }
    }
}

#[async_trait]
impl SessionGate for OnDemandAuthenticator {
    async fn ensure(&self, session: &SessionId, deadline: Instant, reason: &AccessReason) -> Result<(), SessionError> {
        match self.inner.begin(session, Some(reason))? {
            None => Ok(()),
            Some(done) => wait(session, done, Some(deadline)).await,
        }
    }

    fn is_active(&self, session: &SessionId) -> bool {
        self.inner.is_active(session)
    }

    fn inject(&self, session: &SessionId, url: &Url, headers: &mut HeaderMap) -> Result<SecretVersion, SessionError> {
        if !self.inner.is_active(session) {
            headers.remove(reqwest::header::AUTHORIZATION);
            headers.remove(reqwest::header::COOKIE);
            return Err(SecretError::NoSecret(session.clone()).into());
        }
        inject_from_store(&self.inner.store, &self.inner.guard, session, url, headers)
    }

    fn mark_rejected(&self, session: &SessionId, version: SecretVersion) {
        self.inner.mark_rejected(session, version);
    }
}
