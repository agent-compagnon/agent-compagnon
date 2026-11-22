use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
use zeroize::{Zeroize, Zeroizing};

use super::error::{ProtectionFailure, SecretError};
use super::wrapper::KeyWrapper;

/// Length of the session key, in bytes (256 bits).
pub const SESSION_KEY_LEN: usize = 32;

/// Length of the random nonce of each sealing (XChaCha20-Poly1305).
pub(crate) const NONCE_LEN: usize = 24;

/// Fills `buf` from the operating system's random generator.
pub(crate) fn random(buf: &mut [u8]) -> Result<(), SecretError> {
    getrandom::fill(buf).map_err(|_| SecretError::Protection(ProtectionFailure::Random))
}

/// The session key in clear, for the duration of one use. Wiped when dropped.
pub(crate) struct ClearKey([u8; SESSION_KEY_LEN]);

impl ClearKey {
    /// A fresh random key.
    pub(crate) fn generate() -> Result<Self, SecretError> {
        let mut key = Self([0u8; SESSION_KEY_LEN]);
        random(&mut key.0)?;
        Ok(key)
    }

    /// Opens a key wrapped by `wrapper`. A result of the wrong length is refused.
    pub(crate) fn unwrap_from(wrapper: &dyn KeyWrapper, wrapped: &[u8]) -> Result<Self, SecretError> {
        let clear = wrapper.unwrap(wrapped)?;
        let mut key = Self([0u8; SESSION_KEY_LEN]);
        if clear.len() != SESSION_KEY_LEN {
            return Err(SecretError::Protection(ProtectionFailure::InvalidKey));
        }
        key.0.copy_from_slice(&clear);
        Ok(key)
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// A cipher for one sealing or opening. The cipher wipes its copy of the key when dropped (`zeroize` feature).
    pub(crate) fn cipher(&self) -> XChaCha20Poly1305 {
        let mut key = chacha20poly1305::Key::from(self.0);
        let cipher = XChaCha20Poly1305::new(&key);
        key.as_mut_slice().zeroize();
        cipher
    }
}

impl Drop for ClearKey {
    fn drop(&mut self) {
        self.0.zeroize();
        #[cfg(test)]
        key_probe::record(&self.0);
    }
}

/// The session key as the store keeps it: wrapped, never in clear (except with the development wrapper).
pub(crate) struct WrappedKey(Zeroizing<Vec<u8>>);

impl WrappedKey {
    /// Generates a session key and wraps it. A trial round trip checks that the wrapper gives the key back: a
    /// wrapper that does not answer, or answers wrong, fails here rather than at the first use.
    pub(crate) fn create(wrapper: &dyn KeyWrapper) -> Result<Self, SecretError> {
        let key = ClearKey::generate()?;
        let wrapped = Zeroizing::new(wrapper.wrap(key.as_bytes())?);
        let back = ClearKey::unwrap_from(wrapper, &wrapped)?;
        if back.as_bytes() != key.as_bytes() {
            return Err(SecretError::Protection(ProtectionFailure::RoundTrip));
        }
        Ok(Self(wrapped))
    }

    pub(crate) fn open(&self, wrapper: &dyn KeyWrapper) -> Result<ClearKey, SecretError> {
        ClearKey::unwrap_from(wrapper, &self.0)
    }
}

/// Test access to the state of a `ClearKey` right before its memory was released.
#[cfg(test)]
pub(crate) mod key_probe {
    use std::cell::RefCell;

    thread_local! {
        static DROPPED: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn record(key: &[u8]) {
        DROPPED.with(|dropped| dropped.borrow_mut().push(key.to_vec()));
    }

    /// Every key dropped on this thread since the last call.
    pub(crate) fn take() -> Vec<Vec<u8>> {
        DROPPED.with(|dropped| std::mem::take(&mut *dropped.borrow_mut()))
    }
}
