//! The WebView2 profile of the login windows: runtime version, browser arguments, data directory.

use std::io;
use std::path::{Path, PathBuf};

use mcp_proxy_core::config::{WebviewConfig, WebviewProfile};

use crate::ui::AuthUiError;

/// The first WebView2 runtime that honours `IsInPrivateModeEnabled`; wry ignores `incognito` silently before it.
pub const MIN_RUNTIME_VERSION: [u64; 4] = [101, 0, 1210, 39];

/// The arguments wry passes when it is given none.
pub const WRY_DEFAULT_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// Single sign-on with the primary account of Windows (`webview.os_primary_account_sso`).
pub const OS_SSO_FEATURE: &str = "--enable-features=msSingleSignOnOSForPrimaryAccountIsShared";

/// Parses `major.minor.build.patch`, optionally followed by a channel name (`121.0.2277.4 beta`), and checks it is
/// at least [`MIN_RUNTIME_VERSION`]. Unreadable, malformed or older: [`AuthUiError::RuntimeTooOld`], never a fallback
/// to a profile on disk.
pub fn check_runtime_version(version: Option<&str>) -> Result<[u64; 4], AuthUiError> {
    let number = version
        .and_then(|v| v.split_whitespace().next())
        .ok_or(AuthUiError::RuntimeTooOld)?;
    let mut parts = [0u64; 4];
    let mut count = 0;
    for part in number.split('.') {
        if count == parts.len() || part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AuthUiError::RuntimeTooOld);
        }
        parts[count] = part.parse().map_err(|_| AuthUiError::RuntimeTooOld)?;
        count += 1;
    }
    if count != parts.len() || parts < MIN_RUNTIME_VERSION {
        return Err(AuthUiError::RuntimeTooOld);
    }
    Ok(parts)
}

/// The arguments of every login window and of the keeper: WebView2 refuses to share a data directory between
/// webviews whose arguments differ, so they are computed once from the configuration.
pub fn browser_args(os_primary_account_sso: bool) -> String {
    if os_primary_account_sso {
        format!("{WRY_DEFAULT_ARGS} {OS_SSO_FEATURE}")
    } else {
        WRY_DEFAULT_ARGS.to_owned()
    }
}

/// How every login window and the keeper are built. The same value for all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginProfile {
    pub data_directory: PathBuf,
    pub browser_args: String,
    /// `incognito(true)` on every window (`in_memory`, the default); `false` only for `ephemeral_disk`.
    pub incognito: bool,
}

impl LoginProfile {
    pub fn from_config(config: &WebviewConfig, data_directory: PathBuf) -> Self {
        Self {
            data_directory,
            browser_args: browser_args(config.os_primary_account_sso),
            incognito: config.profile == WebviewProfile::InMemory,
        }
    }

    /// Whether the runtime version must be checked before a login window: only InPrivate depends on it.
    pub fn requires_inprivate(&self) -> bool {
        self.incognito
    }
}

/// Empties `dir` and creates it again (at start and at exit). A symbolic link is removed, not followed.
pub fn reset_profile_dir(dir: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(dir)?,
        Ok(_) => std::fs::remove_file(dir)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::create_dir_all(dir)
}
