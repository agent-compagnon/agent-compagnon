//! The contract between the snapshot and the interface (spec 016): the snapshot of the example configuration, in a
//! state where every optional field is filled at least once, serialized as the window receives it, must be the file
//! the tests of `ui/` check against `ui/src/api/types.ts`. A field added, renamed or retyped on either side fails one
//! of the two tests.
//!
//! After a deliberate change of the snapshot: `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p mcp-proxy-gateway --test
//! snapshot_contract`, then `npm test` in `ui/`.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mcp_proxy_core::config::{EnvId, McpId, ProfileId, SessionId, ToolName};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::secrets::SecretStore;
use mcp_proxy_core::session::{SessionRegistry, SessionState};
use mcp_proxy_core::settings::UserSettings;
use mcp_proxy_gateway::registry::{EmbeddedCatalog, InstanceId, NoCatalogCache, Registry};
use mcp_proxy_gateway::updates::{InstanceState, InstanceStatus, McpUpdate};
use mcp_proxy_gateway::view::{Alert, AlertKind, AlertLevel, AppSnapshot, SnapshotInputs};

/// Sun, 22 Nov 2026 14:00:00 UTC.
const NOW_SECS: u64 = 1_795_356_000;

fn at(offset_secs: i64) -> SystemTime {
    let secs = NOW_SECS.checked_add_signed(offset_secs).unwrap();
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/api/snapshot.example.json")
}

fn example_snapshot() -> AppSnapshot {
    let config = Arc::new(mcp_proxy_gateway::modules::embedded_config().unwrap());
    let store = Arc::new(SecretStore::for_platform().unwrap());
    let filter = Arc::new(OutputFilter::new(store, &config.filter).unwrap());
    let embedded = EmbeddedCatalog::builtin(&config).unwrap();
    let mut registry = Registry::new(Arc::clone(&config), &embedded, Box::new(NoCatalogCache), filter).unwrap();

    let mut settings = UserSettings::default();
    settings.choose_profile(&config.profiles[&ProfileId::new("developpeur").unwrap()]);
    settings.exposed_environments.insert(EnvId::new("prod").unwrap());
    registry.apply_settings(&settings);

    let sessions = SessionRegistry::from_config(&config).snapshot();
    let states = [
        ("sso-corp", SessionState::Active { until: at(3 * 3600) }),
        ("sso-tools", SessionState::LoginRequested { since: at(-20) }),
        ("sso-dev", SessionState::Expired),
        ("sso-int", SessionState::LoginRequested { since: at(-5) }),
    ];
    let sessions: Vec<_> = sessions
        .into_iter()
        .map(|(info, state)| {
            let state = states
                .iter()
                .find(|(id, _)| *id == info.id.as_str())
                .map_or(state, |(_, s)| *s);
            (info, state)
        })
        .collect();

    let instance = |env: &str, mcp: &str| InstanceId::new(EnvId::new(env).unwrap(), McpId::new(mcp).unwrap());
    let instances = [
        (
            instance("dev", "gitlab-ops"),
            InstanceStatus {
                state: InstanceState::Connected,
                refreshed_at: Some(at(-60)),
                last_error: None,
            },
        ),
        (
            instance("prod", "kibana-logs"),
            InstanceStatus {
                state: InstanceState::Unreachable,
                refreshed_at: None,
                last_error: Some("serveur injoignable (délai dépassé)".to_owned()),
            },
        ),
    ];
    let updates = [McpUpdate {
        instance: instance("dev", "gitlab-ops"),
        added: vec![ToolName::new("list_runners").unwrap()],
        modified: vec![],
        removed: vec![ToolName::new("retry_job").unwrap()],
        at: at(-60),
    }];
    let alerts = [Alert {
        level: AlertLevel::Error,
        kind: AlertKind::StartupHardening,
        message: "Le dossier des fenêtres de connexion n'a pas pu être vidé (accès refusé) : aucune connexion par \
                  fenêtre n'est possible jusqu'au prochain démarrage de MCP Proxy."
            .to_owned(),
    }];
    AppSnapshot::build(&SnapshotInputs {
        config: &config,
        registry: &registry,
        settings: &settings,
        sessions: &sessions,
        instances: &instances,
        updates: &updates,
        pending_confirmations: 1,
        alerts: &alerts,
        login_requesters: &[(SessionId::new("sso-tools").unwrap(), "Claude Desktop".to_owned())],
        utc_offset: 0,
    })
}

// Exigence : EX-016-06
#[test]
fn the_interface_reads_the_snapshot_of_the_example_configuration() {
    let snapshot = example_snapshot();
    let mut json = serde_json::to_string_pretty(&snapshot).unwrap();
    json.push('\n');
    let path = fixture_path();
    if std::env::var_os("MCP_PROXY_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &json).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        current == json,
        "{} is not the snapshot of the example configuration any more: run `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p \
         mcp-proxy-gateway --test snapshot_contract`, then align ui/src/api/types.ts until `npm test` passes in ui/",
        path.display()
    );

    // Every optional field is filled at least once, so that its type is checked by the interface.
    let tools = snapshot.sessions.iter().find(|s| s.id == "sso-tools").unwrap();
    assert_eq!(tools.state.requested_by.as_deref(), Some("Claude Desktop"));
    let int = snapshot.sessions.iter().find(|s| s.id == "sso-int").unwrap();
    assert_eq!(
        int.state.requested_by, None,
        "a login asked from the interface names no client"
    );
    assert_eq!(snapshot.sessions[0].state.label, "active jusqu'à 17:00");
    assert!(
        snapshot
            .mcps
            .iter()
            .flat_map(|m| &m.instances)
            .any(|i| i.update.is_some())
    );
    assert!(
        snapshot
            .mcps
            .iter()
            .flat_map(|m| &m.instances)
            .any(|i| i.last_error.is_some())
    );
}
