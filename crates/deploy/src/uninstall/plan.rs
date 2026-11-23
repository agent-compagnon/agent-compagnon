//! What the uninstallation removes, in which order: a pure function of the paths and of the state of the LLM
//! clients, computed before anything is touched.

use std::path::{Path, PathBuf};

use crate::clients::{Inspection, LocationId, State, Unreadable};
use crate::install::{InstallPaths, same_location};

/// The folder of the data, under `%LOCALAPPDATA%` (spec 011).
pub const DATA_FOLDER: &str = "MCP Proxy";
/// The WebView2 data of the windows of the bundle (main window, installation, confirmations): Tauri puts it in
/// `%LOCALAPPDATA%\<identifier>` when no folder is given (`app/tauri.conf.json`).
pub const WINDOW_DATA_FOLDER: &str = "com.mcpproxy.app";
/// The parent of the installation folder, under `%LOCALAPPDATA%`, and the folder itself (spec 019).
pub const PROGRAMS_FOLDER: &str = "Programs";
pub const INSTALL_FOLDER: &str = "MCP Proxy";

/// Every path the uninstallation may touch, derived from the two folders of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPaths {
    pub install: InstallPaths,
    /// `%LOCALAPPDATA%`
    pub local_app_data: PathBuf,
    /// `%LOCALAPPDATA%\MCP Proxy`
    pub data: PathBuf,
    /// `%LOCALAPPDATA%\com.mcpproxy.app`
    pub window_data: PathBuf,
}

impl UninstallPaths {
    pub fn new(local_app_data: &Path, app_data: &Path) -> Self {
        Self {
            install: InstallPaths::new(local_app_data, app_data),
            local_app_data: local_app_data.to_path_buf(),
            data: local_app_data.join(DATA_FOLDER),
            window_data: local_app_data.join(WINDOW_DATA_FOLDER),
        }
    }

    /// `%LOCALAPPDATA%\Programs`
    pub fn programs(&self) -> PathBuf {
        self.local_app_data.join(PROGRAMS_FOLDER)
    }
}

/// What is done with the entry `mcp-proxy` of a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientDecision {
    /// It starts the installed exe: removed, after a backup of the file.
    Remove,
    /// It starts the installed exe, but the user unticked the box.
    Asked,
    /// It starts another program: left.
    OtherExe(Option<String>),
    /// The file cannot be read safely: nothing is written, the user is told to look.
    Unreadable(Unreadable),
}

/// One thing the uninstallation takes care of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Client {
        location: LocationId,
        decision: ClientDecision,
    },
    /// The `Run` value.
    Autostart,
    Shortcut,
    /// `HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop` (spec 013).
    Aumid,
    /// The value of `ExcludedApplications` (spec 011).
    ErrorReports,
    /// `%LOCALAPPDATA%\MCP Proxy`.
    Data,
    /// The entry « Applications installées ».
    UninstallEntry,
    /// The exe, set aside before the instance stopped.
    Exe,
    InstallFolder,
    /// `%LOCALAPPDATA%\com.mcpproxy.app`, held by the window of the uninstallation until it closes.
    WindowData,
}

/// The entry `mcp-proxy` of a location, as the uninstallation treats it; `None` when there is none.
pub fn decide(inspection: &Inspection, exe: &Path, remove_clients: bool) -> Option<ClientDecision> {
    let ours = match &inspection.state {
        State::Configured => true,
        State::OtherCommand(summary) => summary
            .command
            .as_deref()
            .is_some_and(|command| same_location(Path::new(command), exe)),
        State::Unreadable(reason) => return Some(ClientDecision::Unreadable(reason.clone())),
        State::NotInstalled | State::NotConfigured { .. } => return None,
    };
    Some(match (ours, &inspection.state) {
        (true, _) if remove_clients => ClientDecision::Remove,
        (true, _) => ClientDecision::Asked,
        (false, State::OtherCommand(summary)) => ClientDecision::OtherExe(summary.command.clone()),
        (false, _) => return None,
    })
}

/// Everything, in order. The LLM clients first: once their entry is gone, a client that restarts no longer starts
/// MCP Proxy. Then what the application put in the registry and the Start menu, its data, and last what lets the
/// user run the uninstallation again: its entry, the exe and its folder. The data of the window go once it closes.
pub fn plan(clients: &[Inspection], exe: &Path, remove_clients: bool) -> Vec<Item> {
    let clients = clients.iter().filter_map(|inspection| {
        decide(inspection, exe, remove_clients).map(|decision| Item::Client {
            location: inspection.location,
            decision,
        })
    });
    clients
        .chain([
            Item::Autostart,
            Item::Shortcut,
            Item::Aumid,
            Item::ErrorReports,
            Item::Data,
            Item::UninstallEntry,
            Item::Exe,
            Item::InstallFolder,
            Item::WindowData,
        ])
        .collect()
}
