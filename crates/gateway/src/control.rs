//! The two emergency gestures of the icon menu (spec 015), for the application to call: « Suspendre la passerelle »
//! / « Reprendre », and « Tout déconnecter ». Neither asks for a confirmation: they must be immediate.

use async_trait::async_trait;
use mcp_proxy_auth::AuthUiError;
use mcp_proxy_core::settings::SettingsError;

use crate::runtime::Gateway;
use crate::view::{Alert, AlertKind, AlertLevel};

#[async_trait]
pub trait GatewayControl: Send + Sync {
    /// Every `tools/call` is refused from now on, those waiting for a confirmation or a login included; the tool
    /// list does not change. Saved: a restart does not lift it. A save failure is returned (and shown as an alert)
    /// once the suspension applies: it then holds until the application stops.
    async fn suspend(&self) -> Result<(), SettingsError>;

    /// Calls pass again. Saved like the suspension.
    async fn resume(&self) -> Result<(), SettingsError>;

    /// No secret any more, every session « jamais connectée », calls waiting for a login refused with a clear
    /// message, browsing data of the login windows cleared and their windows closed. The error is the one of the
    /// browsing data, cleared after the secrets.
    async fn disconnect_all(&self) -> Result<(), AuthUiError>;

    fn is_suspended(&self) -> bool;
}

impl Gateway {
    async fn set_suspended(&self, suspended: bool) -> Result<(), SettingsError> {
        let saved = self.change_settings(|s| s.gateway_suspended = suspended).await;
        let (level, summary) = if suspended {
            (tracing::Level::WARN, "passerelle suspendue par l'utilisateur")
        } else {
            (tracing::Level::INFO, "passerelle reprise par l'utilisateur")
        };
        self.journal().record_event(level, summary);
        if let Err(error) = &saved {
            tracing::error!(%error, suspended, "suspension appliquée mais non enregistrée");
            let message = if suspended {
                format!(
                    "La passerelle est suspendue jusqu'à l'arrêt de MCP Proxy, mais {error} : un redémarrage la reprendrait."
                )
            } else {
                format!("La passerelle a repris, mais {error} : elle sera de nouveau suspendue au prochain démarrage.")
            };
            self.add_alert(Alert {
                level: AlertLevel::Error,
                kind: AlertKind::SettingsNotSaved,
                message,
            });
        }
        saved
    }
}

#[async_trait]
impl GatewayControl for Gateway {
    async fn suspend(&self) -> Result<(), SettingsError> {
        if self.is_suspended() {
            return Ok(());
        }
        let saved = self.set_suspended(true).await;
        // The chain already refuses the calls it was waiting for; whatever still waits for an answer is refused too.
        self.confirmations().pending().refuse_all();
        saved
    }

    async fn resume(&self) -> Result<(), SettingsError> {
        if !self.is_suspended() {
            return Ok(());
        }
        self.set_suspended(false).await
    }

    async fn disconnect_all(&self) -> Result<(), AuthUiError> {
        let cleared = self.authenticator().disconnect_all().await;
        let summary = match &cleared {
            Ok(()) => "toutes les sessions déconnectées par l'utilisateur".to_owned(),
            Err(error) => format!("toutes les sessions déconnectées par l'utilisateur ({error})"),
        };
        self.journal().record_event(tracing::Level::WARN, &summary);
        cleared
    }

    fn is_suspended(&self) -> bool {
        self.settings().gateway_suspended
    }
}
