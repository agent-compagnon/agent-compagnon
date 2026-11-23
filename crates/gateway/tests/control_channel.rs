//! The control channel of the pipe (spec 019): only a client proved in the control role can ask the application to
//! stop, with a fake protection per « user » and in-memory streams.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mcp_proxy_gateway::bridge::{
    ControlRequest, PeerProof, ProofError, ProtectFailure, Role, StopError, prove, request_ping, request_stop,
    serve_control,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use zeroize::Zeroizing;

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

/// The application's side, as `Gateway::serve_connection` does it: the control role reaches `serve_control`, any
/// other role does not. Keeps the stream until `release` fires, as the process keeps it until it ends.
async fn application(
    stream: &mut DuplexStream,
    stopped: Arc<AtomicBool>,
) -> Result<(Role, Option<ControlRequest>), ProofError> {
    let role = prove(user(1), Role::Server, stream).await?;
    if role != Role::Control {
        return Ok((role, None));
    }
    let request = serve_control(stream, "1.2.3", || stopped.store(true, Ordering::SeqCst)).await?;
    Ok((role, Some(request)))
}

// Exigence : EX-019-11
#[tokio::test]
async fn a_proved_control_client_stops_the_application_and_waits_for_its_end() {
    let (client_end, mut app_end) = duplex(4096);
    let stopped = Arc::new(AtomicBool::new(false));
    let client = tokio::spawn(request_stop(user(1), client_end));
    let served = application(&mut app_end, Arc::clone(&stopped)).await.unwrap();
    assert_eq!(served, (Role::Control, Some(ControlRequest::Stop)));
    assert!(stopped.load(Ordering::SeqCst));
    // The application still holds its end: the client cannot have returned.
    tokio::task::yield_now().await;
    assert!(!client.is_finished());
    drop(app_end);
    assert_eq!(client.await.unwrap(), Ok(()));
}

// Exigence : EX-019-11
#[tokio::test]
async fn a_bridge_never_reaches_the_control_channel() {
    let (mut bridge_end, mut app_end) = duplex(4096);
    let stopped = Arc::new(AtomicBool::new(false));
    let bridge = tokio::spawn(async move {
        prove(user(1), Role::Bridge, &mut bridge_end).await.unwrap();
        // What an LLM client would send through the bridge: relayed as MCP bytes, never read as a request.
        let request = b"mcp-proxy.control.v1\0stop";
        bridge_end
            .write_all(&u32::try_from(request.len()).unwrap().to_be_bytes())
            .await
            .unwrap();
        bridge_end.write_all(request).await.unwrap();
        bridge_end
    });
    let served = application(&mut app_end, Arc::clone(&stopped)).await.unwrap();
    assert_eq!(served, (Role::Bridge, None));
    assert!(!stopped.load(Ordering::SeqCst));
    drop(bridge.await.unwrap());
}

// Exigence : EX-019-11
#[tokio::test]
async fn a_control_client_of_another_user_is_refused() {
    let (client_end, mut app_end) = duplex(4096);
    let stopped = Arc::new(AtomicBool::new(false));
    let client = tokio::spawn(request_stop(user(2), client_end));
    let served = application(&mut app_end, Arc::clone(&stopped)).await;
    assert_eq!(served, Err(ProofError::Foreign));
    drop(app_end);
    assert!(matches!(client.await.unwrap(), Err(StopError::Proof(_))));
    assert!(!stopped.load(Ordering::SeqCst));
}

// Exigence : EX-019-11
#[tokio::test]
async fn anything_but_the_stop_request_is_refused() {
    for request in [
        &b"mcp-proxy.control.v1\0quit"[..],
        b"stop",
        br#"{"jsonrpc":"2.0","method":"exit"}"#,
    ] {
        let (mut client_end, mut app_end) = duplex(4096);
        let stopped = Arc::new(AtomicBool::new(false));
        let request = request.to_vec();
        let client = tokio::spawn(async move {
            prove(user(1), Role::Control, &mut client_end).await.unwrap();
            client_end
                .write_all(&u32::try_from(request.len()).unwrap().to_be_bytes())
                .await
                .unwrap();
            client_end.write_all(&request).await.unwrap();
            let mut answer = Vec::new();
            let _ = client_end.read_to_end(&mut answer).await;
            answer
        });
        let served = application(&mut app_end, Arc::clone(&stopped)).await;
        assert_eq!(served, Err(ProofError::Malformed));
        drop(app_end);
        assert!(client.await.unwrap().is_empty(), "no acknowledgement");
        assert!(!stopped.load(Ordering::SeqCst));
    }
}

// Exigence : EX-019-11
#[tokio::test]
async fn a_server_that_does_not_acknowledge_stops_nothing() {
    let (client_end, mut app_end) = duplex(4096);
    let client = tokio::spawn(request_stop(user(1), client_end));
    assert_eq!(prove(user(1), Role::Server, &mut app_end).await, Ok(Role::Control));
    let mut len = [0u8; 4];
    app_end.read_exact(&mut len).await.unwrap();
    let mut request = vec![0u8; u32::from_be_bytes(len) as usize];
    app_end.read_exact(&mut request).await.unwrap();
    app_end.write_all(&2u32.to_be_bytes()).await.unwrap();
    app_end.write_all(b"no").await.unwrap();
    assert_eq!(client.await.unwrap(), Err(StopError::Refused));
}

// Exigence : EX-024-10
#[tokio::test]
async fn a_ping_answers_the_version_and_stops_nothing() {
    let (client_end, mut app_end) = duplex(4096);
    let stopped = Arc::new(AtomicBool::new(false));
    let client = tokio::spawn(request_ping(user(1), client_end));
    let served = application(&mut app_end, Arc::clone(&stopped)).await.unwrap();
    assert_eq!(served, (Role::Control, Some(ControlRequest::Ping)));
    assert_eq!(client.await.unwrap(), Ok(Some("1.2.3".to_owned())));
    assert!(!stopped.load(Ordering::SeqCst));
}

// Exigence : EX-024-10
#[tokio::test]
async fn a_ping_of_another_user_is_refused() {
    let (client_end, mut app_end) = duplex(4096);
    let stopped = Arc::new(AtomicBool::new(false));
    let client = tokio::spawn(request_ping(user(2), client_end));
    assert_eq!(
        application(&mut app_end, Arc::clone(&stopped)).await,
        Err(ProofError::Foreign)
    );
    drop(app_end);
    assert!(client.await.unwrap().is_err());
    assert!(!stopped.load(Ordering::SeqCst));
}

// Exigence : EX-024-10
#[tokio::test]
async fn an_application_older_than_the_ping_proves_itself_then_closes() {
    let (client_end, mut app_end) = duplex(4096);
    let client = tokio::spawn(request_ping(user(1), client_end));
    assert_eq!(prove(user(1), Role::Server, &mut app_end).await, Ok(Role::Control));
    // What a version that knows only the stop does with any other request.
    let mut len = [0u8; 4];
    app_end.read_exact(&mut len).await.unwrap();
    let mut request = vec![0u8; u32::from_be_bytes(len) as usize];
    app_end.read_exact(&mut request).await.unwrap();
    assert_eq!(request, b"mcp-proxy.control.v1\0ping");
    drop(app_end);
    assert_eq!(client.await.unwrap(), Ok(None));
}

// Exigence : EX-024-10
#[tokio::test]
async fn a_malformed_pong_is_refused() {
    for answer in [
        &b"mcp-proxy.control.v1\0pong\0"[..],
        b"mcp-proxy.control.v1\0pong\x001.0 beta",
        b"pong",
    ] {
        let (client_end, mut app_end) = duplex(4096);
        let client = tokio::spawn(request_ping(user(1), client_end));
        assert_eq!(prove(user(1), Role::Server, &mut app_end).await, Ok(Role::Control));
        let mut len = [0u8; 4];
        app_end.read_exact(&mut len).await.unwrap();
        let mut request = vec![0u8; u32::from_be_bytes(len) as usize];
        app_end.read_exact(&mut request).await.unwrap();
        app_end
            .write_all(&u32::try_from(answer.len()).unwrap().to_be_bytes())
            .await
            .unwrap();
        app_end.write_all(answer).await.unwrap();
        assert_eq!(client.await.unwrap(), Err(ProofError::Malformed));
    }
}
