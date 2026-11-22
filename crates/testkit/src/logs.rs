//! What a test logged on its own thread.
//!
//! `tracing` caches, per call site and for the whole process, whether anyone listens. While a single dispatcher is
//! registered, it fills that cache from the default of the thread that reaches the call site first. In a test binary
//! that thread is often another test's, which has no subscriber: the call site is switched off for everyone, and the
//! test that listens misses the line. With more than one dispatcher registered, `tracing` asks all of them, whatever
//! the thread: a witness dispatcher, kept for the life of the process, makes that the only case.

use std::io;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use tracing::subscriber::{DefaultGuard, NoSubscriber};
use tracing::{Dispatch, Subscriber};
use tracing_subscriber::fmt::MakeWriter;

/// Makes `subscriber` the default of the current thread until the guard is dropped, and reliably so when other tests
/// of the same binary log from their own threads.
pub fn set_thread_subscriber<S>(subscriber: S) -> DefaultGuard
where
    S: Subscriber + Send + Sync + 'static,
{
    static WITNESS: OnceLock<Dispatch> = OnceLock::new();
    // Registered before the subscriber: registering the subscriber then refreshes every call site already cached.
    WITNESS.get_or_init(|| Dispatch::new(NoSubscriber::default()));
    #[allow(clippy::disallowed_methods)]
    tracing::subscriber::set_default(subscriber)
}

/// Every line logged on the current thread, down to `TRACE`, while the guard of [`CapturedLogs::start`] lives.
#[derive(Clone, Default)]
pub struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    /// Starts collecting. On a current-thread runtime, the tasks the test spawns are collected too.
    pub fn start() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        let guard = set_thread_subscriber(subscriber);
        (logs, guard)
    }

    /// What was logged so far.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
