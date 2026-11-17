//! Samples committed motion into playback steps. Straight travel uses its
//! kinematic distance profile; curved links map the same progress onto their
//! declared arc, including under constant-speed simulation.
//!
//! `nav_handler` calls [`rasterize_move`] for kinematic HTM motion or a
//! curved link. It pushes a sequence of micro-`OccupancyInterval`s onto the
//! handler's interval buffer, sampling the cell's trapezoidal motion
//! profile at the configured `motion_step_ms` cadence (default 100 ms).
//! Playback's existing linear interpolation between consecutive intervals'
//! `pos` fields then traces a polyline approximation of the curve.
//!
//! The trapezoidal math itself lives in
//! `crate::pathfinding::fh_sipp::kinematics`. The search and the observer
//! consume the same canonical [`TrapezoidalProfile`] so rendered position
//! follows the committed execution profile.

use desgraph::{CircularArc, WorldPos};

use crate::observer::time_slice::BotStepKind;
use crate::pathfinding::{KinematicParams, TrapezoidalProfile, Velocity, trapezoid_position, trapezoidal_profile};

use super::nav_handler::OccupancyInterval;

/// Rasterize a committed `Move` waypoint into a sequence of micro-
/// `OccupancyInterval`s sampling the trapezoidal motion profile.
///
/// Each emitted interval ends at the world-space position the robot occupies
/// at the interval's `end_ms`. Playback's linear interpolation between
/// consecutive intervals' `pos` values traces a polyline that approximates
/// the trapezoidal curve at the configured `motion_step_ms` granularity.
///
/// Position follows the declared curve when present, otherwise the straight
/// segment. `distance_mm` is the full travel length, including tangent leads.
///
/// Slice-boundary handling: each emitted interval has its canonical end
/// position computed BEFORE it reaches `append_interval`. When an interval
/// crosses a slice boundary, `append_interval` falls back to linear
/// interpolation within that single (≤ `motion_step_ms`) micro-interval —
/// negligible error compared to the inter-cell linearization this whole
/// rasterization removes.
#[allow(clippy::too_many_arguments)]
pub(super) fn rasterize_move(
    intervals: &mut Vec<OccupancyInterval>,
    move_start_ms: u64,
    move_end_ms: u64,
    source_pos: WorldPos,
    dest_pos: WorldPos,
    dest_cell: u32,
    distance_mm: u64,
    entry_velocity_mm_s: f64,
    exit_velocity_mm_s: f64,
    params: Option<KinematicParams>,
    motion_step_ms: u32,
    curve: Option<CircularArc>,
    move_kind: BotStepKind,
) {
    let move_duration_ms = move_end_ms.saturating_sub(move_start_ms);
    if move_duration_ms == 0 {
        // Zero-duration Move shouldn't reach this branch in practice (the
        // outer caller already gates `iv.end_ms > iv.start_ms` before
        // emitting), but be conservative: fall back to a single end-anchored
        // interval rather than emitting an unbounded loop.
        intervals.push(OccupancyInterval {
            start_ms: move_start_ms,
            end_ms: move_end_ms,
            cell: dest_cell,
            pos: dest_pos,
            kind: move_kind,
        });
        return;
    }

    let profile: Option<TrapezoidalProfile> = if let Some(params) = params {
        match trapezoidal_profile(Velocity(entry_velocity_mm_s), Velocity(exit_velocity_mm_s), distance_mm, &params) {
            Ok(profile) => Some(profile),
            Err(error) => {
                eprintln!(
                    "rasterize_move: invalid kinematic profile ({error:?}); using constant progress on declared geometry"
                );
                None
            }
        }
    } else {
        None
    };

    // Sample at fixed simtime cadence. Each micro-interval lasts at most
    // `motion_step_ms`; the final one is truncated at the move's end so
    // total interval coverage matches `move_duration_ms` exactly.
    assert!(
        motion_step_ms > 0,
        "motion_step_ms must be > 0 to sample the trapezoidal curve (got 0; rasterization would infinite-loop)",
    );
    let mut step_ms = motion_step_ms as u64;
    if let Some(arc) = curve {
        let arc_start = arc.start.unwrap_or(source_pos);
        let radius = (arc_start.x_mm - arc.center.x_mm).hypot(arc_start.y_mm - arc.center.y_mm);
        let peak_speed =
            profile.as_ref().map_or(distance_mm as f64 * 1000.0 / move_duration_ms as f64, |p| p.peak_velocity_mm_s);
        // Keep the rendered chord segments below five degrees even when the
        // time-based cadence alone would visibly cut across a tight turn.
        let angular_step_ms = (radius * 5.0_f64.to_radians() * 1000.0 / peak_speed).floor().max(1.0) as u64;
        step_ms = step_ms.min(angular_step_ms);
    }
    let mut t_ms: u64 = 0;
    while t_ms < move_duration_ms {
        let t_end_ms = (t_ms + step_ms).min(move_duration_ms);
        let fraction = if t_end_ms == move_duration_ms {
            1.0
        } else if let Some(profile) = &profile {
            (trapezoid_position(profile, t_end_ms as f64 / 1000.0) / distance_mm as f64).clamp(0.0, 1.0)
        } else {
            t_end_ms as f64 / move_duration_ms as f64
        };
        let pos = curve.map_or_else(
            || WorldPos {
                x_mm: source_pos.x_mm + (dest_pos.x_mm - source_pos.x_mm) * fraction,
                y_mm: source_pos.y_mm + (dest_pos.y_mm - source_pos.y_mm) * fraction,
            },
            |arc| arc.point(source_pos, dest_pos, fraction),
        );
        intervals.push(OccupancyInterval {
            start_ms: move_start_ms + t_ms,
            end_ms: move_start_ms + t_end_ms,
            cell: dest_cell,
            pos,
            kind: move_kind,
        });
        t_ms = t_end_ms;
    }
}
