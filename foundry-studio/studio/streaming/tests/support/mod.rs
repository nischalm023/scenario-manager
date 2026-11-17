//! What the streaming server's tests share: a schema of the test's own in the database `STUDIO_TEST_DATABASE_URL`
//! names, brought up by the API's migrations, holding one attempt with its runner token; the server's two listeners as
//! `main` assembles them, on ports of their own, with short bounds so a test runs in seconds, over a scratch cache
//! folder and a folder-backed run-object store; and a fake runner, a fake viewer and a fake frames viewer that speak
//! the sockets from outside. A `Stalled` store holds its puts until released, for the tests that need an upload not
//! yet landed, and a `Gated` store holds each ranged read until the test releases it, failing from a chosen read on,
//! for the tests of a cold fetch's interleavings. The attempt's row is written with its foreign keys' triggers off,
//! since the chain of rows behind an attempt is the API's business, not this server's, so the database must be a
//! superuser's (dev.sh's and the README's throwaway both are). Unset, the tests say they are skipped.

#![allow(dead_code)] // each test file uses its own part of this module

use std::env;
use std::net::SocketAddr;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::serve;
use bytes::Bytes;
use desunits::SimTime;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use run_invocation::Token;
use runstore::{ObjectInfo, RunStorage, RunStore, StoreResult};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::types::Json;
use sqlx::{Executor, FromRow, PgPool};
use studio_protocol::{
    AttemptNumber, EndedBy, Ending, ProgressValue, RunState, RunnerMessage, RunnerReply, STREAM_FORMAT, StreamEnded,
    StreamProgress, StreamRefusal, Ulid, ViewerMessage, ViewerRequest,
};
use studio_streaming::frames::FrameStore;
use studio_streaming::listeners::{Listening, SocketBounds, external, internal};
use studio_streaming::metric_streams::Hubs;
use studio_streaming::provider::Provider;
use studio_streaming::stats::Stats;
use studio_streaming::streams::{StreamKey, Streams};
use studio_streaming::upload::{Rows, Uploader};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use worldview::{Frame, Ms, Tag};

pub mod gated;

pub const DATABASE_VAR: &str = "STUDIO_TEST_DATABASE_URL";
pub const RUN: &str = "01KZAVPQNES65HV4EZADPCS7E8";
pub const TOKEN: &str = "a-token-the-test-minted";
pub const SERVER: &str = "01KZAVPQNES65HV4EZADPCSZZ1";

pub struct Scratch {
    pub pool: PgPool,
    schema: String,
}

impl Scratch {
    pub async fn open() -> Option<Self> {
        let url = env::var(DATABASE_VAR).ok()?;
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_nanos();
        let schema = format!("streaming_test_{nanos:x}_{:x}_{:x}", process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let admin = PgPoolOptions::new().max_connections(1).connect(&url).await.expect("the test database answers");
        admin.execute(format!("CREATE SCHEMA {schema}").as_str()).await.expect("a schema of the test's own");
        admin.close().await;
        let options: PgConnectOptions = url.parse().expect("a connection string");
        let search_path = format!("SET search_path TO {schema}");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |connection, _| {
                let search_path = search_path.clone();
                Box::pin(async move {
                    connection.execute(search_path.as_str()).await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .expect("the scratch schema opens");
        sqlx::migrate!("../api/migrations").run(&pool).await.expect("the API's migrations apply");
        // One attempt with its token, and nothing of the chain of rows behind it.
        for table in ["attempts", "runner_tokens", "attempt_streams", "stream_watch_tickets"] {
            pool.execute(format!("ALTER TABLE {table} DISABLE TRIGGER ALL").as_str()).await.expect("a superuser");
        }
        sqlx::query("INSERT INTO attempts (run_id, n, state, requested_at) VALUES ($1, 1, 'running', now())")
            .bind(RUN)
            .execute(&pool)
            .await
            .expect("the attempt");
        sqlx::query("INSERT INTO runner_tokens (run, n, token_sha256) VALUES ($1, 1, $2)")
            .bind(RUN)
            .bind(Token::presented(TOKEN).sha256())
            .execute(&pool)
            .await
            .expect("its token");
        Some(Self { pool, schema })
    }

    /// A watch ticket for the attempt, as the API mints one, alive for `for_secs` (dead already when negative).
    pub async fn ticket(&self, for_secs: i64) -> String {
        let ticket = Token::mint();
        sqlx::query(
            "INSERT INTO stream_watch_tickets (token_sha256, run_id, n, subject, expires_at) \
             VALUES ($1, $2, 1, 'u-1', now() + make_interval(secs => $3))",
        )
        .bind(ticket.sha256())
        .bind(RUN)
        .bind(for_secs as f64)
        .execute(&self.pool)
        .await
        .expect("a watch ticket");
        ticket.expose().to_owned()
    }

    pub async fn close(self) {
        self.pool.execute(format!("DROP SCHEMA {} CASCADE", self.schema).as_str()).await.expect("the schema drops");
        self.pool.close().await;
    }
}

#[macro_export]
macro_rules! scratch_or_skip {
    () => {
        match support::Scratch::open().await {
            Some(scratch) => scratch,
            None => {
                eprintln!("skipped: set {} to a PostgreSQL the tests may create schemas in", support::DATABASE_VAR);
                return;
            }
        }
    };
}

/// The bounds most tests run under: a ping every 100 ms, and a socket gone only after 5 s of silence, so a socket a
/// test does not read for a while, answering no ping, is not dropped under load.
pub const BOUNDS: SocketBounds = SocketBounds {
    first_message: Duration::from_secs(2),
    ping_every: Duration::from_millis(100),
    silent_after: Duration::from_secs(5),
    write_within: Duration::from_secs(2),
};

/// A runner is lost 300 ms after its socket goes.
pub const GRACE: Duration = Duration::from_millis(300);

pub const KEEP: usize = 50;

/// The server's two listeners on ports of their own, with its stats writer, as `main` assembles them, and what a
/// test reaches in behind them: the streams, the provider, the metric hubs, the uploader and the scratch folder (a
/// cache folder and an object folder under it).
pub struct Server {
    pub internal: SocketAddr,
    pub external: SocketAddr,
    pub streams: Streams,
    pub provider: Provider,
    pub hubs: Hubs,
    pub uploader: Uploader,
    pub dir: Arc<TempDir>,
}

/// How a test's frame store differs from the defaults.
pub struct FramesOptions {
    /// The scratch folder, shared between two servers to play a restart.
    pub dir: Arc<TempDir>,
    pub open_max_bytes: u64,
    /// The store, when a test wants one other than the plain folder store over `dir`.
    pub store: Option<Arc<dyn RunStorage>>,
}

impl Default for FramesOptions {
    fn default() -> Self {
        Self { dir: Arc::new(TempDir::new().expect("a scratch folder")), open_max_bytes: u64::MAX, store: None }
    }
}

impl FramesOptions {
    /// The folder the store writes objects into.
    pub fn objects(&self) -> PathBuf {
        self.dir.path().join("objects")
    }
}

pub async fn server(pool: &PgPool) -> Server {
    server_within(pool, BOUNDS).await
}

pub async fn server_within(pool: &PgPool, bounds: SocketBounds) -> Server {
    server_with(pool, bounds, FramesOptions::default()).await
}

pub async fn server_with(pool: &PgPool, bounds: SocketBounds, frames: FramesOptions) -> Server {
    let stats = Stats::default();
    let id: Ulid = SERVER.parse().unwrap();
    let store: Arc<dyn RunStorage> = match &frames.store {
        Some(store) => store.clone(),
        None => {
            std::fs::create_dir_all(frames.objects()).expect("the objects folder");
            Arc::new(RunStore::local_folder(&frames.objects()).expect("a folder store"))
        }
    };
    let uploader = Uploader::new(store, Some(Rows { pool: pool.clone(), server: id.clone() }))
        .with_backoff(Duration::from_millis(10), Duration::from_millis(50));
    let frame_store = FrameStore::new(frames.dir.path().join("cache"), frames.open_max_bytes, uploader.clone());
    let provider = Provider::new(frame_store.clone());
    let streams = Streams::new(KEEP, GRACE, stats.clone(), frame_store);
    let hubs = Hubs::new(provider.clone());
    stats.writer(pool.clone(), id, Duration::from_millis(100));
    let listening = Listening {
        pool: pool.clone(),
        streams: streams.clone(),
        provider: provider.clone(),
        hubs: hubs.clone(),
        bounds,
    };
    let runners = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let viewers = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (internal_at, external_at) = (runners.local_addr().unwrap(), viewers.local_addr().unwrap());
    let (to_runners, to_viewers) = (internal(listening.clone()), external(listening));
    tokio::spawn(async move { serve(runners, to_runners).await.unwrap() });
    tokio::spawn(async move { serve(viewers, to_viewers).await.unwrap() });
    Server { internal: internal_at, external: external_at, streams, provider, hubs, uploader, dir: frames.dir }
}

pub fn key() -> StreamKey {
    StreamKey { run: run(), attempt: AttemptNumber(1) }
}

/// Until the server holds the attempt's frames through `at_ms` of simtime: a runner's frames are taken in their own
/// time after the test sends them, so a viewer that must find them attaches only then.
pub async fn held_through(server: &Server, at_ms: u64) {
    for _ in 0..100 {
        if server.streams.frames().get(&key()).and_then(|held| held.reach()).is_some_and(|at| at.as_ms() >= at_ms) {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the server never held the frames through {at_ms} ms");
}

/// A frame store over a scratch folder kept for the process, for tests of the streams that never touch it.
pub fn frame_store() -> FrameStore {
    let dir = TempDir::new().expect("a scratch folder").keep();
    std::fs::create_dir_all(dir.join("objects")).expect("the objects folder");
    let store = RunStore::local_folder(&dir.join("objects")).expect("a folder store");
    FrameStore::new(dir.join("cache"), u64::MAX, Uploader::new(Arc::new(store), None))
}

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn send(socket: &mut Socket, text: String) {
    socket.send(Message::text(text)).await.expect("sent");
}

/// The next text frame within 5 s, `None` on a close or nothing.
pub async fn next_text(socket: &mut Socket) -> Option<String> {
    loop {
        match timeout(Duration::from_secs(5), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return Some(text.to_string()),
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            _ => return None,
        }
    }
}

/// The close frame's refusal within 5 s, reading past what comes before it.
pub async fn refusal(socket: &mut Socket) -> Option<StreamRefusal> {
    loop {
        match timeout(Duration::from_secs(5), socket.next()).await {
            Ok(Some(Ok(Message::Close(frame)))) => {
                return frame.and_then(|frame| StreamRefusal::from_reason(&frame.reason));
            }
            Ok(Some(Ok(_))) => continue,
            _ => return None,
        }
    }
}

/// A runner's socket that said hello with `token`.
pub async fn runner(server: &Server, token: &str) -> Socket {
    let (mut socket, _) = connect_async(format!("ws://{}/runners", server.internal)).await.expect("the socket opens");
    let hello = RunnerMessage::Hello { v: STREAM_FORMAT, run: run(), attempt: AttemptNumber(1), token: token.into() };
    send(&mut socket, hello.text()).await;
    socket
}

/// A runner the server welcomed.
pub async fn welcomed(server: &Server) -> Socket {
    let mut socket = runner(server, TOKEN).await;
    let welcome = next_text(&mut socket).await.expect("an answer to the hello");
    assert_eq!(RunnerReply::read(&welcome), Ok(RunnerReply::Welcome { v: STREAM_FORMAT }));
    socket
}

/// A welcomed runner whose socket is read in the background, so the server's pings are answered as a real runner's
/// are; dropping it closes the socket.
pub struct Runner {
    sink: SplitSink<Socket, Message>,
    closed: JoinHandle<Option<StreamRefusal>>,
}

impl Runner {
    pub async fn welcomed(server: &Server) -> Self {
        let (sink, mut source) = welcomed(server).await.split();
        let closed = tokio::spawn(async move {
            while let Some(Ok(frame)) = source.next().await {
                if let Message::Close(frame) = frame {
                    return frame.and_then(|frame| StreamRefusal::from_reason(&frame.reason));
                }
            }
            None
        });
        Self { sink, closed }
    }

    pub async fn send(&mut self, message: RunnerMessage) {
        self.sink.send(Message::text(message.text())).await.expect("sent");
    }

    /// One frame of the picture, as a binary message.
    pub async fn send_frame(&mut self, bytes: Vec<u8>) {
        self.sink.send(Message::binary(bytes)).await.expect("sent");
    }

    /// The refusal the server closed the socket with, within 5 s.
    pub async fn refusal(&mut self) -> Option<StreamRefusal> {
        timeout(Duration::from_secs(5), &mut self.closed).await.ok()?.ok()?
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.closed.abort();
    }
}

/// A frames viewer's socket on the attempt, attached with `ticket` from `from` (`None` for no `from` at all; the
/// JSON is spelled by the test, as the viewer's JavaScript spells it).
pub async fn frames_viewer(server: &Server, ticket: &str, from: Option<Value>) -> Socket {
    let url = format!("ws://{}/frames/{RUN}/1", server.external);
    let (mut socket, _) = connect_async(url).await.expect("the socket opens");
    let mut attach = json!({ "type": "attach", "id": "1", "run": RUN, "sections": [], "token": ticket });
    if let Some(from) = from {
        attach["from"] = from;
    }
    socket.send(Message::text(attach.to_string())).await.expect("attach sent");
    socket
}

/// The next message of any kind, within 5 s.
pub async fn next_raw(socket: &mut Socket) -> Option<Message> {
    match timeout(Duration::from_secs(5), socket.next()).await {
        Ok(Some(Ok(message))) => Some(message),
        _ => None,
    }
}

pub async fn send_binary(socket: &mut Socket, bytes: Vec<u8>) {
    socket.send(Message::binary(bytes)).await.expect("sent");
}

/// A frame whose body is `len` bytes, compressed as the writer compresses them.
pub fn frame(tag: Tag, at_ms: u64, len: usize) -> Vec<u8> {
    match tag {
        // A browser is sent the scenario block with its floor compiled in, so it has to be a real one.
        Tag::Scenario => scenario_frame(),
        _ => Frame::encode(tag, Ms(at_ms), &vec![(at_ms % 251) as u8; len], true).expect("a frame encodes"),
    }
}

/// A real scenario frame, whose world compiles: the first frame of the viewer's fixture recording.
pub fn scenario_frame() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../viewer/tests/fixtures/minimal_easyloop_flow/recording.worldview");
    let bytes = std::fs::read(path).expect("the fixture recording");
    let (_, length) = Frame::decode_at(&bytes, 0).expect("its scenario frame");
    bytes[..length].to_vec()
}

/// A frame's tag and instant off its header.
pub fn stamped(bytes: &[u8]) -> (Tag, u64) {
    let (frame, _) = Frame::decode_at(bytes, 0).expect("a frame");
    (frame.header.tag, frame.header.t.0)
}

/// A viewer's socket on the attempt that said watch with `ticket`.
pub async fn viewer(server: &Server, ticket: &str) -> Socket {
    let url = format!("ws://{}/streams/{RUN}/1", server.external);
    let (mut socket, _) = connect_async(url).await.expect("the socket opens");
    send(&mut socket, ViewerRequest::Watch { v: STREAM_FORMAT, token: ticket.into() }.text()).await;
    socket
}

/// The viewer's messages from `attached` on: the attachment's replay count, then `count` more.
pub async fn watched(socket: &mut Socket, count: usize) -> (u32, Vec<ViewerMessage>) {
    let attached = ViewerMessage::read(&next_text(socket).await.expect("attached")).expect("a viewer message");
    let ViewerMessage::Attached { replayed, .. } = attached else { panic!("attached first, not {attached:?}") };
    let mut seen = Vec::new();
    for _ in 0..count {
        let text = next_text(socket).await.expect("a message");
        seen.push(ViewerMessage::read(&text).expect("a viewer message"));
    }
    (replayed, seen)
}

pub fn run() -> Ulid {
    RUN.parse().unwrap()
}

pub fn started() -> RunnerMessage {
    RunnerMessage::Started { v: STREAM_FORMAT }
}

/// The run's progress at `sim_ms` with `picks` picks so far.
pub fn progress(sim_ms: u64, picks: f64) -> StreamProgress {
    let value = ProgressValue { metric: "picks".to_owned(), label: "Picks".to_owned(), value: Some(picks) };
    StreamProgress { v: STREAM_FORMAT, sim_ms: SimTime::ms(sim_ms), values: vec![value] }
}

pub fn ended(sim_ms: u64) -> StreamEnded {
    let ending = Ending {
        state: RunState::Finished,
        ended_by: EndedBy::Simulator,
        stop_reason: "SimTimeLimit".to_owned(),
        stop_detail: None,
        sim_ms: SimTime::ms(sim_ms),
    };
    StreamEnded { v: STREAM_FORMAT, ending }
}

/// The attempt's `attempt_streams` row once `ready` holds of it, polled for up to 5 s.
pub async fn stream_row(pool: &PgPool, ready: impl Fn(&StreamRow) -> bool) -> StreamRow {
    for _ in 0..50 {
        let row: Option<StreamRow> = sqlx::query_as(
            "SELECT revision, sim_ms, progress_values, closed_by FROM attempt_streams WHERE run_id = $1 AND n = 1",
        )
        .bind(RUN)
        .fetch_optional(pool)
        .await
        .expect("the row reads");
        if let Some(row) = row
            && ready(&row)
        {
            return row;
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("the stream's row never came to what the test waits for");
}

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct StreamRow {
    pub revision: i64,
    pub sim_ms: i64,
    pub progress_values: Json<Vec<ProgressValue>>,
    pub closed_by: Option<String>,
}

/// A folder store whose puts wait while `held` is set: an upload that has not landed yet, for as long as the test
/// says.
#[derive(Debug)]
pub struct Stalled {
    pub inner: RunStore,
    pub held: Arc<AtomicBool>,
}

impl Stalled {
    pub fn over(folder: &Path) -> Self {
        std::fs::create_dir_all(folder).expect("the objects folder");
        Self { inner: RunStore::local_folder(folder).expect("a folder store"), held: Arc::new(AtomicBool::new(true)) }
    }

    async fn wait(&self) {
        while self.held.load(Ordering::Relaxed) {
            sleep(Duration::from_millis(20)).await;
        }
    }
}

#[async_trait]
impl RunStorage for Stalled {
    async fn put(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        self.wait().await;
        self.inner.put(key, bytes).await
    }

    async fn put_file(&self, key: &str, path: &Path) -> StoreResult<ObjectInfo> {
        self.wait().await;
        self.inner.put_file(key, path).await
    }

    async fn create(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        self.inner.create(key, bytes).await
    }

    async fn get(&self, key: &str) -> StoreResult<Bytes> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> StoreResult<Bytes> {
        self.inner.get_range(key, range).await
    }

    async fn list(&self, prefix: &str) -> StoreResult<Vec<ObjectInfo>> {
        self.inner.list(prefix).await
    }

    async fn delete(&self, key: &str) -> StoreResult<()> {
        self.inner.delete(key).await
    }

    async fn head(&self, key: &str) -> StoreResult<ObjectInfo> {
        self.inner.head(key).await
    }
}
