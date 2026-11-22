//! The application: checks, then Tauri (single instance, hidden main window, notification icon, login windows),
//! the gateway and its named pipe. Every start failure ends in a message box, a line of log and a non-zero exit.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;

use mcp_proxy_auth::AuthUi;
use mcp_proxy_auth::webview::reset_profile_dir;
use mcp_proxy_core::secrets::{SecretStore, platform_key_wrapper};
use mcp_proxy_core::trace::SystemWallClock;
use mcp_proxy_gateway::bridge::{PeerProof, pipe_name};
use mcp_proxy_gateway::control::GatewayControl;
use mcp_proxy_gateway::journal_view::FeedItem;
use mcp_proxy_gateway::notify::{ConfirmationPresenter, Information, ToastAction, ToastState, information_toast};
use mcp_proxy_gateway::runtime::{
    DataDirs, Gateway, Logging, Persistence, RuntimeDeps, StartupFailure, check_webview, record_startup_failure,
};
use mcp_proxy_gateway::updates::UpdateEvent;
use mcp_proxy_gateway::view::{Alert, AlertKind, AlertLevel, AppStatus};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::broadcast::error::RecvError;

use crate::confirm::{self, WindowsPresenter};
use crate::dialog;
use crate::dpapi::DpapiProof;
use crate::interface;
use crate::login::{self, WebviewAuthUi};
use crate::notify::{ToastEvent, Toaster};
use crate::{pipe, wer};

pub(crate) const MAIN_WINDOW: &str = "main";
const TRAY: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_QUIT: &str = "quit";
const MENU_SUSPEND: &str = "suspend";
const MENU_DISCONNECT_ALL: &str = "disconnect-all";
const SNAPSHOT_CHANGED: &str = "snapshot-changed";
const JOURNAL_ENTRY: &str = "journal-entry";
const JOURNAL_RESYNC: &str = "journal-resync";

/// The four icons of the notification area, drawn by `cargo xtask icons`.
struct TrayIcons {
    neutral: Image<'static>,
    attention: Image<'static>,
    error: Image<'static>,
    suspended: Image<'static>,
}

impl TrayIcons {
    fn load() -> tauri::Result<Self> {
        Ok(Self {
            neutral: Image::from_bytes(include_bytes!("../icons/tray-neutral.png"))?,
            attention: Image::from_bytes(include_bytes!("../icons/tray-attention.png"))?,
            error: Image::from_bytes(include_bytes!("../icons/tray-error.png"))?,
            suspended: Image::from_bytes(include_bytes!("../icons/tray-suspended.png"))?,
        })
    }

    fn for_status(&self, status: AppStatus) -> Image<'static> {
        match status {
            AppStatus::Neutral => self.neutral.clone(),
            AppStatus::Attention => self.attention.clone(),
            AppStatus::Error => self.error.clone(),
            AppStatus::Suspended => self.suspended.clone(),
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
    let webview = match check_webview(tauri::webview_version().map_err(|e| e.to_string())) {
        Ok(version) => version,
        Err(failure) => return refuse_early(Some(&dirs), &failure),
    };
    // Before any webview uses it; a failure keeps the login windows closed (fail closed), reported once the
    // journal exists.
    let profile_reset = reset_profile_dir(&dirs.webview).map_err(|e| e.kind().to_string());

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            return refuse_early(Some(&dirs), &StartupFailure::Gateway(error.kind().to_string()));
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());

    let app = tauri::Builder::default()
        // Registered first: its setup runs inside `build`, where a second launch hands over to the first instance
        // and exits, before anything below runs.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app)
        }))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            login::deposit_login_header,
            login::submit_basic_credentials,
            confirm::answer_confirmation,
            interface::connect_session,
            interface::disconnect_session,
            interface::set_profile,
            interface::resume_gateway,
            interface::show_confirmations,
            interface::set_environment_exposed,
            interface::set_mcp_exposed,
            interface::acknowledge_mcp_update,
            interface::reload_mcp,
            interface::get_journal,
            interface::export_diagnostic
        ])
        .setup(|app| {
            // Tauri panics on a setup error: a failure here is explained, then the application exits.
            let gateway = app.try_state::<Arc<Gateway>>().map(|g| Arc::clone(g.inner()));
            let built = match gateway {
                Some(gateway) => setup_ui(app, gateway).map_err(|e| e.to_string()),
                None => Err("passerelle absente".to_owned()),
            };
            if let Err(error) = built {
                refuse(&StartupFailure::Gateway(format!(
                    "interface impossible à créer ({error})"
                )));
                app.handle().exit(1);
            }
            Ok(())
        })
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            return refuse_early(Some(&dirs), &StartupFailure::Gateway(error.to_string()));
        }
    };

    // First instance: the notifications, the login windows, the gateway, then its pipe. WinRT is called on a
    // blocking thread, never on the main thread that the event loop initializes.
    let toaster = {
        let dirs = dirs.clone();
        match tauri::async_runtime::block_on(tauri::async_runtime::spawn_blocking(move || Toaster::start(&dirs))) {
            Ok(toaster) => Arc::new(toaster),
            Err(_) => Arc::new(Toaster::unavailable("enregistrement interrompu")),
        }
    };
    let webview_dir = dirs.webview.clone();
    let auth_ui = WebviewAuthUi::with_toaster(
        app.handle(),
        &config.webview,
        webview_dir.clone(),
        profile_reset.clone(),
        Some(Arc::clone(&toaster)),
    );
    let presenter = Arc::new(WindowsPresenter::new(app.handle(), Arc::clone(&toaster)));
    let system = crate::system::description(&webview);
    let gateway = match start_gateway(config, dirs, Arc::new(auth_ui), presenter, system) {
        Ok(gateway) => gateway,
        Err(code) => return code,
    };
    if let Err(reason) = profile_reset {
        tracing::error!(%reason, "dossier du profil des fenêtres de connexion non vidé");
        gateway.add_alert(Alert {
            level: AlertLevel::Error,
            kind: AlertKind::StartupHardening,
            message: format!(
                "Le dossier des fenêtres de connexion n'a pas pu être vidé ({reason}) : aucune connexion par fenêtre \
                 n'est possible jusqu'au prochain démarrage de MCP Proxy."
            ),
        });
    }
    app.manage(Arc::clone(&toaster));
    app.manage(gateway);

    let code = app.run_return(move |app, event| match event {
        // Closing the window hides it; only « Quitter » (explicit code) stops the application.
        RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            // Best effort: WebView2 may still hold files; the folder is emptied again at the next start.
            if let Err(error) = reset_profile_dir(&webview_dir) {
                tracing::info!(kind = %error.kind(), "dossier du profil des fenêtres de connexion non vidé à l'arrêt");
            }
            if let Some(gateway) = app.try_state::<Arc<Gateway>>() {
                // Every waiting confirmation is refused, none can be approved any more.
                gateway.confirmations().pending().shutdown();
                tracing::info!("arrêt de MCP Proxy");
                let _ = gateway.journal().flush();
            }
            if let Some(toaster) = app.try_state::<Arc<Toaster>>() {
                toaster.clear_all();
            }
        }
        _ => {}
    });
    runtime.shutdown_background();
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// Store, gateway, exclusion from error reports, pipe. Every failure is explained before the exit code is returned.
fn start_gateway(
    config: Arc<mcp_proxy_core::config::Config>,
    dirs: DataDirs,
    auth_ui: Arc<dyn AuthUi>,
    presenter: Arc<dyn ConfirmationPresenter>,
    system: String,
) -> Result<Arc<Gateway>, ExitCode> {
    let store = match SecretStore::for_platform() {
        Ok(store) => store,
        Err(error) => {
            let failure = StartupFailure::ProtectionUnavailable(error.to_string());
            return Err(refuse_early(Some(&dirs), &failure));
        }
    };
    let logs = dirs.logs.clone();
    let persistence = Persistence::sealed(&dirs, platform_key_wrapper());
    let started = tauri::async_runtime::block_on(Gateway::start(RuntimeDeps {
        config,
        persistence,
        dirs,
        store,
        auth_ui,
        presenter,
        clock: Arc::new(SystemWallClock),
        logging: Logging::Global,
        utc_offset: 0,
        system,
    }));
    let gateway = match started {
        Ok(gateway) => Arc::new(gateway),
        Err(error) => {
            let failure = StartupFailure::Gateway(error.to_string());
            let _ = record_startup_failure(&logs, SystemTime::now(), &failure);
            dialog::error(failure.title(), &failure.message());
            return Err(ExitCode::FAILURE);
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
    let refused = |failure: StartupFailure| {
        refuse(&failure);
        let _ = gateway.journal().flush();
        ExitCode::FAILURE
    };
    let Some(name) = pipe_name(domain.as_deref(), user.as_deref()) else {
        return Err(refused(StartupFailure::NoUser));
    };
    let server = match tauri::async_runtime::block_on(async { pipe::create_first(&name) }) {
        Ok(server) => server,
        Err(failure) => return Err(refused(failure)),
    };
    let proof: Arc<dyn PeerProof> = Arc::new(DpapiProof);
    tauri::async_runtime::spawn(pipe::accept(server, name, Arc::clone(&gateway), proof));
    Ok(gateway)
}

/// Hidden main window, notification icon, and the task that keeps both in step with the gateway.
fn setup_ui(app: &mut tauri::App, gateway: Arc<Gateway>) -> tauri::Result<()> {
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
    // The profile is chosen in the window (spec 016): the provisional « Profil » menu of spec 014 is gone.
    let settings = gateway.settings();
    let suspend = MenuItem::with_id(
        app,
        MENU_SUSPEND,
        suspend_label(settings.gateway_suspended),
        true,
        None::<&str>,
    )?;
    let disconnect_all = MenuItem::with_id(app, MENU_DISCONNECT_ALL, "Tout déconnecter", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &suspend,
            &disconnect_all,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let tray_menu = Arc::new(TrayMenu { suspend });
    let menu_gateway = Arc::clone(&gateway);
    let menu_items = Arc::clone(&tray_menu);
    TrayIconBuilder::with_id(TRAY)
        .icon(icons.neutral.clone())
        .tooltip("MCP Proxy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            MENU_QUIT => app.exit(0),
            // Immediate, without confirmation (spec 015).
            MENU_SUSPEND => {
                tauri::async_runtime::spawn(toggle_suspension(Arc::clone(&menu_gateway), Arc::clone(&menu_items)));
            }
            MENU_DISCONNECT_ALL => {
                tauri::async_runtime::spawn(disconnect_everything(Arc::clone(&menu_gateway)));
            }
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

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(follow_snapshot(handle.clone(), Arc::clone(&gateway), icons, tray_menu));
    tauri::async_runtime::spawn(follow_journal(handle.clone(), Arc::clone(&gateway)));
    if let Some(toaster) = app.try_state::<Arc<Toaster>>().map(|t| Arc::clone(t.inner())) {
        tauri::async_runtime::spawn(follow_information(handle.clone(), Arc::clone(&gateway), toaster));
    }

    // First launch: the window asks for the profile.
    if tauri::async_runtime::block_on(gateway.snapshot()).profile.first_launch {
        show_main_window(&handle);
    }
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The item of the icon menu that follows the settings.
struct TrayMenu {
    /// « Suspendre la passerelle » or « Reprendre ».
    suspend: MenuItem<tauri::Wry>,
}

fn suspend_label(suspended: bool) -> &'static str {
    if suspended {
        "Reprendre"
    } else {
        "Suspendre la passerelle"
    }
}

/// The suspension item names what it would do, whether the change came from the menu or from the window.
fn sync_menu(menu: &TrayMenu, gateway: &Gateway) {
    let _ = menu
        .suspend
        .set_text(suspend_label(gateway.settings().gateway_suspended));
}

/// « Suspendre la passerelle » / « Reprendre ».
async fn toggle_suspension(gateway: Arc<Gateway>, menu: Arc<TrayMenu>) {
    let saved = if gateway.is_suspended() {
        gateway.resume().await
    } else {
        gateway.suspend().await
    };
    // A save failure is an alert of the snapshot (red icon); the change itself applies.
    if let Err(error) = saved {
        tracing::error!(%error, "suspension non enregistrée");
    }
    sync_menu(&menu, &gateway);
}

/// « Tout déconnecter ».
async fn disconnect_everything(gateway: Arc<Gateway>) {
    if let Err(error) = gateway.disconnect_all().await {
        let message = format!(
            "Toutes les sessions sont déconnectées et aucun secret n'est gardé, mais les données des fenêtres de \
             connexion n'ont pas pu être effacées ({error}). Elles le seront au prochain démarrage de MCP Proxy."
        );
        let _ = tauri::async_runtime::spawn_blocking(move || dialog::error("MCP Proxy", &message)).await;
    }
}

/// Keeps the icon, its menu and the window in step with the gateway.
async fn follow_snapshot(app: AppHandle, gateway: Arc<Gateway>, icons: Arc<TrayIcons>, menu: Arc<TrayMenu>) {
    let mut changes = gateway.changes();
    loop {
        let snapshot = gateway.snapshot().await;
        if let Some(tray) = app.tray_by_id(TRAY) {
            let _ = tray.set_icon(Some(icons.for_status(snapshot.status)));
            let _ = tray.set_tooltip(Some(&snapshot.tooltip));
        }
        sync_menu(&menu, &gateway);
        if let Some(json) = gateway.snapshot_json().await {
            let _ = app.emit_to(MAIN_WINDOW, SNAPSHOT_CHANGED, json);
        }
        match changes.recv().await {
            Ok(()) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

/// Sends every new entry of the journal to the window (spec 018). When entries were lost on the way, the window is
/// told to ask for the whole journal again.
async fn follow_journal(app: AppHandle, gateway: Arc<Gateway>) {
    let mut feed = gateway.journal_feed();
    while let Some(item) = feed.next().await {
        let _ = match item {
            FeedItem::Entry(entry) => app.emit_to(MAIN_WINDOW, JOURNAL_ENTRY, entry),
            FeedItem::Resync => app.emit_to(MAIN_WINDOW, JOURNAL_RESYNC, ()),
        };
    }
}

/// An information toast, when toasts can be seen; a click opens the main window.
fn inform(app: &AppHandle, toaster: &Arc<Toaster>, information: Information) {
    let toaster = Arc::clone(toaster);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let ToastState::Unavailable(reason) = toaster.state() {
            tracing::info!(%reason, "information non notifiée");
            return;
        }
        let content = information_toast(toaster.next_number(), &information);
        let shown = toaster.show(&content, None, move |event| {
            if let ToastEvent::Activated(Some(arguments)) = event
                && ToastAction::parse(&arguments) == Some(ToastAction::Open)
            {
                show_main_window(&app);
            }
        });
        if let Err(reason) = shown {
            tracing::info!(%reason, "information non notifiée");
        }
    });
}

/// The information notifications: settings reset at start, tools of an MCP changed. « Mise à jour prête » comes
/// with spec 020.
async fn follow_information(app: AppHandle, gateway: Arc<Gateway>, toaster: Arc<Toaster>) {
    let mut events = gateway.watcher().tracker().subscribe();
    if gateway
        .snapshot()
        .await
        .alerts
        .iter()
        .any(|a| a.kind == AlertKind::SettingsReset)
    {
        inform(&app, &toaster, Information::SettingsReset);
    }
    let config = Arc::clone(gateway.config());
    loop {
        match events.recv().await {
            Ok(UpdateEvent::UpdatePending { instance }) => {
                let mcp = config
                    .mcp
                    .iter()
                    .find(|m| m.id == instance.mcp)
                    .map(|m| m.label.clone());
                let env = config
                    .environments
                    .iter()
                    .find(|e| e.id == instance.env)
                    .map(|e| e.label.clone());
                if let (Some(mcp), Some(environment)) = (mcp, env) {
                    inform(&app, &toaster, Information::McpUpdated { mcp, environment });
                }
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
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
