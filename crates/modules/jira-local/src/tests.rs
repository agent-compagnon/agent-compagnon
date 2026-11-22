use mcp_proxy_core::module::{JsonObject, LocalModule, ModuleError, StatusCode, ToolDefinition, Url, json};

use super::*;
use crate::tools::{issue_key, max_results};

fn args(value: Value) -> JsonObject {
    value.as_object().cloned().unwrap_or_default()
}

// Exigence : EX-025-11
#[test]
fn the_tools_are_the_embedded_snapshot() {
    let snapshot: Vec<ToolDefinition> =
        json_from_slice(include_bytes!("../../../../config/catalog/jira.json")).unwrap();
    assert_eq!(JiraLocal.tools(), snapshot);
    let names: Vec<String> = snapshot.into_iter().map(|t| t.name).collect();
    assert_eq!(names, [SEARCH_ISSUES, GET_ISSUE, ADD_COMMENT]);
}

#[test]
fn issue_keys_are_checked_and_upper_cased() {
    assert_eq!(issue_key(&args(json!({ "key": "ops-12" }))).unwrap(), "OPS-12");
    assert_eq!(issue_key(&args(json!({ "key": "A_B2-7" }))).unwrap(), "A_B2-7");
    for key in [
        "OPS",
        "OPS-",
        "OPS-012",
        "-12",
        "2OPS-1",
        "OPS-1/../x",
        "OPS-1?a=b",
        " OPS-1",
        "ÉTÉ-1",
    ] {
        assert!(
            matches!(
                issue_key(&args(json!({ "key": key }))),
                Err(ModuleError::InvalidArguments(_))
            ),
            "{key}"
        );
    }
    assert!(matches!(
        issue_key(&args(json!({}))),
        Err(ModuleError::InvalidArguments(_))
    ));
    assert!(matches!(
        issue_key(&args(json!({ "key": 12 }))),
        Err(ModuleError::InvalidArguments(_))
    ));
}

#[test]
fn max_results_has_a_default_and_bounds() {
    assert_eq!(max_results(&args(json!({}))).unwrap(), 20);
    assert_eq!(max_results(&args(json!({ "max_results": 50 }))).unwrap(), 50);
    for bad in [json!(0), json!(51), json!(-1), json!(2.5), json!("10")] {
        assert!(max_results(&args(json!({ "max_results": bad }))).is_err(), "{bad}");
    }
}

#[test]
fn unexpected_arguments_are_refused() {
    let error = tools::only(&args(json!({ "jql": "x", "fields": "*all" })), &["jql", "max_results"]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "arguments invalides : argument inattendu ; arguments acceptés : jql, max_results"
    );
}

#[test]
fn endpoints_keep_a_context_path_and_encode_segments() {
    let base = Url::parse("https://jira.example.internal/jira/?x=1").unwrap();
    assert_eq!(
        endpoint(&base, &["rest", "api", "2", "issue", "OPS-1"])
            .unwrap()
            .as_str(),
        "https://jira.example.internal/jira/rest/api/2/issue/OPS-1"
    );
    let base = Url::parse("https://jira.example.internal").unwrap();
    assert_eq!(
        endpoint(&base, &["rest", "a/b"]).unwrap().as_str(),
        "https://jira.example.internal/rest/a%2Fb"
    );
}

#[test]
fn http_refusals_become_clear_messages() {
    let text = |status, subject| http_failure(status, &subject).to_string();
    assert!(text(StatusCode::BAD_REQUEST, Subject::Search).starts_with("requête JQL invalide"));
    assert!(text(StatusCode::NOT_FOUND, Subject::Issue("OPS-9")).starts_with("ticket introuvable : OPS-9"));
    assert!(text(StatusCode::NOT_FOUND, Subject::Comment("OPS-9")).starts_with("ticket introuvable : OPS-9"));
    assert_eq!(
        http_failure(StatusCode::UNAUTHORIZED, &Subject::Search),
        ModuleError::CredentialRejected
    );
    assert_eq!(
        http_failure(StatusCode::FORBIDDEN, &Subject::Issue("OPS-1")),
        ModuleError::CredentialRejected
    );
    assert_eq!(
        text(StatusCode::BAD_GATEWAY, Subject::Search),
        "Jira est indisponible (HTTP 502) : réessayez plus tard"
    );
    assert_eq!(
        text(StatusCode::IM_A_TEAPOT, Subject::Search),
        "réponse inattendue de Jira (HTTP 418)"
    );
}

#[test]
fn search_output_is_concise() {
    let body = json!({
        "total": 42,
        "issues": [{
            "key": "OPS-1",
            "self": "https://jira.example.internal/rest/api/2/issue/10001",
            "fields": {
                "summary": "Timeout du service",
                "status": { "name": "En cours", "self": "x" },
                "issuetype": { "name": "Bug" },
                "priority": { "name": "Haute" },
                "assignee": { "displayName": "Jean Dupont", "emailAddress": "jean@example.internal" },
                "updated": "2026-11-20T10:00:00.000+0100"
            }
        }]
    });
    let output = render::search(&body);
    assert_eq!(
        output.text,
        "1 tickets affichés sur 42 trouvés :\n- OPS-1 [En cours] Timeout du service — Bug, priorité Haute, assigné à \
         Jean Dupont, mis à jour le 2026-11-20T10:00:00.000+0100"
    );
    let structured = output.structured.unwrap();
    assert_eq!(structured["total"], 42);
    assert_eq!(structured["issues"][0]["assignee"], "Jean Dupont");
    let rendered = structured.to_string();
    assert!(!rendered.contains("emailAddress") && !rendered.contains("self"));
    assert_eq!(
        render::search(&json!({ "total": 0, "issues": [] })).text,
        "Aucun ticket ne correspond à la requête."
    );
}

#[test]
fn issue_output_keeps_the_last_comments_and_shortens_long_texts() {
    let comments: Vec<Value> = (1..=7)
        .map(|i| json!({ "author": { "displayName": format!("A{i}") }, "created": format!("d{i}"), "body": format!("c{i}") }))
        .collect();
    let body = json!({
        "key": "OPS-2",
        "fields": {
            "summary": "s",
            "assignee": null,
            "description": "x".repeat(5000),
            "comment": { "total": 7, "comments": comments }
        }
    });
    let output = render::issue("OPS-2", &body);
    let structured = output.structured.unwrap();
    assert_eq!(structured["comments"].as_array().unwrap().len(), render::LAST_COMMENTS);
    assert_eq!(structured["comments"][0]["body"], "c3");
    assert_eq!(structured["total_comments"], 7);
    assert!(structured["description"].as_str().unwrap().ends_with("… [tronqué]"));
    assert_eq!(
        structured["description"].as_str().unwrap().chars().count(),
        4000 + "… [tronqué]".chars().count()
    );
    assert!(output.text.contains("Assigné à : personne"));
    assert!(output.text.contains("Derniers commentaires (5 sur 7) :"));
}

#[test]
fn comment_output() {
    let output = render::comment(
        "OPS-3",
        &json!({ "id": "10042", "created": "2026-11-22T09:00:00.000+0100" }),
    );
    assert_eq!(
        output.text,
        "Commentaire 10042 ajouté à OPS-3 le 2026-11-22T09:00:00.000+0100."
    );
    assert_eq!(output.structured.unwrap()["id"], "10042");
}
