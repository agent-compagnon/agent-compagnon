//! Integrity seal of the files the application writes and reads back (spec 014): the settings and the catalogue
//! cache. A program that edits such a file without the key cannot make it pass: the reader falls back to the safe
//! state (strictest settings, embedded catalogue).
//!
//! A sealed file is one header line, `MCP-PROXY-SEAL v1 hmac-sha256=<64 hex>`, then the content as written. The
//! HMAC-SHA256 covers a domain label, the purpose of the file (`settings.json`, `catalog/<mcp>.<env>`) and the exact
//! content bytes, so a file sealed for one purpose does not open for another. Comparison is in constant time.
//!
//! The 32-byte key is random, kept on disk wrapped by a [`KeyWrapper`] (user-scoped DPAPI on Windows), held in memory
//! wrapped too, and unwrapped once per use. It never appears in a `Debug`, an error or a log.
//!
//! Known limit (D-014): any program running as the same Windows user can call DPAPI, open the key and seal a file.
//! The seal raises the bar against a program that only writes files; it does not stop one that runs code as the user.

#[cfg(test)]
mod tests;

use std::fmt;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::secrets::KeyWrapper;
use crate::secrets::key::ClearKey;
use crate::settings::store::{io_reason, write_atomic};

/// Length of the sealing key, in bytes.
pub const SEAL_KEY_LEN: usize = 32;

/// Purpose of the settings file.
pub const SETTINGS_PURPOSE: &str = "settings.json";

const KEY_MAGIC: &[u8] = b"MCP-PROXY-SEAL-KEY v1\n";
const HEADER_PREFIX: &[u8] = b"MCP-PROXY-SEAL v1 hmac-sha256=";
const TAG_HEX_LEN: usize = 64;
const DOMAIN: &[u8] = b"mcp-proxy.seal.v1";
/// A wrapped key is a few hundred bytes; anything much larger is not a key file of this application.
const MAX_KEY_FILE_BYTES: u64 = 16 * 1024;

/// Purpose of the catalogue cache file of one instance.
pub fn catalog_purpose(mcp: &str, env: &str) -> String {
    format!("catalog/{mcp}.{env}")
}

/// Why a file could not be sealed or opened. French, quotes nothing of the file nor of the key.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SealError {
    #[error("clé de scellement absente")]
    KeyMissing,
    #[error("clé de scellement illisible ({0})")]
    KeyUnreadable(String),
    #[error("clé de scellement impossible à créer ({0})")]
    KeyCreation(String),
    #[error("fichier sans sceau (écrit par une version précédente de MCP Proxy ou hors de l'appli)")]
    Unsealed,
    #[error("sceau illisible")]
    Malformed,
    #[error("sceau invalide : fichier modifié hors de MCP Proxy, ou scellé par une autre clé")]
    Mismatch,
}

enum KeyState {
    /// The wrapped key, checked at load: it unwraps to a key of the right length.
    Ready(Zeroizing<Vec<u8>>),
    /// Missing or unusable: nothing opens; the first seal replaces it by a new key.
    Unusable(SealError),
}

/// Seals and opens files with one key, stored at `key_path`. Shared by the settings and the catalogue cache.
pub struct Sealer {
    key_path: PathBuf,
    wrapper: Box<dyn KeyWrapper>,
    state: Mutex<KeyState>,
}

impl fmt::Debug for Sealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sealer")
            .field("key_path", &self.key_path)
            .finish_non_exhaustive()
    }
}

fn read_key_file(path: &Path) -> Result<Vec<u8>, SealError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(SealError::KeyMissing),
        Err(e) => return Err(SealError::KeyUnreadable(io_reason(&e))),
    };
    let mut bytes = Vec::new();
    file.take(MAX_KEY_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SealError::KeyUnreadable(io_reason(&e)))?;
    if bytes.len() as u64 > MAX_KEY_FILE_BYTES {
        return Err(SealError::KeyUnreadable("fichier trop volumineux".to_owned()));
    }
    match bytes.strip_prefix(KEY_MAGIC) {
        Some(wrapped) if !wrapped.is_empty() => Ok(wrapped.to_vec()),
        _ => Err(SealError::KeyUnreadable("format inconnu".to_owned())),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &[u8]) -> Option<Vec<u8>> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    text.chunks(2)
        .map(|pair| match pair {
            [high, low] => Some((digit(*high)? << 4) | digit(*low)?),
            _ => None,
        })
        .collect()
}

fn mac(key: &ClearKey, purpose: &str, content: &[u8]) -> Result<Hmac<Sha256>, SealError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| SealError::Malformed)?;
    mac.update(DOMAIN);
    mac.update(&[0]);
    mac.update(purpose.as_bytes());
    mac.update(&[0]);
    mac.update(content);
    Ok(mac)
}

/// Splits a sealed file into its tag and its content.
fn split(file: &[u8]) -> Result<(Vec<u8>, &[u8]), SealError> {
    let rest = file.strip_prefix(HEADER_PREFIX).ok_or(SealError::Unsealed)?;
    if rest.len() < TAG_HEX_LEN + 1 || rest[TAG_HEX_LEN] != b'\n' {
        return Err(SealError::Malformed);
    }
    let tag = unhex(&rest[..TAG_HEX_LEN]).ok_or(SealError::Malformed)?;
    Ok((tag, &rest[TAG_HEX_LEN + 1..]))
}

impl Sealer {
    /// Reads the key at `key_path` and checks that `wrapper` opens it. Never fails: a missing or unusable key is
    /// kept as such, so that nothing opens until a new key is created by the next seal.
    pub fn open(key_path: impl Into<PathBuf>, wrapper: Box<dyn KeyWrapper>) -> Self {
        let key_path = key_path.into();
        let state =
            match read_key_file(&key_path).and_then(|wrapped| Self::check(wrapper.as_ref(), Zeroizing::new(wrapped))) {
                Ok(wrapped) => KeyState::Ready(wrapped),
                Err(error) => {
                    tracing::warn!(%error, "clé de scellement inutilisable : aucun fichier scellé ne sera accepté");
                    KeyState::Unusable(error)
                }
            };
        Self {
            key_path,
            wrapper,
            state: Mutex::new(state),
        }
    }

    fn check(wrapper: &dyn KeyWrapper, wrapped: Zeroizing<Vec<u8>>) -> Result<Zeroizing<Vec<u8>>, SealError> {
        ClearKey::unwrap_from(wrapper, &wrapped).map_err(|e| SealError::KeyUnreadable(e.to_string()))?;
        Ok(wrapped)
    }

    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    /// `Ok` when the key loaded at start, or created since, is usable.
    pub fn key_status(&self) -> Result<(), SealError> {
        match &*self.lock() {
            KeyState::Ready(_) => Ok(()),
            KeyState::Unusable(error) => Err(error.clone()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, KeyState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new random key, wrapped, checked by a round trip, then written. The key is kept only once written: a key
    /// that would not be found at the next start must not seal anything.
    fn create_key(&self) -> Result<Zeroizing<Vec<u8>>, SealError> {
        let creation = |e: crate::secrets::SecretError| SealError::KeyCreation(e.to_string());
        let key = ClearKey::generate().map_err(creation)?;
        let wrapped = Zeroizing::new(self.wrapper.wrap(key.as_bytes()).map_err(creation)?);
        let back = ClearKey::unwrap_from(self.wrapper.as_ref(), &wrapped).map_err(creation)?;
        if back.as_bytes() != key.as_bytes() {
            return Err(SealError::KeyCreation("aller-retour de la clé incorrect".to_owned()));
        }
        let mut file = Zeroizing::new(KEY_MAGIC.to_vec());
        file.extend_from_slice(&wrapped);
        write_atomic(&self.key_path, &file).map_err(|e| SealError::KeyCreation(io_reason(&e)))?;
        Ok(wrapped)
    }

    /// `content` with its seal for `purpose`. A missing or unusable key is replaced by a new one first: every file
    /// sealed by the previous key stops opening, which only ever falls back to the safe state.
    pub fn seal(&self, purpose: &str, content: &[u8]) -> Result<Vec<u8>, SealError> {
        let wrapped = {
            let mut state = self.lock();
            if let KeyState::Unusable(previous) = &*state {
                let wrapped = self.create_key()?;
                tracing::warn!(cause = %previous, "nouvelle clé de scellement créée");
                *state = KeyState::Ready(wrapped);
            }
            match &*state {
                KeyState::Ready(wrapped) => wrapped.clone(),
                KeyState::Unusable(error) => return Err(error.clone()),
            }
        };
        let key = ClearKey::unwrap_from(self.wrapper.as_ref(), &wrapped)
            .map_err(|e| SealError::KeyUnreadable(e.to_string()))?;
        let tag = mac(&key, purpose, content)?.finalize().into_bytes();
        drop(key);
        let mut file = Vec::with_capacity(HEADER_PREFIX.len() + TAG_HEX_LEN + 1 + content.len());
        file.extend_from_slice(HEADER_PREFIX);
        file.extend_from_slice(hex(&tag).as_bytes());
        file.push(b'\n');
        file.extend_from_slice(content);
        Ok(file)
    }

    /// The content of `file` if its seal holds for `purpose`; an error otherwise, the content is then not to be
    /// trusted at all.
    pub fn unseal(&self, purpose: &str, file: &[u8]) -> Result<Vec<u8>, SealError> {
        let (tag, content) = split(file)?;
        let wrapped = match &*self.lock() {
            KeyState::Ready(wrapped) => wrapped.clone(),
            KeyState::Unusable(error) => return Err(error.clone()),
        };
        let key = ClearKey::unwrap_from(self.wrapper.as_ref(), &wrapped)
            .map_err(|e| SealError::KeyUnreadable(e.to_string()))?;
        // `verify_slice` compares in constant time.
        mac(&key, purpose, content)?
            .verify_slice(&tag)
            .map_err(|_| SealError::Mismatch)?;
        Ok(content.to_vec())
    }
}
