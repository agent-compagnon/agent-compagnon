//! Commands of the main window (spec 016), each declared in `build.rs` and granted to the window `main` alone by
//! `capabilities/main.json`. Wiring only: the checks and the effects live in `mcp_proxy_gateway::interface`. An
//! error is a French text without any secret, shown by the window.

use std::sync::Arc;

use mcp_proxy_gateway::control::GatewayControl;
use mcp_proxy_gateway::runtime::Gateway;
use tauri::{Manager, WebviewWindow};

use crate::app::MAIN_WINDOW;
use crate::login;

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
