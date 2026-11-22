//! Mutual proof of identity on the named pipe, before any MCP byte (I-D18.3).
//!
//! Each side draws a 32-byte nonce, seals `context ‖ role ‖ nonce` with user-scoped DPAPI and sends it as its
//! challenge. Each side opens the challenge it receives, checks that it carries the other side's role, and sends the
//! nonce back. Each side then compares the answer with its own nonce in constant time. Only a process of the same
//! Windows user can open a challenge, so a pipe or a client of another user fails here.
//!
//! The role in the challenge stops a reflection: a challenge sent back as is carries the receiver's own role and is
//! refused. Frames are prefixed by their length (big-endian `u32`), 4 KiB at most; the whole exchange is bounded by
//! [`PROOF_TIMEOUT`].
//!
//! Known limit: nothing binds the proof to the pipe instance. A relay between a bridge and the real application
//! would pass; it would need a pipe of the same name created by another user while the application runs, which
//! `first_pipe_instance` and the default security descriptor (no `FILE_CREATE_PIPE_INSTANCE` for other users)
//! prevent.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

/// Bound on the whole exchange, both directions.
pub const PROOF_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest frame accepted, length prefix excluded.
pub const MAX_FRAME: usize = 4096;
pub const NONCE_LEN: usize = 32;

const CONTEXT: &[u8] = b"mcp-proxy.pipe.v1";
const SEPARATOR: u8 = 0;

/// Which end of the pipe this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The application, which created the pipe.
    Server,
    /// `mcp-proxy.exe stdio`, launched by the LLM client.
    Bridge,
}

impl Role {
    fn tag(self) -> &'static [u8] {
        match self {
            Self::Server => b"serveur",
            Self::Bridge => b"pont",
        }
    }

    fn peer(self) -> Self {
        match self {
            Self::Server => Self::Bridge,
            Self::Bridge => Self::Server,
        }
    }

    fn of_tag(tag: &[u8]) -> Option<Self> {
        [Self::Server, Self::Bridge].into_iter().find(|r| r.tag() == tag)
    }
}

/// A protection failure: a system code at most, never any data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectFailure(pub i32);

/// Seals bytes so that only the same user can open them: user-scoped DPAPI in the application
/// (`mcp_proxy_core::platform::windows`), a fake in tests. Both calls may block: they run on the blocking pool.
pub trait PeerProof: Send + Sync {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure>;

    /// Fails, or gives bytes that are not a challenge, for data sealed by another user.
    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure>;
}

/// Why the proof failed. French; never carries a nonce nor a challenge.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProofError {
    #[error("preuve d'identité du canal non terminée dans le délai ({} s)", PROOF_TIMEOUT.as_secs())]
    Timeout,
    #[error("canal fermé par l'autre côté pendant la preuve d'identité")]
    Closed,
    #[error("échange en échec pendant la preuve d'identité ({0})")]
    Io(io::ErrorKind),
    #[error("message de preuve d'identité mal formé")]
    Malformed,
    #[error("défi de l'autre côté impossible à ouvrir : il n'appartient pas à l'utilisateur courant")]
    Foreign,
    #[error("défi refusé : il porte le rôle de ce côté (défi renvoyé)")]
    Reflected,
    #[error("réponse fausse au défi")]
    WrongAnswer,
    #[error("protection DPAPI indisponible (code système {:#010x})", .0.0)]
    Protection(ProtectFailure),
    #[error("générateur d'aléa indisponible")]
    Random,
}

impl ProofError {
    /// The other side is not the same Windows user, or not a side of MCP Proxy.
    pub fn is_foreign_peer(&self) -> bool {
        matches!(
            self,
            Self::Foreign | Self::Reflected | Self::WrongAnswer | Self::Malformed
        )
    }
}

impl From<io::Error> for ProofError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset => Self::Closed,
            kind => Self::Io(kind),
        }
    }
}

/// Runs the proof as `role` on `stream`, bounded by [`PROOF_TIMEOUT`]. On error, the caller must close the stream
/// without relaying anything.
pub async fn prove<S>(proof: Arc<dyn PeerProof>, role: Role, stream: &mut S) -> Result<(), ProofError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(PROOF_TIMEOUT, exchange(proof, role, stream))
        .await
        .unwrap_or(Err(ProofError::Timeout))
}

async fn exchange<S>(proof: Arc<dyn PeerProof>, role: Role, stream: &mut S) -> Result<(), ProofError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut nonce = Zeroizing::new([0u8; NONCE_LEN]);
    getrandom::fill(nonce.as_mut_slice()).map_err(|_| ProofError::Random)?;
    let plain = challenge_plaintext(role, &nonce);
    let challenge = blocking(&proof, move |p| p.protect(&plain))
        .await?
        .map_err(ProofError::Protection)?;
    if challenge.is_empty() || challenge.len() > MAX_FRAME {
        return Err(ProofError::Protection(ProtectFailure(0)));
    }

    let (mut reader, mut writer) = tokio::io::split(stream);
    // Writing and reading at once: neither side waits on a full buffer.
    let (sent, theirs) = tokio::join!(write_frame(&mut writer, &challenge), read_frame(&mut reader));
    sent?;
    let theirs = theirs?;
    let opened = blocking(&proof, move |p| p.unprotect(&theirs))
        .await?
        .map_err(|_| ProofError::Foreign)?;
    let peer_nonce = peer_nonce(&opened, role)?;

    let (sent, answer) = tokio::join!(write_frame(&mut writer, peer_nonce.as_slice()), read_frame(&mut reader));
    sent?;
    let answer = Zeroizing::new(answer?);
    if !constant_time_eq(&answer, nonce.as_slice()) {
        return Err(ProofError::WrongAnswer);
    }
    Ok(())
}

async fn blocking<R: Send + 'static>(
    proof: &Arc<dyn PeerProof>,
    f: impl FnOnce(&dyn PeerProof) -> R + Send + 'static,
) -> Result<R, ProofError> {
    let proof = Arc::clone(proof);
    tokio::task::spawn_blocking(move || f(proof.as_ref()))
        .await
        .map_err(|_| ProofError::Protection(ProtectFailure(0)))
}

fn challenge_plaintext(role: Role, nonce: &[u8; NONCE_LEN]) -> Zeroizing<Vec<u8>> {
    let mut plain = Zeroizing::new(Vec::with_capacity(CONTEXT.len() + 2 + 8 + NONCE_LEN));
    plain.extend_from_slice(CONTEXT);
    plain.push(SEPARATOR);
    plain.extend_from_slice(role.tag());
    plain.push(SEPARATOR);
    plain.extend_from_slice(nonce);
    plain
}

/// The nonce of a challenge opened by `own` role: it must carry the other role.
fn peer_nonce(opened: &[u8], own: Role) -> Result<Zeroizing<Vec<u8>>, ProofError> {
    let rest = opened
        .strip_prefix(CONTEXT)
        .and_then(|r| r.strip_prefix(&[SEPARATOR]))
        .ok_or(ProofError::Malformed)?;
    let split = rest.iter().position(|b| *b == SEPARATOR).ok_or(ProofError::Malformed)?;
    let (tag, nonce) = (&rest[..split], &rest[split + 1..]);
    match Role::of_tag(tag) {
        Some(role) if role == own => Err(ProofError::Reflected),
        Some(role) if role == own.peer() && nonce.len() == NONCE_LEN => Ok(Zeroizing::new(nonce.to_vec())),
        _ => Err(ProofError::Malformed),
    }
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &[u8]) -> Result<(), ProofError> {
    let len = u32::try_from(frame.len()).map_err(|_| ProofError::Malformed)?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(frame).await?;
    writer.flush().await?;
    Ok(())
}

/// A frame of 1 to [`MAX_FRAME`] bytes; a larger announced length is refused before anything else is read.
async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, ProofError> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await?;
    let len = usize::try_from(u32::from_be_bytes(len)).map_err(|_| ProofError::Malformed)?;
    if len == 0 || len > MAX_FRAME {
        return Err(ProofError::Malformed);
    }
    let mut frame = vec![0u8; len];
    reader.read_exact(&mut frame).await?;
    Ok(frame)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_round_trip_and_roles() {
        let nonce = [7u8; NONCE_LEN];
        let plain = challenge_plaintext(Role::Bridge, &nonce);
        assert_eq!(peer_nonce(&plain, Role::Server).unwrap().as_slice(), &nonce);
        assert_eq!(peer_nonce(&plain, Role::Bridge), Err(ProofError::Reflected));
        assert_eq!(peer_nonce(b"autre chose", Role::Server), Err(ProofError::Malformed));
        let short = &plain[..plain.len() - 1];
        assert_eq!(peer_nonce(short, Role::Server), Err(ProofError::Malformed));
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
