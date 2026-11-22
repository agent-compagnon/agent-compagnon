//! What `cargo xtask keygen`, `sign`, `verify` and `check-release` do, through the `update-tool` binary of this
//! crate (xtask stays without dependencies). The private key only ever comes from the environment and is never
//! written anywhere but the file `keygen` creates: no message, no error and no `Debug` shows it.

use std::fmt;

use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::check::{UpdateError, verify_exe, verify_signature};
use super::keys::{KeyId, KeyRing};
use super::manifest::Manifest;
use super::slots::hex_key;
use crate::version::{Installed, Version, version_in};

/// Default variable of the private key, the one the release pipeline sets (spec 021).
pub const SIGNING_KEY_ENV: &str = "ED25519_PRIVATE_KEY";

/// Why the private key was not usable. Never says anything of its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTextError {
    Missing,
    Malformed,
}

impl fmt::Display for KeyTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "variable de la clé privée absente ou vide",
            Self::Malformed => "clé privée illisible : 64 caractères hexadécimaux minuscules attendus",
        })
    }
}

/// The 32-byte seed in hexadecimal, surrounding blanks ignored.
pub fn signing_key(text: Option<Zeroizing<String>>) -> Result<SigningKey, KeyTextError> {
    let text = text.ok_or(KeyTextError::Missing)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(KeyTextError::Missing);
    }
    let seed = Zeroizing::new(hex_key(trimmed).ok_or(KeyTextError::Malformed)?);
    Ok(SigningKey::from_bytes(&seed))
}

/// A new key pair: the seed, to keep secret, and the public key.
pub fn generate() -> Result<(Zeroizing<String>, [u8; 32]), getrandom::Error> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut())?;
    let key = SigningKey::from_bytes(&seed);
    Ok((Zeroizing::new(hex(seed.as_ref())), key.verifying_key().to_bytes()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `cargo xtask sign`: the manifest of an exe, whose version is the one of its mark (and of `--version`, if given).
pub fn sign_exe(exe: &[u8], expected: Option<&Version>, critical: bool, key: &SigningKey) -> Result<Manifest, String> {
    let Installed::Known(version) = version_in(exe) else {
        return Err("marque de version de l'exe absente ou contradictoire".to_owned());
    };
    if let Some(expected) = expected
        && *expected != version
    {
        return Err(format!("l'exe porte la version {version}, pas {expected}"));
    }
    Ok(Manifest::sign(&version, Sha256::digest(exe).into(), critical, key))
}

/// `cargo xtask verify`: the manifest is well formed and signed by one of `keys`, and describes this exe (digest and
/// version mark).
pub fn verify_release(exe: &[u8], sig: &[u8], keys: &KeyRing) -> Result<Manifest, UpdateError> {
    let manifest = Manifest::parse(sig).map_err(UpdateError::Manifest)?;
    verify_signature(&manifest, keys)?;
    verify_exe(&manifest, exe)?;
    Ok(manifest)
}

/// One line about a public key: hexadecimal and identifier.
pub fn describe_key(key: &[u8; 32]) -> String {
    format!("{} (key-id {})", hex(key), KeyId::of(key))
}
