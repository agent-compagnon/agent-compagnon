//! The application: checks, then Tauri (single instance, hidden main window, notification icon), the gateway and
//! its named pipe. Every start failure ends in a message box, a line of log and a non-zero exit.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;

use mcp_proxy_auth::UnavailableAuthUi;
use mcp_proxy_core::confirm::UnavailableConfirmation;
use mcp_proxy_core::secrets::SecretStore;
use mcp_proxy_core::trace::SystemWallClock;
use mcp_proxy_gateway::bridge::{PeerProof, pipe_name};
use mcp_proxy_gateway::runtime::{
    DataDirs, Gateway, Logging, RuntimeDeps, StartupFailure, check_webview, record_startup_failure,
};
use mcp_proxy_gateway::view::{Alert, AlertKind, AlertLevel, AppStatus};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::broadcast::error::RecvError;

use crate::dialog;
use crate::dpapi::DpapiProof;
use crate::{pipe, wer};

const MAIN_WINDOW: &str = "main";
const TRAY: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_QUIT: &str = "quit";
const SNAPSHOT_CHANGED: &str = "snapshot-changed";

/// The three icons of the notification area, drawn by `cargo xtask icons`.
struct TrayIcons {
    neutral: Image<'static>,
    attention: Image<'static>,
    error: Image<'static>,
}

impl TrayIcons {
    fn load() -> tauri::Result<Self> {
        Ok(Self {
            neutral: Image::from_bytes(include_bytes!("../icons/tray-neutral.png"))?,
            attention: Image::from_bytes(include_bytes!("../icons/tray-attention.png"))?,
            error: Image::from_bytes(include_bytes!("../icons/tray-error.png"))?,
        })
    }

    fn for_status(&self, status: AppStatus) -> Image<'static> {
        match status {
            AppStatus::Neutral => self.neutral.clone(),
            AppStatus::Attention => self.attention.clone(),
            AppStatus::Error => self.error.clone(),
        }
    }
}

/// A failure before the journal exists: line in today's log file, message box, exit.
fn refuse_early(dirs: Option<&DataDirs>, failure: &StartupFailure) -> ExitCode {
    if let Some(dirs) = dirs {
        let _ = record_startup_failure(&dirs.logs, SystemTime::now(), failure);
    }
    dialog::error(failure.title(), &failure.message());
    ExitCode::FAILURE
}

/// A failure once the journal exists.
fn refuse(failure: &StartupFailure) {
    tracing::error!(cause = %failure.log_line(), "démarrage refusé");
    dialog::error(failure.title(), &failure.message());
}

pub fn run() -> ExitCode {
    let Some(base) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return refuse_early(None, &StartupFailure::NoDataDir);
    };
    let dirs = DataDirs::under(&base);

    let config = match mcp_proxy_gateway::modules::embedded_config() {
        Ok(config) => Arc::new(config),
        Err(errors) => {
            let lines = errors.to_string().lines().map(str::to_owned).collect();
            return refuse_early(Some(&dirs), &StartupFailure::InvalidConfig(lines));
        }
    };
    if let Err(failure) = check_webview(tauri::webview_version().map_err(|e| e.to_string())) {
        return refuse_early(Some(&dirs), &failure);
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            return refuse_early(Some(&dirs), &StartupFailure::Gateway(error.kind().to_string()));
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());

    let app = tauri::Builder::default()
        // Registered first: a second launch exits here, before anything below runs, and shows the first window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app)
        }))
        .invoke_handler(tauri::generate_handler![get_snapshot])
        .setup(move |app| setup(app, config, dirs))
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        // The setup already explained its failure.
        Err(tauri::Error::Setup(_)) => return ExitCode::FAILURE,
        Err(error) => {
            refuse(&StartupFailure::Gateway(error.to_string()));
            return ExitCode::FAILURE;
        }
    };

    let code = app.run_return(|app, event| match event {
        // Closing the window hides it; only « Quitter » (explicit code) stops the application.
        RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            if let Some(gateway) = app.try_state::<Arc<Gateway>>() {
                tracing::info!("arrêt de MCP Proxy");
                let _ = gateway.journal().flush();
            }
        }
        _ => {}
    });
    runtime.shutdown_background();
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn setup(app: &mut tauri::App, config: Arc<mcp_proxy_core::config::Config>, dirs: DataDirs) -> SetupResult {
    let store = match SecretStore::for_platform() {
        Ok(store) => store,
        Err(error) => {
            let failure = StartupFailure::ProtectionUnavailable(error.to_string());
            let _ = refuse_early(Some(&dirs), &failure);
            return Err(failure.log_line().into());
        }
    };
    let logs = dirs.logs.clone();
    let started = tauri::async_runtime::block_on(Gateway::start(RuntimeDeps {
        config,
        dirs,
        store,
        auth_ui: Arc::new(UnavailableAuthUi),
        broker: Arc::new(UnavailableConfirmation),
        clock: Arc::new(SystemWallClock),
        logging: Logging::Global,
        utc_offset: 0,
    }));
    let gateway = match started {
        Ok(gateway) => Arc::new(gateway),
        Err(error) => {
            let failure = StartupFailure::Gateway(error.to_string());
            let _ = record_startup_failure(&logs, SystemTime::now(), &failure);
            dialog::error(failure.title(), &failure.message());
            return Err(failure.log_line().into());
        }
    };

    if let Err(reason) = wer::exclude_from_error_reports() {
        tracing::warn!(%reason, "exclusion des rapports d'erreur Windows non posée");
        gateway.add_alert(Alert {
            level: AlertLevel::Info,
            kind: AlertKind::StartupHardening,
            message: format!(
                "Exclusion des rapports d'erreur Windows non posée ({reason}) : un plantage pourrait laisser une \
                 image mémoire sur le disque."
            ),
        });
    }

    let domain = std::env::var("USERDOMAIN").ok();
    let user = std::env::var("USERNAME").ok();
    let Some(name) = pipe_name(domain.as_deref(), user.as_deref()) else {
        refuse(&StartupFailure::NoUser);
        return Err(StartupFailure::NoUser.log_line().into());
    };
    let server = tauri::async_runtime::block_on(async { pipe::create_first(&name) });
    let server = match server {
        Ok(server) => server,
        Err(failure) => {
            refuse(&failure);
            return Err(failure.log_line().into());
        }
    };
    let proof: Arc<dyn PeerProof> = Arc::new(DpapiProof);
    tauri::async_runtime::spawn(pipe::accept(server, name, Arc::clone(&gateway), proof));

    let window = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("index.html".into()))
        .title("MCP Proxy")
        .inner_size(960.0, 680.0)
        .visible(false)
        .build()?;
    let hidden = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hidden.hide();
        }
    });

    let icons = Arc::new(TrayIcons::load()?);
    let open = MenuItem::with_id(app, MENU_OPEN, "Ouvrir", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id(TRAY)
        .icon(icons.neutral.clone())
        .tooltip("MCP Proxy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(Arc::clone(&gateway));
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(follow_snapshot(handle.clone(), Arc::clone(&gateway), icons));

    // First launch: the window asks for the profile.
    if tauri::async_runtime::block_on(gateway.snapshot()).profile.first_launch {
        show_main_window(&handle);
    }
    Ok(())
}

type SetupResult = Result<(), Box<dyn std::error::Error>>;

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Keeps the icon and the window in step with the gateway.
async fn follow_snapshot(app: AppHandle, gateway: Arc<Gateway>, icons: Arc<TrayIcons>) {
    let mut changes = gateway.changes();
    loop {
        let snapshot = gateway.snapshot().await;
        if let Some(tray) = app.tray_by_id(TRAY) {
            let _ = tray.set_icon(Some(icons.for_status(snapshot.status)));
            let _ = tray.set_tooltip(Some(&snapshot.tooltip));
        }
        if let Some(json) = gateway.snapshot_json().await {
            let _ = app.emit_to(MAIN_WINDOW, SNAPSHOT_CHANGED, json);
        }
        match changes.recv().await {
            Ok(()) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

/// Everything the interface shows, filtered once more; nothing when the filter is unavailable.
#[tauri::command]
async fn get_snapshot(gateway: tauri::State<'_, Arc<Gateway>>) -> Result<serde_json::Value, String> {
    gateway
        .snapshot_json()
        .await
        .ok_or_else(|| "filtre de sortie indisponible : état non transmis".to_owned())
}
