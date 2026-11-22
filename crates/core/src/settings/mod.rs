//! User settings: chosen profile, exposed environments, disabled MCPs, hardening tree and options.
//!
//! The settings hold no secret. Whatever goes wrong while reading them gives the strictest settings: the
//! `strictest` profile of the configuration and no exposed environment.

mod file;
mod store;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::config::{Config, EnvId, McpId, PolicyTree, Profile, ProfileId};
use crate::policy::PolicyEngine;

pub use store::{JsonFileStore, LoadedSettings, SettingsError, SettingsOrigin, SettingsStore};

/// Settings schema understood by this build. Any other version gives the strictest settings.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSettings {
    pub schema_version: u32,
    /// `None` until the user picks one (first launch, or after a reset).
    pub profile: Option<ProfileId>,
    pub exposed_environments: BTreeSet<EnvId>,
    /// MCPs disabled by the user, in every environment.
    pub disabled_mcp: BTreeSet<McpId>,
    /// Same shape as the configuration trees. Hardening only: `locked` is meaningless here and always `false`.
    pub hardening: PolicyTree,
    pub autostart: bool,
    pub beta_channel: bool,
    pub gateway_suspended: bool,
}

impl Default for UserSettings {
    /// The strictest settings: no profile chosen, nothing exposed, no option enabled.
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            profile: None,
            exposed_environments: BTreeSet::new(),
            disabled_mcp: BTreeSet::new(),
            hardening: PolicyTree::default(),
            autostart: false,
            beta_channel: false,
            gateway_suspended: false,
        }
    }
}

impl UserSettings {
    /// The chosen profile when it exists in `config`.
    pub fn chosen_profile<'c>(&self, config: &'c Config) -> Option<&'c Profile> {
        self.profile.as_ref().and_then(|id| config.profiles.get(id))
    }

    /// The profile in force: the chosen one, or the `strictest` profile while none is chosen or the chosen one
    /// no longer exists. `None` only for a configuration without a `strictest` profile, which validation rejects.
    pub fn active_profile<'c>(&self, config: &'c Config) -> Option<&'c Profile> {
        self.chosen_profile(config).or_else(|| config.strictest_profile())
    }

    /// Environments actually exposed: none while no valid profile is chosen, and only environments of `config`.
    pub fn exposed_environments(&self, config: &Config) -> BTreeSet<EnvId> {
        if self.chosen_profile(config).is_none() {
            return BTreeSet::new();
        }
        self.exposed_environments
            .iter()
            .filter(|e| config.environment(e).is_some())
            .cloned()
            .collect()
    }

    /// Picks a profile: its default environments become the exposed ones, the hardening rules are kept.
    pub fn choose_profile(&mut self, profile: &Profile) {
        self.profile = Some(profile.id.clone());
        self.exposed_environments = profile.default_environments.iter().cloned().collect();
    }

    /// Policy engine for the active profile and this hardening tree.
    pub fn engine<'a>(&'a self, config: &'a Config) -> Option<PolicyEngine<'a>> {
        Some(PolicyEngine::new(config, self.active_profile(config)?, &self.hardening))
    }
}
