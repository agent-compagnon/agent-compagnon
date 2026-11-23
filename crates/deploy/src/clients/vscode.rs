//! VS Code: the user configuration `mcp.json` of the default profile, `%APPDATA%\Code\User\mcp.json` (the file
//! « MCP: Open User Configuration » opens), servers under `servers`, a local server written `"type": "stdio"`,
//! `command` and `args`. The named profiles (`User\profiles\<id>\`) and VS Code Insiders are not handled.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Adapter, Environment, SERVE_ARG};

pub(crate) const FILE: &str = "mcp.json";

pub(crate) struct VsCode;

impl Adapter for VsCode {
    fn section(&self) -> &'static str {
        "servers"
    }

    fn fields(&self, exe: &Path) -> Vec<(&'static str, Value)> {
        vec![
            ("type", json!("stdio")),
            ("command", Value::String(exe.to_string_lossy().into_owned())),
            ("args", json!([SERVE_ARG])),
        ]
    }
}

pub(crate) fn user_dir(env: &Environment) -> PathBuf {
    env.app_data.join("Code").join("User")
}
