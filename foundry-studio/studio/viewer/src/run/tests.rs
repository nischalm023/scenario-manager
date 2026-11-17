//! Within `run`, these tests drive a whole attachment's worth of frames —
//! built here, not read off a disk — through the one entry a browser uses, so
//! the window, the scrub, the replay and the refusals are checked without a
//! server.
//!
//! What they do NOT check is that the rows agree with desvis's; that is
//! `tests/dumped_rows.rs`, over a checked-in recording.

use bytemuck::Zeroable;
use desgraph::CellId;
use desgraph::types::CellKind;
use desunits::{RobotId, SimDuration, SimTime};
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RobotClass, RobotRecord, Step, StepKind,
};
use worldview::frames::robot_state::{RobotSpans, RobotState, RobotStateDelta, RobotStateKeyframe, StateSpan};
use worldview::frames::scalars::{
    AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit, ScalarsDelta, ScalarsKeyframe,
};
use worldview::frames::scenario::{HAND_BUILT_FRAME, Scenario};
use worldview::{Declare, Delta, End, Frame, Keyframe, Ms, Opaque, Payload, Sections, Tag};
use worldview_render::{Cell, FloorDoc, Overlays, Place, Tile};

use super::{Run, Took};

const SLICE_MS: u64 = 10_000;

/// Three cells in a row a metre apart, as the server's compiled floor states
/// them: each one its own tile, which is what desvis draws.
fn floor() -> FloorDoc {
    let cells = (0..3).map(|id| Some(cell(id))).collect();
    FloorDoc { cells, cell_size_mm: 1000.0, ..FloorDoc::default() }
}

fn cell(id: u32) -> Cell {
    Cell {
        id,
        kind: CellKind::Highway,
        tile_mm: [f64::from(id) * 1000.0, 0.0, 1000.0, 1000.0],
        heading_rad: 0.0,
        tile: Tile::Declared,
        wait_excluded: false,
    }
}

fn frame(tag: Tag, t_ms: u64, body: Vec<u8>) -> Vec<u8> {
    Frame::encode(tag, Ms(t_ms), &body, false).expect("a frame encodes")
}

fn sectioned(tag: Tag, t_ms: u64, sections: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
    let borrowed: Vec<(u8, &[u8])> = sections.iter().map(|(id, body)| (*id, body.as_slice())).collect();
    frame(tag, t_ms, Sections::encode(&borrowed))
}

fn metric(name: &str) -> MetricDescriptor {
    MetricDescriptor {
        name: name.into(),
        label: name.into(),
        description: String::new(),
        semantics: MetricSemantics {
            unit: MetricUnit::Count,
            denominator_description: None,
            aggregation: AggregationClass::Additive,
        },
    }
}

/// A scenario block with the compiled floor the wire always carries, and one
/// metric declared up front.
fn scenario_frame(floor: Option<FloorDoc>) -> Vec<u8> {
    let scenario = Scenario {
        run_name: "a synthetic run".into(),
        slice_width: SimDuration::ms(SLICE_MS),
        metrics: vec![metric("picks")],
        floor: floor.map(|floor| Opaque::of(&floor, "the compiled floor").expect("encodes")),
        frame: HAND_BUILT_FRAME.to_owned(),
        ..Scenario::default()
    };
    frame(Tag::Scenario, 0, scenario.to_bytes().expect("encodes"))
}

/// One robot standing on cell 0, facing east, and the run's metric at zero.
fn keyframe_frame(t_ms: u64) -> Vec<u8> {
    sectioned(Tag::Keyframe, t_ms, one_robot_keyframe().encode().expect("encodes"))
}

/// Robot 1 standing in cell 0, and the run's one metric at zero.
fn one_robot_keyframe() -> Keyframe {
    Keyframe {
        playback: Some(PlaybackKeyframe {
            robots: vec![RobotRecord {
                robot: RobotId(1),
                class: RobotClass::Htm,
                at: Position::new(500, 500, 0),
                heading_mrad: Some(0),
                carrying: None,
                cell: CellId(0),
                toward: None,
            }],
            ..PlaybackKeyframe::default()
        }),
        scalars: Some(ScalarsKeyframe { values: vec![Some(0.0)] }),
        ..Keyframe::default()
    }
}

/// The robot walks one cell east over the whole slice, and the run's one
/// metric counts `picks` more.
fn delta_frame(t_ms: u64, to: u32, picks: f64) -> Vec<u8> {
    sectioned(Tag::Delta, t_ms, walking_delta(to, picks).encode().expect("encodes"))
}

/// Robot 1 walking to cell `to` over the whole slice, and `picks` more counted.
fn walking_delta(to: u32, picks: f64) -> Delta {
    Delta {
        playback: Some(PlaybackDelta {
            movers: vec![MoverSteps {
                robot: RobotId(1),
                start_heading_mrad: Some(0),
                steps: vec![Step {
                    at: Position::new(to as i32 * 1_000 + 500, 500, 0),
                    dt: SimDuration::ms(SLICE_MS),
                    kind: StepKind::Move,
                    cell: CellId(to),
                }],
                cell: CellId(to),
            }],
            ..PlaybackDelta::default()
        }),
        scalars: Some(ScalarsDelta { values: vec![Some(picks)] }),
        ..Delta::default()
    }
}

fn end_frame(t_ms: u64) -> Vec<u8> {
    frame(Tag::End, t_ms, End::default().to_bytes().expect("encodes"))
}

/// An attachment at 0 with two slices after its keyframe.
fn attached() -> Run {
    let mut run = Run::default();
    assert_eq!(run.push_frame(&scenario_frame(Some(floor()))).expect("the scenario reads"), Took::Scenario);
    assert_eq!(run.push_frame(&keyframe_frame(0)).expect("the keyframe reads"), Took::Keyframe);
    for (t_ms, to, picks) in [(0, 1, 3.0), (SLICE_MS, 2, 4.0)] {
        assert_eq!(run.push_frame(&delta_frame(t_ms, to, picks)).expect("a delta reads"), Took::Delta);
    }
    run
}

fn where_at(run: &mut Run, t_ms: u64) -> Place {
    run.drawn_at(SimTime::ms(t_ms)).expect("held").expect("reads").robots[0].at
}

#[test]
fn the_scenario_block_is_the_floor_the_browser_draws_on() {
    let mut run = Run::default();
    assert!(run.scene().is_none(), "nothing to draw before the scenario block");
    run.push_frame(&scenario_frame(Some(floor()))).expect("reads");
    let scene = run.scene().expect("the floor arrived");
    assert_eq!(scene.cells().len(), 3);
    assert_eq!(scene.frame().extent(), (3.0, 1.0), "the tiles the server sent, not a square per centre");
}

/// A browser compiles no floor of its own, so a scenario block without one is
/// a wire it cannot draw from — never a reason to keep drawing the floor the
/// last attachment sent.
#[test]
fn a_scenario_block_with_no_compiled_floor_stops_the_run() {
    let mut run = Run::default();
    let refused = run.push_frame(&scenario_frame(None)).expect_err("no floor, no picture");
    assert!(refused.to_string().contains("compiles no floor of its own"), "{refused}");
    assert!(run.scene().is_none());
    assert!(run.push_frame(&keyframe_frame(0)).is_err(), "and the run stays stopped");
}

/// A second scenario block is a second attachment: everything the first one
/// sent goes, and the floor is the new one.
#[test]
fn a_second_scenario_block_starts_the_attachment_over() {
    let mut run = attached();
    run.push_frame(&end_frame(2 * SLICE_MS)).expect("the end reads");
    assert!(run.ended());

    let mut wider = floor();
    wider.cells.push(Some(cell(3)));
    run.push_frame(&scenario_frame(Some(wider))).expect("a second attachment");

    assert_eq!(run.held_window(), None, "no keyframe yet, so nothing is held");
    assert!(!run.ended(), "the last attachment's end is not this one's");
    assert!(run.index().is_empty());
    assert_eq!(run.scene().expect("a floor").cells().len(), 4, "the floor this attachment sent");
    assert!(run.drawn_at(SimTime::ZERO).is_none());
}

/// A scenario block with no floor, after a floor was sent ahead, keeps that
/// floor and takes the run: a dessim recording carries no floor of its own.
#[test]
fn a_scenario_without_a_floor_runs_on_the_floor_sent_ahead() {
    let mut run = Run::default();
    run.take_floor(floor().into_scene());
    assert_eq!(run.push_frame(&scenario_frame(None)).expect("the scenario reads on the held floor"), Took::Scenario);
    assert_eq!(run.scene().expect("the floor sent ahead").cells().len(), floor().cells.len());
    assert_eq!(run.push_frame(&keyframe_frame(0)).expect("the keyframe reads"), Took::Keyframe);
}

/// A scenario block with no floor and none sent ahead is refused.
#[test]
fn a_scenario_without_a_floor_and_none_held_is_refused() {
    let mut run = Run::default();
    assert!(run.push_frame(&scenario_frame(None)).is_err());
}

/// Attachment A brought its floor; attachment B on the same canvas brings
/// none, and none was sent ahead for it. The floor `clear` kept is A's, and
/// B's robots on it would be a picture of no run, so B is refused.
#[test]
fn a_floorless_attachment_after_another_never_runs_on_its_floor() {
    let mut run = attached();
    run.clear();
    let refused = run.push_frame(&scenario_frame(None)).expect_err("B sent no floor of its own");
    assert!(refused.to_string().contains("none was sent ahead for it"), "{refused}");
}

/// A floor sent ahead serves the one scenario block that takes it: the
/// attachment after that one brings its own floor, or is refused.
#[test]
fn a_floor_sent_ahead_serves_one_attachment() {
    let mut run = Run::default();
    run.take_floor(floor().into_scene());
    run.push_frame(&scenario_frame(None)).expect("the first attachment runs on the floor sent ahead");
    run.clear();
    assert!(run.push_frame(&scenario_frame(None)).is_err(), "the second sent no floor, and none was sent ahead for it");
}

/// A floor sent with no run (spec 03 section 3.6's `set_floor`) forgets the
/// run held before it, its declarations included, and holds nothing to
/// scrub; the next run's scenario block is taken as ever.
#[test]
fn a_floor_sent_alone_forgets_the_run_and_holds_no_window() {
    let mut run = attached();
    let mut wider = floor();
    wider.cells.push(Some(cell(3)));
    run.take_floor(wider.into_scene());

    assert_eq!(run.scene().expect("the floor sent").cells().len(), 4);
    assert!(run.census().is_empty(), "the last run's census is not this floor's");
    assert_eq!(run.held_window(), None);
    assert!(run.metric_names().is_empty(), "the last run's declarations are not this floor's");
    assert!(run.drawn_at(SimTime::ZERO).is_none(), "no run, so no movers to draw");
    assert_eq!(run.push_frame(&scenario_frame(Some(floor()))).expect("a run after it"), Took::Scenario);
}

/// The window reaches where the run stopped, which is what the end frame
/// stamps — not the last slice's start plus a nominal width.
#[test]
fn the_end_frames_instant_is_where_the_window_stops() {
    let mut run = attached();
    assert_eq!(run.held_window(), Some((SimTime::ZERO, SimTime::ms(2 * SLICE_MS))), "nominal until the end arrives");
    run.push_frame(&end_frame(16_500)).expect("the end reads");
    assert_eq!(run.held_window(), Some((SimTime::ZERO, SimTime::ms(16_500))), "the run stopped part-way through");

    // The last slice is a short one, so its own delta lands at that instant
    // rather than at a boundary no frame describes.
    let drawn = run.drawn_at(SimTime::ms(16_500)).expect("held").expect("reads");
    assert_eq!(drawn.at_ms, 16_500);
    assert_eq!(run.metric("picks"), Some(7.0), "both slices counted");
}

/// A cursor dragged back to an instant already passed answers what it
/// answered the first time: a seek and a replay are the same picture.
#[test]
fn scrubbing_backwards_answers_what_playing_forwards_answered() {
    let mut run = attached();
    let opened = where_at(&mut run, 0);
    let mid_first = where_at(&mut run, 5_000);
    let mid_second = where_at(&mut run, 15_000);
    // A robot walking its steps is placed by the millimetres its slice
    // resolves to, even where those are exactly a cell's centre — which is
    // what desvis does, and what decides whether a VTM rides the rack.
    assert_eq!(opened, Place { cell: 0, pos_mm: [500.0, 500.0] }, "cell 0's own centre");
    assert_eq!(mid_first, Place { cell: 0, pos_mm: [1000.0, 500.0] }, "half way from cell 0 to cell 1");
    assert_eq!(mid_second, Place { cell: 1, pos_mm: [2000.0, 500.0] }, "and half way on again");

    assert_eq!(where_at(&mut run, 5_000), mid_first, "dragged back");
    assert_eq!(where_at(&mut run, 0), opened);
    assert_eq!(where_at(&mut run, 15_000), mid_second, "and forward again");
}

/// A chart reads the state the picture was drawn from, so a slice the cursor
/// has reached the end of is counted — not left for the next frame.
#[test]
fn a_chart_is_never_a_slice_behind_the_picture() {
    let mut run = attached();
    run.push_frame(&end_frame(2 * SLICE_MS)).expect("the end reads");
    for (cursor_ms, picks) in [(0, 0.0), (9_999, 0.0), (10_000, 3.0), (19_999, 3.0), (20_000, 7.0)] {
        run.drawn_at(SimTime::ms(cursor_ms)).expect("held").expect("reads");
        assert_eq!(run.metric("picks"), Some(picks), "at {cursor_ms} ms");
        assert!(run.slice_overlay().is_some(), "and the slice's own overlay is there too");
    }
}

/// A cursor outside the window is clamped into it rather than refused: the
/// shell decides whether to attach again, and meanwhile the picture holds.
#[test]
fn a_cursor_past_the_window_is_held_at_its_edge() {
    let mut run = attached();
    let far = run.drawn_at(SimTime::ms(900_000)).expect("held").expect("reads");
    assert_eq!(far.at_ms, 2 * SLICE_MS);
    assert!(run.drawn_at(SimTime::ZERO).is_some());

    run.clear();
    assert_eq!(run.held_window(), None, "a re-attach starts an empty window");
    assert!(run.drawn_at(SimTime::ZERO).is_none());
    assert!(run.scene().is_some(), "on the same floor");
}

/// A metric declared after the keyframe widens every scalars array the run
/// states from then on, and a state still laid out for the narrower one
/// refuses the next slice. Both the state being played and the one a scrub
/// replays from are re-described.
#[test]
fn a_metric_declared_after_the_keyframe_reaches_both_held_states() {
    let mut run = Run::default();
    run.push_frame(&scenario_frame(Some(floor()))).expect("reads");
    run.push_frame(&keyframe_frame(0)).expect("reads");
    let declared = Declare::Metric(metric("putaways"));
    assert_eq!(
        run.push_frame(&frame(Tag::Declare, 0, declared.to_bytes().expect("encodes"))).expect("reads"),
        Took::Declaration
    );

    let wider = Delta {
        scalars: Some(ScalarsDelta { values: vec![Some(3.0), Some(5.0)] }),
        playback: Some(PlaybackDelta::default()),
        ..Delta::default()
    };
    run.push_frame(&sectioned(Tag::Delta, 0, wider.encode().expect("encodes"))).expect("reads");
    run.push_frame(&end_frame(SLICE_MS)).expect("reads");

    run.drawn_at(SimTime::ms(SLICE_MS)).expect("held").expect("the wider slice applies");
    assert_eq!((run.metric("picks"), run.metric("putaways")), (Some(3.0), Some(5.0)));
    // And again from the keyframe, which a backwards scrub replays from.
    run.drawn_at(SimTime::ZERO).expect("held").expect("reads");
    run.drawn_at(SimTime::ms(SLICE_MS)).expect("held").expect("reads");
    assert_eq!(run.metric("putaways"), Some(5.0));
}

/// The wire promises one frame per message and slices that are dense, in
/// order, after the keyframe and before the end. Each of those broken is the
/// stream's fault, and a viewer that carried on would draw a run nobody
/// recorded.
#[test]
fn a_stream_that_breaks_its_own_shape_stops_the_run() {
    let opened = || {
        let mut run = Run::default();
        run.push_frame(&scenario_frame(Some(floor()))).expect("reads");
        run
    };
    let ways: Vec<(&str, Run, Vec<u8>)> = vec![
        ("and more besides", opened(), {
            let mut two = keyframe_frame(0);
            two.extend(delta_frame(0, 1, 1.0));
            two
        }),
        ("does not follow", attached(), delta_frame(3 * SLICE_MS, 3, 1.0)),
        ("does not follow", attached(), delta_frame(SLICE_MS, 2, 1.0)),
        ("before the attachment's keyframe", opened(), delta_frame(0, 1, 1.0)),
        (
            "after the run's end",
            {
                let mut run = attached();
                run.push_frame(&end_frame(2 * SLICE_MS)).expect("reads");
                run
            },
            delta_frame(2 * SLICE_MS, 3, 1.0),
        ),
    ];
    for (says, mut run, offending) in ways {
        let refused = run.push_frame(&offending).expect_err("the wire promised otherwise");
        assert!(refused.to_string().contains(says), "expected a complaint about `{says}`, got: {refused}");
        assert!(run.stopped().is_some(), "and the run is stopped from then on");
        assert!(run.drawn_at(SimTime::ZERO).expect("an answer").is_err(), "which every later answer repeats");
    }
}

/// A second keyframe inside one attachment is not something the server sends,
/// and taking it would silently throw away the slices already held.
#[test]
fn a_second_keyframe_in_one_attachment_stops_the_run() {
    let mut run = attached();
    let refused = run.push_frame(&keyframe_frame(2 * SLICE_MS)).expect_err("an attachment sends one keyframe");
    assert!(refused.to_string().contains("a second keyframe"), "{refused}");
}

/// The rows the conversion feeds the renderer, over the whole path a browser
/// takes: frames in, one instant out, instances on the other side.
#[test]
fn the_frames_a_browser_is_sent_become_the_rows_it_draws() {
    let mut run = attached();
    let drawn = run.drawn_at(SimTime::ms(5_000)).expect("held").expect("reads");
    let scene = run.scene().expect("a floor");
    let moment = scene.moment(Zeroable::zeroed(), None, &drawn.playback(Overlays::default()));
    assert_eq!(moment.bot_instances.len(), 1);
    assert_eq!([moment.bot_instances[0].world_x, moment.bot_instances[0].world_y], [1.0, 0.5]);
    assert_eq!(moment.cell_instances.len(), 3);
}

/// The live half of a chart states a point only where the slice stated the
/// slot, which is the rule the server's `series` follows over the file. A
/// metric a run reports intermittently — one measuring something that comes
/// and goes — has to read the same on both sides of the join, or one line
/// would make two different claims about the same kind of gap.
#[test]
fn a_slice_that_states_nothing_about_a_metric_puts_no_point_on_its_line() {
    let mut run = Run::default();
    run.push_frame(&scenario_frame(Some(floor()))).expect("the scenario reads");
    run.push_frame(&keyframe_frame(0)).expect("the keyframe reads");
    // Three slices: the first and third count picks, the middle one says
    // nothing about them at all.
    run.push_frame(&delta_frame(0, 1, 3.0)).expect("a delta reads");
    run.push_frame(&silent_delta(SLICE_MS, 2)).expect("a silent delta reads");
    run.push_frame(&delta_frame(2 * SLICE_MS, 1, 4.0)).expect("a delta reads");

    assert_eq!(run.metric_series("picks"), vec![(SLICE_MS, 3.0)], "the newest slice's end is not known yet");
    run.push_frame(&end_frame(3 * SLICE_MS)).expect("the end reads");
    assert_eq!(
        run.metric_series("picks"),
        vec![(SLICE_MS, 3.0), (3 * SLICE_MS, 7.0)],
        "the middle slice is a gap, and the running total carries across it"
    );
    assert!(run.metric_series("nothing_declared").is_empty(), "a name the run never declared has no line");
}

/// A slice that moves a robot and reports no metric value.
fn silent_delta(t_ms: u64, to: u32) -> Vec<u8> {
    let delta = Delta {
        playback: Some(PlaybackDelta {
            movers: vec![MoverSteps {
                robot: RobotId(1),
                start_heading_mrad: Some(0),
                steps: vec![Step {
                    at: Position::new(to as i32 * 1_000 + 500, 500, 0),
                    dt: SimDuration::ms(SLICE_MS),
                    kind: StepKind::Move,
                    cell: CellId(to),
                }],
                cell: CellId(to),
            }],
            ..PlaybackDelta::default()
        }),
        scalars: Some(ScalarsDelta { values: vec![None] }),
        ..Delta::default()
    };
    sectioned(Tag::Delta, t_ms, delta.encode().expect("encodes"))
}

/// A keyframe at 0 stating where robot 1 is and what it is doing: standing with no task.
fn keyframe_with_robot_state() -> Vec<u8> {
    let mut keyframe = one_robot_keyframe();
    keyframe.robot_state = Some(RobotStateKeyframe(vec![(RobotId(1), RobotState::NoTask { cell: None })]));
    sectioned(Tag::Keyframe, 0, keyframe.encode().expect("encodes"))
}

/// `delta_frame`'s slice, stating robot 1's spans in it too.
fn delta_with_spans(t_ms: u64, to: u32, picks: f64, spans: &[(u64, RobotState)]) -> Vec<u8> {
    let mut delta = walking_delta(to, picks);
    let spans = spans.iter().map(|(dt_ms, state)| StateSpan { dt: SimDuration::ms(*dt_ms), state: *state }).collect();
    delta.robot_state = Some(RobotStateDelta(vec![RobotSpans { robot: RobotId(1), continues: false, spans }]));
    sectioned(Tag::Delta, t_ms, delta.encode().expect("encodes"))
}

/// A run that stops 6.5 s into its second slice: the end block ends that slice there, so its spans lie across 6.5 s
/// and the cursor at the stop counts it. A recording whose last slice states spans past the stop is damage the
/// picture reports where it counts that slice, rather than charting the run one slice short.
#[test]
fn the_last_slice_is_counted_up_to_the_stop_and_one_stating_time_past_it_stops_the_run() {
    let travel = RobotState::EmptyTravel;
    let frames = |last: &[(u64, RobotState)]| {
        let mut run = Run::default();
        run.push_frame(&scenario_frame(Some(floor()))).expect("the scenario reads");
        run.push_frame(&keyframe_with_robot_state()).expect("the keyframe reads");
        run.push_frame(&delta_with_spans(0, 1, 3.0, &[(SLICE_MS, travel)])).expect("a delta reads");
        run.push_frame(&delta_with_spans(SLICE_MS, 2, 4.0, last)).expect("a delta reads");
        run.push_frame(&end_frame(16_500)).expect("the end reads");
        run
    };
    let mut cut = frames(&[(6_500, RobotState::Drop)]);
    cut.drawn_at(SimTime::ms(16_500)).expect("held").expect("the last slice lies across 6.5 s");
    assert_eq!(cut.metric("picks"), Some(7.0), "both slices counted");

    let mut whole = frames(&[(SLICE_MS, RobotState::Drop)]);
    let refused = whole.drawn_at(SimTime::ms(16_500)).expect("held").expect_err("spans 10 s long in a 6.5 s slice");
    assert!(refused.to_string().contains("the spans last 10000 ms and the slice 6500 ms"), "{refused}");
}
