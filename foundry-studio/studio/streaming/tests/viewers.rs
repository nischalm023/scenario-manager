//! A viewer's socket on the streaming server's external port, against the database: it watches only with a live
//! ticket for that very attempt; attached mid-run it gets the stream so far, the newest progress standing for every
//! one before it, and then the live messages; one attached after the end gets the whole replay and its socket closes;
//! and one asking for an attempt the server holds no stream of is refused `not-held`.

mod support;

use run_invocation::Token;
use studio_protocol::{RunnerMessage, StreamRefusal, ViewerMessage};

use support::{RUN, Runner, ended, next_text, progress, refusal, server, started, viewer, watched};

#[tokio::test]
async fn only_a_live_ticket_for_the_attempt_watches_it() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let mut unknown = viewer(&server, "no-ticket-at-all").await;
    assert_eq!(refusal(&mut unknown).await, Some(StreamRefusal::TokenInvalid));
    let expired = scratch.ticket(-1).await;
    let mut late = viewer(&server, &expired).await;
    assert_eq!(refusal(&mut late).await, Some(StreamRefusal::TokenInvalid), "an expired ticket");
    let other = Token::mint();
    sqlx::query("INSERT INTO attempts (run_id, n, state, requested_at) VALUES ($1, 2, 'queued', now())")
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO stream_watch_tickets (token_sha256, run_id, n, subject, expires_at) \
         VALUES ($1, $2, 2, 'u-1', now() + interval '1 minute')",
    )
    .bind(other.sha256())
    .bind(RUN)
    .execute(&scratch.pool)
    .await
    .unwrap();
    let mut elsewhere = viewer(&server, other.expose()).await;
    assert_eq!(refusal(&mut elsewhere).await, Some(StreamRefusal::TokenInvalid), "another attempt's ticket");
    let ticket = scratch.ticket(60).await;
    let mut watching = viewer(&server, &ticket).await;
    assert_eq!(watched(&mut watching, 1).await.0, 1, "attached with the started replayed");
    scratch.close().await;
}

#[tokio::test]
async fn a_viewer_gets_the_stream_so_far_then_live_and_a_late_one_the_whole_replay() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    for message in
        [started(), RunnerMessage::Progress(progress(10_000, 1.0)), RunnerMessage::Progress(progress(20_000, 3.0))]
    {
        runner.send(message).await;
    }
    let ticket = scratch.ticket(60).await;
    let mut watching = viewer(&server, &ticket).await;
    let (replayed, seen) = watched(&mut watching, 2).await;
    assert_eq!(replayed, 2, "{seen:?}");
    assert_eq!(seen[1], ViewerMessage::Progress(progress(20_000, 3.0)), "the newest progress stands for the rest");
    runner.send(RunnerMessage::Progress(progress(30_000, 5.0))).await;
    let live = ViewerMessage::read(&next_text(&mut watching).await.expect("a live message")).unwrap();
    assert_eq!(live, ViewerMessage::Progress(progress(30_000, 5.0)), "live, once");
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    let last = ViewerMessage::read(&next_text(&mut watching).await.expect("the ending")).unwrap();
    assert!(matches!(last, ViewerMessage::Ended(_)), "{last:?}");
    assert!(next_text(&mut watching).await.is_none(), "closed behind the ending");
    let mut after = viewer(&server, &ticket).await;
    let (replayed, seen) = watched(&mut after, 3).await;
    assert_eq!(replayed, 3, "started, the newest progress, the ending: {seen:?}");
    assert_eq!(seen[1], ViewerMessage::Progress(progress(30_000, 5.0)));
    assert_eq!(seen[2], ViewerMessage::Ended(ended(30_000)));
    assert!(next_text(&mut after).await.is_none(), "an ended stream's viewer has nothing to follow");
    scratch.close().await;
}

#[tokio::test]
async fn an_attempt_the_server_holds_no_stream_of_is_refused_not_held() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = viewer(&server, &ticket).await;
    assert_eq!(refusal(&mut watching).await, Some(StreamRefusal::NotHeld));
    scratch.close().await;
}
