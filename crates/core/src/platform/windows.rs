//! Windows: `CryptProtectMemory` with `CRYPTPROTECTMEMORY_SAME_PROCESS`.

use windows::Win32::Security::Cryptography::{
    CRYPTPROTECTMEMORY_BLOCK_SIZE, CRYPTPROTECTMEMORY_SAME_PROCESS, CryptProtectMemory, CryptUnprotectMemory,
};

use crate::secrets::{MemoryProtector, PROTECT_BLOCK_SIZE, ProtectionFailure, SecretError};

const _: () = assert!(PROTECT_BLOCK_SIZE == CRYPTPROTECTMEMORY_BLOCK_SIZE as usize);

/// Encrypts with a key that only lives in the current process: a memory dump or another process cannot decrypt it.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsMemoryProtector;

fn checked_len(buf: &[u8]) -> Result<u32, SecretError> {
    crate::secrets::check_block_len(buf)?;
    u32::try_from(buf.len()).map_err(|_| SecretError::Protection(ProtectionFailure::TooLarge))
}

fn os_failure(error: &windows::core::Error) -> SecretError {
    SecretError::Protection(ProtectionFailure::Os(error.code().0))
}

impl MemoryProtector for WindowsMemoryProtector {
    fn protect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        let len = checked_len(buf)?;
        // SAFETY: `buf` is a live, exclusively borrowed slice of exactly `len` bytes, a multiple of
        // CRYPTPROTECTMEMORY_BLOCK_SIZE as `checked_len` verified; the call only reads and writes inside it.
        unsafe { CryptProtectMemory(buf.as_mut_ptr().cast(), len, CRYPTPROTECTMEMORY_SAME_PROCESS) }
            .map_err(|e| os_failure(&e))
    }

    fn unprotect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        let len = checked_len(buf)?;
        // SAFETY: same as `protect`: an exclusively borrowed slice of `len` bytes, block-aligned.
        unsafe { CryptUnprotectMemory(buf.as_mut_ptr().cast(), len, CRYPTPROTECTMEMORY_SAME_PROCESS) }
            .map_err(|e| os_failure(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exigence : EX-002-01
    #[test]
    fn round_trip_encrypts_in_place() {
        let original: Vec<u8> = (0u8..64).collect();
        let mut buf = original.clone();
        WindowsMemoryProtector.protect(&mut buf).unwrap();
        assert_ne!(buf, original);
        WindowsMemoryProtector.unprotect(&mut buf).unwrap();
        assert_eq!(buf, original);
    }

    // Exigence : EX-002-01
    #[test]
    fn misaligned_buffer_is_refused() {
        let mut buf = [0u8; 15];
        assert_eq!(
            WindowsMemoryProtector.protect(&mut buf),
            Err(SecretError::Protection(ProtectionFailure::Misaligned))
        );
        assert_eq!(
            WindowsMemoryProtector.unprotect(&mut []),
            Err(SecretError::Protection(ProtectionFailure::Misaligned))
        );
    }
}
