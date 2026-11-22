//! Conversions between the registry's `ToolDefinition` and the `Tool` of the MCP SDK.
//!
//! Only the fields the registry knows are kept: `_meta`, `icons`, `execution` and any future field of a server's
//! tool are dropped, as are unknown annotation keys.

use rmcp::model::Tool;
use serde_json::{Map, Value};

use crate::registry::ToolDefinition;

const TOOL_KEYS: [&str; 6] = [
    "name",
    "title",
    "description",
    "inputSchema",
    "outputSchema",
    "annotations",
];
const HINT_KEYS: [&str; 5] = [
    "title",
    "readOnlyHint",
    "destructiveHint",
    "idempotentHint",
    "openWorldHint",
];

fn retain(map: &mut Map<String, Value>, keys: &[&str]) {
    map.retain(|key, value| keys.contains(&key.as_str()) && !value.is_null());
}

/// A tool as given by a server, reduced to what the registry knows.
pub fn definition_from_tool(tool: &Tool) -> Result<ToolDefinition, serde_json::Error> {
    let mut value = serde_json::to_value(tool)?;
    if let Value::Object(map) = &mut value {
        retain(map, &TOOL_KEYS);
        if let Some(Value::Object(hints)) = map.get_mut("annotations") {
            retain(hints, &HINT_KEYS);
        }
    }
    serde_json::from_value(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn unknown_fields_of_a_server_tool_are_dropped() {
        let tool: Tool = serde_json::from_value(json!({
            "name": "search_logs",
            "description": "Search.",
            "inputSchema": { "type": "object" },
            "annotations": { "readOnlyHint": true, "futureHint": 3 },
            "icons": [{ "src": "https://x.example/icon.png" }],
            "_meta": { "vendor": "x" }
        }))
        .unwrap();
        let definition = definition_from_tool(&tool).unwrap();
        assert_eq!(definition.name, "search_logs");
        assert_eq!(definition.annotations.unwrap().read_only_hint, Some(true));
        let back: Tool = serde_json::from_value(
            serde_json::to_value(&ToolDefinition {
                name: "x".to_owned(),
                title: None,
                description: None,
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                annotations: None,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(back.name, "x");
    }
}
