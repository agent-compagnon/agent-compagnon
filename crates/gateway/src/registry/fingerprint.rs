//! Fingerprints: SHA-256 of a canonical JSON form, so that key order never changes them.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::model::{Fingerprint, ToolDefinition};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

enum Step<'a> {
    Value(&'a Value),
    Key(&'a str),
    Raw(&'static str),
}

/// Compact JSON with object keys sorted by their UTF-8 bytes, whatever the map implementation of `serde_json`.
/// Iterative, so that a deeply nested schema cannot overflow the stack.
pub(crate) fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    let mut stack = vec![Step::Value(value)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Raw(text) => out.push_str(text),
            Step::Key(key) => {
                out.push_str(&Value::String(key.to_owned()).to_string());
                out.push(':');
            }
            Step::Value(Value::Array(items)) => {
                out.push('[');
                stack.push(Step::Raw("]"));
                for (i, item) in items.iter().enumerate().rev() {
                    stack.push(Step::Value(item));
                    if i > 0 {
                        stack.push(Step::Raw(","));
                    }
                }
            }
            Step::Value(Value::Object(map)) => {
                out.push('{');
                stack.push(Step::Raw("}"));
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
                for (i, (key, item)) in entries.into_iter().enumerate().rev() {
                    stack.push(Step::Value(item));
                    stack.push(Step::Key(key));
                    if i > 0 {
                        stack.push(Step::Raw(","));
                    }
                }
            }
            Step::Value(scalar) => out.push_str(&scalar.to_string()),
        }
    }
    out
}

/// Fingerprint of one tool: `name`, `description`, `inputSchema`, `outputSchema` and `annotations` as given by the
/// server (after filtering). The title is left out: clients never see the server's title.
pub(super) fn tool_fingerprint(definition: &ToolDefinition) -> Fingerprint {
    let mut map = Map::new();
    map.insert("name".to_owned(), Value::String(definition.name.clone()));
    map.insert("description".to_owned(), json!(definition.description));
    map.insert("inputSchema".to_owned(), definition.input_schema.clone());
    map.insert("outputSchema".to_owned(), json!(definition.output_schema));
    map.insert(
        "annotations".to_owned(),
        serde_json::to_value(&definition.annotations).unwrap_or(Value::Null),
    );
    let canonical = canonical_json(&Value::Object(map));
    Fingerprint(hex(&Sha256::digest(canonical.as_bytes())))
}

/// Fingerprint of an instance: hash of the sorted fingerprints of its tools, one per line.
pub(super) fn instance_fingerprint<'a>(tools: impl IntoIterator<Item = &'a Fingerprint>) -> Fingerprint {
    let mut prints: Vec<&str> = tools.into_iter().map(Fingerprint::as_str).collect();
    prints.sort_unstable();
    let mut hasher = Sha256::new();
    for print in prints {
        hasher.update(print.as_bytes());
        hasher.update(b"\n");
    }
    Fingerprint(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_form_sorts_keys_at_every_depth() {
        let value: Value = serde_json::from_str(r#"{"b":[1,{"z":null,"a":"é\"x"}],"a":{"d":true,"c":1.5}}"#).unwrap();
        assert_eq!(
            canonical_json(&value),
            r#"{"a":{"c":1.5,"d":true},"b":[1,{"a":"é\"x","z":null}]}"#
        );
        assert_eq!(canonical_json(&json!([])), "[]");
        assert_eq!(canonical_json(&json!({})), "{}");
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        let mut value = json!(0);
        for _ in 0..100_000 {
            value = Value::Array(vec![value]);
        }
        assert_eq!(canonical_json(&value).len(), 200_001);
        // Dropping a deeply nested `Value` is recursive in serde_json: unwind it by hand.
        let mut current = value;
        while let Value::Array(mut items) = current {
            current = items.pop().unwrap_or(Value::Null);
        }
    }
}
