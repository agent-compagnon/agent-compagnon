//! The log file: one file per UTC day in an injected directory, kept `log_retention_days` days, each bounded in
//! size.
//!
//! Nothing reaches the disk without the output filter: the only writer handed out, [`FileLog`], is a
//! `FilteringMakeWriter` over the file. Writing never blocks the caller: complete lines go through a bounded queue
//! to a dedicated thread, and a line that does not fit in the queue is dropped and counted.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use zeroize::Zeroizing;

use super::clock::WallClock;
use super::time;
use crate::config::TraceConfig;
use crate::filter::{FilteringMakeWriter, FilteringWriter, OutputFilter};

pub const LOG_FILE_PREFIX: &str = "mcp-proxy-";
pub const LOG_FILE_SUFFIX: &str = ".log";
/// Largest size of one day's file. With the retention, it bounds the whole directory.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Lines waiting for the writer thread. Beyond that, lines are dropped rather than blocking a call.
pub const DEFAULT_QUEUE_LINES: usize = 4096;
/// Written once when a day's file reaches its size bound.
pub const FILE_FULL_NOTICE: &str = "[journal du jour plein : les lignes suivantes ne sont pas écrites avant demain]\n";

/// How long [`FileLog::flush`] waits for the writer thread.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
const LINE_CAPACITY: usize = 512;

/// `mcp-proxy-2026-11-22.log` for the UTC day of `at`.
pub fn log_file_name(at: SystemTime) -> String {
    format!("{LOG_FILE_PREFIX}{}{LOG_FILE_SUFFIX}", time::date(time::day_number(at)))
}

/// Day number of a log file name, `None` for any other file.
pub(crate) fn log_file_day(name: &str) -> Option<i64> {
    time::parse_date(name.strip_prefix(LOG_FILE_PREFIX)?.strip_suffix(LOG_FILE_SUFFIX)?)
}

/// Log files of `dir`, oldest first.
pub fn log_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files: Vec<(i64, PathBuf)> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let day = log_file_day(entry.file_name().to_str()?)?;
            entry.file_type().ok()?.is_file().then(|| (day, entry.path()))
        })
        .collect();
    files.sort();
    Ok(files.into_iter().map(|(_, path)| path).collect())
}

fn dropped_notice(lines: u64) -> String {
    format!("[{lines} ligne(s) de journal perdue(s) : file d'écriture pleine]\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFileOptions {
    /// `%LOCALAPPDATA%\MCP Proxy\logs\` in the application. Nothing is written elsewhere.
    pub dir: PathBuf,
    /// Days kept, today included.
    pub retention_days: u32,
    pub max_file_bytes: u64,
    pub queue_lines: usize,
}

impl LogFileOptions {
    pub fn new(dir: impl Into<PathBuf>, trace: &TraceConfig) -> Self {
        Self {
            dir: dir.into(),
            retention_days: trace.log_retention_days.max(1),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            queue_lines: DEFAULT_QUEUE_LINES,
        }
    }
}

enum Message {
    Line { at: SystemTime, bytes: Zeroizing<Vec<u8>> },
    Flush(SyncSender<()>),
    Stop,
}

/// The unfiltered side of the file: only reachable through the `FilteringMakeWriter` of [`FileLog`].
#[derive(Clone)]
pub struct RawLog {
    sender: SyncSender<Message>,
    clock: Arc<dyn WallClock>,
    dropped: Arc<AtomicU64>,
}

impl RawLog {
    fn send(&self, at: SystemTime, bytes: Zeroizing<Vec<u8>>) {
        if let Err(TrySendError::Full(_)) = self.sender.try_send(Message::Line { at, bytes }) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl<'a> MakeWriter<'a> for RawLog {
    type Writer = LineSender<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LineSender {
            log: self,
            at: self.clock.now(),
            buf: Zeroizing::new(Vec::with_capacity(LINE_CAPACITY)),
        }
    }
}

/// Collects one filtered record and queues it when dropped or flushed. The time of the record, which decides its
/// file, is taken when the record starts.
pub struct LineSender<'a> {
    log: &'a RawLog,
    at: SystemTime,
    buf: Zeroizing<Vec<u8>>,
}

impl LineSender<'_> {
    fn send(&mut self) {
        if !self.buf.is_empty() {
            let bytes = std::mem::replace(&mut self.buf, Zeroizing::new(Vec::new()));
            self.log.send(self.at, bytes);
        }
    }
}

impl Write for LineSender<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send();
        Ok(())
    }
}

impl Drop for LineSender<'_> {
    fn drop(&mut self) {
        self.send();
    }
}

struct Shared {
    filtering: FilteringMakeWriter<RawLog>,
    raw: RawLog,
}

/// Where log lines go: through the output filter, then to the file. Cheap to clone.
#[derive(Clone)]
pub struct FileLog {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for FileLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLog").finish_non_exhaustive()
    }
}

impl<'a> MakeWriter<'a> for FileLog {
    type Writer = FilteringWriter<'a, LineSender<'a>>;

    fn make_writer(&'a self) -> Self::Writer {
        self.shared.filtering.make_writer()
    }
}

impl FileLog {
    /// Writes one record (ending with a newline), filtered.
    pub fn write_record(&self, record: &str) {
        let mut writer = self.make_writer();
        // A failed write only loses the record: nothing to report it to.
        let _ = writer.write_all(record.as_bytes());
    }

    /// Waits until every line queued so far is written, for a few seconds at most. Never called on a call's task.
    pub fn flush(&self) -> io::Result<()> {
        let (reply, done) = mpsc::sync_channel(1);
        self.shared
            .raw
            .sender
            .send(Message::Flush(reply))
            .map_err(|_| io::Error::other("journal fichier arrêté"))?;
        match done.recv_timeout(FLUSH_TIMEOUT) {
            Ok(()) => Ok(()),
            Err(RecvTimeoutError::Timeout) => Err(io::Error::new(io::ErrorKind::TimedOut, "journal fichier lent")),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::other("journal fichier arrêté")),
        }
    }

    /// Lines dropped because the queue was full, not yet reported in the file.
    pub fn dropped(&self) -> u64 {
        self.shared.raw.dropped.load(Ordering::Relaxed)
    }

    pub fn clock(&self) -> &Arc<dyn WallClock> {
        &self.shared.raw.clock
    }
}

/// Stops the writer thread when dropped, after the lines already queued.
pub struct LogGuard {
    sender: SyncSender<Message>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Creates the directory, removes the files past retention and starts the writer thread.
pub fn open_log(
    options: LogFileOptions,
    clock: Arc<dyn WallClock>,
    filter: Arc<OutputFilter>,
) -> io::Result<(FileLog, LogGuard)> {
    fs::create_dir_all(&options.dir)?;
    let (sender, receiver) = mpsc::sync_channel(options.queue_lines.max(1));
    let dropped = Arc::new(AtomicU64::new(0));
    let rolling = Rolling {
        options,
        current: None,
        dropped: Arc::clone(&dropped),
    };
    rolling.purge(time::day_number(clock.now()));
    let thread = std::thread::Builder::new()
        .name("mcp-proxy-log".to_owned())
        .spawn(move || rolling.run(&receiver))?;
    let raw = RawLog {
        sender: sender.clone(),
        clock,
        dropped,
    };
    let log = FileLog {
        shared: Arc::new(Shared {
            filtering: FilteringMakeWriter::new(raw.clone(), filter),
            raw,
        }),
    };
    Ok((
        log,
        LogGuard {
            sender,
            thread: Some(thread),
        },
    ))
}

struct Current {
    day: i64,
    file: File,
    size: u64,
    full: bool,
}

/// State of the writer thread.
struct Rolling {
    options: LogFileOptions,
    current: Option<Current>,
    dropped: Arc<AtomicU64>,
}

impl Rolling {
    fn run(mut self, receiver: &Receiver<Message>) {
        while let Ok(message) = receiver.recv() {
            match message {
                Message::Line { at, bytes } => self.write_line(at, &bytes),
                Message::Flush(reply) => {
                    if let Some(current) = &mut self.current {
                        let _ = current.file.flush();
                    }
                    let _ = reply.send(());
                }
                Message::Stop => return,
            }
        }
    }

    fn write_line(&mut self, at: SystemTime, bytes: &[u8]) {
        let day = time::day_number(at);
        if self.current.as_ref().map(|c| c.day) != Some(day) {
            self.current = self.open(day);
            self.purge(day);
        }
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            self.append(dropped_notice(dropped).as_bytes());
        }
        self.append(bytes);
    }

    fn open(&self, day: i64) -> Option<Current> {
        let path = self
            .options
            .dir
            .join(format!("{LOG_FILE_PREFIX}{}{LOG_FILE_SUFFIX}", time::date(day)));
        // The directory may have been removed while running.
        let _ = fs::create_dir_all(&self.options.dir);
        let file = OpenOptions::new().create(true).append(true).open(path).ok()?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Some(Current {
            day,
            file,
            size,
            full: size >= self.options.max_file_bytes,
        })
    }

    fn append(&mut self, bytes: &[u8]) {
        let max = self.options.max_file_bytes;
        let Some(current) = &mut self.current else {
            return;
        };
        if current.full {
            return;
        }
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if current.size.saturating_add(len) > max {
            current.full = true;
            if current.file.write_all(FILE_FULL_NOTICE.as_bytes()).is_ok() {
                current.size = current.size.saturating_add(FILE_FULL_NOTICE.len() as u64);
            }
            return;
        }
        if current.file.write_all(bytes).is_ok() {
            current.size = current.size.saturating_add(len);
        }
    }

    /// Removes the log files older than the retention, today being `today`. Other files are left alone.
    fn purge(&self, today: i64) {
        let oldest_kept = today - i64::from(self.options.retention_days.max(1)) + 1;
        let Ok(entries) = fs::read_dir(&self.options.dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let Some(day) = name.to_str().and_then(log_file_day) else {
                continue;
            };
            if day < oldest_kept && entry.file_type().is_ok_and(|t| t.is_file()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Timestamps of the `tracing` lines, from the journal's clock, in UTC.
pub struct ClockTime(pub Arc<dyn WallClock>);

impl FormatTime for ClockTime {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        w.write_str(&time::rfc3339(self.0.now()))
    }
}
