//! The two public key slots of the exe (current and next, for a rotation), read at build time from `keys/`. Shared,
//! as source, with `build.rs`: the build and the tests apply the very same rules. Standard library only.

/// Length of an Ed25519 public key.
pub const KEY_LEN: usize = 32;

/// The content of a slot file: `#` comments and blank lines, then at most one line of 64 lowercase hexadecimal
/// characters. No such line: the slot is empty.
pub fn parse_slot(text: &str) -> Result<Option<[u8; KEY_LEN]>, String> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    let Some(line) = lines.next() else {
        return Ok(None);
    };
    if lines.next().is_some() {
        return Err("plus d'une clé dans l'emplacement".to_owned());
    }
    hex_key(line)
        .map(Some)
        .ok_or_else(|| "clé illisible : 64 caractères hexadécimaux minuscules attendus".to_owned())
}

/// 64 lowercase hexadecimal characters.
pub fn hex_key(text: &str) -> Option<[u8; KEY_LEN]> {
    let bytes = text.as_bytes();
    if bytes.len() != KEY_LEN * 2 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut key = [0; KEY_LEN];
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        key[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(key)
}

/// Every build: the development key, whose private half is public, is in no slot.
pub fn build_guard(slots: &[Option<[u8; KEY_LEN]>; 2], dev: &[u8; KEY_LEN]) -> Result<(), String> {
    if slots.iter().flatten().any(|key| key == dev) {
        return Err(
            "la clé de développement figure dans un emplacement de clé de mise à jour (keys/current.pub \
                    ou keys/next.pub) : n'importe qui pourrait signer une mise à jour acceptée par cet exe"
                .to_owned(),
        );
    }
    Ok(())
}

/// A release build (`MCP_PROXY_RELEASE=1`) and `cargo xtask check-release`: [`build_guard`], and a current key, without
/// which the published exe could never be updated again.
pub fn release_guard(slots: &[Option<[u8; KEY_LEN]>; 2], dev: &[u8; KEY_LEN]) -> Result<(), String> {
    build_guard(slots, dev)?;
    if slots[0].is_none() {
        return Err(
            "aucune clé de mise à jour courante (keys/current.pub) : la version publiée ne pourrait plus \
                    jamais être mise à jour"
                .to_owned(),
        );
    }
    Ok(())
}
