//! The `tracing` subscriber of the application: every event, of our crates and of the libraries (`log` records
//! included), formatted to the log file through the output filter.

use std::sync::Arc;

use thiserror::Error;
use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;

use super::logfile::{ClockTime, FileLog};

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("journal : un collecteur de traces global est déjà installé")]
    AlreadyInstalled,
}

/// Level of each source: our crates (`mcp_proxy*`) from `INFO`, libraries from `WARN`.
pub fn default_targets() -> Targets {
    Targets::new()
        .with_default(LevelFilter::WARN)
        .with_target("mcp_proxy", LevelFilter::INFO)
}

/// A subscriber that writes every event enabled by `targets` to `log`, in text without colours, timestamped by the
/// clock of `log`.
pub fn file_subscriber(log: FileLog, targets: Targets) -> impl Subscriber + Send + Sync + 'static {
    let clock = Arc::clone(log.clock());
    tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(log)
            .with_ansi(false)
            .with_timer(ClockTime(clock))
            .with_filter(targets),
    )
}

/// Installs [`file_subscriber`] for the whole process, and routes the `log` records of the libraries to it. Once
/// per process: the application calls it at start-up (spec 011).
pub fn install_global(log: FileLog) -> Result<(), LoggingError> {
    tracing::subscriber::set_global_default(file_subscriber(log, default_targets()))
        .map_err(|_| LoggingError::AlreadyInstalled)?;
    tracing_log::LogTracer::init_with_filter(tracing_log::log::LevelFilter::Info)
        .map_err(|_| LoggingError::AlreadyInstalled)
}
