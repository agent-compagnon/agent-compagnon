//! The checks of spec 024. Each reads what it needs and says what it found; none repairs, creates or rewrites
//! anything. The reads of the platform go through traits; what the gateway knows (configuration, settings,
//! WebView2) comes as facts.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use super::clock::Dates;
use super::report::{CheckId, Finding, Viewpoint};
use super::web::{Web, WebError};
use super::{Check, blocking};
use crate::clients::{ClientFiles, Environment, LocationId, State, inspect_all};
use crate::install::{BACKUP_NAME, InstallPaths, STAGED_NAME, is_enabled, same_location};
use crate::ops::{OpError, PathKind, SystemOps};
use crate::uninstall::{command_exe, entry_for};
use crate::update::{Channel, Source, choose, is_dev_key, parse_listing};
use crate::version::{Installed, Version, is_newer};

const SUPPORT: &str = "Joignez ce rapport à votre demande au support.";

// ---------------------------------------------------------------------------------------------------------- Config

/// What the gateway learnt from the embedded configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFacts {
    /// Addresses whose host is a reserved example name (`example.internal`, `.example`, `.test`, `.invalid`…).
    pub example_hosts: usize,
    pub environments: usize,
    pub mcp: usize,
}

/// « Config »: loaded and valid.
pub struct ConfigCheck {
    /// The validation messages of the configuration compiled into the exe otherwise.
    pub loaded: Result<ConfigFacts, Vec<String>>,
}

#[async_trait]
impl Check for ConfigCheck {
    fn id(&self) -> CheckId {
        CheckId::Config
    }

    async fn run(&self) -> Vec<Finding> {
        match &self.loaded {
            Err(errors) => {
                let mut detail = errors.iter().take(5).cloned().collect::<Vec<_>>().join(" ; ");
                if errors.len() > 5 {
                    detail.push_str(&format!(" ; et {} autres", errors.len() - 5));
                }
                vec![Finding::failure(
                    format!("configuration embarquée invalide : {detail}"),
                    format!(
                        "Cet exe a été construit avec une configuration refusée : MCP Proxy ne démarre pas. \
                         Installez une version publiée. {SUPPORT}"
                    ),
                )]
            }
            Ok(facts) if facts.example_hosts > 0 => vec![Finding::warning(
                format!(
                    "configuration chargée et valide, mais c'est celle d'exemple : {} adresses désignent des noms \
                     réservés (example.internal…), qui ne mènent à aucun serveur réel",
                    facts.example_hosts
                ),
                "Installez la version publiée pour votre entreprise : elle embarque la vraie configuration.",
            )],
            Ok(facts) => vec![Finding::ok(format!(
                "configuration chargée et valide ({} environnements, {} MCP)",
                facts.environments, facts.mcp
            ))],
        }
    }
}

// --------------------------------------------------------------------------------------------------------- WebView2

/// « WebView2 »: the runtime and its version.
pub struct WebviewCheck {
    /// `tauri::webview_version()`; `None`: no runtime.
    pub version: Option<String>,
    /// The version supports InPrivate (`MIN_RUNTIME_VERSION` of the login windows, spec 012).
    pub sufficient: bool,
    /// `webview.profile = "in_memory"` (the default): an old runtime refuses every login window.
    pub in_memory: bool,
}

#[async_trait]
impl Check for WebviewCheck {
    fn id(&self) -> CheckId {
        CheckId::Webview
    }

    async fn run(&self) -> Vec<Finding> {
        let install = "Installez ou mettez à jour le « Microsoft Edge WebView2 Runtime » (programme d'installation \
                       « Evergreen » de Microsoft, sans droits d'administrateur), puis relancez MCP Proxy.";
        let Some(version) = self.version.as_deref().filter(|v| !v.trim().is_empty()) else {
            return vec![Finding::failure(
                "runtime WebView2 absent : MCP Proxy ne peut ouvrir aucune fenêtre",
                install,
            )];
        };
        let version = version.trim();
        if self.sufficient {
            vec![Finding::ok(format!(
                "runtime présent (version {version}), suffisant pour les connexions sans trace (InPrivate)"
            ))]
        } else if self.in_memory {
            vec![Finding::failure(
                format!(
                    "runtime trop ancien (version {version}) pour une connexion sans trace : toute connexion par \
                     fenêtre est refusée"
                ),
                install,
            )]
        } else {
            vec![Finding::warning(
                format!(
                    "runtime trop ancien (version {version}) pour le mode InPrivate ; la configuration se replie sur \
                     un profil temporaire sur disque"
                ),
                install,
            )]
        }
    }
}

// ----------------------------------------------------------------------------------------------------- Installation

/// « Installation »: the exe where it is expected, its folder, the shortcut, the AUMID, the exclusion from error
/// reports, the `Run` value and the entry « Applications installées ». Registry and files: read only.
pub struct InstallationCheck {
    pub ops: Arc<dyn SystemOps>,
    /// `None` without `%LOCALAPPDATA%` or `%APPDATA%`.
    pub paths: Option<InstallPaths>,
    /// This exe.
    pub here: Option<PathBuf>,
}

fn registry_failure(what: &str, error: &OpError) -> Finding {
    Finding::failure(
        format!("{what} illisible dans le registre ({error})"),
        format!("Un logiciel de sécurité ou une stratégie de l'entreprise bloque peut-être la lecture. {SUPPORT}"),
    )
}

fn installation(ops: &dyn SystemOps, paths: &InstallPaths, here: Option<&PathBuf>) -> Vec<Finding> {
    let reinstall = "Désinstallez MCP Proxy depuis les paramètres de Windows (« Applications installées »), puis \
                     lancez l'exe téléchargé et choisissez « Installer ».";
    let restart = "Quittez MCP Proxy (menu de l'icône, « Quitter ») puis relancez-le depuis le menu Démarrer : l'exe \
                   installé le répare à son démarrage.";
    let mut findings = Vec::new();
    let installed = ops.canonical(&paths.exe);
    let here_is_installed = match (&installed, here.and_then(|h| ops.canonical(h))) {
        (Some(installed), Some(here)) => same_location(installed, &here),
        _ => false,
    };
    match ops.kind(&paths.exe) {
        Ok(PathKind::Missing) => {
            return vec![Finding::warning(
                "MCP Proxy n'est pas installé sur ce compte : il tourne depuis l'emplacement de l'exe lancé",
                "Lancez l'exe téléchargé et choisissez « Installer » : les clients LLM, le démarrage avec Windows et \
                 les mises à jour supposent l'exe installé.",
            )];
        }
        Ok(PathKind::File) => {}
        Ok(_) => {
            return vec![Finding::failure(
                "l'emplacement de l'exe installé n'est pas un simple fichier (dossier ou lien)",
                reinstall,
            )];
        }
        Err(error) => {
            return vec![Finding::failure(
                format!("dossier d'installation illisible ({error})"),
                reinstall,
            )];
        }
    }
    let version = match ops.exe_version(&paths.exe) {
        Installed::Known(version) => version,
        Installed::Unknown => {
            findings.push(Finding::failure(
                "version de l'exe installé illisible : l'installation est abîmée et ne sera jamais mise à jour",
                reinstall,
            ));
            return findings;
        }
    };
    let mut present = vec![format!("exe installé en version {version}")];
    if here_is_installed {
        present.push("lancé depuis l'installation".to_owned());
    } else {
        findings.push(Finding::ok(
            "cet exe n'est pas l'exe installé : les contrôles portent sur l'installation",
        ));
    }

    // Writable without writing: the read-only attribute of the folder and of the exe.
    let read_only = [&paths.dir, &paths.exe]
        .into_iter()
        .any(|path| ops.read_only(path).unwrap_or(false));
    if read_only {
        findings.push(Finding::failure(
            "le dossier d'installation ou l'exe est en lecture seule : les mises à jour ne peuvent pas le remplacer",
            "Dans l'Explorateur, propriétés de %LOCALAPPDATA%\\Programs\\MCP Proxy et de mcp-proxy.exe : décochez « \
             Lecture seule ».",
        ));
    } else {
        present.push("dossier sans lecture seule".to_owned());
    }

    for (path, name) in [(&paths.backup, BACKUP_NAME), (&paths.staged, STAGED_NAME)] {
        if ops.exists(path) {
            findings.push(Finding::warning(
                format!("reste d'une mise à jour dans le dossier d'installation : {name}"),
                "Fermez les clients LLM (Claude Desktop, VS Code), puis quittez et relancez MCP Proxy : l'exe \
                 installé le supprime à son démarrage.",
            ));
        }
    }

    if ops.exists(&paths.shortcut) {
        present.push("raccourci du menu Démarrer".to_owned());
    } else {
        findings.push(Finding::warning(
            "raccourci « MCP Proxy » absent du menu Démarrer",
            "La prochaine mise à jour le recrée ; sinon, désinstallez puis réinstallez MCP Proxy.",
        ));
    }

    match ops.aumid_registered() {
        Ok(true) => present.push("identité des notifications (AUMID)".to_owned()),
        Ok(false) => findings.push(Finding::warning(
            "identité des notifications (AUMID) absente du registre : les confirmations s'ouvrent en fenêtre",
            restart,
        )),
        Err(error) => findings.push(registry_failure("identité des notifications (AUMID)", &error)),
    }

    match ops.wer_exclusion() {
        Ok(Some(1)) => present.push("exclusion des rapports d'erreur Windows".to_owned()),
        Ok(_) => findings.push(Finding::warning(
            "exclusion des rapports d'erreur Windows absente : un plantage pourrait laisser une image mémoire sur le \
             disque",
            format!(
                "{restart} Si l'avertissement reste, une stratégie de l'entreprise refuse cette valeur du registre."
            ),
        )),
        Err(error) => findings.push(registry_failure("exclusion des rapports d'erreur Windows", &error)),
    }

    match ops.run_entry() {
        Ok(None) => present.push("démarrage avec Windows désactivé".to_owned()),
        Ok(Some(entry)) if is_enabled(Some(&entry), &paths.exe) => {
            present.push("démarrage avec Windows activé".to_owned());
        }
        Ok(Some(_)) => findings.push(Finding::warning(
            "la valeur « MCP Proxy » du démarrage de Windows lance un autre programme que l'exe installé",
            "Dans le menu de l'icône de MCP Proxy, décochez puis recochez « Démarrer avec Windows ».",
        )),
        Err(error) => findings.push(registry_failure("démarrage avec Windows", &error)),
    }

    match ops.uninstall_entry() {
        Ok(None) => findings.push(Finding::warning(
            "entrée « Applications installées » absente : MCP Proxy ne se désinstalle pas depuis les paramètres de \
             Windows",
            restart,
        )),
        Ok(Some(entry)) => {
            let ours = same_location(std::path::Path::new(command_exe(&entry.uninstall_string)), &paths.exe);
            let expected = ops
                .file_len(&paths.exe)
                .ok()
                .and_then(|size| entry_for(paths, Some(&version), size));
            if !ours {
                findings.push(Finding::warning(
                    "l'entrée « Applications installées » désinstalle un autre exe que l'exe installé",
                    restart,
                ));
            } else if entry.display_version.as_deref() != Some(version.to_string().as_str()) {
                findings.push(Finding::warning(
                    format!(
                        "l'entrée « Applications installées » affiche la version {} alors que l'exe installé est en \
                         version {version}",
                        entry.display_version.as_deref().unwrap_or("(aucune)")
                    ),
                    restart,
                ));
            } else if expected.is_some_and(|e| e != entry) {
                findings.push(Finding::warning(
                    "l'entrée « Applications installées » ne correspond plus à l'installation",
                    restart,
                ));
            } else {
                present.push(format!("entrée « Applications installées » en version {version}"));
            }
        }
        Err(error) => findings.push(registry_failure("entrée « Applications installées »", &error)),
    }

    findings.insert(0, Finding::ok(present.join(", ")));
    findings
}

#[async_trait]
impl Check for InstallationCheck {
    fn id(&self) -> CheckId {
        CheckId::Installation
    }

    async fn run(&self) -> Vec<Finding> {
        let Some(paths) = self.paths.clone() else {
            return vec![Finding::failure(
                "dossiers du compte Windows introuvables (%LOCALAPPDATA%, %APPDATA%)",
                format!("Ouvrez une nouvelle session Windows. {SUPPORT}"),
            )];
        };
        let ops = Arc::clone(&self.ops);
        let here = self.here.clone();
        blocking(move || installation(ops.as_ref(), &paths, here.as_ref())).await
    }
}

// ---------------------------------------------------------------------------------------------------- Notifications

/// `ToastNotifier::Setting` and `ToastNotificationManagerForUser::NotificationMode`, as WinRT gives them (`None`:
/// unreadable). Read only: nothing is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toasts {
    pub setting: Option<i32>,
    pub mode: Option<i32>,
}

pub trait ToastProbe: Send + Sync {
    fn toasts(&self) -> Toasts;
}

/// « Notifications »: toasts enabled for the application.
pub struct NotificationsCheck {
    pub probe: Arc<dyn ToastProbe>,
}

fn notifications(toasts: Toasts) -> Vec<Finding> {
    let settings = "Paramètres de Windows, « Système », « Notifications » : activez les notifications, puis celles de \
                    « MCP Proxy » dans la liste des applications.";
    let fallback = "les confirmations s'ouvrent dans une fenêtre au premier plan à la place";
    match (toasts.setting, toasts.mode) {
        (None, _) | (_, None) => vec![Finding::failure(
            format!("état des notifications illisible : Windows ne dit pas si elles s'affichent ; {fallback}"),
            format!("Redémarrez Windows. {SUPPORT}"),
        )],
        (Some(0), Some(0)) => vec![Finding::ok("notifications activées pour MCP Proxy")],
        (Some(0), Some(_)) => vec![Finding::warning(
            format!("mode « Ne pas déranger » actif : les notifications ne s'affichent pas ; {fallback}"),
            "Désactivez « Ne pas déranger » depuis le centre de notifications (en bas à droite de l'écran).",
        )],
        (Some(1), _) => vec![Finding::warning(
            format!("notifications désactivées pour MCP Proxy ; {fallback}"),
            settings,
        )],
        (Some(2), _) => vec![Finding::warning(
            format!("notifications désactivées pour toutes les applications ; {fallback}"),
            settings,
        )],
        (Some(3), _) => vec![Finding::warning(
            format!("notifications désactivées par une stratégie de l'entreprise ; {fallback}"),
            "Demandez au support informatique de les autoriser pour MCP Proxy.",
        )],
        (Some(_), _) => vec![Finding::warning(
            format!("notifications désactivées ; {fallback}"),
            settings,
        )],
    }
}

#[async_trait]
impl Check for NotificationsCheck {
    fn id(&self) -> CheckId {
        CheckId::Notifications
    }

    async fn run(&self) -> Vec<Finding> {
        let probe = Arc::clone(&self.probe);
        blocking(move || notifications(probe.toasts())).await
    }
}

// ------------------------------------------------------------------------------------------------------------ Appli

/// What asking the running instance gave, through the authenticated control channel (spec 019): a request without
/// effect, `ping`, after the mutual proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeProbe {
    /// No instance holds the pipe.
    NotRunning,
    /// Proved and answered; `version`: `None` for a version that does not know the request (it closes the channel
    /// after the proof).
    Answered { version: Option<String> },
    /// No user name: no pipe can exist (spec 011).
    NoUser,
    /// Every instance of the pipe stays busy.
    Busy,
    /// The proof failed: the holder of the pipe is not MCP Proxy of this user.
    Foreign,
    /// Any other failure, French, without a path.
    Failed(String),
}

#[async_trait]
pub trait InstanceProbe: Send + Sync {
    async fn probe(&self) -> PipeProbe;
}

/// « Appli »: instance running, pipe reachable. From inside the application, the instance is this one: the check
/// then says whether its own pipe answers as a client would reach it.
pub struct AppCheck {
    pub probe: Arc<dyn InstanceProbe>,
    pub viewpoint: Viewpoint,
    /// The version installed, compared to the one running.
    pub installed: Option<Version>,
}

#[async_trait]
impl Check for AppCheck {
    fn id(&self) -> CheckId {
        CheckId::App
    }

    async fn run(&self) -> Vec<Finding> {
        let restart = "Quittez MCP Proxy (menu de l'icône, « Quitter ») puis relancez-le depuis le menu Démarrer.";
        let inside = self.viewpoint == Viewpoint::App;
        match self.probe.probe().await {
            PipeProbe::Answered { version } => {
                let who = if inside { "cette instance" } else { "instance en cours" };
                let running = version.as_deref().and_then(|v| Version::parse(v).ok());
                match (&running, &self.installed) {
                    (Some(running), Some(installed)) if !inside && is_newer(installed, running) => {
                        vec![Finding::warning(
                            format!(
                                "{who} en version {running}, plus ancienne que l'exe installé ({installed}) : une mise \
                                 à jour attend son redémarrage"
                            ),
                            restart,
                        )]
                    }
                    (Some(running), _) => vec![Finding::ok(format!(
                        "{who} en version {running}, canal des clients LLM joignable"
                    ))],
                    (None, _) => vec![Finding::ok(format!(
                        "{who} d'une version antérieure à l'auto-diagnostic, canal des clients LLM joignable"
                    ))],
                }
            }
            PipeProbe::NotRunning if inside => vec![Finding::failure(
                "le canal de cette instance ne répond pas : les clients LLM ne peuvent pas s'y connecter",
                restart,
            )],
            PipeProbe::NotRunning => vec![Finding::warning(
                "MCP Proxy n'est pas lancé",
                "Lancez MCP Proxy depuis le menu Démarrer ; un client LLM le lance aussi de lui-même à sa première \
                 demande.",
            )],
            PipeProbe::NoUser => vec![Finding::failure(
                "nom d'utilisateur Windows introuvable (%USERNAME%) : aucun canal n'est possible",
                format!("Ouvrez une nouvelle session Windows. {SUPPORT}"),
            )],
            PipeProbe::Busy => vec![Finding::failure(
                "le canal de MCP Proxy reste occupé et ne répond pas",
                restart,
            )],
            PipeProbe::Foreign => vec![Finding::failure(
                "le canal de MCP Proxy est tenu par un programme qui n'est pas MCP Proxy de cet utilisateur",
                "Redémarrez Windows ; si l'échec reste, un autre programme occupe le canal : signalez-le au support.",
            )],
            PipeProbe::Failed(reason) => vec![Finding::failure(
                format!("le canal de MCP Proxy ne répond pas ({reason})"),
                restart,
            )],
        }
    }
}

// --------------------------------------------------------------------------------------------------------- Réglages

/// The seal of `settings.json` as a load without effect finds it (spec 014): no key is created, nothing is
/// rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealFacts {
    Valid,
    /// No settings yet: first launch.
    Missing,
    /// Refused (modified, unsealed, key missing or of another user…): why, without content.
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFacts {
    pub seal: SealFacts,
    /// The label of the chosen profile.
    pub profile: Option<String>,
    /// The labels of the exposed environments.
    pub exposed: Vec<String>,
    pub suspended: bool,
    /// Seen from the running application: its settings in memory still apply, the file is for the next start.
    pub running: bool,
}

/// « Réglages »: the seal holds, and what the settings say of what is exposed.
pub struct SettingsCheck {
    pub facts: SettingsFacts,
}

#[async_trait]
impl Check for SettingsCheck {
    fn id(&self) -> CheckId {
        CheckId::Settings
    }

    async fn run(&self) -> Vec<Finding> {
        let open = "Ouvrez la fenêtre de MCP Proxy et choisissez un profil.";
        let facts = &self.facts;
        let mut findings = Vec::new();
        match &facts.seal {
            SealFacts::Refused(reason) => findings.push(Finding::failure(
                if facts.running {
                    format!(
                        "fichier des réglages refusé ({reason}) : les réglages en cours s'appliquent jusqu'à l'arrêt, \
                         puis les plus stricts au prochain démarrage"
                    )
                } else {
                    format!(
                        "réglages refusés ({reason}) : les réglages les plus stricts s'appliquent, aucun environnement \
                         n'est exposé"
                    )
                },
                format!(
                    "{} Si cela se reproduit sans raison, un programme modifie le fichier des réglages : signalez-le \
                     au support.",
                    if facts.running {
                        "Choisissez de nouveau votre profil (sélecteur « Profil » de l'en-tête) : les réglages sont \
                         réenregistrés et scellés."
                    } else {
                        open
                    }
                ),
            )),
            SealFacts::Missing => findings.push(Finding::warning(
                "aucun réglage enregistré (premier lancement) : aucun environnement n'est exposé",
                open,
            )),
            SealFacts::Valid => {}
        }
        if facts.suspended {
            findings.push(Finding::warning(
                "passerelle suspendue : tous les appels des clients LLM sont refusés",
                "Menu de l'icône de MCP Proxy, « Reprendre », ou le bandeau de la fenêtre.",
            ));
        }
        if facts.seal == SealFacts::Valid {
            let valid = match &facts.profile {
                None => {
                    findings.push(Finding::warning(
                        "aucun profil choisi : aucun outil n'est exposé aux clients LLM",
                        open,
                    ));
                    "sceau des réglages valide".to_owned()
                }
                Some(profile) if facts.exposed.is_empty() => {
                    findings.push(Finding::warning(
                        "aucun environnement exposé : aucun outil n'est visible des clients LLM",
                        "Onglet « MCP » de la fenêtre : cliquez sur la pastille d'un environnement pour l'exposer.",
                    ));
                    format!("sceau des réglages valide ; profil « {profile} »")
                }
                Some(profile) => format!(
                    "sceau des réglages valide ; profil « {profile} », environnements exposés : {}",
                    facts.exposed.join(", ")
                ),
            };
            findings.insert(0, Finding::ok(valid));
        }
        findings
    }
}

// ------------------------------------------------------------------------------------------------------ Artifactory

/// « Artifactory »: the Storage API answers, and the latest version published on the channel of the user.
pub struct ArtifactoryCheck {
    pub web: Arc<dyn Web>,
    /// `[update]` of the configuration; `None` when it could not be read.
    pub source: Option<Source>,
    pub channel: Channel,
    /// The installed version when it is known, else this exe's.
    pub current: Version,
    /// The exe embeds an update key: it applies the updates itself.
    pub updates: bool,
    pub dates: Dates,
}

const LISTING_MAX: u64 = 1024 * 1024;

fn host_of(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        (None, _) => "?".to_owned(),
    }
}

fn network_fix(error: WebError) -> &'static str {
    match error {
        WebError::Dns => {
            "Vérifiez la connexion au réseau de l'entreprise (VPN) : le nom du serveur n'est connu que sur ce réseau."
        }
        WebError::Tls => {
            "Le certificat du serveur n'est pas reconnu par Windows (proxy d'inspection, certificat d'entreprise \
             absent du magasin de Windows) : signalez-le au support."
        }
        WebError::Timeout | WebError::Connect | WebError::Transport => {
            "Vérifiez la connexion au réseau de l'entreprise (VPN) et le proxy de Windows, puis relancez \
             l'auto-diagnostic."
        }
        WebError::Refused | WebError::TooLarge => SUPPORT,
    }
}

#[async_trait]
impl Check for ArtifactoryCheck {
    fn id(&self) -> CheckId {
        CheckId::Artifactory
    }

    async fn run(&self) -> Vec<Finding> {
        let Some(url) = self.source.as_ref().and_then(|s| s.listing_url().ok()) else {
            return vec![Finding::failure(
                "adresse d'Artifactory illisible dans la configuration",
                SUPPORT,
            )];
        };
        let host = host_of(&url);
        let answer = match self.web.get(&url, LISTING_MAX).await {
            Ok(answer) => answer,
            Err(error) => {
                return vec![Finding::failure(
                    format!("Artifactory ({host}) injoignable : {error}"),
                    network_fix(error),
                )];
            }
        };
        self.dates.record(answer.date.as_deref());
        match answer.status {
            200 => {}
            404 => {
                return vec![Finding::warning(
                    format!("Artifactory ({host}) joignable, mais le dossier des versions n'existe pas (HTTP 404)"),
                    "Aucune version n'est encore publiée, ou l'adresse de la configuration est fausse : signalez-le \
                     au support.",
                )];
            }
            status => {
                return vec![Finding::failure(
                    format!("Artifactory ({host}) répond HTTP {status} à la liste des versions"),
                    format!("Le dépôt n'est pas lisible sans identifiant, ou le serveur est en erreur. {SUPPORT}"),
                )];
            }
        }
        let Ok(versions) = parse_listing(&answer.body) else {
            return vec![Finding::failure(
                format!("Artifactory ({host}) joignable, mais sa liste des versions est illisible"),
                SUPPORT,
            )];
        };
        let latest = choose(&versions, self.channel, &Version::new(0, 0, 0));
        let channel = match self.channel {
            Channel::Stable => "canal stable",
            Channel::Beta => "canal des préversions",
        };
        match latest {
            None => vec![Finding::warning(
                format!("Artifactory ({host}) joignable, mais aucune version n'y est publiée ({channel})"),
                "Rien à faire tant qu'aucune version n'est publiée.",
            )],
            Some(latest) if is_newer(&latest, &self.current) && !self.updates => vec![Finding::warning(
                format!(
                    "Artifactory ({host}) joignable ; la version {latest} est disponible (version en place : {}), mais \
                     cet exe ne se met pas à jour seul",
                    self.current
                ),
                "Téléchargez la dernière version publiée et lancez-la : elle propose de mettre à jour l'installation.",
            )],
            Some(latest) if is_newer(&latest, &self.current) => vec![Finding::ok(format!(
                "Artifactory ({host}) joignable ; la version {latest} est disponible et sera installée par la mise \
                 à jour automatique (version en place : {})",
                self.current
            ))],
            Some(latest) => vec![Finding::ok(format!(
                "Artifactory ({host}) joignable ; à jour (dernière version publiée : {latest}, {channel})"
            ))],
        }
    }
}

// ----------------------------------------------------------------------------------------------- Clé de mise à jour

/// « Clé de mise à jour »: the slots embedded in this exe (spec 020), never the development key.
pub struct UpdateKeyCheck {
    pub slots: [Option<[u8; 32]>; 2],
}

impl UpdateKeyCheck {
    /// At least one key that is not the development key: the exe updates itself.
    pub fn usable(slots: &[Option<[u8; 32]>; 2]) -> bool {
        slots.iter().flatten().any(|key| !is_dev_key(key))
    }
}

#[async_trait]
impl Check for UpdateKeyCheck {
    fn id(&self) -> CheckId {
        CheckId::UpdateKey
    }

    async fn run(&self) -> Vec<Finding> {
        if self.slots.iter().flatten().any(is_dev_key) {
            return vec![Finding::failure(
                "cet exe embarque la clé de développement, dont la clé privée est publique : il refuse toute mise à \
                 jour",
                "Cet exe ne doit pas être utilisé : installez une version publiée par le pipeline de release.",
            )];
        }
        let keys = self.slots.iter().flatten().count();
        match keys {
            0 => vec![Finding::warning(
                "aucune clé de mise à jour embarquée : cet exe ne se met jamais à jour (construction hors du pipeline \
                 de release)",
                "Installez une version publiée par le pipeline de release ; en attendant, mettez à jour à la main.",
            )],
            1 => vec![Finding::ok(
                "une clé de mise à jour embarquée, qui n'est pas la clé de développement",
            )],
            n => vec![Finding::ok(format!(
                "{n} clés de mise à jour embarquées (courante et suivante), aucune n'est la clé de développement"
            ))],
        }
    }
}

// ------------------------------------------------------------------------------------------------------ Clients LLM

/// « Clients LLM »: detected, configured or not (spec 023). Nothing of the files is said but their state: they may
/// hold the tokens of other servers.
pub struct ClientsCheck {
    pub files: Arc<dyn ClientFiles>,
    /// `None` without `%APPDATA%` or `%LOCALAPPDATA%`.
    pub env: Option<Environment>,
}

fn clients(files: &dyn ClientFiles, env: &Environment) -> Vec<Finding> {
    let target = env.target(files);
    let found = inspect_all(files, env, target.as_deref());
    let configured = found.iter().filter(|i| i.state == State::Configured).count();
    let installed = |id: LocationId| found.iter().any(|i| i.location == id && i.state != State::NotInstalled);
    let both_claude = installed(LocationId::ClaudeDesktop) && installed(LocationId::ClaudeDesktopPackage);
    let dialog = "Fenêtre de MCP Proxy, menu en haut à droite, « Clients LLM… » : « Installer » pour ce client.";
    let mut findings = Vec::new();
    for inspection in &found {
        let label = inspection.location.label();
        let note = if both_claude && inspection.location == LocationId::ClaudeDesktop {
            " (sans doute pas lu : Claude Desktop installé par son paquet MSIX lit le fichier du dossier du paquet)"
        } else {
            ""
        };
        let finding = match &inspection.state {
            State::NotInstalled => Finding::ok(format!("{label} : non installé")),
            State::Configured => Finding::ok(format!("{label} : configuré{note}")),
            State::NotConfigured { .. } if configured > 0 => {
                Finding::ok(format!("{label} : installé, non configuré{note}"))
            }
            State::NotConfigured { .. } => Finding::warning(format!("{label} : installé, non configuré{note}"), dialog),
            State::OtherCommand(_) => Finding::warning(
                format!("{label} : configuré avec un autre exe que l'exe installé{note}"),
                dialog,
            ),
            State::Unreadable(reason) => Finding::failure(
                format!("{label} : fichier de configuration illisible ({reason}){note}"),
                format!(
                    "Corrigez le fichier {} (il est peut-être en cours d'écriture par le client), ou copiez \
                     l'entrée depuis « Clients LLM… ».",
                    inspection.location.file_label()
                ),
            ),
        };
        findings.push(finding);
    }
    if found.iter().all(|i| i.state == State::NotInstalled) {
        return vec![Finding::warning(
            "aucun client LLM détecté (ni Claude Desktop, ni VS Code)",
            "Pour un autre client, « Clients LLM… », « Copier la configuration », puis collez-la dans le fichier de \
             configuration de ce client.",
        )];
    }
    if target.is_none() {
        findings.push(Finding::warning(
            "MCP Proxy n'est pas installé : aucun client ne peut être configuré",
            "Lancez l'exe téléchargé et choisissez « Installer ».",
        ));
    }
    findings
}

#[async_trait]
impl Check for ClientsCheck {
    fn id(&self) -> CheckId {
        CheckId::Clients
    }

    async fn run(&self) -> Vec<Finding> {
        let Some(env) = self.env.clone() else {
            return vec![Finding::failure(
                "dossiers du compte Windows introuvables (%APPDATA%, %LOCALAPPDATA%)",
                format!("Ouvrez une nouvelle session Windows. {SUPPORT}"),
            )];
        };
        let files = Arc::clone(&self.files);
        blocking(move || clients(files.as_ref(), &env)).await
    }
}

// -------------------------------------------------------------------------------------------------------------- MCP

/// An instance exposed to the LLM clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTarget {
    /// « Kibana Logs (REC) ».
    pub label: String,
    /// Its address in the configuration: only its origin is contacted.
    pub url: Url,
}

/// « MCP »: for each exposed instance, name resolution and TLS of its origin, anonymously. Any HTTP answer counts
/// as reachable; a 5xx is a warning (often a proxy that could not reach the server).
pub struct McpCheck {
    pub web: Arc<dyn Web>,
    pub targets: Vec<McpTarget>,
    pub dates: Dates,
}

/// What one origin gave.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reach {
    Answered { status: u16, resolved: bool },
    Failed(WebError),
}

async fn reach(web: &dyn Web, origin: &Url, dates: &Dates) -> Reach {
    let host = origin.host_str().unwrap_or_default().to_owned();
    let port = origin.port_or_known_default().unwrap_or(443);
    let (resolved, answer) = futures::join!(web.resolve(&host, port), web.get(origin, 0));
    match answer {
        Ok(answer) => {
            dates.record(answer.date.as_deref());
            Reach::Answered {
                status: answer.status,
                resolved: resolved.is_ok(),
            }
        }
        // The resolution tells why better than the request, which a proxy may have tried.
        Err(_) if resolved.is_err() => Reach::Failed(WebError::Dns),
        Err(error) => Reach::Failed(error),
    }
}

#[async_trait]
impl Check for McpCheck {
    fn id(&self) -> CheckId {
        CheckId::Mcp
    }

    async fn run(&self) -> Vec<Finding> {
        if self.targets.is_empty() {
            return vec![Finding::ok(
                "aucune instance exposée : aucun serveur MCP n'a été contacté",
            )];
        }
        // One request per origin, all at once; only origins of exposed instances.
        let mut origins: BTreeMap<String, Url> = BTreeMap::new();
        for target in &self.targets {
            let origin = target.url.origin().ascii_serialization();
            if let Ok(url) = Url::parse(&format!("{origin}/")) {
                origins.entry(origin).or_insert(url);
            }
        }
        let web = self.web.as_ref();
        let dates = &self.dates;
        let reached: BTreeMap<String, Reach> = futures::future::join_all(
            origins
                .iter()
                .map(|(key, url)| async move { (key.clone(), reach(web, url, dates).await) }),
        )
        .await
        .into_iter()
        .collect();
        self.targets
            .iter()
            .map(|target| {
                let host = host_of(&target.url);
                let key = target.url.origin().ascii_serialization();
                match reached.get(&key) {
                    Some(Reach::Answered { status, .. }) if *status >= 500 => Finding::warning(
                        format!(
                            "{} ({host}) : joignable, mais répond HTTP {status} (serveur en erreur, ou proxy qui ne le \
                             joint pas)",
                            target.label
                        ),
                        "Réessayez dans quelques minutes ; si l'erreur reste, signalez-la au support du serveur MCP.",
                    ),
                    Some(Reach::Answered { resolved, .. }) => {
                        let via = if *resolved { "" } else { " par le proxy de Windows" };
                        Finding::ok(format!(
                            "{} ({host}) : nom résolu, connexion sécurisée établie{via}",
                            target.label
                        ))
                    }
                    Some(Reach::Failed(error)) => Finding::failure(
                        format!("{} ({host}) : injoignable, {error}", target.label),
                        network_fix(*error),
                    ),
                    None => Finding::failure(format!("{} : adresse illisible", target.label), SUPPORT),
                }
            })
            .collect()
    }
}
