//! The `stdio` bridge and its named pipe (spec 011): the pipe's name, the mutual proof of identity both ends run
//! before any MCP byte, and the byte relay of the bridge. The application and the bridge only wire these to the
//! Windows pipe and to DPAPI.

mod proof;
mod relay;

use std::io;
use std::sync::Arc;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};

pub use self::proof::{MAX_FRAME, NONCE_LEN, PROOF_TIMEOUT, PeerProof, ProofError, ProtectFailure, Role, prove};
pub use self::relay::{RelayStats, relay};

/// Longest kept part of the domain and of the user name in the pipe's name.
const NAME_PART_MAX: usize = 64;

/// `\\.\pipe\mcp-proxy.<domain>.<user>`, both reduced to `[a-z0-9._-]` (anything else becomes `_`), or
/// `\\.\pipe\mcp-proxy.<user>` without a domain. `None` without a user name: there is no pipe to use then.
///
/// Two accounts whose names reduce to the same pipe can only fail to start or to connect: the first instance is
/// required, and the mutual proof fails between two users (I-D18.1).
pub fn pipe_name(domain: Option<&str>, user: Option<&str>) -> Option<String> {
    let user = user.map(reduce).filter(|u| !u.is_empty())?;
    let domain = domain.map(reduce).filter(|d| !d.is_empty());
    Some(match domain {
        Some(domain) => format!(r"\\.\pipe\mcp-proxy.{domain}.{user}"),
        None => format!(r"\\.\pipe\mcp-proxy.{user}"),
    })
}

fn reduce(part: &str) -> String {
    part.trim()
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => c,
            _ => '_',
        })
        .take(NAME_PART_MAX)
        .collect()
}

/// Why the bridge stopped. French, for the standard error that the LLM client shows.
#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("{0}")]
    Proof(#[from] ProofError),
    #[error("relais interrompu ({})", .0.kind())]
    Relay(io::Error),
}

impl BridgeError {
    /// The line written on standard error before a non-zero exit.
    pub fn message(&self) -> String {
        match self {
            Self::Proof(error) if error.is_foreign_peer() => format!(
                "MCP Proxy : le canal de MCP Proxy n'appartient pas à l'utilisateur courant ({error}). Rien n'a été \
                 relayé. Fermez le programme qui occupe le canal, puis relancez MCP Proxy."
            ),
            Self::Proof(error) => {
                format!("MCP Proxy : preuve d'identité du canal en échec ({error}). Rien n'a été relayé.")
            }
            Self::Relay(error) => format!("MCP Proxy : relais interrompu ({}).", error.kind()),
        }
    }
}

/// The bridge's side: proves itself as [`Role::Bridge`] on `pipe`, then relays `input` and `output`. Nothing of
/// `input` is read before the proof succeeds.
pub async fn prove_and_relay<I, O, P>(
    proof: Arc<dyn PeerProof>,
    input: I,
    output: O,
    mut pipe: P,
) -> Result<RelayStats, BridgeError>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    P: AsyncRead + AsyncWrite + Unpin,
{
    prove(proof, Role::Bridge, &mut pipe).await?;
    relay(input, output, pipe).await.map_err(BridgeError::Relay)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exigence : EX-011-14
    #[test]
    fn pipe_name_is_reduced_and_needs_a_user() {
        assert_eq!(
            pipe_name(Some("CORP"), Some("Jean.Dupont")).as_deref(),
            Some(r"\\.\pipe\mcp-proxy.corp.jean.dupont")
        );
        assert_eq!(
            pipe_name(Some("Désktop-01"), Some("José Ñ\\x")).as_deref(),
            Some(r"\\.\pipe\mcp-proxy.d_sktop-01.jos____x")
        );
        assert_eq!(pipe_name(None, Some("ana")).as_deref(), Some(r"\\.\pipe\mcp-proxy.ana"));
        assert_eq!(
            pipe_name(Some(" "), Some("ana")).as_deref(),
            Some(r"\\.\pipe\mcp-proxy.ana")
        );
        assert_eq!(pipe_name(Some("corp"), None), None);
        assert_eq!(pipe_name(Some("corp"), Some("  ")), None);
        let long = "a".repeat(300);
        let name = pipe_name(Some(&long), Some(&long)).unwrap();
        assert_eq!(name.len(), r"\\.\pipe\mcp-proxy..".len() + 2 * NAME_PART_MAX);
        let name = pipe_name(Some("x"), Some("../..\\pipe\\autre")).unwrap();
        assert!(!name["\\\\.\\pipe\\".len()..].contains('\\'), "{name}");
    }
}
