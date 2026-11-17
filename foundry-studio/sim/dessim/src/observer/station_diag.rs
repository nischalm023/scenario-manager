//! Per-slice resource dispatch diagnostics for the desvis resource inspector.
//!
//! One [`StationDiagRecord`] rides on each
//! [`super::time_slice::TimeSliceData`] (and its serialized twin
//! `SliceRecord`), carrying — for the sim timestamps that fall inside that
//! slice — HTM assignments/arrivals, dispatch rejections, pipeline-depth
//! snapshots, per-bot intervals of work, per-VTM idle reasons, and relay
//! fork-Gets. Timestamps are **absolute sim times** (not slice offsets) so
//! consumers can query across slice boundaries without offset math; the
//! slice is purely the transport bucket, mirroring how `tote_state`
//! transitions flow one slice at a time to both live polling and saved
//! profiles.
//!
//! The type keeps its Phase-1 name `StationDiagRecord` (it began as
//! station-only) to preserve the serde field name and struct-literal sites;
//! it now covers bot and relay diagnostics too.

use desgraph::StationId;
use serde::{Deserialize, Serialize};

use crate::SimTime;
use crate::events::{
    DispatchRejectionEvent, DispatchRejectionReason, InventoryVisitPhase, OrderStationPhase, PlanLegKind,
    PresentationStream, VtmIdleReasonKind,
};
use crate::model::{SimDuration, StationArrivalSource};
use crate::observer::activity::ActivityKind;

/// One HTM bound to a station-bound chain (from `HtmAssignmentEvent`).
/// Robot/tote ids are raw `u32`s, matching the slice types' bot-id keying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HtmAssignmentRecord {
    pub robot_id: u32,
    pub station_id: StationId,
    pub tote_id: Option<u32>,
    pub source: StationArrivalSource,
    /// Relay HTM-access cell id the chain fetches through, or `None` for
    /// direct-putaway chains. Stored as a cell id (not `RelayId`) so the
    /// relay inspector — keyed by clicked cell — can match without a
    /// relay→cell table on the desvis side. Defaulted so Phase-1 profiles
    /// (recorded before this field existed) still deserialize.
    #[serde(default)]
    pub relay_cell: Option<u32>,
    /// Planner pass in which this assignment committed. Defaulted (0) so
    /// pre-Phase-5 profiles still deserialize; pass 0 never commits chains
    /// live, so 0 reads as "unrecorded".
    #[serde(default)]
    pub dispatch_time: SimTime,
    pub quoted_arrival: SimTime,
}

/// One HTM arrival at a station presentation cell (from `HtmArrivalEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HtmArrivalRecord {
    pub robot_id: u32,
    pub station_id: StationId,
    pub arrival: SimTime,
}

/// Same-reason dispatch rejections for one station within one slice,
/// aggregated at ingest so a hot rejection path (one event per planning
/// pass) stays bounded: count + most-recent occurrence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectionAggregate {
    pub station_id: StationId,
    pub reason: DispatchRejectionReason,
    pub count: u64,
    pub last_at: SimTime,
    pub last_detail: Option<String>,
}

/// One station pipeline-depth snapshot (from `PipelineSnapshotEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PipelineDepthRecord {
    pub station_id: StationId,
    pub depth: u32,
    pub at: SimTime,
}

/// One completed station cycle (from `CycleCompleteEvent`), spanning
/// `[start, start + dwell)`. Lets the inspector decide "Serving" (a cycle
/// covers the cursor) vs "Idle since <last completion>".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StationCycleRecord {
    pub station_id: StationId,
    /// Operator dwell start (the `CycleComplete` sim time).
    pub start: SimTime,
    /// Operator dwell duration; the cycle ends at `start + dwell`.
    pub dwell: SimDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrderStationPhaseRecord {
    pub order_id: u32,
    pub conveyor_id: u32,
    pub presentation_id: StationId,
    pub waiting_cell: Option<u32>,
    pub at: SimTime,
    pub phase: OrderStationPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PresentationReadinessRecord {
    pub order_id: u32,
    pub pick_index: u32,
    pub conveyor_id: u32,
    pub presentation_id: StationId,
    pub stream: PresentationStream,
    pub ready_at: SimTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InventoryVisitPhaseRecord {
    pub conveyor_id: u32,
    pub order_id: u32,
    pub pick_index: u32,
    pub phase: InventoryVisitPhase,
    pub at: SimTime,
}

/// One interval of a robot's work for the bot inspector: what it was doing,
/// and between which instants. Drives the inspector's "what is this bot
/// doing as-of the cursor" line and its Wait diagnosis; geometry lives in
/// `bot_locations`, not here.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RobotActivityRecord {
    pub robot_id: u32,
    pub kind: ActivityKind,
    pub start: SimTime,
    pub end: SimTime,
}

/// One per-VTM idle-reason classification (from `VtmIdleReasonEvent`), for
/// the bot inspector's "blocked/idle because…" line. `Continuous` (no gap)
/// is not recorded — it carries no diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VtmIdleReasonRecord {
    pub robot_id: u32,
    pub reason: VtmIdleReasonKind,
    /// Idle gap in milliseconds that preceded the cycle at `at`.
    pub gap_ms: u64,
    /// Relay HTM-access cell the VTM idled waiting on.
    pub relay_cell: u32,
    pub at: SimTime,
}

/// One HTM fork-Get at a diving board and the dwell that preceded it (from
/// `HtmRelayPickupEvent`), for the relay inspector. Keyed by the relay's
/// HTM-access cell so it matches a clicked relay cell without a relay→cell
/// table on the desvis side.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HtmRelayPickupRecord {
    pub robot_id: u32,
    pub relay_cell: u32,
    pub wait_ms: u64,
    pub at: SimTime,
}

/// One committed plan leg for the bot inspector's real leg semantics (from
/// `BotPlanSnapshotEvent`). `kind`/`dest_cell`/`enter` mirror `PlanLegInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlanLegRecord {
    pub robot_id: u32,
    pub kind: PlanLegKind,
    pub dest_cell: u32,
    pub enter: SimTime,
}

/// Per-slice resource dispatch diagnostics. See module docs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StationDiagRecord {
    pub assignments: Vec<HtmAssignmentRecord>,
    pub arrivals: Vec<HtmArrivalRecord>,
    pub rejections: Vec<RejectionAggregate>,
    pub pipeline_depths: Vec<PipelineDepthRecord>,
    #[serde(default)]
    pub activity: Vec<RobotActivityRecord>,
    #[serde(default)]
    pub vtm_idle_reasons: Vec<VtmIdleReasonRecord>,
    #[serde(default, alias = "relay_pickup_waits")]
    pub htm_relay_pickups: Vec<HtmRelayPickupRecord>,
    #[serde(default)]
    pub station_cycles: Vec<StationCycleRecord>,
    #[serde(default)]
    pub order_phases: Vec<OrderStationPhaseRecord>,
    #[serde(default)]
    pub presentation_readiness: Vec<PresentationReadinessRecord>,
    #[serde(default)]
    pub inventory_visit_phases: Vec<InventoryVisitPhaseRecord>,
    /// Committed plan legs, emitted once per bot at planning completion.
    #[serde(default)]
    pub plan_legs: Vec<PlanLegRecord>,
}

impl StationDiagRecord {
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
            && self.arrivals.is_empty()
            && self.rejections.is_empty()
            && self.pipeline_depths.is_empty()
            && self.activity.is_empty()
            && self.vtm_idle_reasons.is_empty()
            && self.htm_relay_pickups.is_empty()
            && self.station_cycles.is_empty()
            && self.order_phases.is_empty()
            && self.presentation_readiness.is_empty()
            && self.inventory_visit_phases.is_empty()
            && self.plan_legs.is_empty()
    }

    /// Fold one rejection into this slice's aggregates.
    pub(super) fn add_rejection(&mut self, event: &DispatchRejectionEvent) {
        self.join_rejections(RejectionAggregate {
            station_id: event.station_id,
            reason: event.reason,
            count: event.count,
            last_at: event.at,
            last_detail: event.detail.clone(),
        });
    }

    /// Fold `rejections`, counted for one station and reason, into this slice's aggregate for them.
    pub(super) fn join_rejections(&mut self, rejections: RejectionAggregate) {
        match self
            .rejections
            .iter_mut()
            .find(|aggregate| aggregate.station_id == rejections.station_id && aggregate.reason == rejections.reason)
        {
            Some(aggregate) => {
                aggregate.count += rejections.count;
                if rejections.last_at >= aggregate.last_at {
                    aggregate.last_at = rejections.last_at;
                    aggregate.last_detail = rejections.last_detail;
                }
            }
            None => self.rejections.push(rejections),
        }
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::StationDiagRecord;

    #[test]
    fn older_station_diagnostics_default_easyloop_streams() {
        let record: StationDiagRecord =
            serde_json::from_str(r#"{"assignments":[],"arrivals":[],"rejections":[],"pipeline_depths":[]}"#).unwrap();
        assert!(record.order_phases.is_empty());
        assert!(record.presentation_readiness.is_empty());
        assert!(record.inventory_visit_phases.is_empty());
    }
}
