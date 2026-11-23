#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const OTHER_SERVERS: &str = include_str!("examples/claude-autres-serveurs.json");
const OLD_ENTRY: &str = include_str!("examples/claude-ancienne-entree.json");
const COMMENTS: &str = include_str!("examples/vscode-commentaires.jsonc");
const UNREADABLE: &str = include_str!("examples/illisible.json");
const STAMP: &str = "20261122-140307";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mcp-proxy-clients-{}-{n}", std::process::id()));
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

/// A user folder: `%APPDATA%`, `%LOCALAPPDATA%`, MCP Proxy installed, no client.
struct Home {
    _root: TempDir,
    env: Environment,
}

impl Home {
    fn new() -> Self {
        let root = TempDir::new();
        let env = Environment {
            app_data: root.0.join("Roaming"),
            local_app_data: root.0.join("Local"),
        };
        std::fs::create_dir_all(&env.app_data).unwrap();
        let exe = env.installed_exe();
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"MZ").unwrap();
        Self { _root: root, env }
    }

    fn exe(&self) -> PathBuf {
        self.env.installed_exe()
    }

    fn target(&self) -> Option<PathBuf> {
        self.env.target(&RealFiles)
    }

    /// The client of `id` is installed: its folder exists.
    fn client(&self, id: LocationId) -> PathBuf {
        let location = self.env.location(id);
        std::fs::create_dir_all(&location.dir).unwrap();
        location.file
    }

    fn with_file(&self, id: LocationId, text: &str) -> PathBuf {
        let file = self.client(id);
        std::fs::write(&file, text).unwrap();
        file
    }

    fn state(&self, id: LocationId) -> State {
        inspect(&RealFiles, &self.env, id, self.target().as_deref()).state
    }

    fn plan(&self, id: LocationId, action: Action) -> Plan {
        plan(&RealFiles, &self.env, id, action, self.target().as_deref()).unwrap()
    }

    /// Preview, then apply what the preview showed.
    fn run(&self, id: LocationId, action: Action) -> Applied {
        self.run_with(&RealFiles, id, action)
    }

    fn run_with(&self, files: &dyn ClientFiles, id: LocationId, action: Action) -> Applied {
        let shown = self.plan(id, action);
        apply(
            files,
            &self.env,
            id,
            action,
            self.target().as_deref(),
            &shown.fingerprint,
            STAMP,
        )
        .unwrap()
    }

    /// The names of the files of the folder of `id`.
    fn names(&self, id: LocationId) -> BTreeSet<String> {
        std::fs::read_dir(self.env.location(id).dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn entry(&self) -> String {
        format!(
            "\"mcp-proxy\": {{\n  \"command\": {},\n  \"args\": [\"stdio\"]\n}}",
            Value::String(self.exe().display().to_string())
        )
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Every server of the file, our entry aside, as values; and the other top-level keys.
fn others(text: &str, section: &'static str) -> Value {
    let (_, body) = jsonc::split_bom_for_tests(text);
    let mut value = jsonc::value_for_tests(body);
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    if let Some(servers) = object.get_mut(section).and_then(Value::as_object_mut) {
        servers.remove(ENTRY_NAME);
        // A section left empty by removing our entry is not a difference of the rest of the file.
        if servers.is_empty() {
            object.remove(section);
        }
    }
    value
}

// Exigence : EX-023-01
#[test]
fn installing_in_each_kind_of_file() {
    let home = Home::new();
    let exe = Value::String(home.exe().display().to_string()).to_string();

    // Absent: created, with our entry alone.
    let file = home.client(LocationId::ClaudeDesktop);
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(shown.change, Change::Create { entry: home.entry() });
    assert!(!shown.makes_backup());
    home.run(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(
        read(&file),
        format!(
            "{{\n  \"mcpServers\": {{\n    \"mcp-proxy\": {{\n      \"command\": {exe},\n      \"args\": [\"stdio\"]\n    }}\n  }}\n}}\n"
        )
    );
    assert_eq!(home.state(LocationId::ClaudeDesktop), State::Configured);

    // Empty.
    let file = home.with_file(LocationId::VsCode, "");
    home.run(LocationId::VsCode, Action::Install);
    assert_eq!(
        read(&file),
        format!(
            "{{\n  \"servers\": {{\n    \"mcp-proxy\": {{\n      \"type\": \"stdio\",\n      \"command\": {exe},\n      \"args\": [\"stdio\"]\n    }}\n  }}\n}}\n"
        )
    );
    assert_eq!(home.state(LocationId::VsCode), State::Configured);

    // With other servers: ours comes last, in the layout of the file.
    let file = home.with_file(LocationId::ClaudeDesktop, OTHER_SERVERS);
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(shown.change, Change::Add { entry: home.entry() });
    assert_eq!(shown.other_servers, 2);
    home.run(LocationId::ClaudeDesktop, Action::Install);
    let expected = OTHER_SERVERS.replace(
        "      }\n    }\n  },",
        &format!(
            "      }}\n    }},\n    \"mcp-proxy\": {{\n      \"command\": {exe},\n      \"args\": [\"stdio\"]\n    }}\n  }},"
        ),
    );
    assert_eq!(read(&file), expected);

    // With an old entry: replaced in place, the other server after it untouched.
    let file = home.with_file(LocationId::ClaudeDesktop, OLD_ENTRY);
    assert!(matches!(home.state(LocationId::ClaudeDesktop), State::OtherCommand(_)));
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(
        shown.change,
        Change::Replace {
            previous: EntrySummary {
                command: Some(r"C:\Users\ana\Downloads\mcp-proxy.exe".to_owned()),
                other_keys: vec!["args".to_owned(), "env".to_owned()],
            },
            entry: home.entry(),
        }
    );
    home.run(LocationId::ClaudeDesktop, Action::Install);
    let written = read(&file);
    assert!(written.starts_with(&format!(
        "{{\n  \"mcpServers\": {{\n    \"mcp-proxy\": {{\n      \"command\": {exe},\n      \"args\": [\"stdio\"]\n    }},\n    \"github\""
    )));
    assert!(!written.contains("SENTINEL-old-entry-token"));
    assert_eq!(others(&written, "mcpServers"), others(OLD_ENTRY, "mcpServers"));
    assert_eq!(home.state(LocationId::ClaudeDesktop), State::Configured);

    // With comments, trailing commas, tabs and CRLF: each kept, ours in the same style.
    let file = home.with_file(LocationId::VsCode, COMMENTS);
    home.run(LocationId::VsCode, Action::Install);
    let expected = COMMENTS.replace(
        "\t\t},\r\n\t\t// fin des serveurs",
        &format!(
            "\t\t}},\r\n\t\t\"mcp-proxy\": {{\r\n\t\t\t\"type\": \"stdio\",\r\n\t\t\t\"command\": {exe},\r\n\t\t\t\"args\": [\"stdio\"]\r\n\t\t}},\r\n\t\t// fin des serveurs"
        ),
    );
    assert_eq!(read(&file), expected);
    assert_eq!(home.state(LocationId::VsCode), State::Configured);
}

// Exigence : EX-023-02
#[test]
fn the_rest_of_the_file_is_intact_after_installing_and_removing() {
    for (id, text, section) in [
        (LocationId::ClaudeDesktop, OTHER_SERVERS, "mcpServers"),
        (LocationId::ClaudeDesktopPackage, OTHER_SERVERS, "mcpServers"),
        (LocationId::VsCode, COMMENTS, "servers"),
        (
            LocationId::VsCode,
            "{\"servers\":{\"a\":{\"command\":\"a\"}},\"x\":1}",
            "servers",
        ),
        (LocationId::VsCode, "{\n  \"inputs\": []\n}\n", "servers"),
        (
            LocationId::VsCode,
            "\u{feff}{\n  \"servers\": {\n    \"a\": {}, // a\n  }\n}",
            "servers",
        ),
        (LocationId::ClaudeDesktop, "{\n  \"mcpServers\": {}\n}\n", "mcpServers"),
        (
            LocationId::ClaudeDesktop,
            "{\n  \"mcpServers\": { /* vide */ }\n}\n",
            "mcpServers",
        ),
        (LocationId::VsCode, "{\"servers\": {\"a\": 1,}}", "servers"),
        (
            LocationId::VsCode,
            "{\n  \"servers\": {\n    \"a\": 1 // c\n  },\n  \"x\": [1, 2.50]\n}",
            "servers",
        ),
        (
            LocationId::VsCode,
            "{\n  // top\n  \"servers\": {\n    /* first */\n    \"a\": 1\n  }\n}\n",
            "servers",
        ),
    ] {
        let home = Home::new();
        let file = home.with_file(id, text);
        home.run(id, Action::Install);
        let installed = read(&file);
        assert_eq!(others(&installed, section), others(text, section), "{text}");
        assert_eq!(home.state(id), State::Configured, "{text}");
        let removed = home.run(id, Action::Remove);
        assert!(removed.written);
        let back = read(&file);
        assert_eq!(others(&back, section), others(text, section), "{text}");
        assert_eq!(home.state(id), State::NotConfigured { file_exists: true }, "{text}");
        // Without a section before, the empty section stays: only our entry is ever removed.
        if text.contains(section) {
            assert_eq!(back, text, "byte for byte: {text}");
        }
    }

    // Removing an old entry first in its section: the server after it, its comma, the other keys stay.
    let home = Home::new();
    let file = home.with_file(LocationId::ClaudeDesktop, OLD_ENTRY);
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Remove);
    assert!(matches!(shown.change, Change::Remove { .. }));
    home.run(LocationId::ClaudeDesktop, Action::Remove);
    assert_eq!(
        read(&file),
        "{\n  \"mcpServers\": {\n    \"github\": {\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"@modelcontextprotocol/server-github\"]\n    }\n  }\n}\n"
    );

    // Last in its section, without a trailing comma: the comma before it goes too.
    let text = "{\n  \"servers\": {\n    \"a\": {}, // a\n    \"mcp-proxy\": {}\n  }\n}\n";
    let file = home.with_file(LocationId::VsCode, text);
    home.run(LocationId::VsCode, Action::Remove);
    assert_eq!(read(&file), "{\n  \"servers\": {\n    \"a\": {} // a\n  }\n}\n");

    // Nothing to remove: nothing written.
    let removed = home.run(LocationId::VsCode, Action::Remove);
    assert_eq!(removed.change, Change::Nothing);
    assert!(!removed.written);
}

// Exigence : EX-023-03
#[test]
fn an_unreadable_file_is_never_written() {
    let home = Home::new();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("syntax", UNREADABLE.as_bytes().to_vec()),
        ("not utf-8", vec![0xff, 0xfe, b'{', 0, b'}', 0]),
        ("array", b"[]".to_vec()),
        ("section", b"{\"mcpServers\": []}".to_vec()),
        (
            "twice",
            b"{\"mcpServers\": {\"mcp-proxy\": {}, \"mcp-proxy\": {}}}".to_vec(),
        ),
        ("too large", {
            let mut big = b"{\"a\": \"".to_vec();
            big.resize(usize::try_from(MAX_FILE_SIZE).unwrap() + 10, b'x');
            big.extend_from_slice(b"\"}");
            big
        }),
    ];
    for (case, bytes) in cases {
        let file = home.client(LocationId::ClaudeDesktop);
        std::fs::write(&file, &bytes).unwrap();
        let before = home.names(LocationId::ClaudeDesktop);
        assert!(
            matches!(home.state(LocationId::ClaudeDesktop), State::Unreadable(_)),
            "{case}"
        );
        for action in [Action::Install, Action::Remove] {
            let shown = home.plan(LocationId::ClaudeDesktop, action);
            let Change::Manual { reason, snippet } = &shown.change else {
                panic!("{case}: {:?}", shown.change)
            };
            assert!(!reason.to_string().contains("SENTINEL"), "{case}");
            match action {
                Action::Install => {
                    let snippet = snippet.as_deref().unwrap();
                    let exe = Value::String(home.exe().display().to_string()).to_string();
                    assert!(snippet.contains("\"mcpServers\"") && snippet.contains(&exe), "{case}");
                }
                Action::Remove => assert_eq!(snippet, &None),
            }
            let applied = home.run(LocationId::ClaudeDesktop, action);
            assert!(!applied.written, "{case}");
            assert_eq!(std::fs::read(&file).unwrap(), bytes, "{case}");
            assert_eq!(
                home.names(LocationId::ClaudeDesktop),
                before,
                "{case}: no backup, no temporary file"
            );
        }
    }
    assert_eq!(
        jsonc::find(UNREADABLE, "mcpServers", ENTRY_NAME)
            .err()
            .map(|e| e.to_string()),
        Some("JSON invalide à la ligne 4".to_owned())
    );

    // A link is never followed nor replaced.
    #[cfg(unix)]
    {
        let file = home.client(LocationId::VsCode);
        let target = home.env.app_data.join("elsewhere.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert_eq!(home.state(LocationId::VsCode), State::Unreadable(Unreadable::NotAFile));
        assert!(!home.run(LocationId::VsCode, Action::Install).written);
        assert_eq!(read(&target), "{}");
    }
}

// Exigence : EX-023-04
#[test]
fn a_backup_comes_first_and_installing_twice_changes_nothing() {
    let home = Home::new();
    let file = home.with_file(LocationId::ClaudeDesktop, OTHER_SERVERS);
    let first = home.run(LocationId::ClaudeDesktop, Action::Install);
    let backup = first.backup.unwrap();
    assert_eq!(backup, format!("claude_desktop_config.json.bak-{STAMP}"));
    assert_eq!(read(&file.with_file_name(&backup)), OTHER_SERVERS);
    let installed = read(&file);
    let names = home.names(LocationId::ClaudeDesktop);
    assert_eq!(names.len(), 2, "{names:?}");

    let second = home.plan(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(second.change, Change::Nothing);
    assert!(!second.makes_backup());
    let second = home.run(LocationId::ClaudeDesktop, Action::Install);
    assert!(!second.written);
    assert_eq!(read(&file), installed);
    assert_eq!(home.names(LocationId::ClaudeDesktop), names);

    // The same second again: a second backup, never an overwritten one.
    home.run(LocationId::ClaudeDesktop, Action::Remove);
    let third = home.run(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(third.backup, Some(format!("claude_desktop_config.json.bak-{STAMP}-3")));
    assert_eq!(read(&file.with_file_name(&backup)), OTHER_SERVERS);
    assert_eq!(read(&file), installed);
}

// Exigence : EX-023-05
#[test]
fn each_state_is_detected() {
    let home = Home::new();
    for id in LocationId::ALL {
        assert_eq!(home.state(id), State::NotInstalled);
    }
    home.client(LocationId::VsCode);
    assert_eq!(
        home.state(LocationId::VsCode),
        State::NotConfigured { file_exists: false }
    );
    home.with_file(LocationId::VsCode, "{\"servers\": {\"a\": {}}}");
    let seen = inspect(&RealFiles, &home.env, LocationId::VsCode, home.target().as_deref());
    assert_eq!(seen.state, State::NotConfigured { file_exists: true });
    assert_eq!(seen.other_servers, 1);

    home.with_file(LocationId::ClaudeDesktop, OLD_ENTRY);
    assert_eq!(
        home.state(LocationId::ClaudeDesktop),
        State::OtherCommand(EntrySummary {
            command: Some(r"C:\Users\ana\Downloads\mcp-proxy.exe".to_owned()),
            other_keys: vec!["args".to_owned(), "env".to_owned()],
        })
    );
    home.with_file(LocationId::ClaudeDesktopPackage, UNREADABLE);
    assert_eq!(
        home.state(LocationId::ClaudeDesktopPackage),
        State::Unreadable(Unreadable::Syntax { line: 4 })
    );

    // Ours: the installed exe, compared as Windows compares paths; anything else is another command.
    let exe = home.exe().display().to_string();
    for (entry, ours) in [
        (
            format!(r#"{{"command": {}, "args": ["stdio"]}}"#, Value::String(exe.clone())),
            true,
        ),
        (
            format!(
                r#"{{"command": {}, "args": ["stdio"]}}"#,
                Value::String(exe.to_uppercase())
            ),
            true,
        ),
        (
            format!(r#"{{"command": {}, "args": []}}"#, Value::String(exe.clone())),
            false,
        ),
        (
            format!(
                r#"{{"command": {}, "args": ["stdio"], "env": {{}}}}"#,
                Value::String(exe.clone())
            ),
            false,
        ),
        (r#"{"command": "mcp-proxy.exe", "args": ["stdio"]}"#.to_owned(), false),
        ("\"mcp-proxy.exe\"".to_owned(), false),
    ] {
        home.with_file(
            LocationId::ClaudeDesktop,
            &format!("{{\"mcpServers\": {{\"mcp-proxy\": {entry}}}}}"),
        );
        assert_eq!(
            home.state(LocationId::ClaudeDesktop) == State::Configured,
            ours,
            "{entry}"
        );
    }
    // VS Code wants its `type`.
    home.with_file(
        LocationId::VsCode,
        &format!(
            r#"{{"servers": {{"mcp-proxy": {{"command": {}, "args": ["stdio"]}}}}}}"#,
            Value::String(exe)
        ),
    );
    assert!(matches!(home.state(LocationId::VsCode), State::OtherCommand(_)));

    // Without MCP Proxy installed, no entry is ours.
    std::fs::remove_file(home.exe()).unwrap();
    assert_eq!(home.target(), None);
    home.run_without_target_is_refused();
}

impl Home {
    fn run_without_target_is_refused(&self) {
        assert!(matches!(self.state(LocationId::VsCode), State::OtherCommand(_)));
        assert_eq!(
            plan(&RealFiles, &self.env, LocationId::VsCode, Action::Install, None).err(),
            Some(ClientError::NoTarget)
        );
    }
}

// Exigence : EX-023-09
#[test]
fn what_is_written_is_what_was_shown() {
    let home = Home::new();
    let file = home.with_file(LocationId::ClaudeDesktop, OTHER_SERVERS);
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
    // The client rewrote its file in between.
    let rewritten = OTHER_SERVERS.replace("Ctrl+Space", "Alt+Space");
    std::fs::write(&file, &rewritten).unwrap();
    let target = home.target();
    let refused = apply(
        &RealFiles,
        &home.env,
        LocationId::ClaudeDesktop,
        Action::Install,
        target.as_deref(),
        &shown.fingerprint,
        STAMP,
    );
    assert_eq!(refused.err(), Some(ClientError::Changed));
    assert_eq!(read(&file), rewritten);
    assert_eq!(home.names(LocationId::ClaudeDesktop).len(), 1);

    // Another action, another location or another exe than the preview's: refused too.
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
    let other_exe = home.env.app_data.join("other.exe");
    std::fs::write(&other_exe, b"MZ").unwrap();
    for (id, action, exe) in [
        (LocationId::ClaudeDesktop, Action::Remove, target.clone()),
        (LocationId::ClaudeDesktopPackage, Action::Install, target.clone()),
        (LocationId::ClaudeDesktop, Action::Install, Some(other_exe)),
    ] {
        home.client(id);
        let refused = apply(
            &RealFiles,
            &home.env,
            id,
            action,
            exe.as_deref(),
            &shown.fingerprint,
            STAMP,
        );
        assert_eq!(refused.err(), Some(ClientError::Changed));
    }
    assert_eq!(read(&file), rewritten);

    // Changed after the last reading but before the rename: the last look catches it, nothing stays behind.
    let racing = Racing {
        file: file.clone(),
        reads: AtomicUsize::new(0),
    };
    let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
    let refused = apply(
        &racing,
        &home.env,
        LocationId::ClaudeDesktop,
        Action::Install,
        target.as_deref(),
        &shown.fingerprint,
        STAMP,
    );
    assert_eq!(refused.err(), Some(ClientError::Changed));
    assert_eq!(read(&file), "{}");
    assert_eq!(home.names(LocationId::ClaudeDesktop).len(), 1);
}

/// Rewrites the file once the plan has read it, as a running client would.
struct Racing {
    file: PathBuf,
    reads: AtomicUsize,
}

impl ClientFiles for Racing {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let bytes = RealFiles.read(path)?;
        if path == self.file && self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            std::fs::write(&self.file, "{}")?;
        }
        Ok(bytes)
    }
}

/// Fails the `n`-th file change (creation, rename, removal), counted from 0.
struct Failing {
    n: usize,
    seen: Mutex<usize>,
}

impl Failing {
    fn new(n: usize) -> Self {
        Self { n, seen: Mutex::new(0) }
    }

    fn step(&self) -> io::Result<()> {
        let mut seen = self.seen.lock().unwrap();
        let now = *seen;
        *seen += 1;
        if now == self.n {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    }
}

impl ClientFiles for Failing {
    fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.step()?;
        RealFiles.create_new(path, bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.step()?;
        RealFiles.rename(from, to)
    }
}

// Exigence : EX-023-10
#[test]
fn a_failed_write_leaves_the_file_as_it_was() {
    let home = Home::new();
    // Steps: backup, temporary file, rename.
    for n in 0..3 {
        let file = home.with_file(LocationId::ClaudeDesktop, OTHER_SERVERS);
        let names = home.names(LocationId::ClaudeDesktop);
        let shown = home.plan(LocationId::ClaudeDesktop, Action::Install);
        let failed = apply(
            &Failing::new(n),
            &home.env,
            LocationId::ClaudeDesktop,
            Action::Install,
            home.target().as_deref(),
            &shown.fingerprint,
            STAMP,
        )
        .unwrap_err();
        let text = failed.to_string();
        assert!(text.contains("accès refusé"), "{text}");
        assert!(
            text.contains("rien n'a été écrit") || text.contains("inchangé"),
            "{text}"
        );
        assert!(!text.contains(&home.env.app_data.display().to_string()), "{text}");
        assert_eq!(read(&file), OTHER_SERVERS, "step {n}");
        assert_eq!(home.names(LocationId::ClaudeDesktop), names, "step {n}");
    }
    // A new file: no backup, the rename fails, nothing stays.
    home.client(LocationId::VsCode);
    let shown = home.plan(LocationId::VsCode, Action::Install);
    let failed = apply(
        &Failing::new(1),
        &home.env,
        LocationId::VsCode,
        Action::Install,
        home.target().as_deref(),
        &shown.fingerprint,
        STAMP,
    );
    assert!(failed.is_err());
    assert!(home.names(LocationId::VsCode).is_empty());
}

// Exigence : EX-023-11
#[test]
fn a_client_not_installed_gets_nothing() {
    let home = Home::new();
    for id in LocationId::ALL {
        assert_eq!(
            plan(&RealFiles, &home.env, id, Action::Install, home.target().as_deref()).err(),
            Some(ClientError::NotInstalled(id))
        );
        let removed = remove_entry(&RealFiles, &home.env, id, STAMP).unwrap();
        assert!(!removed.written);
        assert!(!home.env.location(id).dir.exists());
    }
    assert_eq!(
        ClientError::NotInstalled(LocationId::VsCode).to_string(),
        "VS Code n'est pas installé sur ce poste : rien n'est créé"
    );
    // MCP Proxy itself not installed: nothing to add, removing stays possible.
    let file = home.with_file(LocationId::ClaudeDesktop, OLD_ENTRY);
    std::fs::remove_file(home.exe()).unwrap();
    assert_eq!(
        plan(&RealFiles, &home.env, LocationId::ClaudeDesktop, Action::Install, None).err(),
        Some(ClientError::NoTarget)
    );
    let shown = plan(&RealFiles, &home.env, LocationId::ClaudeDesktop, Action::Remove, None).unwrap();
    assert!(matches!(shown.change, Change::Remove { .. }));
    apply(
        &RealFiles,
        &home.env,
        LocationId::ClaudeDesktop,
        Action::Remove,
        None,
        &shown.fingerprint,
        STAMP,
    )
    .unwrap();
    assert!(!read(&file).contains("mcp-proxy"));
}

// Exigence : EX-023-12
#[test]
fn the_two_locations_of_claude_desktop_are_seen_and_changed_apart() {
    let home = Home::new();
    let classic = home.with_file(LocationId::ClaudeDesktop, OTHER_SERVERS);
    let package = home.client(LocationId::ClaudeDesktopPackage);
    assert!(
        package.ends_with("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude/claude_desktop_config.json"),
        "{}",
        package.display()
    );
    assert!(classic.starts_with(&home.env.app_data));
    assert_eq!(
        home.state(LocationId::ClaudeDesktop),
        State::NotConfigured { file_exists: true }
    );
    assert_eq!(
        home.state(LocationId::ClaudeDesktopPackage),
        State::NotConfigured { file_exists: false }
    );
    home.run(LocationId::ClaudeDesktopPackage, Action::Install);
    assert_eq!(home.state(LocationId::ClaudeDesktopPackage), State::Configured);
    assert_eq!(read(&classic), OTHER_SERVERS);
    home.run(LocationId::ClaudeDesktop, Action::Install);
    assert_eq!(
        configured(&RealFiles, &home.env),
        vec![LocationId::ClaudeDesktop, LocationId::ClaudeDesktopPackage]
    );
    // What uninstalling will do: each removed, with its backup.
    for id in configured(&RealFiles, &home.env) {
        let removed = remove_entry(&RealFiles, &home.env, id, STAMP).unwrap();
        assert!(removed.written);
    }
    assert!(configured(&RealFiles, &home.env).is_empty());
    assert_eq!(read(&classic), OTHER_SERVERS);
}

// Exigence : EX-023-13
#[test]
fn nothing_of_the_files_goes_into_a_debug_or_an_error() {
    let home = Home::new();
    home.with_file(LocationId::ClaudeDesktop, OLD_ENTRY);
    home.with_file(LocationId::ClaudeDesktopPackage, OTHER_SERVERS);
    home.with_file(LocationId::VsCode, UNREADABLE);
    for id in LocationId::ALL {
        for action in [Action::Install, Action::Remove] {
            let shown = home.plan(id, action);
            let text = format!("{shown:?} {:?}", inspect(&RealFiles, &home.env, id, None));
            assert!(!text.contains("SENTINEL"), "{text}");
            assert!(!text.contains("server-github"), "{text}");
        }
    }
    assert!(LocationId::parse("claude-desktop").is_some());
    for raw in ["", "Claude-Desktop", "../vscode", r"C:\x.json", "vscode "] {
        assert_eq!(LocationId::parse(raw), None);
    }
    assert_eq!(Action::parse("install"), Some(Action::Install));
    assert_eq!(Action::parse("Install"), None);
    let snippet = generic_snippet(Path::new(r"C:\P\mcp-proxy.exe"));
    assert_eq!(
        snippet,
        "{\n  \"mcpServers\": {\n    \"mcp-proxy\": {\n      \"command\": \"C:\\\\P\\\\mcp-proxy.exe\",\n      \"args\": [\"stdio\"]\n    }\n  }\n}\n"
    );
}
