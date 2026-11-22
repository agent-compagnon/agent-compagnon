//! Configuration: embedded TOML, typed model and strict validation.
//!
//! Loading happens in two phases. Structure (TOML syntax, types, required and unknown keys) is checked by serde and
//! stops at the first problem; every semantic rule is then checked and all violations are reported together.

mod error;
mod ids;
mod model;
mod policy;
mod raw;
mod validate;

#[cfg(test)]
mod tests;

use std::borrow::Cow;

pub use error::{ConfigError, ConfigErrors};
pub use ids::{EnvId, ID_MAX_LEN, IdError, McpAlias, McpId, ProfileId, SessionId, TOOL_NAME_MAX_LEN, ToolName};
pub use model::{
    AuthConfig, Config, Environment, FilterConfig, GatewayConfig, Mcp, McpKind, Origin, Profile, RateLimit, Session,
    SessionRef, SessionScope, ToolConfig, TraceConfig, UpdateConfig, UrlPattern, WebviewConfig, WebviewProfile,
};
pub use policy::{Category, Channel, Decision, EnvPolicy, McpPolicy, PolicyTree, Rule, RuleSet};
pub use validate::SCHEMA_VERSION;

/// The configuration compiled into the executable.
pub const EMBEDDED_TOML: &str = include_str!("../../../../config/default.toml");

/// Local modules compiled into the executable. Step 8 replaces this list with the module registry.
pub const BUILTIN_MODULES: &[&str] = &["jira-local"];

/// Where a configuration comes from. `Embedded` is the only implementation; a signed remote configuration
/// (backlog) would be another one.
pub trait ConfigSource {
    /// Name shown to the user in error reports, in French.
    fn name(&self) -> &str;

    fn read(&self) -> Result<Cow<'_, str>, ConfigError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Embedded;

impl ConfigSource for Embedded {
    fn name(&self) -> &str {
        "config embarquée"
    }

    fn read(&self) -> Result<Cow<'_, str>, ConfigError> {
        Ok(Cow::Borrowed(EMBEDDED_TOML))
    }
}

impl Config {
    /// Loads and validates the embedded configuration with the built-in modules.
    pub fn embedded() -> Result<Self, ConfigErrors> {
        Self::load(&Embedded, BUILTIN_MODULES)
    }

    pub fn load(source: &dyn ConfigSource, known_modules: &[&str]) -> Result<Self, ConfigErrors> {
        let wrap = |errors: Vec<ConfigError>| ConfigErrors {
            source_name: source.name().to_owned(),
            errors,
        };
        let text = source.read().map_err(|e| wrap(vec![e]))?;
        let raw = parse(&text).map_err(|e| wrap(vec![e]))?;
        validate::validate(raw, known_modules).map_err(wrap)
    }

    /// Parses and validates a TOML document.
    pub fn from_toml(text: &str, known_modules: &[&str]) -> Result<Self, ConfigErrors> {
        struct Text<'a>(&'a str);
        impl ConfigSource for Text<'_> {
            fn name(&self) -> &str {
                "config"
            }
            fn read(&self) -> Result<Cow<'_, str>, ConfigError> {
                Ok(Cow::Borrowed(self.0))
            }
        }
        Self::load(&Text(text), known_modules)
    }
}

fn line_of(text: &str, error: &toml::de::Error) -> Option<usize> {
    let start = error.span()?.start;
    Some(text.get(..start)?.matches('\n').count() + 1)
}

fn parse(text: &str) -> Result<raw::RawConfig, ConfigError> {
    let deserializer = toml::Deserializer::parse(text).map_err(|e| ConfigError {
        path: String::new(),
        message: match line_of(text, &e) {
            Some(line) => format!("syntaxe TOML invalide, ligne {line} : {}", e.message()),
            None => format!("syntaxe TOML invalide : {}", e.message()),
        },
    })?;
    serde_path_to_error::deserialize(deserializer).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.inner();
        let location = line_of(text, inner)
            .map(|l| format!(" (ligne {l})"))
            .unwrap_or_default();
        ConfigError {
            path: if path == "." { String::new() } else { path },
            message: format!("structure invalide{location} : {}", inner.message()),
        }
    })
}
