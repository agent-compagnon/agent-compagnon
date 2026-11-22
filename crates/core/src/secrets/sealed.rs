use std::fmt;

use zeroize::{Zeroize, Zeroizing};

use super::credential::MAX_SECRET_LEN;
use super::error::{ProtectionFailure, SecretError};
use super::protector::{MemoryProtector, PROTECT_BLOCK_SIZE};

/// A secret encrypted by a `MemoryProtector`. The buffer is padded to the protector's block size; the real length is
/// kept apart. Only the crate can read it back, and only into a wiped-on-drop copy.
pub struct SealedSecret {
    buf: Box<[u8]>,
    len: usize,
}

/// Decrypted copy of a `SealedSecret`, wiped (padding included) when dropped.
pub(crate) struct Plaintext {
    buf: Zeroizing<Vec<u8>>,
    len: usize,
}

impl Plaintext {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or_default()
    }

    pub(crate) fn as_str(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(self.as_bytes()).map_err(|_| SecretError::Corrupted)
    }
}

fn padded_len(len: usize) -> Result<usize, SecretError> {
    if len > MAX_SECRET_LEN {
        return Err(SecretError::Protection(ProtectionFailure::TooLarge));
    }
    Ok(len.max(1).next_multiple_of(PROTECT_BLOCK_SIZE))
}

impl SealedSecret {
    pub fn seal(protector: &dyn MemoryProtector, plaintext: &[u8]) -> Result<Self, SecretError> {
        let mut sealed = Self {
            buf: vec![0u8; padded_len(plaintext.len())?].into_boxed_slice(),
            len: plaintext.len(),
        };
        // On error, `sealed` is dropped, which wipes the clear copy.
        sealed
            .buf
            .get_mut(..plaintext.len())
            .ok_or(SecretError::Protection(ProtectionFailure::TooLarge))?
            .copy_from_slice(plaintext);
        protector.protect(&mut sealed.buf)?;
        Ok(sealed)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Decrypts a copy; the sealed buffer itself is never decrypted in place.
    pub(crate) fn unseal(&self, protector: &dyn MemoryProtector) -> Result<Plaintext, SecretError> {
        let mut buf = Zeroizing::new(self.buf.to_vec());
        protector.unprotect(&mut buf)?;
        Ok(Plaintext { buf, len: self.len })
    }
}

impl Drop for SealedSecret {
    fn drop(&mut self) {
        self.buf.zeroize();
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
