//! What the diagnosis found, and how it reads: the text of the report and its structure for the window.

use serde_json::{Value, json};

/// The outcome of a finding, ordered from the best to the worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    Ok,
    /// Something waits for an action, nothing is broken yet.
    Warning,
    Failure,
}

impl Status {
    /// As the report writes it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "avertissement",
            Self::Failure => "échec",
        }
    }

    /// As the window receives it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Failure => "failure",
        }
    }

    /// `mcp-proxy.exe doctor`: 0 when all is well, 1 with warnings, 2 with failures.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Warning => 1,
            Self::Failure => 2,
        }
    }
}

/// One thing a check found: French, for someone who has not read the code; `fix` says what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub status: Status,
    pub message: String,
    pub fix: Option<String>,
}

impl Finding {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            status: Status::Ok,
            message: message.into(),
            fix: None,
        }
    }

    pub fn warning(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            status: Status::Warning,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }

    pub fn failure(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            status: Status::Failure,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }
}

/// The checks of spec 024, in the order of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckId {
    Config,
    Webview,
    Installation,
    Notifications,
    App,
    Settings,
    Artifactory,
    UpdateKey,
    Clients,
    Mcp,
    Clock,
}

impl CheckId {
    pub const ALL: [Self; 11] = [
        Self::Config,
        Self::Webview,
        Self::Installation,
        Self::Notifications,
        Self::App,
        Self::Settings,
        Self::Artifactory,
        Self::UpdateKey,
        Self::Clients,
        Self::Mcp,
        Self::Clock,
    ];

    /// The name of the check, as the table of the spec gives it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Config => "Config",
            Self::Webview => "WebView2",
            Self::Installation => "Installation",
            Self::Notifications => "Notifications",
            Self::App => "Appli",
            Self::Settings => "Réglages",
            Self::Artifactory => "Artifactory",
            Self::UpdateKey => "Clé de mise à jour",
            Self::Clients => "Clients LLM",
            Self::Mcp => "MCP",
            Self::Clock => "Horloge",
        }
    }

    /// The identifier the window receives.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Webview => "webview",
            Self::Installation => "installation",
            Self::Notifications => "notifications",
            Self::App => "app",
            Self::Settings => "settings",
            Self::Artifactory => "artifactory",
            Self::UpdateKey => "updateKey",
            Self::Clients => "clients",
            Self::Mcp => "mcp",
            Self::Clock => "clock",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub id: CheckId,
    /// Never empty.
    pub findings: Vec<Finding>,
}

impl CheckResult {
    /// The worst of its findings.
    pub fn status(&self) -> Status {
        self.findings.iter().map(|f| f.status).max().unwrap_or(Status::Ok)
    }
}

/// Where the report comes from: the checks of the application do not mean the same from inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewpoint {
    /// `mcp-proxy.exe doctor`, a process of its own.
    Command,
    /// « Auto-diagnostic » in the window, or the diagnostic export: the running application itself.
    App,
}

impl Viewpoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::App => "app",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Command => "lancé par mcp-proxy.exe doctor",
            Self::App => "lancé depuis la fenêtre de MCP Proxy",
        }
    }
}

/// What happened to the file of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Not written (the report attached to a diagnostic export).
    No,
    /// Written there, as the user reads it (`%LOCALAPPDATA%\MCP Proxy\logs\doctor-….txt`).
    At(String),
    /// Could not be written: why, in French, without any path.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Of this exe.
    pub version: String,
    /// RFC 3339, UTC.
    pub at: String,
    pub viewpoint: Viewpoint,
    pub checks: Vec<CheckResult>,
    pub saved: Saved,
}

impl Report {
    /// The worst of every finding, an unwritten report file included.
    pub fn worst(&self) -> Status {
        let file = match self.saved {
            Saved::Failed(_) => Status::Failure,
            Saved::No | Saved::At(_) => Status::Ok,
        };
        self.checks.iter().map(CheckResult::status).fold(file, Status::max)
    }

    /// The exit code of `mcp-proxy.exe doctor`.
    pub fn exit_code(&self) -> u8 {
        self.worst().exit_code()
    }

    /// The checks whose worst finding is a warning, and those whose worst is a failure.
    pub fn counts(&self) -> (usize, usize) {
        let count = |status| self.checks.iter().filter(|c| c.status() == status).count();
        (count(Status::Warning), count(Status::Failure))
    }

    /// One line, for the window and the message box: « 2 avertissements, 1 échec ».
    pub fn summary(&self) -> String {
        let (warnings, failures) = self.counts();
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {many}")
            }
        };
        match (warnings, failures) {
            (0, 0) => "tout va bien".to_owned(),
            (w, 0) => plural(w, "avertissement", "avertissements"),
            (0, f) => plural(f, "échec", "échecs"),
            (w, f) => format!(
                "{}, {}",
                plural(w, "avertissement", "avertissements"),
                plural(f, "échec", "échecs")
            ),
        }
    }

    /// Every text of the report passed through `filter` (the output filter of the caller). `None` when one text
    /// could not be filtered: nothing of the report is to be shown then.
    pub fn filtered(mut self, filter: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        for check in &mut self.checks {
            for finding in &mut check.findings {
                finding.message = filter(&finding.message)?;
                if let Some(fix) = &finding.fix {
                    finding.fix = Some(filter(fix)?);
                }
            }
        }
        Some(self)
    }

    /// The report as text: a header, one block per check, the file it is saved in last.
    pub fn text(&self) -> String {
        let mut out = String::new();
        out.push_str("MCP Proxy — auto-diagnostic\n");
        out.push_str(&format!(
            "Version {}, {} (UTC), {}.\n",
            self.version,
            self.at,
            self.viewpoint.label()
        ));
        out.push_str(&format!("Résultat : {}.\n", self.summary()));
        for check in &self.checks {
            out.push('\n');
            out.push_str(&format!("[{}] {}\n", check.status().label(), check.id.name()));
            let mut told: Vec<&str> = Vec::new();
            for finding in &check.findings {
                let mark = match finding.status {
                    Status::Ok => "-",
                    Status::Warning => "!",
                    Status::Failure => "×",
                };
                out.push_str(&format!("  {mark} {}\n", finding.message));
                // The same advice for several findings of a check is said once, after the first.
                if let Some(fix) = finding.fix.as_deref().filter(|fix| !told.contains(fix)) {
                    out.push_str(&format!("    À faire : {fix}\n"));
                    told.push(fix);
                }
            }
        }
        out.push('\n');
        match &self.saved {
            Saved::No => out.push_str("Rapport joint à l'export de diagnostic.\n"),
            Saved::At(file) => out.push_str(&format!("Rapport enregistré dans {file}\n")),
            Saved::Failed(reason) => out.push_str(&format!(
                "[échec] Rapport non enregistré dans le dossier des logs ({reason}).\n"
            )),
        }
        out
    }

    /// The structure the window receives (camelCase); `text` is what « Copier » copies.
    pub fn to_json(&self) -> Value {
        let (warnings, failures) = self.counts();
        let checks: Vec<Value> = self
            .checks
            .iter()
            .map(|check| {
                json!({
                    "id": check.id.as_str(),
                    "name": check.id.name(),
                    "status": check.status().as_str(),
                    "findings": check.findings.iter().map(|f| json!({
                        "status": f.status.as_str(),
                        "message": f.message,
                        "fix": f.fix,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let (file, file_error) = match &self.saved {
            Saved::No => (None, None),
            Saved::At(file) => (Some(file.as_str()), None),
            Saved::Failed(reason) => (None, Some(reason.as_str())),
        };
        json!({
            "version": self.version,
            "at": self.at,
            "viewpoint": self.viewpoint.as_str(),
            "status": self.worst().as_str(),
            "summary": self.summary(),
            "warnings": warnings,
            "failures": failures,
            "checks": checks,
            "file": file,
            "fileError": file_error,
            "text": self.text(),
        })
    }
}
