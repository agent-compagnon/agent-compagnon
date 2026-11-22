//! What the main window asks for (spec 016): log in to a session, disconnect it, choose the profile, resume the
//! gateway, show the waiting confirmations. The application only passes the arguments of its commands on; every
//! check and every effect is here.
//!
//! The arguments come from the page: they are checked against the configuration and never echoed back. No command
//! answers a confirmation: the detail window of spec 013 stays the only place where one is approved.

use mcp_proxy_core::config::{ProfileId, SessionId};
use mcp_proxy_core::session::SessionError;
use thiserror::Error;

use crate::runtime::Gateway;
use crate::view::{Alert, AlertKind, AlertLevel};

/// Why a command of the window was refused or only half done. French, shown in the window, without any secret.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InterfaceError {
    #[error("session inconnue")]
    UnknownSession,
    #[error("profil inconnu")]
    UnknownProfile,
    /// The profile applies, but the settings file keeps the previous one.
    #[error("le profil s'applique jusqu'à l'arrêt de MCP Proxy, mais {0}")]
    ProfileNotSaved(String),
}

impl Gateway {
    /// The session of the configuration named `raw`.
    pub fn known_session(&self, raw: &str) -> Result<SessionId, InterfaceError> {
        SessionId::new(raw)
            .ok()
            .filter(|id| self.config().sessions.iter().any(|s| &s.id == id))
            .ok_or(InterfaceError::UnknownSession)
    }

    /// The profile of the configuration named `raw`.
    pub fn known_profile(&self, raw: &str) -> Result<ProfileId, InterfaceError> {
        ProfileId::new(raw)
            .ok()
            .filter(|id| self.config().profiles.contains_key(id))
            .ok_or(InterfaceError::UnknownProfile)
    }

    /// « Se connecter »: starts the login of `session`, or joins the one in progress, and waits for its end. No
    /// notification: the user asked for it.
    pub async fn connect_session(&self, session: &SessionId) -> Result<(), SessionError> {
        let ended = self.authenticator().connect(session).await;
        if let Err(error) = &ended {
            tracing::info!(session = session.as_str(), %error, "connexion demandée depuis la fenêtre non aboutie");
        }
        ended
    }

    /// « Déconnecter »: the secret is wiped, the session shows « expirée », the site data of its hosts is cleared, a
    /// login in progress is cancelled.
    pub async fn disconnect_session(&self, session: &SessionId) {
        self.authenticator().disconnect(session).await;
    }

    /// The profile chosen in the window (first launch or change, after the user confirmed): the exposed
    /// environments go back to the defaults of the profile, the hardening of the user is kept. The banner of a
    /// settings reset, which asked for this choice, goes away. A save failure is returned once the profile applies,
    /// and stays shown as an alert.
    pub async fn set_profile(&self, profile: &ProfileId) -> Result<(), InterfaceError> {
        let config = std::sync::Arc::clone(self.config());
        let chosen = config.profiles.get(profile).ok_or(InterfaceError::UnknownProfile)?;
        let saved = self.change_settings(|s| s.choose_profile(chosen)).await;
        self.journal().record_event(
            tracing::Level::INFO,
            &format!("profil « {} » choisi par l'utilisateur", chosen.label),
        );
        self.remove_alerts(AlertKind::SettingsReset);
        match saved {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::error!(%error, "profil appliqué mais non enregistré");
                self.add_alert(Alert {
                    level: AlertLevel::Error,
                    kind: AlertKind::SettingsNotSaved,
                    message: format!(
                        "Le profil « {} » s'applique jusqu'à l'arrêt de MCP Proxy, mais {error}.",
                        chosen.label
                    ),
                });
                Err(InterfaceError::ProfileNotSaved(error.to_string()))
            }
        }
    }

    /// « Voir » in the banner of the waiting confirmations: their detail windows. Nothing is approved here.
    pub async fn show_confirmations(&self) {
        self.confirmations().show_waiting_details().await;
    }
}
