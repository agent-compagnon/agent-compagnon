//! The installation window (`install.html`): what it shows, the answers it may send, and what each answer does in
//! each state. The page holds no decision: an answer that does not fit the current state is ignored.

use serde_json::{Map, Value, json};

use crate::install::paths::LOCATION_LABEL;
use crate::version::Version;

/// Its only command, granted by the capability `install`.
pub const INSTALL_COMMAND: &str = "answer_install";
/// The label of its window.
pub const INSTALL_WINDOW: &str = "install";

/// What the window offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposal {
    /// No installation. `autostart`: the box « Démarrer MCP Proxy avec Windows » checked at first (the `Run` value
    /// already names the installation path).
    Install {
        autostart: bool,
    },
    Update {
        installed: Version,
    },
}

/// What the page may send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// « Installer », with the box.
    Install { autostart: bool },
    /// « Mettre à jour ».
    Update,
    /// « Pas maintenant ».
    Later,
    /// « Continuer sans installer », after a failure.
    Continue,
    /// « Quitter », or the window closed.
    Quit,
}

/// What the application does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install {
        autostart: bool,
    },
    Update,
    /// This exe from here, without the installation (`--no-install`).
    RunHere,
    /// The installed exe.
    RunInstalled,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Proposed,
    Working,
    Failed,
    Finished,
}

/// An answer the page sent in another form than the expected ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedAnswer;

#[derive(Debug)]
pub struct InstallDialog {
    proposal: Proposal,
    own: Version,
    state: State,
}

impl InstallDialog {
    pub fn new(proposal: Proposal, own: Version) -> Self {
        Self {
            proposal,
            own,
            state: State::Proposed,
        }
    }

    pub fn proposal(&self) -> &Proposal {
        &self.proposal
    }

    /// The data of the page, frozen in `window.__MCP_PROXY_INSTALL__` before it runs.
    pub fn view(&self) -> Value {
        let (kind, installed, autostart) = match &self.proposal {
            Proposal::Install { autostart } => ("install", Value::Null, *autostart),
            Proposal::Update { installed } => ("update", Value::String(installed.to_string()), false),
        };
        json!({
            "kind": kind,
            "version": self.own.to_string(),
            "installedVersion": installed,
            "autostart": autostart,
            "location": LOCATION_LABEL,
            "command": INSTALL_COMMAND,
        })
    }

    /// The next action for `answer`, or `None` when it does not fit: a second click while working, « Mettre à jour »
    /// on an installation, anything after the end.
    pub fn answer(&mut self, answer: Answer) -> Option<Action> {
        let action = match (self.state, &self.proposal, answer) {
            (State::Proposed, Proposal::Install { .. }, Answer::Install { autostart }) => {
                self.state = State::Working;
                return Some(Action::Install { autostart });
            }
            (State::Proposed, Proposal::Update { .. }, Answer::Update) => {
                self.state = State::Working;
                return Some(Action::Update);
            }
            (State::Proposed, Proposal::Install { .. }, Answer::Later) | (State::Failed, _, Answer::Continue) => {
                Action::RunHere
            }
            (State::Proposed, Proposal::Update { .. }, Answer::Later) => Action::RunInstalled,
            (State::Proposed | State::Failed, _, Answer::Quit) => Action::Quit,
            _ => return None,
        };
        self.state = State::Finished;
        Some(action)
    }

    /// The action failed: the window shows why, and offers to continue from here or to quit.
    pub fn failed(&mut self) {
        self.state = State::Failed;
    }

    /// Closing the window is refused while the installation runs (it would stop halfway); otherwise it quits.
    pub fn may_close(&self) -> bool {
        self.state != State::Working
    }
}

/// `{ "choice": "install", "autostart": true }`, `{ "choice": "update" }`, `{ "choice": "later" }`,
/// `{ "choice": "continue" }` or `{ "choice": "quit" }`, nothing more.
pub fn parse_answer(answer: Option<&Value>) -> Result<Answer, MalformedAnswer> {
    let object: &Map<String, Value> = answer.and_then(Value::as_object).ok_or(MalformedAnswer)?;
    let only = |keys: &[&str]| object.keys().all(|k| keys.contains(&k.as_str()));
    let choice = object.get("choice").and_then(Value::as_str).ok_or(MalformedAnswer)?;
    let answer = match choice {
        "install" => {
            let autostart = object
                .get("autostart")
                .and_then(Value::as_bool)
                .ok_or(MalformedAnswer)?;
            if !only(&["choice", "autostart"]) {
                return Err(MalformedAnswer);
            }
            return Ok(Answer::Install { autostart });
        }
        "update" => Answer::Update,
        "later" => Answer::Later,
        "continue" => Answer::Continue,
        "quit" => Answer::Quit,
        _ => return Err(MalformedAnswer),
    };
    if only(&["choice"]) {
        Ok(answer)
    } else {
        Err(MalformedAnswer)
    }
}
