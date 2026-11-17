//! The population that moves through a world: robot classes and the cohorts
//! that mint robots, carrier pools and the carriers they mint.
//!
//! A cohort and a carrier pool are generators in exactly the sense a tiled
//! region is (SPEC §3's table): they declare a count and a policy, and
//! expansion turns that into individually addressable entities, so a
//! curated pre-fill or one damaged robot is speakable without a second
//! mechanism. What each generated entity carries is described here alongside
//! what generated it.

use crate::ids::EntityId;
use crate::units::Steps;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A kind of robot, distinguished by *configuration* as well as model: two VTMs
/// on one chassis with different end effectors are two classes, because what
/// they can carry differs.
///
/// Who made it and who rebranded it are not here: a rebrand is a party and a
/// role, and it rides in `lineage` (SPEC §10.1).
///
/// Deprecated. A robot entity is to describe itself by one component per aspect
/// (how it drives, how it lifts, what it takes a carrier with), because a type
/// is the set of components an entity carries, never a tag inside one. Which
/// fleet a cohort joins is then read from its assignment, and `role` goes when
/// `cell.traversable_by`, which lists roles, gives way to comparing a cell's
/// clearance with the robot's height.
#[deprecated(note = "describe a robot by one component per aspect; `role` goes with `cell.traversable_by`")]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct RobotClass {
    pub role: String,
    /// How the class gets about: `floor` drives on cell_grids, `ladder` climbs
    /// the racking and needs no cells beneath it. Absent reads as `floor` — see
    /// SPEC-GAPS; the ladder case is the exception and the exception is the one
    /// that has to be declared.
    #[serde(default)]
    pub locomotion: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub configuration: Option<String>,
    #[serde(default)]
    pub carries: Vec<String>,
    #[serde(default)]
    pub footprint: Option<[Steps; 2]>,
    /// Checked against the lowest level of any rack whose footprint a cell_grid
    /// admitting this class crosses (SPEC §7).
    #[serde(default)]
    pub height: Option<Steps>,
    #[serde(default)]
    pub native_frame: Option<EntityId>,
}

/// N robots of one class plus an assignment policy; a generator of `robot:{i}`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Cohort {
    pub robot_class: EntityId,
    pub count: usize,
    #[serde(default)]
    pub assignment: Option<Assignment>,
}

/// Where a cohort's robots may work. `hard` means they never leave the listed
/// entities; anything else leaves them free.
///
/// The other view expansion re-serializes, onto every robot a cohort mints, and
/// it guards its unstated field for the same reason [`Placement`] guards its
/// absent optional. A cohort that confines its robots to nothing writes no `to`
/// at all; an empty list and an absent one both read back as no entities, so
/// writing `"to": []` onto every robot would put a spelling in the expanded form
/// that the source never had and the next implementation has no reason to
/// reproduce.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Assignment {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<EntityId>,
}

#[allow(deprecated)]
impl RobotClass {
    /// Whether this class needs cells under it.
    pub fn is_floor(&self) -> bool {
        self.locomotion.as_deref().unwrap_or("floor") == "floor"
    }
}

impl Assignment {
    pub fn is_hard(&self) -> bool {
        self.kind == "hard"
    }
}

/// A population of carriers of one type with a permanent designation and a
/// placement rule; a generator of `carrier:{i}`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct CarrierPool {
    pub carrier_type: EntityId,
    /// Permanent per carrier — the anti-starvation ruling — so it belongs to the
    /// individual carrier, not to its type.
    pub designation: String,
    pub count: usize,
    #[serde(default)]
    pub placement: Option<Placement>,
}

/// Where a pool's carriers start.
///
/// This is one of the two typed views expansion writes back out as JSON — every
/// carrier a pool mints carries a re-serialized copy of it — so an absent
/// optional here must serialize to *nothing*, never to an explicit null. SPEC §9
/// holds two implementations to the canonical projection of the expanded form,
/// and a null nobody wrote is a byte the other implementation has no reason to
/// produce.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Placement {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sector: Option<EntityId>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// One robot, as a cohort mints it. This shape began as this implementation's
/// reading of a gap and is a SPEC §10 row since v0.3 (SPEC-GAPS 10).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Robot {
    pub robot_class: EntityId,
    pub cohort: EntityId,
    pub index: usize,
    #[serde(default)]
    pub assignment: Option<Assignment>,
}

/// One carrier, as a pool mints it. As with [`Robot`], a SPEC §10 row since
/// v0.3 (SPEC-GAPS 10).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Carrier {
    pub carrier_type: EntityId,
    pub pool: EntityId,
    pub index: usize,
    pub designation: String,
    #[serde(default)]
    pub placement: Option<Placement>,
}
