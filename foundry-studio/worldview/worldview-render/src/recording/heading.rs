//! Within `recording`, `heading` is WHICH WAY A ROBOT FACES part-way through
//! a slice, worked out from the steps a recording states.
//!
//! A step carries a place and a duration, never an orientation, so the facing
//! has to be inferred: a move faces along its own displacement, and a rotate
//! is exactly the turn from the heading the robot arrived on to the one it
//! departs on. Sweeping that turn across the step's own duration is what makes
//! a stop-and-rotate read as a turn rather than an unexplained pause.
//!
//! The recording carries one heading per robot per slice — the one it began
//! on — so without this a robot would hold its opening facing for a whole ten
//! seconds and then snap. desvis has already inferred the same thing from the
//! simulator's step journal (`app/heading.rs`); this is that inference over
//! the recording's own words, which is what makes the two pictures agree.

use std::f32::consts::{PI, TAU};

use worldview::frames::playback::{Step, StepKind};

/// Under a millimetre of travel is journal rounding, not a direction.
const MIN_TRAVEL_MM: f64 = 1.0;

/// The direction `steps[i]` travels, in world radians: 0 points along +x,
/// turning toward +y. Nil for a step that covers no ground — a wait, a
/// rotation, or a move that stayed put — which has no direction of its own.
fn step_heading(steps: &[Step], places: &[[f64; 2]], start_mm: [f64; 2], i: usize) -> Option<f32> {
    if !matches!(steps[i].kind, StepKind::Move | StepKind::Reverse) {
        return None;
    }
    let from = if i == 0 { start_mm } else { places[i - 1] };
    let (dx, dy) = (places[i][0] - from[0], places[i][1] - from[1]);
    if dx.hypot(dy) < MIN_TRAVEL_MM {
        return None;
    }
    let travel = dy.atan2(dx) as f32;
    Some(match steps[i].kind {
        StepKind::Reverse => (travel + PI).rem_euclid(TAU),
        _ => travel,
    })
}

/// Turn `from` toward `to` the short way round, so a robot turning from north
/// to east sweeps ninety degrees rather than two hundred and seventy.
fn lerp_heading(from: f32, to: f32, t: f32) -> f32 {
    let mut delta = (to - from) % TAU;
    if delta > PI {
        delta -= TAU;
    } else if delta < -PI {
        delta += TAU;
    }
    from + delta * t
}

/// Where the robot has come to rest facing, once the instant is past
/// everything this slice states. `opened_on` is the facing it began the slice
/// with, which stands where the slice gives nothing to derive from.
pub(super) fn settled(steps: &[Step], places: &[[f64; 2]], start_mm: [f64; 2], opened_on: f32) -> f32 {
    (0..steps.len()).rev().find_map(|i| step_heading(steps, places, start_mm, i)).unwrap_or(opened_on)
}

/// Facing at `progress` (0..1) through `steps[active]`.
pub(super) fn part_way(
    steps: &[Step],
    places: &[[f64; 2]],
    start_mm: [f64; 2],
    active: usize,
    progress: f32,
    opened_on: f32,
) -> f32 {
    let Some(step) = steps.get(active) else {
        return opened_on;
    };
    let arrived_on = (0..=active).rev().find_map(|i| step_heading(steps, places, start_mm, i)).unwrap_or(opened_on);
    if step.kind != StepKind::Rotate {
        return arrived_on;
    }
    // A rotation whose departing move falls in the next slice has nothing to
    // aim at yet, so the robot holds and the next slice finishes the turn.
    match (active + 1..steps.len()).find_map(|i| step_heading(steps, places, start_mm, i)) {
        Some(departs_on) => lerp_heading(arrived_on, departs_on, progress),
        None => arrived_on,
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use desgraph::CellId;
    use desunits::SimDuration;
    use worldview::frames::playback::Position;

    use super::*;

    const EAST: f32 = 0.0;
    const NORTH: f32 = FRAC_PI_2;

    fn step(kind: StepKind, dt_ms: u64) -> Step {
        Step { at: Position::default(), dt: SimDuration::ms(dt_ms), kind, cell: CellId(0) }
    }

    /// The same four claims desvis's own `heading` module makes, over the
    /// recording's steps: a move faces along its travel, a reverse faces
    /// against it, a rotation sweeps the short way round, and a rotation with
    /// no departing move holds.
    #[test]
    fn a_turn_sweeps_between_the_moves_on_either_side_of_it() {
        let steps = [step(StepKind::Move, 1000), step(StepKind::Rotate, 2100), step(StepKind::Move, 1000)];
        let places = [[1000.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0]];
        let start = [0.0, 0.0];
        for (progress, expected) in [(0.0, EAST), (0.5, NORTH / 2.0), (1.0, NORTH)] {
            let swept = part_way(&steps, &places, start, 1, progress, 0.0);
            assert!((swept - expected).abs() < 1e-5, "at {progress} through the turn: {swept}");
        }
        assert!((settled(&steps, &places, start, 0.0) - NORTH).abs() < 1e-5);
    }

    #[test]
    fn a_reverse_keeps_the_chassis_facing_against_its_travel() {
        let steps = [step(StepKind::Reverse, 1000)];
        let places = [[-1000.0, 0.0]];
        assert!((part_way(&steps, &places, [0.0, 0.0], 0, 0.5, NORTH) - EAST).abs() < 1e-5);
    }

    #[test]
    fn a_turn_takes_the_short_way_and_holds_when_nothing_departs() {
        let steps = [step(StepKind::Move, 1000), step(StepKind::Rotate, 2100), step(StepKind::Move, 1000)];
        let places = [[-1000.0, 0.0], [-1000.0, 0.0], [-1000.0, -1000.0]];
        let mid = part_way(&steps, &places, [0.0, 0.0], 1, 0.5, 0.0);
        assert!((mid - 5.0 * NORTH / 2.0).abs() < 1e-5, "expected south-west, got {mid}");

        let alone = [step(StepKind::Rotate, 2100)];
        assert!((part_way(&alone, &[[0.0, 0.0]], [0.0, 0.0], 0, 0.5, NORTH) - NORTH).abs() < 1e-5);
        assert!((settled(&alone, &[[0.0, 0.0]], [0.0, 0.0], NORTH) - NORTH).abs() < 1e-5);
    }
}
