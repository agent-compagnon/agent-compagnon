//! Project automation, run with `cargo xtask <command>`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

mod icons;
mod trace;

const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci                    run every check: fmt, clippy (host), tests, ui/ (install, lint, typecheck, tests, build),
                        clippy (Windows target, which reads ui/dist), traceability
  ci lint               fmt, both clippy runs, ui/ lint and typecheck (ui/ is built before the Windows run)
  ci test               tests, ui/ tests and build, traceability
  trace                 check that specs, tests and docs/recette.md agree
  trace --write-index   regenerate the index of specs/README.md, then check
  trace --report        print the criterion -> citing files matrix, then check
  icons                 draw the icons of app/icons/ (application, notification area)
  snapshot-catalog --url <url> --out config/catalog/<mcp>.json [--header-env NAME]
                        write the tools of an MCP server as an embedded catalogue snapshot; the bearer token, if
                        any, is read from the environment variable NAME and never printed";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Lint,
    Test,
}

struct Step {
    name: String,
    groups: Vec<Group>,
    program: String,
    args: Vec<String>,
    dir: PathBuf,
}

impl Step {
    fn new(name: &str, groups: &[Group], program: &str, args: &[&str], dir: &Path) -> Self {
        Self {
            name: name.to_owned(),
            groups: groups.to_vec(),
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            dir: dir.to_owned(),
        }
    }
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}

/// Runs this very binary, already built, rather than going through cargo again.
fn trace_step(root: &Path) -> Step {
    match std::env::current_exe() {
        Ok(exe) => Step::new("traçabilité", &[Group::Test], &exe.to_string_lossy(), &["trace"], root),
        Err(_) => {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
            Step::new("traçabilité", &[Group::Test], &cargo, &["xtask", "trace"], root)
        }
    }
}

fn ci_steps(root: &Path) -> Vec<Step> {
    use Group::{Lint, Test};
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut steps = vec![
        Step::new("fmt", &[Lint], &cargo, &["fmt", "--all", "--check"], root),
        Step::new(
            "clippy (hôte)",
            &[Lint],
            &cargo,
            &["clippy", "--all-targets", "--", "-D", "warnings"],
            root,
        ),
        Step::new("tests (hôte)", &[Test], &cargo, &["test"], root),
    ];
    // `tauri::generate_context!` reads `ui/dist` when `app/` is compiled: ui/ is built before the Windows run.
    let ui = root.join("ui");
    if ui.join("package.json").is_file() {
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        for (name, groups, args) in [
            ("ui: npm ci", &[Lint, Test][..], &["ci", "--no-audit", "--no-fund"][..]),
            ("ui: lint", &[Lint][..], &["run", "lint"][..]),
            ("ui: typecheck", &[Lint][..], &["run", "typecheck"][..]),
            ("ui: test", &[Test][..], &["test"][..]),
            ("ui: build", &[Lint, Test][..], &["run", "build"][..]),
        ] {
            steps.push(Step::new(name, groups, npm, args, &ui));
        }
    }
    steps.push(Step::new(
        "clippy (Windows)",
        &[Lint],
        &cargo,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--target",
            WINDOWS_TARGET,
            "--",
            "-D",
            "warnings",
        ],
        root,
    ));
    steps.push(trace_step(root));
    steps
}

fn format_duration(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

fn run_ci(groups: &[Group]) -> ExitCode {
    let root = workspace_root();
    let steps: Vec<Step> = ci_steps(&root)
        .into_iter()
        .filter(|s| s.groups.iter().any(|g| groups.contains(g)))
        .collect();
    let started = Instant::now();
    let mut done: Vec<(String, Duration)> = Vec::new();

    for step in &steps {
        println!("\n==> {} : {} {}", step.name, step.program, step.args.join(" "));
        let step_start = Instant::now();
        let status = Command::new(&step.program)
            .args(&step.args)
            .current_dir(&step.dir)
            .status();
        let elapsed = step_start.elapsed();
        let failure = match status {
            Ok(status) if status.success() => None,
            Ok(status) => Some(format!(
                "code de sortie {}",
                status.code().map_or("?".to_owned(), |c| c.to_string())
            )),
            Err(e) => Some(format!("lancement impossible : {e}")),
        };
        if let Some(reason) = failure {
            print_summary(&done, started.elapsed());
            eprintln!("ÉCHEC : {} ({reason}, {})", step.name, format_duration(elapsed));
            let skipped: Vec<&str> = steps.iter().skip(done.len() + 1).map(|s| s.name.as_str()).collect();
            if !skipped.is_empty() {
                eprintln!("non lancées : {}", skipped.join(", "));
            }
            return ExitCode::FAILURE;
        }
        done.push((step.name.clone(), elapsed));
    }

    print_summary(&done, started.elapsed());
    println!("ci : OK");
    ExitCode::SUCCESS
}

fn print_summary(done: &[(String, Duration)], total: Duration) {
    println!("\n==> résumé");
    for (name, elapsed) in done {
        println!("  ok  {name:<20} {}", format_duration(*elapsed));
    }
    println!("  total                {}", format_duration(total));
}

/// Delegates to the `snapshot-catalog` binary of the gateway crate, which has the MCP client: xtask stays light.
fn snapshot_catalog(args: &[&str]) -> ExitCode {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(cargo)
        .args([
            "run",
            "--quiet",
            "--package",
            "mcp-proxy-gateway",
            "--bin",
            "snapshot-catalog",
            "--",
        ])
        .args(args)
        .current_dir(workspace_root())
        .status();
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("lancement impossible : {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["ci"] => run_ci(&[Group::Lint, Group::Test]),
        ["ci", "lint"] => run_ci(&[Group::Lint]),
        ["ci", "test"] => run_ci(&[Group::Test]),
        ["trace"] => trace::run(&workspace_root(), trace::Mode::Check),
        ["trace", "--write-index"] => trace::run(&workspace_root(), trace::Mode::WriteIndex),
        ["trace", "--report"] => trace::run(&workspace_root(), trace::Mode::Report),
        ["snapshot-catalog", rest @ ..] => snapshot_catalog(rest),
        ["icons"] => match icons::write_all(&workspace_root().join("app").join("icons")) {
            Ok(names) => {
                println!("app/icons : {}", names.join(", "));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("icônes non écrites : {e}");
                ExitCode::FAILURE
            }
        },
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
