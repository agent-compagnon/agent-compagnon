//! What the main window asks for: log in to a session, disconnect it, choose the profile, resume the gateway, show
//! the waiting confirmations (spec 016); expose an environment, cut an MCP, acknowledge the update of an MCP, reload
//! its list (spec 017); export a diagnostic to the path the user chose in the save dialog (spec 018). The application only passes the arguments of its commands on; every check and every effect
//! is here.
//!
//! The arguments come from the page: they are checked against the configuration and never echoed back. No command
//! answers a confirmation: the detail window of spec 013 stays the only place where one is approved.

use std::path::{Path, PathBuf};

use mcp_proxy_core::config::{EnvId, McpId, ProfileId, SessionId};
use mcp_proxy_core::diagnostics::{DiagnosticState, Diagnostics, archive_name};
use mcp_proxy_core::session::SessionError;
use serde::Serialize;
use thiserror::Error;

use crate::journal::instance_lines;
use crate::registry::InstanceId;
use crate::runtime::Gateway;
use crate::view::{Alert, AlertKind, AlertLevel};

/// Why a command of the window was refused or only half done. French, shown in the window, without any secret.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InterfaceError {
    #[error("session inconnue")]
    UnknownSession,
    #[error("profil inconnu")]
    UnknownProfile,
    #[error("environnement inconnu")]
    UnknownEnvironment,
    #[error("MCP inconnu")]
    UnknownMcp,
    #[error("ce MCP n'existe pas dans cet environnement")]
    UnknownInstance,
    /// Nothing is exposed before a profile is chosen (spec 005): exposing would not take effect.
    #[error("choisissez d'abord un profil")]
    NoProfile,
    /// A reload contacts the server: only for what the user exposes (no traffic to other environments).
    #[error("ce MCP n'est pas exposé dans cet environnement, sa liste n'est pas rechargée")]
    NotExposed,
    #[error("ce MCP est coupé, sa liste n'est pas rechargée")]
    Disabled,
    /// The profile applies, but the settings file keeps the previous one.
    #[error("le profil s'applique jusqu'à l'arrêt de MCP Proxy, mais {0}")]
    ProfileNotSaved(String),
    /// The change applies, but the settings file keeps the previous settings.
    #[error("le changement s'applique jusqu'à l'arrêt de MCP Proxy, mais {0}")]
    NotSaved(String),
    /// « Redémarrer pour mettre à jour » without an update put in place (spec 020).
    #[error("aucune mise à jour prête à appliquer")]
    NoUpdateReady,
    /// `DiagnosticError`, already French and without content (« export de diagnostic impossible : … »).
    #[error("{0}")]
    Export(String),
}

/// How « Exporter un diagnostic » ended, as the window shows it. Serialized `{ "status": "saved", "path": … }` or
/// `{ "status": "cancelled" }`; a failure is the error of the command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ExportOutcome {
    /// The archive is at `path`, the one the user chose (`.zip` added when missing).
    Saved { path: String },
    /// The user closed the save dialog: nothing written, nothing to say.
    Cancelled,
}

/// The archive is a zip: a name chosen without that extension gets it, so that Windows opens it.
pub fn archive_path(chosen: &Path) -> PathBuf {
    let zip = chosen
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if zip {
        chosen.to_path_buf()
    } else {
        let mut name = chosen.as_os_str().to_owned();
        name.push(".zip");
        PathBuf::from(name)
    }
}

/// What the diagnostic archive says of the system (`systeme.txt`): `Windows 11 Pro 24H2 (build 26100.2314),
/// WebView2 131.0.2903.70`, from the values the application reads in the registry; « inconnu » for a missing one.
pub fn describe_system(product: Option<&str>, release: Option<&str>, build: Option<&str>, webview: &str) -> String {
    let product = product
        .filter(|p| !p.trim().is_empty())
        .unwrap_or("Windows (version inconnue)");
    let mut text = product.trim().to_owned();
    if let Some(release) = release.filter(|r| !r.trim().is_empty()) {
        text.push(' ');
        text.push_str(release.trim());
    }
    text.push_str(&format!(
        " (build {})",
        build.filter(|b| !b.trim().is_empty()).unwrap_or("inconnu")
    ));
    text.push_str(&format!(", WebView2 {webview}"));
    text
}

/// Where the save dialog opens: the « Téléchargements » folder of the user when it exists, else where Windows
/// decides.
pub fn default_export_dir(user_profile: Option<&Path>) -> Option<PathBuf> {
    let downloads = user_profile?.join("Downloads");
    downloads.is_dir().then_some(downloads)
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

    /// The environment of the configuration named `raw`.
    pub fn known_environment(&self, raw: &str) -> Result<EnvId, InterfaceError> {
        EnvId::new(raw)
            .ok()
            .filter(|id| self.config().environment(id).is_some())
            .ok_or(InterfaceError::UnknownEnvironment)
    }

    /// The MCP of the configuration named `raw`.
    pub fn known_mcp(&self, raw: &str) -> Result<McpId, InterfaceError> {
        McpId::new(raw)
            .ok()
            .filter(|id| self.config().mcp(id).is_some())
            .ok_or(InterfaceError::UnknownMcp)
    }

    /// The instance of the MCP `mcp` in the environment `env`, both of the configuration.
    pub fn known_instance(&self, env: &str, mcp: &str) -> Result<InstanceId, InterfaceError> {
        let env = self.known_environment(env)?;
        let mcp = self.known_mcp(mcp)?;
        let declared = self.config().mcp(&mcp).is_some_and(|m| m.instances.contains_key(&env));
        if declared {
            Ok(InstanceId::new(env, mcp))
        } else {
            Err(InterfaceError::UnknownInstance)
        }
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

    /// A pastille of the MCP tab: exposes `env` to the LLM clients, or withdraws it. Exposing needs a chosen profile,
    /// whose rules apply there as everywhere; withdrawing is always possible. Asking for the current state changes
    /// nothing and writes nothing.
    pub async fn set_environment_exposed(&self, env: &EnvId, exposed: bool) -> Result<(), InterfaceError> {
        let config = std::sync::Arc::clone(self.config());
        let label = config
            .environment(env)
            .map(|e| e.label.clone())
            .ok_or(InterfaceError::UnknownEnvironment)?;
        let settings = self.settings();
        if exposed && settings.chosen_profile(&config).is_none() {
            return Err(InterfaceError::NoProfile);
        }
        if settings.exposed_environments.contains(env) == exposed {
            return Ok(());
        }
        let saved = self
            .change_settings(|s| {
                if exposed {
                    s.exposed_environments.insert(env.clone());
                } else {
                    s.exposed_environments.remove(env);
                }
            })
            .await;
        let text = if exposed {
            format!("environnement « {label} » exposé par l'utilisateur")
        } else {
            format!("environnement « {label} » retiré par l'utilisateur")
        };
        self.journal().record_event(tracing::Level::INFO, &text);
        self.saved_or_alert(saved, &format!("L'exposition de {label}"))
    }

    /// The switch of a row of the MCP tab: cuts `mcp` (in every environment, spec 004) or puts it back.
    pub async fn set_mcp_exposed(&self, mcp: &McpId, exposed: bool) -> Result<(), InterfaceError> {
        let config = std::sync::Arc::clone(self.config());
        let label = config
            .mcp(mcp)
            .map(|m| m.label.clone())
            .ok_or(InterfaceError::UnknownMcp)?;
        if self.settings().disabled_mcp.contains(mcp) != exposed {
            return Ok(());
        }
        let saved = self
            .change_settings(|s| {
                if exposed {
                    s.disabled_mcp.remove(mcp);
                } else {
                    s.disabled_mcp.insert(mcp.clone());
                }
            })
            .await;
        let text = if exposed {
            format!("MCP « {label} » rétabli par l'utilisateur")
        } else {
            format!("MCP « {label} » coupé par l'utilisateur")
        };
        self.journal().record_event(tracing::Level::INFO, &text);
        self.saved_or_alert(saved, &format!("La coupure de {label}"))
    }

    /// « J'ai vu » in the banner of an updated MCP: its pending update is forgotten. Nothing pending: nothing to do.
    pub fn acknowledge_mcp_update(&self, instance: &InstanceId) {
        self.watcher().tracker().acknowledge(instance);
    }

    /// « Recharger » in the row of an instance: its list is read again now. Only for an exposed instance whose MCP
    /// is not cut: the user chose to reach that server.
    pub async fn reload_mcp(&self, instance: &InstanceId) -> Result<(), InterfaceError> {
        let settings = self.settings();
        if settings.disabled_mcp.contains(&instance.mcp) {
            return Err(InterfaceError::Disabled);
        }
        if !self.registry().read().await.exposure().exposes(instance) {
            return Err(InterfaceError::NotExposed);
        }
        if self.watcher().reload(instance) {
            Ok(())
        } else {
            Err(InterfaceError::UnknownInstance)
        }
    }

    /// A save failure of a change that applies anyway: an alert, and the error for the window.
    fn saved_or_alert(
        &self,
        saved: Result<(), mcp_proxy_core::settings::SettingsError>,
        what: &str,
    ) -> Result<(), InterfaceError> {
        match saved {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::error!(%error, "réglage appliqué mais non enregistré");
                self.add_alert(Alert {
                    level: AlertLevel::Error,
                    kind: AlertKind::SettingsNotSaved,
                    message: format!("{what} s'applique jusqu'à l'arrêt de MCP Proxy, mais {error}."),
                });
                Err(InterfaceError::NotSaved(error.to_string()))
            }
        }
    }

    /// The name the save dialog proposes: `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip`, UTC (spec 009).
    pub fn diagnostic_file_name(&self) -> String {
        archive_name(self.clock().now())
    }

    /// « Exporter un diagnostic », once the user chose `chosen` in the save dialog: the archive of spec 009 (logs
    /// filtered again, settings, state of the instances, last self-diagnostic report), written there. The journal
    /// records the export, not its path. Runs off the async threads: the export reads and compresses the logs.
    pub async fn export_diagnostic(&self, chosen: &Path) -> Result<PathBuf, InterfaceError> {
        let diagnostics = Diagnostics {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            system: self.system().to_owned(),
            config: std::sync::Arc::clone(self.config()),
            log_dir: self.log_dir().to_path_buf(),
            filter: std::sync::Arc::clone(self.journal().filter()),
            clock: std::sync::Arc::clone(self.clock()),
            log: self.journal().log().cloned(),
        };
        let state = DiagnosticState {
            settings: self.settings(),
            instances: instance_lines(self.watcher().tracker()),
            doctor_report: None,
        };
        let path = archive_path(chosen);
        let written = tokio::task::spawn_blocking(move || diagnostics.export(&path, &state))
            .await
            .map_err(|_| InterfaceError::Export("export de diagnostic impossible : tâche interrompue".to_owned()))?;
        match written {
            Ok(path) => {
                self.journal()
                    .record_event(tracing::Level::INFO, "diagnostic exporté par l'utilisateur");
                Ok(path)
            }
            Err(error) => {
                self.journal().record_event(tracing::Level::WARN, &error.to_string());
                Err(InterfaceError::Export(error.to_string()))
            }
        }
    }

    /// « Voir » in the banner of the waiting confirmations: their detail windows. Nothing is approved here.
    pub async fn show_confirmations(&self) {
        self.confirmations().show_waiting_details().await;
    }
}
