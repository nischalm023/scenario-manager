//! A recording read as its bytes come in reads as its file does.
//!
//! What watches a run reads the bytes the run's recording is written as, in
//! whatever pieces they come, so every way of cutting those bytes has to give
//! what the file gives: the bytes whole, a byte at a time, or in pieces that
//! end inside frames give the file's run model, and at every frame boundary
//! the slices taken are the ones the file cut there reads. The small generated
//! floor is run once, recorded through the harness, and its bytes are read
//! every way.

use std::path::{Path, PathBuf};
use std::slice;
use std::sync::OnceLock;

use dessim::config::SimConfig;
use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::metrics::profile::{ResolvedWorkload, ScenarioSource, SummaryStats};
use dessim::metrics::worldview::{IncomingRecording, Recording, RunRecording, RunScenario, read_worldview};
use dessim::observer::output::MetricsOutput;
use dessim::runner::RunnerHooks;
use dessim::sim_types::SimResult;
use native_modules as _;
use serde::Serialize;
use serde_json::Value;
use worldview::{Frame, IndexRecord, Ms, Reader, Tag};

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("worldview-incoming-{}-{name}", std::process::id()))
}

/// The bytes of one run of the small generated floor, recorded through the
/// harness as it ran. Every test here reads the same run.
fn recorded() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop_spec.json");
        let path = scratch("run.worldview");
        let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
        let prepared = prepare(&spec, &RunOverrides::default()).expect("the fixture prepares");
        prepared.execute(RunnerHooks::default(), Some(target)).expect("the run is recorded");
        let bytes = std::fs::read(&path).expect("the recording");
        let _ = std::fs::remove_file(&path);
        bytes
    })
}

/// What `read_worldview` reads from `bytes` written out as a file.
fn read_as_file(bytes: &[u8], name: &str) -> Result<Recording, String> {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("the file is written");
    let read = read_worldview(&path);
    let _ = std::fs::remove_file(&path);
    read
}

/// Every frame of the recording, in file order.
fn frames(bytes: &[u8]) -> Vec<IndexRecord> {
    Reader::from_bytes(bytes).expect("the recording opens").index().records().to_vec()
}

fn json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("the run model is JSON")
}

/// The bytes whole, trailer and all, a byte at a time, and in pieces of a few
/// odd sizes all give the run model the file gives, closed by its end block.
#[test]
fn bytes_in_any_pieces_read_as_the_file_does() {
    let bytes = recorded();
    let file = read_as_file(bytes, "whole.worldview").expect("the file reads");
    assert!(!file.truncated, "the run closed its recording");
    let expected = json(&file.profile);
    for size in [bytes.len(), 1, 7, 1_000, 65_537] {
        let mut incoming = IncomingRecording::default();
        for piece in bytes.chunks(size) {
            incoming.push(piece).unwrap_or_else(|e| panic!("in pieces of {size} bytes: {e}"));
        }
        assert!(incoming.has_ended(), "in pieces of {size} bytes, the end block is in");
        let read = incoming.into_recording().unwrap_or_else(|e| panic!("in pieces of {size} bytes: {e}"));
        assert!(!read.truncated, "in pieces of {size} bytes, the run's end is read");
        assert!(json(&read.profile) == expected, "in pieces of {size} bytes, another run model than the file's");
    }
}

/// At every frame boundary the slices taken, the counts they sum to and the
/// roster are what the file cut at that boundary reads. Bytes that end inside
/// a frame wait for the rest of it, and the scenario block and the end block
/// are in only once they are whole.
#[test]
fn at_every_frame_boundary_it_holds_what_the_file_cut_there_reads() {
    let bytes = recorded();
    let mut incoming = IncomingRecording::default();
    let mut taken = 0;
    for frame in frames(bytes) {
        let what = format!("the {} frame at {}", frame.tag, frame.offset);
        let (inside, end) = ((frame.offset + u64::from(frame.len) / 2) as usize, frame.end_offset() as usize);
        let before = json(&incoming.slices());
        incoming.push(&bytes[taken..inside]).unwrap_or_else(|e| panic!("half of {what}: {e}"));
        assert!(json(&incoming.slices()) == before, "half of {what} changes no slice");
        assert_eq!(incoming.slice_width().is_some(), frame.tag != Tag::Scenario, "half of {what}");
        assert_eq!(incoming.robots().is_some(), frame.tag != Tag::Scenario, "half of {what}");
        assert!(!incoming.has_ended(), "half of {what}");

        incoming.push(&bytes[inside..end]).unwrap_or_else(|e| panic!("the rest of {what}: {e}"));
        taken = end;
        let cut = read_as_file(&bytes[..end], "cut.worldview").unwrap_or_else(|e| panic!("cut after {what}: {e}"));
        assert!(json(&incoming.slices()) == json(&cut.profile.time_slices), "the slices through {what}");
        assert_eq!(incoming.cumulative_grid(), Some(&cut.profile.cumulative_grid), "the counts through {what}");
        let width = incoming.slice_width().map(|width| width.as_ms());
        assert_eq!(width, Some(cut.profile.metadata.slice_width_ms), "through {what}");
        let robots = incoming.robots().map(|robots| json(&robots));
        assert_eq!(robots, Some(json(&cut.profile.ids.robots)), "the roster through {what}");
        assert_eq!(incoming.has_ended(), !cut.truncated, "through {what}");
    }
    assert!(incoming.has_ended(), "the last frame is the end frame");
}

/// A frame whose bytes do not hash to the checksum its header records is
/// damage: it is refused once its last byte is in, rather than waited on, and
/// every call after that answers with the same refusal.
#[test]
fn a_frame_whose_checksum_fails_is_refused_once_it_is_whole() {
    let mut bytes = recorded().to_vec();
    let frames = frames(&bytes);
    let damaged = frames.iter().filter(|frame| frame.tag == Tag::Delta).nth(3).expect("the run states four slices");
    let last = damaged.end_offset() as usize - 1;
    bytes[last] ^= 0x01;
    let mut incoming = IncomingRecording::default();
    for byte in &bytes[..last] {
        incoming.push(slice::from_ref(byte)).expect("every frame ahead of it, and it until it is whole");
    }
    let refused = incoming.push(&bytes[last..=last]).expect_err("a whole frame that fails its checksum");
    assert!(refused.contains("crc32c") && refused.contains(&damaged.offset.to_string()), "{refused}");
    assert_eq!(incoming.push(&bytes[last + 1..]), Err(refused.clone()), "the damage stands");
    assert_eq!(incoming.into_recording().err(), Some(refused), "and the recording does not read");
}

/// A recording's frames run forward in simtime: a frame stamped before the
/// one ahead of it is damage, refused as the file's own scan refuses it.
#[test]
fn a_frame_stamped_before_the_one_ahead_of_it_is_refused() {
    let bytes = recorded();
    let end = *frames(bytes).last().expect("the recording has frames");
    assert_eq!(end.tag, Tag::End, "the last frame is the end frame");
    let (frame, _) = Frame::decode_at(&bytes[end.offset as usize..], end.offset).expect("the end frame decodes");
    let body = frame.contents(end.offset).expect("its body");
    let restamped = Frame::encode(Tag::End, Ms::ZERO, &body, true).expect("encodes");
    let backwards = [&bytes[..end.offset as usize], &restamped].concat();
    let refused = IncomingRecording::default().push(&backwards).expect_err("an end stamped at the run's start");
    assert!(refused.contains("cannot follow"), "{refused}");
    let Err(scanned) = read_as_file(&backwards, "backwards.worldview") else { panic!("the file reads") };
    assert!(scanned.contains("cannot follow"), "{scanned}");
}

/// A recording opens with its one scenario block: bytes that begin with any
/// other frame open nothing, and a second scenario block is damage.
#[test]
fn a_recording_opens_with_its_one_scenario_block() {
    let bytes = recorded();
    let opening = &bytes[..frames(bytes)[0].end_offset() as usize];
    let refused = IncomingRecording::default()
        .push(&bytes[opening.len()..])
        .expect_err("the frame after the block opens nothing");
    assert!(refused.contains("first frame"), "{refused}");
    let mut incoming = IncomingRecording::default();
    incoming.push(opening).expect("the scenario block opens the recording");
    let refused = incoming.push(opening).expect_err("a second scenario block");
    assert!(refused.contains("is a second"), "{refused}");
}

/// How many orders the run's scenario resolved to is known once the scenario
/// block is in, before any slice: what the window shows a live run's fully
/// picked orders out of.
#[test]
fn the_order_total_is_known_from_the_scenario_block() {
    let bytes = recorded();
    let whole = read_as_file(bytes, "orders.worldview").expect("the file reads");
    let mut incoming = IncomingRecording::default();
    assert_eq!(incoming.orders(), None, "nothing is known before the scenario block");
    incoming.push(&bytes[..frames(bytes)[0].end_offset() as usize]).expect("the scenario block");
    assert!(incoming.slices().is_empty(), "no slice is in yet");
    let resolved = whole.profile.workload.orders.len();
    assert!(resolved > 0, "the fixture resolves orders");
    assert_eq!(incoming.orders(), Some(resolved));
}

/// A run capped with `--orders` takes no more of its scenario's orders than
/// the cap, and a loaded scenario's block states its whole list: the total is
/// the cap the block's own configuration states.
#[test]
fn a_capped_runs_order_total_is_its_cap() {
    let profile = read_as_file(recorded(), "uncapped.worldview").expect("the file reads").profile;
    let listed = profile.workload.orders.len();
    assert!(listed > 1, "the fixture resolves orders enough to cap");
    let scenario = RunScenario {
        name: profile.name,
        metadata: profile.metadata,
        documents: profile.documents,
        ids: profile.ids,
        sim_config: SimConfig { max_orders: Some(listed - 1), ..profile.sim_config },
        scenario: ScenarioSource::Loaded {
            dir_name: "orders".to_owned(),
            skus: profile.workload.skus,
            orders: profile.workload.orders,
        },
        workload: ResolvedWorkload::default(),
        vtm_fleet: profile.vtm_fleet,
        run: String::new(),
        attempt: 0,
    };
    let path = scratch("capped.worldview");
    let recording = RunRecording::create(&path, None, scenario).expect("the recording opens");
    recording
        .finish(&MetricsOutput::default(), &mut SummaryStats::default(), &SimResult::default())
        .expect("a run that made no slice still closes");
    let bytes = std::fs::read(&path).expect("the recording");
    let _ = std::fs::remove_file(&path);
    let mut incoming = IncomingRecording::default();
    incoming.push(&bytes[..frames(&bytes)[0].end_offset() as usize]).expect("the scenario block");
    assert_eq!(incoming.orders(), Some(listed - 1));
}
