//! The updater (spec 020): the exe updates itself from a generic release repository of Artifactory, read
//! anonymously (D11), and never installs a binary that is not authentic, intact and strictly more recent (INV-7).
//!
//! Platform independent: the HTTP client ([`Fetch`]) and the replacement ([`ExeReplacer`]) are injected. The
//! application gives [`AnonymousClient`] and [`RenameReplacer`] over its `SystemOps`; the gateway schedules the
//! checks and applies the result (D12).

mod check;
mod discover;
mod fetch;
mod keys;
mod manifest;
pub mod release;
mod replace;
mod schedule;
pub mod slots;
pub mod tool_commands;

#[cfg(test)]
mod tests;

pub use check::{
    CheckOutcome, EXE_DOWNLOAD_MAX, SIG_NAME, Source, UpdateError, UpdateReady, Updater, UpdaterDeps, verify_exe,
    verify_manifest, verify_signature,
};
pub use discover::{Channel, ListingError, choose, parse_listing};
pub use fetch::{AnonymousClient, Fetch, FetchError, Limits, MAX_REDIRECTS, allowed};
pub use keys::{DEV_KEY_FINGERPRINT, KeyError, KeyId, KeyRing, check_release, embedded_slots, fingerprint, is_dev_key};
pub use manifest::{HEADER, MANIFEST_MAX, Manifest, ManifestError};
pub use replace::{
    Cleanup, ExeReplacer, RenameReplacer, ReplaceError, ReplaceStep, UpdatePaths, VerifiedExe, clean_leftovers,
    clear_backup, is_leftover,
};
pub use schedule::{FIRST_CHECK_DELAY, check_interval};
