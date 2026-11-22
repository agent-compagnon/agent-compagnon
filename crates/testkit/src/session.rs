//! A session gate with a fixed credential or scripted states.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use http::HeaderMap;
use mcp_proxy_core::config::{Origin, SessionId};
use mcp_proxy_core::secrets::{Credential, OriginGuard, SecretError, SecretStore};
use mcp_proxy_core::session::{AccessReason, SessionError, SessionGate, StoreSessionGate};
use secrecy::SecretString;
use url::Url;

enum Scripted {
    Outcome(Result<(), SessionError>),
    /// Never returns: the caller must give up at its deadline.
    Hang,
}

/// Injection goes through the real `StoreSessionGate` (origin check included); `ensure` follows a script, then
/// answers from the store, after a simulated login if one is queued. Every call is counted.
pub struct FakeSessionGate {
    inner: StoreSessionGate,
    store: Arc<SecretStore>,
    script: Mutex<VecDeque<Scripted>>,
    /// Bearer tokens a login would obtain, used in order when `ensure` finds no secret.
    logins: Mutex<VecDeque<String>>,
    ensures: AtomicUsize,
    injections: AtomicUsize,
    rejected: Mutex<Vec<SessionId>>,
}

impl std::fmt::Debug for FakeSessionGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSessionGate").finish_non_exhaustive()
    }
}

impl FakeSessionGate {
    pub fn new(store: Arc<SecretStore>, guard: Arc<OriginGuard>) -> Self {
        Self {
            inner: StoreSessionGate::new(Arc::clone(&store), guard),
            store,
            script: Mutex::new(VecDeque::new()),
            logins: Mutex::new(VecDeque::new()),
            ensures: AtomicUsize::new(0),
            injections: AtomicUsize::new(0),
            rejected: Mutex::new(Vec::new()),
        }
    }

    /// One session with a bearer token, allowed towards `origins` only. The store is the one an output filter must
    /// be built on for the token to be masked.
    pub fn with_bearer(session: &SessionId, token: &str, origins: &[&str]) -> Result<Self, SecretError> {
        let store = Arc::new(SecretStore::for_platform()?);
        let origins: BTreeSet<Origin> = origins
            .iter()
            .filter_map(|o| Url::parse(o).ok())
            .filter_map(|u| Origin::of(&u))
            .collect();
        let guard = Arc::new(OriginGuard::new([(session.clone(), origins)]));
        store.put(
            session,
            Credential::Bearer {
                token: SecretString::from(token.to_owned()),
            },
        )?;
        Ok(Self::new(store, guard))
    }

    pub fn store(&self) -> &Arc<SecretStore> {
        &self.store
    }

    /// The next calls to `ensure` return these outcomes, in order, before falling back to the store.
    pub fn script(&self, outcomes: impl IntoIterator<Item = Result<(), SessionError>>) {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(outcomes.into_iter().map(Scripted::Outcome));
    }

    /// The next call to `ensure` never returns, like a login nobody completes.
    pub fn hang(&self) {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Scripted::Hang);
    }

    /// When `ensure` finds no secret, the next of these tokens is stored, as a completed login would.
    pub fn logins(&self, tokens: impl IntoIterator<Item = &'static str>) {
        self.logins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(tokens.into_iter().map(str::to_owned));
    }

    pub fn ensure_count(&self) -> usize {
        self.ensures.load(Ordering::SeqCst)
    }

    pub fn injection_count(&self) -> usize {
        self.injections.load(Ordering::SeqCst)
    }

    /// Sessions passed to `mark_rejected`, in order.
    pub fn rejected(&self) -> Vec<SessionId> {
        self.rejected.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[async_trait]
impl SessionGate for FakeSessionGate {
    async fn ensure(&self, session: &SessionId, deadline: Instant, reason: &AccessReason) -> Result<(), SessionError> {
        self.ensures.fetch_add(1, Ordering::SeqCst);
        let scripted = self.script.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
        match scripted {
            Some(Scripted::Outcome(outcome)) => outcome,
            Some(Scripted::Hang) => {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                Err(SessionError::LoginPending(session.clone()))
            }
            None => {
                if !self.store.has(session) {
                    let login = self.logins.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
                    if let Some(token) = login {
                        let token = SecretString::from(token);
                        self.store.put(session, Credential::Bearer { token })?;
                    }
                }
                self.inner.ensure(session, deadline, reason).await
            }
        }
    }

    fn is_active(&self, session: &SessionId) -> bool {
        self.inner.is_active(session)
    }

    fn inject(&self, session: &SessionId, url: &Url, headers: &mut HeaderMap) -> Result<(), SessionError> {
        self.injections.fetch_add(1, Ordering::SeqCst);
        self.inner.inject(session, url, headers)
    }

    fn mark_rejected(&self, session: &SessionId) {
        self.rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session.clone());
        self.inner.mark_rejected(session);
    }
}
