use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use secrecy::ExposeSecret;

use super::clock::{Clock, SystemClock};
use super::credential::{Credential, ExposedCookie, ExposedCredential};
use super::error::SecretError;
use super::protector::{MemoryProtector, platform_protector};
use super::sealed::{Plaintext, SealedSecret};
use super::variants::derive_variants;
use crate::config::SessionId;

enum SealedCredential {
    Bearer { token: SealedSecret },
    Cookies { pairs: Vec<(String, SealedSecret)> },
    Basic { login: String, password: SealedSecret },
}

enum OpenedCredential {
    Bearer { token: Plaintext },
    Cookies { pairs: Vec<(String, Plaintext)> },
    Basic { login: String, password: Plaintext },
}

struct Entry {
    credential: SealedCredential,
    variants: Vec<SealedSecret>,
}

/// Variants of a secret that was wiped or replaced, still masked until `until`.
struct Retired {
    variants: Vec<SealedSecret>,
    until: Instant,
}

#[derive(Default)]
struct State {
    entries: HashMap<SessionId, Entry>,
    retired: Vec<Retired>,
}

impl State {
    fn retire(&mut self, entry: Entry, until: Instant) {
        let Entry { credential, variants } = entry;
        drop(credential);
        if !variants.is_empty() {
            self.retired.push(Retired { variants, until });
        }
    }

    fn purge(&mut self, now: Instant) {
        self.retired.retain(|r| r.until > now);
    }
}

/// Grace period of retired variants until the gateway wires it to `upstream_timeout_secs` + 10 s (step 3): the
/// value for the embedded configuration.
pub const DEFAULT_VARIANT_GRACE: Duration = Duration::from_secs(130);

impl SealedCredential {
    fn seal(credential: &Credential, protector: &dyn MemoryProtector) -> Result<Self, SecretError> {
        let seal = |value: &str| SealedSecret::seal(protector, value.as_bytes());
        Ok(match credential {
            Credential::Bearer { token } => Self::Bearer {
                token: seal(token.expose_secret())?,
            },
            Credential::Cookies { pairs } => Self::Cookies {
                pairs: pairs
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), seal(value.expose_secret())?)))
                    .collect::<Result<_, SecretError>>()?,
            },
            Credential::Basic { login, password } => Self::Basic {
                login: login.clone(),
                password: seal(password.expose_secret())?,
            },
        })
    }

    fn open(&self, protector: &dyn MemoryProtector) -> Result<OpenedCredential, SecretError> {
        Ok(match self {
            Self::Bearer { token } => OpenedCredential::Bearer {
                token: token.unseal(protector)?,
            },
            Self::Cookies { pairs } => OpenedCredential::Cookies {
                pairs: pairs
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), value.unseal(protector)?)))
                    .collect::<Result<_, SecretError>>()?,
            },
            Self::Basic { login, password } => OpenedCredential::Basic {
                login: login.clone(),
                password: password.unseal(protector)?,
            },
        })
    }
}

impl OpenedCredential {
    fn expose<R>(&self, f: impl FnOnce(ExposedCredential<'_>) -> R) -> Result<R, SecretError> {
        Ok(match self {
            Self::Bearer { token } => f(ExposedCredential::Bearer { token: token.as_str()? }),
            Self::Cookies { pairs } => {
                let pairs = pairs
                    .iter()
                    .map(|(name, value)| {
                        Ok(ExposedCookie {
                            name,
                            value: value.as_str()?,
                        })
                    })
                    .collect::<Result<Vec<_>, SecretError>>()?;
                f(ExposedCredential::Cookies { pairs: &pairs })
            }
            Self::Basic { login, password } => f(ExposedCredential::Basic {
                login,
                password: password.as_str()?,
            }),
        })
    }
}

/// Secrets of every session, sealed outside of use. A secret belongs to exactly one session.
///
/// When a secret is wiped or replaced, it can no longer be injected, but its variants stay known to the output filter
/// for a grace period: a response to a call sent before the wipe may still carry it. They are erased at the first
/// store operation after the period.
///
/// The lock is never held while a caller's closure runs, so closures may call back into the store.
pub struct SecretStore {
    protector: Box<dyn MemoryProtector>,
    clock: Arc<dyn Clock>,
    grace: Duration,
    state: RwLock<State>,
}

impl SecretStore {
    /// A store with `DEFAULT_VARIANT_GRACE` and the system clock.
    pub fn new(protector: Box<dyn MemoryProtector>) -> Self {
        Self {
            protector,
            clock: Arc::new(SystemClock),
            grace: DEFAULT_VARIANT_GRACE,
            state: RwLock::new(State::default()),
        }
    }

    /// How long the variants of a wiped or replaced secret stay masked.
    #[must_use]
    pub fn with_variant_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    fn retired_until(&self) -> Instant {
        let now = self.clock.now();
        now.checked_add(self.grace).unwrap_or(now)
    }

    /// A store using `platform_protector()`.
    pub fn for_platform() -> Self {
        Self::new(platform_protector())
    }

    // A poisoned lock only means that another thread panicked: the map itself stays consistent, and wiping must keep
    // working whatever happened.
    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the secret of `session`, sealed together with its filter variants; the variants of the previous
    /// secret are retired. On error, the previous secret of the session is left untouched.
    pub fn put(&self, session: &SessionId, credential: Credential) -> Result<(), SecretError> {
        credential.validate()?;
        let protector = self.protector.as_ref();
        let entry = Entry {
            credential: SealedCredential::seal(&credential, protector)?,
            variants: derive_variants(&credential)
                .iter()
                .map(|v| SealedSecret::seal(protector, v))
                .collect::<Result<_, _>>()?,
        };
        drop(credential);
        let until = self.retired_until();
        let mut state = self.write();
        if let Some(previous) = state.entries.insert(session.clone(), entry) {
            state.retire(previous, until);
        }
        Ok(())
    }

    /// Calls `f` with the decrypted credential of `session`. The clear copy is wiped when `f` returns.
    pub fn with_exposed<R>(
        &self,
        session: &SessionId,
        f: impl FnOnce(ExposedCredential<'_>) -> R,
    ) -> Result<R, SecretError> {
        let opened = {
            let state = self.read();
            let entry = state
                .entries
                .get(session)
                .ok_or_else(|| SecretError::NoSecret(session.clone()))?;
            entry.credential.open(self.protector.as_ref())?
        };
        opened.expose(f)
    }

    /// Calls `f` with the filter variants of every active secret and of every retired one still in its grace period,
    /// decrypted for the duration of the call. Expired retired variants are erased first.
    ///
    /// Fails (and `f` is not called) if any variant cannot be decrypted: the caller must then fail closed, since
    /// filtering with a partial list could let a secret through.
    pub fn with_variants<R>(&self, f: impl FnOnce(&[&[u8]]) -> R) -> Result<R, SecretError> {
        let now = self.clock.now();
        let opened = {
            let mut state = self.write();
            state.purge(now);
            let retired = state.retired.iter().flat_map(|r| &r.variants);
            state
                .entries
                .values()
                .flat_map(|entry| &entry.variants)
                .chain(retired)
                .map(|sealed| sealed.unseal(self.protector.as_ref()))
                .collect::<Result<Vec<_>, _>>()?
        };
        let needles: Vec<&[u8]> = opened.iter().map(Plaintext::as_bytes).collect();
        Ok(f(&needles))
    }

    /// Erases the secret of `session` and retires its variants. Returns whether there was one.
    pub fn wipe(&self, session: &SessionId) -> bool {
        let until = self.retired_until();
        let mut state = self.write();
        match state.entries.remove(session) {
            Some(entry) => {
                state.retire(entry, until);
                true
            }
            None => false,
        }
    }

    /// Erases every secret and retires their variants.
    pub fn wipe_all(&self) {
        let until = self.retired_until();
        let mut state = self.write();
        let entries = std::mem::take(&mut state.entries);
        for entry in entries.into_values() {
            state.retire(entry, until);
        }
    }

    /// Whether `session` has an active secret (retired variants do not count).
    pub fn has(&self, session: &SessionId) -> bool {
        self.read().entries.contains_key(session)
    }
}

impl fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.read();
        let mut sessions: Vec<&str> = state.entries.keys().map(SessionId::as_str).collect();
        sessions.sort_unstable();
        f.debug_struct("SecretStore")
            .field("sessions", &sessions)
            .field("retired", &state.retired.len())
            .finish_non_exhaustive()
    }
}
