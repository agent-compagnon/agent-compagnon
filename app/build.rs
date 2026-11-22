//! Tauri code generation. `get_snapshot` is the only command of the application, and only the capabilities of
//! `capabilities/` grant it.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["get_snapshot"])),
    )?;
    Ok(())
}
