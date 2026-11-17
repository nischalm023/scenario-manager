//! Within `robots`, an aspect is one of the components a robot model file gives
//! a robot entity, one per part of the machine
//! (`sim/dessim/tests/fixtures/robots/README.md`). The simulator reads figures
//! from four of them, whose structs are here: how the robot drives on the
//! floor, how it rides the racking, how it lifts, and what it takes a carrier
//! with. Each struct keeps its units in its field names and ignores the fields
//! it does not read, `$comment` among them. The other two, `lineage` and
//! `body`, have no struct because nothing reads them.

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// The name of every aspect. An entity that carries any of them describes a
/// robot, so the fleet it feeds takes its figures from the world.
pub const COMPONENTS: [&str; 6] = ["lineage", "body", "floor_drive", "rack_ladder", "lift", "end_effector"];

/// An aspect the simulator reads figures from, tied to the component name an
/// entity carries it under.
pub trait Aspect: DeserializeOwned {
    const NAME: &'static str;
}

/// A top speed and the acceleration that reaches it. A stated deceleration
/// that differs from the acceleration is refused when the figures are read,
/// because every consumer holds one figure for both.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Profile {
    pub max_speed_mm_s: f64,
    pub accel_mm_s2: f64,
    #[serde(default)]
    pub decel_mm_s2: Option<f64>,
}

/// The physical model a catalog floor drive states. These names mirror the
/// app-owned vocabulary; the pathfinder receives the concrete DriveModel, never
/// selects one from a robot's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorDriveKind {
    Constant,
    Trapezoidal,
}

/// Driving on the floor, with a carrier aboard and without.
#[derive(Clone, Debug, Deserialize)]
pub struct FloorDrive {
    /// Absent only in older world files, whose run settings still select the
    /// model. Studio requires this to be stated by the catalog item.
    #[serde(default)]
    pub kind: Option<FloorDriveKind>,
    pub loaded: Drive,
    pub empty: Drive,
}

/// How a robot drives in one load state, and how long it takes to turn in
/// place: a quarter turn, and a reversal made as one action when a source
/// states one.
#[derive(Clone, Debug, Deserialize)]
pub struct Drive {
    #[serde(flatten)]
    pub profile: Profile,
    /// Constant-mode execution cruise, independent of the catalog maximum
    /// used by spatial estimates. Constant models must state it; trapezoidal
    /// models do not use it. Older worlds may use their legacy run setting.
    #[serde(default)]
    pub constant_speed_mm_s: Option<f64>,
    #[serde(default)]
    pub quarter_turn_secs: Option<f64>,
    #[serde(default)]
    pub reversal_secs: Option<f64>,
}

/// Riding the racking along one aisle.
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
pub struct RackLadder {
    pub profile: Profile,
}

/// Raising and lowering what holds the carrier: a speed profile, timed strokes
/// between named heights, or both.
#[derive(Clone, Debug, Deserialize)]
pub struct Lift {
    #[serde(default)]
    pub profile: Option<Profile>,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
}

/// One timed lift or lower. A stroke may leave out where it starts.
#[derive(Clone, Debug, Deserialize)]
pub struct Stroke {
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
    pub secs: f64,
}

/// The part that takes and places a carrier.
#[derive(Clone, Debug, Deserialize)]
pub struct EndEffector {
    #[serde(default)]
    pub kind: Option<String>,
    pub pick_secs: f64,
    pub drop_secs: f64,
    #[serde(default)]
    pub align_before_pick_secs: Option<f64>,
    #[serde(default)]
    pub align_before_drop_secs: Option<f64>,
    #[serde(default)]
    pub reach_slots_deep: Option<u32>,
}

impl Aspect for FloorDrive {
    const NAME: &'static str = "floor_drive";
}

impl Aspect for RackLadder {
    const NAME: &'static str = "rack_ladder";
}

impl Aspect for Lift {
    const NAME: &'static str = "lift";
}

impl Aspect for EndEffector {
    const NAME: &'static str = "end_effector";
}
