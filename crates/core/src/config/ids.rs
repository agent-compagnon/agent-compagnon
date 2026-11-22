//! Validated identifiers used throughout the configuration.

use std::fmt;

/// Maximum length of a configuration identifier (`EnvId`, `SessionId`, `McpId`, `ProfileId`, MCP alias).
pub const ID_MAX_LEN: usize = 32;

/// Maximum length of an upstream tool name, as allowed by the MCP specification.
pub const TOOL_NAME_MAX_LEN: usize = 128;

/// Reason why a string is not a valid identifier. Rendered in French for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    Empty,
    TooLong { max: usize },
    InvalidChar(char),
    BadEdge,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("identifiant vide"),
            Self::TooLong { max } => write!(f, "identifiant trop long ({max} caractères au plus)"),
            Self::InvalidChar(c) => write!(f, "caractère interdit « {} »", c.escape_default()),
            Self::BadEdge => f.write_str("un identifiant ne commence ni ne finit par « - »"),
        }
    }
}

/// Checks the `[a-z0-9-]` identifier grammar shared by every configuration identifier.
pub fn check_id(value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }
    if value.len() > ID_MAX_LEN {
        return Err(IdError::TooLong { max: ID_MAX_LEN });
    }
    if let Some(c) = value
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return Err(IdError::InvalidChar(c));
    }
    if value.starts_with('-') || value.ends_with('-') {
        return Err(IdError::BadEdge);
    }
    Ok(())
}

fn check_tool_name(value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }
    if value.len() > TOOL_NAME_MAX_LEN {
        return Err(IdError::TooLong { max: TOOL_NAME_MAX_LEN });
    }
    if let Some(c) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(IdError::InvalidChar(c));
    }
    Ok(())
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $check:path) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                $check(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(
    /// Environment identifier (`dev`, `prod`…).
    EnvId,
    check_id
);
id_type!(
    /// Session identifier (`sso-corp`, `sso-prod`…).
    SessionId,
    check_id
);
id_type!(
    /// MCP identifier (`kibana-logs`…).
    McpId,
    check_id
);
id_type!(
    /// Profile identifier (`developpeur`…).
    ProfileId,
    check_id
);
id_type!(
    /// Short MCP name used in exposed tool names (`kibana`…). Same grammar as identifiers, so that `_` stays an
    /// unambiguous separator in `{env}_{alias}_{tool}`.
    McpAlias,
    check_id
);
id_type!(
    /// Tool name as declared by the upstream server: 1 to 128 characters in `[A-Za-z0-9._-]`.
    ToolName,
    check_tool_name
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert!(EnvId::new("prod").is_ok());
        assert!(EnvId::new("sso-prod-2").is_ok());
        assert_eq!(EnvId::new(""), Err(IdError::Empty));
        assert_eq!(EnvId::new("Prod"), Err(IdError::InvalidChar('P')));
        assert_eq!(EnvId::new("a_b"), Err(IdError::InvalidChar('_')));
        assert_eq!(EnvId::new("-a"), Err(IdError::BadEdge));
        assert_eq!(EnvId::new("a".repeat(33)), Err(IdError::TooLong { max: 32 }));
    }

    #[test]
    fn tool_names() {
        assert!(ToolName::new("Search.Logs_v2-beta").is_ok());
        assert_eq!(ToolName::new("a b"), Err(IdError::InvalidChar(' ')));
        assert!(ToolName::new("a".repeat(128)).is_ok());
        assert_eq!(ToolName::new("a".repeat(129)), Err(IdError::TooLong { max: 128 }));
    }
}
