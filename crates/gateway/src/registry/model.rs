//! Types of the registry: instances, tool definitions as exchanged with servers and files, and registry entries.

use std::fmt;

use mcp_proxy_core::config::{Category, EnvId, McpId, ToolName};
use mcp_proxy_core::policy::{CategorySource, ToolAnnotations};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One MCP in one environment. Exists only for the environments listed in `[mcp.instances]`; instances of the same
/// MCP are never merged, since their versions may differ.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceId {
    pub env: EnvId,
    pub mcp: McpId,
}

impl InstanceId {
    pub fn new(env: EnvId, mcp: McpId) -> Self {
        Self { env, mcp }
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.env, self.mcp)
    }
}

/// MCP tool annotations (`ToolAnnotations` of the MCP specification).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

impl ToolHints {
    /// The hints exposed to clients, derived from the retained category only (S10). An unknown tool always has the
    /// `Destructive` category, hence `destructiveHint = true`.
    pub fn for_category(category: Category) -> Self {
        let (read_only, destructive) = match category {
            Category::Read => (true, None),
            Category::Write => (false, Some(false)),
            Category::Destructive => (false, Some(true)),
        };
        Self {
            title: None,
            read_only_hint: Some(read_only),
            destructive_hint: destructive,
            idempotent_hint: None,
            open_world_hint: None,
        }
    }

    /// The part the policy engine derives a category from.
    pub fn policy_annotations(&self) -> ToolAnnotations {
        ToolAnnotations {
            read_only_hint: self.read_only_hint,
            destructive_hint: self.destructive_hint,
        }
    }
}

/// A tool definition, in the MCP `Tool` shape. Used for the embedded catalogue, the cache, the lists received from
/// servers and the lists sent to clients. Spec 006 converts it to and from the types of the MCP SDK.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolHints>,
}

/// Where the tools of an instance come from, by order of preference: `Live`, then `Cache`, then `Embedded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogSource {
    /// Snapshot compiled into the executable (`config/catalog/`).
    Embedded,
    /// Local cache written after a previous live list.
    Cache,
    /// List received from the server during this run.
    Live,
}

/// SHA-256 of the canonical JSON of a tool, or of the sorted fingerprints of an instance, in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub(super) String);

impl Fingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A tool of an instance, as the registry knows it. Every text and schema has been through the output filter.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolEntry {
    pub instance: InstanceId,
    pub original_name: ToolName,
    pub exposed_name: String,
    /// `PROD · Kibana Logs · search_logs`.
    pub title: String,
    /// `[PROD — Production] ` followed by the server description.
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    /// Hints sent to clients, recomputed from `category`.
    pub annotations: ToolHints,
    /// Hints given by the server: used for the category of a declared tool without one, and for the fingerprint.
    /// Never sent to clients.
    pub server_annotations: Option<ToolHints>,
    /// Title given by the server, kept for the interface. Never sent to clients as is.
    pub server_title: Option<String>,
    /// Description given by the server, without the environment prefix.
    pub server_description: Option<String>,
    pub category: Category,
    pub category_source: CategorySource,
    /// Declared in `[mcp.tools]` of the configuration. Being listed by a server or a cache never makes a tool known.
    pub known: bool,
    pub fingerprint: Fingerprint,
    pub origin: CatalogSource,
}

impl ToolEntry {
    /// The definition sent to clients: exposed name, title and description, filtered schemas, recomputed hints.
    pub fn exposed_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.exposed_name.clone(),
            title: Some(self.title.clone()),
            description: Some(self.description.clone()),
            input_schema: self.input_schema.clone(),
            output_schema: self.output_schema.clone(),
            annotations: Some(self.annotations.clone()),
        }
    }

    /// The definition as the server gave it, after filtering: what the cache stores.
    pub fn server_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.original_name.as_str().to_owned(),
            title: self.server_title.clone(),
            description: self.server_description.clone(),
            input_schema: self.input_schema.clone(),
            output_schema: self.output_schema.clone(),
            annotations: self.server_annotations.clone(),
        }
    }
}

/// Difference between two lists of one instance, by original tool name, each list sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolDiff {
    pub added: Vec<ToolName>,
    pub removed: Vec<ToolName>,
    /// Present in both lists with a different fingerprint (description, input schema or annotations).
    pub modified: Vec<ToolName>,
}

impl ToolDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}
