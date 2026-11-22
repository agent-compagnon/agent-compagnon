//! `mcp-proxy.exe`, see the library.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    mcp_proxy::main()
}
