use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::SecretString;
use serde_json::{Value, json};

use super::*;
use crate::config::{Channel, Config, Decision, EnvId, FilterConfig, McpId, SessionId, ToolName};
use crate::diagnostics::{DiagnosticState, Diagnostics, archive_name};
use crate::filter::{OutputFilter, SECRET_MARKER, UNFILTERED_RECORD};
use crate::secrets::test_support::TestWrapper;
use crate::secrets::{Credential, SecretStore};

const SECRET: &str = "S3NT1NEL-6f1c2b9e-do-not-print";
const PASSWORD: &str = "mot-de-passe-9d41c7e2";
/// 2026-11-22T14:03:07.123Z.
const NOW_MS: u64 = 1_795_356_187_123;
const DAY: Duration = Duration::from_secs(86_400);

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(NOW_MS)
}

/// A directory of its own under the system temporary directory, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mcp-proxy-trace-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    store: Arc<SecretStore>,
    wrapper: TestWrapper,
    filter: Arc<OutputFilter>,
    clock: ManualWallClock,
}

fn sid(id: &str) -> SessionId {
    SessionId::new(id).unwrap()
}

fn fixture() -> Fixture {
    let wrapper = TestWrapper::default();
    let store = Arc::new(SecretStore::new(Box::new(wrapper.clone())).unwrap());
    store
        .put(
            &sid("sso-dev"),
            Credential::Bearer {
                token: SecretString::from(SECRET.to_owned()),
            },
        )
        .unwrap();
    store
        .put(
            &sid("basic-dev"),
            Credential::Basic {
                login: "jdoe".to_owned(),
                password: SecretString::from(PASSWORD.to_owned()),
            },
        )
        .unwrap();
    let filter = Arc::new(OutputFilter::new(Arc::clone(&store), &FilterConfig::default()).unwrap());
    Fixture {
        store,
        wrapper,
        filter,
        clock: ManualWallClock::new(now()),
    }
}

/// Every variant of every known secret, as the filter searches them.
fn variants(store: &SecretStore) -> Vec<Vec<u8>> {
    store
        .with_variants(|v| v.iter().filter(|x| !x.is_empty()).map(|x| x.to_vec()).collect())
        .unwrap()
}

#[track_caller]
fn assert_no_variant(store: &SecretStore, what: &str, haystack: &[u8]) {
    let variants = variants(store);
    assert!(variants.len() > 10, "{} variants only", variants.len());
    for variant in &variants {
        assert!(
            memchr::memmem::find(haystack, variant).is_none(),
            "{what} contains a variant: {}",
            String::from_utf8_lossy(variant)
        );
    }
}

fn config(capacity: usize, max_bytes: usize, slow_secs: u64) -> JournalConfig {
    JournalConfig {
        ring_capacity: capacity,
        ring_max_bytes: max_bytes,
        slow_threshold: Duration::from_secs(slow_secs),
    }
}

fn journal(f: &Fixture, config: JournalConfig, log: Option<FileLog>) -> Journal {
    Journal::new(config, Arc::clone(&f.filter), Arc::new(f.clock.clone()), log)
}

fn open(f: &Fixture, dir: &Path, retention_days: u32, max_file_bytes: u64) -> (FileLog, LogGuard) {
    let options = LogFileOptions {
        dir: dir.to_path_buf(),
        retention_days,
        max_file_bytes,
        queue_lines: DEFAULT_QUEUE_LINES,
    };
    open_log(options, Arc::new(f.clock.clone()), Arc::clone(&f.filter)).unwrap()
}

fn env(id: &str) -> EnvId {
    EnvId::new(id).unwrap()
}

fn mcp(id: &str) -> McpId {
    McpId::new(id).unwrap()
}

fn record(id: u64, status: CallStatus) -> CallRecord {
    CallRecord {
        id,
        started_at: now(),
        duration: Duration::from_millis(120),
        client: "claude-ai".to_owned(),
        tool: "dev_kibana_search_logs".to_owned(),
        env: Some(env("dev")),
        mcp: Some(mcp("kibana-logs")),
        session: Some(sid("sso-dev")),
        policy: Some(PolicyRecord {
            decision: Decision::Allow,
            explanation: "autorisé par le profil développeur".to_owned(),
        }),
        status,
        request: json!({ "q": "visible-request-word" }),
        response: json!({ "content": [{ "type": "text", "text": "visible-response-word" }] }),
        filter_report: crate::filter::FilterReport::default(),
    }
}

/// A record whose content carries a raw secret, as a faulty producer would build it.
fn leaky_record(id: u64, status: CallStatus) -> CallRecord {
    let mut r = record(id, status);
    r.client = format!("client {SECRET}");
    r.tool = format!("outil-{SECRET}");
    r.request = json!({ "q": SECRET, "password_hint": PASSWORD, "auth": format!("Bearer {SECRET}") });
    r.response = json!({ "content": [{ "type": "text", "text": format!("jeton {SECRET} et {PASSWORD}") }] });
    if let Some(policy) = &mut r.policy {
        policy.explanation = format!("explication {SECRET}");
    }
    r
}

fn read_logs(dir: &Path) -> String {
    let mut text = String::new();
    for file in log_files(dir).unwrap() {
        text.push_str(&fs::read_to_string(file).unwrap());
    }
    text
}

fn entries_text(journal: &Journal) -> String {
    format!("{:?}", journal.snapshot(&JournalFilter::default()))
}

// --- Buffer -----------------------------------------------------------------------------------------------------

// Exigence : EX-009-01
#[test]
fn the_buffer_keeps_the_most_recent_entries_within_its_count() {
    let f = fixture();
    let journal = journal(&f, config(3, 1 << 20, 5), None);
    for id in 1..=5 {
        journal.record(record(id, CallStatus::Error));
    }
    let entries = journal.snapshot(&JournalFilter::default());
    let ids: Vec<u64> = entries.iter().map(|e| e.details.as_ref().unwrap().call_id).collect();
    assert_eq!(ids, [3, 4, 5]);
    assert_eq!(entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4, 5]);
    assert_eq!(journal.usage().entries, 3);
}

// Exigence : EX-009-01
#[test]
fn the_buffer_keeps_the_most_recent_entries_within_its_bytes() {
    let f = fixture();
    let big = "x".repeat(10_000);
    let max_bytes = 35_000;
    let journal = journal(&f, config(1000, max_bytes, 5), None);
    for id in 1..=10 {
        let mut r = record(id, CallStatus::Error);
        r.response = json!({ "text": big });
        journal.record(r);
        let usage = journal.usage();
        assert!(usage.bytes <= max_bytes, "{usage:?}");
    }
    let entries = journal.snapshot(&JournalFilter::default());
    assert_eq!(entries.len(), 3, "three entries of about 10 kB fit in 35 kB");
    let ids: Vec<u64> = entries.iter().map(|e| e.details.as_ref().unwrap().call_id).collect();
    assert_eq!(ids, [8, 9, 10]);

    // An entry heavier than the whole buffer keeps its summary only.
    let mut huge = record(11, CallStatus::Error);
    huge.response = json!({ "text": "y".repeat(50_000) });
    journal.record(huge);
    let last = journal.snapshot(&JournalFilter::default()).pop().unwrap();
    assert!(last.details.is_none());
    assert!(last.summary.contains("détails trop volumineux"), "{}", last.summary);
    assert!(journal.usage().bytes <= max_bytes);
}

// --- Secrets ----------------------------------------------------------------------------------------------------

// Exigence : EX-009-02
#[test]
fn a_secret_in_a_call_reaches_neither_the_buffer_nor_the_subscribers_nor_the_file_nor_the_archive() {
    let f = fixture();
    let dir = TempDir::new("secret");
    let logs = dir.path().join("logs");
    let (log, guard) = open(&f, &logs, 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log.clone()));
    let mut events = journal.subscribe();
    for (id, status) in [
        CallStatus::Error,
        CallStatus::Blocked,
        CallStatus::Refused,
        CallStatus::LeakMasked,
        CallStatus::Ok,
    ]
    .into_iter()
    .enumerate()
    {
        journal.record(leaky_record(id as u64 + 1, status));
    }
    let mut slow = leaky_record(6, CallStatus::Ok);
    slow.duration = Duration::from_secs(30);
    journal.record(slow);
    journal.record_tool_changes(
        &env("dev"),
        &mcp("kibana-logs"),
        &[ToolName::new("new_tool").unwrap()],
        &[],
        &[],
    );
    journal.record_panic("src/lib.rs:1:1", &format!("panic avec {SECRET}"));

    let entries = journal.snapshot(&JournalFilter::default());
    assert_eq!(entries.len(), 8);
    assert!(entries.iter().filter(|e| e.details.is_some()).count() >= 5);
    assert_no_variant(&f.store, "buffer", entries_text(&journal).as_bytes());
    let mut sent = Vec::new();
    while let Ok(entry) = events.try_recv() {
        sent.push(entry);
    }
    assert_eq!(sent.len(), 8);
    assert_no_variant(&f.store, "subscription", format!("{sent:?}").as_bytes());
    // The content is there, masked.
    let error = &entries[0];
    let details = error.details.as_ref().unwrap();
    assert!(
        details.request.to_string().contains(SECRET_MARKER),
        "{}",
        details.request
    );
    assert!(details.response.to_string().contains(SECRET_MARKER));

    journal.flush().unwrap();
    let file = read_logs(&logs);
    assert_eq!(file.lines().count(), 8, "{file}");
    assert!(file.contains(SECRET_MARKER), "{file}");
    assert_no_variant(&f.store, "log file", file.as_bytes());

    let diagnostics = diagnostics(&f, &logs, Some(log));
    let archive = diagnostics.export(dir.path(), &DiagnosticState::default()).unwrap();
    for (name, content) in unzip(&archive) {
        assert_no_variant(&f.store, &name, &content);
    }
    drop(guard);
}

// --- Content ----------------------------------------------------------------------------------------------------

// Exigence : EX-009-03
#[test]
fn a_normal_call_writes_one_line_without_content() {
    let f = fixture();
    let dir = TempDir::new("normal");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log));
    journal.record(record(1, CallStatus::Ok));
    journal.flush().unwrap();

    let file = read_logs(dir.path());
    assert_eq!(file.lines().count(), 1, "{file}");
    let line = file.lines().next().unwrap();
    assert!(
        line.starts_with("2026-11-22T14:03:07.123Z INFO  mcp.call call_id=1 status=ok"),
        "{line}"
    );
    for field in [
        "slow=false",
        "duration_ms=120",
        "client=\"claude-ai\"",
        "env=dev",
        "mcp=kibana-logs",
        "tool=\"dev_kibana_search_logs\"",
        "session=sso-dev",
        "policy=allow",
    ] {
        assert!(line.contains(field), "{field}: {line}");
    }
    for content in [
        "visible-request-word",
        "visible-response-word",
        "request=",
        "response=",
        "profil",
    ] {
        assert!(!line.contains(content), "{content}: {line}");
    }
    let entry = journal.snapshot(&JournalFilter::default()).pop().unwrap();
    assert!(entry.details.is_none());
    assert_eq!(entry.summary, "claude-ai → dev_kibana_search_logs : réussi, 120 ms");
}

// Exigence : EX-009-03
#[test]
fn an_erroneous_blocked_refused_or_slow_call_writes_its_filtered_content() {
    let f = fixture();
    let dir = TempDir::new("content");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log));
    let mut slow = leaky_record(4, CallStatus::Ok);
    slow.duration = Duration::from_secs(6);
    for r in [
        leaky_record(1, CallStatus::Error),
        leaky_record(2, CallStatus::Blocked),
        leaky_record(3, CallStatus::Refused),
        slow,
    ] {
        let mut r = r;
        r.request["word"] = json!("visible-request-word");
        journal.record(r);
    }
    journal.flush().unwrap();
    let file = read_logs(dir.path());
    assert_eq!(file.lines().count(), 4, "{file}");
    for line in file.lines() {
        assert!(line.contains(" WARN  mcp.call "), "{line}");
        assert!(line.contains("request={"), "{line}");
        assert!(line.contains("visible-request-word"), "{line}");
        assert!(line.contains("response={"), "{line}");
        assert!(line.contains(SECRET_MARKER), "{line}");
        assert!(line.contains("explanation=\"explication "), "{line}");
        assert!(!line.contains(SECRET), "{line}");
    }
    assert!(file.contains("status=ok slow=true duration_ms=6000"), "{file}");
    for entry in journal.snapshot(&JournalFilter::default()) {
        let details = entry.details.expect("details kept");
        assert!(details.request.to_string().contains("visible-request-word"));
        assert!(!details.request.to_string().contains(SECRET));
    }
}

// Exigence : EX-009-03
#[test]
fn content_is_kept_for_every_status_but_the_normal_ones() {
    use CallStatus::*;
    for status in [Ok, InputRequired, Suspended] {
        assert!(!keeps_content(status, false), "{status:?}");
        assert!(keeps_content(status, true), "{status:?} slow");
    }
    for status in [
        LeakMasked,
        Error,
        Denied,
        Refused,
        Timeout,
        Blocked,
        RateLimited,
        AuthRequired,
        Cancelled,
    ] {
        assert!(keeps_content(status, false), "{status:?}");
    }
}

// Exigence : EX-009-03
#[test]
fn a_long_content_is_cut_in_the_file() {
    let f = fixture();
    let dir = TempDir::new("long");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log));
    let mut r = record(1, CallStatus::Error);
    r.response = json!({ "text": "é".repeat(MAX_FILE_CONTENT_BYTES) });
    journal.record(r);
    journal.flush().unwrap();
    let file = read_logs(dir.path());
    assert!(file.len() < 2 * MAX_FILE_CONTENT_BYTES, "{}", file.len());
    assert!(file.contains("…[tronqué]"));
}

// --- Slow calls -------------------------------------------------------------------------------------------------

// Exigence : EX-009-04
#[test]
fn the_slow_call_threshold_is_configurable() {
    let f = fixture();
    let at = |journal: &Journal, millis: u64| {
        let mut r = record(1, CallStatus::Ok);
        r.duration = Duration::from_millis(millis);
        journal.record(r);
        journal.snapshot(&JournalFilter::default()).pop().unwrap()
    };
    let default = journal(
        &f,
        JournalConfig::from_config(&Config::embedded(crate::config::TEST_MODULES).unwrap()),
        None,
    );
    assert_eq!(
        JournalConfig::from_config(&Config::embedded(crate::config::TEST_MODULES).unwrap()).slow_threshold,
        Duration::from_secs(5)
    );
    assert!(!at(&default, 4_999).slow);
    let slow = at(&default, 5_000);
    assert!(slow.slow);
    assert!(slow.details.is_some(), "a slow call keeps its content");
    assert!(slow.summary.contains("(lent)"), "{}", slow.summary);

    let two = journal(&f, config(100, 1 << 20, 2), None);
    assert!(!at(&two, 1_999).slow);
    assert!(at(&two, 2_000).slow);
    assert!(at(&two, 4_000).is_problem());
}

// --- Rotation and retention -------------------------------------------------------------------------------------

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// Exigence : EX-009-05
#[test]
fn log_files_rotate_daily_and_are_kept_retention_days() {
    let f = fixture();
    let dir = TempDir::new("rotation");
    fs::write(dir.path().join("doctor-20200101-0000.txt"), "rapport").unwrap();
    // Older than the retention when the log opens.
    fs::write(dir.path().join("mcp-proxy-2026-11-10.log"), "ancien\n").unwrap();
    let (log, _guard) = open(&f, dir.path(), 3, DEFAULT_MAX_FILE_BYTES);
    log.write_record("jour 1\n");
    log.flush().unwrap();
    assert_eq!(
        names(dir.path()),
        ["doctor-20200101-0000.txt", "mcp-proxy-2026-11-22.log"]
    );

    f.clock.advance(DAY);
    log.write_record("jour 2\n");
    f.clock.advance(DAY);
    log.write_record("jour 3\n");
    log.flush().unwrap();
    assert_eq!(
        names(dir.path()),
        [
            "doctor-20200101-0000.txt",
            "mcp-proxy-2026-11-22.log",
            "mcp-proxy-2026-11-23.log",
            "mcp-proxy-2026-11-24.log"
        ]
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("mcp-proxy-2026-11-23.log")).unwrap(),
        "jour 2\n"
    );

    // Day 4: the first day leaves, other files stay.
    f.clock.advance(DAY);
    log.write_record("jour 4\n");
    log.flush().unwrap();
    assert_eq!(
        names(dir.path()),
        [
            "doctor-20200101-0000.txt",
            "mcp-proxy-2026-11-23.log",
            "mcp-proxy-2026-11-24.log",
            "mcp-proxy-2026-11-25.log"
        ]
    );
    // A week later, only today's file is left.
    f.clock.advance(7 * DAY);
    log.write_record("plus tard\n");
    log.flush().unwrap();
    assert_eq!(
        names(dir.path()),
        ["doctor-20200101-0000.txt", "mcp-proxy-2026-12-02.log"]
    );
}

// Exigence : EX-009-05
#[test]
fn a_full_day_file_stops_with_a_notice_until_the_next_day() {
    let f = fixture();
    let dir = TempDir::new("full");
    let (log, _guard) = open(&f, dir.path(), 7, 100);
    for i in 0..20 {
        log.write_record(&format!("ligne {i:02} ..........\n"));
    }
    log.flush().unwrap();
    let today = fs::read_to_string(dir.path().join(log_file_name(now()))).unwrap();
    assert!(today.ends_with(FILE_FULL_NOTICE), "{today}");
    assert!(today.len() <= 100 + FILE_FULL_NOTICE.len(), "{}", today.len());
    assert_eq!(today.matches(FILE_FULL_NOTICE).count(), 1);

    f.clock.advance(DAY);
    log.write_record("lendemain\n");
    log.flush().unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join(log_file_name(now() + DAY))).unwrap(),
        "lendemain\n"
    );
}

// --- Journal for the interface ----------------------------------------------------------------------------------

fn statuses(entries: &[JournalEntry]) -> Vec<Option<CallStatus>> {
    entries.iter().map(|e| e.status).collect()
}

// Exigence : EX-009-06
#[test]
fn subscribers_receive_new_entries() {
    let f = fixture();
    let journal = journal(&f, config(100, 1 << 20, 5), None);
    journal.record(record(1, CallStatus::Ok));
    let mut events = journal.subscribe();
    let mut other = journal.subscribe();
    journal.record(record(2, CallStatus::Denied));
    journal.record_tool_changes(
        &env("dev"),
        &mcp("kibana-logs"),
        &[],
        &[],
        &[ToolName::new("old").unwrap()],
    );
    let first = events.try_recv().unwrap();
    assert_eq!((first.seq, first.status), (2, Some(CallStatus::Denied)));
    let second = events.try_recv().unwrap();
    assert_eq!(second.kind, EntryKind::ToolsChanged);
    assert!(events.try_recv().is_err(), "only new entries");
    assert_eq!(other.try_recv().unwrap(), first);
    assert_eq!(journal.snapshot(&JournalFilter::default()).len(), 3);
}

// Exigence : EX-009-06
#[test]
fn filters_select_by_text_environment_and_status() {
    use CallStatus::*;
    let f = fixture();
    let journal = journal(&f, config(100, 1 << 20, 5), None);
    let all = [
        Ok,
        LeakMasked,
        Error,
        Denied,
        Refused,
        Timeout,
        Blocked,
        RateLimited,
        AuthRequired,
        Suspended,
        InputRequired,
        Cancelled,
    ];
    for (i, status) in all.into_iter().enumerate() {
        journal.record(record(i as u64 + 1, status));
    }
    let mut confirmed = record(20, Ok);
    confirmed.policy = Some(PolicyRecord {
        decision: Decision::Confirm(Channel::Notification),
        explanation: String::new(),
    });
    journal.record(confirmed);
    let mut slow = record(21, Ok);
    slow.duration = Duration::from_secs(9);
    journal.record(slow);
    let mut prod = record(22, Ok);
    prod.env = Some(env("prod"));
    prod.tool = "prod_kibana_search_logs".to_owned();
    prod.client = "Visual Studio Code".to_owned();
    journal.record(prod);
    journal.record_tool_changes(
        &env("prod"),
        &mcp("kibana-logs"),
        &[ToolName::new("delai").unwrap()],
        &[],
        &[],
    );
    journal.record_panic("src/main.rs:3:5", "boum");

    let select = |filter: JournalFilter| journal.snapshot(&filter);
    assert_eq!(select(JournalFilter::default()).len(), 17);

    let problems = select(JournalFilter {
        status: StatusFilter::Problems,
        ..Default::default()
    });
    assert_eq!(
        statuses(&problems),
        [
            Some(LeakMasked),
            Some(Error),
            Some(Timeout),
            Some(Blocked),
            Some(RateLimited),
            Some(AuthRequired),
            Some(Cancelled),
            Some(Ok), // slow
            None,     // panic
        ]
    );
    let policies = select(JournalFilter {
        status: StatusFilter::Policies,
        ..Default::default()
    });
    assert_eq!(
        statuses(&policies),
        [
            Some(Denied),
            Some(Refused),
            Some(Timeout),
            Some(Suspended),
            Some(InputRequired),
            Some(Ok), // confirmation accepted
        ]
    );

    let prod_only = select(JournalFilter {
        env: Some(env("prod")),
        ..Default::default()
    });
    assert_eq!(prod_only.len(), 2);
    assert!(prod_only.iter().all(|e| e.env == Some(env("prod"))));

    let text = |t: &str| {
        select(JournalFilter {
            text: Some(t.to_owned()),
            ..Default::default()
        })
    };
    assert_eq!(text("visual STUDIO").len(), 1, "client, case ignored");
    assert_eq!(text("REUSSI").len(), 4, "status label, accents ignored");
    assert_eq!(text("délai dépassé").len(), 1);
    assert_eq!(text("rate_limited").len(), 1, "status key");
    assert_eq!(text("prod_kibana").len(), 1, "tool");
    assert_eq!(text("kibana-logs").len(), 16, "MCP: every call and the list change");
    assert_eq!(text("liste d'outils").len(), 1);
    assert_eq!(text("  ").len(), 17, "a blank text selects everything");
    assert_eq!(text("visible-request-word").len(), 0, "content is not searched");

    let combined = select(JournalFilter {
        text: Some("kibana".to_owned()),
        env: Some(env("dev")),
        status: StatusFilter::Problems,
    });
    assert_eq!(combined.len(), 8);
}

// --- Archive ----------------------------------------------------------------------------------------------------

fn diagnostics(f: &Fixture, logs: &Path, log: Option<FileLog>) -> Diagnostics {
    Diagnostics {
        version: "0.1.0".to_owned(),
        system: "Windows 11 (test)".to_owned(),
        config: Arc::new(Config::embedded(crate::config::TEST_MODULES).unwrap()),
        log_dir: logs.to_path_buf(),
        filter: Arc::clone(&f.filter),
        clock: Arc::new(f.clock.clone()),
        log,
    }
}

fn unzip(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let mut content = Vec::new();
        file.read_to_end(&mut content).unwrap();
        files.push((file.name().unwrap().into_owned(), content));
    }
    files
}

// Exigence : EX-009-07
#[test]
fn the_archive_holds_the_expected_files_and_no_secret() {
    let f = fixture();
    let dir = TempDir::new("archive");
    let logs = dir.path().join("logs");
    let (log, _guard) = open(&f, &logs, 7, DEFAULT_MAX_FILE_BYTES);
    log.write_record("jour 1\n");
    f.clock.advance(DAY);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log.clone()));
    journal.record(leaky_record(1, CallStatus::Error));
    fs::write(
        logs.join("doctor-20261123-1000.txt"),
        format!("rapport ancien {SECRET}"),
    )
    .unwrap();
    fs::write(
        logs.join("doctor-20261123-1200.txt"),
        format!("rapport récent {PASSWORD}"),
    )
    .unwrap();
    fs::write(logs.join("autre.txt"), "pas dans l'archive").unwrap();
    let out = dir.path().join("exports");
    fs::create_dir_all(&out).unwrap();

    let settings = crate::settings::UserSettings {
        gateway_suspended: true,
        ..Default::default()
    };
    let state = DiagnosticState {
        settings,
        instances: vec![
            "dev/kibana-logs : connecté".to_owned(),
            format!("prod/kibana-logs : injoignable ({SECRET})"),
        ],
        doctor_report: None,
    };
    let diagnostics = diagnostics(&f, &logs, Some(log));
    let path = diagnostics.export(&out, &state).unwrap();
    assert_eq!(path, out.join("mcp-proxy-diagnostic-20261123-1403.zip"));
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), archive_name(f.clock.now()));
    assert_eq!(
        names(&out),
        ["mcp-proxy-diagnostic-20261123-1403.zip"],
        "no partial file left"
    );

    let files = unzip(&path);
    let listed: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        listed,
        [
            "LISEZMOI.txt",
            "version.txt",
            "systeme.txt",
            "config.txt",
            "reglages.json",
            "instances.txt",
            "auto-diagnostic.txt",
            "logs/mcp-proxy-2026-11-22.log",
            "logs/mcp-proxy-2026-11-23.log",
        ]
    );
    let text = |name: &str| String::from_utf8(files.iter().find(|(n, _)| n == name).unwrap().1.clone()).unwrap();
    assert_eq!(text("version.txt"), "MCP Proxy 0.1.0\n");
    assert!(text("systeme.txt").contains("Windows 11 (test)"));
    let config = text("config.txt");
    assert!(config.contains("kibana-logs"), "{config}");
    assert!(config.contains(".example.internal"), "{config}");
    assert!(!config.contains("https://"), "hosts only: {config}");
    let settings: Value = serde_json::from_str(&text("reglages.json")).unwrap();
    assert_eq!(settings["gateway_suspended"], json!(true));
    assert!(text("instances.txt").contains("dev/kibana-logs : connecté"));
    assert!(text("auto-diagnostic.txt").starts_with("rapport récent"));
    assert_eq!(text("logs/mcp-proxy-2026-11-22.log"), "jour 1\n");
    assert!(text("logs/mcp-proxy-2026-11-23.log").contains("status=error"));
    for (name, content) in &files {
        assert_no_variant(&f.store, name, content);
    }

    // Exact path chosen by the user.
    let chosen = out.join("choisi.zip");
    assert_eq!(diagnostics.export(&chosen, &state).unwrap(), chosen);
    assert!(chosen.is_file());
}

// Exigence : EX-009-13
#[test]
fn the_export_masks_a_secret_written_before_it_was_known() {
    let f = fixture();
    let dir = TempDir::new("late");
    let logs = dir.path().join("logs");
    let (log, _guard) = open(&f, &logs, 7, DEFAULT_MAX_FILE_BYTES);
    let late = "jeton-arrive-plus-tard-51e0a7c3";
    log.write_record(&format!("ligne écrite avant la connexion : {late}\n"));
    log.flush().unwrap();
    assert!(read_logs(&logs).contains(late), "not known yet: written as is");

    f.store
        .put(
            &sid("sso-prod"),
            Credential::Bearer {
                token: SecretString::from(late.to_owned()),
            },
        )
        .unwrap();
    let path = diagnostics(&f, &logs, Some(log))
        .export(dir.path(), &DiagnosticState::default())
        .unwrap();
    let files = unzip(&path);
    let (_, content) = files.iter().find(|(n, _)| n.starts_with("logs/")).unwrap();
    let content = String::from_utf8_lossy(content);
    assert!(!content.contains(late), "{content}");
    assert!(content.contains(&format!("connexion : {SECRET_MARKER}")), "{content}");
    for (name, content) in &files {
        assert_no_variant(&f.store, name, content);
    }
}

// Exigence : EX-009-13
#[test]
fn no_archive_is_written_without_the_filter() {
    let f = fixture();
    let dir = TempDir::new("nofilter");
    let logs = dir.path().join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join(log_file_name(now())), "ligne\n").unwrap();
    f.wrapper.break_now();
    let error = diagnostics(&f, &logs, None)
        .export(dir.path(), &DiagnosticState::default())
        .unwrap_err();
    assert!(matches!(error, crate::diagnostics::DiagnosticError::FilterUnavailable));
    assert_eq!(names(dir.path()), ["logs"], "nothing written");
}

// --- tracing lines ----------------------------------------------------------------------------------------------

// Exigence : EX-009-08
#[test]
fn a_tracing_error_with_a_secret_is_masked_in_the_file() {
    let f = fixture();
    let dir = TempDir::new("tracing");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let subscriber = file_subscriber(log.clone(), default_targets());
    tracing::subscriber::with_default(subscriber, || {
        tracing::error!(token = %SECRET, "échec amont avec {SECRET} et Bearer {SECRET}");
        // A library event, at its own level.
        tracing::warn!(target: "reqwest::connect", "connexion refusée : {PASSWORD}");
        tracing::info!(target: "reqwest::connect", "ignoré : niveau des bibliothèques");
        tracing::info!(target: "mcp_proxy_gateway::upstream", "info de la passerelle");
    });
    log.flush().unwrap();
    let file = read_logs(dir.path());
    assert_eq!(file.lines().count(), 3, "{file}");
    let first = file.lines().next().unwrap();
    assert!(first.starts_with("2026-11-22T14:03:07.123Z ERROR"), "{first}");
    assert!(first.contains(&format!("échec amont avec {SECRET_MARKER}")), "{first}");
    assert!(
        file.contains("WARN reqwest::connect: connexion refusée : [SECRET MASQUÉ]"),
        "{file}"
    );
    assert!(file.contains("info de la passerelle"));
    assert_no_variant(&f.store, "log file", file.as_bytes());
}

// Exigence : EX-009-12
#[test]
fn without_the_filter_the_fixed_line_is_written_instead() {
    let f = fixture();
    let dir = TempDir::new("broken");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log.clone()));
    f.wrapper.break_now();
    tracing::subscriber::with_default(file_subscriber(log.clone(), default_targets()), || {
        tracing::error!("jeton {SECRET}");
    });
    journal.record(leaky_record(1, CallStatus::Error));
    log.flush().unwrap();
    let file = read_logs(dir.path());
    assert_eq!(file, UNFILTERED_RECORD.repeat(2));
    let entry = journal.snapshot(&JournalFilter::default()).pop().unwrap();
    assert_eq!(entry.client.as_deref(), Some(SECRET_MARKER));
    let details = entry.details.unwrap();
    assert_eq!(details.request, json!(SECRET_MARKER));
    assert_eq!(details.response, json!(SECRET_MARKER));
    assert!(!format!("{:?}", journal.snapshot(&JournalFilter::default())).contains(SECRET));
}

// --- Tool lists and panics --------------------------------------------------------------------------------------

// Exigence : EX-009-10
#[test]
fn a_tool_list_change_is_journaled() {
    let f = fixture();
    let dir = TempDir::new("tools");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log));
    let names = |list: &[&str]| list.iter().map(|n| ToolName::new(*n).unwrap()).collect::<Vec<_>>();
    journal.record_tool_changes(
        &env("dev"),
        &mcp("kibana-logs"),
        &names(&["a", "b"]),
        &names(&["c"]),
        &names(&["d"]),
    );
    journal.record_tool_changes(&env("dev"), &mcp("kibana-logs"), &[], &[], &[]);
    journal.flush().unwrap();
    let entries = journal.snapshot(&JournalFilter::default());
    assert_eq!(entries.len(), 1, "an empty change is not journaled");
    let entry = &entries[0];
    assert_eq!(entry.kind, EntryKind::ToolsChanged);
    assert_eq!(
        entry.summary,
        "dev / kibana-logs : liste d'outils changée, 2 nouveaux (a, b) ; 1 modifié (c) ; 1 supprimé (d)"
    );
    assert_eq!(
        (entry.env.clone(), entry.mcp.clone()),
        (Some(env("dev")), Some(mcp("kibana-logs")))
    );
    let file = read_logs(dir.path());
    assert!(
        file.contains("INFO  mcp.tools env=dev mcp=kibana-logs added=2 modified=1 removed=1"),
        "{file}"
    );
}

// Exigence : EX-009-11
#[test]
fn a_panic_is_journaled_with_its_location_and_filtered_message() {
    let f = fixture();
    let dir = TempDir::new("panic");
    let (log, _guard) = open(&f, dir.path(), 7, DEFAULT_MAX_FILE_BYTES);
    let journal = journal(&f, config(100, 1 << 20, 5), Some(log));
    let location = std::panic::Location::caller();
    let payload: Box<dyn std::any::Any + Send> = Box::new(format!("échec : {SECRET}"));
    let (place, message) = panic::describe(Some(location), payload.as_ref());
    assert!(place.starts_with(&format!("{}:", file!())), "{place}");
    journal.record_panic(&place, &message);
    journal.flush().unwrap();
    let entry = journal.snapshot(&JournalFilter::default()).pop().unwrap();
    assert_eq!(entry.kind, EntryKind::Panic);
    assert!(entry.is_problem());
    assert_eq!(entry.summary, format!("panic à {place} : échec : {SECRET_MARKER}"));
    let file = read_logs(dir.path());
    assert!(
        file.contains(&format!(
            " ERROR panic location={}",
            serde_json::to_string(&place).unwrap()
        )),
        "{file}"
    );
    assert_no_variant(&f.store, "log file", file.as_bytes());

    let other: Box<dyn std::any::Any + Send> = Box::new(42_u32);
    assert_eq!(panic::describe(None, other.as_ref()).1, "(message non textuel)");
    let long: Box<dyn std::any::Any + Send> = Box::new("x".repeat(5000));
    assert_eq!(panic::describe(None, long.as_ref()).1.chars().count(), 1001);
}

// --- Keys -------------------------------------------------------------------------------------------------------

#[test]
fn decision_keys_are_distinct() {
    let keys = [
        decision_key(Decision::Allow),
        decision_key(Decision::Confirm(Channel::Elicitation)),
        decision_key(Decision::Confirm(Channel::Notification)),
        decision_key(Decision::Deny),
    ];
    let unique: std::collections::BTreeSet<_> = keys.iter().collect();
    assert_eq!(unique.len(), 4);
}
