use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use secrecy::ExposeSecret;

use super::clock::{Clock, SystemClock};
use super::credential::{Credential, ExposedCookie, ExposedCredential};
use super::error::SecretError;
use super::key::{ClearKey, WrappedKey};
use super::sealed::{Plaintext, SealedSecret, associated_data};
use super::variants::derive_variants;
use super::wrapper::{KeyWrapper, platform_key_wrapper};
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
    session: SessionId,
    variants: Vec<SealedSecret>,
    until: Instant,
}

#[derive(Default)]
struct State {
    entries: HashMap<SessionId, Entry>,
    retired: Vec<Retired>,
}

impl State {
    fn retire(&mut self, session: SessionId, entry: Entry, until: Instant) {
        let Entry { credential, variants } = entry;
        drop(credential);
        if !variants.is_empty() {
            self.retired.push(Retired {
                session,
                variants,
                until,
            });
        }
    }

    fn purge(&mut self, now: Instant) {
        self.retired.retain(|r| r.until > now);
    }
}

/// Grace period of retired variants until the gateway wires it to `upstream_timeout_secs` + 10 s (step 3): the
/// value for the embedded configuration.
pub const DEFAULT_VARIANT_GRACE: Duration = Duration::from_secs(130);

// Slots of the associated data (`associated_data`): where a sealed value sits in its session's credential.
const SLOT_BEARER: &str = "bearer";
const SLOT_VARIANT: &str = "variant";

fn cookie_slot(name: &str) -> String {
    format!("cookie:{name}")
}

fn basic_slot(login: &str) -> String {
    format!("basic:{login}")
}

impl SealedCredential {
    fn seal(credential: &Credential, session: &SessionId, key: &ClearKey) -> Result<Self, SecretError> {
        let seal = |slot: &str, value: &str| SealedSecret::seal(key, &associated_data(session, slot), value.as_bytes());
        Ok(match credential {
            Credential::Bearer { token } => Self::Bearer {
                token: seal(SLOT_BEARER, token.expose_secret())?,
            },
            Credential::Cookies { pairs } => Self::Cookies {
                pairs: pairs
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), seal(&cookie_slot(name), value.expose_secret())?)))
                    .collect::<Result<_, SecretError>>()?,
            },
            Credential::Basic { login, password } => Self::Basic {
                login: login.clone(),
                password: seal(&basic_slot(login), password.expose_secret())?,
            },
        })
    }

    fn open(&self, session: &SessionId, key: &ClearKey) -> Result<OpenedCredential, SecretError> {
        let open = |slot: &str, sealed: &SealedSecret| sealed.unseal(key, &associated_data(session, slot));
        Ok(match self {
            Self::Bearer { token } => OpenedCredential::Bearer {
                token: open(SLOT_BEARER, token)?,
            },
            Self::Cookies { pairs } => OpenedCredential::Cookies {
                pairs: pairs
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), open(&cookie_slot(name), value)?)))
                    .collect::<Result<_, SecretError>>()?,
            },
            Self::Basic { login, password } => OpenedCredential::Basic {
                login: login.clone(),
                password: open(&basic_slot(login), password)?,
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
/// Envelope: every value is sealed (XChaCha20-Poly1305, random nonce, session in the associated data) with a random
/// 256-bit session key created with the store. The store keeps that key only wrapped by its `KeyWrapper`, and unwraps
/// it once per use (`put`, `with_exposed`, `with_variants`); the clear key is wiped as soon as the values are sealed
/// or decrypted, before the caller's closure runs. If the wrapper fails, the use fails and the closure is not called.
///
/// The lock is never held while a caller's closure runs, so closures may call back into the store.
pub struct SecretStore {
    wrapper: Box<dyn KeyWrapper>,
    key: WrappedKey,
    clock: Arc<dyn Clock>,
    grace: Duration,
    state: RwLock<State>,
}

impl SecretStore {
    /// A store with a fresh session key wrapped by `wrapper`, `DEFAULT_VARIANT_GRACE` and the system clock.
    ///
    /// Fails with `KeyProtectionUnavailable` if the wrapper cannot wrap the key, or if a trial round trip does not
    /// give it back: there is no fallback to a weaker protection.
    pub fn new(wrapper: Box<dyn KeyWrapper>) -> Result<Self, SecretError> {
        let key = WrappedKey::create(wrapper.as_ref()).map_err(|error| match error {
            SecretError::Protection(failure) => SecretError::KeyProtectionUnavailable(failure),
            other => other,
        })?;
        Ok(Self {
            wrapper,
            key,
            clock: Arc::new(SystemClock),
            grace: DEFAULT_VARIANT_GRACE,
            state: RwLock::new(State::default()),
        })
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

    /// A store using `platform_key_wrapper()`: user-scoped DPAPI on Windows. Fails, with an explicit error, if DPAPI
    /// does not answer.
    pub fn for_platform() -> Result<Self, SecretError> {
        Self::new(platform_key_wrapper())
    }

    /// Unwraps the session key for one use.
    fn open_key(&self) -> Result<ClearKey, SecretError> {
        self.key.open(self.wrapper.as_ref())
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
        let key = self.open_key()?;
        let variant_aad = associated_data(session, SLOT_VARIANT);
        let entry = Entry {
            credential: SealedCredential::seal(&credential, session, &key)?,
            variants: derive_variants(&credential)
                .iter()
                .map(|v| SealedSecret::seal(&key, &variant_aad, v))
                .collect::<Result<_, _>>()?,
        };
        drop(key);
        drop(credential);
        let until = self.retired_until();
        let mut state = self.write();
        if let Some(previous) = state.entries.insert(session.clone(), entry) {
            state.retire(session.clone(), previous, until);
        }
        Ok(())
    }

    /// Calls `f` with the decrypted credential of `session`. The session key is unwrapped once and wiped before `f`
    /// runs; the clear copy is wiped when `f` returns.
    pub fn with_exposed<R>(
        &self,
        session: &SessionId,
        f: impl FnOnce(ExposedCredential<'_>) -> R,
    ) -> Result<R, SecretError> {
        if !self.has(session) {
            return Err(SecretError::NoSecret(session.clone()));
        }
        let key = self.open_key()?;
        let opened = {
            let state = self.read();
            let entry = state
                .entries
                .get(session)
                .ok_or_else(|| SecretError::NoSecret(session.clone()))?;
            entry.credential.open(session, &key)?
        };
        drop(key);
        opened.expose(f)
    }

    /// Calls `f` with the filter variants of every active secret and of every retired one still in its grace period,
    /// decrypted for the duration of the call. Expired retired variants are erased first. The session key is
    /// unwrapped once for all of them, and wiped before `f` runs.
    ///
    /// Fails (and `f` is not called) if the key cannot be unwrapped or any variant cannot be decrypted: the caller
    /// must then fail closed, since filtering with a partial list could let a secret through.
    pub fn with_variants<R>(&self, f: impl FnOnce(&[&[u8]]) -> R) -> Result<R, SecretError> {
        let now = self.clock.now();
        let key = self.open_key()?;
        let key_ref = &key;
        let opened = {
            let mut state = self.write();
            state.purge(now);
            let active = state.entries.iter().map(|(session, entry)| (session, &entry.variants));
            let retired = state.retired.iter().map(|r| (&r.session, &r.variants));
            active
                .chain(retired)
                .flat_map(|(session, variants)| {
                    let aad = associated_data(session, SLOT_VARIANT);
                    variants
                        .iter()
                        .map(move |sealed| sealed.unseal(key_ref, &aad))
                        .collect::<Vec<_>>()
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        drop(key);
        let needles: Vec<&[u8]> = opened.iter().map(Plaintext::as_bytes).collect();
        Ok(f(&needles))
    }

    /// Erases the secret of `session` and retires its variants. Returns whether there was one.
    pub fn wipe(&self, session: &SessionId) -> bool {
        let until = self.retired_until();
        let mut state = self.write();
        match state.entries.remove(session) {
            Some(entry) => {
                state.retire(session.clone(), entry, until);
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
        for (session, entry) in entries {
            state.retire(session, entry, until);
        }
    }

    /// Whether `session` has an active secret (retired variants do not count).
    pub fn has(&self, session: &SessionId) -> bool {
        self.read().entries.contains_key(session)
    }

    /// Moves the sealed credential of `from` under `to`, as an attacker able to rearrange the store's memory
    /// would: opening it for `to` must fail.
    #[cfg(test)]
    pub(crate) fn move_entry_for_test(&self, from: &SessionId, to: &SessionId) {
        let mut state = self.write();
        if let Some(entry) = state.entries.remove(from) {
            state.entries.insert(to.clone(), entry);
        }
    }

    /// Alters one bit of the sealed credential (or of the first variant) of `session`.
    #[cfg(test)]
    pub(crate) fn tamper_for_test(&self, session: &SessionId, variant: bool) {
        let mut state = self.write();
        let Some(entry) = state.entries.get_mut(session) else {
            return;
        };
        if variant {
            if let Some(sealed) = entry.variants.first_mut() {
                sealed.tamper(0);
            }
            return;
        }
        match &mut entry.credential {
            SealedCredential::Bearer { token } => token.tamper(0),
            SealedCredential::Cookies { pairs } => pairs.iter_mut().for_each(|(_, v)| v.tamper(0)),
            SealedCredential::Basic { password, .. } => password.tamper(0),
        }
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
