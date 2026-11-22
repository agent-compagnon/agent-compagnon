//! Masking variants of a credential: every byte string under which a secret could leave the process.
//!
//! Every intermediate buffer is `Zeroizing` and allocated at its final capacity, so that no reallocation leaves an
//! unwiped copy behind: the variants are as sensitive as the secret itself.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use secrecy::ExposeSecret;
use zeroize::Zeroizing;

use super::credential::Credential;

/// A derived variant shorter than this is ignored: it would mask arbitrary text. Raw values are always kept.
pub(crate) const MIN_DERIVED_LEN: usize = 8;

type Bytes = Zeroizing<Vec<u8>>;

/// Deduplicated variant list. Raw values are kept whatever their length, derived ones from `MIN_DERIVED_LEN` on.
#[derive(Default)]
struct Variants(Vec<Bytes>);

impl Variants {
    fn push(&mut self, value: &[u8]) {
        if !value.is_empty() && !self.0.iter().any(|v| v.as_slice() == value) {
            self.0.push(Zeroizing::new(value.to_vec()));
        }
    }

    fn raw(&mut self, value: &[u8]) {
        self.push(value);
    }

    fn derived(&mut self, value: &[u8]) {
        if value.len() >= MIN_DERIVED_LEN {
            self.push(value);
        }
    }

    /// The raw value and every encoded form of it.
    fn value(&mut self, value: &[u8]) {
        self.raw(value);
        self.encodings(value);
        if let Some(signature) = jwt_signature(value) {
            self.derived(signature);
            if let Some(decoded) = decode_base64url(signature) {
                self.derived(&encode(&STANDARD, &decoded));
                self.derived(&encode(&STANDARD_NO_PAD, &decoded));
                self.derived(&hex(&decoded, false));
                self.derived(&hex(&decoded, true));
            }
        }
    }

    fn encodings(&mut self, value: &[u8]) {
        for (padded, unpadded) in [(&STANDARD, &STANDARD_NO_PAD), (&URL_SAFE, &URL_SAFE_NO_PAD)] {
            self.derived(&encode(padded, value));
            self.derived(&encode(unpadded, value));
            for offset in 0..3 {
                self.derived(&aligned_base64(unpadded, value, offset));
            }
        }
        self.derived(&hex(value, false));
        self.derived(&hex(value, true));
        for upper in [true, false] {
            self.derived(&percent_encode(value, upper, false));
            self.derived(&percent_encode(value, upper, true));
        }
        if let Ok(text) = std::str::from_utf8(value) {
            for form in [JsonEscape::Minimal, JsonEscape::Solidus, JsonEscape::Ascii] {
                self.derived(json_escape(text, form).as_bytes());
            }
        }
    }
}

/// Every variant the output filter must find for this credential, deduplicated.
///
/// - Raw value of each secret (token, Basic password, each cookie value), whatever its length.
/// - Base64 and base64url, padded and unpadded, plus the three alignment fragments of each (see `aligned_base64`).
/// - Hexadecimal in both cases; percent-encoding and `x-www-form-urlencoded` with both hex cases; JSON-escaped forms.
/// - A JWT-shaped value also yields its signature segment, and the signature bytes in base64 and hexadecimal.
/// - Basic: `login:password` and its base64 form. Cookies: the percent-decoded value too, treated as a raw value.
pub(crate) fn derive_variants(credential: &Credential) -> Vec<Zeroizing<Vec<u8>>> {
    let mut variants = Variants::default();
    match credential {
        Credential::Bearer { token } => variants.value(token.expose_secret().as_bytes()),
        Credential::Cookies { pairs } => {
            for (_, value) in pairs {
                let value = value.expose_secret().as_bytes();
                variants.value(value);
                if let Some(decoded) = percent_decode(value) {
                    variants.value(&decoded);
                }
            }
        }
        Credential::Basic { login, password } => {
            let password = password.expose_secret().as_bytes();
            variants.value(password);
            let mut user_pass = Zeroizing::new(Vec::with_capacity(login.len() + 1 + password.len()));
            user_pass.extend_from_slice(login.as_bytes());
            user_pass.push(b':');
            user_pass.extend_from_slice(password);
            variants.derived(&user_pass);
            variants.derived(&encode(&STANDARD, &user_pass));
            variants.derived(&encode(&STANDARD_NO_PAD, &user_pass));
        }
    }
    variants.0
}

fn encode(engine: &impl base64::Engine, value: &[u8]) -> Bytes {
    Zeroizing::new(engine.encode(value).into_bytes())
}

/// The characters of `base64(k arbitrary bytes ‖ value)` that only depend on `value`, for `k = offset`.
///
/// Character `i` covers bits `6i..6i+6`; it is fully determined by `value` when `8k ≤ 6i` and `6i + 6 ≤ 8(k + n)`,
/// hence the range `ceil(8k/6) .. floor(8(k+n)/6)`. Padding is never part of the fragment.
pub(crate) fn aligned_base64(engine: &impl base64::Engine, value: &[u8], offset: usize) -> Bytes {
    let mut buf = Zeroizing::new(Vec::with_capacity(offset + value.len()));
    buf.resize(offset, 0);
    buf.extend_from_slice(value);
    let encoded = encode(engine, &buf);
    let start = (8 * offset).div_ceil(6);
    let end = (8 * (offset + value.len())) / 6;
    Zeroizing::new(encoded.get(start..end).unwrap_or_default().to_vec())
}

pub(crate) fn hex(value: &[u8], upper: bool) -> Bytes {
    let digits: &[u8; 16] = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = Zeroizing::new(Vec::with_capacity(value.len() * 2));
    for b in value {
        out.push(digits[usize::from(b >> 4)]);
        out.push(digits[usize::from(b & 0x0f)]);
    }
    out
}

/// RFC 3986 percent-encoding (unreserved characters kept), or `application/x-www-form-urlencoded` (WHATWG: `*-._`
/// and alphanumerics kept, space as `+`).
pub(crate) fn percent_encode(value: &[u8], upper: bool, form: bool) -> Bytes {
    let digits: &[u8; 16] = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = Zeroizing::new(Vec::with_capacity(value.len() * 3));
    for &b in value {
        let kept = b.is_ascii_alphanumeric()
            || if form {
                b"*-._".contains(&b)
            } else {
                b"-._~".contains(&b)
            };
        if kept {
            out.push(b);
        } else if form && b == b' ' {
            out.push(b'+');
        } else {
            out.extend_from_slice(&[b'%', digits[usize::from(b >> 4)], digits[usize::from(b & 0x0f)]]);
        }
    }
    out
}

/// Decodes `%XX` sequences; `None` when the value contains none, or a malformed one.
fn percent_decode(value: &[u8]) -> Option<Bytes> {
    if !value.contains(&b'%') {
        return None;
    }
    let digit = |c: u8| char::from(c).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Zeroizing::new(Vec::with_capacity(value.len()));
    let mut rest = value;
    while let Some((&b, tail)) = rest.split_first() {
        if b == b'%' {
            let (hi, lo) = (digit(*tail.first()?)?, digit(*tail.get(1)?)?);
            out.push(hi << 4 | lo);
            rest = tail.get(2..)?;
        } else {
            out.push(b);
            rest = tail;
        }
    }
    Some(out)
}

#[derive(Clone, Copy)]
pub(crate) enum JsonEscape {
    /// What `serde_json` writes: `"`, `\` and control characters.
    Minimal,
    /// Minimal, plus `/` written `\/` (PHP, some Java encoders).
    Solidus,
    /// Minimal, plus every non-ASCII character as `\uxxxx` (Python's default `ensure_ascii`).
    Ascii,
}

pub(crate) fn json_escape(text: &str, form: JsonEscape) -> Zeroizing<String> {
    // Worst case is 6 bytes per input byte (`\u00xx` for a control character): growing would leave an unwiped copy.
    let mut out = Zeroizing::new(String::with_capacity(text.len() * 6));
    // Written digit by digit: a temporary `format!` buffer would hold part of the secret without being wiped.
    let unit = |out: &mut String, u: u32| {
        out.push_str("\\u");
        for shift in [12, 8, 4, 0] {
            out.extend(char::from_digit((u >> shift) & 0xf, 16));
        }
    };
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '/' if matches!(form, JsonEscape::Solidus) => out.push_str("\\/"),
            c if u32::from(c) < 0x20 => unit(&mut out, u32::from(c)),
            c if !c.is_ascii() && matches!(form, JsonEscape::Ascii) => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    unit(&mut out, u32::from(*u));
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn is_base64url(segment: &[u8]) -> bool {
    segment
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
}

/// Signature segment of a compact JWS (`header.payload.signature`, header starting with `eyJ`).
fn jwt_signature(value: &[u8]) -> Option<&[u8]> {
    let mut segments = value.split(|b| *b == b'.');
    let (header, payload, signature) = (segments.next()?, segments.next()?, segments.next()?);
    let shaped = segments.next().is_none()
        && header.starts_with(b"eyJ")
        && [header, payload, signature].iter().all(|s| is_base64url(s))
        && !signature.is_empty();
    shaped.then_some(signature)
}

fn decode_base64url(segment: &[u8]) -> Option<Bytes> {
    URL_SAFE_NO_PAD.decode(segment).ok().map(Zeroizing::new)
}
