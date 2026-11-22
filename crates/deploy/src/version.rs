//! Versions of MCP Proxy: the one of this exe, the one of an installed exe read in its file, and their order.
//!
//! The installed exe is never executed to learn its version, and its version resource is not read
//! (`GetFileVersionInfo` exists only in `unsafe`, D18). Every build carries a version mark instead,
//! `\x01mcp-proxy:version=<semver>\x01`, found by a plain search through the bytes of the file.

use std::cmp::Ordering;
use std::path::Path;

pub use semver::Version;

const MARK_END: u8 = 0x01;
/// Longest version text read after the prefix.
const VERSION_MAX: usize = 64;
/// Larger files are not read: no build of MCP Proxy comes close.
pub const EXE_SIZE_MAX: u64 = 512 * 1024 * 1024;

/// The mark of this build. Read through `black_box` by [`own_version`], so that the linker keeps it in every exe.
static VERSION_MARK: &str = concat!("\u{1}mcp-proxy:version=", env!("CARGO_PKG_VERSION"), "\u{1}");

/// What precedes the version in a mark, taken from the mark itself at run time: no other copy of these bytes is
/// compiled in, so nothing else in an exe can look like a mark.
pub fn mark_prefix() -> &'static [u8] {
    let mark = std::hint::black_box(VERSION_MARK).as_bytes();
    let end = mark.iter().position(|b| *b == b'=').map_or(mark.len(), |i| i + 1);
    &mark[..end]
}

/// The version of this exe. `0.0.0` if the mark could not be read back, which makes any installed version count
/// as newer or equal: nothing is replaced then.
pub fn own_version() -> Version {
    let mark = std::hint::black_box(VERSION_MARK).as_bytes();
    match marked_version(mark) {
        Marked::One(version) => version,
        Marked::None | Marked::Conflicting => Version::new(0, 0, 0),
    }
}

/// The version of an installed exe, as far as its file tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    Known(Version),
    /// No mark, several different marks, a file too large or unreadable.
    Unknown,
}

/// Reads the version mark of the exe at `path` without executing it.
pub fn installed_version(path: &Path) -> Installed {
    let readable = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= EXE_SIZE_MAX);
    if !readable {
        return Installed::Unknown;
    }
    std::fs::read(path).map_or(Installed::Unknown, |bytes| version_in(&bytes))
}

/// The version mark in the bytes of an exe.
pub fn version_in(bytes: &[u8]) -> Installed {
    match marked_version(bytes) {
        Marked::One(version) => Installed::Known(version),
        Marked::None | Marked::Conflicting => Installed::Unknown,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Marked {
    None,
    One(Version),
    Conflicting,
}

/// Every well-formed mark in `bytes`: exactly one version, however often it appears, or nothing usable.
fn marked_version(bytes: &[u8]) -> Marked {
    let mut found: Option<Version> = None;
    let prefix = mark_prefix();
    for start in memchr::memmem::find_iter(bytes, prefix) {
        let rest = &bytes[start + prefix.len()..];
        let window = &rest[..rest.len().min(VERSION_MAX + 1)];
        let Some(end) = window.iter().position(|b| *b == MARK_END) else {
            continue;
        };
        let Some(version) = std::str::from_utf8(&window[..end])
            .ok()
            .and_then(|text| Version::parse(text).ok())
        else {
            continue;
        };
        match &found {
            None => found = Some(version),
            Some(previous) if *previous == version => {}
            Some(_) => return Marked::Conflicting,
        }
    }
    found.map_or(Marked::None, Marked::One)
}

/// Semver precedence: pre-releases come before their release (`1.4.0-rc.1 < 1.4.0`), build metadata is ignored.
pub fn compare(a: &Version, b: &Version) -> Ordering {
    a.cmp_precedence(b)
}

/// `a` is strictly more recent than `b`.
pub fn is_newer(a: &Version, b: &Version) -> bool {
    compare(a, b) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// A mark built at run time, so that the test binary holds no second mark of its own.
    fn mark(version: &str) -> Vec<u8> {
        let mut bytes = mark_prefix().to_vec();
        bytes.extend_from_slice(version.as_bytes());
        bytes.push(MARK_END);
        bytes
    }

    // Exigence : EX-019-03
    #[test]
    fn precedence_with_pre_releases() {
        assert!(is_newer(&v("1.4.0"), &v("1.3.9")));
        assert!(is_newer(&v("1.10.0"), &v("1.9.0")), "numbers, not text");
        assert!(is_newer(&v("2.0.0"), &v("1.99.99")));
        assert!(
            is_newer(&v("1.4.0"), &v("1.4.0-rc.1")),
            "a release follows its pre-releases"
        );
        assert!(is_newer(&v("1.4.0-rc.2"), &v("1.4.0-rc.1")));
        assert!(
            is_newer(&v("1.4.0-rc.10"), &v("1.4.0-rc.9")),
            "numeric identifiers compare as numbers"
        );
        assert!(is_newer(&v("1.4.0-beta"), &v("1.4.0-alpha.9")));
        assert!(
            is_newer(&v("1.4.0-alpha.1"), &v("1.4.0-alpha")),
            "more identifiers come after"
        );
        assert!(is_newer(&v("1.4.0-rc.1"), &v("1.3.0")));
        assert!(!is_newer(&v("1.4.0"), &v("1.4.0")));
        assert!(!is_newer(&v("1.4.0-rc.1"), &v("1.4.0")));
        assert_eq!(
            compare(&v("1.4.0+build.7"), &v("1.4.0+build.8")),
            Ordering::Equal,
            "build metadata ignored"
        );
        assert!(!is_newer(&v("1.4.0+zzz"), &v("1.4.0")));
    }

    // Exigence : EX-019-10
    #[test]
    fn the_mark_is_read_among_other_bytes() {
        let mut exe = b"MZ\x90\x00 some code".to_vec();
        exe.extend_from_slice(mark_prefix());
        exe.extend_from_slice(b" not a version");
        exe.extend_from_slice(b"\x00\x00");
        exe.extend_from_slice(&mark("1.4.0-rc.2"));
        exe.extend_from_slice(b" more");
        exe.extend_from_slice(&mark("1.4.0-rc.2"));
        assert_eq!(marked_version(&exe), Marked::One(v("1.4.0-rc.2")));

        assert_eq!(marked_version(b"MZ nothing here"), Marked::None);
        let mut cut = mark("1.4.0");
        cut.pop();
        assert_eq!(marked_version(&cut), Marked::None, "a mark cut by the end of the file");
        assert_eq!(marked_version(&mark(&"1".repeat(VERSION_MAX + 4))), Marked::None);
        assert_eq!(marked_version(&mark("1.4")), Marked::None, "not a semver");
        let mut two = mark("1.4.0");
        two.extend_from_slice(&mark("9.9.9"));
        assert_eq!(
            marked_version(&two),
            Marked::Conflicting,
            "two versions: none is believed"
        );
    }

    // Exigence : EX-019-10
    #[test]
    fn this_build_carries_its_mark() {
        assert_eq!(own_version(), v(env!("CARGO_PKG_VERSION")));
        // The test binary itself is an exe of this build: its mark survives the linker.
        let exe = std::env::current_exe().unwrap();
        assert_eq!(installed_version(&exe), Installed::Known(own_version()));
    }

    // Exigence : EX-019-10
    #[test]
    fn unreadable_files_give_an_unknown_version() {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(installed_version(&dir.join("absent.exe")), Installed::Unknown);
        assert_eq!(installed_version(&dir), Installed::Unknown, "a directory");
        let empty = dir.join("empty.exe");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(installed_version(&empty), Installed::Unknown);
        let marked = dir.join("marked.exe");
        let mut bytes = b"MZ".to_vec();
        bytes.extend_from_slice(&mark("0.3.1"));
        std::fs::write(&marked, &bytes).unwrap();
        assert_eq!(installed_version(&marked), Installed::Known(v("0.3.1")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
