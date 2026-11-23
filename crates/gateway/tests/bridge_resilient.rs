//! The resilient bridge (spec 026), with a fake protection per « user », in-memory streams, a connector the test
//! feeds with instances of the application, and a fake launcher. Time is tokio's paused clock: no test waits for
//! real, none depends on a timing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mcp_proxy_gateway::bridge::{
    AppLauncher, BridgeStop, ConnectFailure, Connector, MISSING_CONFIRM, PeerProof, ProofError, ProtectFailure,
    RECONNECT_WINDOW, RELAUNCH_GRACE, RESTARTING, RelaunchTarget, RelayDeps, ResilientRelay, Role, prove,
};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf, duplex,
};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use zeroize::Zeroizing;

/// Seals for one « user »: only the same user opens it.
struct FakeUser(u8);

impl PeerProof for FakeUser {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectFailure> {
        Ok([&[b'U', self.0][..], data].concat())
    }

    fn unprotect(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectFailure> {
        match data {
            [b'U', user, rest @ ..] if *user == self.0 => Ok(Zeroizing::new(rest.to_vec())),
            _ => Err(ProtectFailure(-1)),
        }
    }
}

fn user(id: u8) -> Arc<dyn PeerProof> {
    Arc::new(FakeUser(id))
}

/// The pipes of the instances of the application, in the order the test gives them; « absent » otherwise.
#[derive(Clone, Default)]
struct Pipes(Arc<Mutex<Pipeline>>);

#[derive(Default)]
struct Pipeline {
    ready: VecDeque<DuplexStream>,
    /// Set by a test to make every attempt find a pipe of the application.
    serving: Option<tokio::sync::mpsc::UnboundedSender<DuplexStream>>,
    attempts: usize,
}

impl Pipes {
    /// The next connection reaches a new instance, whose end the test gets.
    fn instance(&self, size: usize) -> DuplexStream {
        let (bridge, app) = duplex(size);
        self.0.lock().unwrap().ready.push_back(bridge);
        app
    }

    fn attempts(&self) -> usize {
        self.0.lock().unwrap().attempts
    }
}

impl Connector for Pipes {
    type Pipe = DuplexStream;

    async fn connect(&mut self) -> Result<DuplexStream, ConnectFailure> {
        let mut pipeline = self.0.lock().unwrap();
        pipeline.attempts += 1;
        if let Some(pipe) = pipeline.ready.pop_front() {
            return Ok(pipe);
        }
        if let Some(serving) = &pipeline.serving {
            let (bridge, app) = duplex(1 << 16);
            if serving.send(app).is_ok() {
                return Ok(bridge);
            }
        }
        Err(ConnectFailure::Absent)
    }
}

/// What a launch would start, and every launch.
struct Launcher {
    target: Mutex<RelaunchTarget>,
    launched: Mutex<Vec<(PathBuf, Instant)>>,
    /// Called at each launch: the « system » that starts the application.
    on_launch: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Launcher {
    fn new(target: RelaunchTarget) -> Arc<Self> {
        Arc::new(Self {
            target: Mutex::new(target),
            launched: Mutex::new(Vec::new()),
            on_launch: Mutex::new(None),
        })
    }

    fn installed() -> Arc<Self> {
        Self::new(RelaunchTarget::Exe(installed_exe()))
    }

    fn set(&self, target: RelaunchTarget) {
        *self.target.lock().unwrap() = target;
    }

    fn launches(&self) -> Vec<(PathBuf, Instant)> {
        self.launched.lock().unwrap().clone()
    }
}

impl AppLauncher for Launcher {
    fn target(&self) -> RelaunchTarget {
        self.target.lock().unwrap().clone()
    }

    fn launch(&self, exe: &Path) -> Result<(), String> {
        self.launched.lock().unwrap().push((exe.to_path_buf(), Instant::now()));
        if let Some(on_launch) = self.on_launch.lock().unwrap().as_ref() {
            on_launch();
        }
        Ok(())
    }
}

fn installed_exe() -> PathBuf {
    PathBuf::from(r"C:\Users\Ana\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe")
}

/// The LLM client's side of the bridge: what it writes, and the lines it reads.
struct Client {
    writer: DuplexStream,
    reader: Lines<BufReader<DuplexStream>>,
}

impl Client {
    async fn send(&mut self, message: Value) {
        self.send_raw(&format!("{message}\n")).await;
    }

    async fn send_raw(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = self
            .reader
            .next_line()
            .await
            .unwrap()
            .expect("the client's stream is open");
        serde_json::from_str(&line).unwrap()
    }

    /// End of the client's writing: its input closes.
    async fn close(&mut self) {
        self.writer.shutdown().await.unwrap();
    }

    /// `None` at the end of the stream.
    async fn next(&mut self) -> Option<Value> {
        self.reader
            .next_line()
            .await
            .unwrap()
            .map(|line| serde_json::from_str(&line).unwrap())
    }
}

struct Bridge {
    client: Client,
    pipes: Pipes,
    launcher: Arc<Launcher>,
    diagnostics: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<Result<(), BridgeStop>>,
}

impl Bridge {
    fn said(&self) -> Vec<String> {
        self.diagnostics.lock().unwrap().clone()
    }
}

fn start_bridge(launcher: Arc<Launcher>, pipes: Pipes, size: usize) -> Bridge {
    let (input, writer) = duplex(size);
    let (reader, output) = duplex(size);
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let said = Arc::clone(&diagnostics);
    let relay = ResilientRelay::new(RelayDeps {
        proof: user(1),
        input,
        output,
        connector: pipes.clone(),
        launcher: launcher.clone(),
        diagnostics: Arc::new(move |line: &str| said.lock().unwrap().push(line.to_owned())),
    });
    Bridge {
        client: Client {
            writer,
            reader: BufReader::new(reader).lines(),
        },
        pipes,
        launcher,
        diagnostics,
        task: tokio::spawn(relay.run()),
    }
}

/// A bridge whose first connection reaches an instance, returned proved.
async fn connected_bridge() -> (Bridge, App) {
    let pipes = Pipes::default();
    let app = pipes.instance(1 << 16);
    let bridge = start_bridge(Launcher::installed(), pipes, 1 << 16);
    (bridge, App::accept(app).await)
}

/// One instance of the application: proved as the server, then lines read and written by the test.
struct App {
    reader: Lines<BufReader<ReadHalf<DuplexStream>>>,
    writer: WriteHalf<DuplexStream>,
}

impl App {
    async fn accept(mut stream: DuplexStream) -> Self {
        prove(user(1), Role::Server, &mut stream).await.unwrap();
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: BufReader::new(reader).lines(),
            writer,
        }
    }

    async fn recv(&mut self) -> Value {
        let line = self
            .reader
            .next_line()
            .await
            .unwrap()
            .expect("the bridge's pipe is open");
        serde_json::from_str(&line).unwrap()
    }

    async fn send(&mut self, message: Value) {
        self.send_raw(&format!("{message}\n")).await;
    }

    async fn send_raw(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    /// Everything the bridge still sends, until it closes the pipe.
    async fn rest(mut self) -> Vec<Value> {
        let mut all = Vec::new();
        while let Ok(Some(line)) = self.reader.next_line().await {
            all.push(serde_json::from_str(&line).unwrap());
        }
        all
    }
}

fn call(id: Value, tool: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": {} } })
}

fn answer(id: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": "ok" }] } })
}

fn is_restarting_error(message: &Value, id: &Value) -> bool {
    message["id"] == *id && message["error"]["message"] == RESTARTING && message.get("result").is_none()
}

const LEGACY_VERSION: &str = "2025-06-18";

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": { "protocolVersion": LEGACY_VERSION, "capabilities": {}, "clientInfo": { "name": "client", "version": "1" } },
    })
}

fn initialized() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
}

fn init_answer(version: &str, server: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 0,
        "result": {
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": true } },
            "serverInfo": { "name": "mcp-proxy", "version": server },
        },
    })
}

/// The old protocol's handshake through the bridge, with the first instance.
async fn handshake(bridge: &mut Bridge, app: &mut App) {
    bridge.client.send(initialize()).await;
    assert_eq!(app.recv().await, initialize());
    app.send(init_answer(LEGACY_VERSION, "0.1.0")).await;
    assert_eq!(bridge.client.recv().await, init_answer(LEGACY_VERSION, "0.1.0"));
    bridge.client.send(initialized()).await;
    assert_eq!(app.recv().await, initialized());
}

/// A call served from end to end.
async fn served(bridge: &mut Bridge, app: &mut App, id: Value) {
    bridge.client.send(call(id.clone(), "lire")).await;
    assert_eq!(app.recv().await, call(id.clone(), "lire"));
    app.send(answer(id.clone())).await;
    assert_eq!(bridge.client.recv().await, answer(id));
}

// Exigence : EX-026-01
// Exigence : EX-026-03
#[tokio::test(start_paused = true)]
async fn a_cut_then_a_new_instance_without_any_replay() {
    let (mut bridge, mut app) = connected_bridge().await;
    // 2026-07-28: no `initialize`, every request carries its context.
    served(&mut bridge, &mut app, json!(1)).await;

    let next = bridge.pipes.instance(1 << 16);
    drop(app);
    let mut app = App::accept(next).await;
    // The first message of the new instance is the client's next request: nothing replayed.
    served(&mut bridge, &mut app, json!(2)).await;
    served(&mut bridge, &mut app, json!(3)).await;
    assert!(!bridge.task.is_finished(), "the client never saw its stream close");
    assert!(bridge.launcher.launches().is_empty());

    bridge.client.close().await;
    assert!(app.rest().await.is_empty());
    assert_eq!(bridge.client.reader.next_line().await.unwrap(), None);
    assert!(bridge.task.await.unwrap().is_ok());
}

// Exigence : EX-026-02
// Exigence : EX-026-12
#[tokio::test(start_paused = true)]
async fn the_old_protocol_replays_its_session_and_absorbs_the_answer() {
    let (mut bridge, mut app) = connected_bridge().await;
    handshake(&mut bridge, &mut app).await;
    served(&mut bridge, &mut app, json!(1)).await;

    let next = bridge.pipes.instance(1 << 16);
    drop(app);
    let mut app = App::accept(next).await;
    // The new instance gets the client's `initialize`, as the client wrote it, then `initialized`.
    assert_eq!(app.recv().await, initialize());
    // Another server version, same session: absorbed.
    app.send(init_answer(LEGACY_VERSION, "0.1.1")).await;
    assert_eq!(app.recv().await, initialized());
    // The client only learns that the tools may have changed: the server announced `listChanged`.
    assert_eq!(
        bridge.client.recv().await,
        json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })
    );
    served(&mut bridge, &mut app, json!(2)).await;

    // A second cut replays the same session again.
    let next = bridge.pipes.instance(1 << 16);
    drop(app);
    let mut app = App::accept(next).await;
    assert_eq!(app.recv().await, initialize());
    app.send(init_answer(LEGACY_VERSION, "0.1.2")).await;
    assert_eq!(app.recv().await, initialized());
    assert_eq!(bridge.client.recv().await["method"], "notifications/tools/list_changed");
    served(&mut bridge, &mut app, json!(3)).await;
    assert!(!bridge.task.is_finished());
}

// Exigence : EX-026-12
#[tokio::test(start_paused = true)]
async fn a_new_instance_that_answers_another_session_stops_the_bridge() {
    for (case, other) in [
        ("protocol version", init_answer("2025-03-26", "0.2.0")),
        (
            "capabilities",
            json!({ "jsonrpc": "2.0", "id": 0, "result": { "protocolVersion": LEGACY_VERSION, "capabilities": { "tools": {} } } }),
        ),
        (
            "error",
            json!({ "jsonrpc": "2.0", "id": 0, "error": { "code": -32602, "message": "non" } }),
        ),
    ] {
        let (mut bridge, mut app) = connected_bridge().await;
        handshake(&mut bridge, &mut app).await;
        let next = bridge.pipes.instance(1 << 16);
        drop(app);
        let mut app = App::accept(next).await;
        assert_eq!(app.recv().await, initialize());
        // Something the server sends before its answer never reaches the client.
        app.send(json!({ "jsonrpc": "2.0", "method": "notifications/message", "params": { "level": "info" } }))
            .await;
        app.send(other).await;
        let stop = bridge.task.await.unwrap();
        assert!(matches!(stop, Err(BridgeStop::Incompatible)), "{case}: {stop:?}");
        assert!(
            stop.unwrap_err().message().contains("Redémarrez le serveur MCP"),
            "{case}"
        );
        assert!(
            app.rest().await.is_empty(),
            "{case}: neither `initialized` nor anything else"
        );
        assert_eq!(
            bridge.client.next().await,
            None,
            "{case}: the duplicate answer was absorbed"
        );
    }
}

// Exigence : EX-026-04
#[tokio::test(start_paused = true)]
async fn each_request_in_flight_gets_exactly_one_error() {
    let (mut bridge, mut app) = connected_bridge().await;
    for id in [json!(1), json!("deux"), json!(3), json!(4)] {
        bridge.client.send(call(id.clone(), "lent")).await;
        assert_eq!(app.recv().await, call(id, "lent"));
    }
    // A notification has no id: nothing to answer.
    bridge
        .client
        .send(json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": 9 } }))
        .await;
    // The client gave up on 4: no answer is owed.
    bridge
        .client
        .send(json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 4 } }))
        .await;
    assert_eq!(app.recv().await["method"], "notifications/progress");
    assert_eq!(app.recv().await["method"], "notifications/cancelled");
    // 1 is answered just before the cut; 3 is half written when it comes.
    app.send(answer(json!(1))).await;
    assert_eq!(bridge.client.recv().await, answer(json!(1)));
    app.send_raw(r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"te"#)
        .await;
    let next = bridge.pipes.instance(1 << 16);
    drop(app);

    let mut errors = vec![bridge.client.recv().await, bridge.client.recv().await];
    errors.sort_by_key(|e| e["id"].to_string());
    assert!(is_restarting_error(&errors[0], &json!("deux")), "{errors:?}");
    assert!(is_restarting_error(&errors[1], &json!(3)), "{errors:?}");
    let mut app = App::accept(next).await;
    served(&mut bridge, &mut app, json!(5)).await;
    // Nothing came between the two errors and the answer to 5: no second error, no half answer.
    bridge.client.close().await;
    assert!(app.rest().await.is_empty());
    assert_eq!(bridge.client.next().await, None);
}

// Exigence : EX-026-10
#[tokio::test(start_paused = true)]
async fn requests_of_the_server_waiting_for_the_client_are_cancelled() {
    let (mut bridge, mut app) = connected_bridge().await;
    let elicitation =
        json!({ "jsonrpc": "2.0", "id": 0, "method": "elicitation/create", "params": { "message": "Confirmer ?" } });
    app.send(elicitation.clone()).await;
    assert_eq!(bridge.client.recv().await, elicitation);
    let next = bridge.pipes.instance(1 << 16);
    drop(app);

    // The client is told the request is cancelled; no error is owed to it for a request it did not send.
    let cancel = bridge.client.recv().await;
    assert_eq!(cancel["method"], "notifications/cancelled");
    assert_eq!(cancel["params"]["requestId"], 0);
    let mut app = App::accept(next).await;

    // The new instance asks something under the same id before the client answered the old request: refused to
    // the server, never shown to the client, whose late answer could be taken for an answer to it.
    let reused =
        json!({ "jsonrpc": "2.0", "id": 0, "method": "elicitation/create", "params": { "message": "Autre ?" } });
    app.send(reused).await;
    let refused = app.recv().await;
    assert_eq!(refused["id"], 0);
    assert!(refused["error"]["message"].is_string(), "{refused}");
    // The next id is the server's own.
    let fresh =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "elicitation/create", "params": { "message": "Celle-ci ?" } });
    app.send(fresh.clone()).await;
    assert_eq!(bridge.client.recv().await, fresh);
    bridge
        .client
        .send(json!({ "jsonrpc": "2.0", "id": 1, "result": { "action": "decline" } }))
        .await;
    assert_eq!(app.recv().await["id"], 1);

    // A late answer to a request of the previous instance goes nowhere.
    let next = bridge.pipes.instance(1 << 16);
    app.send(json!({ "jsonrpc": "2.0", "id": 7, "method": "elicitation/create", "params": {} }))
        .await;
    assert_eq!(bridge.client.recv().await["id"], 7);
    drop(app);
    assert_eq!(bridge.client.recv().await["params"]["requestId"], 7);
    let mut app = App::accept(next).await;
    bridge
        .client
        .send(json!({ "jsonrpc": "2.0", "id": 7, "result": { "action": "accept" } }))
        .await;
    served(&mut bridge, &mut app, json!(8)).await;
    bridge.client.close().await;
    assert!(app.rest().await.is_empty(), "the late answer was not relayed");
}

// Exigence : EX-026-05
// Exigence : EX-026-13
#[tokio::test(start_paused = true)]
async fn without_the_application_for_60_s_the_bridge_stops() {
    let (mut bridge, app) = connected_bridge().await;
    bridge.launcher.set(RelaunchTarget::StoppedOnPurpose(installed_exe()));
    drop(app);
    let cut = Instant::now();
    // A request meanwhile is answered at once: the error, not a wait.
    bridge.client.send(call(json!(10), "lire")).await;
    assert!(is_restarting_error(&bridge.client.recv().await, &json!(10)));
    assert!(cut.elapsed() < Duration::from_secs(1));

    let stop = bridge.task.await.unwrap();
    assert!(matches!(stop, Err(BridgeStop::NotBack { proof: None })), "{stop:?}");
    assert!(cut.elapsed() >= RECONNECT_WINDOW);
    assert!(cut.elapsed() < RECONNECT_WINDOW + Duration::from_secs(1));
    let message = stop.unwrap_err().message();
    assert!(message.contains("n'est pas revenue dans les 60 s"), "{message}");
    assert_eq!(bridge.client.next().await, None, "the stream ends after the bridge");
    assert!(
        bridge.launcher.launches().is_empty(),
        "stopped on purpose: not launched again"
    );
    assert!(bridge.pipes.attempts() > 100, "attempts all along");
}

// Exigence : EX-026-13
#[tokio::test(start_paused = true)]
async fn the_end_of_the_input_ends_the_bridge() {
    // During a cut: at once, without waiting for the application.
    let (mut bridge, app) = connected_bridge().await;
    drop(app);
    let cut = Instant::now();
    bridge.client.close().await;
    assert!(bridge.task.await.unwrap().is_ok());
    assert!(cut.elapsed() < Duration::from_secs(1));

    // While connected: the pipe's writing side closes, the answers still owed are relayed.
    let (mut bridge, mut app) = connected_bridge().await;
    bridge.client.send(call(json!(1), "lire")).await;
    assert_eq!(app.recv().await, call(json!(1), "lire"));
    bridge.client.close().await;
    assert_eq!(app.reader.next_line().await.unwrap(), None);
    app.send(answer(json!(1))).await;
    drop(app);
    assert_eq!(bridge.client.recv().await, answer(json!(1)));
    assert_eq!(bridge.client.next().await, None);
    assert!(bridge.task.await.unwrap().is_ok());
}

// Exigence : EX-026-13
#[tokio::test(start_paused = true)]
async fn initialized_sent_during_the_cut_is_replayed() {
    let (mut bridge, mut app) = connected_bridge().await;
    bridge.client.send(initialize()).await;
    assert_eq!(app.recv().await, initialize());
    app.send(init_answer(LEGACY_VERSION, "0.1.0")).await;
    assert_eq!(bridge.client.recv().await, init_answer(LEGACY_VERSION, "0.1.0"));
    // The application goes before the client confirmed.
    drop(app);
    bridge.client.send(initialized()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut app = App::accept(bridge.pipes.instance(1 << 16)).await;
    assert_eq!(app.recv().await, initialize());
    app.send(init_answer(LEGACY_VERSION, "0.1.0")).await;
    assert_eq!(app.recv().await, initialized());
    assert_eq!(bridge.client.recv().await["method"], "notifications/tools/list_changed");
    served(&mut bridge, &mut app, json!(1)).await;
}

// Exigence : EX-026-06
#[tokio::test(start_paused = true)]
async fn large_and_fragmented_messages_are_split_right_across_a_cut() {
    let pipes = Pipes::default();
    // Small buffers: every message crosses them in many pieces.
    let app_end = pipes.instance(64);
    let mut bridge = start_bridge(Launcher::installed(), pipes, 64);
    let mut app = App::accept(app_end).await;

    let big = "é".repeat(3 * 1024 * 1024);
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "x", "arguments": { "texte": big } } });
    let response = json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [{ "type": "text", "text": big }] } });
    let sent = {
        let request = request.clone();
        let mut writer = bridge.client.writer;
        tokio::spawn(async move {
            writer.write_all(format!("{request}\n").as_bytes()).await.unwrap();
            // Two messages in one write, then one byte at a time.
            let two = format!("{}\n{}\n", call(json!(2), "a"), call(json!(3), "b"));
            writer.write_all(two.as_bytes()).await.unwrap();
            for byte in format!("{}\n", call(json!(4), "c")).bytes() {
                writer.write_all(&[byte]).await.unwrap();
            }
            writer
        })
    };
    assert_eq!(app.recv().await, request);
    for (id, tool) in [(2, "a"), (3, "b"), (4, "c")] {
        assert_eq!(app.recv().await, call(json!(id), tool));
    }
    bridge.client.writer = sent.await.unwrap();
    let answered = {
        let mut app = app;
        let response = response.clone();
        tokio::spawn(async move {
            app.send(response).await;
            app.send_raw(&format!("{}\n{}\n", answer(json!(2)), answer(json!(3))))
                .await;
            app
        })
    };
    assert_eq!(bridge.client.recv().await, response);
    assert_eq!(bridge.client.recv().await, answer(json!(2)));
    assert_eq!(bridge.client.recv().await, answer(json!(3)));
    let mut app = answered.await.unwrap();

    // Half of the answer to 4 is in the pipe, and half of the client's next request in its input, at the cut.
    app.send_raw(&format!("{}", answer(json!(4)))[..20]).await;
    let half = format!("{}\n", call(json!(5), "d"));
    bridge.client.send_raw(&half[..15]).await;
    let next = bridge.pipes.instance(64);
    drop(app);
    assert!(
        is_restarting_error(&bridge.client.recv().await, &json!(4)),
        "the half answer is not relayed"
    );
    let mut app = App::accept(next).await;
    bridge.client.send_raw(&half[15..]).await;
    // The new instance gets the request once and whole.
    assert_eq!(app.recv().await, call(json!(5), "d"));
    app.send(answer(json!(5))).await;
    assert_eq!(bridge.client.recv().await, answer(json!(5)));
    bridge.client.close().await;
    assert!(app.rest().await.is_empty());
}

/// A pipe of another user: it sends a challenge the bridge cannot open, then records everything it receives.
async fn squatter(mut stream: DuplexStream) -> Vec<u8> {
    let challenge = FakeUser(2)
        .protect(b"mcp-proxy.pipe.v1\0serveur\0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .unwrap();
    stream
        .write_all(&u32::try_from(challenge.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&challenge).await.unwrap();
    let mut all = Vec::new();
    let _ = stream.read_to_end(&mut all).await;
    all
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// Exigence : EX-026-09
#[tokio::test(start_paused = true)]
async fn a_pipe_that_fails_the_proof_gets_no_mcp_byte() {
    let (mut bridge, mut app) = connected_bridge().await;
    handshake(&mut bridge, &mut app).await;
    // While the application is away, another program holds the pipe's name, twice, then a silent one.
    let first = tokio::spawn(squatter(bridge.pipes.instance(4096)));
    let second = tokio::spawn(squatter(bridge.pipes.instance(4096)));
    let (silent_end, mut silent) = {
        let end = bridge.pipes.instance(4096);
        (end, Vec::new())
    };
    let silent_task = tokio::spawn(async move {
        let mut end = silent_end;
        let _ = end.read_to_end(&mut silent).await;
        silent
    });
    drop(app);
    // A request of the client in the meantime: answered by the bridge, sent to nobody.
    bridge.client.send(call(json!("pendant"), "ecrire")).await;
    assert!(is_restarting_error(&bridge.client.recv().await, &json!("pendant")));
    // The silent one is left once the proof's delay is over.
    let silent = silent_task.await.unwrap();
    let real = bridge.pipes.instance(1 << 16);
    let mut app = App::accept(real).await;
    assert_eq!(app.recv().await, initialize(), "the real application gets the replay");
    app.send(init_answer(LEGACY_VERSION, "0.1.0")).await;
    assert_eq!(app.recv().await, initialized());
    assert_eq!(bridge.client.recv().await["method"], "notifications/tools/list_changed");
    served(&mut bridge, &mut app, json!(2)).await;

    for seen in [first.await.unwrap(), second.await.unwrap(), silent] {
        for mcp in [&b"initialize"[..], b"jsonrpc", b"pendant", b"ecrire"] {
            assert!(!contains(&seen, mcp), "{:?}", String::from_utf8_lossy(&seen));
        }
    }
    let said = bridge.said().join("\n");
    assert!(said.contains("preuve d'identité du canal en échec"), "{said}");
}

// Exigence : EX-026-09
#[tokio::test(start_paused = true)]
async fn a_pipe_that_keeps_failing_the_proof_ends_the_wait() {
    let (bridge, app) = connected_bridge().await;
    bridge.launcher.set(RelaunchTarget::StoppedOnPurpose(installed_exe()));
    let (sender, mut squatters) = tokio::sync::mpsc::unbounded_channel();
    bridge.pipes.0.lock().unwrap().serving = Some(sender);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Some(stream) = squatters.recv().await {
            let recorded = Arc::clone(&recorded);
            tokio::spawn(async move {
                let bytes = squatter(stream).await;
                recorded.lock().unwrap().extend(bytes);
            });
        }
    });
    drop(app);
    let stop = bridge.task.await.unwrap();
    assert!(
        matches!(
            stop,
            Err(BridgeStop::NotBack {
                proof: Some(ProofError::Foreign)
            })
        ),
        "{stop:?}"
    );
    assert!(stop.unwrap_err().message().contains("rien ne lui a été envoyé"));
    assert!(!contains(&seen.lock().unwrap(), b"jsonrpc"));
}

// Exigence : EX-026-11
#[tokio::test(start_paused = true)]
async fn after_a_crash_the_installed_exe_is_launched_once_after_the_grace_period() {
    let (mut bridge, app) = connected_bridge().await;
    let pipes = bridge.pipes.clone();
    let started = Arc::new(Mutex::new(None));
    let instance = Arc::clone(&started);
    *bridge.launcher.on_launch.lock().unwrap() = Some(Box::new(move || {
        *instance.lock().unwrap() = Some(pipes.instance(1 << 16));
    }));
    drop(app);
    let cut = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if started.lock().unwrap().is_some() {
            break;
        }
    }
    let launches = bridge.launcher.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].0, installed_exe());
    let waited = launches[0].1.duration_since(cut);
    assert!(
        waited >= RELAUNCH_GRACE && waited < RELAUNCH_GRACE + Duration::from_secs(1),
        "{waited:?}"
    );
    let end = started.lock().unwrap().take().unwrap();
    let mut app = App::accept(end).await;
    served(&mut bridge, &mut app, json!(1)).await;

    // A new crash within minutes: no second launch, the bridge waits, then stops.
    drop(app);
    let said = Arc::clone(&bridge.diagnostics);
    let stop = bridge.task.await.unwrap();
    assert!(matches!(stop, Err(BridgeStop::NotBack { .. })), "{stop:?}");
    assert_eq!(
        bridge.launcher.launches().len(),
        1,
        "an application that fails again is not started again"
    );
    let said = said.lock().unwrap().join("\n");
    assert!(said.contains("relance de MCP Proxy"), "{said}");
}

// Exigence : EX-026-11
#[tokio::test(start_paused = true)]
async fn an_uninstalled_exe_stops_the_bridge_without_any_launch() {
    let (bridge, app) = connected_bridge().await;
    // The uninstallation set the exe aside before stopping the application.
    bridge.launcher.set(RelaunchTarget::Missing);
    drop(app);
    let cut = Instant::now();
    let stop = bridge.task.await.unwrap();
    assert!(matches!(stop, Err(BridgeStop::Gone)), "{stop:?}");
    assert!(cut.elapsed() >= MISSING_CONFIRM && cut.elapsed() < MISSING_CONFIRM + Duration::from_secs(1));
    assert!(bridge.launcher.launches().is_empty());
    assert!(stop.unwrap_err().message().contains("désinstallé"));
}

// Exigence : EX-026-11
#[tokio::test(start_paused = true)]
async fn an_exe_absent_for_an_instant_stops_nothing() {
    let (mut bridge, app) = connected_bridge().await;
    bridge.launcher.set(RelaunchTarget::StoppedOnPurpose(installed_exe()));
    drop(app);
    // An update renames the exe, then puts the new one in place.
    tokio::time::sleep(Duration::from_secs(1)).await;
    bridge.launcher.set(RelaunchTarget::Missing);
    tokio::time::sleep(MISSING_CONFIRM / 4).await;
    bridge.launcher.set(RelaunchTarget::StoppedOnPurpose(installed_exe()));
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!bridge.task.is_finished());
    let mut app = App::accept(bridge.pipes.instance(1 << 16)).await;
    served(&mut bridge, &mut app, json!(1)).await;
    assert!(
        bridge.launcher.launches().is_empty(),
        "the application stopped on purpose: not launched"
    );
}

// Exigence : EX-026-11
#[tokio::test(start_paused = true)]
async fn the_first_connection_launches_the_application_as_before() {
    let pipes = Pipes::default();
    let launcher = Launcher::new(RelaunchTarget::StoppedOnPurpose(installed_exe()));
    let started = Arc::new(Mutex::new(None));
    let instance = Arc::clone(&started);
    let system = pipes.clone();
    *launcher.on_launch.lock().unwrap() = Some(Box::new(move || {
        *instance.lock().unwrap() = Some(system.instance(1 << 16));
    }));
    // A client started after « Quitter » starts the application, at once.
    let mut bridge = start_bridge(launcher, pipes, 1 << 16);
    loop {
        tokio::task::yield_now().await;
        if started.lock().unwrap().is_some() {
            break;
        }
    }
    let end = started.lock().unwrap().take().unwrap();
    let mut app = App::accept(end).await;
    served(&mut bridge, &mut app, json!(1)).await;
    assert_eq!(bridge.launcher.launches().len(), 1);

    // Nothing to start, nothing waited for.
    let bridge = start_bridge(Launcher::new(RelaunchTarget::Missing), Pipes::default(), 64);
    assert!(matches!(bridge.task.await.unwrap(), Err(BridgeStop::NotStartable)));
    // No pipe within 15 s.
    let bridge = start_bridge(Launcher::installed(), Pipes::default(), 64);
    let stop = bridge.task.await.unwrap();
    assert!(matches!(stop, Err(BridgeStop::NoPipe)), "{stop:?}");
    assert_eq!(bridge.launcher.launches().len(), 1);
    // A failed first proof stops at once, as before.
    let pipes = Pipes::default();
    let squatted = tokio::spawn(squatter(pipes.instance(4096)));
    let bridge = start_bridge(Launcher::installed(), pipes, 64);
    let stop = bridge.task.await.unwrap();
    assert!(
        matches!(stop, Err(BridgeStop::FirstProof(ProofError::Foreign))),
        "{stop:?}"
    );
    assert!(
        stop.unwrap_err()
            .message()
            .contains("n'appartient pas à l'utilisateur courant")
    );
    assert!(!contains(&squatted.await.unwrap(), b"jsonrpc"));
}

// Exigence : EX-026-11
#[tokio::test(start_paused = true)]
async fn two_bridges_launch_the_application_at_most_once_each() {
    // The single instance: whoever launches first starts the application; a second launch hands over and ends.
    let running = Arc::new(Mutex::new(false));
    let mut bridges = Vec::new();
    let mut apps = Vec::new();
    for _ in 0..2 {
        let (bridge, app) = connected_bridge().await;
        bridges.push(bridge);
        apps.push(app);
    }
    let mut connections = Vec::new();
    for bridge in &bridges {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<DuplexStream>();
        connections.push(receiver);
        let running = Arc::clone(&running);
        let pipes = bridge.pipes.clone();
        *bridge.launcher.on_launch.lock().unwrap() = Some(Box::new(move || {
            // The first launch starts the application; the second one hands over to it.
            *running.lock().unwrap() = true;
            pipes.0.lock().unwrap().serving = Some(sender.clone());
        }));
    }
    // Both lose the application at once (a crash).
    drop(apps);
    for (bridge, connections) in bridges.iter_mut().zip(&mut connections) {
        let mut app = App::accept(connections.recv().await.unwrap()).await;
        served(bridge, &mut app, json!(1)).await;
        std::mem::forget(app);
    }
    tokio::time::sleep(Duration::from_secs(600)).await;
    for bridge in &bridges {
        assert_eq!(bridge.launcher.launches().len(), 1, "one launch per bridge, no loop");
        assert!(!bridge.task.is_finished());
    }
    assert!(*running.lock().unwrap());
}

// Exigence : EX-026-14
#[tokio::test(start_paused = true)]
async fn the_diagnostics_hold_no_content_of_any_message() {
    const SENTINEL: &str = "SENTINELLE-contenu-9b2f";
    let (mut bridge, mut app) = connected_bridge().await;
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "x", "arguments": { "q": SENTINEL } } });
    bridge.client.send(request.clone()).await;
    assert_eq!(app.recv().await, request);
    app.send(json!({ "jsonrpc": "2.0", "method": "notifications/message", "params": { "data": SENTINEL } }))
        .await;
    assert_eq!(bridge.client.recv().await["params"]["data"], SENTINEL);
    drop(app);
    let error = bridge.client.recv().await;
    assert!(is_restarting_error(&error, &json!(1)));
    assert!(!error.to_string().contains(SENTINEL));
    bridge.client.send(call(json!(SENTINEL), "y")).await;
    let error = bridge.client.recv().await;
    assert!(
        is_restarting_error(&error, &json!(SENTINEL)),
        "only the id is given back"
    );
    bridge.client.close().await;
    assert!(bridge.task.await.unwrap().is_ok());
    let said = bridge.diagnostics.lock().unwrap().join("\n");
    assert!(
        said.contains("connexion à l'appli perdue (1 requête(s) en cours interrompue(s))"),
        "{said}"
    );
    assert!(!said.contains(SENTINEL), "{said}");
    assert!(!said.contains("tools/call"), "{said}");
}

#[allow(dead_code)]
fn assert_send<T: Send>(_: &T) {}

#[allow(dead_code)]
fn relay_is_send(relay: ResilientRelay<DuplexStream, DuplexStream, Pipes>) {
    let future = relay.run();
    assert_send(&future);
}
