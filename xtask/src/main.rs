//! Project automation, run with `cargo xtask <command>`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

#[cfg(test)]
mod commands;
mod icons;
mod release;
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
                        any, is read from the environment variable NAME and never printed
  keygen --out <file>   create an update signing key pair: the private key in <file> (which must not exist), the
                        public key printed, for crates/deploy/keys/current.pub or next.pub
  sign --exe <exe> [--version X.Y.Z] [--critical true|false] [--key-env NAME] [--out <file>]
                        write <exe>.sig, the signed manifest of spec 020; the private key is read from NAME
                        (ED25519_PRIVATE_KEY by default) and never printed
  verify --exe <exe> [--sig <file>] [--public-key <hex>]...
                        check a manifest and its exe with these keys, or with the embedded ones
  check-release [--exe <exe>]
                        refuse the development key and a missing current key; with --exe, check the built exe
  check-tag <tag>       check that a release tag is v<version of the workspace>, X.Y.Z or X.Y.Z-rc.N (spec 021)
  check-ci              check .gitlab-ci.yml without GitLab: valid YAML, release jobs on protected tags only, known
                        `cargo xtask` commands, variables documented in docs/release.md (spec 021)";

/// Where `check-ci` reads the pipeline and its documentation, relative to the workspace root.
const CHECK_CI_ARGS: [&str; 4] = ["--ci-file", ".gitlab-ci.yml", "--doc", "docs/release.md"];

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
    let mut check_ci = vec![
        "run",
        "--quiet",
        "--package",
        "mcp-proxy-ci-check",
        "--bin",
        "check-ci",
        "--",
    ];
    check_ci.extend(CHECK_CI_ARGS);
    steps.push(Step::new("pipeline (check-ci)", &[Test], &cargo, &check_ci, root));
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

/// Runs a binary of a crate of the workspace, which has the dependencies xtask goes without (MCP client,
/// cryptography): xtask stays light. Its exit code is passed on.
fn delegate(package: &str, bin: &str, args: &[&str]) -> ExitCode {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(cargo)
        .args(["run", "--quiet", "--package", package, "--bin", bin, "--"])
        .args(args)
        .current_dir(workspace_root())
        .status();
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => status
            .code()
            .and_then(|c| u8::try_from(c).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from),
        Err(e) => {
            eprintln!("lancement impossible : {e}");
            ExitCode::FAILURE
        }
    }
}

/// What a command line asks for. Free arguments are checked by the command itself.
enum Action<'a> {
    Ci(&'static [Group]),
    Trace(trace::Mode),
    Delegate {
        package: &'static str,
        bin: &'static str,
        args: Vec<&'a str>,
    },
    CheckTag(&'a [&'a str]),
    CheckCi,
    Icons,
    Help,
}

fn parse<'a>(args: &'a [&'a str]) -> Option<Action<'a>> {
    Some(match args {
        ["ci"] => Action::Ci(&[Group::Lint, Group::Test]),
        ["ci", "lint"] => Action::Ci(&[Group::Lint]),
        ["ci", "test"] => Action::Ci(&[Group::Test]),
        ["trace"] => Action::Trace(trace::Mode::Check),
        ["trace", "--write-index"] => Action::Trace(trace::Mode::WriteIndex),
        ["trace", "--report"] => Action::Trace(trace::Mode::Report),
        ["snapshot-catalog", rest @ ..] => Action::Delegate {
            package: "mcp-proxy-gateway",
            bin: "snapshot-catalog",
            args: rest.to_vec(),
        },
        [command @ ("keygen" | "sign" | "verify" | "check-release"), rest @ ..] => Action::Delegate {
            package: "mcp-proxy-deploy",
            bin: "update-tool",
            args: std::iter::once(*command).chain(rest.iter().copied()).collect(),
        },
        ["check-tag", rest @ ..] => Action::CheckTag(rest),
        ["check-ci"] => Action::CheckCi,
        ["icons"] => Action::Icons,
        ["help" | "--help" | "-h"] => Action::Help,
        _ => return None,
    })
}

fn check_tag(args: &[&str]) -> ExitCode {
    let [tag] = args else {
        eprintln!("check-tag : un seul argument attendu, le tag (par exemple v1.4.0)\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let version = env!("CARGO_PKG_VERSION");
    match release::check_tag(tag, version) {
        Ok(channel) => {
            println!(
                "check-tag : {tag} = version {version} du workspace ({})",
                channel.label()
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("check-tag : ÉCHEC : {message}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let Some(action) = parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    match action {
        Action::Ci(groups) => run_ci(groups),
        Action::Trace(mode) => trace::run(&workspace_root(), mode),
        Action::Delegate { package, bin, args } => delegate(package, bin, &args),
        Action::CheckTag(rest) => check_tag(rest),
        Action::CheckCi => delegate("mcp-proxy-ci-check", "check-ci", &CHECK_CI_ARGS),
        Action::Icons => match icons::write_all(&workspace_root().join("app").join("icons")) {
            Ok(names) => {
                println!("app/icons : {}", names.join(", "));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("icônes non écrites : {e}");
                ExitCode::FAILURE
            }
        },
        Action::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exigence : EX-021-09
    #[test]
    fn every_form_given_to_check_ci_is_a_command_of_xtask() {
        for form in commands::FORMS {
            let words: Vec<&str> = form.split_whitespace().filter(|w| *w != "…").collect();
            assert!(parse(&words).is_some(), "{form}");
        }
    }

    // Exigence : EX-021-09
    #[test]
    fn unknown_commands_are_refused() {
        for words in [
            &["deploy"][..],
            &["ci", "build"],
            &["trace", "--all"],
            &["check-ci", "--fix"],
            &[],
        ] {
            assert!(parse(words).is_none(), "{words:?}");
        }
    }

    #[test]
    fn the_ci_runs_check_ci_last() {
        let steps = ci_steps(&workspace_root());
        let last = steps.last().map(|s| s.args.clone()).unwrap_or_default();
        assert!(last.iter().any(|a| a == "check-ci"), "{last:?}");
    }
}
