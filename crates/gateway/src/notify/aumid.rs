//! The identity of MCP Proxy for the toasts, registered in the registry only (no Start menu shortcut, no
//! `SetCurrentProcessExplicitAppUserModelID`, D18): `HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop` with
//! `DisplayName` and `IconUri`. The icon is a PNG written to the data folder: `IconUri` takes an image file.

use std::fs;
use std::io;
use std::path::Path;

use super::toast::AUMID;

pub const DISPLAY_NAME: &str = "MCP Proxy";

/// What the application writes under `HKEY_CURRENT_USER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AumidRegistration {
    /// Relative to `HKEY_CURRENT_USER`.
    pub key: String,
    pub display_name: String,
    pub icon_uri: String,
}

/// The values for an icon written at `icon` (`DataDirs::notification_icon`). `None` when the path is not valid
/// Unicode: the registry value could not name it.
pub fn aumid_registration(icon: &Path) -> Option<AumidRegistration> {
    Some(AumidRegistration {
        key: format!(r"Software\Classes\AppUserModelId\{AUMID}"),
        display_name: DISPLAY_NAME.to_owned(),
        icon_uri: icon.to_str()?.to_owned(),
    })
}

/// Writes the icon of the notifications, unless the file already holds exactly `png`. Idempotent.
pub fn write_icon(icon: &Path, png: &[u8]) -> io::Result<()> {
    if fs::read(icon).is_ok_and(|existing| existing == png) {
        return Ok(());
    }
    if let Some(parent) = icon.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(icon, png)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::runtime::DataDirs;

    // Exigence : EX-013-12
    #[test]
    fn registry_values_for_a_data_folder() {
        let base = Path::new(r"C:\Users\alice\AppData\Local");
        let dirs = DataDirs::under(base);
        let registration = aumid_registration(&dirs.notification_icon).unwrap();
        assert_eq!(registration.key, r"Software\Classes\AppUserModelId\McpProxy.Desktop");
        assert_eq!(registration.display_name, "MCP Proxy");
        assert_eq!(
            Path::new(&registration.icon_uri),
            base.join("MCP Proxy").join("notification-icon.png")
        );
        assert!(registration.icon_uri.ends_with(".png"));
    }

    // Exigence : EX-022-06
    #[test]
    fn the_uninstallation_removes_what_the_application_writes() {
        use mcp_proxy_deploy::uninstall::{AUMID_KEY, DATA_FOLDER, UninstallPaths};
        let base = Path::new(r"/home/alice/AppData/Local");
        let dirs = DataDirs::under(base);
        let uninstall = UninstallPaths::new(base, Path::new("/home/alice/AppData/Roaming"));
        assert_eq!(DataDirs::FOLDER, DATA_FOLDER);
        assert_eq!(dirs.root, uninstall.data);
        for path in [
            &dirs.logs,
            &dirs.catalog_cache,
            &dirs.settings,
            &dirs.settings_key,
            &dirs.webview,
            &dirs.notification_icon,
            &dirs.update,
        ] {
            assert!(path.starts_with(&uninstall.data), "{}", path.display());
        }
        assert_eq!(aumid_registration(&dirs.notification_icon).unwrap().key, AUMID_KEY);
    }

    // Exigence : EX-013-12
    #[test]
    fn icon_is_written_once() {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-aumid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let icon = dir.join("MCP Proxy").join("notification-icon.png");
        write_icon(&icon, b"png-1").unwrap();
        assert_eq!(fs::read(&icon).unwrap(), b"png-1");
        let written = fs::metadata(&icon).unwrap().modified().unwrap();
        write_icon(&icon, b"png-1").unwrap();
        assert_eq!(fs::metadata(&icon).unwrap().modified().unwrap(), written, "unchanged");
        write_icon(&icon, b"png-2").unwrap();
        assert_eq!(fs::read(&icon).unwrap(), b"png-2");
        let _ = fs::remove_dir_all(&dir);
    }
}
