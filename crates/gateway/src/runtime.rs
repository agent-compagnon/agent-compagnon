//! Assembly of the gateway at start (spec 011): everything the application runs, in order, from the configuration
//! and the secret store it was given. The application only adds what is Windows-specific: DPAPI, the named pipe,
//! the windows, the icon and the message boxes.
//!
//! Start order (I011.2): secret store (given), output filter, log file, global collector, journal, panic hook,
//! settings and catalogue cache (given, sealed in the application, spec 014), registry, sessions, on-demand
//! authenticator, upstreams, list watcher, confirmation broker, call chain, MCP server. A failure of any step stops
//! the start: there is no gateway without its filter, its journal or its chain.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use mcp_proxy_auth::{AuthUi, AuthenticatorDeps, BasicProbe, HttpProbe, OnDemandAuthenticator, methods_from_config};
use mcp_proxy_core::config::Config;
use mcp_proxy_core::filter::{FilterError, OutputFilter};
use mcp_proxy_core::sealed::Sealer;
use mcp_proxy_core::secrets::{KeyWrapper, OriginGuard, SecretStore};
use mcp_proxy_core::session::{SessionGate, SessionRegistry};
use mcp_proxy_core::settings::{SealedSettingsStore, SettingsError, SettingsOrigin, SettingsStore, UserSettings};
use mcp_proxy_core::trace::{
    Journal, JournalConfig, LogFileOptions, LogGuard, LoggingError, WallClock, install_global, install_panic_hook,
    log_file_name, open_log, rfc3339,
};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{RwLock, broadcast, watch};
use tokio::task::JoinHandle;

use crate::bridge::{PeerProof, ProofError, Role, prove};
use crate::journal::follow_tool_changes;
use crate::modules;
use crate::notify::{ConfirmationPresenter, NotificationBroker, PendingConfirmations};
use crate::registry::{
    CacheSeal, CatalogError, EmbeddedCatalog, FileCatalogCache, Registry, RegistryError, SealedCatalog,
};
use crate::server::{CallChain, CallChainDeps, ChainError, GatewayServer, OutputGate};
use crate::updates::{ListWatcher, ListWatcherDeps, UpdateTracker, WatchTiming};
use crate::upstream::Upstreams;
use crate::view::{Alert, AlertKind, AlertLevel, AppSnapshot, SnapshotInputs};

/// Extra time a retired secret variant stays in the filter after `upstream_timeout_secs`: a response to a call sent
/// before the wipe may still carry it.
const VARIANT_GRACE_MARGIN: Duration = Duration::from_secs(10);

/// Where the application keeps its non-secret data: `%LOCALAPPDATA%\MCP Proxy\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirs {
    pub root: PathBuf,
    pub logs: PathBuf,
    pub catalog_cache: PathBuf,
    pub settings: PathBuf,
    /// Key of the seal of the settings and of the catalogue cache (spec 014), wrapped by DPAPI.
    pub settings_key: PathBuf,
    /// Shared profile of the login windows (spec 012), emptied at start and at exit.
    pub webview: PathBuf,
    /// Icon of the notifications, named by the registered AUMID (spec 013).
    pub notification_icon: PathBuf,
}

impl DataDirs {
    pub const FOLDER: &'static str = "MCP Proxy";

    /// `<base>\MCP Proxy\` with `logs\`, `cache\catalog\`, `webview\`, `settings.json`, `settings.key` and
    /// `notification-icon.png`.
    pub fn under(base: &Path) -> Self {
        let root = base.join(Self::FOLDER);
        Self {
            logs: root.join("logs"),
            catalog_cache: root.join("cache").join("catalog"),
            settings: root.join("settings.json"),
            settings_key: root.join("settings.key"),
            webview: root.join("webview"),
            notification_icon: root.join("notification-icon.png"),
            root,
        }
    }
}

/// Whether the log file becomes the process-wide `tracing` collector, with the panic hook. Once per process: the
/// application does it, tests do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logging {
    Global,
    Local,
}

/// How the settings and the catalogue cache are kept (spec 014).
pub struct Persistence {
    pub settings: Box<dyn SettingsStore + Send + Sync>,
    /// Applied to the files of the catalogue cache, in `DataDirs::catalog_cache`.
    pub cache_seal: Box<dyn CacheSeal>,
}

impl std::fmt::Debug for Persistence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Persistence").finish_non_exhaustive()
    }
}

impl Persistence {
    /// `settings.json` and the cache files sealed by one key, kept in `settings.key` wrapped by `wrapper`
    /// (user-scoped DPAPI in the application).
    pub fn sealed(dirs: &DataDirs, wrapper: Box<dyn KeyWrapper>) -> Self {
        let sealer = Arc::new(Sealer::open(&dirs.settings_key, wrapper));
        Self {
            settings: Box::new(SealedSettingsStore::new(&dirs.settings, Arc::clone(&sealer))),
            cache_seal: Box::new(SealedCatalog(sealer)),
        }
    }
}

/// What the gateway is started from.
pub struct RuntimeDeps {
    pub config: Arc<Config>,
    pub dirs: DataDirs,
    /// Sealed settings and cache in the application ([`Persistence::sealed`]).
    pub persistence: Persistence,
    /// From `SecretStore::for_platform`: if it failed, there is no gateway.
    pub store: SecretStore,
    /// Login windows: the WebView2 windows of the application (spec 012), `UnavailableAuthUi` in tests.
    pub auth_ui: Arc<dyn AuthUi>,
    /// Shows the confirmations outside the chat (spec 013): toasts and detail windows of the application,
    /// `UnavailablePresenter` in tests.
    pub presenter: Arc<dyn ConfirmationPresenter>,
    pub clock: Arc<dyn WallClock>,
    pub logging: Logging,
    /// Local offset from UTC (east positive) for the labels of the snapshot.
    pub utc_offset: i32,
}

/// Why the gateway did not start. French, without any secret: shown in a message box.
#[derive(Debug, Error)]
pub enum StartupError {
    #[error("filtre de sortie indisponible ({0}) : aucune passerelle ne démarre sans lui")]
    Filter(#[from] FilterError),
    #[error("journal impossible à ouvrir dans le dossier des logs ({})", .0.kind())]
    LogFile(io::Error),
    #[error("{0}")]
    Logging(#[from] LoggingError),
    #[error("catalogue embarqué invalide : {0}")]
    Catalog(#[from] CatalogError),
    #[error("registre des outils impossible à construire : {0}")]
    Registry(#[from] RegistryError),
    #[error("client de contrôle des identifiants impossible à créer")]
    Probe,
    #[error("{0}")]
    Chain(#[from] ChainError),
}

/// The running gateway. Dropping it stops its tasks.
pub struct Gateway {
    config: Arc<Config>,
    output: OutputGate,
    journal: Journal,
    registry: Arc<RwLock<Registry>>,
    settings: watch::Sender<UserSettings>,
    settings_store: Box<dyn SettingsStore + Send + Sync>,
    sessions: Arc<SessionRegistry>,
    authenticator: Arc<OnDemandAuthenticator>,
    watcher: ListWatcher,
    server: GatewayServer,
    confirmations: Arc<NotificationBroker>,
    alerts: Mutex<Vec<Alert>>,
    changes: broadcast::Sender<()>,
    utc_offset: i32,
    _tasks: Vec<AbortOnDrop>,
    _log_guard: LogGuard,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway").finish_non_exhaustive()
    }
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Gateway {
    /// Starts everything; must run inside a tokio runtime.
    pub async fn start(deps: RuntimeDeps) -> Result<Self, StartupError> {
        let config = deps.config;
        let grace = Duration::from_secs(config.gateway.upstream_timeout_secs) + VARIANT_GRACE_MARGIN;
        let store = Arc::new(deps.store.with_variant_grace(grace));
        let filter = Arc::new(OutputFilter::new(Arc::clone(&store), &config.filter)?);

        let (log, log_guard) = open_log(
            LogFileOptions::new(&deps.dirs.logs, &config.trace),
            Arc::clone(&deps.clock),
            Arc::clone(&filter),
        )
        .map_err(StartupError::LogFile)?;
        if deps.logging == Logging::Global {
            install_global(log.clone())?;
        }
        let journal = Journal::new(
            JournalConfig::from_config(&config),
            Arc::clone(&filter),
            Arc::clone(&deps.clock),
            Some(log),
        );
        if deps.logging == Logging::Global {
            install_panic_hook(journal.clone());
        }
        tracing::info!(version = env!("CARGO_PKG_VERSION"), "démarrage de MCP Proxy");

        let settings_store = deps.persistence.settings;
        let loaded = settings_store.load();
        let mut alerts = Vec::new();
        if let SettingsOrigin::Reset(error) = &loaded.origin {
            tracing::warn!(%error, "réglages remplacés par les plus stricts");
            journal.record_event(
                tracing::Level::WARN,
                &format!("réglages remplacés par les plus stricts : {error}"),
            );
            alerts.push(settings_reset_alert(error));
        }
        let settings = loaded.settings;

        let embedded = EmbeddedCatalog::builtin(&config)?;
        let cache = FileCatalogCache::with_seal(
            &deps.dirs.catalog_cache,
            Arc::clone(&filter),
            deps.persistence.cache_seal,
        );
        let mut registry = Registry::new(Arc::clone(&config), &embedded, Box::new(cache), Arc::clone(&filter))?;
        registry.apply_settings(&settings);
        let registry_events = registry.subscribe();
        let tool_changes = follow_tool_changes(registry.subscribe(), journal.clone());
        let registry = Arc::new(RwLock::new(registry));

        let guard = Arc::new(OriginGuard::from_config(&config));
        let sessions = Arc::new(SessionRegistry::from_config(&config));
        let probe: Arc<dyn BasicProbe> = Arc::new(
            HttpProbe::new(
                Arc::clone(&guard),
                Duration::from_secs(config.gateway.upstream_timeout_secs),
            )
            .map_err(|_| StartupError::Probe)?,
        );
        let authenticator = Arc::new(OnDemandAuthenticator::new(AuthenticatorDeps {
            registry: Arc::clone(&sessions),
            store: Arc::clone(&store),
            guard,
            ui: deps.auth_ui,
            methods: methods_from_config(&config, &probe),
            clock: Arc::clone(&deps.clock),
        }));
        let gate: Arc<dyn SessionGate> = authenticator.clone();
        let upstreams = Upstreams::from_config(&config, &gate, &modules::builtin());

        let output = OutputGate::new(filter);
        let confirmations = Arc::new(NotificationBroker::new(
            PendingConfirmations::new(),
            deps.presenter,
            output.clone(),
        ));
        let (settings_sender, settings_receiver) = watch::channel(settings);
        let watcher = ListWatcher::spawn(ListWatcherDeps {
            config: Arc::clone(&config),
            registry: Arc::clone(&registry),
            upstreams: upstreams.clone(),
            gate: Arc::clone(&gate),
            settings: settings_receiver.clone(),
            output: output.clone(),
            tracker: UpdateTracker::new(&config),
            timing: WatchTiming::from_config(&config),
            sessions: Some(sessions.subscribe()),
        });
        let chain = Arc::new(CallChain::new(CallChainDeps {
            config: Arc::clone(&config),
            registry: Arc::clone(&registry),
            settings: settings_receiver.clone(),
            upstreams,
            gate,
            broker: confirmations.clone(),
            output: output.clone(),
            trace: Arc::new(journal.clone()),
        })?);
        let server = GatewayServer::new(
            Arc::clone(&registry),
            output.clone(),
            chain,
            Duration::from_secs(config.gateway.list_ttl_secs),
        );

        let (changes, _) = broadcast::channel(16);
        let forwarder = tokio::spawn(forward_changes(
            changes.clone(),
            registry_events,
            sessions.subscribe(),
            watcher.tracker().subscribe(),
            settings_receiver,
            confirmations.pending().subscribe(),
        ));
        tracing::info!("passerelle prête");

        Ok(Self {
            config,
            output,
            journal,
            registry,
            settings: settings_sender,
            settings_store,
            sessions,
            authenticator,
            watcher,
            server,
            confirmations,
            alerts: Mutex::new(alerts),
            changes,
            utc_offset: deps.utc_offset,
            _tasks: vec![AbortOnDrop(tool_changes), AbortOnDrop(forwarder)],
            _log_guard: log_guard,
        })
    }

    pub fn config(&self) -> &Arc<Config> {
        &self.config
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn authenticator(&self) -> &Arc<OnDemandAuthenticator> {
        &self.authenticator
    }

    pub fn watcher(&self) -> &ListWatcher {
        &self.watcher
    }

    /// The broker of the confirmations outside the chat, and through it the waiting requests that the toasts and
    /// the detail windows answer.
    pub fn confirmations(&self) -> &Arc<NotificationBroker> {
        &self.confirmations
    }

    /// `text` through the output filter, for what the application shows outside the snapshot (notifications).
    /// `None` when the filter is unavailable: nothing is shown then.
    pub fn filter_text(&self, text: &str) -> Option<String> {
        self.output.text(text)
    }

    /// One accepted connection: the mutual proof as [`Role::Server`], then the MCP server until the client goes
    /// away. A failed proof closes the connection without any MCP byte, and is logged without any nonce.
    pub async fn serve_connection<S>(&self, proof: Arc<dyn PeerProof>, mut stream: S) -> Result<(), ProofError>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(error) = prove(proof, Role::Server, &mut stream).await {
            tracing::warn!(%error, "connexion au canal refusée avant tout échange MCP");
            return Err(error);
        }
        tracing::info!("client connecté au canal");
        self.server.clone().run(stream).await;
        Ok(())
    }

    /// Fires whenever the snapshot may have changed: tools, sessions, instance states, settings, alerts, waiting
    /// confirmations.
    pub fn changes(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    /// A banner known outside of the gateway (start-up hardening that failed).
    pub fn add_alert(&self, alert: Alert) {
        self.alerts.lock().unwrap_or_else(PoisonError::into_inner).push(alert);
        let _ = self.changes.send(());
    }

    /// Withdraws the banners of `kind`, once what they asked for is done.
    pub fn remove_alerts(&self, kind: AlertKind) {
        let removed = {
            let mut alerts = self.alerts.lock().unwrap_or_else(PoisonError::into_inner);
            let before = alerts.len();
            alerts.retain(|a| a.kind != kind);
            alerts.len() != before
        };
        if removed {
            let _ = self.changes.send(());
        }
    }

    /// What the interface shows, unfiltered: for the icon, which only reads its status.
    pub async fn snapshot(&self) -> AppSnapshot {
        let registry = self.registry.read().await;
        let settings = self.settings.borrow().clone();
        let sessions = self.sessions.snapshot();
        let tracker = self.watcher.tracker();
        let alerts = self.alerts.lock().unwrap_or_else(PoisonError::into_inner).clone();
        AppSnapshot::build(&SnapshotInputs {
            config: &self.config,
            registry: &registry,
            settings: &settings,
            sessions: &sessions,
            instances: &tracker.statuses(),
            updates: &tracker.pending(),
            pending_confirmations: self.confirmations.pending().count(),
            alerts: &alerts,
            login_requesters: &self.authenticator.login_requesters(),
            utc_offset: self.utc_offset,
        })
    }

    /// The snapshot as sent to the window: serialized, then through the output filter once more. `None` when the
    /// filter is unavailable: nothing is sent then.
    pub async fn snapshot_json(&self) -> Option<Value> {
        let snapshot = self.snapshot().await;
        let mut value = serde_json::to_value(&snapshot).ok()?;
        self.output.filter().filter_json(&mut value).ok()?;
        Some(value)
    }

    pub fn settings(&self) -> UserSettings {
        self.settings.borrow().clone()
    }

    /// Changes the settings: applied at once (exposure, chain, list watcher), then saved. A save failure is
    /// returned after the change is applied: the change holds until the application stops, and the file keeps the
    /// previous settings.
    pub async fn change_settings(&self, change: impl FnOnce(&mut UserSettings)) -> Result<(), SettingsError> {
        let mut settings = self.settings();
        change(&mut settings);
        self.registry.write().await.apply_settings(&settings);
        self.settings.send_replace(settings.clone());
        self.settings_store.save(&settings)
    }
}

/// The banner of a settings reset: why, and what applies now.
fn settings_reset_alert(error: &SettingsError) -> Alert {
    let cause = match error {
        SettingsError::Seal(_) => "Vos réglages n'ont pas pu être vérifiés",
        _ => "Vos réglages étaient illisibles",
    };
    Alert {
        level: AlertLevel::Error,
        kind: AlertKind::SettingsReset,
        message: format!(
            "{cause} ({error}) : les réglages les plus stricts les remplacent, aucun environnement n'est exposé. \
             Choisissez de nouveau votre profil."
        ),
    }
}

/// Coalesces every source of change into one signal.
async fn forward_changes(
    changes: broadcast::Sender<()>,
    mut registry: broadcast::Receiver<crate::registry::RegistryEvent>,
    mut sessions: broadcast::Receiver<mcp_proxy_core::session::SessionEvent>,
    mut instances: broadcast::Receiver<crate::updates::UpdateEvent>,
    mut settings: watch::Receiver<UserSettings>,
    mut confirmations: watch::Receiver<usize>,
) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let closed = tokio::select! {
            r = registry.recv() => matches!(r, Err(RecvError::Closed)),
            r = sessions.recv() => matches!(r, Err(RecvError::Closed)),
            r = instances.recv() => matches!(r, Err(RecvError::Closed)),
            r = settings.changed() => r.is_err(),
            r = confirmations.changed() => r.is_err(),
        };
        if closed {
            return;
        }
        let _ = changes.send(());
    }
}

/// Why the application refuses to start, before or around the gateway. French, for the message box and the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupFailure {
    /// Validation errors of the embedded configuration, one per line.
    InvalidConfig(Vec<String>),
    WebviewMissing,
    /// `SecretStore::for_platform` failed: no protection of the secrets, no gateway.
    ProtectionUnavailable(String),
    /// The pipe's name already exists: another program holds it.
    PipeTaken,
    /// No Windows user name to build the pipe's name from.
    NoUser,
    /// No `%LOCALAPPDATA%`.
    NoDataDir,
    Gateway(String),
    /// The pipe could not be created for another reason.
    Pipe(String),
}

impl StartupFailure {
    pub fn title(&self) -> &'static str {
        "MCP Proxy ne peut pas démarrer"
    }

    /// The text of the message box.
    pub fn message(&self) -> String {
        match self {
            Self::InvalidConfig(errors) => format!(
                "La configuration embarquée est invalide :\n\n{}\n\nCette version de MCP Proxy est défectueuse : \
                 installez une autre version.",
                errors.join("\n")
            ),
            Self::WebviewMissing => {
                "Le runtime Microsoft Edge WebView2 est absent ou illisible. Installez « Microsoft \
                                     Edge WebView2 Runtime » (ou demandez-le à votre support), puis relancez MCP \
                                     Proxy."
                    .to_owned()
            }
            Self::ProtectionUnavailable(reason) => format!(
                "La protection des secrets par Windows (DPAPI) ne répond pas ({reason}). MCP Proxy refuse de démarrer \
                 sans elle : aucun secret ne serait protégé. Fermez puis rouvrez votre session Windows, puis \
                 relancez MCP Proxy."
            ),
            Self::PipeTaken => "Un autre programme occupe le canal de MCP Proxy. MCP Proxy refuse de démarrer pour \
                                ne pas servir les clients LLM à travers un canal qui ne lui appartient pas. Fermez ce \
                                programme (ou fermez puis rouvrez votre session Windows), puis relancez MCP Proxy."
                .to_owned(),
            Self::NoUser => "Le nom de l'utilisateur Windows est introuvable (variable USERNAME) : le canal de MCP \
                             Proxy ne peut pas être nommé."
                .to_owned(),
            Self::NoDataDir => {
                "Le dossier des données locales de Windows est introuvable (variable LOCALAPPDATA).".to_owned()
            }
            Self::Gateway(reason) => format!("La passerelle n'a pas pu démarrer : {reason}."),
            Self::Pipe(reason) => format!("Le canal de MCP Proxy n'a pas pu être créé ({reason})."),
        }
    }

    /// One line for the log file.
    pub fn log_line(&self) -> String {
        self.message()
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Appends one line to today's log file, for a failure that happens before the journal exists (invalid
/// configuration, WebView2 missing, no protection of the secrets, no filter). Only fixed texts and the validation
/// messages of the configuration compiled into the exe go through here: there is no secret yet at that point.
pub fn record_startup_failure(logs: &Path, now: SystemTime, failure: &StartupFailure) -> io::Result<()> {
    fs::create_dir_all(logs)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join(log_file_name(now)))?;
    writeln!(
        file,
        "{} ERROR mcp_proxy::startup: démarrage refusé : {}",
        rfc3339(now),
        failure.log_line()
    )
}

/// The WebView2 runtime is usable when `tauri::webview_version()` gives a version whose first part is a number.
pub fn check_webview(version: Result<String, String>) -> Result<String, StartupFailure> {
    let version = version.map_err(|_| StartupFailure::WebviewMissing)?;
    let version = version.trim();
    let major = version.split('.').next().unwrap_or_default();
    match major.parse::<u32>() {
        Ok(major) if major > 0 => Ok(version.to_owned()),
        _ => Err(StartupFailure::WebviewMissing),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    // Exigence : EX-011-15
    #[test]
    fn webview_version_must_be_readable() {
        assert_eq!(
            check_webview(Ok("131.0.2903.70".to_owned())).as_deref(),
            Ok("131.0.2903.70")
        );
        assert_eq!(
            check_webview(Err("absent".to_owned())),
            Err(StartupFailure::WebviewMissing)
        );
        assert_eq!(check_webview(Ok(String::new())), Err(StartupFailure::WebviewMissing));
        assert_eq!(
            check_webview(Ok("0.0.0.0".to_owned())),
            Err(StartupFailure::WebviewMissing)
        );
        assert_eq!(
            check_webview(Ok("inconnue".to_owned())),
            Err(StartupFailure::WebviewMissing)
        );
    }

    // Exigence : EX-011-15
    #[test]
    fn startup_failures_are_explained_and_logged() {
        let failures = [
            StartupFailure::InvalidConfig(vec!["mcp[0].id : identifiant invalide".to_owned()]),
            StartupFailure::WebviewMissing,
            StartupFailure::ProtectionUnavailable("DPAPI : code système 0x8009000b".to_owned()),
            StartupFailure::PipeTaken,
            StartupFailure::NoUser,
            StartupFailure::NoDataDir,
            StartupFailure::Gateway("filtre de sortie indisponible".to_owned()),
            StartupFailure::Pipe("accès refusé".to_owned()),
        ];
        for failure in &failures {
            assert!(failure.message().len() > 40, "{failure:?}");
            assert!(!failure.log_line().contains('\n'));
        }
        assert!(failures[0].message().contains("mcp[0].id : identifiant invalide"));
        assert!(failures[1].message().contains("WebView2"));
        assert!(failures[2].message().contains("DPAPI"));
        assert!(
            failures[3]
                .message()
                .contains("Un autre programme occupe le canal de MCP Proxy")
        );

        let dir = std::env::temp_dir().join(format!("mcp-proxy-startup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // 2026-11-22T14:03:07Z
        let now = UNIX_EPOCH + Duration::from_secs(1_795_356_187);
        record_startup_failure(&dir, now, &StartupFailure::PipeTaken).unwrap();
        record_startup_failure(&dir, now, &StartupFailure::WebviewMissing).unwrap();
        let text = fs::read_to_string(dir.join("mcp-proxy-2026-11-22.log")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("2026-11-22T14:03:07.000Z ERROR mcp_proxy::startup: démarrage refusé : Un autre"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dirs() {
        let dirs = DataDirs::under(Path::new("base"));
        assert_eq!(dirs.root, Path::new("base").join("MCP Proxy"));
        assert_eq!(dirs.logs, Path::new("base").join("MCP Proxy").join("logs"));
        assert_eq!(
            dirs.catalog_cache,
            Path::new("base").join("MCP Proxy").join("cache").join("catalog")
        );
        assert_eq!(dirs.settings, Path::new("base").join("MCP Proxy").join("settings.json"));
        assert_eq!(
            dirs.settings_key,
            Path::new("base").join("MCP Proxy").join("settings.key")
        );
        assert_eq!(dirs.webview, Path::new("base").join("MCP Proxy").join("webview"));
    }
}
