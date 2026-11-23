//! Self-diagnostic (spec 024): `mcp-proxy.exe doctor`, before the single instance, and the probes the window's
//! « Auto-diagnostic » shares with it. Wiring only: the checks, the report and its file are in
//! `mcp_proxy_deploy::doctor` and `mcp_proxy_gateway::doctor`.
//!
//! Every probe reads: the registry by `winreg` (through `WindowsOps`), the state of the toasts by WinRT (nothing is
//! registered), the running instance by the `ping` of the control channel, after the mutual proof (it stops
//! nothing and shows nothing).

use std::os::windows::io::AsRawHandle as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::trace::SystemWallClock;
use mcp_proxy_deploy::clients::RealFiles;
use mcp_proxy_deploy::doctor::{
    AnonymousWeb, InstanceProbe, NoNetwork, PipeProbe, Published, RealReportFiles, Saved, ToastProbe, Toasts, Web,
};
use mcp_proxy_deploy::ops::OpError;
use mcp_proxy_deploy::update::embedded_slots;
use mcp_proxy_gateway::bridge::{pipe_name, request_ping};
use mcp_proxy_gateway::doctor::{DoctorProbes, run_command};
use mcp_proxy_gateway::notify::AUMID;
use tokio::net::windows::named_pipe::ClientOptions;
use windows::UI::Notifications::ToastNotificationManager;
use windows::core::HSTRING;

use crate::dialog;
use crate::dpapi::DpapiProof;
use crate::system_ops::WindowsOps;

const TITLE: &str = "MCP Proxy — auto-diagnostic";
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
/// How long a busy pipe is retried before it is said not to answer.
const BUSY_RETRIES: u32 = 20;
const BUSY_WAIT: Duration = Duration::from_millis(100);

/// The state of the toasts of the AUMID, read: `Setting` of its notifier and the « Ne pas déranger » mode. Called
/// on a blocking thread.
struct WindowsToasts;

impl ToastProbe for WindowsToasts {
    fn toasts(&self) -> Toasts {
        let setting = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
            .and_then(|n| n.Setting())
            .ok()
            .map(|s| s.0);
        let mode = ToastNotificationManager::GetDefault()
            .and_then(|m| m.NotificationMode())
            .ok()
            .map(|m| m.0);
        Toasts { setting, mode }
    }
}

/// The running instance, asked through its pipe as a control client: proof, then `ping`.
struct PipeInstance {
    name: Option<String>,
}

#[async_trait]
impl InstanceProbe for PipeInstance {
    async fn probe(&self) -> PipeProbe {
        let Some(name) = &self.name else {
            return PipeProbe::NoUser;
        };
        let mut busy = 0;
        let pipe = loop {
            match ClientOptions::new().open(name) {
                Ok(pipe) => break pipe,
                Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => return PipeProbe::NotRunning,
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    if busy == BUSY_RETRIES {
                        return PipeProbe::Busy;
                    }
                    busy += 1;
                    tokio::time::sleep(BUSY_WAIT).await;
                }
                Err(error) => return PipeProbe::Failed(OpError::from(error).to_string()),
            }
        };
        match request_ping(Arc::new(DpapiProof), pipe).await {
            Ok(version) => PipeProbe::Answered { version },
            Err(error) if error.is_foreign_peer() => PipeProbe::Foreign,
            Err(error) => PipeProbe::Failed(error.to_string()),
        }
    }
}

/// The probes of this machine, for the command and for the window.
pub fn probes() -> DoctorProbes {
    let domain = std::env::var("USERDOMAIN").ok();
    let user = std::env::var("USERNAME").ok();
    let web: Arc<dyn Web> = match AnonymousWeb::new() {
        Ok(web) => Arc::new(web),
        Err(_) => Arc::new(NoNetwork),
    };
    DoctorProbes {
        ops: Arc::new(WindowsOps::new()),
        files: Arc::new(RealFiles),
        toasts: Arc::new(WindowsToasts),
        instance: Arc::new(PipeInstance {
            name: pipe_name(domain.as_deref(), user.as_deref()),
        }),
        web,
        webview_version: tauri::webview_version().ok(),
        app_data: std::env::var_os("APPDATA").map(PathBuf::from),
        local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        here: std::env::current_exe().ok(),
        slots: embedded_slots(),
    }
}

/// Without redirection, the standard output of a windowed exe has no handle: `GetStdHandle` gives none, which the
/// standard library returns as a null handle. A comparison, no `unsafe`.
fn stdout_redirected() -> bool {
    !std::io::stdout().as_raw_handle().is_null()
}

fn message(published: &Published) -> String {
    let summary = published.report.summary();
    match &published.report.saved {
        Saved::At(file) => format!(
            "Auto-diagnostic terminé : {summary}.\n\nRapport enregistré dans :\n{file}\n\nPour l'ouvrir, collez ce \
             chemin dans la barre d'adresse de l'Explorateur de fichiers."
        ),
        Saved::Failed(reason) => format!(
            "Auto-diagnostic terminé : {summary}, mais le rapport n'a pas pu être enregistré ({reason}).\n\nLancez \
             « mcp-proxy.exe doctor > rapport.txt » depuis un dossier inscriptible pour l'obtenir."
        ),
        Saved::No => format!("Auto-diagnostic terminé : {summary}."),
    }
}

/// `mcp-proxy.exe doctor`: never hands over to the running instance, never shows its window. Exit code: 0, 1 or 2
/// after the worst result.
pub fn start() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            dialog::error(
                TITLE,
                &format!("Auto-diagnostic impossible à démarrer ({}).", error.kind()),
            );
            return ExitCode::from(2);
        }
    };
    let probes = probes();
    let mut stdout = std::io::stdout();
    let published = runtime.block_on(run_command(
        &probes,
        Arc::new(SystemWallClock),
        &RealReportFiles,
        &mut stdout,
    ));
    // A check still blocked past its bound is left behind.
    runtime.shutdown_background();
    if !stdout_redirected() {
        dialog::report(TITLE, &message(&published), published.exit_code == 2);
    }
    ExitCode::from(published.exit_code)
}
