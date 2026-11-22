//! `mcp-proxy.exe`: the application (notification icon, gateway) without argument, the `stdio` bridge with
//! `stdio`. Wiring only: the logic lives in `crates/` and is tested there (spec 011).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("MCP Proxy is a Windows application: check it with `--target x86_64-pc-windows-msvc`.");

mod app;
mod bridge;
mod dialog;
mod dpapi;
mod pipe;
mod wer;

use std::process::ExitCode;

use mcp_proxy_gateway::launch::{LaunchMode, parse_launch};

fn main() -> ExitCode {
    match parse_launch(std::env::args_os().skip(1)) {
        Ok(LaunchMode::App) => app::run(),
        Ok(LaunchMode::Stdio) => bridge::run(),
        Err(error) => {
            bridge::write_stderr(&format!("MCP Proxy : {error}"));
            dialog::error("MCP Proxy", &error.to_string());
            ExitCode::from(2)
        }
    }
}
