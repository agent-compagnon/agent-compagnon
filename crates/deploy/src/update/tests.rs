#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use mcp_proxy_testkit::{CapturedLogs, DEV_SIGNING_SEED, FakeArtifactory, Served};
use sha2::{Digest, Sha256};
use url::Url;

use super::slots::{build_guard, hex_key, parse_slot, release_guard};
use super::*;
use crate::fake::{FakeReplacer, FakeSystem, Op, World};
use crate::install::InstallPaths;
use crate::ops::{OpError, SystemOps};
use crate::version::{Installed, Version, mark_prefix};

const REPO: &str = "generic-release-local";
const PATH: &str = "mcp-proxy";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn dev_key() -> SigningKey {
    SigningKey::from_bytes(&DEV_SIGNING_SEED)
}

fn other_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

/// The bytes of an exe of `version`, with its version mark.
fn exe(version: &str) -> Vec<u8> {
    let mut bytes = b"MZ\x90\x00 an exe of MCP Proxy ".to_vec();
    bytes.extend_from_slice(mark_prefix());
    bytes.extend_from_slice(version.as_bytes());
    bytes.push(0x01);
    bytes.extend_from_slice(&[0xAB; 4096]);
    bytes
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn sig(version: &str, exe: &[u8], critical: bool, key: &SigningKey) -> String {
    Manifest::sign(&v(version), digest(exe), critical, key).render()
}

fn ring(keys: &[&SigningKey]) -> KeyRing {
    let bytes: Vec<[u8; 32]> = keys.iter().map(|k| k.verifying_key().to_bytes()).collect();
    KeyRing::new(&bytes).unwrap()
}

/// A folder of the system's temporary directory, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mcp-proxy-update-{}-{name}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn is_empty(&self) -> bool {
        std::fs::read_dir(&self.0)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    art: FakeArtifactory,
    downloads: TempDir,
    replacer: Arc<FakeReplacer>,
    updater: Updater,
}

async fn start(current: &str, keys: KeyRing) -> Fixture {
    let art = FakeArtifactory::start(REPO, PATH).await.unwrap();
    let downloads = TempDir::new("downloads");
    let replacer = Arc::new(FakeReplacer::new(v(current)));
    let updater = Updater::new(UpdaterDeps {
        source: Source {
            base_url: art.base_url(),
            repository: REPO.to_owned(),
            path: PATH.to_owned(),
        },
        fetch: Arc::new(AnonymousClient::new().unwrap()),
        replacer: Arc::clone(&replacer) as Arc<dyn ExeReplacer>,
        keys,
        downloads: downloads.0.clone(),
        current: v(current),
    });
    Fixture {
        art,
        downloads,
        replacer,
        updater,
    }
}

/// A release of `version`, signed with the development key.
fn publish(art: &FakeArtifactory, version: &str, critical: bool) -> Vec<u8> {
    let bytes = exe(version);
    art.publish(
        version,
        bytes.clone(),
        sig(version, &bytes, critical, &dev_key()).into_bytes(),
    );
    bytes
}

// Exigence : EX-020-01
#[test]
fn the_highest_version_of_the_channel_is_chosen() {
    let versions: Vec<Version> = ["1.2.0", "1.3.0", "1.4.0-rc.1", "1.4.0-rc.2", "1.3.1"]
        .iter()
        .map(|t| v(t))
        .collect();
    let floor = v("1.2.0");
    assert_eq!(choose(&versions, Channel::Stable, &floor), Some(v("1.3.1")));
    assert_eq!(choose(&versions, Channel::Beta, &floor), Some(v("1.4.0-rc.2")));
    assert_eq!(
        choose(&versions, Channel::Beta, &v("1.4.0-rc.2")),
        None,
        "the same version is not newer"
    );
    assert_eq!(choose(&versions, Channel::Stable, &v("1.3.1")), None, "nothing newer");
    assert_eq!(choose(&versions, Channel::Stable, &v("2.0.0")), None, "no going back");
    assert_eq!(choose(&[], Channel::Beta, &floor), None);

    // Pre-releases in semver order: numbers compared as numbers, a release after its pre-releases.
    let pre: Vec<Version> = ["1.4.0-rc.9", "1.4.0-rc.10", "1.4.0-beta.3", "1.4.0-alpha"]
        .iter()
        .map(|t| v(t))
        .collect();
    assert_eq!(choose(&pre, Channel::Beta, &floor), Some(v("1.4.0-rc.10")));
    assert_eq!(
        choose(&pre, Channel::Stable, &floor),
        None,
        "stable ignores pre-releases"
    );
    let mut with_release = pre.clone();
    with_release.push(v("1.4.0"));
    assert_eq!(choose(&with_release, Channel::Beta, &floor), Some(v("1.4.0")));
    // A pre-release above the running release is offered on the beta channel only.
    assert_eq!(
        choose(&[v("1.4.1-rc.1")], Channel::Beta, &v("1.4.0")),
        Some(v("1.4.1-rc.1"))
    );
    // A release after a running pre-release is offered on both channels.
    assert_eq!(
        choose(&[v("1.4.0")], Channel::Stable, &v("1.4.0-rc.2")),
        Some(v("1.4.0"))
    );
    assert_eq!(Channel::of(true), Channel::Beta);
    assert_eq!(Channel::of(false), Channel::Stable);
}

// Exigence : EX-020-01
#[test]
fn only_folders_named_by_a_semver_are_versions() {
    let body = serde_json::json!({
        "repo": REPO,
        "path": "/mcp-proxy",
        "children": [
            { "uri": "/1.3.0", "folder": true },
            { "uri": "/1.4.0-rc.1", "folder": true },
            { "uri": "/1.5.0", "folder": false },
            { "uri": "/latest", "folder": true },
            { "uri": "/v1.6.0", "folder": true },
            { "uri": "/1.6", "folder": true },
            { "uri": "/01.6.0", "folder": true },
            { "uri": "/1.6.0/x", "folder": true },
            { "uri": "1.7.0", "folder": true },
            { "uri": "/1.8.0" },
            { "folder": true },
            { "uri": 9, "folder": true },
            { "uri": "/ 1.9.0", "folder": true },
            "1.10.0"
        ]
    });
    let versions = parse_listing(body.to_string().as_bytes()).unwrap();
    assert_eq!(versions, [v("1.3.0"), v("1.4.0-rc.1")]);
    assert_eq!(parse_listing(br#"{"children": []}"#).unwrap(), []);
    assert_eq!(parse_listing(b"<html>"), Err(ListingError));
    assert_eq!(parse_listing(br#"{"files": []}"#), Err(ListingError));
    assert_eq!(parse_listing(br#"{"children": {}}"#), Err(ListingError));
}

// Exigence : EX-020-02
#[test]
fn a_well_formed_manifest_is_read_and_verified() {
    let key = dev_key();
    let text = sig("1.4.0", b"exe", true, &key);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[0], "mcp-proxy-signature v1\n");
    assert_eq!(lines[1], "version: 1.4.0\n");
    assert_eq!(
        lines[2],
        format!("sha256: {}\n", Manifest::parse(text.as_bytes()).unwrap().sha256_hex())
    );
    assert_eq!(lines[3], "critical: true\n");
    assert_eq!(lines[4], "key-id: b2f6a826\n");
    assert!(lines[5].starts_with("signature: ") && lines[5].ends_with("==\n"));

    let manifest = Manifest::parse(text.as_bytes()).unwrap();
    assert_eq!(manifest.version, v("1.4.0"));
    assert_eq!(manifest.sha256, digest(b"exe"));
    assert!(manifest.critical);
    assert_eq!(manifest.key_id.to_string(), "b2f6a826");
    assert_eq!(manifest.render(), text, "rendered back unchanged");
    assert!(verify_signature(&manifest, &ring(&[&key])).is_ok());
    let checked = verify_manifest(text.as_bytes(), &ring(&[&other_key(), &key]), &v("1.4.0"), &v("1.3.0")).unwrap();
    assert_eq!(checked.version, v("1.4.0"));
    // `Debug` names the version and the key, not the signature.
    let debug = format!("{manifest:?}");
    assert!(debug.contains("1.4.0") && !debug.contains("signature: "), "{debug}");
}

// Exigence : EX-020-02
#[test]
fn a_manifest_signed_by_another_implementation_is_accepted() {
    // Written with Python's `cryptography` (Ed25519, same development seed), not by this crate.
    let independent = "mcp-proxy-signature v1\nversion: 1.4.0\nsha256: \
                       d989fca9b4d2f21b21f5ccf0e492e4def6d50c194733081f07cef579e5eb20aa\ncritical: true\nkey-id: \
                       b2f6a826\nsignature: \
                       suS4xKzcxWo3sNC5Yf/peJ+Zbu+9ilZHpdxennrJ57mLWHtL92H3xxq7grmRRy80UL4HLxP69JROOQ8yhLa7Bg==\n";
    let manifest = verify_manifest(independent.as_bytes(), &ring(&[&dev_key()]), &v("1.4.0"), &v("1.3.0")).unwrap();
    assert_eq!(manifest.sha256, digest(b"independent vector"));
    assert_eq!(manifest.render(), independent);
    assert_eq!(
        Manifest::sign(&v("1.4.0"), digest(b"independent vector"), true, &dev_key()).render(),
        independent
    );
}

// Exigence : EX-020-02
#[test]
fn every_altered_field_breaks_the_signature() {
    let key = dev_key();
    let keys = ring(&[&key, &other_key()]);
    let text = sig("1.4.0", b"exe", false, &key);
    let other_digest: String = digest(b"another exe").iter().map(|b| format!("{b:02x}")).collect();
    let other_id = KeyId::of(other_key().verifying_key().as_bytes()).to_string();
    let original_digest = Manifest::parse(text.as_bytes()).unwrap().sha256_hex();
    for (from, to) in [
        ("version: 1.4.0", "version: 1.4.1"),
        (original_digest.as_str(), other_digest.as_str()),
        ("critical: false", "critical: true"),
        ("key-id: b2f6a826", &format!("key-id: {other_id}")),
    ] {
        let altered = text.replace(from, to);
        assert_ne!(altered, text);
        let manifest = Manifest::parse(altered.as_bytes()).unwrap();
        assert_eq!(
            verify_signature(&manifest, &keys),
            Err(UpdateError::Signature),
            "{from} -> {to}"
        );
    }
    // One bit of the signature.
    let manifest = Manifest::parse(text.as_bytes()).unwrap();
    let encoded = text
        .lines()
        .last()
        .unwrap()
        .strip_prefix("signature: ")
        .unwrap()
        .to_owned();
    let flipped = if encoded.starts_with('A') { 'B' } else { 'A' };
    let altered = text.replace(&encoded, &format!("{flipped}{}", &encoded[1..]));
    let altered = Manifest::parse(altered.as_bytes()).unwrap();
    assert_eq!(verify_signature(&altered, &keys), Err(UpdateError::Signature));
    assert!(verify_signature(&manifest, &keys).is_ok());
}

// Exigence : EX-020-02
#[test]
fn an_unknown_key_is_refused() {
    let text = sig("1.4.0", b"exe", false, &other_key());
    let manifest = Manifest::parse(text.as_bytes()).unwrap();
    assert_eq!(
        verify_signature(&manifest, &ring(&[&dev_key()])),
        Err(UpdateError::UnknownKey)
    );
    assert_eq!(
        verify_signature(&manifest, &KeyRing::default()),
        Err(UpdateError::UnknownKey)
    );
    assert_eq!(
        verify_manifest(text.as_bytes(), &ring(&[&dev_key()]), &v("1.4.0"), &v("1.0.0")).unwrap_err(),
        UpdateError::UnknownKey
    );
}

// Exigence : EX-020-02
#[test]
fn only_the_exact_format_is_read() {
    let text = sig("1.4.0", b"exe", false, &dev_key());
    let parse = |t: &str| Manifest::parse(t.as_bytes()).map(|_| ());
    assert_eq!(parse(&text), Ok(()));
    assert_eq!(parse(&text.replace('\n', "\r\n")), Err(ManifestError::CarriageReturn));
    assert_eq!(
        parse(text.trim_end_matches('\n')),
        Err(ManifestError::Missing("signature"))
    );
    assert_eq!(parse(&format!("{text}\n")), Err(ManifestError::Trailing));
    assert_eq!(parse(&format!("{text}extra: 1\n")), Err(ManifestError::Trailing));
    assert_eq!(parse(""), Err(ManifestError::Missing("mcp-proxy-signature")));
    assert_eq!(
        parse(&format!("\u{feff}{text}")),
        Err(ManifestError::Invalid("mcp-proxy-signature"))
    );
    assert_eq!(
        parse(&text.replace(" v1", " v2")),
        Err(ManifestError::Invalid("mcp-proxy-signature"))
    );
    assert_eq!(Manifest::parse(&[0xff, 0xfe]).map(|_| ()), Err(ManifestError::NotUtf8));
    assert_eq!(parse(&"x".repeat(MANIFEST_MAX + 1)), Err(ManifestError::TooLarge));

    // Each line missing.
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let names = [
        "mcp-proxy-signature",
        "version",
        "sha256",
        "critical",
        "key-id",
        "signature",
    ];
    for (i, name) in names.iter().enumerate() {
        let without: String = lines
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, l)| *l)
            .collect();
        assert!(parse(&without).is_err(), "without {name}");
    }
    // Lines in another order.
    let mut swapped = lines.clone();
    swapped.swap(1, 2);
    assert_eq!(parse(&swapped.concat()), Err(ManifestError::Invalid("version")));

    // One spelling per field.
    for (from, to, line) in [
        ("version: 1.4.0", "version:1.4.0", "version"),
        ("version: 1.4.0", "version:  1.4.0", "version"),
        ("version: 1.4.0", "Version: 1.4.0", "version"),
        ("version: 1.4.0", "version: v1.4.0", "version"),
        ("version: 1.4.0", "version: 1.4", "version"),
        ("version: 1.4.0", "version: 1.4.0 ", "version"),
        ("critical: false", "critical: False", "critical"),
        ("critical: false", "critical: 0", "critical"),
        ("key-id: b2f6a826", "key-id: B2F6A826", "key-id"),
        ("key-id: b2f6a826", "key-id: b2f6a82", "key-id"),
    ] {
        assert_eq!(
            parse(&text.replace(from, to)),
            Err(ManifestError::Invalid(line)),
            "{to}"
        );
    }
    let sha = Manifest::parse(text.as_bytes()).unwrap().sha256_hex();
    assert_eq!(
        parse(&text.replace(&sha, &sha.to_uppercase())),
        Err(ManifestError::Invalid("sha256"))
    );
    assert_eq!(
        parse(&text.replace(&sha, &sha[1..])),
        Err(ManifestError::Invalid("sha256"))
    );
    let encoded = text
        .lines()
        .last()
        .unwrap()
        .strip_prefix("signature: ")
        .unwrap()
        .to_owned();
    assert_eq!(
        parse(&text.replace(&encoded, encoded.trim_end_matches('='))),
        Err(ManifestError::Invalid("signature")),
        "padding required"
    );
    assert_eq!(
        parse(&text.replace(&encoded, &encoded[4..])),
        Err(ManifestError::Invalid("signature")),
        "64 bytes"
    );
    // The last character before the padding carries unused bits: another spelling of the same bytes.
    let mut chars: Vec<char> = encoded.chars().collect();
    let last = chars.len() - 3;
    chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
    let respelled: String = chars.into_iter().collect();
    if respelled != encoded {
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &respelled).ok();
        let original = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded).ok();
        if decoded == original {
            assert_eq!(
                parse(&text.replace(&encoded, &respelled)),
                Err(ManifestError::Invalid("signature"))
            );
        }
    }
}

// Exigence : EX-020-03
#[test]
fn the_version_must_be_the_folder_and_strictly_newer() {
    let keys = ring(&[&dev_key()]);
    let text = sig("1.4.0", b"exe", false, &dev_key());
    assert!(verify_manifest(text.as_bytes(), &keys, &v("1.4.0"), &v("1.3.9")).is_ok());
    assert_eq!(
        verify_manifest(text.as_bytes(), &keys, &v("1.5.0"), &v("1.3.0")).unwrap_err(),
        UpdateError::VersionMismatch {
            folder: v("1.5.0"),
            manifest: v("1.4.0")
        }
    );
    assert_eq!(
        verify_manifest(text.as_bytes(), &keys, &v("1.4.0"), &v("1.4.0")).unwrap_err(),
        UpdateError::NotNewer {
            version: v("1.4.0"),
            floor: v("1.4.0")
        },
        "same version"
    );
    assert_eq!(
        verify_manifest(text.as_bytes(), &keys, &v("1.4.0"), &v("1.5.0")).unwrap_err(),
        UpdateError::NotNewer {
            version: v("1.4.0"),
            floor: v("1.5.0")
        },
        "no going back"
    );
    // The digest and the mark of the downloaded exe.
    let bytes = exe("1.4.0");
    let manifest = Manifest::sign(&v("1.4.0"), digest(&bytes), false, &dev_key());
    assert_eq!(verify_exe(&manifest, &bytes), Ok(digest(&bytes)));
    assert_eq!(
        verify_exe(&manifest, &bytes[..bytes.len() - 1]),
        Err(UpdateError::Digest),
        "truncated"
    );
    let old = exe("1.3.0");
    let signed_old = Manifest::sign(&v("1.4.0"), digest(&old), false, &dev_key());
    assert_eq!(
        verify_exe(&signed_old, &old),
        Err(UpdateError::Mark),
        "an older exe signed as 1.4.0"
    );
}

/// What every failure must leave: no call of the replacer, nothing downloaded left.
fn assert_abandoned(fixture: &Fixture, case: &str) {
    assert!(fixture.replacer.calls().is_empty(), "{case}: the replacer was called");
    assert!(fixture.downloads.is_empty(), "{case}: downloaded files left behind");
}

// Exigence : EX-020-03
// Exigence : EX-020-04
#[tokio::test]
async fn no_failure_ever_reaches_the_replacer() {
    let key = dev_key();
    let good = exe("1.4.0");
    let good_sig = sig("1.4.0", &good, false, &key);
    type Setup = Box<dyn Fn(&FakeArtifactory)>;
    type Expected = fn(&UpdateError) -> bool;
    let cases: Vec<(&str, Setup, Expected)> = vec![
        (
            "signature invalide",
            Box::new({
                let good = good.clone();
                let forged = sig("1.4.0", &good, false, &other_key()).replace(
                    &format!("key-id: {}", KeyId::of(other_key().verifying_key().as_bytes())),
                    "key-id: b2f6a826",
                );
                move |art| art.publish("1.4.0", good.clone(), forged.clone().into_bytes())
            }),
            |e| *e == UpdateError::Signature,
        ),
        (
            "champ modifié après signature",
            Box::new({
                let good = good.clone();
                let altered = good_sig.replace("critical: false", "critical: true");
                move |art| art.publish("1.4.0", good.clone(), altered.clone().into_bytes())
            }),
            |e| *e == UpdateError::Signature,
        ),
        (
            "clé inconnue",
            Box::new({
                let good = good.clone();
                let other = sig("1.4.0", &good, false, &other_key());
                move |art| art.publish("1.4.0", good.clone(), other.clone().into_bytes())
            }),
            |e| *e == UpdateError::UnknownKey,
        ),
        (
            "manifeste CRLF",
            Box::new({
                let good = good.clone();
                let crlf = good_sig.replace('\n', "\r\n");
                move |art| art.publish("1.4.0", good.clone(), crlf.clone().into_bytes())
            }),
            |e| *e == UpdateError::Manifest(ManifestError::CarriageReturn),
        ),
        (
            "ancienne version republiée sous un numéro plus élevé",
            Box::new({
                let good = good.clone();
                let good_sig = good_sig.clone();
                move |art| art.publish("1.5.0", good.clone(), good_sig.clone().into_bytes())
            }),
            |e| matches!(e, UpdateError::VersionMismatch { .. }),
        ),
        (
            "exe remplacé",
            Box::new({
                let good_sig = good_sig.clone();
                move |art| {
                    art.publish(
                        "1.4.0",
                        exe("1.4.0").into_iter().rev().collect(),
                        good_sig.clone().into_bytes(),
                    )
                }
            }),
            |e| *e == UpdateError::Digest,
        ),
        (
            "exe tronqué, servi entier",
            Box::new({
                let good = good.clone();
                let good_sig = good_sig.clone();
                move |art| art.publish("1.4.0", good[..good.len() / 2].to_vec(), good_sig.clone().into_bytes())
            }),
            |e| *e == UpdateError::Digest,
        ),
        (
            "téléchargement coupé",
            Box::new({
                let good = good.clone();
                let good_sig = good_sig.clone();
                move |art| {
                    art.publish("1.4.0", good.clone(), good_sig.clone().into_bytes());
                    art.serve(
                        "1.4.0",
                        "mcp-proxy.exe",
                        Served::Truncated {
                            bytes: good.clone(),
                            sent: 100,
                        },
                    );
                }
            }),
            |e| {
                matches!(
                    e,
                    UpdateError::Download {
                        cause: FetchError::Transport,
                        ..
                    }
                )
            },
        ),
        (
            "marque de version différente",
            Box::new({
                let old = exe("1.3.5");
                let signed = sig("1.4.0", &old, false, &dev_key());
                move |art| art.publish("1.4.0", old.clone(), signed.clone().into_bytes())
            }),
            |e| *e == UpdateError::Mark,
        ),
        (
            "redirection vers une autre origine",
            Box::new({
                let good = good.clone();
                let good_sig = good_sig.clone();
                move |art| {
                    art.publish("1.4.0", good.clone(), good_sig.clone().into_bytes());
                    let elsewhere = art.other_origin(&art.file_path("1.4.0", "mcp-proxy.exe"));
                    art.serve("1.4.0", "mcp-proxy.exe", Served::Redirect(elsewhere));
                }
            }),
            |e| {
                matches!(
                    e,
                    UpdateError::Download {
                        cause: FetchError::Redirected,
                        ..
                    }
                )
            },
        ),
        (
            "signature absente",
            Box::new({
                let good = good.clone();
                move |art| {
                    art.publish("1.4.0", good.clone(), Vec::new());
                    art.serve("1.4.0", "mcp-proxy.exe.sig", Served::Status(404));
                }
            }),
            |e| {
                matches!(
                    e,
                    UpdateError::Download {
                        cause: FetchError::Status(404),
                        ..
                    }
                )
            },
        ),
        (
            "exe absent",
            Box::new({
                let good_sig = good_sig.clone();
                move |art| {
                    art.publish("1.4.0", Vec::new(), good_sig.clone().into_bytes());
                    art.serve("1.4.0", "mcp-proxy.exe", Served::Status(404));
                }
            }),
            |e| {
                matches!(
                    e,
                    UpdateError::Download {
                        cause: FetchError::Status(404),
                        ..
                    }
                )
            },
        ),
        (
            "liste en erreur",
            Box::new(|art| art.serve_listing(Served::Status(500))),
            |e| *e == UpdateError::Listing(FetchError::Status(500)),
        ),
        (
            "liste illisible",
            Box::new(|art| art.serve_listing(Served::Bytes(b"<html>maintenance</html>".to_vec()))),
            |e| *e == UpdateError::ListingFormat(ListingError),
        ),
    ];
    for (case, setup, expected) in cases {
        let fixture = start("1.3.0", ring(&[&key])).await;
        setup(&fixture.art);
        let (logs, _guard) = CapturedLogs::start();
        let error = fixture.updater.check(Channel::Stable, None).await.unwrap_err();
        assert!(expected(&error), "{case}: {error:?}");
        assert_abandoned(&fixture, case);
        // Neither the address, nor a path, nor anything of the answers in the text of the failure or the log.
        let ours: Vec<String> = logs
            .text()
            .lines()
            .filter(|l| l.contains(" mcp_proxy_deploy"))
            .map(str::to_owned)
            .collect();
        let text = format!("{error} {}", ours.join("\n"));
        for needle in [
            "127.0.0.1",
            "localhost",
            "/artifactory",
            fixture.downloads.0.to_str().unwrap(),
            "maintenance",
        ] {
            assert!(!text.contains(needle), "{case}: « {needle} » in {text}");
        }
        if matches!(error, UpdateError::Signature | UpdateError::Digest | UpdateError::Mark) {
            assert_eq!(fixture.art.hits("1.4.0", "mcp-proxy.exe.sig"), 1, "{case}");
        }
    }
}

// Exigence : EX-020-03
// Exigence : EX-020-04
#[tokio::test]
async fn a_forged_manifest_is_refused_before_the_exe_is_downloaded() {
    let fixture = start("1.3.0", ring(&[&dev_key()])).await;
    let bytes = exe("1.4.0");
    fixture.art.publish(
        "1.4.0",
        bytes.clone(),
        sig("1.4.0", &bytes, false, &other_key()).into_bytes(),
    );
    assert_eq!(
        fixture.updater.check(Channel::Stable, None).await,
        Err(UpdateError::UnknownKey)
    );
    assert_eq!(fixture.art.hits("1.4.0", "mcp-proxy.exe"), 0);
    assert_abandoned(&fixture, "clé inconnue");
}

// Exigence : EX-020-03
// Exigence : EX-020-04
#[tokio::test]
async fn the_same_or_an_older_version_is_never_downloaded() {
    // Same version.
    let fixture = start("1.4.0", ring(&[&dev_key()])).await;
    publish(&fixture.art, "1.4.0", false);
    publish(&fixture.art, "1.3.0", true);
    assert_eq!(
        fixture.updater.check(Channel::Beta, None).await,
        Ok(CheckOutcome::UpToDate)
    );
    assert_eq!(fixture.art.hits("1.4.0", "mcp-proxy.exe"), 0);
    assert_eq!(fixture.art.hits("1.3.0", "mcp-proxy.exe.sig"), 0);
    assert_abandoned(&fixture, "version identique");

    // The installed exe is already more recent than the running one (an update put in place, not yet started).
    let fixture = fixture_with_installed("1.3.0", "1.5.0").await;
    publish(&fixture.art, "1.4.0", false);
    assert_eq!(
        fixture.updater.check(Channel::Stable, None).await,
        Ok(CheckOutcome::UpToDate)
    );
    assert_abandoned(&fixture, "version inférieure à l'installée");

    // Nor below an update already put in place by this run.
    let fixture = start("1.3.0", ring(&[&dev_key()])).await;
    publish(&fixture.art, "1.4.0", false);
    assert_eq!(
        fixture.updater.check(Channel::Stable, Some(&v("1.4.0"))).await,
        Ok(CheckOutcome::UpToDate)
    );
    assert_abandoned(&fixture, "déjà en place");
}

async fn fixture_with_installed(current: &str, installed: &str) -> Fixture {
    let fixture = start(current, ring(&[&dev_key()])).await;
    fixture.replacer.set_installed(Installed::Known(v(installed)));
    fixture
}

// Exigence : EX-020-04
#[tokio::test]
async fn without_a_key_or_a_readable_installation_nothing_is_attempted() {
    let fixture = start("1.3.0", KeyRing::default()).await;
    publish(&fixture.art, "1.4.0", false);
    assert_eq!(
        fixture.updater.check(Channel::Stable, None).await,
        Err(UpdateError::NoKey)
    );
    assert!(fixture.art.requests().is_empty(), "nothing is asked of Artifactory");
    assert_abandoned(&fixture, "aucune clé");

    let fixture = fixture_with_installed("1.3.0", "1.3.0").await;
    fixture.replacer.set_installed(Installed::Unknown);
    publish(&fixture.art, "1.4.0", false);
    assert_eq!(
        fixture.updater.check(Channel::Stable, None).await,
        Err(UpdateError::InstalledUnknown)
    );
    assert_abandoned(&fixture, "version installée illisible");
}

// Exigence : EX-020-07
#[tokio::test]
async fn end_to_end_from_the_repository_to_the_replacer() {
    let fixture = start("1.3.0", ring(&[&other_key(), &dev_key()])).await;
    publish(&fixture.art, "1.2.0", false);
    let stable = publish(&fixture.art, "1.4.0", true);
    let beta = publish(&fixture.art, "1.5.0-rc.1", false);
    fixture.art.add_child("README.md", false);

    let outcome = fixture.updater.check(Channel::Stable, None).await.unwrap();
    assert_eq!(
        outcome,
        CheckOutcome::Ready(UpdateReady {
            version: v("1.4.0"),
            critical: true
        })
    );
    assert_eq!(
        fixture.replacer.calls(),
        [(v("1.4.0"), stable)],
        "the verified bytes, once"
    );
    assert!(
        fixture.downloads.is_empty(),
        "the downloads are removed once the exe is in place"
    );

    // The next check knows what is in place: nothing more on the stable channel, the pre-release on the beta one.
    assert_eq!(
        fixture.updater.check(Channel::Stable, Some(&v("1.4.0"))).await,
        Ok(CheckOutcome::UpToDate)
    );
    let outcome = fixture.updater.check(Channel::Beta, Some(&v("1.4.0"))).await.unwrap();
    assert_eq!(
        outcome,
        CheckOutcome::Ready(UpdateReady {
            version: v("1.5.0-rc.1"),
            critical: false
        })
    );
    assert_eq!(fixture.replacer.calls()[1], (v("1.5.0-rc.1"), beta));

    // Anonymous: no credential, no cookie, no referer in any request.
    let requests = fixture.art.requests();
    assert!(requests.len() >= 5);
    for request in &requests {
        for (name, _) in &request.headers {
            assert!(
                !["authorization", "cookie", "referer", "proxy-authorization"].contains(&name.as_str()),
                "{request:?}"
            );
        }
    }
    assert_eq!(requests[0].path, format!("/artifactory/api/storage/{REPO}/{PATH}"));
}

// Exigence : EX-020-04
#[tokio::test]
async fn a_failed_replacement_leaves_no_download() {
    let fixture = start("1.3.0", ring(&[&dev_key()])).await;
    publish(&fixture.art, "1.4.0", false);
    fixture.replacer.fail_with(ReplaceError {
        step: ReplaceStep::SetAside,
        cause: None,
        restored: true,
    });
    let error = fixture.updater.check(Channel::Stable, None).await.unwrap_err();
    assert!(matches!(error, UpdateError::Replace(_)), "{error:?}");
    assert_eq!(fixture.replacer.calls().len(), 1, "called once every check passed");
    assert!(fixture.downloads.is_empty());
}

// Exigence : EX-020-12
#[tokio::test]
async fn downloads_are_anonymous_bounded_and_stay_on_their_origin() {
    let art = FakeArtifactory::start(REPO, PATH).await.unwrap();
    let client = AnonymousClient::new().unwrap();
    let limits = Limits {
        max_bytes: 1000,
        timeout: Duration::from_secs(30),
    };
    let url =
        |version: &str, name: &str| Url::parse(&format!("{}{}", art.origin(), art.file_path(version, name))).unwrap();

    art.serve("1.0.0", "a", Served::Bytes(b"contenu".to_vec()));
    assert_eq!(client.get(&url("1.0.0", "a"), limits).await.unwrap(), b"contenu");

    // A redirect on the same origin is followed.
    art.serve("1.0.0", "moved", Served::Redirect(art.file_path("1.0.0", "a")));
    assert_eq!(client.get(&url("1.0.0", "moved"), limits).await.unwrap(), b"contenu");

    // Towards another origin, it is not, and nothing is asked there.
    let before = art.requests().len();
    art.serve(
        "1.0.0",
        "away",
        Served::Redirect(art.other_origin(&art.file_path("1.0.0", "a"))),
    );
    assert_eq!(
        client.get(&url("1.0.0", "away"), limits).await,
        Err(FetchError::Redirected)
    );
    assert_eq!(art.requests().len(), before + 1);

    // An endless chain of redirects stops.
    art.serve("1.0.0", "loop", Served::Redirect(art.file_path("1.0.0", "loop")));
    assert_eq!(
        client.get(&url("1.0.0", "loop"), limits).await,
        Err(FetchError::Redirected)
    );
    assert!(art.hits("1.0.0", "loop") <= MAX_REDIRECTS + 1);

    // Bounded: announced or streamed.
    art.serve("1.0.0", "big", Served::Bytes(vec![0; 1001]));
    assert_eq!(
        client.get(&url("1.0.0", "big"), limits).await,
        Err(FetchError::TooLarge)
    );
    art.serve(
        "1.0.0",
        "cut",
        Served::Truncated {
            bytes: vec![1; 900],
            sent: 10,
        },
    );
    assert_eq!(
        client.get(&url("1.0.0", "cut"), limits).await,
        Err(FetchError::Transport)
    );
    assert_eq!(
        client.get(&url("1.0.0", "absent"), limits).await,
        Err(FetchError::Status(404))
    );

    // Plain HTTP only towards the loopback; nothing else than HTTP(S).
    for refused in [
        "http://artifactory.example.internal/x",
        "ftp://127.0.0.1/x",
        "file:///etc/passwd",
    ] {
        assert_eq!(
            client.get(&Url::parse(refused).unwrap(), limits).await,
            Err(FetchError::Insecure),
            "{refused}"
        );
    }
    assert!(allowed(&Url::parse("https://artifactory.example.internal/x").unwrap()));

    for request in art.requests() {
        assert!(
            request
                .headers
                .iter()
                .all(|(n, _)| !["authorization", "cookie", "referer"].contains(&n.as_str())),
            "{request:?}"
        );
    }
}

/// y = 2: (y² − 1) / (d·y² + 1) has no square root modulo 2²⁵⁵ − 19.
fn off_curve() -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[0] = 2;
    bytes
}

// Exigence : EX-020-05
#[test]
fn the_development_key_is_never_embedded() {
    let dev_public = dev_key().verifying_key().to_bytes();
    // `keys/dev.pub` is the public half of the seed of the testkit, and the exe only knows its fingerprint.
    assert_eq!(parse_slot(include_str!("../../keys/dev.pub")), Ok(Some(dev_public)));
    assert_eq!(fingerprint(&dev_public), DEV_KEY_FINGERPRINT);
    assert!(is_dev_key(&dev_public));
    assert!(!is_dev_key(&other_key().verifying_key().to_bytes()));

    // Every build refuses it in a slot; a release also wants a current key.
    let other = other_key().verifying_key().to_bytes();
    assert!(build_guard(&[None, None], &dev_public).is_ok());
    assert!(build_guard(&[Some(other), None], &dev_public).is_ok());
    assert!(build_guard(&[Some(dev_public), None], &dev_public).is_err());
    assert!(build_guard(&[Some(other), Some(dev_public)], &dev_public).is_err());
    assert!(
        release_guard(&[None, Some(other)], &dev_public).is_err(),
        "no current key"
    );
    assert!(release_guard(&[Some(other), None], &dev_public).is_ok());
    assert!(release_guard(&[Some(other), Some(dev_public)], &dev_public).is_err());

    // Should a slot hold it all the same, the exe leaves it out.
    let (logs, _guard) = CapturedLogs::start();
    let keys = KeyRing::from_slots(&[Some(dev_public), Some(other)]);
    assert_eq!(keys.ids(), [KeyId::of(&other)]);
    assert!(logs.text().contains("clé de développement refusée"), "{}", logs.text());
    assert!(KeyRing::from_slots(&[Some(dev_public), None]).is_empty());
    assert!(
        KeyRing::from_slots(&[Some(off_curve()), None]).is_empty(),
        "not a point of the curve"
    );

    // This build: whatever the slots hold, the development key is not among them.
    assert!(embedded_slots().iter().flatten().all(|k| !is_dev_key(k)));
    assert!(KeyRing::embedded().ids().iter().all(|id| *id != KeyId::of(&dev_public)));

    // `cargo xtask check-release`, on the slots and on the bytes of a built exe.
    let mut built = b"MZ code ".to_vec();
    built.extend_from_slice(&other);
    built.extend_from_slice(b" more code");
    assert!(check_release(&[Some(other), None], &dev_public, Some(&built)).is_ok());
    assert!(check_release(&[Some(other), None], &dev_public, None).is_ok());
    let mut with_dev = built.clone();
    with_dev.extend_from_slice(&dev_public);
    assert!(check_release(&[Some(other), None], &dev_public, Some(&with_dev)).is_err());
    assert!(
        check_release(&[Some(other), None], &dev_public, Some(b"MZ without keys")).is_err(),
        "built without the key"
    );
    assert!(check_release(&[None, None], &dev_public, Some(&built)).is_err());
}

// Exigence : EX-020-05
#[test]
fn slot_files_are_read_strictly() {
    let hex = "8f4d6578a8e95e6b17957e2a71f6147db8bc74d04ea7b75199db7578d4c5d57a";
    let key = hex_key(hex).unwrap();
    assert_eq!(parse_slot(""), Ok(None));
    assert_eq!(parse_slot("# commentaire\n\n   \n"), Ok(None));
    assert_eq!(parse_slot(&format!("# clé\n{hex}\n")), Ok(Some(key)));
    assert_eq!(parse_slot(&format!("  {hex}  ")), Ok(Some(key)));
    assert!(parse_slot(&format!("{hex}\n{hex}\n")).is_err(), "two keys");
    assert!(parse_slot(&hex.to_uppercase()).is_err());
    assert!(parse_slot(&hex[1..]).is_err());
    assert!(parse_slot(&format!("{hex}0")).is_err());
    assert!(parse_slot("zz4d6578a8e95e6b17957e2a71f6147db8bc74d04ea7b75199db7578d4c5d57a").is_err());
    // The files of the repository are well formed.
    for text in [
        include_str!("../../keys/current.pub"),
        include_str!("../../keys/next.pub"),
    ] {
        assert!(parse_slot(text).is_ok());
    }
}

const LOCAL: &str = "/home/ana/AppData/Local";
const ROAMING: &str = "/home/ana/AppData/Roaming";

fn install_paths() -> InstallPaths {
    InstallPaths::new(Path::new(LOCAL), Path::new(ROAMING))
}

fn update_paths() -> UpdatePaths {
    UpdatePaths::new(install_paths(), &Path::new(LOCAL).join("MCP Proxy"))
}

/// 1.3.0 installed; 1.4.0 downloaded and verified in `update\1.4.0\`.
fn replaceable() -> (World, VerifiedExe) {
    let paths = update_paths();
    let mut world = World::default();
    for dir in paths.install.dir.ancestors() {
        world.dirs.insert(dir.to_path_buf());
    }
    let folder = paths.downloads.join("1.4.0");
    for dir in folder.ancestors() {
        world.dirs.insert(dir.to_path_buf());
    }
    world.files.insert(paths.install.exe.clone(), exe("1.3.0"));
    let path = folder.join("mcp-proxy.exe");
    world.files.insert(path.clone(), exe("1.4.0"));
    let verified = VerifiedExe {
        path,
        version: v("1.4.0"),
        sha256: digest(&exe("1.4.0")),
    };
    (world, verified)
}

// Exigence : EX-020-13
// Exigence : EX-022-02
#[test]
fn the_installed_exe_is_renamed_aside_then_replaced() {
    let paths = install_paths();
    let (world, verified) = replaceable();
    let ops = Arc::new(FakeSystem::new(world));
    let replacer = RenameReplacer::new(Arc::clone(&ops) as Arc<dyn SystemOps>, paths.clone());
    assert_eq!(replacer.installed(), Installed::Known(v("1.3.0")));
    replacer.replace(&verified).unwrap();
    assert_eq!(
        ops.log(),
        [
            Op::RemoveFile(paths.staged.clone()),
            Op::Copy {
                from: verified.path.clone(),
                to: paths.staged.clone()
            },
            Op::RemoveFile(paths.backup.clone()),
            Op::Rename {
                from: paths.exe.clone(),
                to: paths.backup.clone()
            },
            Op::Rename {
                from: paths.staged.clone(),
                to: paths.exe.clone()
            },
            Op::RemoveFile(paths.backup.clone()),
            Op::SetUninstallEntry(
                crate::uninstall::entry_for(&paths, Some(&v("1.4.0")), exe("1.4.0").len() as u64).unwrap()
            ),
        ]
    );
    let world = ops.world();
    assert_eq!(world.files.get(&paths.exe), Some(&exe("1.4.0")));
    assert!(!world.files.contains_key(&paths.staged));
    assert_eq!(replacer.installed(), Installed::Known(v("1.4.0")));
}

// Exigence : EX-020-13
#[test]
fn a_replacement_that_fails_keeps_the_installed_exe() {
    let paths = install_paths();
    let steps = 6;
    for failing in 0..steps {
        let (world, verified) = replaceable();
        let ops = Arc::new(FakeSystem::new(world.clone()));
        ops.fail_at(failing);
        let replacer = RenameReplacer::new(Arc::clone(&ops) as Arc<dyn SystemOps>, paths.clone());
        let result = replacer.replace(&verified);
        let after = ops.world();
        if failing == steps - 1 {
            // Removing `.old` fails while a process runs it: the replacement holds.
            assert!(result.is_ok());
            assert_eq!(after.files.get(&paths.exe), Some(&exe("1.4.0")));
            assert_eq!(after.files.get(&paths.backup), Some(&exe("1.3.0")));
            continue;
        }
        let error = result.unwrap_err();
        assert!(error.restored, "step {failing}: {error}");
        assert_eq!(after.files.get(&paths.exe), Some(&exe("1.3.0")), "step {failing}");
        assert!(!after.files.contains_key(&paths.staged), "step {failing}: copy left");
        let expected = match failing {
            0 | 1 => ReplaceStep::Copy,
            2 => ReplaceStep::ClearBackup,
            3 => ReplaceStep::SetAside,
            _ => ReplaceStep::PutInPlace,
        };
        assert_eq!(error.step, expected, "step {failing}");
    }

    // The previous exe cannot be put back either: said so.
    let (world, verified) = replaceable();
    let ops = Arc::new(FakeSystem::new(world));
    ops.fail_at(4);
    ops.fail_at(5);
    let error = RenameReplacer::new(Arc::clone(&ops) as Arc<dyn SystemOps>, paths.clone())
        .replace(&verified)
        .unwrap_err();
    assert!(!error.restored);
    assert!(error.to_string().contains("mcp-proxy.exe.old"), "{error}");

    // A copy that is not what was verified is never put in place.
    let (world, mut verified) = replaceable();
    verified.sha256 = digest(b"something else");
    let ops = Arc::new(FakeSystem::new(world));
    let error = RenameReplacer::new(Arc::clone(&ops) as Arc<dyn SystemOps>, paths.clone())
        .replace(&verified)
        .unwrap_err();
    assert_eq!(error.step, ReplaceStep::CopyAltered);
    assert_eq!(ops.world().files.get(&paths.exe), Some(&exe("1.3.0")));
    assert!(!ops.world().files.contains_key(&paths.staged));
    assert!(!ops.log().iter().any(|op| matches!(op, Op::Rename { .. })));
}

/// Files on disk, through the default bodies of the trait.
struct RealFiles;

impl SystemOps for RealFiles {
    fn write_shortcut(&self, _: &crate::ops::Shortcut) -> Result<(), OpError> {
        Err(OpError::Refused("hors sujet"))
    }
    fn run_entry(&self) -> Result<Option<String>, OpError> {
        Ok(None)
    }
    fn set_run_entry(&self, _: &str) -> Result<(), OpError> {
        Ok(())
    }
    fn remove_run_entry(&self) -> Result<(), OpError> {
        Ok(())
    }
    fn uninstall_entry(&self) -> Result<Option<crate::uninstall::UninstallEntry>, OpError> {
        Ok(None)
    }
    fn set_uninstall_entry(&self, _: &crate::uninstall::UninstallEntry) -> Result<(), OpError> {
        Ok(())
    }
    fn remove_uninstall_entry(&self) -> Result<crate::ops::Removal, OpError> {
        Ok(crate::ops::Removal::Absent)
    }
    fn remove_aumid(&self) -> Result<crate::ops::Removal, OpError> {
        Ok(crate::ops::Removal::Absent)
    }
    fn aumid_registered(&self) -> Result<bool, OpError> {
        Ok(false)
    }
    fn wer_exclusion(&self) -> Result<Option<u32>, OpError> {
        Ok(None)
    }
    fn remove_wer_exclusion(&self) -> Result<crate::ops::Removal, OpError> {
        Ok(crate::ops::Removal::Absent)
    }
    fn delete_running_exe(&self, _: &Path) -> Result<(), OpError> {
        Err(OpError::Refused("hors sujet"))
    }
    fn stop_instance(&self) -> Result<crate::ops::Stopped, OpError> {
        Ok(crate::ops::Stopped::NotRunning)
    }
    fn launch(&self, _: &Path, _: &[&str]) -> Result<(), OpError> {
        Ok(())
    }
}

// Exigence : EX-020-13
#[test]
fn the_replacement_works_on_real_files() {
    let root = TempDir::new("real");
    let paths = InstallPaths::new(&root.0.join("Local"), &root.0.join("Roaming"));
    std::fs::create_dir_all(&paths.dir).unwrap();
    std::fs::write(&paths.exe, exe("1.3.0")).unwrap();
    std::fs::write(&paths.backup, b"an older leftover").unwrap();
    let source = root.0.join("downloaded.exe");
    std::fs::write(&source, exe("1.4.0")).unwrap();
    let verified = VerifiedExe {
        path: source,
        version: v("1.4.0"),
        sha256: digest(&exe("1.4.0")),
    };
    RenameReplacer::new(Arc::new(RealFiles), paths.clone())
        .replace(&verified)
        .unwrap();
    assert_eq!(std::fs::read(&paths.exe).unwrap(), exe("1.4.0"));
    assert!(!paths.staged.exists());
    assert!(!paths.backup.exists(), "removable here: no process runs it");
}

// Exigence : EX-020-14
#[test]
fn leftovers_are_cleaned_at_start() {
    let paths = update_paths();
    let (mut world, _) = replaceable();
    let dir = &paths.install.dir;
    for name in [
        "mcp-proxy.exe.old",
        "mcp-proxy.exe.new",
        "MCP-PROXY.EXE.OLD",
        "mcp-proxy.exe.__relocated__.exe",
        ".mcp-proxy.exe.__selfdelete__.exe",
        "notes.txt",
    ] {
        world.files.insert(dir.join(name), b"x".to_vec());
    }
    world.dirs.insert(paths.downloads.join("1.3.9"));
    world
        .files
        .insert(paths.downloads.join("1.3.9").join("mcp-proxy.exe.sig"), b"x".to_vec());
    world.files.insert(paths.downloads.join("stray.tmp"), b"x".to_vec());
    let ops = FakeSystem::new(world);
    let (logs, _guard) = CapturedLogs::start();
    let cleanup = clean_leftovers(&ops, &paths);
    assert_eq!(cleanup, Cleanup { removed: 8, left: 0 });
    let world = ops.world();
    assert!(world.files.contains_key(&paths.install.exe));
    assert!(world.files.contains_key(&dir.join("notes.txt")));
    assert!(world.dirs.contains(&paths.downloads), "the folder stays, empty");
    assert!(!world.files.keys().any(|p| p.starts_with(&paths.downloads)));
    assert!(
        !world
            .dirs
            .iter()
            .any(|p| p.starts_with(&paths.downloads) && *p != paths.downloads)
    );
    assert_eq!(world.files.keys().filter(|p| p.starts_with(dir)).count(), 2);
    assert!(!logs.text().contains("/home/ana"), "{}", logs.text());

    // Nothing to clean, nothing logged; a leftover still held is counted.
    let (world, _) = replaceable();
    let mut world = world;
    world.files.remove(&paths.downloads.join("1.4.0").join("mcp-proxy.exe"));
    world.dirs.remove(&paths.downloads.join("1.4.0"));
    world.files.insert(dir.join("mcp-proxy.exe.old"), b"held".to_vec());
    let ops = FakeSystem::new(world);
    ops.fail_at(0);
    assert_eq!(clean_leftovers(&ops, &paths), Cleanup { removed: 0, left: 1 });
    let ops = FakeSystem::new(World::default());
    assert_eq!(clean_leftovers(&ops, &paths), Cleanup::default());
}

#[test]
fn addresses_of_the_repository() {
    let source = Source {
        base_url: "https://artifactory.example.internal/artifactory/".to_owned(),
        repository: REPO.to_owned(),
        path: "tools/mcp-proxy".to_owned(),
    };
    assert_eq!(
        source.listing_url().unwrap().as_str(),
        "https://artifactory.example.internal/artifactory/api/storage/generic-release-local/tools/mcp-proxy"
    );
    assert_eq!(
        source.file_url(&v("1.4.0-rc.1"), SIG_NAME).unwrap().as_str(),
        "https://artifactory.example.internal/artifactory/generic-release-local/tools/mcp-proxy/1.4.0-rc.1/mcp-proxy.exe.sig"
    );
    assert_eq!(check_interval(6), Duration::from_secs(6 * 3600));
    assert_eq!(check_interval(0), Duration::from_secs(3600));
    assert_eq!(FIRST_CHECK_DELAY, Duration::from_secs(60));
}
