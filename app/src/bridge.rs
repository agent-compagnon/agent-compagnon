//! `mcp-proxy.exe stdio`: the resilient relay of `gateway::bridge` (spec 026) wired to the application's named pipe,
//! to DPAPI, to the standard streams and to the launch of the application. No secret and no MCP logic here.

use std::io::Write as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;

use mcp_proxy_deploy::install::{InstallPaths, NO_INSTALL_ARG};
use mcp_proxy_gateway::bridge::{
    AppLauncher, ConnectFailure, Connector, RelaunchTarget, RelayDeps, ResilientRelay, pipe_name, relaunch_target,
    stopped_marker,
};
use mcp_proxy_gateway::runtime::DataDirs;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use crate::dpapi::DpapiProof;

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// Standard error belongs to the LLM client, which shows it: one line, French, no secret.
pub fn write_stderr(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

pub fn run() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            write_stderr(&format!("MCP Proxy : démarrage du pont impossible ({}).", error.kind()));
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(bridge());
    // A read of stdin may still be pending on the blocking pool: do not wait for it.
    runtime.shutdown_background();
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            write_stderr(&message);
            ExitCode::FAILURE
        }
    }
}

async fn bridge() -> Result<(), String> {
    let domain = std::env::var("USERDOMAIN").ok();
    let user = std::env::var("USERNAME").ok();
    let name = pipe_name(domain.as_deref(), user.as_deref()).ok_or_else(|| {
        "MCP Proxy : nom de l'utilisateur Windows introuvable (variable USERNAME) : canal de MCP Proxy inconnu."
            .to_owned()
    })?;
    let relay = ResilientRelay::new(RelayDeps {
        proof: Arc::new(DpapiProof),
        input: tokio::io::stdin(),
        output: tokio::io::stdout(),
        connector: Pipe { name },
        launcher: Arc::new(Launcher::from_environment()),
        diagnostics: Arc::new(write_stderr),
    });
    relay.run().await.map_err(|stop| stop.message())
}

/// The application's pipe, by its name.
struct Pipe {
    name: String,
}

impl Connector for Pipe {
    type Pipe = NamedPipeClient;

    async fn connect(&mut self) -> Result<NamedPipeClient, ConnectFailure> {
        ClientOptions::new()
            .open(&self.name)
            .map_err(|error| match error.raw_os_error() {
                Some(ERROR_FILE_NOT_FOUND) => ConnectFailure::Absent,
                Some(ERROR_PIPE_BUSY) => ConnectFailure::Busy,
                _ => ConnectFailure::Other(error.kind()),
            })
    }
}

/// Paths read once, at the start of the bridge: the system then gives this exe's path as it was started, before an
/// update may rename it.
struct Launcher {
    running: Option<PathBuf>,
    install: Option<InstallPaths>,
    marker: Option<PathBuf>,
}

impl Launcher {
    fn from_environment() -> Self {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        // Only the exe's folder is read here: without `APPDATA`, the shortcut's path is not needed.
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
        Self {
            running: std::env::current_exe().ok(),
            install: local
                .as_deref()
                .map(|local| InstallPaths::new(local, roaming.as_deref().unwrap_or(local))),
            marker: local
                .as_deref()
                .map(|local| stopped_marker(&DataDirs::under(local).root)),
        }
    }
}

impl AppLauncher for Launcher {
    fn target(&self) -> RelaunchTarget {
        let stopped = self.marker.as_deref().is_some_and(Path::exists);
        relaunch_target(
            self.running.as_deref(),
            self.install.as_ref(),
            &|path| path.exists(),
            stopped,
        )
    }

    /// Detached from the LLM client: no console, no inherited handle, and out of the client's job when the job allows
    /// it, so that closing the client does not close the application. With `--no-install`: a launch by an LLM client
    /// never shows the installation dialog (spec 019).
    fn launch(&self, exe: &Path) -> Result<(), String> {
        let spawn = |flags: u32| {
            Command::new(exe)
                .arg(NO_INSTALL_ARG)
                .creation_flags(flags)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        };
        let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        spawn(detached | CREATE_BREAKAWAY_FROM_JOB)
            .or_else(|_| spawn(detached))
            .map(|_| ())
            .map_err(|e| format!("MCP Proxy : lancement de l'appli impossible ({}).", e.kind()))
    }
}
