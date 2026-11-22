//! What can be checked in `.gitlab-ci.yml` without GitLab (spec 021), for `cargo xtask check-ci`:
//!
//! - the file is valid YAML (one document, a mapping, no duplicate key, no unknown alias, no `include`);
//! - its structure holds together: every job's stage is declared, `extends`, `dependencies` and `needs` name existing
//!   jobs, every job has a `script`;
//! - the release jobs (stages `build`, `authenticode`, `sign`, `publish`) run on protected tags only, and nothing
//!   turns on a trace of the commands (`set -x`, `Set-PSDebug`, `CI_DEBUG_TRACE`), which would print secrets;
//! - a job that declares the `release` environment, and so receives the private key, runs no build tool nor package
//!   manager (cargo, rustup, npm…): it runs tools built by an earlier job, without the key;
//! - every `cargo xtask …` it cites is a form of xtask (`xtask/src/commands.rs`), and every `update-tool …` a
//!   command of that tool (`crates/deploy/src/update/tool_commands.rs`);
//! - every variable it uses or defines is documented in `docs/release.md`, as `` `NAME` ``.
//!
//! GitLab's own rules (keywords, `rules:if` syntax, runners) are not checked here: the first pipeline does it
//! (`docs/recette.md`).

use std::collections::{BTreeMap, BTreeSet};

use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlLoader};

#[path = "../../../xtask/src/commands.rs"]
mod commands;

/// The forms of `cargo xtask`, shared with xtask.
pub use commands::FORMS as XTASK_FORMS;

#[path = "../../deploy/src/update/tool_commands.rs"]
mod tool_commands;

/// The signing tool of spec 020, run directly by `sign`.
pub const TOOL: &str = "update-tool";

/// The environment whose variables hold the secrets of a release.
pub const RELEASE_ENVIRONMENT: &str = "release";

/// Programs that build code or install packages: whatever they run (build scripts, procedural macros, install
/// scripts) would run with the variables of the job.
pub const BUILD_TOOLS: [&str; 29] = [
    "cargo",
    "cargo-xwin",
    "rustup",
    "rustc",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "node",
    "corepack",
    "pip",
    "pip3",
    "pipx",
    "apt",
    "apt-get",
    "apk",
    "dnf",
    "yum",
    "microdnf",
    "zypper",
    "choco",
    "winget",
    "scoop",
    "make",
    "cmake",
    "ninja",
    "msbuild",
    "dotnet",
    "go",
];

/// Stages whose jobs build, sign or publish a release: protected tags only.
pub const RELEASE_STAGES: [&str; 4] = ["build", "authenticode", "sign", "publish"];

/// Top-level keys that are not jobs.
const GLOBAL_KEYS: [&str; 10] = [
    "default",
    "include",
    "stages",
    "variables",
    "workflow",
    "image",
    "services",
    "cache",
    "before_script",
    "after_script",
];

/// Anything that prints the commands of a job, with the values of its variables.
const TRACE_MARKERS: [&str; 5] = [
    "set -x",
    "set -o xtrace",
    "Set-PSDebug",
    "CI_DEBUG_TRACE",
    "CI_DEBUG_SERVICES",
];

/// GitLab follows `extends` 11 levels deep at most.
const MAX_EXTENDS_DEPTH: usize = 11;

const XTASK: &str = "cargo xtask";

/// One thing wrong, with where it was found: a path in the YAML (`sign.script[1]`) or a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub location: String,
    pub message: String,
}

impl Problem {
    fn new(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            message: message.into(),
        }
    }
}

/// What `check` found.
#[derive(Debug, Default)]
pub struct Report {
    pub problems: Vec<Problem>,
    /// Jobs that can run (hidden ones excluded).
    pub jobs: usize,
    /// `cargo xtask` commands cited.
    pub commands: usize,
    /// Distinct variables used or defined.
    pub variables: usize,
}

/// A text value of the file, with its path.
struct Text<'a> {
    path: String,
    text: &'a str,
}

/// Everything read from the file in one walk.
#[derive(Default)]
struct Walk<'a> {
    texts: Vec<Text<'a>>,
    /// Keys of every `variables:` mapping, with their path.
    defined: Vec<(String, String)>,
}

fn key_text(key: &Yaml) -> Option<&str> {
    match key {
        Yaml::String(text) => Some(text),
        _ => None,
    }
}

fn walk<'a>(node: &'a Yaml, path: &str, out: &mut Walk<'a>, problems: &mut Vec<Problem>) {
    match node {
        Yaml::String(text) => out.texts.push(Text {
            path: path.to_owned(),
            text,
        }),
        Yaml::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(item, &format!("{path}[{index}]"), out, problems);
            }
        }
        Yaml::Hash(hash) => {
            for (key, value) in hash {
                let Some(name) = key_text(key) else {
                    problems.push(Problem::new(path, format!("clé non textuelle : {key:?}")));
                    continue;
                };
                let child = if path.is_empty() {
                    name.to_owned()
                } else {
                    format!("{path}.{name}")
                };
                if name == "variables"
                    && let Yaml::Hash(variables) = value
                {
                    for variable in variables.keys().filter_map(key_text) {
                        out.defined.push((variable.to_owned(), format!("{child}.{variable}")));
                    }
                }
                walk(value, &child, out, problems);
            }
        }
        Yaml::Alias(_) | Yaml::BadValue => {
            problems.push(Problem::new(path, "alias inconnu ou valeur illisible"));
        }
        Yaml::Real(_) | Yaml::Integer(_) | Yaml::Boolean(_) | Yaml::Null => {}
    }
}

fn get<'a>(hash: &'a Hash, key: &str) -> Option<&'a Yaml> {
    hash.get(&Yaml::String(key.to_owned()))
}

/// The names of a list of strings, or of a single string.
fn names(value: &Yaml) -> Vec<&str> {
    match value {
        Yaml::String(name) => vec![name.as_str()],
        Yaml::Array(items) => items.iter().filter_map(Yaml::as_str).collect(),
        _ => Vec::new(),
    }
}

struct Jobs<'a> {
    /// Every top-level entry that is not a global key, hidden ones included, in file order.
    all: Vec<(&'a str, &'a Hash)>,
    by_name: BTreeMap<&'a str, &'a Hash>,
}

impl<'a> Jobs<'a> {
    /// The value of `key` for `job`, its own or inherited through `extends` (the last parent wins, as in GitLab;
    /// lists and scalars are replaced, not merged, which is all the checks below need).
    fn effective(&self, job: &str, key: &str, depth: usize) -> Option<&'a Yaml> {
        let hash = self.by_name.get(job)?;
        if let Some(value) = get(hash, key) {
            return Some(value);
        }
        if depth >= MAX_EXTENDS_DEPTH {
            return None;
        }
        let parents = get(hash, "extends").map(names).unwrap_or_default();
        parents
            .iter()
            .rev()
            .find_map(|parent| self.effective(parent, key, depth + 1))
    }

    fn stage(&self, job: &str) -> Option<&'a str> {
        self.effective(job, "stage", 0).and_then(Yaml::as_str)
    }

    /// `true` when the `extends` chain of `job` loops or is deeper than GitLab allows.
    fn extends_too_deep(&self, job: &str, depth: usize) -> bool {
        if depth > MAX_EXTENDS_DEPTH {
            return true;
        }
        let Some(hash) = self.by_name.get(job) else {
            return false;
        };
        let parents = get(hash, "extends").map(names).unwrap_or_default();
        parents.iter().any(|parent| self.extends_too_deep(parent, depth + 1))
    }
}

fn is_hidden(job: &str) -> bool {
    job.starts_with('.')
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A rule condition that holds only on a protected tag: a conjunction (no `||`) with a conjunct on
/// `$CI_COMMIT_TAG` (set, matched by `=~` or `!= null`) and the conjunct `$CI_COMMIT_REF_PROTECTED == "true"`.
pub fn requires_protected_tag(condition: &str) -> bool {
    let compact: String = condition
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')')
        .collect();
    if compact.contains("||") {
        return false;
    }
    let conjuncts: Vec<&str> = compact.split("&&").collect();
    let on_tag = conjuncts
        .iter()
        .any(|c| *c == "$CI_COMMIT_TAG" || c.starts_with("$CI_COMMIT_TAG=~") || *c == "$CI_COMMIT_TAG!=null");
    let protected = conjuncts
        .iter()
        .any(|c| *c == "$CI_COMMIT_REF_PROTECTED==\"true\"" || *c == "$CI_COMMIT_REF_PROTECTED=='true'");
    on_tag && protected
}

/// The variables a text refers to: `$NAME`, `${NAME…}`, and PowerShell's `$env:NAME`.
pub fn variable_references(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('$') {
        let mut start = from + offset + 1;
        if text[start..].starts_with('{') {
            start += 1;
        }
        if text
            .get(start..start + 4)
            .is_some_and(|p| p.eq_ignore_ascii_case("env:"))
        {
            start += 4;
        }
        let len = text[start..].chars().take_while(|c| is_name_char(*c)).count();
        let name = &text[start..start + len];
        if name.chars().next().is_some_and(|c| !c.is_ascii_digit()) {
            found.push(name);
        }
        from = start + len;
    }
    found
}

/// The words after each `cargo xtask` of a line, up to the end of the shell command.
pub fn xtask_citations(line: &str) -> Vec<Vec<&str>> {
    let mut found = Vec::new();
    for (at, _) in line.match_indices(XTASK) {
        let starts = line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "(;&|'\"`{".contains(c));
        let rest = &line[at + XTASK.len()..];
        let ends = rest.chars().next().is_none_or(char::is_whitespace);
        if starts && ends {
            let end = rest.find(|c| ";&|)}#<>".contains(c)).unwrap_or(rest.len());
            found.push(rest[..end].split_whitespace().collect());
        }
    }
    found
}

/// The commands a shell line runs (bash or PowerShell, roughly): for each one, the program's name without its
/// directory nor `.exe`/`.cmd`/`.ps1`/`.bat`, then its arguments. Quotes are not understood, which can only find more
/// commands than there are.
pub fn shell_commands(line: &str) -> Vec<(String, Vec<&str>)> {
    const WRAPPERS: [&str; 5] = ["sudo", "exec", "command", "env", "time"];
    let mut found = Vec::new();
    for segment in line.split([';', '|', '&', '(', ')', '{', '}', '`']) {
        let mut words = segment.split_whitespace().skip_while(|w| {
            WRAPPERS.contains(w)
                || w.split_once('=')
                    .is_some_and(|(name, _)| !name.is_empty() && name.chars().all(is_name_char))
        });
        let Some(program) = words.next() else { continue };
        let program = program.trim_matches(|c| c == '"' || c == '\'' || c == '$');
        let base = program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(program)
            .to_ascii_lowercase();
        let base = [".exe", ".cmd", ".ps1", ".bat"]
            .iter()
            .find_map(|ext| base.strip_suffix(ext))
            .map_or(base.clone(), str::to_owned);
        if !base.is_empty() {
            found.push((base, words.collect()));
        }
    }
    found
}

/// Whether a text of the file is a line of a script, from its path (`sign.script[1]`).
fn is_script(path: &str) -> bool {
    path.split('.').any(|segment| {
        ["script", "before_script", "after_script"]
            .iter()
            .any(|key| segment == *key || segment.strip_prefix(key).is_some_and(|rest| rest.starts_with('[')))
    })
}

/// Whether a job declares the release environment.
fn declares_release_environment(value: &Yaml) -> bool {
    match value {
        Yaml::String(name) => name == RELEASE_ENVIRONMENT,
        Yaml::Hash(environment) => get(environment, "name").and_then(Yaml::as_str) == Some(RELEASE_ENVIRONMENT),
        _ => false,
    }
}

/// Whether the words of a citation are one of the forms of xtask.
pub fn is_known_command(words: &[&str], forms: &[&str]) -> bool {
    forms.iter().any(|form| {
        let fixed: Vec<&str> = form.split_whitespace().collect();
        match fixed.split_last() {
            Some((&"…", head)) => words.len() >= head.len() && words[..head.len()] == *head,
            _ => words == fixed.as_slice(),
        }
    })
}

/// Whether `docs/release.md` documents a variable: its name between backticks.
fn is_documented(doc: &str, name: &str) -> bool {
    doc.contains(&format!("`{name}`"))
}

/// Checks the text of `.gitlab-ci.yml` against the text of `docs/release.md` and the forms of xtask.
pub fn check(ci: &str, doc: &str, forms: &[&str]) -> Report {
    let mut report = Report::default();
    let documents = match YamlLoader::load_from_str(ci) {
        Ok(documents) => documents,
        Err(e) => {
            report.problems.push(Problem::new(
                format!("ligne {}, colonne {}", e.marker().line(), e.marker().col() + 1),
                format!("YAML invalide : {}", e.info()),
            ));
            return report;
        }
    };
    let root = match documents.as_slice() {
        [Yaml::Hash(root)] => root,
        [_] => {
            report.problems.push(Problem::new("racine", "une table est attendue"));
            return report;
        }
        _ => {
            report
                .problems
                .push(Problem::new("fichier", "un seul document YAML est attendu"));
            return report;
        }
    };

    let mut walked = Walk::default();
    walk(&documents[0], "", &mut walked, &mut report.problems);

    if get(root, "include").is_some() {
        report.problems.push(Problem::new(
            "include",
            "fichier inclus : check-ci ne vérifie que ce fichier, tout doit y être",
        ));
    }

    let mut all = Vec::new();
    for (key, value) in root {
        let Some(name) = key_text(key) else { continue };
        if GLOBAL_KEYS.contains(&name) {
            continue;
        }
        match value {
            Yaml::Hash(hash) => all.push((name, hash)),
            _ if is_hidden(name) => {}
            _ => report.problems.push(Problem::new(name, "un job doit être une table")),
        }
    }
    let jobs = Jobs {
        by_name: all.iter().copied().collect(),
        all,
    };
    report.jobs = jobs.all.iter().filter(|(name, _)| !is_hidden(name)).count();

    check_structure(root, &jobs, &mut report.problems);
    check_release_jobs(&jobs, &mut report.problems);
    check_secret_jobs(root, &jobs, &mut report.problems);
    check_texts(&walked, doc, forms, &mut report);
    report
}

fn check_structure(root: &Hash, jobs: &Jobs<'_>, problems: &mut Vec<Problem>) {
    let mut stages: Vec<&str> = vec![".pre"];
    match get(root, "stages") {
        Some(Yaml::Array(items)) if items.iter().all(|s| s.as_str().is_some()) => {
            stages.extend(items.iter().filter_map(Yaml::as_str));
        }
        Some(_) => problems.push(Problem::new("stages", "liste de noms d'étapes attendue")),
        None => problems.push(Problem::new("stages", "liste des étapes absente")),
    }
    stages.push(".post");
    let stage_index = |stage: &str| stages.iter().position(|s| *s == stage);

    for (name, hash) in &jobs.all {
        let name = *name;
        for parent in get(hash, "extends").map(names).unwrap_or_default() {
            if !jobs.by_name.contains_key(parent) {
                problems.push(Problem::new(
                    format!("{name}.extends"),
                    format!("job inconnu : {parent}"),
                ));
            }
        }
        if jobs.extends_too_deep(name, 0) {
            problems.push(Problem::new(
                format!("{name}.extends"),
                "chaîne d'extends circulaire ou trop longue",
            ));
            continue;
        }
        let explicit_stage = jobs.stage(name);
        if is_hidden(name) && explicit_stage.is_none() {
            continue;
        }
        let stage = explicit_stage.unwrap_or("test");
        let Some(index) = stage_index(stage) else {
            problems.push(Problem::new(
                format!("{name}.stage"),
                format!("étape non déclarée dans stages : {stage}"),
            ));
            continue;
        };
        if jobs.effective(name, "script", 0).is_none() && jobs.effective(name, "trigger", 0).is_none() {
            problems.push(Problem::new(name, "aucun script"));
        }
        for dependency in jobs.effective(name, "dependencies", 0).map(names).unwrap_or_default() {
            let earlier = jobs
                .by_name
                .contains_key(dependency)
                .then(|| jobs.stage(dependency).unwrap_or("test"))
                .and_then(stage_index)
                .is_some_and(|i| i < index);
            if is_hidden(dependency) || !earlier {
                problems.push(Problem::new(
                    format!("{name}.dependencies"),
                    format!("{dependency} n'est pas un job d'une étape antérieure"),
                ));
            }
        }
        if let Some(Yaml::Array(needs)) = jobs.effective(name, "needs", 0) {
            for need in needs {
                let target = match need {
                    Yaml::String(target) => Some(target.as_str()),
                    Yaml::Hash(need) => get(need, "job").and_then(Yaml::as_str),
                    _ => None,
                };
                if target.is_none_or(|t| is_hidden(t) || !jobs.by_name.contains_key(t)) {
                    problems.push(Problem::new(
                        format!("{name}.needs"),
                        format!("job inconnu : {}", target.unwrap_or("?")),
                    ));
                }
            }
        }
    }
}

fn check_release_jobs(jobs: &Jobs<'_>, problems: &mut Vec<Problem>) {
    for (name, _) in &jobs.all {
        let name = *name;
        let Some(stage) = jobs.stage(name).filter(|s| RELEASE_STAGES.contains(s)) else {
            continue;
        };
        for keyword in ["only", "except"] {
            if jobs.effective(name, keyword, 0).is_some() {
                problems.push(Problem::new(
                    format!("{name}.{keyword}"),
                    format!("job de l'étape {stage} : {keyword} interdit, rules attendu"),
                ));
            }
        }
        let rules = match jobs.effective(name, "rules", 0) {
            Some(Yaml::Array(rules)) if !rules.is_empty() => rules,
            _ => {
                problems.push(Problem::new(
                    format!("{name}.rules"),
                    format!("job de l'étape {stage} sans rules : il tournerait hors des tags protégés"),
                ));
                continue;
            }
        };
        for (index, rule) in rules.iter().enumerate() {
            let rule = match rule {
                Yaml::Hash(rule) => rule,
                _ => {
                    problems.push(Problem::new(format!("{name}.rules[{index}]"), "règle illisible"));
                    continue;
                }
            };
            if get(rule, "when").and_then(Yaml::as_str) == Some("never") {
                continue;
            }
            let condition = get(rule, "if").and_then(Yaml::as_str);
            if !condition.is_some_and(requires_protected_tag) {
                problems.push(Problem::new(
                    format!("{name}.rules[{index}]"),
                    format!(
                        "job de l'étape {stage} : la règle doit exiger un tag protégé \
                         ($CI_COMMIT_TAG && $CI_COMMIT_REF_PROTECTED == \"true\", sans ||)"
                    ),
                ));
            }
        }
    }
}

/// The jobs that receive the secrets run no build tool nor package manager, in their own scripts, in those they
/// inherit through `extends`, or in the `before_script` and `after_script` of `default`.
fn check_secret_jobs(root: &Hash, jobs: &Jobs<'_>, problems: &mut Vec<Problem>) {
    let default = get(root, "default").and_then(Yaml::as_hash);
    for (name, _) in &jobs.all {
        let name = *name;
        if !jobs
            .effective(name, "environment", 0)
            .is_some_and(declares_release_environment)
        {
            continue;
        }
        for key in ["before_script", "script", "after_script"] {
            let (lines, origin) = match jobs.effective(name, key, 0) {
                Some(lines) => (lines, format!("{name}.{key}")),
                None => match default.and_then(|d| get(d, key)).or_else(|| get(root, key)) {
                    Some(lines) => (lines, format!("{name}.{key} (hérité de default)")),
                    None => continue,
                },
            };
            let texts: Vec<&str> = match lines {
                Yaml::String(text) => vec![text.as_str()],
                Yaml::Array(items) => items.iter().filter_map(Yaml::as_str).collect(),
                _ => Vec::new(),
            };
            for line in texts.iter().flat_map(|t| t.lines()) {
                for (program, _) in shell_commands(line) {
                    if BUILD_TOOLS.contains(&program.as_str()) {
                        problems.push(Problem::new(
                            &origin,
                            format!(
                                "job de l'environnement {RELEASE_ENVIRONMENT} : {program} interdit, il exécuterait du \
                                 code tiers avec les secrets (construire l'outil dans un job sans l'environnement)"
                            ),
                        ));
                    }
                }
            }
        }
    }
}

fn check_texts(walked: &Walk<'_>, doc: &str, forms: &[&str], report: &mut Report) {
    let mut variables: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, path) in &walked.defined {
        if let Some(marker) = TRACE_MARKERS.iter().find(|m| name == *m) {
            report.problems.push(Problem::new(
                path,
                format!("trace des commandes interdite ({marker}) : elle afficherait les secrets"),
            ));
        }
        variables.entry(name).or_insert(path);
    }
    for text in &walked.texts {
        for marker in TRACE_MARKERS {
            if text.text.contains(marker) {
                report.problems.push(Problem::new(
                    &text.path,
                    format!("trace des commandes interdite ({marker}) : elle afficherait les secrets"),
                ));
            }
        }
        for line in text.text.lines() {
            for (program, words) in shell_commands(line).into_iter().filter(|_| is_script(&text.path)) {
                if program != TOOL {
                    continue;
                }
                report.commands += 1;
                let known = words.first().is_some_and(|c| tool_commands::options(c).is_some());
                if !known {
                    report.problems.push(Problem::new(
                        &text.path,
                        format!("commande inconnue : {TOOL} {}", words.join(" ")),
                    ));
                }
            }
            for words in xtask_citations(line) {
                report.commands += 1;
                if !is_known_command(&words, forms) {
                    report.problems.push(Problem::new(
                        &text.path,
                        format!("commande inconnue : {XTASK} {}", words.join(" ")),
                    ));
                }
            }
        }
        for name in variable_references(text.text) {
            variables.entry(name).or_insert(&text.path);
        }
    }
    report.variables = variables.len();
    let undocumented: BTreeSet<(&str, &str)> = variables
        .iter()
        .filter(|(name, _)| !is_documented(doc, name))
        .map(|(name, path)| (*path, *name))
        .collect();
    for (path, name) in undocumented {
        report.problems.push(Problem::new(
            path,
            format!("variable non documentée dans docs/release.md : `{name}`"),
        ));
    }
}

#[cfg(test)]
mod tests;
