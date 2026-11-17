//! The streaming server's binary: the configuration read, the database opened and checked for the API's migrations 0023
//! and 0025, the run-object store opened and the cache folder made, the two ports bound, and only then the registration
//! row written (which refuses a database without the API's migration 0022), so no attempt is pointed at a server that
//! cannot listen; the heartbeat, the stats writer and the cache sweeper started, the two listeners served until SIGTERM
//! or Ctrl-C, and on the way out the row deleted first, so no attempt is pointed here any more, then the open streams
//! closed, the uploads still queued given a bounded wait, and the figures still waiting written.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::serve as serve_on;
use runstore::{RunStore, STORAGE_BUCKET_VAR, STORAGE_DIR_VAR};
use sqlx::postgres::PgPoolOptions;
use studio_protocol::Ulid;
use studio_protocol::run_stream::RUNNER_GRACE;
use studio_streaming::cache::{self, Sweep};
use studio_streaming::config::Config;
use studio_streaming::frames::FrameStore;
use studio_streaming::listeners::{Listening, SocketBounds, external, internal};
use studio_streaming::metric_streams::Hubs;
use studio_streaming::provider::Provider;
use studio_streaming::registry::{self, Registration};
use studio_streaming::stats::{Stats, WRITE_EVERY};
use studio_streaming::streams::Streams;
use studio_streaming::upload::{Rows, Uploader};
use tokio::fs::create_dir_all;
use tokio::net::TcpListener;
use tokio::select;
use tokio::signal::ctrl_c;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::timeout;
use ulid::Ulid as RandomUlid;

/// How long the uploads still queued have to land once the server is asked to stop (wall clock).
const DRAIN_WITHIN: Duration = Duration::from_secs(20);

#[tokio::main]
async fn main() -> ExitCode {
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(words) => {
            eprintln!("studio-streaming: {words}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<(), String> {
    let config = Config::from_env().map_err(|fault| fault.to_string())?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&config.database_url)
        .await
        .map_err(|fault| format!("cannot open the database: {fault}"))?;
    registry::require_live_frames(&pool).await?;
    let store = RunStore::from_env().map_err(|fault| {
        format!("cannot open the run-object store that {STORAGE_DIR_VAR} or {STORAGE_BUCKET_VAR} names: {fault}")
    })?;
    create_dir_all(&config.cache_dir)
        .await
        .map_err(|fault| format!("cannot make the cache folder {}: {fault}", config.cache_dir.display()))?;
    let runners = TcpListener::bind(config.runners_bind)
        .await
        .map_err(|fault| format!("cannot listen for runners on {}: {fault}", config.runners_bind))?;
    let viewers = TcpListener::bind(config.viewers_bind)
        .await
        .map_err(|fault| format!("cannot listen for viewers on {}: {fault}", config.viewers_bind))?;
    let id: Ulid = RandomUlid::new().to_string().parse().map_err(|_| "a fresh ULID parses".to_owned())?;
    let registration = Registration::new(id.clone(), &config);
    registry::register(&pool, &registration).await?;
    let stats = Stats::default();
    let uploader = Uploader::new(Arc::new(store.clone()), Some(Rows { pool: pool.clone(), server: id.clone() }));
    let frames = FrameStore::new(config.cache_dir.clone(), config.open_max_bytes, uploader.clone());
    let provider = Provider::new(frames.clone());
    let streams = Streams::new(config.keep, RUNNER_GRACE, stats.clone(), frames);
    let hubs = Hubs::new(provider.clone());
    let beating = registry::beat(pool.clone(), registration, streams.clone(), config.heartbeat_every);
    let writing = stats.writer(pool.clone(), id.clone(), WRITE_EVERY);
    let sweep = Sweep { ttl: config.cache_ttl, max_bytes: config.cache_max_bytes, every: config.cache_sweep_every };
    let sweeping = cache::sweeper(pool.clone(), streams.clone(), provider.clone(), sweep);
    let bounds = SocketBounds::default();
    let listening = Listening { pool: pool.clone(), streams: streams.clone(), provider, hubs, bounds };
    eprintln!(
        "studio-streaming {id} takes runners on {} (reached at {}) and viewers on {} (reached at {}); the frames go \
         to {store} and are cached under {} ({} bytes there)",
        runners.local_addr().map_err(|fault| fault.to_string())?,
        config.runner_url,
        viewers.local_addr().map_err(|fault| fault.to_string())?,
        config.viewer_url,
        config.cache_dir.display(),
        cache::total_bytes(&config.cache_dir).await,
    );
    let outcome = select! {
        served = serve_on(runners, internal(listening.clone())) => {
            served.map_err(|fault| format!("the runners' listener failed: {fault}"))
        }
        served = serve_on(viewers, external(listening)) => {
            served.map_err(|fault| format!("the viewers' listener failed: {fault}"))
        }
        () = stop_asked() => Ok(()),
    };
    beating.abort();
    registry::unregister(&pool, &id).await;
    streams.stop();
    sweeping.abort();
    if timeout(DRAIN_WITHIN, uploader.drain_all()).await.is_err() {
        eprintln!("studio-streaming: uploads were still queued after {DRAIN_WITHIN:?}; the disk copies stay");
    }
    writing.abort();
    stats.write_waiting(&pool, &id).await;
    eprintln!("studio-streaming {id} stopped; its open streams are gone with it, its disk copies stay");
    outcome
}

/// SIGTERM (the platform) or Ctrl-C (a developer).
async fn stop_asked() {
    let mut term = match signal(SignalKind::terminate()) {
        Ok(term) => term,
        Err(fault) => {
            eprintln!("studio-streaming: SIGTERM cannot be heard: {fault}");
            let _ = ctrl_c().await;
            return;
        }
    };
    select! {
        _ = term.recv() => {}
        _ = ctrl_c() => {}
    }
}
