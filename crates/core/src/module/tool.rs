//! Tool definitions in the MCP `Tool` shape, shared by the registry (embedded catalogue, cache, lists of servers and
//! clients) and the local modules, which declare their tools with them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Category;
use crate::policy::ToolAnnotations;

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
/// servers and the lists sent to clients. The gateway converts it to and from the types of the MCP SDK.
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
