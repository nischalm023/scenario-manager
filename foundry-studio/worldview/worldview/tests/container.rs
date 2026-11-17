//! Reading back a run written frame by frame: that a seek lands where the
//! format says at every boundary, that a file cut off mid-write opens as the
//! prefix it really is, that a byte altered anywhere is refused rather than
//! read as something else, and that a live file grows under a reader.

mod support;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use support::{PLAYBACK, RunShape, SCALARS, SLICE_WIDTH_MS, build, sections};
use tempfile::TempDir;
use worldview::container::frame::Tag;
use worldview::container::index::{Index, IndexRecord, OrderCheck, Trailer};
use worldview::container::recover::{load, scan};
use worldview::container::writer::Writer;
use worldview::{Error, Frame, Ms, Reader, Sections};

fn scanned(bytes: &[u8]) -> Index {
    scan(bytes, 0, &mut OrderCheck::default()).expect("the frames scan").0
}

fn small() -> RunShape {
    RunShape { cadence: 2, slices: 6, seed: 3, finished: true }
}

fn on_disk(bytes: &[u8]) -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = directory.path().join("run.worldview");
    std::fs::write(&path, bytes).expect("write the run");
    (directory, path)
}

#[test]
fn every_frame_the_index_names_reads_back_whole() {
    let run = build(RunShape::default());
    let mut reader = Reader::from_bytes(&run.bytes).expect("opens");
    for record in reader.index().records().to_vec() {
        let frame = reader.frame_at(record.offset).expect("the frame is where the record says");
        assert_eq!(frame.byte_len(), u64::from(record.len), "at offset {}", record.offset);
        assert_eq!(frame.header.t, record.t);
        assert_eq!(frame.header.tag, record.tag);
        if record.tag.carries_sections() {
            let slice = (record.t.0 / SLICE_WIDTH_MS) as u32;
            let read = reader.sections_at(&record).expect("the sections decode");
            for (id, body) in sections(record.tag, slice, run.shape.seed) {
                assert_eq!(read.get(id), Some(body.as_slice()), "section {id} of the frame at {}", record.t);
            }
        }
    }
}

#[test]
fn a_seek_lands_on_the_right_keyframe_at_every_boundary() {
    let run = build(RunShape::default());
    let reader = Reader::from_bytes(&run.bytes).expect("opens");
    for keyframe in &run.keyframes {
        for step in [0u64, 1, SLICE_WIDTH_MS, SLICE_WIDTH_MS * 3] {
            let asked = Ms(keyframe.0 + step);
            let plan = reader.seek(asked).expect("a plan");
            assert_eq!(plan.keyframe.t, *keyframe, "seeking to {asked}");
        }
        if keyframe.0 > 0 {
            let just_before = Ms(keyframe.0 - 1);
            let plan = reader.seek(just_before).expect("a plan");
            assert!(plan.keyframe.t < *keyframe, "a millisecond early is still the keyframe before it");
        }
    }
}

#[test]
fn a_seek_names_exactly_the_deltas_stamped_at_or_before_the_instant() {
    let run = build(RunShape::default());
    let reader = Reader::from_bytes(&run.bytes).expect("opens");
    let asked = Ms(305_000);
    let plan = reader.seek(asked).expect("a plan");

    assert_eq!(plan.keyframe.t, Ms(300_000), "the boundary at slice 30");
    let stamps: Vec<u64> = plan.deltas.iter().map(|delta| delta.t.0).collect();
    assert_eq!(stamps, vec![300_000], "the slice holding 305000 ms, and nothing after it");
    assert_eq!(plan.reaches(), Ms(300_000));

    let later = reader.seek(Ms(345_000)).expect("a plan");
    let stamps: Vec<u64> = later.deltas.iter().map(|delta| delta.t.0).collect();
    assert_eq!(stamps, vec![300_000, 310_000, 320_000, 330_000, 340_000]);
    assert!(later.deltas.iter().all(|delta| delta.t <= Ms(345_000)));
}

#[test]
fn a_seek_plan_names_byte_ranges_a_ranged_reader_can_fetch_on_their_own() {
    let run = build(RunShape::default());
    let reader = Reader::from_bytes(&run.bytes).expect("opens");
    let plan = reader.seek(Ms(455_000)).expect("a plan");
    let ranges = plan.ranges();

    assert!(ranges.windows(2).all(|pair| pair[0].0 < pair[1].0), "a ranged reader fetches them in file order");
    for (offset, len) in ranges {
        let fetched = &run.bytes[offset as usize..offset as usize + len as usize];
        let (frame, consumed) = Frame::decode_at(fetched, offset).expect("the range holds one whole frame");
        assert_eq!(consumed, fetched.len(), "and nothing but that frame");
        assert!(frame.contents(offset).is_ok(), "which decompresses on its own");
    }
}

#[test]
fn a_seek_carries_every_declaration_made_at_or_before_the_instant() {
    let run = build(RunShape::default());
    let reader = Reader::from_bytes(&run.bytes).expect("opens");
    let asked = Ms(475_000);
    let plan = reader.seek(asked).expect("a plan");

    let expected: Vec<Ms> = run.declares.iter().copied().filter(|t| *t <= asked).collect();
    assert_eq!(plan.declares.iter().map(|record| record.t).collect::<Vec<_>>(), expected);
    assert!(expected.len() > 1, "the fixture declares more than once, so cumulative means something");
    assert!(plan.declares.iter().any(|record| record.t < plan.keyframe.t), "declarations from before the keyframe too");
}

#[test]
fn a_seek_before_the_first_keyframe_resolves_to_the_first_keyframe() {
    let scenario = Frame::encode(Tag::Scenario, Ms::ZERO, b"the run", true).expect("encodes");
    let mut bytes = scenario;
    bytes.extend(Frame::encode(Tag::Delta, Ms(10_000), &[0], true).expect("encodes"));
    bytes.extend(Frame::encode(Tag::Keyframe, Ms(20_000), &[0], true).expect("encodes"));
    let reader = Reader::from_bytes(&bytes).expect("opens");

    let plan = reader.seek(Ms::ZERO).expect("a plan");
    assert_eq!(plan.keyframe.t, Ms(20_000), "the file states nothing earlier");
    assert!(plan.deltas.is_empty(), "and nothing before the keyframe is applied to it");
}

#[test]
fn a_file_with_no_keyframe_says_so_rather_than_guessing() {
    let mut bytes = Frame::encode(Tag::Scenario, Ms::ZERO, b"the run", true).expect("encodes");
    bytes.extend(Frame::encode(Tag::Delta, Ms(10_000), &[0], true).expect("encodes"));
    let reader = Reader::from_bytes(&bytes).expect("opens");
    assert!(matches!(reader.seek(Ms(10_000)).unwrap_err(), Error::NoKeyframe));
}

#[test]
fn a_file_cut_off_at_any_byte_opens_as_its_verified_prefix() {
    let run = build(small());
    let whole = scanned(&run.bytes);

    for cut in 0..run.bytes.len() {
        let mut reader = Reader::from_bytes(&run.bytes[..cut])
            .unwrap_or_else(|e| panic!("cut at {cut} is a torn tail, not damage: {e}"));
        let kept = reader.index().records().to_vec();
        assert_eq!(kept, whole.records()[..kept.len()], "cut at {cut} kept frames the run never wrote");
        for record in &kept {
            let frame = reader.frame_at(record.offset).expect("a verified frame reads back whole");
            assert_eq!(frame.byte_len(), u64::from(record.len), "cut at {cut}");
        }
    }
}

#[test]
fn a_single_altered_byte_is_refused_as_damage_and_never_misread() {
    let run = build(small());
    let third = scanned(&run.bytes).records()[2].offset as usize;
    let magic = worldview::MAGIC.len();

    for (what, at) in [
        ("the tag", third),
        ("the flags", third + 1),
        ("a reserved byte", third + 2),
        ("the checksum", third + 16),
        ("a payload byte", third + 24),
        ("the magic", 20),
        ("the version", 20 + magic),
    ] {
        let mut altered = run.bytes.clone();
        altered[at] ^= 0x7f;
        let refused = match Reader::from_bytes(&altered) {
            Err(error) => Some(error),
            Ok(mut reader) => reader.frames_from(0).find_map(|step| step.err()),
        };
        assert!(refused.is_some(), "{what} altered at byte {at} was read as if nothing had happened");
    }
}

#[test]
fn a_valid_trailer_opens_the_file_without_a_scan() {
    let run = build(small());
    let (_directory, path) = on_disk(&run.bytes);
    let loaded = load(&path).expect("opens");

    assert!(loaded.through_trailer, "a finished run is opened through its trailer");
    assert_eq!(loaded.index.records(), scanned(&run.bytes).records(), "and finds what a scan would");
}

#[test]
fn a_finished_file_in_memory_opens_through_its_trailer_rather_than_by_scanning() {
    let run = build(small());
    let third = scanned(&run.bytes).records()[2].offset as usize;
    let mut altered = run.bytes.clone();
    altered[third + 24] ^= 0x7f;

    let mut reader = Reader::from_bytes(&altered).expect("the trailer names the frames, so opening reads no frame");
    assert_eq!(reader.index().records(), scanned(&run.bytes).records(), "the index is the block's");
    assert!(reader.frame_at(third as u64).is_err(), "and the damage surfaces when that frame is read");
}

#[test]
fn asking_a_frame_that_carries_no_sections_for_them_is_a_caller_error_and_not_damage() {
    let run = build(small());
    let mut reader = Reader::from_bytes(&run.bytes).expect("opens");
    let scenario = reader.index().records()[0];
    let error = reader.sections_at(&scenario).expect_err("the scenario block is one payload of its own");
    assert!(matches!(error, Error::NotASectionedFrame { offset: 0, tag: Tag::Scenario }), "got {error}");
}

#[test]
fn a_trailer_that_lies_is_ignored_and_the_frames_are_scanned() {
    let run = build(small());
    let frames = scanned(&run.bytes);

    for (what, at, value) in [
        ("an index offset past the end of the file", 8usize, u64::MAX),
        ("an index offset naming the scenario frame", 8, 0),
        ("an end offset that is not where the end frame is", 16, 24),
    ] {
        let mut bytes = run.bytes.clone();
        let tail = bytes.len() - 24;
        bytes[tail + at..tail + at + 8].copy_from_slice(&value.to_le_bytes());
        let (_directory, path) = on_disk(&bytes);
        let loaded = load(&path).expect("opens by scanning");
        assert!(!loaded.through_trailer, "{what} should send the reader to the frames");
        assert_eq!(loaded.index.records(), frames.records(), "{what}");
    }
}

#[test]
fn a_live_file_opens_by_scanning_and_refresh_takes_in_what_was_appended() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = directory.path().join("live.worldview");
    let mut writer = Writer::create(&path).expect("creates the file");
    writer.append_scenario(Ms::ZERO, b"the run").expect("the scenario frame");
    writer.append_sections(Tag::Keyframe, Ms::ZERO, &[(PLAYBACK, b"whole".as_slice())]).expect("a keyframe");

    let loaded = load(&path).expect("opens");
    assert!(!loaded.through_trailer, "a live file has no trailer yet");
    let mut reader = Reader::open(&path).expect("opens");
    assert_eq!(reader.index().len(), 2);
    assert_eq!(reader.refresh().expect("nothing new yet"), 0);

    for slice in 1..4u64 {
        let at = Ms(slice * SLICE_WIDTH_MS);
        writer.append_sections(Tag::Delta, at, &[(SCALARS, b"values".as_slice())]).expect("a delta");
    }
    assert_eq!(reader.refresh().expect("three more frames"), 3);
    assert_eq!(reader.live_edge(), Some(Ms(30_000)));
    assert_eq!(reader.seek(Ms(25_000)).expect("a plan").deltas.len(), 2, "the deltas of slices 1 and 2");
}

#[test]
fn a_scenario_frame_anywhere_but_first_is_refused() {
    let mut bytes = Frame::encode(Tag::Scenario, Ms::ZERO, b"the run", true).expect("encodes");
    bytes.extend(Frame::encode(Tag::Scenario, Ms::ZERO, b"again", true).expect("encodes"));
    let error = Reader::from_bytes(&bytes).expect_err("one file, one scenario frame");
    assert!(matches!(error, Error::ScenarioNotFirst { .. }), "got {error}");

    let headless = Frame::encode(Tag::Keyframe, Ms::ZERO, b"whole", true).expect("encodes");
    assert!(Reader::from_bytes(&headless).is_err(), "and a file that opens with anything else is not one either");
}

#[test]
fn a_keyframe_behind_the_delta_of_its_own_instant_is_refused_by_the_reader_too() {
    let mut bytes = Frame::encode(Tag::Scenario, Ms::ZERO, b"the run", true).expect("encodes");
    bytes.extend(Frame::encode(Tag::Delta, Ms(30_000), &[0], true).expect("encodes"));
    bytes.extend(Frame::encode(Tag::Keyframe, Ms(30_000), &[0], true).expect("encodes"));
    let error = Reader::from_bytes(&bytes).expect_err("a seek to 30000 ms would step over the keyframe");
    assert!(matches!(error, Error::FrameOutOfOrder { t: Ms(30_000), .. }), "got {error}");
}

#[test]
fn a_finished_file_and_the_same_bytes_in_memory_read_the_same() {
    let run = build(small());
    let (_directory, path) = on_disk(&run.bytes);
    let through_file = Reader::open(&path).expect("opens").index().records().to_vec();
    let in_memory = Reader::from_bytes(&run.bytes).expect("opens").index().records().to_vec();
    assert_eq!(through_file, in_memory);
    assert_eq!(Path::new(&path).extension().and_then(|e| e.to_str()), Some("worldview"));
}

/// The three real frames a hand-built file opens with.
fn opening_frames() -> Vec<u8> {
    let mut bytes = Frame::encode(Tag::Scenario, Ms::ZERO, b"the run", true).expect("encodes");
    let whole = Sections::encode(&[(PLAYBACK, b"whole".as_slice())]);
    bytes.extend(Frame::encode(Tag::Keyframe, Ms::ZERO, &whole, true).expect("encodes"));
    let step = Sections::encode(&[(PLAYBACK, b"step".as_slice())]);
    bytes.extend(Frame::encode(Tag::Delta, Ms(10_000), &step, true).expect("encodes"));
    bytes
}

/// A file whose index block says whatever the test wants, closed by a trailer
/// that points at the block and the end frame truthfully — so the only thing
/// a reader can object to is the block's own records.
fn file_with_block(block: &Index, closing: Ms) -> Vec<u8> {
    let mut bytes = opening_frames();
    let index_offset = bytes.len() as u64;
    bytes.extend(Frame::encode(Tag::Index, closing, &block.block_bytes(), true).expect("encodes"));
    let end_offset = bytes.len() as u64;
    bytes.extend(Frame::encode(Tag::End, closing, b"stopped", true).expect("encodes"));
    bytes.extend(Trailer { index_offset, end_offset }.to_bytes());
    bytes
}

fn rebuilt(records: &[IndexRecord]) -> Index {
    let mut index = Index::new();
    for record in records {
        index.push(*record);
    }
    index
}

#[test]
fn a_block_whose_records_do_not_describe_the_frames_sends_the_reader_to_the_frames() {
    let truthful = scanned(&opening_frames()).records().to_vec();
    let doctored = |change: fn(&mut Vec<IndexRecord>)| {
        let mut records = truthful.clone();
        change(&mut records);
        rebuilt(&records)
    };

    for (what, block) in [
        ("a record claiming a frame shorter than a header", doctored(|records| records[2].len = 0)),
        ("records running backwards in time", doctored(|records| records[1].t = Ms(20_000))),
        ("an end frame named inside the block", doctored(|records| records[2].tag = Tag::End)),
    ] {
        let bytes = file_with_block(&block, Ms(10_000));
        let (_directory, path) = on_disk(&bytes);
        let loaded = load(&path).expect("opens by scanning");
        assert!(!loaded.through_trailer, "{what} should send the reader to the frames");
        assert_eq!(loaded.index.records(), scanned(&bytes).records(), "{what}");
    }
}

#[test]
fn a_closing_frame_stamped_behind_the_block_is_refused_rather_than_trusted() {
    let bytes = file_with_block(&rebuilt(scanned(&opening_frames()).records()), Ms::ZERO);
    let (_directory, path) = on_disk(&bytes);
    let error = load(&path).expect_err("the index and end frames begin before the delta the block ends with");
    assert!(matches!(error, Error::FrameOutOfOrder { .. }), "got {error}");
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new().append(true).open(path).expect("opens the live file for appending");
    file.write_all(bytes).expect("appends");
    file.flush().expect("flushes");
}

fn live_file(directory: &Path, last: Ms) -> PathBuf {
    let path = directory.join("live.worldview");
    let mut writer = Writer::create(&path).expect("creates the file");
    writer.append_scenario(Ms::ZERO, b"the run").expect("the scenario frame");
    writer.append_sections(Tag::Keyframe, Ms::ZERO, &[(PLAYBACK, b"whole".as_slice())]).expect("a keyframe");
    writer.append_sections(Tag::Delta, last, &[(SCALARS, b"values".as_slice())]).expect("a delta");
    path
}

#[test]
fn refresh_waits_out_a_half_written_frame_and_raises_on_a_damaged_one() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = live_file(directory.path(), Ms(10_000));
    let mut reader = Reader::open(&path).expect("opens");
    assert_eq!(reader.index().len(), 3);

    let step = Sections::encode(&[(PLAYBACK, b"step".as_slice())]);
    let frame = Frame::encode(Tag::Delta, Ms(20_000), &step, true).expect("encodes");
    let half = frame.len() / 2;
    append_bytes(&path, &frame[..half]);
    assert_eq!(reader.refresh().expect("a frame mid-write is not damage"), 0);

    let mut rest = frame[half..].to_vec();
    let last = rest.len() - 1;
    rest[last] ^= 0x01;
    append_bytes(&path, &rest);
    let error = reader.refresh().expect_err("a whole frame whose checksum fails is damage");
    assert!(matches!(error, Error::ChecksumMismatch { .. }), "got {error}");
}

#[test]
fn refresh_keeps_the_order_rule_going_from_where_the_file_already_reached() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = live_file(directory.path(), Ms(30_000));
    let mut reader = Reader::open(&path).expect("opens");

    let whole = Sections::encode(&[(PLAYBACK, b"whole".as_slice())]);
    append_bytes(&path, &Frame::encode(Tag::Keyframe, Ms(30_000), &whole, true).expect("encodes"));
    let error = reader.refresh().expect_err("a seek to 30000 ms would step over the delta already written");
    assert!(matches!(error, Error::FrameOutOfOrder { t: Ms(30_000), .. }), "got {error}");
}

/// A run told to write into an output directory that does not exist yet makes
/// it, rather than simulating for an hour and then failing on the last step.
#[test]
fn creating_a_file_makes_the_folder_it_goes_in() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = directory.path().join("runs").join("today").join("run.worldview");
    let writer = Writer::create(&path).expect("the folders are made on the way");
    drop(writer);
    assert!(path.exists(), "{} was not created", path.display());
}
