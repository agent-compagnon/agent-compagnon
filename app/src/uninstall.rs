//! Uninstallation (spec 022): `mcp-proxy.exe --uninstall`, from « Applications installées ». Wiring only: the plan,
//! the checks of every path, the steps and the report are in `mcp_proxy_deploy::uninstall`.
//!
//! The window is a local page of the bundle (`uninstall.html`), labelled `uninstall`, with its own capability
//! (`capabilities/uninstall.json`) granting its only command. It runs in a Tauri application of its own, without
//! the single instance and without the gateway, like the installation window. Once it closes, the WebView2 data of
//! the windows go, then the process ends and the helper of `self_replace` deletes the exe.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use mcp_proxy_auth::webview::{app_page_allowed, destination_for_log, js_data};
use mcp_proxy_deploy::clients::{Environment, RealFiles, inspect_all};
use mcp_proxy_deploy::uninstall::{
    Action, Context, Report, UNINSTALL_WINDOW, UninstallDialog, UninstallPaths, after_close_message, launch,
    parse_answer, plan, prepare, remove, remove_window_data, report_view,
};
use mcp_proxy_deploy::version::own_version;
use mcp_proxy_gateway::clients_view::backup_stamp;
use mcp_proxy_gateway::runtime::check_webview;
use serde_json::Value;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::app;
use crate::dialog;
use crate::system_ops::WindowsOps;

const TITLE: &str = "Désinstaller MCP Proxy";
/// Between two attempts at a folder that a closing WebView2 still holds.
const PAUSE: Duration = Duration::from_millis(250);

fn pause() {
    std::thread::sleep(PAUSE);
}

/// What the command of the window reaches, managed by Tauri.
struct Uninstaller {
    dialog: Mutex<UninstallDialog>,
    ops: WindowsOps,
    paths: UninstallPaths,
    env: Environment,
    /// The report of the removal, for what goes once the window closes.
    report: Mutex<Option<Report>>,
}

impl Uninstaller {
    fn dialog(&self) -> MutexGuard<'_, UninstallDialog> {
        self.dialog.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn report(&self) -> MutexGuard<'_, Option<Report>> {
        self.report.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn context<'a>(&'a self, stamp: &'a str) -> Context<'a> {
        Context {
            ops: &self.ops,
            files: &RealFiles,
            env: &self.env,
            paths: &self.paths,
            stamp,
            pause: &pause,
        }
    }

    /// On a blocking thread: the stop of the running instance, the registry and the files block. The state of the
    /// clients is read again: the window may have stayed open a while.
    fn uninstall(&self, remove_clients: bool) -> Result<Value, String> {
        prepare(&self.ops, &self.paths.install).map_err(|error| error.to_string())?;
        let found = inspect_all(&RealFiles, &self.env, Some(&self.paths.install.exe));
        let steps = plan(&found, &self.paths.install.exe, remove_clients);
        let stamp = backup_stamp(SystemTime::now());
        let report = remove(&self.context(&stamp), &steps);
        let view = report_view(&report);
        *self.report() = Some(report);
        Ok(view)
    }

    /// Once the window closed and its WebView2 let go.
    fn after_close(&self) {
        let Some(report) = self.report().clone() else { return };
        let stamp = backup_stamp(SystemTime::now());
        let Some(done) = remove_window_data(&self.context(&stamp), &report) else {
            return;
        };
        if let Some(message) = after_close_message(&done) {
            dialog::error(TITLE, &message);
        }
    }
}

/// `--uninstall`: only from the installed exe; anything else is explained and changes nothing.
pub fn start() -> ExitCode {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
    let (Some(local), Some(roaming), Ok(here)) = (local, roaming, std::env::current_exe()) else {
        dialog::error(
            TITLE,
            "Les dossiers de votre compte Windows (%LOCALAPPDATA%, %APPDATA%) sont introuvables : rien n'a été modifié.",
        );
        return ExitCode::FAILURE;
    };
    let paths = UninstallPaths::new(&local, &roaming);
    let ops = WindowsOps::new();
    if let Some(refusal) = launch(&ops, &paths.install, &here).refusal() {
        dialog::error(TITLE, refusal);
        return ExitCode::FAILURE;
    }
    if let Err(failure) = check_webview(tauri::webview_version().map_err(|e| e.to_string())) {
        dialog::error(failure.title(), &failure.message());
        return ExitCode::FAILURE;
    }
    let env = Environment {
        app_data: roaming,
        local_app_data: local,
    };
    let found = inspect_all(&RealFiles, &env, Some(&paths.install.exe));
    let dialog = UninstallDialog::new(own_version(), &plan(&found, &paths.install.exe, true));
    let script = format!(
        "Object.defineProperty(window, '__MCP_PROXY_UNINSTALL__', {{ value: Object.freeze({}) }});",
        js_data(&dialog.view())
    );
    let uninstaller = Arc::new(Uninstaller {
        dialog: Mutex::new(dialog),
        ops,
        paths,
        env,
        report: Mutex::new(None),
    });
    run_window(uninstaller, &script)
}

fn run_window(uninstaller: Arc<Uninstaller>, script: &str) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            dialog::error(
                TITLE,
                &format!(
                    "Désinstallation impossible à démarrer ({}). Rien n'a été modifié.",
                    error.kind()
                ),
            );
            return ExitCode::FAILURE;
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());
    let script = script.to_owned();
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![answer_uninstall])
        .setup(move |app| {
            if let Err(error) = build_window(app.handle(), &script) {
                tracing::error!(%error, "fenêtre de désinstallation impossible à ouvrir");
                dialog::error(
                    TITLE,
                    "La fenêtre de désinstallation n'a pas pu s'ouvrir. Rien n'a été modifié.",
                );
                app.handle().exit(1);
            }
            Ok(())
        })
        .build(app::context());
    let app = match app {
        Ok(app) => app,
        Err(_) => {
            dialog::error(
                TITLE,
                "La fenêtre de désinstallation n'a pas pu s'ouvrir. Rien n'a été modifié.",
            );
            return ExitCode::FAILURE;
        }
    };
    app.manage(Arc::clone(&uninstaller));
    let code = app.run_return(|_, _: RunEvent| {});
    runtime.shutdown_background();
    if uninstaller.dialog().removal_ran() {
        uninstaller.after_close();
    }
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn build_window(app: &AppHandle, script: &str) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, UNINSTALL_WINDOW, WebviewUrl::App("uninstall.html".into()))
        .title(TITLE)
        .inner_size(560.0, 700.0)
        .resizable(false)
        .center()
        .visible(true)
        .focused(true)
        .devtools(false)
        .initialization_script(script)
        .on_navigation(|url| {
            let allowed = app_page_allowed(url);
            if !allowed {
                tracing::warn!(destination = %destination_for_log(url), "navigation de la fenêtre de désinstallation bloquée");
            }
            allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let Some(uninstaller) = handle.try_state::<Arc<Uninstaller>>() else {
                handle.exit(0);
                return;
            };
            let mut dialog = uninstaller.dialog();
            if dialog.may_close() {
                let answer = dialog.close_answer();
                let _ = dialog.answer(answer);
                handle.exit(0);
            } else {
                api.prevent_close();
            }
        }
    });
    let _ = window.set_focus();
    Ok(())
}

/// The answer of the uninstallation window. « Désinstaller » gets the report back, or why nothing was changed;
/// « Annuler » and « Fermer » end this process.
#[tauri::command]
async fn answer_uninstall(window: WebviewWindow, answer: Option<Value>) -> Result<Value, String> {
    if window.label() != UNINSTALL_WINDOW {
        return Ok(Value::Null);
    }
    let Some(uninstaller) = window.try_state::<Arc<Uninstaller>>().map(|u| Arc::clone(u.inner())) else {
        return Ok(Value::Null);
    };
    let Ok(answer) = parse_answer(answer.as_ref()) else {
        tracing::info!("réponse de la fenêtre de désinstallation ignorée : mal formée");
        return Ok(Value::Null);
    };
    let Some(action) = uninstaller.dialog().answer(answer) else {
        return Ok(Value::Null);
    };
    let remove_clients = match action {
        Action::Quit => {
            window.app_handle().exit(0);
            return Ok(Value::Null);
        }
        Action::Uninstall { remove_clients } => remove_clients,
    };
    let working = Arc::clone(&uninstaller);
    let done = tauri::async_runtime::spawn_blocking(move || working.uninstall(remove_clients))
        .await
        .unwrap_or_else(|_| Err("désinstallation interrompue".to_owned()));
    match done {
        Ok(report) => {
            uninstaller.dialog().reported();
            Ok(report)
        }
        Err(message) => {
            uninstaller.dialog().failed();
            Err(message)
        }
    }
}
