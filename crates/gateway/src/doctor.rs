//! The self-diagnostic (spec 024), assembled: the facts the gateway knows (configuration, sealed settings, exposure),
//! the probes the application gives ([`DoctorProbes`]), the checks of `mcp_proxy_deploy::doctor`, the output filter
//! over every text, then the report file.
//!
//! Two ways in. `mcp-proxy.exe doctor` ([`run_command`]), a process of its own launched before the single instance:
//! it reads the settings on disk without effect (no key created, nothing rewritten) and filters with an empty store
//! (the generic patterns still apply). « Auto-diagnostic » in the window and the diagnostic export
//! ([`Gateway::diagnose`]): the running gateway, its settings, its exposure and its filter, which knows the secrets of
//! the sessions.
//!
//! Nothing here writes but the report file; the MCP check contacts only the origins of the exposed instances.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use mcp_proxy_auth::webview::check_runtime_version;
use mcp_proxy_core::config::{AuthConfig, Config, FilterConfig, WebviewProfile};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::sealed::Sealer;
use mcp_proxy_core::secrets::{KeyWrapper, SecretStore, platform_key_wrapper};
use mcp_proxy_core::settings::{SealedSettingsStore, SettingsError, SettingsOrigin, SettingsStore, UserSettings};
use mcp_proxy_core::trace::{WallClock, rfc3339};
use mcp_proxy_deploy::clients::{ClientFiles, Environment};
use mcp_proxy_deploy::doctor::{
    AppCheck, ArtifactoryCheck, Check, ClientsCheck, ClockCheck, ConfigCheck, ConfigFacts, Dates, InstallationCheck,
    InstanceProbe, McpCheck, McpTarget, NotificationsCheck, Published, Report, ReportFiles, Saved, SealFacts,
    SettingsCheck, SettingsFacts, ToastProbe, UpdateKeyCheck, Viewpoint, Web, WebviewCheck, publish, run,
};
use mcp_proxy_deploy::install::InstallPaths;
use mcp_proxy_deploy::ops::SystemOps;
use mcp_proxy_deploy::update::{Channel, Source};
use mcp_proxy_deploy::version::{Installed, own_version};
use serde_json::Value;
use url::Url;

use crate::clients_view::backup_stamp;
use crate::interface::InterfaceError;
use crate::registry::{Exposure, InstanceId};
use crate::runtime::{DataDirs, Gateway};

/// The log folder as the user reads it, without their name.
pub const LOGS_LABEL: &str = r"%LOCALAPPDATA%\MCP Proxy\logs";

/// What only the application can read: the platform, behind the traits of `mcp_proxy_deploy::doctor`.
pub struct DoctorProbes {
    pub ops: Arc<dyn SystemOps>,
    pub files: Arc<dyn ClientFiles>,
    pub toasts: Arc<dyn ToastProbe>,
    pub instance: Arc<dyn InstanceProbe>,
    pub web: Arc<dyn Web>,
    /// `tauri::webview_version()`, `None` without a runtime.
    pub webview_version: Option<String>,
    /// `%APPDATA%`, `%LOCALAPPDATA%`.
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    /// This exe.
    pub here: Option<PathBuf>,
    /// The update keys compiled in (`update::embedded_slots`).
    pub slots: [Option<[u8; 32]>; 2],
}

impl std::fmt::Debug for DoctorProbes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoctorProbes").finish_non_exhaustive()
    }
}

/// Hosts reserved for examples and tests (RFC 2606, RFC 6761), and the example domain of the project.
fn is_example(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    [
        "example.internal",
        "example",
        "example.com",
        "example.net",
        "example.org",
        "test",
        "invalid",
    ]
    .iter()
    .any(|reserved| host == *reserved || host.ends_with(&format!(".{reserved}")))
}

/// What the check « Config » says of a valid configuration.
pub fn config_facts(config: &Config) -> ConfigFacts {
    let mut urls: Vec<String> = vec![config.update.base_url.clone()];
    for session in &config.sessions {
        match &session.auth {
            AuthConfig::JwtHeader { login_url, .. } => urls.push(login_url.to_string()),
            AuthConfig::Cookie {
                login_url, cookie_url, ..
            } => urls.extend([login_url.to_string(), cookie_url.to_string()]),
            AuthConfig::Basic { probe_url, .. } => urls.extend(probe_url.iter().map(ToString::to_string)),
        }
    }
    for mcp in &config.mcp {
        urls.extend(mcp.instances.values().map(ToString::to_string));
    }
    let example_hosts = urls
        .iter()
        .filter_map(|u| Url::parse(u).ok())
        .filter(|u| u.host_str().is_some_and(is_example))
        .count();
    ConfigFacts {
        example_hosts,
        environments: config.environments.len(),
        mcp: config.mcp.len(),
    }
}

/// The settings on disk, read as the next start would read them: the key is opened, never created; the file is
/// read, never rewritten. The settings themselves are the strictest unless the seal holds.
pub fn read_settings(dirs: &DataDirs, wrapper: Box<dyn KeyWrapper>) -> (SealFacts, UserSettings) {
    let sealer = Arc::new(Sealer::open(&dirs.settings_key, wrapper));
    let loaded = SealedSettingsStore::new(&dirs.settings, sealer).load();
    let seal = match &loaded.origin {
        SettingsOrigin::Stored => SealFacts::Valid,
        SettingsOrigin::Missing => SealFacts::Missing,
        SettingsOrigin::Reset(SettingsError::Seal(error)) => SealFacts::Refused(error.to_string()),
        SettingsOrigin::Reset(error) => SealFacts::Refused(error.to_string()),
    };
    (seal, loaded.settings)
}

fn settings_facts(seal: SealFacts, settings: &UserSettings, config: &Config, running: bool) -> SettingsFacts {
    SettingsFacts {
        running,
        seal,
        profile: settings.chosen_profile(config).map(|p| p.label.clone()),
        exposed: settings
            .exposed_environments(config)
            .iter()
            .filter_map(|env| config.environment(env).map(|e| e.label.clone()))
            .collect(),
        suspended: settings.gateway_suspended,
    }
}

/// The instances the user exposes, and only them: no traffic towards an environment that was not chosen.
pub fn mcp_targets(config: &Config, exposure: &Exposure) -> Vec<McpTarget> {
    let mut targets = Vec::new();
    for mcp in &config.mcp {
        for (env, url) in &mcp.instances {
            if !exposure.exposes(&InstanceId::new(env.clone(), mcp.id.clone())) {
                continue;
            }
            let env_label = config.environment(env).map_or(env.as_str(), |e| e.label.as_str());
            targets.push(McpTarget {
                label: format!("{} en {env_label}", mcp.label),
                url: url.clone(),
            });
        }
    }
    targets
}

/// Everything a diagnosis starts from.
struct Inputs {
    viewpoint: Viewpoint,
    config: Result<Arc<Config>, Vec<String>>,
    seal: SealFacts,
    settings: UserSettings,
    exposure: Exposure,
    now: SystemTime,
    clock: Arc<dyn WallClock>,
}

/// Every check of the table, in its order; unfiltered, not saved.
async fn diagnose(probes: &DoctorProbes, inputs: Inputs) -> Report {
    let config = inputs.config.as_ref().ok();
    let paths = match (&probes.local_app_data, &probes.app_data) {
        (Some(local), Some(roaming)) => Some(InstallPaths::new(local, roaming)),
        _ => None,
    };
    let env = match (&probes.app_data, &probes.local_app_data) {
        (Some(app_data), Some(local_app_data)) => Some(Environment {
            app_data: app_data.clone(),
            local_app_data: local_app_data.clone(),
        }),
        _ => None,
    };
    let installed =
        paths
            .as_ref()
            .filter(|p| probes.ops.exists(&p.exe))
            .and_then(|p| match probes.ops.exe_version(&p.exe) {
                Installed::Known(version) => Some(version),
                Installed::Unknown => None,
            });
    let clock = Arc::clone(&inputs.clock);
    let dates = Dates::with_clock(Arc::new(move || clock.now()));
    // Without a configuration, nothing is exposed: the seal is still told.
    let settings = match config {
        Some(config) => settings_facts(
            inputs.seal.clone(),
            &inputs.settings,
            config,
            inputs.viewpoint == Viewpoint::App,
        ),
        None => SettingsFacts {
            running: false,
            seal: inputs.seal.clone(),
            profile: None,
            exposed: Vec::new(),
            suspended: inputs.settings.gateway_suspended,
        },
    };
    let mut checks: Vec<Box<dyn Check>> = vec![
        Box::new(ConfigCheck {
            loaded: inputs.config.as_ref().map(|c| config_facts(c)).map_err(Clone::clone),
        }),
        Box::new(WebviewCheck {
            version: probes.webview_version.clone(),
            sufficient: check_runtime_version(probes.webview_version.as_deref()).is_ok(),
            in_memory: config.is_none_or(|c| c.webview.profile == WebviewProfile::InMemory),
        }),
        Box::new(InstallationCheck {
            ops: Arc::clone(&probes.ops),
            paths,
            here: probes.here.clone(),
        }),
        Box::new(NotificationsCheck {
            probe: Arc::clone(&probes.toasts),
        }),
        Box::new(AppCheck {
            probe: Arc::clone(&probes.instance),
            viewpoint: inputs.viewpoint,
            installed: installed.clone(),
        }),
        Box::new(SettingsCheck { facts: settings }),
    ];
    checks.push(Box::new(ArtifactoryCheck {
        web: Arc::clone(&probes.web),
        source: config.map(|c| Source {
            base_url: c.update.base_url.clone(),
            repository: c.update.repository.clone(),
            path: c.update.path.clone(),
        }),
        channel: Channel::of(inputs.settings.beta_channel),
        current: installed.unwrap_or_else(own_version),
        updates: UpdateKeyCheck::usable(&probes.slots),
        dates: dates.clone(),
    }));
    checks.push(Box::new(UpdateKeyCheck { slots: probes.slots }));
    checks.push(Box::new(ClientsCheck {
        files: Arc::clone(&probes.files),
        env,
    }));
    checks.push(Box::new(McpCheck {
        web: Arc::clone(&probes.web),
        targets: config.map(|c| mcp_targets(c, &inputs.exposure)).unwrap_or_default(),
        dates: dates.clone(),
    }));
    let checks = run(checks, Some(ClockCheck { dates })).await;
    Report {
        version: own_version().to_string(),
        at: rfc3339(inputs.now),
        viewpoint: inputs.viewpoint,
        checks,
        saved: Saved::No,
    }
}

fn filter_of(filter: &OutputFilter) -> impl Fn(&str) -> Option<String> + '_ {
    |text: &str| filter.filter_text(text).ok().map(|(filtered, _)| filtered.into_owned())
}

/// `mcp-proxy.exe doctor`: the diagnosis from outside the application, its report on `stdout` and in a new file of
/// the log folder. The exit code of the process is `Published::exit_code`.
pub async fn run_command(
    probes: &DoctorProbes,
    clock: Arc<dyn WallClock>,
    files: &dyn ReportFiles,
    stdout: &mut dyn Write,
) -> Published {
    run_command_with(
        probes,
        clock,
        files,
        stdout,
        platform_key_wrapper,
        crate::modules::embedded_config(),
    )
    .await
}

/// [`run_command`] with the key protection and the configuration given: for the tests.
pub async fn run_command_with(
    probes: &DoctorProbes,
    clock: Arc<dyn WallClock>,
    files: &dyn ReportFiles,
    stdout: &mut dyn Write,
    wrapper: fn() -> Box<dyn KeyWrapper>,
    config: Result<Config, mcp_proxy_core::config::ConfigErrors>,
) -> Published {
    let base = probes.local_app_data.clone().unwrap_or_default();
    let dirs = DataDirs::under(&base);
    let config = config
        .map(Arc::new)
        .map_err(|errors| errors.errors.iter().map(ToString::to_string).collect::<Vec<_>>());
    let (seal, settings) = read_settings(&dirs, wrapper());
    let exposure = match &config {
        Ok(config) => Exposure::from_settings(&settings, config),
        Err(_) => Exposure::none(),
    };
    let filter_config = config
        .as_ref()
        .map(|c| c.filter.clone())
        .unwrap_or_else(|_| FilterConfig::default());
    let now = clock.now();
    let report = diagnose(
        probes,
        Inputs {
            viewpoint: Viewpoint::Command,
            config,
            seal,
            settings,
            exposure,
            now,
            clock,
        },
    )
    .await;
    // A store of its own, empty: this process holds no secret, the generic patterns of the filter still apply.
    let filter = SecretStore::for_platform()
        .ok()
        .and_then(|store| OutputFilter::new(Arc::new(store), &filter_config).ok());
    let stamp = backup_stamp(now);
    match &filter {
        Some(filter) => {
            let apply = filter_of(filter);
            let report = report.filtered(&apply).unwrap_or_else(|| unfiltered(now));
            publish(report, &dirs.logs, LOGS_LABEL, &stamp, files, stdout, &apply)
        }
        None => publish(unfiltered(now), &dirs.logs, LOGS_LABEL, &stamp, files, stdout, &|_| {
            None
        }),
    }
}

/// A report that says nothing: its texts could not go through the filter.
fn unfiltered(now: SystemTime) -> Report {
    Report {
        version: own_version().to_string(),
        at: rfc3339(now),
        viewpoint: Viewpoint::Command,
        checks: Vec::new(),
        saved: Saved::No,
    }
}

/// How the window asked for the diagnosis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Save {
    /// « Auto-diagnostic »: the report is also written to a new file of the log folder.
    ToLogs,
    /// The diagnostic export: the report goes into the archive only.
    No,
}

impl Gateway {
    /// The diagnosis from inside the application: its settings, its exposure, its filter. `None` when the filter is
    /// unavailable: nothing of the report is shown then.
    pub async fn diagnose(&self, probes: &DoctorProbes, save: Save) -> Option<Published> {
        let (seal, _) = {
            let dirs = self.dirs().clone();
            tokio::task::spawn_blocking(move || read_settings(&dirs, platform_key_wrapper()))
                .await
                .ok()?
        };
        let exposure = self.registry().read().await.exposure().clone();
        let now = self.clock().now();
        let report = diagnose(
            probes,
            Inputs {
                viewpoint: Viewpoint::App,
                config: Ok(Arc::clone(self.config())),
                seal,
                settings: self.settings(),
                exposure,
                now,
                clock: Arc::clone(self.clock()),
            },
        )
        .await;
        let filter = |text: &str| self.filter_text(text);
        let report = report.filtered(&filter)?;
        match save {
            Save::No => {
                let text = filter(&report.text())?;
                let exit_code = report.exit_code();
                Some(Published {
                    report,
                    text,
                    file: None,
                    exit_code,
                })
            }
            Save::ToLogs => {
                let logs = self.log_dir().to_path_buf();
                let stamp = backup_stamp(now);
                Some(publish(
                    report,
                    &logs,
                    LOGS_LABEL,
                    &stamp,
                    &mcp_proxy_deploy::doctor::RealReportFiles,
                    &mut std::io::sink(),
                    &filter,
                ))
            }
        }
    }

    /// « Auto-diagnostic »: the report as the window shows it, its text filtered once more.
    pub async fn doctor_view(&self, probes: &DoctorProbes) -> Result<Value, InterfaceError> {
        let unavailable =
            || InterfaceError::Doctor("filtre de sortie indisponible : aucun rapport n'est montré".to_owned());
        let published = self.diagnose(probes, Save::ToLogs).await.ok_or_else(unavailable)?;
        let mut view = published.report.to_json();
        view["text"] = Value::String(published.text);
        self.journal()
            .filter()
            .filter_json(&mut view)
            .map_err(|_| unavailable())?;
        Ok(view)
    }
}
