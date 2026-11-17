//! Within `observer`, `nav_handler` turns a MOVEMENT COMMIT into recorded
//! fact: it reads `NavEventData`'s raw pathfinder waypoint chain and writes
//! the three things downstream consumers need from it — per-slice occupancy
//! (`BotStep`s, for playback), the D30's journal rows (`htm_leg_log`, for
//! the reports) and the D30's travel and route-hold spans for the robot state
//! channel (`robot_state`). It also owns the journal's `Wait` rows for holds
//! whose bounding events land elsewhere, because `htm_leg_log` has one writer.
//!
//! Processes `NavEventData`'s raw pathfinder waypoint chain, dispatching on
//! [`WaypointAction`] to emit semantically-tagged [`BotStep`]s. Move steps
//! lerp position; Wait/Rotate steps hold in place. The handler also
//! synthesizes Wait BotSteps to fill any gap between when the bot stops
//! moving and when its next action begins (e.g., a bot waiting for an
//! oracle reservation, or sitting idle between an `HtmStore` and a
//! `ParkIdleHtms` dispatch).
//!
//! ## Why per-step `Wait` synthesis matters
//!
//! Without explicit Wait fillers, the renderer's "lerp prev_pos → step.pos
//! over delta_ms" interpolation paints a smooth drift across the dwell
//! window. For a bot that finishes Move-into-c64 at T=19s and starts
//! Move-into-c65 at T=47s (after a 28-second wait at c64), collapsing the
//! 28-second dwell into the next step's `delta_ms` makes the bot appear to
//! drift toward c65 over the entire wait — visually broaching c65 well
//! before it actually moves. Inserting a Wait step at c64 for the dwell
//! preserves "stationary at c64" semantics.

use desgraph::{CellId, CellKind, RelayId, RobotType, WorldPos};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use worldview::frames::robot_state::{RobotState, WaitCause};

use desgraph::{StationId, StationType};

use crate::events::{NavEventData, TripEndpoint, TripPurpose, VtmNavEventData, VtmNavKind};
use crate::model::{RobotId, SimDuration, SimTime};
use crate::observer::activity::{ActivityKind, LegNav, Milestone};
use crate::observer::duty::BusyInterval;
use crate::observer::output::{
    Activity, HtmHold, HtmJournalEndpoint, HtmLegRecord, HtmWaitCause, HtmWaitPosition, HtmWaitReason, TripType,
};
use crate::observer::robot_state::{Claim, role_of};
use crate::observer::time_slice::{BotLocation, BotStep, BotStepKind, ScalarCounters, standing_height_mm};
use crate::observer::traffic::MotionSpan;
use crate::pathfinding::Waypoint;
use crate::pathfinding::{MotionModel, Velocity, WaypointAction, trapezoidal_profile};

use super::nav_kinematic::rasterize_move;

use super::state::{SimObserverState, TripContext};

/// One contiguous interval of bot occupancy: `[start_ms, end_ms]` at `cell`/`pos`,
/// labeled with `kind`. The handler builds a vector of these from the raw
/// waypoint chain (filling Wait gaps explicitly), then emits each interval
/// to the time slices, splitting at slice boundaries.
pub(super) struct OccupancyInterval {
    pub(super) start_ms: u64,
    pub(super) end_ms: u64,
    pub(super) cell: u32,
    pub(super) pos: WorldPos,
    pub(super) kind: BotStepKind,
}

/// One HTM nav chain cut into its travel runs and holds, in order, and the
/// turning after its last run, which belongs to that run.
struct HtmChain {
    segments: Vec<ChainSegment>,
    trailing_angle_millidegrees: u64,
}

/// One cut of an HTM nav chain: a travel run, or a hold at one cell. `moving`
/// is the run's `Move` time alone — the share of the run that carried the
/// robot somewhere, which is what distance splits on.
enum ChainSegment {
    Travel { start: SimTime, end: SimTime, moving: SimDuration, angle_millidegrees: u64 },
    Hold { start: SimTime, end: SimTime, cell: CellId },
}

/// Append a hold, merging into a trailing hold that abuts it at the same cell
/// so a gap directly before a `Wait` waypoint reads as one stationary row
/// rather than two.
fn push_hold(segments: &mut Vec<ChainSegment>, start: SimTime, end: SimTime, cell: CellId) {
    if end <= start {
        return;
    }
    if let Some(ChainSegment::Hold { end: prev_end, cell: prev_cell, .. }) = segments.last_mut()
        && *prev_end == start
        && *prev_cell == cell
    {
        *prev_end = end;
        return;
    }
    segments.push(ChainSegment::Hold { start, end, cell });
}

impl SimObserverState {
    pub(super) fn handle_nav(&mut self, data: &NavEventData) {
        let htm_model = (data.robot_type == RobotType::Htm).then(|| self.config.htm_model(data.robot_id));
        let params = htm_model
            .as_ref()
            .filter(|model| model.kind == MotionModel::HtmKinematic)
            .map(|model| if data.carrying_cargo { model.kinematics.loaded } else { model.kinematics.unloaded });
        let bot_key = data.robot_id.0;
        // Where the leg sets off from, before this nav moves the bot on.
        let leg_origin = self.bot_positions.get(&bot_key).copied().map(CellId);
        if let (Some(first), Some(last)) = (data.waypoints.first(), data.waypoints.last()) {
            self.begin_navigation(bot_key, first.start_time().as_ms(), last.end_time().as_ms());
        }
        let is_vtm = matches!(data.robot_type, RobotType::Vtm);
        let sw = self.config.slice_width_ms;

        // One trip-context read serves all three consumers below: distance
        // attribution, the loaded-Wait KPI routing, and the per-leg journal.
        let trip_ctx: Option<TripContext> =
            if is_vtm { None } else { self.current_trip.get(&RobotId(bot_key)).copied() };

        // Accumulate travel distance, an HTM's by trip purpose too.
        if data.distance_mm > 0 {
            let slice_idx = (data.time.as_ms() / sw) as usize;
            let purpose = trip_ctx.map(|context| context.purpose);
            book_travel(&mut self.slice_to_write(slice_idx).scalars, is_vtm, purpose, data.distance_mm);
        }

        // KPI #4/#5: an explicit loaded `Wait` waypoint is a corridor-
        // deconfliction hold the pathfinder baked into the leg. Resolve the
        // routing once, all the way to the overlay counter: to-station =>
        // inbound "can't present yet" (#4), to-return-relay => outbound
        // "blocked returning" (#5). Only loaded, non-VTM HTMs on those two
        // legs qualify (trip_ctx is None for VTMs); the booking happens inline
        // in the `Wait` arm below so the synthetic gap-filler Waits (which
        // aren't `WaypointAction::Wait`) stay excluded. Empty-leg purposes are
        // spelled out so a future loaded-leg variant forces a routing decision
        // here instead of silently booking nothing.
        let loaded_wait_counter: Option<fn(&mut ScalarCounters) -> &mut u64> = if data.carrying_cargo {
            match trip_ctx.map(|ctx| ctx.purpose) {
                Some(TripPurpose::ToStation) => Some(|s| &mut s.htm_duty_wait_loaded_ms),
                Some(TripPurpose::ToReturnRelay) => Some(|s| &mut s.htm_duty_dwell_loaded_ms),
                Some(TripPurpose::ToRelay | TripPurpose::ToParking) | None => None,
            }
        } else {
            None
        };

        // --- Build occupancy intervals from the raw waypoint chain ---
        // Walk the chain, dispatching on WaypointAction. PathStart/PathEnd are
        // chain markers that update tracking but don't emit BotSteps. Move
        // emits a Move interval; Wait/Rotate emit a Wait/Rotate interval at
        // the bot's current position. Any gap between an action's `start` and
        // the cursor (`current_time_ms`) is filled with a synthetic Wait at
        // the prior cell — this is the dwell that makes Wait waypoints look
        // stationary instead of squished into the next Move's lerp.
        let prior_cell = self.bot_positions.get(&bot_key).copied();
        let prior_pos = self.bot_world_pos.get(&bot_key).copied();
        let mut current_cell = prior_cell.unwrap_or(0);
        let mut current_pos = prior_pos.unwrap_or_default();
        let mut current_time_ms = data.time.as_ms();
        let mut intervals: Vec<OccupancyInterval> = Vec::new();
        let mut motions = Vec::new();

        // Per-waypoint cumulative grid update (visit + presence).
        // Keyed off the waypoint, not the synthesized intervals, so phantom
        // dwell fillers don't double-count cell visits.
        for w in &data.waypoints {
            let cell_key = w.cell.0;
            let cell_pos = self.config.cell_pos(cell_key);
            let delta_ms = w.duration().as_ms();

            // PathStart/PathEnd are chain markers (zero-duration). They share
            // a cell with an adjacent Move/Wait/Rotate, so counting them as
            // visits would double-count.
            let is_marker = matches!(w.action, WaypointAction::PathStart { .. } | WaypointAction::PathEnd { .. });
            if !is_marker {
                // The same visit is booked twice: once into the run's totals,
                // and once into the slice the waypoint STARTS in. Booking it
                // whole to one slice is what the totals already do, so the
                // slices add back up to them exactly.
                let slice_idx = (w.start_time().as_ms() / self.config.slice_width_ms) as usize;
                if is_vtm {
                    *self.output.cumulative_grid.nav_visits_vtm.entry(cell_key).or_insert(0) += 1;
                    *self.output.cumulative_grid.robot_presence_vtm_ms.entry(cell_key).or_insert(0) += delta_ms;
                    let slice_grid = self.grid_share(slice_idx);
                    *slice_grid.nav_visits_vtm.entry(cell_key).or_insert(0) += 1;
                    *slice_grid.robot_presence_vtm_ms.entry(cell_key).or_insert(0) += delta_ms;
                } else {
                    *self.output.cumulative_grid.nav_visits_htm.entry(cell_key).or_insert(0) += 1;
                    *self.output.cumulative_grid.robot_presence_htm_ms.entry(cell_key).or_insert(0) += delta_ms;
                    let slice_grid = self.grid_share(slice_idx);
                    *slice_grid.nav_visits_htm.entry(cell_key).or_insert(0) += 1;
                    *slice_grid.robot_presence_htm_ms.entry(cell_key).or_insert(0) += delta_ms;
                }
            }

            match &w.action {
                WaypointAction::PathStart { time } => {
                    // The committed path declares its starting position and time.
                    current_cell = cell_key;
                    current_pos = cell_pos;
                    current_time_ms = time.as_ms();
                }
                WaypointAction::PathEnd { .. } => {
                    // Marker only — tracking already updated by the prior Move.
                }
                WaypointAction::Move { start, end, entry_velocity_mm_s, exit_velocity_mm_s } => {
                    let move_start = start.as_ms();
                    let move_end = end.as_ms();
                    if move_start > current_time_ms {
                        intervals.push(OccupancyInterval {
                            start_ms: current_time_ms,
                            end_ms: move_start,
                            cell: current_cell,
                            pos: current_pos,
                            kind: BotStepKind::Wait,
                        });
                    }
                    let curve = self.config.link_curves.get(&(CellId(current_cell), w.cell)).copied();
                    let move_kind = if data.robot_type == RobotType::Htm
                        && self.config.htm_reverse_links.contains(&(CellId(current_cell), w.cell))
                    {
                        BotStepKind::Reverse
                    } else {
                        BotStepKind::Move
                    };
                    let move_distance_mm = curve
                        .map_or_else(
                            || (cell_pos.x_mm - current_pos.x_mm).hypot(cell_pos.y_mm - current_pos.y_mm),
                            |arc| arc.length_mm(current_pos, cell_pos),
                        )
                        .round() as u64;
                    let kinematic = params.is_some();
                    if data.robot_type == RobotType::Htm && move_distance_mm > 0 {
                        let profile = params.and_then(|params| {
                            trapezoidal_profile(
                                Velocity(*entry_velocity_mm_s),
                                Velocity(*exit_velocity_mm_s),
                                move_distance_mm,
                                &params,
                            )
                            .ok()
                        });
                        motions.push(MotionSpan {
                            source: current_cell,
                            destination: cell_key,
                            start_ms: move_start,
                            end_ms: move_end,
                            distance_mm: move_distance_mm as f64,
                            profile,
                        });
                    }
                    let needs_rasterization = (kinematic || curve.is_some()) && move_distance_mm > 0;
                    if needs_rasterization {
                        rasterize_move(
                            &mut intervals,
                            move_start,
                            move_end,
                            current_pos,
                            cell_pos,
                            cell_key,
                            move_distance_mm,
                            *entry_velocity_mm_s,
                            *exit_velocity_mm_s,
                            params,
                            self.config.motion_step_ms,
                            curve,
                            move_kind,
                        );
                    } else {
                        intervals.push(OccupancyInterval {
                            start_ms: move_start,
                            end_ms: move_end,
                            cell: cell_key,
                            pos: cell_pos,
                            kind: move_kind,
                        });
                    }
                    current_cell = cell_key;
                    current_pos = cell_pos;
                    current_time_ms = move_end;
                }
                WaypointAction::Wait { start, end } | WaypointAction::Rotate { start, end } => {
                    let act_start = start.as_ms();
                    let act_end = end.as_ms();
                    if act_start > current_time_ms {
                        intervals.push(OccupancyInterval {
                            start_ms: current_time_ms,
                            end_ms: act_start,
                            cell: current_cell,
                            pos: current_pos,
                            kind: BotStepKind::Wait,
                        });
                    }
                    let kind = if matches!(w.action, WaypointAction::Rotate { .. }) {
                        // Rotation is a carve-out subset of HTM travel duty:
                        // turning in place happens inside a leg.
                        // Tally it separately from the authoritative Rotate
                        // window, clipping per slice the same way duty does.
                        // VTMs never rotate, so this is HTM-only.
                        if !is_vtm {
                            self.add_htm_duty_overlay(*start, *end, |s| &mut s.htm_duty_rotating_ms);
                        }
                        BotStepKind::Rotate
                    } else {
                        // KPI #4/#5: book this loaded `Wait` window into the
                        // leg's overlay (routing resolved as
                        // `loaded_wait_counter`). The leg's opening wait counts
                        // like every later one: a D30 the path search parks on
                        // the presentation cell until the outbound corridor
                        // clears is blocked returning, holding both the tote and
                        // the station's only presentation cell.
                        if let Some(select) = loaded_wait_counter {
                            self.add_htm_duty_overlay(*start, *end, select);
                        }
                        BotStepKind::Wait
                    };
                    intervals.push(OccupancyInterval {
                        start_ms: act_start,
                        end_ms: act_end,
                        cell: cell_key,
                        pos: cell_pos,
                        kind,
                    });
                    // Position unchanged.
                    current_time_ms = act_end;
                }
            }
        }

        // The chain's travel runs and holds (HTM only) are what the robot state
        // channel states of this nav, and the journal's rows where a TripStart
        // gave the trip its purpose; a nav before any is journaled nowhere, never
        // with a fabricated record.
        if !is_vtm {
            let chain = self.htm_chain(data);
            self.note_htm_chain(data, trip_ctx, &chain.segments);
            if let Some(ctx) = trip_ctx {
                self.push_htm_chain_rows(data, ctx, &chain);
            }
        }

        // --- Emit intervals to slices, splitting at slice boundaries ---
        for iv in &intervals {
            if iv.end_ms <= iv.start_ms {
                continue;
            }
            self.append_interval(bot_key, data.robot_type, iv);
        }

        for motion in motions {
            if motion.end_ms <= motion.start_ms {
                continue;
            }
            for index in (motion.start_ms / sw)..=((motion.end_ms - 1) / sw) {
                let location = self
                    .slice_to_write(index as usize)
                    .bot_locations
                    .get_mut(&bot_key)
                    .expect("playback covers committed motion");
                location.motion.get_or_insert_with(Vec::new).push(motion.clone());
            }
        }

        // Update tracked position to the chain's final state.
        self.bot_positions.insert(bot_key, current_cell);
        self.bot_world_pos.insert(bot_key, current_pos);

        if let (false, Some(w)) = (is_vtm, data.waypoints.last()) {
            let leg_nav = LegNav { arrival: w.end_time(), carrying: data.carrying_cargo };
            self.report_leg(data.robot_id, data.time, leg_nav, leg_origin);
            let role = role_of(self.hold_position(CellId(current_cell)));
            self.robot_state_log.stand(data.robot_id, w.end_time(), role);
        }
    }

    /// Cut one HTM nav chain into its travel runs and holds, in order: what the
    /// journal's rows and the robot state channel's spans are both read from.
    fn htm_chain(&self, data: &NavEventData) -> HtmChain {
        let mut segments: Vec<ChainSegment> = Vec::new();
        // `bot_positions` is keyed and valued by raw ids; convert at the read.
        let mut cell = CellId(self.bot_positions.get(&data.robot_id.0).copied().unwrap_or(0));
        let mut cursor = data.time;
        // Geometric turns accumulate between rows and are handed to the next
        // travel run, so the chain's total is conserved however it is cut.
        let mut pending_angle_millidegrees: u64 = 0;
        let mut prev_waypoint: Option<&Waypoint> = None;

        for waypoint in &data.waypoints {
            if let Some(prev) = prev_waypoint {
                let curve = if matches!(waypoint.action, WaypointAction::Move { .. }) {
                    self.config.link_curves.get(&(prev.cell, waypoint.cell))
                } else {
                    None
                };
                pending_angle_millidegrees += curve.map_or_else(
                    || prev.orientation.angle_millidegrees_to(waypoint.orientation) as u64,
                    |arc| {
                        (arc.sweep(self.config.cell_pos(prev.cell.0), self.config.cell_pos(waypoint.cell.0))
                            .abs()
                            .to_degrees()
                            * 1000.0)
                            .round() as u64
                    },
                );
            }
            prev_waypoint = Some(waypoint);
            match waypoint.action {
                // Markers carry no duration; PathStart declares the chain's origin.
                WaypointAction::PathStart { time } => {
                    cell = waypoint.cell;
                    cursor = time;
                }
                WaypointAction::PathEnd { .. } => {}
                WaypointAction::Move { start, end, .. } | WaypointAction::Rotate { start, end } => {
                    push_hold(&mut segments, cursor, start, cell);
                    let is_move = matches!(waypoint.action, WaypointAction::Move { .. });
                    let moving = if is_move { end - start } else { SimDuration::ZERO };
                    match segments.last_mut() {
                        Some(ChainSegment::Travel { end: run_end, moving: run_moving, angle_millidegrees, .. })
                            if *run_end == start =>
                        {
                            *run_end = end;
                            *run_moving += moving;
                            *angle_millidegrees += pending_angle_millidegrees;
                        }
                        _ => segments.push(ChainSegment::Travel {
                            start,
                            end,
                            moving,
                            angle_millidegrees: pending_angle_millidegrees,
                        }),
                    }
                    pending_angle_millidegrees = 0;
                    if is_move {
                        cell = waypoint.cell;
                    }
                    cursor = end;
                }
                WaypointAction::Wait { start, end } => {
                    push_hold(&mut segments, cursor.min(start), start, cell);
                    push_hold(&mut segments, start, end, waypoint.cell);
                    cursor = end;
                }
            }
        }

        HtmChain { segments, trailing_angle_millidegrees: pending_angle_millidegrees }
    }

    /// State the chain's runs and holds in the robot state channel, and the
    /// stand before it departs, from the instant the nav is stamped at: the
    /// robot's next move is committed from that instant. A run to a parking cell is travel to park,
    /// whatever it carries; with no TripStart to say where the trip goes, a run
    /// is told only by what it carries.
    fn note_htm_chain(&mut self, data: &NavEventData, ctx: Option<TripContext>, segments: &[ChainSegment]) {
        let travel = if ctx.is_some_and(|ctx| ctx.to == TripEndpoint::Parking) {
            RobotState::TravelToPark
        } else if data.carrying_cargo {
            RobotState::LadenTravel
        } else {
            RobotState::EmptyTravel
        };
        for segment in segments {
            let (span, state) = match *segment {
                ChainSegment::Travel { start, end, .. } => ((start, end), travel),
                ChainSegment::Hold { start, end, cell } => {
                    let cell = role_of(self.hold_position(cell));
                    ((start, end), RobotState::Waiting { cause: WaitCause::RouteHold, cell })
                }
            };
            self.robot_state_log.note(data.robot_id, span, state, Claim::Committed);
        }
        self.robot_state_log.await_departure(data.robot_id, self.current_world_time(), data.time);
    }

    /// Journal one HTM nav chain as alternating `Travel` and `Wait` rows.
    ///
    /// A travel RUN is a maximal contiguous stretch of `Move` and `Rotate`
    /// actions — rotation rides inside it because #261 §2.2 makes in-path
    /// chassis rotation part of travel. A HOLD is any stretch of the chain no
    /// run covers: an explicit `Wait` waypoint the pathfinder baked into the
    /// selected route, or a span between two committed actions. Each row's
    /// window is the real `[start, end)` of what it covers, so the rows are
    /// disjoint and together cover the chain exactly.
    ///
    /// Before #272 one row per nav event carried the SUMMED move time stamped
    /// at the first move — a fictional window as soon as anything interrupted
    /// the moves, which any hold row would have overlapped. That is the
    /// "move-only gap" #272 recorded, and splitting at holds is what closes
    /// it: the interrupted time now lands on a row instead of in the space
    /// between rows.
    ///
    /// `distance_mm` is apportioned across runs by each run's share of the
    /// chain's move time, taken as prefix differences so the parts sum to the
    /// event's total exactly. That is the same time-proportional split the
    /// D30 report already applies when clipping an interval row to a bucket,
    /// and a run is never finer than the row that split is defined on.
    fn push_htm_chain_rows(&mut self, data: &NavEventData, ctx: TripContext, chain: &HtmChain) {
        let segments = &chain.segments;
        let pending_angle_millidegrees = chain.trailing_angle_millidegrees;
        let from = self.resolve_endpoint(ctx.from);
        let to = self.resolve_endpoint(ctx.to);
        let from_zone_id = self.relay_zone(from);
        let to_zone_id = self.relay_zone(to);
        let presentation_id = ctx.presentation_id.map(|id| id.0);
        let total_moving = segments.iter().fold(SimDuration::ZERO, |sum, segment| match segment {
            ChainSegment::Travel { moving, .. } => sum + *moving,
            ChainSegment::Hold { .. } => sum,
        });
        let mut consumed_moving = SimDuration::ZERO;
        let mut distance_before_mm = 0u64;
        let mut last_travel_row: Option<usize> = None;
        let mut angle_so_far = 0u64;
        let mut turns_so_far = 0u32;
        let rounded_turns = |angle: u64| (angle as f64 / 90_000.0).round_ties_even() as u32;

        for segment in segments {
            match segment {
                ChainSegment::Travel { start, end, moving, angle_millidegrees } => {
                    angle_so_far += angle_millidegrees;
                    let turns_after = rounded_turns(angle_so_far);
                    let turn_count = turns_after - turns_so_far;
                    turns_so_far = turns_after;
                    // A rotation-only run moved nowhere, so it takes no
                    // distance; the whole chain moving nowhere puts the
                    // event's distance on its first run rather than losing it.
                    let distance_mm = if total_moving > SimDuration::ZERO {
                        consumed_moving += *moving;
                        let distance_to_mm = apportion(data.distance_mm, consumed_moving, total_moving);
                        let distance_mm = distance_to_mm - distance_before_mm;
                        distance_before_mm = distance_to_mm;
                        distance_mm
                    } else if last_travel_row.is_none() {
                        data.distance_mm
                    } else {
                        0
                    };
                    last_travel_row = Some(self.output.htm_leg_log.len());
                    self.output.htm_leg_log.push(HtmLegRecord {
                        start: *start,
                        activity: Activity::Travel,
                        from,
                        to,
                        from_zone_id,
                        to_zone_id,
                        distance_mm,
                        duration: *end - *start,
                        presentation_id,
                        robot_id: data.robot_id,
                        carrying_cargo: data.carrying_cargo,
                        turn_count,
                        wait_reason: None,
                        hold: None,
                        notes: String::new(),
                    });
                }
                ChainSegment::Hold { start, end, cell } => {
                    // Every hold inside a chain is a route hold, the leg's
                    // opening one included: the only thing the schedule holds a
                    // D30 for between two of its own committed actions is the
                    // selected path. An opening hold is the path search waiting
                    // out the corridor before the first move — the search
                    // writes it as an explicit `Wait` waypoint on the origin
                    // cell (`pathfinding::fh_sipp::reconstruct`) and commits it
                    // to the reservation table, so it is simulation time the
                    // robot spends standing there, not a planning artefact.
                    // Ruling #49 governs another shape, station time past both
                    // the current world time and the dwell's end with nothing
                    // to end it, which no visit takes: it ends at its
                    // departure or, when the run stops inside it, at the
                    // current world time or its dwell's end if that is later.
                    self.push_htm_hold_row(
                        data.robot_id,
                        (*start, *end - *start),
                        (from, to),
                        HtmWaitCause::RouteHold,
                        Some(*cell),
                        data.carrying_cargo,
                        presentation_id,
                    );
                }
            }
        }

        match last_travel_row {
            // Turns after the final run belong to it — the chain's geometric
            // total is a property of the chain, not of where it was cut.
            Some(index) => {
                self.output.htm_leg_log[index].turn_count +=
                    rounded_turns(angle_so_far + pending_angle_millidegrees) - turns_so_far;
            }
            // A chain that never moved still journals its turns, so a
            // rotation-only nav event keeps the zero-duration row it has
            // always produced.
            None if pending_angle_millidegrees > 0 || data.distance_mm > 0 => {
                self.output.htm_leg_log.push(HtmLegRecord {
                    start: data.time,
                    activity: Activity::Travel,
                    from,
                    to,
                    from_zone_id,
                    to_zone_id,
                    distance_mm: data.distance_mm,
                    duration: SimDuration::ZERO,
                    presentation_id,
                    robot_id: data.robot_id,
                    carrying_cargo: data.carrying_cargo,
                    turn_count: rounded_turns(pending_angle_millidegrees),
                    wait_reason: None,
                    hold: None,
                    notes: String::new(),
                });
            }
            None => {}
        }
    }

    /// Which kind of cell the map says `cell` is — the positional half of a
    /// hold's [`HtmHold`]. `Unknown` when the cell-kind table has no entry
    /// (tests and mapless runs run with an empty table), so an unplaced hold
    /// reads as unplaced rather than as floor.
    pub(super) fn hold_position(&self, cell: CellId) -> HtmWaitPosition {
        if self.relay_access_cells.contains(&cell) {
            return HtmWaitPosition::Relay;
        }
        match self.config.cell_kinds.get(cell.0 as usize).copied().flatten() {
            Some(CellKind::Charging) => HtmWaitPosition::Charging,
            Some(CellKind::Presentation) => HtmWaitPosition::Presentation,
            Some(CellKind::QueueCell) => HtmWaitPosition::StationQueue,
            Some(CellKind::Relay) => HtmWaitPosition::Relay,
            Some(
                CellKind::Storage
                | CellKind::Aisle
                | CellKind::Open
                | CellKind::Highway
                | CellKind::StationArea
                | CellKind::Junction
                | CellKind::Blank
                // TODO(wait-taxonomy): a hold at an Exchange cell is a belt
                // handoff wait -- its own D30 class, not Route. Filed with the
                // EasyLoop design; minting the class moves pinned traces, so
                // it waits for that ruling. A relay access cell is an Exchange
                // cell too, and reports as Relay above.
                | CellKind::Exchange,
            ) => HtmWaitPosition::Route,
            None => HtmWaitPosition::Unknown,
        }
    }

    /// Push one `Wait` journal row for a hold — an interval the solved
    /// schedule leaves between two committed actions. `span` is
    /// `(start, duration)`; `endpoints` is the `(from, to)` the row shows,
    /// equal for a dwell at one endpoint and the leg's own pair for a hold
    /// partway along it. `cell` is where the schedule left the robot, or
    /// `None` when no cell fact places it.
    ///
    /// The legacy `wait_reason` is filled in only for the two causes the
    /// pre-#272 vocabulary names correctly; every hold carries the full facts
    /// in `hold`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_htm_hold_row(
        &mut self,
        robot_id: RobotId,
        span: (SimTime, SimDuration),
        endpoints: (HtmJournalEndpoint, HtmJournalEndpoint),
        cause: HtmWaitCause,
        cell: Option<CellId>,
        carrying_cargo: bool,
        presentation_id: Option<u64>,
    ) {
        let (start, duration) = span;
        // A zero-length hold carries no wall-clock — skip it rather than emit
        // a noise row.
        if duration == SimDuration::ZERO {
            return;
        }
        let (from, to) = endpoints;
        let wait_reason = match cause {
            HtmWaitCause::NoTask => Some(HtmWaitReason::NoTask),
            // Both deposit dwells keep the one frozen legacy name; the
            // resident/drove split lives on the cause, per this enum's rule.
            HtmWaitCause::AwaitingDepositResident | HtmWaitCause::AwaitingDepositDrove => {
                Some(HtmWaitReason::AwaitingDeposit)
            }
            HtmWaitCause::AwaitingSlot
            | HtmWaitCause::AwaitingStationService
            | HtmWaitCause::AwaitingDeparture
            | HtmWaitCause::PostStationService
            | HtmWaitCause::RouteHold => None,
        };
        let position = cell.map_or(HtmWaitPosition::Unknown, |cell| self.hold_position(cell));
        self.output.htm_leg_log.push(HtmLegRecord {
            start,
            activity: Activity::Wait,
            from,
            to,
            from_zone_id: self.relay_zone(from),
            to_zone_id: self.relay_zone(to),
            distance_mm: 0,
            duration,
            presentation_id,
            robot_id,
            carrying_cargo,
            turn_count: 0,
            wait_reason,
            hold: Some(HtmHold { cause, position }),
            notes: String::new(),
        });
    }

    /// Resolve a semantic `TripEndpoint` into a journal endpoint, baking in the
    /// station kind (Pick/Putaway) from `config.station_types` so the xlsx
    /// export needs no config lookup. Mirrors `accumulate_cycle`'s legacy
    /// convention: a station with no recorded type is treated as Pick.
    fn resolve_endpoint(&self, endpoint: TripEndpoint) -> HtmJournalEndpoint {
        match endpoint {
            TripEndpoint::Parking => HtmJournalEndpoint::Parking,
            TripEndpoint::Relay(relay_id) => HtmJournalEndpoint::Relay(relay_id),
            TripEndpoint::Station(station_id) => match station_kind(&self.config.station_types, station_id) {
                TripType::Putaway => HtmJournalEndpoint::PutawayStation(station_id),
                TripType::Extraction => HtmJournalEndpoint::ExtractionStation(station_id),
                TripType::Pick => HtmJournalEndpoint::PickStation(station_id),
            },
        }
    }

    /// Zone of a journal endpoint, `Some` only when it is a `Relay` and the
    /// relay→zone table is populated. Stamps the `From/To Relay Zone` columns.
    fn relay_zone(&self, endpoint: HtmJournalEndpoint) -> Option<u32> {
        match endpoint {
            HtmJournalEndpoint::Relay(relay_id) => self.config.relay_zones.get(&relay_id).copied(),
            _ => None,
        }
    }

    /// Push a no-task `Wait` journal row for an HTM gap `[start, start +
    /// duration)` at `location`. Called once per gap between a robot's
    /// intervals, from the pass that builds them. Stationary, so `from == to`
    /// and distance/turns are zero. A no-task wait belongs to no cycle (Trip ID
    /// `None`).
    ///
    /// `location` is where the bot stood: the origin of the departure that
    /// ends the gap or, for the gap to the end of the run, the destination of
    /// its last leg (`last_trip_to`). Either resolves to `Parking` when the bot
    /// never completed a leg.
    pub(super) fn push_htm_no_task_wait_row(
        &mut self,
        robot_id: RobotId,
        start: SimTime,
        duration: SimDuration,
        location: HtmJournalEndpoint,
        cell: Option<CellId>,
        carrying_cargo: bool,
    ) {
        // The parked-vs-elsewhere split (#272) is positional and reads the
        // captured origin CELL, not `location`: `HtmJournalEndpoint::Parking`
        // doubles as the fallback for an unknown trip origin, so keying the
        // split off it would call every unplaced idle "parked".
        self.push_htm_hold_row(
            robot_id,
            (start, duration),
            (location, location),
            HtmWaitCause::NoTask,
            cell,
            carrying_cargo,
            None,
        );
    }

    /// Resolve a robot's idle location from a `TripEndpoint` source, defaulting
    /// to `Parking` when the bot has not completed a leg yet.
    pub(super) fn idle_location(&self, source: Option<TripEndpoint>) -> HtmJournalEndpoint {
        source.map(|ep| self.resolve_endpoint(ep)).unwrap_or(HtmJournalEndpoint::Parking)
    }

    /// Push an `AwaitingDeposit` `Wait` journal row for the relay dwell `[start,
    /// start + duration)` — the HTM sitting at the relay until the VTM deposits
    /// the tote it then forks up. This is the dominant HTM wait. The bot holds
    /// no tote yet (it is *awaiting* the deposit), so `carrying_cargo` is false.
    ///
    /// `resident` splits the two populations this dwell mixes. A bot that drove
    /// to meet the deposit and got there first stands at the end of its pickup
    /// leg, inside the leg's travel interval: the leg's Travel row is move-only
    /// `[depart, arrival]` and this dwell is `[arrival, pickup]`, so the two
    /// partition the leg. A bot the task found already on the relay cell stood
    /// there in the gap before the task, and the gap's no-task rows leave that
    /// stand to this row.
    pub(super) fn push_htm_awaiting_deposit_row(
        &mut self,
        robot_id: RobotId,
        relay_id: RelayId,
        start: SimTime,
        duration: SimDuration,
        resident: bool,
    ) {
        let cause = if resident { HtmWaitCause::AwaitingDepositResident } else { HtmWaitCause::AwaitingDepositDrove };
        self.push_htm_relay_dwell_row(robot_id, relay_id, start, duration, cause, false);
    }

    /// Push a `Wait` journal row for a dwell on a relay's HTM-access cell —
    /// the shared body of the two relay dwells, which differ only in what the
    /// schedule was waiting on and whether the tote was aboard.
    ///
    /// `AwaitingDeposit` is the inbound dwell before the fork-Get;
    /// `AwaitingSlot` is the return dwell from arrival to the deposit. Each
    /// comes after its leg's last travel row, so neither double-counts against
    /// travel.
    pub(super) fn push_htm_relay_dwell_row(
        &mut self,
        robot_id: RobotId,
        relay_id: RelayId,
        start: SimTime,
        duration: SimDuration,
        cause: HtmWaitCause,
        carrying_cargo: bool,
    ) {
        let presentation_id = self.current_trip.get(&robot_id).and_then(|ctx| ctx.presentation_id).map(|id| id.0);
        let relay = HtmJournalEndpoint::Relay(relay_id);
        // The relay's own access cell, so the position fact comes from the
        // event's relay id rather than from wherever tracking last left the
        // bot. An unmapped relay leaves the position `Unknown`.
        let cell = self.config.relay_htm_access_cells.get(&relay_id.0).copied().map(CellId);
        self.push_htm_hold_row(
            robot_id,
            (start, duration),
            (relay, relay),
            cause,
            cell,
            carrying_cargo,
            presentation_id,
        );
    }

    /// Rail-mounted VTM movement event. One event = one phase of a VTM
    /// cycle (`Alignment`, `Fork`, or `RailMove`); see [`VtmNavKind`].
    /// Each phase becomes a single `OccupancyInterval` and is fed into the
    /// same per-slice splitting machinery as HTM nav events.
    pub(super) fn handle_vtm_nav(&mut self, data: &VtmNavEventData) {
        self.begin_navigation(data.robot_id.0, data.kind.start().as_ms(), data.kind.end().as_ms());
        // Journey-leg journal row (Travel, Alignment, or Mechanical).
        // Additional output — the duty/grid accounting below is unchanged.
        self.push_vtm_nav_leg(data);

        let bot_key = data.robot_id.0;
        let sw = self.config.slice_width_ms;

        // Cumulative grid + travel-distance updates. Only RailMove contributes
        // distance; both kinds contribute to presence_ms at their cell(s).
        let start_ms = data.kind.start().as_ms();
        let end_ms = data.kind.end().as_ms();
        let duration_ms = end_ms.saturating_sub(start_ms);
        let slice_idx = (start_ms / sw) as usize;
        self.extend_slices_through(slice_idx);

        let iv = match &data.kind {
            VtmNavKind::Alignment { at, .. } | VtmNavKind::Fork { at, .. } => {
                let cell_key = at.cell().0;
                *self.output.cumulative_grid.nav_visits_vtm.entry(cell_key).or_insert(0) += 1;
                *self.output.cumulative_grid.robot_presence_vtm_ms.entry(cell_key).or_insert(0) += duration_ms;
                let slice_grid = self.grid_share(slice_idx);
                *slice_grid.nav_visits_vtm.entry(cell_key).or_insert(0) += 1;
                *slice_grid.robot_presence_vtm_ms.entry(cell_key).or_insert(0) += duration_ms;
                OccupancyInterval { start_ms, end_ms, cell: cell_key, pos: at.pos(), kind: BotStepKind::Wait }
            }
            VtmNavKind::RailMove { from, to, distance_mm, .. } => {
                let to_cell_key = to.cell().0;
                *self.output.cumulative_grid.nav_visits_vtm.entry(to_cell_key).or_insert(0) += 1;
                *self.output.cumulative_grid.robot_presence_vtm_ms.entry(to_cell_key).or_insert(0) += duration_ms;
                let slice_grid = self.grid_share(slice_idx);
                *slice_grid.nav_visits_vtm.entry(to_cell_key).or_insert(0) += 1;
                *slice_grid.robot_presence_vtm_ms.entry(to_cell_key).or_insert(0) += duration_ms;
                if *distance_mm > 0 {
                    self.slice_to_write(slice_idx).scalars.vtm_travel_distance_mm += *distance_mm;
                }
                // Ensure prior position matches `from` so the lerp endpoints
                // are correct. The first VtmForward of the cycle sees the
                // VTM at the source slot via InitialPositions / prior cycle's
                // final Fork(Put). For safety on first-ever movement, anchor
                // at `from`.
                if !self.bot_positions.contains_key(&bot_key) {
                    self.bot_positions.insert(bot_key, from.cell().0);
                    self.bot_world_pos.insert(bot_key, from.pos());
                }
                OccupancyInterval { start_ms, end_ms, cell: to_cell_key, pos: to.pos(), kind: BotStepKind::Move }
            }
        };

        if iv.end_ms > iv.start_ms {
            self.append_interval(bot_key, RobotType::Vtm, &iv);
        }

        // Update tracked position to the phase's end state.
        let (final_cell, final_pos) = match &data.kind {
            VtmNavKind::Alignment { at, .. } | VtmNavKind::Fork { at, .. } => (at.cell(), at.pos()),
            VtmNavKind::RailMove { to, .. } => (to.cell(), to.pos()),
        };
        self.bot_positions.insert(bot_key, final_cell.0);
        self.bot_world_pos.insert(bot_key, final_pos);

        // `VtmNav` is the whole source of DA50 duty: the rail cycle carries
        // both clocks of every leg. The interval below and the duty booking
        // are two readings of this one leg, so they name it the same way.
        let kind = match &data.kind {
            VtmNavKind::Alignment { .. } | VtmNavKind::Fork { .. } => ActivityKind::ForkOp,
            VtmNavKind::RailMove { .. } => ActivityKind::Travel,
        };
        self.mark_activity(
            data.robot_id,
            Milestone::Span {
                kind,
                start: data.kind.start(),
                end: data.kind.end(),
                carries_after: None,
                resident_since: None,
                known_from: self.current_world_time(),
            },
        );
        // `record_vtm_leg` routes each clock to its duty consumers.
        self.record_vtm_leg(
            data.robot_id,
            BusyInterval { start: data.kind.start(), end: data.kind.end(), kind },
            BusyInterval { start: data.geo_start, end: data.geo_end, kind },
        );
    }

    /// Navigation commits arrive in sequence for each robot. Reject overlaps;
    /// only empty, carried-forward idle snapshots may be refreshed. Recorded
    /// Move, Reverse, Rotate and explicit Wait steps are never rewritten.
    fn begin_navigation(&mut self, bot_key: u32, start_ms: u64, end_ms: u64) {
        assert!(end_ms >= start_ms, "robot {bot_key}: navigation ends before it starts");
        if let Some(&previous_end) = self.navigation_end_ms.get(&bot_key) {
            assert!(
                start_ms >= previous_end,
                "robot {bot_key}: committed navigation starts at {start_ms}ms before previous navigation ends at {previous_end}ms"
            );
        }
        let sw = self.config.slice_width_ms;
        let from_slice = (start_ms / sw) as usize;
        for (index, slice) in self.slices().iter().enumerate().skip(from_slice) {
            if let Some(location) = slice.bot_locations.get(&bot_key) {
                let recorded_end = slice.start_ms + location.steps.iter().map(|step| step.delta_ms).sum::<u64>();
                assert!(
                    location.steps.is_empty() || (index == from_slice && recorded_end <= start_ms),
                    "robot {bot_key}: navigation would overwrite committed playback in slice {index}"
                );
            }
        }
        self.navigation_end_ms.insert(bot_key, end_ms);
        for index in from_slice..self.slices().len() {
            if self.slices()[index].bot_locations.get(&bot_key).is_some_and(|loc| loc.steps.is_empty()) {
                self.slice_to_write(index).bot_locations.remove(&bot_key);
            }
        }
    }

    /// Emit one occupancy interval to the slices, splitting at slice
    /// boundaries. Each per-slice portion lands as a single `BotStep` whose
    /// `delta_ms` is exactly the portion's duration — never inflated to
    /// include preceding gaps. Any gap between the prior step's end (within
    /// the slice) and the portion's start is filled with a synthetic Wait
    /// `BotStep` at the prior position, preserving the lerp invariant
    /// (`Move` lerps over its own delta only; `Wait`/`Rotate` hold).
    fn append_interval(&mut self, bot_key: u32, robot_type: RobotType, iv: &OccupancyInterval) {
        let sw = self.config.slice_width_ms;
        let mut cursor = iv.start_ms;
        let travels = matches!(iv.kind, BotStepKind::Move | BotStepKind::Reverse);

        while cursor < iv.end_ms {
            let slice_idx = (cursor / sw) as usize;
            let slice_start_ms = slice_idx as u64 * sw;
            let slice_end_ms = slice_start_ms + sw;
            let portion_end_ms = iv.end_ms.min(slice_end_ms);
            let portion_delta = portion_end_ms - cursor;

            // For a Move that crosses a slice boundary, the BotStep recorded
            // for this slice's portion ends at the interpolated boundary
            // position, not at the cell center. The next slice picks up from
            // the interpolated position (carried forward via the slice's
            // start_pos / start_cell). For Wait/Rotate, both halves stay at
            // iv.pos.
            let portion_pos = if travels && portion_end_ms < iv.end_ms {
                let total_delta = (iv.end_ms - iv.start_ms) as f64;
                let elapsed = (portion_end_ms - iv.start_ms) as f64;
                let fraction = elapsed / total_delta;
                let prev_pos = self.bot_world_pos.get(&bot_key).copied().unwrap_or_default();
                WorldPos {
                    x_mm: prev_pos.x_mm + (iv.pos.x_mm - prev_pos.x_mm) * fraction,
                    y_mm: prev_pos.y_mm + (iv.pos.y_mm - prev_pos.y_mm) * fraction,
                }
            } else {
                iv.pos
            };

            // A portion after the first continues a leg the boundary before
            // it cut, so the bot opens that slice between two cells and the
            // recording says which one it is heading for.
            let toward = (travels && cursor > iv.start_ms).then_some(iv.cell);
            self.append_step_with_gap(
                bot_key,
                robot_type,
                slice_idx,
                cursor,
                portion_delta,
                iv.cell,
                portion_pos,
                iv.kind,
                toward,
            );

            // Advance only the bot's world position so the next portion's
            // boundary lerp uses the correct origin. DO NOT advance
            // bot_positions (the cell) until the full interval has been
            // consumed — for an in-progress Move that crosses a slice
            // boundary, the next slice's `start_cell` should be the move's
            // SOURCE (where the bot was last action-ready), not the
            // destination it's still en route to.
            self.bot_world_pos.insert(bot_key, portion_pos);

            cursor = portion_end_ms;
        }

        // Interval fully consumed — now commit the bot's tracked cell to
        // this interval's destination cell and pos.
        self.bot_positions.insert(bot_key, iv.cell);
        self.bot_world_pos.insert(bot_key, iv.pos);
    }

    /// Push one `BotStep` into a slice, prefacing it with a synthetic Wait
    /// step if there's a gap between the slice's prior coverage end and
    /// `step_start_ms`. Initializes the bot's `BotLocation` if absent.
    #[allow(clippy::too_many_arguments)]
    fn append_step_with_gap(
        &mut self,
        bot_key: u32,
        robot_type: RobotType,
        slice_idx: usize,
        step_start_ms: u64,
        delta_ms: u64,
        cell_id: u32,
        pos: WorldPos,
        kind: BotStepKind,
        toward: Option<u32>,
    ) {
        let sw = self.config.slice_width_ms;
        let slice_start_ms = slice_idx as u64 * sw;

        // Use the bot's current tracked cell/pos for the slice's start_cell
        // and start_pos and for the gap-filler position. These reflect where
        // the bot was at the end of the prior interval (or at sim start).
        let start_cell = self
            .bot_positions
            .get(&bot_key)
            .copied()
            .unwrap_or_else(|| panic!("nav event for bot {bot_key} with no known position"));
        let start_pos = self
            .bot_world_pos
            .get(&bot_key)
            .copied()
            .unwrap_or_else(|| panic!("nav event for bot {bot_key} with no known world position"));

        let z_mm = standing_height_mm(robot_type);
        // A leg the rasterizer cut can name the cell the bot is already in —
        // a curved or micro-stepped move that ends where it began. A bot is
        // not heading for the cell it is in, so that one reads as nothing.
        let toward = toward.filter(|cell| *cell != start_cell);
        let bot_loc = self.slice_to_write(slice_idx).bot_locations.entry(bot_key).or_insert_with(|| BotLocation {
            motion: Some(Vec::new()),
            start_cell,
            start_pos,
            start_z_mm: z_mm,
            start_toward: toward,
            start_heading_rad: None,
            robot_type,
            steps: SmallVec::new(),
        });

        // Compute the offset within the slice already covered by prior steps.
        let prior_coverage: u64 = bot_loc.steps.iter().map(|s| s.delta_ms).sum();
        let want_offset = step_start_ms.saturating_sub(slice_start_ms);
        if want_offset > prior_coverage {
            // Fill the gap with a Wait step at the prior position.
            let gap = want_offset - prior_coverage;
            let (gap_cell, gap_pos) =
                bot_loc.steps.last().map(|s| (s.cell_id, s.pos)).unwrap_or((bot_loc.start_cell, bot_loc.start_pos));
            bot_loc.steps.push(BotStep::whole(gap_cell, gap_pos, gap, BotStepKind::Wait));
        }

        bot_loc.steps.push(BotStep { cell_id, pos, z_mm, delta_ms, kind });
    }
}

/// Book `distance_mm` of travel into a slice's counters: a VTM's as rail
/// travel, an HTM's as floor travel and by the purpose
/// of the trip it was on.
fn book_travel(scalars: &mut ScalarCounters, is_vtm: bool, purpose: Option<TripPurpose>, distance_mm: u64) {
    if is_vtm {
        scalars.vtm_travel_distance_mm += distance_mm;
        return;
    }
    scalars.htm_travel_distance_mm += distance_mm;
    let trip = &mut scalars.trip_distance;
    match purpose {
        Some(TripPurpose::ToRelay) => trip.htm_to_relay_mm += distance_mm,
        Some(TripPurpose::ToStation) => trip.htm_to_station_mm += distance_mm,
        Some(TripPurpose::ToReturnRelay) => trip.htm_to_return_mm += distance_mm,
        Some(TripPurpose::ToParking) => trip.htm_to_parking_mm += distance_mm,
        None => {}
    }
}

/// `total_mm` scaled by `part / whole`, in `u128` so a long leg cannot
/// overflow the product. Callers take prefix differences of this so a split
/// into parts sums back to `total_mm` exactly.
fn apportion(total_mm: u64, part: SimDuration, whole: SimDuration) -> u64 {
    debug_assert!(whole > SimDuration::ZERO, "apportion over a zero window");
    let scaled = (total_mm as u128) * (part.as_ms() as u128) / (whole.as_ms() as u128);
    u64::try_from(scaled).unwrap_or(total_mm)
}

/// Resolve a station's pick/putaway kind from the config map. Matches
/// `accumulate_cycle`'s legacy convention: an unrecorded station type is Pick.
fn station_kind(station_types: &FxHashMap<StationId, StationType>, station_id: StationId) -> TripType {
    match station_types.get(&station_id) {
        Some(StationType::Putaway) => TripType::Putaway,
        Some(StationType::Extraction | StationType::ContainerPickup) => TripType::Extraction,
        // TODO(box-flow): box induction wants its own TripType once anything
        // routes to it; today no demand does.
        Some(StationType::Pick | StationType::BoxInduction) | None => TripType::Pick,
    }
}
