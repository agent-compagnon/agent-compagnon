//! The version of Windows, for the diagnostic export: read in the registry (`winreg`), formatted by the gateway.

use mcp_proxy_gateway::interface::describe_system;
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// `Windows 11 Pro 24H2 (build 26100.2314), WebView2 …`; a value that cannot be read is said unknown.
pub fn description(webview: &str) -> String {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(KEY).ok();
    let text = |name: &str| key.as_ref().and_then(|k| k.get_value::<String, _>(name).ok());
    let build = text("CurrentBuild").map(
        |build| match key.as_ref().and_then(|k| k.get_value::<u32, _>("UBR").ok()) {
            Some(revision) => format!("{build}.{revision}"),
            None => build,
        },
    );
    describe_system(
        text("ProductName").as_deref(),
        text("DisplayVersion").as_deref(),
        build.as_deref(),
        webview,
    )
}
