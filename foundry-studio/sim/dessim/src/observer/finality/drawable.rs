//! Within `finality`, `drawable` makes a slice ready for playback to draw,
//! once, as the slice is made final. Every robot gets an entry in it, so
//! playback places the robots at any instant of the slice without reading the
//! slices before it: a robot standing still through the slice gets an entry
//! carried from the slice before. The slice's tote changes go into time order;
//! where a tote stands, playback still reads from the slices before, back to
//! the last checkpoint (`ToteStateRecord::positions_at`).

use std::f32::consts::{PI, TAU};

use desgraph::{RobotType, WorldPos};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::observer::state::SimObserverState;
use crate::observer::time_slice::{BotLocation, BotStepKind, TimeSliceData, ToteStateRecord, standing_height_mm};

/// Each robot from `InitialPositions`: its cell, position, type and heading.
type CarriedBots = FxHashMap<u32, (u32, WorldPos, RobotType, Option<f32>)>;

impl SimObserverState {
    /// Make slice `index` drawable: every robot gets an entry in it. The slice
    /// before it, when there is one, is drawable already.
    ///
    /// Recorded positions and steps remain authoritative. Each robot's heading
    /// is carried from the slice before, so a slice reached by a seek still
    /// faces each robot the way it last moved.
    pub(in crate::observer) fn make_drawable(&mut self, index: usize) {
        let mut carried: CarriedBots =
            self.initial_bot_state.iter().map(|&(id, cell, pos, rt, heading)| (id, (cell, pos, rt, heading))).collect();
        if let Some(previous) = index.checked_sub(1).and_then(|previous| self.slices().get(previous)) {
            carry_forward(&mut carried, previous);
        }
        let slice = self.slice_to_finalize(index);
        backfill_standing_robots(slice, &carried);
        sort_tote_changes(&mut slice.tote_state);
    }
}

/// Give every carried robot without an entry in `slice` a standing one, and
/// carry each robot's heading into its entry.
fn backfill_standing_robots(slice: &mut TimeSliceData, carried: &CarriedBots) {
    for (&bot_id, &(cell, pos, rt, heading)) in carried {
        let location = slice.bot_locations.entry(bot_id).or_insert_with(|| BotLocation {
            motion: Some(Vec::new()),
            start_cell: cell,
            start_pos: pos,
            start_z_mm: standing_height_mm(rt),
            start_toward: None,
            start_heading_rad: heading,
            robot_type: rt,
            steps: SmallVec::new(),
        });
        if heading.is_some() {
            location.start_heading_rad = heading;
        }
    }
}

/// Advance the carried positions and headings past one slice.
fn carry_forward(carried: &mut CarriedBots, slice: &TimeSliceData) {
    for (&bot_id, loc) in &slice.bot_locations {
        if let Some(entry) = carried.get_mut(&bot_id) {
            entry.3 = last_motion_heading(loc).or(loc.start_heading_rad).or(entry.3);
            if let Some(last_step) = loc.steps.last() {
                entry.0 = last_step.cell_id;
                entry.1 = last_step.pos;
            } else {
                entry.0 = loc.start_cell;
                entry.1 = loc.start_pos;
            }
        }
    }
}

/// Births and moves recorded within one slice, in time order: handlers
/// record them in the order the observer reads the facts.
fn sort_tote_changes(tote_state: &mut ToteStateRecord) {
    tote_state.creations.sort_by_key(|birth| (birth.offset_ms, birth.tote_id));
    tote_state.transitions.sort_by_key(|(offset, _, _)| *offset);
}

fn last_motion_heading(location: &BotLocation) -> Option<f32> {
    location.steps.iter().enumerate().rev().find_map(|(index, step)| {
        if !matches!(step.kind, BotStepKind::Move | BotStepKind::Reverse) {
            return None;
        }
        let from = if index == 0 { location.start_pos } else { location.steps[index - 1].pos };
        let (dx, dy) = (step.pos.x_mm - from.x_mm, step.pos.y_mm - from.y_mm);
        if dx.hypot(dy) < 1.0 {
            return None;
        }
        let travel = dy.atan2(dx) as f32;
        Some(if step.kind == BotStepKind::Reverse { (travel + PI).rem_euclid(TAU) } else { travel })
    })
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;
    use crate::observer::time_slice::{BotStep, MetricsConfig};

    fn pos(x_mm: f64, y_mm: f64) -> WorldPos {
        WorldPos { x_mm, y_mm }
    }

    fn step(kind: BotStepKind, position: WorldPos) -> BotStep {
        BotStep::whole(0, position, 1000, kind)
    }

    /// One HTM over three slices, standing at the origin facing east.
    fn state() -> SimObserverState {
        let mut state = SimObserverState::new(MetricsConfig::default());
        state.initial_bot_state.push((1, 0, pos(0.0, 0.0), RobotType::Htm, Some(0.0)));
        state.extend_slices_through(2);
        state
    }

    fn moving(start: WorldPos, steps: impl IntoIterator<Item = BotStep>) -> BotLocation {
        BotLocation {
            motion: Some(Vec::new()),
            start_cell: 0,
            start_pos: start,
            start_z_mm: 0.0,
            start_toward: None,
            start_heading_rad: None,
            robot_type: RobotType::Htm,
            steps: steps.into_iter().collect(),
        }
    }

    #[test]
    fn movement_heading_reaches_existing_and_stationary_later_slices() {
        let mut state = state();
        state.slice_to_write(0).bot_locations.insert(
            1,
            moving(
                pos(0.0, 0.0),
                [
                    step(BotStepKind::Move, pos(1000.0, 0.0)),
                    step(BotStepKind::Rotate, pos(1000.0, 0.0)),
                    step(BotStepKind::Move, pos(1000.0, 1000.0)),
                ],
            ),
        );
        state.slice_to_write(1).bot_locations.insert(
            1,
            moving(
                pos(1000.0, 1000.0),
                [
                    step(BotStepKind::Rotate, pos(1000.0, 1000.0)),
                    step(BotStepKind::Reverse, pos(1000.0, 2000.0)),
                    step(BotStepKind::Wait, pos(1000.0, 2000.0)),
                ],
            ),
        );
        for index in 0..3 {
            state.make_drawable(index);
        }
        assert_eq!(state.slices()[0].bot_locations[&1].start_heading_rad, Some(0.0));
        assert_eq!(state.slices()[1].bot_locations[&1].start_heading_rad, Some(FRAC_PI_2));
        let stationary = &state.slices()[2].bot_locations[&1];
        assert!(stationary.steps.is_empty());
        assert_eq!(stationary.start_pos, pos(1000.0, 2000.0));
        assert_eq!(stationary.start_heading_rad, Some(3.0 * FRAC_PI_2));
    }
}
