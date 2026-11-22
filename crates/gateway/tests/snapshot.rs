//! `snapshot-catalog` against the fake MCP server.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use http::{HeaderName, HeaderValue};
use mcp_proxy_core::config::{EnvId, McpId};
use mcp_proxy_gateway::registry::{EmbeddedCatalog, InstanceId};
use mcp_proxy_testkit::{MockMcpServer, catalog_tools};

const TOKEN: &str = "jeton-instantane-41d2e8";
const VARIABLE: &str = "MCP_PROXY_TEST_SNAPSHOT_TOKEN";

async fn server() -> MockMcpServer {
    MockMcpServer::builder()
        .tools(catalog_tools("kibana-logs").unwrap())
        .page_size(2)
        .require_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
        )
        .start()
        .await
        .unwrap()
}

fn out_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mcp-proxy-snapshot-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("kibana-logs.json");
    let _ = std::fs::remove_file(&file);
    file
}

async fn snapshot(url: String, out: &Path, token: &str) -> Output {
    let out = out.to_path_buf();
    let token = token.to_owned();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_snapshot-catalog"))
            .args(["--url", &url, "--out"])
            .arg(&out)
            .args(["--header-env", VARIABLE])
            .env(VARIABLE, token)
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

fn printed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// Exigence : EX-006-08
#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_is_read_back_by_the_embedded_catalog() {
    let server = server().await;
    let out = out_file("ok");
    let output = snapshot(server.url(), &out, TOKEN).await;
    let text = printed(&output);
    assert!(output.status.success(), "{text}");
    assert!(!text.contains(TOKEN), "{text}");

    let written = std::fs::read_to_string(&out).unwrap();
    assert!(!written.contains(TOKEN));
    let config = mcp_proxy_gateway::modules::embedded_config().unwrap();
    let catalog = EmbeddedCatalog::from_files(&config, &[("kibana-logs.json", &written)]).unwrap();
    let instance = InstanceId::new(EnvId::new("dev").unwrap(), McpId::new("kibana-logs").unwrap());
    let names: Vec<String> = catalog
        .for_instance(&instance)
        .unwrap()
        .iter()
        .map(|t| t.name.clone())
        .collect();
    let expected: Vec<String> = catalog_tools("kibana-logs")
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, expected);

    let lists = server
        .requests()
        .into_iter()
        .filter(|r| r.mcp_method.as_deref() == Some("tools/list"));
    assert!(lists.count() >= 2, "every page is listed");
    for request in server.requests() {
        assert_eq!(
            request.headers.get("authorization").unwrap(),
            &format!("Bearer {TOKEN}")
        );
    }
}

// Exigence : EX-006-08
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_token_is_reported_without_being_printed() {
    let server = server().await;
    let out = out_file("refused");
    let wrong = "jeton-refuse-9b77c1";
    let output = snapshot(server.url(), &out, wrong).await;
    let text = printed(&output);
    assert!(!output.status.success());
    assert!(text.contains("identifiant refusé"), "{text}");
    assert!(!text.contains(wrong), "{text}");
    assert!(!out.exists());
    let received = server.requests();
    assert!(!received.is_empty());
    assert!(
        received
            .iter()
            .all(|r| r.headers.get("authorization").unwrap() == &format!("Bearer {wrong}"))
    );
}
