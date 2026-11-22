//! Platform-specific code. Like the rest of the workspace, it is written with safe APIs only: `unsafe` is forbidden.

#[cfg(windows)]
pub mod windows;
