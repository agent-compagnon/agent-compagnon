//! Commands of the main window (specs 016 to 018, 020), each declared in `build.rs` and granted to the window `main` alone by
//! `capabilities/main.json`. Wiring only: the checks and the effects live in `mcp_proxy_gateway::interface`. An
//! error is a French text without any secret, shown by the window.

use std::path::PathBuf;
use std::sync::Arc;

use mcp_proxy_gateway::control::GatewayControl;
use mcp_proxy_gateway::interface::{ExportOutcome, default_export_dir};
use mcp_proxy_gateway::runtime::Gateway;
use tauri::{Manager, WebviewWindow};

use crate::app::MAIN_WINDOW;
use crate::{dialog, login};

/// The gateway, for the main window only (defence in depth: the capability already restricts the commands to it).
fn gateway_for(window: &WebviewWindow) -> Result<Arc<Gateway>, String> {
    if window.label() != MAIN_WINDOW {
        tracing::warn!("commande de la fenêtre principale refusée à une autre fenêtre");
        return Err("commande réservée à la fenêtre principale".to_owned());
    }
    window
        .app_handle()
        .try_state::<Arc<Gateway>>()
        .map(|g| Arc::clone(g.inner()))
        .ok_or_else(|| "passerelle indisponible".to_owned())
}

/// « Se connecter »: the login window opens (or comes to the front if the login already runs). Answers at once;
/// the card follows the login through `snapshot-changed`.
#[tauri::command]
pub async fn connect_session(window: WebviewWindow, session: String) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let id = gateway.known_session(&session).map_err(|e| e.to_string())?;
    login::front_window_of(window.app_handle(), &id);
    tauri::async_runtime::spawn(async move {
        let _ = gateway.connect_session(&id).await;
    });
    Ok(())
}

/// « Déconnecter »: the card then shows « expirée ».
#[tauri::command]
pub async fn disconnect_session(window: WebviewWindow, session: String) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let id = gateway.known_session(&session).map_err(|e| e.to_string())?;
    gateway.disconnect_session(&id).await;
    Ok(())
}

/// The profile chosen at first launch, or confirmed in the change dialog.
#[tauri::command]
pub async fn set_profile(window: WebviewWindow, profile: String) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let id = gateway.known_profile(&profile).map_err(|e| e.to_string())?;
    gateway.set_profile(&id).await.map_err(|e| e.to_string())
}

/// « Reprendre » in the banner of the suspended gateway.
#[tauri::command]
pub async fn resume_gateway(window: WebviewWindow) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    gateway.resume().await.map_err(|e| e.to_string())
}

/// « Voir » in the banner of the waiting confirmations: their detail windows (spec 013). Nothing is answered.
#[tauri::command]
pub async fn show_confirmations(window: WebviewWindow) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    gateway.show_confirmations().await;
    Ok(())
}

/// A pastille of the MCP tab: exposes the environment, or withdraws it.
#[tauri::command]
pub async fn set_environment_exposed(window: WebviewWindow, environment: String, exposed: bool) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let id = gateway.known_environment(&environment).map_err(|e| e.to_string())?;
    gateway
        .set_environment_exposed(&id, exposed)
        .await
        .map_err(|e| e.to_string())
}

/// The switch of a row of the MCP tab: cuts the MCP in every environment, or puts it back.
#[tauri::command]
pub async fn set_mcp_exposed(window: WebviewWindow, mcp: String, exposed: bool) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let id = gateway.known_mcp(&mcp).map_err(|e| e.to_string())?;
    gateway.set_mcp_exposed(&id, exposed).await.map_err(|e| e.to_string())
}

/// « J'ai vu » in the banner of an updated MCP.
#[tauri::command]
pub async fn acknowledge_mcp_update(window: WebviewWindow, environment: String, mcp: String) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let instance = gateway.known_instance(&environment, &mcp).map_err(|e| e.to_string())?;
    gateway.acknowledge_mcp_update(&instance);
    Ok(())
}

/// « Recharger » in the row of an exposed instance: its list is read again.
#[tauri::command]
pub async fn reload_mcp(window: WebviewWindow, environment: String, mcp: String) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    let instance = gateway.known_instance(&environment, &mcp).map_err(|e| e.to_string())?;
    gateway.reload_mcp(&instance).await.map_err(|e| e.to_string())
}

/// The Journal tab: every entry of the buffer, oldest first, filtered once more. The later ones come by
/// `journal-entry` (or `journal-resync`, after which the window asks again).
#[tauri::command]
pub async fn get_journal(window: WebviewWindow) -> Result<serde_json::Value, String> {
    let gateway = gateway_for(&window)?;
    gateway
        .journal_json()
        .ok_or_else(|| "filtre de sortie indisponible : journal non transmis".to_owned())
}

/// « Exporter un diagnostic »: the save dialog of Windows, then the archive where the user chose. The page never
/// gives a path: it only learns where the archive went, or that the dialog was closed.
#[tauri::command]
pub async fn export_diagnostic(window: WebviewWindow) -> Result<ExportOutcome, String> {
    let gateway = gateway_for(&window)?;
    let profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let choose = dialog::save_archive(
        &window,
        &gateway.diagnostic_file_name(),
        default_export_dir(profile.as_deref()),
    );
    let chosen = tauri::async_runtime::spawn_blocking(choose)
        .await
        .map_err(|_| "dialogue d'enregistrement interrompu".to_owned())?;
    let Some(chosen) = chosen else {
        return Ok(ExportOutcome::Cancelled);
    };
    let path = gateway.export_diagnostic(&chosen).await.map_err(|e| e.to_string())?;
    Ok(ExportOutcome::Saved {
        path: path.display().to_string(),
    })
}

/// « Redémarrer pour mettre à jour », once the window asked: the installed exe, already replaced, is started again
/// and this process stops. Refused without an update put in place.
#[tauri::command]
pub async fn restart_for_update(window: WebviewWindow) -> Result<(), String> {
    let gateway = gateway_for(&window)?;
    gateway.restart_for_update().map_err(|e| e.to_string())
}
