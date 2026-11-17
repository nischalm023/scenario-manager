//! # dessim
//!
//! Discrete-event simulation library for warehouse throughput analysis.
//!
//! The primary entry point is [`run_simulation`], which accepts a [`SimRequest`]
//! describing the warehouse layout, scenario parameters, and simulation config,
//! and returns a [`SimResult`] with aggregate metrics.

// Let derive macros reference `::dessim::...` paths from inside this crate.
extern crate self as dessim;

pub mod composition;
pub mod config;
pub mod error;
pub mod events;
pub mod harness;
pub mod metrics;
pub mod model;
pub mod observer;
pub mod pathfinding;
pub mod prelude;
pub mod reporting;
pub mod runner;
pub mod scenario;
pub mod scenario_gen;
pub mod scenario_loader;
pub mod scheduler;
pub mod signals;
pub mod sim_spec;
pub mod sim_types;
pub mod simtime;
pub mod state;
pub mod task;
pub mod utils;
pub mod world_spec;
pub mod worldmap;

pub use dessim_task_macros::task;
pub use dessim_trace_macros::trace;
pub use sim_types::{Counters, RunFlags, SimResult, StopReason, StopSignal};

pub use config::{
    FidelityMode, HtmConfig, OperatorConfig, OrderConfig, OrderStrategy, PickSequencing, RelayConfig, SimConfig,
    TraceConfig, TraceFormat, TraceWindow, VtmConfig, VtmScoringConfig,
};

pub use pathfinding::{
    Cached, FHSippPathfinder, HtmKinematics, MotionModel, Orientation, Path, PathCacheKey, Pathfinder, Waypoint,
    WaypointAction,
};

pub use scenario::{
    BulkSourceScenario, CsvScenario, GeneratedScenario, InlineOrder, InlineOrderLine, InlineScenario, InlineSku,
    ScenarioConfig,
};
pub use scenario_gen::{DemandPool, DistributionCenterId, ScenarioSourceKind, ScenarioSummary};
pub use sim_spec::{SLICE_WIDTH_MS, SimSpec, apply_cli_settings, apply_fleet, apply_scenario_settings};
pub use simtime::{SimDuration, SimOffset, SimTime, TimeRange, TimeRangeError};

pub mod robots;
#[cfg(feature = "trace")]
pub(crate) mod trace_output;
#[cfg(feature = "perf")]
pub(crate) mod tracing_agg;

#[cfg(feature = "trace")]
pub const TRACE_RECORD_TARGET: &str = "dessim::trace";

// The exported macros expand in the crate that invokes them, which need not
// depend on `tracing` itself.
#[cfg(feature = "trace")]
#[doc(hidden)]
pub use tracing as __tracing;

#[cfg(feature = "trace")]
#[macro_export]
macro_rules! trace_span {
    ($name:ident) => {
        let _span = $crate::__tracing::info_span!(::core::stringify!($name)).entered();
    };
    ($name:expr) => {
        let _span = $crate::__tracing::info_span!($name).entered();
    };
}
#[cfg(not(feature = "trace"))]
#[macro_export]
macro_rules! trace_span {
    ($name:ident) => {};
    ($name:expr) => {};
}

#[cfg(feature = "trace")]
#[macro_export]
macro_rules! trace_event {
    ($kind:ident, $name:ident $(, $field:ident)* $(,)?) => {
        $crate::__tracing::info!(
            target: $crate::TRACE_RECORD_TARGET,
            trace_kind = ::core::stringify!($kind),
            trace_name = ::core::stringify!($name)
            $(, $field = %$field)*
        );
    };
    ($kind:ident, $name:ident $(, $field:ident = $value:expr)* $(,)?) => {
        $crate::__tracing::info!(
            target: $crate::TRACE_RECORD_TARGET,
            trace_kind = ::core::stringify!($kind),
            trace_name = ::core::stringify!($name)
            $(, $field = %$value)*
        );
    };
}
// Disabled, the fields still have to COUNT as used, or every value a call
// site computes purely to trace warns as unused in the default build. The
// closure captures them and is never called, so nothing is evaluated —
// a plain `let _ = &value` would run the call sites that pass one.
#[cfg(not(feature = "trace"))]
#[macro_export]
macro_rules! trace_event {
    ($kind:ident, $name:ident $(, $field:ident)* $(,)?) => {{
        let _ = || { $( let _ = &$field; )* };
    }};
    ($kind:ident, $name:ident $(, $field:ident = $value:expr)* $(,)?) => {{
        let _ = || { $( let _ = &$value; )* };
    }};
}

#[macro_export]
macro_rules! perf_span {
    ($name:expr) => {
        $crate::trace_span!($name);
    };
}

#[cfg(feature = "debug-logs")]
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        ::std::eprintln!($($arg)*);
    };
}
#[cfg(not(feature = "debug-logs"))]
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        if false {
            ::std::eprintln!($($arg)*);
        }
    }};
}
