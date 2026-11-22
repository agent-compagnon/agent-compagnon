//! Embeds the public keys of the updater (spec 020): `keys/current.pub` and `keys/next.pub` become the two slots of
//! `mcp_proxy_deploy::update::keys`. Every build fails if a slot holds the development key (`keys/dev.pub`, whose
//! private half is public); a release build (`MCP_PROXY_RELEASE=1`) also fails without a current key.

#[path = "src/update/slots.rs"]
mod slots;

use std::path::PathBuf;

fn read(dir: &std::path::Path, name: &str) -> Result<Option<[u8; slots::KEY_LEN]>, String> {
    println!("cargo:rerun-if-changed=keys/{name}");
    let text = std::fs::read_to_string(dir.join(name)).map_err(|e| format!("keys/{name} illisible : {e}"))?;
    slots::parse_slot(&text).map_err(|e| format!("keys/{name} : {e}"))
}

fn literal(slot: Option<[u8; slots::KEY_LEN]>) -> String {
    match slot {
        None => "None".to_owned(),
        Some(key) => format!("Some({key:?})"),
    }
}

fn main() -> Result<(), String> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let keys = manifest.join("keys");
    println!("cargo:rerun-if-env-changed=MCP_PROXY_RELEASE");
    let slots = [read(&keys, "current.pub")?, read(&keys, "next.pub")?];
    let dev = read(&keys, "dev.pub")?.ok_or("keys/dev.pub : clé de développement absente")?;
    slots::build_guard(&slots, &dev)?;
    if std::env::var("MCP_PROXY_RELEASE").is_ok_and(|v| v == "1") {
        slots::release_guard(&slots, &dev)?;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let code = format!(
        "/// The key slots read from `keys/` by `build.rs`: current, then next.\npub(crate) const SLOTS: [Option<[u8; 32]>; 2] = [{}, {}];\n",
        literal(slots[0]),
        literal(slots[1])
    );
    std::fs::write(out.join("embedded_keys.rs"), code).map_err(|e| format!("embedded_keys.rs : {e}"))
}
