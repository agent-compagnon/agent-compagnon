//! `LocalUpstream`: one instance of a local module (spec 025), behind the same `Upstream` contract as a remote MCP,
//! so that its calls go through the whole call chain.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mcp_proxy_core::config::{Config, McpKind, Origin, SessionId};
use mcp_proxy_core::module::{LocalModule, ModuleContext, ModuleError, ModuleRegistry, ToolOutput};
use mcp_proxy_core::secrets::{AuthenticatedHttpClient, RejectionCounter, SecretError};
use mcp_proxy_core::session::{SessionError, SessionGate};
use rmcp::model::{CallToolResult, ContentBlock};
use thiserror::Error;
use tokio::sync::broadcast;
use url::Url;

use super::{Upstream, UpstreamCall, UpstreamError};
use crate::registry::{InstanceId, ToolDefinition};

/// Why an instance of a local module has no upstream. French.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LocalUpstreamError {
    #[error("instance {0} inconnue de la config")]
    UnknownInstance(InstanceId),
    #[error("{0} n'est pas un module local")]
    NotLocal(InstanceId),
    #[error("{instance} : module local « {module} » absent de l'exécutable")]
    UnknownModule { instance: InstanceId, module: String },
    #[error("{0} : aucune session pour cette instance")]
    NoSession(InstanceId),
    #[error("{0} : adresse de base sans origine")]
    NoOrigin(InstanceId),
    #[error("{instance} : {error}")]
    Client { instance: InstanceId, error: SecretError },
}

/// How to run one instance of a local module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalUpstreamConfig {
    pub instance: InstanceId,
    pub module: String,
    pub base_url: Url,
    pub session: SessionId,
    /// Bounds each call of the module, and each of its requests.
    pub timeout: Duration,
}

impl LocalUpstreamConfig {
    /// From the configuration: module, `base_url`, session of the MCP, `upstream_timeout_secs`.
    pub fn for_instance(config: &Config, instance: &InstanceId) -> Result<Self, LocalUpstreamError> {
        let mcp = config
            .mcp(&instance.mcp)
            .ok_or_else(|| LocalUpstreamError::UnknownInstance(instance.clone()))?;
        let McpKind::Local { module } = &mcp.kind else {
            return Err(LocalUpstreamError::NotLocal(instance.clone()));
        };
        let base_url = mcp
            .instances
            .get(&instance.env)
            .ok_or_else(|| LocalUpstreamError::UnknownInstance(instance.clone()))?
            .clone();
        let session = config
            .session_for(&instance.mcp, &instance.env)
            .ok_or_else(|| LocalUpstreamError::NoSession(instance.clone()))?
            .id
            .clone();
        Ok(Self {
            instance: instance.clone(),
            module: module.clone(),
            base_url,
            session,
            timeout: Duration::from_secs(config.gateway.upstream_timeout_secs),
        })
    }
}

pub struct LocalUpstream {
    instance: InstanceId,
    module: Arc<dyn LocalModule>,
    context: ModuleContext,
    /// 401 and 403 seen by the context's client: the module cannot hide nor fake one.
    rejections: RejectionCounter,
    timeout: Duration,
    /// Never fires: the tools of a module only change with the executable.
    changes: broadcast::Sender<()>,
}

impl fmt::Debug for LocalUpstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalUpstream")
            .field("instance", &self.instance)
            .field("module", &self.module.id())
            .field("base_url", &self.context.base_url.as_str())
            .finish_non_exhaustive()
    }
}

fn result_from(output: ToolOutput) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(output.text)]);
    result.structured_content = output.structured;
    result
}

fn tool_error(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

impl LocalUpstream {
    /// The module's context gets a client injecting through `gate`, narrowed to the origin of `base_url`.
    pub fn new(
        config: LocalUpstreamConfig,
        module: Arc<dyn LocalModule>,
        gate: Arc<dyn SessionGate>,
    ) -> Result<Self, LocalUpstreamError> {
        let instance = config.instance;
        if module.id() != config.module {
            return Err(LocalUpstreamError::UnknownModule {
                instance,
                module: config.module,
            });
        }
        let origin = Origin::of(&config.base_url).ok_or_else(|| LocalUpstreamError::NoOrigin(instance.clone()))?;
        let (http, rejections) =
            AuthenticatedHttpClient::through_gate(config.session, gate, config.timeout).map_err(|error| {
                LocalUpstreamError::Client {
                    instance: instance.clone(),
                    error,
                }
            })?;
        let context = ModuleContext {
            http: http.only_towards(origin),
            base_url: config.base_url,
            env: instance.env.clone(),
        };
        Ok(Self {
            instance,
            module,
            context,
            rejections,
            timeout: config.timeout,
            changes: broadcast::channel(1).0,
        })
    }

    /// An upstream for every instance of every local MCP of `config` whose module is in `modules`. Validation
    /// refuses unknown modules, so every instance should have one; one that has not is reported and left without an
    /// upstream (its calls are refused).
    pub fn all_from_config(
        config: &Config,
        modules: &ModuleRegistry,
        gate: &Arc<dyn SessionGate>,
    ) -> Vec<Result<Self, LocalUpstreamError>> {
        config
            .mcp
            .iter()
            .filter_map(|mcp| match &mcp.kind {
                McpKind::Local { module } => Some((mcp, module)),
                McpKind::Remote => None,
            })
            .flat_map(|(mcp, module)| {
                mcp.instances.keys().map(move |env| {
                    let instance = InstanceId::new(env.clone(), mcp.id.clone());
                    let target = LocalUpstreamConfig::for_instance(config, &instance)?;
                    let module = modules.get(module).ok_or_else(|| LocalUpstreamError::UnknownModule {
                        instance: instance.clone(),
                        module: module.clone(),
                    })?;
                    Self::new(target, Arc::clone(module), Arc::clone(gate))
                })
            })
            .collect()
    }

    pub fn context(&self) -> &ModuleContext {
        &self.context
    }
}

#[async_trait]
impl Upstream for LocalUpstream {
    fn instance(&self) -> &InstanceId {
        &self.instance
    }

    async fn list_tools(&self) -> Result<Vec<ToolDefinition>, UpstreamError> {
        Ok(self.module.tools())
    }

    async fn call_tool(&self, call: UpstreamCall) -> Result<CallToolResult, UpstreamError> {
        let before = self.rejections.count();
        let arguments = call.arguments.unwrap_or_default();
        let outcome = tokio::time::timeout(
            self.timeout,
            self.module.call(&self.context, call.tool.as_str(), arguments),
        )
        .await;
        // The client has already marked the secret it sent rejected: the chain logs in again and replays once.
        if self.rejections.count() != before {
            return Err(UpstreamError::AuthRejected);
        }
        match outcome {
            Err(_) => Err(UpstreamError::Timeout(self.timeout)),
            Ok(Ok(output)) => Ok(result_from(output)),
            Ok(Err(ModuleError::Http(SecretError::Timeout))) => Err(UpstreamError::Timeout(self.timeout)),
            Ok(Err(ModuleError::Http(SecretError::Connect))) => {
                Err(UpstreamError::Connect("service injoignable".to_owned()))
            }
            Ok(Err(ModuleError::Http(SecretError::Transport))) => {
                Err(UpstreamError::Protocol("échange avec le service interrompu".to_owned()))
            }
            Ok(Err(ModuleError::Http(error))) => Err(UpstreamError::Session(SessionError::Injection(error))),
            // `CredentialRejected` lands here when the client saw no 401 nor 403: the module cannot expire a session.
            Ok(Err(error)) => Ok(tool_error(error.to_string())),
        }
    }

    fn changes(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }
}
