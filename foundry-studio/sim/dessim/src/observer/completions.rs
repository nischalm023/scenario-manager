//! Count physical flow completions through a reporting cutoff. Accepted work
//! may extend past that cutoff; its journal remains intact for diagnostics.

use desgraph::StationId;
use rustc_hash::FxHashMap;

use super::output::MetricsOutput;

#[derive(Default)]
pub struct FlowCompletions {
    pub picked_units: u64,
    pub completed_orders: u64,
    pub inducted_units: u64,
    pub inducted_totes: u64,
    pub picks_per_station: FxHashMap<StationId, u64>,
}

impl MetricsOutput {
    pub fn completions_through(&self, end_ms: u64) -> FlowCompletions {
        let mut counts = FlowCompletions::default();
        for pick in self.pick_log.iter().filter(|row| row.sim_time_ms <= end_ms) {
            counts.picked_units += u64::from(pick.quantity);
            *counts.picks_per_station.entry(pick.station_id).or_default() += u64::from(pick.quantity);
        }
        counts.completed_orders =
            self.completed_order_log.iter().filter(|row| row.sim_time_ms <= end_ms).count() as u64;
        for fill in self.putaway_log.iter().filter(|row| row.sim_time_ms <= end_ms) {
            counts.inducted_units += u64::from(fill.quantity);
            counts.inducted_totes += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OrderId, SkuId};
    use crate::observer::output::{CompletedOrderRecord, PickRecord, PutawayRecord};

    #[test]
    fn cutoff_includes_exact_completion_and_excludes_future_plans_without_erasing_them() {
        let mut output = MetricsOutput::default();
        for (time, quantity) in [(1001, 99), (500, 2), (1000, 3)] {
            output.pick_log.push(PickRecord {
                sim_time_ms: time,
                station_id: StationId(1),
                sku: SkuId(1),
                quantity,
                dc_id: None,
                order_id: None,
                tote_id: None,
                source_zone_id: None,
                source_relay_id: None,
                presentation_id: None,
            });
            output.putaway_log.push(PutawayRecord {
                sim_time_ms: time,
                station_id: StationId(2),
                sku: SkuId(1),
                quantity,
                presentation_id: None,
            });
            output.completed_order_log.push(CompletedOrderRecord {
                sim_time_ms: time,
                order_id: OrderId(time as u32),
                station_id: None,
                dc_id: None,
            });
        }
        let counts = output.completions_through(1000);
        assert_eq!(counts.picked_units, 5);
        assert_eq!(counts.inducted_units, 5);
        assert_eq!(counts.inducted_totes, 2);
        assert_eq!(counts.completed_orders, 2);
        assert_eq!(counts.picks_per_station[&StationId(1)], 5);
        assert_eq!(output.pick_log.len(), 3);
        assert_eq!(output.putaway_log.len(), 3);
        assert_eq!(output.completed_order_log.len(), 3);
    }
}
