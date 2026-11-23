//! Running the plan. Nothing is removed before the exe is set aside and the running instance has stopped; then every
//! item is tried, whatever happened to the previous ones, and each says what it did. An item already gone is fine.
//! When one failed, the exe goes back in place and the entry « Applications installées » stays, so that the
//! uninstallation can be run again once the cause is gone.
//!
//! The journal names the items and the error kinds, never a path.

use std::fmt;
use std::path::Path;

use crate::clients::{self, Change, ClientError, ClientFiles, Environment};
use crate::install::{EXE_NAME, InstallPaths, SHORTCUT_NAME, same_location};
use crate::ops::{OpError, PathKind, Removal, SystemOps};
use crate::uninstall::entry::command_exe;
use crate::uninstall::guard::{TreeRemoval, check_owned, remove_tree};
use crate::uninstall::plan::{
    ClientDecision, DATA_FOLDER, INSTALL_FOLDER, Item, PROGRAMS_FOLDER, UninstallPaths, WINDOW_DATA_FOLDER,
};
use crate::update::is_leftover;

/// Attempts at the data folder: the WebView2 processes of the stopped application may still hold its profile for a
/// moment.
pub const DATA_ATTEMPTS: u32 = 10;
/// Attempts at the data of the window, once it closed: its WebView2 processes end after it.
pub const WINDOW_DATA_ATTEMPTS: u32 = 40;

/// Why nothing was removed. French, shown in the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallError {
    /// A `mcp-proxy.exe.old` left by an update is still run, by a bridge of an older version.
    OldVersionHeld(OpError),
    SetAside(OpError),
    /// The running instance did not stop. `restored`: the exe is back under its name.
    NotStopped {
        cause: OpError,
        restored: bool,
    },
}

impl fmt::Display for UninstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OldVersionHeld(cause) => write!(
                f,
                "Une ancienne version de MCP Proxy est encore utilisée, sans doute par un client LLM ({cause}). Rien \
                 n'a été modifié. Fermez vos clients LLM (Claude Desktop, VS Code…), puis relancez la désinstallation."
            ),
            Self::SetAside(cause) => write!(
                f,
                "Le programme de MCP Proxy n'a pas pu être mis de côté ({cause}). Rien n'a été modifié."
            ),
            Self::NotStopped { cause, restored } => {
                write!(
                    f,
                    "MCP Proxy, ouvert, n'a pas pu être fermé ({cause}). Rien n'a été modifié. Quittez MCP Proxy \
                     depuis son icône, puis relancez la désinstallation."
                )?;
                if !restored {
                    f.write_str(
                        " Le programme est resté sous le nom « mcp-proxy.exe.old » dans son dossier : renommez-le en \
                         « mcp-proxy.exe ».",
                    )?;
                }
                Ok(())
            }
        }
    }
}

/// Before anything is removed: the exe is renamed `mcp-proxy.exe.old` in its folder, so that an LLM client that
/// starts its bridge again finds nothing to start (and cannot start the application again), then the running
/// instance is asked to stop. An instance that does not stop gets its exe back: nothing changed.
pub fn prepare(ops: &dyn SystemOps, paths: &InstallPaths) -> Result<(), UninstallError> {
    if let Err(error) = ops.remove_file(&paths.backup)
        && !error.is_not_found()
    {
        tracing::warn!(cause = %error, "désinstallation : ancienne version encore utilisée");
        return Err(UninstallError::OldVersionHeld(error));
    }
    ops.rename(&paths.exe, &paths.backup).map_err(|error| {
        tracing::warn!(cause = %error, "désinstallation : exe impossible à mettre de côté");
        UninstallError::SetAside(error)
    })?;
    match ops.stop_instance() {
        Ok(stopped) => {
            tracing::info!(?stopped, "désinstallation : exe mis de côté, instance arrêtée");
            Ok(())
        }
        Err(cause) => {
            let restored = ops.rename(&paths.backup, &paths.exe).is_ok();
            tracing::warn!(cause = %cause, restored, "désinstallation : MCP Proxy en cours ne s'est pas arrêté");
            Err(UninstallError::NotStopped { cause, restored })
        }
    }
}

/// What became of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Removed,
    Absent,
    /// Left on purpose (not ours, unticked, kept for running the uninstallation again…): why, in French.
    Kept(String),
    /// Goes once the window closes.
    AtClose,
    /// Stays because something else failed: MCP Proxy remains installed, to run the uninstallation again.
    KeptToRetry,
    /// What could not be removed, and the first cause, in French.
    Failed {
        cause: String,
        left: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub item: Item,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub items: Vec<Done>,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.items.iter().any(|d| matches!(d.outcome, Outcome::Failed { .. }))
    }

    /// MCP Proxy stays installed, so that the uninstallation can be run again.
    pub fn kept_installed(&self) -> bool {
        self.items
            .iter()
            .any(|d| d.item == Item::Exe && d.outcome == Outcome::KeptToRetry)
    }

    fn push(&mut self, item: Item, outcome: Outcome) {
        tracing::info!(
            item = item.name(),
            outcome = outcome.name(),
            "désinstallation : élément traité"
        );
        self.items.push(Done { item, outcome });
    }
}

impl Item {
    /// For the journal: no path, no name of a client file.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Client { .. } => "client LLM",
            Self::Autostart => "démarrage avec Windows",
            Self::Shortcut => "raccourci",
            Self::Aumid => "notifications",
            Self::ErrorReports => "rapports d'erreur",
            Self::Data => "données",
            Self::UninstallEntry => "entrée de désinstallation",
            Self::Exe => "exe",
            Self::InstallFolder => "dossier d'installation",
            Self::WindowData => "données de la fenêtre",
        }
    }
}

impl Outcome {
    fn name(&self) -> &'static str {
        match self {
            Self::Removed => "retiré",
            Self::Absent => "absent",
            Self::Kept(_) => "laissé",
            Self::AtClose => "à la fermeture",
            Self::KeptToRetry => "gardé pour relancer",
            Self::Failed { .. } => "échec",
        }
    }
}

/// What the uninstallation works with.
pub struct Context<'a> {
    pub ops: &'a dyn SystemOps,
    pub files: &'a dyn ClientFiles,
    pub env: &'a Environment,
    pub paths: &'a UninstallPaths,
    /// The stamp of the backups of the client files.
    pub stamp: &'a str,
    /// Between two attempts at a folder that a closing program may still hold.
    pub pause: &'a dyn Fn(),
}

fn failed(cause: &OpError, left: Vec<String>) -> Outcome {
    Outcome::Failed {
        cause: cause.to_string(),
        left,
    }
}

fn of(result: Result<Removal, OpError>, not_ours: &str) -> Outcome {
    match result {
        Ok(Removal::Removed) => Outcome::Removed,
        Ok(Removal::Absent) => Outcome::Absent,
        Ok(Removal::NotOurs) => Outcome::Kept(not_ours.to_owned()),
        Err(error) => failed(&error, Vec::new()),
    }
}

/// `%LOCALAPPDATA%\<folder>\<relative>`, as the user reads it.
fn label(folder: &str, relative: &Path) -> String {
    let relative = relative.to_string_lossy().replace('/', r"\");
    if relative.is_empty() {
        format!(r"%LOCALAPPDATA%\{folder}")
    } else {
        format!(r"%LOCALAPPDATA%\{folder}\{relative}")
    }
}

fn client(ctx: &Context<'_>, location: clients::LocationId, decision: &ClientDecision) -> Outcome {
    match decision {
        ClientDecision::Asked => Outcome::Kept("laissée, comme demandé".to_owned()),
        ClientDecision::OtherExe(command) => Outcome::Kept(format!(
            "elle lance un autre programme ({}) : laissée",
            command.as_deref().unwrap_or("commande inconnue")
        )),
        ClientDecision::Unreadable(reason) => Outcome::Kept(format!(
            "fichier illisible ({reason}) : regardez à la main s'il contient une entrée « mcp-proxy »"
        )),
        ClientDecision::Remove => match clients::remove_entry(ctx.files, ctx.env, location, ctx.stamp) {
            Ok(applied) if applied.written => Outcome::Removed,
            Ok(applied) => match applied.change {
                Change::Manual { reason, .. } => {
                    Outcome::Kept(format!("{reason} : retirez l'entrée « mcp-proxy » à la main"))
                }
                _ => Outcome::Absent,
            },
            Err(ClientError::NotInstalled(_)) => Outcome::Absent,
            Err(error) => Outcome::Failed {
                cause: error.to_string(),
                left: Vec::new(),
            },
        },
    }
}

fn autostart(ctx: &Context<'_>) -> Outcome {
    let exe = &ctx.paths.install.exe;
    match ctx.ops.run_entry() {
        Err(error) => failed(&error, Vec::new()),
        Ok(None) => Outcome::Absent,
        Ok(Some(value)) if same_location(Path::new(command_exe(&value)), exe) => match ctx.ops.remove_run_entry() {
            Ok(()) => Outcome::Removed,
            Err(error) => failed(&error, Vec::new()),
        },
        Ok(Some(_)) => Outcome::Kept("la valeur « MCP Proxy » lance un autre programme : laissée".to_owned()),
    }
}

/// Removes a file of ours, or the link standing in its place; never a folder.
fn remove_file_of_ours(ops: &dyn SystemOps, path: &Path) -> Result<Removal, OpError> {
    let gone = |result: Result<(), OpError>| match result {
        Ok(()) => Ok(Removal::Removed),
        Err(error) if error.is_not_found() => Ok(Removal::Absent),
        Err(error) => Err(error),
    };
    match ops.kind(path)? {
        PathKind::Missing => Ok(Removal::Absent),
        PathKind::File => gone(ops.remove_file(path)),
        PathKind::Link => gone(ops.remove_link(path)),
        PathKind::Dir | PathKind::Other => Ok(Removal::NotOurs),
    }
}

fn shortcut(ctx: &Context<'_>) -> Outcome {
    let shortcut = &ctx.paths.install.shortcut;
    // The temporary file of an interrupted write (spec 019).
    let temporary = shortcut.with_file_name(format!("{SHORTCUT_NAME}.tmp"));
    if let Err(error) = remove_file_of_ours(ctx.ops, &temporary) {
        tracing::info!(cause = %error, "désinstallation : raccourci temporaire laissé");
    }
    of(
        remove_file_of_ours(ctx.ops, shortcut),
        "un dossier porte le nom du raccourci : laissé",
    )
}

/// `path` must be `%LOCALAPPDATA%\<folder>`; removed whole, tried again `attempts` times when something holds it.
fn owned_tree(ctx: &Context<'_>, path: &Path, folder: &str, attempts: u32) -> Outcome {
    if let Err(refusal) = check_owned(path, &ctx.paths.local_app_data, folder) {
        tracing::error!(%refusal, "désinstallation : dossier refusé");
        return Outcome::Kept(format!("{refusal} : rien n'a été supprimé"));
    }
    let mut attempt = 1;
    loop {
        let outcome = match remove_tree(ctx.ops, path) {
            Err(error) => failed(&error, Vec::new()),
            Ok(TreeRemoval::Removed) => Outcome::Removed,
            Ok(TreeRemoval::Absent) => Outcome::Absent,
            Ok(TreeRemoval::NotAFolder) => {
                Outcome::Kept("ce n'est pas un simple dossier (lien ou fichier) : laissé tel quel".to_owned())
            }
            Ok(TreeRemoval::Partial { left, cause }) => {
                let mut labels: Vec<String> = left.iter().map(|p| label(folder, p)).collect();
                labels.dedup();
                failed(&cause, labels)
            }
        };
        if !matches!(outcome, Outcome::Failed { .. }) || attempt >= attempts {
            return outcome;
        }
        attempt += 1;
        (ctx.pause)();
    }
}

fn uninstall_entry(ctx: &Context<'_>) -> Outcome {
    match ctx.ops.uninstall_entry() {
        Err(error) => failed(&error, Vec::new()),
        Ok(None) => Outcome::Absent,
        Ok(Some(entry)) if same_location(Path::new(command_exe(&entry.uninstall_string)), &ctx.paths.install.exe) => {
            of(
                ctx.ops.remove_uninstall_entry(),
                "l'entrée contient d'autres clés : laissée",
            )
        }
        Ok(Some(_)) => Outcome::Kept("l'entrée « MCP Proxy » désigne un autre programme : laissée".to_owned()),
    }
}

/// Only the files the installation and the updates put there; then the folder, if nothing else is in it.
fn install_folder(ctx: &Context<'_>) -> Outcome {
    let paths = &ctx.paths.install;
    let ops = ctx.ops;
    let programs = ctx.paths.programs();
    let checked = check_owned(&programs, &ctx.paths.local_app_data, PROGRAMS_FOLDER)
        .and_then(|()| check_owned(&paths.dir, &programs, INSTALL_FOLDER));
    if let Err(refusal) = checked {
        tracing::error!(%refusal, "désinstallation : dossier d'installation refusé");
        return Outcome::Kept(format!("{refusal} : rien n'a été supprimé"));
    }
    match ops.kind(&paths.dir) {
        Err(error) => return failed(&error, Vec::new()),
        Ok(PathKind::Missing) => return Outcome::Absent,
        Ok(PathKind::Dir) => {}
        Ok(_) => return Outcome::Kept("ce n'est pas un simple dossier (lien ou fichier) : laissé tel quel".to_owned()),
    }
    if ops.exists(&paths.backup) {
        // The exe could only be scheduled for deletion where it is: the folder empties once the window closes.
        return Outcome::Kept(
            "le programme est supprimé à la fermeture de cette fenêtre ; le dossier, alors vide, peut être supprimé à \
             la main"
                .to_owned(),
        );
    }
    let mut entries = match ops.entries(&paths.dir) {
        Ok(entries) => entries,
        Err(error) => return failed(&error, Vec::new()),
    };
    entries.sort();
    let mut others = Vec::new();
    let mut left = Vec::new();
    let mut cause = None;
    for entry in entries {
        let name = entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let ours = (name.eq_ignore_ascii_case(EXE_NAME) || is_leftover(&name))
            && matches!(ops.kind(&entry), Ok(PathKind::File));
        if !ours {
            others.push(name);
            continue;
        }
        if let Err(error) = ops.remove_file(&entry)
            && !error.is_not_found()
        {
            left.push(name);
            cause.get_or_insert(error);
        }
    }
    let folder = |name: &str| format!(r"%LOCALAPPDATA%\{PROGRAMS_FOLDER}\{INSTALL_FOLDER}\{name}");
    if let Some(cause) = cause {
        return failed(&cause, left.iter().map(|n| folder(n)).collect());
    }
    if !others.is_empty() {
        let names: Vec<String> = others.iter().map(|n| format!("« {n} »")).collect();
        return Outcome::Kept(format!(
            "il contient d'autres fichiers, qui ne viennent pas de MCP Proxy : {}. Le dossier et ces fichiers sont \
             laissés",
            names.join(", ")
        ));
    }
    match ops.remove_dir(&paths.dir) {
        Ok(()) => Outcome::Removed,
        Err(error) if error.is_not_found() => Outcome::Absent,
        Err(error) => failed(&error, Vec::new()),
    }
}

fn one(ctx: &Context<'_>, item: &Item) -> Outcome {
    match item {
        Item::Client { location, decision } => client(ctx, *location, decision),
        Item::Autostart => autostart(ctx),
        Item::Shortcut => shortcut(ctx),
        Item::Aumid => of(ctx.ops.remove_aumid(), "la clé contient d'autres valeurs : laissée"),
        Item::ErrorReports => of(
            ctx.ops.remove_wer_exclusion(),
            "la valeur n'est pas celle que pose MCP Proxy : laissée",
        ),
        Item::Data => owned_tree(ctx, &ctx.paths.data, DATA_FOLDER, DATA_ATTEMPTS),
        Item::UninstallEntry => uninstall_entry(ctx),
        Item::Exe => match ctx.ops.delete_running_exe(&ctx.paths.install.backup) {
            Ok(()) => Outcome::AtClose,
            // Moved out of its folder before the helper could start: it waits in the temporary folder.
            Err(error) if !ctx.ops.exists(&ctx.paths.install.backup) => failed(
                &error,
                vec![r"%TEMP%\.mcp-proxy.….__relocated__.exe, à supprimer une fois cette fenêtre fermée".to_owned()],
            ),
            Err(error) => failed(
                &error,
                vec![format!(
                    "%LOCALAPPDATA%\\{PROGRAMS_FOLDER}\\{INSTALL_FOLDER}\\mcp-proxy.exe.old et son dossier, à \
                     supprimer une fois cette fenêtre fermée"
                )],
            ),
        },
        Item::InstallFolder => install_folder(ctx),
        Item::WindowData => Outcome::AtClose,
    }
}

/// Every item of `plan` but the data of the window, once [`prepare`] succeeded. The entry « Applications
/// installées », the exe and its folder go only if everything before went; otherwise the exe is put back.
pub fn remove(ctx: &Context<'_>, plan: &[Item]) -> Report {
    let mut report = Report::default();
    let last = [Item::UninstallEntry, Item::Exe, Item::InstallFolder, Item::WindowData];
    for item in plan.iter().filter(|i| !last.contains(i)) {
        let outcome = one(ctx, item);
        report.push(item.clone(), outcome);
    }
    for item in last.iter().filter(|i| plan.contains(i)) {
        let outcome = if report.failed() {
            keep(ctx, item)
        } else {
            one(ctx, item)
        };
        report.push(item.clone(), outcome);
    }
    report
}

/// After a failure: MCP Proxy stays installed.
fn keep(ctx: &Context<'_>, item: &Item) -> Outcome {
    let paths = &ctx.paths.install;
    match item {
        Item::Exe if ctx.ops.exists(&paths.backup) => match ctx.ops.rename(&paths.backup, &paths.exe) {
            Ok(()) => Outcome::KeptToRetry,
            Err(error) => failed(
                &error,
                vec![
                    "le programme, resté sous le nom « mcp-proxy.exe.old » : renommez-le en « mcp-proxy.exe »"
                        .to_owned(),
                ],
            ),
        },
        _ => Outcome::KeptToRetry,
    }
}

/// Once the window closed: its WebView2 data, tried again while its processes end. Nothing if MCP Proxy stays
/// installed.
pub fn remove_window_data(ctx: &Context<'_>, report: &Report) -> Option<Done> {
    let pending = report
        .items
        .iter()
        .any(|d| d.item == Item::WindowData && d.outcome == Outcome::AtClose);
    if !pending {
        return None;
    }
    let outcome = owned_tree(ctx, &ctx.paths.window_data, WINDOW_DATA_FOLDER, WINDOW_DATA_ATTEMPTS);
    tracing::info!(
        outcome = outcome.name(),
        "désinstallation : données de la fenêtre traitées"
    );
    Some(Done {
        item: Item::WindowData,
        outcome,
    })
}
