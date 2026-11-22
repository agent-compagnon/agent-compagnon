//! The commands of `update-tool` and their options. The release pipeline (spec 021) runs the binary directly in
//! `sign`, without cargo: this file is also compiled into `mcp-proxy-ci-check` (`#[path]`), which checks the
//! commands `.gitlab-ci.yml` gives it.

/// Every command of `update-tool`, with the options it accepts.
pub const COMMANDS: [(&str, &[&str]); 4] = [
    ("keygen", &["--out"]),
    ("sign", &["--exe", "--version", "--critical", "--key-env", "--out"]),
    ("verify", &["--exe", "--sig", "--public-key"]),
    ("check-release", &["--exe"]),
];

/// The options of `command`, if it is a command of `update-tool`.
pub fn options(command: &str) -> Option<&'static [&'static str]> {
    COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, options)| *options)
}
