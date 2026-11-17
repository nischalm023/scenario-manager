//! What a browser receives, over a recording written here rather than by a run.
//!
//! The recording is synthetic and small, and its truth is written out by hand
//! beside it, so a fault in the code that serves a run cannot hide by being
//! used on both sides of an assertion. Its world document is a real fixture,
//! because compiling a floor is the one thing here that no hand-written stand-in
//! could stand in for.
//!
//! The wire is checked by decoding it with `worldview/worldview` — the crate a
//! browser decodes it with — and comparing what comes out against what the file
//! itself says at the same instant.

use std::collections::BTreeMap;
use std::fs::{File, FileTimes};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use bytes::Bytes;
use crucible::attach::{Attachment, Wanted};
use crucible::floor;
use crucible::protocol::AttachFrom;
use crucible::recording::{Cache, local_path, under_ceiling};
use crucible::series;
use desgraph::types::{CellId, StationId};
use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::runner::RunnerHooks;
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use runstore::layout::{AttemptNumber, RunId, recording_key};
use runstore::{ObjectInfo, RunStorage, RunStore, StoreResult};
use tempfile::TempDir;
use worldview::frames::end::End;
use worldview::frames::heat::{HeatDelta, HeatKeyframe, HeatLayer, HeatLayerDelta};
use worldview::frames::instant;
use worldview::frames::motion::MotionDelta;
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RobotClass, RobotRecord, StationChange, StationState, Step,
    StepKind, ToteContents, ToteLocation, ToteRecord,
};
use worldview::frames::scalars::{
    AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit, ScalarsDelta, ScalarsKeyframe,
};
use worldview::frames::scenario::RobotEntry;
use worldview::frames::state::state_at;
use worldview::frames::tables::TablesDelta;
use worldview::frames::{PLAYBACK, SCALARS};
use worldview::{
    Declare, Delta, Descriptors, Document, Frame, Index, Keyframe, Opaque, Payload, Reader, RunState, Scenario,
    Sections, Tag, WriteFrames, Writer,
};
use worldview_floor::FloorDoc;

mod support;
use support::{DEPTH, LATE, PICKS, SLICE_MS, SLICES, write_run};

/// A run written into a folder that lasts as long as the test.
struct Recorded {
    _dir: TempDir,
    path: PathBuf,
}

fn recorded() -> Recorded {
    let dir = TempDir::new().expect("a folder");
    let path = dir.path().join("recording.worldview");
    write_run(&path);
    Recorded { _dir: dir, path }
}

/// Every frame one attachment sends, decoded the way a browser decodes them.
struct Wire {
    scenario: Scenario,
    index_records: usize,
    keyframe: (SimTime, Keyframe),
    keyframe_sections: Vec<u8>,
    deltas: Vec<(SimTime, Delta, Vec<u8>)>,
    declares: Vec<Declare>,
    end: Option<End>,
}

fn read_wire(recording: &Path, from: AttachFrom, sections: &[&str]) -> (u64, Wire) {
    let names: Vec<String> = sections.iter().map(|name| (*name).to_owned()).collect();
    let scene = floor::of(recording).expect("the world document compiles");
    let wanted = Wanted::parse(&names).expect("known sections");
    let (opened, mut attachment) = Attachment::open(recording, &from, wanted, scene).expect("the attachment opens");

    let mut scenario = None;
    let mut index_records = 0;
    let mut keyframe = None;
    let mut keyframe_sections = Vec::new();
    let mut deltas = Vec::new();
    let mut declares = Vec::new();
    let mut end = None;
    while let Some(next) = attachment.next_frame() {
        let sent = next.expect("a frame");
        let (frame, consumed) = Frame::decode_at(&sent.bytes, 0).expect("the frame decodes");
        assert_eq!(consumed, sent.bytes.len(), "one binary message is exactly one frame");
        let body = frame.contents(0).expect("the body decompresses");
        match frame.header.tag {
            Tag::Scenario => scenario = Some(Scenario::from_bytes(&body, 0).expect("the block decodes")),
            Tag::Index => index_records = Index::from_block(&body, 0).expect("the block decodes").len(),
            Tag::Declare => declares.push(Declare::from_bytes(&body, 0).expect("the declaration decodes")),
            Tag::Keyframe => {
                let read = Sections::decode(body, 0).expect("the sections decode");
                keyframe_sections = read.ids();
                keyframe = Some((instant(frame.header.t), Keyframe::decode(&read, 0).expect("the keyframe decodes")));
            }
            Tag::Delta => {
                let read = Sections::decode(body, 0).expect("the sections decode");
                let ids = read.ids();
                deltas.push((instant(frame.header.t), Delta::decode(&read, 0).expect("the delta decodes"), ids));
            }
            Tag::End => end = Some(End::from_bytes(&body, 0).expect("the end block decodes")),
        }
    }
    let wire = Wire {
        scenario: scenario.expect("the stream opens with the scenario block"),
        index_records,
        keyframe: keyframe.expect("one keyframe travels"),
        keyframe_sections,
        deltas,
        declares,
        end,
    };
    (opened.from_ms, wire)
}

impl Wire {
    /// The state the frames describe at `t`, rebuilt as a browser rebuilds it.
    fn state_at(&self, t: SimTime) -> RunState {
        let mut descriptors = Descriptors::from_scenario(&self.scenario);
        for declared in &self.declares {
            descriptors.push(declared);
        }
        let (at, keyframe) = &self.keyframe;
        let mut state = RunState::from_keyframe(*at, keyframe, &descriptors).expect("the keyframe rebuilds");
        for (index, (begins, delta, _)) in self.deltas.iter().enumerate() {
            if *begins >= t {
                break;
            }
            let ends = self.deltas.get(index + 1).map_or(*begins + SimDuration::ms(SLICE_MS), |(next, _, _)| *next);
            if ends > t {
                break;
            }
            state.apply(*begins, ends, delta).expect("the delta applies");
        }
        state
    }
}

#[test]
fn a_stream_from_the_start_rebuilds_the_state_the_file_states() {
    let run = recorded();
    let (from_ms, wire) = read_wire(&run.path, AttachFrom::At(0), &["playback", "scalars", "heat"]);
    assert_eq!(from_ms, 0);
    assert_eq!(wire.keyframe.0, SimTime::ZERO, "the keyframe at or before zero is the run's first");
    assert_eq!(wire.deltas.len() as u64, SLICES, "every slice after it travels");
    assert!(wire.end.is_some(), "the end block closes the stream");

    let mut file = Reader::open(&run.path).expect("the file opens");
    let (meant, _) = state_at(&mut file, SimTime::ms(6 * SLICE_MS)).expect("the file states one");
    assert_eq!(wire.state_at(SimTime::ms(6 * SLICE_MS)), meant, "the wire rebuilds what the file rebuilds");
}

#[test]
fn a_stream_from_an_instant_opens_on_the_keyframe_before_it() {
    let run = recorded();
    let asked = 5 * SLICE_MS;
    let (from_ms, wire) = read_wire(&run.path, AttachFrom::At(asked), &["playback", "scalars", "heat"]);
    assert_eq!(from_ms, asked);
    assert_eq!(wire.keyframe.0, SimTime::ms(3 * SLICE_MS), "the last keyframe at or before the instant asked for");
    assert_eq!(wire.deltas.first().expect("a delta").0, SimTime::ms(3 * SLICE_MS), "and the deltas after it");
    assert_eq!(wire.index_records, 10, "the scenario record, two keyframes, one declaration and six deltas");

    let mut file = Reader::open(&run.path).expect("the file opens");
    let (meant, _) = state_at(&mut file, SimTime::ms(asked)).expect("the file states one");
    assert_eq!(wire.state_at(SimTime::ms(asked)), meant, "a seek on the wire and a seek in the file agree");
}

#[test]
fn live_on_a_finished_run_opens_where_the_run_stopped() {
    let run = recorded();
    let (from_ms, wire) = read_wire(&run.path, AttachFrom::Live, &["playback"]);
    assert_eq!(from_ms, SLICES * SLICE_MS, "the last instant the recording reaches");
    assert_eq!(wire.keyframe.0, SimTime::ms(6 * SLICE_MS), "the last keyframe before it");
}

#[test]
fn the_scenario_a_browser_receives_drops_the_data_and_folds_the_declarations() {
    let run = recorded();
    let (_, early) = read_wire(&run.path, AttachFrom::At(0), &["playback"]);
    assert_eq!(early.scenario.data, None, "the workload files do not travel, and say so as nil");
    assert_eq!(early.scenario.producer, None, "nor the producer's own rebuild document");
    assert!(!early.scenario.world.is_empty(), "the world document does");
    assert_eq!(early.scenario.cells.len(), 8, "and the id tables");
    assert_eq!(names(&early.scenario), vec![PICKS, DEPTH], "what the run declared at its start");
    assert_eq!(early.declares.len(), 1, "and the later declaration arrives as its own frame");

    let (_, late) = read_wire(&run.path, AttachFrom::At(5 * SLICE_MS), &["playback"]);
    assert_eq!(names(&late.scenario), vec![PICKS, DEPTH, LATE], "folded into the block for an attachment past it");
    assert!(late.declares.is_empty(), "so it does not arrive twice");
}

fn names(scenario: &Scenario) -> Vec<&str> {
    scenario.metrics.iter().map(|metric| metric.name.as_str()).collect()
}

#[test]
fn only_the_sections_the_attachment_asked_for_travel() {
    let run = recorded();
    let (_, wire) = read_wire(&run.path, AttachFrom::At(0), &["playback"]);
    assert_eq!(wire.keyframe_sections, vec![PLAYBACK]);
    assert!(wire.deltas.iter().all(|(_, _, ids)| *ids == vec![PLAYBACK]), "every delta too");
    assert!(wire.keyframe.1.scalars.is_none() && wire.keyframe.1.heat.is_none(), "and the rest is absent, not empty");

    let (_, default) = read_wire(&run.path, AttachFrom::At(0), &[]);
    assert_eq!(default.keyframe_sections, vec![PLAYBACK, SCALARS]);
}

/// Two requests for one uncached recording arrive together whenever a browser
/// asks to attach and for its charts at once. They must not race each other's
/// temporary file: both get the whole recording, once.
#[tokio::test]
async fn two_first_requests_for_one_recording_fetch_it_once() {
    let run = recorded();
    let store = TempDir::new().expect("a folder");
    let temp = TempDir::new().expect("a folder");
    let id = RunId::new("01TESTFETCH").expect("a key segment");
    let storage = RunStore::local_folder(store.path()).expect("a folder store");
    let bytes = std::fs::read(&run.path).expect("the recording");
    storage.put(&recording_key(&id, AttemptNumber::FIRST), bytes.clone().into()).await.expect("stored");

    let cache = Cache::new(u64::MAX);
    let (first, second) = tokio::join!(
        cache.fetch(&storage, temp.path(), &id, AttemptNumber::FIRST, None),
        cache.fetch(&storage, temp.path(), &id, AttemptNumber::FIRST, None),
    );
    let (first, second) = (first.expect("the first fetch"), second.expect("the second"));
    assert_eq!(first, second, "one copy, under one name");
    assert_eq!(std::fs::read(&first).expect("readable"), bytes, "and it is the whole object");
    let left_over: Vec<_> = std::fs::read_dir(first.parent().expect("a folder"))
        .expect("readable")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains("arriving"))
        .collect();
    assert!(left_over.is_empty(), "and nothing half-written is left behind");
}

/// A store that counts how often the cache asks it what an object is, which is
/// what a replay from a held copy must never do.
#[derive(Debug)]
struct Counting {
    inner: RunStore,
    heads: AtomicUsize,
}

#[async_trait]
impl RunStorage for Counting {
    async fn put(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        self.inner.put(key, bytes).await
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
        self.heads.fetch_add(1, Ordering::Relaxed);
        self.inner.head(key).await
    }
}

/// A replay of a recording this machine holds asks storage nothing when the
/// registry's size for it matches the copy, and asks once when there is no size
/// to go on or the size disagrees. The rule rests on an object being written
/// whole under a key that names one attempt and never appended to; this pins it.
#[tokio::test]
async fn a_held_recording_at_the_registered_size_asks_storage_nothing() {
    let run = recorded();
    let store = TempDir::new().expect("a folder");
    let temp = TempDir::new().expect("a folder");
    let id = RunId::new("01TESTHELD").expect("a key segment");
    let inner = RunStore::local_folder(store.path()).expect("a folder store");
    let bytes = std::fs::read(&run.path).expect("the recording");
    let size = bytes.len() as u64;
    inner.put(&recording_key(&id, AttemptNumber::FIRST), bytes.clone().into()).await.expect("stored");
    let storage = Counting { inner, heads: AtomicUsize::new(0) };
    let cache = Cache::new(u64::MAX);
    let heads = || storage.heads.load(Ordering::Relaxed);

    // Nothing is here yet, so knowing the size does not spare the fetch.
    let path = cache.fetch(&storage, temp.path(), &id, AttemptNumber::FIRST, Some(size)).await.expect("fetched");
    assert_eq!(heads(), 1, "an absent copy is fetched, which begins by asking storage");
    assert_eq!(std::fs::read(&path).expect("readable"), bytes);

    cache.fetch(&storage, temp.path(), &id, AttemptNumber::FIRST, Some(size)).await.expect("served");
    assert_eq!(heads(), 1, "a copy at the registered size was served without a word to storage");

    // A reconciled attempt has no size on its row: storage is asked, once.
    cache.fetch(&storage, temp.path(), &id, AttemptNumber::FIRST, None).await.expect("served");
    assert_eq!(heads(), 2);

    // A size that disagrees with the copy is settled by storage, and the copy
    // storage agrees with stands.
    cache.fetch(&storage, temp.path(), &id, AttemptNumber::FIRST, Some(size + 1)).await.expect("served");
    assert_eq!(heads(), 3);
    assert_eq!(std::fs::read(&path).expect("readable"), bytes, "the copy storage agreed with was kept");
}

/// The cache is bounded: a fetch that takes it past the ceiling throws away the
/// runs nobody has read for longest, and never the one just asked for.
#[test]
fn the_cache_stays_under_its_ceiling() {
    let run = recorded();
    let temp = TempDir::new().expect("a folder");
    let mut held = Vec::new();
    for attempt in 1..=3u32 {
        let id = RunId::new(format!("01TESTRUN{attempt}")).expect("a key segment");
        let path = local_path(temp.path(), &id, AttemptNumber::FIRST);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("made");
        std::fs::copy(&run.path, &path).expect("copied");
        // Read times a second apart, written outright. Reading the copy was
        // not enough: a copy on macOS keeps the source's timestamps, and a
        // read there leaves the access time alone, so every copy had one read
        // time and "oldest read" fell to the order the folder listed them in.
        let read_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + u64::from(attempt));
        File::options()
            .write(true)
            .open(&path)
            .expect("opened")
            .set_times(FileTimes::new().set_accessed(read_at).set_modified(read_at))
            .expect("read time set");
        held.push(path);
    }
    let one = std::fs::metadata(&held[0]).expect("readable").len();
    under_ceiling(temp.path(), one * 2, &held[2]);
    assert!(!held[0].exists(), "the one read longest ago went");
    assert!(held[2].exists(), "and the one just asked for stayed");
}

#[test]
fn the_journal_does_not_travel_to_a_browser() {
    let refused = Wanted::parse(&["tables".to_owned()]).expect_err("the journal is not a browser's");
    assert!(refused.to_string().contains("does not travel"), "{refused}");
    assert!(refused.to_string().contains("motion"), "and the refusal names what does: {refused}");
    // The traffic overlay reads motion, so a browser may ask for it.
    assert!(Wanted::parse(&["motion".to_owned()]).is_ok());
}

/// Later keyframes never travel, which rests on the deltas alone reconstructing
/// every one of them. Walk the recording and hold the accumulated state against
/// each keyframe it states. A failure here is a defect in the WRITER — the
/// answer is to fix what it wrote, never to start sending keyframes.
#[test]
fn the_deltas_alone_reconstruct_every_later_keyframe() {
    let run = recorded();
    reaches_every_keyframe(&run.path);
}

/// The same invariant over a recording phase 1's writer actually wrote, from a
/// real run of the simulator rather than a fixture written by hand: a hand-made
/// file can only prove the check, and this proves the producer.
#[test]
fn a_recorded_run_reconstructs_every_later_keyframe_from_its_deltas() {
    // `minimal_putaway` with a second VTM, because with one its zone saturates:
    // as the barrier reaches a clearance's departure, every relay's reservation
    // is still unfinalized, the pickups and VTM cycles that would free one are
    // decided only later in scheduler time, and the run stops (fwd task Xj7z1w7Y).
    reaches_every_keyframe(&recorded_run("minimal_putaway_two_vtms_spec.json", 600).1);
}

/// And over a site-sized document, where a run has hundreds of robots turning
/// and a keyframe states a heading for every one of them. A keyframe goes out
/// every 300 s of simtime and no frame begins at or after the run's stop, so
/// the run lasts 310 s to hold a keyframe after its first.
#[test]
fn a_recorded_cencora_run_reconstructs_every_later_keyframe_from_its_deltas() {
    reaches_every_keyframe(&recorded_run("cencora_v3_spec.json", 310).1);
}

/// Run a fixture, recorded as it goes. The folder is handed back with the
/// path, because dropping it takes the recording with it.
fn recorded_run(fixture: &str, seconds: u64) -> (TempDir, PathBuf) {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures").join(fixture);
    let overrides = RunOverrides {
        sim_duration: Some(Duration::from_secs(seconds)),
        num_threads: Some(1),
        run_name: Some("keyframe-invariant".to_owned()),
        ..RunOverrides::default()
    };
    let prepared = prepare(&spec, &overrides).expect("the fixture prepares");
    let dir = TempDir::new().expect("a folder");
    let path = dir.path().join("recorded.worldview");
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    prepared.execute(RunnerHooks::default(), Some(target)).expect("the run records");
    (dir, path)
}

fn reaches_every_keyframe(recording: &Path) {
    let mut file = Reader::open(recording).expect("the file opens");
    let scenario = Scenario::read_from(&mut file).expect("the scenario block");
    let mut descriptors = Descriptors::from_scenario(&scenario);
    let records: Vec<_> = file.index().records().to_vec();

    let mut state: Option<RunState> = None;
    let mut pending: Option<(SimTime, Delta)> = None;
    let mut stated_here: Option<(SimTime, RunState)> = None;
    let mut checked = 0;
    for record in records {
        let at = instant(record.t);
        match record.tag {
            // A metric the run first states mid-run is declared before the
            // slice that states it, and the state reads it from then on.
            Tag::Declare => {
                descriptors.push(&file.payload_at::<Declare>(&record).expect("the declaration"));
                if let Some(held) = state.as_mut() {
                    held.scalars.describe(&descriptors);
                }
            }
            Tag::Keyframe => {
                let sections = file.sections_at(&record).expect("its sections");
                let keyframe = Keyframe::decode(&sections, record.offset).expect("the keyframe");
                stated_here = Some((at, RunState::from_keyframe(at, &keyframe, &descriptors).expect("it rebuilds")));
            }
            Tag::Delta => {
                let sections = file.sections_at(&record).expect("its sections");
                let delta = Delta::decode(&sections, record.offset).expect("the delta");
                if let Some((begins, before)) = pending.take()
                    && let Some(held) = state.as_mut()
                {
                    held.apply(begins, at, &before).expect("the delta applies");
                }
                // Standing at K means holding the headings K states, which the
                // delta stamped at K carries — the same step `state_at` takes
                // before it answers, and the reason a seek and a replay agree.
                if let Some((keyframe_at, stated)) = stated_here.take() {
                    match state.take() {
                        None => {}
                        Some(mut walked) => {
                            walked.enter_slice(keyframe_at, &delta).expect("the slice opens");
                            reaches(&walked, &stated, record.t.0);
                            checked += 1;
                        }
                    }
                    state = Some(stated);
                }
                pending = Some((at, delta));
            }
            _ => {}
        }
    }
    assert!(checked >= 1, "the run has a keyframe after its first to check against");
}

/// Hold one accumulated state against the keyframe written at the same instant.
/// Everything a viewer places, and everything it reads off a chart or an
/// overlay, compared exactly — headings included.
fn reaches(walked: &RunState, stated: &RunState, at_ms: u64) {
    let places = |state: &RunState| -> BTreeMap<RobotId, (Position, Option<ToteId>)> {
        state.playback.robots.iter().map(|(robot, held)| (*robot, (held.at, held.carrying))).collect()
    };
    assert_eq!(places(walked), places(stated), "the deltas place every robot as the keyframe at {at_ms} ms does");
    let totes = |state: &RunState| -> BTreeMap<ToteId, ToteLocation> {
        state.playback.totes.iter().map(|(tote, held)| (*tote, held.location)).collect()
    };
    assert_eq!(totes(walked), totes(stated), "and every container at {at_ms} ms");
    assert_eq!(walked.playback.reserved_relay_cells, stated.playback.reserved_relay_cells, "relays at {at_ms} ms");
    assert_eq!(walked.playback.stations, stated.playback.stations, "station depths at {at_ms} ms");
    assert_eq!(walked.scalars, stated.scalars, "metric values at {at_ms} ms");
    assert_eq!(walked.heat, stated.heat, "grid counters at {at_ms} ms");

    let headings = |state: &RunState| -> BTreeMap<RobotId, Option<i32>> {
        state.playback.robots.iter().map(|(robot, held)| (*robot, held.heading_mrad)).collect()
    };
    assert_eq!(headings(walked), headings(stated), "and which way each one faces at {at_ms} ms");
}

#[test]
fn the_floor_and_the_census_decode_to_the_renderers_own_types() {
    let run = recorded();
    let (_, wire) = read_wire(&run.path, AttachFrom::At(0), &["playback"]);
    let floor: FloorDoc = wire.scenario.floor.expect("a floor travels").read("floor").expect("it decodes");
    let census: Vec<(String, Vec<u32>)> =
        wire.scenario.census.expect("a census travels").read("census").expect("it decodes");

    let drawn = floor.cells.iter().filter(|cell| cell.is_some()).count();
    assert!(drawn > 0, "the fixture's document has cells");
    assert!(floor.cell_size_mm > 0.0, "and one size for them");
    assert!(!floor.edges.is_empty(), "the hops between them travel as arrows");
    assert!(census.iter().any(|(_, cells)| !cells.is_empty()), "and every entity that is somewhere says where");

    let scene = floor.into_scene();
    assert_eq!(scene.cells().iter().filter(|cell| cell.is_some()).count(), drawn, "and it reads back as a scene");
}

#[test]
fn the_series_is_the_running_total_the_file_states() {
    let run = recorded();
    let asked = [PICKS.to_owned(), DEPTH.to_owned(), "nothing".to_owned()];
    let read = series::of(&run.path, &asked, None, 2_000).expect("the series assembles");

    let picks: Vec<(u64, f64)> = (0..=SLICES).map(|slice| (slice * SLICE_MS, slice as f64)).collect();
    assert_eq!(read[0].points, picks, "one pick a slice, adding up");
    // The gauge states nothing in the run's first keyframe, so its series opens
    // at the end of the first slice rather than at zero.
    let depth: Vec<(u64, f64)> = (0..SLICES).map(|slice| ((slice + 1) * SLICE_MS, slice as f64)).collect();
    assert_eq!(read[1].points, depth, "a gauge reads the newest value, not a sum of them");
    assert_eq!(read[2].metric, "nothing", "a metric the run never declared comes back named");
    assert!(read[2].points.is_empty(), "and empty");
}

#[test]
fn a_series_stops_where_it_was_asked_to() {
    let run = recorded();
    let asked = [PICKS.to_owned()];
    let read = series::of(&run.path, &asked, Some(3 * SLICE_MS), 2_000).expect("the series assembles");
    assert_eq!(read[0].points.last(), Some(&(3 * SLICE_MS, 3.0)), "and no further");
}
