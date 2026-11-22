//! The public keys an update may be signed with: two slots embedded in the exe (current and next, for a rotation),
//! never the development key.

use std::fmt;

use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};

use super::slots::{KEY_LEN, release_guard};

include!(concat!(env!("OUT_DIR"), "/embedded_keys.rs"));

/// SHA-256 of the development public key (`keys/dev.pub`). The key itself is not compiled into the exe, so that
/// `cargo xtask check-release --exe` can look for its bytes in a build: they are only there if a slot holds it.
pub const DEV_KEY_FINGERPRINT: [u8; 32] = [
    0xb2, 0xf6, 0xa8, 0x26, 0xd6, 0xbc, 0x5d, 0x7b, 0x68, 0x8d, 0xc7, 0x67, 0x06, 0x0b, 0xd5, 0xb8, 0x81, 0xdb, 0x86,
    0x71, 0xca, 0x6b, 0xf8, 0xfd, 0x58, 0xc0, 0xec, 0x32, 0xae, 0xd8, 0x7a, 0xb7,
];

/// The 4 first bytes of the SHA-256 of a public key, written as 8 lowercase hexadecimal characters in a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyId(pub [u8; 4]);

impl KeyId {
    pub fn of(key: &[u8; KEY_LEN]) -> Self {
        let digest: [u8; 32] = Sha256::digest(key).into();
        Self([digest[0], digest[1], digest[2], digest[3]])
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

/// The SHA-256 of a public key.
pub fn fingerprint(key: &[u8; KEY_LEN]) -> [u8; 32] {
    Sha256::digest(key).into()
}

pub fn is_dev_key(key: &[u8; KEY_LEN]) -> bool {
    fingerprint(key) == DEV_KEY_FINGERPRINT
}

/// The keys accepted for an update.
#[derive(Clone, Default)]
pub struct KeyRing {
    keys: Vec<(KeyId, VerifyingKey)>,
}

impl fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.keys.iter().map(|(id, _)| id.to_string()))
            .finish()
    }
}

/// A key that cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Not a point of the curve.
    Invalid,
    /// The development key, refused by every exe.
    Development,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "clé publique invalide",
            Self::Development => "clé de développement refusée",
        })
    }
}

fn verifying_key(bytes: &[u8; KEY_LEN]) -> Result<VerifyingKey, KeyError> {
    VerifyingKey::from_bytes(bytes).map_err(|_| KeyError::Invalid)
}

impl KeyRing {
    /// Exactly these keys, the development key included: for tests and for `cargo xtask verify`.
    pub fn new(keys: &[[u8; KEY_LEN]]) -> Result<Self, KeyError> {
        let keys = keys
            .iter()
            .map(|bytes| Ok((KeyId::of(bytes), verifying_key(bytes)?)))
            .collect::<Result<_, KeyError>>()?;
        Ok(Self { keys })
    }

    /// The keys of the slots, without the development key nor an invalid key, each left out with a line of log.
    pub fn from_slots(slots: &[Option<[u8; KEY_LEN]>]) -> Self {
        let mut keys = Vec::new();
        for (slot, bytes) in slots.iter().enumerate() {
            let Some(bytes) = bytes else { continue };
            let id = KeyId::of(bytes);
            let key = if is_dev_key(bytes) {
                Err(KeyError::Development)
            } else {
                verifying_key(bytes)
            };
            match key {
                Ok(key) => keys.push((id, key)),
                Err(error) => tracing::error!(slot, key_id = %id, %error, "clé de mise à jour écartée"),
            }
        }
        Self { keys }
    }

    /// The keys embedded in this exe.
    pub fn embedded() -> Self {
        Self::from_slots(&embedded_slots())
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn ids(&self) -> Vec<KeyId> {
        self.keys.iter().map(|(id, _)| *id).collect()
    }

    /// Every key of this identifier (two keys of the same identifier are possible, though unlikely).
    pub(crate) fn find(&self, id: KeyId) -> impl Iterator<Item = &VerifyingKey> {
        self.keys.iter().filter(move |(k, _)| *k == id).map(|(_, key)| key)
    }
}

/// The slots compiled into this exe, as `keys/` gave them. Read through `black_box`, so that their bytes are in
/// every exe, where `cargo xtask check-release --exe` looks for them.
pub fn embedded_slots() -> [Option<[u8; KEY_LEN]>; 2] {
    *std::hint::black_box(&SLOTS)
}

/// `cargo xtask check-release`: the slots compiled in pass [`release_guard`]; with the bytes of a built exe, the
/// development key is not in it and the current key is.
pub fn check_release(
    slots: &[Option<[u8; KEY_LEN]>; 2],
    dev: &[u8; KEY_LEN],
    exe: Option<&[u8]>,
) -> Result<(), String> {
    release_guard(slots, dev)?;
    let Some(exe) = exe else { return Ok(()) };
    if memchr::memmem::find(exe, dev).is_some() {
        return Err("l'exe contient la clé de développement".to_owned());
    }
    match &slots[0] {
        Some(current) if memchr::memmem::find(exe, current).is_some() => Ok(()),
        _ => Err(
            "l'exe ne contient pas la clé de mise à jour courante : il n'a pas été construit avec ces clés".to_owned(),
        ),
    }
}
