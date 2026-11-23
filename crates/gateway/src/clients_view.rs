//! The dialog « Clients LLM » (spec 023): the state of each configuration file, the preview of a change, and the
//! change itself. Everything that decides is in `mcp_proxy_deploy::clients`; here, the forms the window receives
//! (camelCase, contract `ui/src/api/clients.example.json`) and the journal of what the user did.
//!
//! The window names a location and an action, never a path; the paths come from the [`Environment`] the
//! application resolved. What the window receives says nothing of the other servers of a file but their number, and
//! of an existing entry `mcp-proxy` but its command and the names of its fields. Nothing of a file goes to the
//! journal.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mcp_proxy_core::trace::rfc3339;
use mcp_proxy_deploy::clients::{
    self, Action, Applied, Change, Client, ClientError, ClientFiles, EntrySummary, Environment, Inspection, LocationId,
    Plan, State,
};
use serde::Serialize;

use crate::interface::InterfaceError;
use crate::runtime::Gateway;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientsView {
    /// The command each client is configured with (the installed exe); `None` when MCP Proxy is not installed, and
    /// nothing can be added.
    pub command: Option<String>,
    /// « Copier la configuration »: `{ "mcpServers": { "mcp-proxy": … } }`, for any other client.
    pub snippet: Option<String>,
    pub locations: Vec<LocationView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationView {
    /// What the window sends back: `claude-desktop`, `claude-desktop-msix`, `vscode`.
    pub id: &'static str,
    pub client: ClientKind,
    /// The file, from `%APPDATA%` or `%LOCALAPPDATA%`: no user name.
    pub file: &'static str,
    pub state: StateView,
    pub other_servers: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientKind {
    ClaudeDesktop,
    Vscode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StateKind {
    NotInstalled,
    NotConfigured,
    Configured,
    OtherCommand,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    pub kind: StateKind,
    pub file_exists: bool,
    /// `otherCommand`: the entry found.
    pub previous: Option<EntryView>,
    /// `unreadable`: why, without content.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub command: Option<String>,
    /// The names of its other fields; never their values (an `env` may hold a token).
    pub other_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Nothing,
    Create,
    Add,
    Replace,
    Remove,
    Manual,
}

/// What « Installer » or « Retirer » would do, shown before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewView {
    pub location: &'static str,
    pub action: &'static str,
    pub file: &'static str,
    pub change: ChangeKind,
    /// The entry written, `"mcp-proxy": { … }`.
    pub entry: Option<String>,
    /// The entry replaced or removed.
    pub previous: Option<EntryView>,
    /// `manual`: why nothing can be written.
    pub reason: Option<String>,
    /// `manual`, to install: what to paste.
    pub snippet: Option<String>,
    /// A copy of the file is made first.
    pub backup: bool,
    pub other_servers: usize,
    /// Sent back with the action: the file must not have changed since.
    pub fingerprint: String,
}

/// How an applied action ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ApplyOutcome {
    /// Written; the backup is a file name, in the folder of the file.
    Applied { backup: Option<String> },
    /// Already as asked: nothing written.
    Nothing,
    /// The file is no longer what the preview showed: nothing written, the window shows a new preview.
    Changed,
}

fn entry_view(summary: &EntrySummary) -> EntryView {
    EntryView {
        command: summary.command.clone(),
        other_keys: summary.other_keys.clone(),
    }
}

fn location_view(inspection: &Inspection) -> LocationView {
    let id = inspection.location;
    let state = |kind| StateView {
        kind,
        file_exists: false,
        previous: None,
        reason: None,
    };
    let state = match &inspection.state {
        State::NotInstalled => state(StateKind::NotInstalled),
        State::NotConfigured { file_exists } => StateView {
            file_exists: *file_exists,
            ..state(StateKind::NotConfigured)
        },
        State::Configured => StateView {
            file_exists: true,
            ..state(StateKind::Configured)
        },
        State::OtherCommand(summary) => StateView {
            file_exists: true,
            previous: Some(entry_view(summary)),
            ..state(StateKind::OtherCommand)
        },
        State::Unreadable(reason) => StateView {
            file_exists: true,
            reason: Some(reason.to_string()),
            ..state(StateKind::Unreadable)
        },
    };
    LocationView {
        id: id.as_str(),
        client: match id.client() {
            Client::ClaudeDesktop => ClientKind::ClaudeDesktop,
            Client::VsCode => ClientKind::Vscode,
        },
        file: id.file_label(),
        state,
        other_servers: inspection.other_servers,
    }
}

/// The view of the dialog, from the inspections and the installed exe (`None` when it is not there).
pub fn view_of(inspections: &[Inspection], target: Option<&Path>) -> ClientsView {
    ClientsView {
        command: target.map(|exe| exe.display().to_string()),
        snippet: target.map(clients::generic_snippet),
        locations: inspections.iter().map(location_view).collect(),
    }
}

/// The view of the dialog now.
pub fn clients_view(files: &dyn ClientFiles, env: &Environment) -> ClientsView {
    let target = env.target(files);
    view_of(&clients::inspect_all(files, env, target.as_deref()), target.as_deref())
}

pub fn preview_view(plan: &Plan) -> PreviewView {
    let mut view = PreviewView {
        location: plan.location.as_str(),
        action: plan.action.as_str(),
        file: plan.location.file_label(),
        change: ChangeKind::Nothing,
        entry: None,
        previous: None,
        reason: None,
        snippet: None,
        backup: plan.makes_backup(),
        other_servers: plan.other_servers,
        fingerprint: plan.fingerprint.clone(),
    };
    match &plan.change {
        Change::Nothing => {}
        Change::Create { entry } => {
            view.change = ChangeKind::Create;
            view.entry = Some(entry.clone());
        }
        Change::Add { entry } => {
            view.change = ChangeKind::Add;
            view.entry = Some(entry.clone());
        }
        Change::Replace { previous, entry } => {
            view.change = ChangeKind::Replace;
            view.entry = Some(entry.clone());
            view.previous = Some(entry_view(previous));
        }
        Change::Remove { previous } => {
            view.change = ChangeKind::Remove;
            view.previous = Some(entry_view(previous));
        }
        Change::Manual { reason, snippet } => {
            view.change = ChangeKind::Manual;
            view.reason = Some(reason.to_string());
            view.snippet = snippet.clone();
        }
    }
    view
}

/// `20261122-140307` (UTC), for the name of a backup.
pub fn backup_stamp(at: std::time::SystemTime) -> String {
    let digits: String = rfc3339(at).chars().take(19).filter(char::is_ascii_digit).collect();
    let (date, time) = digits.split_at(digits.len().min(8));
    format!("{date}-{time}")
}

/// The error as the window shows it, the client named once.
fn described(id: LocationId, error: &ClientError) -> String {
    match error {
        ClientError::NotInstalled(_) => error.to_string(),
        _ => format!("{} : {error}", id.label()),
    }
}

/// The paths the application resolved, and the files they lead to.
#[derive(Clone)]
pub struct ClientsAccess {
    pub env: Environment,
    pub files: Arc<dyn ClientFiles>,
}

impl ClientsAccess {
    /// From `%APPDATA%` and `%LOCALAPPDATA%`; `None` when either is missing.
    pub fn new(app_data: Option<PathBuf>, local_app_data: Option<PathBuf>) -> Option<Self> {
        Some(Self {
            env: Environment {
                app_data: app_data?,
                local_app_data: local_app_data?,
            },
            files: Arc::new(clients::RealFiles),
        })
    }
}

impl Gateway {
    /// The location the window named.
    pub fn known_location(&self, raw: &str) -> Result<LocationId, InterfaceError> {
        LocationId::parse(raw).ok_or(InterfaceError::UnknownClient)
    }

    pub fn known_client_action(&self, raw: &str) -> Result<Action, InterfaceError> {
        Action::parse(raw).ok_or(InterfaceError::UnknownAction)
    }

    /// The dialog « Clients LLM »: reads, writes nothing.
    pub async fn llm_clients(&self, access: &ClientsAccess) -> Result<ClientsView, InterfaceError> {
        let access = access.clone();
        tokio::task::spawn_blocking(move || clients_view(access.files.as_ref(), &access.env))
            .await
            .map_err(|_| InterfaceError::Clients("lecture des clients LLM interrompue".to_owned()))
    }

    /// The preview of « Installer » or « Retirer »: reads, writes nothing.
    pub async fn preview_llm_client(
        &self,
        access: &ClientsAccess,
        id: LocationId,
        action: Action,
    ) -> Result<PreviewView, InterfaceError> {
        let access = access.clone();
        let planned = tokio::task::spawn_blocking(move || {
            let target = access.env.target(access.files.as_ref());
            clients::plan(access.files.as_ref(), &access.env, id, action, target.as_deref()).map(|p| preview_view(&p))
        })
        .await
        .map_err(|_| InterfaceError::Clients("aperçu interrompu".to_owned()))?;
        planned.map_err(|error| InterfaceError::Clients(described(id, &error)))
    }

    /// « Installer » or « Retirer » once the user saw the preview whose `fingerprint` the window sends back.
    pub async fn apply_llm_client(
        &self,
        access: &ClientsAccess,
        id: LocationId,
        action: Action,
        fingerprint: String,
    ) -> Result<ApplyOutcome, InterfaceError> {
        let access = access.clone();
        let stamp = backup_stamp(self.clock().now());
        let applied: Result<Applied, ClientError> = tokio::task::spawn_blocking(move || {
            let target = access.env.target(access.files.as_ref());
            clients::apply(
                access.files.as_ref(),
                &access.env,
                id,
                action,
                target.as_deref(),
                &fingerprint,
                &stamp,
            )
        })
        .await
        .map_err(|_| InterfaceError::Clients("écriture interrompue".to_owned()))?;
        let label = id.label();
        match applied {
            Ok(Applied {
                written: true, backup, ..
            }) => {
                let text = match action {
                    Action::Install => format!("MCP Proxy ajouté à la configuration de {label} par l'utilisateur"),
                    Action::Remove => format!("MCP Proxy retiré de la configuration de {label} par l'utilisateur"),
                };
                self.journal().record_event(tracing::Level::INFO, &text);
                Ok(ApplyOutcome::Applied { backup })
            }
            Ok(_) => Ok(ApplyOutcome::Nothing),
            Err(ClientError::Changed) => {
                self.journal().record_event(
                    tracing::Level::INFO,
                    &format!("configuration de {label} changée depuis l'aperçu : rien n'a été écrit"),
                );
                Ok(ApplyOutcome::Changed)
            }
            Err(error) => {
                self.journal().record_event(
                    tracing::Level::WARN,
                    &format!("configuration de {label} non modifiée : {error}"),
                );
                Err(InterfaceError::Clients(described(id, &error)))
            }
        }
    }
}
