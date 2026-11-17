//! Within `config`, `robots` is the per-robot-type part of a run's settings:
//! what a fleet's machines cost in time (fork transfers, alignment scans)
//! and how they move (speeds, accelerations, turn times).
//! One struct per robot type — VTM and HTM — each `#[serde(default)]`, so a
//! spec states only the figures it wants to move off the default. The VTM and
//! HTM robot figures come from the world's robot entities when they carry
//! aspects (`robots::materialize`), and a spec may state them only for a world
//! whose robot entities carry none. The VTM and HTM structs also refuse a
//! stated figure the simulation cannot use, such as a negative time or a speed
//! of zero ([`InvalidFigure`]), whichever source stated it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::SimDuration;
use crate::pathfinding::{HtmKinematics, KinematicParams};
use crate::robots::vtm::motion::CycleTimes;

/// VTM isolation mode — a diagnostic lever that isolates the VTM subsystem's
/// throughput cost by swapping the geometric per-cycle model in the scheduler
/// for an idealized one, to bracket "how much throughput is the VTM subsystem
/// actually costing us?" without re-modelling the transport.
///
/// All modes keep the demand-pull hook intact (totes still move only when a
/// pick pulls them) and keep the downstream tote/relay state transitions —
/// only the *cycle timing* (and, for `Teleport`, capacity) changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VtmIsolationMode {
    /// Production: full geometric cycle (travel + fork + buffer) per task.
    #[default]
    Off,
    /// Ideal scheduler at the *real* fleet capacity. Every cycle takes a
    /// fixed `ideal_cycle_secs`; the per-VTM `available_at` frontier and the
    /// shared per-zone pool still enforce throughput. Removes only the
    /// schedulable slack (empty repositioning, greedy mismatch, idle) — i.e.
    /// each zone behaves as an M/D/c server with c = VTMs-in-zone.
    IdealRate,
    /// Unconstrained ceiling: a tote arrives at its relay exactly when demand
    /// asks for it. Cycle ≈ 0 and the capacity frontier is ignored (infinite
    /// VTMs). Answers "what if VTM transport were free?".
    Teleport,
}

/// VTM (Vertical Transfer Module) timing parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VtmConfig {
    /// The floor graph's base per-cell speed, m/s: the shared pathfinder's
    /// crossing time for a cell, read by the nearest-robot prescreen that
    /// chooses HTMs, the station fetch bounds, the reservation search's
    /// goal-dwell check and the run's metrics. No DFW6 VTM travels on the
    /// floor graph; a VTM's legs come from the cycle model's chassis and lift
    /// figures below.
    pub speed: f64,
    /// Tote load time in seconds.
    pub load_time_secs: f64,
    /// Tote unload time in seconds.
    pub unload_time_secs: f64,
    /// VTM isolation mode (diagnostic lever; `Off` in production).
    #[serde(alias = "approximation")]
    pub isolation: VtmIsolationMode,
    /// `IdealRate` cycle time, seconds. `None` → derive a fleet-mean forward
    /// cycle from the motion calibration at build time. Ignored unless
    /// `isolation == IdealRate`.
    pub ideal_cycle_secs: Option<f64>,
    /// Fork transfer to take a tote, seconds. A pick takes this long wherever
    /// it happens: the rack on a forward cycle, the relay on a return.
    pub pick_secs: f64,
    /// Fork transfer to place a tote, seconds.
    pub drop_secs: f64,
    /// Level alignment scan, seconds, once per fork insertion. Stands for both
    /// operations unless one of the two below names its own.
    pub alignment_secs: f64,
    /// Alignment scan before a pick, seconds. `None` → `alignment_secs`, so a
    /// spec that states only the scalar still governs both operations.
    pub pick_alignment_secs: Option<f64>,
    /// Alignment scan before a drop, seconds. `None` → `alignment_secs`.
    pub drop_alignment_secs: Option<f64>,
    /// Max chassis speed along the aisle for the cycle model, m/s. Not
    /// [`VtmConfig::speed`], which is the floor graph's base per-cell speed:
    /// it prices floor-graph estimates such as the prescreen that chooses
    /// HTMs, and no DFW6 VTM travels on the floor graph.
    pub chassis_speed: f64,
    /// Chassis acceleration, and equally deceleration, for the cycle model. m/s².
    pub chassis_accel: f64,
    /// Max fork-lift speed for the cycle model, m/s.
    pub lift_speed: f64,
    /// Fork-lift acceleration, and equally deceleration, for the cycle model. m/s².
    pub lift_accel: f64,
    /// Force the number of VTMs registered per relay zone (grow or shrink).
    /// `None` → use the map's per-zone `vtm_count` (default 2). A capacity-
    /// sensitivity lever: e.g. `Some(1)` halves the fleet, `Some(4)` doubles
    /// it, to find where VTM transport stops being the binding constraint.
    /// When growing, extra VTMs reuse existing initial positions in the zone.
    pub per_zone_override: Option<usize>,
}

/// The robot figures are [`CycleTimes::default`]'s placeholder robot.
impl Default for VtmConfig {
    fn default() -> Self {
        let placeholder = CycleTimes::default();
        Self {
            speed: 6.0,
            load_time_secs: 2.25,
            unload_time_secs: 2.25,
            isolation: VtmIsolationMode::Off,
            ideal_cycle_secs: None,
            pick_secs: placeholder.pick_secs,
            drop_secs: placeholder.drop_secs,
            alignment_secs: placeholder.pick_alignment_secs,
            pick_alignment_secs: None,
            drop_alignment_secs: None,
            chassis_speed: placeholder.chassis_speed,
            chassis_accel: placeholder.chassis_accel,
            lift_speed: placeholder.lift_speed,
            lift_accel: placeholder.lift_accel,
            per_zone_override: None,
        }
    }
}

impl VtmConfig {
    /// Bundle the per-operation fork/alignment fields and the chassis/lift
    /// profile for the motion model.
    pub fn cycle_times(&self) -> CycleTimes {
        CycleTimes {
            pick_secs: self.pick_secs,
            drop_secs: self.drop_secs,
            pick_alignment_secs: self.pick_alignment_secs.unwrap_or(self.alignment_secs),
            drop_alignment_secs: self.drop_alignment_secs.unwrap_or(self.alignment_secs),
            chassis_speed: self.chassis_speed,
            chassis_accel: self.chassis_accel,
            lift_speed: self.lift_speed,
            lift_accel: self.lift_accel,
        }
    }

    /// Set the fields [`VtmConfig::cycle_times`] reads, so that it returns
    /// `times`. Both alignments are stated, so the lone `alignment_secs`
    /// stands for neither.
    pub fn set_cycle_times(&mut self, times: CycleTimes) {
        self.pick_secs = times.pick_secs;
        self.drop_secs = times.drop_secs;
        self.pick_alignment_secs = Some(times.pick_alignment_secs);
        self.drop_alignment_secs = Some(times.drop_alignment_secs);
        self.chassis_speed = times.chassis_speed;
        self.chassis_accel = times.chassis_accel;
        self.lift_speed = times.lift_speed;
        self.lift_accel = times.lift_accel;
    }

    /// Refuse a kinematics or split-alignment figure the simulation cannot
    /// use. The cycle model divides by the chassis and lift figures and the
    /// floor graph by `speed`, so each must be above zero; an alignment is a
    /// dwell, so a stated one may be zero. The fork and scalar alignment
    /// figures are not checked here.
    pub fn validate(&self) -> Result<(), InvalidFigure> {
        above_zero("vtm.chassis_speed", self.chassis_speed)?;
        above_zero("vtm.chassis_accel", self.chassis_accel)?;
        above_zero("vtm.lift_speed", self.lift_speed)?;
        above_zero("vtm.lift_accel", self.lift_accel)?;
        above_zero("vtm.speed", self.speed)?;
        zero_or_more("vtm.pick_alignment_secs", self.pick_alignment_secs)?;
        zero_or_more("vtm.drop_alignment_secs", self.drop_alignment_secs)
    }
}

/// Destination ordering for idle HTMs. Charging is a separate future policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleRelaySelection {
    #[default]
    Nearest,
    Random,
}

/// HTM (Horizontal Transfer Module) timing + chassis-kinematics parameters.
///
/// The motion fields are configurable per run, split by which motion model
/// consumes them:
/// - `loaded_speed` / `empty_speed` — **kinematic-mode** max chassis speed (the
///   trapezoidal v_max peak).
/// - `loaded_constant_speed` / `empty_constant_speed` — **ConstantSpeed-mode**
///   flat per-cell cruise, separate from the kinematic v_max (default 6.0 m/s,
///   an optimistic flat baseline that sits above the v_max).
/// - `loaded_accel` / `empty_accel` — kinematic accel (= decel).
/// - `loaded_rotate` / `empty_rotate` — 90° in-place turn (both modes).
/// - `loaded_rotate_180` / `empty_rotate_180` — 180° in-place reversal between
///   cardinal facings, when one is stated; unstated, a reversal is
///   composed from two 90° turns.
///
/// The forks carry two pairs of handoff figures, because the two places an
/// HTM hands a tote over are not the same machine: `pickup_secs` /
/// `deposit_secs` are how long a handoff takes at a relay's diving board, and
/// `conveyor_pickup_secs` / `conveyor_deposit_secs` how long one takes at the
/// fulfillment flow's station belts, each falling back to its relay twin when
/// unstated. docs/walmart-facts.md section 7 records what the belt handoffs
/// take at DFW6.
///
/// Defaults: the robot figures are a placeholder robot, which a run takes when
/// its world's robot entities carry no aspect or when it is assembled without
/// any world, such as a unit test: the simulator's former compiled defaults,
/// not a real robot's figures. The ConstantSpeed cruise is 6.0 m/s,
/// above the kinematic v_max: an optimistic flat baseline, equal to the VTM
/// shared speed. [`HtmConfig::to_kinematics`] bundles them into
/// [`HtmKinematics`]. The struct is `#[serde(default)]`, so any legacy
/// `htm.speed` key in existing JSON is ignored without a migration.
///
/// Speeds are m/s, accelerations m/s², rotation times seconds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HtmConfig {
    /// Idle relay selection. Startup always samples relay/charging cells randomly.
    pub idle_selection: IdleRelaySelection,
    /// Fork pick time at a diving board (tote lift onto the forks).
    #[serde(alias = "load_time_secs")]
    pub pickup_secs: f64,
    /// Fork deposit time at a diving board (tote set-down).
    #[serde(alias = "unload_time_secs")]
    pub deposit_secs: f64,
    /// Fork pick time at a station belt, seconds: how long an HTM takes to
    /// lift a tote off a station's conveyor in the fulfillment flow, both
    /// collecting an inventory tote from an EasyLoop and picking up a carton
    /// at box induction. `None` uses `pickup_secs`.
    pub conveyor_pickup_secs: Option<f64>,
    /// Fork deposit time at a station belt, seconds: how long an HTM takes to
    /// set an inventory tote down on an EasyLoop in the fulfillment flow.
    /// `None` uses `deposit_secs`.
    pub conveyor_deposit_secs: Option<f64>,
    /// Kinematic-mode max chassis speed (trapezoidal v_max), loaded (fork up). m/s.
    pub loaded_speed: f64,
    /// Kinematic-mode max chassis speed (trapezoidal v_max), empty (fork down). m/s.
    pub empty_speed: f64,
    /// ConstantSpeed-mode flat per-cell cruise, loaded — a flat baseline,
    /// separate from the kinematic v_max above (default 6.0). m/s.
    pub loaded_constant_speed: f64,
    /// ConstantSpeed-mode flat per-cell cruise, empty. m/s.
    pub empty_constant_speed: f64,
    /// Chassis acceleration/deceleration when loaded (kinematic mode). m/s².
    pub loaded_accel: f64,
    /// Chassis acceleration/deceleration when empty (kinematic mode). m/s².
    pub empty_accel: f64,
    /// 90° in-place rotation time when loaded. s.
    pub loaded_rotate: f64,
    /// 90° in-place rotation time when empty. s.
    pub empty_rotate: f64,
    /// 180° in-place reversal time when loaded, seconds. Between cardinal
    /// facings the HTM reverses in one rotate that takes this long; between
    /// two off-cardinal headings on a curved link it still makes two quarter
    /// turns. Unstated, every reversal is two 90° turns.
    pub loaded_rotate_180: Option<f64>,
    /// 180° in-place reversal time when empty, seconds, on the same terms.
    pub empty_rotate_180: Option<f64>,
}

impl Default for HtmConfig {
    fn default() -> Self {
        Self {
            idle_selection: IdleRelaySelection::Nearest,
            pickup_secs: 2.0,
            deposit_secs: 2.0,
            conveyor_pickup_secs: None,
            conveyor_deposit_secs: None,
            loaded_speed: 4.2,
            empty_speed: 4.5,
            // ConstantSpeed-mode flat cruise (operational default): 6.0 m/s
            // loaded and empty — the same as the VTM shared speed, and ABOVE
            // the kinematic v_max (4.5), so constant mode is an optimistic flat
            // baseline rather than a ramp-limited effective speed (kinematic
            // mode carries the realistic profile). Override via
            // `--settings htm.{loaded,empty}_constant_speed=…`.
            loaded_constant_speed: 6.0,
            empty_constant_speed: 6.0,
            loaded_accel: 2.0,
            empty_accel: 2.0,
            loaded_rotate: 1.0,
            empty_rotate: 1.0,
            loaded_rotate_180: None,
            empty_rotate_180: None,
        }
    }
}

impl HtmConfig {
    /// Fork pick time at a diving board.
    pub fn pickup_time(&self) -> SimDuration {
        SimDuration::from_secs(self.pickup_secs)
    }

    /// Fork deposit time at a diving board.
    pub fn deposit_time(&self) -> SimDuration {
        SimDuration::from_secs(self.deposit_secs)
    }

    /// Fork pick time at a station belt, falling back to the diving-board
    /// figure when none is stated.
    pub fn conveyor_pickup_time(&self) -> SimDuration {
        SimDuration::from_secs(self.conveyor_pickup_secs.unwrap_or(self.pickup_secs))
    }

    /// Fork deposit time at a station belt; falls back the same way.
    pub fn conveyor_deposit_time(&self) -> SimDuration {
        SimDuration::from_secs(self.conveyor_deposit_secs.unwrap_or(self.deposit_secs))
    }

    /// Refuse a stated belt or turn figure the simulation cannot use. A belt
    /// handoff is a dwell, so it may be zero; a turn that takes no time would
    /// flip the robot's facing with no rotate in its plan, so a 90° turn and a
    /// stated reversal must be above zero. The relay figures are not checked
    /// here.
    pub fn validate(&self) -> Result<(), InvalidFigure> {
        zero_or_more("htm.conveyor_pickup_secs", self.conveyor_pickup_secs)?;
        zero_or_more("htm.conveyor_deposit_secs", self.conveyor_deposit_secs)?;
        above_zero("htm.loaded_rotate", self.loaded_rotate)?;
        above_zero("htm.empty_rotate", self.empty_rotate)?;
        self.loaded_rotate_180.map_or(Ok(()), |secs| above_zero("htm.loaded_rotate_180", secs))?;
        self.empty_rotate_180.map_or(Ok(()), |secs| above_zero("htm.empty_rotate_180", secs))
    }

    /// Bundle the chassis kinematics — speeds, accelerations, and the 90° and
    /// 180° turn figures — into the [`HtmKinematics`] the HTM's drives are
    /// built from.
    /// Manufacturer specs are m/s and m/s²; the search uses mm/s and mm/s², so
    /// speeds/accels are scaled by 1000. Decel matches accel for each load
    /// state. Rotation times convert seconds → ms.
    ///
    /// This is the legacy settings boundary for worlds without robot aspects
    /// and synthetic maps. Catalog models construct their kinematics directly.
    pub fn to_kinematics(&self) -> HtmKinematics {
        HtmKinematics {
            loaded: KinematicParams {
                max_velocity_mm_s: self.loaded_speed * 1000.0,
                acceleration_mm_s2: self.loaded_accel * 1000.0,
                deceleration_mm_s2: self.loaded_accel * 1000.0,
            },
            unloaded: KinematicParams {
                max_velocity_mm_s: self.empty_speed * 1000.0,
                acceleration_mm_s2: self.empty_accel * 1000.0,
                deceleration_mm_s2: self.empty_accel * 1000.0,
            },
            loaded_constant_speed_mm_s: self.loaded_constant_speed * 1000.0,
            empty_constant_speed_mm_s: self.empty_constant_speed * 1000.0,
            loaded_turn: SimDuration::ms((self.loaded_rotate * 1000.0) as u64),
            empty_turn: SimDuration::ms((self.empty_rotate * 1000.0) as u64),
            loaded_reversal: self.loaded_rotate_180.map(SimDuration::from_secs),
            empty_reversal: self.empty_rotate_180.map(SimDuration::from_secs),
        }
    }
}

/// A stated figure the simulation cannot use, such as a negative time or a
/// speed of zero, refused before the run is built. Names the settings key,
/// which a spec's `settings` block and `--settings` spell the same way, so the
/// message points at the figure to fix whichever source stated it.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum InvalidFigure {
    #[error("settings {key} is {value}; a run needs a finite figure above zero")]
    MustBeAboveZero { key: &'static str, value: f64 },
    #[error("settings {key} is {value}; a run needs a finite figure, zero or more")]
    MustBeZeroOrMore { key: &'static str, value: f64 },
}

pub(super) fn above_zero(key: &'static str, value: f64) -> Result<(), InvalidFigure> {
    if value.is_finite() && value > 0.0 { Ok(()) } else { Err(InvalidFigure::MustBeAboveZero { key, value }) }
}

/// A figure a spec may leave unstated; only a stated one is checked.
fn zero_or_more(key: &'static str, value: Option<f64>) -> Result<(), InvalidFigure> {
    match value {
        Some(value) if !(value.is_finite() && value >= 0.0) => Err(InvalidFigure::MustBeZeroOrMore { key, value }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{HtmConfig, InvalidFigure};
    use crate::config::SimConfig;
    use crate::model::SimDuration;

    type Set = fn(&mut SimConfig, f64);

    fn validate_with(set: Set, value: f64) -> Result<(), InvalidFigure> {
        let mut config = SimConfig::default();
        set(&mut config, value);
        config.validate()
    }

    /// Every checked figure is refused under its own key, and a dwell that may
    /// be zero accepts zero.
    #[test]
    fn each_checked_figure_is_refused_under_its_own_key() {
        let above_zero: [(&str, Set); 9] = [
            ("vtm.chassis_speed", |config, value| config.vtm.chassis_speed = value),
            ("vtm.chassis_accel", |config, value| config.vtm.chassis_accel = value),
            ("vtm.lift_speed", |config, value| config.vtm.lift_speed = value),
            ("vtm.lift_accel", |config, value| config.vtm.lift_accel = value),
            ("vtm.speed", |config, value| config.vtm.speed = value),
            ("htm.loaded_rotate", |config, value| config.htm.loaded_rotate = value),
            ("htm.empty_rotate", |config, value| config.htm.empty_rotate = value),
            ("htm.loaded_rotate_180", |config, value| config.htm.loaded_rotate_180 = Some(value)),
            ("htm.empty_rotate_180", |config, value| config.htm.empty_rotate_180 = Some(value)),
        ];
        for (key, set) in above_zero {
            for bad in [0.0, -1.5, f64::NAN, f64::INFINITY] {
                let refused = validate_with(set, bad);
                assert!(
                    matches!(refused, Err(InvalidFigure::MustBeAboveZero { key: named, .. }) if named == key),
                    "{key}={bad}"
                );
            }
        }
        let zero_or_more: [(&str, Set); 4] = [
            ("vtm.pick_alignment_secs", |config, value| config.vtm.pick_alignment_secs = Some(value)),
            ("vtm.drop_alignment_secs", |config, value| config.vtm.drop_alignment_secs = Some(value)),
            ("htm.conveyor_pickup_secs", |config, value| config.htm.conveyor_pickup_secs = Some(value)),
            ("htm.conveyor_deposit_secs", |config, value| config.htm.conveyor_deposit_secs = Some(value)),
        ];
        for (key, set) in zero_or_more {
            for bad in [-1.5, f64::NAN, f64::INFINITY] {
                let refused = validate_with(set, bad);
                assert!(
                    matches!(refused, Err(InvalidFigure::MustBeZeroOrMore { key: named, .. }) if named == key),
                    "{key}={bad}"
                );
            }
            assert_eq!(validate_with(set, 0.0), Ok(()), "{key}=0 is a dwell of nothing");
        }
        assert_eq!(SimConfig::default().validate(), Ok(()), "every compiled default passes");
    }

    /// Unstated, a belt handoff takes its relay twin's time; stated, the belt
    /// figure moves only the handoff it names.
    #[test]
    fn a_belt_figure_falls_back_to_its_relay_twin_and_moves_only_its_own_handoff() {
        let mut htm = HtmConfig::default();
        assert_eq!(htm.conveyor_pickup_time(), htm.pickup_time());
        assert_eq!(htm.conveyor_deposit_time(), htm.deposit_time());
        htm.conveyor_deposit_secs = Some(7.28);
        assert_eq!(htm.conveyor_deposit_time(), SimDuration::ms(7_280));
        assert_eq!(htm.deposit_time(), SimDuration::seconds(2), "the relay drop keeps its own figure");
        assert_eq!(htm.conveyor_pickup_time(), htm.pickup_time(), "the unstated belt pickup still falls back");
    }

    /// Unstated, the bundle carries no reversal and the search composes one
    /// from two 90° turns; stated, the loaded and empty figures travel apart.
    /// They differ here, so a swap between them shows.
    #[test]
    fn stated_reversals_reach_the_kinematics_bundle_loaded_and_empty_apart() {
        let mut htm = HtmConfig::default();
        assert_eq!(htm.to_kinematics().reversal_duration_for(true), None);
        assert_eq!(htm.to_kinematics().reversal_duration_for(false), None);
        htm.loaded_rotate_180 = Some(2.88);
        htm.empty_rotate_180 = Some(2.5);
        let kinematics = htm.to_kinematics();
        assert_eq!(kinematics.reversal_duration_for(true), Some(SimDuration::ms(2_880)));
        assert_eq!(kinematics.reversal_duration_for(false), Some(SimDuration::ms(2_500)));
        assert_eq!(kinematics.turn_duration_for(false), SimDuration::seconds(1), "the 90° figure is untouched");
    }
}
