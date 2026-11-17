//! Within `app`, `heading` is WHICH WAY A BOT FACES, reconstructed from the
//! step journal that only ever recorded where it stood.
//!
//! A `BotStep` carries a position and a duration, never an orientation, so the
//! renderer has to infer facing. A `Move` faces along its own displacement; a
//! `Rotate` is precisely the turn from the heading the bot arrived on to the
//! one it departs on, and sweeping that turn across the step's `delta_ms` is
//! what makes an HTM's stop-and-rotate read as a turn on screen instead of an
//! unexplained pause.

use std::f32::consts::{PI, TAU};

use desgraph::WorldPos;
use dessim::observer::time_slice::{BotLocation, BotStep, BotStepKind};

/// Under a millimetre of travel is journal rounding, not a direction.
const MIN_TRAVEL_MM: f64 = 1.0;

/// The direction `steps[i]` travels, as an angle in the renderer's world
/// frame: 0 points along +x, turning toward +y. RenderFrame preserves world
/// northing, so headings use the same unmirrored displacement as positions.
/// `None` for a step that covers no ground — a wait, a rotation, or a move that
/// stayed put — since such a step has no direction of its own.
fn step_heading(steps: &[BotStep], start_pos: WorldPos, i: usize) -> Option<f32> {
    if !matches!(steps[i].kind, BotStepKind::Move | BotStepKind::Reverse) {
        return None;
    }
    let from = if i == 0 { start_pos } else { steps[i - 1].pos };
    let (dx, dy) = (steps[i].pos.x_mm - from.x_mm, steps[i].pos.y_mm - from.y_mm);
    if dx.hypot(dy) < MIN_TRAVEL_MM {
        return None;
    }
    let travel = dy.atan2(dx) as f32;
    Some(if steps[i].kind == BotStepKind::Reverse { (travel + PI).rem_euclid(TAU) } else { travel })
}

/// Interpolate `from` toward `to` the short way round, so a bot turning from
/// north to east sweeps 90 degrees rather than 270.
fn lerp_heading(from: f32, to: f32, t: f32) -> f32 {
    let mut delta = (to - from) % TAU;
    if delta > PI {
        delta -= TAU;
    } else if delta < -PI {
        delta += TAU;
    }
    from + delta * t
}

/// Which way a bot drawn from `location` faces: partway through
/// `active = (step, progress)`, or, with none, once it has settled. Live and on
/// playback alike: where a slice's own steps say nothing, the heading the
/// slice states the bot started on stands, and only a slice from before
/// recordings stated one falls back on `previous`, the last heading drawn.
pub(crate) fn drawn_heading(location: &BotLocation, active: Option<(usize, f32)>, previous: Option<f32>) -> f32 {
    let start = location.start_heading_rad.or(previous).unwrap_or(0.0);
    match active {
        Some((index, progress)) => heading_at(&location.steps, location.start_pos, index, progress, start),
        None => settled_heading(&location.steps, location.start_pos, start),
    }
}

/// Where the bot has come to rest facing, once the cursor is past everything
/// this slice journaled. `previous` carries the last drawn heading for a slice
/// whose steps never move the bot.
pub(crate) fn settled_heading(steps: &[BotStep], start_pos: WorldPos, previous: f32) -> f32 {
    (0..steps.len()).rev().find_map(|i| step_heading(steps, start_pos, i)).unwrap_or(previous)
}

/// Facing at `progress` (0..1) through `steps[active]`.
///
/// `previous` is the bot's last drawn heading, standing in wherever this slice
/// gives nothing to derive from: the opening steps of a slice, or the tail of
/// a rotation whose departing move landed in the next slice. It keeps a bot
/// pointing where it already pointed rather than snapping to due east.
pub(crate) fn heading_at(steps: &[BotStep], start_pos: WorldPos, active: usize, progress: f32, previous: f32) -> f32 {
    let Some(step) = steps.get(active) else {
        return previous;
    };
    let arrived_on = (0..=active).rev().find_map(|i| step_heading(steps, start_pos, i)).unwrap_or(previous);
    if step.kind != BotStepKind::Rotate {
        return arrived_on;
    }
    // A rotation whose departing move falls outside this slice has nothing to
    // aim at yet, so hold and let the next slice finish the turn.
    match (active + 1..steps.len()).find_map(|i| step_heading(steps, start_pos, i)) {
        Some(departs_on) => lerp_heading(arrived_on, departs_on, progress),
        None => arrived_on,
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use desgraph::{RobotType, WorldPos};
    use dessim::observer::time_slice::{BotLocation, BotStep, BotStepKind};
    use worldview_render::RenderFrame;

    use super::{drawn_heading, heading_at, settled_heading};

    const EAST: f32 = 0.0;
    const NORTH: f32 = FRAC_PI_2;

    fn at(x_mm: f64, y_mm: f64) -> WorldPos {
        WorldPos { x_mm, y_mm }
    }

    fn step(kind: BotStepKind, x_mm: f64, y_mm: f64, delta_ms: u64) -> BotStep {
        BotStep::whole(0, at(x_mm, y_mm), delta_ms, kind)
    }

    /// World northing and rendered northing agree, including oblique travel.
    #[test]
    fn move_faces_along_its_own_displacement() {
        let steps = [step(BotStepKind::Move, 1000.0, 0.0, 1000), step(BotStepKind::Move, 1000.0, 1000.0, 1000)];
        assert!((heading_at(&steps, at(0.0, 0.0), 0, 0.5, 0.0) - EAST).abs() < 1e-5);
        assert!((heading_at(&steps, at(0.0, 0.0), 1, 0.5, 0.0) - NORTH).abs() < 1e-5);
    }

    #[test]
    fn reverse_motion_keeps_the_chassis_facing_against_travel() {
        let steps = [step(BotStepKind::Reverse, -1000.0, 0.0, 1000)];
        let heading = heading_at(&steps, at(0.0, 0.0), 0, 0.5, NORTH);
        assert!((heading - EAST).abs() < 1e-5, "westward reverse travel must retain an east-facing chassis");
        assert!((settled_heading(&steps, at(0.0, 0.0), NORTH) - EAST).abs() < 1e-5);
    }

    #[test]
    fn oblique_motion_faces_the_same_direction_as_rendered_displacement() {
        let start = at(100.0, 200.0);
        let frame = RenderFrame::default();
        for (dx, dy) in [(1000.0, 500.0), (-600.0, 900.0), (-300.0, -1000.0), (800.0, -600.0)] {
            let steps = [step(BotStepKind::Move, start.x_mm + dx, start.y_mm + dy, 1000)];
            let from = frame.world_mm(start.x_mm, start.y_mm);
            let to = frame.world_mm(start.x_mm + dx, start.y_mm + dy);
            let expected = (to[1] - from[1]).atan2(to[0] - from[0]);
            assert!((heading_at(&steps, start, 0, 0.5, 0.0) - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn rotate_sweeps_from_the_arriving_heading_to_the_departing_one() {
        let steps = [
            step(BotStepKind::Move, 1000.0, 0.0, 1000),
            step(BotStepKind::Rotate, 1000.0, 0.0, 2100),
            step(BotStepKind::Move, 1000.0, 1000.0, 1000),
        ];
        let start = at(0.0, 0.0);
        let sweep: Vec<f32> = [0.0, 0.5, 1.0].iter().map(|&t| heading_at(&steps, start, 1, t, 0.0)).collect();
        assert!((sweep[0] - EAST).abs() < 1e-5, "starts on the arriving heading, got {}", sweep[0]);
        assert!((sweep[1] - NORTH / 2.0).abs() < 1e-5, "half way through the turn, got {}", sweep[1]);
        assert!((sweep[2] - NORTH).abs() < 1e-5, "ends on the departing heading, got {}", sweep[2]);
    }

    /// The turn has to run the short way round: west to south is a quarter
    /// turn through south-west, never three quarters the other way.
    #[test]
    fn rotate_takes_the_short_way_round() {
        let steps = [
            step(BotStepKind::Move, -1000.0, 0.0, 1000),
            step(BotStepKind::Rotate, -1000.0, 0.0, 2100),
            step(BotStepKind::Move, -1000.0, -1000.0, 1000),
        ];
        let mid = heading_at(&steps, at(0.0, 0.0), 1, 0.5, 0.0);
        assert!((mid - 5.0 * NORTH / 2.0).abs() < 1e-5, "expected south-west, got {mid}");
    }

    /// A rotation split across a slice boundary has no departing move to aim
    /// at, so the bot holds its heading rather than snapping to due east.
    #[test]
    fn rotate_without_a_departing_move_holds_its_heading() {
        let steps = [step(BotStepKind::Rotate, 0.0, 0.0, 2100)];
        assert!((heading_at(&steps, at(0.0, 0.0), 0, 0.5, NORTH) - NORTH).abs() < 1e-5);
    }

    #[test]
    fn settled_heading_reports_the_last_move_and_falls_back_when_there_is_none() {
        let start = at(0.0, 0.0);
        let moved = [step(BotStepKind::Move, 1000.0, 0.0, 1000), step(BotStepKind::Wait, 1000.0, 0.0, 500)];
        assert!((settled_heading(&moved, start, NORTH) - EAST).abs() < 1e-5);
        let idle = [step(BotStepKind::Wait, 0.0, 0.0, 500)];
        assert!((settled_heading(&idle, start, NORTH) - NORTH).abs() < 1e-5);
    }

    fn stationary(heading: Option<f32>) -> BotLocation {
        BotLocation {
            motion: Some(Vec::new()),
            start_cell: 0,
            start_pos: WorldPos { x_mm: 0.0, y_mm: 0.0 },
            start_z_mm: 0.0,
            start_toward: None,
            start_heading_rad: heading,
            robot_type: RobotType::Htm,
            steps: Default::default(),
        }
    }

    #[test]
    fn seeking_to_stationary_slices_does_not_depend_on_the_previous_frame() {
        let later = stationary(Some(FRAC_PI_2));
        let earlier = stationary(Some(0.0));
        for prior in [None, Some(0.0), Some(PI)] {
            let jumped_forward = drawn_heading(&later, None, prior);
            assert_eq!(jumped_forward, FRAC_PI_2);
            let jumped_back = drawn_heading(&earlier, None, Some(jumped_forward));
            assert_eq!(jumped_back, 0.0);
            assert_eq!(drawn_heading(&later, None, Some(jumped_back)), FRAC_PI_2);
        }
        assert_eq!(drawn_heading(&stationary(None), None, Some(PI)), PI, "legacy history fallback remains");
    }

    #[test]
    fn seeking_inside_a_rotation_uses_its_recorded_start_heading() {
        let mut location = stationary(Some(0.0));
        location.steps.extend([
            BotStep::whole(0, location.start_pos, 1000, BotStepKind::Rotate),
            BotStep::whole(1, WorldPos { x_mm: 0.0, y_mm: 1000.0 }, 1000, BotStepKind::Move),
        ]);
        for prior in [None, Some(PI), Some(FRAC_PI_2)] {
            let halfway = drawn_heading(&location, Some((0, 0.5)), prior);
            assert!((halfway - FRAC_PI_2 / 2.0).abs() < 1e-6);
            assert_eq!(drawn_heading(&location, None, Some(halfway)), FRAC_PI_2);
        }
    }
}
