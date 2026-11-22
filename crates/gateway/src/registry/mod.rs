//! Tool registry: which tools exist, under which name they are exposed, and which ones are exposed, before any
//! connection (spec 005).
//!
//! Each instance takes its tools from the best available source: the live list of this run, otherwise the local
//! cache, otherwise the embedded snapshot. Every definition goes through the output filter before it enters the
//! registry. Whether a tool is known, and its category, only ever come from the configuration (S11); the hints sent
//! to clients are recomputed from that category (S10).

mod cache;
mod catalog;
mod exposure;
mod fingerprint;
mod model;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use mcp_proxy_core::config::{Config, IdError, ToolName};
use mcp_proxy_core::filter::{FilterError, OutputFilter};
use mcp_proxy_core::naming::{NamingError, exposed_tool_name};
use mcp_proxy_core::policy::{CategorySource, tool_category};
use mcp_proxy_core::settings::UserSettings;
use serde_json::Value;
use tokio::sync::broadcast;

pub use cache::{
    CACHE_SCHEMA_VERSION, CacheError, CacheSeal, CachedCatalog, CatalogCache, FileCatalogCache, NoCatalogCache, NoSeal,
    SealedCatalog,
};
pub use catalog::{CatalogError, EmbeddedCatalog};
pub use exposure::Exposure;
pub(crate) use fingerprint::canonical_json;
pub use model::{CatalogSource, Fingerprint, InstanceId, ToolDefinition, ToolDiff, ToolEntry, ToolHints};

/// Capacity of the event channel. A receiver that lags behind gets `RecvError::Lagged` and should re-read the
/// registry.
const EVENT_CAPACITY: usize = 64;

/// Changes the interface and the MCP server react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryEvent {
    /// The set of exposed instances changed (profile, environments, disabled MCPs).
    ExposureChanged,
    /// The tools of an instance changed. Not sent when a new list has the same fingerprints.
    ToolsChanged { instance: InstanceId, diff: ToolDiff },
}

/// Why a list of tools was refused. Nothing in the registry changes. French, quotes nothing the filter did not
/// see.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("instance {0} inconnue de la config")]
    UnknownInstance(InstanceId),
    #[error("{instance} : outil n° {index} : nom invalide ({reason})")]
    InvalidToolName {
        instance: InstanceId,
        index: usize,
        reason: IdError,
    },
    #[error("{instance} : outil n° {index} : le nom contient une donnée masquée par le filtre de sortie")]
    MaskedName { instance: InstanceId, index: usize },
    #[error("{instance} : outil n° {index} : définition illisible après filtrage")]
    InvalidDefinition { instance: InstanceId, index: usize },
    #[error("{instance} : outil « {tool} » : un schéma d'entrée ou de sortie n'est pas un objet JSON")]
    InvalidSchema { instance: InstanceId, tool: ToolName },
    #[error("{instance} : outil « {tool} » en double")]
    DuplicateTool { instance: InstanceId, tool: ToolName },
    #[error("nom exposé « {exposed_name} » en collision : {first} et {second}")]
    Collision {
        exposed_name: String,
        first: String,
        second: String,
    },
    #[error("nom exposé : {0}")]
    Naming(#[from] NamingError),
    #[error("{0}")]
    Filter(#[source] FilterError),
}

/// What happened with the cache of an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOperation {
    /// The cache could not be read or its content was refused: the embedded snapshot was used.
    Load,
    /// The live list could not be written; the registry itself was updated.
    Store,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheWarning {
    pub instance: InstanceId,
    pub operation: CacheOperation,
    /// French, quotes no content of the cache.
    pub reason: String,
}

#[derive(Debug, Clone)]
struct InstanceTools {
    /// `None` while no source had anything for this instance.
    source: Option<CatalogSource>,
    tools: BTreeMap<ToolName, ToolEntry>,
    fingerprint: Fingerprint,
}

impl InstanceTools {
    fn new(source: Option<CatalogSource>, tools: BTreeMap<ToolName, ToolEntry>) -> Self {
        let fingerprint = fingerprint::instance_fingerprint(tools.values().map(|t| &t.fingerprint));
        Self {
            source,
            tools,
            fingerprint,
        }
    }
}

pub struct Registry {
    config: Arc<Config>,
    filter: Arc<OutputFilter>,
    cache: Box<dyn CatalogCache>,
    instances: BTreeMap<InstanceId, InstanceTools>,
    /// Inverse table: exposed name → tool, for every instance whether exposed or not.
    by_exposed: BTreeMap<String, (InstanceId, ToolName)>,
    exposure: Exposure,
    events: broadcast::Sender<RegistryEvent>,
    cache_warnings: Vec<CacheWarning>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("instances", &self.instances.len())
            .field("tools", &self.by_exposed.len())
            .field("exposure", &self.exposure)
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// One entry per instance of the configuration, filled from its cache, otherwise its embedded snapshot.
    /// Nothing is exposed until [`Registry::set_exposure`] or [`Registry::apply_settings`].
    ///
    /// An unreadable or refused cache is ignored and reported in [`Registry::cache_warnings`]. An embedded snapshot
    /// that cannot be used, or a filter failure, is an error: the registry is not built.
    pub fn new(
        config: Arc<Config>,
        embedded: &EmbeddedCatalog,
        cache: Box<dyn CatalogCache>,
        filter: Arc<OutputFilter>,
    ) -> Result<Self, RegistryError> {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let mut registry = Self {
            config: Arc::clone(&config),
            filter,
            cache,
            instances: BTreeMap::new(),
            by_exposed: BTreeMap::new(),
            exposure: Exposure::none(),
            events,
            cache_warnings: Vec::new(),
        };
        for mcp in &config.mcp {
            for env in mcp.instances.keys() {
                let instance = InstanceId::new(env.clone(), mcp.id.clone());
                let tools = registry.initial_tools(&instance, embedded)?;
                registry.install(instance, tools);
            }
        }
        Ok(registry)
    }

    fn initial_tools(
        &mut self,
        instance: &InstanceId,
        embedded: &EmbeddedCatalog,
    ) -> Result<InstanceTools, RegistryError> {
        let warn = |reason: String| CacheWarning {
            instance: instance.clone(),
            operation: CacheOperation::Load,
            reason,
        };
        match self.cache.load(instance) {
            Ok(Some(cached)) => match self.build_checked(instance, cached.tools, CatalogSource::Cache) {
                Ok(tools) => return Ok(InstanceTools::new(Some(CatalogSource::Cache), tools)),
                Err(e @ RegistryError::Filter(_)) => return Err(e),
                Err(e) => self.cache_warnings.push(warn(e.to_string())),
            },
            Ok(None) => {}
            Err(e) => self.cache_warnings.push(warn(e.to_string())),
        }
        match embedded.for_instance(instance) {
            Some(snapshot) => {
                let tools = self.build_checked(instance, snapshot.to_vec(), CatalogSource::Embedded)?;
                Ok(InstanceTools::new(Some(CatalogSource::Embedded), tools))
            }
            None => Ok(InstanceTools::new(None, BTreeMap::new())),
        }
    }

    /// Replaces the tools of `instance` with the list its server just gave, and updates the cache.
    ///
    /// The whole list is refused, and nothing changes, if a name is invalid or masked by the filter, a tool is
    /// listed twice, a schema is not an object, two exposed names collide, or the filter fails. A cache that cannot
    /// be written does not refuse the list: it is reported in [`Registry::cache_warnings`].
    pub fn apply_live_list(
        &mut self,
        instance: &InstanceId,
        tools: Vec<ToolDefinition>,
    ) -> Result<ToolDiff, RegistryError> {
        let Some(previous) = self.instances.get(instance) else {
            return Err(RegistryError::UnknownInstance(instance.clone()));
        };
        let tools = self.build_checked(instance, tools, CatalogSource::Live)?;
        let next = InstanceTools::new(Some(CatalogSource::Live), tools);
        let diff = diff(&previous.tools, &next.tools);
        // The cache already holds this list when the previous one came from it or from a live list with the same
        // fingerprint.
        let cache_is_current = matches!(previous.source, Some(CatalogSource::Cache | CatalogSource::Live))
            && previous.fingerprint == next.fingerprint;
        let cached = (!cache_is_current).then(|| CachedCatalog {
            tools: next.tools.values().map(ToolEntry::server_definition).collect(),
        });
        self.install(instance.clone(), next);

        if let Some(cached) = cached
            && let Err(e) = self.cache.store(instance, &cached)
        {
            self.cache_warnings.push(CacheWarning {
                instance: instance.clone(),
                operation: CacheOperation::Store,
                reason: e.to_string(),
            });
        }
        if !diff.is_empty() {
            // No receiver is not an error: nobody is listening yet.
            let _ = self.events.send(RegistryEvent::ToolsChanged {
                instance: instance.clone(),
                diff: diff.clone(),
            });
        }
        Ok(diff)
    }

    /// Changes what is exposed. Sends `ExposureChanged` only if something changed.
    pub fn set_exposure(&mut self, exposure: Exposure) {
        if exposure != self.exposure {
            self.exposure = exposure;
            let _ = self.events.send(RegistryEvent::ExposureChanged);
        }
    }

    /// `set_exposure` from the user settings.
    pub fn apply_settings(&mut self, settings: &UserSettings) {
        let exposure = Exposure::from_settings(settings, &self.config);
        self.set_exposure(exposure);
    }

    pub fn exposure(&self) -> &Exposure {
        &self.exposure
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RegistryEvent> {
        self.events.subscribe()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Exposed tools, sorted by exposed name.
    pub fn exposed_tools(&self) -> Vec<&ToolEntry> {
        self.by_exposed
            .values()
            .filter_map(|(i, t)| self.exposed_entry(i, t))
            .collect()
    }

    pub fn exposed_count(&self) -> usize {
        self.by_exposed
            .values()
            .filter(|(i, t)| self.exposed_entry(i, t).is_some())
            .count()
    }

    /// More exposed tools than `exposed_tools_warning_threshold` (D3).
    pub fn over_threshold(&self) -> bool {
        self.exposed_count() > self.config.gateway.exposed_tools_warning_threshold
    }

    /// The exposed tool with this exposed name. A tool that exists but is not exposed does not resolve.
    pub fn resolve(&self, exposed_name: &str) -> Option<&ToolEntry> {
        let (instance, tool) = self.by_exposed.get(exposed_name)?;
        self.exposed_entry(instance, tool)
    }

    /// Inverse table, whatever the exposure: for the interface and the journal, never to serve a call.
    pub fn lookup(&self, exposed_name: &str) -> Option<&ToolEntry> {
        let (instance, tool) = self.by_exposed.get(exposed_name)?;
        self.instances.get(instance)?.tools.get(tool)
    }

    pub fn instances(&self) -> impl Iterator<Item = &InstanceId> {
        self.instances.keys()
    }

    /// Tools of an instance, exposed or not, sorted by original name.
    pub fn tools(&self, instance: &InstanceId) -> impl Iterator<Item = &ToolEntry> {
        self.instances.get(instance).into_iter().flat_map(|i| i.tools.values())
    }

    pub fn instance_source(&self, instance: &InstanceId) -> Option<CatalogSource> {
        self.instances.get(instance)?.source
    }

    pub fn instance_fingerprint(&self, instance: &InstanceId) -> Option<&Fingerprint> {
        Some(&self.instances.get(instance)?.fingerprint)
    }

    pub fn cache_warnings(&self) -> &[CacheWarning] {
        &self.cache_warnings
    }

    fn exposed_entry(&self, instance: &InstanceId, tool: &ToolName) -> Option<&ToolEntry> {
        if !self.exposure.exposes(instance) {
            return None;
        }
        self.instances.get(instance)?.tools.get(tool)
    }

    /// Replaces an instance and its part of the inverse table. Collisions were checked by `build_checked`.
    fn install(&mut self, instance: InstanceId, tools: InstanceTools) {
        self.by_exposed.retain(|_, (i, _)| i != &instance);
        for entry in tools.tools.values() {
            self.by_exposed.insert(
                entry.exposed_name.clone(),
                (instance.clone(), entry.original_name.clone()),
            );
        }
        self.instances.insert(instance, tools);
    }

    /// `build` plus collisions with the other instances.
    fn build_checked(
        &self,
        instance: &InstanceId,
        definitions: Vec<ToolDefinition>,
        origin: CatalogSource,
    ) -> Result<BTreeMap<ToolName, ToolEntry>, RegistryError> {
        let tools = self.build(instance, definitions, origin)?;
        for entry in tools.values() {
            if let Some((other, tool)) = self.by_exposed.get(&entry.exposed_name)
                && other != instance
            {
                return Err(RegistryError::Collision {
                    exposed_name: entry.exposed_name.clone(),
                    first: format!("{other} {tool}"),
                    second: format!("{instance} {}", entry.original_name),
                });
            }
        }
        Ok(tools)
    }

    fn build(
        &self,
        instance: &InstanceId,
        definitions: Vec<ToolDefinition>,
        origin: CatalogSource,
    ) -> Result<BTreeMap<ToolName, ToolEntry>, RegistryError> {
        let config = &self.config;
        let (Some(mcp), Some(env)) = (config.mcp(&instance.mcp), config.environment(&instance.env)) else {
            return Err(RegistryError::UnknownInstance(instance.clone()));
        };
        if !mcp.instances.contains_key(&instance.env) {
            return Err(RegistryError::UnknownInstance(instance.clone()));
        }
        let env_tag = if env.description.is_empty() {
            format!("[{}]", env.label)
        } else {
            format!("[{} — {}]", env.label, env.description)
        };

        let mut tools = BTreeMap::new();
        let mut exposed: BTreeMap<String, ToolName> = BTreeMap::new();
        for (index, definition) in definitions.into_iter().enumerate() {
            let definition = self.filtered(instance, index, definition)?;
            let name = ToolName::new(definition.name.as_str()).map_err(|reason| RegistryError::InvalidToolName {
                instance: instance.clone(),
                index,
                reason,
            })?;
            let is_object = |v: &Value| v.is_object();
            if !is_object(&definition.input_schema) || definition.output_schema.as_ref().is_some_and(|v| !is_object(v))
            {
                return Err(RegistryError::InvalidSchema {
                    instance: instance.clone(),
                    tool: name,
                });
            }
            if tools.contains_key(&name) {
                return Err(RegistryError::DuplicateTool {
                    instance: instance.clone(),
                    tool: name,
                });
            }
            let exposed_name = exposed_tool_name(&instance.env, &mcp.alias, &name, config.gateway.tool_name_max_len)?;
            if let Some(other) = exposed.insert(exposed_name.clone(), name.clone()) {
                return Err(RegistryError::Collision {
                    exposed_name,
                    first: format!("{instance} {other}"),
                    second: format!("{instance} {name}"),
                });
            }

            let server_hints = definition.annotations.as_ref().map(ToolHints::policy_annotations);
            let (category, category_source) =
                tool_category(config, &instance.env, &instance.mcp, &name, server_hints.as_ref());
            let description = match definition.description.as_deref().map(str::trim) {
                Some(text) if !text.is_empty() => format!("{env_tag} {text}"),
                _ => env_tag.clone(),
            };
            let fingerprint = fingerprint::tool_fingerprint(&definition);
            let entry = ToolEntry {
                instance: instance.clone(),
                title: format!("{} · {} · {}", env.label, mcp.label, name),
                description,
                input_schema: definition.input_schema,
                output_schema: definition.output_schema,
                annotations: ToolHints::for_category(category),
                server_annotations: definition.annotations,
                server_title: definition.title,
                server_description: definition.description,
                category,
                category_source,
                known: category_source != CategorySource::Unknown,
                fingerprint,
                origin,
                exposed_name,
                original_name: name.clone(),
            };
            tools.insert(name, entry);
        }
        Ok(tools)
    }

    /// The definition after the output filter. Its name must come out unchanged: a name cannot be masked.
    fn filtered(
        &self,
        instance: &InstanceId,
        index: usize,
        definition: ToolDefinition,
    ) -> Result<ToolDefinition, RegistryError> {
        let invalid = || RegistryError::InvalidDefinition {
            instance: instance.clone(),
            index,
        };
        let name = definition.name.clone();
        let mut value = serde_json::to_value(definition).map_err(|_| invalid())?;
        self.filter.filter_json(&mut value).map_err(RegistryError::Filter)?;
        let filtered: ToolDefinition = serde_json::from_value(value).map_err(|_| invalid())?;
        if filtered.name != name {
            return Err(RegistryError::MaskedName {
                instance: instance.clone(),
                index,
            });
        }
        Ok(filtered)
    }
}

fn diff(before: &BTreeMap<ToolName, ToolEntry>, after: &BTreeMap<ToolName, ToolEntry>) -> ToolDiff {
    let mut diff = ToolDiff::default();
    for (name, entry) in after {
        match before.get(name) {
            None => diff.added.push(name.clone()),
            Some(old) if old.fingerprint != entry.fingerprint => diff.modified.push(name.clone()),
            Some(_) => {}
        }
    }
    diff.removed = before.keys().filter(|n| !after.contains_key(*n)).cloned().collect();
    diff
}
