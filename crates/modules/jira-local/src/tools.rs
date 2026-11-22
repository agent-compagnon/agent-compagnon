//! Tool definitions and argument checks. The definitions are also the embedded snapshot `config/catalog/jira.json`
//! (a test keeps both equal).

use mcp_proxy_core::module::{JsonObject, ModuleError, ToolDefinition, ToolHints, Value, json};

pub(crate) const SEARCH_ISSUES: &str = "search_issues";
pub(crate) const GET_ISSUE: &str = "get_issue";
pub(crate) const ADD_COMMENT: &str = "add_comment";

pub(crate) const MAX_JQL_LEN: usize = 2000;
pub(crate) const DEFAULT_MAX_RESULTS: u64 = 20;
pub(crate) const MAX_RESULTS: u64 = 50;
/// Jira refuses longer comments.
pub(crate) const MAX_COMMENT_LEN: usize = 32_767;
pub(crate) const MAX_KEY_LEN: usize = 64;

const KEY_PATTERN: &str = "^[A-Za-z][A-Za-z0-9_]*-[1-9][0-9]*$";

fn hints(read_only: bool) -> Option<ToolHints> {
    Some(ToolHints {
        read_only_hint: Some(read_only),
        destructive_hint: (!read_only).then_some(false),
        ..ToolHints::default()
    })
}

fn key_schema() -> Value {
    json!({
        "type": "string",
        "description": "Issue key: project key, dash, number, e.g. OPS-123.",
        "pattern": KEY_PATTERN,
        "maxLength": MAX_KEY_LEN
    })
}

pub(crate) fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: SEARCH_ISSUES.to_owned(),
            title: Some("Search issues".to_owned()),
            description: Some(
                "Searches Jira issues with a JQL query and returns, for each match, its key, summary, status, type, \
                 priority, assignee and last update, most recently updated first unless the query has an ORDER BY. \
                 Example: project = OPS AND status != Done ORDER BY updated DESC."
                    .to_owned(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "jql": {
                        "type": "string",
                        "description": "JQL query, e.g. project = OPS AND text ~ \"timeout\".",
                        "minLength": 1,
                        "maxLength": MAX_JQL_LEN
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of issues returned.",
                        "minimum": 1,
                        "maximum": MAX_RESULTS,
                        "default": DEFAULT_MAX_RESULTS
                    }
                },
                "required": ["jql"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: hints(true),
        },
        ToolDefinition {
            name: GET_ISSUE.to_owned(),
            title: Some("Get issue".to_owned()),
            description: Some(
                "Returns one Jira issue: summary, status, type, priority, assignee, reporter, dates, description and \
                 its last comments. Long texts are shortened."
                    .to_owned(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": { "key": key_schema() },
                "required": ["key"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: hints(true),
        },
        ToolDefinition {
            name: ADD_COMMENT.to_owned(),
            title: Some("Add comment".to_owned()),
            description: Some(
                "Adds a comment to a Jira issue, as the connected user. The text is posted as is (Jira wiki markup). \
                 Returns the id and date of the new comment."
                    .to_owned(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": key_schema(),
                    "body": {
                        "type": "string",
                        "description": "Text of the comment.",
                        "minLength": 1,
                        "maxLength": MAX_COMMENT_LEN
                    }
                },
                "required": ["key", "body"],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: hints(false),
        },
    ]
}

fn invalid(message: impl Into<String>) -> ModuleError {
    ModuleError::InvalidArguments(message.into())
}

/// Refuses any argument outside `allowed`: the schemas say `additionalProperties: false`.
pub(crate) fn only(args: &JsonObject, allowed: &[&str]) -> Result<(), ModuleError> {
    match args.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(_) => Err(invalid(format!(
            "argument inattendu ; arguments acceptés : {}",
            allowed.join(", ")
        ))),
        None => Ok(()),
    }
}

pub(crate) fn text<'a>(args: &'a JsonObject, name: &str, max_len: usize) -> Result<&'a str, ModuleError> {
    let value = match args.get(name) {
        None | Some(Value::Null) => return Err(invalid(format!("« {name} » est obligatoire"))),
        Some(Value::String(value)) => value,
        Some(_) => return Err(invalid(format!("« {name} » doit être une chaîne"))),
    };
    if value.trim().is_empty() {
        return Err(invalid(format!("« {name} » est vide")));
    }
    if value.chars().count() > max_len {
        return Err(invalid(format!("« {name} » dépasse {max_len} caractères")));
    }
    Ok(value)
}

pub(crate) fn max_results(args: &JsonObject) -> Result<u64, ModuleError> {
    match args.get("max_results") {
        None | Some(Value::Null) => Ok(DEFAULT_MAX_RESULTS),
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_RESULTS).contains(n))
            .ok_or_else(|| invalid(format!("« max_results » doit être un entier de 1 à {MAX_RESULTS}"))),
    }
}

/// A valid issue key, upper-cased (Jira keys are case-insensitive).
pub(crate) fn issue_key(args: &JsonObject) -> Result<String, ModuleError> {
    let key = text(args, "key", MAX_KEY_LEN)?;
    let valid = key.split_once('-').is_some_and(|(project, number)| {
        let mut chars = project.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !number.is_empty()
            && !number.starts_with('0')
            && number.chars().all(|c| c.is_ascii_digit())
    });
    if valid {
        Ok(key.to_ascii_uppercase())
    } else {
        Err(invalid("« key » n'est pas une clé de ticket (exemple : OPS-123)"))
    }
}
