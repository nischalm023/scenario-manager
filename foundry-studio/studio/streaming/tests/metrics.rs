//! One metric of an attempt on the external port, against the database (`support`): a fake runner sends a real
//! recording's frames, its end block last, and metric viewers admitted by their watch ticket get the descriptor, the
//! buckets so far, each later bucket live, then `ended`. The end-block figures and the whole-run tables come out the
//! same live, from the disk copy and from the store, and so do the figures and stop reason of a run stopped before its
//! first slice; the viewers of one metric share one hub, which goes once the last
//! has left; a cold attempt is fetched once for every viewer, the metric viewer that started the fetch attaching
//! before it is done; a failed fetch refuses each viewer and ends none; and a metric the server does not compute, a bad
//! ticket, another format, an attempt with nothing to read yet, trailing windows the recording does not cut, and a
//! copy the disk will not open or that is not a recording are refused with the protocol's reasons.

mod support;

use std::fs::{File, Permissions, set_permissions, write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use runstore::layout::{AttemptNumber as StoredAttempt, RunId, recording_key};
use serde_json::{Value, json};
use studio_metrics::{ReportSeries, SummarySeries};
use studio_protocol::{
    FileFormat, METRIC_STREAM_FORMAT, MetricMessage, MetricPoint, MetricRequest, RunOutcome, RunnerMessage,
    StreamRefusal,
};
use studio_streaming::frames::LIVE_FILE;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use worldview::container::reader::frame_at;
use worldview::{Reader, Scenario, Tag};

use support::gated::Gated;
use support::{
    BOUNDS, FramesOptions, RUN, Runner, Server, Socket, ended, frames_viewer, held_through, key, next_raw, next_text,
    refusal, send, server, server_with, stamped, started,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/tests/fixtures/minimal_easyloop_flow/recording.worldview")
}

/// The viewer's fixture recording as a runner sends it: every frame but the index block, its end block last; how
/// many deltas are among them, and its slice width in milliseconds.
fn recording() -> (Vec<Vec<u8>>, usize, u64) {
    frames_until(|_| false)
}

/// The fixture's first `count` segments as the runner sends them: its frames up to keyframe `count + 1`.
fn segments(count: usize) -> (Vec<Vec<u8>>, usize, u64) {
    let mut keyframes = 0;
    frames_until(|tag| {
        keyframes += usize::from(tag == Tag::Keyframe);
        keyframes == count + 1
    })
}

fn frames_until(mut stop: impl FnMut(Tag) -> bool) -> (Vec<Vec<u8>>, usize, u64) {
    let mut reader = Reader::open(&fixture()).expect("the fixture opens");
    let width = Scenario::read_from(&mut reader).expect("the scenario block").slice_width.as_ms();
    let records = reader.index().records().to_vec();
    let mut file = File::open(fixture()).expect("the fixture reads");
    let (mut frames, mut deltas) = (Vec::new(), 0);
    for record in records.into_iter().filter(|record| record.tag != Tag::Index) {
        if stop(record.tag) {
            break;
        }
        deltas += usize::from(record.tag == Tag::Delta);
        frames.push(frame_at(&mut file, record.offset).expect("a frame reads").to_bytes());
    }
    (frames, deltas, width)
}

async fn metrics_viewer(server: &Server, ticket: &str, metric: &str, trailing: Option<Value>) -> Socket {
    let url = format!("ws://{}/metrics/{RUN}/1", server.external);
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("the socket opens");
    let mut attach = json!({ "type": "attach", "v": METRIC_STREAM_FORMAT, "metric": metric, "token": ticket });
    if let Some(trailing) = trailing {
        attach["trailing"] = trailing;
    }
    send(&mut socket, attach.to_string()).await;
    socket
}

async fn next_message(socket: &mut Socket) -> MetricMessage {
    let text = next_text(socket).await.expect("a metric message");
    MetricMessage::read(&text).expect("a metric message reads")
}

/// Every row until something other than `rows` comes, which is returned with them.
async fn rows_until(socket: &mut Socket) -> (Vec<MetricPoint>, Option<MetricMessage>) {
    let mut rows = Vec::new();
    loop {
        match next_raw(socket).await {
            Some(Message::Text(text)) => match MetricMessage::read(&text).expect("reads") {
                MetricMessage::Rows { rows: batch, .. } => rows.extend(batch),
                other => return (rows, Some(other)),
            },
            _ => return (rows, None),
        }
    }
}

/// The attempt's whole run of `metric` after the run: `attached`, every row, then `ended`.
async fn whole(server: &Server, ticket: &str, metric: &str) -> Vec<MetricPoint> {
    let mut socket = metrics_viewer(server, ticket, metric, None).await;
    let attached = next_message(&mut socket).await;
    assert!(matches!(attached, MetricMessage::Attached { live: false, .. }), "{attached:?}");
    let (rows, last) = rows_until(&mut socket).await;
    assert!(matches!(last, Some(MetricMessage::Ended { .. })), "{metric}: {last:?}");
    rows
}

/// An open attempt: the buckets of the frames held so far, then each later bucket as its frame comes, then `ended`.
#[tokio::test]
async fn a_metric_viewer_gets_the_buckets_so_far_then_each_live_then_ended() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let (frames, deltas, width) = segments(1);
    assert!(deltas >= 3, "the fixture's first segment holds {deltas} deltas");
    let (early, late) = frames.split_at(frames.len() - 1);
    for frame in early {
        runner.send_frame(frame.clone()).await;
    }
    held_through(&server, (deltas as u64 - 2) * width).await;
    let ticket = scratch.ticket(60).await;
    let mut watching = metrics_viewer(&server, &ticket, "picks", None).await;
    let MetricMessage::Attached { descriptor, reached_sim_ms, live, .. } = next_message(&mut watching).await else {
        panic!("attached first")
    };
    assert_eq!((descriptor.metric.as_str(), descriptor.label.as_str()), ("picks", "Picks"));
    assert!(live, "the run goes");
    // The newest slice taken waits for the next frame to say where it ends.
    assert_eq!(reached_sim_ms.as_ms(), (deltas as u64 - 2) * width, "the buckets so far reach the last delta's start");
    let mut history = Vec::new();
    while history.len() < deltas - 2 {
        let MetricMessage::Rows { rows, .. } = next_message(&mut watching).await else { panic!("rows") };
        history.extend(rows);
    }
    assert_eq!(history.len(), deltas - 2);
    assert!(history.windows(2).all(|pair| pair[0].to_sim_ms < pair[1].to_sim_ms), "rising");
    assert!(history.iter().all(|row| row.value.is_some()), "a count is stated in every slice");
    runner.send_frame(late[0].clone()).await;
    let MetricMessage::Rows { rows, .. } = next_message(&mut watching).await else { panic!("the live bucket") };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].from_sim_ms, history.last().unwrap().to_sim_ms, "the next slice");
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    // With no end block to say where the last slice ends, the copy's end states it whole.
    let MetricMessage::Rows { rows: last, .. } = next_message(&mut watching).await else { panic!("the last bucket") };
    assert_eq!((last.len(), last[0].to_sim_ms.as_ms()), (1, deltas as u64 * width), "the last slice, whole");
    let MetricMessage::Ended { outcome, .. } = next_message(&mut watching).await else { panic!("ended") };
    assert_eq!(outcome, RunOutcome::NoEndBlock, "the copy carries no end block");
    assert!(next_text(&mut watching).await.is_none(), "closed behind the ending");
    scratch.close().await;
}

/// After the end, the disk copy answers whole and says it is not live; a cumulative series' buckets span the run from
/// its start, and a trailing window asked for comes at its step.
#[tokio::test]
async fn a_viewer_after_the_end_reads_the_disk_copy_whole() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let (frames, deltas, _) = segments(1);
    for frame in &frames {
        runner.send_frame(frame.clone()).await;
    }
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    assert_eq!(runner.refusal().await, None);
    server.uploader.drain(&key()).await;
    let ticket = scratch.ticket(60).await;
    let rows = whole(&server, &ticket, "cumulative_picks").await;
    assert_eq!(rows.len(), deltas);
    assert!(rows.iter().all(|row| row.from_sim_ms.as_ms() == 0), "a cumulative bucket spans the run");
    assert!(rows.windows(2).all(|pair| pair[0].value <= pair[1].value), "a running total never falls");
    let trailing = json!({ "window_sim_ms": 20_000, "step_sim_ms": 20_000 });
    let mut windowed = metrics_viewer(&server, &ticket, "picks", Some(trailing)).await;
    let MetricMessage::Attached { descriptor, .. } = next_message(&mut windowed).await else { panic!("attached") };
    assert_eq!(serde_json::to_value(descriptor.bucket).unwrap()["kind"], "trailing");
    let (rows, last) = rows_until(&mut windowed).await;
    assert!(!rows.is_empty() && matches!(last, Some(MetricMessage::Ended { .. })), "{last:?}");
    let spans: Vec<(u64, u64)> = rows.iter().map(|row| (row.from_sim_ms.as_ms(), row.to_sim_ms.as_ms())).collect();
    assert!(spans.iter().all(|(from, to)| to % 20_000 == 0 && to - from == 20_000), "{spans:?}");
    scratch.close().await;
}

/// The runner sends its end block last: every end-of-run figure and every report series, its whole-run table
/// included, comes out the same live, from the disk copy, from the live segments fetched cold, and from the final
/// recording fetched cold.
#[tokio::test]
async fn the_end_of_the_run_reads_the_same_live_from_disk_and_from_the_store() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let (frames, _, _) = recording();
    let (end, before) = frames.split_last().expect("frames");
    assert_eq!(stamped(end).0, Tag::End, "the runner sends the end block last");
    for frame in before {
        runner.send_frame(frame.clone()).await;
    }
    held_through(&server, 600_000).await;
    let ticket = scratch.ticket(60).await;
    let figures = SummarySeries::ALL.map(SummarySeries::name);
    let names: Vec<&str> = figures.iter().copied().chain(ReportSeries::ALL.map(ReportSeries::name)).collect();
    let mut viewers = Vec::new();
    for name in &names {
        let mut socket = metrics_viewer(&server, &ticket, name, None).await;
        let attached = next_message(&mut socket).await;
        assert!(matches!(attached, MetricMessage::Attached { live: true, .. }), "{name}: {attached:?}");
        viewers.push(socket);
    }
    // The end block goes out on the live stream, ahead of the runner's ending.
    runner.send_frame(end.clone()).await;
    runner.send(RunnerMessage::Ended(ended(600_000))).await;
    let mut live = Vec::new();
    for (name, socket) in names.iter().zip(&mut viewers) {
        let (rows, last) = rows_until(socket).await;
        let ended = matches!(&last, Some(MetricMessage::Ended { outcome, .. }) if *outcome == RunOutcome::SimTimeLimit);
        assert!(ended, "{name}: {last:?}");
        live.push(rows);
    }
    // The fixture's run extracts nothing, assigns no destination and completes no DC order: the extraction series state
    // the warehouse's none, as desvis's Order extraction tab does, and the other two are empty, in every copy.
    let extracted_nothing = ["extractions_by_station", "extracted_units_by_station", "extractions_per_hour_by_station"];
    let unrecorded = ["presentations_by_destination", "orders_by_dc_station"];
    for (name, rows) in names.iter().zip(&live) {
        let whole_run = rows.iter().filter(|row| row.whole_run).count();
        assert!(rows.iter().filter(|row| row.whole_run).all(|row| row.from_sim_ms.as_ms() == 0), "{name}: from 0");
        if extracted_nothing.contains(name) {
            let stated: Vec<(&[String], Option<f64>)> =
                rows.iter().map(|row| (row.keys.as_slice(), row.value)).collect();
            assert_eq!(stated, [(&["all".to_owned()][..], Some(0.0))], "{name}");
        } else if unrecorded.contains(name) {
            assert!(rows.is_empty(), "{name}: {rows:?}");
        } else if figures.contains(name) {
            assert!(whole_run > 0 && whole_run == rows.len(), "{name}: its figures, each the whole run's: {rows:?}");
        } else {
            assert!(whole_run > 0, "{name}: its whole-run table came live: {rows:?}");
        }
    }
    let picks = &live[names.iter().position(|name| *name == "run_total_picks").expect("a figure")];
    assert!(picks[0].value.is_some_and(|picks| picks > 0.0), "{picks:?}");

    server.uploader.drain(&key()).await;
    same_as_live(&names, &live, &every(&server, &ticket, &names).await, "from the disk copy");

    let folder = server.streams.frames().folder(&key());
    server.streams.frames().forget(&key());
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    same_as_live(&names, &live, &every(&server, &ticket, &names).await, "from the live segments, cold");

    let (run, attempt) = (RunId::new(RUN).unwrap(), StoredAttempt::FIRST);
    let object = recording_key(&run, attempt);
    server.uploader.storage().put(&object, std::fs::read(fixture()).unwrap().into()).await.unwrap();
    sqlx::query("UPDATE attempts SET object_key = $1 WHERE run_id = $2 AND n = 1")
        .bind(&object)
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    same_as_live(&names, &live, &every(&server, &ticket, &names).await, "from the final recording, cold");
    assert!(folder.join("recording.worldview").is_file());
    scratch.close().await;
}

/// A run stopped before its first slice sends its scenario block and then its end block, no keyframe between: its
/// end-of-run figures and its stop reason come out the same live, from the disk copy and from the live segments
/// fetched cold, on the metric streams and in the picture alike.
#[tokio::test]
async fn a_run_stopped_before_its_first_slice_ends_the_same_live_from_disk_and_from_the_store() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let (frames, _, _) = recording();
    let (scenario, end) = (frames[0].clone(), frames.last().expect("frames").clone());
    assert_eq!((stamped(&scenario).0, stamped(&end).0), (Tag::Scenario, Tag::End));
    runner.send_frame(scenario).await;
    held_through(&server, 0).await;
    let ticket = scratch.ticket(60).await;
    let figures = SummarySeries::ALL.map(SummarySeries::name);
    let mut viewers = Vec::new();
    for name in figures {
        let mut socket = metrics_viewer(&server, &ticket, name, None).await;
        let attached = next_message(&mut socket).await;
        assert!(matches!(attached, MetricMessage::Attached { live: true, .. }), "{name}: {attached:?}");
        viewers.push(socket);
    }
    let mut picture = frames_viewer(&server, &ticket, None).await;
    attached_frames(&mut picture).await;
    runner.send_frame(end).await;
    runner.send(RunnerMessage::Ended(ended(0))).await;
    let mut live = Vec::new();
    for socket in &mut viewers {
        live.push(rows_until(socket).await);
    }
    let ended = Some(MetricMessage::Ended { v: METRIC_STREAM_FORMAT, outcome: RunOutcome::SimTimeLimit });
    assert!(live.iter().all(|(_, last)| *last == ended), "{live:?}");
    let picks = &live[figures.iter().position(|name| *name == "run_total_picks").expect("a figure")].0;
    assert!(picks.first().and_then(|row| row.value).is_some_and(|picks| picks > 0.0), "{picks:?}");
    let pictured = |(frames, texts): (usize, Vec<Value>)| (frames, texts.last().map(|text| text["outcome"].clone()));
    let picture_live = pictured(drained(&mut picture).await);
    assert_eq!(picture_live, (2, Some(json!("sim-time-limit"))), "the scenario block and the end block");

    server.uploader.drain(&key()).await;
    assert_eq!(ended_reads(&server, &ticket, &figures).await, live, "from the disk copy");
    assert_eq!(pictured(replayed(&server, &ticket).await), picture_live, "from the disk copy");
    let folder = server.streams.frames().folder(&key());
    server.streams.frames().forget(&key());
    tokio::fs::remove_dir_all(&folder).await.unwrap();
    assert_eq!(ended_reads(&server, &ticket, &figures).await, live, "from the live segments, cold");
    assert_eq!(pictured(replayed(&server, &ticket).await), picture_live, "from the live segments, cold");
    scratch.close().await;
}

/// Each metric in `names` read whole after the run, one after another: its rows and the message that ended them.
async fn ended_reads(server: &Server, ticket: &str, names: &[&str]) -> Vec<(Vec<MetricPoint>, Option<MetricMessage>)> {
    let mut read = Vec::new();
    for name in names {
        let mut socket = metrics_viewer(server, ticket, name, None).await;
        let attached = next_message(&mut socket).await;
        assert!(matches!(attached, MetricMessage::Attached { live: false, .. }), "{name}: {attached:?}");
        read.push(rows_until(&mut socket).await);
    }
    read
}

/// A frames viewer's whole replay: how many frames, and the text replies.
async fn replayed(server: &Server, ticket: &str) -> (usize, Vec<Value>) {
    let mut socket = frames_viewer(server, ticket, None).await;
    attached_frames(&mut socket).await;
    drained(&mut socket).await
}

/// The whole run of each metric in `names`, read one after another.
async fn every(server: &Server, ticket: &str, names: &[&str]) -> Vec<Vec<MetricPoint>> {
    let mut read = Vec::new();
    for name in names {
        read.push(whole(server, ticket, name).await);
    }
    read
}

/// Each metric's rows read `how` equal to the ones sent live, the first that differs named.
fn same_as_live(names: &[&str], live: &[Vec<MetricPoint>], read: &[Vec<MetricPoint>], how: &str) {
    for ((name, live), read) in names.iter().zip(live).zip(read) {
        assert_eq!(read, live, "{name}, {how}");
    }
}

/// Two viewers of one metric of an open attempt get the same live row from the attempt's one hub, which a viewer of
/// another metric shares too; the hub goes once all have left.
#[tokio::test]
async fn the_viewers_of_an_attempt_share_its_one_hub_which_goes_with_the_last() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let (frames, deltas, width) = segments(1);
    let (early, late) = frames.split_at(frames.len() - 1);
    for frame in early {
        runner.send_frame(frame.clone()).await;
    }
    held_through(&server, (deltas as u64 - 2) * width).await;
    let ticket = scratch.ticket(60).await;
    let (mut first, mut second) =
        (metrics_viewer(&server, &ticket, "picks", None).await, metrics_viewer(&server, &ticket, "picks", None).await);
    let mut histories = Vec::new();
    for socket in [&mut first, &mut second] {
        assert!(matches!(next_message(socket).await, MetricMessage::Attached { live: true, .. }));
        let mut history = Vec::new();
        // Every slice but the newest, whose end only the next frame states.
        while history.len() < deltas - 2 {
            let MetricMessage::Rows { rows, .. } = next_message(socket).await else { panic!("rows") };
            history.extend(rows);
        }
        histories.push(history);
    }
    assert_eq!(histories[0], histories[1], "one hub's rows");
    let mut other = metrics_viewer(&server, &ticket, "cumulative_picks", None).await;
    assert!(matches!(next_message(&mut other).await, MetricMessage::Attached { live: true, .. }));
    assert_eq!(server.hubs.running(), 1, "one hub for the attempt, whatever its viewers ask");
    runner.send_frame(late[0].clone()).await;
    let lives = [next_message(&mut first).await, next_message(&mut second).await];
    assert!(matches!(&lives[0], MetricMessage::Rows { rows, .. } if rows.len() == 1), "{:?}", lives[0]);
    assert_eq!(lives[0], lives[1]);
    drop((first, other));
    sleep(Duration::from_millis(100)).await;
    assert_eq!(server.hubs.running(), 1, "the other viewer still holds it");
    drop(second);
    for _ in 0..50 {
        if server.hubs.running() == 0 {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(server.hubs.running(), 0, "the last viewer gone, the hub goes");
    scratch.close().await;
}

/// The attempt ended and was forgotten by memory and disk, its live segments in a store whose every read waits for
/// the test: the metric viewer's attach starts the one fetch and is answered once the scenario block has landed,
/// before the two segments; a frames viewer joins the same fetch, and the store is read once per object for both.
#[tokio::test]
async fn every_viewer_of_a_cold_attempt_follows_its_one_fetch_the_one_that_started_it_first() {
    let scratch = scratch_or_skip!();
    let options = FramesOptions::default();
    let store = Arc::new(Gated::over(&options.objects()));
    let server = server_with(&scratch.pool, BOUNDS, FramesOptions { store: Some(store.clone()), ..options }).await;
    let deltas = cold(&server, &store).await;

    let ticket = scratch.ticket(60).await;
    let mut watching = metrics_viewer(&server, &ticket, "picks", None).await;
    store.release(1);
    let MetricMessage::Attached { live, .. } = next_message(&mut watching).await else { panic!("attached") };
    assert!(!live);
    assert!(store.reads.load(Ordering::SeqCst) <= 2, "attached with the segments still in the store");
    let mut picture = frames_viewer(&server, &ticket, None).await;
    assert_eq!(attached_frames(&mut picture).await, 0, "joined the fetch with only the scenario block landed");
    store.release(2);
    let (rows, last) = rows_until(&mut watching).await;
    assert_eq!(rows.len(), deltas, "{rows:?}");
    assert!(rows.windows(2).all(|pair| pair[0].to_sim_ms < pair[1].to_sim_ms), "in order");
    assert!(matches!(last, Some(MetricMessage::Ended { .. })), "{last:?}");
    let (sent, texts) = drained(&mut picture).await;
    assert_eq!(sent, 1 + 1 + deltas, "the scenario block, the first keyframe and every delta: {texts:?}");
    assert_eq!(texts.last().map(|text| text["type"].clone()), Some(json!("ended")));
    assert_eq!(store.reads.load(Ordering::SeqCst), 3, "the scenario block and two segments, read once");
    scratch.close().await;
}

/// A fetch failing after the scenario block landed refuses the metric viewer `unavailable` and tells the frames viewer
/// in an error, never `ended` over what landed; the next viewer fetches again and reads the run whole.
#[tokio::test]
async fn a_failed_fetch_refuses_every_viewer_and_ends_none() {
    let scratch = scratch_or_skip!();
    let options = FramesOptions::default();
    let store = Arc::new(Gated::over(&options.objects()));
    let server = server_with(&scratch.pool, BOUNDS, FramesOptions { store: Some(store.clone()), ..options }).await;
    let deltas = cold(&server, &store).await;
    store.fail_from.store(1, Ordering::SeqCst);

    let ticket = scratch.ticket(60).await;
    let mut watching = metrics_viewer(&server, &ticket, "picks", None).await;
    store.release(1);
    assert!(matches!(next_message(&mut watching).await, MetricMessage::Attached { .. }));
    let mut picture = frames_viewer(&server, &ticket, None).await;
    attached_frames(&mut picture).await;
    store.release(1);
    let mut said = Vec::new();
    let refused = loop {
        match next_raw(&mut watching).await {
            Some(Message::Text(text)) => said.push(MetricMessage::read(&text).expect("reads")),
            Some(Message::Close(frame)) => break frame.and_then(|frame| StreamRefusal::from_reason(&frame.reason)),
            other => panic!("the socket ended without a close: {other:?}"),
        }
    };
    assert_eq!(refused, Some(StreamRefusal::Unavailable));
    assert!(!said.iter().any(|message| matches!(message, MetricMessage::Ended { .. })), "{said:?}");
    let (_, texts) = drained(&mut picture).await;
    assert_eq!(texts.last().map(|text| text["type"].clone()), Some(json!("error")), "{texts:?}");

    store.fail_from.store(usize::MAX, Ordering::SeqCst);
    let mut again = metrics_viewer(&server, &ticket, "picks", None).await;
    store.release(3);
    assert!(matches!(next_message(&mut again).await, MetricMessage::Attached { .. }));
    let (rows, last) = rows_until(&mut again).await;
    assert_eq!(rows.len(), deltas);
    assert!(matches!(last, Some(MetricMessage::Ended { .. })), "{last:?}");
    scratch.close().await;
}

/// An ended attempt's uploads finish and its row says so, and its segments then read whole, ended as carrying no end
/// block, as the runner sent none. Had its server died with its last segment and the queue's end still queued, one
/// segment in the store and the row not saying the uploads finished, the attempt is served from the segment held and
/// both its streams, `/metrics` and `/frames` alike, end `recording-incomplete`, never with the run's own ending.
#[tokio::test]
async fn segments_whose_uploads_never_finished_end_both_streams_incomplete() {
    let scratch = scratch_or_skip!();
    let options = FramesOptions::default();
    let store = Arc::new(Gated::over(&options.objects()));
    let server = server_with(&scratch.pool, BOUNDS, FramesOptions { store: Some(store.clone()), ..options }).await;
    let deltas = cold(&server, &store).await;
    let finished = "SELECT uploads_finished_at IS NOT NULL FROM attempt_streams WHERE run_id = $1 AND n = 1";
    let said: bool = sqlx::query_scalar(finished).bind(RUN).fetch_one(&scratch.pool).await.unwrap();
    assert!(said, "the row says the ended attempt's uploads finished");
    let ticket = scratch.ticket(60).await;
    store.release(3);
    let mut whole = metrics_viewer(&server, &ticket, "picks", None).await;
    assert!(matches!(next_message(&mut whole).await, MetricMessage::Attached { .. }));
    let (rows, last) = rows_until(&mut whole).await;
    assert_eq!(rows.len(), deltas);
    assert_eq!(last, Some(MetricMessage::Ended { v: METRIC_STREAM_FORMAT, outcome: RunOutcome::NoEndBlock }));

    tokio::fs::remove_dir_all(server.streams.frames().folder(&key())).await.unwrap();
    sqlx::query("UPDATE attempt_streams SET segments_uploaded = 1, uploads_finished_at = NULL WHERE run_id = $1")
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .unwrap();
    store.release(2);
    let mut short = metrics_viewer(&server, &ticket, "picks", None).await;
    assert!(matches!(next_message(&mut short).await, MetricMessage::Attached { .. }));
    let (rows, last) = rows_until(&mut short).await;
    assert!(!rows.is_empty() && rows.len() < deltas, "the one segment held: {} of {deltas} slices", rows.len());
    let incomplete = MetricMessage::Ended { v: METRIC_STREAM_FORMAT, outcome: RunOutcome::RecordingIncomplete };
    assert_eq!(last, Some(incomplete));
    let mut picture = frames_viewer(&server, &ticket, None).await;
    attached_frames(&mut picture).await;
    let (frames, texts) = drained(&mut picture).await;
    assert!(frames > 0, "the picture is served as far as it goes");
    let ended = texts.last().expect("the frames viewer's last word");
    assert_eq!((&ended["type"], &ended["outcome"]), (&json!("ended"), &json!("recording-incomplete")), "{texts:?}");
    scratch.close().await;
}

#[tokio::test]
async fn a_metric_the_server_does_not_compute_a_bad_ticket_and_another_format_are_refused() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let ticket = scratch.ticket(60).await;
    let mut unknown = metrics_viewer(&server, &ticket, "no_such_series", None).await;
    assert_eq!(refusal(&mut unknown).await, Some(StreamRefusal::UnknownMetric));
    let mut forged = metrics_viewer(&server, "not-a-ticket", "picks", None).await;
    assert_eq!(refusal(&mut forged).await, Some(StreamRefusal::TokenInvalid));
    let url = format!("ws://{}/metrics/{RUN}/1", server.external);
    let (mut other_format, _) = tokio_tungstenite::connect_async(url).await.expect("the socket opens");
    let attach = MetricRequest::Attach { v: FileFormat(9), metric: "picks".into(), token: ticket, trailing: None };
    send(&mut other_format, attach.text()).await;
    assert_eq!(refusal(&mut other_format).await, Some(StreamRefusal::UnsupportedVersion));
    scratch.close().await;
}

/// Each failure closes the socket with the wire's reason, before anything else is sent, so a page knows whether
/// another try may succeed: trailing windows the recording's 10 s slices do not cut, and any over a running total,
/// `unsupported-trailing`; a series of time in a state, from a recording made before the robot state channel,
/// `not-recorded`; a disk copy the disk will not open, `unavailable`; one that is not a recording,
/// `recording-unreadable`.
#[tokio::test]
async fn each_failure_closes_the_metric_socket_with_the_wires_reason() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    let (frames, _, width) = segments(1);
    assert_eq!(width, 10_000);
    for frame in &frames {
        runner.send_frame(frame.clone()).await;
    }
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    assert_eq!(runner.refusal().await, None);
    server.uploader.drain(&key()).await;
    let ticket = scratch.ticket(60).await;

    let uncut = json!({ "window_sim_ms": 25_000, "step_sim_ms": 10_000 });
    let mut unfit = metrics_viewer(&server, &ticket, "picks", Some(uncut)).await;
    assert_eq!(refused_first(&mut unfit).await, Some(StreamRefusal::UnsupportedTrailing));
    let whole_slices = json!({ "window_sim_ms": 20_000, "step_sim_ms": 20_000 });
    let mut running_total = metrics_viewer(&server, &ticket, "cumulative_picks", Some(whole_slices)).await;
    assert_eq!(refused_first(&mut running_total).await, Some(StreamRefusal::UnsupportedTrailing));
    let mut states = metrics_viewer(&server, &ticket, "d30_state_share", None).await;
    assert_eq!(refused_first(&mut states).await, Some(StreamRefusal::NotRecorded));

    let copy = server.streams.frames().folder(&key()).join(LIVE_FILE);
    set_permissions(&copy, Permissions::from_mode(0o000)).unwrap();
    // Root reads a file whatever its mode, so only a process the disk refuses can see the refusal.
    if File::open(&copy).is_err() {
        let mut shut = metrics_viewer(&server, &ticket, "picks", None).await;
        assert_eq!(refused_first(&mut shut).await, Some(StreamRefusal::Unavailable));
    }
    set_permissions(&copy, Permissions::from_mode(0o644)).unwrap();
    write(&copy, b"not a recording").unwrap();
    let mut damaged = metrics_viewer(&server, &ticket, "picks", None).await;
    assert_eq!(refused_first(&mut damaged).await, Some(StreamRefusal::RecordingUnreadable));
    scratch.close().await;
}

/// The socket's first message, which must be its close: the refusal it carries.
async fn refused_first(socket: &mut Socket) -> Option<StreamRefusal> {
    match next_raw(socket).await {
        Some(Message::Close(frame)) => frame.and_then(|frame| StreamRefusal::from_reason(&frame.reason)),
        other => panic!("refused before anything else is sent, not {other:?}"),
    }
}

/// A running attempt never streamed and stored nowhere, and one whose runner said `started` and sent no frame yet, hold
/// nothing to read yet: `not-held`, which a page takes as worth another try. Once the attempt has ended with nothing
/// stored, nothing ever will be: `no-recording`, which a page takes as final.
#[tokio::test]
async fn an_attempt_with_nothing_to_read_is_refused_not_held_while_it_goes_and_no_recording_once_ended() {
    let scratch = scratch_or_skip!();
    let server = server(&scratch.pool).await;
    let ticket = scratch.ticket(60).await;
    let mut never = metrics_viewer(&server, &ticket, "picks", None).await;
    assert_eq!(refusal(&mut never).await, Some(StreamRefusal::NotHeld), "never streamed, nothing stored");
    let mut runner = Runner::welcomed(&server).await;
    runner.send(started()).await;
    for _ in 0..50 {
        if server.streams.frames().get(&key()).is_some() {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let mut early = metrics_viewer(&server, &ticket, "picks", None).await;
    assert_eq!(refusal(&mut early).await, Some(StreamRefusal::NotHeld), "started, no scenario block yet");
    sqlx::query("UPDATE attempts SET state = 'failed', ended_at = now(), ended_by = 'platform' WHERE run_id = $1")
        .bind(RUN)
        .execute(&scratch.pool)
        .await
        .expect("the attempt ends, its runner having sent nothing");
    let mut ended = metrics_viewer(&server, &ticket, "picks", None).await;
    assert_eq!(refusal(&mut ended).await, Some(StreamRefusal::NoRecording), "ended, nothing stored");
    assert!(StreamRefusal::NoRecording.is_final_for_viewer() && !StreamRefusal::NotHeld.is_final_for_viewer());
    drop(runner);
    scratch.close().await;
}

/// The fixture's scenario block and first two segments, sent and ended, uploaded and then forgotten by memory and
/// disk: three objects in the store, each one ranged read. How many deltas they hold.
async fn cold(server: &Server, store: &Gated) -> usize {
    let mut runner = Runner::welcomed(server).await;
    runner.send(started()).await;
    let (frames, deltas, _) = segments(2);
    for frame in &frames {
        runner.send_frame(frame.clone()).await;
    }
    runner.send(RunnerMessage::Ended(ended(30_000))).await;
    assert_eq!(runner.refusal().await, None);
    server.uploader.drain(&key()).await;
    server.streams.frames().forget(&key());
    tokio::fs::remove_dir_all(server.streams.frames().folder(&key())).await.unwrap();
    assert_eq!(store.reads.load(Ordering::SeqCst), 0, "nothing read yet");
    deltas
}

/// A frames viewer's `attached`: its live edge, the newest slice the copy held as it attached.
async fn attached_frames(socket: &mut Socket) -> u64 {
    match next_raw(socket).await {
        Some(Message::Text(text)) => {
            let attached: Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(attached["type"], "attached", "{attached}");
            attached["live_edge_ms"].as_u64().expect("an instant")
        }
        other => panic!("attached first, not {other:?}"),
    }
}

/// Every message of a frames viewer until its socket closes: how many frames, and the text replies.
async fn drained(socket: &mut Socket) -> (usize, Vec<Value>) {
    let (mut frames, mut texts) = (0, Vec::new());
    while let Some(message) = next_raw(socket).await {
        match message {
            Message::Binary(_) => frames += 1,
            Message::Text(text) => texts.push(serde_json::from_str(&text).expect("JSON")),
            Message::Close(_) => break,
            _ => {}
        }
    }
    (frames, texts)
}
