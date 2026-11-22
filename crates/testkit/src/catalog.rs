//! Tools served by the fake MCP servers: those of the embedded catalogue, plus test tools.

use rmcp::model::Tool;
use serde_json::json;

const CATALOG_FILES: &[(&str, &str)] = &[
    ("db-readonly", include_str!("../../../config/catalog/db-readonly.json")),
    ("gitlab-ops", include_str!("../../../config/catalog/gitlab-ops.json")),
    ("jira", include_str!("../../../config/catalog/jira.json")),
    ("k8s-admin", include_str!("../../../config/catalog/k8s-admin.json")),
    ("kibana-logs", include_str!("../../../config/catalog/kibana-logs.json")),
];

/// The tools of `config/catalog/<mcp>.json`, or `None` for an MCP without snapshot (or an unreadable one).
pub fn catalog_tools(mcp: &str) -> Option<Vec<Tool>> {
    let (_, text) = CATALOG_FILES.iter().find(|(name, _)| *name == mcp)?;
    serde_json::from_str(text).ok()
}

/// Tools with a test behaviour, served on demand (`MockMcpBuilder::with_test_tools`):
/// - `echo_headers`: returns the HTTP headers received with the call;
/// - `slow`: sends progress notifications for `millis` milliseconds, then answers;
/// - `fail`: answers with a JSON-RPC error quoting the received `Authorization` header;
/// - `http_fail`: the HTTP layer answers 400 with a plain-text body quoting the received `Authorization` header.
pub fn test_tools() -> Vec<Tool> {
    let tools = json!([
        {
            "name": "echo_headers",
            "description": "Returns the HTTP headers received with the call.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "slow",
            "description": "Answers after a delay, with progress notifications.",
            "inputSchema": {
                "type": "object",
                "properties": { "millis": { "type": "integer", "minimum": 0 } }
            }
        },
        {
            "name": "fail",
            "description": "Always fails with a JSON-RPC error.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "http_fail",
            "description": "Always fails at the HTTP layer.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ]);
    serde_json::from_value(tools).unwrap_or_default()
}
