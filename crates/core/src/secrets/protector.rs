use super::error::{ProtectionFailure, SecretError};

/// Buffers handed to a `MemoryProtector` have a length that is a non-zero multiple of this
/// (`CRYPTPROTECTMEMORY_BLOCK_SIZE` on Windows).
pub const PROTECT_BLOCK_SIZE: usize = 16;

/// Encrypts and decrypts a buffer in place, for the lifetime of the process.
pub trait MemoryProtector: Send + Sync {
    fn protect(&self, buf: &mut [u8]) -> Result<(), SecretError>;
    fn unprotect(&self, buf: &mut [u8]) -> Result<(), SecretError>;
}

pub(crate) fn check_block_len(buf: &[u8]) -> Result<(), SecretError> {
    if buf.is_empty() || !buf.len().is_multiple_of(PROTECT_BLOCK_SIZE) {
        return Err(SecretError::Protection(ProtectionFailure::Misaligned));
    }
    Ok(())
}

/// The protector of the current platform: `CryptProtectMemory` on Windows, `InsecureDevProtector` elsewhere.
pub fn platform_protector() -> Box<dyn MemoryProtector> {
    #[cfg(windows)]
    {
        Box::new(crate::platform::windows::WindowsMemoryProtector)
    }
    #[cfg(not(windows))]
    {
        Box::new(InsecureDevProtector::new())
    }
}

/// Development-only protector that does **not** encrypt anything. It exists so that the store can be tested on the
/// Linux development host; it still enforces the block alignment of the Windows API.
#[cfg(not(windows))]
#[derive(Debug)]
pub struct InsecureDevProtector(());

#[cfg(not(windows))]
impl InsecureDevProtector {
    pub fn new() -> Self {
        tracing::warn!("secrets en mémoire NON chiffrés : protecteur de développement, réservé aux tests hors Windows");
        Self(())
    }
}

#[cfg(not(windows))]
impl Default for InsecureDevProtector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(windows))]
impl MemoryProtector for InsecureDevProtector {
    fn protect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        check_block_len(buf)
    }

    fn unprotect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        check_block_len(buf)
    }
}
