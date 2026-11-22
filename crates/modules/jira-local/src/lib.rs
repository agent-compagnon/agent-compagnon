//! Local MCP module for Jira (REST API v2): search issues, read one, add a comment.
//!
//! Written against `mcp_proxy_core::module` only. The module never holds a credential: every request goes through
//! the `AuthenticatedHttpClient` of its context, which only reaches the origin of the instance's `base_url`.

mod render;
mod tools;

#[cfg(test)]
mod tests;

use mcp_proxy_core::module::{
    DEFAULT_BODY_LIMIT, HeaderValue, JsonObject, LocalModule, Method, ModuleContext, ModuleError, Request, Response,
    StatusCode, ToolDefinition, ToolOutput, Url, Value, async_trait, header, json, json_from_slice, read_body,
};

use crate::tools::{ADD_COMMENT, GET_ISSUE, MAX_COMMENT_LEN, MAX_JQL_LEN, SEARCH_ISSUES};

/// Fields asked for by `search_issues`.
const SEARCH_FIELDS: &str = "summary,status,issuetype,priority,assignee,updated";
/// Fields asked for by `get_issue`.
const ISSUE_FIELDS: &str = "summary,status,issuetype,priority,assignee,reporter,created,updated,description,comment";

/// The `jira-local` module.
#[derive(Debug, Clone, Copy, Default)]
pub struct JiraLocal;

impl JiraLocal {
    pub const ID: &'static str = "jira-local";
}

/// What a request is about, for error messages.
enum Subject<'a> {
    Search,
    Issue(&'a str),
    Comment(&'a str),
}

/// `base_url` followed by `segments`, each percent-encoded: a context path (`https://host/jira`) is kept.
fn endpoint(base: &Url, segments: &[&str]) -> Result<Url, ModuleError> {
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| ModuleError::Failed("adresse de Jira inutilisable".to_owned()))?;
        path.pop_if_empty();
        path.extend(segments);
    }
    Ok(url)
}

/// A clear message for a refusal of Jira, without its body.
fn http_failure(status: StatusCode, subject: &Subject<'_>) -> ModuleError {
    let code = status.as_u16();
    match (status, subject) {
        (StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN, _) => ModuleError::CredentialRejected,
        (StatusCode::BAD_REQUEST, Subject::Search) => ModuleError::Failed(
            "requête JQL invalide : vérifiez les noms de champs, les opérateurs, les valeurs et les guillemets"
                .to_owned(),
        ),
        (StatusCode::NOT_FOUND, Subject::Issue(key) | Subject::Comment(key)) => ModuleError::Failed(format!(
            "ticket introuvable : {key} (inexistant, ou invisible pour ce compte)"
        )),
        (StatusCode::BAD_REQUEST, Subject::Comment(key)) => {
            ModuleError::Failed(format!("commentaire refusé par Jira pour {key} (HTTP 400)"))
        }
        (StatusCode::TOO_MANY_REQUESTS, _) => {
            ModuleError::Failed("Jira limite le nombre de requêtes : réessayez dans quelques instants".to_owned())
        }
        _ if status.is_server_error() => {
            ModuleError::Failed(format!("Jira est indisponible (HTTP {code}) : réessayez plus tard"))
        }
        _ => ModuleError::Failed(format!("réponse inattendue de Jira (HTTP {code})")),
    }
}

async fn json_of(response: Response, subject: &Subject<'_>) -> Result<Value, ModuleError> {
    let status = response.status();
    if !status.is_success() {
        return Err(http_failure(status, subject));
    }
    let body = read_body(response, DEFAULT_BODY_LIMIT).await?;
    json_from_slice(&body).map_err(|_| ModuleError::Failed("réponse de Jira illisible (JSON attendu)".to_owned()))
}

fn json_request(ctx: &ModuleContext, method: Method, url: Url) -> Request {
    let mut request = ctx.http.request(method, url);
    request
        .headers_mut()
        .insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    request
}

impl JiraLocal {
    async fn search_issues(&self, ctx: &ModuleContext, args: &JsonObject) -> Result<ToolOutput, ModuleError> {
        tools::only(args, &["jql", "max_results"])?;
        let jql = tools::text(args, "jql", MAX_JQL_LEN)?;
        let max_results = tools::max_results(args)?;
        let mut url = endpoint(&ctx.base_url, &["rest", "api", "2", "search"])?;
        url.query_pairs_mut()
            .append_pair("jql", jql)
            .append_pair("maxResults", &max_results.to_string())
            .append_pair("fields", SEARCH_FIELDS);
        let response = ctx.http.execute(json_request(ctx, Method::GET, url)).await?;
        let body = json_of(response, &Subject::Search).await?;
        Ok(render::search(&body))
    }

    async fn get_issue(&self, ctx: &ModuleContext, args: &JsonObject) -> Result<ToolOutput, ModuleError> {
        tools::only(args, &["key"])?;
        let key = tools::issue_key(args)?;
        let mut url = endpoint(&ctx.base_url, &["rest", "api", "2", "issue", &key])?;
        url.query_pairs_mut().append_pair("fields", ISSUE_FIELDS);
        let response = ctx.http.execute(json_request(ctx, Method::GET, url)).await?;
        let body = json_of(response, &Subject::Issue(&key)).await?;
        Ok(render::issue(&key, &body))
    }

    async fn add_comment(&self, ctx: &ModuleContext, args: &JsonObject) -> Result<ToolOutput, ModuleError> {
        tools::only(args, &["key", "body"])?;
        let key = tools::issue_key(args)?;
        let text = tools::text(args, "body", MAX_COMMENT_LEN)?;
        let url = endpoint(&ctx.base_url, &["rest", "api", "2", "issue", &key, "comment"])?;
        let mut request = json_request(ctx, Method::POST, url);
        request
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        *request.body_mut() = Some(json!({ "body": text }).to_string().into());
        let response = ctx.http.execute(request).await?;
        let body = json_of(response, &Subject::Comment(&key)).await?;
        Ok(render::comment(&key, &body))
    }
}

#[async_trait]
impl LocalModule for JiraLocal {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        tools::definitions()
    }

    async fn call(&self, ctx: &ModuleContext, tool: &str, args: JsonObject) -> Result<ToolOutput, ModuleError> {
        match tool {
            SEARCH_ISSUES => self.search_issues(ctx, &args).await,
            GET_ISSUE => self.get_issue(ctx, &args).await,
            ADD_COMMENT => self.add_comment(ctx, &args).await,
            other => Err(ModuleError::UnknownTool(other.to_owned())),
        }
    }
}
