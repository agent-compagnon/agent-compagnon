//! Platform-specific code. `unsafe` is allowed in these submodules only.

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;
