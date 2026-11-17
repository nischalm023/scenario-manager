//! What a browser gets over a REAL socket: `serve` behind an axum upgrade, a
//! websocket client on the other end, and a run whose recording is already
//! stored.
//!
//! The unit tests reach into the reader and the write side separately; this one
//! is the two of them on one socket, which is where the rules that only exist
//! between them live — a second `attach` replacing the first, `detach` stopping
//! the frames without closing the socket, and a recording that will not read
//! ending the attachment with a message naming the run rather than a panic.
//!
//! The registry is a schema of its own inside the database `DATABASE_URL`
//! names, as in `tests/registry.rs`; with none set these report why they are
//! skipped, because the workspace has to test without a database in sight.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use crucible::config::Config;
use crucible::protocol::{AttachFrom, AttemptState, Reply, Request};
use crucible::registry::Registry;
use crucible::runs::Runs;
use crucible::ws;
use futures::{SinkExt, StreamExt};
use runstore::layout::{AttemptNumber, RunId, recording_key};
use runstore::{RunStorage, RunStore};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, PgPool};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

mod support;

/// A server of its own: its schema, its storage, its temporary folder and the
/// address it answers on.
struct Standing {
    address: String,
    temp: TempDir,
    _storage: TempDir,
    pool: PgPool,
    schema: String,
}

impl Standing {
    /// Start one over a run whose recording is already stored, or say that no
    /// database was named.
    async fn open(damaged: bool) -> Option<Self> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let schema = format!("crucible_socket_{}", std::process::id() as u64 * 1_000 + damaged as u64);
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap_or_else(|failure| panic!("DATABASE_URL names a database this test cannot reach: {failure}"));
        let _ = admin.execute(format!("DROP SCHEMA IF EXISTS {schema} CASCADE").as_str()).await;
        admin.execute(format!("CREATE SCHEMA {schema}").as_str()).await.expect("a schema of our own");
        admin.close().await;

        let options: PgConnectOptions = url.parse().expect("DATABASE_URL is a connection string");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect({
                let schema = schema.clone();
                move |connection, _| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        connection.execute(format!("SET search_path TO {schema}").as_str()).await?;
                        Ok(())
                    })
                }
            })
            .connect_with(options)
            .await
            .expect("a pool in that schema");
        let registry = Registry::from_pool(pool.clone()).await.expect("the schema migrates");

        let storage_dir = TempDir::new().expect("a folder");
        let temp = TempDir::new().expect("a folder");
        let storage = RunStore::local_folder(storage_dir.path()).expect("a folder store");
        let id = RunId::new(RUN).expect("a key segment");
        let recording = support::written(temp.path(), damaged);
        let bytes = std::fs::read(&recording).expect("the recording");
        storage.put(&recording_key(&id, AttemptNumber::FIRST), bytes.into()).await.expect("stored");

        registry.submit(RUN, "socket", "runs/01TESTSOCKET/spec.json").await.expect("the run");
        registry.queue_first_attempt(RUN).await.expect("its attempt");
        let claimed = registry.claim_queued().await.expect("claimable").expect("one attempt");
        registry
            .attempt_ended(
                RUN,
                claimed.n,
                AttemptState::Finished,
                Some("reached the horizon"),
                Some(&recording_key(&id, AttemptNumber::FIRST)),
                None,
                None,
            )
            .await
            .expect("it ends");

        let config = config(temp.path());
        let runs = Runs::new(config, registry, Arc::new(storage));
        let app = Router::new().route("/ws", get(upgrade)).with_state(runs);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = format!("ws://{}/ws", listener.local_addr().expect("bound"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        Some(Standing { address, temp, _storage: storage_dir, pool, schema })
    }

    async fn close(self) {
        let _ = self.pool.execute(format!("DROP SCHEMA {} CASCADE", self.schema).as_str()).await;
        self.pool.close().await;
        drop(self.temp);
    }
}

const RUN: &str = "01TESTSOCKET";

fn config(temp: &Path) -> Config {
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: String::new(),
        temp_dir: temp.to_path_buf(),
        runs_at_once: 1,
        max_message_bytes: 32 * 1024 * 1024,
        attach_lag: Duration::from_secs(5),
        series_points: 2_000,
        cache_bytes: u64::MAX,
    }
}

async fn upgrade(upgrade: WebSocketUpgrade, State(runs): State<Arc<Runs>>) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| ws::serve(socket, runs, 32 * 1024 * 1024))
}

/// One socket, driven by hand: what was said, and what came back.
struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Client {
    async fn open(address: &str) -> Self {
        let (socket, _) = connect_async(address).await.expect("the server answers");
        Self { socket }
    }

    async fn say(&mut self, request: Request) {
        let text = serde_json::to_string(&request).expect("encodes");
        self.socket.send(Message::Text(text.into())).await.expect("sent");
    }

    /// Read until the reply that answers this request id, stepping over the
    /// frames and the replies that answer an earlier one.
    async fn answering(&mut self, id: &str) -> Reply {
        for _ in 0..64 {
            let (reply, _) = self.reply().await;
            if reply.answers() == Some(id) {
                return reply;
            }
        }
        panic!("nothing answered request {id}");
    }

    /// Read until the next JSON reply, counting the binary frames on the way.
    async fn reply(&mut self) -> (Reply, usize) {
        let mut frames = 0;
        while let Some(message) = self.socket.next().await {
            match message.expect("the socket holds") {
                Message::Binary(_) => frames += 1,
                Message::Text(text) => return (serde_json::from_str(&text).expect("a reply"), frames),
                Message::Close(_) => break,
                _ => continue,
            }
        }
        panic!("the socket closed without a reply");
    }
}

fn skipped(what: &str) {
    eprintln!("skipped: {what} needs DATABASE_URL to name a PostgreSQL this test may make a schema in");
}

#[tokio::test]
async fn one_socket_attaches_replaces_and_detaches() {
    let Some(standing) = Standing::open(false).await else { return skipped("the socket test") };
    let mut client = Client::open(&standing.address).await;

    // Before anything is asked for: what this server takes in one message. A
    // form encoding a submission out of files somebody chose has to know the
    // ceiling while they are still choosing, and there is nowhere else to ask.
    let (greeting, frames) = client.reply().await;
    assert_eq!(frames, 0, "the greeting is the first thing on the socket");
    assert!(matches!(greeting, Reply::Hello { max_message_bytes: 33_554_432 }), "got {greeting:?}");

    client
        .say(Request::Attach { id: "1".into(), run: RUN.into(), from: AttachFrom::At(0), sections: Vec::new() })
        .await;
    let reply = client.answering("1").await;
    let Reply::Attached { attempt, from_ms, live_edge_ms, .. } = reply else { panic!("got {reply:?}") };
    assert_eq!((attempt, from_ms), (1, 0), "the stored attempt, at the instant asked for");
    assert!(live_edge_ms > 0, "and the run reaches somewhere");

    // A second attach replaces the first: its own reply arrives, and the socket
    // is still the same socket.
    client.say(Request::Attach { id: "2".into(), run: RUN.into(), from: AttachFrom::Live, sections: Vec::new() }).await;
    let reply = client.answering("2").await;
    let Reply::Attached { from_ms, live_edge_ms, .. } = reply else { panic!("the second attach: got {reply:?}") };
    assert_eq!(from_ms, live_edge_ms, "`live` on a finished run is where it stopped");

    // `detach` stops the frames; the `status` after it proves the socket lives
    // on, which is the whole point of not closing it.
    client.say(Request::Detach { id: "3".into() }).await;
    let reply = client.answering("3").await;
    assert!(matches!(reply, Reply::Detached { .. }), "detach is answered: got {reply:?}");

    client.say(Request::Status { id: "4".into(), run: RUN.into() }).await;
    let reply = client.answering("4").await;
    assert!(matches!(reply, Reply::Status { .. }), "the socket lives on: got {reply:?}");
    standing.close().await;
}

#[tokio::test]
async fn an_attachment_to_a_damaged_recording_says_which_run_and_where() {
    let Some(standing) = Standing::open(true).await else { return skipped("the damaged-recording test") };
    let mut client = Client::open(&standing.address).await;
    client
        .say(Request::Attach { id: "1".into(), run: RUN.into(), from: AttachFrom::At(0), sections: Vec::new() })
        .await;

    let reply = client.answering("1").await;
    let Reply::Error { message, id } = reply else { panic!("a truncated recording is not servable: got {reply:?}") };
    assert_eq!(id.as_deref(), Some("1"), "the refusal answers the attach");
    assert!(message.contains(RUN), "and names the run: {message}");
    assert!(message.contains("attempt 1"), "and which try at it: {message}");

    client.say(Request::Status { id: "2".into(), run: RUN.into() }).await;
    let reply = client.answering("2").await;
    assert!(matches!(reply, Reply::Status { .. }), "and the socket lives on: got {reply:?}");
    standing.close().await;
}
