//! HTTP backend of the Streamable HTTP transport, with the credential injected when each request is sent.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::stream::BoxStream;
use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::session::{SessionError, SessionGate};
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{StatusCode, redirect};
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, JsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse, StreamableHttpResponseLimits,
};
use sse_stream::Sse;
use tokio::sync::broadcast;
use url::Url;

type HttpError = StreamableHttpError<reqwest::Error>;

/// What happened to the credential during the exchanges of one transport, read by `HttpUpstream` to tell why a
/// request failed.
#[derive(Debug, Default)]
pub(super) struct InjectionLog {
    rejections: AtomicU64,
    refused: Mutex<Option<SessionError>>,
}

impl InjectionLog {
    pub(super) fn rejections(&self) -> u64 {
        self.rejections.load(Ordering::SeqCst)
    }

    /// The last injection refusal, if any, cleared by this call.
    pub(super) fn take_refusal(&self) -> Option<SessionError> {
        self.refused.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Wraps `reqwest::Client` for the SDK's Streamable HTTP transport:
///
/// - the authentication header is never part of the transport configuration: `SessionGate::inject` adds it to each
///   request right before sending, after checking the destination against the session's origins. If the check
///   fails, nothing is sent;
/// - redirects are never followed; TLS is native (the Windows certificate store on Windows);
/// - a 401 or 403, with or without `WWW-Authenticate`, marks the session rejected; the body of the response is
///   dropped;
/// - the response size limits of the transport are passed to the SDK's reqwest implementation, which enforces them.
#[derive(Clone)]
pub struct InjectingHttpClient {
    remote: reqwest::Client,
    /// Never goes through a proxy, so that a plain-`http` loopback request is not relayed elsewhere.
    loopback: reqwest::Client,
    gate: Arc<dyn SessionGate>,
    session: Option<SessionId>,
    log: Arc<InjectionLog>,
    /// Told of every accepted `initialize`, including the transparent re-initialization of an expired session: the
    /// server may have been restarted with other tools.
    initialized: Option<broadcast::Sender<()>>,
}

impl fmt::Debug for InjectingHttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InjectingHttpClient")
            .field("session", &self.session.as_ref().map(SessionId::as_str))
            .finish_non_exhaustive()
    }
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1")
}

/// Status of an error the SDK built from a 401 or 403 response, whatever its shape.
fn rejected_status(error: &HttpError) -> Option<StatusCode> {
    match error {
        StreamableHttpError::AuthRequired(_) => Some(StatusCode::UNAUTHORIZED),
        StreamableHttpError::InsufficientScope(_) => Some(StatusCode::FORBIDDEN),
        StreamableHttpError::Client(e) => e
            .status()
            .filter(|s| matches!(*s, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)),
        // The SDK's reqwest client formats an unchallenged 401/403 as `HTTP {status}: {body}`.
        StreamableHttpError::UnexpectedServerResponse(text) => [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN]
            .into_iter()
            .find(|s| text.starts_with(&format!("HTTP {s}"))),
        _ => None,
    }
}

impl InjectingHttpClient {
    /// `session` is `None` for a server that needs no credential: nothing is ever injected.
    pub fn new(
        gate: Arc<dyn SessionGate>,
        session: Option<SessionId>,
        connect_timeout: Duration,
    ) -> Result<Self, reqwest::Error> {
        let builder = || {
            reqwest::Client::builder()
                .tls_backend_native()
                .redirect(redirect::Policy::none())
                .referer(false)
                .connect_timeout(connect_timeout)
        };
        Ok(Self {
            remote: builder().build()?,
            loopback: builder().no_proxy().build()?,
            gate,
            session,
            log: Arc::default(),
            initialized: None,
        })
    }

    pub(super) fn share_log(&mut self, log: Arc<InjectionLog>) {
        self.log = log;
    }

    pub(super) fn report_initializations(&mut self, sender: broadcast::Sender<()>) {
        self.initialized = Some(sender);
    }

    /// The headers to send to `uri`: the transport's, without any authentication header, plus the session's
    /// credential when the session is active. An error means nothing must be sent.
    fn prepare(
        &self,
        uri: &str,
        custom: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(&reqwest::Client, HashMap<HeaderName, HeaderValue>), HttpError> {
        let url = Url::parse(uri).map_err(|_| refused_error())?;
        let client = if is_loopback(&url) {
            &self.loopback
        } else {
            &self.remote
        };
        let mut headers = custom;
        headers.remove(&AUTHORIZATION);
        headers.remove(&COOKIE);
        let Some(session) = self.session.as_ref().filter(|s| self.gate.is_active(s)) else {
            return Ok((client, headers));
        };
        let mut injected = HeaderMap::new();
        if let Err(error) = self.gate.inject(session, &url, &mut injected) {
            *self.log.refused.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
            return Err(refused_error());
        }
        for (name, value) in &injected {
            headers.insert(name.clone(), value.clone());
        }
        Ok((client, headers))
    }

    /// Marks the session rejected on a 401 or 403, and drops the body the SDK may have kept.
    fn checked<T>(&self, result: Result<T, HttpError>) -> Result<T, HttpError> {
        match result {
            Err(error) => match rejected_status(&error) {
                Some(status) => {
                    self.log.rejections.fetch_add(1, Ordering::SeqCst);
                    if let Some(session) = &self.session {
                        self.gate.mark_rejected(session);
                    }
                    Err(StreamableHttpError::UnexpectedServerResponse(
                        format!("HTTP {status} : identifiant refusé").into(),
                    ))
                }
                None => Err(error),
            },
            ok => ok,
        }
    }
}

fn refused_error() -> HttpError {
    StreamableHttpError::Io(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "en-tête d'authentification non injecté : requête non envoyée",
    ))
}

impl StreamableHttpClient for InjectingHttpClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        self.post_message_with_response_limits(
            uri,
            message,
            session_id,
            None,
            custom_headers,
            StreamableHttpResponseLimits::default(),
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        let mut limits = StreamableHttpResponseLimits::default();
        limits.max_sse_event_size = max_sse_event_size;
        self.post_message_with_response_limits(uri, message, session_id, None, custom_headers, limits)
            .await
    }

    async fn post_message_with_response_limits(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        limits: StreamableHttpResponseLimits,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        let (client, headers) = self.prepare(&uri, custom_headers)?;
        let initialize = matches!(
            &message,
            JsonRpcMessage::Request(request) if matches!(request.request, ClientRequest::InitializeRequest(_))
        );
        let result = client
            .post_message_with_response_limits(uri, message, session_id, None, headers, limits)
            .await;
        let result = self.checked(result);
        if initialize
            && result.is_ok()
            && let Some(sender) = &self.initialized
        {
            let _ = sender.send(());
        }
        result
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), HttpError> {
        let (client, headers) = self.prepare(&uri, custom_headers)?;
        let result = client.delete_session(uri, session_id, None, headers).await;
        self.checked(result)
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, HttpError> {
        let (client, headers) = self.prepare(&uri, custom_headers)?;
        let result = client.get_stream(uri, session_id, last_event_id, None, headers).await;
        self.checked(result)
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, HttpError> {
        let (client, headers) = self.prepare(&uri, custom_headers)?;
        let result = client
            .get_stream_with_max_sse_event_size(uri, session_id, last_event_id, None, headers, max_sse_event_size)
            .await;
        self.checked(result)
    }
}
