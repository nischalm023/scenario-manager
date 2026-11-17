//! Within `activity`, `distribution` counts each fleet's robots at a slice's
//! start by the kind of interval covering that instant, a gap included. Each
//! slice is derived once: when B passes the instant it samples, or, for a
//! slice B never passed, from the final intervals when the stream closes.

use rustc_hash::FxHashMap;

use desgraph::RobotType;

use crate::model::SimTime;
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::passed_slice_start;

use super::ActivityKind;
use super::log::RobotActivity;

impl RobotActivity {
    /// The kind of the work covering `sample`, read as B stands: its end may
    /// be unknown, but a bot mid-leg is not idle.
    fn kind_at(&self, sample: SimTime) -> ActivityKind {
        let covering = self.work_from(sample).find(|work| work.end > sample).filter(|work| work.start <= sample);
        covering.map_or(ActivityKind::Gap, |work| work.kind)
    }
}

impl SimObserverState {
    /// Derive the distribution of each slice whose start, the instant it
    /// samples, B has passed: every milestone at or before it is behind B.
    pub(super) fn derive_settled_distribution(&mut self, floor: SimTime) {
        let width = self.config.slice_width();
        let mut index = self.activity.distribution_upto;
        while index < self.slices().len() && passed_slice_start(floor, index, width) {
            self.derive_slice_distribution(index);
            index += 1;
        }
        self.activity.distribution_upto = index;
    }

    /// The distribution of every slice B never passed, from the final
    /// intervals.
    pub(super) fn derive_final_distribution(&mut self) {
        for index in self.activity.distribution_upto..self.slices().len() {
            self.derive_slice_distribution(index);
        }
        self.activity.distribution_upto = self.slices().len();
    }

    fn derive_slice_distribution(&mut self, index: usize) {
        let (htm, vtm) = self.distribution_at(SimTime::ms(index as u64 * self.config.slice_width_ms));
        let scalars = &mut self.slice_to_finalize(index).scalars;
        scalars.htm_activity_counts = htm;
        scalars.vtm_activity_counts = vtm;
    }

    /// How many robots of each fleet were inside an interval of each kind at
    /// `sample`, and how many in a gap. A robot of any other fleet counts in
    /// neither.
    fn distribution_at(&self, sample: SimTime) -> (FxHashMap<ActivityKind, u32>, FxHashMap<ActivityKind, u32>) {
        let mut htm: FxHashMap<ActivityKind, u32> = FxHashMap::default();
        let mut vtm: FxHashMap<ActivityKind, u32> = FxHashMap::default();
        for (robot, robot_type) in &self.output.robot_types {
            let fleet = match robot_type {
                RobotType::Htm => &mut htm,
                RobotType::Vtm => &mut vtm,
                RobotType::Rtp => continue,
            };
            let kind = self.activity.robots.get(robot).map_or(ActivityKind::Gap, |r| r.kind_at(sample));
            *fleet.entry(kind).or_insert(0) += 1;
        }
        (htm, vtm)
    }
}
