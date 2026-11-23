//! The resilient bridge (spec 026): `mcp-proxy.exe stdio` survives a restart of the application.
//!
//! While connected, both directions are relayed message by message, each on its own (neither waits on the other:
//! a full pipe never blocks the answers). The bridge only reads ids and methods ([`super::messages`]): it follows
//! the requests of the client still without answer, the requests of the server still waiting for the client, and
//! the `initialize` of the old protocol.
//!
//! When the pipe closes, every request of the client still without answer gets one error, and every request of the
//! server waiting for the client is cancelled towards it. The bridge then waits for the application, 60 s at most,
//! answering at once the requests that arrive meanwhile. Each new connection runs the mutual proof before any MCP
//! byte; for the old protocol, the `initialize` and `initialized` of the client are replayed, and the answer, which
//! the client already had, is absorbed, once checked to be the same session (protocol version, capabilities).
//!
//! The bridge starts the application only if it stopped without saying so (a crash), after a grace period that
//! leaves the restart of an update, or an installation, to the one who stopped it; at most once per cut and once in
//! [`RELAUNCH_COOLDOWN`]; never an exe that is gone, and a bridge whose exe is gone stops.

use std::collections::BTreeSet;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use super::BridgeError;
use super::messages::{
    Handshake, INITIALIZE, INITIALIZED, Id, Kind, Line, LineReader, cancelled, inspect, refused_to_server, restarting,
    tools_list_changed,
};
use super::proof::{PROOF_TIMEOUT, PeerProof, ProofError, Role, prove};
use super::relaunch::RelaunchTarget;

/// How long the bridge waits for the pipe at its start, after launching the application.
pub const FIRST_WAIT: Duration = Duration::from_secs(15);
/// How long the bridge waits for the application after a cut.
pub const RECONNECT_WINDOW: Duration = Duration::from_secs(60);
/// Between two attempts.
pub const RETRY: Duration = Duration::from_millis(100);
/// After a cut, the time left to whoever stopped the application to start it again, before the bridge does.
pub const RELAUNCH_GRACE: Duration = Duration::from_secs(10);
/// Least time between two launches by the same bridge: an application that fails again soon is not started again.
pub const RELAUNCH_COOLDOWN: Duration = Duration::from_secs(300);
/// How long the exe must stay absent before the bridge stops: an update renames it for an instant.
pub const MISSING_CONFIRM: Duration = Duration::from_secs(2);
/// Bound on the answer of a new instance to the replayed `initialize`.
pub const REPLAY_TIMEOUT: Duration = PROOF_TIMEOUT;

/// Why the pipe could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFailure {
    /// No pipe of this name: the application is not running.
    Absent,
    /// Every instance of the pipe is taken for now.
    Busy,
    Other(io::ErrorKind),
}

/// Opens the application's pipe: the named pipe in the application, in-memory streams in tests.
pub trait Connector: Send {
    type Pipe: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn connect(&mut self) -> impl Future<Output = Result<Self::Pipe, ConnectFailure>> + Send;
}

/// Starts the application: a detached process in the application, a fake in tests.
pub trait AppLauncher: Send + Sync {
    /// What a launch would start now; read again at each attempt (the exe may have gone, or come back).
    fn target(&self) -> RelaunchTarget;

    /// French message on failure, without any path.
    fn launch(&self, exe: &Path) -> Result<(), String>;
}

/// A line for the standard error, which the LLM client shows: French, without any content of message.
pub type Diagnostics = Arc<dyn Fn(&str) + Send + Sync>;

/// Why the bridge stopped. French, for the standard error.
#[derive(Debug, Error)]
pub enum BridgeStop {
    #[error("{}", BridgeError::Proof(.0.clone()).message())]
    FirstProof(ProofError),
    #[error("MCP Proxy : canal de MCP Proxy inaccessible ({0}).")]
    Unreachable(io::ErrorKind),
    #[error(
        "MCP Proxy : l'appli MCP Proxy n'a pas ouvert son canal dans les {} s. Lancez MCP Proxy depuis le menu \
         Démarrer, puis réessayez.",
        FIRST_WAIT.as_secs()
    )]
    NoPipe,
    #[error("{0}")]
    Launch(String),
    #[error("MCP Proxy : exe de MCP Proxy introuvable ou illisible, appli non lancée.")]
    NotStartable,
    #[error(
        "MCP Proxy : l'appli s'est arrêtée et son exe n'est plus là (MCP Proxy désinstallé ou déplacé) : le pont \
         s'arrête."
    )]
    Gone,
    #[error(
        "MCP Proxy : l'appli MCP Proxy n'est pas revenue dans les {} s{} : le pont s'arrête. Relancez MCP Proxy, puis \
         redémarrez le serveur MCP dans le client LLM.",
        RECONNECT_WINDOW.as_secs(),
        .proof.as_ref().map(|e| format!(
            " (le canal est tenu par un programme qui n'a pas fait la preuve d'identité de MCP Proxy : {e} ; rien ne \
             lui a été envoyé)"
        )).unwrap_or_default()
    )]
    NotBack { proof: Option<ProofError> },
    #[error(
        "MCP Proxy : la version de MCP Proxy relancée ne reprend pas la session du client (version du protocole ou \
         capacités différentes) : le pont s'arrête. Redémarrez le serveur MCP dans le client LLM."
    )]
    Incompatible,
    #[error("MCP Proxy : relais interrompu ({0}).")]
    Output(io::ErrorKind),
}

impl BridgeStop {
    /// The line written on standard error before a non-zero exit.
    pub fn message(&self) -> String {
        self.to_string()
    }
}

pub struct RelayDeps<I, O, C> {
    pub proof: Arc<dyn PeerProof>,
    /// What the LLM client writes (standard input).
    pub input: I,
    /// What it reads (standard output).
    pub output: O,
    pub connector: C,
    pub launcher: Arc<dyn AppLauncher>,
    pub diagnostics: Diagnostics,
}

pub struct ResilientRelay<I, O, C: Connector> {
    proof: Arc<dyn PeerProof>,
    input: LineReader<I>,
    output: O,
    connector: C,
    launcher: Arc<dyn AppLauncher>,
    diagnostics: Diagnostics,
    tracker: Mutex<Tracker>,
    last_launch: Option<Instant>,
}

struct Connected<P> {
    reader: LineReader<ReadHalf<P>>,
    writer: WriteHalf<P>,
}

impl<P: AsyncRead + AsyncWrite> Connected<P> {
    fn new(pipe: P) -> Self {
        let (reader, writer) = tokio::io::split(pipe);
        Self {
            reader: LineReader::new(reader),
            writer,
        }
    }
}

/// How a connection ended.
enum Outcome {
    /// The application closed the pipe.
    Lost,
    /// The client closed its input, then the application its pipe.
    InputEnded,
    OutputFailed(io::ErrorKind),
}

/// How the wait for the application ended without a connection.
enum Exit {
    /// The client closed its input.
    Done,
    Stop(BridgeStop),
}

/// Why an attempt at a new connection failed.
enum Failure {
    Connect(ConnectFailure),
    Proof(ProofError),
    /// Closed, or silent, during the replay.
    Lost,
    Incompatible,
}

impl<I, O, C> ResilientRelay<I, O, C>
where
    I: AsyncRead + Unpin + Send,
    O: AsyncWrite + Unpin + Send,
    C: Connector,
{
    pub fn new(deps: RelayDeps<I, O, C>) -> Self {
        Self {
            proof: deps.proof,
            input: LineReader::new(deps.input),
            output: deps.output,
            connector: deps.connector,
            launcher: deps.launcher,
            diagnostics: deps.diagnostics,
            tracker: Mutex::new(Tracker::default()),
            last_launch: None,
        }
    }

    /// Relays until the client closes its input (`Ok`), or the bridge has to stop. Nothing of the input is read
    /// before the first proof succeeds.
    pub async fn run(mut self) -> Result<(), BridgeStop> {
        let result = self.relay().await;
        let _ = self.output.shutdown().await;
        result
    }

    async fn relay(&mut self) -> Result<(), BridgeStop> {
        let mut connected = self.first_connection().await?;
        loop {
            let outcome = self.serve(connected).await;
            let cut = lock(&self.tracker).cut();
            let interrupted = cut.iter().filter(|line| line.is_error).count();
            for line in &cut {
                write_out(&mut self.output, &line.text)
                    .await
                    .map_err(|e| BridgeStop::Output(e.kind()))?;
            }
            match outcome {
                Outcome::InputEnded => return Ok(()),
                Outcome::OutputFailed(kind) => return Err(BridgeStop::Output(kind)),
                Outcome::Lost => {}
            }
            self.say(&format!(
                "MCP Proxy : connexion à l'appli perdue ({interrupted} requête(s) en cours interrompue(s)) ; attente de \
                 son retour, {} s au plus.",
                RECONNECT_WINDOW.as_secs()
            ));
            connected = match self.reconnect().await {
                Ok(connected) => connected,
                Err(Exit::Done) => return Ok(()),
                Err(Exit::Stop(stop)) => return Err(stop),
            };
            let announce = lock(&self.tracker)
                .session()
                .is_some_and(|s| s.handshake.tools_list_changed());
            if announce {
                // The new instance may expose other tools: the client asks for the list again.
                write_out(&mut self.output, &tools_list_changed())
                    .await
                    .map_err(|e| BridgeStop::Output(e.kind()))?;
            }
            self.say("MCP Proxy : reconnecté à l'appli.");
        }
    }

    fn say(&self, line: &str) {
        (self.diagnostics)(line);
    }

    /// As before spec 026: the application is launched once if the pipe does not exist, then awaited
    /// [`FIRST_WAIT`]; a failed proof stops the bridge at once.
    async fn first_connection(&mut self) -> Result<Connected<C::Pipe>, BridgeStop> {
        let deadline = Instant::now() + FIRST_WAIT;
        let mut launched = false;
        loop {
            match self.connector.connect().await {
                Ok(mut pipe) => {
                    prove(Arc::clone(&self.proof), Role::Bridge, &mut pipe)
                        .await
                        .map_err(BridgeStop::FirstProof)?;
                    return Ok(Connected::new(pipe));
                }
                Err(ConnectFailure::Absent) if !launched => {
                    launched = true;
                    let exe = match self.launcher.target() {
                        RelaunchTarget::Exe(exe) | RelaunchTarget::StoppedOnPurpose(exe) => exe,
                        RelaunchTarget::Missing | RelaunchTarget::Unknown => return Err(BridgeStop::NotStartable),
                    };
                    self.launcher.launch(&exe).map_err(BridgeStop::Launch)?;
                    self.last_launch = Some(Instant::now());
                }
                Err(ConnectFailure::Absent | ConnectFailure::Busy) => {}
                Err(ConnectFailure::Other(kind)) => return Err(BridgeStop::Unreachable(kind)),
            }
            if Instant::now() >= deadline {
                return Err(BridgeStop::NoPipe);
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// Relays both directions until the pipe closes.
    async fn serve(&mut self, connected: Connected<C::Pipe>) -> Outcome {
        let Connected { mut reader, mut writer } = connected;
        // Answers of the bridge to the server, written by the upstream side, which owns the pipe's writer.
        let (inject, mut injected) = mpsc::unbounded_channel::<Vec<u8>>();
        // Ends the downstream side between two messages, never while it writes to the client.
        let stop = Notify::new();
        let tracker = &self.tracker;
        let input = &mut self.input;
        let output = &mut self.output;
        let diagnostics = &self.diagnostics;

        let upstream = async {
            loop {
                let line = tokio::select! {
                    biased;
                    Some(line) = injected.recv() => line,
                    read = input.next() => match read {
                        Ok(Line::Message(line)) => {
                            let forward = lock(tracker).client_sent(&line);
                            if !forward {
                                continue;
                            }
                            line
                        }
                        Ok(Line::Oversized) => {
                            oversized(diagnostics);
                            continue;
                        }
                        Ok(Line::End) | Err(_) => {
                            let _ = writer.shutdown().await;
                            return Upstream::InputEnded;
                        }
                    },
                };
                if writer.write_all(&line).await.is_err() || writer.flush().await.is_err() {
                    return Upstream::PipeFailed;
                }
            }
        };
        let downstream = async {
            loop {
                let read = tokio::select! {
                    read = reader.next() => read,
                    () = stop.notified() => return Downstream::Stopped,
                };
                match read {
                    Ok(Line::Message(line)) => {
                        let verdict = lock(tracker).server_sent(&line);
                        match verdict {
                            FromServer::Relay => {
                                if let Err(error) = write_out(output, &line).await {
                                    return Downstream::OutputFailed(error.kind());
                                }
                            }
                            FromServer::Answer(answer) => {
                                let _ = inject.send(answer);
                            }
                        }
                    }
                    Ok(Line::Oversized) => oversized(diagnostics),
                    Ok(Line::End) | Err(_) => return Downstream::PipeClosed,
                }
            }
        };
        tokio::pin!(upstream, downstream);

        let mut input_ended = false;
        let mut upstream_done = false;
        loop {
            tokio::select! {
                up = &mut upstream, if !upstream_done => {
                    upstream_done = true;
                    match up {
                        Upstream::InputEnded => input_ended = true,
                        Upstream::PipeFailed => stop.notify_one(),
                    }
                }
                down = &mut downstream => {
                    return match down {
                        Downstream::OutputFailed(kind) => Outcome::OutputFailed(kind),
                        Downstream::PipeClosed | Downstream::Stopped if input_ended => Outcome::InputEnded,
                        Downstream::PipeClosed | Downstream::Stopped => Outcome::Lost,
                    };
                }
            }
        }
    }

    /// Waits for the application after a cut: [`RECONNECT_WINDOW`] at most.
    async fn reconnect(&mut self) -> Result<Connected<C::Pipe>, Exit> {
        let cut = Instant::now();
        let deadline = cut + RECONNECT_WINDOW;
        let mut launched = false;
        let mut missing_since: Option<Instant> = None;
        let mut told: BTreeSet<String> = BTreeSet::new();
        loop {
            let (absent, last_proof) = match self.attempt().await {
                Ok(connected) => return Ok(connected),
                Err(Failure::Incompatible) => return Err(Exit::Stop(BridgeStop::Incompatible)),
                Err(Failure::Proof(error)) => {
                    let line = format!(
                        "MCP Proxy : preuve d'identité du canal en échec ({error}) : rien n'est envoyé, nouvel essai."
                    );
                    if told.insert(line.clone()) {
                        self.say(&line);
                    }
                    (false, Some(error))
                }
                Err(Failure::Connect(failure)) => (failure == ConnectFailure::Absent, None),
                Err(Failure::Lost) => (false, None),
            };
            let now = Instant::now();
            match self.launcher.target() {
                RelaunchTarget::Missing => {
                    let since = *missing_since.get_or_insert(now);
                    if now.duration_since(since) >= MISSING_CONFIRM {
                        return Err(Exit::Stop(BridgeStop::Gone));
                    }
                }
                RelaunchTarget::Exe(exe) => {
                    missing_since = None;
                    let cooled = self
                        .last_launch
                        .is_none_or(|at| now.duration_since(at) >= RELAUNCH_COOLDOWN);
                    if absent && !launched && cooled && now.duration_since(cut) >= RELAUNCH_GRACE {
                        launched = true;
                        self.last_launch = Some(now);
                        self.say("MCP Proxy : l'appli ne revient pas d'elle-même (plantage ?) : relance de MCP Proxy.");
                        if let Err(message) = self.launcher.launch(&exe) {
                            self.say(&message);
                        }
                    }
                }
                RelaunchTarget::StoppedOnPurpose(_) => {
                    missing_since = None;
                    let line = "MCP Proxy : l'appli a été arrêtée volontairement : le pont ne la relance pas.";
                    if told.insert(line.to_owned()) {
                        self.say(line);
                    }
                }
                RelaunchTarget::Unknown => missing_since = None,
            }
            if now >= deadline {
                return Err(Exit::Stop(BridgeStop::NotBack { proof: last_proof }));
            }
            self.serve_input_while_away(RETRY).await?;
        }
    }

    /// Connection, proof, and for the old protocol the replay of the session.
    async fn attempt(&mut self) -> Result<Connected<C::Pipe>, Failure> {
        let mut pipe = self.connector.connect().await.map_err(Failure::Connect)?;
        prove(Arc::clone(&self.proof), Role::Bridge, &mut pipe)
            .await
            .map_err(Failure::Proof)?;
        let mut connected = Connected::new(pipe);
        let session = lock(&self.tracker).session().cloned();
        if let Some(session) = session {
            replay(&mut connected, &session).await?;
        }
        Ok(connected)
    }

    /// For `duration`, every request of the client gets the error at once; `initialized` is kept for the replay.
    async fn serve_input_while_away(&mut self, duration: Duration) -> Result<(), Exit> {
        let sleep = tokio::time::sleep(duration);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                biased;
                read = self.input.next() => match read {
                    Ok(Line::Message(line)) => {
                        let answers = lock(&self.tracker).while_away(&line);
                        for answer in answers {
                            write_out(&mut self.output, &answer)
                                .await
                                .map_err(|e| Exit::Stop(BridgeStop::Output(e.kind())))?;
                        }
                    }
                    Ok(Line::Oversized) => oversized(&self.diagnostics),
                    Ok(Line::End) | Err(_) => return Err(Exit::Done),
                },
                () = &mut sleep => return Ok(()),
            }
        }
    }
}

/// `initialize` again, then its answer, which must be the session the client knows, then `initialized`. Nothing the
/// new instance sends before its answer reaches the client.
async fn replay<P: AsyncRead + AsyncWrite>(connected: &mut Connected<P>, session: &Session) -> Result<(), Failure> {
    let Connected { reader, writer } = connected;
    if writer.write_all(&session.request).await.is_err() || writer.flush().await.is_err() {
        return Err(Failure::Lost);
    }
    let answer = tokio::time::timeout(REPLAY_TIMEOUT, async {
        loop {
            match reader.next().await {
                Ok(Line::Message(line)) => {
                    if inspect(&line) == [Kind::Response { id: session.id.clone() }] {
                        return Ok(Handshake::of_answer(&line));
                    }
                }
                Ok(Line::Oversized) => {}
                Ok(Line::End) | Err(_) => return Err(Failure::Lost),
            }
        }
    })
    .await
    .map_err(|_| Failure::Lost)??;
    if answer.as_ref() != Some(&session.handshake) {
        return Err(Failure::Incompatible);
    }
    if let Some(initialized) = &session.initialized
        && (writer.write_all(initialized).await.is_err() || writer.flush().await.is_err())
    {
        return Err(Failure::Lost);
    }
    Ok(())
}

enum Upstream {
    InputEnded,
    PipeFailed,
}

enum Downstream {
    PipeClosed,
    Stopped,
    OutputFailed(io::ErrorKind),
}

fn oversized(diagnostics: &Diagnostics) {
    diagnostics(&format!(
        "MCP Proxy : message de plus de {} Mio ignoré.",
        super::messages::MAX_MESSAGE / (1024 * 1024)
    ));
}

async fn write_out<O: AsyncWrite + Unpin>(output: &mut O, line: &[u8]) -> io::Result<()> {
    output.write_all(line).await?;
    output.flush().await
}

fn lock(tracker: &Mutex<Tracker>) -> MutexGuard<'_, Tracker> {
    tracker.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The session of an old-protocol client, as the first instance answered it.
#[derive(Debug, Clone)]
struct Session {
    id: Id,
    request: Vec<u8>,
    handshake: Handshake,
    initialized: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
enum Init {
    #[default]
    Unknown,
    /// The client sent `initialize`; no answer yet.
    Asked {
        id: Id,
        request: Vec<u8>,
    },
    Known(Session),
}

/// What the bridge follows of the messages, across connections.
#[derive(Debug, Default)]
struct Tracker {
    /// Requests of the client without answer.
    client: BTreeSet<Id>,
    /// Requests of the server waiting for the client.
    server: BTreeSet<Id>,
    /// Requests of a previous instance: the client's late answer goes nowhere, and a new instance may not reuse
    /// the id until then.
    stale: BTreeSet<Id>,
    init: Init,
}

enum FromServer {
    Relay,
    /// Not relayed: this answer goes back to the server.
    Answer(Vec<u8>),
}

/// A line the bridge writes to the client at a cut.
struct CutLine {
    text: Vec<u8>,
    is_error: bool,
}

impl Tracker {
    fn session(&self) -> Option<&Session> {
        match &self.init {
            Init::Known(session) => Some(session),
            _ => None,
        }
    }

    fn keep_initialized(&mut self, line: &[u8]) {
        if let Init::Known(session) = &mut self.init
            && session.initialized.is_none()
        {
            session.initialized = Some(line.to_vec());
        }
    }

    /// A message of the client, on its way to the server: `false` when it must not be relayed.
    fn client_sent(&mut self, line: &[u8]) -> bool {
        let kinds = inspect(line);
        let single = kinds.len() == 1;
        let mut relay = true;
        for kind in kinds {
            match kind {
                Kind::Request { id, method } => {
                    if single && method == INITIALIZE && matches!(self.init, Init::Unknown) {
                        self.init = Init::Asked {
                            id: id.clone(),
                            request: line.to_vec(),
                        };
                    }
                    self.client.insert(id);
                }
                Kind::Notification { method, cancelled } => {
                    if single && method == INITIALIZED {
                        self.keep_initialized(line);
                    }
                    if let Some(id) = cancelled {
                        self.client.remove(&id);
                    }
                }
                Kind::Response { id } => {
                    if self.stale.remove(&id) {
                        relay = !single;
                    } else {
                        self.server.remove(&id);
                    }
                }
                Kind::Other => {}
            }
        }
        relay
    }

    fn server_sent(&mut self, line: &[u8]) -> FromServer {
        let kinds = inspect(line);
        let single = kinds.len() == 1;
        let mut verdict = FromServer::Relay;
        for kind in kinds {
            match kind {
                Kind::Response { id } => {
                    self.client.remove(&id);
                    if let Init::Asked { id: asked, request } = &self.init
                        && *asked == id
                    {
                        self.init = match Handshake::of_answer(line) {
                            Some(handshake) => Init::Known(Session {
                                id,
                                request: request.clone(),
                                handshake,
                                initialized: None,
                            }),
                            None => Init::Unknown,
                        };
                    }
                }
                Kind::Request { id, .. } => {
                    if single && self.stale.remove(&id) {
                        verdict = FromServer::Answer(refused_to_server(&id));
                    } else {
                        self.server.insert(id);
                    }
                }
                Kind::Notification {
                    cancelled: Some(id), ..
                } => {
                    self.server.remove(&id);
                }
                Kind::Notification { .. } | Kind::Other => {}
            }
        }
        verdict
    }

    /// The pipe closed: one error per request of the client, one cancellation per request of the server.
    fn cut(&mut self) -> Vec<CutLine> {
        let mut lines: Vec<CutLine> = std::mem::take(&mut self.client)
            .iter()
            .map(|id| CutLine {
                text: restarting(id),
                is_error: true,
            })
            .collect();
        for id in std::mem::take(&mut self.server) {
            lines.push(CutLine {
                text: cancelled(&id),
                is_error: false,
            });
            self.stale.insert(id);
        }
        if matches!(self.init, Init::Asked { .. }) {
            self.init = Init::Unknown;
        }
        lines
    }

    /// A message of the client while no application is there: what the bridge answers it.
    fn while_away(&mut self, line: &[u8]) -> Vec<Vec<u8>> {
        let kinds = inspect(line);
        let single = kinds.len() == 1;
        let mut answers = Vec::new();
        for kind in kinds {
            match kind {
                Kind::Request { id, .. } => answers.push(restarting(&id)),
                Kind::Notification { method, .. } if single && method == INITIALIZED => self.keep_initialized(line),
                Kind::Response { id } => {
                    self.stale.remove(&id);
                }
                Kind::Notification { .. } | Kind::Other => {}
            }
        }
        answers
    }
}
