//! A fake MCP server over Streamable HTTP, on `127.0.0.1` and a port chosen by the system.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::any;
use http::header::{AUTHORIZATION, CONTENT_TYPE, LOCATION, WWW_AUTHENTICATE};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, DiscoverResult, ErrorCode, Implementation,
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, ProtocolVersion, ServerCapabilities,
    ServerConfig, SubscriptionFilter, Tool,
};
use rmcp::service::{NotificationContext, Peer, RequestContext, RoleServer, SubscriptionContext};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::catalog::test_tools;

/// Largest request body the fake server reads.
const MAX_BODY: usize = 4 * 1024 * 1024;

/// Protocol generations a fake server accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    /// `initialize` handshake only (2024-11-05 to 2025-11-25); `server/discover` is refused.
    Legacy,
    /// 2026-07-28 only: `server/discover` and per-request metadata; `initialize` is refused.
    Modern,
    /// Both, on the same endpoint.
    Both,
}

/// A request received by a fake server, whatever its outcome.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    /// JSON-RPC method of a POST body, if any.
    pub mcp_method: Option<String>,
    /// Tool name of a `tools/call`.
    pub tool: Option<String>,
}

struct Shared {
    generation: Generation,
    tools: Mutex<Vec<Tool>>,
    test_tools: bool,
    page_size: Option<usize>,
    required_header: Option<(HeaderName, HeaderValue)>,
    www_authenticate: bool,
    redirect_to: Option<String>,
    journal: Mutex<Vec<RecordedRequest>>,
    /// Peers of legacy sessions, notified by `notifications/tools/list_changed`.
    peers: Mutex<Vec<Peer<RoleServer>>>,
    /// Wakes the 2026-07-28 `subscriptions/listen` streams.
    changes: broadcast::Sender<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn all_tools(&self) -> Vec<Tool> {
        let mut tools = lock(&self.tools).clone();
        if self.test_tools {
            tools.extend(test_tools());
        }
        tools
    }
}

pub struct MockMcpBuilder {
    generation: Generation,
    tools: Vec<Tool>,
    test_tools: bool,
    page_size: Option<usize>,
    required_header: Option<(HeaderName, HeaderValue)>,
    www_authenticate: bool,
    redirect_to: Option<String>,
}

impl MockMcpBuilder {
    pub fn generation(mut self, generation: Generation) -> Self {
        self.generation = generation;
        self
    }

    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = tools;
        self
    }

    /// Also serves `echo_headers`, `slow`, `fail` and `http_fail` (see `test_tools`).
    pub fn with_test_tools(mut self) -> Self {
        self.test_tools = true;
        self
    }

    /// Splits `tools/list` into pages of `size` tools.
    pub fn page_size(mut self, size: usize) -> Self {
        self.page_size = Some(size.max(1));
        self
    }

    /// Every request to `/mcp` without exactly this header gets a 401.
    pub fn require_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.required_header = Some((name, value));
        self
    }

    /// Whether the 401 carries a `WWW-Authenticate` challenge.
    pub fn www_authenticate(mut self, enabled: bool) -> Self {
        self.www_authenticate = enabled;
        self
    }

    /// `/redirect` answers `307` towards `target`, for instance another fake server's `/mcp`.
    pub fn redirect_to(mut self, target: impl Into<String>) -> Self {
        self.redirect_to = Some(target.into());
        self
    }

    pub async fn start(self) -> std::io::Result<MockMcpServer> {
        let (changes, _) = broadcast::channel(16);
        let shared = Arc::new(Shared {
            generation: self.generation,
            tools: Mutex::new(self.tools),
            test_tools: self.test_tools,
            page_size: self.page_size,
            required_header: self.required_header,
            www_authenticate: self.www_authenticate,
            redirect_to: self.redirect_to,
            journal: Mutex::new(Vec::new()),
            peers: Mutex::new(Vec::new()),
            changes,
        });
        let ct = CancellationToken::new();
        let handler = MockHandler(Arc::clone(&shared));
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(self.generation != Generation::Modern)
            .with_sse_keep_alive(Some(Duration::from_secs(5)))
            .with_cancellation_token(ct.child_token());
        let service: StreamableHttpService<MockHandler, LocalSessionManager> =
            StreamableHttpService::new(move || Ok(handler.clone()), Arc::default(), config);
        let router = Router::new()
            .nest_service("/mcp", service)
            .route("/redirect", any(redirect))
            .layer(middleware::from_fn_with_state(Arc::clone(&shared), gatekeeper))
            .with_state(Arc::clone(&shared));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let shutdown = ct.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(MockMcpServer { addr, shared, ct })
    }
}

/// A running fake MCP server. Stops when dropped.
pub struct MockMcpServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    ct: CancellationToken,
}

impl MockMcpServer {
    pub fn builder() -> MockMcpBuilder {
        MockMcpBuilder {
            generation: Generation::Both,
            tools: Vec::new(),
            test_tools: false,
            page_size: None,
            required_header: None,
            www_authenticate: true,
            redirect_to: None,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>/mcp`.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// `http://127.0.0.1:<port>/redirect`.
    pub fn redirect_url(&self) -> String {
        format!("http://{}/redirect", self.addr)
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.shared.journal).clone()
    }

    /// JSON-RPC methods received so far, in order.
    pub fn mcp_methods(&self) -> Vec<String> {
        self.requests().into_iter().filter_map(|r| r.mcp_method).collect()
    }

    /// Replaces the tools (test tools stay). With `notify`, connected clients are told: by
    /// `notifications/tools/list_changed` on legacy sessions, through `subscriptions/listen` in 2026-07-28.
    pub async fn set_tools(&self, tools: Vec<Tool>, notify: bool) {
        *lock(&self.shared.tools) = tools;
        if !notify {
            return;
        }
        let peers = lock(&self.shared.peers).clone();
        for peer in peers {
            let _ = peer.notify_tool_list_changed().await;
        }
        let _ = self.shared.changes.send(());
    }

    pub fn shutdown(&self) {
        self.ct.cancel();
    }
}

impl Drop for MockMcpServer {
    fn drop(&mut self) {
        self.ct.cancel();
    }
}

fn plain(status: StatusCode, body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    response
}

fn header_text(headers: &HeaderMap, name: &HeaderName) -> String {
    headers
        .get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .unwrap_or_default()
}

/// Records every request, enforces the required header on `/mcp`, and serves `http_fail`.
async fn gatekeeper(State(shared): State<Arc<Shared>>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        return plain(StatusCode::PAYLOAD_TOO_LARGE, "corps trop grand".to_owned());
    };
    let json: Option<Value> = serde_json::from_slice(&bytes).ok();
    let mcp_method = json
        .as_ref()
        .and_then(|v| v.get("method"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let tool = json
        .as_ref()
        .filter(|_| mcp_method.as_deref() == Some("tools/call"))
        .and_then(|v| v.pointer("/params/name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    lock(&shared.journal).push(RecordedRequest {
        method: parts.method.clone(),
        path: parts.uri.path().to_owned(),
        headers: parts.headers.clone(),
        mcp_method,
        tool: tool.clone(),
    });

    if parts.uri.path().starts_with("/mcp") {
        if let Some((name, value)) = &shared.required_header
            && parts.headers.get(name) != Some(value)
        {
            let mut response = plain(StatusCode::UNAUTHORIZED, "authentification requise".to_owned());
            if shared.www_authenticate {
                response
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"mock\""));
            }
            return response;
        }
        if tool.as_deref() == Some("http_fail") {
            let received = header_text(&parts.headers, &AUTHORIZATION);
            return plain(
                StatusCode::BAD_REQUEST,
                format!("refus du serveur amont, en-tête reçu : {received}"),
            );
        }
    }
    next.run(Request::from_parts(parts, Body::from(bytes))).await
}

async fn redirect(State(shared): State<Arc<Shared>>) -> Response {
    let mut response = Response::new(Body::empty());
    match shared
        .redirect_to
        .as_deref()
        .and_then(|t| HeaderValue::from_str(t).ok())
    {
        Some(target) => {
            *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
            response.headers_mut().insert(LOCATION, target);
        }
        None => *response.status_mut() = StatusCode::NOT_FOUND,
    }
    response
}

#[derive(Clone)]
struct MockHandler(Arc<Shared>);

impl MockHandler {
    fn received_headers(context: &RequestContext<RoleServer>) -> HeaderMap {
        context
            .extensions
            .get::<http::request::Parts>()
            .map(|parts| parts.headers.clone())
            .unwrap_or_default()
    }
}

impl ServerHandler for MockHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new("mock-mcp", "1.0.0"))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        match self.0.generation {
            Generation::Legacy => Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25)),
            Generation::Modern => Cow::Borrowed(&[ProtocolVersion::V_2026_07_28]),
            Generation::Both => Cow::Borrowed(ProtocolVersion::KNOWN_VERSIONS),
        }
    }

    async fn discover(&self, _: RequestContext<RoleServer>) -> Result<DiscoverResult, ErrorData> {
        if self.0.generation == Generation::Legacy {
            return Err(ErrorData::new(ErrorCode::METHOD_NOT_FOUND, "Method not found", None));
        }
        Ok(DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        ))
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        lock(&self.0.peers).push(context.peer);
    }

    fn accepted_subscription_filter(&self, requested: &SubscriptionFilter) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mut changes = self.0.changes.subscribe();
        loop {
            tokio::select! {
                () = context.cancelled() => return Ok(()),
                change = changes.recv() => match change {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = context.sink().notify_tool_list_changed().await;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
            }
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.0.all_tools();
        let Some(size) = self.0.page_size else {
            return Ok(ListToolsResult::with_all_items(tools));
        };
        let start = match request.and_then(|r| r.cursor) {
            Some(cursor) => cursor
                .parse::<usize>()
                .map_err(|_| ErrorData::invalid_params("curseur invalide", None))?,
            None => 0,
        };
        let end = start.saturating_add(size).min(tools.len());
        let page = tools.get(start..end).map(<[Tool]>::to_vec).unwrap_or_default();
        let mut result = ListToolsResult::with_all_items(page);
        result.next_cursor = (end < tools.len()).then(|| end.to_string());
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.clone().unwrap_or_default());
        match request.name.as_ref() {
            "echo_headers" if self.0.test_tools => {
                let headers = Self::received_headers(&context);
                let map: serde_json::Map<String, Value> = headers
                    .iter()
                    .map(|(n, v)| {
                        (
                            n.as_str().to_owned(),
                            Value::String(String::from_utf8_lossy(v.as_bytes()).into_owned()),
                        )
                    })
                    .collect();
                Ok(CallToolResult::success(vec![ContentBlock::text(Value::Object(map).to_string())]).into())
            }
            "slow" if self.0.test_tools => {
                let millis = arguments.get("millis").and_then(Value::as_u64).unwrap_or(300);
                let steps = 3u32;
                for step in 1..=steps {
                    tokio::time::sleep(Duration::from_millis(millis) / steps).await;
                    if let Some(token) = context.meta.get_progress_token() {
                        let mut progress = ProgressNotificationParam::new(token, f64::from(step));
                        progress.total = Some(f64::from(steps));
                        progress.message = Some(format!("étape {step}"));
                        let _ = context.peer.notify_progress(progress).await;
                    }
                }
                Ok(CallToolResult::success(vec![ContentBlock::text("terminé")]).into())
            }
            "fail" if self.0.test_tools => {
                let received = header_text(&Self::received_headers(&context), &AUTHORIZATION);
                Err(ErrorData::internal_error(
                    format!("échec simulé, en-tête reçu : {received}"),
                    None,
                ))
            }
            name if lock(&self.0.tools).iter().any(|t| t.name == name) => {
                let body = json!({ "tool": name, "arguments": arguments });
                Ok(CallToolResult::success(vec![ContentBlock::text(body.to_string())]).into())
            }
            name => Err(ErrorData::invalid_params(format!("outil inconnu : {name}"), None)),
        }
    }
}
