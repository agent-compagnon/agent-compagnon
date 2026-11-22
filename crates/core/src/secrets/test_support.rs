//! Test doubles of `KeyWrapper`, shared by the secret store and output filter tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use zeroize::Zeroizing;

use super::{KeyWrapper, ProtectionFailure, SecretError};

/// Wrapper that really transforms the key (so that using the wrapped form as the key would be noticed), counts its
/// calls, and can be switched to failing after the store was created.
#[derive(Clone, Default)]
pub(crate) struct TestWrapper {
    pub(crate) wraps: Arc<AtomicUsize>,
    pub(crate) unwraps: Arc<AtomicUsize>,
    pub(crate) broken: Arc<AtomicBool>,
}

impl TestWrapper {
    pub(crate) fn unwraps(&self) -> usize {
        self.unwraps.load(Ordering::SeqCst)
    }

    /// From now on, every `wrap` and `unwrap` fails.
    pub(crate) fn break_now(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), SecretError> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(SecretError::Protection(ProtectionFailure::Os(-1)));
        }
        Ok(())
    }
}

fn xor(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|b| b ^ 0xa5).collect()
}

impl KeyWrapper for TestWrapper {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError> {
        self.wraps.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        Ok(xor(key))
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        self.unwraps.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        Ok(Zeroizing::new(xor(wrapped)))
    }
}

/// Wrapper that gives back something other than the key it was given.
pub(crate) struct LyingWrapper;

impl KeyWrapper for LyingWrapper {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError> {
        Ok(key.to_vec())
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        Ok(Zeroizing::new(wrapped.iter().map(|b| b.wrapping_add(1)).collect()))
    }
}
