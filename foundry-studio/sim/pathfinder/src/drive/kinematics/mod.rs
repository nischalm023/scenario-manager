//! Within `drive`, the robot physics the drive models are built from.
//!
//! - [`profile`] — `TrapezoidalProfile` struct + `trapezoidal_profile`,
//!   `trapezoid_position`, `trapezoid_velocity`. The full motion
//!   profile, consumed by Phase 4 visualization rasterization.
//! - [`cost`] — `trapezoidal_cost` (thin wrapper over
//!   `trapezoidal_profile`), which the `Trapezoidal` drive model prices
//!   moves with.
//!
//! The shared types ([`Velocity`], [`KinematicParams`], [`KinematicError`])
//! and the HTM figures a run states ([`HtmKinematics`]) live here.

mod cost;
mod profile;

pub(crate) use cost::trapezoidal_cost;
pub use profile::{TrapezoidalProfile, trapezoid_position, trapezoidal_profile};

use crate::SimDuration;

/// Real velocity in mm/s. Used in NodeMeta for accurate waypoint reconstruction.
/// NOT part of search dedup — that uses SpeedBucket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Velocity(pub f64);

impl Velocity {
    pub const ZERO: Velocity = Velocity(0.0);

    pub fn mm_per_sec(self) -> f64 {
        self.0
    }
}

/// Per-robot kinematic parameters for trapezoidal motion profiles.
#[derive(Clone, Copy, Debug)]
pub struct KinematicParams {
    pub max_velocity_mm_s: f64,
    pub acceleration_mm_s2: f64,
    pub deceleration_mm_s2: f64,
}

/// Per-robot HTM chassis kinematics, bundled so the whole HTM motion profile
/// travels through the pathfinder as one value.
///
/// Holds the loaded/unloaded trapezoidal [`KinematicParams`] (kinematic v_max +
/// accel/decel), the loaded/empty `ConstantSpeed` flat cruise speeds, and the
/// loaded/empty 90° rotation durations, plus the 180° reversal figures a run
/// may state. [`Constant::htm`](crate::Constant::htm) and
/// [`Trapezoidal::htm`](crate::Trapezoidal::htm) build the HTM's drives from
/// them: the constant speeds for `ConstantSpeed`-mode cost, the trapezoidal
/// params for `HtmKinematic`-mode cost, and `turn_duration_for` /
/// `reversal_duration_for` for rotate cost (both modes).
///
/// A catalog model constructs this directly from its own source figures;
/// legacy settings use `HtmConfig::to_kinematics`. A fixed diagnostic measures with
/// `robots::reference_htm`, the M5F read from its model file.
#[derive(Clone, Copy, Debug)]
pub struct HtmKinematics {
    /// Trapezoidal params used while carrying a tote (fork up).
    pub loaded: KinematicParams,
    /// Trapezoidal params used while empty (fork down).
    pub unloaded: KinematicParams,
    /// ConstantSpeed-mode flat per-cell cruise (mm/s) while carrying — distinct
    /// from `loaded.max_velocity_mm_s` (the kinematic v_max peak).
    pub loaded_constant_speed_mm_s: f64,
    /// ConstantSpeed-mode flat per-cell cruise (mm/s) while empty.
    pub empty_constant_speed_mm_s: f64,
    /// 90° in-place rotation duration while carrying a tote.
    pub loaded_turn: SimDuration,
    /// 90° in-place rotation duration while empty.
    pub empty_turn: SimDuration,
    /// 180° in-place reversal while carrying a tote, when the run states one.
    /// `None` leaves a reversal to two 90° turns.
    pub loaded_reversal: Option<SimDuration>,
    /// 180° in-place reversal while empty, when the run states one.
    pub empty_reversal: Option<SimDuration>,
}

impl HtmKinematics {
    /// Uniform per-load HTM profile — no loaded/empty or constant/v_max split.
    /// Used where a single fixed HTM speed is wanted independent of any robot:
    /// geometry/logic tests (round-number traces) and the FPI stable reference
    /// (dessim's `metrics::fpi`).
    pub fn uniform(speed_mm_s: u64, turn: SimDuration) -> Self {
        // accel value is unused in ConstantSpeed mode — just nonzero/valid.
        let params = KinematicParams {
            max_velocity_mm_s: speed_mm_s as f64,
            acceleration_mm_s2: speed_mm_s as f64,
            deceleration_mm_s2: speed_mm_s as f64,
        };
        Self {
            loaded: params,
            unloaded: params,
            loaded_constant_speed_mm_s: speed_mm_s as f64,
            empty_constant_speed_mm_s: speed_mm_s as f64,
            loaded_turn: turn,
            empty_turn: turn,
            loaded_reversal: None,
            empty_reversal: None,
        }
    }

    /// 90° in-place rotation duration for the given load state.
    pub fn turn_duration_for(&self, carrying_cargo: bool) -> SimDuration {
        if carrying_cargo { self.loaded_turn } else { self.empty_turn }
    }

    /// 180° in-place reversal duration for the given load state, when the run
    /// states one. `None` means the search composes a reversal from 90° turns.
    pub fn reversal_duration_for(&self, carrying_cargo: bool) -> Option<SimDuration> {
        if carrying_cargo { self.loaded_reversal } else { self.empty_reversal }
    }

    /// ConstantSpeed-mode flat per-cell cruise (mm/s) for the given load state.
    pub fn constant_speed_mm_s(&self, carrying_cargo: bool) -> f64 {
        if carrying_cargo { self.loaded_constant_speed_mm_s } else { self.empty_constant_speed_mm_s }
    }
}

/// Errors produced by the trapezoidal kinematic computation.
///
/// Bad params (`a == 0`, `dec == 0`, `v_max == 0`) are configuration
/// invariants — they would divide by zero in the profile math, so we
/// surface them as `Result::Err` rather than panicking. The expected
/// callers either take these from validated robot figures or fall back to
/// constant-speed mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KinematicError {
    /// `acceleration_mm_s2 <= 0`.
    NonPositiveAcceleration,
    /// `deceleration_mm_s2 <= 0`.
    NonPositiveDeceleration,
    /// `max_velocity_mm_s <= 0`.
    NonPositiveMaxVelocity,
    /// `entry_v` exceeds `max_velocity_mm_s` — the search would have
    /// to brake immediately, which the upstream cell didn't account
    /// for. Either the predecessor over-budgeted exit velocity or the
    /// param block changed mid-plan; both are bugs.
    EntryExceedsMaxVelocity,
    /// One of the [`KinematicParams`] fields is non-finite (NaN or ±inf).
    /// Configuration bug — the profile math would propagate the
    /// non-finiteness through every subsequent computation. Surfaced
    /// distinctly from `NonPositive*` so the caller can tell "user said
    /// 0 / negative" from "user said NaN / inf".
    NonFiniteParam,
}
