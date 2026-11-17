//! Within `finality`, `rows_due` decides which journal rows go out with each
//! slice made final: a row goes with the first slice made final once B has
//! passed the instant that makes the row final, and the run's last slice
//! takes every row left that the run filed by its stop.
//!
//! A frame states only simtime B has passed, so a reader holding the frames up
//! to an instant holds nothing about simtime past it. Most rows state one
//! instant, their own. A leg states a span, from its start to its close, and a
//! VTM cycle one from its start to its end, split into its legs; each is due
//! once B has passed the span's end, and its start stays the instant the row
//! is filed at. A row filed after the run's stop, which only work the
//! scheduler committed past a simtime limit files, is never stated.

use crate::SimTime;
use crate::observer::output::{HtmLegRecord, MetricsOutput, VtmCycleRecord, VtmLegRecord};

use super::recording::SliceRows;

/// Which rows a frame takes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Due {
    /// Every row the journals hold as the frame is made whose instant of finality is before this one.
    FinalBefore(SimTime),
    /// Every row not yet taken that the run filed by its stop, here, as the summary counts the run's work by it;
    /// the rest are dropped. A row filed by the stop keeps its whole duration, though it may end after the stop.
    FiledBy(SimTime),
}

/// Which journal rows each slice made final takes, as the run goes.
///
/// The observer does not append a journal in the order its rows become due: a
/// row appended ahead of its slice waits for it, and one appended after its
/// slice went out (a stand reported when the visit closes, a gap closed by the
/// run's end) goes with the slice being made final, where it became known.
/// Rows due at one instant keep the order they were appended in, so a sheet
/// that sorts by the instant a row is due at reads the same as one of the
/// finished journal.
#[derive(Default)]
pub(super) struct RowsDue {
    htm_legs: Waiting,
    vtm_legs: Waiting,
    vtm_cycles: Waiting,
    vtm_operations: Waiting,
    picks: Waiting,
    putaways: Waiting,
    completed_orders: Waiting,
    aisle_states: Waiting,
    aisle_flux: Waiting,
}

impl RowsDue {
    /// The rows `due` takes, each journal's in the order they were appended.
    pub(super) fn take(&mut self, output: &MetricsOutput, due: Due) -> SliceRows {
        SliceRows {
            htm_legs: self.htm_legs.take(&output.htm_leg_log, htm_travel, due),
            vtm_legs: self.vtm_legs.take(&output.vtm_leg_log, vtm_leg, due),
            vtm_cycles: self.vtm_cycles.take(&output.vtm_cycle_log, vtm_cycle, due),
            vtm_operations: self.vtm_operations.take(&output.vtm_operation_log, |row| at(row.sim_time_ms), due),
            picks: self.picks.take(&output.pick_log, |row| at(row.sim_time_ms), due),
            putaways: self.putaways.take(&output.putaway_log, |row| at(row.sim_time_ms), due),
            completed_orders: self.completed_orders.take(&output.completed_order_log, |row| at(row.sim_time_ms), due),
            aisle_states: self.aisle_states.take(&output.aisle_state_log, |row| at(row.sim_time_ms), due),
            aisle_flux: self.aisle_flux.take(&output.aisle_flux_log, |row| at(row.sim_time_ms), due),
        }
    }
}

/// Drop from the run's journals every row it filed after its `stop`, once the recording has taken its rows: the run's
/// own figures, as the recording, end at the stop.
pub(super) fn drop_rows_filed_after(output: &mut MetricsOutput, stop: SimTime) {
    let by = |filed: u64| filed <= stop.as_ms();
    output.htm_leg_log.retain(|row| by(htm_travel(row).0));
    output.vtm_leg_log.retain(|leg| by(vtm_leg(leg).0));
    output.vtm_cycle_log.retain(|row| by(vtm_cycle(row).0));
    output.vtm_operation_log.retain(|row| by(row.sim_time_ms));
    output.pick_log.retain(|row| by(row.sim_time_ms));
    output.putaway_log.retain(|row| by(row.sim_time_ms));
    output.completed_order_log.retain(|row| by(row.sim_time_ms));
    output.aisle_state_log.retain(|row| by(row.sim_time_ms));
    output.aisle_flux_log.retain(|row| by(row.sim_time_ms));
    output.extraction_log.retain(|row| by(row.sim_time_ms));
}

/// An HTM's travel row is filed at its start and final at its end.
fn htm_travel(row: &HtmLegRecord) -> (u64, u64) {
    (row.start.as_ms(), (row.start + row.duration).as_ms())
}

fn vtm_leg(leg: &VtmLegRecord) -> (u64, u64) {
    (leg.start_ms, leg.start_ms + leg.duration_ms)
}

/// A VTM cycle is filed at its start and final at its end.
fn vtm_cycle(row: &VtmCycleRecord) -> (u64, u64) {
    (row.sim_time_ms, row.sim_time_ms + row.total_cycle_ms)
}

/// A row stating one instant: filed at it and final at it.
fn at(ms: u64) -> (u64, u64) {
    (ms, ms)
}

/// One journal's rows appended but not yet taken.
#[derive(Default)]
struct Waiting {
    /// How many of the journal's rows have been looked at.
    seen: usize,
    /// The positions of the rows looked at and not yet taken, append order.
    positions: Vec<u32>,
}

impl Waiting {
    /// The rows of `journal` that `due` takes, by `instants`, the instant each row is filed at and the one that
    /// makes it final, in milliseconds.
    fn take<T: Clone>(&mut self, journal: &[T], instants: impl Fn(&T) -> (u64, u64), due: Due) -> Vec<T> {
        self.positions.extend(self.seen as u32..journal.len() as u32);
        self.seen = journal.len();
        let mut taken = Vec::new();
        self.positions.retain(|&position| {
            let row = &journal[position as usize];
            let (filed, final_at) = instants(row);
            let (takes, waits) = match due {
                Due::FinalBefore(until) => (final_at < until.as_ms(), final_at >= until.as_ms()),
                Due::FiledBy(stop) => (filed <= stop.as_ms(), false),
            };
            if takes {
                taken.push(row.clone());
            }
            waits
        });
        taken
    }
}

#[cfg(test)]
mod tests {
    use desgraph::StationId;
    use serde_json::{from_value, json};

    use super::*;
    use crate::events::VtmCycleDirection;
    use crate::model::ids::SkuId;
    use crate::model::{RobotId, SimDuration, SimTime};
    use crate::observer::output::{
        Activity, HtmJournalEndpoint, HtmLegRecord, PutawayRecord, VtmCycleRecord, VtmJournalEndpoint, VtmLegRecord,
    };

    /// Each slice takes the rows appended by then whose instant is before its
    /// end: a row appended ahead of its slice waits for it, one appended after
    /// its slice went out goes with the slice being made final, and the last
    /// slice takes what is left. Rows sharing an instant keep their append
    /// order, so a stable sort by instant reads as the journal's own.
    #[test]
    fn a_row_goes_out_with_the_first_slice_made_final_once_b_has_passed_its_instant() {
        let putaway = |sim_time_ms, quantity| PutawayRecord {
            sim_time_ms,
            station_id: StationId(1),
            sku: SkuId(2),
            quantity,
            presentation_id: None,
        };
        let mut output = MetricsOutput {
            putaway_log: vec![putaway(5_000, 1), putaway(15_000, 2), putaway(5_000, 3), putaway(10_000, 8)],
            ..MetricsOutput::default()
        };
        let mut due = RowsDue::default();
        let mut take = |output: &MetricsOutput, due_by| -> Vec<(u64, u32)> {
            due.take(output, due_by).putaways.iter().map(|row| (row.sim_time_ms, row.quantity)).collect()
        };
        let before = |ms| Due::FinalBefore(SimTime::ms(ms));
        assert_eq!(take(&output, before(10_000)), [(5_000, 1), (5_000, 3)], "the 10 s and 15 s rows wait for theirs");
        output.putaway_log.extend([putaway(25_000, 4), putaway(8_000, 5), putaway(12_000, 6)]);
        assert_eq!(
            take(&output, before(20_000)),
            [(15_000, 2), (10_000, 8), (8_000, 5), (12_000, 6)],
            "the 8 s row was appended after its slice went out, and goes with the next"
        );
        output.putaway_log.extend([putaway(31_000, 7), putaway(36_000, 9)]);
        let last = Due::FiledBy(SimTime::ms(35_000));
        assert_eq!(
            take(&output, last),
            [(25_000, 4), (31_000, 7)],
            "the last slice takes what the run filed by its stop"
        );
        assert_eq!(take(&output, last), [], "each row goes out once, and one filed after the stop never");
    }

    /// A leg of either fleet goes out with the first slice made final once B
    /// has passed its close, not its start: a leg closing on a slice's end
    /// goes with the slice after it, one spanning slices waits for the slice
    /// it closes in, and one appended after that slice went out goes with the
    /// slice being made final. Each keeps its start as the instant it is
    /// filed at.
    #[test]
    fn a_leg_goes_out_once_b_has_passed_its_close() {
        let htm_leg = |start_ms, duration_ms, robot| HtmLegRecord {
            start: SimTime::ms(start_ms),
            activity: Activity::Travel,
            from: HtmJournalEndpoint::Parking,
            to: HtmJournalEndpoint::PickStation(StationId(1)),
            from_zone_id: None,
            to_zone_id: None,
            distance_mm: 0,
            duration: SimDuration::ms(duration_ms),
            presentation_id: None,
            robot_id: RobotId(robot),
            carrying_cargo: false,
            turn_count: 0,
            wait_reason: None,
            hold: None,
            notes: String::new(),
        };
        let vtm_leg = |start_ms, duration_ms, robot| VtmLegRecord {
            start_ms,
            duration_ms,
            activity: Activity::Travel,
            from: VtmJournalEndpoint::Aisle,
            to: VtmJournalEndpoint::Aisle,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm: 0,
            vtm_id: robot,
            vtm_cycle_id: None,
            tote_id: None,
            presentation_id: None,
            direction: VtmCycleDirection::Forward,
            carrying_tote: false,
            wait_reason: None,
            notes: String::new(),
        };
        // Robot 1 closes at 3 s, robot 2 on the first slice's end, robot 3
        // at 25 s; robot 4's leg, from 2 s to 8 s, is appended once the first
        // slice has gone out.
        let ahead = [(1_000, 2_000, 1), (5_000, 5_000, 2), (5_000, 20_000, 3)];
        let mut output = MetricsOutput {
            htm_leg_log: ahead.iter().map(|&(start, duration, robot)| htm_leg(start, duration, robot)).collect(),
            vtm_leg_log: ahead.iter().map(|&(start, duration, robot)| vtm_leg(start, duration, robot)).collect(),
            ..MetricsOutput::default()
        };
        let mut due = RowsDue::default();
        let mut take = |output: &MetricsOutput, until| {
            let rows = due.take(output, Due::FinalBefore(SimTime::ms(until)));
            let htm: Vec<(u64, u32)> = rows.htm_legs.iter().map(|leg| (leg.start.as_ms(), leg.robot_id.0)).collect();
            let vtm: Vec<(u64, u32)> = rows.vtm_legs.iter().map(|leg| (leg.start_ms, leg.vtm_id)).collect();
            assert_eq!(htm, vtm, "both fleets' legs go out alike");
            htm
        };
        assert_eq!(take(&output, 10_000), [(1_000, 1)], "robot 2 closes on the slice's end, robot 3 after it");
        output.htm_leg_log.push(htm_leg(2_000, 6_000, 4));
        output.vtm_leg_log.push(vtm_leg(2_000, 6_000, 4));
        assert_eq!(take(&output, 20_000), [(5_000, 2), (2_000, 4)], "robot 4's leg was learned late");
        assert_eq!(take(&output, 30_000), [(5_000, 3)], "robot 3's leg waits for the slice it closes in");

        // The run stops at 35 s: robot 5's leg, filed at 32 s, closes past the stop and goes with the last slice;
        // robot 6's, filed at 36 s, is work committed past the stop and is never stated.
        output.htm_leg_log.extend([htm_leg(32_000, 8_000, 5), htm_leg(36_000, 1_000, 6)]);
        let rows = due.take(&output, Due::FiledBy(SimTime::ms(35_000)));
        assert_eq!(rows.htm_legs.iter().map(|leg| leg.robot_id.0).collect::<Vec<_>>(), [5]);
    }

    /// A VTM cycle goes out with the first slice made final once B has passed
    /// its end, not its start: its row states the end and the split of its
    /// legs, which the frame of the slice it starts in cannot state before B
    /// has passed them. Each keeps its start as the instant it is filed at.
    #[test]
    fn a_vtm_cycle_goes_out_once_b_has_passed_its_end() {
        let cycle = |start_ms: u64, total_ms: u64, vtm_id: u32| -> VtmCycleRecord {
            from_value(json!({
                "sim_time_ms": start_ms, "vtm_id": vtm_id, "zone_id": 1, "direction": "Forward",
                "empty_leg_ms": 0, "loaded_leg_ms": 0, "fork_ms": 0, "total_cycle_ms": total_ms,
            }))
            .expect("a cycle row")
        };
        // VTM 1's cycle ends at 3 s, VTM 2's on the first slice's end, VTM 3's
        // at 25 s.
        let output = MetricsOutput {
            vtm_cycle_log: vec![cycle(1_000, 2_000, 1), cycle(5_000, 5_000, 2), cycle(5_000, 20_000, 3)],
            ..MetricsOutput::default()
        };
        let mut due = RowsDue::default();
        let mut take = |until| -> Vec<(u64, u32)> {
            let rows = due.take(&output, Due::FinalBefore(SimTime::ms(until))).vtm_cycles;
            rows.iter().map(|cycle| (cycle.sim_time_ms, cycle.vtm_id)).collect()
        };
        assert_eq!(take(10_000), [(1_000, 1)], "VTM 2's cycle ends on the slice's end, VTM 3's after it");
        assert_eq!(take(20_000), [(5_000, 2)]);
        assert_eq!(take(30_000), [(5_000, 3)], "VTM 3's cycle waits for the slice it ends in");
    }
}
