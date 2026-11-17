//! The `worldview` binary, run against a real file: that `inspect` lists the
//! frames the writer wrote and the sections inside them, that `seek` names
//! the frames a reader would read, and that `index --verify` compares the
//! block against the frames themselves.
//!
//! Only built when the `cli` feature is on, which is also the only time the
//! binary exists.
#![cfg(feature = "cli")]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use support::{RunShape, build};

/// A written run whose path is plain text, so a test hands the binary the
/// same argument strings a person would type.
fn fixture(directory: &Path, shape: RunShape) -> String {
    let path = directory.join("run.worldview");
    std::fs::write(&path, build(shape).bytes).expect("write the run");
    path.to_str().expect("a temp directory path is text").to_owned()
}

fn invoke(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_worldview")).args(args).output().expect("the binary runs")
}

fn run(args: &[&str]) -> String {
    let result = invoke(args);
    assert!(result.status.success(), "worldview {args:?} failed: {}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout).expect("output is text")
}

#[test]
fn inspect_says_how_the_file_opened_then_lists_every_frame_with_its_sections() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let file = fixture(directory.path(), RunShape::default());
    let printed = run(&["inspect", &file]);

    assert!(printed.contains("opened through the trailer"), "got:\n{printed}");
    assert_eq!(printed.matches("keyframe").count(), 4, "three boundaries and the totals line, got:\n{printed}");
    assert!(printed.contains("scenario"), "the opening frame is listed");
    assert!(printed.contains("zstd"), "the flags column says how a payload is stored");
    assert!(printed.contains("1:"), "a keyframe's sections are listed by id and length, got:\n{printed}");
    assert!(printed.contains("frames"), "the totals name their unit");
}

#[test]
fn inspect_of_a_file_still_being_written_says_it_was_scanned() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let file = fixture(directory.path(), RunShape { finished: false, ..RunShape::default() });
    let printed = run(&["inspect", &file]);
    assert!(printed.contains("opened by scanning"), "got:\n{printed}");
}

#[test]
fn seek_names_the_keyframe_the_declarations_and_the_deltas_it_would_read() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let file = fixture(directory.path(), RunShape::default());
    let printed = run(&["seek", &file, "455000"]);

    assert!(printed.contains("keyframe at       300000 ms"), "got:\n{printed}");
    assert!(printed.contains("declare  at"), "declarations are cumulative and travel with the plan, got:\n{printed}");
    assert!(printed.contains("16 deltas applied"), "slices 30 through 45, got:\n{printed}");
    assert!(printed.contains("reaching 450000 ms"), "got:\n{printed}");
}

#[test]
fn seek_past_the_last_frame_says_a_live_caller_would_tail() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let file = fixture(directory.path(), RunShape::default());
    let printed = run(&["seek", &file, "9000000"]);
    assert!(printed.contains("would tail"), "got:\n{printed}");
}

#[test]
fn index_verify_says_whether_the_block_names_the_frames_ahead_of_it() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let file = fixture(directory.path(), RunShape { cadence: 2, slices: 6, seed: 3, finished: true });
    let printed = run(&["index", &file, "--verify"]);

    assert!(printed.contains("from the index block"), "got:\n{printed}");
    assert!(printed.contains("verified: the block names"), "got:\n{printed}");
}

#[test]
fn index_of_a_file_without_a_block_falls_back_to_a_scan() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let file = fixture(directory.path(), RunShape { finished: false, ..RunShape::default() });
    let printed = run(&["index", &file, "--verify"]);

    assert!(printed.contains("no index block; from a scan"), "got:\n{printed}");
    assert!(printed.contains("nothing to verify"), "got:\n{printed}");
}

#[test]
fn inspect_of_a_damaged_file_lists_what_it_still_holds_and_says_where_the_damage_is() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = directory.path().join("torn.worldview");
    let mut bytes = build(RunShape { cadence: 2, slices: 6, seed: 3, finished: false }).bytes;
    bytes[200] ^= 0x7f;
    std::fs::write(&path, &bytes).expect("write the run");
    let result = invoke(&["inspect", path.to_str().expect("text")]);

    assert!(!result.status.success(), "a damaged file is not a clean read");
    let printed = String::from_utf8(result.stdout).expect("output is text");
    assert!(printed.contains("frames verified through byte"), "got:\n{printed}");
    assert!(printed.contains("scenario"), "the frames ahead of the damage are still listed, got:\n{printed}");
    let complaint = String::from_utf8_lossy(&result.stderr).to_string();
    assert!(complaint.contains("damage at offset"), "got: {complaint}");
}

#[test]
fn a_file_that_is_not_a_worldview_is_refused_with_what_it_found() {
    let directory = tempfile::tempdir().expect("a temp directory");
    let path = directory.path().join("other.worldview");
    std::fs::write(&path, vec![0xab; 4096]).expect("write the impostor");
    let result = invoke(&["inspect", path.to_str().expect("text")]);

    assert!(!result.status.success());
    let complaint = String::from_utf8_lossy(&result.stderr).to_string();
    assert!(complaint.contains("not a .worldview"), "got: {complaint}");
    assert!(complaint.contains("\\xab"), "and says what it found instead: {complaint}");
}
