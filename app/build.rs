//! Tauri code generation. The commands of the application, each granted only by a capability: `get_snapshot` to the
//! main window (`capabilities/main.json`), `submit_basic_credentials` to the Basic windows
//! (`capabilities/basic.json`), `answer_confirmation` to the detail windows of the confirmations
//! (`capabilities/confirm.json`), `deposit_login_header` to no static capability: each login window receives its own
//! at runtime (spec 012).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "get_snapshot",
            "deposit_login_header",
            "submit_basic_credentials",
            "answer_confirmation",
        ])),
    )?;
    Ok(())
}
