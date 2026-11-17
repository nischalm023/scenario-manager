//! A runner's socket on the streaming server's internal port, against the database: a hello with the attempt's token
//! is welcomed, and one with a wrong token, a Job known stopped or another format is refused in the close frame; a
//! second socket of the runner takes the stream over and the first is closed `superseded`; a runner that stops
//! answering the server's pings is taken for gone though its socket stays open; and one gone without its ending is
//! lost after the grace, which its viewers are told, and takes its stream up again when it comes back.

mod support;

use std::time::Duration;

use studio_protocol::{RunnerMessage, StreamRefusal, ViewerMessage};
use studio_streaming::listeners::SocketBounds;
use tokio_tungstenite::connect_async;

use support::{
    BOUNDS, RUN, Runner, TOKEN, ended, next_text, progress, refusal, runner, send, server, server_within, started,
    stream_row, viewer, watched, welcomed,
};

#[tokio::test]
async fn a_runner_with_its_attempts_token_is_welcomed_and_any_other_refused() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut wrong = runner(&server, "not-the-token").await;
    assert_eq!(refusal(&mut wrong).await, Some(StreamRefusal::TokenInvalid));
    let (mut other, _) = connect_async(format!("ws://{}/runners", server.internal)).await.unwrap();
    let hello = format!(r#"{{"type":"hello","v":1,"run":"{RUN}","attempt":1,"token":"{TOKEN}"}}"#);
    send(&mut other, hello).await;
    assert_eq!(refusal(&mut other).await, Some(StreamRefusal::UnsupportedVersion), "another format");
    welcomed(&server).await;
    // The door's own rule: a Job known stopped makes its token open nothing.
    sqlx::query(
        "UPDATE attempts SET job = 'run-x-1', dispatched_at = now(), job_claimed_at = now(), job_stopped_at = now()",
    )
    .execute(&scratch.pool)
    .await
    .unwrap();
    let mut stopped = runner(&server, TOKEN).await;
    assert_eq!(refusal(&mut stopped).await, Some(StreamRefusal::TokenInvalid));
    scratch.close().await;
}

#[tokio::test]
async fn a_reconnecting_runner_takes_the_stream_over_and_the_old_socket_is_closed() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut first = Runner::welcomed(&server).await;
    first.send(started()).await;
    first.send(RunnerMessage::Progress(progress(10_000, 1.0))).await;
    let mut second = Runner::welcomed(&server).await;
    assert_eq!(first.refusal().await, Some(StreamRefusal::Superseded));
    second.send(started()).await;
    second.send(RunnerMessage::Progress(progress(20_000, 2.0))).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = viewer(&server, &ticket).await;
    let (replayed, seen) = watched(&mut watching, 2).await;
    assert_eq!(replayed, 2, "one started and the newest progress: {seen:?}");
    assert_eq!(seen[1], ViewerMessage::Progress(progress(20_000, 2.0)), "the stream went on under the second socket");
    scratch.close().await;
}

#[tokio::test]
async fn a_runner_that_stops_answering_pings_is_taken_for_gone_and_then_lost() {
    let scratch = scratch_or_skip!();
    let silent_soon = SocketBounds { silent_after: Duration::from_millis(400), ..BOUNDS };
    let server = server_within(&scratch.pool, silent_soon).await;
    let mut quiet = welcomed(&server).await;
    send(&mut quiet, started().text()).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = viewer(&server, &ticket).await;
    watched(&mut watching, 1).await;
    // The runner's socket is not read from now on, so the server's pings go unanswered while the socket stays open;
    // the viewer reads on. (Reading the runner's socket for its close reason would answer the pings.)
    let lost = ViewerMessage::read(&next_text(&mut watching).await.expect("the loss")).unwrap();
    assert!(matches!(lost, ViewerMessage::RunnerLost { .. }), "{lost:?}");
    stream_row(&scratch.pool, |row| row.closed_by.as_deref() == Some("server")).await;
    drop(quiet);
    scratch.close().await;
}

#[tokio::test]
async fn a_runner_gone_without_its_ending_is_lost_after_the_grace_and_takes_its_stream_up_again() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut first = Runner::welcomed(&server).await;
    first.send(started()).await;
    first.send(RunnerMessage::Progress(progress(10_000, 1.0))).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = viewer(&server, &ticket).await;
    assert_eq!(watched(&mut watching, 2).await.0, 2);
    drop(first);
    let lost = ViewerMessage::read(&next_text(&mut watching).await.expect("the loss")).unwrap();
    assert!(matches!(lost, ViewerMessage::RunnerLost { .. }), "{lost:?}");
    stream_row(&scratch.pool, |row| row.closed_by.as_deref() == Some("server")).await;
    let mut back = Runner::welcomed(&server).await;
    back.send(started()).await;
    back.send(RunnerMessage::Progress(progress(20_000, 2.0))).await;
    for expected in ["started", "progress"] {
        let text = next_text(&mut watching).await.expect("the stream goes on");
        assert!(text.contains(&format!("\"type\":\"{expected}\"")), "{text}");
    }
    stream_row(&scratch.pool, |row| row.closed_by.is_none() && row.sim_ms == 20_000).await;
    back.send(RunnerMessage::Ended(ended(30_000))).await;
    let last = ViewerMessage::read(&next_text(&mut watching).await.expect("the ending")).unwrap();
    assert!(matches!(last, ViewerMessage::Ended(_)), "{last:?}");
    assert!(next_text(&mut watching).await.is_none(), "the viewer's socket closes behind the ending");
    let row = stream_row(&scratch.pool, |row| row.closed_by.as_deref() == Some("runner")).await;
    assert_eq!(row.sim_ms, 30_000, "the ending's simtime");
    scratch.close().await;
}
