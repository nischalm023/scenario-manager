//! A VTM cycle combines the selected robot model's chassis and lift motions.
//! The axes move together, so each leg takes the longer of the two; handling
//! follows at each end. `CycleTimes` adapters retain legacy diagnostic inputs.

use super::model::VtmMotion;
use crate::model::SimDuration;
use crate::model::relay::config::DEFAULT_RELAY_HEIGHT_M;

/// Legacy settings and diagnostic input for a symmetric VTM cycle. Catalog
/// execution uses `VtmMotion` directly, retaining its chassis kind and load
/// profiles instead of reducing them to these fields.
///
/// Handling is keyed by **operation**, not by endpoint: a pick takes
/// `pick_secs` wherever it happens, and every cycle makes one pick and one
/// drop — forward picks at the rack and drops at the relay, return mirrors it.
/// Alignment is keyed the same way: the scan before the pick's fork insertion
/// and the scan before the drop's are separate figures, because a robot may
/// state them separately and the DFW6 bot specification does
/// (docs/walmart-facts.md section 6b).
///
/// [`CycleBreakdown`] still reports per *endpoint*; this is only the input
/// side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycleTimes {
    /// Fork transfer to take a tote. s.
    pub pick_secs: f64,
    /// Fork transfer to place a tote. s.
    pub drop_secs: f64,
    /// Level alignment scan before the fork insertion that takes a tote. s.
    pub pick_alignment_secs: f64,
    /// Level alignment scan before the fork insertion that places a tote. s.
    pub drop_alignment_secs: f64,
    /// Horizontal (aisle) max chassis speed. m/s.
    pub chassis_speed: f64,
    /// Horizontal chassis acceleration, and equally deceleration. m/s².
    pub chassis_accel: f64,
    /// Vertical fork-lift max speed. m/s.
    pub lift_speed: f64,
    /// Vertical fork-lift acceleration, and equally deceleration. m/s².
    pub lift_accel: f64,
}

/// A placeholder robot, which a run takes when its world's robot entities
/// carry no aspect or when it is assembled without any world, such as a unit
/// test: the simulator's former compiled defaults, not a real robot's figures.
/// `VtmConfig::default` takes its figures from here.
impl Default for CycleTimes {
    fn default() -> Self {
        Self {
            pick_secs: 4.0,
            drop_secs: 2.0,
            pick_alignment_secs: 1.933,
            drop_alignment_secs: 1.933,
            chassis_speed: 3.0,
            chassis_accel: 2.0,
            lift_speed: 3.0,
            lift_accel: 2.0,
        }
    }
}

impl CycleTimes {
    /// Fixed (non-travel) seconds per cycle: a pick and its alignment, a drop
    /// and its alignment. Direction-independent — a cycle does one of each.
    pub fn fixed_cycle_secs(&self) -> f64 {
        self.pick_secs + self.drop_secs + self.pick_alignment_secs + self.drop_alignment_secs
    }
}

/// Trapezoidal motion profile: time to travel `distance` meters with given
/// max speed and symmetric acceleration/deceleration.
pub(super) fn trapezoidal_time(distance: f64, max_speed: f64, accel: f64) -> f64 {
    if distance <= 0.0 {
        return 0.0;
    }
    let accel_dist = max_speed * max_speed / (2.0 * accel);
    if distance < 2.0 * accel_dist {
        // Triangle profile: never reach max speed.
        2.0 * (distance / accel).sqrt()
    } else {
        // Trapezoid: accel + cruise + decel.
        2.0 * (max_speed / accel) + (distance - 2.0 * accel_dist) / max_speed
    }
}

fn shelf_height_with(z_index: u16, vtc: u16, rack_height_m: f64) -> f64 {
    if vtc <= 1 {
        return 0.0;
    }
    (z_index as f64 / (vtc - 1) as f64) * rack_height_m
}

// ---------------------------------------------------------------------------
// Isochrone fill ordering
// ---------------------------------------------------------------------------

/// A rack position with its travel time from the relay (bottom-left origin).
#[derive(Clone, Copy)]
pub struct RackPosition {
    /// Horizontal position along aisle (meters).
    pub x_m: f64,
    /// Vertical height (meters).
    pub z_m: f64,
    /// Travel time from relay at (0, 0) in seconds.
    pub travel_secs: f64,
}

/// Generate rack positions sorted by travel time from the relay (origin at
/// bottom-left corner of the zone). Both axes use trapezoidal profiles and
/// move simultaneously, so travel_time = max(h_time, v_time).
///
/// Returns all `x_cells × z_levels` positions sorted fastest-first.
/// Use the first `fill_pct` fraction to model partially-filled racks.
///
/// This ordering is also suitable for inventory placement in the sim:
/// filling closest-first minimizes average VTM travel time.
pub fn isochrone_fill_order(
    aisle_length_m: f64,
    rack_height_m: f64,
    x_cells: usize,
    z_levels: usize,
    times: CycleTimes,
) -> Vec<RackPosition> {
    let mut positions = Vec::with_capacity(x_cells * z_levels);
    for xi in 0..x_cells {
        let x_m = if x_cells <= 1 { 0.0 } else { xi as f64 / (x_cells - 1) as f64 * aisle_length_m };
        for zi in 0..z_levels {
            let z_m = if z_levels <= 1 { 0.0 } else { zi as f64 / (z_levels - 1) as f64 * rack_height_m };
            let t = travel_time(x_m, z_m, times);
            positions.push(RackPosition { x_m, z_m, travel_secs: t });
        }
    }
    positions.sort_by(|a, b| a.travel_secs.partial_cmp(&b.travel_secs).unwrap());
    positions
}

// ---------------------------------------------------------------------------
// Synthetic workload VTM task time estimation
// ---------------------------------------------------------------------------

/// Whether the VTM does all tasks in one direction (pick-only or putaway-only)
/// or alternates forward/reverse (mixed pick+putaway).
#[derive(Clone, Copy, Debug)]
pub enum Directionality {
    /// forward → forward → forward ... (or reverse → reverse → ...)
    Unidirectional,
    /// forward → reverse → forward → reverse ...
    Bidirectional,
}

/// How the VTM picks its next horizontal position between tasks.
#[derive(Clone, Copy, Debug)]
pub enum Locality {
    /// Each task at a fully random x-position along the aisle.
    FullRandom,
    /// Bounded random walk: drift ±walk_cells between tasks.
    RandomWalk { walk_cells: u32 },
}

/// Result of a synthetic workload task time estimation.
#[derive(Debug)]
pub struct TaskTimeResult {
    pub n: usize,
    pub avg_secs: f64,
    pub min_secs: f64,
    pub max_secs: f64,
    /// Average travel time per task (horizontal + vertical repositioning).
    pub avg_travel_secs: f64,
    /// Average fork pick/place time per task.
    pub avg_fork_secs: f64,
    /// Average alignment time across both serviced endpoints per task.
    pub avg_alignment_secs: f64,
    /// Average relay-side pickup/drop transfer time per task.
    pub avg_relay_transfer_secs: f64,
}

impl std::fmt::Display for TaskTimeResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "avg={:.1}s [{:.1}-{:.1}]", self.avg_secs, self.min_secs, self.max_secs)
    }
}

/// Xorshift64 helper.
fn xor_next(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

/// Time for one VTM task, tracking full (x, z) state. The relay stands at the
/// default relay height, as in a zone whose map states no diving boards.
///
/// Returns (task time, travel, alignment, rack transfer, relay transfer, end x, end z).
///
/// Forward (storage→relay):
///   1. Travel from (vtm_x, vtm_z) to rack at (task_x, shelf_h)
///   2. Align + fork pick from rack
///   3. Travel down to relay at (task_x, relay height)
///   4. Align + place at relay
///   Ends at (task_x, relay height)
///
/// Reverse (relay→storage):
///   1. Travel from (vtm_x, vtm_z) to relay at (task_x, relay height)
///   2. Align + pick from relay
///   3. Travel up to rack at (task_x, shelf_h)
///   4. Align + fork place at rack
///   Ends at (task_x, shelf_h)
fn task_time_full(
    vtm_x: f64,
    vtm_z: f64,
    task_x: f64,
    shelf_h: f64,
    is_forward: bool,
    times: CycleTimes,
) -> (f64, f64, f64, f64, f64, f64, f64) {
    let bh = DEFAULT_RELAY_HEIGHT_M;

    let (first_h, fork, relay_transfer, end_z) = if is_forward {
        (shelf_h, times.pick_secs, times.drop_secs, bh) // vtm → rack (pick) → relay (drop)
    } else {
        (bh, times.drop_secs, times.pick_secs, shelf_h) // vtm → relay (pick) → rack (drop)
    };

    let leg1 = travel_time((vtm_x - task_x).abs(), (vtm_z - first_h).abs(), times); // to first endpoint
    let leg2 = travel_time(0.0, (shelf_h - bh).max(0.0), times); // vertical rack↔relay
    let alignment = times.pick_alignment_secs + times.drop_alignment_secs;
    let travel = leg1 + leg2;
    let task_time = travel + alignment + fork + relay_transfer;

    (task_time, travel, alignment, fork, relay_transfer, task_x, end_z)
}

/// Synthetic workload VTM task time estimation.
///
/// Simulates `n` VTM tasks within a zone of given dimensions, on the run's
/// cycle figures `times`, and reports average time per task. It adds nothing
/// the run's cycle does not: the 0.5 s response time per movement step and the
/// 80/20 shallow/deep fork weighting it once took from the A5 tool are gone,
/// since no source for either is in hand.
///
/// A **task** is one directional movement: forward (storage→relay) or
/// reverse (relay→storage). Each task involves horizontal travel to reach
/// the x-position, vertical travel to the shelf height, fork extension,
/// alignment and transfer at both serviced endpoints.
///
/// - `aisle_length_m`: physical horizontal length of the zone aisle.
/// - `cell_width_m`: width of one rack cell along the aisle (for random walk step size).
/// - `rack_height_m`: physical height of the racking.
/// - `vtc`: vertical tote capacity (slots per rack column).
/// - `dir`: unidirectional (pick-only) or bidirectional (mixed pick+putaway).
/// - `locality`: full random or bounded random walk.
/// - `physical_levels`: number of physical rack levels (NOT vtc hack).
/// - `fill_pct`: rack fill level (0.0-1.0). Positions filled closest-first
///   along isochrones from the relay origin.
/// - `n`: number of tasks to simulate.
pub fn estimate_task_time(
    aisle_length_m: f64,
    cell_width_m: f64,
    rack_height_m: f64,
    physical_levels: u16,
    dir: Directionality,
    locality: Locality,
    fill_pct: f64,
    n: usize,
    times: CycleTimes,
) -> TaskTimeResult {
    // Build isochrone-ordered positions using physical geometry.
    let x_cells = (aisle_length_m / cell_width_m).ceil().max(1.0) as usize;
    let z_levels = physical_levels.max(1) as usize;
    let all_positions = isochrone_fill_order(aisle_length_m, rack_height_m, x_cells, z_levels, times);
    let filled_count =
        ((all_positions.len() as f64 * fill_pct.clamp(0.01, 1.0)).ceil() as usize).max(1).min(all_positions.len());
    let filled = &all_positions[..filled_count];

    if n == 0 {
        return TaskTimeResult {
            n: 0,
            avg_secs: 0.0,
            min_secs: 0.0,
            max_secs: 0.0,
            avg_travel_secs: 0.0,
            avg_fork_secs: 0.0,
            avg_alignment_secs: 0.0,
            avg_relay_transfer_secs: 0.0,
        };
    }

    let mut rng: u64 = 0xCAFE_BABE_1234_5678;
    let mut sum = 0.0;
    let mut travel_sum = 0.0;
    let mut alignment_sum = 0.0;
    let mut fork_sum = 0.0;
    let mut relay_transfer_sum = 0.0;
    let mut min = f64::MAX;
    let mut max = 0.0f64;

    // VTM state: (x-position along aisle, z-height).
    let mut vtm_x: f64 = 0.0;
    let mut vtm_z: f64 = 0.0;

    for i in 0..n {
        let is_forward = match dir {
            Directionality::Unidirectional => true,
            Directionality::Bidirectional => i % 2 == 0,
        };

        // Pick a position from the filled set.
        let (task_x, shelf_h) = match locality {
            Locality::FullRandom => {
                let idx = (xor_next(&mut rng) % filled_count as u64) as usize;
                (filled[idx].x_m, filled[idx].z_m)
            }
            Locality::RandomWalk { walk_cells } => {
                if walk_cells == 0 {
                    // Degenerate: zero walk range → pick random from filled set.
                    let idx = (xor_next(&mut rng) % filled_count as u64) as usize;
                    (filled[idx].x_m, filled[idx].z_m)
                } else {
                    let step_cells = (xor_next(&mut rng) % walk_cells as u64) as f64 + 1.0;
                    let sign = if xor_next(&mut rng) % 2 == 0 { 1.0 } else { -1.0 };
                    let target_x = (vtm_x + sign * step_cells * cell_width_m).clamp(0.0, aisle_length_m);
                    let tolerance = cell_width_m * (walk_cells as f64 + 1.0);
                    let nearby: Vec<usize> = filled
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| (p.x_m - target_x).abs() <= tolerance)
                        .map(|(i, _)| i)
                        .collect();
                    if nearby.is_empty() {
                        let idx = (xor_next(&mut rng) % filled_count as u64) as usize;
                        (filled[idx].x_m, filled[idx].z_m)
                    } else {
                        let idx = nearby[(xor_next(&mut rng) % nearby.len() as u64) as usize];
                        (filled[idx].x_m, filled[idx].z_m)
                    }
                }
            }
        };

        let (t, travel, alignment, fork, relay_transfer, end_x, end_z) =
            task_time_full(vtm_x, vtm_z, task_x, shelf_h, is_forward, times);
        sum += t;
        travel_sum += travel;
        alignment_sum += alignment;
        fork_sum += fork;
        relay_transfer_sum += relay_transfer;
        min = min.min(t);
        max = max.max(t);
        vtm_x = end_x;
        vtm_z = end_z;
    }

    TaskTimeResult {
        n,
        avg_secs: sum / n as f64,
        min_secs: min,
        max_secs: max,
        avg_travel_secs: travel_sum / n as f64,
        avg_fork_secs: fork_sum / n as f64,
        avg_alignment_secs: alignment_sum / n as f64,
        avg_relay_transfer_secs: relay_transfer_sum / n as f64,
    }
}

/// Uniform-pitch shelf height in meters for an explicit rack height
/// (public wrapper over the geometry-parameterized model).
pub fn shelf_height_uniform(z_index: u16, physical_shelves: u16, rack_height_m: f64) -> f64 {
    shelf_height_with(z_index, physical_shelves, rack_height_m)
}

fn travel_time(h_distance_m: f64, v_distance_m: f64, times: CycleTimes) -> f64 {
    let h_time = trapezoidal_time(h_distance_m, times.chassis_speed, times.chassis_accel);
    let v_time = trapezoidal_time(v_distance_m, times.lift_speed, times.lift_accel);
    h_time.max(v_time)
}

/// Detailed breakdown of a VTM cycle leg.
///
/// `PartialEq` supports `geo_cycle != cycle` divergence checks; equality is
/// exact — non-diverged cycles are clones, never re-derived.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleBreakdown {
    /// First travel leg. Forward: current/relay → rack. Return: current/rack → relay.
    pub first_travel_secs: f64,
    /// Second travel leg. Forward: rack → relay. Return: relay → rack.
    pub second_travel_secs: f64,
    /// Travel to target: max(horizontal, vertical). Axes move simultaneously.
    pub travel_secs: f64,
    /// Horizontal component of travel.
    pub h_travel_secs: f64,
    /// Vertical component of travel.
    pub v_travel_secs: f64,
    /// Alignment before the rack-side fork insertion.
    pub rack_alignment_secs: f64,
    /// Rack-side fork transfer and retraction.
    pub rack_transfer_secs: f64,
    /// Alignment before the relay-side fork insertion.
    pub relay_alignment_secs: f64,
    /// Relay-side pickup or drop transfer.
    pub relay_transfer_secs: f64,
    /// Total = travel + both alignments + both transfers.
    pub total_secs: f64,
}

impl CycleBreakdown {
    /// Total cycle time as a `SimDuration`.
    // TODO: compute cycle timings in SimDuration natively rather than f64 secs.
    pub fn total(&self) -> SimDuration {
        SimDuration::ms((self.total_secs * 1000.0) as u64)
    }
}

impl std::fmt::Display for CycleBreakdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.1}s total (travel {:.1}s [h={:.1} v={:.1}], alignment {:.1}s, rack transfer {:.1}s, relay transfer {:.1}s)",
            self.total_secs,
            self.travel_secs,
            self.h_travel_secs,
            self.v_travel_secs,
            self.rack_alignment_secs + self.relay_alignment_secs,
            self.rack_transfer_secs,
            self.relay_transfer_secs,
        )
    }
}

/// Forward leg: navigate to rack, pick tote, navigate to relay, place tote,
/// with an explicit tote height in meters.
///
/// `tote_height` is the slot's floor/beam height — the fork-interface height
/// where the forks slide under the tote — not a slot midpoint. The pick/lift
/// micro-motion is folded into the fork-time constants.
pub fn forward_cycle_with_heights(
    h_dist_to_rack: f64,
    h_dist_rack_to_relay: f64,
    tote_height: f64,
    relay_height: f64,
    times: CycleTimes,
) -> CycleBreakdown {
    forward_cycle_with_model(h_dist_to_rack, h_dist_rack_to_relay, tote_height, relay_height, times.into())
}

/// Forward cycle for one selected model: empty to the rack, laden to the relay.
pub fn forward_cycle_with_model(
    h_dist_to_rack: f64,
    h_dist_rack_to_relay: f64,
    tote_height: f64,
    relay_height: f64,
    model: VtmMotion,
) -> CycleBreakdown {
    let times = model.handling;
    // 1. Travel to rack (horizontal + vertical simultaneous)
    let h1 = model.chassis.travel_secs(h_dist_to_rack, false);
    let v1 = model.lift.travel_secs(tote_height);
    let leg1 = h1.max(v1);

    // 2. Align and pick the tote from the rack.
    let rack_alignment = times.pick_alignment_secs;
    let rack_transfer = times.pick_secs;

    // 3. Travel to relay (horizontal + vertical down simultaneous)
    let v_down = (tote_height - relay_height).abs();
    let h2 = model.chassis.travel_secs(h_dist_rack_to_relay, true);
    let v2 = model.lift.travel_secs(v_down);
    let leg2 = h2.max(v2);

    // 4. Align and drop the tote into the relay.
    let relay_alignment = times.drop_alignment_secs;
    let relay_transfer = times.drop_secs;

    CycleBreakdown {
        first_travel_secs: leg1,
        second_travel_secs: leg2,
        travel_secs: leg1 + leg2,
        h_travel_secs: h1 + h2,
        v_travel_secs: v1 + v2,
        rack_alignment_secs: rack_alignment,
        rack_transfer_secs: rack_transfer,
        relay_alignment_secs: relay_alignment,
        relay_transfer_secs: relay_transfer,
        total_secs: leg1 + rack_alignment + rack_transfer + leg2 + relay_alignment + relay_transfer,
    }
}

/// Return leg: navigate to relay, pick tote, navigate to rack, place tote,
/// with an explicit tote height in meters.
/// See [`forward_cycle_with_heights`] for the height semantics.
pub fn return_cycle_with_heights(
    h_dist_to_relay: f64,
    h_dist_relay_to_rack: f64,
    tote_height: f64,
    relay_height: f64,
    times: CycleTimes,
) -> CycleBreakdown {
    return_cycle_with_model(h_dist_to_relay, h_dist_relay_to_rack, tote_height, relay_height, times.into())
}

/// Return cycle for one selected model: empty to the relay, laden to the rack.
pub fn return_cycle_with_model(
    h_dist_to_relay: f64,
    h_dist_relay_to_rack: f64,
    tote_height: f64,
    relay_height: f64,
    model: VtmMotion,
) -> CycleBreakdown {
    let times = model.handling;
    // 1. Travel to relay (horizontal + vertical to relay height)
    let h1 = model.chassis.travel_secs(h_dist_to_relay, false);
    let v1 = model.lift.travel_secs(relay_height);
    let leg1 = h1.max(v1);

    // 2. Align and pick the tote from the relay.
    let relay_alignment = times.pick_alignment_secs;
    let relay_transfer = times.pick_secs;

    // 3. Travel to rack (horizontal + vertical up to shelf)
    let v_up = (tote_height - relay_height).abs();
    let h2 = model.chassis.travel_secs(h_dist_relay_to_rack, true);
    let v2 = model.lift.travel_secs(v_up);
    let leg2 = h2.max(v2);

    // 4. Align and drop the tote into the rack.
    let rack_alignment = times.drop_alignment_secs;
    let rack_transfer = times.drop_secs;

    CycleBreakdown {
        first_travel_secs: leg1,
        second_travel_secs: leg2,
        travel_secs: leg1 + leg2,
        h_travel_secs: h1 + h2,
        v_travel_secs: v1 + v2,
        rack_alignment_secs: rack_alignment,
        rack_transfer_secs: rack_transfer,
        relay_alignment_secs: relay_alignment,
        relay_transfer_secs: relay_transfer,
        total_secs: leg1 + relay_alignment + relay_transfer + leg2 + rack_alignment + rack_transfer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The geometry these tests are written on: a 9 m rack, travelled by the
    /// placeholder robot.
    const RACK_HEIGHT_M: f64 = 9.0;

    fn shelf_height(z_index: u16, vtc: u16) -> f64 {
        shelf_height_with(z_index, vtc, RACK_HEIGHT_M)
    }

    fn forward_cycle(to_rack: f64, rack_to_relay: f64, z_index: u16, vtc: u16, relay: f64) -> CycleBreakdown {
        forward_cycle_with_heights(to_rack, rack_to_relay, shelf_height(z_index, vtc), relay, CycleTimes::default())
    }

    fn return_cycle(to_relay: f64, relay_to_rack: f64, z_index: u16, vtc: u16, relay: f64) -> CycleBreakdown {
        return_cycle_with_heights(to_relay, relay_to_rack, shelf_height(z_index, vtc), relay, CycleTimes::default())
    }

    fn dual_cycle_secs(to_rack: f64, rack_to_relay: f64, z_index: u16, vtc: u16) -> f64 {
        forward_cycle(to_rack, rack_to_relay, z_index, vtc, 0.0).total_secs
            + return_cycle(rack_to_relay, to_rack, z_index, vtc, 0.0).total_secs
    }

    #[test]
    fn trapezoidal_short_distance() {
        // 1m at 3m/s, 2m/s² — never reaches max speed (triangle)
        let t = trapezoidal_time(1.0, 3.0, 2.0);
        assert!((t - 1.414).abs() < 0.01, "got {t}"); // 2*sqrt(1/2)
    }

    #[test]
    fn trapezoidal_long_distance() {
        // 30m at 3m/s, 2m/s² — reaches max speed
        // accel_dist = 9/4 = 2.25m, cruise = 30 - 4.5 = 25.5m
        // time = 2*(3/2) + 25.5/3 = 3 + 8.5 = 11.5s
        let t = trapezoidal_time(30.0, 3.0, 2.0);
        assert!((t - 11.5).abs() < 0.01, "got {t}");
    }

    #[test]
    fn shelf_height_bounds() {
        assert!((shelf_height(0, 10) - 0.0).abs() < 0.001);
        assert!((shelf_height(9, 10) - 9.0).abs() < 0.001); // top shelf
        assert!((shelf_height(4, 10) - 4.0).abs() < 0.001); // mid shelf
    }

    #[test]
    fn dual_cycle_sanity() {
        // A dual cycle is two cycles, so it charges the ruled non-travel
        // cost twice. Subtracting it leaves the travel these bounds are
        // actually about — otherwise a ruling that moves the constants
        // re-breaks the bound, which is how this test first went red.
        let overhead = 2.0 * CycleTimes::default().fixed_cycle_secs();

        // Ground level tote, 10m horizontal each way, vtc=10
        let t = dual_cycle_secs(10.0, 10.0, 0, 10);
        // No vertical movement, just horizontal travel
        let travel = t - overhead;
        assert!(travel > 10.0 && travel < 25.0, "travel {travel} (total {t}, overhead {overhead})");

        // Top shelf tote, 10m horizontal — vertical (9m) is shorter than
        // horizontal (10m) at same speed/accel, so horizontal dominates.
        let t_top = dual_cycle_secs(10.0, 10.0, 9, 10);
        assert!(t_top >= t, "top shelf should be >= ground level");

        // Short horizontal, top shelf — vertical should dominate
        let t_short = dual_cycle_secs(2.0, 2.0, 9, 10);
        let t_short_ground = dual_cycle_secs(2.0, 2.0, 0, 10);
        assert!(t_short > t_short_ground, "vertical should dominate at short horizontal");
    }

    #[test]
    fn matches_spec_ballpark() {
        // Spec: 23.4s outbound, 17.9s inbound for their reference layout
        // (32m aisle, 11.6m max height, avg height 5.6m)
        // Our config: 24m aisle (half = 12m avg travel), 9m rack, mid-shelf
        let mid = forward_cycle(12.0, 12.0, 5, 10, 0.0).total_secs + return_cycle(12.0, 12.0, 5, 10, 0.0).total_secs;
        let top = forward_cycle(12.0, 12.0, 9, 10, 0.0).total_secs + return_cycle(12.0, 12.0, 9, 10, 0.0).total_secs;
        let ground = forward_cycle(12.0, 12.0, 0, 10, 0.0).total_secs + return_cycle(12.0, 12.0, 0, 10, 0.0).total_secs;

        // Dual-cycle time grows monotonically with shelf height, and the
        // mid-shelf cycle lands in the spec ballpark.
        assert!(top >= mid && mid >= ground, "top={top:.1} mid={mid:.1} ground={ground:.1}");
        assert!(mid > 30.0 && mid < 55.0, "mid={mid:.1}");
    }

    #[test]
    fn every_fork_insertion_has_alignment_and_transfers_follow_the_operation() {
        // Previously asserted `relay_transfer_secs == 2.0` in both directions,
        // i.e. transfer keyed by endpoint. The DA50 spec sheet keys it by
        // operation — "Pick ... SD" 6.2 s vs "Drop ... SD" 5.0 s — so the relay
        // carries the drop on a forward cycle and the pick on a return.
        // Alignment follows the same operation, one scan per fork insertion.
        let fwd = forward_cycle(12.0, 12.0, 5, 10, 0.0);
        let ret = return_cycle(12.0, 12.0, 5, 10, 0.0);
        let d = CycleTimes::default();

        assert_eq!(fwd.rack_transfer_secs, d.pick_secs, "forward picks at the rack");
        assert_eq!(fwd.relay_transfer_secs, d.drop_secs, "forward drops at the relay");
        assert_eq!(ret.relay_transfer_secs, d.pick_secs, "return picks at the relay");
        assert_eq!(ret.rack_transfer_secs, d.drop_secs, "return drops at the rack");

        assert_eq!(fwd.rack_alignment_secs, d.pick_alignment_secs, "forward aligns for a pick at the rack");
        assert_eq!(fwd.relay_alignment_secs, d.drop_alignment_secs, "forward aligns for a drop at the relay");
        assert_eq!(ret.relay_alignment_secs, d.pick_alignment_secs, "return aligns for a pick at the relay");
        assert_eq!(ret.rack_alignment_secs, d.drop_alignment_secs, "return aligns for a drop at the rack");

        for cycle in [fwd, ret] {
            assert_eq!(
                cycle.total_secs,
                cycle.travel_secs
                    + cycle.rack_alignment_secs
                    + cycle.rack_transfer_secs
                    + cycle.relay_alignment_secs
                    + cycle.relay_transfer_secs
            );
            // One pick and one drop either way: the total is direction-independent.
            assert!((cycle.total_secs - cycle.travel_secs - d.fixed_cycle_secs()).abs() < 1e-9);
        }
    }

    #[test]
    fn synthetic_workload_all_variants() {
        // 24m aisle, 1m cell width, 9m rack, vtc=740, n=10000
        let configs = [
            ("Uni/Random", Directionality::Unidirectional, Locality::FullRandom),
            ("Uni/Walk", Directionality::Unidirectional, Locality::RandomWalk { walk_cells: 3 }),
            ("Bi/Random", Directionality::Bidirectional, Locality::FullRandom),
            ("Bi/Walk", Directionality::Bidirectional, Locality::RandomWalk { walk_cells: 3 }),
        ];
        for (label, dir, loc) in &configs {
            let mc = estimate_task_time(24.0, 1.0, 9.0, 10, *dir, *loc, 1.0, 10000, CycleTimes::default());
            assert!(mc.avg_secs > 5.0 && mc.avg_secs < 40.0, "{label}: avg={:.1}s out of range", mc.avg_secs);
        }
        // Random walk should be faster than full random.
        let full = estimate_task_time(
            24.0,
            1.0,
            9.0,
            10,
            Directionality::Unidirectional,
            Locality::FullRandom,
            1.0,
            10000,
            CycleTimes::default(),
        );
        let walk = estimate_task_time(
            24.0,
            1.0,
            9.0,
            10,
            Directionality::Unidirectional,
            Locality::RandomWalk { walk_cells: 3 },
            1.0,
            10000,
            CycleTimes::default(),
        );
        assert!(
            walk.avg_secs < full.avg_secs,
            "walk ({:.1}s) should be faster than full random ({:.1}s)",
            walk.avg_secs,
            full.avg_secs
        );
    }

    /// The estimate travels and handles on the run's figures, none of them the
    /// placeholder's. With one slot, at the floor of the rack, every task is
    /// forward and stays put, so each after the first travels only the lift
    /// down from the relay; with one slot at each end of a 10 m aisle, the
    /// longest task is a chassis run the length of the aisle.
    #[test]
    fn the_estimate_travels_and_handles_on_the_runs_figures() {
        let times = CycleTimes {
            pick_secs: 6.2,
            drop_secs: 5.0,
            pick_alignment_secs: 3.56,
            drop_alignment_secs: 2.3,
            chassis_speed: 1.5,
            chassis_accel: 0.5,
            lift_speed: 1.0,
            lift_accel: 0.25,
        };
        let uni = Directionality::Unidirectional;
        let one_slot = estimate_task_time(10.0, 20.0, 5.0, 2, uni, Locality::FullRandom, 0.5, 2, times);
        assert_eq!(one_slot.avg_fork_secs, 6.2, "a forward task picks at the rack");
        assert_eq!(one_slot.avg_relay_transfer_secs, 5.0, "and drops at the relay");
        assert_eq!(one_slot.avg_alignment_secs, 3.56 + 2.3);
        let lower = trapezoidal_time(DEFAULT_RELAY_HEIGHT_M, 1.0, 0.25);
        assert_eq!(one_slot.avg_travel_secs, lower / 2.0, "{one_slot:?}");

        let aisle_ends = estimate_task_time(10.0, 5.0, 5.0, 1, uni, Locality::FullRandom, 1.0, 50, times);
        let run = trapezoidal_time(10.0, 1.5, 0.5);
        assert!((aisle_ends.max_secs - (run + 3.56 + 2.3 + 6.2 + 5.0)).abs() < 1e-9, "{aisle_ends:?}");
    }
}
