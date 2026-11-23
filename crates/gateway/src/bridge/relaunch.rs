//! Which exe the bridge may start when the application is not there (spec 026), and the marker by which the
//! application says it stopped on purpose.
//!
//! A bridge started from the installation keeps the image of the exe it was started from, even once an update
//! renamed that file `mcp-proxy.exe.old`: it starts the installed exe by its installation path, never its own file.
//! The application writes the marker as it exits on purpose (« Quitter », the restart of an update, the stop asked by
//! the installation or the uninstallation) and removes it once its pipe exists again; a crash leaves no marker.

use std::io;
use std::path::{Path, PathBuf};

use mcp_proxy_deploy::install::{BACKUP_NAME, EXE_NAME, InstallPaths, STAGED_NAME, same_location};
use mcp_proxy_deploy::update::is_leftover;

/// File of the data folder (`%LOCALAPPDATA%\MCP Proxy\`) present while the application is stopped on purpose.
pub const STOPPED_MARKER: &str = "arret-volontaire";

/// What a bridge without application can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaunchTarget {
    /// Start this exe.
    Exe(PathBuf),
    /// The application stopped on purpose: whoever stopped it starts it again, or nobody should. The exe is the one
    /// a bridge started by its LLM client would launch.
    StoppedOnPurpose(PathBuf),
    /// The exe to start is gone: uninstalled, or deleted.
    Missing,
    /// This exe's path could not be read.
    Unknown,
}

/// `running`: this exe as the system gave it at start; `install`: the installation's paths, when known.
pub fn relaunch_target(
    running: Option<&Path>,
    install: Option<&InstallPaths>,
    exists: &dyn Fn(&Path) -> bool,
    stopped_on_purpose: bool,
) -> RelaunchTarget {
    let Some(running) = running else {
        return RelaunchTarget::Unknown;
    };
    let exe = match install {
        Some(paths) if from_installation(running, paths) => paths.exe.clone(),
        _ => running.to_path_buf(),
    };
    if !exists(&exe) {
        RelaunchTarget::Missing
    } else if stopped_on_purpose {
        RelaunchTarget::StoppedOnPurpose(exe)
    } else {
        RelaunchTarget::Exe(exe)
    }
}

/// The installed exe, or a name the installation and the updates give it in its folder (`.old`, `.new`, a
/// leftover): never started itself, it may be an older version.
fn from_installation(running: &Path, paths: &InstallPaths) -> bool {
    if same_location(running, &paths.exe) {
        return true;
    }
    let in_folder = running.parent().is_some_and(|dir| same_location(dir, &paths.dir));
    let name = running.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    in_folder
        && (name.eq_ignore_ascii_case(EXE_NAME)
            || name.eq_ignore_ascii_case(BACKUP_NAME)
            || name.eq_ignore_ascii_case(STAGED_NAME)
            || is_leftover(name))
}

/// `<data>\arret-volontaire`.
pub fn stopped_marker(data: &Path) -> PathBuf {
    data.join(STOPPED_MARKER)
}

/// Written as the application exits on purpose. Best effort: without it, a bridge takes the stop for a crash and
/// starts the application again after its grace period.
pub fn mark_stopped_on_purpose(data: &Path) -> io::Result<()> {
    std::fs::write(stopped_marker(data), b"")
}

/// Removed once the pipe of a new instance exists.
pub fn clear_stopped_on_purpose(data: &Path) -> io::Result<()> {
    match std::fs::remove_file(stopped_marker(data)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn paths() -> InstallPaths {
        InstallPaths::new(
            Path::new(r"C:\Users\Ana\AppData\Local"),
            Path::new(r"C:\Users\Ana\AppData\Roaming"),
        )
    }

    fn target(running: &str, present: &[&Path], stopped: bool) -> RelaunchTarget {
        let present: BTreeSet<PathBuf> = present.iter().map(|p| p.to_path_buf()).collect();
        relaunch_target(
            Some(Path::new(running)),
            Some(&paths()),
            &|p| present.contains(p),
            stopped,
        )
    }

    // Exigence : EX-026-11
    #[test]
    fn the_installed_exe_by_its_installation_path_never_an_old_one() {
        let paths = paths();
        let installed = paths.exe.to_string_lossy().into_owned();
        assert_eq!(
            target(&installed, &[&paths.exe], false),
            RelaunchTarget::Exe(paths.exe.clone())
        );
        // The system may give the path in another case, or with the `\\?\` prefix.
        let prefixed = format!(r"\\?\{}", installed.to_uppercase());
        assert_eq!(
            target(&prefixed, &[&paths.exe], false),
            RelaunchTarget::Exe(paths.exe.clone())
        );
        // A bridge whose image was renamed by an update starts the new exe, never its own file.
        for own in [
            &paths.backup,
            &paths.staged,
            &paths.dir.join(".mcp-proxy.ab12.__relocated__.exe"),
        ] {
            let running = own.to_string_lossy().into_owned();
            let present = [own.as_path(), paths.exe.as_path()];
            assert_eq!(
                target(&running, &present, false),
                RelaunchTarget::Exe(paths.exe.clone()),
                "{running}"
            );
        }
        // Run from elsewhere (an exe in the downloads), the bridge starts that exe, as before.
        let downloads = PathBuf::from(r"C:\Users\Ana\Downloads\mcp-proxy.exe");
        let running = downloads.to_string_lossy().into_owned();
        assert_eq!(
            target(&running, &[&downloads, &paths.exe], false),
            RelaunchTarget::Exe(downloads.clone())
        );
        // Another file of the installation folder is not ours: started as itself.
        let other = paths.dir.join("autre.exe");
        let running = other.to_string_lossy().into_owned();
        assert_eq!(target(&running, &[&other], false), RelaunchTarget::Exe(other.clone()));
        // Without the installation's paths, this exe.
        let alone = relaunch_target(Some(&downloads), None, &|_| true, false);
        assert_eq!(alone, RelaunchTarget::Exe(downloads));
    }

    // Exigence : EX-026-11
    #[test]
    fn nothing_to_start_after_an_uninstallation_or_a_stop_on_purpose() {
        let paths = paths();
        let installed = paths.exe.to_string_lossy().into_owned();
        // The uninstallation set the exe aside before stopping the application: nothing to start.
        assert_eq!(target(&installed, &[&paths.backup], false), RelaunchTarget::Missing);
        assert_eq!(target(&installed, &[&paths.backup], true), RelaunchTarget::Missing);
        assert_eq!(
            target(&installed, &[&paths.exe], true),
            RelaunchTarget::StoppedOnPurpose(paths.exe.clone())
        );
        assert_eq!(
            relaunch_target(None, Some(&paths), &|_| true, false),
            RelaunchTarget::Unknown
        );
    }

    // Exigence : EX-026-11
    #[test]
    fn the_marker_of_a_stop_on_purpose() {
        let data = std::env::temp_dir().join(format!("mcp-proxy-marker-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        assert!(!stopped_marker(&data).exists());
        clear_stopped_on_purpose(&data).unwrap();
        mark_stopped_on_purpose(&data).unwrap();
        mark_stopped_on_purpose(&data).unwrap();
        assert!(stopped_marker(&data).exists());
        clear_stopped_on_purpose(&data).unwrap();
        assert!(!stopped_marker(&data).exists());
        assert!(mark_stopped_on_purpose(&data.join("absent")).is_err());
        std::fs::remove_dir_all(&data).unwrap();
    }
}
