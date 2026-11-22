//! The updater in the application (spec 020). Wiring only: the checks, their order and the replacement are in
//! `mcp_proxy_deploy::update`, the schedule and the critical restart in `mcp_proxy_gateway::self_update`.
//!
//! Only the installed exe updates itself: a copy run from elsewhere is not replaced behind the user's back.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mcp_proxy_deploy::ops::SystemOps;
use mcp_proxy_deploy::update::{
    AnonymousClient, ExeReplacer, KeyRing, RenameReplacer, Source, UpdatePaths, Updater, UpdaterDeps, check_interval,
    clean_leftovers,
};
use mcp_proxy_deploy::version::own_version;
use mcp_proxy_gateway::runtime::{DataDirs, Gateway};
use mcp_proxy_gateway::self_update::{Restarter, SelfUpdateDeps, SelfUpdateTask, TokioPause, update_information};
use tauri::AppHandle;

use crate::app::{InstalledHere, inform};
use crate::dialog;
use crate::notify::Toaster;

/// Asked by « Redémarrer pour mettre à jour » or by a critical release: the application exits, and once its event
/// loop has released the single instance and the pipe, [`relaunch_if_asked`] starts the installed exe.
#[derive(Clone, Default)]
pub struct RestartRequest(Arc<AtomicBool>);

struct AppRestarter {
    app: AppHandle,
    request: RestartRequest,
}

impl Restarter for AppRestarter {
    fn restart(&self) {
        tracing::info!("redémarrage de MCP Proxy pour appliquer une mise à jour");
        self.request.0.store(true, Ordering::SeqCst);
        self.app.exit(0);
    }
}

/// How long the tasks of the runtime get to drop their handles (the pipe above all) before the new process starts.
const SHUTDOWN_BOUND: Duration = Duration::from_secs(5);

/// After the event loop: the runtime is shut down (the pipe closes with its tasks), then the installed exe starts.
pub fn relaunch_if_asked(
    request: &RestartRequest,
    installed: Option<(Arc<dyn SystemOps>, PathBuf)>,
    runtime: tokio::runtime::Runtime,
) {
    if !request.0.load(Ordering::SeqCst) {
        runtime.shutdown_background();
        return;
    }
    runtime.shutdown_timeout(SHUTDOWN_BOUND);
    let Some((ops, exe)) = installed else { return };
    if let Err(error) = ops.launch(&exe, &[]) {
        dialog::error(
            "MCP Proxy",
            &format!(
                "La nouvelle version de MCP Proxy n'a pas pu être lancée ({error}). Relancez MCP Proxy depuis le menu \
                 Démarrer."
            ),
        );
    }
}

/// The leftovers of an earlier replacement, then the checks; nothing outside the installation.
pub fn start(
    app: &AppHandle,
    gateway: &Arc<Gateway>,
    installed: Option<&InstalledHere>,
    dirs: &DataDirs,
    request: RestartRequest,
) -> Option<SelfUpdateTask> {
    let installed = installed?;
    let paths = UpdatePaths::new(installed.paths.clone(), &dirs.root);
    let ops = Arc::clone(&installed.ops);
    let cleaning = paths.clone();
    tauri::async_runtime::spawn_blocking(move || clean_leftovers(ops.as_ref(), &cleaning));

    let fetch = match AnonymousClient::new() {
        Ok(fetch) => Arc::new(fetch),
        Err(error) => {
            tracing::error!(%error, "mises à jour : client de téléchargement impossible à créer");
            return None;
        }
    };
    let config = &gateway.config().update;
    let replacer: Arc<dyn ExeReplacer> = Arc::new(RenameReplacer::new(Arc::clone(&installed.ops), paths.install));
    let deps = SelfUpdateDeps {
        updater: Updater::new(UpdaterDeps {
            source: Source {
                base_url: config.base_url.clone(),
                repository: config.repository.clone(),
                path: config.path.clone(),
            },
            fetch,
            replacer,
            keys: KeyRing::embedded(),
            downloads: paths.downloads,
            current: own_version(),
        }),
        interval: check_interval(config.check_interval_hours),
        pause: Arc::new(TokioPause),
        restarter: Arc::new(AppRestarter {
            app: app.clone(),
            request,
        }),
    };
    let gateway = Arc::clone(gateway);
    tauri::async_runtime::block_on(async move { gateway.start_self_update(deps) })
}

/// Paths of the installation, for the relaunch.
pub fn relaunch_target(installed: Option<&InstalledHere>) -> Option<(Arc<dyn SystemOps>, PathBuf)> {
    installed.map(|i| (Arc::clone(&i.ops), i.paths.exe.clone()))
}

/// The notification of an update put in place, once per version.
pub async fn follow(app: AppHandle, gateway: Arc<Gateway>, toaster: Arc<Toaster>) {
    let mut changes = gateway.update_changes();
    let mut shown: Option<(String, bool)> = None;
    loop {
        let ready = changes.borrow_and_update().clone();
        if let Some(ready) = ready {
            let key = (ready.version.to_string(), ready.critical);
            if shown.as_ref() != Some(&key) {
                inform(&app, &toaster, update_information(&ready));
                shown = Some(key);
            }
        }
        if changes.changed().await.is_err() {
            return;
        }
    }
}
