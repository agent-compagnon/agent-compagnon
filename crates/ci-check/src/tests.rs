use super::*;

const DELIVERED_CI: &str = include_str!("../../../.gitlab-ci.yml");
const DELIVERED_DOC: &str = include_str!("../../../docs/release.md");

/// A small pipeline that passes, to be broken one way at a time.
const BASE: &str = r#"
stages: [lint, build, sign]
variables:
  MODE: "a"
workflow:
  rules:
    - if: $CI_COMMIT_TAG
.release:
  rules:
    - if: $CI_COMMIT_TAG && $CI_COMMIT_REF_PROTECTED == "true"
lint:
  stage: lint
  script:
    - cargo xtask ci lint
build:
  stage: build
  extends: .release
  script:
    - cargo xtask check-tag "$CI_COMMIT_TAG" && echo done
sign:
  stage: sign
  extends: .release
  dependencies: [build]
  script:
    - cargo xtask sign --exe dist/mcp-proxy.exe --key-env KEY; if (-not $?) { exit 1 }
"#;

const BASE_DOC: &str = "`MODE`, `CI_COMMIT_TAG`, `CI_COMMIT_REF_PROTECTED`.";

const FORMS: &[&str] = &["ci", "ci lint", "check-tag …", "sign …"];

fn problems(ci: &str) -> Vec<Problem> {
    check(ci, BASE_DOC, FORMS).problems
}

fn messages(ci: &str) -> String {
    problems(ci)
        .iter()
        .map(|p| format!("{} : {}", p.location, p.message))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_base_fixture_passes() {
    assert_eq!(problems(BASE), Vec::new());
}

// Exigence : EX-021-01
#[test]
fn the_delivered_pipeline_passes() {
    let report = check(DELIVERED_CI, DELIVERED_DOC, XTASK_FORMS);
    assert_eq!(report.problems, Vec::new());
    assert!(report.jobs >= 7, "{}", report.jobs);
    assert!(report.commands >= 7, "{}", report.commands);
}

// Exigence : EX-021-01
#[test]
fn invalid_yaml_is_refused() {
    for ci in [
        "stages: [lint\nlint:\n  script: [x]\n",
        "lint:\n  script: [a]\n   stage: lint\n",
        "lint:\n\tscript: [a]\n",
        "stages: [lint]\nlint:\n  script: [a]\nlint:\n  script: [b]\n",
        "stages: [lint]\nlint:\n  script: *missing\n",
        "stages: [lint]\n---\nstages: [test]\n",
        "- lint\n- test\n",
        "stages: [lint]\nlint: just text\n",
    ] {
        assert!(!problems(ci).is_empty(), "accepté : {ci:?}");
    }
    let message = messages("stages: [lint\nlint:\n  script: [x]\n");
    assert!(
        message.contains("YAML invalide") && message.contains("ligne"),
        "{message}"
    );
}

// Exigence : EX-021-01
#[test]
fn an_unknown_xtask_command_is_refused() {
    for (cited, shown) in [
        ("cargo xtask deploy", "cargo xtask deploy"),
        ("cargo xtask ci build", "cargo xtask ci build"),
        ("cargo xtask ci lint --fix", "cargo xtask ci lint --fix"),
        ("cargo xtask", "cargo xtask "),
        ("cd ui && cargo xtask chek-tag v1", "cargo xtask chek-tag v1"),
    ] {
        let ci = BASE.replace("cargo xtask ci lint", cited);
        let message = messages(&ci);
        assert!(
            message.contains(&format!("commande inconnue : {shown}")),
            "{cited} : {message}"
        );
    }
    // A longer word that merely starts like the command is not a citation of it.
    assert_eq!(
        problems(&BASE.replace("cargo xtask ci lint", "cargo xtasks ci")),
        Vec::new()
    );
}

// Exigence : EX-021-01
#[test]
fn an_undocumented_variable_is_refused() {
    for (line, name) in [
        ("echo $UNDOCUMENTED", "UNDOCUMENTED"),
        ("echo \"${UNDOCUMENTED#v}\"", "UNDOCUMENTED"),
        ("Write-Output $env:UNDOCUMENTED", "UNDOCUMENTED"),
        ("Write-Output $Env:UNDOCUMENTED", "UNDOCUMENTED"),
        ("echo $$UNDOCUMENTED", "UNDOCUMENTED"),
    ] {
        let ci = BASE.replace("cargo xtask ci lint", line);
        let message = messages(&ci);
        assert!(
            message.contains(&format!("variable non documentée dans docs/release.md : `{name}`")),
            "{line} : {message}"
        );
        assert!(message.starts_with("lint.script[0]"), "{message}");
    }
    let defined = BASE.replace("  MODE: \"a\"", "  MODE: \"a\"\n  OTHER: \"b\"");
    assert!(messages(&defined).contains("variables.OTHER : variable non documentée"));
    let in_rule = BASE.replace("- if: $CI_COMMIT_TAG\n.release", "- if: $SCHEDULE_ONLY\n.release");
    assert!(messages(&in_rule).contains("`SCHEDULE_ONLY`"));
    // Documented only as `$NAME`, or in plain text: not documented.
    let doc = format!("{BASE_DOC} `$EXTRA` EXTRA");
    let ci = BASE.replace("cargo xtask ci lint", "echo $EXTRA");
    assert!(!check(&ci, &doc, FORMS).problems.is_empty());
}

#[test]
fn variable_references_are_found_in_every_syntax() {
    assert_eq!(
        variable_references("a $A ${B} ${C#v} $env:D ${env:E} $1 $? $(x) $$F $_G"),
        vec!["A", "B", "C", "D", "E", "F", "_G"]
    );
    assert_eq!(variable_references("$"), Vec::<&str>::new());
    assert_eq!(variable_references("é$é ${"), Vec::<&str>::new());
}

#[test]
fn xtask_citations_stop_at_the_end_of_the_shell_command() {
    assert_eq!(
        xtask_citations("cargo xtask ci lint; if (-not $?) { exit 1 }"),
        vec![vec!["ci", "lint"]]
    );
    assert_eq!(
        xtask_citations("(cargo xtask trace) && cargo xtask sign --exe a | tee x"),
        vec![vec!["trace"], vec!["sign", "--exe", "a"]]
    );
    assert_eq!(xtask_citations("mycargo xtask ci"), Vec::<Vec<&str>>::new());
    assert!(is_known_command(&["check-tag"], FORMS));
    assert!(!is_known_command(&["ci", "lint", "x"], FORMS));
}

// Exigence : EX-021-07
#[test]
fn the_structure_of_the_pipeline_is_checked() {
    for (from, to, expected) in [
        (
            "  stage: lint\n",
            "  stage: deploy\n",
            "lint.stage : étape non déclarée dans stages : deploy",
        ),
        (
            "  extends: .release\n  script:\n    - cargo xtask check-tag",
            "  extends: .relase\n  script:\n    - cargo xtask check-tag",
            "build.extends : job inconnu : .relase",
        ),
        (
            "dependencies: [build]",
            "dependencies: [buld]",
            "sign.dependencies : buld n'est pas un job d'une étape antérieure",
        ),
        (
            "dependencies: [build]",
            "dependencies: [sign]",
            "sign.dependencies : sign n'est pas un job d'une étape antérieure",
        ),
        (
            "dependencies: [build]",
            "dependencies: [.release]",
            "sign.dependencies : .release n'est pas un job",
        ),
        (
            "dependencies: [build]",
            "needs: [{job: bild}]",
            "sign.needs : job inconnu : bild",
        ),
        ("  script:\n    - cargo xtask ci lint\n", "", "lint : aucun script"),
        ("stages: [lint, build, sign]\n", "", "stages : liste des étapes absente"),
        (
            "stages: [lint, build, sign]\n",
            "include: other.yml\nstages: [lint, build, sign]\n",
            "include : fichier inclus",
        ),
        (
            ".release:\n",
            ".release:\n  extends: build\n",
            "chaîne d'extends circulaire",
        ),
    ] {
        let ci = BASE.replacen(from, to, 1);
        assert_ne!(ci, BASE, "{from:?}");
        let message = messages(&ci);
        assert!(message.contains(expected), "{expected} :\n{message}");
    }
}

// Exigence : EX-021-08
#[test]
fn release_jobs_run_on_protected_tags_only() {
    let protected = r#"$CI_COMMIT_TAG && $CI_COMMIT_REF_PROTECTED == "true""#;
    for condition in [
        "$CI_COMMIT_TAG",
        r#"$CI_COMMIT_REF_PROTECTED == "true""#,
        r#"$CI_COMMIT_TAG || $CI_COMMIT_REF_PROTECTED == "true""#,
        r#"$CI_COMMIT_TAG && $CI_COMMIT_REF_PROTECTED == "true" || $CI_PIPELINE_SOURCE == "push""#,
        r#"$CI_COMMIT_TAG == null && $CI_COMMIT_REF_PROTECTED == "true""#,
        r#"$CI_COMMIT_TAG_MESSAGE && $CI_COMMIT_REF_PROTECTED == "true""#,
        r#"$CI_COMMIT_TAG && $CI_COMMIT_REF_PROTECTED == "false""#,
    ] {
        assert!(!requires_protected_tag(condition), "{condition}");
        let ci = BASE.replace(protected, condition);
        let message = messages(&ci);
        assert!(
            message.contains("build.rules[0] : job de l'étape build"),
            "{condition} :\n{message}"
        );
        assert!(
            message.contains("sign.rules[0] : job de l'étape sign"),
            "{condition} :\n{message}"
        );
    }
    for condition in [
        protected,
        r#"($CI_COMMIT_TAG) && $CI_COMMIT_REF_PROTECTED=="true" && $MODE == "a""#,
        r#"$CI_COMMIT_TAG =~ /^v\d/ && $CI_COMMIT_REF_PROTECTED == 'true'"#,
    ] {
        assert!(requires_protected_tag(condition), "{condition}");
    }

    // Without rules, with only/except, or with a rule that only says "never" first: refused.
    let no_rules = BASE.replace("  extends: .release\n  dependencies", "  dependencies");
    assert!(messages(&no_rules).contains("sign.rules : job de l'étape sign sans rules"));
    let only = BASE.replace("  extends: .release\n  dependencies", "  only: [tags]\n  dependencies");
    assert!(messages(&only).contains("sign.only : job de l'étape sign : only interdit"));
    let never_then_open = BASE.replace(
        "  extends: .release\n  dependencies",
        "  rules:\n    - if: $MODE == \"b\"\n      when: never\n    - when: always\n  dependencies",
    );
    assert!(messages(&never_then_open).contains("sign.rules[1]"));
    let never_then_tag = BASE.replace(
        "  extends: .release\n  dependencies",
        &format!("  rules:\n    - if: $MODE == \"b\"\n      when: never\n    - if: {protected}\n  dependencies"),
    );
    assert_eq!(problems(&never_then_tag), Vec::new());

    // A job of a release stage that is hidden but has its stage is checked too: enabling it must stay safe.
    let hidden = format!("{BASE}.authenticode:\n  stage: sign\n  script: [signtool]\n");
    assert!(messages(&hidden).contains(".authenticode.rules"));
}

// Exigence : EX-021-08
#[test]
fn the_delivered_release_jobs_lose_their_protection_if_the_rule_is_weakened() {
    let weakened = DELIVERED_CI.replace(r#" && $CI_COMMIT_REF_PROTECTED == "true""#, "");
    assert_ne!(weakened, DELIVERED_CI);
    let report = check(&weakened, DELIVERED_DOC, XTASK_FORMS);
    for job in ["build:windows", "build:linux-xwin", "sign", "publish", ".authenticode"] {
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.location.starts_with(&format!("{job}.rules"))),
            "{job} : {:?}",
            report.problems
        );
    }
}

// Exigence : EX-021-08
#[test]
fn a_trace_of_the_commands_is_refused() {
    for line in [
        "set -x",
        "set -o xtrace && make",
        "Set-PSDebug -Trace 1",
        "export CI_DEBUG_TRACE=true",
    ] {
        let ci = BASE.replace("cargo xtask ci lint", line);
        assert!(messages(&ci).contains("trace des commandes interdite"), "{line}");
    }
    let in_variables = BASE.replace("  MODE: \"a\"", "  MODE: \"a\"\n  CI_DEBUG_TRACE: \"true\"");
    assert!(messages(&in_variables).contains("trace des commandes interdite"));
}
