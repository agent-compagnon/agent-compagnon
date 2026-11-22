use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use zeroize::Zeroizing;

use super::*;
use crate::config::{Config, Decision, EnvId, ProfileId, Rule};
use crate::secrets::test_support::TestWrapper;
use crate::secrets::{ProtectionFailure, SecretError};
use crate::settings::{SealedSettingsStore, SettingsError, SettingsOrigin, SettingsStore, UserSettings};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mcp-proxy-sealed-{}-{n}", std::process::id()));
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

/// A Windows user as DPAPI sees it: what one user wraps, another cannot unwrap.
struct UserWrapper(u8);

impl KeyWrapper for UserWrapper {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError> {
        Ok([&[b'U', self.0][..], &key.iter().map(|b| b ^ 0x5c).collect::<Vec<_>>()].concat())
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        match wrapped {
            [b'U', user, rest @ ..] if *user == self.0 => Ok(Zeroizing::new(rest.iter().map(|b| b ^ 0x5c).collect())),
            _ => Err(SecretError::Protection(ProtectionFailure::Os(-2146893813))),
        }
    }
}

fn config() -> Config {
    Config::embedded(crate::config::TEST_MODULES).unwrap()
}

/// The softest settings a file could hold: the developer profile, every environment exposed, everything allowed.
fn permissive(config: &Config) -> UserSettings {
    let mut settings = UserSettings::default();
    settings.choose_profile(&config.profiles[&ProfileId::new("developpeur").unwrap()]);
    settings.exposed_environments = config.environments.iter().map(|e| e.id.clone()).collect();
    settings.hardening.rules.default = Some(Rule {
        decision: Decision::Allow,
        locked: false,
    });
    settings.autostart = true;
    settings.beta_channel = true;
    settings
}

fn store_in(dir: &TempDir, wrapper: impl KeyWrapper + 'static) -> SealedSettingsStore {
    let sealer = Arc::new(Sealer::open(dir.file("settings.key"), Box::new(wrapper)));
    SealedSettingsStore::new(dir.file("settings.json"), sealer)
}

/// What the strictest fallback is: no profile chosen (the `strictest` one applies), nothing exposed, no hardening
/// kept, every option back to its default.
fn assert_strictest(origin: &SettingsOrigin, settings: &UserSettings) {
    assert!(matches!(origin, SettingsOrigin::Reset(_)), "{origin:?}");
    assert_eq!(*settings, UserSettings::default());
    let config = config();
    assert!(settings.active_profile(&config).unwrap().strictest);
    assert!(settings.exposed_environments(&config).is_empty());
}

fn seal_error(origin: &SettingsOrigin) -> &SealError {
    match origin {
        SettingsOrigin::Reset(SettingsError::Seal(error)) => error,
        other => panic!("expected a seal error, got {other:?}"),
    }
}

// Exigence : EX-014-01
#[test]
fn round_trip_then_a_new_start_reads_the_same_settings() {
    let dir = TempDir::new();
    let config = config();
    let settings = permissive(&config);
    store_in(&dir, UserWrapper(1)).save(&settings).unwrap();

    // A new start: a new sealer reads the key from disk.
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert_eq!(loaded.origin, SettingsOrigin::Stored);
    assert_eq!(loaded.settings, settings);

    let file = std::fs::read(dir.file("settings.json")).unwrap();
    assert!(file.starts_with(b"MCP-PROXY-SEAL v1 hmac-sha256="));
    // The content stays readable after its header line.
    let content = &file[file.iter().position(|b| *b == b'\n').unwrap() + 1..];
    let json: serde_json::Value = serde_json::from_slice(content).unwrap();
    assert_eq!(json["profile"], "developpeur");
    let names: Vec<_> = std::fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}: no temporary file left");
}

// Exigence : EX-014-01
#[test]
fn a_missing_file_is_a_first_launch_not_a_reset() {
    let dir = TempDir::new();
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert_eq!(loaded.origin, SettingsOrigin::Missing);
    assert_eq!(loaded.settings, UserSettings::default());
    assert!(!dir.file("settings.key").exists(), "no key is created by a load");
}

// Exigence : EX-014-01
#[test]
fn one_byte_changed_anywhere_gives_the_strictest_settings() {
    let dir = TempDir::new();
    let config = config();
    store_in(&dir, UserWrapper(1)).save(&permissive(&config)).unwrap();
    let original = std::fs::read(dir.file("settings.json")).unwrap();
    // Every position, header included, and the file truncated or extended.
    let mut variants: Vec<Vec<u8>> = (0..original.len())
        .map(|i| {
            let mut changed = original.clone();
            changed[i] ^= 0x01;
            changed
        })
        .collect();
    variants.push(original[..original.len() - 1].to_vec());
    variants.push([original.as_slice(), b" "].concat());
    for changed in variants {
        std::fs::write(dir.file("settings.json"), &changed).unwrap();
        let loaded = store_in(&dir, UserWrapper(1)).load();
        assert_strictest(&loaded.origin, &loaded.settings);
    }
}

// Exigence : EX-014-01
#[test]
fn the_profile_changed_by_hand_with_the_seal_kept_is_refused() {
    let dir = TempDir::new();
    let mut settings = UserSettings::default();
    settings.choose_profile(&config().profiles[&ProfileId::new("lecture-seule").unwrap()]);
    store_in(&dir, UserWrapper(1)).save(&settings).unwrap();
    let text = String::from_utf8(std::fs::read(dir.file("settings.json")).unwrap()).unwrap();
    let edited = text
        .replace("\"lecture-seule\"", "\"developpeur\"")
        .replace("\"exposed_environments\": []", "\"exposed_environments\": [\"prod\"]");
    assert_ne!(edited, text);
    std::fs::write(dir.file("settings.json"), edited).unwrap();
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert_eq!(*seal_error(&loaded.origin), SealError::Mismatch);
    assert_strictest(&loaded.origin, &loaded.settings);
}

// Exigence : EX-014-01
#[test]
fn a_file_without_seal_gives_the_strictest_settings() {
    let dir = TempDir::new();
    let config = config();
    let store = store_in(&dir, UserWrapper(1));
    store.save(&permissive(&config)).unwrap();
    // What a previous version wrote: the plain JSON, valid, without the header line.
    let plain = crate::settings::store::render(&permissive(&config)).unwrap();
    std::fs::write(dir.file("settings.json"), &plain).unwrap();
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert_eq!(*seal_error(&loaded.origin), SealError::Unsealed);
    assert_strictest(&loaded.origin, &loaded.settings);

    // A header without its tag, or with a tag that is not hexadecimal.
    for header in [
        &b"MCP-PROXY-SEAL v1 hmac-sha256=\n"[..],
        b"MCP-PROXY-SEAL v1 hmac-sha256=zz",
    ] {
        std::fs::write(dir.file("settings.json"), [header, plain.as_slice()].concat()).unwrap();
        let loaded = store_in(&dir, UserWrapper(1)).load();
        assert_eq!(*seal_error(&loaded.origin), SealError::Malformed);
        assert_strictest(&loaded.origin, &loaded.settings);
    }
}

// Exigence : EX-014-01
#[test]
fn a_missing_key_gives_the_strictest_settings_and_the_next_save_creates_one() {
    let dir = TempDir::new();
    let config = config();
    store_in(&dir, UserWrapper(1)).save(&permissive(&config)).unwrap();
    std::fs::remove_file(dir.file("settings.key")).unwrap();

    let store = store_in(&dir, UserWrapper(1));
    let loaded = store.load();
    assert_eq!(*seal_error(&loaded.origin), SealError::KeyMissing);
    assert_strictest(&loaded.origin, &loaded.settings);

    // The user chooses a profile again: a new key seals the file, which then opens at the next start.
    let mut chosen = UserSettings::default();
    chosen.choose_profile(&config.profiles[&ProfileId::new("support").unwrap()]);
    store.save(&chosen).unwrap();
    assert!(dir.file("settings.key").exists());
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert_eq!(loaded.origin, SettingsOrigin::Stored);
    assert_eq!(loaded.settings, chosen);
}

// Exigence : EX-014-01
#[test]
fn an_unreadable_key_gives_the_strictest_settings() {
    let dir = TempDir::new();
    let config = config();
    store_in(&dir, UserWrapper(1)).save(&permissive(&config)).unwrap();
    let key = std::fs::read(dir.file("settings.key")).unwrap();
    assert!(key.starts_with(b"MCP-PROXY-SEAL-KEY v1\n"));

    // Copied into the profile of another Windows user: DPAPI does not open it.
    let loaded = store_in(&dir, UserWrapper(2)).load();
    assert!(
        matches!(seal_error(&loaded.origin), SealError::KeyUnreadable(_)),
        "{:?}",
        loaded.origin
    );
    assert_strictest(&loaded.origin, &loaded.settings);

    // Damaged, emptied, or replaced by a directory.
    for damaged in [&b"cle"[..], b"", b"MCP-PROXY-SEAL-KEY v1\n"] {
        std::fs::write(dir.file("settings.key"), damaged).unwrap();
        let loaded = store_in(&dir, UserWrapper(1)).load();
        assert!(
            matches!(seal_error(&loaded.origin), SealError::KeyUnreadable(_)),
            "{:?}",
            loaded.origin
        );
        assert_strictest(&loaded.origin, &loaded.settings);
    }
    // A key of the wrong length once unwrapped.
    std::fs::write(
        dir.file("settings.key"),
        [&b"MCP-PROXY-SEAL-KEY v1\nU\x01"[..], &[0u8; 16]].concat(),
    )
    .unwrap();
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert!(
        matches!(seal_error(&loaded.origin), SealError::KeyUnreadable(_)),
        "{:?}",
        loaded.origin
    );
    std::fs::remove_file(dir.file("settings.key")).unwrap();
    std::fs::create_dir(dir.file("settings.key")).unwrap();
    let loaded = store_in(&dir, UserWrapper(1)).load();
    assert!(
        matches!(seal_error(&loaded.origin), SealError::KeyUnreadable(_)),
        "{:?}",
        loaded.origin
    );

    // DPAPI that stops answering after the start: nothing opens, nothing is sealed.
    let wrapper = TestWrapper::default();
    let dir = TempDir::new();
    let sealer = Sealer::open(dir.file("settings.key"), Box::new(wrapper.clone()));
    let sealed = sealer.seal(SETTINGS_PURPOSE, b"{}").unwrap();
    wrapper.break_now();
    assert!(matches!(
        sealer.unseal(SETTINGS_PURPOSE, &sealed),
        Err(SealError::KeyUnreadable(_))
    ));
    assert!(sealer.seal(SETTINGS_PURPOSE, b"{}").is_err());
}

// Exigence : EX-014-01
#[test]
fn the_key_of_another_file_does_not_open_it() {
    let config = config();
    let first = TempDir::new();
    let second = TempDir::new();
    store_in(&first, UserWrapper(1)).save(&permissive(&config)).unwrap();
    store_in(&second, UserWrapper(1))
        .save(&UserSettings::default())
        .unwrap();
    // Same Windows user, but the key of another installation.
    std::fs::copy(second.file("settings.key"), first.file("settings.key")).unwrap();
    let loaded = store_in(&first, UserWrapper(1)).load();
    assert_eq!(*seal_error(&loaded.origin), SealError::Mismatch);
    assert_strictest(&loaded.origin, &loaded.settings);
}

// Exigence : EX-014-01
#[test]
fn a_seal_binds_the_file_to_its_purpose() {
    let dir = TempDir::new();
    let sealer = Sealer::open(dir.file("settings.key"), Box::new(UserWrapper(1)));
    let cache = sealer.seal(&catalog_purpose("jira", "rec"), b"{}").unwrap();
    assert_eq!(sealer.unseal(&catalog_purpose("jira", "rec"), &cache).unwrap(), b"{}");
    assert_eq!(
        sealer.unseal(&catalog_purpose("jira", "prod"), &cache),
        Err(SealError::Mismatch)
    );
    assert_eq!(sealer.unseal(SETTINGS_PURPOSE, &cache), Err(SealError::Mismatch));
}

// Exigence : EX-014-01
#[test]
fn neither_debug_nor_errors_show_the_key() {
    let dir = TempDir::new();
    let sealer = Sealer::open(dir.file("settings.key"), Box::new(UserWrapper(1)));
    let sealed = sealer.seal(SETTINGS_PURPOSE, b"{}").unwrap();
    let wrapped = std::fs::read(dir.file("settings.key")).unwrap();
    let key: Vec<u8> = wrapped[KEY_MAGIC.len() + 2..].iter().map(|b| b ^ 0x5c).collect();
    assert_eq!(key.len(), SEAL_KEY_LEN);
    let key_hex = hex(&key);
    let mut texts = vec![format!("{sealer:?}"), String::from_utf8_lossy(&sealed).into_owned()];
    for error in [
        SealError::KeyMissing,
        SealError::KeyUnreadable("x".to_owned()),
        SealError::KeyCreation("x".to_owned()),
        SealError::Unsealed,
        SealError::Malformed,
        SealError::Mismatch,
    ] {
        texts.push(format!("{error} {error:?}"));
    }
    for text in texts {
        assert!(!text.contains(&key_hex), "{text}");
    }
    assert!(!sealed.windows(key.len()).any(|w| w == key.as_slice()));
}

#[test]
fn a_save_without_usable_protection_fails() {
    let dir = TempDir::new();
    let wrapper = TestWrapper::default();
    wrapper.break_now();
    let sealer = Arc::new(Sealer::open(dir.file("settings.key"), Box::new(wrapper)));
    let store = SealedSettingsStore::new(dir.file("settings.json"), sealer);
    assert!(matches!(
        store.save(&UserSettings::default()),
        Err(SettingsError::Save(_))
    ));
    assert!(!dir.file("settings.json").exists());
    assert!(!dir.file("settings.key").exists());
}

fn decisions(config: &Config, settings: &UserSettings) -> Vec<Decision> {
    let engine = settings.engine(config).unwrap();
    let mut out = Vec::new();
    for mcp in &config.mcp {
        for env in mcp.instances.keys() {
            for tool in mcp.tools.keys() {
                out.push(engine.resolve(env, &mcp.id, tool, None).decision);
            }
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    // Exigence : EX-014-03
    #[test]
    fn the_fallback_is_never_softer_than_the_strictest_profile(
        flips in proptest::collection::vec((any::<proptest::sample::Index>(), 1u8..=255), 1..4),
        cut in proptest::option::of(any::<proptest::sample::Index>()),
        drop_key in any::<bool>(),
    ) {
        let dir = TempDir::new();
        let config = config();
        let saved = permissive(&config);
        store_in(&dir, UserWrapper(1)).save(&saved).unwrap();
        let mut file = std::fs::read(dir.file("settings.json")).unwrap();
        for (index, mask) in &flips {
            let i = index.index(file.len());
            file[i] ^= mask;
        }
        if let Some(cut) = cut {
            file.truncate(cut.index(file.len()));
        }
        std::fs::write(dir.file("settings.json"), &file).unwrap();
        if drop_key {
            std::fs::remove_file(dir.file("settings.key")).unwrap();
        }
        let loaded = store_in(&dir, UserWrapper(1)).load();
        prop_assert!(matches!(loaded.origin, SettingsOrigin::Reset(_)));

        // The `strictest` profile with nothing exposed: no exposed tool, and every decision at least as strict.
        let strictest = UserSettings {
            profile: config.strictest_profile().map(|p| p.id.clone()),
            ..UserSettings::default()
        };
        prop_assert!(loaded.settings.exposed_environments(&config).is_empty());
        prop_assert!(loaded.settings.active_profile(&config).unwrap().strictest);
        prop_assert!(!loaded.settings.autostart && !loaded.settings.beta_channel);
        let strict = decisions(&config, &strictest);
        for (got, floor) in decisions(&config, &loaded.settings).into_iter().zip(strict) {
            prop_assert!(got.severity() >= floor.severity(), "{got:?} < {floor:?}");
        }
        prop_assert!(!loaded.settings.exposed_environments.contains(&EnvId::new("prod").unwrap()));
    }
}

// Run on Windows only: cross-compiled from Linux, never executed there.
#[cfg(windows)]
mod dpapi {
    use super::*;
    use crate::secrets::DpapiKeyWrapper;

    // Exigence : EX-014-04
    #[test]
    fn the_key_is_kept_by_dpapi_and_seals_round_trip() {
        let dir = TempDir::new();
        let sealer = Sealer::open(dir.file("settings.key"), Box::new(DpapiKeyWrapper));
        assert_eq!(sealer.key_status(), Err(SealError::KeyMissing));
        let sealed = sealer.seal(SETTINGS_PURPOSE, b"{\"a\":1}").unwrap();
        let reopened = Sealer::open(dir.file("settings.key"), Box::new(DpapiKeyWrapper));
        assert_eq!(reopened.key_status(), Ok(()));
        assert_eq!(reopened.unseal(SETTINGS_PURPOSE, &sealed).unwrap(), b"{\"a\":1}");
        let key_file = std::fs::read(dir.file("settings.key")).unwrap();
        assert!(
            key_file.len() > KEY_MAGIC.len() + SEAL_KEY_LEN,
            "DPAPI output, not the bare key"
        );
    }

    // Exigence : EX-014-04
    #[test]
    fn a_key_wrapped_for_nobody_does_not_open() {
        let dir = TempDir::new();
        std::fs::write(dir.file("settings.key"), [KEY_MAGIC, &[0x5a; 64]].concat()).unwrap();
        let sealer = Sealer::open(dir.file("settings.key"), Box::new(DpapiKeyWrapper));
        assert!(matches!(sealer.key_status(), Err(SealError::KeyUnreadable(_))));
    }
}
