//! `snapshot-catalog --url <url> --out <file> [--header-env <NAME>]`, run by `cargo xtask snapshot-catalog`.
//!
//! Lists every page of a server's tools and writes them in the format of the embedded catalogue (spec 005). With
//! `--header-env`, the variable holds a bearer token sent as `Authorization: Bearer …` to the URL's origin only. The
//! token is never printed: every message and the file go through the output filter, which knows it.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_core::config::{EnvId, FilterConfig, McpId, Origin, SessionId};
use mcp_proxy_core::filter::OutputFilter;
use mcp_proxy_core::secrets::{Credential, OriginGuard, SecretStore};
use mcp_proxy_core::session::{SessionGate, StoreSessionGate};
use mcp_proxy_gateway::registry::InstanceId;
use mcp_proxy_gateway::upstream::{HttpUpstream, HttpUpstreamConfig, Upstream};
use secrecy::SecretString;
use url::Url;

const USAGE: &str = "usage : snapshot-catalog --url <url> --out <fichier> [--header-env <VARIABLE>]";
const TIMEOUT: Duration = Duration::from_secs(60);

struct Args {
    url: Url,
    out: PathBuf,
    header_env: Option<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let (mut url, mut out, mut header_env) = (None, None, None);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let value = iter.next().ok_or_else(|| format!("valeur manquante après {flag}"))?;
        match flag.as_str() {
            "--url" => url = Some(Url::parse(value).map_err(|_| "URL invalide".to_owned())?),
            "--out" => out = Some(PathBuf::from(value)),
            "--header-env" => header_env = Some(value.clone()),
            other => return Err(format!("option inconnue : {other}")),
        }
    }
    Ok(Args {
        url: url.ok_or("--url manquant")?,
        out: out.ok_or("--out manquant")?,
        header_env,
    })
}

/// Fails with a message that may quote the server: the caller filters it.
async fn run(args: Args, store: Arc<SecretStore>, filter: &OutputFilter) -> Result<usize, String> {
    let session = SessionId::new("snapshot").map_err(|e| e.to_string())?;
    let mut origins = BTreeSet::new();
    if let Some(name) = &args.header_env {
        let token = std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("variable d'environnement {name} absente ou vide"))?;
        store
            .put(
                &session,
                Credential::Bearer {
                    token: SecretString::from(token),
                },
            )
            .map_err(|e| e.to_string())?;
        origins.extend(Origin::of(&args.url));
    }
    let guard = Arc::new(OriginGuard::new([(session.clone(), origins)]));
    let gate: Arc<dyn SessionGate> = Arc::new(StoreSessionGate::new(Arc::clone(&store), guard));
    let instance = InstanceId::new(
        EnvId::new("snapshot").map_err(|e| e.to_string())?,
        McpId::new("snapshot").map_err(|e| e.to_string())?,
    );
    let session = args.header_env.is_some().then_some(session);
    let mut target = HttpUpstreamConfig::new(instance, args.url, session, TIMEOUT);
    target.list_requires_auth = target.session.is_some();
    let tools = HttpUpstream::new(target, gate)
        .list_tools()
        .await
        .map_err(|e| e.to_string())?;

    let mut value = serde_json::to_value(&tools).map_err(|e| e.to_string())?;
    filter.filter_json(&mut value).map_err(|e| e.to_string())?;
    let mut text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(&args.out, text).map_err(|e| format!("écriture de {} impossible : {e}", args.out.display()))?;
    Ok(tools.len())
}

#[tokio::main]
async fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&raw) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("snapshot-catalog : {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let out = args.out.display().to_string();
    let store = match SecretStore::for_platform() {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("snapshot-catalog : {error}");
            return ExitCode::FAILURE;
        }
    };
    let filter = match OutputFilter::new(Arc::clone(&store), &FilterConfig::default()) {
        Ok(filter) => filter,
        Err(error) => {
            eprintln!("snapshot-catalog : {error}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = run(args, store, &filter).await;
    let report = |text: String| match filter.filter_text(&text) {
        Ok((filtered, _)) => filtered.into_owned(),
        Err(_) => "message non affiché : filtre de sortie indisponible".to_owned(),
    };
    match outcome {
        Ok(count) => {
            println!("{}", report(format!("{count} outils écrits dans {out}")));
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("snapshot-catalog : {}", report(message));
            ExitCode::FAILURE
        }
    }
}
