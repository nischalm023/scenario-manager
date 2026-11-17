//! Within `write`, `producer` is THE RUNNING TRUTH A KEYFRAME STATES: where
//! every robot and tote stands and what every robot is doing, which relay
//! cells are held, how deep each station's queue is, what every metric adds up
//! to and what every cell has been counted for, carried from slice to slice. It is the writing half of
//! the reader's rebuild and has to agree with it step for step.
//!
//! TODO(module): it mirrors `worldview::frames::state`, and wants to sit
//! beside it.

use desgraph::types::{CellId, StationId};
use desunits::{RobotId, SimTime, ToteId};
use rustc_hash::{FxHashMap, FxHashSet};
use worldview::frames::Keyframe;
use worldview::frames::heat::{HeatKeyframe, HeatLayer};
use worldview::frames::playback::{
    PlaybackDelta, PlaybackKeyframe, RobotRecord, StationState, ToteLocation, ToteRecord, ToteTransit,
};
use worldview::frames::robot_state::{RobotState, RobotStateDelta, RobotStateKeyframe};
use worldview::frames::scalars::{MetricDescriptor, ScalarsKeyframe};

use super::RecordingError;
use super::delta::{birth_place, whole_mm};
use crate::metrics::profile::{ClassedScalar, DynamicGrid, ScalarClass};
use crate::metrics::worldview::wire::{
    designation_byte, metric_descriptor, milliradians, tote_location, tote_type_byte,
};
use crate::model::ToteType;
use crate::observer::time_slice::TimeSliceData;

/// Where everything stands as of the next slice to be written — what a
/// keyframe at that instant states.
pub(super) struct Producer {
    robots: FxHashMap<RobotId, RobotRecord>,
    totes: FxHashMap<ToteId, ToteRecord>,
    reserved_relay_cells: FxHashSet<CellId>,
    stations: FxHashMap<StationId, u32>,
    /// Conveyor journeys a slice has stated and that are not over. A keyframe
    /// states the ones still under way, because where a container last stood
    /// does not say that it is between landmarks.
    transits: Vec<ToteTransit>,
    /// Which slot of a scalars array each metric sits in, and the metric in
    /// each slot as the run declared it, in the order it declared them.
    slot_at: FxHashMap<String, usize>,
    declared: Vec<MetricDescriptor>,
    /// What a keyframe states for each metric: the run's sum so far where the
    /// readings add up, the newest reading where they do not, and nil for a
    /// metric no slice has stated yet.
    standing: Vec<Option<f64>>,
    /// The heading the reader is holding for each robot, so a delta states
    /// only the ones that changed.
    headings: FxHashMap<RobotId, Option<i32>>,
    /// Every cell counter already stated, which a keyframe restates whole.
    heat: FxHashMap<String, FxHashMap<CellId, u64>>,
    /// What each robot is doing where the next slice to be written stands,
    /// once its delta has said what begins there.
    robot_states: FxHashMap<RobotId, RobotState>,
}

impl Producer {
    pub(super) fn new() -> Self {
        Self {
            robots: FxHashMap::default(),
            totes: FxHashMap::default(),
            reserved_relay_cells: FxHashSet::default(),
            stations: FxHashMap::default(),
            transits: Vec::new(),
            slot_at: FxHashMap::default(),
            declared: Vec::new(),
            standing: Vec::new(),
            headings: FxHashMap::default(),
            heat: FxHashMap::default(),
            robot_states: FxHashMap::default(),
        }
    }

    /// The metrics `metrics` states that have no slot yet, each with its
    /// class, sorted by name: the flattening writes a family's keys in no
    /// order of their own, and two runs of the same fixture must lay their
    /// slots out the same way.
    pub(super) fn unslotted(&self, metrics: &[ClassedScalar]) -> Vec<(String, ScalarClass)> {
        let mut unslotted: Vec<(String, ScalarClass)> = metrics
            .iter()
            .filter(|metric| !self.slot_at.contains_key(&metric.key))
            .map(|metric| (metric.key.clone(), metric.class))
            .collect();
        unslotted.sort_unstable_by(|(one, _), (other, _)| one.cmp(other));
        unslotted
    }

    /// Give `name` the next slot, described by `class`, and return the
    /// metric as the run declares it. A slot never moves once stated.
    pub(super) fn slot(&mut self, name: String, class: ScalarClass) -> &MetricDescriptor {
        let slot = self.declared.len();
        self.slot_at.insert(name.clone(), slot);
        self.declared.push(metric_descriptor(&name, class));
        self.standing.push(None);
        &self.declared[slot]
    }

    /// Every slot's metric as the run declared it, in slot order.
    pub(super) fn declared(&self) -> &[MetricDescriptor] {
        &self.declared
    }

    /// Every robot whose heading at this slice's start is not the one the
    /// reader holds — a robot that turned where it stands as much as one that
    /// walked. Taken before the slice is seeded, because seeding is what
    /// moves the producer to those headings.
    pub(super) fn take_headings(&mut self, slice: &TimeSliceData) -> Vec<(RobotId, Option<i32>)> {
        let mut stated = Vec::new();
        for (&bot, location) in &slice.bot_locations {
            let robot = RobotId(bot);
            let heading = location.start_heading_rad.map(milliradians);
            if self.headings.get(&robot).copied().flatten() != heading {
                stated.push((robot, heading));
                self.headings.insert(robot, heading);
            }
        }
        stated.sort_by_key(|(robot, _)| robot.0);
        stated
    }

    /// Take from the slice what a keyframe at its start states and a delta
    /// does not: every robot's class, where it stands and which way it faces;
    /// the totes a checkpoint slice restates; the relay cells held; and the
    /// records stamped at the slice's own first instant, which happen at
    /// exactly the instant a keyframe speaks about.
    pub(super) fn seed_from_slice(&mut self, slice: &TimeSliceData) -> Result<(), RecordingError> {
        for (&bot, location) in &slice.bot_locations {
            let robot = RobotId(bot);
            let at = whole_mm(location.start_pos, location.start_z_mm, bot, slice.start_ms)?;
            let cell = CellId(location.start_cell);
            let toward = location.start_toward.map(CellId);
            let record = self.robots.entry(robot).or_insert(RobotRecord {
                robot,
                class: location.robot_type.into(),
                at,
                heading_mrad: None,
                carrying: None,
                cell,
                toward,
            });
            record.class = location.robot_type.into();
            record.at = at;
            record.cell = cell;
            record.toward = toward;
            record.heading_mrad = location.start_heading_rad.map(milliradians);
        }
        let order_totes: FxHashSet<u32> = slice.tote_state.order_totes.iter().copied().collect();
        for (tote, state) in &slice.tote_state.start_states {
            let designation = u8::from(order_totes.contains(tote));
            // A checkpoint restates where every container stands. What kind of
            // container it is and what is in it were stated at its birth and
            // by its contents records since, so only the place is taken here.
            self.totes.entry(ToteId(*tote)).and_modify(|record| record.location = tote_location(state)).or_insert(
                ToteRecord {
                    tote: ToteId(*tote),
                    tote_type: tote_type_byte(ToteType::default()),
                    designation,
                    location: tote_location(state),
                    contents_units: 0,
                },
            );
        }
        for birth in slice.tote_state.creations.iter().filter(|birth| birth.offset_ms == 0) {
            let Ok(location) = birth_place(slice, birth.offset_ms, birth.tote_id) else { continue };
            let tote = ToteId(birth.tote_id);
            let record = self.totes.entry(tote).or_insert(ToteRecord {
                tote,
                tote_type: tote_type_byte(birth.tote_type),
                designation: designation_byte(birth.designation),
                location,
                contents_units: 0,
            });
            record.tote_type = tote_type_byte(birth.tote_type);
            record.designation = designation_byte(birth.designation);
            record.location = location;
        }
        for (_, tote, units) in slice.tote_state.contents.iter().filter(|(offset, _, _)| *offset == 0) {
            if let Some(record) = self.totes.get_mut(&ToteId(*tote)) {
                record.contents_units = *units;
            }
        }
        Ok(())
    }

    /// The relay cells held before the run's first slice. Only the first
    /// slice states them: after that the set is what its own transitions
    /// have made it, so a keyframe and the deltas that reach it agree. The
    /// observer re-snapshots its running set whenever it creates a slice,
    /// which for a slice created out of order is a set from a later instant;
    /// taking those would make a keyframe disagree with its own transitions.
    pub(super) fn seed_reservations(&mut self, slice: &TimeSliceData) {
        self.reserved_relay_cells = slice.relay_state.start_reserved.iter().map(|&cell| CellId(cell)).collect();
    }

    /// Take what a slice states about its own start: the state each robot it
    /// names begins it in, where that state begins with the slice. A keyframe
    /// written at that start states those. A robot the playback has never
    /// placed has no keyframe to be stated in, so a slice naming one is
    /// refused.
    pub(super) fn enter_robot_states(&mut self, at: SimTime, delta: &RobotStateDelta) -> Result<(), RecordingError> {
        for entry in &delta.0 {
            if !self.robots.contains_key(&entry.robot) {
                let detail = "is named by the robot state channel and placed by no keyframe";
                return Err(RecordingError::RobotState { robot: entry.robot, at, detail });
            }
            if let Some(first) = entry.spans.first().filter(|_| !entry.continues) {
                self.robot_states.insert(entry.robot, first.state);
            }
        }
        Ok(())
    }

    /// Take the state a whole slice ends each robot it names in.
    pub(super) fn leave_robot_states(&mut self, delta: &RobotStateDelta) {
        for entry in &delta.0 {
            if let Some(last) = entry.spans.last() {
                self.robot_states.insert(entry.robot, last.state);
            }
        }
    }

    /// Every placed robot's state at `at`, which a keyframe states for each
    /// robot it places.
    fn robot_state_keyframe(&self, at: SimTime) -> Result<RobotStateKeyframe, RecordingError> {
        let mut states = Vec::with_capacity(self.robots.len());
        for &robot in self.robots.keys() {
            let detail = "is placed with no state the robot state channel stated";
            let state = self.robot_states.get(&robot).ok_or(RecordingError::RobotState { robot, at, detail })?;
            states.push((robot, *state));
        }
        Ok(RobotStateKeyframe(states))
    }

    pub(super) fn keyframe(&self, at: SimTime) -> Result<Keyframe, RecordingError> {
        let playback = PlaybackKeyframe {
            robots: self.robots.values().copied().collect(),
            // A container that has left the run is gone, and nothing draws it.
            totes: self.totes.values().filter(|tote| tote.location != ToteLocation::Extracted).copied().collect(),
            reserved_relay_cells: self.reserved_relay_cells.iter().copied().collect(),
            stations: self
                .stations
                .iter()
                .map(|(&station, &pipeline_depth)| StationState { station, pipeline_depth })
                .collect(),
            transits_in_flight: self.transits.iter().filter(|transit| transit.end > at).copied().collect(),
        };
        let heat = HeatKeyframe {
            layers: self
                .heat
                .iter()
                .map(|(name, counts)| HeatLayer {
                    name: name.clone(),
                    counts: counts.iter().map(|(&cell, &count)| (cell, count)).collect(),
                })
                .collect(),
        };
        Ok(Keyframe {
            playback: Some(playback),
            scalars: Some(ScalarsKeyframe { values: self.standing.clone() }),
            heat: Some(heat),
            robot_state: Some(self.robot_state_keyframe(at)?),
        })
    }

    /// One slice's own values, in descriptor order, nil where the slice said
    /// nothing about a metric.
    pub(super) fn slice_values(&self, metrics: &[ClassedScalar]) -> Vec<Option<f64>> {
        let mut values = vec![None; self.declared.len()];
        for metric in metrics {
            if let Some(&slot) = self.slot_at.get(&metric.key) {
                values[slot] = Some(metric.value);
            }
        }
        values
    }

    /// Carry the slice's own changes into the running truth, so the next
    /// keyframe states where everything stands once this slice has run. This
    /// is the writer's side of the reader's rebuild, and the two have to
    /// agree step for step.
    pub(super) fn apply_slice(
        &mut self,
        slice: &TimeSliceData,
        metrics: &[ClassedScalar],
        grid: &DynamicGrid,
        delta: &PlaybackDelta,
    ) -> Result<(), RecordingError> {
        for (&bot, location) in &slice.bot_locations {
            if let Some(record) = self.robots.get_mut(&RobotId(bot))
                && let Some(last) = location.steps.last()
            {
                record.at = whole_mm(last.pos, last.z_mm, bot, slice.start_ms)?;
            }
        }
        // The cells come from the delta, not from the slice, so the producer's
        // running truth moves exactly as a reader's rebuild moves it.
        for mover in &delta.movers {
            if let Some(record) = self.robots.get_mut(&mover.robot) {
                record.cell = mover.cell;
                record.toward = mover.heading_for();
            }
        }
        for birth in &slice.tote_state.creations {
            let Ok(location) = birth_place(slice, birth.offset_ms, birth.tote_id) else { continue };
            let tote = ToteId(birth.tote_id);
            self.totes.insert(
                tote,
                ToteRecord {
                    tote,
                    tote_type: tote_type_byte(birth.tote_type),
                    designation: designation_byte(birth.designation),
                    location,
                    contents_units: 0,
                },
            );
        }
        for (_, tote, state) in &slice.tote_state.transitions {
            if let Some(record) = self.totes.get_mut(&ToteId(*tote)) {
                record.location = tote_location(state);
            }
        }
        for (_, tote, units) in &slice.tote_state.contents {
            if let Some(record) = self.totes.get_mut(&ToteId(*tote)) {
                record.contents_units = *units;
            }
        }
        for (cell, _, reserved) in &slice.relay_state.transitions {
            match *reserved {
                true => self.reserved_relay_cells.insert(CellId(*cell)),
                false => self.reserved_relay_cells.remove(&CellId(*cell)),
            };
        }
        // In the delta's own order, which is by offset: the writer's running
        // truth has to move the way a reader moves it, or a keyframe would
        // state a depth the deltas before it never reached.
        for change in &delta.station_changes {
            self.stations.insert(change.station, change.pipeline_depth);
        }
        // A journey that has landed is a place, not a journey; the ones left
        // are what the next keyframe states as under way.
        let opened = SimTime::ms(slice.start_ms);
        self.transits.retain(|transit| transit.end > opened);
        self.transits.extend(slice.tote_state.transits.iter().map(|transit| ToteTransit {
            tote: ToteId(transit.tote_id),
            from_cell: CellId(transit.from_cell),
            to_cell: CellId(transit.to_cell),
            start: SimTime::ms(transit.start_ms),
            end: SimTime::ms(transit.end_ms),
        }));
        for record in self.robots.values_mut() {
            record.carrying = None;
        }
        for record in self.totes.values() {
            if let ToteLocation::OnBot(robot) = record.location
                && let Some(bot) = self.robots.get_mut(&robot)
            {
                bot.carrying = Some(record.tote);
            }
        }
        for (slot, stated) in self.slice_values(metrics).into_iter().enumerate() {
            let Some(value) = stated else { continue };
            let running = self.standing[slot].filter(|_| self.declared[slot].semantics.aggregation.accumulates());
            self.standing[slot] = Some(running.unwrap_or(0.0) + value);
        }
        for (name, counts) in grid {
            let stated = self.heat.entry(name.clone()).or_default();
            for (&cell, &count) in counts {
                *stated.entry(CellId(cell)).or_insert(0) += count;
            }
        }
        Ok(())
    }
}
