//! Mutual proof of the named pipe (spec 011), with a fake protection per « user » and in-memory streams.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use mcp_proxy_gateway::bridge::{MAX_FRAME, PeerProof, ProofError, ProtectFailure, Role, prove, prove_and_relay};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, duplex};
use zeroize::Zeroizing;

const MCP_REQUEST: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
const MCP_ANSWER: &[u8] = br#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;

/// Seals for one « user »: only the same user opens it.
struct FakeUser(u8);

impl PeerProof for FakeUser {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure> {
        let mut sealed = vec![b'U', self.0];
        sealed.extend(data.iter().map(|b| b ^ 0x5a));
        Ok(sealed)
    }

    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure> {
        match data {
            [b'U', user, rest @ ..] if *user == self.0 => Ok(Zeroizing::new(rest.iter().map(|b| b ^ 0x5a).collect())),
            _ => Err(ProtectFailure(-1)),
        }
    }
}

fn user(id: u8) -> Arc<dyn PeerProof> {
    Arc::new(FakeUser(id))
}

/// The application's side: proof, then one answer to whatever the client sent.
async fn application(proof: Arc<dyn PeerProof>, mut stream: DuplexStream) -> Result<Vec<u8>, ProofError> {
    prove(proof, Role::Server, &mut stream).await?;
    let mut request = vec![0u8; MCP_REQUEST.len()];
    stream.read_exact(&mut request).await.unwrap();
    stream.write_all(MCP_ANSWER).await.unwrap();
    stream.shutdown().await.unwrap();
    Ok(request)
}

/// Runs the bridge against `pipe` with an LLM client that sends `MCP_REQUEST`; returns its result and what the
/// client got back.
async fn bridge(proof: Arc<dyn PeerProof>, pipe: DuplexStream) -> (Result<(), ProofError>, Vec<u8>) {
    let (client_in, mut client_writer) = duplex(4096);
    let (mut client_reader, client_out) = duplex(4096);
    client_writer.write_all(MCP_REQUEST).await.unwrap();
    drop(client_writer);
    let result = prove_and_relay(proof, client_in, client_out, pipe).await;
    let mut received = Vec::new();
    client_reader.read_to_end(&mut received).await.unwrap();
    let result = result.map(|_| ()).map_err(|e| match e {
        mcp_proxy_gateway::bridge::BridgeError::Proof(p) => p,
        other => panic!("unexpected relay error: {other}"),
    });
    (result, received)
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &[u8]) {
    w.write_all(&u32::try_from(frame.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    w.write_all(frame).await.unwrap();
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Vec<u8> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.unwrap();
    let mut frame = vec![0u8; u32::from_be_bytes(len) as usize];
    r.read_exact(&mut frame).await.unwrap();
    frame
}

/// Everything the honest side sent to the adversary, until it closed the stream.
async fn drain(mut stream: DuplexStream) -> Vec<u8> {
    let mut all = Vec::new();
    let _ = stream.read_to_end(&mut all).await;
    all
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// Exigence : EX-011-09
#[tokio::test]
async fn same_user_proves_then_relays_both_ways() {
    let (pipe, app_end) = duplex(64);
    let app = tokio::spawn(application(user(1), app_end));
    let (result, received) = bridge(user(1), pipe).await;
    assert_eq!(result, Ok(()));
    assert_eq!(app.await.unwrap().unwrap(), MCP_REQUEST);
    assert_eq!(received, MCP_ANSWER);
}

// Exigence : EX-011-09
#[tokio::test]
async fn another_user_is_refused_on_both_sides_without_any_mcp_byte() {
    let (pipe, app_end) = duplex(64);
    let app = tokio::spawn(application(user(1), app_end));
    let (result, received) = bridge(user(2), pipe).await;
    let app = app.await.unwrap();
    assert!(
        matches!(result, Err(ProofError::Foreign | ProofError::Closed)),
        "{result:?}"
    );
    assert!(matches!(app, Err(ProofError::Foreign | ProofError::Closed)), "{app:?}");
    assert!(result.is_err() && app.is_err());
    assert!(received.is_empty(), "nothing reaches the LLM client");
}

// Exigence : EX-011-09
#[tokio::test]
async fn a_foreign_pipe_gets_nothing_from_the_bridge() {
    // A pipe of another user: it cannot open the bridge's challenge, so it sends a challenge of its own and waits.
    let (pipe, mut squatter) = duplex(4096);
    let other = FakeUser(2);
    let squatter_task = tokio::spawn(async move {
        let challenge = other
            .protect(b"mcp-proxy.pipe.v1\0serveur\0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap();
        write_frame(&mut squatter, &challenge).await;
        drain(squatter).await
    });
    let (result, received) = bridge(user(1), pipe).await;
    assert_eq!(result, Err(ProofError::Foreign));
    let seen = squatter_task.await.unwrap();
    assert!(!contains(&seen, MCP_REQUEST), "no MCP byte reached the foreign pipe");
    assert!(received.is_empty());
}

// Exigence : EX-011-09
#[tokio::test]
async fn a_wrong_answer_is_refused() {
    // A client that opens nothing but answers with random bytes of the right length.
    let (client, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let (mut reader, mut writer) = tokio::io::split(client);
    let me = FakeUser(1);
    let challenge = me
        .protect(b"mcp-proxy.pipe.v1\0pont\0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .unwrap();
    write_frame(&mut writer, &challenge).await;
    let _server_challenge = read_frame(&mut reader).await;
    write_frame(&mut writer, &[0u8; 32]).await;
    writer.write_all(MCP_REQUEST).await.unwrap();
    assert_eq!(app.await.unwrap(), Err(ProofError::WrongAnswer));
}

// Exigence : EX-011-09
#[tokio::test]
async fn a_reflected_challenge_is_refused() {
    // A client that cannot open the server's challenge sends it back as its own.
    let (client, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let (mut reader, mut writer) = tokio::io::split(client);
    let server_challenge = read_frame(&mut reader).await;
    write_frame(&mut writer, &server_challenge).await;
    writer.write_all(MCP_REQUEST).await.unwrap();
    assert_eq!(app.await.unwrap(), Err(ProofError::Reflected));
    let mut rest = Vec::new();
    let _ = reader.read_to_end(&mut rest).await;
    assert!(!contains(&rest, MCP_ANSWER), "no MCP byte from the application");

    // The same against the bridge: a pipe that sends the bridge's challenge back.
    let (pipe, squatter) = duplex(4096);
    let squatter_task = tokio::spawn(async move {
        let (mut reader, mut writer) = tokio::io::split(squatter);
        let challenge = read_frame(&mut reader).await;
        write_frame(&mut writer, &challenge).await;
        let mut all = Vec::new();
        let _ = reader.read_to_end(&mut all).await;
        all
    });
    let (result, received) = bridge(user(1), pipe).await;
    assert_eq!(result, Err(ProofError::Reflected));
    assert!(!contains(&squatter_task.await.unwrap(), MCP_REQUEST));
    assert!(received.is_empty());
}

// Exigence : EX-011-09
#[tokio::test]
async fn too_long_or_malformed_messages_are_refused() {
    let (client, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let (_reader, mut writer) = tokio::io::split(client);
    writer
        .write_all(&u32::try_from(MAX_FRAME + 1).unwrap().to_be_bytes())
        .await
        .unwrap();
    writer.write_all(MCP_REQUEST).await.unwrap();
    assert_eq!(app.await.unwrap(), Err(ProofError::Malformed));

    // Well framed, sealed by the same user, but not a challenge.
    let (client, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let (_reader, mut writer) = tokio::io::split(client);
    write_frame(&mut writer, &FakeUser(1).protect(MCP_REQUEST).unwrap()).await;
    assert_eq!(app.await.unwrap(), Err(ProofError::Malformed));

    // An empty frame.
    let (client, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let (_reader, mut writer) = tokio::io::split(client);
    writer.write_all(&0u32.to_be_bytes()).await.unwrap();
    assert_eq!(app.await.unwrap(), Err(ProofError::Malformed));

    // A client that speaks MCP at once: its first bytes read as an absurd length.
    let (pipe, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let (mut reader, mut writer) = tokio::io::split(pipe);
    writer.write_all(MCP_REQUEST).await.unwrap();
    assert_eq!(app.await.unwrap(), Err(ProofError::Malformed));
    let mut rest = Vec::new();
    let _ = reader.read_to_end(&mut rest).await;
    assert!(!contains(&rest, MCP_ANSWER));
}

// Exigence : EX-011-09
#[tokio::test(start_paused = true)]
async fn a_silent_peer_times_out_on_both_sides() {
    let (client, app_end) = duplex(4096);
    let app = tokio::spawn(application(user(1), app_end));
    let started = tokio::time::Instant::now();
    assert_eq!(app.await.unwrap(), Err(ProofError::Timeout));
    assert!(started.elapsed() >= Duration::from_secs(5));
    drop(client);

    let (pipe, silent) = duplex(4096);
    let (result, received) = bridge(user(1), pipe).await;
    assert_eq!(result, Err(ProofError::Timeout));
    assert!(received.is_empty());
    let seen = drain(silent).await;
    assert!(!contains(&seen, MCP_REQUEST));
}

#[test]
fn messages_name_the_cause_without_any_nonce() {
    use mcp_proxy_gateway::bridge::BridgeError;
    let foreign = BridgeError::Proof(ProofError::Foreign).message();
    assert!(
        foreign.contains("n'appartient pas à l'utilisateur courant"),
        "{foreign}"
    );
    let timeout = BridgeError::Proof(ProofError::Timeout).message();
    assert!(timeout.contains("délai"), "{timeout}");
}
