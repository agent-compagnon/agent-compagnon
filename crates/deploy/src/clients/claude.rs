//! Claude Desktop: `claude_desktop_config.json`, servers under `mcpServers`, `command` and `args`.
//!
//! Two places. The documented one, `%APPDATA%\Claude\`, which the « Edit Config » button of the application opens.
//! And, for the application installed by its MSIX package, the folder Windows redirects it to,
//! `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\` (reported by users in
//! `anthropics/claude-code#26073`, not documented: to check on a real installation).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Adapter, Environment, SERVE_ARG};

pub(crate) const FILE: &str = "claude_desktop_config.json";
/// Package family name of the MSIX package, as reported; an assumption until checked.
pub(crate) const PACKAGE: &str = "Claude_pzs8sxrjxfjjc";

pub(crate) struct ClaudeDesktop;

impl Adapter for ClaudeDesktop {
    fn section(&self) -> &'static str {
        "mcpServers"
    }

    fn fields(&self, exe: &Path) -> Vec<(&'static str, Value)> {
        vec![
            ("command", Value::String(exe.to_string_lossy().into_owned())),
            ("args", json!([SERVE_ARG])),
        ]
    }
}

pub(crate) fn classic_dir(env: &Environment) -> PathBuf {
    env.app_data.join("Claude")
}

pub(crate) fn package_dir(env: &Environment) -> PathBuf {
    env.local_app_data
        .join("Packages")
        .join(PACKAGE)
        .join("LocalCache")
        .join("Roaming")
        .join("Claude")
}
