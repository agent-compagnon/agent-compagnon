use std::fmt;

use chacha20poly1305::aead::AeadInOut;
use zeroize::{Zeroize, Zeroizing};

use super::credential::MAX_SECRET_LEN;
use super::error::{ProtectionFailure, SecretError};
use super::key::{ClearKey, NONCE_LEN, random};
use crate::config::SessionId;

/// Length of the Poly1305 authentication tag appended to each ciphertext.
const TAG_LEN: usize = 16;

/// Domain of the associated data, so that a ciphertext of this store can never be taken for anything else.
const AAD_DOMAIN: &[u8] = b"mcp-proxy.secret.v1";

/// Associated data of a sealed value: the session it belongs to and its place in the credential. A value sealed for
/// one session (or one slot) does not open for another. Session identifiers, cookie names and Basic logins contain
/// no NUL byte, so the encoding is unambiguous.
pub(crate) fn associated_data(session: &SessionId, slot: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(AAD_DOMAIN.len() + session.as_str().len() + slot.len() + 2);
    aad.extend_from_slice(AAD_DOMAIN);
    aad.push(0);
    aad.extend_from_slice(session.as_str().as_bytes());
    aad.push(0);
    aad.extend_from_slice(slot.as_bytes());
    aad
}

/// A secret sealed with the session key of a store (XChaCha20-Poly1305, random nonce, associated data). Only the
/// crate can open it, and only into a wiped-on-drop copy.
pub struct SealedSecret {
    nonce: [u8; NONCE_LEN],
    /// Ciphertext followed by the authentication tag.
    buf: Box<[u8]>,
}

/// Decrypted copy of a `SealedSecret`, wiped when dropped.
pub(crate) struct Plaintext(Zeroizing<Vec<u8>>);

impl Plaintext {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn as_str(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(self.as_bytes()).map_err(|_| SecretError::Corrupted)
    }
}

impl SealedSecret {
    pub(crate) fn seal(key: &ClearKey, aad: &[u8], plaintext: &[u8]) -> Result<Self, SecretError> {
        if plaintext.len() > MAX_SECRET_LEN {
            return Err(SecretError::Protection(ProtectionFailure::TooLarge));
        }
        let mut nonce = [0u8; NONCE_LEN];
        random(&mut nonce)?;
        // Room for the tag up front: encrypting in place must not reallocate and leave a clear copy behind.
        let mut buf = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
        buf.extend_from_slice(plaintext);
        key.cipher()
            .encrypt_in_place(&nonce.into(), aad, &mut *buf)
            .map_err(|_| SecretError::Protection(ProtectionFailure::TooLarge))?;
        Ok(Self {
            nonce,
            buf: buf.as_slice().into(),
        })
    }

    /// Length of the clear value.
    pub fn len(&self) -> usize {
        self.buf.len().saturating_sub(TAG_LEN)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decrypts a copy; the sealed buffer itself is never decrypted in place. Fails with `Corrupted` if the
    /// ciphertext was altered, or if `key` or `aad` are not the ones it was sealed with.
    pub(crate) fn unseal(&self, key: &ClearKey, aad: &[u8]) -> Result<Plaintext, SecretError> {
        let mut buf = Zeroizing::new(self.buf.to_vec());
        key.cipher()
            .decrypt_in_place(&self.nonce.into(), aad, &mut *buf)
            .map_err(|_| SecretError::Corrupted)?;
        Ok(Plaintext(buf))
    }

    /// Nonce and ciphertext, to compare two sealings in tests.
    #[cfg(test)]
    pub(crate) fn debug_bytes(&self) -> Vec<u8> {
        [self.nonce.as_slice(), &self.buf].concat()
    }

    /// Flips one bit of the ciphertext, to test that an altered secret is detected.
    #[cfg(test)]
    pub(crate) fn tamper(&mut self, index: usize) {
        if let Some(byte) = self.buf.get_mut(index) {
            *byte ^= 0x01;
        }
    }
}

impl Drop for SealedSecret {
    fn drop(&mut self) {
        self.buf.zeroize();
        self.nonce.zeroize();
        #[cfg(test)]
        drop_probe::record(&self.buf);
    }
}

impl fmt::Debug for SealedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SealedSecret(***)")
    }
}

/// Test access to the state of a `SealedSecret` buffer as it was right before its memory was released.
#[cfg(test)]
pub(crate) mod drop_probe {
    use std::cell::RefCell;

    thread_local! {
        static LAST: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    }

    pub(crate) fn record(buf: &[u8]) {
        LAST.with(|last| *last.borrow_mut() = Some(buf.to_vec()));
    }

    pub(crate) fn take() -> Option<Vec<u8>> {
        LAST.with(|last| last.borrow_mut().take())
    }
}
