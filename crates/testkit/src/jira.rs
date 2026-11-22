//! `FakeJira`: the part of the Jira REST API v2 the `jira-local` module uses, behind Basic authentication.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use mcp_proxy_core::secrets::basic_authorization;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const MAX_BODY: usize = 1 << 20;

/// Text present in every error body of `FakeJira`: a module that copies a raw body would show it.
pub const RAW_ERROR_MARKER: &str = "FAKE-JIRA-RAW-BODY";

/// A JQL query containing this word is refused with a 400, as Jira does for a syntax error.
pub const INVALID_JQL: &str = "INVALID";

/// One request received, credential header included (this is a test double).
#[derive(Debug, Clone)]
pub struct JiraRequest {
    pub method: Method,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    /// The `Authorization` header matched the expected identifiers.
    pub authorized: bool,
}

#[derive(Debug, Clone)]
struct Issue {
    summary: String,
    description: String,
    comments: Vec<String>,
}

struct JiraState {
    expected: String,
    issues: BTreeMap<String, Issue>,
    requests: Vec<JiraRequest>,
    next_comment: u64,
}

fn lock(state: &Mutex<JiraState>) -> MutexGuard<'_, JiraState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn basic(login: &str, password: &str) -> String {
    basic_authorization(login, &SecretString::from(password.to_owned()))
        .ok()
        .and_then(|v| v.to_str().ok().map(str::to_owned))
        .unwrap_or_default()
}

/// Serves `/rest/api/2/{myself, search, issue/{key}, issue/{key}/comment}` on `127.0.0.1`. Every request must carry
/// `Authorization: Basic` with the current identifiers, otherwise 401. Starts with two issues, `OPS-1` and `OPS-2`.
/// Stops when dropped.
pub struct FakeJira {
    addr: SocketAddr,
    state: Arc<Mutex<JiraState>>,
    ct: CancellationToken,
}

impl FakeJira {
    pub async fn start(login: &str, password: &str) -> std::io::Result<Self> {
        let mut issues = BTreeMap::new();
        issues.insert(
            "OPS-1".to_owned(),
            Issue {
                summary: "Timeout du service de paiement".to_owned(),
                description: "Les appels au service de paiement dépassent 30 s depuis ce matin.".to_owned(),
                comments: vec!["Reproduit en recette.".to_owned()],
            },
        );
        issues.insert(
            "OPS-2".to_owned(),
            Issue {
                summary: "Certificat à renouveler".to_owned(),
                description: "Le certificat expire le mois prochain.".to_owned(),
                comments: Vec::new(),
            },
        );
        let state = Arc::new(Mutex::new(JiraState {
            expected: basic(login, password),
            issues,
            requests: Vec::new(),
            next_comment: 10_000,
        }));
        let router = Router::new().fallback(handle).with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ct = CancellationToken::new();
        let shutdown = ct.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(Self { addr, state, ct })
    }

    /// `http://127.0.0.1:<port>`: the `base_url` of an instance.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Requests now need these identifiers, as after a password change.
    pub fn set_credentials(&self, login: &str, password: &str) {
        lock(&self.state).expected = basic(login, password);
    }

    /// Adds or replaces an issue without comments.
    pub fn put_issue(&self, key: &str, summary: &str, description: &str) {
        lock(&self.state).issues.insert(
            key.to_owned(),
            Issue {
                summary: summary.to_owned(),
                description: description.to_owned(),
                comments: Vec::new(),
            },
        );
    }

    pub fn comments(&self, key: &str) -> Vec<String> {
        lock(&self.state)
            .issues
            .get(key)
            .map(|i| i.comments.clone())
            .unwrap_or_default()
    }

    pub fn requests(&self) -> Vec<JiraRequest> {
        lock(&self.state).requests.clone()
    }

    /// Requests to `path` (e.g. `/rest/api/2/search`), probe excluded unless asked for.
    pub fn requests_to(&self, path: &str) -> Vec<JiraRequest> {
        self.requests().into_iter().filter(|r| r.path == path).collect()
    }
}

impl Drop for FakeJira {
    fn drop(&mut self) {
        self.ct.cancel();
    }
}

fn reply(status: StatusCode, body: &Value) -> Response {
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn failure(status: StatusCode, message: &str) -> Response {
    reply(
        status,
        &json!({ "errorMessages": [format!("{message} ({RAW_ERROR_MARKER})")], "errors": {} }),
    )
}

fn issue_json(key: &str, issue: &Issue) -> Value {
    let comments: Vec<Value> = issue
        .comments
        .iter()
        .enumerate()
        .map(|(i, body)| {
            json!({
                "id": (10_000 + i).to_string(),
                "author": { "displayName": "Jean Dupont", "emailAddress": "jean.dupont@example.internal" },
                "created": "2026-11-21T09:00:00.000+0100",
                "body": body
            })
        })
        .collect();
    json!({
        "id": "10001",
        "self": format!("http://jira.example.internal/rest/api/2/issue/{key}"),
        "key": key,
        "fields": {
            "summary": issue.summary,
            "description": issue.description,
            "status": { "name": "En cours", "id": "3" },
            "issuetype": { "name": "Bug", "id": "1" },
            "priority": { "name": "Haute", "id": "2" },
            "assignee": { "displayName": "Jean Dupont", "emailAddress": "jean.dupont@example.internal" },
            "reporter": { "displayName": "Marie Martin" },
            "created": "2026-11-20T08:00:00.000+0100",
            "updated": "2026-11-21T09:00:00.000+0100",
            "comment": { "total": comments.len(), "maxResults": comments.len(), "comments": comments }
        }
    })
}

async fn handle(State(state): State<Arc<Mutex<JiraState>>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, MAX_BODY).await.unwrap_or_default().to_vec();
    let query: BTreeMap<String, String> = parts
        .uri
        .query()
        .map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect())
        .unwrap_or_default();
    let path = parts.uri.path().to_owned();
    let mut state = lock(&state);
    let authorized = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == state.expected);
    state.requests.push(JiraRequest {
        method: parts.method.clone(),
        path: path.clone(),
        query: query.clone(),
        headers: parts.headers.clone(),
        body: body.clone(),
        authorized,
    });
    if !authorized {
        return failure(StatusCode::UNAUTHORIZED, "You are not authenticated");
    }
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (&parts.method, segments.as_slice()) {
        (&Method::GET, ["rest", "api", "2", "myself"]) => reply(StatusCode::OK, &json!({ "name": "jdoe" })),
        (&Method::GET, ["rest", "api", "2", "search"]) => {
            let jql = query.get("jql").cloned().unwrap_or_default();
            if jql.contains(INVALID_JQL) {
                return failure(StatusCode::BAD_REQUEST, &format!("Error in the JQL Query: {jql}"));
            }
            let max = query
                .get("maxResults")
                .and_then(|m| m.parse::<usize>().ok())
                .unwrap_or(50);
            let issues: Vec<Value> = state
                .issues
                .iter()
                .take(max)
                .map(|(key, issue)| issue_json(key, issue))
                .collect();
            reply(
                StatusCode::OK,
                &json!({ "startAt": 0, "maxResults": max, "total": state.issues.len(), "issues": issues }),
            )
        }
        (&Method::GET, ["rest", "api", "2", "issue", key]) => match state.issues.get(*key) {
            Some(issue) => reply(StatusCode::OK, &issue_json(key, issue)),
            None => failure(StatusCode::NOT_FOUND, "Issue Does Not Exist"),
        },
        (&Method::POST, ["rest", "api", "2", "issue", key, "comment"]) => {
            let text = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|v| v.get("body").and_then(Value::as_str).map(str::to_owned));
            let Some(text) = text else {
                return failure(StatusCode::BAD_REQUEST, "Comment body can not be empty!");
            };
            let id = state.next_comment;
            let Some(issue) = state.issues.get_mut(*key) else {
                return failure(StatusCode::NOT_FOUND, "Issue Does Not Exist");
            };
            issue.comments.push(text.clone());
            state.next_comment += 1;
            reply(
                StatusCode::CREATED,
                &json!({ "id": id.to_string(), "body": text, "created": "2026-11-22T10:00:00.000+0100" }),
            )
        }
        _ => failure(StatusCode::NOT_FOUND, "Not found"),
    }
}
