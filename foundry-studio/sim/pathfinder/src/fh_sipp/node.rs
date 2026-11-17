//! Per-state metadata for the SIPP-A\* search graph.
//!
//! Each accepted search state records a [`NodeMeta`] that captures the
//! action that produced it from its predecessor (kind, start, end, source).
//! `reconstruct_waypoints` reads `NodeMeta` directly to emit `WaypointAction`
//! variants — no g-delta arithmetic, no inference.

use super::state::SafeIntervalId;
use crate::{SimTime, Velocity};
use desgraph::types::CellId;

/// What action produced a state from its predecessor in the A\* search tree.
///
/// `Wait` is the explicit event-driven wait successor (spec §4 + FHSippPathfinder
/// draft §Event-Driven Wait Successors). Gap-fill waits at Move/Rotate
/// seams are NOT recorded as `Wait` here — reconstruction emits them from
/// `action_start vs prior_action_end` gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NodeActionKind {
    /// Only for the start state; metadata records `PathStart.time`.
    Start,
    Move,
    Rotate,
    /// Explicit event-driven wait at `state.cell`. `action_start =
    /// predecessor's action-ready time`, `action_end = event_time`.
    Wait,
}

/// Per-state metadata read by `reconstruct_waypoints` to emit
/// `WaypointAction` variants directly — no g-delta arithmetic, no inference.
#[derive(Clone, Copy, Debug)]
pub(super) struct NodeMeta {
    pub action_kind: NodeActionKind,
    /// Action's start timestamp (absolute SimTime).
    pub action_start: SimTime,
    /// Action's end timestamp (= g_score in SimTime form).
    pub action_end: SimTime,
    /// For Move actions: the source cell. None for Start / Rotate / Wait.
    pub source_cell: Option<CellId>,
    /// Safe interval admitting this state's reservation on `state.cell`.
    /// Same as `SippState.interval`; duplicated for direct access at
    /// reconstruction.
    pub interval: SafeIntervalId,
    /// Real velocity at cell entry (mm/s). In `MotionModel::ConstantSpeed`
    /// mode this is always `Velocity::ZERO` — the field is unused on that
    /// path and reconstruction substitutes the configured cruise speed.
    /// In `MotionModel::HtmKinematic` mode this is the trapezoidal-profile
    /// entry velocity from `kinematics::trapezoidal_cost`, which may
    /// legitimately be zero for rest-to-rest profiles.
    pub entry_velocity: Velocity,
    /// Real velocity at cell exit (mm/s). Same `ConstantSpeed`-vs-`HtmKinematic`
    /// semantics as `entry_velocity`: zero is a sentinel under ConstantSpeed
    /// and a real physically-meaningful endpoint under HtmKinematic.
    pub exit_velocity: Velocity,
}
