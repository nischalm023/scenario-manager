//! The server binary: the assembly, and the order of it.
//!
//! The order matters in one place. Every attempt the registry still calls
//! `running` is swept to `partial` BEFORE a worker starts, because those
//! attempts belong to a process that is gone, and a worker that claimed one
//! would be running a second copy of a run whose first copy nobody can account
//! for. Nothing is repaired and nothing is synthesised — a `partial` attempt
//! says the server stopped, and that is all anyone knows about it.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use runstore::{RunStorage, RunStore};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::{error, info};

use crucible::config::Config;
use crucible::recording;
use crucible::registry::Registry;
use crucible::runs::Runs;
use crucible::ws;

/// Database connections beyond one per worker: each running attempt also has a
/// progress writer, and the websocket's own reads need not queue behind either.
const CONNECTIONS_BESIDE_THE_WORKERS: u32 = 4;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            error!("{failure}");
            ExitCode::FAILURE
        }
    }
}

/// Everything the server needs, in the order it needs it.
async fn serve() -> Result<(), String> {
    let config = Config::from_env().map_err(|failure| failure.to_string())?;
    let store = RunStore::from_env().map_err(|failure| failure.to_string())?;
    store.probe().await.map_err(|failure| format!("{store} is not usable: {failure}"))?;
    info!(storage = %store, "run output is stored in");
    let storage: Arc<dyn RunStorage> = Arc::new(store);
    let connections = config.runs_at_once as u32 * 2 + CONNECTIONS_BESIDE_THE_WORKERS;
    let registry = Registry::connect(&config.database_url, connections).await.map_err(|failure| failure.to_string())?;

    let swept = registry.sweep_interrupted().await.map_err(|failure| failure.to_string())?;
    if swept > 0 {
        info!(attempts = swept, "attempts left running by a stopped server are now partial");
    }
    // The recordings this machine fetched and the floors it compiled from them
    // survive a restart, which is the point of them; the ceiling they are kept
    // under has to survive it too, so it is applied before anything is served.
    recording::under_ceiling(&config.temp_dir, config.cache_bytes, Path::new(""));

    let bind = config.bind.clone();
    let max_message_bytes = config.max_message_bytes;
    let runs = Runs::new(config, registry, storage);
    runs.start_workers();

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(upgrade))
        .layer(CorsLayer::permissive())
        .with_state(Socket { runs, max_message_bytes });

    let listener = TcpListener::bind(&bind).await.map_err(|failure| format!("cannot listen on {bind}: {failure}"))?;
    info!(address = %bind, "crucible is listening");
    axum::serve(listener, app).await.map_err(|failure| failure.to_string())
}

/// What a websocket handler is handed.
#[derive(Clone)]
struct Socket {
    runs: Arc<Runs>,
    max_message_bytes: usize,
}

/// How far past this server's own ceiling the transport still assembles a
/// message, so that what a browser reads back is this server's refusal in
/// words rather than a socket that shut.
const TRANSPORT_MARGIN: usize = 1024 * 1024;

async fn upgrade(upgrade: WebSocketUpgrade, State(socket): State<Socket>) -> impl IntoResponse {
    // The transport's ceiling sits ABOVE the server's own, by that margin. The
    // two were equal, and a message one byte over was dropped by the protocol
    // layer before `ws`'s refusal could be written — so a browser that sent too
    // much was told nothing and watched its socket close. With the margin, a
    // submission a little over arrives whole and is refused in a sentence the
    // form shows. Anything far larger is still the transport's to drop, and a
    // page that was told the ceiling on connect refuses it before sending.
    upgrade
        .max_message_size(socket.max_message_bytes.saturating_add(TRANSPORT_MARGIN))
        .max_frame_size(socket.max_message_bytes.saturating_add(TRANSPORT_MARGIN))
        .on_upgrade(move |ws| ws::serve(ws, socket.runs, socket.max_message_bytes))
}
