//! In-memory secrets: sealed outside of use, bound to one session, only sent to that session's origins.
//!
//! A secret is only ever readable inside a closure (`SecretStore::with_exposed`, `SecretStore::with_variants`), on a
//! decrypted copy that is wiped when the closure returns. No API hands the clear value back to the caller.
//!
//! Protection is an envelope: values are sealed with an AEAD under a random session key, and that key is only kept
//! wrapped by a `KeyWrapper` (user-scoped DPAPI on Windows), unwrapped once per use.

mod clock;
mod credential;
mod error;
pub(crate) mod http;
mod key;
mod origin;
mod sealed;
mod store;
pub(crate) mod variants;
mod wrapper;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub use clock::{Clock, SystemClock};
pub use credential::{Credential, ExposedCookie, ExposedCredential, MAX_SECRET_LEN};
pub use error::{ProtectionFailure, SecretError};
pub use http::{AuthenticatedHttpClient, basic_authorization};
pub use key::SESSION_KEY_LEN;
pub use origin::OriginGuard;
pub use sealed::SealedSecret;
pub use store::{DEFAULT_VARIANT_GRACE, SecretStore, SecretVersion};
#[cfg(windows)]
pub use wrapper::DpapiKeyWrapper;
#[cfg(not(windows))]
pub use wrapper::InsecureDevKeyWrapper;
pub use wrapper::{KeyWrapper, platform_key_wrapper};
