//! What a robot is DRAWN holding across an EasyLoop's two handoffs.
//!
//! The location log (`InventoryMoveEvent`, bridged from the tote's own
//! `move_to`) and the render channel (`ToteMovedEvent`, carrying a
//! `ToteVisualState`) are two independent streams, and desvis draws a bot as
//! laden purely because some tote's visual state names it. So a flow that
//! moves a tote in the model and emits nothing on the render channel leaves
//! the tote drawn wherever it last was: measured before this test, the HTM
//! that set a tote on the belt kept carrying a phantom copy of it for 63
//! seconds of sim time, and the HTM that lifted the real one off drove away
//! empty-handed.
//!
//! These assertions compare the two streams at the exact instants the belt
//! changes hands. They are about what is DRAWN, not about the schedule: the
//! deposit and the collect are already correct in the plan.

mod common;

use std::sync::OnceLock;

use dessim::events::SimEvent;
use dessim::model::{RobotId, ToteLocation};
use dessim::observer::time_slice::ToteVisualState;
use native_modules as _;
use rustc_hash::FxHashMap;

/// One tote's render states in time order.
type Timeline = Vec<(u64, ToteVisualState)>;

/// What the renderer would draw for `tote` at `at_ms`: the last visual state
/// emitted at or before that instant, ties broken by emission order, which is
/// how `playback.rs::build_tote_instances_for_cursor` replays a slice.
fn drawn_at(timeline: &Timeline, at_ms: u64) -> Option<&ToteVisualState> {
    timeline.iter().rev().find(|(t, _)| *t <= at_ms).map(|(_, s)| s)
}

struct Run {
    timelines: FxHashMap<u32, Timeline>,
    /// (tote, instant) for every board.
    boards: Vec<(u32, u64)>,
    /// (tote, instant, robot that lifted it) for every lift-off.
    lifts: Vec<(u32, u64, RobotId)>,
}

/// One 30-minute DFW6 smoke run, shared by the assertions below when they run
/// in one process; none of them perturbs it. The run is minutes of
/// unoptimised simulation, and the gate runs every test as its own process,
/// so the three are in the slow gate.
fn run_smoke() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(build_run)
}

fn build_run() -> Run {
    let (world, scenario, config) = common::load_world_spec("walmart_dfw6_smoke_spec.json");
    let (events, _result, _metrics) = common::run_world_capture_all(&world, &scenario, config);

    let mut timelines: FxHashMap<u32, Timeline> = FxHashMap::default();
    let mut boards = Vec::new();
    let mut lifts = Vec::new();
    for event in &events {
        match event {
            SimEvent::ToteMoved(moved) => {
                timelines.entry(moved.tote_id.0).or_default().push((moved.time.as_ms(), moved.new_state.clone()));
            }
            SimEvent::InventoryMove(moved) => {
                if matches!(moved.to, ToteLocation::OnConveyor { .. } | ToteLocation::OnConveyance { .. }) {
                    boards.push((moved.tote_id.0, moved.at.as_ms()));
                }
                if matches!(moved.from, ToteLocation::OnConveyor { .. } | ToteLocation::OnConveyance { .. }) {
                    let ToteLocation::WithRobot { robot_id } = moved.to else {
                        panic!("a tote leaving a belt goes onto a robot, not {:?}", moved.to);
                    };
                    lifts.push((moved.tote_id.0, moved.at.as_ms(), robot_id));
                }
            }
            _ => {}
        }
    }
    // Stable: equal timestamps keep emission order, the observer's own rule.
    for timeline in timelines.values_mut() {
        timeline.sort_by_key(|(t, _)| *t);
    }
    Run { timelines, boards, lifts }
}

/// An HTM that sets a tote on the belt is drawn UNLADEN from that instant.
///
/// The bot's own record carries no ladenness, so the only way it can look
/// unladen is for the tote to stop naming it. Nothing else in the run emits
/// for a belt, which is why the deposit has to.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn a_tote_set_on_the_belt_stops_being_drawn_on_the_robot() {
    let run = run_smoke();
    assert!(!run.boards.is_empty(), "the smoke run must board totes or this proves nothing");
    for (tote, at_ms) in &run.boards {
        let timeline = run
            .timelines
            .get(tote)
            .unwrap_or_else(|| panic!("tote {tote} boarded a belt at {at_ms}ms and was never drawn anywhere"));
        let drawn = drawn_at(timeline, *at_ms)
            .unwrap_or_else(|| panic!("tote {tote} boarded at {at_ms}ms with no state drawn yet"));
        assert!(
            matches!(drawn, ToteVisualState::OnConveyor { .. }),
            "tote {tote} boarded a belt at {at_ms}ms but is drawn {drawn:?}; \
             an HTM leaving the dropoff cell would look laden. Timeline: {timeline:?}"
        );
    }
}

/// An HTM that lifts a tote off the belt is drawn LADEN from that instant,
/// and drawn holding THAT tote rather than some other robot's.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn a_tote_lifted_off_the_belt_is_drawn_on_the_robot_that_lifted_it() {
    let run = run_smoke();
    assert!(!run.lifts.is_empty(), "the smoke run must lift totes off or this proves nothing");
    for (tote, at_ms, robot) in &run.lifts {
        let timeline = run
            .timelines
            .get(tote)
            .unwrap_or_else(|| panic!("tote {tote} was lifted off a belt at {at_ms}ms and was never drawn anywhere"));
        let drawn = drawn_at(timeline, *at_ms)
            .unwrap_or_else(|| panic!("tote {tote} lifted at {at_ms}ms with no state drawn yet"));
        assert_eq!(
            drawn,
            &ToteVisualState::OnBot { bot_id: robot.0 },
            "tote {tote} was lifted off a belt by {robot:?} at {at_ms}ms but is drawn {drawn:?}; \
             the collecting HTM would look unladen. Timeline: {timeline:?}"
        );
    }
}

/// The ride between the two handoffs stays on the belt: no robot is drawn
/// holding a tote that is riding.
///
/// The pair above check the two instants; this checks the ~90s in between,
/// which is where the phantom copy was actually visible. It samples what the
/// renderer WOULD draw at the ride's midpoint as well as every state emitted
/// inside the ride, because a gap in the timeline is exactly how the phantom
/// survived: nothing was emitted, so nothing contradicted it.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn a_riding_tote_stays_drawn_on_the_belt_for_the_whole_ride() {
    let run = run_smoke();
    assert!(!run.boards.is_empty() && !run.lifts.is_empty(), "nothing rode");
    let mut rides = 0usize;
    for (tote, board_ms) in &run.boards {
        // The lift that ends THIS ride: a tote can board more than once.
        let Some(lift_ms) =
            run.lifts.iter().filter(|(t, at, _)| t == tote && at > board_ms).map(|(_, at, _)| *at).min()
        else {
            continue;
        };
        rides += 1;
        let timeline = &run.timelines[tote];
        let mid_ms = board_ms + (lift_ms - board_ms) / 2;
        let mut sampled: Vec<(u64, &ToteVisualState)> =
            timeline.iter().filter(|(t, _)| *t >= *board_ms && *t < lift_ms).map(|(t, s)| (*t, s)).collect();
        if let Some(mid) = drawn_at(timeline, mid_ms) {
            sampled.push((mid_ms, mid));
        }
        for (t, state) in sampled {
            assert!(
                matches!(state, ToteVisualState::OnConveyor { .. }),
                "tote {tote} rode a belt over [{board_ms}, {lift_ms}]ms but is drawn {state:?} \
                 at {t}ms. Timeline: {timeline:?}"
            );
        }
    }
    assert!(rides > 0, "no board was matched to a lift, so nothing was checked");
}
