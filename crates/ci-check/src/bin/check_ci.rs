//! `check-ci`, launched by `cargo xtask check-ci`: checks `.gitlab-ci.yml` against `docs/release.md` and the forms of
//! xtask (spec 021). Exit codes: 0, 1 (problems found or file unreadable), 2 (arguments).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;

use mcp_proxy_ci_check::{XTASK_FORMS, check};

const USAGE: &str = "usage : check-ci --ci-file <.gitlab-ci.yml> --doc <docs/release.md>";

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(Path::new(path)).map_err(|e| format!("{path} illisible : {e}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (ci_file, doc_file) = match args.as_slice() {
        [a, ci, b, doc] if a == "--ci-file" && b == "--doc" => (ci, doc),
        [b, doc, a, ci] if a == "--ci-file" && b == "--doc" => (ci, doc),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (ci, doc) = match (read(ci_file), read(doc_file)) {
        (Ok(ci), Ok(doc)) => (ci, doc),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("check-ci : ÉCHEC : {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = check(&ci, &doc, XTASK_FORMS);
    if report.problems.is_empty() {
        println!(
            "check-ci : OK ({ci_file} : {} jobs, {} commandes cargo xtask, {} variables documentées dans {doc_file})",
            report.jobs, report.commands, report.variables
        );
        return ExitCode::SUCCESS;
    }
    for problem in &report.problems {
        eprintln!("{ci_file} : {} : {}", problem.location, problem.message);
    }
    eprintln!("check-ci : ÉCHEC ({} problème(s))", report.problems.len());
    ExitCode::FAILURE
}
