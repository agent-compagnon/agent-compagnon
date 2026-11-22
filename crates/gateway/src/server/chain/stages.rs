//! Every stage but the confirmation.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use mcp_proxy_core::config::{Decision, SessionId};
use mcp_proxy_core::session::{AccessReason, SessionError};
use mcp_proxy_core::trace::CallStatus;
use rmcp::model::CallToolResult;
use serde_json::Value;

use super::{CallContext, Halt, Middleware, Refusal, ResolvedTool, Shared, suspended};
use crate::registry::ToolHints;
use crate::upstream::{Upstream, UpstreamCall, UpstreamError};

fn refuse(status: CallStatus, message: impl Into<String>) -> Result<(), Halt> {
    Err(Refusal::new(status, message).into())
}

pub(super) fn cancelled() -> Halt {
    Refusal::new(CallStatus::Cancelled, "appel annulé par le client.").into()
}

/// Whole seconds, rounded up, at least 1: what the user reads.
pub(super) fn whole_seconds(duration: std::time::Duration) -> u64 {
    duration.as_secs_f64().ceil().max(1.0) as u64
}

pub(super) struct Resolution(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for Resolution {
    fn name(&self) -> &'static str {
        "resolution"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        // Only `resolve` may serve a call: it knows nothing of tools that are not exposed.
        let resolved = {
            let registry = self.0.registry.read().await;
            registry.resolve(&context.exposed_name).map(|entry| {
                (
                    entry.instance.clone(),
                    entry.original_name.clone(),
                    entry.server_annotations.as_ref().map(ToolHints::policy_annotations),
                )
            })
        };
        let Some((instance, tool, annotations)) = resolved else {
            return refuse(
                CallStatus::Denied,
                format!(
                    "outil « {} » inconnu ou non exposé, appel refusé.",
                    context.exposed_name
                ),
            );
        };
        let config = &self.0.config;
        let env_label = config
            .environment(&instance.env)
            .map_or_else(|| instance.env.to_string(), |e| e.label.clone());
        let mcp = config.mcp(&instance.mcp);
        let session = config.session_for(&instance.mcp, &instance.env);
        context.tool = Some(ResolvedTool {
            env_label,
            mcp_label: mcp.map_or_else(|| instance.mcp.to_string(), |m| m.label.clone()),
            rate_limit: mcp.and_then(|m| m.rate_limit),
            session: session.map(|s| s.id.clone()),
            session_label: session.map(|s| s.label.clone()).unwrap_or_default(),
            instance,
            tool,
            annotations,
        });
        Ok(())
    }
}

pub(super) struct Suspension(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for Suspension {
    fn name(&self) -> &'static str {
        "suspension"
    }

    async fn handle(&self, _: &mut CallContext) -> Result<(), Halt> {
        if self.0.is_suspended() {
            return Err(suspended());
        }
        Ok(())
    }
}

pub(super) struct Policy(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for Policy {
    fn name(&self) -> &'static str {
        "policy"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        let tool = context.resolved()?.clone();
        let config = &self.0.config;
        let resolved = {
            let settings = self.0.settings.borrow();
            settings.engine(config).map(|engine| {
                let resolution = engine.resolve(
                    &tool.instance.env,
                    &tool.instance.mcp,
                    &tool.tool,
                    tool.annotations.as_ref(),
                );
                let explanation = resolution.describe(config);
                (resolution, explanation)
            })
        };
        let Some((resolution, explanation)) = resolved else {
            return refuse(
                CallStatus::Denied,
                "aucun profil de politique applicable, appel refusé.",
            );
        };
        let decision = resolution.decision;
        context.policy = Some(resolution);
        context.explanation = Some(explanation.clone());
        if decision == Decision::Deny {
            return refuse(
                CallStatus::Denied,
                format!(
                    "appel interdit par la politique ({}). Raison : {explanation}.",
                    tool.short_path()
                ),
            );
        }
        Ok(())
    }
}

pub(super) struct Arguments(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for Arguments {
    fn name(&self) -> &'static str {
        "arguments"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        let arguments = Value::Object(context.args.clone().unwrap_or_default());
        match self.0.output.filter().contains_known_secret(&arguments) {
            Ok(false) => Ok(()),
            Ok(true) => refuse(
                CallStatus::Blocked,
                "appel bloqué, les arguments contiennent un secret connu.",
            ),
            Err(_) => refuse(
                CallStatus::Blocked,
                "appel bloqué, le filtre est indisponible pour vérifier les arguments.",
            ),
        }
    }
}

pub(super) struct RateLimit(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for RateLimit {
    fn name(&self) -> &'static str {
        "rate_limit"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        let tool = context.resolved()?;
        let Some(limit) = &tool.rate_limit else {
            return Ok(());
        };
        match self.0.limiter.acquire(&tool.instance, limit, Instant::now()) {
            Ok(()) => Ok(()),
            Err(wait) => refuse(
                CallStatus::RateLimited,
                format!(
                    "limite de débit atteinte pour {} ({}). Réessayez dans {} s.",
                    tool.mcp_label,
                    tool.env_label,
                    whole_seconds(wait)
                ),
            ),
        }
    }
}

/// `ensure`, bounded by what is left of the budget and by `auth_timeout_secs`, whatever the gate does.
async fn ensure_session(shared: &Shared, context: &CallContext, session: &SessionId) -> Result<(), Halt> {
    let tool = context.resolved()?;
    let auth_timeout = std::time::Duration::from_secs(shared.config.gateway.auth_timeout_secs);
    let deadline = context.deadline.min(Instant::now() + auth_timeout);
    let reason = AccessReason::ToolCall {
        env: tool.instance.env.clone(),
        mcp: tool.instance.mcp.clone(),
        tool: tool.tool.clone(),
        // Shown in the login notification (spec 013).
        client: shared.output.text(&context.client.name).unwrap_or_default(),
    };
    let ensured = tokio::select! {
        biased;
        () = shared.suspended() => return Err(suspended()),
        ensured = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            shared.gate.ensure(session, deadline, &reason),
        ) => ensured,
        () = context.cancel.cancelled() => return Err(cancelled()),
    };
    let label = &tool.session_label;
    let message = match ensured {
        Ok(Ok(())) => return Ok(()),
        Err(_) | Ok(Err(SessionError::LoginPending(_))) => format!(
            "connexion à « {label} » requise. La fenêtre de connexion est ouverte sur le poste de l'utilisateur. \
             Réessayez une fois la connexion faite."
        ),
        Ok(Err(SessionError::Cancelled(_))) => {
            format!("connexion à « {label} » annulée par l'utilisateur, appel refusé.")
        }
        Ok(Err(SessionError::LoginFailed(_))) => format!(
            "connexion à « {label} » échouée, appel refusé. Relancez la connexion depuis MCP Proxy, puis réessayez."
        ),
        Ok(Err(SessionError::Disconnected(_))) => format!(
            "connexion à « {label} » interrompue, l'utilisateur a déconnecté toutes les sessions ; appel refusé. \
             Réessayez : une nouvelle connexion sera demandée."
        ),
        Ok(Err(SessionError::Unavailable(_))) => {
            format!("connexion à « {label} » impossible pour le moment, appel refusé.")
        }
        Ok(Err(error)) => format!("session « {label} » inutilisable : {error}."),
    };
    Err(Refusal::new(CallStatus::AuthRequired, message).into())
}

pub(super) struct Session(pub(super) Arc<Shared>);

#[async_trait]
impl Middleware for Session {
    fn name(&self) -> &'static str {
        "session"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        match context.resolved()?.session.clone() {
            Some(session) => ensure_session(&self.0, context, &session).await,
            None => Ok(()),
        }
    }
}

pub(super) struct UpstreamCallStage(pub(super) Arc<Shared>);

async fn call_once(upstream: &dyn Upstream, context: &CallContext) -> Result<CallToolResult, UpstreamError> {
    let call = UpstreamCall {
        tool: context
            .resolved()
            .map(|t| t.tool.clone())
            .map_err(|_| UpstreamError::Cancelled)?,
        arguments: context.args.clone(),
        progress: context.progress.clone(),
    };
    tokio::select! {
        outcome = upstream.call_tool(call) => outcome,
        () = context.cancel.cancelled() => Err(UpstreamError::Cancelled),
    }
}

#[async_trait]
impl Middleware for UpstreamCallStage {
    fn name(&self) -> &'static str {
        "upstream"
    }

    async fn handle(&self, context: &mut CallContext) -> Result<(), Halt> {
        let tool = context.resolved()?.clone();
        // Suspended while the call was confirmed or connected: nothing leaves.
        if self.0.is_suspended() {
            return Err(suspended());
        }
        let Some(upstream) = self.0.upstreams.get(&tool.instance).cloned() else {
            return refuse(
                CallStatus::Error,
                format!("aucun serveur disponible pour {}.", tool.instance),
            );
        };
        let mut outcome = call_once(upstream.as_ref(), context).await;
        // The upstream has already marked the session rejected and wiped its secret: one new login, one replay.
        if let (Err(UpstreamError::AuthRejected), Some(session)) = (&outcome, &tool.session) {
            ensure_session(&self.0, context, session).await?;
            outcome = call_once(upstream.as_ref(), context).await;
            if matches!(outcome, Err(UpstreamError::AuthRejected)) {
                return refuse(
                    CallStatus::AuthRequired,
                    format!(
                        "{} : identifiant refusé par le serveur, même après une nouvelle connexion ; appel abandonné.",
                        tool.instance
                    ),
                );
            }
        }
        match outcome {
            Ok(result) => {
                context.result = Some(result);
                Ok(())
            }
            Err(UpstreamError::Cancelled) if context.cancel.is_cancelled() => Err(cancelled()),
            Err(UpstreamError::AuthRejected) => refuse(
                CallStatus::AuthRequired,
                format!("{} : {}", tool.instance, UpstreamError::AuthRejected),
            ),
            Err(error) => refuse(CallStatus::Error, format!("{} : {error}", tool.instance)),
        }
    }
}
