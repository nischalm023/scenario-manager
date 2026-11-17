//! Both writers of a workbook write the same book: desvis from the run model
//! the harness returns as a run ends, the `workbook` command from the run's
//! recording.
//!
//! A recording states each journal row in the frame written once B has passed
//! it, so the rows reach its reader in another order than the run appended
//! them. Every sheet that prints a journal orders the rows itself, so a book
//! reads the same from either.

use std::io::{Cursor, Read};
use std::path::Path;

use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::metrics::profile::PerfProfile;
use dessim::metrics::worldview::read_worldview;
use dessim::runner::RunnerHooks;
use workbook::{XlsxData, build_workbook};
use zip::ZipArchive;

/// Every part of the book built from `profile`, by name, but the one stating
/// when the book was made.
fn book(profile: &PerfProfile) -> Vec<(String, String)> {
    let mut workbook = build_workbook(&XlsxData::from_profile(profile)).expect("the book builds");
    let mut archive = ZipArchive::new(Cursor::new(workbook.save_to_buffer().expect("the book saves"))).unwrap();
    let mut parts = Vec::new();
    for index in 0..archive.len() {
        let mut part = archive.by_index(index).unwrap();
        if part.name() == "docProps/core.xml" {
            continue;
        }
        let mut text = String::new();
        part.read_to_string(&mut text).unwrap();
        parts.push((part.name().to_owned(), text));
    }
    parts
}

#[test]
fn a_book_from_the_recording_is_the_book_from_the_run() {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sim/dessim/tests/fixtures/minimal_easyloop_spec.json");
    let path = std::env::temp_dir().join(format!("same-book-{}.worldview", std::process::id()));
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    let prepared = prepare(&spec, &RunOverrides::default()).expect("the fixture prepares");
    let (run, _) = prepared.execute(RunnerHooks::default(), Some(target)).expect("the fixture runs and is recorded");
    let recording = read_worldview(&path).expect("the recording reads");
    std::fs::remove_file(&path).ok();

    let legs = |profile: &PerfProfile| serde_json::to_value(&profile.summary.htm_leg_log).unwrap();
    assert_ne!(
        legs(&run),
        legs(&recording.profile),
        "the fixture's legs have to reach the recording in another order than the run appended them, or this proves \
         nothing"
    );
    let (from_run, from_recording) = (book(&run), book(&recording.profile));
    assert_eq!(
        from_run.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        from_recording.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        "the same parts"
    );
    for ((name, was), (_, is)) in from_run.iter().zip(&from_recording) {
        assert!(was == is, "{name} differs between the book from the run and the book from its recording");
    }
}
