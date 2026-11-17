//! The picture end to end, against the database (`support`): a fake runner sends its locked segments down the internal
//! port and the whole T is proved: a frames viewer, admitted by its watch ticket, replays from memory mid-run and
//! follows live without a second keyframe; the segments go up to the run-object store and the row says so; each
//! locked segment is an `edge` on the events socket; a late viewer reads the disk copy; a cold attempt is fetched from
//! the store once for two attachers, the final recording once the API names it, even for an attempt no server
//! streamed; the end block travels live, last, and stays in the disk copy and the uploaded segment; frames out of
//! order, too large, or before `started` close the runner with the typed reason; a viewer that stops reading is told
//! it lagged with the instant it reached; a replay after a restart comes from the disk copy; a server restarted mid-run
//! numbers on from its earlier process's segments and never passes its part for the whole run; and the sweeper's
//! three never-rules hold.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use runstore::layout::{AttemptNumber as StoredAttempt, RunId, live_scenario_key, live_segment_key, recording_key};
use serde_json::{Value, json};
use sqlx::Row;
use studio_protocol::{RunnerMessage, StreamRefusal, ViewerMessage};
use studio_streaming::cache::sweep_once;
use studio_streaming::frames::InMemory;
use studio_streaming::listeners::SocketBounds;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use worldview::frames::scalars::{AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit};
use worldview::{Declare, Descriptors, End, Frame, Ms, Payload, Reader, Scenario, Tag, Writer};

use support::{
    BOUNDS, FramesOptions, RUN, Runner, Server, Socket, Stalled, TOKEN, ended, frame, frames_viewer, held_through, key,
    next_raw, next_text, refusal, scenario_frame, server, server_with, stamped, started, stream_row, viewer, watched,
};

/// The next text message as JSON.
async fn reply(socket: &mut Socket) -> Value {
    match next_raw(socket).await {
        Some(Message::Text(text)) => serde_json::from_str(&text).expect("JSON"),
        other => panic!("a text message, not {other:?}"),
    }
}

/// The next binary message's tag and instant.
async fn binary(socket: &mut Socket) -> (Tag, u64) {
    match next_raw(socket).await {
        Some(Message::Binary(bytes)) => stamped(&bytes),
        other => panic!("a binary message, not {other:?}"),
    }
}

/// Every message until the socket closes: the frames' stamps and the text replies.
async fn drain(socket: &mut Socket) -> (Vec<(Tag, u64)>, Vec<Value>) {
    let (mut frames, mut texts) = (Vec::new(), Vec::new());
    while let Some(message) = next_raw(socket).await {
        match message {
            Message::Binary(bytes) => frames.push(stamped(&bytes)),
            Message::Text(text) => texts.push(serde_json::from_str(&text).expect("JSON")),
            Message::Close(_) => break,
            _ => {}
        }
    }
    (frames, texts)
}

/// The next `edge` on an events socket, reading past anything else.
async fn next_edge(socket: &mut Socket) -> u64 {
    loop {
        let text = next_text(socket).await.expect("a message");
        if let Ok(ViewerMessage::Edge { locked_sim_ms, .. }) = ViewerMessage::read(&text) {
            return locked_sim_ms.as_ms();
        }
    }
}

/// A finished recording in memory, and its frames as the runner sends them: scenario, keyframe, three deltas, the
/// index block and the end block.
fn recording() -> Vec<u8> {
    let mut writer = Writer::new(Vec::new());
    let (scenario, _) = Frame::decode_at(&scenario_frame(), 0).unwrap();
    writer.append_scenario(Ms::ZERO, &scenario.contents(0).unwrap()).unwrap();
    writer.append_sections(Tag::Keyframe, Ms::ZERO, &[(1, b"all".as_slice())]).unwrap();
    for at in [0, 10_000, 20_000] {
        writer.append_sections(Tag::Delta, Ms(at), &[(1, b"moved".as_slice())]).unwrap();
    }
    let end = End { stop_reason: "SimTimeLimit".to_owned(), frames_written: 5, ..End::default() };
    writer.finish(Ms(20_000), &end.to_bytes().unwrap()).unwrap()
}

/// Segment 0 of a run down the runner's socket: the scenario, the keyframe at 0 and three deltas.
async fn segment_zero(runner: &mut Runner) {
    for (tag, at) in
        [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 10_000), (Tag::Delta, 20_000)]
    {
        runner.send_frame(frame(tag, at, 64)).await;
    }
}

#[tokio::test]
async fn a_runner_sends_its_locked_picture_which_is_fanned_out_uploaded_recorded_and_replayed_cold() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let ticket = scratch.ticket(60).await;
    let mut events = viewer(&server, &ticket).await;
    assert_eq!(watched(&mut events, 1).await.0, 1, "attached with the started replayed");
    segment_zero(&mut runner).await;
    // The socket goes quiet: the segment counts as whole, goes up, and is said as an edge. The stand-in viewer answers
    // pings only while it is read, and the viewers' door refuses one silent for `silent_after`, so it is let go here
    // and another opened for the ending.
    assert_eq!(next_edge(&mut events).await, 30_000);
    drop(events);
    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    let mut row = None;
    for _ in 0..50 {
        let read = sqlx::query(
            "SELECT scenario_key, segments_uploaded, locked_sim_ms, EXTRACT(EPOCH FROM cached_at)::bigint AS cached_epoch \
             FROM attempt_streams WHERE run_id = $1",
        )
        .bind(RUN)
        .fetch_optional(&scratch.pool)
        .await
        .unwrap();
        if let Some(read) = read.filter(|row| row.get::<i32, _>("segments_uploaded") == 1) {
            row = Some(read);
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    let row = row.expect("the uploads were written down");
    assert_eq!(
        row.get::<Option<String>, _>("scenario_key").as_deref(),
        Some(live_scenario_key(&run, attempt).as_str())
    );
    assert_eq!(row.get::<i64, _>("locked_sim_ms"), 30_000);
    assert!(row.get::<Option<i64>, _>("cached_epoch").is_some(), "the disk copy is written down");
    let store = server.uploader.storage();
    assert!(store.exists(&live_segment_key(&run, attempt, 0)).await.unwrap());

    // Mid-run: the replay from memory, then live without a second keyframe.
    let mut watching = frames_viewer(&server, &ticket, Some(json!(0))).await;
    let attached = reply(&mut watching).await;
    assert_eq!(attached["type"], "attached");
    assert_eq!((attached["from_ms"].as_u64(), attached["live_edge_ms"].as_u64()), (Some(0), Some(20_000)));
    assert_eq!(
        (attached["run"].as_str(), attached["attempt"].as_u64(), attached["id"].as_str()),
        (Some(RUN), Some(1), Some("1"))
    );
    let mut replayed = Vec::new();
    for _ in 0..5 {
        replayed.push(binary(&mut watching).await);
    }
    assert_eq!(
        replayed,
        [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 10_000), (Tag::Delta, 20_000)]
    );
    runner.send_frame(frame(Tag::Keyframe, 30_000, 64)).await;
    runner.send_frame(frame(Tag::Delta, 30_000, 64)).await;
    assert_eq!(binary(&mut watching).await, (Tag::Delta, 30_000), "a later keyframe never travels");
    let mut events = viewer(&server, &ticket).await;
    assert_eq!(watched(&mut events, 1).await.0, 1, "attached with the started replayed");
    runner.send(RunnerMessage::Ended(ended(40_000))).await;
    assert_eq!(next_edge(&mut events).await, 40_000, "the tail's edge, ahead of the ending");
    let last = ViewerMessage::read(&next_text(&mut events).await.expect("the ending")).unwrap();
    assert!(matches!(last, ViewerMessage::Ended(_)), "{last:?}");
    let over = reply(&mut watching).await;
    let no_end_block = (Some("ended"), Some("no-end-block"));
    assert_eq!((over["type"].as_str(), over["outcome"].as_str()), no_end_block, "the runner sent no end block");
    assert!(next_raw(&mut watching).await.is_none_or(|message| matches!(message, Message::Close(_))));
    assert_eq!(runner.refusal().await, None, "the socket closes clean behind the ending");
    let mut back = support::runner(&server, TOKEN).await;
    assert_eq!(refusal(&mut back).await, Some(StreamRefusal::AttemptEnded), "an ended stream takes no runner");

    // Late: the memory is released once the uploads are done, and the live file on disk serves the whole run.
    server.uploader.drain(&key()).await;
    let held = server.streams.frames().get(&key()).expect("still held");
    assert!(matches!(held.attach(), InMemory::Released), "released");
    let mut late = frames_viewer(&server, &ticket, None).await;
    let attached = reply(&mut late).await;
    assert_eq!((attached["from_ms"].as_u64(), attached["live_edge_ms"].as_u64()), (Some(0), Some(30_000)));
    let (sent, texts) = drain(&mut late).await;
    assert_eq!(
        sent,
        [
            (Tag::Scenario, 0),
            (Tag::Keyframe, 0),
            (Tag::Delta, 0),
            (Tag::Delta, 10_000),
            (Tag::Delta, 20_000),
            (Tag::Delta, 30_000)
        ],
        "one keyframe, every delta"
    );
    assert_eq!(texts.last().and_then(|text| text["outcome"].as_str()), Some("no-end-block"), "nor holds the disk copy");
    let opened: Option<i64> =
        sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM last_opened_at)::bigint FROM attempt_streams WHERE run_id = $1")
            .bind(RUN)
            .fetch_one(&scratch.pool)
            .await
            .unwrap();
    assert!(opened.is_some(), "the open is written down");

    // Evicted: two attachers at once are served from one fetch of the scenario block and the segments.
    let folder = server.streams.frames().folder(&key());
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    let (mut first, mut second) =
        (frames_viewer(&server, &ticket, None).await, frames_viewer(&server, &ticket, None).await);
    reply(&mut first).await;
    reply(&mut second).await;
    assert_eq!(drain(&mut first).await.0.len(), 6);
    assert_eq!(drain(&mut second).await.0.len(), 6);
    assert!(folder.join("live.worldview").is_file());

    // Once the API has stored the final recording (`attempts.object_key`, sent by the runner to the API's door),
    // that is what a cold replay fetches, byte for byte, with the end block's words.
    let bytes = recording();
    let object = recording_key(&run, attempt);
    store.put(&object, bytes.clone().into()).await.unwrap();
    sqlx::query("UPDATE attempts SET object_key = $1 WHERE run_id = $2 AND n = 1")
        .bind(&object)
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    let mut from_recording = frames_viewer(&server, &ticket, None).await;
    reply(&mut from_recording).await;
    let (sent, texts) = drain(&mut from_recording).await;
    assert_eq!(sent.len(), 6, "scenario, keyframe, three deltas, the end block: {sent:?}");
    assert_eq!(texts.last().and_then(|text| text["outcome"].as_str()), Some("sim-time-limit"));
    assert_eq!(tokio::fs::read(folder.join("recording.worldview")).await.unwrap(), bytes);

    // A frames viewer is admitted as an events viewer is: by a live ticket for this attempt, and no other way.
    let mut unknown = frames_viewer(&server, "no-ticket-at-all", None).await;
    assert_eq!(refusal(&mut unknown).await, Some(StreamRefusal::TokenInvalid));
    scratch.close().await;
}

#[tokio::test]
async fn a_segment_sent_again_after_a_lost_connection_is_taken_once() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut first = Runner::welcomed(&server).await;
    first.send(started()).await;
    segment_zero(&mut first).await;
    first.send_frame(frame(Tag::Keyframe, 30_000, 64)).await;
    first.send_frame(frame(Tag::Delta, 30_000, 64)).await;
    sleep(Duration::from_millis(1_200)).await;
    drop(first);
    // The runner comes back and, as its stream module does, says started, the scenario block and the lost segment.
    let mut again = Runner::welcomed(&server).await;
    again.send(started()).await;
    again.send_frame(frame(Tag::Scenario, 0, 64)).await;
    for (tag, at) in [(Tag::Keyframe, 30_000), (Tag::Delta, 30_000), (Tag::Delta, 40_000), (Tag::Delta, 50_000)] {
        again.send_frame(frame(tag, at, 64)).await;
    }
    again.send(RunnerMessage::Ended(ended(60_000))).await;
    assert_eq!(again.refusal().await, None, "nothing was refused");
    server.uploader.drain(&key()).await;
    let ticket = scratch.ticket(60).await;
    let mut late = frames_viewer(&server, &ticket, None).await;
    reply(&mut late).await;
    let (sent, _) = drain(&mut late).await;
    assert_eq!(
        sent,
        [
            (Tag::Scenario, 0),
            (Tag::Keyframe, 0),
            (Tag::Delta, 0),
            (Tag::Delta, 10_000),
            (Tag::Delta, 20_000),
            (Tag::Delta, 30_000),
            (Tag::Delta, 40_000),
            (Tag::Delta, 50_000)
        ],
        "segment 1 once, whole"
    );
    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    let store = server.uploader.storage();
    assert!(store.exists(&live_segment_key(&run, attempt, 1)).await.unwrap());
    assert!(!store.exists(&live_segment_key(&run, attempt, 2)).await.unwrap(), "no third segment");
    scratch.close().await;
}

/// A metric first declared in a keyframe's slice is declared ahead of that keyframe, here after segment 0 closed on
/// the quiet, so the declaration opens segment 1. The connection is lost in that segment and the runner sends it again
/// from its keyframe: the stored copy, the scenario block and the uploaded segments in order as a cold fetch copies
/// them, declares that metric once, after every metric the scenario names.
#[tokio::test]
async fn a_declaration_ahead_of_a_keyframe_survives_a_lost_connection_into_the_stored_copy() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut first = Runner::welcomed(&server).await;
    first.send(started()).await;
    segment_zero(&mut first).await;
    sleep(Duration::from_millis(1_200)).await;
    let metric = MetricDescriptor {
        name: "declared_at_the_keyframe".to_owned(),
        label: "Declared at the keyframe".to_owned(),
        description: String::new(),
        semantics: MetricSemantics {
            unit: MetricUnit::Count,
            denominator_description: None,
            aggregation: AggregationClass::Additive,
        },
    };
    let declared = Declare::Metric(metric).to_bytes().expect("a declaration encodes");
    first.send_frame(Frame::encode(Tag::Declare, Ms(30_000), &declared, true).expect("a frame encodes")).await;
    first.send_frame(frame(Tag::Keyframe, 30_000, 64)).await;
    first.send_frame(frame(Tag::Delta, 30_000, 64)).await;
    sleep(Duration::from_millis(1_200)).await;
    drop(first);
    let mut again = Runner::welcomed(&server).await;
    again.send(started()).await;
    again.send_frame(frame(Tag::Scenario, 0, 64)).await;
    for (tag, at) in [(Tag::Keyframe, 30_000), (Tag::Delta, 30_000), (Tag::Delta, 40_000)] {
        again.send_frame(frame(tag, at, 64)).await;
    }
    again.send(RunnerMessage::Ended(ended(50_000))).await;
    assert_eq!(again.refusal().await, None, "nothing was refused");
    server.uploader.drain(&key()).await;
    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    let store = server.uploader.storage();
    let mut copy = store.get(&live_scenario_key(&run, attempt)).await.expect("the scenario block").to_vec();
    for segment in 0..2 {
        copy.extend(store.get(&live_segment_key(&run, attempt, segment)).await.expect("a segment"));
    }
    let mut reader = Reader::from_bytes(&copy).expect("the stored copy reads");
    let mut descriptors = Descriptors::from_scenario(&Scenario::read_from(&mut reader).expect("its scenario block"));
    let named = descriptors.metrics.len();
    let records = reader.index().records().to_vec();
    for record in records.iter().filter(|record| record.tag == Tag::Declare) {
        descriptors.push(&reader.payload_at::<Declare>(record).expect("a declaration reads"));
    }
    let names: Vec<&str> = descriptors.metrics.iter().map(|metric| metric.name.as_str()).collect();
    assert_eq!(names.len(), named + 1, "every metric the scenario names, then the one declared at 30 s, once");
    assert_eq!(names.last(), Some(&"declared_at_the_keyframe"));
    scratch.close().await;
}

#[tokio::test]
async fn frames_before_started_out_of_order_or_too_large_close_the_runner_with_the_reason() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut early = Runner::welcomed(&server).await;
    early.send_frame(frame(Tag::Scenario, 0, 16)).await;
    assert_eq!(early.refusal().await, Some(StreamRefusal::Binary), "before started");

    let mut headless = Runner::welcomed(&server).await;
    headless.send(started()).await;
    headless.send_frame(frame(Tag::Declare, 0, 16)).await;
    assert_eq!(headless.refusal().await, Some(StreamRefusal::FrameOrder), "a frame before the scenario block");

    let mut garbled = Runner::welcomed(&server).await;
    garbled.send(started()).await;
    garbled.send_frame(b"not a container frame at all".to_vec()).await;
    assert_eq!(garbled.refusal().await, Some(StreamRefusal::FrameOrder), "it does not decode");

    let mut orderless = Runner::welcomed(&server).await;
    orderless.send(started()).await;
    orderless.send_frame(frame(Tag::Scenario, 0, 16)).await;
    orderless.send_frame(frame(Tag::Keyframe, 10_000, 16)).await;
    orderless.send_frame(frame(Tag::Delta, 5_000, 16)).await;
    assert_eq!(orderless.refusal().await, Some(StreamRefusal::FrameOrder), "simtime ran backwards");
    scratch.close().await;

    let scratch = scratch_or_skip!();
    // Room for the scenario block and a little more, so the keyframe is what passes the bound.
    let small = FramesOptions { open_max_bytes: scenario_frame().len() as u64 + 200, ..FramesOptions::default() };
    let server = server_with(&scratch.pool, BOUNDS, small).await;
    let mut big = Runner::welcomed(&server).await;
    big.send(started()).await;
    big.send_frame(frame(Tag::Scenario, 0, 16)).await;
    let mut seed = 3u64;
    let body: Vec<u8> = (0..4_096)
        .map(|_| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 56) as u8
        })
        .collect();
    big.send_frame(Frame::encode(Tag::Keyframe, Ms::ZERO, &body, true).unwrap()).await;
    assert_eq!(big.refusal().await, Some(StreamRefusal::FrameTooLarge));
    scratch.close().await;
}

#[tokio::test]
async fn a_frames_viewer_that_stops_reading_is_told_it_lagged_with_the_instant_it_reached() {
    let scratch = scratch_or_skip!();
    let slow = SocketBounds { write_within: Duration::from_millis(300), ..BOUNDS };
    let server = server_with(&scratch.pool, slow, FramesOptions::default()).await;
    let frames = server.streams.frames().open(&key());
    frames.push(frame(Tag::Scenario, 0, 64)).await.unwrap();
    frames.push(frame(Tag::Keyframe, 0, 64)).await.unwrap();
    // Incompressible bodies, so the frames are as large on the wire as in memory.
    let mut seed = 7u64;
    for n in 0..300u64 {
        let body: Vec<u8> = (0..65_536)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                (seed >> 56) as u8
            })
            .collect();
        frames.push(Frame::encode(Tag::Delta, Ms(n * 10_000), &body, true).unwrap()).await.unwrap();
    }
    let ticket = scratch.ticket(60).await;
    let mut stalled = frames_viewer(&server, &ticket, Some(json!(0))).await;
    sleep(Duration::from_millis(1_500)).await;
    let (sent, texts) = drain(&mut stalled).await;
    let lagged = texts.iter().find(|text| text["type"] == "lagged").expect("told it lagged");
    let resume = lagged["resume_from_ms"].as_u64().expect("an instant");
    let last = sent.last().expect("some frames came through").1;
    assert!(resume <= last && sent.iter().any(|(_, at)| *at == resume), "resume at {resume}, last {last}");
    assert!(sent.len() < 302, "the whole run did not go through a stalled socket");
    scratch.close().await;
}

/// The disk copy gone, a viewer's attach fetches the live segments onto disk and is served from there, `live` the
/// newest keyframe; once the final recording is stored, a cold attach fetches it, byte for byte, with the end block's
/// words.
#[tokio::test]
async fn a_cold_attempt_is_fetched_from_the_store_onto_disk_and_served_from_there() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    segment_zero(&mut runner).await;
    sleep(Duration::from_millis(1_200)).await;
    runner.send_frame(frame(Tag::Keyframe, 30_000, 64)).await;
    runner.send_frame(frame(Tag::Delta, 30_000, 64)).await;
    runner.send(RunnerMessage::Ended(ended(40_000))).await;
    assert_eq!(runner.refusal().await, None);
    server.uploader.drain(&key()).await;
    let store = server.uploader.storage();
    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    assert!(store.exists(&live_segment_key(&run, attempt, 1)).await.unwrap(), "segment 1 went up at the end");

    let folder = server.streams.frames().folder(&key());
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    let ticket = scratch.ticket(60).await;
    let mut watching = frames_viewer(&server, &ticket, Some(json!("live"))).await;
    let attached = reply(&mut watching).await;
    assert_eq!(attached["from_ms"].as_u64(), Some(30_000), "live on a fetched copy is the newest keyframe");
    let (sent, texts) = drain(&mut watching).await;
    assert_eq!(sent, [(Tag::Scenario, 0), (Tag::Keyframe, 30_000), (Tag::Delta, 30_000)]);
    assert_eq!(texts.last().map(|text| text["type"].as_str()), Some(Some("ended")));
    let mut reader = Reader::open(&folder.join("live.worldview")).expect("fetched into place");
    assert_eq!(reader.frames_from(0).count(), 7);

    let bytes = recording();
    store.put(&recording_key(&run, attempt), bytes.clone().into()).await.unwrap();
    sqlx::query("UPDATE attempts SET object_key = $1 WHERE run_id = $2 AND n = 1")
        .bind(recording_key(&run, attempt))
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    let mut replaying = frames_viewer(&server, &ticket, None).await;
    reply(&mut replaying).await;
    let (sent, texts) = drain(&mut replaying).await;
    assert_eq!(
        sent,
        [
            (Tag::Scenario, 0),
            (Tag::Keyframe, 0),
            (Tag::Delta, 0),
            (Tag::Delta, 10_000),
            (Tag::Delta, 20_000),
            (Tag::End, 20_000)
        ]
    );
    assert_eq!(texts.last().and_then(|text| text["outcome"].as_str()), Some("sim-time-limit"), "the end block's");
    assert_eq!(tokio::fs::read(folder.join("recording.worldview")).await.unwrap(), bytes);
    scratch.close().await;
}

/// The runner sends the end block after its last delta: a viewer following live is sent it, then `ended` in the end
/// block's own words, and the disk copy and the uploaded last segment keep it, so a late viewer reads the same.
#[tokio::test]
async fn the_end_block_travels_live_and_stays_in_the_disk_copy_and_the_store() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    segment_zero(&mut runner).await;
    held_through(&server, 20_000).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = frames_viewer(&server, &ticket, Some(json!(0))).await;
    reply(&mut watching).await;
    for _ in 0..5 {
        binary(&mut watching).await;
    }
    let end = End { stop_reason: "Complete".to_owned(), frames_written: 6, ..End::default() };
    runner.send_frame(Frame::encode(Tag::End, Ms(25_000), &end.to_bytes().unwrap(), true).unwrap()).await;
    assert_eq!(binary(&mut watching).await, (Tag::End, 25_000), "the end block, live");
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    let over = reply(&mut watching).await;
    assert_eq!((over["type"].as_str(), over["outcome"].as_str()), (Some("ended"), Some("complete")));
    assert_eq!(runner.refusal().await, None, "taken, and nothing refused");

    server.uploader.drain(&key()).await;
    let mut late = frames_viewer(&server, &ticket, None).await;
    reply(&mut late).await;
    let (sent, texts) = drain(&mut late).await;
    assert_eq!(sent.last(), Some(&(Tag::End, 25_000)), "kept in the disk copy: {sent:?}");
    assert_eq!(texts.last().and_then(|text| text["outcome"].as_str()), Some("complete"));
    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    let uploaded = server.uploader.storage().get(&live_segment_key(&run, attempt, 0)).await.unwrap();
    let mut tags = Vec::new();
    let mut at = 0;
    while at < uploaded.len() {
        let (frame, length) = Frame::decode_at(&uploaded[at..], 0).expect("a frame");
        tags.push(frame.header.tag);
        at += length;
    }
    assert_eq!(tags.last(), Some(&Tag::End), "uploaded with its segment: {tags:?}");
    scratch.close().await;
}

/// An end block whose stop reason no outcome names, as a later simulator's might, ends the picture with the reason
/// in place of `ended`: every frame goes out, the end block among them, and the viewer is told the copy does not say
/// how the run ended.
#[tokio::test]
async fn an_end_block_whose_stop_reason_no_outcome_names_ends_the_picture_with_why() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    segment_zero(&mut runner).await;
    let end = End { stop_reason: "Paused".to_owned(), frames_written: 6, ..End::default() };
    runner.send_frame(Frame::encode(Tag::End, Ms(25_000), &end.to_bytes().unwrap(), true).unwrap()).await;
    // Held before the ending, which lets the memory go once the uploads are done.
    held_through(&server, 25_000).await;
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = frames_viewer(&server, &ticket, None).await;
    reply(&mut watching).await;
    let (sent, texts) = drain(&mut watching).await;
    assert_eq!(sent.last(), Some(&(Tag::End, 25_000)), "every frame, the end block among them: {sent:?}");
    let last = texts.last().expect("a last word");
    assert_eq!(last["type"], "error", "{texts:?}");
    assert!(last["message"].as_str().is_some_and(|message| message.contains("Paused")), "{last}");
    scratch.close().await;
}

/// An attempt whose final recording the API stored, though no streaming server ever streamed it (no
/// `attempt_streams` row), is fetched from the store and served whole.
#[tokio::test]
async fn a_stored_recording_no_server_streamed_is_served_from_the_store() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    server.uploader.storage().put(&recording_key(&run, attempt), recording().into()).await.unwrap();
    sqlx::query("UPDATE attempts SET object_key = $1 WHERE run_id = $2 AND n = 1")
        .bind(recording_key(&run, attempt))
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    let ticket = scratch.ticket(60).await;
    let mut watching = frames_viewer(&server, &ticket, None).await;
    assert_eq!(reply(&mut watching).await["type"], "attached");
    let (sent, texts) = drain(&mut watching).await;
    assert_eq!(sent.len(), 6, "scenario, keyframe, three deltas, the end block: {sent:?}");
    assert_eq!(texts.last().and_then(|text| text["outcome"].as_str()), Some("sim-time-limit"));
    scratch.close().await;
}

#[tokio::test]
async fn a_replay_after_a_restart_comes_from_the_disk_copy() {
    let scratch = scratch_or_skip!();
    let options = FramesOptions::default();
    let dir = options.dir.clone();
    let first = server_with(&scratch.pool, BOUNDS, options).await;
    let mut runner = Runner::welcomed(&first).await;
    runner.send(started()).await;
    for (tag, at) in [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 10_000)] {
        runner.send_frame(frame(tag, at, 64)).await;
    }
    runner.send(RunnerMessage::Ended(ended(20_000))).await;
    stream_row(&scratch.pool, |row| row.closed_by.as_deref() == Some("runner")).await;
    first.uploader.drain(&key()).await;

    // Another server over the same cache folder knows nothing of the attempt but finds its file.
    let second = server_with(&scratch.pool, BOUNDS, FramesOptions { dir, ..FramesOptions::default() }).await;
    assert!(second.streams.frames().get(&key()).is_none());
    let ticket = scratch.ticket(60).await;
    let mut late = frames_viewer(&second, &ticket, Some(json!(10_000))).await;
    let attached = reply(&mut late).await;
    assert_eq!((attached["from_ms"].as_u64(), attached["live_edge_ms"].as_u64()), (Some(10_000), Some(10_000)));
    let (sent, texts) = drain(&mut late).await;
    assert_eq!(sent, [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 10_000)]);
    assert_eq!(texts.last().map(|text| text["type"].as_str()), Some(Some("ended")), "closed by the file's end");
    scratch.close().await;
}

/// A streaming server restarted while a run goes, over the same cache and store, takes the attempt up after its earlier
/// process from the segment the runner sends again: it numbers on from the segments already in the store, starting
/// over the one that segment falls inside, rather than overwrite them. It holds only what was sent to it, so its copy
/// ends `recording-incomplete` live and from its disk, and its uploads are never said to have finished: a cold read of
/// the segments, the whole history in order, ends so too, until the final recording is stored and read whole.
#[tokio::test]
async fn a_server_restarted_mid_run_numbers_on_and_never_passes_its_part_for_the_whole_run() {
    let scratch = scratch_or_skip!();
    let options = FramesOptions::default();
    let dir = options.dir.clone();
    let first = server_with(&scratch.pool, BOUNDS, options).await;
    let mut runner = Runner::welcomed(&first).await;
    runner.send(started()).await;
    segment_zero(&mut runner).await;
    sleep(Duration::from_millis(1_200)).await;
    runner.send_frame(frame(Tag::Keyframe, 30_000, 64)).await;
    runner.send_frame(frame(Tag::Delta, 30_000, 64)).await;
    sleep(Duration::from_millis(1_200)).await;
    first.uploader.drain(&key()).await;

    // The server's process stops; a new one over the same folders knows nothing of the attempt until its runner comes
    // back and, as its stream module does, says started, the scenario block and its open segment again.
    let second = server_with(&scratch.pool, BOUNDS, FramesOptions { dir, ..FramesOptions::default() }).await;
    let mut again = Runner::welcomed(&second).await;
    again.send(started()).await;
    for (tag, at) in [(Tag::Scenario, 0), (Tag::Keyframe, 30_000), (Tag::Delta, 30_000), (Tag::Delta, 40_000)] {
        again.send_frame(frame(tag, at, 64)).await;
    }
    held_through(&second, 40_000).await;
    let ticket = scratch.ticket(60).await;
    let mut live = frames_viewer(&second, &ticket, None).await;
    reply(&mut live).await;
    let end = End { stop_reason: "SimTimeLimit".to_owned(), frames_written: 9, ..End::default() };
    again.send_frame(Frame::encode(Tag::End, Ms(45_000), &end.to_bytes().unwrap(), true).unwrap()).await;
    again.send(RunnerMessage::Ended(ended(50_000))).await;
    assert_eq!(again.refusal().await, None, "nothing was refused");
    let (sent, texts) = drain(&mut live).await;
    let taken_up =
        [(Tag::Scenario, 0), (Tag::Keyframe, 30_000), (Tag::Delta, 30_000), (Tag::Delta, 40_000), (Tag::End, 45_000)];
    assert_eq!(sent, taken_up, "what the new process holds");
    assert_eq!(texts.last().and_then(|text| text["outcome"].as_str()), Some("recording-incomplete"), "live");
    second.uploader.drain(&key()).await;

    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    let store = second.uploader.storage();
    let stored = |segment| live_segment_key(&run, attempt, segment);
    let segment_0 = [(Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 10_000), (Tag::Delta, 20_000)];
    assert_eq!(tags_of(&store.get(&stored(0)).await.unwrap()), segment_0, "the earlier process's, not overwritten");
    assert_eq!(tags_of(&store.get(&stored(1)).await.unwrap()), taken_up[1..], "started over whole");
    assert!(!store.exists(&stored(2)).await.unwrap(), "no third segment");
    let row = sqlx::query(
        "SELECT segments_uploaded, uploads_finished_at IS NOT NULL AS finished FROM attempt_streams WHERE run_id = $1",
    )
    .bind(RUN)
    .fetch_one(&scratch.pool)
    .await
    .unwrap();
    assert_eq!((row.get::<i32, _>("segments_uploaded"), row.get::<bool, _>("finished")), (2, false));

    let incomplete = Some("recording-incomplete".to_owned());
    assert_eq!(replayed(&second, &ticket).await, (taken_up.to_vec(), incomplete.clone()), "from its own disk");
    tokio::fs::remove_dir_all(second.streams.frames().folder(&key())).await.unwrap();
    // One keyframe goes to a viewer, the deltas carrying it forward.
    let whole_history = [&taken_up[..1], &segment_0, &taken_up[2..]].concat();
    assert_eq!(replayed(&second, &ticket).await, (whole_history, incomplete), "the segments, cold");
    store.put(&recording_key(&run, attempt), recording().into()).await.unwrap();
    sqlx::query("UPDATE attempts SET object_key = $1 WHERE run_id = $2 AND n = 1")
        .bind(recording_key(&run, attempt))
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    let recorded = replayed(&second, &ticket).await.1;
    assert_eq!(recorded.as_deref(), Some("sim-time-limit"), "the final recording, in their place");
    scratch.close().await;
}

/// A frames viewer's whole replay of the attempt: the frames' stamps, and the outcome its `ended` says.
async fn replayed(server: &Server, ticket: &str) -> (Vec<(Tag, u64)>, Option<String>) {
    let mut socket = frames_viewer(server, ticket, None).await;
    reply(&mut socket).await;
    let (sent, texts) = drain(&mut socket).await;
    (sent, texts.last().and_then(|text| text["outcome"].as_str().map(str::to_owned)))
}

/// The stamps of the frames an uploaded object holds, in order.
fn tags_of(object: &[u8]) -> Vec<(Tag, u64)> {
    let mut stamps = Vec::new();
    let mut at = 0;
    while at < object.len() {
        let (frame, length) = Frame::decode_at(&object[at..], 0).expect("a frame");
        stamps.push((frame.header.tag, frame.header.t.0));
        at += length;
    }
    stamps
}

#[tokio::test]
async fn the_sweeper_spares_an_open_attempt_one_not_yet_uploaded_and_one_being_read_and_clears_the_rest() {
    let scratch = scratch_or_skip!();
    let mut options = FramesOptions::default();
    let stalled = Arc::new(Stalled::over(&options.objects()));
    let held = stalled.held.clone();
    options.store = Some(stalled);
    let server = server_with(&scratch.pool, BOUNDS, options).await;
    let streams = server.streams.clone();
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    for (tag, at) in [(Tag::Scenario, 0), (Tag::Keyframe, 0), (Tag::Delta, 0)] {
        runner.send_frame(frame(tag, at, 64)).await;
    }
    sleep(Duration::from_millis(1_500)).await;
    let folder = streams.frames().folder(&key());
    assert!(folder.join("live.worldview").exists());
    let provider = &server.provider;
    assert_eq!(
        sweep_once(&scratch.pool, &streams, provider, Duration::ZERO, 0).await,
        0,
        "an open attempt is never swept"
    );
    runner.send(RunnerMessage::Ended(ended(10_000))).await;
    sleep(Duration::from_millis(300)).await;
    assert!(server.uploader.pending(&key()) > 0, "the puts are held");
    let pending = sweep_once(&scratch.pool, &streams, provider, Duration::ZERO, 0).await;
    assert_eq!(pending, 0, "nor one whose uploads are not done");
    assert!(folder.join("live.worldview").exists());
    held.store(false, Ordering::Relaxed);
    server.uploader.drain(&key()).await;
    let young = sweep_once(&scratch.pool, &streams, provider, Duration::from_secs(3_600), u64::MAX).await;
    assert_eq!(young, 0, "young and small");
    // A reading of the copy holds it: the sweeper leaves it until the reading is dropped.
    let reading = provider.open(&key(), Ok(None)).await.expect("the disk copy");
    assert_eq!(sweep_once(&scratch.pool, &streams, provider, Duration::ZERO, u64::MAX).await, 0, "nor one being read");
    drop(reading);
    assert_eq!(sweep_once(&scratch.pool, &streams, provider, Duration::ZERO, u64::MAX).await, 1, "past the TTL");
    assert!(!folder.exists());
    let cached: Option<i64> =
        sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM cached_at)::bigint FROM attempt_streams WHERE run_id = $1")
            .bind(RUN)
            .fetch_one(&scratch.pool)
            .await
            .unwrap();
    assert!(cached.is_none(), "cached_at is cleared");
    scratch.close().await;
}
