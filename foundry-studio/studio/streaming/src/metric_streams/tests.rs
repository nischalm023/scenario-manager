//! The hubs without sockets: an open attempt's frames pushed into memory, simrunner's canned three hours, through the
//! one hub every member of the attempt shares, whatever it asks; a member that opens its view while frames come gets
//! every row once; one that stops reading lags only itself, and the hub goes once every member has left; an ask the
//! recording cannot serve fails its member alone; a hub that fails is forgotten at once; and a burst the hub cannot
//! keep up with fails no hub.

use std::fs::File;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use desunits::SimDuration;
use studio_metrics::{Ask, Metric, ReportSeries, StateSeries, SummarySeries, TimeSeries, Unrecorded, Unservable};
use studio_protocol::{Bucket, MetricPoint, TrailingSpec};
use tempfile::TempDir;
use tokio::runtime::Builder;
use tokio::time::{sleep, timeout};
use worldview::container::reader::frame_at;
use worldview::{Reader, Tag};

use super::Hubs;
use super::hub::member::{Lagged, Member, Told, Viewed};
use super::hub::{HubEnd, HubFault};
use crate::frames::FrameStore;
use crate::frames::tests::key;
use crate::provider::{Provider, Unreached};
use crate::upload::Uploader;

/// Simrunner's canned recording, its frames as a runner sends them: every one but the index block.
pub(super) fn canned() -> Vec<(Tag, Vec<u8>)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../simrunner/fixtures/canned/recording.worldview");
    let reader = Reader::open(&path).expect("the canned recording opens");
    let mut file = File::open(&path).expect("it reads");
    let records = reader.index().records().to_vec();
    records
        .into_iter()
        .filter(|record| record.tag != Tag::Index)
        .map(|record| (record.tag, frame_at(&mut file, record.offset).expect("a frame").to_bytes()))
        .collect()
}

pub(super) fn hubs(dir: &TempDir) -> (Hubs, FrameStore) {
    let objects = dir.path().join("objects");
    std::fs::create_dir_all(&objects).unwrap();
    let frames = FrameStore::new(dir.path().join("cache"), u64::MAX, Uploader::for_tests(&objects));
    (Hubs::new(Provider::new(frames.clone())), frames)
}

fn ask(metric: Metric) -> Ask {
    Ask { metric, trailing: None }
}

pub(super) const PICKS: Ask = Ask { metric: Metric::Series(TimeSeries::PicksPerSlice), trailing: None };

pub(super) async fn told(member: &mut Member) -> Result<Told, Lagged> {
    timeout(Duration::from_secs(10), member.next()).await.expect("the hub went on")
}

/// The member's next batch of rows, the notice that the scenario block is in passed over.
async fn next_rows(member: &mut Member) -> Vec<MetricPoint> {
    loop {
        match told(member).await {
            Ok(Told::Rows(batch)) => return batch.to_vec(),
            Ok(Told::Described) => {}
            other => panic!("a row, not {other:?}"),
        }
    }
}

/// The member's view, once the hub has its follower.
pub(super) async fn viewed(member: &mut Member) -> Result<Viewed, Unservable> {
    loop {
        if let Some(viewed) = member.view().await.expect("the hub's follower holds") {
            return viewed;
        }
        assert!(matches!(told(member).await, Ok(Told::Described)), "the scenario block is in");
    }
}

/// Every row the member is told until the hub's end, and that end.
async fn rows_to_the_end(member: &mut Member) -> (Vec<MetricPoint>, HubEnd) {
    let mut rows = Vec::new();
    loop {
        match told(member).await {
            Ok(Told::Rows(batch)) => rows.extend(batch.iter().cloned()),
            Ok(Told::Ended(end)) => return (rows, end),
            Ok(Told::Described) => {}
            other => panic!("a row or the end, not {other:?}"),
        }
    }
}

/// Members asking different metrics of one attempt share its one hub, and one that opens its view while the frames
/// come, rows of its ask going out between its join and its view's opening, gets from its history and then the rows its
/// view makes final every row once: what a member there from the start got.
#[tokio::test]
async fn every_member_of_an_attempt_shares_its_hub_and_one_joining_late_misses_and_repeats_no_row() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let frames = store.open(&key(1));
    let recording = canned();
    frames.push(recording[0].1.clone()).await.expect("the scenario block");
    let mut early = hubs.join(&key(1), PICKS, Ok(None)).member;
    let mut other = hubs.join(&key(1), ask(Metric::Summary(SummarySeries::TotalPicks)), Ok(None)).member;
    assert_eq!(hubs.running(), 1, "one hub for the attempt, whatever its members ask");
    let opened = viewed(&mut early).await.expect("served");
    assert!(opened.rows.is_empty() && opened.live, "nothing final yet, and the attempt goes on");
    viewed(&mut other).await.expect("served");
    let deltas = |frames: &[(Tag, Vec<u8>)]| frames.iter().filter(|(tag, _)| *tag == Tag::Delta).count();
    let (first, rest) = recording[1..].split_at(recording.len() / 2);
    let (between, rest) = rest.split_at(40);
    for (_, bytes) in first {
        frames.push(bytes.clone()).await.expect("a frame of the run");
    }
    let mut late = hubs.join(&key(1), PICKS, Ok(None)).member;
    for (_, bytes) in between {
        frames.push(bytes.clone()).await.expect("a frame of the run");
    }
    // The rows of every slice so far go out but the newest's, whose end only the next frame states: those after the
    // late member joined among them, before its view opens.
    let mut from_the_start = Vec::new();
    while from_the_start.len() < deltas(first) + deltas(between) - 1 {
        from_the_start.extend(next_rows(&mut early).await);
    }
    let late_history = viewed(&mut late).await.expect("served").rows;
    for (_, bytes) in rest {
        frames.push(bytes.clone()).await.expect("a frame of the run");
    }
    store.close(&key(1));
    let (early_rest, end) = rows_to_the_end(&mut early).await;
    from_the_start.extend(early_rest);
    let (late_rest, _) = rows_to_the_end(&mut late).await;
    assert!(matches!(end, HubEnd::Over(_)), "{end:?}");
    assert_eq!(from_the_start.len(), deltas(&recording), "a row per slice");
    assert_eq!([late_history, late_rest].concat(), from_the_start, "the late member's history and rows, once each");
    let (total, _) = rows_to_the_end(&mut other).await;
    assert_eq!(total.len(), 1, "the end block's figure, to its own member only");
}

/// A copy cut short, with no end block to say where its last slice ends, states that slice whole as the copy ends: to
/// a view there from the start, and to one opened after the follower took the copy's last frame, which holds every
/// other row in its history.
#[tokio::test]
async fn a_copy_cut_short_states_its_last_slice_whole_to_every_view_as_it_ends() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let frames = store.open(&key(1));
    let recording: Vec<(Tag, Vec<u8>)> = canned().into_iter().filter(|(tag, _)| *tag != Tag::End).collect();
    let deltas = recording.iter().filter(|(tag, _)| *tag == Tag::Delta).count();
    frames.push(recording[0].1.clone()).await.expect("the scenario block");
    let mut early = hubs.join(&key(1), PICKS, Ok(None)).member;
    viewed(&mut early).await.expect("served");
    for (_, bytes) in &recording[1..] {
        frames.push(bytes.clone()).await.expect("a frame of the run");
    }
    let mut from_the_start = Vec::new();
    while from_the_start.len() < deltas - 1 {
        from_the_start.extend(next_rows(&mut early).await);
    }
    let mut late = hubs.join(&key(1), PICKS, Ok(None)).member;
    let late_history = viewed(&mut late).await.expect("served").rows;
    store.close(&key(1));
    let (early_rest, end) = rows_to_the_end(&mut early).await;
    let (late_rest, _) = rows_to_the_end(&mut late).await;
    assert!(matches!(end, HubEnd::Over(_)), "{end:?}");
    assert_eq!(early_rest.len(), 1, "the last slice, as the copy ends");
    let last = &early_rest[0];
    assert_eq!(last.to_sim_ms - last.from_sim_ms, SimDuration::ms(10_000), "stated whole");
    assert_eq!([late_history, late_rest].concat(), [from_the_start, early_rest].concat(), "the late view's rows too");
}

/// A socket joining an attempt whose hub another socket still holds is served from that hub's one reading of the
/// recording, though every copy has gone since: the attempt's memory released and its disk copy deleted.
#[tokio::test]
async fn a_member_joining_a_held_hub_is_served_from_its_reading_though_the_copy_has_gone() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let frames = store.open(&key(1));
    let recording = canned();
    for (_, bytes) in &recording {
        frames.push(bytes.clone()).await.expect("a frame of the run");
    }
    store.close(&key(1));
    let mut first = hubs.join(&key(1), PICKS, Ok(None)).member;
    let history = viewed(&mut first).await.expect("served").rows;
    let (rest, end) = rows_to_the_end(&mut first).await;
    assert!(matches!(end, HubEnd::Over(_)), "{end:?}");
    store.uploader().drain(&key(1)).await;
    std::fs::remove_dir_all(store.folder(&key(1))).expect("the disk copy deleted");
    let total = ask(Metric::Series(TimeSeries::CumulativePicks));
    let joined = hubs.join(&key(1), total, Ok(None));
    assert!(joined.over.is_some(), "the held hub read its copy to the end");
    let mut late = joined.member;
    let totals = viewed(&mut late).await.expect("served from the hub's reading").rows;
    assert_eq!(totals.len(), history.len() + rest.len(), "a running total per slice");
}

/// A burst the hub cannot keep up with: the canned three hours, over a thousand slices, taken by the attempt while the
/// hub's task gets no turn, as a runner's burst outruns it. No hub fails: the member gets every row, in batches of the
/// slices the burst queued rather than one per slice, then the run's end.
#[tokio::test]
async fn a_burst_the_hub_cannot_keep_up_with_fails_no_hub_and_goes_through_in_batches() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let frames = store.open(&key(1));
    let recording = canned();
    frames.push(recording[0].1.clone()).await.expect("the scenario block");
    let mut member = hubs.join(&key(1), PICKS, Ok(None)).member;
    viewed(&mut member).await.expect("served");
    // The burst comes from a thread and a runtime of its own while this test's one runtime thread waits for it, so the
    // hub gets no turn until all of it is in.
    thread::scope(|scope| {
        scope.spawn(|| {
            let runtime = Builder::new_current_thread().build().expect("the burst's runtime");
            runtime.block_on(async {
                for (_, bytes) in &recording[1..] {
                    frames.push(bytes.clone()).await.expect("a frame of the run");
                }
            });
        });
    });
    store.close(&key(1));
    let deltas = recording.iter().filter(|(tag, _)| *tag == Tag::Delta).count();
    assert!(deltas > 1_000, "a burst of over a thousand slices: {deltas}");
    let (mut rows, mut batches) = (0, 0);
    let end = loop {
        match told(&mut member).await {
            Ok(Told::Rows(batch)) => (rows, batches) = (rows + batch.len(), batches + 1),
            Ok(Told::Ended(end)) => break end,
            Ok(Told::Described) => {}
            other => panic!("a row or the end, not {other:?}"),
        }
    };
    assert!(matches!(end, HubEnd::Over(_)), "{end:?}");
    assert_eq!(rows, deltas, "every row");
    assert!(batches * 4 < deltas, "{batches} batches for {deltas} slices");
}

#[tokio::test]
async fn a_member_that_stops_reading_lags_only_itself_and_the_hub_goes_with_the_last() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let frames = store.open(&key(1));
    let recording = canned();
    frames.push(recording[0].1.clone()).await.expect("the scenario block");
    let mut reading = hubs.join(&key(1), PICKS, Ok(None)).member;
    let mut slow = hubs.join(&key(1), PICKS, Ok(None)).member;
    viewed(&mut reading).await.expect("served");
    viewed(&mut slow).await.expect("served");
    let (mut rows, mut slices) = (0, 0);
    for (tag, bytes) in &recording[1..] {
        frames.push(bytes.clone()).await.expect("a frame of the run");
        // Each delta after the first, and the end block, ends a slice, one row of picks: the reading member takes it
        // before the next frame goes in.
        slices += usize::from(*tag == Tag::Delta);
        if (*tag == Tag::Delta && slices > 1) || *tag == Tag::End {
            rows += next_rows(&mut reading).await.len();
        }
    }
    let deltas = recording.iter().filter(|(tag, _)| *tag == Tag::Delta).count();
    assert!(deltas > 1_024, "more slices than a member may fall behind by: {deltas}");
    assert_eq!(rows, deltas, "the reading member got every row");
    assert_eq!(told(&mut slow).await, Err(Lagged), "the member that stopped reading lagged");
    drop((reading, slow));
    for _ in 0..50 {
        if hubs.running() == 0 {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(hubs.running(), 0, "the hub goes with its last member");
}

/// An ask the recording cannot serve is refused to its member alone, as its view opens, before any row: trailing
/// windows not whole slices, a running total's, an end-block figure's, and a series of time in a state from a
/// recording without the robot state channel. A member of the same attempt asking what can be served is served.
#[tokio::test]
async fn an_ask_the_recording_cannot_serve_is_refused_to_its_member_alone() {
    let dir = TempDir::new().unwrap();
    let (hubs, store) = hubs(&dir);
    let recording = canned();
    store.open(&key(1)).push(recording[0].1.clone()).await.expect("the scenario block is taken");
    let windows = |window_ms, step_ms| TrailingSpec {
        window_sim_ms: SimDuration::ms(window_ms),
        step_sim_ms: SimDuration::ms(step_ms),
    };
    let refused = [
        (
            Ask { trailing: Some(windows(25_000, 10_000)), ..PICKS },
            Unservable::UnsupportedTrailing(windows(25_000, 10_000)),
        ),
        (
            Ask { metric: Metric::Series(TimeSeries::CumulativePicks), trailing: Some(windows(30_000, 10_000)) },
            Unservable::UnsupportedTrailing(windows(30_000, 10_000)),
        ),
        (
            Ask { metric: Metric::Summary(SummarySeries::TotalPicks), trailing: Some(windows(30_000, 10_000)) },
            Unservable::UnsupportedTrailing(windows(30_000, 10_000)),
        ),
        (ask(Metric::State(StateSeries::D30State)), Unservable::NotRecorded(Unrecorded::RobotState)),
    ];
    let mut served = hubs.join(&key(1), ask(Metric::Report(ReportSeries::Da50Operations)), Ok(None)).member;
    for (asked, why) in refused {
        let mut member = hubs.join(&key(1), asked, Ok(None)).member;
        assert_eq!(viewed(&mut member).await, Err(why), "{asked:?}");
    }
    let fits = Ask { trailing: Some(windows(30_000, 20_000)), ..PICKS };
    let mut member = hubs.join(&key(1), fits, Ok(None)).member;
    assert_eq!(viewed(&mut member).await.expect("served").descriptor.bucket, Bucket::Trailing(windows(30_000, 20_000)));
    assert!(viewed(&mut served).await.is_ok(), "the attempt's other member is served");
    assert_eq!(hubs.running(), 1);
}

#[tokio::test]
async fn a_hub_that_fails_is_forgotten_so_the_next_join_starts_afresh() {
    let dir = TempDir::new().unwrap();
    let (hubs, _store) = hubs(&dir);
    let mut first = hubs.join(&key(1), PICKS, Ok(None)).member;
    let failed = told(&mut first).await;
    let nowhere = HubEnd::Failed(HubFault::Unreached(Unreached::Nowhere));
    assert_eq!(failed, Ok(Told::Ended(nowhere.clone())), "{failed:?}");
    assert_eq!(hubs.running(), 0, "forgotten while its member still holds it");
    let mut again = hubs.join(&key(1), PICKS, Ok(None)).member;
    assert_eq!(told(&mut again).await, Ok(Told::Ended(nowhere)), "a fresh hub, whose own walk fails as the first did");
}
