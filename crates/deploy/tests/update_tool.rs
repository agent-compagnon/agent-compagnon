//! `update-tool`, the binary behind `cargo xtask keygen | sign | verify | check-release`, run as the pipeline runs
//! it: the private key goes from the file of `keygen` to a variable, and never appears in what the tool prints.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mcp_proxy_deploy::version::mark_prefix;

fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_update-tool"))
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-update-tool-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exe(dir: &Path, version: &str) -> PathBuf {
    let mut bytes = b"MZ an exe ".to_vec();
    bytes.extend_from_slice(mark_prefix());
    bytes.extend_from_slice(version.as_bytes());
    bytes.push(0x01);
    let path = dir.join("mcp-proxy.exe");
    std::fs::write(&path, bytes).unwrap();
    path
}

// Exigence : EX-020-15
#[test]
fn keygen_sign_and_verify_never_show_the_private_key() {
    let dir = TempDir::new("cycle");
    let key_file = dir.0.join("signing.key");
    let generated = tool().args(["keygen", "--out"]).arg(&key_file).output().unwrap();
    assert!(generated.status.success(), "{}", text(&generated));
    let private = std::fs::read_to_string(&key_file).unwrap().trim().to_owned();
    assert_eq!(private.len(), 64);
    let shown = text(&generated);
    assert!(!shown.contains(&private), "keygen printed the private key");
    let public = shown
        .lines()
        .find_map(|l| l.strip_prefix("clé publique : "))
        .and_then(|l| l.split(' ').next())
        .unwrap()
        .to_owned();
    assert_eq!(public.len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&key_file).unwrap().permissions().mode() & 0o077,
            0,
            "readable by others"
        );
    }
    // Never overwrites a key.
    let again = tool().args(["keygen", "--out"]).arg(&key_file).output().unwrap();
    assert_eq!(again.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&key_file).unwrap().trim(), private);

    let exe = exe(&dir.0, "1.4.0");
    let signed = tool()
        .args(["sign", "--exe"])
        .arg(&exe)
        .args([
            "--version",
            "v1.4.0",
            "--critical",
            "true",
            "--key-env",
            "MCP_PROXY_TEST_SIGNING_KEY",
        ])
        .env("MCP_PROXY_TEST_SIGNING_KEY", format!("{private}\n"))
        .output()
        .unwrap();
    assert!(signed.status.success(), "{}", text(&signed));
    assert!(!text(&signed).contains(&private), "sign printed the private key");
    let sig = std::fs::read_to_string(dir.0.join("mcp-proxy.exe.sig")).unwrap();
    assert!(
        sig.contains("version: 1.4.0\n") && sig.contains("critical: true\n"),
        "{sig}"
    );
    assert!(!sig.contains(&private));

    let verified = tool()
        .args(["verify", "--exe"])
        .arg(&exe)
        .args(["--public-key", &public])
        .output()
        .unwrap();
    assert!(verified.status.success(), "{}", text(&verified));
    assert!(text(&verified).contains("version 1.4.0"));

    // Another key, an altered exe: refused.
    let other = "1111111111111111111111111111111111111111111111111111111111111111";
    let refused = tool()
        .args(["verify", "--exe"])
        .arg(&exe)
        .args([
            "--public-key",
            "8f4d6578a8e95e6b17957e2a71f6147db8bc74d04ea7b75199db7578d4c5d57a",
        ])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused));
    std::fs::write(&exe, b"MZ altered").unwrap();
    let altered = tool()
        .args(["verify", "--exe"])
        .arg(&exe)
        .args(["--public-key", &public])
        .output()
        .unwrap();
    assert_eq!(altered.status.code(), Some(1));

    // A wrong version, a malformed or missing key: refused, and the value of the variable is never repeated.
    let exe = self::exe(&dir.0, "1.4.0");
    let wrong = tool()
        .args(["sign", "--exe"])
        .arg(&exe)
        .args(["--version", "1.5.0"])
        .env("ED25519_PRIVATE_KEY", &private)
        .output()
        .unwrap();
    assert_eq!(wrong.status.code(), Some(1));
    assert!(!text(&wrong).contains(&private));
    for value in [
        &format!("{private}zz"),
        &private.to_uppercase(),
        &other[..10].to_owned(),
        &String::new(),
    ] {
        let failed = tool()
            .args(["sign", "--exe"])
            .arg(&exe)
            .env("ED25519_PRIVATE_KEY", value)
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1));
        let shown = text(&failed);
        assert!(shown.contains("ED25519_PRIVATE_KEY"), "{shown}");
        if !value.is_empty() {
            assert!(!shown.contains(value.as_str()), "{shown}");
        }
    }
    let missing = tool()
        .args(["sign", "--exe"])
        .arg(&exe)
        .env_remove("ED25519_PRIVATE_KEY")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
}

// Exigence : EX-020-05
#[test]
fn check_release_refuses_a_build_without_its_release_key() {
    let output = tool().arg("check-release").output().unwrap();
    if mcp_proxy_deploy::update::embedded_slots()[0].is_none() {
        // The repository holds no release key yet: this very build would never be updated.
        assert_eq!(output.status.code(), Some(1), "{}", text(&output));
        assert!(text(&output).contains("keys/current.pub"), "{}", text(&output));
    } else {
        assert!(output.status.success(), "{}", text(&output));
    }
    // The bytes of a build that holds the development key are refused.
    let dir = TempDir::new("check");
    let built = dir.0.join("built.exe");
    let mut bytes = b"MZ".to_vec();
    bytes.extend_from_slice(&[
        0x8f, 0x4d, 0x65, 0x78, 0xa8, 0xe9, 0x5e, 0x6b, 0x17, 0x95, 0x7e, 0x2a, 0x71, 0xf6, 0x14, 0x7d, 0xb8, 0xbc,
        0x74, 0xd0, 0x4e, 0xa7, 0xb7, 0x51, 0x99, 0xdb, 0x75, 0x78, 0xd4, 0xc5, 0xd5, 0x7a,
    ]);
    std::fs::write(&built, bytes).unwrap();
    let output = tool().args(["check-release", "--exe"]).arg(&built).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
}

#[test]
fn arguments_are_strict() {
    for args in [
        &[][..],
        &["unknown"][..],
        &["keygen"][..],
        &["sign", "--exe"][..],
        &["sign", "--exe", "x", "--critical", "yes"][..],
        &["verify", "--exe", "x", "--other", "y"][..],
    ] {
        let output = tool().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", text(&output));
    }
}
