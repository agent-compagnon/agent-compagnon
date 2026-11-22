//! The forms of `cargo xtask`, as `cargo xtask check-ci` checks the commands that `.gitlab-ci.yml` cites (spec 021).
//! Each form is a list of fixed words; a trailing `…` stands for free arguments, which the command checks itself.
//!
//! This file is also compiled into `mcp-proxy-ci-check` (`#[path]`), so that the checker and xtask share one list;
//! a test of xtask makes sure that its dispatch accepts every form.

/// Every form of `cargo xtask`, the free arguments of a command being left to that command.
pub const FORMS: &[&str] = &[
    "ci",
    "ci lint",
    "ci test",
    "trace",
    "trace --write-index",
    "trace --report",
    "icons",
    "snapshot-catalog …",
    "keygen …",
    "sign …",
    "verify …",
    "check-release …",
    "check-tag …",
    "check-ci",
    "help",
];
