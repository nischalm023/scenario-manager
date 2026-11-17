//! Full-run interval extraction for the inspector's per-resource timeline
//! strip. Descendant of `station_diag`, so it reads the store's private
//! record vecs directly. Pure queries over the whole run (not cursor-gated) —
//! the strip shows `[0, max_ms]` at once. Callers cache the result per
//! (resource, store-generation) since extraction over thousands of intervals
//! is not free to redo every frame.

use desgraph::StationId;
use dessim::events::VtmIdleReasonKind;
use dessim::model::SimTime;
use dessim::observer::activity::ActivityKind;

use super::StationDiagStore;

/// One stretch `[start, end)` of what a bot was doing, for the strip: an
/// interval of work, or a gap. `idle_reason` is set only for a VTM's gap
/// whose reason was recorded (annotates the gap with why the VTM was idle).
pub struct ActivitySpan {
    pub start: SimTime,
    pub end: SimTime,
    pub kind: ActivityKind,
    pub idle_reason: Option<VtmIdleReasonKind>,
}

/// A station serving span `[start, end)` (one completed cycle's dwell).
pub struct ServingSpan {
    pub start: SimTime,
    pub end: SimTime,
}

/// A relay reserved span `[start, end)` reconstructed from reserve/free
/// transitions. An open-ended final reservation runs to `run_end`.
pub struct ReservedSpan {
    pub start: SimTime,
    pub end: SimTime,
}

impl StationDiagStore {
    /// What `robot_id` was doing across the run: the observer's intervals,
    /// with every stretch none of them covers filled in as idle. A VTM's idle
    /// stretch carries the most recent idle reason recorded at or before it
    /// begins. Nothing at all when the recording carries no activity, which
    /// would otherwise read as a bot idle for the whole run.
    pub fn activity_spans(&self, robot_id: u32, run_end: SimTime) -> Vec<ActivitySpan> {
        if !self.records_activity() {
            return Vec::new();
        }
        let work: Vec<&_> = self.activity.iter().filter(|record| record.robot_id == robot_id).collect();
        let mut spans = Vec::with_capacity(work.len() * 2);
        let mut cursor = SimTime::ZERO;
        for record in &work {
            if record.start > cursor {
                spans.push(ActivitySpan {
                    start: cursor,
                    end: record.start,
                    kind: ActivityKind::Gap,
                    idle_reason: self.vtm_idle_reason_at(robot_id, cursor),
                });
            }
            if record.end > record.start {
                spans.push(ActivitySpan { start: record.start, end: record.end, kind: record.kind, idle_reason: None });
            }
            cursor = cursor.max(record.end);
        }
        if run_end > cursor {
            spans.push(ActivitySpan {
                start: cursor,
                end: run_end,
                kind: ActivityKind::Gap,
                idle_reason: self.vtm_idle_reason_at(robot_id, cursor),
            });
        }
        spans
    }

    /// The most recent VTM idle-reason kind recorded at or before `at` for
    /// `robot_id`. Used to annotate a VTM's gap.
    fn vtm_idle_reason_at(&self, robot_id: u32, at: SimTime) -> Option<VtmIdleReasonKind> {
        let mut reason = None;
        for record in &self.vtm_idle_reasons {
            if record.at > at {
                break;
            }
            if record.robot_id == robot_id {
                reason = Some(record.reason);
            }
        }
        reason
    }

    /// Serving spans (cycle dwell windows) for `station` across the whole run.
    pub fn station_serving_spans(&self, station: StationId) -> Vec<ServingSpan> {
        self.station_cycles
            .iter()
            .filter(|record| record.station_id == station)
            .map(|record| ServingSpan { start: record.start, end: record.start + record.dwell })
            .collect()
    }

    /// Rejection occurrence markers for `station` across the whole run, as
    /// `(at, count)` pairs (one per per-slice aggregate). The strip paints
    /// these as tick marks in the idle gaps.
    pub fn station_rejection_marks(&self, station: StationId) -> Vec<(SimTime, u64)> {
        self.rejections
            .iter()
            .filter(|aggregate| aggregate.station_id == station)
            .map(|aggregate| (aggregate.last_at, aggregate.count))
            .collect()
    }

    /// Reserved spans for a relay HTM-access `cell`, reconstructed from the
    /// relay's reserve/free pickup activity. The store does not carry the raw
    /// relay_state transitions (those live per-slice on `SliceRecord`); the
    /// inspector reconstructs reserved spans there and uses this only for the
    /// pickup-wait ticks. Returns pickup-wait `(at, wait_ms)` marks.
    pub fn relay_pickup_marks(&self, cell: u32) -> Vec<(SimTime, u64)> {
        self.htm_relay_pickups
            .iter()
            .filter(|record| record.relay_cell == cell)
            .map(|record| (record.at, record.wait_ms))
            .collect()
    }
}

impl ActivitySpan {
    /// Whether the pointer time falls within this interval.
    pub fn covers(&self, at: SimTime) -> bool {
        self.start <= at && at < self.end
    }
}

impl ServingSpan {
    pub fn covers(&self, at: SimTime) -> bool {
        self.start <= at && at < self.end
    }
}

impl ReservedSpan {
    pub fn covers(&self, at: SimTime) -> bool {
        self.start <= at && at < self.end
    }
}
