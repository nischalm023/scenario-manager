//! Within `live_run`, `utilization` is THE UTILIZATION PANEL OF A RUN STILL
//! GOING: each robot's work, each station's cycles and the DA50s' idle
//! reasons, read off the slices the window has received. Once the run has
//! ended the panel reads the end block's summary instead, as playback does,
//! so what is read here only has to say what the slices state: a DA50 lane
//! shows the work its activity records state, and the idle reasons count the
//! cycles that began after an idle gap, the only ones a slice records.

// std's `HashMap`, not `FxHashMap`: these are the Gantt's inputs, which the
// summary hands it as std maps, and the Gantt takes one map type from either.
use std::collections::HashMap;

use desgraph::{RobotType, StationId, StationType};
use dessim::events::VtmIdleReasonKind;
use dessim::metrics::profile::{RobotIdentity, SliceRecord};
use dessim::model::{RobotId, SimDuration, SimTime};
use dessim::observer::activity::ActivityKind;
use dessim::observer::duty::{BusyInterval, DutyBreakdown, StationBusyInterval};

/// What the slices received say about each fleet's and each station's work.
#[derive(Default)]
pub(crate) struct UtilizationSeen {
    /// Each robot's class, as the roster the recording states gives it.
    classes: HashMap<RobotId, RobotType>,
    pub(crate) vtm_timelines: HashMap<RobotId, Vec<BusyInterval>>,
    pub(crate) htm_timelines: HashMap<RobotId, Vec<BusyInterval>>,
    pub(crate) vtm_duty: HashMap<RobotId, DutyBreakdown>,
    pub(crate) htm_duty: HashMap<RobotId, DutyBreakdown>,
    pub(crate) vtm_idle_reason_counts: HashMap<VtmIdleReasonKind, u64>,
    pub(crate) station_timelines: HashMap<StationId, Vec<StationBusyInterval>>,
    /// The run's stations by type, as its map states them.
    pub(crate) station_types: HashMap<StationId, StationType>,
    /// Each station's busy time, summed over its cycles.
    station_busy: HashMap<StationId, SimDuration>,
    /// The simtime the slices received reach.
    pub(crate) seen_until: SimTime,
}

impl UtilizationSeen {
    pub(crate) fn new(station_types: HashMap<StationId, StationType>) -> Self {
        Self { station_types, ..Self::default() }
    }

    /// Give every robot the roster states a lane, a robot that never works
    /// included, as the summary gives the whole fleet one.
    pub(crate) fn seat(&mut self, roster: Vec<RobotIdentity>) {
        for robot in roster {
            match robot.class {
                RobotType::Vtm => {
                    self.vtm_timelines.entry(robot.robot).or_default();
                    self.vtm_duty.entry(robot.robot).or_default();
                }
                RobotType::Htm => {
                    self.htm_timelines.entry(robot.robot).or_default();
                    self.htm_duty.entry(robot.robot).or_default();
                }
                RobotType::Rtp => {}
            }
            self.classes.insert(robot.robot, robot.class);
        }
    }

    /// Add one slice, the next after those added, which ends at `ends`.
    pub(crate) fn add_slice(&mut self, slice: &SliceRecord, ends: SimTime) {
        let diag = &slice.station_diag;
        for record in &diag.activity {
            // The summary leaves a gap, and work that takes no time, off the
            // lanes; an RTP's work has a lane in neither fleet.
            if record.kind == ActivityKind::Gap || record.end <= record.start {
                continue;
            }
            let robot = RobotId(record.robot_id);
            let (timelines, duty) = match self.classes.get(&robot) {
                Some(RobotType::Vtm) => (&mut self.vtm_timelines, &mut self.vtm_duty),
                Some(RobotType::Htm) => (&mut self.htm_timelines, &mut self.htm_duty),
                _ => continue,
            };
            let interval = BusyInterval { start: record.start, end: record.end, kind: record.kind };
            timelines.entry(robot).or_default().push(interval);
            duty.entry(robot).or_default().add(interval.kind, interval.duration());
        }
        for idle in &diag.vtm_idle_reasons {
            *self.vtm_idle_reason_counts.entry(idle.reason).or_default() += 1;
        }
        for cycle in &diag.station_cycles {
            let end = cycle.start + cycle.dwell;
            self.station_timelines
                .entry(cycle.station_id)
                .or_default()
                .push(StationBusyInterval { start: cycle.start, end });
            *self.station_busy.entry(cycle.station_id).or_default() += cycle.dwell;
        }
        self.seen_until = self.seen_until.max(ends);
    }

    /// Each station's busy share of the simtime the slices received reach.
    pub(crate) fn station_busy_share(&self) -> HashMap<StationId, f64> {
        let seen_ms = self.seen_until.as_ms().max(1) as f64;
        self.station_busy.iter().map(|(&station, busy)| (station, (busy.as_ms() as f64 / seen_ms).min(1.0))).collect()
    }
}

#[cfg(test)]
mod tests {
    use dessim::observer::station_diag::{
        RobotActivityRecord, StationCycleRecord, StationDiagRecord, VtmIdleReasonRecord,
    };

    use super::*;

    fn robot(id: u32, class: RobotType) -> RobotIdentity {
        RobotIdentity { robot: RobotId(id), name: id.to_string(), class, class_entity: None }
    }

    fn work(robot_id: u32, kind: ActivityKind, start_ms: u64, end_ms: u64) -> RobotActivityRecord {
        RobotActivityRecord { robot_id, kind, start: SimTime::ms(start_ms), end: SimTime::ms(end_ms) }
    }

    fn slice(diag: StationDiagRecord) -> SliceRecord {
        SliceRecord {
            start_ms: 0,
            metrics: Default::default(),
            grid: Default::default(),
            bot_locations: Default::default(),
            relay_state: Default::default(),
            tote_state: Default::default(),
            station_diag: diag,
        }
    }

    /// Every robot the roster states gets a lane, one that never works
    /// included; each interval of work lands on its robot's lane and in its
    /// duty, in its robot's fleet, while a gap and an RTP's work land on none;
    /// each idle reason is counted; and a station's busy share is its cycles
    /// over the simtime the slices reach.
    #[test]
    fn a_slices_work_lands_on_its_robots_lane_in_its_fleet() {
        let mut seen = UtilizationSeen::default();
        seen.seat(vec![
            robot(1, RobotType::Vtm),
            robot(2, RobotType::Htm),
            robot(3, RobotType::Htm),
            robot(4, RobotType::Rtp),
        ]);
        let diag = StationDiagRecord {
            activity: vec![
                work(1, ActivityKind::Travel, 0, 4_000),
                work(2, ActivityKind::ForkOp, 1_000, 3_000),
                work(2, ActivityKind::Gap, 3_000, 5_000),
                work(4, ActivityKind::Travel, 0, 1_000),
            ],
            vtm_idle_reasons: vec![VtmIdleReasonRecord {
                robot_id: 1,
                reason: VtmIdleReasonKind::AwaitingTote,
                gap_ms: 500,
                relay_cell: 9,
                at: SimTime::ms(4_000),
            }],
            station_cycles: vec![StationCycleRecord {
                station_id: StationId(5),
                start: SimTime::ms(2_000),
                dwell: SimDuration::ms(2_500),
            }],
            ..Default::default()
        };
        seen.add_slice(&slice(diag), SimTime::ms(10_000));

        assert_eq!(seen.vtm_timelines.keys().copied().collect::<Vec<_>>(), [RobotId(1)]);
        assert_eq!(seen.vtm_timelines[&RobotId(1)].len(), 1);
        assert_eq!(seen.vtm_duty[&RobotId(1)].traveling, SimDuration::ms(4_000));
        let mut htm: Vec<RobotId> = seen.htm_timelines.keys().copied().collect();
        htm.sort();
        assert_eq!(htm, [RobotId(2), RobotId(3)], "the idle D30 has a lane too");
        assert_eq!(seen.htm_timelines[&RobotId(2)].len(), 1, "the gap is on no lane");
        assert_eq!(seen.htm_duty[&RobotId(2)].loading_unloading, SimDuration::ms(2_000));
        assert!(seen.htm_timelines[&RobotId(3)].is_empty());
        assert_eq!(seen.vtm_idle_reason_counts[&VtmIdleReasonKind::AwaitingTote], 1);
        assert_eq!(seen.station_timelines[&StationId(5)].len(), 1);
        assert_eq!(seen.station_busy_share()[&StationId(5)], 0.25);
        assert_eq!(seen.seen_until, SimTime::ms(10_000));
    }
}
