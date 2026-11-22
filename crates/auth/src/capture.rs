//! Checks of what a login window hands over: URL patterns, captured headers and JWTs. Every value is a candidate
//! until it passes these checks; a refusal says why without ever quoting the value.
//!
//! The JWT signature is **not** verified: the proxy has neither the keys of the identity provider nor the role of
//! validating tokens, the servers do. These checks only keep out a value that is malformed, expired, not yet valid,
//! or meant for another audience or issuer. They do not prove that the token was issued by the provider: a page on
//! an allowed host could hand over a forged token of the right shape, which the servers would then refuse (401,
//! the session expires and a new login is asked for). The token never leaves towards anything but the session's
//! origins.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mcp_proxy_core::config::UrlPattern;
use mcp_proxy_core::session::EXPIRY_MARGIN;
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::ui::HeaderCapture;

/// Longest candidate accepted, in bytes (header value of a deposit, spec 012).
pub const MAX_CANDIDATE_LEN: usize = 16 * 1024;

/// Whether `url` matches `pattern`: equal to the prefix, or starting with it when the pattern ends with `*`. The
/// prefix is canonical and its host terminated (I1.15), so `https://a.example/*` never matches `a.example.evil`.
pub fn url_matches(pattern: &UrlPattern, url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if pattern.wildcard {
        url.as_str().starts_with(pattern.prefix.as_str())
    } else {
        url.as_str() == pattern.prefix.as_str()
    }
}

/// Why a candidate was not retained. Never carries the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CaptureRejection {
    #[error("URL hors du motif de capture")]
    UrlOutsidePattern,
    #[error("en-tête autre que celui attendu")]
    WrongHeader,
    #[error("schéma autre que celui attendu")]
    WrongScheme,
    #[error("valeur trop longue")]
    TooLong,
    #[error("jeton refusé : {0}")]
    Jwt(#[from] JwtRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum JwtRejection {
    #[error("structure invalide (trois segments base64url attendus)")]
    Structure,
    #[error("en-tête illisible")]
    Header,
    #[error("charge utile illisible")]
    Payload,
    #[error("expiration absente")]
    NoExpiry,
    #[error("jeton expiré ou sur le point de l'être")]
    Expired,
    #[error("jeton pas encore valide")]
    NotYetValid,
    #[error("audience inattendue")]
    Audience,
    #[error("émetteur inattendu")]
    Issuer,
}

/// What the configuration expects of a token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JwtExpectations {
    pub audience: Option<String>,
    pub issuer: Option<String>,
}

/// What a valid token tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JwtClaims {
    pub expires_at: SystemTime,
}

fn decode_json(segment: &str) -> Option<serde_json::Map<String, Value>> {
    let bytes = URL_SAFE_NO_PAD.decode(segment).ok()?;
    match serde_json::from_slice(&bytes).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn numeric_date(value: &Value) -> Option<SystemTime> {
    let secs = value.as_f64().filter(|s| s.is_finite() && *s >= 0.0)?;
    // Sub-second precision is irrelevant here; truncation errs on the side of expiring earlier.
    let whole = Duration::from_secs(secs.trunc().min(u64::MAX as f64) as u64);
    UNIX_EPOCH.checked_add(whole)
}

/// Checks the structure and claims of `token` at `now`. A token whose `exp` falls within the expiry margin is
/// refused: the session would be expired as soon as it is active.
pub fn check_jwt(token: &str, now: SystemTime, expect: &JwtExpectations) -> Result<JwtClaims, JwtRejection> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(JwtRejection::Structure);
    };
    let is_b64url = |s: &str| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if header.is_empty()
        || payload.is_empty()
        || signature.is_empty()
        || ![header, payload, signature].into_iter().all(is_b64url)
    {
        return Err(JwtRejection::Structure);
    }
    let header = decode_json(header).ok_or(JwtRejection::Header)?;
    if !header.get("alg").is_some_and(Value::is_string) {
        return Err(JwtRejection::Header);
    }
    let claims = decode_json(payload).ok_or(JwtRejection::Payload)?;
    let expires_at = claims
        .get("exp")
        .map(|v| numeric_date(v).ok_or(JwtRejection::Payload))
        .transpose()?
        .ok_or(JwtRejection::NoExpiry)?;
    if expires_at <= now.checked_add(EXPIRY_MARGIN).unwrap_or(now) {
        return Err(JwtRejection::Expired);
    }
    if let Some(not_before) = claims.get("nbf") {
        let not_before = numeric_date(not_before).ok_or(JwtRejection::Payload)?;
        if not_before > now {
            return Err(JwtRejection::NotYetValid);
        }
    }
    if let Some(audience) = &expect.audience {
        let matches = match claims.get("aud") {
            Some(Value::String(aud)) => aud == audience,
            Some(Value::Array(auds)) => auds.iter().any(|a| a.as_str() == Some(audience)),
            _ => false,
        };
        if !matches {
            return Err(JwtRejection::Audience);
        }
    }
    if let Some(issuer) = &expect.issuer
        && claims.get("iss").and_then(Value::as_str) != Some(issuer.as_str())
    {
        return Err(JwtRejection::Issuer);
    }
    Ok(JwtClaims { expires_at })
}

impl HeaderCapture {
    /// The token of a deposited header, if it is the expected header, towards a URL of the pattern, with the
    /// expected scheme: `{scheme} {token}`. The JWT itself is checked by `check_jwt`.
    pub fn candidate_token<'v>(&self, url: &Url, header: &str, value: &'v str) -> Result<&'v str, CaptureRejection> {
        if value.len() > MAX_CANDIDATE_LEN {
            return Err(CaptureRejection::TooLong);
        }
        if !url_matches(&self.url_pattern, url) {
            return Err(CaptureRejection::UrlOutsidePattern);
        }
        if !header.eq_ignore_ascii_case(&self.header) {
            return Err(CaptureRejection::WrongHeader);
        }
        let scheme_len = self.scheme.len();
        let (Some(scheme), Some(rest)) = (value.get(..scheme_len), value.get(scheme_len..)) else {
            return Err(CaptureRejection::WrongScheme);
        };
        let token = match rest.strip_prefix(' ') {
            Some(token) if scheme.eq_ignore_ascii_case(&self.scheme) && !token.is_empty() => token,
            _ => return Err(CaptureRejection::WrongScheme),
        };
        if token.contains(' ') {
            return Err(CaptureRejection::WrongScheme);
        }
        Ok(token)
    }
}
