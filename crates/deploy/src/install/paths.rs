//! Where the installation goes. Every path is derived from the two folders the application reads in the
//! environment (`%LOCALAPPDATA%`, `%APPDATA%`): nothing here reads it.

use std::path::{Path, PathBuf};

pub const EXE_NAME: &str = "mcp-proxy.exe";
/// The copy being put in place, in the installation folder, renamed over the exe once complete.
pub const STAGED_NAME: &str = "mcp-proxy.exe.new";
/// The previous exe, set aside during an update until the new one runs.
pub const BACKUP_NAME: &str = "mcp-proxy.exe.old";
pub const SHORTCUT_NAME: &str = "MCP Proxy.lnk";
/// The installation folder as the user reads it, without their name.
pub const LOCATION_LABEL: &str = r"%LOCALAPPDATA%\Programs\MCP Proxy";

/// The paths of the installation. The data stay in `%LOCALAPPDATA%\MCP Proxy\`, a different folder that nothing here
/// touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPaths {
    /// `%LOCALAPPDATA%\Programs\MCP Proxy`
    pub dir: PathBuf,
    pub exe: PathBuf,
    pub staged: PathBuf,
    pub backup: PathBuf,
    /// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\MCP Proxy.lnk`
    pub shortcut: PathBuf,
}

impl InstallPaths {
    pub fn new(local_app_data: &Path, app_data: &Path) -> Self {
        let dir = local_app_data.join("Programs").join("MCP Proxy");
        let shortcut = app_data
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(SHORTCUT_NAME);
        Self {
            exe: dir.join(EXE_NAME),
            staged: dir.join(STAGED_NAME),
            backup: dir.join(BACKUP_NAME),
            dir,
            shortcut,
        }
    }
}

/// Two canonical paths name the same file under Windows: same text, letters compared without case, a `\\?\` prefix
/// ignored.
pub fn same_location(a: &Path, b: &Path) -> bool {
    fn key(path: &Path) -> String {
        let text = path.to_string_lossy();
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        text.replace('/', r"\").trim_end_matches('\\').to_lowercase()
    }
    key(a) == key(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installation_and_the_data_are_two_folders() {
        let local = Path::new("/u/AppData/Local");
        let paths = InstallPaths::new(local, Path::new("/u/AppData/Roaming"));
        let data = local.join("MCP Proxy");
        assert_eq!(paths.dir, local.join("Programs").join("MCP Proxy"));
        assert_eq!(paths.exe.file_name().and_then(|n| n.to_str()), Some("mcp-proxy.exe"));
        for path in [&paths.exe, &paths.staged, &paths.backup] {
            assert_eq!(path.parent(), Some(paths.dir.as_path()));
            assert!(!path.starts_with(&data), "{}", path.display());
        }
        assert!(!data.starts_with(&paths.dir) && !paths.dir.starts_with(&data));
        assert!(
            paths
                .shortcut
                .ends_with("Microsoft/Windows/Start Menu/Programs/MCP Proxy.lnk")
        );
    }

    #[test]
    fn locations_compare_as_windows_does() {
        assert!(same_location(
            Path::new(r"\\?\C:\Users\Ana\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe"),
            Path::new(r"c:\users\ana\appdata\local\programs\mcp proxy\MCP-PROXY.EXE"),
        ));
        assert!(!same_location(
            Path::new(r"C:\Users\Ana\Downloads\mcp-proxy.exe"),
            Path::new(r"C:\Users\Ana\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe"),
        ));
        assert!(!same_location(
            Path::new(r"C:\a\mcp-proxy.exe.new"),
            Path::new(r"C:\a\mcp-proxy.exe")
        ));
    }
}
