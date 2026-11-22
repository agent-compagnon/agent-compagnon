use secrecy::ExposeSecret;
use zeroize::Zeroizing;

use super::credential::Credential;

/// Every byte string the output filter must find for this credential. Sealed by `SecretStore::put` next to the
/// credential, exposed by `SecretStore::with_variants`.
///
/// Step 2a: raw secret values only. Step 2b adds the encoded forms (base64 with alignment offsets, hex, URL
/// encoding, JSON escaping, JWT signature, Basic `login:password`) and the length rules.
pub(crate) fn derive_variants(credential: &Credential) -> Vec<Zeroizing<Vec<u8>>> {
    let mut variants: Vec<Zeroizing<Vec<u8>>> = Vec::new();
    let mut add = |value: &[u8]| {
        if !value.is_empty() && !variants.iter().any(|v| v.as_slice() == value) {
            variants.push(Zeroizing::new(value.to_vec()));
        }
    };
    match credential {
        Credential::Bearer { token } => add(token.expose_secret().as_bytes()),
        Credential::Cookies { pairs } => {
            for (_, value) in pairs {
                add(value.expose_secret().as_bytes());
            }
        }
        Credential::Basic { password, .. } => add(password.expose_secret().as_bytes()),
    }
    variants
}
