//! `SystemOps` for Windows (specs 019 and 022): the values of `HKCU` by `winreg` (`Run`, the entry « Applications
//! installées », the AUMID, the exclusion from error reports), the shortcut by `mslnk` (a pure Rust writer of `.lnk`
//! files, without COM), the running instance through the control channel of the pipe, processes by `std::process`,
//! the deletion of the running exe by `self_replace`. The files go through the default `std::fs` bodies of the
//! trait. What these operations decide is in `mcp_proxy_deploy`; nothing here elevates: everything is under `HKCU`
//! and `%LOCALAPPDATA%`. Every key and value has a fixed name: no operation takes a key from its caller.

use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_deploy::install::{EXE_NAME, RUN_KEY, RUN_VALUE};
use mcp_proxy_deploy::ops::{OpError, Removal, Shortcut, Stopped, SystemOps};
use mcp_proxy_deploy::uninstall::{AUMID_KEY, UNINSTALL_KEY, UninstallEntry, WER_KEY};
use mcp_proxy_gateway::bridge::{StopError, pipe_name, request_stop};
use tokio::net::windows::named_pipe::ClientOptions;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WRITE};

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

/// A key of `HKCU`, `None` when it does not exist.
fn open(path: &str, access: u32) -> Result<Option<RegKey>, OpError> {
    match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(path, access) {
        Ok(key) => Ok(Some(key)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// A value, `None` when it does not exist.
fn value<T: winreg::types::FromRegValue>(key: &RegKey, name: &str) -> Result<Option<T>, OpError> {
    match key.get_value::<T, _>(name) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Removes `path`, a key of `HKCU` that holds values only: one with subkeys was not written by MCP Proxy.
fn remove_key_of_values(path: &str) -> Result<Removal, OpError> {
    let Some(key) = open(path, KEY_READ)? else {
        return Ok(Removal::Absent);
    };
    if key.query_info()?.sub_keys > 0 {
        return Ok(Removal::NotOurs);
    }
    drop(key);
    let (parent, name) = path
        .rsplit_once('\\')
        .ok_or(OpError::Refused("clé du registre mal nommée"))?;
    let Some(parent) = open(parent, KEY_WRITE)? else {
        return Ok(Removal::Absent);
    };
    match parent.delete_subkey(name) {
        Ok(()) => Ok(Removal::Removed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Removal::Absent),
        Err(error) => Err(error.into()),
    }
}

const ENTRY_NAME: &str = "DisplayName";
const ENTRY_VERSION: &str = "DisplayVersion";
const ENTRY_PUBLISHER: &str = "Publisher";
const ENTRY_LOCATION: &str = "InstallLocation";
const ENTRY_ICON: &str = "DisplayIcon";
const ENTRY_UNINSTALL: &str = "UninstallString";
const ENTRY_NO_MODIFY: &str = "NoModify";
const ENTRY_NO_REPAIR: &str = "NoRepair";
const ENTRY_SIZE: &str = "EstimatedSize";

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

    fn uninstall_entry(&self) -> Result<Option<UninstallEntry>, OpError> {
        let Some(key) = open(UNINSTALL_KEY, KEY_QUERY_VALUE)? else {
            return Ok(None);
        };
        let text = |name: &str| -> Result<String, OpError> { Ok(value::<String>(&key, name)?.unwrap_or_default()) };
        let flag = |name: &str| -> Result<bool, OpError> { Ok(value::<u32>(&key, name)? == Some(1)) };
        Ok(Some(UninstallEntry {
            display_name: text(ENTRY_NAME)?,
            display_version: value::<String>(&key, ENTRY_VERSION)?,
            publisher: text(ENTRY_PUBLISHER)?,
            install_location: text(ENTRY_LOCATION)?,
            display_icon: text(ENTRY_ICON)?,
            uninstall_string: text(ENTRY_UNINSTALL)?,
            no_modify: flag(ENTRY_NO_MODIFY)?,
            no_repair: flag(ENTRY_NO_REPAIR)?,
            estimated_size: value::<u32>(&key, ENTRY_SIZE)?.unwrap_or_default(),
        }))
    }

    fn set_uninstall_entry(&self, entry: &UninstallEntry) -> Result<(), OpError> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(UNINSTALL_KEY, KEY_SET_VALUE | KEY_QUERY_VALUE)?;
        key.set_value(ENTRY_NAME, &entry.display_name)?;
        match &entry.display_version {
            Some(version) => key.set_value(ENTRY_VERSION, version)?,
            None => match key.delete_value(ENTRY_VERSION) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            },
        }
        key.set_value(ENTRY_PUBLISHER, &entry.publisher)?;
        key.set_value(ENTRY_LOCATION, &entry.install_location)?;
        key.set_value(ENTRY_ICON, &entry.display_icon)?;
        key.set_value(ENTRY_UNINSTALL, &entry.uninstall_string)?;
        key.set_value(ENTRY_NO_MODIFY, &u32::from(entry.no_modify))?;
        key.set_value(ENTRY_NO_REPAIR, &u32::from(entry.no_repair))?;
        key.set_value(ENTRY_SIZE, &entry.estimated_size)?;
        Ok(())
    }

    fn remove_uninstall_entry(&self) -> Result<Removal, OpError> {
        remove_key_of_values(UNINSTALL_KEY)
    }

    fn remove_aumid(&self) -> Result<Removal, OpError> {
        remove_key_of_values(AUMID_KEY)
    }

    fn aumid_registered(&self) -> Result<bool, OpError> {
        Ok(open(AUMID_KEY, KEY_QUERY_VALUE)?.is_some())
    }

    fn wer_exclusion(&self) -> Result<Option<u32>, OpError> {
        let Some(key) = open(WER_KEY, KEY_QUERY_VALUE)? else {
            return Ok(None);
        };
        match key.get_value::<u32, _>(EXE_NAME) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            // Another type of value under that name.
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn remove_wer_exclusion(&self) -> Result<Removal, OpError> {
        let Some(key) = open(WER_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)? else {
            return Ok(Removal::Absent);
        };
        match key.get_value::<u32, _>(EXE_NAME) {
            Ok(1) => {}
            Ok(_) => return Ok(Removal::NotOurs),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Removal::Absent),
            // Another type of value under that name.
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => return Ok(Removal::NotOurs),
            Err(error) => return Err(error.into()),
        }
        match key.delete_value(EXE_NAME) {
            Ok(()) => Ok(Removal::Removed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Removal::Absent),
            Err(error) => Err(error.into()),
        }
    }

    /// `self_replace` renames the exe into `%TEMP%` when it can (else it stays where it is), copies it there under a
    /// name its own start-up code recognises, and starts that copy, which waits for this process to end, deletes
    /// the exe, then goes itself (`FILE_FLAG_DELETE_ON_CLOSE`). Read in its sources (1.5.1), I022.4.
    fn delete_running_exe(&self, exe: &Path) -> Result<(), OpError> {
        Ok(self_replace::self_delete_at(exe)?)
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
