//! In-memory secrets: sealed outside of use, bound to one session, only sent to that session's origins.
//!
//! A secret is only ever readable inside a closure (`SecretStore::with_exposed`, `SecretStore::with_variants`), on a
//! decrypted copy that is wiped when the closure returns. No API hands the clear value back to the caller.

mod credential;
mod error;
mod origin;
mod protector;
mod sealed;
mod store;
mod variants;

#[cfg(test)]
mod tests;

pub use credential::{Credential, ExposedCookie, ExposedCredential, MAX_SECRET_LEN};
pub use error::{ProtectionFailure, SecretError};
pub use origin::OriginGuard;
#[cfg(not(windows))]
pub use protector::InsecureDevProtector;
#[cfg(any(windows, test))]
pub(crate) use protector::check_block_len;
pub use protector::{MemoryProtector, PROTECT_BLOCK_SIZE, platform_protector};
pub use sealed::SealedSecret;
pub use store::SecretStore;
