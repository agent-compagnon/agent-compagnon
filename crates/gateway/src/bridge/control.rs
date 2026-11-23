//! The control channel of the pipe (spec 019): after the mutual proof, a client proved in the control role may ask
//! the application to stop, or ask whether it answers (`ping`, spec 024), and nothing else. The installation of a
//! new version and the uninstallation ask for the stop; the self-diagnostic asks the ping, which has no effect at
//! all: nothing is called, journaled or shown, and the connection closes after the answer. A bridge proves itself in
//! the bridge role and only ever relays MCP: no byte an LLM client sends reaches this channel.
//!
//! The client sends one request frame, the application answers one frame. After a stop, it keeps the connection open
//! until its process ends: the client therefore learns that the application is gone when the pipe closes, within
//! [`STOP_TIMEOUT`].

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

use super::proof::{PROOF_TIMEOUT, PeerProof, ProofError, Role, prove, read_frame, write_frame};

const STOP_REQUEST: &[u8] = b"mcp-proxy.control.v1\0stop";
const STOP_ACK: &[u8] = b"mcp-proxy.control.v1\0ok";
const PING_REQUEST: &[u8] = b"mcp-proxy.control.v1\0ping";
/// Followed by the version of the running application.
const PONG_PREFIX: &[u8] = b"mcp-proxy.control.v1\0pong\0";
/// Longest version accepted in an answer.
const VERSION_MAX: usize = 64;
/// From the acknowledgement to the end of the application's process: it refuses its confirmations, flushes its
/// journal and closes its windows.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Why the running application could not be stopped. French; never carries a nonce.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StopError {
    #[error("{0}")]
    Proof(#[from] ProofError),
    #[error("l'appli n'a pas accepté la demande d'arrêt")]
    Refused,
    #[error("l'appli ne s'est pas arrêtée dans les {} s", STOP_TIMEOUT.as_secs())]
    StillRunning,
    #[error("canal fermé avant la réponse de l'appli")]
    Closed,
}

/// The installation's side: proves itself as [`Role::Control`], asks the application to stop, and returns once its
/// end of the pipe is closed.
pub async fn request_stop<P>(proof: Arc<dyn PeerProof>, mut pipe: P) -> Result<(), StopError>
where
    P: AsyncRead + AsyncWrite + Unpin,
{
    prove(proof, Role::Control, &mut pipe).await?;
    let answer = tokio::time::timeout(PROOF_TIMEOUT, async {
        write_frame(&mut pipe, STOP_REQUEST).await?;
        read_frame(&mut pipe).await
    })
    .await;
    match answer {
        Ok(Ok(answer)) if answer == STOP_ACK => {}
        Ok(Ok(_)) => return Err(StopError::Refused),
        Ok(Err(_)) => return Err(StopError::Closed),
        Err(_) => return Err(StopError::Proof(ProofError::Timeout)),
    }
    wait_closed(&mut pipe).await
}

/// Nothing more is ever sent: reading ends when the application's process closes the pipe.
async fn wait_closed<P: AsyncRead + Unpin>(pipe: &mut P) -> Result<(), StopError> {
    let mut rest = [0u8; 64];
    let gone = tokio::time::timeout(STOP_TIMEOUT, async {
        loop {
            match pipe.read(&mut rest).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
    gone.map_err(|_| StopError::StillRunning)
}

/// The self-diagnostic's side (spec 024): proves itself as [`Role::Control`] and asks whether the application
/// answers. `Some(version)`: it answered with its version; `None`: it proved itself, then closed the channel, as a
/// version older than the self-diagnostic does. The application stops nothing and shows nothing.
pub async fn request_ping<P>(proof: Arc<dyn PeerProof>, mut pipe: P) -> Result<Option<String>, ProofError>
where
    P: AsyncRead + AsyncWrite + Unpin,
{
    prove(proof, Role::Control, &mut pipe).await?;
    let answer = tokio::time::timeout(PROOF_TIMEOUT, async {
        write_frame(&mut pipe, PING_REQUEST).await?;
        read_frame(&mut pipe).await
    })
    .await
    .map_err(|_| ProofError::Timeout)?;
    let Ok(answer) = answer else { return Ok(None) };
    let version = answer
        .strip_prefix(PONG_PREFIX)
        .filter(|v| !v.is_empty() && v.len() <= VERSION_MAX && v.iter().all(|b| b.is_ascii_graphic()))
        .and_then(|v| std::str::from_utf8(v).ok())
        .map(str::to_owned);
    version.map(Some).ok_or(ProofError::Malformed)
}

/// What a proved control client asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRequest {
    Stop,
    /// Answered with `version`; nothing else happens.
    Ping,
}

/// The application's side, once the client proved the control role: reads the request (bounded by
/// [`PROOF_TIMEOUT`]). A stop calls `on_stop`, then acknowledges; a ping answers `version` and nothing else.
/// Anything else closes the connection, `on_stop` not called.
pub async fn serve_control<S>(
    stream: &mut S,
    version: &str,
    on_stop: impl FnOnce(),
) -> Result<ControlRequest, ProofError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = tokio::time::timeout(PROOF_TIMEOUT, read_frame(stream))
        .await
        .map_err(|_| ProofError::Timeout)??;
    if request == PING_REQUEST {
        write_frame(stream, &[PONG_PREFIX, version.as_bytes()].concat()).await?;
        return Ok(ControlRequest::Ping);
    }
    if request != STOP_REQUEST {
        return Err(ProofError::Malformed);
    }
    on_stop();
    write_frame(stream, STOP_ACK).await?;
    Ok(ControlRequest::Stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exigence : EX-019-11
    #[tokio::test(start_paused = true)]
    async fn the_wait_for_the_end_is_bounded() {
        let (mut client, server) = tokio::io::duplex(64);
        let waited = tokio::spawn(async move { wait_closed(&mut client).await });
        // The application keeps its end open: the client gives up after `STOP_TIMEOUT`.
        assert_eq!(waited.await.ok(), Some(Err(StopError::StillRunning)));
        drop(server);

        let (mut client, server) = tokio::io::duplex(64);
        drop(server);
        assert_eq!(wait_closed(&mut client).await, Ok(()));
        assert!(StopError::StillRunning.to_string().contains("20 s"));
    }
}
