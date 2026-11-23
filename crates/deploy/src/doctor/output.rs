//! Where the report goes: always a new file of the log folder, whose path ends the report, and the standard output
//! (visible when it is redirected: the exe has no console, D18). The file is the only thing the diagnosis writes.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::report::{Report, Saved};

/// `doctor-AAAAMMJJ-HHMMSS.txt` (UTC), in the log folder; the diagnostic export takes the latest when it is given
/// none (spec 009).
pub const REPORT_PREFIX: &str = "doctor-";
/// Same second: `doctor-…_2.txt` up to `_9`, which sort after the first.
const ATTEMPTS: usize = 9;

/// The `n`-th name tried for a report of `stamp` (`AAAAMMJJ-HHMMSS`), from 1.
pub fn report_name(stamp: &str, n: usize) -> String {
    if n <= 1 {
        format!("{REPORT_PREFIX}{stamp}.txt")
    } else {
        format!("{REPORT_PREFIX}{stamp}_{n}.txt")
    }
}

/// The file system of the report, behind a trait for the tests.
pub trait ReportFiles: Send + Sync {
    /// The log folder, created if missing.
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)
    }

    /// A new file: fails if `path` exists.
    fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
}

/// The real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealReportFiles;

impl ReportFiles for RealReportFiles {}

/// What [`publish`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The report as written, its last line included.
    pub report: Report,
    pub text: String,
    /// The file written.
    pub file: Option<PathBuf>,
    pub exit_code: u8,
}

fn reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "accès refusé".to_owned(),
        io::ErrorKind::NotFound => "dossier introuvable".to_owned(),
        io::ErrorKind::StorageFull => "disque plein".to_owned(),
        io::ErrorKind::ReadOnlyFilesystem => "disque en lecture seule".to_owned(),
        io::ErrorKind::AlreadyExists => "trop de rapports dans la même seconde".to_owned(),
        kind => format!("erreur d'écriture ({kind})"),
    }
}

/// Writes `report` to a new file of `logs` (`logs_label`: the same folder as the user reads it, without their
/// name), then the same text to `stdout`. Every text goes through `filter` first; a text it cannot filter writes
/// nothing but a fixed line, and the exit code is 2. A file that cannot be written is said at the end of the
/// report, on the standard output, with the exit code 2.
pub fn publish(
    mut report: Report,
    logs: &Path,
    logs_label: &str,
    stamp: &str,
    files: &dyn ReportFiles,
    stdout: &mut dyn Write,
    filter: &dyn Fn(&str) -> Option<String>,
) -> Published {
    let mut written = None;
    let mut unfiltered = false;
    match files.create_dir_all(logs) {
        Err(error) => report.saved = Saved::Failed(reason(&error)),
        Ok(()) => {
            for n in 1..=ATTEMPTS {
                let name = report_name(stamp, n);
                report.saved = Saved::At(format!("{logs_label}\\{name}"));
                let Some(text) = filter(&report.text()) else {
                    unfiltered = true;
                    break;
                };
                let path = logs.join(&name);
                match files.create_new(&path, text.as_bytes()) {
                    Ok(()) => {
                        written = Some(path);
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists && n < ATTEMPTS => {}
                    Err(error) => {
                        report.saved = Saved::Failed(reason(&error));
                        break;
                    }
                }
            }
        }
    }
    let text = match (unfiltered, filter(&report.text())) {
        (false, Some(text)) => text,
        _ => {
            report.saved = Saved::Failed("filtre de sortie indisponible".to_owned());
            "MCP Proxy — auto-diagnostic\n[échec] Filtre de sortie indisponible : aucun rapport n'est écrit.\n"
                .to_owned()
        }
    };
    // A standard output that is closed (no redirection) swallows the text: the file and the message box remain.
    let _ = stdout.write_all(text.as_bytes()).and_then(|()| stdout.flush());
    let exit_code = if written.is_some() { report.exit_code() } else { 2 };
    Published {
        report,
        text,
        file: written,
        exit_code,
    }
}
