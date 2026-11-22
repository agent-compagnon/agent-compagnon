//! Output filter: nothing that leaves the process (to the LLM, to the logs, to a confirmation) may carry a secret.
//!
//! Two layers, in this order:
//! 1. the blacklist of exact values: every variant of every known secret (`SecretStore::with_variants`), searched
//!    with `memchr::memmem` on needles borrowed for the duration of one pass, never copied nor compiled;
//! 2. generic patterns: JWTs, `Authorization` and cookie headers, sensitive fields, private keys, passwords in URLs.
//!
//! Both layers are applied until the text no longer changes, so filtering is idempotent. The filter fails closed: if
//! the blacklist cannot be read, nothing is returned but an error, and the caller must not send anything.

mod exact;
mod generic;
mod keys;
mod writer;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::config::FilterConfig;
use crate::secrets::{SecretError, SecretStore};
use generic::GenericPatterns;
pub use keys::{DEFAULT_KEY_SEGMENTS, DEFAULT_KEYS};
pub use writer::{FilteringMakeWriter, FilteringWriter, UNFILTERED_RECORD};

/// Replaces a known secret (exact value or one of its variants), a private key or a password in a URL.
pub const SECRET_MARKER: &str = "[SECRET MASQUÉ]";
/// Replaces a JWT found by its shape.
pub const JWT_MARKER: &str = "[JWT MASQUÉ]";
/// Replaces the value of an `Authorization`, `Cookie` or `Set-Cookie` header.
pub const HEADER_MARKER: &str = "[EN-TÊTE MASQUÉ]";
/// Replaces the value of a sensitive field.
pub const FIELD_MARKER: &str = "[CHAMP MASQUÉ]";
pub const MARKERS: [&str; 4] = [SECRET_MARKER, JWT_MARKER, HEADER_MARKER, FIELD_MARKER];

/// Passes after which a text that keeps changing is replaced as a whole. Every pass that changes something replaces
/// at least one byte outside markers, so this bound is only reached by pathological inputs.
const MAX_PASSES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum HitKind {
    /// A variant of a known secret.
    KnownSecret,
    Jwt,
    AuthorizationHeader,
    CookieHeader,
    SensitiveField,
    PrivateKey,
    UrlCredentials,
}

/// What a filter pass masked. Never contains the value found, nor where it was.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterReport {
    pub exact_hits: u32,
    pub generic_hits: u32,
    /// Kinds of what was masked, sorted, without duplicates.
    pub kinds: Vec<HitKind>,
}

impl FilterReport {
    pub fn is_clean(&self) -> bool {
        self.exact_hits == 0 && self.generic_hits == 0
    }

    pub fn merge(&mut self, other: &FilterReport) {
        self.exact_hits = self.exact_hits.saturating_add(other.exact_hits);
        self.generic_hits = self.generic_hits.saturating_add(other.generic_hits);
        for kind in &other.kinds {
            self.kind(*kind);
        }
    }

    fn kind(&mut self, kind: HitKind) {
        if let Err(at) = self.kinds.binary_search(&kind) {
            self.kinds.insert(at, kind);
        }
    }

    fn exact_n(&mut self, n: u32) {
        if n > 0 {
            self.exact_hits = self.exact_hits.saturating_add(n);
            self.kind(HitKind::KnownSecret);
        }
    }

    pub(crate) fn generic(&mut self, kind: HitKind) {
        self.generic_n(kind, 1);
    }

    pub(crate) fn generic_n(&mut self, kind: HitKind, n: usize) {
        if n > 0 {
            self.generic_hits = self.generic_hits.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
            self.kind(kind);
        }
    }
}

/// Errors of the output filter. On any of them, the content must not be sent.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum FilterError {
    /// The blacklist could not be read: filtering with a partial list could let a secret through.
    #[error("filtre de sortie indisponible : contenu non transmis ({0})")]
    Unavailable(#[source] SecretError),

    #[error("filtre de sortie : motif générique invalide")]
    Pattern,
}

/// The output filter. Cheap to share behind an `Arc`.
pub struct OutputFilter {
    store: Arc<SecretStore>,
    patterns: GenericPatterns,
}

impl fmt::Debug for OutputFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputFilter")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

/// Needles of one pass: borrowed from `with_variants`, deduplicated, never copied.
fn needles<'a>(variants: &[&'a [u8]]) -> Vec<&'a [u8]> {
    let mut needles: Vec<&[u8]> = variants.iter().copied().filter(|v| !v.is_empty()).collect();
    needles.sort_unstable();
    needles.dedup();
    needles
}

impl OutputFilter {
    /// Generic patterns are compiled once; the sensitive keys are the defaults plus the additions of `[filter]`.
    pub fn new(store: Arc<SecretStore>, config: &FilterConfig) -> Result<Self, FilterError> {
        Ok(Self {
            store,
            patterns: GenericPatterns::new(keys::SensitiveKeys::new(config))?,
        })
    }

    fn with_needles<R>(&self, f: impl FnOnce(&[&[u8]]) -> R) -> Result<R, FilterError> {
        self.store
            .with_variants(|variants| f(&needles(variants)))
            .map_err(FilterError::Unavailable)
    }

    /// Filters every key, string leaf and number of `value` in place: blacklist, then generic patterns, and string
    /// values of sensitive keys. Numbers and booleans are only touched if they contain a known secret.
    ///
    /// On error, `value` is replaced by `SECRET_MARKER` so that even a caller that ignored the error sends nothing.
    pub fn filter_json(&self, value: &mut Value) -> Result<FilterReport, FilterError> {
        let result = self.with_needles(|needles| {
            let mut report = FilterReport::default();
            self.walk(value, needles, &mut report);
            report
        });
        if result.is_err() {
            *value = Value::String(SECRET_MARKER.to_owned());
        }
        result
    }

    /// Filters a text: blacklist, then generic patterns. Borrows `text` when nothing was masked.
    pub fn filter_text<'a>(&self, text: &'a str) -> Result<(Cow<'a, str>, FilterReport), FilterError> {
        self.with_needles(|needles| {
            let mut report = FilterReport::default();
            let filtered = self.filter_str(text, needles, &mut report);
            (filtered.map_or(Cow::Borrowed(text), Cow::Owned), report)
        })
    }

    /// Blacklist only, on keys, string leaves and numbers: used to block incoming arguments.
    pub fn contains_known_secret(&self, value: &Value) -> Result<bool, FilterError> {
        self.with_needles(|needles| {
            let mut stack = vec![value];
            while let Some(node) = stack.pop() {
                match node {
                    Value::String(s) if exact::contains(s, needles) => return true,
                    Value::Number(n) if exact::contains(&n.to_string(), needles) => return true,
                    Value::Array(items) => stack.extend(items),
                    Value::Object(map) => {
                        if map.keys().any(|k| exact::contains(k, needles)) {
                            return true;
                        }
                        stack.extend(map.values());
                    }
                    _ => {}
                }
            }
            false
        })
    }

    /// Both layers until a fixed point. `None` when nothing changed.
    fn filter_str(&self, text: &str, needles: &[&[u8]], report: &mut FilterReport) -> Option<String> {
        let mut current: Option<String> = None;
        for _ in 0..MAX_PASSES {
            let input = current.as_deref().unwrap_or(text);
            let mut changed = None;
            if let Some((masked, n)) = exact::mask(input, needles) {
                report.exact_n(n);
                changed = Some(masked);
            }
            let after_exact = changed.as_deref().unwrap_or(input);
            if let Some(masked) = self.patterns.apply(after_exact, report) {
                changed = Some(masked);
            }
            match changed {
                Some(next) if next != input => current = Some(next),
                _ => return current,
            }
        }
        report.exact_n(1);
        Some(SECRET_MARKER.to_owned())
    }

    /// Iterative, so that a deeply nested document cannot overflow the stack.
    fn walk(&self, root: &mut Value, needles: &[&[u8]], report: &mut FilterReport) {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            match node {
                Value::String(s) => {
                    if let Some(filtered) = self.filter_str(s, needles, report) {
                        *s = filtered;
                    }
                }
                Value::Number(n) => {
                    if let Some((masked, hits)) = exact::mask(&n.to_string(), needles) {
                        report.exact_n(hits);
                        *node = Value::String(masked);
                    }
                }
                Value::Array(items) => stack.extend(items.iter_mut()),
                Value::Object(map) => {
                    *map = self.filter_object(std::mem::take(map), needles, report);
                    stack.extend(map.values_mut());
                }
                Value::Bool(_) | Value::Null => {}
            }
        }
    }

    /// Filters the keys of an object and masks string values of sensitive keys. Values are walked by the caller.
    fn filter_object(
        &self,
        map: Map<String, Value>,
        needles: &[&[u8]],
        report: &mut FilterReport,
    ) -> Map<String, Value> {
        let mut out = Map::new();
        for (key, mut value) in map {
            let filtered = self.filter_str(&key, needles, report);
            // Either form may be the sensitive one: the masked key of a second pass must give the same answer.
            let sensitive = self.patterns.keys().is_sensitive(&key)
                || filtered
                    .as_deref()
                    .is_some_and(|k| self.patterns.keys().is_sensitive(k));
            let key = filtered.unwrap_or(key);
            if sensitive && let Value::String(s) = &mut value {
                let already = self.filter_str(s, needles, report);
                let current = already.as_deref().unwrap_or(s);
                if !MARKERS.contains(&current) {
                    report.generic(HitKind::SensitiveField);
                    *s = FIELD_MARKER.to_owned();
                } else if let Some(masked) = already {
                    *s = masked;
                }
            }
            // Two keys that only differed by a secret now collide. Renaming one would add text that could itself
            // match a short secret: the values are grouped in an array instead.
            match out.get_mut(&key) {
                Some(existing) => {
                    let first = std::mem::take(existing);
                    *existing = Value::Array(vec![first, value]);
                }
                None => {
                    out.insert(key, value);
                }
            }
        }
        out
    }
}
