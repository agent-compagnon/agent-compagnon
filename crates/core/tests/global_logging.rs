//! The process-wide pieces: global subscriber, `log` records of the libraries, panic hook. Alone in its test binary,
//! hence its own process: nothing global leaks into the other tests.

#![cfg(not(windows))]
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use mcp_proxy_core::config::{Config, FilterConfig, SessionId};
use mcp_proxy_core::filter::{OutputFilter, SECRET_MARKER};
use mcp_proxy_core::secrets::{Credential, InsecureDevKeyWrapper, SecretStore};
use mcp_proxy_core::trace::{
    EntryKind, Journal, JournalConfig, JournalFilter, LogFileOptions, ManualWallClock, install_global,
    install_panic_hook, log_files, open_log,
};
use secrecy::SecretString;

const SECRET: &str = "S3NT1NEL-global-0b7e4d1a";

// Exigence : EX-009-11, EX-009-12
#[test]
fn library_records_and_panics_reach_the_file_filtered() {
    let dir = std::env::temp_dir().join(format!("mcp-proxy-global-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let store = Arc::new(SecretStore::new(Box::new(InsecureDevKeyWrapper::default())).unwrap());
    store
        .put(
            &SessionId::new("sso-dev").unwrap(),
            Credential::Bearer {
                token: SecretString::from(SECRET.to_owned()),
            },
        )
        .unwrap();
    let filter = Arc::new(OutputFilter::new(store, &FilterConfig::default()).unwrap());
    let config = Config::embedded().unwrap();
    let clock = Arc::new(ManualWallClock::new(UNIX_EPOCH + Duration::from_secs(1_795_356_187)));
    let (log, guard) = open_log(
        LogFileOptions::new(&dir, &config.trace),
        clock.clone(),
        Arc::clone(&filter),
    )
    .unwrap();
    install_global(log.clone()).unwrap();
    assert!(install_global(log.clone()).is_err(), "once per process");
    let journal = Journal::new(JournalConfig::from_config(&config), filter, clock, Some(log.clone()));
    install_panic_hook(journal.clone());

    // A library that logs through the `log` crate (reqwest, native-tls).
    tracing_log::log::warn!(target: "native_tls", "poignée de main refusée, jeton {SECRET}");
    tracing_log::log::debug!(target: "native_tls", "trop détaillé : non écrit");
    tracing::warn!(target: "rmcp::service", "Bearer {SECRET}");

    let request = format!("{{\"token\":\"{SECRET}\"}}");
    let outcome = std::panic::catch_unwind(|| {
        let _request = &request;
        panic!("échec en traitant {SECRET}");
    });
    assert!(outcome.is_err());
    // The default hook again, before the assertions can panic.
    drop(std::panic::take_hook());

    journal.flush().unwrap();
    let mut text = String::new();
    for file in log_files(&dir).unwrap() {
        text.push_str(&fs::read_to_string(file).unwrap());
    }
    assert!(!text.contains(SECRET), "{text}");
    assert!(
        text.contains(&format!(
            "WARN native_tls: poignée de main refusée, jeton {SECRET_MARKER}"
        )),
        "{text}"
    );
    assert!(!text.contains("trop détaillé"), "{text}");
    assert!(text.contains("WARN rmcp::service: Bearer "), "{text}");
    let panic_line = text.lines().find(|l| l.contains(" ERROR panic ")).unwrap();
    assert!(
        panic_line.contains("location=\"crates/core/tests/global_logging.rs:"),
        "{panic_line}"
    );
    assert!(
        panic_line.contains(&format!("échec en traitant {SECRET_MARKER}")),
        "{panic_line}"
    );
    assert!(!panic_line.contains("token"), "no request data: {panic_line}");

    let panics: Vec<_> = journal
        .snapshot(&JournalFilter::default())
        .into_iter()
        .filter(|e| e.kind == EntryKind::Panic)
        .collect();
    assert_eq!(panics.len(), 1);
    assert!(panics[0].summary.contains(SECRET_MARKER), "{}", panics[0].summary);

    drop(guard);
    let _ = fs::remove_dir_all(&dir);
}
