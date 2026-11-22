//! `mcp-proxy.exe stdio`: connects to the application's pipe (launching the application if needed), proves both
//! sides, then relays stdin and stdout. No secret and no MCP logic here.

use std::io::Write as _;
use std::os::windows::process::CommandExt as _;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_gateway::bridge::{pipe_name, prove_and_relay};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::time::Instant;

use crate::dpapi::DpapiProof;

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// How long the bridge waits for the pipe after launching the application.
const LAUNCH_WAIT: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_millis(100);

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
    let pipe = connect(&name).await?;
    prove_and_relay(Arc::new(DpapiProof), tokio::io::stdin(), tokio::io::stdout(), pipe)
        .await
        .map(|_| ())
        .map_err(|error| error.message())
}

/// The pipe, after launching the application once if it does not exist yet.
async fn connect(name: &str) -> Result<NamedPipeClient, String> {
    let deadline = Instant::now() + LAUNCH_WAIT;
    let mut launched = false;
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                if !launched {
                    launch_application()?;
                    launched = true;
                }
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {}
            Err(error) => {
                return Err(format!(
                    "MCP Proxy : canal de MCP Proxy inaccessible ({}).",
                    error.kind()
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "MCP Proxy : l'appli MCP Proxy n'a pas ouvert son canal dans les {} s. Lancez MCP Proxy depuis le \
                 menu Démarrer, puis réessayez.",
                LAUNCH_WAIT.as_secs()
            ));
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// The same exe without argument, detached from the LLM client: no console, no inherited handle, and out of the
/// client's job when the job allows it, so that closing the client does not close the application.
fn launch_application() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| {
        format!(
            "MCP Proxy : chemin de l'exe illisible ({}), appli non lancée.",
            e.kind()
        )
    })?;
    let spawn = |flags: u32| {
        Command::new(&exe)
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
