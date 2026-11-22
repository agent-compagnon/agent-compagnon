use http::{HeaderName, HeaderValue};
use mcp_proxy_testkit::{MockMcpServer, catalog_tools, test_tools};

#[test]
fn every_catalog_snapshot_is_served() {
    for mcp in ["db-readonly", "gitlab-ops", "jira", "k8s-admin", "kibana-logs"] {
        assert!(!catalog_tools(mcp).unwrap().is_empty(), "{mcp}");
    }
    assert!(catalog_tools("unknown").is_none());
    assert_eq!(test_tools().len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn required_header_is_enforced_and_requests_are_recorded() {
    let server = MockMcpServer::builder()
        .require_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer ok"),
        )
        .www_authenticate(false)
        .start()
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let response = client.post(server.url()).body(body).send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert!(response.headers().get("www-authenticate").is_none());
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].mcp_method.as_deref(), Some("tools/list"));
}
