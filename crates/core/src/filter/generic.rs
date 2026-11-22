//! Generic patterns: what looks like a secret even when it is not a known one.

use regex::{Captures, Regex};

use super::keys::{KeyClass, SensitiveKeys};
use super::{FIELD_MARKER, FilterError, FilterReport, HEADER_MARKER, HitKind, JWT_MARKER, MARKERS, SECRET_MARKER};

/// HTTP authentication schemes kept in front of a masked `Authorization` value.
const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "negotiate", "digest", "ntlm"];

type Step<'a> = &'a dyn Fn(&str, &mut FilterReport) -> Option<String>;

pub(crate) struct GenericPatterns {
    private_key: Regex,
    url_credentials: Regex,
    jwt: Regex,
    quoted_fields: [Regex; 3],
    key_value: Regex,
    keys: SensitiveKeys,
}

fn compile(pattern: &str) -> Result<Regex, FilterError> {
    Regex::new(pattern).map_err(|_| FilterError::Pattern)
}

fn is_marker(value: &str) -> bool {
    MARKERS.contains(&value)
}

/// Rebuilds `text` with the replacements `(range, replacement)`, which must be sorted and disjoint.
fn splice(text: &str, replacements: &[(std::ops::Range<usize>, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (range, replacement) in replacements {
        out.push_str(text.get(pos..range.start).unwrap_or_default());
        out.push_str(replacement);
        pos = range.end;
    }
    out.push_str(text.get(pos..).unwrap_or_default());
    out
}

impl GenericPatterns {
    pub(crate) fn new(keys: SensitiveKeys) -> Result<Self, FilterError> {
        Ok(Self {
            private_key: compile(
                r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----|\z)",
            )?,
            // The password runs up to the last `@` of the authority, so that it may itself contain `@`.
            url_credentials: compile(r"[A-Za-z][A-Za-z0-9+.\-]*://[^\s/?#@:]*:(?P<pw>[^\s/?#]+)@")?,
            // Compact JWS (3 segments) or JWE (5 segments).
            jwt: compile(
                r"eyJ[A-Za-z0-9_\-]{4,}\.[A-Za-z0-9_\-]*\.[A-Za-z0-9_\-]*(?:\.[A-Za-z0-9_\-]*\.[A-Za-z0-9_\-]*)?",
            )?,
            quoted_fields: [
                compile(r#""(?P<key>[^"\\\r\n]{1,128})"[ \t]*:[ \t]*"(?P<val>(?:[^"\\]|\\.)*)""#)?,
                compile(r#"'(?P<key>[^'\\\r\n]{1,128})'[ \t]*:[ \t]*'(?P<val>(?:[^'\\]|\\.)*)'"#)?,
                // JSON serialized inside a JSON string: `{\"password\":\"…\"}`.
                compile(r#"\\"(?P<key>[^"\\\r\n]{1,128})\\"[ \t]*:[ \t]*\\"(?P<val>(?:[^"\\]|\\[^"])*)\\""#)?,
            ],
            key_value: compile(r"(?P<key>[A-Za-z0-9_][A-Za-z0-9_.\-]{0,127})[ \t]*(?P<sep>[:=])")?,
            keys,
        })
    }

    pub(crate) fn keys(&self) -> &SensitiveKeys {
        &self.keys
    }

    /// Applies every generic pattern once. `None` when nothing changed.
    pub(crate) fn apply(&self, text: &str, report: &mut FilterReport) -> Option<String> {
        let mut current: Option<String> = None;
        let steps: [Step<'_>; 5] = [
            &|t, r| self.private_keys(t, r),
            &|t, r| self.url_credentials(t, r),
            &|t, r| self.jwts(t, r),
            &|t, r| self.quoted_fields(t, r),
            &|t, r| self.key_values(t, r),
        ];
        for step in steps {
            let input = current.as_deref().unwrap_or(text);
            if let Some(next) = step(input, report) {
                current = Some(next);
            }
        }
        current
    }

    fn private_keys(&self, text: &str, report: &mut FilterReport) -> Option<String> {
        let replacements: Vec<_> = self
            .private_key
            .find_iter(text)
            .map(|m| (m.range(), SECRET_MARKER))
            .collect();
        report.generic_n(HitKind::PrivateKey, replacements.len());
        (!replacements.is_empty()).then(|| splice(text, &replacements))
    }

    fn url_credentials(&self, text: &str, report: &mut FilterReport) -> Option<String> {
        let replacements: Vec<_> = self
            .url_credentials
            .captures_iter(text)
            .filter_map(|c| c.name("pw"))
            .map(|pw| (pw.range(), SECRET_MARKER))
            .collect();
        report.generic_n(HitKind::UrlCredentials, replacements.len());
        (!replacements.is_empty()).then(|| splice(text, &replacements))
    }

    fn jwts(&self, text: &str, report: &mut FilterReport) -> Option<String> {
        let replacements: Vec<_> = self.jwt.find_iter(text).map(|m| (m.range(), JWT_MARKER)).collect();
        report.generic_n(HitKind::Jwt, replacements.len());
        (!replacements.is_empty()).then(|| splice(text, &replacements))
    }

    /// `"key": "value"` (and the single-quoted and escaped forms): a string value under a sensitive key.
    fn quoted_fields(&self, text: &str, report: &mut FilterReport) -> Option<String> {
        let mut current: Option<String> = None;
        for pattern in &self.quoted_fields {
            let input = current.as_deref().unwrap_or(text);
            let replacements: Vec<_> = pattern
                .captures_iter(input)
                .filter_map(|c: Captures<'_>| {
                    let (key, val) = (c.name("key")?, c.name("val")?);
                    let masked = self.keys.is_sensitive(key.as_str()) && !is_marker(val.as_str());
                    masked.then(|| (val.range(), FIELD_MARKER))
                })
                .collect();
            report.generic_n(HitKind::SensitiveField, replacements.len());
            if !replacements.is_empty() {
                current = Some(splice(input, &replacements));
            }
        }
        current
    }

    /// `key=value` and `key: value` for sensitive keys, including `Authorization:` and `Cookie:` / `Set-Cookie:`
    /// header lines.
    fn key_values(&self, text: &str, report: &mut FilterReport) -> Option<String> {
        let mut replacements: Vec<(std::ops::Range<usize>, &str)> = Vec::new();
        let mut from = 0;
        while let Some(c) = self.key_value.captures_at(text, from) {
            let (Some(key), Some(sep)) = (c.name("key"), c.name("sep")) else {
                break;
            };
            from = sep.end();
            let Some(class) = self.keys.classify(key.as_str()) else {
                continue;
            };
            let mut start = skip_blanks(text, sep.end());
            if class == KeyClass::Authorization {
                start = skip_scheme(text, start);
            }
            let Some(value) = value_range(text, start, sep.as_str() == "=") else {
                continue;
            };
            from = value.end.max(from);
            let current = text.get(value.clone()).unwrap_or_default();
            if current.is_empty() || is_marker(current) {
                continue;
            }
            let (marker, kind) = match class {
                KeyClass::Authorization => (HEADER_MARKER, HitKind::AuthorizationHeader),
                KeyClass::Cookie if sep.as_str() == ":" => (HEADER_MARKER, HitKind::CookieHeader),
                _ => (FIELD_MARKER, HitKind::SensitiveField),
            };
            report.generic(kind);
            replacements.push((value, marker));
        }
        (!replacements.is_empty()).then(|| splice(text, &replacements))
    }
}

fn skip_blanks(text: &str, mut pos: usize) -> usize {
    while text.as_bytes().get(pos).is_some_and(|b| *b == b' ' || *b == b'\t') {
        pos += 1;
    }
    pos
}

/// Skips an authentication scheme followed by blanks, if any.
fn skip_scheme(text: &str, pos: usize) -> usize {
    let rest = text.get(pos..).unwrap_or_default();
    for scheme in AUTH_SCHEMES {
        let Some(word) = rest.get(..scheme.len()) else {
            continue;
        };
        let after = pos + scheme.len();
        let blank = text.as_bytes().get(after).is_some_and(|b| *b == b' ' || *b == b'\t');
        if word.eq_ignore_ascii_case(scheme) && blank {
            return skip_blanks(text, after);
        }
    }
    pos
}

/// The value starting at `start`: the inside of a quoted value, else up to a delimiter for `key=value` (URL query,
/// cookie list), else up to the end of the line. A leading marker is part of the value, so that `[CHAMP MASQUÉ]`
/// is recognised as an already masked value.
fn value_range(text: &str, start: usize, equals: bool) -> Option<std::ops::Range<usize>> {
    let rest = text.get(start..)?;
    let line_end = rest.find(['\r', '\n']).unwrap_or(rest.len());
    let line = rest.get(..line_end)?;
    if let Some(quote) = line.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let body = line.get(1..)?;
        let mut escaped = false;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                c if c == quote => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        return Some(start + 1..start + 1 + end);
    }
    if equals {
        let marker = MARKERS.iter().find(|m| line.starts_with(**m)).map_or(0, |m| m.len());
        let tail = line.get(marker..)?;
        let end = tail
            .find(|c: char| c.is_whitespace() || "&;,\"'<>".contains(c))
            .unwrap_or(tail.len());
        return Some(start..start + marker + end);
    }
    Some(start..start + line.trim_end().len())
}
