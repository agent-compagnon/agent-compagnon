use std::fmt;

use secrecy::{ExposeSecret, SecretString};

use super::error::SecretError;

/// Largest accepted secret value, in bytes.
pub const MAX_SECRET_LEN: usize = 64 * 1024;

/// What a session authenticates with. Built from `secrecy` types, consumed by `SecretStore::put`.
///
/// Cookie names and the Basic login are not secrets; cookie values, tokens and passwords are.
pub enum Credential {
    Bearer { token: SecretString },
    Cookies { pairs: Vec<(String, SecretString)> },
    Basic { login: String, password: SecretString },
}

/// A credential readable for the duration of a `SecretStore::with_exposed` closure.
pub enum ExposedCredential<'a> {
    Bearer { token: &'a str },
    Cookies { pairs: &'a [ExposedCookie<'a>] },
    Basic { login: &'a str, password: &'a str },
}

pub struct ExposedCookie<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

/// Bytes accepted in an HTTP header value: visible ASCII, space, tab and obs-text.
fn is_header_value_byte(b: u8) -> bool {
    b == b'\t' || (b >= 0x20 && b != 0x7f)
}

/// RFC 9110 `tchar`.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn check_secret(value: &str, what: &'static str) -> Result<(), SecretError> {
    if value.is_empty() {
        return Err(SecretError::InvalidCredential(what));
    }
    if value.len() > MAX_SECRET_LEN {
        return Err(SecretError::InvalidCredential("valeur secrète trop longue"));
    }
    if !value.bytes().all(is_header_value_byte) {
        return Err(SecretError::InvalidCredential(
            "caractère de contrôle dans une valeur secrète",
        ));
    }
    Ok(())
}

impl Credential {
    /// Values must fit in the header they are injected into, so that injection can never fail or split a header.
    pub(crate) fn validate(&self) -> Result<(), SecretError> {
        match self {
            Self::Bearer { token } => check_secret(token.expose_secret(), "jeton vide"),
            Self::Cookies { pairs } => {
                if pairs.is_empty() {
                    return Err(SecretError::InvalidCredential("aucun cookie"));
                }
                for (i, (name, value)) in pairs.iter().enumerate() {
                    if name.is_empty() || !name.bytes().all(is_token_byte) {
                        return Err(SecretError::InvalidCredential("nom de cookie invalide"));
                    }
                    if pairs.iter().take(i).any(|(other, _)| other == name) {
                        return Err(SecretError::InvalidCredential("cookie en double"));
                    }
                    let value = value.expose_secret();
                    check_secret(value, "valeur de cookie vide")?;
                    if value.contains(';') {
                        return Err(SecretError::InvalidCredential("« ; » dans une valeur de cookie"));
                    }
                }
                Ok(())
            }
            Self::Basic { login, password } => {
                if login.is_empty() || login.len() > MAX_SECRET_LEN || !login.bytes().all(is_header_value_byte) {
                    return Err(SecretError::InvalidCredential("identifiant Basic invalide"));
                }
                if login.contains(':') {
                    return Err(SecretError::InvalidCredential("« : » dans l'identifiant Basic"));
                }
                check_secret(password.expose_secret(), "mot de passe vide")
            }
        }
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer { .. } => f.write_str("Credential::Bearer(***)"),
            Self::Cookies { pairs } => {
                f.write_str("Credential::Cookies([")?;
                for (i, (name, _)) in pairs.iter().enumerate() {
                    let sep = if i == 0 { "" } else { ", " };
                    write!(f, "{sep}{name}=***")?;
                }
                f.write_str("])")
            }
            Self::Basic { login, .. } => write!(f, "Credential::Basic {{ login: {login:?}, password: *** }}"),
        }
    }
}

impl fmt::Debug for ExposedCredential<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer { .. } => f.write_str("ExposedCredential::Bearer(***)"),
            Self::Cookies { pairs } => {
                f.write_str("ExposedCredential::Cookies(")?;
                f.debug_list().entries(pairs.iter()).finish()?;
                f.write_str(")")
            }
            Self::Basic { login, .. } => {
                write!(f, "ExposedCredential::Basic {{ login: {login:?}, password: *** }}")
            }
        }
    }
}

impl fmt::Debug for ExposedCookie<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}=***", self.name)
    }
}
