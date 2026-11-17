//! Companion-report samples from completion journals, physical rack snapshots,
//! and elapsed HTM waits. Wait intervals are clipped and split into buckets;
//! the renderer divides their sum by fleet size and observed bucket duration.

use std::error::Error;

use desgraph::RobotType;
use desmap::{WarehouseMap, ZoneHolds};
use dessim::events::{ToteSource, VtmCycleDirection};
use dessim::metrics::profile::SummaryStats;
use dessim::observer::output::{AisleStateRecord, PickRecord, PutawayRecord, VtmCycleRecord};
use dessim::observer::time_slice::BotStepKind;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Slice;

/// The columns of the run's summary this report reads, taken straight off it.
///
/// Named here rather than deserialized from the whole summary: the fields it
/// wants are six, the summary has sixty, and passing it through a JSON value
/// copied every journal twice for the sake of four of them.
#[derive(Default)]
pub struct Summary {
    pub sim_time_secs: f64,
    htm_fleet_size: u32,
    pick_log: Vec<Flow>,
    putaway_log: Vec<Flow>,
    vtm_cycle_log: Vec<Cycle>,
    aisle_state_log: Vec<AisleStateRecord>,
}

impl Summary {
    pub fn of(summary: &SummaryStats) -> Self {
        Self {
            sim_time_secs: summary.sim_time_secs,
            htm_fleet_size: summary.htm_fleet_size as u32,
            pick_log: summary.pick_log.iter().map(Flow::pick).collect(),
            putaway_log: summary.putaway_log.iter().map(Flow::putaway).collect(),
            vtm_cycle_log: summary.vtm_cycle_log.iter().map(Cycle::of).collect(),
            aisle_state_log: summary.aisle_state_log.clone(),
        }
    }
}

struct Flow {
    sim_time_ms: u64,
    quantity: u64,
    presentation_id: Option<u64>,
}

impl Flow {
    fn pick(row: &PickRecord) -> Self {
        Self { sim_time_ms: row.sim_time_ms, quantity: u64::from(row.quantity), presentation_id: row.presentation_id }
    }

    fn putaway(row: &PutawayRecord) -> Self {
        Self { sim_time_ms: row.sim_time_ms, quantity: u64::from(row.quantity), presentation_id: row.presentation_id }
    }
}

struct Cycle {
    sim_time_ms: u64,
    total_cycle_ms: u64,
    direction: VtmCycleDirection,
    tote_source: ToteSource,
    presentation_id: Option<u64>,
}

impl Cycle {
    fn of(row: &VtmCycleRecord) -> Self {
        Self {
            sim_time_ms: row.sim_time_ms,
            total_cycle_ms: row.total_cycle_ms,
            direction: row.direction,
            tote_source: row.tote_source,
            presentation_id: row.presentation_id,
        }
    }
}

#[derive(Default, Serialize)]
struct Wait {
    elapsed_wait_ms: u64,
    implicit_hold_ms: u64,
}

impl Wait {
    fn add(&mut self, other: &Self) {
        self.elapsed_wait_ms += other.elapsed_wait_ms;
        self.implicit_hold_ms += other.implicit_hold_ms;
    }
}

pub struct Series {
    pub summary: Summary,
    bin_ms: u64,
    waits: Vec<FxHashMap<u32, Wait>>,
}

impl Series {
    pub fn new(bin_ms: u64) -> Result<Self, Box<dyn Error>> {
        if bin_ms == 0 {
            return Err("Series interval must be positive".into());
        }
        Ok(Self { summary: Summary::default(), bin_ms, waits: Vec::new() })
    }

    fn bin(&mut self, at: u64, cell: u32) -> &mut Wait {
        let index = (at / self.bin_ms) as usize;
        self.waits.resize_with(self.waits.len().max(index + 1), FxHashMap::default);
        self.waits[index].entry(cell).or_default()
    }

    fn wait(&mut self, mut start: u64, end: u64, cell: u32, implicit: bool) {
        while start < end {
            let stop = end.min((start / self.bin_ms + 1) * self.bin_ms);
            let bin = self.bin(start, cell);
            bin.elapsed_wait_ms += stop - start;
            if implicit {
                bin.implicit_hold_ms += stop - start;
            }
            start = stop;
        }
    }

    pub fn add(&mut self, record: &Slice, until: u64) {
        for loc in record.bot_locations.values() {
            if loc.robot_type != RobotType::Htm {
                continue;
            }
            let mut at = record.start_ms;
            let mut cell = loc.start_cell;
            for step in &loc.steps {
                let end = at.saturating_add(step.delta_ms);
                if step.kind == BotStepKind::Wait && at < until && end > at {
                    self.wait(at, end.min(until), step.cell_id, false);
                }
                at = end;
                cell = step.cell_id;
                if at >= until {
                    break;
                }
            }
            if at < until {
                self.wait(at, until, cell, true);
            }
        }
    }

    pub fn finish(
        mut self,
        mut output: Value,
        map: &WarehouseMap,
        exclusions: &FxHashMap<u32, &str>,
        until: u64,
    ) -> Result<Value, Box<dyn Error>> {
        if until == 0 || self.summary.htm_fleet_size == 0 {
            return Err("Series require a positive observed duration and HTM fleet size".into());
        }
        let excluded: FxHashSet<_> = exclusions.keys().copied().collect();
        let equipment: FxHashSet<_> =
            map.conveyor_machines.iter().flat_map(|machine| machine.cells.iter().map(|cell| cell.0)).collect();
        let count = until.div_ceil(self.bin_ms) as usize;
        self.waits.resize_with(count, FxHashMap::default);
        let mut totals = Wait::default();
        let mut bins = Vec::new();
        for (i, cells) in self.waits.into_iter().enumerate().take(count) {
            let mut eligible = Wait::default();
            for (cell, wait) in cells {
                if !excluded.contains(&cell) {
                    eligible.add(&wait);
                }
            }
            totals.add(&eligible);
            bins.push(json!({"start_ms":i as u64*self.bin_ms,"end_ms":until.min((i+1) as u64*self.bin_ms),
                "wait":eligible,"picks":0,"pick_transfers":0,"inducted_units":0,"filled_totes":0,"stored_totes":0}));
        }
        let index = |at: u64| (at / self.bin_ms).min(count as u64 - 1) as usize;
        let mut change = |at: u64, key: &str, amount: u64| {
            if at <= until {
                let bin = &mut bins[index(at)];
                bin[key] = json!(bin[key].as_u64().unwrap() + amount);
            }
        };
        for row in &self.summary.pick_log {
            change(row.sim_time_ms, "picks", row.quantity);
            change(row.sim_time_ms, "pick_transfers", 1);
        }
        let mut fills = FxHashSet::default();
        for row in &self.summary.putaway_log {
            change(row.sim_time_ms, "inducted_units", row.quantity);
            change(row.sim_time_ms, "filled_totes", 1);
            if row.sim_time_ms <= until {
                fills.extend(row.presentation_id);
            }
        }
        for row in &self.summary.vtm_cycle_log {
            if row.direction == VtmCycleDirection::Return
                && row.tote_source == ToteSource::Putaway
                && row.presentation_id.is_some_and(|id| fills.contains(&id))
            {
                change(row.sim_time_ms + row.total_cycle_ms, "stored_totes", 1);
            }
        }
        let zones: FxHashSet<_> =
            map.zones.iter().filter(|z| z.holds == ZoneHolds::Inventory).map(|z| z.id.0).collect();
        self.summary.aisle_state_log.sort_by_key(|row| row.sim_time_ms);
        let mut deltas: Vec<_> = self
            .summary
            .pick_log
            .iter()
            .map(|r| (r.sim_time_ms, -(r.quantity as i64)))
            .chain(self.summary.putaway_log.iter().map(|r| (r.sim_time_ms, r.quantity as i64)))
            .collect();
        deltas.sort_by_key(|&(at, _)| at);
        let mut inventory_cursor = 0;
        let mut net_inventory_units = 0;
        let mut current = FxHashMap::default();
        let mut cursor = 0;
        let mut racks = Vec::new();
        let times = std::iter::once(0).chain(bins.iter().map(|bin| bin["end_ms"].as_u64().unwrap()));
        for at in times {
            while let Some(&(_, delta)) = deltas.get(inventory_cursor).filter(|&&(when, _)| when <= at) {
                net_inventory_units += delta;
                inventory_cursor += 1;
            }
            while let Some(row) = self.summary.aisle_state_log.get(cursor).filter(|row| row.sim_time_ms <= at) {
                if zones.contains(&row.zone_id) {
                    current.insert(row.zone_id, *row);
                }
                cursor += 1;
            }
            let sum = |f: fn(&AisleStateRecord) -> u32| current.values().map(|row| u64::from(f(row))).sum::<u64>();
            racks.push(
                json!({"at_ms":at,"net_inventory_units":net_inventory_units,"capacity":sum(|r|r.storage_capacity),
                "occupied":sum(|r|r.storage_occupied),"stocked":sum(|r|r.storage_non_empty),
                "empty":sum(|r|r.storage_empty),"unknown":sum(|r|r.storage_unknown)}),
            );
        }
        let old_wait = output["eligible_wait_s"].as_f64().ok_or("Missing cumulative traffic total")?;
        if (totals.elapsed_wait_ms as f64 / 1000.0 - old_wait).abs() > 0.001 {
            return Err("Time-series wait does not reconcile with cumulative traffic".into());
        }
        let mut excluded: Vec<_> = excluded.into_iter().collect();
        excluded.sort_unstable();
        let mut equipment: Vec<_> = equipment.into_iter().collect();
        equipment.sort_unstable();
        output["series"] = json!({"bin_ms":self.bin_ms,"htm_fleet_size":self.summary.htm_fleet_size,
            "bins":bins,"racks":racks,"totals":totals,
            "excluded_cell_ids":excluded,"equipment_cell_ids":equipment,
            "filter":"WarehouseMap::navigation_wait_exclusions"});
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_wait_splits_at_boundaries_and_sums_across_htms() {
        let mut series = Series::new(1000).unwrap();
        series.wait(800, 1200, 9, false);
        series.wait(0, 900, 8, false);
        assert_eq!(series.waits[0].values().map(|w| w.elapsed_wait_ms).sum::<u64>(), 1100);
        assert_eq!(series.waits[1][&9].elapsed_wait_ms, 200);
    }

    #[test]
    fn implicit_holds_are_an_explicit_subset_of_total_wait() {
        let mut series = Series::new(1000).unwrap();
        series.wait(0, 700, 9, true);
        series.wait(800, 900, 9, false);
        assert_eq!(series.waits[0][&9].implicit_hold_ms, 700);
        assert_eq!(series.waits[0][&9].elapsed_wait_ms, 800);
    }
}
