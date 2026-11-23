//! The dialog « Clients LLM » (spec 023) as the window receives it: the view, the previews and the outcomes,
//! serialized, must be `ui/src/api/clients.example.json`, which the tests of `ui/` check against `types.ts`. Then
//! the three commands on the assembly the application runs: what is journaled, what is refused.
//!
//! After a deliberate change of the view: `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p mcp-proxy-gateway --test
//! clients`, then `npm test` in `ui/`.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use mcp_proxy_core::secrets::{SecretStore, platform_key_wrapper};
use mcp_proxy_core::trace::{EntryKind, JournalFilter, ManualWallClock};
use mcp_proxy_deploy::clients::{Action, LocationId, RealFiles, plan};
use mcp_proxy_gateway::clients_view::{ApplyOutcome, ClientsAccess, backup_stamp, clients_view, preview_view};
use mcp_proxy_gateway::interface::InterfaceError;
use mcp_proxy_gateway::notify::UnavailablePresenter;
use mcp_proxy_gateway::runtime::{DataDirs, Gateway, Logging, Persistence, RuntimeDeps};
use mcp_proxy_testkit::{CapturedLogs, FakeAuthUi};
use serde_json::{Value, json};

/// Sun, 22 Nov 2026 14:03:07 UTC.
const NOW_SECS: u64 = 1_795_356_187;
const SENTINEL: &str = "ghp_SENTINEL0123456789abcdef";
const FIXTURE_EXE: &str = r"C:\Users\exemple\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-clients-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn access(root: &Path) -> ClientsAccess {
    ClientsAccess::new(Some(root.join("Roaming")), Some(root.join("Local"))).unwrap()
}

fn install_exe(access: &ClientsAccess) {
    let exe = access.env.installed_exe();
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"MZ").unwrap();
}

fn write(access: &ClientsAccess, id: LocationId, text: &str) {
    let location = access.env.location(id);
    std::fs::create_dir_all(&location.dir).unwrap();
    std::fs::write(&location.file, text).unwrap();
}

fn other_servers(exe: &str) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"github\": {{\n      \"command\": \"npx\",\n      \"env\": {{ \"TOKEN\": \"{SENTINEL}\" }}\n    }},\n    \"mcp-proxy\": {{\n      \"command\": {},\n      \"args\": [\"stdio\"]\n    }}\n  }}\n}}\n",
        Value::String(exe.to_owned())
    )
}

/// The serialized form, the temporary exe written as a Windows one (as is, and as it appears inside JSON text),
/// the fingerprints fixed.
fn normalized(value: Value, exe: &Path) -> Value {
    fn inner(s: &str) -> String {
        let quoted = Value::String(s.to_owned()).to_string();
        quoted[1..quoted.len() - 1].to_owned()
    }
    // `entry` and `snippet` are JSON text: the exe is written there escaped.
    fn walk(value: &mut Value, from: &str, to: &str, json_text: bool) {
        match value {
            Value::String(s) if json_text => *s = s.replace(&inner(from), &inner(to)),
            Value::String(s) => *s = s.replace(from, to),
            Value::Array(items) => items.iter_mut().for_each(|v| walk(v, from, to, false)),
            Value::Object(map) => map
                .iter_mut()
                .for_each(|(k, v)| walk(v, from, to, k == "entry" || k == "snippet")),
            _ => {}
        }
    }
    let mut value = value;
    walk(&mut value, &exe.display().to_string(), FIXTURE_EXE, false);
    for preview in value["previews"].as_array_mut().into_iter().flatten() {
        preview["fingerprint"] = json!("0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0");
    }
    value
}

// Exigence : EX-023-14
#[test]
fn the_window_receives_exactly_the_contract() {
    let root = TempDir::new();
    let access = access(&root.0);
    install_exe(&access);
    let exe = access.env.installed_exe();
    let files = RealFiles;
    // Claude Desktop configured in its documented folder, its MSIX package with an old entry, VS Code unreadable.
    write(
        &access,
        LocationId::ClaudeDesktop,
        &other_servers(&exe.display().to_string()),
    );
    write(
        &access,
        LocationId::ClaudeDesktopPackage,
        "{\"mcpServers\": {\"mcp-proxy\": {\"command\": \"C:\\\\Users\\\\exemple\\\\Downloads\\\\mcp-proxy.exe\", \"args\": [\"stdio\"], \"env\": {}}}}",
    );
    write(
        &access,
        LocationId::VsCode,
        "{\n  \"servers\": {\n    \"a\": {}\n    \"b\": {}\n  }\n}\n",
    );
    let view = clients_view(&files, &access.env);

    let target = Some(exe.clone());
    let preview = |id, action| preview_view(&plan(&files, &access.env, id, action, target.as_deref()).unwrap());
    let mut previews = vec![
        preview(LocationId::ClaudeDesktop, Action::Install),
        preview(LocationId::ClaudeDesktop, Action::Remove),
        preview(LocationId::ClaudeDesktopPackage, Action::Install),
        preview(LocationId::VsCode, Action::Install),
    ];
    write(
        &access,
        LocationId::VsCode,
        "{\n  \"servers\": {\n    \"a\": {}\n  }\n}\n",
    );
    previews.push(preview(LocationId::VsCode, Action::Install));
    std::fs::remove_file(access.env.location(LocationId::VsCode).file).unwrap();
    previews.push(preview(LocationId::VsCode, Action::Install));

    // Without MCP Proxy installed, and without any client.
    let bare = TempDir::new();
    let bare = clients_view(&files, &self::access(&bare.0).env);

    let fixture = normalized(
        json!({
            "getLlmClients": view,
            "notInstalled": bare,
            "previews": previews,
            "outcomes": [
                ApplyOutcome::Applied { backup: Some(format!("claude_desktop_config.json.bak-{}", backup_stamp(UNIX_EPOCH + Duration::from_secs(NOW_SECS)))) },
                ApplyOutcome::Applied { backup: None },
                ApplyOutcome::Nothing,
                ApplyOutcome::Changed,
            ],
        }),
        &exe,
    );
    let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    assert!(!text.contains(SENTINEL), "no value of another server");
    assert!(!text.contains("github"), "no name of another server");
    assert!(!text.contains(&root.0.display().to_string()), "no path of this machine");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/api/clients.example.json");
    if std::env::var_os("MCP_PROXY_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    } else {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == text,
            "{} is not the view of the clients any more: run `MCP_PROXY_WRITE_FIXTURES=1 cargo test -p \
             mcp-proxy-gateway --test clients`, then align ui/src/api/types.ts until `npm test` passes in ui/",
            path.display()
        );
    }

    let kinds: Vec<&str> = fixture["getLlmClients"]["locations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["state"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["configured", "otherCommand", "unreadable"]);
    let changes: Vec<&str> = fixture["previews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["change"].as_str().unwrap())
        .collect();
    assert_eq!(changes, ["nothing", "remove", "replace", "manual", "add", "create"]);
    assert_eq!(fixture["notInstalled"]["command"], Value::Null);
    assert_eq!(
        backup_stamp(UNIX_EPOCH + Duration::from_secs(NOW_SECS)),
        "20261122-140307"
    );
}

async fn start(dir: &Path, clock: ManualWallClock) -> Gateway {
    let dirs = DataDirs::under(dir);
    Gateway::start(RuntimeDeps {
        config: Arc::new(mcp_proxy_gateway::modules::embedded_config().unwrap()),
        persistence: Persistence::sealed(&dirs, platform_key_wrapper()),
        dirs,
        store: SecretStore::for_platform().unwrap(),
        auth_ui: Arc::new(FakeAuthUi::default()),
        presenter: Arc::new(UnavailablePresenter),
        clock: Arc::new(clock),
        logging: Logging::Local,
        utc_offset: 0,
        system: "système de test".to_owned(),
    })
    .await
    .unwrap()
}

fn events(gateway: &Gateway) -> Vec<String> {
    gateway
        .journal()
        .snapshot(&JournalFilter::default())
        .into_iter()
        .filter(|e| e.kind == EntryKind::Event)
        .map(|e| e.summary)
        .collect()
}

// Exigence : EX-023-13
// Exigence : EX-023-14
#[tokio::test(flavor = "current_thread")]
async fn the_commands_journal_what_the_user_did_and_nothing_of_the_files() {
    let (logs, _guard) = CapturedLogs::start();
    let data = TempDir::new();
    let home = TempDir::new();
    let clock = ManualWallClock::new(UNIX_EPOCH + Duration::from_secs(NOW_SECS));
    let gateway = start(&data.0, clock).await;
    let access = access(&home.0);
    install_exe(&access);
    let exe = access.env.installed_exe().display().to_string();
    let text = other_servers(&exe).replace(
        ",\n    \"mcp-proxy\": {\n      \"command\": ",
        ",\n    \"old\": {\n      \"command\": ",
    );
    write(&access, LocationId::ClaudeDesktop, &text);

    assert_eq!(gateway.known_location("claude"), Err(InterfaceError::UnknownClient));
    assert_eq!(
        gateway.known_location(r"C:\config.json"),
        Err(InterfaceError::UnknownClient)
    );
    assert_eq!(
        gateway.known_client_action("delete"),
        Err(InterfaceError::UnknownAction)
    );
    let id = gateway.known_location("claude-desktop").unwrap();
    let install = gateway.known_client_action("install").unwrap();

    let view = gateway.llm_clients(&access).await.unwrap();
    assert_eq!(view.command.as_deref(), Some(exe.as_str()));
    let shown = gateway.preview_llm_client(&access, id, install).await.unwrap();
    assert!(shown.backup);
    // A rewrite by the client between the preview and the click: refused, then applied once shown again.
    std::fs::write(access.env.location(id).file, text.replace("npx", "npx.cmd")).unwrap();
    let outcome = gateway
        .apply_llm_client(&access, id, install, shown.fingerprint.clone())
        .await
        .unwrap();
    assert_eq!(outcome, ApplyOutcome::Changed);
    let shown = gateway.preview_llm_client(&access, id, install).await.unwrap();
    let outcome = gateway
        .apply_llm_client(&access, id, install, shown.fingerprint)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        ApplyOutcome::Applied {
            backup: Some("claude_desktop_config.json.bak-20261122-140307".to_owned())
        }
    );
    let again = gateway.preview_llm_client(&access, id, install).await.unwrap();
    assert_eq!(
        gateway
            .apply_llm_client(&access, id, install, again.fingerprint)
            .await
            .unwrap(),
        ApplyOutcome::Nothing
    );

    // A client not installed: refused, nothing created, the error says it once.
    let vscode = gateway.known_location("vscode").unwrap();
    let refused = gateway.preview_llm_client(&access, vscode, install).await.unwrap_err();
    assert_eq!(
        refused.to_string(),
        "VS Code n'est pas installé sur ce poste : rien n'est créé"
    );
    assert!(!access.env.location(vscode).dir.exists());

    let events = events(&gateway);
    assert!(
        events.contains(&"configuration de Claude Desktop changée depuis l'aperçu : rien n'a été écrit".to_owned())
    );
    assert!(events.contains(&"MCP Proxy ajouté à la configuration de Claude Desktop par l'utilisateur".to_owned()));
    let home_text = home.0.display().to_string();
    for text in events.iter().chain([&logs.text()]) {
        assert!(!text.contains(SENTINEL), "{text}");
        assert!(!text.contains(&home_text), "{text}");
        assert!(!text.contains("mcpServers"), "{text}");
    }
}
