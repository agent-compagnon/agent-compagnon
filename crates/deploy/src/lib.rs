//! Installation, uninstallation and updater. Everything that decides lives here and is tested under Linux; the
//! application only implements [`ops::SystemOps`] for Windows.

pub mod clients;
pub mod fake;
pub mod install;
pub mod ops;
pub mod uninstall;
pub mod update;
pub mod version;
