//! Search-state identity for SIPP-A\*.
//!
//! - [`SafeIntervalId`] is the canonical identity of a free interval on a
//!   cell's reservation timeline. Stable under different `after` query
//!   parameters (otherwise SIPP dedup breaks).
//! - [`SippState`] is the ordinary SIPP identity: `(cell, orientation,
//!   interval)`.
//! - [`SpeedBucket`] is the speed a state is reached at, in 100 mm/s steps:
//!   SIPP-IP's dedup axis, so arrivals at different speeds stay distinct.
//! - [`SearchKey`] wraps `SippState` with an optional `wait_label` to keep
//!   event-driven wait successors distinguishable from earliest-arrival
//!   states. See spec §4 + FHSippPathfinder draft §Search State.

use super::super::Orientation;
use crate::{SimTime, Velocity};
use desgraph::types::CellId;

/// Canonical identity of a safe interval on a cell's reservation timeline.
///
/// `(free_start, free_end)`. Trailing open interval has `free_end = None`.
/// Identity must NOT be clipped by query-time `after` parameter — otherwise
/// two queries from different `after` values would produce different IDs for
/// the same physical interval, breaking SIPP dedup.
///
/// Sentinel `(SimTime::ZERO, None)` is used outside the reserved-path A\*
/// (e.g., spatial-only `find_path`) to mean "no temporal constraint" — all
/// states at a `(cell, orient)` collapse to a single entry.
pub type SafeIntervalId = (SimTime, Option<SimTime>);

/// Ordinary SIPP identity. Dedup axis is `(cell, orientation, interval)`.
///
/// `interval` is the canonical safe interval that admits this state's
/// reservation on `cell`. Different intervals at the same `(cell, orient)`
/// are different nodes (reached via different physical time-paths, with
/// different successor sets).
///
/// Search dedup is on [`SearchKey`], not on `SippState` directly — so
/// event-driven wait states with the same `SippState` but different
/// `wait_label` are kept distinct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct SippState {
    pub cell: CellId,
    pub orientation: Orientation,
    pub interval: SafeIntervalId,
    /// SIPP-IP velocity dedup axis. Two arrivals at the same `(cell,
    /// orientation, interval)` with different speed buckets are distinct
    /// nodes — required for SIPP-IP completeness (Ali & Yakovlev 2023).
    pub speed_bucket: SpeedBucket,
}

/// Full search-state dedup key.
///
/// `wait_label = None` for ordinary SIPP states (earliest-arrival dominance
/// applies). `wait_label = Some(event_time)` for explicit event-driven wait
/// successors, preserving a small finite set of later-departure states that
/// immediate-neighbor interval enumeration cannot synthesize on its own.
///
/// See spec §4 "Wait successor (event-driven)" and FHSippPathfinder draft
/// §Event-Driven Wait Successors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct SearchKey {
    pub sipp: SippState,
    pub wait_label: Option<SimTime>,
}

impl SearchKey {
    /// Construct an ordinary (non-wait-labelled) key for a SIPP state.
    pub fn ordinary(sipp: SippState) -> Self {
        SearchKey { sipp, wait_label: None }
    }

    pub fn cell(&self) -> CellId {
        self.sipp.cell
    }

    pub fn orientation(&self) -> Orientation {
        self.sipp.orientation
    }

    pub fn interval(&self) -> SafeIntervalId {
        self.sipp.interval
    }
}

/// Discretized velocity for search-state dedup. Fixed 10 cm/s (100 mm/s) buckets.
/// Two arrivals at the same (cell, orient, interval) with different speed buckets
/// are distinct search nodes — required for SIPP-IP completeness (Ali & Yakovlev 2023).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpeedBucket(u32);

const SPEED_BUCKET_SIZE_MM_S: u64 = 100; // 10 cm/s = 100 mm/s

impl SpeedBucket {
    pub const ZERO: SpeedBucket = SpeedBucket(0);

    pub fn from_velocity(v: Velocity) -> Self {
        SpeedBucket((v.0 / SPEED_BUCKET_SIZE_MM_S as f64).floor() as u32)
    }

    pub fn to_velocity(self) -> Velocity {
        Velocity(self.0 as f64 * SPEED_BUCKET_SIZE_MM_S as f64)
    }
}
