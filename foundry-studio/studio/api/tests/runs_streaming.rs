//! Ruling D174 end to end on the local invoker: a run's container streams its progress to the streaming server the
//! dispatcher picked, and a viewer holding a ticket the API minted watches it live on the server's external port.
//! What the tests need is in `support::containers`; any of it missing, they say they are skipped.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::serve;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use studio_api::db::mint_ulid;
use runstore::RunStore;
use studio_api::runs::HOST_ALIAS;
use studio_protocol::run_stream::RUNNER_GRACE;
use studio_protocol::{RunState, STREAM_FORMAT, ViewerMessage, ViewerRequest};
use studio_streaming::frames::FrameStore;
use studio_streaming::listeners::{Listening, SocketBounds, external, internal};
use studio_streaming::metric_streams::Hubs;
use studio_streaming::provider::Provider;
use studio_streaming::registry::{self, Registration};
use studio_streaming::stats::Stats;
use studio_streaming::streams::Streams;
use studio_streaming::upload::{Rows, Uploader};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use support::containers::{RemovedAtEnd, local, loops, needed, run_of, state_is, submitted};
use support::runs::{send, until};

/// A streaming server started in this process, registered in the scratch schema and beating there as its binary does,
/// is what the dispatcher points the attempt at; the runner in its container reaches it through the host's alias and
/// streams the run; a viewer with the API's ticket is attached and sees `started`, the run's progress, and `ended` with
/// `finished`, the report's own simtime; and the server's `attempt_streams` row holds that simtime, the runner's
/// counters by name, and the runner's closing.
#[tokio::test]
async fn a_run_streams_its_progress_to_the_streaming_server_which_a_viewer_with_a_ticket_watches_live() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &needed.image, &[], 5).await;
    let stack = &local.stack;
    // The server: both listeners on ports of their own, the runners' one where a container reaches it.
    let server_id = mint_ulid();
    let stats = Stats::default();
    // The picture's frames (live frames) over a scratch cache folder and a folder store, as `main` assembles them.
    let scratch_dir = tempdir().expect("a scratch folder");
    let objects = scratch_dir.path().join("objects");
    std::fs::create_dir_all(&objects).expect("the objects folder");
    let store = RunStore::local_folder(&objects).expect("a folder store");
    let rows = Rows { pool: scratch.pool.clone(), server: server_id.clone() };
    let frames = FrameStore::new(scratch_dir.path().join("cache"), u64::MAX, Uploader::new(Arc::new(store), Some(rows)));
    let provider = Provider::new(frames.clone());
    let streams = Streams::new(50, RUNNER_GRACE, stats.clone(), frames);
    let hubs = Hubs::new(provider.clone());
    stats.writer(scratch.pool.clone(), server_id.clone(), Duration::from_millis(200));
    let pool = scratch.pool.clone();
    let listening = Listening { pool, streams: streams.clone(), provider, hubs, bounds: SocketBounds::default() };
    let bind = if cfg!(target_os = "linux") { "0.0.0.0:0" } else { "127.0.0.1:0" };
    let runners = TcpListener::bind(bind).await.expect("a port for the runners");
    let viewers = TcpListener::bind("127.0.0.1:0").await.expect("a port for the viewers");
    let (runners_port, viewers_at) = (runners.local_addr().unwrap().port(), viewers.local_addr().unwrap());
    let (to_runners, to_viewers) = (internal(listening.clone()), external(listening));
    tokio::spawn(async move { serve(runners, to_runners).await.unwrap() });
    tokio::spawn(async move { serve(viewers, to_viewers).await.unwrap() });
    let registration = Registration {
        id: server_id,
        runner_url: format!("ws://{HOST_ALIAS}:{runners_port}/runners"),
        viewer_url: format!("ws://{viewers_at}/streams"),
    };
    registry::register(&scratch.pool, &registration).await.expect("the server registered");
    // Beating every 2 s of wall clock, as the server does by default, so the dispatcher's pick and the watch ticket
    // find it alive however long the run takes to start.
    let beating = registry::beat(scratch.pool.clone(), registration, streams, Duration::from_secs(2));

    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let run = &runs[0];
    until("the stream to open", || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM attempt_streams").fetch_one(&scratch.pool).await.unwrap() > 0
    })
    .await;
    let (status, ticket) = send(&stack.router, "POST", &format!("/api/runs/{run}/watch"), None).await;
    assert_eq!(status.as_u16(), 200, "{ticket}");
    let (mut viewer, _) = connect_async(ticket["url"].as_str().expect("its url")).await.expect("the server's port");
    let watch = ViewerRequest::Watch { v: STREAM_FORMAT, token: ticket["token"].as_str().unwrap().to_owned() };
    viewer.send(Message::text(watch.text())).await.expect("the watch said");
    let mut seen = Vec::new();
    while let Ok(Some(Ok(message))) = timeout(Duration::from_secs(120), viewer.next()).await {
        match message {
            Message::Text(text) => {
                let message = ViewerMessage::read(&text).expect("a viewer's message");
                let over = matches!(message, ViewerMessage::Ended(_));
                seen.push(message);
                if over {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = viewer.close(None).await;
    assert!(matches!(seen.first(), Some(ViewerMessage::Attached { .. })), "{seen:?}");
    assert!(seen.iter().any(|message| matches!(message, ViewerMessage::Started { .. })), "{seen:?}");
    assert!(seen.iter().any(|message| matches!(message, ViewerMessage::Progress(_))), "{seen:?}");
    let Some(ViewerMessage::Ended(ended)) = seen.last() else { panic!("ended last: {seen:?}") };
    assert_eq!((ended.v, ended.ending.state), (STREAM_FORMAT, RunState::Finished), "{ended:?}");
    until("the run to end", || state_is(stack, run, "finished")).await;
    let read = run_of(stack, run).await;
    let reached = read["progress"]["sim_ms"].as_u64().expect("the simtime reached");
    assert_eq!(ended.ending.sim_ms.as_ms(), reached, "the stream's ending has the report's simtime");
    let (sim_ms, closed_by, progress): (i64, Option<String>, Value) =
        sqlx::query_as("SELECT sim_ms, closed_by, progress_values FROM attempt_streams WHERE run_id = $1 AND n = 1")
            .bind(run.as_str())
            .fetch_one(&scratch.pool)
            .await
            .expect("the server wrote the run's figures");
    assert_eq!((sim_ms as u64, closed_by.as_deref()), (reached, Some("runner")));
    assert!(
        progress.as_array().is_some_and(|values| values.iter().any(|value| value["metric"] == "picks")),
        "{progress}"
    );
    drop(running);
    // Awaited once aborted, so no beat is left to write into the dropped schema.
    beating.abort();
    assert!(beating.await.is_err_and(|stopped| stopped.is_cancelled()), "the beats end by the abort");
    scratch.close().await;
}
