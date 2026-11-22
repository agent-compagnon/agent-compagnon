use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TREE: AtomicUsize = AtomicUsize::new(0);

/// Throw-away repository layout in the temporary directory, removed on drop.
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "xtask-trace-{}-{}",
            std::process::id(),
            NEXT_TREE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(SPECS_DIR)).unwrap();
        let tree = Self { root };
        tree.write(INDEX_FILE, &readme(""));
        tree
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap()
    }

    fn analyze(&self, code_files: &[&str]) -> Analysis {
        let files: Vec<String> = code_files.iter().map(|f| (*f).to_owned()).collect();
        analyze(&self.root, &files).unwrap()
    }

    /// Brings the index in line with the specs, so that a test only sees the errors it provokes.
    fn sync_index(&self) {
        let analysis = self.analyze(&[]);
        write_index(&self.root, &analysis).unwrap();
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn readme(index: &str) -> String {
    format!("# Specs\n\n{INDEX_BEGIN} — généré, ne pas modifier -->\n{index}{INDEX_END}\n\nSuite.\n")
}

fn spec(id: &str, status: &str, deps: &str, body: &str) -> String {
    format!(
        "---\nid: \"{id}\"\ntitre: \"Spec {id}\"\nstatut: \"{status}\"\ndepend_de: [{deps}]\n---\n\n# {id}\n\n## Tests\n\n{body}"
    )
}

/// A code citation, built at run time so that this file does not cite the fixture identifiers itself.
fn cite(ids: &str) -> String {
    format!("#[test]\nfn t() {{}}\n// {CITATION_MARKER} : {ids}\n")
}

fn messages(issues: &[Issue]) -> String {
    issues.iter().map(|i| format!("  {i}\n")).collect()
}

#[track_caller]
fn assert_issue(issues: &[Issue], location: &str, id: Option<&str>, fragment: &str) {
    assert!(
        issues
            .iter()
            .any(|i| i.location.starts_with(location) && i.id.as_deref() == id && i.message.contains(fragment)),
        "no issue at `{location}` ({id:?}) containing `{fragment}`; got:\n{}",
        messages(issues)
    );
}

#[track_caller]
fn assert_clean(analysis: &Analysis) {
    assert!(
        analysis.errors.is_empty(),
        "unexpected errors:\n{}",
        messages(&analysis.errors)
    );
}

// --- Spec format --------------------------------------------------------------------------------------------------

// Exigence : EX-030-01
#[test]
fn well_formed_spec_is_read() {
    let tree = Tree::new();
    tree.write(
        "specs/100-base.md",
        "---\nid: \"100\"\ntitre: \"Base\"\nstatut: \"backlog\"\ndepend_de: []\n---\n",
    );
    tree.write(
        "specs/101-feature.md",
        "---\nid: \"101\"\ntitre: \"Fonction `x`, d'exemple\"\netape: \"2a\"\nstatut: \"en cours\"\n\
         depend_de: [\"100\"]\nsuivi: \"github#1\"\n---\n\n## Tests\n\n\
         - **EX-101-01** — automatique.\n  - **EX-101-02** (manuel) — à la main.\n\
         - ~~**EX-101-03** — retiré.~~ (retiré le 2026-11-22)\n- Pas un critère : `**EX-101-09**`.\n\
         ```\n- **EX-101-08** — dans un bloc de code.\n```\n",
    );
    let analysis = tree.analyze(&[]);
    let spec = analysis.specs.iter().find(|s| s.id == "101").unwrap();
    assert_eq!(spec.file, "101-feature.md");
    assert_eq!(spec.title, "Fonction `x`, d'exemple");
    assert_eq!(spec.stage.as_deref(), Some("2a"));
    assert_eq!(spec.status, Status::InProgress);
    assert_eq!(spec.depends_on, ["100"]);
    let criteria: Vec<(&str, bool, bool)> = spec
        .criteria
        .iter()
        .map(|c| (c.id.as_str(), c.manual, c.withdrawn))
        .collect();
    assert_eq!(
        criteria,
        [
            ("EX-101-01", false, false),
            ("EX-101-02", true, false),
            ("EX-101-03", false, true)
        ]
    );
    assert_eq!(spec.criteria[0].line, 12);
    assert!(!analysis.format_errors);
}

// Exigence : EX-030-02
#[test]
fn missing_header_is_reported() {
    let tree = Tree::new();
    tree.write("specs/100-base.md", "# 100 — sans en-tête\n");
    let analysis = tree.analyze(&[]);
    assert_issue(&analysis.errors, "specs/100-base.md", None, "en-tête absent");
    assert!(analysis.format_errors);

    tree.write("specs/100-base.md", "---\nid: \"100\"\n");
    assert_issue(&tree.analyze(&[]).errors, "specs/100-base.md", None, "fermeture");
}

// Exigence : EX-030-02
#[test]
fn id_must_match_file_name() {
    let tree = Tree::new();
    tree.write("specs/100-base.md", &spec("101", "prévue", "", ""));
    assert_issue(
        &tree.analyze(&[]).errors,
        "specs/100-base.md",
        None,
        "différent du numéro du fichier",
    );
}

// Exigence : EX-030-02
#[test]
fn unknown_status_is_reported() {
    let tree = Tree::new();
    tree.write("specs/100-base.md", &spec("100", "finie", "", ""));
    assert_issue(
        &tree.analyze(&[]).errors,
        "specs/100-base.md",
        None,
        "statut inconnu « finie »",
    );
}

// Exigence : EX-030-02
#[test]
fn unknown_dependency_is_reported() {
    let tree = Tree::new();
    tree.write("specs/100-base.md", &spec("100", "prévue", "\"099\"", ""));
    assert_issue(
        &tree.analyze(&[]).errors,
        "specs/100-base.md",
        None,
        "spec inexistante « 099 »",
    );
}

// Exigence : EX-030-02
#[test]
fn header_syntax_is_strict() {
    let tree = Tree::new();
    for (header, fragment) in [
        ("id: 100", "valeur de `id` invalide"),
        ("id: \"100\"\nid: \"100\"", "en double"),
        ("id: \"100\"\ncouleur: \"bleu\"", "clé inconnue `couleur`"),
        ("id: \"100\"\ndepend_de: \"001\"", "doit être une liste"),
    ] {
        let mut text = spec("100", "prévue", "", "");
        text = text.replacen("id: \"100\"", header, 1);
        if header.contains("depend_de") {
            text = text.replacen("depend_de: []\n", "", 1);
        }
        tree.write("specs/100-base.md", &text);
        assert_issue(&tree.analyze(&[]).errors, "specs/100-base.md", None, fragment);
    }
    tree.write("specs/1-base.md", "");
    assert_issue(
        &tree.analyze(&[]).errors,
        "specs/1-base.md",
        None,
        "nom de fichier invalide",
    );
}

// Exigence : EX-030-03
#[test]
fn malformed_duplicate_or_foreign_criteria_are_errors() {
    let tree = Tree::new();
    tree.write(
        "specs/100-base.md",
        &spec(
            "100",
            "prévue",
            "",
            "- **EX-100-01** — un.\n- **EX-100-01** — encore.\n- **EX-100-2** — mal formé.\n\
             - **EX-101-01** — numéro d'une autre spec.\n",
        ),
    );
    let errors = tree.analyze(&[]).errors;
    assert_issue(&errors, "specs/100-base.md:", Some("EX-100-01"), "en double");
    assert_issue(&errors, "specs/100-base.md:", Some("EX-100-2"), "mal formé");
    assert_issue(&errors, "specs/100-base.md:", Some("EX-101-01"), "autre spec");
}

// Exigence : EX-030-03
#[test]
fn duplicate_across_specs_is_an_error() {
    let tree = Tree::new();
    tree.write("specs/100-a.md", &spec("100", "prévue", "", "- **EX-100-01** — un.\n"));
    tree.write(
        "specs/101-b.md",
        &spec("101", "prévue", "", "- **EX-100-01** — copie.\n"),
    );
    let errors = tree.analyze(&[]).errors;
    assert_issue(
        &errors,
        "specs/101-b.md:",
        Some("EX-100-01"),
        "déjà dans specs/100-a.md",
    );
}

// --- Coverage -----------------------------------------------------------------------------------------------------

fn implemented_tree() -> Tree {
    let tree = Tree::new();
    tree.write(
        "specs/100-base.md",
        &spec(
            "100",
            "implémentée",
            "",
            "- **EX-100-01** — auto.\n- **EX-100-02** — auto.\n- **EX-100-03** (manuel) — à la main.\n",
        ),
    );
    tree.sync_index();
    tree
}

// Exigence : EX-030-04
#[test]
fn implemented_spec_requires_every_automatic_criterion_in_code() {
    let tree = implemented_tree();
    tree.write("crates/a/src/lib.rs", &cite("EX-100-01"));
    tree.write("docs/recette.md", "- [ ] EX-100-03 — vérifier.\n");
    let analysis = tree.analyze(&["crates/a/src/lib.rs"]);
    assert_eq!(analysis.errors.len(), 1, "{}", messages(&analysis.errors));
    assert_issue(&analysis.errors, "specs/100-base.md:", Some("EX-100-02"), "non couvert");

    tree.write("crates/b/src/lib.rs", &cite("EX-100-02"));
    let analysis = tree.analyze(&["crates/a/src/lib.rs", "crates/b/src/lib.rs"]);
    assert_clean(&analysis);
    assert_eq!(analysis.coverage(&analysis.specs[0]), (3, 3));
}

// Exigence : EX-030-04
#[test]
fn several_identifiers_on_one_citation_line() {
    let tree = implemented_tree();
    tree.write("crates/a/src/lib.rs", &cite("EX-100-01, EX-100-02"));
    tree.write("docs/recette.md", "- [ ] EX-100-03\n");
    assert_clean(&tree.analyze(&["crates/a/src/lib.rs"]));
}

// Exigence : EX-030-05
#[test]
fn manual_criterion_must_be_in_the_recette() {
    let tree = implemented_tree();
    tree.write("crates/a/src/lib.rs", &cite("EX-100-01, EX-100-02, EX-100-03"));
    let analysis = tree.analyze(&["crates/a/src/lib.rs"]);
    assert_eq!(analysis.errors.len(), 1, "{}", messages(&analysis.errors));
    assert_issue(
        &analysis.errors,
        "specs/100-base.md:",
        Some("EX-100-03"),
        "docs/recette.md",
    );

    tree.write("docs/recette.md", "## 100\n\n- [ ] **EX-100-03** — vérifier.\n");
    assert_clean(&tree.analyze(&["crates/a/src/lib.rs"]));
}

// Exigence : EX-030-05
#[test]
fn automatic_criterion_in_the_recette_only_is_not_covered() {
    let tree = implemented_tree();
    tree.write("docs/recette.md", "- [ ] EX-100-01, EX-100-02, EX-100-03\n");
    let errors = tree.analyze(&[]).errors;
    assert_issue(&errors, "specs/100-base.md:", Some("EX-100-01"), "non couvert");
    assert_issue(&errors, "specs/100-base.md:", Some("EX-100-02"), "non couvert");
}

// Exigence : EX-030-06
#[test]
fn in_progress_spec_only_warns() {
    let tree = Tree::new();
    tree.write(
        "specs/100-base.md",
        &spec(
            "100",
            "en cours",
            "",
            "- **EX-100-01** — auto.\n- **EX-100-02** (manuel) — main.\n",
        ),
    );
    tree.sync_index();
    let analysis = tree.analyze(&[]);
    assert_clean(&analysis);
    assert_issue(
        &analysis.warnings,
        "specs/100-base.md:",
        Some("EX-100-01"),
        "non couvert",
    );
    assert_issue(
        &analysis.warnings,
        "specs/100-base.md:",
        Some("EX-100-02"),
        "non couvert",
    );
    assert_eq!(print_outcome(&analysis), ExitCode::SUCCESS);
}

// Exigence : EX-030-06
#[test]
fn other_statuses_require_nothing() {
    let tree = Tree::new();
    for (id, status) in [
        ("100", "prévue"),
        ("101", "proposée"),
        ("102", "backlog"),
        ("103", "à la demande"),
    ] {
        tree.write(
            &format!("specs/{id}-x.md"),
            &spec(id, status, "", &format!("- **EX-{id}-01** — auto.\n")),
        );
    }
    tree.sync_index();
    let analysis = tree.analyze(&[]);
    assert_clean(&analysis);
    assert!(analysis.warnings.is_empty());
}

// Exigence : EX-030-07
#[test]
fn unknown_cited_identifier_names_the_file() {
    let tree = implemented_tree();
    tree.write("crates/a/src/lib.rs", &cite("EX-100-01, EX-100-02, EX-100-09"));
    tree.write("docs/recette.md", "Intro.\n\n- [ ] EX-100-03\n- [ ] EX-200-01\n");
    let errors = tree.analyze(&["crates/a/src/lib.rs"]).errors;
    assert_issue(&errors, "crates/a/src/lib.rs:3", Some("EX-100-09"), "absent des specs");
    assert_issue(&errors, "docs/recette.md:4", Some("EX-200-01"), "absent des specs");
    assert_eq!(errors.len(), 2, "{}", messages(&errors));
}

// Exigence : EX-030-07
#[test]
fn malformed_citation_is_an_error() {
    let tree = implemented_tree();
    tree.write("crates/a/src/lib.rs", &cite("EX-100-1"));
    let errors = tree.analyze(&["crates/a/src/lib.rs"]).errors;
    assert_issue(&errors, "crates/a/src/lib.rs:3", Some("EX-100-1"), "mal formé");
}

// Exigence : EX-030-07
#[test]
fn only_the_citation_comment_counts_in_code() {
    let tree = implemented_tree();
    tree.write(
        "crates/a/src/lib.rs",
        "const FIXTURE: &str = \"EX-100-01 EX-999-01\";\n// see EX-100-02\n",
    );
    let analysis = tree.analyze(&["crates/a/src/lib.rs"]);
    assert!(analysis.citations.is_empty());
    assert_issue(&analysis.errors, "specs/100-base.md:", Some("EX-100-01"), "non couvert");
}

// Exigence : EX-030-08
#[test]
fn withdrawn_criterion_is_not_required_but_must_not_be_cited() {
    let tree = Tree::new();
    tree.write(
        "specs/100-base.md",
        &spec(
            "100",
            "implémentée",
            "",
            "- **EX-100-01** — auto.\n- ~~**EX-100-02** — retiré.~~ (retiré le 2026-11-22)\n",
        ),
    );
    tree.sync_index();
    tree.write("crates/a/src/lib.rs", &cite("EX-100-01"));
    let analysis = tree.analyze(&["crates/a/src/lib.rs"]);
    assert_clean(&analysis);
    assert_eq!(analysis.coverage(&analysis.specs[0]), (1, 1));
    assert!(analysis.expected_index.contains("| implémentée | — | 1 |"));

    tree.write("crates/b/src/lib.rs", &cite("EX-100-02"));
    let errors = tree.analyze(&["crates/a/src/lib.rs", "crates/b/src/lib.rs"]).errors;
    assert_issue(&errors, "crates/b/src/lib.rs:3", Some("EX-100-02"), "retiré");
}

// --- Index --------------------------------------------------------------------------------------------------------

// Exigence : EX-030-09
#[test]
fn stale_index_is_an_error_and_write_index_fixes_it() {
    let tree = Tree::new();
    tree.write(
        "specs/100-base.md",
        &spec("100", "prévue", "", "- **EX-100-01** — auto.\n"),
    );
    let analysis = tree.analyze(&[]);
    assert_issue(&analysis.errors, INDEX_FILE, None, "index périmé");

    assert!(write_index(&tree.root, &analysis).unwrap());
    let readme = tree.read(INDEX_FILE);
    assert_eq!(
        readme,
        readme_with("| [100](100-base.md) | — | Spec 100 | prévue | — | 1 |\n")
    );
    let analysis = tree.analyze(&[]);
    assert_clean(&analysis);
    assert!(
        !write_index(&tree.root, &analysis).unwrap(),
        "second write must be a no-op"
    );
}

fn readme_with(rows: &str) -> String {
    readme(&format!("{INDEX_HEADER}{rows}"))
}

// Exigence : EX-030-09
#[test]
fn index_follows_header_changes() {
    let tree = Tree::new();
    tree.write("specs/100-base.md", &spec("100", "prévue", "", ""));
    tree.sync_index();
    tree.write("specs/100-base.md", &spec("100", "en cours", "", ""));
    assert_issue(&tree.analyze(&[]).errors, INDEX_FILE, None, "index périmé");
}

// Exigence : EX-030-09
#[test]
fn index_is_not_rewritten_while_a_header_is_invalid() {
    let tree = Tree::new();
    tree.write("specs/100-base.md", &spec("100", "prévue", "", ""));
    tree.write("specs/101-bad.md", "pas d'en-tête\n");
    let analysis = tree.analyze(&[]);
    assert!(write_index(&tree.root, &analysis).is_err());
    assert_eq!(tree.read(INDEX_FILE), readme(""));
}

// Exigence : EX-030-09
#[test]
fn missing_index_markers_are_an_error() {
    let tree = Tree::new();
    tree.write(INDEX_FILE, "# Specs\n");
    assert_issue(&tree.analyze(&[]).errors, INDEX_FILE, None, "marqueurs");
}

// Exigence : EX-030-09
#[test]
fn index_rows_use_stage_title_and_dependency_ranges() {
    let deps = |list: &[&str]| format_dependencies(&list.iter().map(|d| (*d).to_owned()).collect::<Vec<_>>());
    assert_eq!(deps(&[]), "—");
    assert_eq!(deps(&["005", "006", "007"]), "005, 006, 007");
    assert_eq!(deps(&["007", "010", "011", "012"]), "007, 010, 011, 012");
    assert_eq!(
        deps(&["001", "002", "003", "004", "005", "006", "007", "008", "009", "010"]),
        "001 à 010"
    );
    assert_eq!(deps(&["001", "003", "004", "005", "006"]), "001, 003 à 006");
}

// --- Code files ---------------------------------------------------------------------------------------------------

#[test]
fn only_code_roots_are_scanned() {
    for path in [
        "crates/core/src/lib.rs",
        "app/src/main.rs",
        "xtask/src/main.rs",
        "ui/src/App.vue",
    ] {
        assert!(is_code_path(path), "{path}");
    }
    for path in [
        "specs/001-x.md",
        "docs/recette.md",
        "README.md",
        "ui/node_modules/x/index.js",
        "ui/dist/app.js",
        "crates/core/target/x.rs",
    ] {
        assert!(!is_code_path(path), "{path}");
    }
}

// --- Real repository ----------------------------------------------------------------------------------------------

// Exigence : EX-030-10
#[test]
fn real_repository_is_aligned_and_checked_by_ci() {
    let root = crate::workspace_root();
    let files = tracked_code_files(&root).unwrap();
    assert!(files.iter().any(|f| f == "xtask/src/trace/tests.rs"), "{files:?}");
    let analysis = analyze(&root, &files).unwrap();
    assert_clean(&analysis);

    let steps = crate::ci_steps(&root);
    let position = |name: &str| steps.iter().position(|s| s.name == name);
    let trace = position("traçabilité").expect("ci has no traceability step");
    assert!(trace > position("tests (hôte)").unwrap());
    assert_eq!(steps[trace].args.last().map(String::as_str), Some("trace"));
}
