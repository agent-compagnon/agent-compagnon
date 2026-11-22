//! Exclusion from Windows Error Reporting, so that no crash dump holds the process memory: DWORD `1` named after
//! the exe under `HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications` (`winreg`).

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const KEY: &str = r"Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications";

/// Idempotent. The error names the step only.
pub fn exclude_from_error_reports() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("chemin de l'exe illisible ({})", e.kind()))?;
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "nom de l'exe illisible".to_owned())?;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(KEY)
        .map_err(|e| format!("clé du registre inaccessible ({})", e.kind()))?;
    key.set_value(name, &1u32)
        .map_err(|e| format!("valeur du registre non écrite ({})", e.kind()))
}
