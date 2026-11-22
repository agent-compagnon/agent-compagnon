use zeroize::Zeroizing;

use super::error::SecretError;

/// Keeps the session key of a `SecretStore` wrapped: the store only ever holds what `wrap` returned, and opens it
/// with `unwrap` once per use.
pub trait KeyWrapper: Send + Sync {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError>;
    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError>;
}

/// The wrapper of the current platform: user-scoped DPAPI on Windows, `InsecureDevKeyWrapper` elsewhere.
pub fn platform_key_wrapper() -> Box<dyn KeyWrapper> {
    #[cfg(windows)]
    {
        Box::new(DpapiKeyWrapper)
    }
    #[cfg(not(windows))]
    {
        Box::new(InsecureDevKeyWrapper::new())
    }
}

/// Wraps the session key with user-scoped DPAPI (WinRT projection, `crate::platform::windows`).
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DpapiKeyWrapper;

#[cfg(windows)]
fn dpapi_failure(error: crate::platform::windows::DpapiError) -> SecretError {
    SecretError::Protection(super::error::ProtectionFailure::Os(error.0))
}

#[cfg(windows)]
impl KeyWrapper for DpapiKeyWrapper {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError> {
        crate::platform::windows::protect(key).map_err(dpapi_failure)
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        crate::platform::windows::unprotect(wrapped).map_err(dpapi_failure)
    }
}

/// Development-only wrapper that keeps the session key **as is**. It exists so that the store can be tested on the
/// Linux development host; it is not compiled on Windows.
#[cfg(not(windows))]
#[derive(Debug)]
pub struct InsecureDevKeyWrapper(());

#[cfg(not(windows))]
impl InsecureDevKeyWrapper {
    pub fn new() -> Self {
        tracing::warn!("secrets en mémoire NON chiffrés : protecteur de développement, réservé aux tests hors Windows");
        Self(())
    }
}

#[cfg(not(windows))]
impl Default for InsecureDevKeyWrapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(windows))]
impl KeyWrapper for InsecureDevKeyWrapper {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError> {
        Ok(key.to_vec())
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        Ok(Zeroizing::new(wrapped.to_vec()))
    }
}
