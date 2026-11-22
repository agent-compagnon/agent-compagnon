//! `cargo xtask trace`: checks that the specs, the tests that cite their acceptance criteria and the manual
//! acceptance checklist (`docs/recette.md`) agree, and that the index of `specs/README.md` matches the spec headers.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, ExitCode};

#[cfg(test)]
mod tests;

const SPECS_DIR: &str = "specs";
const INDEX_FILE: &str = "specs/README.md";
const RECETTE_FILE: &str = "docs/recette.md";
const CODE_ROOTS: [&str; 4] = ["crates/", "app/", "xtask/", "ui/"];
/// Dependencies and build outputs, should one ever be tracked by mistake.
const SKIPPED_DIRS: [&str; 3] = ["target", "node_modules", "dist"];
const INDEX_BEGIN: &str = "<!-- index:début";
const INDEX_END: &str = "<!-- index:fin -->";
/// A test cites a criterion with a comment made of this word, a colon and one or more identifiers.
const CITATION_MARKER: &str = "Exigence";
const INDEX_HEADER: &str = "| Spec | Étape | Titre | Statut | Dépend de | Critères |\n|---|---|---|---|---|---|\n";
/// A run of at least this many consecutive dependencies is written `001 à 010` in the index.
const DEPENDENCY_RANGE_MIN: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Planned,
    InProgress,
    Implemented,
    Proposed,
    Backlog,
    OnDemand,
}

impl Status {
    const ALL: [Self; 6] = [
        Self::Planned,
        Self::InProgress,
        Self::Implemented,
        Self::Proposed,
        Self::Backlog,
        Self::OnDemand,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Planned => "prévue",
            Self::InProgress => "en cours",
            Self::Implemented => "implémentée",
            Self::Proposed => "proposée",
            Self::Backlog => "backlog",
            Self::OnDemand => "à la demande",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.label() == value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Criterion {
    pub id: String,
    pub manual: bool,
    pub withdrawn: bool,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub id: String,
    /// File name inside `specs/`.
    pub file: String,
    pub title: String,
    pub stage: Option<String>,
    pub status: Status,
    pub depends_on: Vec<String>,
    pub criteria: Vec<Criterion>,
}

impl Spec {
    fn active_criteria(&self) -> impl Iterator<Item = &Criterion> {
        self.criteria.iter().filter(|c| !c.withdrawn)
    }
}

/// An error or a warning, located by file (and line when known) and by criterion when relevant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub location: String,
    pub id: Option<String>,
    pub message: String,
}

impl Issue {
    fn new(location: impl Into<String>, id: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            id: id.map(str::to_owned),
            message: message.into(),
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.id {
            Some(id) => write!(f, "{} : {id} : {}", self.location, self.message),
            None => write!(f, "{} : {}", self.location, self.message),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Code,
    Recette,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Citation {
    pub id: String,
    pub file: String,
    pub line: usize,
    pub source: Source,
}

#[derive(Debug)]
pub struct Analysis {
    /// Specs whose header is valid, sorted by identifier.
    pub specs: Vec<Spec>,
    pub citations: Vec<Citation>,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    /// At least one spec file has an invalid name or header: the index cannot be regenerated safely.
    pub format_errors: bool,
    pub expected_index: String,
}

impl Analysis {
    pub fn is_covered(&self, criterion: &Criterion) -> bool {
        let wanted = if criterion.manual {
            Source::Recette
        } else {
            Source::Code
        };
        self.citations
            .iter()
            .any(|c| c.id == criterion.id && c.source == wanted)
    }

    pub fn coverage(&self, spec: &Spec) -> (usize, usize) {
        let active: Vec<&Criterion> = spec.active_criteria().collect();
        let covered = active.iter().filter(|c| self.is_covered(c)).count();
        (covered, active.len())
    }
}

// --- Spec parsing -------------------------------------------------------------------------------------------------

enum Value {
    Text(String),
    List(Vec<String>),
}

fn parse_quoted(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then(|| inner.to_owned())
}

fn parse_list(raw: &str) -> Option<Vec<String>> {
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    inner.split(',').map(|item| parse_quoted(item.trim())).collect()
}

fn is_spec_id(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|b| b.is_ascii_digit())
}

pub fn is_criterion_id(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 9
        && value.starts_with("EX-")
        && b[3..6].iter().all(u8::is_ascii_digit)
        && b[6] == b'-'
        && b[7..9].iter().all(u8::is_ascii_digit)
}

struct Header {
    fields: BTreeMap<String, Value>,
    /// Number of lines taken by the header, both `---` lines included.
    lines: usize,
}

/// Strict reader for the subset of YAML used by spec headers: `key: "value"` and `key: ["a", "b"]` lines between
/// two `---` lines.
fn parse_header(text: &str) -> Result<Header, Vec<String>> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(vec!["en-tête absent : la première ligne doit être `---`".to_owned()]);
    }
    let mut fields = BTreeMap::new();
    let mut problems = Vec::new();
    for (index, line) in lines.enumerate() {
        let line = line.trim_end();
        if line == "---" {
            return if problems.is_empty() {
                Ok(Header {
                    fields,
                    lines: index + 2,
                })
            } else {
                Err(problems)
            };
        }
        let Some((key, raw)) = line.split_once(':') else {
            problems.push(format!("en-tête : ligne illisible « {line} »"));
            continue;
        };
        let key = key.trim();
        let raw = raw.trim();
        let value = if raw.starts_with('[') {
            parse_list(raw).map(Value::List)
        } else {
            parse_quoted(raw).map(Value::Text)
        };
        let Some(value) = value else {
            problems.push(format!(
                "en-tête : valeur de `{key}` invalide (attendu \"texte\" ou [\"a\", \"b\"])"
            ));
            continue;
        };
        if fields.insert(key.to_owned(), value).is_some() {
            problems.push(format!("en-tête : clé `{key}` en double"));
        }
    }
    Err(vec!["en-tête absent : pas de ligne `---` de fermeture".to_owned()])
}

/// Spec fields read from a header, before cross-checks between specs.
struct SpecHeader {
    id: String,
    title: String,
    stage: Option<String>,
    status: Status,
    depends_on: Vec<String>,
}

fn read_header_fields(mut fields: BTreeMap<String, Value>) -> Result<SpecHeader, Vec<String>> {
    let mut problems = Vec::new();
    for key in fields.keys() {
        if !["id", "titre", "etape", "statut", "depend_de", "suivi"].contains(&key.as_str()) {
            problems.push(format!("en-tête : clé inconnue `{key}`"));
        }
    }
    let mut text = |key: &str, required: bool| match fields.remove(key) {
        Some(Value::Text(value)) if !value.trim().is_empty() => Some(value),
        Some(_) => {
            problems.push(format!("en-tête : `{key}` doit être un texte non vide"));
            None
        }
        None => {
            if required {
                problems.push(format!("en-tête : `{key}` manquant"));
            }
            None
        }
    };
    let id = text("id", true);
    let title = text("titre", true);
    let stage = text("etape", false);
    let status = text("statut", true);
    let _tracking = text("suivi", false);
    let depends_on = match fields.remove("depend_de") {
        Some(Value::List(list)) => Some(list),
        Some(Value::Text(_)) => {
            problems.push("en-tête : `depend_de` doit être une liste".to_owned());
            None
        }
        None => {
            problems.push("en-tête : `depend_de` manquant".to_owned());
            None
        }
    };
    let status = status.and_then(|s| {
        let parsed = Status::parse(&s);
        if parsed.is_none() {
            let known: Vec<&str> = Status::ALL.iter().map(|s| s.label()).collect();
            problems.push(format!("statut inconnu « {s} » (attendu : {})", known.join(", ")));
        }
        parsed
    });
    if let Some(id) = &id
        && !is_spec_id(id)
    {
        problems.push(format!("`id` « {id} » invalide : trois chiffres attendus"));
    }
    match (id, title, status, depends_on) {
        (Some(id), Some(title), Some(status), Some(depends_on)) if problems.is_empty() => Ok(SpecHeader {
            id,
            title,
            stage,
            status,
            depends_on,
        }),
        _ => Err(problems),
    }
}

enum CriterionLine {
    Valid(Criterion),
    Malformed(String),
}

/// Reads an acceptance-criterion bullet: `- **EX-NNN-nn** …`, `(manuel)` right after the identifier, and
/// `~~…~~` around a withdrawn criterion.
fn parse_criterion_line(line: &str, line_no: usize) -> Option<CriterionLine> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* "))?;
    let (withdrawn, rest) = match rest.strip_prefix("~~") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let rest = rest.strip_prefix("**EX")?;
    let Some(end) = rest.find("**") else {
        return Some(CriterionLine::Malformed(format!("EX{rest}")));
    };
    let id = format!("EX{}", &rest[..end]);
    if !is_criterion_id(&id) {
        return Some(CriterionLine::Malformed(id));
    }
    let manual = rest[end + 2..].trim_start().starts_with("(manuel)");
    Some(CriterionLine::Valid(Criterion {
        id,
        manual,
        withdrawn,
        line: line_no,
    }))
}

fn parse_criteria(text: &str, skip_lines: usize, location: &str, issues: &mut Vec<Issue>) -> Vec<Criterion> {
    let mut criteria = Vec::new();
    let mut in_fence = false;
    for (index, line) in text.lines().enumerate().skip(skip_lines) {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        match parse_criterion_line(line, index + 1) {
            Some(CriterionLine::Valid(c)) => criteria.push(c),
            Some(CriterionLine::Malformed(id)) => issues.push(Issue::new(
                format!("{location}:{}", index + 1),
                Some(&id),
                "identifiant de critère mal formé (attendu EX-NNN-nn)",
            )),
            None => {}
        }
    }
    criteria
}

/// Identifier taken from a spec file name `NNN-nom.md`.
fn file_spec_id(name: &str) -> Option<&str> {
    let id = name.get(..3)?;
    (is_spec_id(id) && name[3..].starts_with('-') && name.len() > 7).then_some(id)
}

// --- Citations ----------------------------------------------------------------------------------------------------

fn take_identifier(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(text.len());
    &text[..end]
}

fn is_blank(c: char) -> bool {
    c.is_whitespace() || c == '\u{a0}' || c == '\u{202f}'
}

/// Identifiers cited on one line of code: the marker word, a colon, then identifiers separated by commas or spaces.
fn code_citations_in_line(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in line.match_indices(CITATION_MARKER) {
        let after = line[at + CITATION_MARKER.len()..].trim_start_matches(is_blank);
        let Some(mut rest) = after.strip_prefix(':') else {
            continue;
        };
        loop {
            rest = rest.trim_start_matches(|c: char| is_blank(c) || c == ',');
            if !rest.starts_with("EX-") {
                break;
            }
            let id = take_identifier(rest);
            found.push(id.to_owned());
            rest = &rest[id.len()..];
        }
    }
    found
}

/// Every `EX-…` token of a line of the manual checklist.
fn recette_citations_in_line(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in line.match_indices("EX-") {
        let starts_word = line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-'));
        if starts_word {
            found.push(take_identifier(&line[at..]).to_owned());
        }
    }
    found
}

fn collect_citations(file: &str, text: &str, source: Source, issues: &mut Vec<Issue>, out: &mut Vec<Citation>) {
    for (index, line) in text.lines().enumerate() {
        let ids = match source {
            Source::Code => code_citations_in_line(line),
            Source::Recette => recette_citations_in_line(line),
        };
        for id in ids {
            if is_criterion_id(&id) {
                out.push(Citation {
                    id,
                    file: file.to_owned(),
                    line: index + 1,
                    source,
                });
            } else {
                issues.push(Issue::new(
                    format!("{file}:{}", index + 1),
                    Some(&id),
                    "identifiant cité mal formé (attendu EX-NNN-nn)",
                ));
            }
        }
    }
}

/// Code files to scan, from the paths tracked by git.
pub fn is_code_path(path: &str) -> bool {
    CODE_ROOTS.iter().any(|root| path.starts_with(root))
        && !path.split('/').any(|segment| SKIPPED_DIRS.contains(&segment))
}

pub fn tracked_code_files(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("lancement de `git ls-files` impossible : {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git ls-files` a échoué : {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output
        .stdout
        .split(|&b| b == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .filter(|path| is_code_path(path))
        .collect())
}

// --- Index --------------------------------------------------------------------------------------------------------

fn format_dependencies(deps: &[String]) -> String {
    if deps.is_empty() {
        return "—".to_owned();
    }
    let numbers: Option<Vec<u32>> = deps.iter().map(|d| d.parse().ok()).collect();
    let Some(numbers) = numbers else {
        return deps.join(", ");
    };
    let mut parts = Vec::new();
    let mut start = 0;
    while start < deps.len() {
        let mut end = start;
        while end + 1 < deps.len() && numbers[end + 1] == numbers[end] + 1 {
            end += 1;
        }
        if end - start + 1 >= DEPENDENCY_RANGE_MIN {
            parts.push(format!("{} à {}", deps[start], deps[end]));
        } else {
            parts.extend(deps[start..=end].iter().cloned());
        }
        start = end + 1;
    }
    parts.join(", ")
}

pub fn render_index(specs: &[Spec]) -> String {
    let mut out = INDEX_HEADER.to_owned();
    for spec in specs {
        let _ = writeln!(
            out,
            "| [{id}]({file}) | {stage} | {title} | {status} | {deps} | {count} |",
            id = spec.id,
            file = spec.file,
            stage = spec.stage.as_deref().unwrap_or("—"),
            title = spec.title,
            status = spec.status.label(),
            deps = format_dependencies(&spec.depends_on),
            count = spec.active_criteria().count(),
        );
    }
    out
}

/// Byte range of the generated part of the index: the lines strictly between the two markers.
fn index_range(readme: &str) -> Option<(usize, usize)> {
    let begin_line = readme
        .match_indices(INDEX_BEGIN)
        .find(|(at, _)| *at == 0 || readme[..*at].ends_with('\n'))?
        .0;
    let start = begin_line + readme[begin_line..].find('\n')? + 1;
    let mut offset = start;
    for line in readme[start..].split_inclusive('\n') {
        if line.trim_end() == INDEX_END {
            return Some((start, offset));
        }
        offset += line.len();
    }
    None
}

// --- Analysis -----------------------------------------------------------------------------------------------------

fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Checks the specs under `root/specs`, the citations found in `code_files` (paths relative to `root`) and in
/// `root/docs/recette.md`, and the index of `root/specs/README.md`.
pub fn analyze(root: &Path, code_files: &[String]) -> io::Result<Analysis> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut format_errors = false;

    let mut names: Vec<String> = fs::read_dir(root.join(SPECS_DIR))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    names.sort();

    let mut specs = Vec::new();
    // Every criterion found, valid header or not, so that a broken header does not turn its citations into noise.
    let mut known: BTreeMap<String, (String, Criterion)> = BTreeMap::new();
    let mut spec_files: BTreeMap<String, String> = BTreeMap::new();
    for name in names {
        let location = format!("{SPECS_DIR}/{name}");
        let Some(file_id) = file_spec_id(&name).filter(|_| name.ends_with(".md")) else {
            errors.push(Issue::new(
                &location,
                None,
                "nom de fichier invalide (attendu NNN-nom.md)",
            ));
            format_errors = true;
            continue;
        };
        let text = String::from_utf8_lossy(&fs::read(root.join(&location))?).into_owned();
        if let Some(other) = spec_files.insert(file_id.to_owned(), location.clone()) {
            errors.push(Issue::new(
                &location,
                None,
                format!("numéro de spec {file_id} déjà pris par {other}"),
            ));
            format_errors = true;
        }

        let (header, skip) = match parse_header(&text) {
            Ok(header) => {
                let skip = header.lines;
                (read_header_fields(header.fields), skip)
            }
            Err(problems) => (Err(problems), 0),
        };
        let criteria = parse_criteria(&text, skip, &location, &mut errors);
        for criterion in &criteria {
            if criterion.id[3..6] != *file_id {
                errors.push(Issue::new(
                    format!("{location}:{}", criterion.line),
                    Some(&criterion.id),
                    format!("le critère porte le numéro d'une autre spec que {file_id}"),
                ));
            }
            if let Some((other, _)) = known.get(&criterion.id) {
                errors.push(Issue::new(
                    format!("{location}:{}", criterion.line),
                    Some(&criterion.id),
                    format!("identifiant de critère en double (déjà dans {other})"),
                ));
            } else {
                known.insert(criterion.id.clone(), (location.clone(), criterion.clone()));
            }
        }
        match header {
            Ok(header) if header.id != file_id => {
                errors.push(Issue::new(
                    &location,
                    None,
                    format!("`id` « {} » différent du numéro du fichier « {file_id} »", header.id),
                ));
                format_errors = true;
            }
            Ok(header) => specs.push(Spec {
                id: header.id,
                file: name.clone(),
                title: header.title,
                stage: header.stage,
                status: header.status,
                depends_on: header.depends_on,
                criteria,
            }),
            Err(problems) => {
                format_errors = true;
                errors.extend(problems.into_iter().map(|p| Issue::new(&location, None, p)));
            }
        }
    }

    let ids: BTreeSet<&str> = spec_files.keys().map(String::as_str).collect();
    for spec in &specs {
        for dep in &spec.depends_on {
            if !ids.contains(dep.as_str()) {
                errors.push(Issue::new(
                    format!("{SPECS_DIR}/{}", spec.file),
                    None,
                    format!("dépendance vers une spec inexistante « {dep} »"),
                ));
            }
        }
    }

    let mut citations = Vec::new();
    for file in code_files {
        if let Some(text) = read_optional(&root.join(file))? {
            collect_citations(file, &text, Source::Code, &mut errors, &mut citations);
        }
    }
    if let Some(text) = read_optional(&root.join(RECETTE_FILE))? {
        collect_citations(RECETTE_FILE, &text, Source::Recette, &mut errors, &mut citations);
    }
    for citation in &citations {
        let location = format!("{}:{}", citation.file, citation.line);
        match known.get(&citation.id) {
            None => errors.push(Issue::new(
                location,
                Some(&citation.id),
                "identifiant cité absent des specs",
            )),
            Some((spec, criterion)) if criterion.withdrawn => errors.push(Issue::new(
                location,
                Some(&citation.id),
                format!("critère retiré (barré dans {spec}) encore cité"),
            )),
            Some(_) => {}
        }
    }

    let mut analysis = Analysis {
        specs,
        citations,
        errors: Vec::new(),
        warnings: Vec::new(),
        format_errors,
        expected_index: String::new(),
    };
    for spec in &analysis.specs {
        let location = format!("{SPECS_DIR}/{}", spec.file);
        for criterion in spec.active_criteria().filter(|c| !analysis.is_covered(c)) {
            let message = if criterion.manual {
                "critère manuel non couvert : absent de docs/recette.md"
            } else {
                "critère non couvert : aucun fichier de code ne le cite"
            };
            let issue = Issue::new(format!("{location}:{}", criterion.line), Some(&criterion.id), message);
            match spec.status {
                Status::Implemented => errors.push(issue),
                Status::InProgress => warnings.push(issue),
                _ => {}
            }
        }
    }

    analysis.expected_index = render_index(&analysis.specs);
    match read_optional(&root.join(INDEX_FILE))? {
        None => errors.push(Issue::new(INDEX_FILE, None, "fichier absent")),
        Some(readme) => match index_range(&readme) {
            None => errors.push(Issue::new(
                INDEX_FILE,
                None,
                format!("marqueurs de l'index introuvables (`{INDEX_BEGIN} …` puis `{INDEX_END}`)"),
            )),
            Some((start, end)) if readme[start..end] != analysis.expected_index => errors.push(Issue::new(
                INDEX_FILE,
                None,
                "index périmé : lancer `cargo xtask trace --write-index`",
            )),
            Some(_) => {}
        },
    }

    analysis.errors = errors;
    analysis.warnings = warnings;
    Ok(analysis)
}

/// Rewrites the generated part of the index. Refused while a spec file name or header is invalid, so that a broken
/// spec never silently drops out of the index.
pub fn write_index(root: &Path, analysis: &Analysis) -> Result<bool, String> {
    if analysis.format_errors {
        return Err("index non régénéré : corriger d'abord les noms et en-têtes de specs".to_owned());
    }
    let path = root.join(INDEX_FILE);
    let readme = fs::read_to_string(&path).map_err(|e| format!("{INDEX_FILE} : lecture impossible : {e}"))?;
    let (start, end) =
        index_range(&readme).ok_or_else(|| format!("{INDEX_FILE} : marqueurs de l'index introuvables"))?;
    if readme[start..end] == analysis.expected_index {
        return Ok(false);
    }
    let updated = format!("{}{}{}", &readme[..start], analysis.expected_index, &readme[end..]);
    fs::write(&path, updated).map_err(|e| format!("{INDEX_FILE} : écriture impossible : {e}"))?;
    Ok(true)
}

// --- Command ------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Check,
    WriteIndex,
    Report,
}

fn print_report(analysis: &Analysis) {
    println!("matrice critère → fichiers qui le citent");
    for spec in analysis.specs.iter().filter(|s| !s.criteria.is_empty()) {
        println!("\n{} — {}", spec.id, spec.title);
        for criterion in &spec.criteria {
            let mut places: Vec<String> = analysis
                .citations
                .iter()
                .filter(|c| c.id == criterion.id)
                .map(|c| format!("{}:{}", c.file, c.line))
                .collect();
            places.dedup();
            let kind = match (criterion.withdrawn, criterion.manual) {
                (true, _) => " (retiré)",
                (false, true) => " (manuel)",
                (false, false) => "",
            };
            let places = if places.is_empty() {
                "—".to_owned()
            } else {
                places.join(", ")
            };
            println!("  {}{kind:<10} {places}", criterion.id);
        }
    }
    println!();
}

fn print_outcome(analysis: &Analysis) -> ExitCode {
    for spec in &analysis.specs {
        let (covered, total) = analysis.coverage(spec);
        println!(
            "  {}  {:<12}  {:>2}/{:<2}  {}",
            spec.id,
            spec.status.label(),
            covered,
            total,
            spec.title
        );
    }
    for warning in &analysis.warnings {
        println!("avertissement : {warning}");
    }
    for error in &analysis.errors {
        eprintln!("erreur : {error}");
    }
    if analysis.errors.is_empty() {
        println!(
            "trace : OK ({} specs, {} avertissement(s))",
            analysis.specs.len(),
            analysis.warnings.len()
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("trace : {} erreur(s)", analysis.errors.len());
        ExitCode::FAILURE
    }
}

fn analyze_repository(root: &Path) -> Result<Analysis, String> {
    let files = tracked_code_files(root)?;
    analyze(root, &files).map_err(|e| format!("lecture impossible : {e}"))
}

pub fn run(root: &Path, mode: Mode) -> ExitCode {
    let mut analysis = match analyze_repository(root) {
        Ok(analysis) => analysis,
        Err(e) => {
            eprintln!("trace : {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut index_failed = false;
    if mode == Mode::WriteIndex {
        match write_index(root, &analysis) {
            Ok(true) => println!("index de {INDEX_FILE} régénéré"),
            Ok(false) => println!("index de {INDEX_FILE} déjà à jour"),
            Err(e) => {
                eprintln!("trace : {e}");
                index_failed = true;
            }
        }
        analysis = match analyze_repository(root) {
            Ok(analysis) => analysis,
            Err(e) => {
                eprintln!("trace : {e}");
                return ExitCode::FAILURE;
            }
        };
    }
    if mode == Mode::Report {
        print_report(&analysis);
    }
    let outcome = print_outcome(&analysis);
    if index_failed { ExitCode::FAILURE } else { outcome }
}
