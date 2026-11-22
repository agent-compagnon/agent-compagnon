//! The local modules compiled into the executable: the only place where they are declared.

use std::sync::Arc;

use mcp_proxy_core::config::{Config, ConfigErrors};
use mcp_proxy_core::module::{LocalModule, ModuleRegistry};
use mcp_proxy_module_jira_local::JiraLocal;

/// Every module of the executable. Configuration validation accepts these ids, and only these, for `module`.
pub fn builtin() -> ModuleRegistry {
    let modules: [Arc<dyn LocalModule>; 1] = [Arc::new(JiraLocal)];
    // Ids are distinct by construction of this list; an empty registry would refuse every local MCP, failing closed.
    ModuleRegistry::new(modules).unwrap_or_default()
}

/// The embedded configuration, validated against the built-in modules.
pub fn embedded_config() -> Result<Config, ConfigErrors> {
    Config::embedded(&builtin().ids())
}

#[cfg(test)]
mod tests {
    use mcp_proxy_core::config::EMBEDDED_TOML;
    use mcp_proxy_core::module::DuplicateModule;

    use super::*;

    // Exigence : EX-025-09
    #[test]
    fn the_builtin_modules_are_the_only_ones_accepted_by_the_configuration() {
        assert_eq!(builtin().ids(), ["jira-local"]);
        assert!(builtin().get("jira-local").is_some());
        assert!(embedded_config().is_ok());

        let unknown = EMBEDDED_TOML.replace("module = \"jira-local\"", "module = \"jira-cloud\"");
        assert_ne!(unknown, EMBEDDED_TOML);
        let errors = Config::from_toml(&unknown, &builtin().ids()).unwrap_err();
        assert!(errors.errors.iter().any(|e| e.path.ends_with(".module")), "{errors}");
        // The core alone knows no module.
        assert!(Config::embedded(&[]).is_err());

        let twice: [Arc<dyn LocalModule>; 2] = [Arc::new(JiraLocal), Arc::new(JiraLocal)];
        assert_eq!(ModuleRegistry::new(twice).unwrap_err(), DuplicateModule("jira-local"));
    }
}
