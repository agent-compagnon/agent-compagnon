//! Scripts handed to the windows, and the escaping of the data written into them.

use serde_json::{Value, json};

use crate::ui::{BasicPromptSpec, HeaderCapture};

/// The command the capture script calls: the only command a remote page can reach.
pub const DEPOSIT_COMMAND: &str = "deposit_login_header";

/// The command of the Basic window, allowed for that window only.
pub const BASIC_COMMAND: &str = "submit_basic_credentials";

/// Source of the capture script, with its configuration placeholder.
pub const CAPTURE_SOURCE: &str = include_str!("capture.js");

const CONFIG_PLACEHOLDER: &str = "__MCP_PROXY_CAPTURE_CONFIG__";

/// `value` written as a JavaScript expression that can only be data. JSON is a subset of JavaScript expressions;
/// on top of that, every character outside printable ASCII, and `<`, `>`, `&`, `'` and `` ` ``, is written as a
/// `\uXXXX` escape. These characters can only occur inside JSON strings, where the escape is equivalent, so the
/// value read back is unchanged, and the text contains no line terminator, no quote other than the JSON delimiters,
/// nothing an HTML parser or a template literal would interpret.
pub fn js_data(value: &Value) -> String {
    let json = value.to_string();
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        let plain = matches!(c, ' '..='~') && !matches!(c, '<' | '>' | '&' | '\'' | '`');
        if plain {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// The capture script of a `jwt_header` window, its configuration written in as data.
pub fn capture_script(capture: &HeaderCapture) -> String {
    let pattern = capture.url_pattern();
    let config = json!({
        "prefix": pattern.prefix.as_str(),
        "wildcard": pattern.wildcard,
        "header": capture.header().to_ascii_lowercase(),
        "scheme": capture.scheme(),
        "command": DEPOSIT_COMMAND,
    });
    CAPTURE_SOURCE.replacen(CONFIG_PLACEHOLDER, &js_data(&config), 1)
}

/// What the Basic page shows, defined before the page runs as a frozen, read-only `window.__MCP_PROXY_BASIC__`.
/// Never a password: only the login of the previous attempt.
pub fn basic_prompt_script(spec: &BasicPromptSpec) -> String {
    let data = json!({
        "title": spec.title(),
        "login": spec.login(),
        "error": spec.error(),
        "attempt": spec.attempt(),
        "maxAttempts": spec.max_attempts(),
        "command": BASIC_COMMAND,
    });
    format!(
        "Object.defineProperty(window, '__MCP_PROXY_BASIC__', {{ value: Object.freeze({}) }});",
        js_data(&data)
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use mcp_proxy_core::config::SessionId;

    use super::*;

    // Exigence : EX-012-15
    #[test]
    fn basic_page_data_is_written_as_data() {
        let spec = BasicPromptSpec {
            session: SessionId::new("sso-tools").unwrap(),
            title: "Connexion — SSO Outils".to_owned(),
            login: Some("alice'); alert(1); ('</script>".to_owned()),
            error: Some("identifiants refusés".to_owned()),
            attempt: 2,
            max_attempts: 3,
        };
        let script = basic_prompt_script(&spec);
        let prefix = "Object.defineProperty(window, '__MCP_PROXY_BASIC__', { value: Object.freeze(";
        let suffix = ") });";
        assert!(script.starts_with(prefix) && script.ends_with(suffix), "{script}");
        let data = &script[prefix.len()..script.len() - suffix.len()];
        assert!(
            data.chars().all(|c| matches!(c, ' '..='~') && !"<>&'`".contains(c)),
            "{data}"
        );
        let value: Value = serde_json::from_str(data).unwrap();
        assert_eq!(
            value,
            json!({
                "title": "Connexion — SSO Outils",
                "login": "alice'); alert(1); ('</script>",
                "error": "identifiants refusés",
                "attempt": 2,
                "maxAttempts": 3,
                "command": "submit_basic_credentials",
            })
        );
    }
}
