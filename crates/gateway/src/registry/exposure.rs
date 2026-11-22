//! Which instances are exposed to clients.

use std::collections::BTreeSet;

use mcp_proxy_core::config::{Config, EnvId, McpId, ProfileId};
use mcp_proxy_core::settings::UserSettings;

use super::model::InstanceId;

/// A tool is exposed if and only if a profile is chosen, its environment is exposed and its MCP is not disabled.
/// Suspending the gateway does not change the exposure: calls are refused instead (spec 015).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exposure {
    profile: Option<ProfileId>,
    environments: BTreeSet<EnvId>,
    disabled_mcp: BTreeSet<McpId>,
}

impl Exposure {
    /// Nothing exposed: the state before the settings are read.
    pub fn none() -> Self {
        Self::default()
    }

    /// From the user settings: a profile that no longer exists counts as none, and only environments of `config`
    /// are kept.
    pub fn from_settings(settings: &UserSettings, config: &Config) -> Self {
        let profile = settings.chosen_profile(config).map(|p| p.id.clone());
        Self {
            environments: if profile.is_some() {
                settings.exposed_environments(config)
            } else {
                BTreeSet::new()
            },
            profile,
            disabled_mcp: settings.disabled_mcp.clone(),
        }
    }

    pub fn profile(&self) -> Option<&ProfileId> {
        self.profile.as_ref()
    }

    pub fn environments(&self) -> &BTreeSet<EnvId> {
        &self.environments
    }

    pub fn disabled_mcp(&self) -> &BTreeSet<McpId> {
        &self.disabled_mcp
    }

    pub fn exposes(&self, instance: &InstanceId) -> bool {
        self.profile.is_some()
            && self.environments.contains(&instance.env)
            && !self.disabled_mcp.contains(&instance.mcp)
    }
}
