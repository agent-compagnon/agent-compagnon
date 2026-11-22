//! The launch modes of `mcp-proxy.exe` (specs 011 and 019).

use std::ffi::OsString;

use mcp_proxy_deploy::install::{AUTOSTART_ARG, NO_INSTALL_ARG};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// No argument: the application (single instance, notification icon, gateway), which offers the installation
    /// when it does not run from it (spec 019).
    App,
    /// `--autostart`, from the `Run` value at session opening: the application without its window, and without any
    /// installation dialog.
    Autostart,
    /// `--no-install`: the application from where it is, without offering the installation (relaunch after a
    /// declined or failed installation, launch by the bridge).
    InPlace,
    /// `stdio`: the bridge launched by an LLM client.
    Stdio,
}

/// Why the command line was refused. French, shown to whoever launched the exe.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LaunchError {
    #[error("argument inconnu : « {0} ». Utilisation : mcp-proxy.exe (appli) ou mcp-proxy.exe stdio (pont).")]
    Unknown(String),
    #[error("argument illisible (caractères non Unicode). Utilisation : mcp-proxy.exe ou mcp-proxy.exe stdio.")]
    NotUnicode,
    #[error("trop d'arguments. Utilisation : mcp-proxy.exe (appli) ou mcp-proxy.exe stdio (pont).")]
    TooMany,
}

/// Longest part of a refused argument quoted back.
const QUOTED_MAX: usize = 64;

/// Reads the arguments that follow the program name. Exact and case-sensitive: anything else is refused rather than
/// guessed, and the other internal commands of step 7 (`--uninstall`, `doctor`…) are not known yet.
pub fn parse_launch<I>(args: I) -> Result<LaunchMode, LaunchError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(LaunchMode::App);
    };
    if args.next().is_some() {
        return Err(LaunchError::TooMany);
    }
    let first = first.into_string().map_err(|_| LaunchError::NotUnicode)?;
    match first.as_str() {
        "stdio" => Ok(LaunchMode::Stdio),
        AUTOSTART_ARG => Ok(LaunchMode::Autostart),
        NO_INSTALL_ARG => Ok(LaunchMode::InPlace),
        other => Err(LaunchError::Unknown(other.chars().take(QUOTED_MAX).collect())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<LaunchMode, LaunchError> {
        parse_launch(args.iter().map(OsString::from))
    }

    // Exigence : EX-011-03
    #[test]
    fn modes() {
        assert_eq!(parse(&[]), Ok(LaunchMode::App));
        assert_eq!(parse(&["stdio"]), Ok(LaunchMode::Stdio));
        assert_eq!(parse(&["STDIO"]), Err(LaunchError::Unknown("STDIO".to_owned())));
        assert_eq!(parse(&["--stdio"]), Err(LaunchError::Unknown("--stdio".to_owned())));
        assert_eq!(parse(&["doctor"]), Err(LaunchError::Unknown("doctor".to_owned())));
        assert_eq!(parse(&["stdio", "x"]), Err(LaunchError::TooMany));
        assert_eq!(parse(&[""]), Err(LaunchError::Unknown(String::new())));
        let long = "x".repeat(500);
        assert_eq!(parse(&[&long]), Err(LaunchError::Unknown("x".repeat(QUOTED_MAX))));
        assert!(LaunchError::TooMany.to_string().contains("mcp-proxy.exe stdio"));
    }

    // Exigence : EX-019-09
    #[test]
    fn internal_arguments_of_the_installation() {
        assert_eq!(parse(&["--autostart"]), Ok(LaunchMode::Autostart));
        assert_eq!(parse(&["--no-install"]), Ok(LaunchMode::InPlace));
        assert_eq!(
            parse(&["--AUTOSTART"]),
            Err(LaunchError::Unknown("--AUTOSTART".to_owned()))
        );
        assert_eq!(parse(&["autostart"]), Err(LaunchError::Unknown("autostart".to_owned())));
        assert_eq!(parse(&["--install"]), Err(LaunchError::Unknown("--install".to_owned())));
        assert_eq!(parse(&["--autostart", "stdio"]), Err(LaunchError::TooMany));
        assert_eq!(parse(&["stdio", "--no-install"]), Err(LaunchError::TooMany));
        assert_eq!((AUTOSTART_ARG, NO_INSTALL_ARG), ("--autostart", "--no-install"));
    }

    // Exigence : EX-011-03
    #[cfg(unix)]
    #[test]
    fn non_unicode_is_refused() {
        use std::os::unix::ffi::OsStringExt;
        let args = vec![OsString::from_vec(vec![0x73, 0xff])];
        assert_eq!(parse_launch(args), Err(LaunchError::NotUnicode));
    }
}
