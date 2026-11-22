//! What the interface reads about the tool lists of the servers: the state of each instance and the updates waiting
//! to be acknowledged. Nothing here is secret: errors are filtered before they are recorded.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use mcp_proxy_core::config::{Config, McpId, ToolName};
use tokio::sync::broadcast;

use crate::registry::{InstanceId, ToolDiff};

const EVENT_CAPACITY: usize = 64;

/// State of an instance, as shown by the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstanceState {
    /// No list was read from the server during this run: its tools come from the cache or the embedded snapshot.
    Catalogue,
    /// The last reload succeeded.
    Connected,
    /// The last reload needed a session that is not active, or whose credential the server refused.
    SessionRequired,
    /// The last reload failed otherwise (server unreachable, error, list refused): the last known list is kept.
    Unreachable,
    /// The MCP is disabled by the user: its server is not contacted.
    Disabled,
}

impl InstanceState {
    /// French label for the interface.
    pub fn label(self) -> &'static str {
        match self {
            Self::Catalogue => "catalogue",
            Self::Connected => "connecté",
            Self::SessionRequired => "connexion requise",
            Self::Unreachable => "injoignable",
            Self::Disabled => "coupé",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceStatus {
    pub state: InstanceState,
    /// Last successful reload.
    pub refreshed_at: Option<SystemTime>,
    /// Why the last reload failed, in French, after the output filter. `None` after a success.
    pub last_error: Option<String>,
}

/// Tools that changed on an instance since the user last acknowledged its updates, by original name, each list
/// sorted. Feeds the banner « MCP mis à jour, N nouveaux outils ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpUpdate {
    pub instance: InstanceId,
    pub added: Vec<ToolName>,
    pub modified: Vec<ToolName>,
    pub removed: Vec<ToolName>,
    /// Last change folded into this update.
    pub at: SystemTime,
}

impl McpUpdate {
    /// `pending` with `diff` folded in, relative to what the user last acknowledged: a tool added then removed
    /// leaves no trace, a tool removed then listed again counts as modified (it may differ). `None` when nothing
    /// remains.
    fn fold(pending: Option<Self>, instance: &InstanceId, diff: &ToolDiff, at: SystemTime) -> Option<Self> {
        let set = |v: Option<&Vec<ToolName>>| v.into_iter().flatten().cloned().collect::<BTreeSet<_>>();
        let mut added = set(pending.as_ref().map(|p| &p.added));
        let mut modified = set(pending.as_ref().map(|p| &p.modified));
        let mut removed = set(pending.as_ref().map(|p| &p.removed));
        for name in &diff.added {
            if removed.remove(name) {
                modified.insert(name.clone());
            } else {
                added.insert(name.clone());
            }
        }
        for name in &diff.modified {
            if !added.contains(name) {
                modified.insert(name.clone());
            }
        }
        for name in &diff.removed {
            if !added.remove(name) {
                modified.remove(name);
                removed.insert(name.clone());
            }
        }
        if added.is_empty() && modified.is_empty() && removed.is_empty() {
            return None;
        }
        Some(Self {
            instance: instance.clone(),
            added: added.into_iter().collect(),
            modified: modified.into_iter().collect(),
            removed: removed.into_iter().collect(),
            at,
        })
    }
}

/// How one reload ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Same fingerprint: nothing was written, no client was told.
    Unchanged,
    /// The registry took the new list; clients were told through the registry's `ToolsChanged`.
    Changed(ToolDiff),
    /// The last known list is kept; the instance is in this state.
    Failed(InstanceState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    /// The state shown for an instance changed.
    StateChanged { instance: InstanceId, state: InstanceState },
    /// A reload of the list ended.
    Reloaded {
        instance: InstanceId,
        outcome: ReloadOutcome,
    },
    /// The pending update of an instance changed (created or grown).
    UpdatePending { instance: InstanceId },
    /// The user acknowledged the update of an instance.
    Acknowledged { instance: InstanceId },
}

#[derive(Debug, Clone)]
struct Slot {
    mcp: McpId,
    /// State given by the last reload, whatever the settings.
    base: InstanceState,
    disabled: bool,
    refreshed_at: Option<SystemTime>,
    last_error: Option<String>,
    pending: Option<McpUpdate>,
}

impl Slot {
    fn state(&self) -> InstanceState {
        if self.disabled {
            InstanceState::Disabled
        } else {
            self.base
        }
    }
}

/// States and pending updates of every instance, shared by the list watcher and the interface.
#[derive(Debug, Clone)]
pub struct UpdateTracker {
    slots: Arc<Mutex<BTreeMap<InstanceId, Slot>>>,
    events: broadcast::Sender<UpdateEvent>,
}

impl UpdateTracker {
    /// Every instance of the configuration, in `Catalogue`.
    pub fn new(config: &Config) -> Self {
        let mut slots = BTreeMap::new();
        for mcp in &config.mcp {
            for env in mcp.instances.keys() {
                slots.insert(
                    InstanceId::new(env.clone(), mcp.id.clone()),
                    Slot {
                        mcp: mcp.id.clone(),
                        base: InstanceState::Catalogue,
                        disabled: false,
                        refreshed_at: None,
                        last_error: None,
                        pending: None,
                    },
                );
            }
        }
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            slots: Arc::new(Mutex::new(slots)),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UpdateEvent> {
        self.events.subscribe()
    }

    pub fn state(&self, instance: &InstanceId) -> Option<InstanceState> {
        self.lock().get(instance).map(Slot::state)
    }

    pub fn status(&self, instance: &InstanceId) -> Option<InstanceStatus> {
        self.lock().get(instance).map(|slot| InstanceStatus {
            state: slot.state(),
            refreshed_at: slot.refreshed_at,
            last_error: slot.last_error.clone(),
        })
    }

    /// Every instance, sorted.
    pub fn statuses(&self) -> Vec<(InstanceId, InstanceStatus)> {
        self.lock()
            .iter()
            .map(|(instance, slot)| {
                let status = InstanceStatus {
                    state: slot.state(),
                    refreshed_at: slot.refreshed_at,
                    last_error: slot.last_error.clone(),
                };
                (instance.clone(), status)
            })
            .collect()
    }

    /// Updates waiting to be acknowledged, sorted by instance.
    pub fn pending(&self) -> Vec<McpUpdate> {
        self.lock().values().filter_map(|s| s.pending.clone()).collect()
    }

    pub fn pending_for(&self, instance: &InstanceId) -> Option<McpUpdate> {
        self.lock().get(instance)?.pending.clone()
    }

    /// Clears the pending update of `instance`. `false` if there was none.
    pub fn acknowledge(&self, instance: &InstanceId) -> bool {
        let cleared = self
            .lock()
            .get_mut(instance)
            .and_then(|slot| slot.pending.take())
            .is_some();
        if cleared {
            self.send(UpdateEvent::Acknowledged {
                instance: instance.clone(),
            });
        }
        cleared
    }

    /// A reload gave a list the registry took.
    pub(crate) fn record_success(&self, instance: &InstanceId, diff: &ToolDiff, at: SystemTime) {
        let mut events = Vec::new();
        {
            let mut slots = self.lock();
            let Some(slot) = slots.get_mut(instance) else {
                return;
            };
            let before = slot.state();
            slot.base = InstanceState::Connected;
            slot.refreshed_at = Some(at);
            slot.last_error = None;
            if !diff.is_empty() {
                let next = McpUpdate::fold(slot.pending.take(), instance, diff, at);
                if next.is_some() {
                    events.push(UpdateEvent::UpdatePending {
                        instance: instance.clone(),
                    });
                }
                slot.pending = next;
            }
            if slot.state() != before {
                events.insert(0, Self::state_event(instance, slot.state()));
            }
        }
        let outcome = if diff.is_empty() {
            ReloadOutcome::Unchanged
        } else {
            ReloadOutcome::Changed(diff.clone())
        };
        events.push(UpdateEvent::Reloaded {
            instance: instance.clone(),
            outcome,
        });
        events.into_iter().for_each(|e| self.send(e));
    }

    /// A reload failed: the last known list stays, the instance goes to `state`. `message` is already filtered.
    pub(crate) fn record_failure(&self, instance: &InstanceId, state: InstanceState, message: String) {
        let changed = {
            let mut slots = self.lock();
            let Some(slot) = slots.get_mut(instance) else {
                return;
            };
            let before = slot.state();
            slot.base = state;
            slot.last_error = Some(message);
            (slot.state() != before).then(|| slot.state())
        };
        if let Some(shown) = changed {
            self.send(Self::state_event(instance, shown));
        }
        self.send(UpdateEvent::Reloaded {
            instance: instance.clone(),
            outcome: ReloadOutcome::Failed(state),
        });
    }

    /// Marks the instances of the disabled MCPs as `Disabled`, and the others back to the state of their last
    /// reload.
    pub(crate) fn set_disabled(&self, disabled: &BTreeSet<McpId>) {
        let mut events = Vec::new();
        {
            let mut slots = self.lock();
            for (instance, slot) in slots.iter_mut() {
                let before = slot.state();
                slot.disabled = disabled.contains(&slot.mcp);
                if slot.state() != before {
                    events.push(Self::state_event(instance, slot.state()));
                }
            }
        }
        events.into_iter().for_each(|e| self.send(e));
    }

    pub(crate) fn is_disabled(&self, instance: &InstanceId) -> bool {
        self.lock().get(instance).is_some_and(|s| s.disabled)
    }

    fn state_event(instance: &InstanceId, state: InstanceState) -> UpdateEvent {
        UpdateEvent::StateChanged {
            instance: instance.clone(),
            state,
        }
    }

    fn send(&self, event: UpdateEvent) {
        // No receiver is not an error: the interface may not be open.
        let _ = self.events.send(event);
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<InstanceId, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_proxy_core::config::EnvId;

    fn names(list: &[&str]) -> Vec<ToolName> {
        list.iter().map(|n| ToolName::new(*n).unwrap()).collect()
    }

    fn diff(added: &[&str], modified: &[&str], removed: &[&str]) -> ToolDiff {
        ToolDiff {
            added: names(added),
            removed: names(removed),
            modified: names(modified),
        }
    }

    fn instance() -> InstanceId {
        InstanceId::new(EnvId::new("dev").unwrap(), McpId::new("kibana-logs").unwrap())
    }

    #[test]
    fn updates_fold_relative_to_the_last_acknowledgement() {
        let at = SystemTime::UNIX_EPOCH;
        let first = McpUpdate::fold(None, &instance(), &diff(&["a", "b"], &["c"], &["d"]), at).unwrap();
        assert_eq!(
            (first.added.clone(), first.modified.clone(), first.removed.clone()),
            (names(&["a", "b"]), names(&["c"]), names(&["d"]))
        );
        // `a` disappears again, `b` changes (still new), `d` comes back, `c` is removed.
        let second = McpUpdate::fold(Some(first), &instance(), &diff(&["d"], &["b"], &["a", "c"]), at).unwrap();
        assert_eq!(second.added, names(&["b"]));
        assert_eq!(second.modified, names(&["d"]));
        assert_eq!(second.removed, names(&["c"]));
        let added_then_removed = McpUpdate::fold(None, &instance(), &diff(&["x"], &[], &[]), at);
        assert!(McpUpdate::fold(added_then_removed, &instance(), &diff(&[], &[], &["x"]), at).is_none());
    }
}
