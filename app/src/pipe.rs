//! The named pipe of the application: first instance required, remote clients rejected, default security
//! descriptor (tokio's safe API). Every connection runs the mutual proof, then the MCP server.

use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_gateway::bridge::PeerProof;
use mcp_proxy_gateway::runtime::{Gateway, StartupFailure};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// `CreateNamedPipe` with `FILE_FLAG_FIRST_PIPE_INSTANCE` on an existing pipe.
const ERROR_ACCESS_DENIED: i32 = 5;

fn options(first: bool) -> ServerOptions {
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true);
    options
}

/// The first instance, created at start: if the name exists, another program holds it and the application stops.
/// Must run inside the tokio runtime.
pub fn create_first(name: &str) -> Result<NamedPipeServer, StartupFailure> {
    options(true).create(name).map_err(|error| {
        if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
            StartupFailure::PipeTaken
        } else {
            StartupFailure::Pipe(error.kind().to_string())
        }
    })
}

/// Serves connections forever: each connected instance is replaced by a new one before it is handed over.
pub async fn accept(mut server: NamedPipeServer, name: String, gateway: Arc<Gateway>, proof: Arc<dyn PeerProof>) {
    loop {
        if let Err(error) = server.connect().await {
            tracing::warn!(kind = %error.kind(), "connexion au canal en échec");
            server = next_instance(&name).await;
            continue;
        }
        let connected = std::mem::replace(&mut server, next_instance(&name).await);
        let gateway = Arc::clone(&gateway);
        let proof = Arc::clone(&proof);
        tokio::spawn(async move {
            let _ = gateway.serve_connection(proof, connected).await;
        });
    }
}

/// A new instance of the pipe; retried every second while it cannot be created.
async fn next_instance(name: &str) -> NamedPipeServer {
    loop {
        match options(false).create(name) {
            Ok(server) => return server,
            Err(error) => {
                tracing::error!(kind = %error.kind(), "nouvelle instance du canal impossible à créer");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}
