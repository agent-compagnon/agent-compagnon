//! Self-diagnostic (spec 024): why « it does not work », in a few seconds, without reading a log.
//!
//! Every check ([`Check`]) gives one or more [`Finding`]s: `ok`, `avertissement` or `échec`, a message and what to
//! do, in French. The checks read, they never repair: what they need of the platform comes through traits
//! ([`crate::ops::SystemOps`], [`crate::clients::ClientFiles`], [`ToastProbe`], [`InstanceProbe`], [`Web`]) or as
//! facts gathered by the caller (configuration, sealed settings, WebView2). They run together, each bounded by
//! [`CHECK_TIMEOUT`], so that the whole diagnosis is bounded too ([`run`]).
//!
//! The network checks go through a client of their own ([`AnonymousWeb`]): no cookie, no credential, no redirect
//! followed, the origin of an exposed instance only. Their `Date` headers measure the clock ([`ClockCheck`]) without
//! any request of its own.
//!
//! The report ([`Report`]) renders as text and as a structure; [`publish`] writes it to a new file of the log folder
//! and to the standard output. Nothing here filters: the caller passes the output filter, which every text goes
//! through before it leaves.

mod checks;
mod clock;
mod output;
mod report;
mod web;

#[cfg(test)]
mod tests;

use std::time::Duration;

use async_trait::async_trait;

pub use checks::{
    AppCheck, ArtifactoryCheck, ClientsCheck, ConfigCheck, ConfigFacts, InstallationCheck, InstanceProbe, McpCheck,
    McpTarget, NotificationsCheck, PipeProbe, SealFacts, SettingsCheck, SettingsFacts, ToastProbe, Toasts,
    UpdateKeyCheck, WebviewCheck,
};
pub use clock::{ClockCheck, Dates, http_date, skew_status};
pub use output::{Published, REPORT_PREFIX, RealReportFiles, ReportFiles, publish, report_name};
pub use report::{CheckId, CheckResult, Finding, Report, Saved, Status, Viewpoint};
pub use web::{AnonymousWeb, Answer, NoNetwork, Web, WebError, classify_text};

/// Longest time one check may take; they run together, so this also bounds the whole diagnosis.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(12);

/// One check of the table of spec 024.
#[async_trait]
pub trait Check: Send + Sync {
    fn id(&self) -> CheckId;
    async fn run(&self) -> Vec<Finding>;
}

/// Runs every check at once, each within [`CHECK_TIMEOUT`], then the clock from what the others heard. The results
/// keep the order of `checks`, the clock last.
pub async fn run(checks: Vec<Box<dyn Check>>, clock: Option<ClockCheck>) -> Vec<CheckResult> {
    let runs = checks.iter().map(|check| async move {
        let findings = match tokio::time::timeout(CHECK_TIMEOUT, check.run()).await {
            Ok(findings) if !findings.is_empty() => findings,
            Ok(_) => vec![Finding::failure(
                "contrôle sans résultat",
                "Relancez l'auto-diagnostic ; si cela se reproduit, joignez ce rapport à votre demande au support.",
            )],
            Err(_) => vec![Finding::failure(
                format!("contrôle interrompu : pas de réponse en {} s", CHECK_TIMEOUT.as_secs()),
                "Relancez l'auto-diagnostic. Si le contrôle reste bloqué, vérifiez la connexion au réseau de \
                 l'entreprise (VPN) et joignez ce rapport à votre demande au support.",
            )],
        };
        CheckResult {
            id: check.id(),
            findings,
        }
    });
    let mut results = futures::future::join_all(runs).await;
    if let Some(clock) = clock {
        results.push(CheckResult {
            id: CheckId::Clock,
            findings: clock.run(),
        });
    }
    results
}

/// The work of a check that blocks (registry, files, WinRT), off the asynchronous threads. A task that could not
/// finish is a failure of the check, not of the diagnosis.
pub(crate) async fn blocking<F>(work: F) -> Vec<Finding>
where
    F: FnOnce() -> Vec<Finding> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        vec![Finding::failure(
            "contrôle interrompu avant sa fin",
            "Relancez l'auto-diagnostic ; si cela se reproduit, joignez ce rapport à votre demande au support.",
        )]
    })
}
