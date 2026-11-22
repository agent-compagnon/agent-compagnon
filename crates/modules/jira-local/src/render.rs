//! Concise outputs: a readable text and the same fields structured, limited to what is useful to an LLM.

use mcp_proxy_core::module::{Map, ToolOutput, Value, json};

const MAX_SUMMARY_CHARS: usize = 300;
const MAX_DESCRIPTION_CHARS: usize = 4000;
const MAX_COMMENT_CHARS: usize = 1000;
/// Comments shown by `get_issue`, the most recent ones.
pub(crate) const LAST_COMMENTS: usize = 5;

/// `text` cut to `max` characters, with a marker when cut.
fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}… [tronqué]", &text[..end]),
        None => text.to_owned(),
    }
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, key| v.get(key)).and_then(Value::as_str)
}

fn name_of(fields: &Value, field: &str) -> Option<String> {
    string_at(fields, &[field, "name"]).map(str::to_owned)
}

fn person(fields: &Value, field: &str) -> Option<String> {
    string_at(fields, &[field, "displayName"]).map(str::to_owned)
}

fn put(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    map.insert(key.to_owned(), value.map_or(Value::Null, Value::String));
}

/// The common fields of an issue, structured.
fn summary_fields(key: &str, fields: &Value) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("key".to_owned(), Value::String(key.to_owned()));
    put(
        &mut map,
        "summary",
        string_at(fields, &["summary"]).map(|s| shorten(s, MAX_SUMMARY_CHARS)),
    );
    put(&mut map, "status", name_of(fields, "status"));
    put(&mut map, "type", name_of(fields, "issuetype"));
    put(&mut map, "priority", name_of(fields, "priority"));
    put(&mut map, "assignee", person(fields, "assignee"));
    put(&mut map, "updated", string_at(fields, &["updated"]).map(str::to_owned));
    map
}

fn field<'a>(map: &'a Map<String, Value>, key: &str) -> &'a str {
    map.get(key).and_then(Value::as_str).unwrap_or("—")
}

fn issue_line(map: &Map<String, Value>) -> String {
    let assignee = map
        .get("assignee")
        .and_then(Value::as_str)
        .map_or_else(|| "non assigné".to_owned(), |a| format!("assigné à {a}"));
    format!(
        "- {} [{}] {} — {}, priorité {}, {assignee}, mis à jour le {}",
        field(map, "key"),
        field(map, "status"),
        field(map, "summary"),
        field(map, "type"),
        field(map, "priority"),
        field(map, "updated"),
    )
}

pub(crate) fn search(body: &Value) -> ToolOutput {
    let issues: Vec<Map<String, Value>> = body
        .get("issues")
        .and_then(Value::as_array)
        .map(|issues| {
            issues
                .iter()
                .filter_map(|issue| {
                    let key = issue.get("key")?.as_str()?;
                    Some(summary_fields(key, issue.get("fields").unwrap_or(&Value::Null)))
                })
                .collect()
        })
        .unwrap_or_default();
    let total = body.get("total").and_then(Value::as_u64).unwrap_or(issues.len() as u64);
    let mut text = match (issues.len(), total) {
        (0, _) => "Aucun ticket ne correspond à la requête.".to_owned(),
        (n, t) if (n as u64) < t => format!("{n} tickets affichés sur {t} trouvés :"),
        (1, _) => "1 ticket trouvé :".to_owned(),
        (n, _) => format!("{n} tickets trouvés :"),
    };
    for issue in &issues {
        text.push('\n');
        text.push_str(&issue_line(issue));
    }
    ToolOutput::text(text).with_structured(json!({ "total": total, "issues": issues }))
}

pub(crate) fn issue(key: &str, body: &Value) -> ToolOutput {
    let fields = body.get("fields").unwrap_or(&Value::Null);
    let key = body.get("key").and_then(Value::as_str).unwrap_or(key);
    let mut map = summary_fields(key, fields);
    put(&mut map, "reporter", person(fields, "reporter"));
    put(&mut map, "created", string_at(fields, &["created"]).map(str::to_owned));
    put(
        &mut map,
        "description",
        string_at(fields, &["description"]).map(|d| shorten(d, MAX_DESCRIPTION_CHARS)),
    );
    let all: &[Value] = fields
        .get("comment")
        .and_then(|c| c.get("comments"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let comments: Vec<Value> = all[all.len().saturating_sub(LAST_COMMENTS)..]
        .iter()
        .map(|c| {
            json!({
                "author": string_at(c, &["author", "displayName"]),
                "created": string_at(c, &["created"]),
                "body": string_at(c, &["body"]).map(|b| shorten(b, MAX_COMMENT_CHARS)),
            })
        })
        .collect();
    let total_comments = fields
        .get("comment")
        .and_then(|c| c.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(all.len() as u64);

    let mut text = format!(
        "{} — {}\nStatut : {} · Type : {} · Priorité : {}\nAssigné à : {} · Rapporteur : {}\nCréé le {} · mis à jour \
         le {}\n\nDescription :\n{}",
        field(&map, "key"),
        field(&map, "summary"),
        field(&map, "status"),
        field(&map, "type"),
        field(&map, "priority"),
        map.get("assignee").and_then(Value::as_str).unwrap_or("personne"),
        field(&map, "reporter"),
        field(&map, "created"),
        field(&map, "updated"),
        map.get("description").and_then(Value::as_str).unwrap_or("(aucune)"),
    );
    if comments.is_empty() {
        text.push_str("\n\nAucun commentaire.");
    } else {
        text.push_str(&format!(
            "\n\nDerniers commentaires ({} sur {total_comments}) :",
            comments.len()
        ));
        for comment in &comments {
            text.push_str(&format!(
                "\n- {} le {} : {}",
                comment.get("author").and_then(Value::as_str).unwrap_or("—"),
                comment.get("created").and_then(Value::as_str).unwrap_or("—"),
                comment.get("body").and_then(Value::as_str).unwrap_or(""),
            ));
        }
    }
    map.insert("comments".to_owned(), Value::Array(comments));
    map.insert("total_comments".to_owned(), json!(total_comments));
    ToolOutput::text(text).with_structured(Value::Object(map))
}

pub(crate) fn comment(key: &str, body: &Value) -> ToolOutput {
    let id = string_at(body, &["id"]).map(str::to_owned);
    let created = string_at(body, &["created"]).map(str::to_owned);
    let text = match (&id, &created) {
        (Some(id), Some(created)) => format!("Commentaire {id} ajouté à {key} le {created}."),
        (Some(id), None) => format!("Commentaire {id} ajouté à {key}."),
        _ => format!("Commentaire ajouté à {key}."),
    };
    ToolOutput::text(text).with_structured(json!({ "key": key, "id": id, "created": created }))
}
