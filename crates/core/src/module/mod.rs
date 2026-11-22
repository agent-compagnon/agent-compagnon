//! Contract of the local MCP modules (spec 025): MCPs written inside the application, for services that have no MCP
//! server.
//!
//! A module crate depends on this module only. What it is given, `ModuleContext`, holds an HTTP client already bound
//! to its session and to the origin of its instance, the instance's base URL and its environment: nothing reaches
//! the secret store, the injected headers or the version of the secret (INV-4). A module is called through the same
//! call chain as a remote MCP (policy, argument scan, rate limit, confirmation, session, output filter, trace): it
//! never sees a call the chain refused, and what it returns is filtered before leaving.

mod tool;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub use async_trait::async_trait;
pub use reqwest::header::{self, HeaderValue};
pub use reqwest::{Method, Request, Response, StatusCode};
pub use serde_json::{Map, Value, from_slice as json_from_slice, json};
use thiserror::Error;
pub use url::Url;

pub use self::tool::{ToolDefinition, ToolHints};
pub use crate::config::EnvId;
pub use crate::secrets::{AuthenticatedHttpClient, SecretError};

/// Arguments of a tool call, as received from the client.
pub type JsonObject = Map<String, Value>;

/// What a module is given for each call of one instance.
#[derive(Debug)]
pub struct ModuleContext {
    /// Bound to the session of the instance and narrowed to the origin of `base_url`.
    pub http: AuthenticatedHttpClient,
    pub base_url: Url,
    pub env: EnvId,
}

/// The answer of a tool: a text for the LLM and, optionally, the same content structured.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Option<Value>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            structured: None,
        }
    }

    #[must_use]
    pub fn with_structured(mut self, structured: Value) -> Self {
        self.structured = Some(structured);
        self
    }
}

/// Why a module could not answer. Messages are in French, written for the LLM: they must not quote a raw response
/// body. They still go through the output filter.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ModuleError {
    #[error("outil inconnu de ce module : {0}")]
    UnknownTool(String),
    #[error("arguments invalides : {0}")]
    InvalidArguments(String),
    /// The service refused the credential. The gateway only acts on it (new login, one replay) if the HTTP client saw
    /// a 401 or 403: a module cannot expire a session on its own.
    #[error("identifiant refusé par le service")]
    CredentialRejected,
    /// The request could not be sent or received: destination refused, no usable secret, timeout, network.
    #[error("{0}")]
    Http(#[from] SecretError),
    /// Any other failure, in a clear sentence (`ticket introuvable : OPS-1`).
    #[error("{0}")]
    Failed(String),
}

#[async_trait]
pub trait LocalModule: Send + Sync {
    /// The value of `module` in the configuration: `[a-z0-9-]`, unique among the modules of the executable.
    fn id(&self) -> &'static str;

    /// Every tool of the module. Names are the original names, without the environment prefix.
    fn tools(&self) -> Vec<ToolDefinition>;

    async fn call(&self, ctx: &ModuleContext, tool: &str, args: JsonObject) -> Result<ToolOutput, ModuleError>;
}

/// The modules compiled into the executable, by id.
#[derive(Clone, Default)]
pub struct ModuleRegistry {
    modules: BTreeMap<&'static str, Arc<dyn LocalModule>>,
}

impl fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.modules.keys()).finish()
    }
}

/// Two modules of the executable share an id.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("module local « {0} » déclaré deux fois")]
pub struct DuplicateModule(pub &'static str);

impl ModuleRegistry {
    pub fn new(modules: impl IntoIterator<Item = Arc<dyn LocalModule>>) -> Result<Self, DuplicateModule> {
        let mut registry = Self::default();
        for module in modules {
            let id = module.id();
            if registry.modules.insert(id, module).is_some() {
                return Err(DuplicateModule(id));
            }
        }
        Ok(registry)
    }

    /// The ids, sorted: what configuration validation accepts for `module`.
    pub fn ids(&self) -> Vec<&'static str> {
        self.modules.keys().copied().collect()
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn LocalModule>> {
        self.modules.get(id)
    }
}

/// Largest response body `read_body` accepts by default.
pub const DEFAULT_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// The body of `response`, refused beyond `limit` bytes.
pub async fn read_body(mut response: Response, limit: usize) -> Result<Vec<u8>, ModuleError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        if e.is_timeout() {
            SecretError::Timeout
        } else {
            SecretError::Transport
        }
    })? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ModuleError::Failed(format!(
                "réponse du service trop volumineuse (plus de {} Kio)",
                limit / 1024
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
