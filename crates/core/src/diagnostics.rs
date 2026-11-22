//! Diagnostic export: one zip archive for the support, `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip` (UTC), with the
//! log files, the version, the system, a summary of the configuration, the settings, the state of the instances
//! and the self-diagnostic report if there is one.
//!
//! Every file of the archive goes through the output filter at export time, the log files included: a secret known
//! now but written before it was known does not leave. If the filter is unavailable, no archive is written.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;
use url::Url;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::config::{AuthConfig, Config, McpKind, SessionRef, SessionScope};
use crate::filter::OutputFilter;
use crate::settings::UserSettings;
use crate::trace::{FileLog, WallClock, log_files, time};

pub const ARCHIVE_PREFIX: &str = "mcp-proxy-diagnostic-";
/// Self-diagnostic reports of spec 024, in the log directory.
pub const DOCTOR_REPORT_PREFIX: &str = "doctor-";
/// Largest part of a log file put in the archive: its end, the most recent lines.
pub const MAX_EXPORTED_LOG_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum DiagnosticError {
    /// The archive could not be written. The message names no content.
    #[error("export de diagnostic impossible : {0}")]
    Io(String),
    #[error("export de diagnostic impossible : filtre de sortie indisponible, aucune archive écrite")]
    FilterUnavailable,
}

fn io_error(error: &io::Error) -> DiagnosticError {
    DiagnosticError::Io(error.kind().to_string())
}

fn zip_error(_: zip::result::ZipError) -> DiagnosticError {
    DiagnosticError::Io("archive zip".to_owned())
}

/// What changes between two exports.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticState {
    pub settings: UserSettings,
    /// One line per instance: identifiers, state, last error (already filtered by its producer, filtered again).
    pub instances: Vec<String>,
    /// Self-diagnostic report (spec 024). `None`: the most recent `doctor-*.txt` of the log directory, if any.
    pub doctor_report: Option<String>,
}

/// Everything the export needs that does not change while the application runs.
pub struct Diagnostics {
    pub version: String,
    /// Operating system description given by the application, `std::env::consts` appended.
    pub system: String,
    pub config: Arc<Config>,
    pub log_dir: PathBuf,
    pub filter: Arc<OutputFilter>,
    pub clock: Arc<dyn WallClock>,
    /// Flushed before the files are read.
    pub log: Option<FileLog>,
}

impl std::fmt::Debug for Diagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Diagnostics")
            .field("version", &self.version)
            .field("log_dir", &self.log_dir)
            .finish_non_exhaustive()
    }
}

/// `mcp-proxy-diagnostic-20261122-1403.zip`.
pub fn archive_name(at: std::time::SystemTime) -> String {
    format!("{ARCHIVE_PREFIX}{}.zip", time::compact(at))
}

impl Diagnostics {
    /// Writes the archive. `dest` is a directory, where the archive gets its dated name, or the path of the archive
    /// itself. Written next to its destination then renamed: an export that fails leaves nothing behind.
    pub fn export(&self, dest: &Path, state: &DiagnosticState) -> Result<PathBuf, DiagnosticError> {
        let now = self.clock.now();
        let path = if dest.is_dir() {
            dest.join(archive_name(now))
        } else {
            dest.to_path_buf()
        };
        if let Some(log) = &self.log {
            // A slow writer only means the last lines are missing.
            let _ = log.flush();
        }
        let mut partial = path.clone().into_os_string();
        partial.push(".part");
        let partial = PathBuf::from(partial);
        let result = self.write_archive(&partial, now, state);
        match result.and_then(|()| fs::rename(&partial, &path).map_err(|e| io_error(&e))) {
            Ok(()) => Ok(path),
            Err(error) => {
                let _ = fs::remove_file(&partial);
                Err(error)
            }
        }
    }

    fn write_archive(
        &self,
        path: &Path,
        now: std::time::SystemTime,
        state: &DiagnosticState,
    ) -> Result<(), DiagnosticError> {
        let t = time::utc(now);
        let stamp = DateTime::from_date_and_time(
            u16::try_from(t.year).unwrap_or(1980),
            u8::try_from(t.month).unwrap_or(1),
            u8::try_from(t.day).unwrap_or(1),
            u8::try_from(t.hour).unwrap_or(0),
            u8::try_from(t.minute).unwrap_or(0),
            u8::try_from(t.second).unwrap_or(0),
        )
        .unwrap_or_default();
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(stamp);
        let file = File::create(path).map_err(|e| io_error(&e))?;
        let mut zip = ZipWriter::new(BufWriter::new(file));

        let mut add = |name: &str, text: &str| -> Result<(), DiagnosticError> {
            let (filtered, _) = self
                .filter
                .filter_text(text)
                .map_err(|_| DiagnosticError::FilterUnavailable)?;
            zip.start_file(name, options).map_err(zip_error)?;
            zip.write_all(filtered.as_bytes()).map_err(|e| io_error(&e))
        };

        add("LISEZMOI.txt", &readme(now))?;
        add("version.txt", &format!("MCP Proxy {}\n", self.version))?;
        add(
            "systeme.txt",
            &format!(
                "{}\nos={} famille={} architecture={}\n",
                self.system,
                std::env::consts::OS,
                std::env::consts::FAMILY,
                std::env::consts::ARCH
            ),
        )?;
        add("config.txt", &config_summary(&self.config))?;
        let settings = state
            .settings
            .to_json()
            .map_err(|_| DiagnosticError::Io("réglages".to_owned()))?;
        add("reglages.json", &String::from_utf8_lossy(&settings))?;
        let mut instances = state.instances.join("\n");
        instances.push('\n');
        add("instances.txt", &instances)?;
        let doctor = match &state.doctor_report {
            Some(report) => Some(report.clone()),
            None => latest_doctor_report(&self.log_dir),
        };
        if let Some(report) = doctor {
            add("auto-diagnostic.txt", &report)?;
        }
        // A missing directory only means there is no log yet.
        for log in log_files(&self.log_dir).unwrap_or_default() {
            let Some(name) = log.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let text = read_tail(&log, MAX_EXPORTED_LOG_BYTES).map_err(|e| io_error(&e))?;
            add(&format!("logs/{name}"), &text)?;
        }

        let mut out = zip.finish().map_err(zip_error)?;
        out.flush().map_err(|e| io_error(&e))?;
        out.into_inner()
            .map_err(|e| io_error(e.error()))?
            .sync_all()
            .map_err(|e| io_error(&e))
    }
}

fn readme(now: std::time::SystemTime) -> String {
    format!(
        "Diagnostic MCP Proxy, {} (UTC).\n\n\
         Contenu : version, système, résumé de la configuration (identifiants, libellés, hôtes), réglages, état des \
         instances, rapport d'auto-diagnostic s'il existe, fichiers de log.\n\
         Tout le contenu est repassé par le filtre de sortie à l'export : aucun secret connu n'y figure.\n",
        time::rfc3339(now)
    )
}

/// The end of a file, as text: at most `max` bytes, from the start of a line when it is cut.
fn read_tail(path: &Path, max: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut bytes = Vec::new();
    if len > max {
        io::Seek::seek(&mut file, io::SeekFrom::Start(len - max))?;
        file.take(max).read_to_end(&mut bytes)?;
        if let Some(newline) = bytes.iter().position(|b| *b == b'\n') {
            bytes.drain(..=newline);
        }
    } else {
        file.read_to_end(&mut bytes)?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn latest_doctor_report(dir: &Path) -> Option<String> {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(DOCTOR_REPORT_PREFIX) && n.ends_with(".txt"))
        })
        .map(|e| e.path())
        .collect();
    // Timestamped names: the last one in order is the most recent.
    reports.sort();
    let path = reports.pop()?;
    read_tail(&path, MAX_EXPORTED_LOG_BYTES).ok()
}

fn host(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        (None, _) => "-".to_owned(),
    }
}

/// Identifiers, labels and hosts: no path, query or credential of any URL.
pub fn config_summary(config: &Config) -> String {
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(format!("schema_version = {}", config.schema_version));
    let g = &config.gateway;
    line(format!(
        "passerelle : budget {} s, confirmation {} s, connexion {} s, amont {} s, appel lent {} s, rechargement {} s",
        g.call_budget_secs,
        g.confirmation_timeout_secs,
        g.auth_timeout_secs,
        g.upstream_timeout_secs,
        g.slow_call_threshold_secs,
        g.refresh_interval_secs
    ));
    line(format!(
        "traces : tampon {} entrées, {} octets, rétention {} jours",
        config.trace.ring_capacity, config.trace.ring_max_bytes, config.trace.log_retention_days
    ));
    line(String::new());
    line("Environnements :".to_owned());
    for env in &config.environments {
        let sensitive = if env.sensitive { " (sensible)" } else { "" };
        line(format!("  {} — {}{sensitive}", env.id, env.label));
    }
    line("Sessions :".to_owned());
    for session in &config.sessions {
        let scope = match &session.scope {
            SessionScope::Global => "globale".to_owned(),
            SessionScope::Env(env) => format!("environnement {env}"),
        };
        let (method, login) = match &session.auth {
            AuthConfig::JwtHeader { login_url, .. } => ("jwt", host(login_url)),
            AuthConfig::Cookie { login_url, .. } => ("cookie", host(login_url)),
            AuthConfig::Basic { .. } => ("basic", "-".to_owned()),
        };
        let origins: Vec<&str> = session
            .allowed_origins
            .iter()
            .map(|o| o.as_str().split_once("://").map_or(o.as_str(), |(_, host)| host))
            .collect();
        line(format!(
            "  {} — {} ({scope}, {method}, connexion {login}, origines {})",
            session.id,
            session.label,
            origins.join(" ")
        ));
    }
    line("MCP :".to_owned());
    for mcp in &config.mcp {
        let kind = match &mcp.kind {
            McpKind::Remote => "distant".to_owned(),
            McpKind::Local { module } => format!("module local {module}"),
        };
        let session = match &mcp.session {
            SessionRef::Env => "session de l'environnement".to_owned(),
            SessionRef::Global(id) => format!("session {id}"),
        };
        line(format!(
            "  {} (alias {}) — {} ({kind}, {session}, {} outils déclarés)",
            mcp.id,
            mcp.alias,
            mcp.label,
            mcp.tools.len()
        ));
        for (env, url) in &mcp.instances {
            line(format!("    {env} : {}", host(url)));
        }
    }
    line("Profils :".to_owned());
    for profile in config.profiles.values() {
        line(format!("  {} — {}", profile.id, profile.label));
    }
    out
}
