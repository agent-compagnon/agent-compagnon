//! Embedded catalogue: tool snapshots compiled into the executable, used until a cache or a live list exists.

use std::collections::BTreeMap;

use mcp_proxy_core::config::{Config, EnvId, McpId};

use super::model::{InstanceId, ToolDefinition};

/// Snapshots of `config/catalog/`. A test checks that this list matches the directory.
const BUILTIN_FILES: &[(&str, &str)] = &[
    (
        "db-readonly.json",
        include_str!("../../../../config/catalog/db-readonly.json"),
    ),
    (
        "gitlab-ops.json",
        include_str!("../../../../config/catalog/gitlab-ops.json"),
    ),
    ("jira.json", include_str!("../../../../config/catalog/jira.json")),
    (
        "k8s-admin.json",
        include_str!("../../../../config/catalog/k8s-admin.json"),
    ),
    (
        "kibana-logs.json",
        include_str!("../../../../config/catalog/kibana-logs.json"),
    ),
];

/// Why a snapshot file cannot be used. French, quotes no content of the file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("catalogue « {file} » : nom de fichier attendu <mcp>.json ou <mcp>.<environnement>.json")]
    FileName { file: String },
    #[error("catalogue « {file} » : MCP inconnu de la config")]
    UnknownMcp { file: String },
    #[error("catalogue « {file} » : ce MCP n'a pas d'instance dans cet environnement")]
    NoInstance { file: String },
    #[error("catalogue « {file} » : fichier en double")]
    Duplicate { file: String },
    #[error("catalogue « {file} » : JSON non conforme, ligne {line}, colonne {column}")]
    Json { file: String, line: usize, column: usize },
}

/// Snapshots by MCP, with optional per-environment overrides.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddedCatalog {
    by_mcp: BTreeMap<McpId, Vec<ToolDefinition>>,
    by_instance: BTreeMap<InstanceId, Vec<ToolDefinition>>,
}

impl EmbeddedCatalog {
    /// The snapshots compiled into the executable, checked against `config`.
    pub fn builtin(config: &Config) -> Result<Self, CatalogError> {
        Self::from_files(config, BUILTIN_FILES)
    }

    /// Builds a catalogue from `(file name, JSON content)` pairs. File names are `<mcp>.json`, or
    /// `<mcp>.<env>.json` for an environment whose server differs; each file is a JSON array of tool definitions.
    pub fn from_files(config: &Config, files: &[(&str, &str)]) -> Result<Self, CatalogError> {
        let mut catalog = Self::default();
        for (file, text) in files {
            let err_file = || (*file).to_owned();
            let stem = file
                .strip_suffix(".json")
                .ok_or_else(|| CatalogError::FileName { file: err_file() })?;
            let (mcp, env) = match stem.split_once('.') {
                None => (stem, None),
                Some((mcp, env)) => (mcp, Some(env)),
            };
            let mcp = McpId::new(mcp).map_err(|_| CatalogError::FileName { file: err_file() })?;
            let env = env
                .map(EnvId::new)
                .transpose()
                .map_err(|_| CatalogError::FileName { file: err_file() })?;
            let declared = config
                .mcp(&mcp)
                .ok_or_else(|| CatalogError::UnknownMcp { file: err_file() })?;
            let tools: Vec<ToolDefinition> = serde_json::from_str(text).map_err(|e| CatalogError::Json {
                file: err_file(),
                line: e.line(),
                column: e.column(),
            })?;
            let replaced = match env {
                None => catalog.by_mcp.insert(mcp, tools).is_some(),
                Some(env) => {
                    if !declared.instances.contains_key(&env) {
                        return Err(CatalogError::NoInstance { file: err_file() });
                    }
                    catalog.by_instance.insert(InstanceId::new(env, mcp), tools).is_some()
                }
            };
            if replaced {
                return Err(CatalogError::Duplicate { file: err_file() });
            }
        }
        Ok(catalog)
    }

    /// The snapshot of an instance: its environment's file, otherwise the MCP's.
    pub fn for_instance(&self, instance: &InstanceId) -> Option<&[ToolDefinition]> {
        self.by_instance
            .get(instance)
            .or_else(|| self.by_mcp.get(&instance.mcp))
            .map(Vec::as_slice)
    }

    #[cfg(test)]
    pub(super) fn builtin_file_names() -> Vec<&'static str> {
        BUILTIN_FILES.iter().map(|(name, _)| *name).collect()
    }
}
