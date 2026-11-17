//! App-level store over the per-slice station dispatch diagnostics.
//!
//! Built from the `station_diag` payload of the time-series `SliceRecord`s,
//! the same for a live run's received slices and for profile playback.
//! Rebuilt whenever the time series changes, as it does with each final slice
//! a live run receives: a full rebuild from the slices is cheap and always
//! consistent.
//!
//! All queries are as-of a playback cursor: they only consider records whose
//! sim timestamp is at or before the cursor, so scrubbing replays history
//! faithfully.

mod diagnosis;
mod recent;
#[cfg(test)]
mod tests;
mod timeline;

pub use diagnosis::StationDiagnosis;
pub use timeline::{ActivitySpan, ReservedSpan, ServingSpan};

use std::cmp::Reverse;

use desgraph::StationId;
use dessim::events::{
    DispatchRejectionReason, InventoryVisitPhase, OrderStationPhase, PlanLegKind, PresentationStream, VtmIdleReasonKind,
};
use dessim::model::{SimDuration, SimTime, StationArrivalSource};
use dessim::observer::activity::ActivityKind;
use dessim::observer::station_diag::{
    HtmArrivalRecord, HtmAssignmentRecord, HtmRelayPickupRecord, InventoryVisitPhaseRecord, OrderStationPhaseRecord,
    PipelineDepthRecord, PlanLegRecord, PresentationReadinessRecord, RejectionAggregate, RobotActivityRecord,
    StationCycleRecord, StationDiagRecord, VtmIdleReasonRecord,
};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Default)]
pub struct OrderPhaseCounts {
    pub conveyor_id: Option<u32>,
    pub assigned: usize,
    pub preparing: usize,
    pub dispatched: usize,
    pub admitted: usize,
    pub waiting: usize,
    pub presenting: usize,
    pub physical_residents: usize,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct InventoryVisitCounts {
    pub conwip: usize,
    pub upstream: usize,
    pub loaded_transit: usize,
}

/// Trailing window for the inspector's "recent blockers" list.
pub const REJECTION_WINDOW: SimDuration = SimDuration::ms(60_000);

/// Trailing window for the "recent events" feed, and the cap on how many of
/// those events are shown (newest first).
pub const RECENT_EVENTS_WINDOW: SimDuration = SimDuration::ms(120_000);
pub const RECENT_EVENTS_MAX: usize = 10;

/// One row in a resource's "recent events" feed.
pub struct RecentEvent {
    pub at: SimTime,
    pub text: String,
}

/// One HTM currently en-route to a station (assigned, not yet arrived).
pub struct InboundHtm {
    pub robot_id: u32,
    pub tote_id: Option<u32>,
    pub source: StationArrivalSource,
    pub relay_cell: Option<u32>,
    pub dispatch_time: SimTime,
    pub quoted_arrival: SimTime,
}

impl From<&HtmAssignmentRecord> for InboundHtm {
    fn from(a: &HtmAssignmentRecord) -> Self {
        Self {
            robot_id: a.robot_id,
            tote_id: a.tote_id,
            source: a.source,
            relay_cell: a.relay_cell,
            dispatch_time: a.dispatch_time,
            quoted_arrival: a.quoted_arrival,
        }
    }
}

/// Rejections of one reason within the query window, most recent last-seen.
pub struct RejectionSummary {
    pub reason: DispatchRejectionReason,
    pub count: u64,
    pub last_at: SimTime,
    pub last_detail: Option<String>,
}

/// Time-sorted flattening of every slice's [`StationDiagRecord`].
#[derive(Default)]
pub struct StationDiagStore {
    /// Sorted by `dispatch_time`.
    assignments: Vec<HtmAssignmentRecord>,
    /// Sorted by `arrival`.
    arrivals: Vec<HtmArrivalRecord>,
    /// Sorted by `last_at`. Per-slice same-reason aggregates, kept as-is.
    rejections: Vec<RejectionAggregate>,
    /// Sorted by `at`.
    pipeline_depths: Vec<PipelineDepthRecord>,
    /// Sorted by `start`. Each robot's intervals of work.
    activity: Vec<RobotActivityRecord>,
    /// Sorted by `at`. Per-VTM idle-reason classifications (no `Continuous`).
    vtm_idle_reasons: Vec<VtmIdleReasonRecord>,
    /// Sorted by `at`. Relay pickup dwells keyed by relay HTM-access cell.
    htm_relay_pickups: Vec<HtmRelayPickupRecord>,
    /// Sorted by `start`. Completed station cycle spans.
    station_cycles: Vec<StationCycleRecord>,
    /// Sorted by `enter`. Committed plan legs (one snapshot per bot).
    plan_legs: Vec<PlanLegRecord>,
    order_phases: Vec<OrderStationPhaseRecord>,
    presentation_readiness: Vec<PresentationReadinessRecord>,
    inventory_visit_phases: Vec<InventoryVisitPhaseRecord>,
    conveyor_presentations: FxHashMap<u32, FxHashSet<StationId>>,
}

impl StationDiagStore {
    pub fn build<'a>(slices: impl IntoIterator<Item = &'a StationDiagRecord>) -> Self {
        let mut store = Self::default();
        for slice in slices {
            store.assignments.extend(slice.assignments.iter().cloned());
            store.arrivals.extend(slice.arrivals.iter().copied());
            store.rejections.extend(slice.rejections.iter().cloned());
            store.pipeline_depths.extend(slice.pipeline_depths.iter().copied());
            store.activity.extend(slice.activity.iter().copied());
            store.vtm_idle_reasons.extend(slice.vtm_idle_reasons.iter().copied());
            store.htm_relay_pickups.extend(slice.htm_relay_pickups.iter().copied());
            store.station_cycles.extend(slice.station_cycles.iter().copied());
            store.plan_legs.extend(slice.plan_legs.iter().copied());
            store.order_phases.extend(slice.order_phases.iter().copied());
            store.presentation_readiness.extend(slice.presentation_readiness.iter().copied());
            store.inventory_visit_phases.extend(slice.inventory_visit_phases.iter().copied());
        }
        // Slices arrive time-bucketed but events within the sim are recorded
        // at plan-commit time, which can bucket "future" physical timestamps
        // ahead of earlier ones — sort so the cursor queries can early-exit.
        store.assignments.sort_by_key(|record| record.dispatch_time);
        store.arrivals.sort_by_key(|record| record.arrival);
        store.rejections.sort_by_key(|record| record.last_at);
        store.pipeline_depths.sort_by_key(|record| record.at);
        store.activity.sort_by_key(|record| record.start);
        store.vtm_idle_reasons.sort_by_key(|record| record.at);
        store.htm_relay_pickups.sort_by_key(|record| record.at);
        store.station_cycles.sort_by_key(|record| record.start);
        store.plan_legs.sort_by_key(|record| record.enter);
        store.order_phases.sort_by_key(|record| (record.at, phase_precedence(record.phase)));
        store.presentation_readiness.sort_by_key(|record| record.ready_at);
        store.inventory_visit_phases.sort_by_key(|record| (record.at, inventory_phase_precedence(record.phase)));
        for record in &store.order_phases {
            store.conveyor_presentations.entry(record.conveyor_id).or_default().insert(record.presentation_id);
        }
        store
    }

    pub fn has_order_phases(&self, station: StationId) -> bool {
        self.order_phases.iter().any(|record| record.presentation_id == station)
    }

    pub fn has_inventory_visit_phases(&self, conveyor_id: u32) -> bool {
        self.inventory_visit_phases.iter().any(|record| record.conveyor_id == conveyor_id)
    }

    pub fn conveyor_for_station(&self, station: StationId) -> Option<u32> {
        self.order_phases.iter().find(|record| record.presentation_id == station).map(|record| record.conveyor_id)
    }

    pub fn order_phase_counts(&self, station: StationId, cursor: SimTime) -> OrderPhaseCounts {
        let mut current = FxHashMap::default();
        let mut admitted = FxHashMap::default();
        let mut counts = OrderPhaseCounts::default();
        for record in self.order_phases.iter().take_while(|record| record.at <= cursor) {
            if record.presentation_id != station {
                continue;
            }
            counts.conveyor_id = Some(record.conveyor_id);
            current.insert(record.order_id, record.phase);
            match record.phase {
                OrderStationPhase::Admitted => {
                    admitted.insert(record.order_id, true);
                }
                OrderStationPhase::AreaExited => {
                    admitted.insert(record.order_id, false);
                }
                _ => {}
            }
        }
        counts.physical_residents = admitted.values().filter(|&&resident| resident).count();
        for phase in current.into_values() {
            match phase {
                OrderStationPhase::Assigned => counts.assigned += 1,
                OrderStationPhase::PreparationStarted => counts.preparing += 1,
                OrderStationPhase::Dispatched => counts.dispatched += 1,
                OrderStationPhase::Admitted => counts.admitted += 1,
                OrderStationPhase::Waiting => counts.waiting += 1,
                OrderStationPhase::Presenting => counts.presenting += 1,
                OrderStationPhase::FinalPickCompleted | OrderStationPhase::AreaExited => {}
            }
        }
        counts.assigned = counts.assigned
            + counts.preparing
            + counts.dispatched
            + counts.admitted
            + counts.waiting
            + counts.presenting;
        counts
    }

    pub fn shared_operator_busy_since(&self, conveyor_id: u32, cursor: SimTime) -> Option<SimTime> {
        let presentations = self.conveyor_presentations.get(&conveyor_id)?;
        self.station_cycles
            .iter()
            .filter(|cycle| presentations.contains(&cycle.station_id))
            .filter(|cycle| cycle.start <= cursor && cursor < cycle.start + cycle.dwell)
            .map(|cycle| cycle.start)
            .max()
    }

    pub fn order_count_series(&self, station: StationId, until: SimTime) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
        let mut assigned = 0_i64;
        let mut residents = 0_i64;
        let mut assigned_points = vec![[0.0, 0.0]];
        let mut resident_points = vec![[0.0, 0.0]];
        let mut index = 0;
        while index < self.order_phases.len() && self.order_phases[index].at <= until {
            let at = self.order_phases[index].at;
            let mut assigned_delta = 0_i64;
            let mut resident_delta = 0_i64;
            while index < self.order_phases.len() && self.order_phases[index].at == at {
                let record = self.order_phases[index];
                if record.presentation_id == station {
                    assigned_delta += match record.phase {
                        OrderStationPhase::Assigned => 1,
                        OrderStationPhase::FinalPickCompleted => -1,
                        _ => 0,
                    };
                    resident_delta += match record.phase {
                        OrderStationPhase::Admitted => 1,
                        OrderStationPhase::AreaExited => -1,
                        _ => 0,
                    };
                }
                index += 1;
            }
            if assigned_delta != 0 || resident_delta != 0 {
                let x = at.as_ms() as f64 / 60_000.0;
                assigned_points.push([x, assigned as f64]);
                resident_points.push([x, residents as f64]);
                assigned += assigned_delta;
                residents += resident_delta;
                assigned_points.push([x, assigned as f64]);
                resident_points.push([x, residents as f64]);
            }
        }
        assigned_points.push([until.as_ms() as f64 / 60_000.0, assigned as f64]);
        resident_points.push([until.as_ms() as f64 / 60_000.0, residents as f64]);
        (assigned_points, resident_points)
    }

    pub fn inventory_visit_counts(&self, conveyor_id: u32, cursor: SimTime) -> InventoryVisitCounts {
        let mut phases = FxHashMap::default();
        for record in self.inventory_visit_phases.iter().take_while(|record| record.at <= cursor) {
            if record.conveyor_id == conveyor_id {
                phases.insert((record.order_id, record.pick_index), record.phase);
            }
        }
        let mut counts = InventoryVisitCounts::default();
        for phase in phases.into_values() {
            match phase {
                InventoryVisitPhase::Released => {
                    counts.conwip += 1;
                    counts.upstream += 1;
                }
                InventoryVisitPhase::Loaded => {
                    counts.conwip += 1;
                    counts.upstream += 1;
                    counts.loaded_transit += 1;
                }
                InventoryVisitPhase::Boarded => counts.conwip += 1,
                InventoryVisitPhase::PickCompleted => {}
            }
        }
        counts
    }

    pub fn inventory_visit_series(
        &self,
        conveyor_id: u32,
        until: SimTime,
    ) -> (Vec<[f64; 2]>, Vec<[f64; 2]>, Vec<[f64; 2]>) {
        let mut values = [0_i64; 3];
        let mut series = [vec![[0.0, 0.0]], vec![[0.0, 0.0]], vec![[0.0, 0.0]]];
        let mut index = 0;
        while index < self.inventory_visit_phases.len() && self.inventory_visit_phases[index].at <= until {
            let at = self.inventory_visit_phases[index].at;
            let mut delta = [0_i64; 3];
            while index < self.inventory_visit_phases.len() && self.inventory_visit_phases[index].at == at {
                let record = self.inventory_visit_phases[index];
                if record.conveyor_id == conveyor_id {
                    match record.phase {
                        InventoryVisitPhase::Released => {
                            delta[0] += 1;
                            delta[1] += 1;
                        }
                        InventoryVisitPhase::Loaded => delta[2] += 1,
                        InventoryVisitPhase::Boarded => {
                            delta[1] -= 1;
                            delta[2] -= 1;
                        }
                        InventoryVisitPhase::PickCompleted => delta[0] -= 1,
                    }
                }
                index += 1;
            }
            if delta != [0, 0, 0] {
                let x = at.as_ms() as f64 / 60_000.0;
                for n in 0..3 {
                    series[n].push([x, values[n] as f64]);
                    values[n] += delta[n];
                    series[n].push([x, values[n] as f64]);
                }
            }
        }
        for index in 0..3 {
            series[index].push([until.as_ms() as f64 / 60_000.0, values[index] as f64]);
        }
        let [conwip, upstream, loaded] = series;
        (conwip, upstream, loaded)
    }

    pub fn latest_readiness_pair(&self, station: StationId, cursor: SimTime) -> Option<(SimTime, SimTime, u32, u32)> {
        let mut visits = FxHashMap::default();
        for record in self.presentation_readiness.iter().take_while(|record| record.ready_at <= cursor) {
            if record.presentation_id != station {
                continue;
            }
            let pair = visits.entry((record.order_id, record.pick_index)).or_insert((None, None));
            match record.stream {
                PresentationStream::Order => pair.0 = Some(record.ready_at),
                PresentationStream::Inventory => pair.1 = Some(record.ready_at),
            }
        }
        visits
            .into_iter()
            .filter_map(|((order, pick), (order_ready, inventory_ready))| {
                Some((order_ready?, inventory_ready?, order, pick))
            })
            .max_by_key(|&(order_ready, inventory_ready, _, _)| order_ready.max(inventory_ready))
    }

    /// A bot's committed plan legs with `enter >= cursor` — the remaining plan
    /// from the frontier — capped at `max`. Legs are globally sorted by
    /// `enter`, so this filters + truncates. Returns `(kind, dest_cell, enter)`
    /// tuples for the inspector.
    pub fn plan_legs_from(&self, robot_id: u32, cursor: SimTime, max: usize) -> Vec<(PlanLegKind, u32, SimTime)> {
        self.plan_legs
            .iter()
            .filter(|leg| leg.robot_id == robot_id && leg.enter >= cursor)
            .take(max)
            .map(|leg| (leg.kind, leg.dest_cell, leg.enter))
            .collect()
    }

    /// HTMs assigned to `station` at or before `cursor` with no arrival at or
    /// before `cursor`. Matching is per robot in time order: each arrival
    /// closes that robot's earliest still-open assignment (an arrival's
    /// dispatch always precedes it, so count-matching in sorted order is
    /// exact). Sorted by quoted arrival.
    pub fn inbound_htms_at(&self, station: StationId, cursor: SimTime) -> Vec<InboundHtm> {
        let mut open: Vec<&HtmAssignmentRecord> = Vec::new();
        for assignment in self.assignments.iter().filter(|a| a.station_id == station) {
            if assignment.dispatch_time > cursor {
                break;
            }
            open.push(assignment);
        }
        for arrival in self.arrivals.iter().filter(|r| r.station_id == station) {
            if arrival.arrival > cursor {
                break;
            }
            if let Some(position) = open.iter().position(|a| a.robot_id == arrival.robot_id) {
                open.remove(position);
            }
        }
        let mut inbound: Vec<InboundHtm> = open.into_iter().map(InboundHtm::from).collect();
        inbound.sort_by_key(|htm| htm.quoted_arrival);
        inbound
    }

    /// Rejections attributed to `station` within `(cursor - window, cursor]`,
    /// grouped by reason (counts summed, most recent occurrence kept), sorted
    /// by descending count.
    ///
    /// Granularity note: counts are per-slice aggregates keyed by their
    /// latest occurrence, so a bucket straddling the window edge contributes
    /// its whole slice count — the window is accurate to one slice width.
    pub fn rejections_in_window(
        &self,
        station: StationId,
        cursor: SimTime,
        window: SimDuration,
    ) -> Vec<RejectionSummary> {
        let window_start = cursor.saturating_sub(window);
        let mut summaries: Vec<RejectionSummary> = Vec::new();
        for aggregate in self.rejections.iter().filter(|r| r.station_id == station) {
            if aggregate.last_at > cursor {
                break;
            }
            if aggregate.last_at <= window_start {
                continue;
            }
            match summaries.iter_mut().find(|summary| summary.reason == aggregate.reason) {
                Some(summary) => {
                    summary.count += aggregate.count;
                    // Store is sorted by last_at, so this aggregate is the
                    // most recent so far for its reason.
                    summary.last_at = aggregate.last_at;
                    summary.last_detail = aggregate.last_detail.clone();
                }
                None => summaries.push(RejectionSummary {
                    reason: aggregate.reason,
                    count: aggregate.count,
                    last_at: aggregate.last_at,
                    last_detail: aggregate.last_detail.clone(),
                }),
            }
        }
        summaries.sort_by_key(|summary| Reverse(summary.count));
        summaries
    }

    /// The bot's live assignment at `cursor`: its latest assignment at or
    /// before the cursor, unless a matching arrival has already closed it.
    pub fn active_assignment_for_bot(&self, robot_id: u32, cursor: SimTime) -> Option<&HtmAssignmentRecord> {
        let mut latest: Option<&HtmAssignmentRecord> = None;
        for assignment in &self.assignments {
            if assignment.dispatch_time > cursor {
                break;
            }
            if assignment.robot_id == robot_id {
                latest = Some(assignment);
            }
        }
        let assignment = latest?;
        let arrived = self.arrivals.iter().take_while(|r| r.arrival <= cursor).any(|r| {
            r.robot_id == robot_id && r.station_id == assignment.station_id && r.arrival >= assignment.dispatch_time
        });
        if arrived { None } else { Some(assignment) }
    }

    /// Latest pipeline-depth snapshot for `station` at or before `cursor`.
    pub fn latest_pipeline_depth(&self, station: StationId, cursor: SimTime) -> Option<u32> {
        let mut latest = None;
        for record in &self.pipeline_depths {
            if record.at > cursor {
                break;
            }
            if record.station_id == station {
                latest = Some(record.depth);
            }
        }
        latest
    }

    /// Whether any robot's intervals of work have been recorded. A recording
    /// made before they were carries none (it carried robot state instead), so
    /// nothing can be said of what its robots were doing; neither can it of a
    /// live run before B has passed any interval, nor of a run in which no
    /// robot ever worked, which read the same and are said the same way.
    pub fn records_activity(&self) -> bool {
        !self.activity.is_empty()
    }

    /// What the bot is doing as of `cursor`, and since when: the interval
    /// containing it, or else the gap it is idle in, which began where its
    /// last interval ended, or at zero before its first. `None` when the
    /// recording carries no activity to say it from.
    pub fn activity_at(&self, robot_id: u32, cursor: SimTime) -> Option<(ActivityKind, SimTime)> {
        if !self.records_activity() {
            return None;
        }
        let mut doing = (ActivityKind::Gap, SimTime::ZERO);
        for record in self.activity.iter().take_while(|record| record.start <= cursor) {
            if record.robot_id != robot_id {
                continue;
            }
            doing = if cursor < record.end { (record.kind, record.start) } else { (ActivityKind::Gap, record.end) };
        }
        Some(doing)
    }

    /// The most recent VTM idle-reason classification for `robot_id` at or
    /// before `cursor`. Only non-`Continuous` reasons are stored, so a
    /// result always names a real constraint.
    pub fn latest_vtm_idle_reason(
        &self,
        robot_id: u32,
        cursor: SimTime,
    ) -> Option<(VtmIdleReasonKind, SimDuration, SimTime)> {
        let mut latest = None;
        for record in &self.vtm_idle_reasons {
            if record.at > cursor {
                break;
            }
            if record.robot_id == robot_id {
                latest = Some((record.reason, SimDuration::ms(record.gap_ms), record.at));
            }
        }
        latest
    }

    /// Whether the relay HTM-access `cell` had a pickup dwell within
    /// `(cursor - window, cursor]`, and the sum of those dwells. Powers the
    /// relay inspector's "recent pickup waits".
    pub fn htm_relay_pickups_in_window(&self, cell: u32, cursor: SimTime, window: SimDuration) -> (u64, SimDuration) {
        let window_start = cursor.saturating_sub(window);
        let mut count = 0;
        let mut total_ms = 0;
        for record in &self.htm_relay_pickups {
            if record.at > cursor {
                break;
            }
            if record.at > window_start && record.relay_cell == cell {
                count += 1;
                total_ms += record.wait_ms;
            }
        }
        (count, SimDuration::ms(total_ms))
    }

    /// HTM assignments routed through relay HTM-access `cell` that are open
    /// (dispatched, not yet arrived) as-of `cursor`, sorted by quoted arrival.
    /// Mirrors `inbound_htms_at` but keyed by relay cell.
    pub fn inbound_htms_via_relay(&self, cell: u32, cursor: SimTime) -> Vec<InboundHtm> {
        let mut open: Vec<&HtmAssignmentRecord> = Vec::new();
        for assignment in self.assignments.iter().filter(|a| a.relay_cell == Some(cell)) {
            if assignment.dispatch_time > cursor {
                break;
            }
            open.push(assignment);
        }
        // Close by arrival at the assignment's station (arrivals aren't
        // relay-keyed; a relay assignment closes when its HTM reaches the
        // station it was fetching for).
        for arrival in &self.arrivals {
            if arrival.arrival > cursor {
                break;
            }
            if let Some(position) =
                open.iter().position(|a| a.robot_id == arrival.robot_id && a.station_id == arrival.station_id)
            {
                open.remove(position);
            }
        }
        let mut inbound: Vec<InboundHtm> = open.into_iter().map(InboundHtm::from).collect();
        inbound.sort_by_key(|htm| htm.quoted_arrival);
        inbound
    }
}

fn phase_precedence(phase: OrderStationPhase) -> u8 {
    match phase {
        OrderStationPhase::Assigned => 0,
        OrderStationPhase::PreparationStarted => 1,
        OrderStationPhase::Dispatched => 2,
        OrderStationPhase::Admitted => 3,
        OrderStationPhase::Waiting => 4,
        OrderStationPhase::Presenting => 5,
        OrderStationPhase::FinalPickCompleted => 6,
        OrderStationPhase::AreaExited => 7,
    }
}

fn inventory_phase_precedence(phase: InventoryVisitPhase) -> u8 {
    match phase {
        InventoryVisitPhase::Released => 0,
        InventoryVisitPhase::Loaded => 1,
        InventoryVisitPhase::Boarded => 2,
        InventoryVisitPhase::PickCompleted => 3,
    }
}
