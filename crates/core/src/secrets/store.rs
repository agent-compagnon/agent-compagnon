use std::collections::HashMap;
use std::fmt;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use secrecy::ExposeSecret;

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
/// The lock is never held while a caller's closure runs, so closures may call back into the store.
pub struct SecretStore {
    protector: Box<dyn MemoryProtector>,
    entries: RwLock<HashMap<SessionId, Entry>>,
}

impl SecretStore {
    pub fn new(protector: Box<dyn MemoryProtector>) -> Self {
        Self {
            protector,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// A store using `platform_protector()`.
    pub fn for_platform() -> Self {
        Self::new(platform_protector())
    }

    // A poisoned lock only means that another thread panicked: the map itself stays consistent, and wiping must keep
    // working whatever happened.
    fn read(&self) -> RwLockReadGuard<'_, HashMap<SessionId, Entry>> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<SessionId, Entry>> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the secret of `session`, sealed together with its filter variants. On error, the previous secret of
    /// the session is left untouched.
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
        let previous = self.write().insert(session.clone(), entry);
        drop(previous);
        Ok(())
    }

    /// Calls `f` with the decrypted credential of `session`. The clear copy is wiped when `f` returns.
    pub fn with_exposed<R>(
        &self,
        session: &SessionId,
        f: impl FnOnce(ExposedCredential<'_>) -> R,
    ) -> Result<R, SecretError> {
        let opened = {
            let entries = self.read();
            let entry = entries
                .get(session)
                .ok_or_else(|| SecretError::NoSecret(session.clone()))?;
            entry.credential.open(self.protector.as_ref())?
        };
        opened.expose(f)
    }

    /// Calls `f` with the filter variants of every active secret, decrypted for the duration of the call.
    ///
    /// Fails (and `f` is not called) if any variant cannot be decrypted: the caller must then fail closed, since
    /// filtering with a partial list could let a secret through.
    pub fn with_variants<R>(&self, f: impl FnOnce(&[&[u8]]) -> R) -> Result<R, SecretError> {
        let opened = {
            let entries = self.read();
            entries
                .values()
                .flat_map(|entry| &entry.variants)
                .map(|sealed| sealed.unseal(self.protector.as_ref()))
                .collect::<Result<Vec<_>, _>>()?
        };
        let needles: Vec<&[u8]> = opened.iter().map(Plaintext::as_bytes).collect();
        Ok(f(&needles))
    }

    /// Erases the secret of `session`. Returns whether there was one.
    pub fn wipe(&self, session: &SessionId) -> bool {
        let removed = self.write().remove(session);
        removed.is_some()
    }

    pub fn wipe_all(&self) {
        let removed = std::mem::take(&mut *self.write());
        drop(removed);
    }

    pub fn has(&self, session: &SessionId) -> bool {
        self.read().contains_key(session)
    }
}

impl fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let entries = self.read();
        let mut sessions: Vec<&str> = entries.keys().map(SessionId::as_str).collect();
        sessions.sort_unstable();
        f.debug_struct("SecretStore")
            .field("sessions", &sessions)
            .finish_non_exhaustive()
    }
}
