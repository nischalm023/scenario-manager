//! A run watched through its recording reads as its file does. The small
//! generated floor is run once through the harness with the window's tap, and
//! the pieces the tap was handed reach a window one at a time, through the
//! poll a frame runs: at every piece, what the window draws is what the file
//! cut there reads.

use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};

use desgraph::{RobotType, WorldPos};
use dessim::StopSignal;
use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::metrics::profile::SliceRecord;
use dessim::metrics::worldview::{Recording, read_worldview};
use dessim::observer::time_slice::BotLocation;
use dessim::runner::RunnerHooks;
use serde::Serialize;
use serde_json::Value;
use worldview::{Reader, Tag};

use super::{FromRun, LiveRun, RunTap, UtilizationSeen};
use crate::app::{DesvisApp, grid_data};
use crate::types::{DataSource, SimStatus};

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("desvis-live-run-{}-{name}", std::process::id()))
}

/// One run of the small generated floor: the pieces the window's tap was
/// handed, in order, and the file the run wrote.
struct Recorded {
    pieces: Vec<Vec<u8>>,
    file: Vec<u8>,
}

fn recorded() -> &'static Recorded {
    static RUN: OnceLock<Recorded> = OnceLock::new();
    RUN.get_or_init(|| {
        let spec =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../sim/dessim/tests/fixtures/minimal_easyloop_spec.json");
        let path = scratch("run.worldview");
        let (tap, tapped) = mpsc::channel();
        let target =
            RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: Some(Box::new(RunTap(tap))) };
        let prepared = prepare(&spec, &RunOverrides::default()).expect("the fixture prepares");
        prepared.execute(RunnerHooks::default(), Some(target)).expect("the run is recorded");
        let pieces = tapped
            .try_iter()
            .map(|message| match message {
                FromRun::Bytes(bytes) => bytes,
                FromRun::Ended(_) => panic!("a tap hands over bytes, and only the run's thread says how it ended"),
            })
            .collect();
        let file = std::fs::read(&path).expect("the recording");
        let _ = std::fs::remove_file(&path);
        Recorded { pieces, file }
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

fn json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("the run model is JSON")
}

/// Enough cells for every cell the run counts visits on.
fn cells(recording: &Recording) -> usize {
    recording.profile.ids.cells.iter().map(|(cell, _)| cell.0 as usize + 1).max().expect("the floor has cells")
}

/// A window watching a run whose messages the test hands it.
fn watching(cells: usize) -> (DesvisApp, Sender<FromRun>, Arc<StopSignal>) {
    let mut app = DesvisApp::for_tests(cells);
    let (to_window, from_run) = mpsc::channel();
    let stop = StopSignal::new();
    app.live_run = Some(LiveRun::new(from_run, stop.clone(), UtilizationSeen::default()));
    app.sim_status = SimStatus::Running;
    app.data_source = DataSource::Live;
    (app, to_window, stop)
}

/// Hand the window `pieces`, each as the tap handed it, and let it poll.
fn hand(app: &mut DesvisApp, to_window: &Sender<FromRun>, pieces: &[Vec<u8>]) {
    for piece in pieces {
        to_window.send(FromRun::Bytes(piece.clone())).expect("the window is watching");
    }
    app.poll_live_run();
}

/// Each robot stands where the newest slice that moved any robot leaves it.
fn assert_robots_at_the_live_edge(app: &DesvisApp, at: &str) {
    let Some(newest) = app.time_series.iter().rev().find(|slice| !slice.bot_locations.is_empty()) else { return };
    for (&robot, location) in &newest.bot_locations {
        let left = location.steps.last().map_or(location.start_pos, |step| step.pos);
        let marker = app.bot_positions.iter().find(|marker| marker.bot_id == robot).expect("every robot is drawn");
        assert_eq!(marker.pos_mm, [left.x_mm, left.y_mm], "{at}: robot {robot} stands at the live edge");
    }
}

/// Every piece the tap hands the window is folded in as it comes: the bytes
/// the window received are the file's, and at every piece its slices, slice
/// width, heat and robots are what the file cut there reads. Once the run has
/// returned, the window holds the profile the whole file reads.
#[test]
fn a_watched_run_draws_at_every_piece_what_its_file_cut_there_reads() {
    let run = recorded();
    // The cases the intake exists for: a metric the run declares after its
    // first slice, and a keyframe after the first.
    let tags: Vec<Tag> =
        Reader::from_bytes(&run.file).expect("the file opens").index().records().iter().map(|r| r.tag).collect();
    let first_delta = tags.iter().position(|tag| *tag == Tag::Delta).expect("the run states a slice");
    assert!(tags[first_delta..].contains(&Tag::Declare), "the fixture declares a metric after its first slice");
    assert!(tags.iter().filter(|tag| **tag == Tag::Keyframe).count() >= 2, "the fixture writes a second keyframe");
    let whole = read_as_file(&run.file, "whole.worldview").expect("the file reads");
    let cells = cells(&whole);
    let (mut app, to_window, _stop) = watching(cells);
    let mut received = Vec::new();
    for piece in &run.pieces {
        received.extend_from_slice(piece);
        hand(&mut app, &to_window, std::slice::from_ref(piece));
        let at = format!("after {} bytes", received.len());
        assert!(matches!(app.sim_status, SimStatus::Running), "{at}: the run is still going");
        let Ok(cut) = read_as_file(&received, "cut.worldview") else {
            assert!(app.time_series.is_empty(), "{at}: nothing is drawn before the scenario block is whole");
            continue;
        };
        assert!(json(&app.time_series) == json(&cut.profile.time_slices), "{at}: the slices");
        if app.time_series.is_empty() {
            continue;
        }
        assert_eq!(app.slice_width_ms, cut.profile.metadata.slice_width_ms, "{at}: the slice width");
        let heat = grid_data::dynamic_grid_to_heatmap(&cut.profile.cumulative_grid, cells);
        let shown = app.heatmap_data.as_ref().expect("a live run shows its heat");
        assert_eq!(shown.vtm_counts, heat.vtm_counts, "{at}: the DA50 heat");
        assert_eq!(shown.htm_counts, heat.htm_counts, "{at}: the D30 heat");
        assert_robots_at_the_live_edge(&app, &at);
    }
    assert!(received == run.file, "the bytes the window received are the file's");

    to_window.send(FromRun::Ended(Ok(()))).expect("the window is watching");
    app.poll_live_run();
    assert!(matches!(app.sim_status, SimStatus::Complete(_)), "the run returned: {:?}", app.sim_status);
    let DataSource::Profile(profile) = &app.data_source else { panic!("a run that returned is its profile") };
    assert!(json(profile.as_ref()) == json(&whole.profile), "the profile is the one the whole file reads");
    assert!(app.live_run.is_none(), "nothing is watched once the run has returned");
}

/// Slices that arrive while the cursor is engaged leave the cursor, the robots
/// it stands and the heat it shows where they were: the series grows, and
/// nothing teleports to the live edge.
#[test]
fn slices_arriving_leave_an_engaged_cursor_where_it_was() {
    let run = recorded();
    let (mut app, to_window, _stop) = watching(cells(&read_as_file(&run.file, "engaged.worldview").unwrap()));
    let half = run.pieces.len() / 2;
    hand(&mut app, &to_window, &run.pieces[..half]);
    assert!(app.time_series.len() > 2, "half the run states a few slices");
    let edge = app.time_series.len() as f64 * app.slice_width_ms as f64;
    app.engage_cursor(app.slice_width_ms as f64 * 1.5, edge);
    let robots: Vec<(u32, [f64; 2])> = app.bot_positions.iter().map(|marker| (marker.bot_id, marker.pos_mm)).collect();
    let heat = app.heatmap_data.as_ref().map(|heat| heat.htm_counts.clone());
    let slices = app.time_series.len();

    hand(&mut app, &to_window, &run.pieces[half..]);
    assert!(app.time_series.len() > slices, "slices went on arriving");
    assert_eq!(app.time_cursor_ms, app.slice_width_ms as f64 * 1.5, "the cursor stays where it was put");
    let after: Vec<(u32, [f64; 2])> = app.bot_positions.iter().map(|marker| (marker.bot_id, marker.pos_mm)).collect();
    assert_eq!(after, robots, "the robots stay where the cursor stands them");
    assert_eq!(app.heatmap_data.as_ref().map(|heat| heat.htm_counts.clone()), heat, "the cursor's heat stays");
}

/// A run that fails shows the failure's words and keeps what the window had
/// received before it, which stays to be looked at.
#[test]
fn a_run_that_fails_keeps_what_was_received() {
    let run = recorded();
    let (mut app, to_window, _stop) = watching(cells(&read_as_file(&run.file, "failed.worldview").unwrap()));
    hand(&mut app, &to_window, &run.pieces[..run.pieces.len() / 2]);
    let received = json(&app.time_series);
    to_window.send(FromRun::Ended(Err("the run panicked: no path".into()))).expect("the window is watching");
    app.poll_live_run();
    assert!(matches!(&app.sim_status, SimStatus::Error(words) if words == "the run panicked: no path"));
    assert!(json(&app.time_series) == received, "the slices received stay");
    assert!(matches!(app.data_source, DataSource::Live), "a failed run is no profile");
    assert!(app.live_run.is_some(), "its Utilization panel stays");
}

/// Bytes the window cannot read end the reading, and stop the run, since
/// nothing would read what it went on to write.
#[test]
fn bytes_the_window_cannot_read_stop_the_run() {
    let run = recorded();
    let mut damaged = run.pieces[0].clone();
    damaged[0] ^= 0xff;
    let (mut app, to_window, stop) = watching(4);
    hand(&mut app, &to_window, &[damaged]);
    assert!(
        matches!(&app.sim_status, SimStatus::Error(words) if words.contains("cannot be read")),
        "{:?}",
        app.sim_status
    );
    assert!(stop.is_stopped(), "the run is told to stop");
}

/// A robot standing still in the newest slice that moved any robot faces the
/// heading that slice states it started on, as playback draws it: north after
/// a northward move, not east, whatever the window drew before.
#[test]
fn a_robot_standing_at_the_live_edge_faces_as_playback_draws_it() {
    let standing = BotLocation {
        motion: Some(Vec::new()),
        start_cell: 0,
        start_pos: WorldPos::default(),
        start_z_mm: 0.0,
        start_toward: None,
        start_heading_rad: Some(FRAC_PI_2),
        robot_type: RobotType::Htm,
        steps: Default::default(),
    };
    let slice = SliceRecord {
        start_ms: 0,
        metrics: Default::default(),
        grid: Default::default(),
        bot_locations: [(7, standing)].into_iter().collect(),
        relay_state: Default::default(),
        tote_state: Default::default(),
        station_diag: Default::default(),
    };
    let mut app = DesvisApp::for_tests(4);
    app.time_series.push(slice);
    app.stand_robots_at_live_edge();
    let marker = app.bot_positions.iter().find(|marker| marker.bot_id == 7).expect("robot 7 is drawn");
    assert_eq!(marker.heading_rad, FRAC_PI_2);
}
