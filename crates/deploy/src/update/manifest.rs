//! The `.sig` file published next to each exe: UTF-8, LF line endings, six lines in this order.
//!
//! ```text
//! mcp-proxy-signature v1
//! version: 1.4.0
//! sha256: <64 lowercase hexadecimal characters>
//! critical: false
//! key-id: <8 lowercase hexadecimal characters>
//! signature: <standard base64>
//! ```
//!
//! `signature` is the Ed25519 signature of every byte before its line, line endings included. Reading is strict:
//! one spelling only for every field, nothing before, between or after the lines.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer as _, SigningKey};

use super::keys::KeyId;
use crate::version::Version;

pub const HEADER: &str = "mcp-proxy-signature v1";
/// Larger files are not read: a manifest is about 250 bytes.
pub const MANIFEST_MAX: usize = 1024;

/// What a manifest says. Only built by [`Manifest::parse`] or [`Manifest::sign`].
#[derive(Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    pub sha256: [u8; 32],
    pub critical: bool,
    pub key_id: KeyId,
    pub(crate) signature: Signature,
    /// The bytes the signature covers.
    pub(crate) signed: Vec<u8>,
}

impl fmt::Debug for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manifest")
            .field("version", &self.version.to_string())
            .field("critical", &self.critical)
            .field("key_id", &self.key_id.to_string())
            .finish_non_exhaustive()
    }
}

/// Why a manifest is not well formed. Names the line, never quotes the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    TooLarge,
    NotUtf8,
    /// A carriage return: only LF line endings are accepted.
    CarriageReturn,
    /// The file ends before this line, or its last line has no line ending.
    Missing(&'static str),
    /// This line is not exactly what is expected.
    Invalid(&'static str),
    /// Something after the signature line.
    Trailing,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(f, "manifeste de plus de {MANIFEST_MAX} octets"),
            Self::NotUtf8 => f.write_str("manifeste hors UTF-8"),
            Self::CarriageReturn => f.write_str("manifeste avec des fins de ligne CRLF"),
            Self::Missing(line) => write!(f, "ligne « {line} » absente du manifeste"),
            Self::Invalid(line) => write!(f, "ligne « {line} » du manifeste mal formée"),
            Self::Trailing => f.write_str("contenu inattendu après la signature du manifeste"),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0; N];
    for (i, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// The lines before the signature, each ending with LF: what the signature covers.
fn signed_lines(version: &Version, sha256: &[u8; 32], critical: bool, key_id: KeyId) -> String {
    format!(
        "{HEADER}\nversion: {version}\nsha256: {}\ncritical: {critical}\nkey-id: {key_id}\n",
        hex(sha256)
    )
}

/// Reads the lines one by one, each ending with LF.
struct Lines<'a> {
    rest: &'a str,
    offset: usize,
}

impl<'a> Lines<'a> {
    fn next(&mut self, name: &'static str) -> Result<&'a str, ManifestError> {
        let end = self.rest.find('\n').ok_or(ManifestError::Missing(name))?;
        let line = &self.rest[..end];
        self.rest = &self.rest[end + 1..];
        self.offset += end + 1;
        Ok(line)
    }

    /// The value of `name: value`.
    fn field(&mut self, name: &'static str) -> Result<&'a str, ManifestError> {
        let line = self.next(name)?;
        line.strip_prefix(name)
            .and_then(|l| l.strip_prefix(": "))
            .ok_or(ManifestError::Invalid(name))
    }
}

impl Manifest {
    /// Strict reading: one spelling only for every field.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MANIFEST_MAX {
            return Err(ManifestError::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| ManifestError::NotUtf8)?;
        if text.contains('\r') {
            return Err(ManifestError::CarriageReturn);
        }
        let mut lines = Lines { rest: text, offset: 0 };
        if lines.next("mcp-proxy-signature")? != HEADER {
            return Err(ManifestError::Invalid("mcp-proxy-signature"));
        }
        let version = lines.field("version")?;
        let version = Version::parse(version)
            .ok()
            .filter(|v| v.to_string() == version)
            .ok_or(ManifestError::Invalid("version"))?;
        let sha256 = unhex::<32>(lines.field("sha256")?).ok_or(ManifestError::Invalid("sha256"))?;
        let critical = match lines.field("critical")? {
            "true" => true,
            "false" => false,
            _ => return Err(ManifestError::Invalid("critical")),
        };
        let key_id = KeyId(unhex::<4>(lines.field("key-id")?).ok_or(ManifestError::Invalid("key-id"))?);
        let signed_len = lines.offset;
        let encoded = lines.field("signature")?;
        let signature = STANDARD
            .decode(encoded)
            .ok()
            // Only the canonical encoding: no other spelling of the same bytes.
            .filter(|decoded| STANDARD.encode(decoded) == encoded)
            .and_then(|decoded| <[u8; 64]>::try_from(decoded).ok())
            .ok_or(ManifestError::Invalid("signature"))?;
        if !lines.rest.is_empty() {
            return Err(ManifestError::Trailing);
        }
        Ok(Self {
            version,
            sha256,
            critical,
            key_id,
            signature: Signature::from_bytes(&signature),
            signed: bytes[..signed_len].to_vec(),
        })
    }

    /// Signs the version and the digest of an exe. Used by `cargo xtask sign` and by tests.
    pub fn sign(version: &Version, sha256: [u8; 32], critical: bool, key: &SigningKey) -> Self {
        let key_id = KeyId::of(key.verifying_key().as_bytes());
        let signed = signed_lines(version, &sha256, critical, key_id).into_bytes();
        let signature = key.sign(&signed);
        Self {
            version: version.clone(),
            sha256,
            critical,
            key_id,
            signature,
            signed,
        }
    }

    /// The file, as published.
    pub fn render(&self) -> String {
        format!(
            "{}signature: {}\n",
            String::from_utf8_lossy(&self.signed),
            STANDARD.encode(self.signature.to_bytes())
        )
    }

    pub fn sha256_hex(&self) -> String {
        hex(&self.sha256)
    }
}
