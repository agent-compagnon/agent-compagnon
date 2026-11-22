//! Sensitive key names, for JSON objects and `key=value` / `key: value` text.

use std::collections::BTreeSet;

use crate::config::FilterConfig;

/// A key is sensitive when its last word is one of these.
pub const DEFAULT_KEY_SEGMENTS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "credential",
    "credentials",
    "authorization",
    "cookie",
];

/// A key is sensitive when it is one of these, whatever its separators and case.
pub const DEFAULT_KEYS: &[&str] = &[
    "apikey",
    "api_key",
    "access_key",
    "private_key",
    "client_secret",
    "session_id",
    "x-api-key",
];

/// What a sensitive key stands for, which decides the marker and the reported kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyClass {
    Authorization,
    Cookie,
    Field,
}

pub(crate) struct SensitiveKeys {
    segments: BTreeSet<String>,
    compact: BTreeSet<String>,
}

/// Words of a key: split on every non-alphanumeric character, on camelCase boundaries (`apiKey`, `APIKey`) and between
/// letters and digits.
pub(crate) fn segments(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in key.split(|c: char| !c.is_alphanumeric()).filter(|r| !r.is_empty()) {
        let chars: Vec<char> = run.chars().collect();
        let mut current = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let boundary = i > 0 && {
                let prev = chars[i - 1];
                let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
                (c.is_uppercase() && (prev.is_lowercase() || prev.is_numeric()))
                    || (c.is_uppercase() && prev.is_uppercase() && next_lower)
                    || (c.is_numeric() != prev.is_numeric())
            };
            if boundary && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            current.extend(c.to_lowercase());
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

/// Lower-case alphanumerics only: `X-Api-Key`, `x_api_key` and `xApiKey` compare equal.
fn compact(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl SensitiveKeys {
    /// The defaults, plus the additions of `[filter]`. Additions never remove a default.
    pub(crate) fn new(config: &FilterConfig) -> Self {
        let segments = DEFAULT_KEY_SEGMENTS
            .iter()
            .copied()
            .chain(config.extra_key_segments.iter().map(String::as_str))
            .map(compact)
            .filter(|s| !s.is_empty())
            .collect();
        let compact = DEFAULT_KEYS
            .iter()
            .copied()
            .chain(config.extra_keys.iter().map(String::as_str))
            .map(compact)
            .filter(|s| !s.is_empty())
            .collect();
        Self { segments, compact }
    }

    /// `None` when the key is not sensitive.
    pub(crate) fn classify(&self, key: &str) -> Option<KeyClass> {
        let words = segments(key);
        // Trailing numbers do not count: `password2` and `token_1` are still a password and a token.
        let last = words.iter().rev().find(|w| !w.chars().all(char::is_numeric));
        let compact = compact(key);
        let sensitive = last.is_some_and(|w| self.segments.contains(w)) || self.compact.contains(&compact);
        if !sensitive {
            return None;
        }
        Some(match (last.map(String::as_str), compact.as_str()) {
            (Some("authorization"), _) => KeyClass::Authorization,
            (_, "cookie" | "setcookie" | "setcookie2") => KeyClass::Cookie,
            _ => KeyClass::Field,
        })
    }

    pub(crate) fn is_sensitive(&self, key: &str) -> bool {
        self.classify(key).is_some()
    }
}
