//! Installation at first launch (spec 019): the decision, taken before the single instance, the pipe and the
//! gateway, then the installation window when there is something to offer. Wiring only: the decision, the steps and
//! their undoing, the answers of the window are in `mcp_proxy_deploy::install`.
//!
//! The window is a local page of the bundle (`install.html`), labelled `install`, with its own capability
//! (`capabilities/install.json`) granting its only command. It runs in a Tauri application of its own, without
//! the single instance (a running instance would otherwise take the launch over) and without the gateway; once the
//! user answered, this process launches the right exe and ends.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};

use mcp_proxy_auth::webview::{app_page_allowed, destination_for_log, js_data};
use mcp_proxy_deploy::install::{
    Action, INSTALL_WINDOW, InstallDialog, InstallPaths, InstallPlan, Proposal, Situation, Start, autostart_enabled,
    delegate, install, parse_answer, run_here, update,
};
use mcp_proxy_deploy::ops::SystemOps;
use mcp_proxy_deploy::version::own_version;
use mcp_proxy_gateway::launch::LaunchMode;
use mcp_proxy_gateway::runtime::check_webview;
use serde_json::Value;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::app::{self, AppStart, InstalledHere};
use crate::dialog;
use crate::system_ops::WindowsOps;

const TITLE: &str = "MCP Proxy";

/// Where the installation goes, from `%LOCALAPPDATA%` and `%APPDATA%`; `None` when either is missing.
fn install_paths() -> Option<InstallPaths> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from)?;
    Some(InstallPaths::new(&local, &roaming))
}

/// The application modes (`App`, `Autostart`, `InPlace`): decide, then run here, hand over, or offer.
pub fn start(mode: LaunchMode) -> ExitCode {
    let start = match mode {
        LaunchMode::App => Start::Interactive,
        LaunchMode::Autostart => Start::Autostart,
        LaunchMode::InPlace => Start::InPlace,
        LaunchMode::Stdio => Start::Bridge,
    };
    let hidden = start == Start::Autostart;
    let (Some(paths), Ok(here)) = (install_paths(), std::env::current_exe()) else {
        // Nothing to compare with: the application runs from here, without the autostart item.
        return app::run(AppStart {
            hidden,
            installed: None,
        });
    };
    let ops = Arc::new(WindowsOps::new());
    let own = own_version();
    let situation = Situation::inspect(ops.as_ref(), &paths, &here, start, own.clone());
    let installed = situation.here_is_installed.then(|| InstalledHere {
        ops: Arc::clone(&ops) as Arc<dyn SystemOps>,
        paths: paths.clone(),
    });
    match situation.plan() {
        InstallPlan::Continue => app::run(AppStart { hidden, installed }),
        InstallPlan::Delegate { autostart } => match delegate(ops.as_ref(), &paths, autostart) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                if !autostart {
                    dialog::error(
                        TITLE,
                        &format!("{error} MCP Proxy démarre depuis l'emplacement de cet exe, sans l'installer."),
                    );
                }
                app::run(AppStart {
                    hidden,
                    installed: None,
                })
            }
        },
        InstallPlan::Install => {
            let autostart = autostart_enabled(ops.as_ref(), &paths);
            offer(Proposal::Install { autostart }, own, ops, paths, here)
        }
        InstallPlan::Update { installed } => offer(Proposal::Update { installed }, own, ops, paths, here),
    }
}

/// What the command of the window reaches, managed by Tauri.
struct Installer {
    dialog: Mutex<InstallDialog>,
    ops: Arc<WindowsOps>,
    paths: InstallPaths,
    /// This exe, the source of the copy.
    here: PathBuf,
}

impl Installer {
    fn dialog(&self) -> std::sync::MutexGuard<'_, InstallDialog> {
        self.dialog.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The action of an answer, on a blocking thread: copies, registry and the stop of the running instance block.
    fn act(&self, action: Action) -> Result<(), String> {
        let ops = self.ops.as_ref();
        let done = match action {
            Action::Install { autostart } => install(ops, &self.paths, &self.here, autostart),
            Action::Update => update(ops, &self.paths, &self.here),
            Action::RunHere => run_here(ops, &self.here),
            Action::RunInstalled => delegate(ops, &self.paths, false),
            Action::Quit => Ok(()),
        };
        done.map_err(|error| error.to_string())
    }
}

/// The installation window, until the user answered.
fn offer(
    proposal: Proposal,
    own: mcp_proxy_deploy::version::Version,
    ops: Arc<WindowsOps>,
    paths: InstallPaths,
    here: PathBuf,
) -> ExitCode {
    if let Err(failure) = check_webview(tauri::webview_version().map_err(|e| e.to_string())) {
        dialog::error(failure.title(), &failure.message());
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            dialog::error(
                TITLE,
                &format!("Installation impossible à démarrer ({}).", error.kind()),
            );
            return ExitCode::FAILURE;
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());
    let dialog = InstallDialog::new(proposal, own);
    let script = format!(
        "Object.defineProperty(window, '__MCP_PROXY_INSTALL__', {{ value: Object.freeze({}) }});",
        js_data(&dialog.view())
    );
    let installer = Arc::new(Installer {
        dialog: Mutex::new(dialog),
        ops,
        paths,
        here,
    });
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![answer_install])
        .setup(move |app| {
            if let Err(error) = build_window(app.handle(), &script) {
                tracing::error!(%error, "fenêtre d'installation impossible à ouvrir");
                dialog::error(TITLE, "La fenêtre d'installation n'a pas pu s'ouvrir.");
                app.handle().exit(1);
            }
            Ok(())
        })
        .build(app::context());
    let app = match app {
        Ok(app) => app,
        Err(_) => {
            dialog::error(TITLE, "La fenêtre d'installation n'a pas pu s'ouvrir.");
            return ExitCode::FAILURE;
        }
    };
    app.manage(installer);
    let code = app.run_return(|_, _: RunEvent| {});
    runtime.shutdown_background();
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn build_window(app: &AppHandle, script: &str) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, INSTALL_WINDOW, WebviewUrl::App("install.html".into()))
        .title("Installer MCP Proxy")
        .inner_size(500.0, 440.0)
        .resizable(false)
        .center()
        .visible(true)
        .focused(true)
        .devtools(false)
        .initialization_script(script)
        .on_navigation(|url| {
            let allowed = app_page_allowed(url);
            if !allowed {
                tracing::warn!(destination = %destination_for_log(url), "navigation de la fenêtre d'installation bloquée");
            }
            allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let may_close = handle
                .try_state::<Arc<Installer>>()
                .is_none_or(|i| i.dialog().may_close());
            if may_close {
                handle.exit(0);
            } else {
                api.prevent_close();
            }
        }
    });
    let _ = window.set_focus();
    Ok(())
}

/// The answer of the installation window. An answer that does not fit the state is ignored; a failure comes back to
/// the page, which shows it and offers to continue from here or to quit; success ends this process.
#[tauri::command]
async fn answer_install(window: WebviewWindow, answer: Option<Value>) -> Result<(), String> {
    if window.label() != INSTALL_WINDOW {
        return Ok(());
    }
    let Some(installer) = window.try_state::<Arc<Installer>>().map(|i| Arc::clone(i.inner())) else {
        return Ok(());
    };
    let Ok(answer) = parse_answer(answer.as_ref()) else {
        tracing::info!("réponse de la fenêtre d'installation ignorée : mal formée");
        return Ok(());
    };
    let Some(action) = installer.dialog().answer(answer) else {
        return Ok(());
    };
    let acting = Arc::clone(&installer);
    let done = tauri::async_runtime::spawn_blocking(move || acting.act(action))
        .await
        .unwrap_or_else(|_| Err("installation interrompue".to_owned()));
    match done {
        Ok(()) => {
            window.app_handle().exit(0);
            Ok(())
        }
        Err(message) => {
            installer.dialog().failed();
            Err(message)
        }
    }
}
