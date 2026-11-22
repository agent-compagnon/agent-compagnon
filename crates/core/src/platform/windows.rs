//! Windows: user-scoped DPAPI through the WinRT projection of the `windows` crate, without any `unsafe`.
//!
//! `protect` seals bytes so that only a process of the same Windows user can open them (`LOCAL=user` descriptor);
//! `unprotect` opens them. Shared by the secret store (session key), the sealed settings (spec 014) and the proof of
//! identity of the named pipe (spec 011).
//!
//! Known limit: the WinRT buffers that hold the clear bytes on their way to and from DPAPI are released by the
//! runtime without being wiped; only the copies made here are.

use std::fmt;

use windows::Security::Cryptography::CryptographicBuffer;
use windows::Security::Cryptography::DataProtection::DataProtectionProvider;
use windows::Storage::Streams::IBuffer;
use windows::core::{Array, HSTRING};
use zeroize::{Zeroize, Zeroizing};

/// Protection descriptor: the current Windows user, on this machine.
const USER_SCOPE: &str = "LOCAL=user";

/// A DPAPI failure: the `HRESULT` only, never any data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpapiError(pub i32);

impl From<windows::core::Error> for DpapiError {
    fn from(error: windows::core::Error) -> Self {
        Self(error.code().0)
    }
}

impl fmt::Display for DpapiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DPAPI : code système {:#010x}", self.0)
    }
}

/// Copies a WinRT buffer into a wiped-on-drop vector, then wipes the intermediate array.
fn read_buffer(buffer: &IBuffer) -> Result<Zeroizing<Vec<u8>>, DpapiError> {
    let mut array = Array::<u8>::new();
    let copied = CryptographicBuffer::CopyToByteArray(buffer, &mut array);
    let out = Zeroizing::new(array.to_vec());
    array.zeroize();
    copied?;
    Ok(out)
}

/// Seals `data` for the current Windows user.
pub fn protect(data: &[u8]) -> Result<Vec<u8>, DpapiError> {
    let provider = DataProtectionProvider::CreateOverloadExplicit(&HSTRING::from(USER_SCOPE))?;
    let clear = CryptographicBuffer::CreateFromByteArray(data)?;
    let protected = provider.ProtectAsync(&clear)?.join()?;
    Ok(read_buffer(&protected)?.to_vec())
}

/// Opens bytes sealed by `protect` for the current Windows user. Fails for data sealed for another user.
pub fn unprotect(data: &[u8]) -> Result<Zeroizing<Vec<u8>>, DpapiError> {
    let provider = DataProtectionProvider::new()?;
    let protected = CryptographicBuffer::CreateFromByteArray(data)?;
    let clear = provider.UnprotectAsync(&protected)?.join()?;
    read_buffer(&clear)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Run on Windows only (EX-002-16, manual): cross-compiled from Linux, never executed there.
    #[test]
    fn dpapi_round_trip() {
        let original: Vec<u8> = (0u8..32).collect();
        let protected = protect(&original).unwrap();
        assert_ne!(protected, original);
        assert!(!protected.windows(original.len()).any(|w| w == original.as_slice()));
        assert_eq!(unprotect(&protected).unwrap().as_slice(), original.as_slice());
    }

    #[test]
    fn garbage_is_refused() {
        let opened = unprotect(&[0x5a; 64]);
        assert!(opened.map(|clear| clear.is_empty()).unwrap_or(true));
    }
}
