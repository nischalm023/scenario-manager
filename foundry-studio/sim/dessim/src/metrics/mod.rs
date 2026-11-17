//! Metrics reporting and profile serialization.
//!
//! Events live in `crate::events`. Observer (including output types,
//! time slices, and robots' work as intervals) lives in `crate::observer`.

pub mod descriptor;
pub mod fpi;
pub mod profile;
pub mod reporting;
pub mod streams;
pub mod worldview;

use serde::{Deserialize, Serialize};

// Re-export events from their new home for backward compatibility.
pub use crate::events::SimEvent;
pub use crate::events::{
    ArrivalErrorEvent, CycleCompleteEvent, DispatchTrigger, HtmDispatchEvent, InitialPositionsEvent, NavEventData,
    OrderCompleteEvent, OrderDispatchEvent, PipelineSnapshotEvent, ResourceEvent, ResourceEventKind,
    WatchdogTimeoutEvent,
};

// ---------------------------------------------------------------------------
// DetailedMetrics (legacy — will be replaced by ScalarCounters)
// ---------------------------------------------------------------------------

/// Serializable metrics snapshot included in SimResult and PerfProfile.
///
/// TODO: Replace with observer's `ScalarCounters` which is a strict superset.
/// Currently populated as Default (zeros) — the observer's cumulative_scalars
/// has the real data, accessible via MetricsOutput after the sim completes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DetailedMetrics {
    pub pathfinder_queries: u64,
    pub pathfinder_failures: u64,
}
