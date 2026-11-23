//! The uninstallation window (`uninstall.html`): what it shows, the answers it may send, and the report it gets
//! back. The page decides nothing: an answer that does not fit the current state is ignored.

use serde_json::{Map, Value, json};

use crate::clients::LocationId;
use crate::install::LOCATION_LABEL;
use crate::uninstall::plan::{ClientDecision, Item};
use crate::uninstall::run::{Done, Outcome, Report};
use crate::version::Version;

/// Its only command, granted by the capability `uninstall`.
pub const UNINSTALL_COMMAND: &str = "answer_uninstall";
/// The label of its window.
pub const UNINSTALL_WINDOW: &str = "uninstall";
/// The data folder as the user reads it.
pub const DATA_LABEL: &str = r"%LOCALAPPDATA%\MCP Proxy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// « Désinstaller », with the box « Retirer MCP Proxy des clients LLM ».
    Uninstall { remove_clients: bool },
    /// « Annuler », or the window closed before anything was done.
    Cancel,
    /// « Fermer », or the window closed, once the report is shown.
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Uninstall { remove_clients: bool },
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Proposed,
    Working,
    /// Nothing was changed: the user may try again.
    Failed,
    /// The report is shown.
    Reported,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedAnswer;

#[derive(Debug)]
pub struct UninstallDialog {
    version: Version,
    /// The entry of each client, as it would be treated with the box ticked.
    clients: Vec<(LocationId, ClientDecision)>,
    state: State,
    reported: bool,
}

fn client_view(location: LocationId, decision: &ClientDecision) -> Value {
    let (entry, detail) = match decision {
        ClientDecision::Remove | ClientDecision::Asked => ("ours", Value::Null),
        ClientDecision::OtherExe(command) => ("other", command.clone().map_or(Value::Null, Value::String)),
        ClientDecision::Unreadable(reason) => ("unreadable", Value::String(reason.to_string())),
    };
    json!({
        "location": location.as_str(),
        "label": location.label(),
        "file": location.file_label(),
        "entry": entry,
        "detail": detail,
    })
}

impl UninstallDialog {
    /// `plan` is the plan with the box ticked: its client items say what each entry is.
    pub fn new(version: Version, plan: &[Item]) -> Self {
        let clients = plan
            .iter()
            .filter_map(|item| match item {
                Item::Client { location, decision } => Some((*location, decision.clone())),
                _ => None,
            })
            .collect();
        Self {
            version,
            clients,
            state: State::Proposed,
            reported: false,
        }
    }

    /// The data of the page, frozen in `window.__MCP_PROXY_UNINSTALL__` before it runs. Paths only as the user
    /// reads them, from `%LOCALAPPDATA%` or `%APPDATA%`.
    pub fn view(&self) -> Value {
        json!({
            "version": self.version.to_string(),
            "location": LOCATION_LABEL,
            "data": DATA_LABEL,
            "clients": self.clients.iter().map(|(l, d)| client_view(*l, d)).collect::<Vec<_>>(),
            "command": UNINSTALL_COMMAND,
        })
    }

    pub fn answer(&mut self, answer: Answer) -> Option<Action> {
        match (self.state, answer) {
            (State::Proposed | State::Failed, Answer::Uninstall { remove_clients }) => {
                self.state = State::Working;
                Some(Action::Uninstall { remove_clients })
            }
            (State::Proposed | State::Failed, Answer::Cancel) | (State::Reported | State::Failed, Answer::Close) => {
                self.state = State::Finished;
                Some(Action::Quit)
            }
            _ => None,
        }
    }

    /// Nothing could be removed (the running instance did not stop…): the window says why and offers to try again.
    pub fn failed(&mut self) {
        self.state = State::Failed;
    }

    /// The removal ran: the window shows its report.
    pub fn reported(&mut self) {
        self.state = State::Reported;
        self.reported = true;
    }

    /// The removal ran during this process: what goes once the window closes is due.
    pub fn removal_ran(&self) -> bool {
        self.reported
    }

    /// Closing the window is refused while the removal runs.
    pub fn may_close(&self) -> bool {
        self.state != State::Working
    }

    /// What closing the window means now.
    pub fn close_answer(&self) -> Answer {
        if self.reported { Answer::Close } else { Answer::Cancel }
    }
}

/// `{ "choice": "uninstall", "removeClients": true }`, `{ "choice": "cancel" }` or `{ "choice": "close" }`, nothing
/// more.
pub fn parse_answer(answer: Option<&Value>) -> Result<Answer, MalformedAnswer> {
    let object: &Map<String, Value> = answer.and_then(Value::as_object).ok_or(MalformedAnswer)?;
    let only = |keys: &[&str]| object.keys().all(|k| keys.contains(&k.as_str()));
    match object.get("choice").and_then(Value::as_str).ok_or(MalformedAnswer)? {
        "uninstall" if only(&["choice", "removeClients"]) => {
            let remove_clients = object
                .get("removeClients")
                .and_then(Value::as_bool)
                .ok_or(MalformedAnswer)?;
            Ok(Answer::Uninstall { remove_clients })
        }
        "cancel" if only(&["choice"]) => Ok(Answer::Cancel),
        "close" if only(&["choice"]) => Ok(Answer::Close),
        _ => Err(MalformedAnswer),
    }
}

/// What an item is, as the report names it.
pub fn item_label(item: &Item) -> String {
    match item {
        Item::Client { location, .. } => format!("Entrée « mcp-proxy » de {}", location.label()),
        Item::Autostart => "Démarrage avec Windows".to_owned(),
        Item::Shortcut => "Raccourci « MCP Proxy » du menu Démarrer".to_owned(),
        Item::Aumid => "Inscription des notifications de MCP Proxy".to_owned(),
        Item::ErrorReports => "Exclusion des rapports d'erreur de Windows".to_owned(),
        Item::Data => format!("Journaux, réglages et autres données ({DATA_LABEL})"),
        Item::UninstallEntry => "Entrée « MCP Proxy » des applications installées".to_owned(),
        Item::Exe => "Programme mcp-proxy.exe".to_owned(),
        Item::InstallFolder => format!("Dossier d'installation ({LOCATION_LABEL})"),
        Item::WindowData => r"Données des fenêtres de MCP Proxy (%LOCALAPPDATA%\com.mcpproxy.app)".to_owned(),
    }
}

fn left_view(done: &Done) -> Option<Value> {
    let why = match &done.outcome {
        Outcome::Kept(why) => why.clone(),
        Outcome::Failed { cause, left } if left.is_empty() => format!("non retiré : {cause}"),
        Outcome::Failed { cause, left } => format!("non retiré ({cause}) ; reste : {}", left.join(" ; ")),
        // Said once, by `keptInstalled`.
        Outcome::Removed | Outcome::Absent | Outcome::AtClose | Outcome::KeptToRetry => return None,
    };
    Some(json!({
        "what": item_label(&done.item),
        "why": why,
        "failed": matches!(done.outcome, Outcome::Failed { .. }),
    }))
}

/// The report the command sends back: what went, what goes when the window closes, what stays and why. What was
/// already absent is not listed.
pub fn report_view(report: &Report) -> Value {
    let labels = |wanted: fn(&Outcome) -> bool| -> Vec<String> {
        report
            .items
            .iter()
            .filter(|d| wanted(&d.outcome))
            .map(|d| item_label(&d.item))
            .collect()
    };
    json!({
        "removed": labels(|o| *o == Outcome::Removed),
        "atClose": labels(|o| *o == Outcome::AtClose),
        "left": report.items.iter().filter_map(left_view).collect::<Vec<_>>(),
        "keptInstalled": report.kept_installed(),
    })
}

/// The message box after the window closed, when the data of the window could not be removed.
pub fn after_close_message(done: &Done) -> Option<String> {
    let view = left_view(done)?;
    let why = view["why"].as_str().unwrap_or_default();
    Some(format!(
        "La désinstallation est terminée, sauf : {} — {why}. Vous pouvez supprimer ce dossier à la main.",
        item_label(&done.item)
    ))
}
