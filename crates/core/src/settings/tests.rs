use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;

use super::store::{parse, render};
use super::*;
use crate::config::{Category, Channel, Config, Decision, EnvId, McpId, PolicyTree, ProfileId, Rule, ToolName};
use crate::policy::tests::{insert, tree_strategy};
use crate::policy::{RulePosition, Selector};

/// A fresh, empty directory under the system temporary directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mcp-proxy-settings-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rule(decision: Decision) -> Rule {
    Rule {
        decision,
        locked: false,
    }
}

fn full_settings() -> UserSettings {
    let mut hardening = PolicyTree::default();
    let prod = EnvId::new("prod").unwrap();
    let k8s = McpId::new("k8s-admin").unwrap();
    let positions = [
        (None, None, Selector::Default, Decision::Confirm(Channel::Elicitation)),
        (
            Some(&prod),
            None,
            Selector::Category(Category::Read),
            Decision::Confirm(Channel::Notification),
        ),
        (
            Some(&prod),
            Some(&k8s),
            Selector::Category(Category::Write),
            Decision::Deny,
        ),
        (
            Some(&prod),
            Some(&k8s),
            Selector::Tool(ToolName::new("list_pods").unwrap()),
            Decision::Allow,
        ),
    ];
    for (env, mcp, selector, decision) in positions {
        let position = RulePosition {
            env: env.cloned(),
            mcp: mcp.cloned(),
            selector,
        };
        insert(&mut hardening, &position, rule(decision));
    }
    UserSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        profile: Some(ProfileId::new("support").unwrap()),
        exposed_environments: [prod, EnvId::new("rec").unwrap()].into(),
        disabled_mcp: [k8s].into(),
        hardening,
        autostart: true,
        beta_channel: true,
        gateway_suspended: true,
    }
}

fn assert_strictest(loaded: &LoadedSettings) {
    assert_eq!(loaded.settings, UserSettings::default());
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let profile = loaded.settings.active_profile(&config).unwrap();
    assert!(profile.strictest);
    assert!(loaded.settings.exposed_environments(&config).is_empty());
}

// Exigence : EX-004-08
#[test]
fn round_trip_through_the_file() {
    let dir = TempDir::new();
    let store = JsonFileStore::new(dir.file("nested/settings.json"));
    let settings = full_settings();
    store.save(&settings).unwrap();
    let loaded = store.load();
    assert_eq!(loaded.origin, SettingsOrigin::Stored);
    assert_eq!(loaded.settings, settings);

    // Saving again replaces the file and leaves no temporary file behind.
    let mut changed = settings.clone();
    changed.gateway_suspended = false;
    store.save(&changed).unwrap();
    assert_eq!(store.load().settings, changed);
    let names: Vec<_> = std::fs::read_dir(dir.file("nested"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["settings.json"]);
}

// Exigence : EX-004-08
#[test]
fn file_format_mirrors_the_config_trees() {
    let text = String::from_utf8(render(&full_settings()).unwrap()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["profile"], "support");
    assert_eq!(value["hardening"]["default"]["decision"], "confirm");
    assert_eq!(value["hardening"]["default"]["channel"], "elicitation");
    assert_eq!(value["hardening"]["env"]["prod"]["read"]["channel"], "notification");
    assert_eq!(
        value["hardening"]["env"]["prod"]["mcp"]["k8s-admin"]["write"]["decision"],
        "deny"
    );
    assert_eq!(
        value["hardening"]["env"]["prod"]["mcp"]["k8s-admin"]["tool"]["list_pods"]["rule"]["decision"],
        "allow"
    );
    assert!(value["hardening"]["default"].get("locked").is_none());
}

// Exigence : EX-004-08
#[test]
fn missing_file_gives_the_strictest_settings() {
    let dir = TempDir::new();
    let loaded = JsonFileStore::new(dir.file("settings.json")).load();
    assert_eq!(loaded.origin, SettingsOrigin::Missing);
    assert_strictest(&loaded);
}

fn valid_json() -> serde_json::Value {
    serde_json::from_slice(&render(&full_settings()).unwrap()).unwrap()
}

// Exigence : EX-004-08
#[test]
fn unreadable_file_gives_the_strictest_settings() {
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("not json", b"{ profile: support".to_vec()),
        ("not utf-8", vec![0xff, 0xfe, 0x00]),
        ("array", b"[]".to_vec()),
        ("too big", vec![b' '; 1024 * 1024 + 1]),
    ];
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let mut edits: Vec<(&str, Edit)> = vec![
        ("other schema version", Box::new(|v| v["schema_version"] = 2.into())),
        (
            "missing field",
            Box::new(|v| drop(v.as_object_mut().unwrap().remove("gateway_suspended"))),
        ),
        ("unknown field", Box::new(|v| v["extra"] = true.into())),
        ("bad profile id", Box::new(|v| v["profile"] = "Support!".into())),
        (
            "bad environment id",
            Box::new(|v| v["exposed_environments"] = serde_json::json!(["PROD"])),
        ),
        ("bad mcp id", Box::new(|v| v["disabled_mcp"] = serde_json::json!([""]))),
        (
            "locked user rule",
            Box::new(|v| v["hardening"]["default"]["locked"] = true.into()),
        ),
        (
            "unknown decision",
            Box::new(|v| v["hardening"]["default"]["decision"] = "maybe".into()),
        ),
        (
            "confirm without channel",
            Box::new(|v| v["hardening"]["default"] = serde_json::json!({ "decision": "confirm" })),
        ),
        (
            "channel with deny",
            Box::new(|v| {
                v["hardening"]["default"] = serde_json::json!({ "decision": "deny", "channel": "notification" })
            }),
        ),
        (
            "unknown selector",
            Box::new(|v| v["hardening"]["env"]["prod"]["admin"] = serde_json::json!({ "decision": "deny" })),
        ),
        (
            "bad tool name",
            Box::new(|v| {
                v["hardening"]["env"]["prod"]["mcp"]["k8s-admin"]["tool"]["a b"] =
                    serde_json::json!({ "rule": { "decision": "deny" } })
            }),
        ),
        ("wrong type", Box::new(|v| v["autostart"] = "yes".into())),
    ];
    for (name, edit) in edits.drain(..) {
        let mut value = valid_json();
        edit(&mut value);
        cases.push((name, serde_json::to_vec(&value).unwrap()));
    }
    for (name, bytes) in cases {
        let dir = TempDir::new();
        let path = dir.file("settings.json");
        std::fs::write(&path, &bytes).unwrap();
        let loaded = JsonFileStore::new(&path).load();
        assert!(
            matches!(loaded.origin, SettingsOrigin::Reset(_)),
            "{name}: {:?}",
            loaded.origin
        );
        assert_strictest(&loaded);
    }
}

// Exigence : EX-004-08
#[test]
fn a_directory_in_place_of_the_file_gives_the_strictest_settings() {
    let dir = TempDir::new();
    let path = dir.file("settings.json");
    std::fs::create_dir(&path).unwrap();
    let store = JsonFileStore::new(&path);
    let loaded = store.load();
    assert!(
        matches!(loaded.origin, SettingsOrigin::Reset(SettingsError::Io(_))),
        "{:?}",
        loaded.origin
    );
    assert_strictest(&loaded);
    assert!(matches!(store.save(&full_settings()), Err(SettingsError::Save(_))));
}

// Exigence : EX-004-12
#[test]
fn errors_never_quote_the_file() {
    let marker = "S3NT1NEL-value-from-the-file";
    let bodies = [
        format!("{{\"profile\": \"{marker}\""),
        format!("{{\"schema_version\": 1, \"profile\": \"{marker}\"}}"),
        valid_json()
            .to_string()
            .replace("\"support\"", &format!("\"{marker}\"")),
        valid_json()
            .to_string()
            .replace("\"list_pods\"", &format!("\"{marker} !\"")),
    ];
    for body in bodies {
        let error = parse(body.as_bytes()).unwrap_err();
        assert!(!error.to_string().contains(marker), "{error}");
        assert!(error.to_string().starts_with("réglages invalides"), "{error}");
    }
}

// Exigence : EX-004-08
#[test]
fn defaults_are_the_strictest() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let settings = UserSettings::default();
    assert_eq!(settings.profile, None);
    assert!(settings.exposed_environments.is_empty());
    assert!(settings.disabled_mcp.is_empty());
    assert!(!settings.autostart && !settings.beta_channel && !settings.gateway_suspended);
    let engine = settings.engine(&config).unwrap();
    assert!(engine.profile().strictest);
    assert_eq!(engine.profile().unknown_tools, Decision::Deny);
}

// Exigence : EX-004-11
#[test]
fn a_profile_absent_from_the_config_falls_back_to_strictest_with_nothing_exposed() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let mut settings = full_settings();
    settings.profile = Some(ProfileId::new("ancien-profil").unwrap());
    assert!(settings.active_profile(&config).unwrap().strictest);
    assert!(settings.exposed_environments(&config).is_empty());
}

// Exigence : EX-004-11
#[test]
fn choosing_a_profile_exposes_its_defaults_and_keeps_hardening() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let mut settings = full_settings();
    let hardening = settings.hardening.clone();
    let developer = &config.profiles[&ProfileId::new("developpeur").unwrap()];
    settings.choose_profile(developer);
    assert_eq!(settings.active_profile(&config).unwrap().id, developer.id);
    let expected: std::collections::BTreeSet<_> = developer.default_environments.iter().cloned().collect();
    assert_eq!(settings.exposed_environments(&config), expected);
    assert_eq!(settings.hardening, hardening);
}

// Exigence : EX-004-11
#[test]
fn environments_unknown_to_the_config_are_not_exposed() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let mut settings = full_settings();
    settings.exposed_environments.insert(EnvId::new("ancien-env").unwrap());
    let exposed = settings.exposed_environments(&config);
    assert!(!exposed.contains(&EnvId::new("ancien-env").unwrap()));
    assert!(exposed.contains(&EnvId::new("prod").unwrap()));
}

/// Locks are meaningless in the user tree: generated trees go through the file without them.
fn unlocked(mut tree: PolicyTree) -> PolicyTree {
    let strip = |set: &mut crate::config::RuleSet| {
        for rule in [&mut set.default, &mut set.read, &mut set.write, &mut set.destructive]
            .into_iter()
            .flatten()
        {
            rule.locked = false;
        }
    };
    strip(&mut tree.rules);
    for env in tree.env.values_mut() {
        strip(&mut env.rules);
        for mcp in env.mcp.values_mut() {
            strip(&mut mcp.rules);
            mcp.tools.values_mut().for_each(|r| r.locked = false);
        }
    }
    tree
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    // Exigence : EX-004-08
    #[test]
    fn any_hardening_tree_round_trips(tree in tree_strategy(true, 0.3), flags in any::<(bool, bool, bool)>()) {
        let mut settings = full_settings();
        settings.hardening = unlocked(tree);
        (settings.autostart, settings.beta_channel, settings.gateway_suspended) = flags;
        let bytes = render(&settings).unwrap();
        prop_assert_eq!(parse(&bytes).unwrap(), settings);
    }
}
