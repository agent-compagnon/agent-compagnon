//! `CapturedLogs` against the call-site cache of `tracing`.

#![allow(clippy::unwrap_used, clippy::panic)]

use mcp_proxy_testkit::CapturedLogs;

fn emit() {
    tracing::info!("ligne attendue");
}

/// Fails with a bare `tracing::subscriber::set_default`: the other thread, which has no subscriber, reaches the call
/// site first and switches it off for the whole process.
#[test]
fn a_line_is_captured_even_when_another_thread_reached_its_call_site_first() {
    let (logs, _guard) = CapturedLogs::start();
    std::thread::spawn(emit).join().unwrap();
    assert_eq!(logs.text(), "", "the line of another thread is not this test's");

    emit();
    let text = logs.text();
    assert!(text.contains("INFO") && text.contains("ligne attendue"), "{text}");
}

#[test]
fn nothing_is_captured_once_the_guard_is_dropped() {
    let (logs, guard) = CapturedLogs::start();
    tracing::warn!("pendant");
    drop(guard);
    tracing::warn!("après");
    let text = logs.text();
    assert!(text.contains("pendant") && !text.contains("après"), "{text}");
}
