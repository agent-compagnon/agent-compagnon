//! Configuration of the LLM clients (spec 023): MCP Proxy added to, or removed from, the configuration file of
//! Claude Desktop or VS Code, as the entry `mcp-proxy` that starts `<installed exe> stdio`.
//!
//! These files belong to other programs. Only the entry `mcp-proxy` is ever touched; the rest of the file stays
//! byte for byte ([`jsonc`]). A change is shown before it is written ([`plan`]), and what is written is what was
//! shown ([`apply`] refuses a file that changed since); a backup comes first, the write is atomic, and a file that
//! cannot be read safely is never written: the user gets the entry to paste instead.
//!
//! Every path is derived from the two folders the application reads in the environment ([`Environment`]); the
//! window only names a location ([`LocationId`]) and an action. One adapter per client ([`claude`], [`vscode`]): a
//! change of format at one editor is fixed there. Nothing here logs a path or a content.

mod claude;
mod files;
mod jsonc;
mod vscode;

#[cfg(test)]
mod tests;

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub use files::{ClientFiles, Kind, RealFiles};

use crate::install::{InstallPaths, same_location};
use crate::ops::OpError;

/// The name of our entry in every client.
pub const ENTRY_NAME: &str = "mcp-proxy";
/// The argument that makes the installed exe a bridge (spec 011).
pub const SERVE_ARG: &str = "stdio";
/// A configuration file larger than this is not one: it is not read.
pub const MAX_FILE_SIZE: u64 = 1024 * 1024;

/// The two folders of the user the application reads in its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// `%APPDATA%`
    pub app_data: PathBuf,
    /// `%LOCALAPPDATA%`
    pub local_app_data: PathBuf,
}

impl Environment {
    /// The exe every client is configured with: the installed one (spec 019), whoever runs.
    pub fn installed_exe(&self) -> PathBuf {
        InstallPaths::new(&self.local_app_data, &self.app_data).exe
    }

    /// The installed exe if it is there: without it, nothing is added to a client.
    pub fn target(&self, files: &dyn ClientFiles) -> Option<PathBuf> {
        let exe = self.installed_exe();
        matches!(files.kind(&exe), Ok(Kind::File)).then_some(exe)
    }

    pub fn location(&self, id: LocationId) -> Location {
        let (dir, file) = match id {
            LocationId::ClaudeDesktop => (claude::classic_dir(self), claude::FILE),
            LocationId::ClaudeDesktopPackage => (claude::package_dir(self), claude::FILE),
            LocationId::VsCode => (vscode::user_dir(self), vscode::FILE),
        };
        Location {
            id,
            file: dir.join(file),
            dir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    ClaudeDesktop,
    VsCode,
}

/// A configuration file of a client. Claude Desktop has two (see [`claude`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LocationId {
    /// `%APPDATA%\Claude\claude_desktop_config.json`
    ClaudeDesktop,
    /// The same file in the folder of the MSIX package.
    ClaudeDesktopPackage,
    /// `%APPDATA%\Code\User\mcp.json`
    VsCode,
}

impl LocationId {
    pub const ALL: [Self; 3] = [Self::ClaudeDesktop, Self::ClaudeDesktopPackage, Self::VsCode];

    /// The identifier the window sends.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "claude-desktop",
            Self::ClaudeDesktopPackage => "claude-desktop-msix",
            Self::VsCode => "vscode",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == raw)
    }

    pub fn client(self) -> Client {
        match self {
            Self::ClaudeDesktop | Self::ClaudeDesktopPackage => Client::ClaudeDesktop,
            Self::VsCode => Client::VsCode,
        }
    }

    /// For the journal and the messages: no path, no user name.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "Claude Desktop",
            Self::ClaudeDesktopPackage => "Claude Desktop (paquet MSIX)",
            Self::VsCode => "VS Code",
        }
    }

    /// The file as the user reads it, without their name.
    pub fn file_label(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => r"%APPDATA%\Claude\claude_desktop_config.json",
            Self::ClaudeDesktopPackage => {
                r"%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude_desktop_config.json"
            }
            Self::VsCode => r"%APPDATA%\Code\User\mcp.json",
        }
    }

    fn adapter(self) -> &'static dyn Adapter {
        match self.client() {
            Client::ClaudeDesktop => &claude::ClaudeDesktop,
            Client::VsCode => &vscode::VsCode,
        }
    }
}

/// What a client's format says: where the servers are, and how our entry is written.
trait Adapter: Sync {
    /// The key of the servers at the top of the file.
    fn section(&self) -> &'static str;
    /// Our entry, in the order its fields are written.
    fn fields(&self, exe: &Path) -> Vec<(&'static str, Value)>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub id: LocationId,
    /// The folder of the client: without it, the client is not installed and nothing is created.
    pub dir: PathBuf,
    pub file: PathBuf,
}

/// Why a file is not read, or not changed. French, without any path or content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    TooLarge,
    NotUtf8,
    /// A folder, a link…
    NotAFile,
    Io(OpError),
    Syntax {
        line: usize,
    },
    NotAnObject,
    OnlyComments,
    SectionNotAnObject(&'static str),
    Duplicate(&'static str),
    /// The change could not be proven to touch our entry only.
    Unsafe,
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("fichier trop volumineux pour une configuration (plus de 1 Mio)"),
            Self::NotUtf8 => f.write_str("fichier qui n'est pas du texte UTF-8"),
            Self::NotAFile => f.write_str("l'emplacement n'est pas un simple fichier (dossier ou lien)"),
            Self::Io(error) => write!(f, "lecture impossible : {error}"),
            Self::Syntax { line } => write!(f, "JSON invalide à la ligne {line}"),
            Self::NotAnObject => f.write_str("le fichier ne contient pas un objet JSON"),
            Self::OnlyComments => f.write_str("le fichier ne contient que des commentaires"),
            Self::SectionNotAnObject(key) => write!(f, "« {key} » n'est pas un objet"),
            Self::Duplicate(key) => write!(f, "la clé « {key} » apparaît plusieurs fois"),
            Self::Unsafe => f.write_str("la modification n'a pas pu être vérifiée"),
        }
    }
}

/// An entry `mcp-proxy` that is not ours as we would write it, as the preview shows it: its command, and the names
/// of its other fields. Their values are not shown: an `env` may hold a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySummary {
    pub command: Option<String>,
    pub other_keys: Vec<String>,
}

/// The state of a location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Its folder does not exist.
    NotInstalled,
    NotConfigured {
        file_exists: bool,
    },
    /// Our entry, with the installed exe.
    Configured,
    /// An entry `mcp-proxy` with another command, other arguments or other fields.
    OtherCommand(EntrySummary),
    Unreadable(Unreadable),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub location: LocationId,
    pub state: State,
    /// The other servers of the file.
    pub other_servers: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Remove,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Remove => "remove",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [Self::Install, Self::Remove].into_iter().find(|a| a.as_str() == raw)
    }
}

/// What an action would do, as the preview shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Already as asked: nothing is written.
    Nothing,
    /// The file does not exist: it is created with our entry alone. `entry` is `"mcp-proxy": { … }`.
    Create {
        entry: String,
    },
    Add {
        entry: String,
    },
    Replace {
        previous: EntrySummary,
        entry: String,
    },
    Remove {
        previous: EntrySummary,
    },
    /// The file cannot be changed safely: nothing is written. To install, the snippet to paste.
    Manual {
        reason: Unreadable,
        snippet: Option<String>,
    },
}

/// Why an action was refused. French, without any path or content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The folder of the client does not exist: nothing is created.
    NotInstalled(LocationId),
    /// MCP Proxy is not installed: there is no exe to point at.
    NoTarget,
    /// The file is not what the preview showed.
    Changed,
    Backup(OpError),
    Io(OpError),
    Refused(&'static str),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled(id) => write!(f, "{} n'est pas installé sur ce poste : rien n'est créé", id.label()),
            Self::NoTarget => f.write_str(
                "MCP Proxy n'est pas installé : les clients LLM sont configurés avec l'exe installé, qui n'existe pas encore",
            ),
            Self::Changed => f.write_str("le fichier a changé depuis l'aperçu : rien n'a été écrit"),
            Self::Backup(error) => write!(f, "sauvegarde impossible, rien n'a été écrit : {error}"),
            Self::Io(error) => write!(f, "écriture impossible, le fichier est inchangé : {error}"),
            Self::Refused(text) => f.write_str(text),
        }
    }
}

/// What the file holds: absent, unreadable, or its bytes and text.
enum Content {
    Absent,
    Text { bytes: Vec<u8>, text: String },
}

fn read(files: &dyn ClientFiles, location: &Location) -> Result<Option<Content>, Unreadable> {
    if !matches!(files.kind(&location.dir), Ok(Kind::Dir)) {
        return Ok(None);
    }
    match files.kind(&location.file).map_err(|e| Unreadable::Io(e.into()))? {
        Kind::Missing => return Ok(Some(Content::Absent)),
        Kind::File => {}
        Kind::Dir | Kind::Other => return Err(Unreadable::NotAFile),
    }
    let bytes = files.read(&location.file).map_err(|e| Unreadable::Io(e.into()))?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(Unreadable::TooLarge);
    }
    let text = String::from_utf8(bytes.clone()).map_err(|_| Unreadable::NotUtf8)?;
    Ok(Some(Content::Text { bytes, text }))
}

fn summary(entry: &Value) -> EntrySummary {
    let Value::Object(map) = entry else {
        return EntrySummary {
            command: None,
            other_keys: Vec::new(),
        };
    };
    EntrySummary {
        command: map.get("command").and_then(Value::as_str).map(str::to_owned),
        other_keys: map.keys().filter(|k| *k != "command").cloned().collect(),
    }
}

/// Our entry as we would write it with `exe`, the command compared as Windows compares paths.
fn is_ours(entry: &Value, expected: &[(&'static str, Value)]) -> bool {
    let Value::Object(found) = entry else { return false };
    found.len() == expected.len()
        && expected.iter().all(|(key, want)| match (*key, found.get(*key)) {
            ("command", Some(Value::String(got))) => want
                .as_str()
                .is_some_and(|want| same_location(Path::new(got), Path::new(want))),
            (_, Some(got)) => got == want,
            (_, None) => false,
        })
}

/// The state of one location. `target`: the installed exe, `None` when MCP Proxy is not installed (an entry is then
/// never ours).
pub fn inspect(files: &dyn ClientFiles, env: &Environment, id: LocationId, target: Option<&Path>) -> Inspection {
    let location = env.location(id);
    let adapter = id.adapter();
    let state_of = |content: Content| -> Result<(State, usize), Unreadable> {
        let Content::Text { text, .. } = content else {
            return Ok((State::NotConfigured { file_exists: false }, 0));
        };
        let found = jsonc::find(&text, adapter.section(), ENTRY_NAME)?;
        let state = match &found.entry {
            None => State::NotConfigured { file_exists: true },
            Some(entry) if target.is_some_and(|exe| is_ours(entry, &adapter.fields(exe))) => State::Configured,
            Some(entry) => State::OtherCommand(summary(entry)),
        };
        Ok((state, found.others))
    };
    let (state, other_servers) = match read(files, &location) {
        Ok(None) => (State::NotInstalled, 0),
        Ok(Some(content)) => state_of(content).unwrap_or_else(|reason| (State::Unreadable(reason), 0)),
        Err(reason) => (State::Unreadable(reason), 0),
    };
    Inspection {
        location: id,
        state,
        other_servers,
    }
}

/// Every location, in the order of the window.
pub fn inspect_all(files: &dyn ClientFiles, env: &Environment, target: Option<&Path>) -> Vec<Inspection> {
    LocationId::ALL
        .into_iter()
        .map(|id| inspect(files, env, id, target))
        .collect()
}

/// An action as computed for the preview, and the proof of what it was computed from.
pub struct Plan {
    pub location: LocationId,
    pub action: Action,
    pub change: Change,
    pub other_servers: usize,
    /// Sent back by the window with « Installer » or « Retirer »: the file and the change must still be these.
    pub fingerprint: String,
    original: Option<Vec<u8>>,
    text: Option<String>,
}

impl Plan {
    /// A backup is made before the write: the file exists and changes.
    pub fn makes_backup(&self) -> bool {
        self.text.is_some() && self.original.is_some()
    }
}

/// No content in a `Debug`: the file may hold the tokens of other servers.
impl fmt::Debug for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plan")
            .field("location", &self.location)
            .field("action", &self.action)
            .field("change", &self.change)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

fn fingerprint(id: LocationId, action: Action, original: Option<&[u8]>, text: Option<&str>) -> String {
    let mut hash = Sha256::new();
    hash.update(id.as_str());
    hash.update([0]);
    hash.update(action.as_str());
    for part in [original, text.map(str::as_bytes)] {
        match part {
            None => hash.update([0]),
            Some(bytes) => {
                hash.update([1]);
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What `action` would do to the location `id`. Reads, writes nothing.
pub fn plan(
    files: &dyn ClientFiles,
    env: &Environment,
    id: LocationId,
    action: Action,
    target: Option<&Path>,
) -> Result<Plan, ClientError> {
    let location = env.location(id);
    let adapter = id.adapter();
    let section = adapter.section();
    let fields = match (action, target) {
        (Action::Install, None) => return Err(ClientError::NoTarget),
        (Action::Install, Some(exe)) => adapter.fields(exe),
        (Action::Remove, _) => Vec::new(),
    };
    let entry = jsonc::member_text(ENTRY_NAME, &fields);
    let snippet = || (action == Action::Install).then(|| jsonc::fresh(section, ENTRY_NAME, &fields));
    let manual = |reason: Unreadable, original: Option<Vec<u8>>| Plan {
        location: id,
        action,
        change: Change::Manual {
            reason,
            snippet: snippet(),
        },
        other_servers: 0,
        fingerprint: fingerprint(id, action, original.as_deref(), None),
        original,
        text: None,
    };
    let content = match read(files, &location) {
        Ok(None) if action == Action::Install => return Err(ClientError::NotInstalled(id)),
        Ok(None) => Content::Absent,
        Ok(Some(content)) => content,
        Err(reason) => return Ok(manual(reason, None)),
    };
    let (original, text) = match content {
        Content::Absent => (None, String::new()),
        Content::Text { bytes, text } => (Some(bytes), text),
    };
    let found = match jsonc::find(&text, section, ENTRY_NAME) {
        Ok(found) => found,
        Err(reason) => return Ok(manual(reason, original)),
    };
    let done = |change: Change, original: Option<Vec<u8>>, text: Option<String>| Plan {
        location: id,
        action,
        change,
        other_servers: found.others,
        fingerprint: fingerprint(id, action, original.as_deref(), text.as_deref()),
        original,
        text,
    };
    match (action, &found.entry) {
        (Action::Install, Some(current)) if is_ours(current, &fields) => Ok(done(Change::Nothing, original, None)),
        (Action::Remove, None) => Ok(done(Change::Nothing, original, None)),
        (Action::Install, current) => match jsonc::set(&text, section, ENTRY_NAME, &fields) {
            Ok(changed) => {
                let change = match (current, &original) {
                    (Some(current), _) => Change::Replace {
                        previous: summary(current),
                        entry,
                    },
                    (None, None) => Change::Create { entry },
                    (None, Some(_)) => Change::Add { entry },
                };
                Ok(done(change, original, Some(changed)))
            }
            Err(reason) => Ok(manual(reason, original)),
        },
        (Action::Remove, Some(current)) => match jsonc::remove(&text, section, ENTRY_NAME) {
            Ok(Some(changed)) => Ok(done(
                Change::Remove {
                    previous: summary(current),
                },
                original,
                Some(changed),
            )),
            Ok(None) => Ok(done(Change::Nothing, original, None)),
            Err(reason) => Ok(manual(reason, original)),
        },
    }
}

/// What an applied action did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub change: Change,
    /// The file name of the backup (in the folder of the file), when one was made.
    pub backup: Option<String>,
    pub written: bool,
}

/// `action`, as the preview whose `fingerprint` the window sends back showed it. The file is read again: if it,
/// the installed exe or the change differ from the preview, [`ClientError::Changed`] and nothing is written. The
/// backup is named `<file>.bak-<stamp>`.
pub fn apply(
    files: &dyn ClientFiles,
    env: &Environment,
    id: LocationId,
    action: Action,
    target: Option<&Path>,
    fingerprint: &str,
    stamp: &str,
) -> Result<Applied, ClientError> {
    let plan = plan(files, env, id, action, target)?;
    if plan.fingerprint != fingerprint {
        return Err(ClientError::Changed);
    }
    write_plan(files, env, plan, stamp)
}

fn write_plan(files: &dyn ClientFiles, env: &Environment, plan: Plan, stamp: &str) -> Result<Applied, ClientError> {
    let Some(text) = &plan.text else {
        return Ok(Applied {
            change: plan.change,
            backup: None,
            written: false,
        });
    };
    let location = env.location(plan.location);
    let backup = files::write(files, &location.file, plan.original.as_deref(), text, stamp)?;
    Ok(Applied {
        change: plan.change,
        backup,
        written: true,
    })
}

/// The locations where an entry `mcp-proxy` is found, ours or not: what uninstalling offers to remove (spec 022).
pub fn configured(files: &dyn ClientFiles, env: &Environment) -> Vec<LocationId> {
    let target = Some(env.installed_exe());
    inspect_all(files, env, target.as_deref())
        .into_iter()
        .filter(|i| matches!(i.state, State::Configured | State::OtherCommand(_)))
        .map(|i| i.location)
        .collect()
}

/// Removes the entry `mcp-proxy` of `id` without a preview, with the same guarantees (backup, atomic write,
/// nothing written to a file that cannot be read safely): for the uninstallation (spec 022), where the user ticks
/// the clients to clean.
pub fn remove_entry(
    files: &dyn ClientFiles,
    env: &Environment,
    id: LocationId,
    stamp: &str,
) -> Result<Applied, ClientError> {
    let plan = plan(files, env, id, Action::Remove, None)?;
    write_plan(files, env, plan, stamp)
}

/// The configuration to paste in any other client: `{ "mcpServers": { "mcp-proxy": { … } } }`, the most common
/// form.
pub fn generic_snippet(exe: &Path) -> String {
    jsonc::fresh(
        claude::ClaudeDesktop.section(),
        ENTRY_NAME,
        &claude::ClaudeDesktop.fields(exe),
    )
}
