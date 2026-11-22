//! `SystemOps` for Windows (spec 019): the `Run` value by `winreg`, the shortcut by `mslnk` (a pure Rust writer of
//! `.lnk` files, without COM), the running instance through the control channel of the pipe, processes by
//! `std::process`. The files go through the default `std::fs` bodies of the trait. What these operations decide is
//! in `mcp_proxy_deploy`; nothing here elevates: everything is under `HKCU` and `%LOCALAPPDATA%`.

use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_deploy::install::{RUN_KEY, RUN_VALUE};
use mcp_proxy_deploy::ops::{OpError, Shortcut, Stopped, SystemOps};
use mcp_proxy_gateway::bridge::{StopError, pipe_name, request_stop};
use tokio::net::windows::named_pipe::ClientOptions;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};

use crate::dpapi::DpapiProof;

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// How long a busy pipe is retried before the instance is said not to answer.
const BUSY_RETRIES: u32 = 50;
const BUSY_WAIT: Duration = Duration::from_millis(100);

pub struct WindowsOps {
    /// `None` without a user name: no instance can run then (spec 011).
    pipe: Option<String>,
}

impl WindowsOps {
    pub fn new() -> Self {
        let domain = std::env::var("USERDOMAIN").ok();
        let user = std::env::var("USERNAME").ok();
        Self {
            pipe: pipe_name(domain.as_deref(), user.as_deref()),
        }
    }
}

// The text of `StopError::StillRunning` below names this bound.
const _: () = assert!(mcp_proxy_gateway::bridge::STOP_TIMEOUT.as_secs() == 20);

fn stop_failure(error: &StopError) -> OpError {
    OpError::Refused(match error {
        StopError::Proof(_) => "son canal n'a pas prouvé son identité",
        StopError::Refused => "demande d'arrêt refusée",
        StopError::StillRunning => "toujours ouvert après 20 s",
        StopError::Closed => "canal fermé avant sa réponse",
    })
}

/// A path `mslnk` can write without failing on its own assumptions: Unicode, starting with a drive letter.
fn shortcut_target(target: &Path) -> Option<&str> {
    let text = target.to_str()?;
    let mut chars = text.chars();
    let drive = matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(letter), Some(':'), Some('\\')) if letter.is_ascii_alphabetic()
    );
    (drive && text.len() > 3).then_some(text)
}

fn run_key(access: u32) -> Result<RegKey, OpError> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags(RUN_KEY, access)?;
    Ok(key)
}

impl SystemOps for WindowsOps {
    fn write_shortcut(&self, shortcut: &Shortcut) -> Result<(), OpError> {
        let target = shortcut_target(&shortcut.target).ok_or(OpError::Refused(
            "chemin de l'exe impossible à inscrire dans un raccourci",
        ))?;
        let name = shortcut
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(OpError::Refused("nom du raccourci illisible"))?;
        let temporary = shortcut.path.with_file_name(format!("{name}.tmp"));
        // `mslnk` relies on its own checks of the path: a panic there must not stop the installation halfway.
        let written =
            std::panic::catch_unwind(|| mslnk::ShellLink::new(target).and_then(|link| link.create_lnk(&temporary)));
        match written {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(OpError::Refused("raccourci impossible à écrire"));
            }
        }
        std::fs::rename(&temporary, &shortcut.path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            OpError::from(error)
        })
    }

    fn run_entry(&self) -> Result<Option<String>, OpError> {
        let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        match key.get_value::<String, _>(RUN_VALUE) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn set_run_entry(&self, command: &str) -> Result<(), OpError> {
        Ok(run_key(KEY_SET_VALUE)?.set_value(RUN_VALUE, &command)?)
    }

    fn remove_run_entry(&self) -> Result<(), OpError> {
        let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        match key.delete_value(RUN_VALUE) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// Called on a blocking thread: the pipe is driven by the runtime of the application.
    fn stop_instance(&self) -> Result<Stopped, OpError> {
        let Some(name) = self.pipe.clone() else {
            return Ok(Stopped::NotRunning);
        };
        tauri::async_runtime::block_on(async move {
            let mut busy = 0;
            let pipe = loop {
                match ClientOptions::new().open(&name) {
                    Ok(pipe) => break pipe,
                    Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                        return Ok(Stopped::NotRunning);
                    }
                    Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && busy < BUSY_RETRIES => {
                        busy += 1;
                        tokio::time::sleep(BUSY_WAIT).await;
                    }
                    Err(error) => return Err(OpError::from(error)),
                }
            };
            request_stop(Arc::new(DpapiProof), pipe)
                .await
                .map(|()| Stopped::Stopped)
                .map_err(|error| {
                    tracing::warn!(%error, "arrêt de l'instance en cours refusé ou trop long");
                    stop_failure(&error)
                })
        })
    }

    /// Detached, no console, no inherited handle, in the exe's own folder (never the folder of the download).
    fn launch(&self, exe: &Path, args: &[&str]) -> Result<(), OpError> {
        let mut command = Command::new(exe);
        command
            .args(args)
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        command.spawn().map(|_| ()).map_err(OpError::from)
    }
}
