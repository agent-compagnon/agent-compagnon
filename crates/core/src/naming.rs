//! Exposed tool names: `{env}_{alias}_{tool}` (S12).

use sha2::{Digest, Sha256};

use crate::config::{EnvId, McpAlias, ToolName};

/// Smallest accepted `tool_name_max_len`: room for a meaningful prefix plus the `_` and 6-character hash suffix.
pub const MIN_EXPOSED_NAME_LEN: usize = 16;

/// Largest accepted `tool_name_max_len`: the MCP specification limit.
pub const MAX_EXPOSED_NAME_LEN: usize = 128;

const HASH_SUFFIX_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("longueur maximale {max_len} hors de l'intervalle {MIN_EXPOSED_NAME_LEN}–{MAX_EXPOSED_NAME_LEN}")]
pub struct NamingError {
    pub max_len: usize,
}

/// Computes the name exposed to LLM clients for an upstream tool.
///
/// The result is lowercase, only contains `[a-z0-9_-]` and is at most `max_len` characters long. A name that does
/// not fit is truncated and suffixed with `_` and the first 6 hex digits of the SHA-256 of the untransformed full
/// name, so that truncation stays deterministic and two long names sharing a prefix still differ. Collisions remain
/// possible and must be checked by the caller.
pub fn exposed_tool_name(
    env: &EnvId,
    alias: &McpAlias,
    tool: &ToolName,
    max_len: usize,
) -> Result<String, NamingError> {
    if !(MIN_EXPOSED_NAME_LEN..=MAX_EXPOSED_NAME_LEN).contains(&max_len) {
        return Err(NamingError { max_len });
    }
    let full = format!("{env}_{alias}_{tool}");
    let mut name: String = full
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.len() > max_len {
        let digest = Sha256::digest(full.as_bytes());
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        // Every character is ASCII at this point, so byte truncation is char-safe.
        name.truncate(max_len - HASH_SUFFIX_LEN - 1);
        name.push('_');
        name.push_str(&hash[..HASH_SUFFIX_LEN]);
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(env: &str, alias: &str, tool: &str, max: usize) -> String {
        exposed_tool_name(
            &EnvId::new(env).unwrap(),
            &McpAlias::new(alias).unwrap(),
            &ToolName::new(tool).unwrap(),
            max,
        )
        .unwrap()
    }

    #[test]
    fn prefix_and_sanitize() {
        assert_eq!(name("prod", "kibana", "search_logs", 64), "prod_kibana_search_logs");
        assert_eq!(name("prod", "kibana", "Search.Logs", 64), "prod_kibana_search_logs");
    }

    #[test]
    fn truncation_is_deterministic_and_bounded() {
        let long = "a".repeat(100);
        let first = name("prod", "kibana", &long, 64);
        assert_eq!(first.len(), 64);
        assert_eq!(first, name("prod", "kibana", &long, 64));
        assert_ne!(first, name("prod", "kibana", &format!("{long}b"), 64));
        assert!(first.starts_with("prod_kibana_aaa"));
    }

    #[test]
    fn rejects_unusable_limits() {
        let env = EnvId::new("prod").unwrap();
        let alias = McpAlias::new("kibana").unwrap();
        let tool = ToolName::new("x").unwrap();
        assert!(exposed_tool_name(&env, &alias, &tool, 15).is_err());
        assert!(exposed_tool_name(&env, &alias, &tool, 129).is_err());
    }
}
