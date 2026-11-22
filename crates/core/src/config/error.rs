use std::fmt;

/// One configuration problem, located by its TOML path (`mcp[2].instances.dev.url`). Messages are in French: they
/// are shown to the user as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{} : {}", self.path, self.message)
        }
    }
}

/// Every problem found while loading a configuration. Never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrors {
    /// Human-readable name of the source (`config embarquée`).
    pub source_name: String,
    pub errors: Vec<ConfigError>,
}

impl ConfigErrors {
    pub fn iter(&self) -> impl Iterator<Item = &ConfigError> {
        self.errors.iter()
    }
}

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.errors.len();
        let plural = if count > 1 { "s" } else { "" };
        write!(f, "{} invalide ({count} erreur{plural}) :", self.source_name)?;
        for error in &self.errors {
            write!(f, "\n- {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {}
