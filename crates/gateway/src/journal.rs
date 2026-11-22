//! What the gateway adds to the journal of spec 009, besides the calls: tool list changes, and the state of the
//! instances for the diagnostic export.

use mcp_proxy_core::trace::{Journal, rfc3339};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::task::JoinHandle;

use crate::registry::RegistryEvent;
use crate::updates::UpdateTracker;

/// Journals every tool list change the registry takes (new, modified, removed tools), until the registry is dropped.
pub fn follow_tool_changes(mut events: broadcast::Receiver<RegistryEvent>, journal: Journal) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(RegistryEvent::ToolsChanged { instance, diff }) => {
                    journal.record_tool_changes(
                        &instance.env,
                        &instance.mcp,
                        &diff.added,
                        &diff.modified,
                        &diff.removed,
                    );
                }
                Ok(_) => {}
                Err(RecvError::Lagged(lost)) => {
                    tracing::warn!(
                        lost,
                        "changements de liste d'outils non journalisés : file d'événements pleine"
                    );
                }
                Err(RecvError::Closed) => return,
            }
        }
    })
}

/// One line per instance, for the diagnostic archive: `dev/kibana-logs : connecté, dernier rechargement …`. The
/// last error was filtered when it was recorded; the export filters it again.
pub fn instance_lines(tracker: &UpdateTracker) -> Vec<String> {
    tracker
        .statuses()
        .into_iter()
        .map(|(instance, status)| {
            let mut line = format!("{}/{} : {}", instance.env, instance.mcp, status.state.label());
            if let Some(at) = status.refreshed_at {
                line.push_str(&format!(", dernier rechargement {}", rfc3339(at)));
            }
            if let Some(error) = &status.last_error {
                line.push_str(&format!(", dernière erreur : {error}"));
            }
            line
        })
        .collect()
}
