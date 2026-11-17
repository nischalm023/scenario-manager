//! Immutable motion and handling for each deployed robot. Catalog classes
//! share one model between their physical instances; a model's identity stays
//! with its routes, reservations and playback. Only `legacy` constructs a
//! fleet from run settings for synthetic maps and older worlds.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use site_model::ids::EntityId;

use super::aspects::FloorDriveKind;
use super::figures::HtmFigures;
use super::vtm::model::VtmMotion;
use crate::SimDuration;
use crate::config::SimConfig;
use crate::pathfinding::{Constant, DriveModel, HtmKinematics, KinematicParams, MotionModel, Trapezoidal};

/// Stable within a run: catalog ids are assigned in sorted source-id order.
/// Zero belongs exclusively to the legacy settings model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RobotModelId(pub u32);

pub struct FloorRobotModel {
    pub id: RobotModelId,
    pub source: Option<EntityId>,
    pub execution: [Arc<dyn DriveModel>; 2],
    pub estimate: [Arc<dyn DriveModel>; 2],
    pub kinematics: HtmKinematics,
    pub kind: MotionModel,
    pub figures: HtmFigures,
}

impl fmt::Debug for FloorRobotModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FloorRobotModel")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("kind", &self.kind)
            .field("figures", &self.figures)
            .finish()
    }
}

impl FloorRobotModel {
    /// Resolve only absent legacy selectors from settings. Stated catalog
    /// figures construct the drives directly, without a fleet config overlay.
    pub fn from_catalog(id: RobotModelId, source: EntityId, figures: HtmFigures, legacy: &SimConfig) -> Self {
        let kind = match figures.kind {
            Some(FloorDriveKind::Constant) => MotionModel::ConstantSpeed,
            Some(FloorDriveKind::Trapezoidal) => MotionModel::HtmKinematic,
            None => legacy.motion_model,
        };
        let cruise = |stated: Option<f64>, maximum: f64, fallback: f64| {
            stated.unwrap_or(if figures.kind.is_none() { fallback } else { maximum })
        };
        let kinematics = figures.kinematics(
            cruise(figures.loaded_constant_speed, figures.loaded_speed, legacy.htm.loaded_constant_speed),
            cruise(figures.empty_constant_speed, figures.empty_speed, legacy.htm.empty_constant_speed),
        );
        Self::new(id, Some(source), figures, kind, kinematics)
    }

    pub fn legacy(config: &SimConfig) -> Self {
        Self::new(
            RobotModelId(0),
            None,
            HtmFigures::from_legacy(&config.htm),
            config.motion_model,
            config.htm.to_kinematics(),
        )
    }

    fn new(
        id: RobotModelId,
        source: Option<EntityId>,
        figures: HtmFigures,
        kind: MotionModel,
        kinematics: HtmKinematics,
    ) -> Self {
        // Search remains constant-speed at this model's maximum (D142).
        // Reservations use its separately selected physical execution model.
        let search = HtmKinematics {
            loaded_constant_speed_mm_s: kinematics.loaded.max_velocity_mm_s,
            empty_constant_speed_mm_s: kinematics.unloaded.max_velocity_mm_s,
            ..kinematics
        };
        let estimate: [Arc<dyn DriveModel>; 2] =
            [Arc::new(Constant::htm(&search, false)), Arc::new(Constant::htm(&search, true))];
        let execution: [Arc<dyn DriveModel>; 2] = match kind {
            MotionModel::ConstantSpeed => {
                [Arc::new(Constant::htm(&kinematics, false)), Arc::new(Constant::htm(&kinematics, true))]
            }
            MotionModel::HtmKinematic => {
                [Arc::new(Trapezoidal::htm(&kinematics, false)), Arc::new(Trapezoidal::htm(&kinematics, true))]
            }
        };
        Self { id, source, execution, estimate, kinematics, kind, figures }
    }
}

impl HtmFigures {
    pub fn kinematics(&self, loaded_cruise_m_s: f64, empty_cruise_m_s: f64) -> HtmKinematics {
        let params = |speed, accel| KinematicParams {
            max_velocity_mm_s: speed * 1000.0,
            acceleration_mm_s2: accel * 1000.0,
            deceleration_mm_s2: accel * 1000.0,
        };
        HtmKinematics {
            loaded: params(self.loaded_speed, self.loaded_accel),
            unloaded: params(self.empty_speed, self.empty_accel),
            loaded_constant_speed_mm_s: loaded_cruise_m_s * 1000.0,
            empty_constant_speed_mm_s: empty_cruise_m_s * 1000.0,
            loaded_turn: SimDuration::ms((self.loaded_rotate * 1000.0) as u64),
            empty_turn: SimDuration::ms((self.empty_rotate * 1000.0) as u64),
            loaded_reversal: self.loaded_rotate_180.map(SimDuration::from_secs),
            empty_reversal: self.empty_rotate_180.map(SimDuration::from_secs),
        }
    }

    pub fn pickup_time(&self) -> SimDuration {
        SimDuration::from_secs(self.pickup_secs)
    }
    pub fn deposit_time(&self) -> SimDuration {
        SimDuration::from_secs(self.deposit_secs)
    }
    pub fn conveyor_pickup_time(&self) -> SimDuration {
        SimDuration::from_secs(self.conveyor_pickup_secs.unwrap_or(self.pickup_secs))
    }
    pub fn conveyor_deposit_time(&self) -> SimDuration {
        SimDuration::from_secs(self.conveyor_deposit_secs.unwrap_or(self.deposit_secs))
    }
}

/// Index bindings follow ScenarioData's physical initial-position vectors.
/// A deserialized scenario has no usable bindings until its world is rebuilt;
/// lookup refuses missing entries rather than substituting a legacy model.
#[derive(Clone, Debug, Default)]
pub struct RuntimeRobots {
    pub htm: Vec<Arc<FloorRobotModel>>,
    pub vtm: Vec<Arc<VtmMotion>>,
    pub vtm_sources: Vec<Option<EntityId>>,
}

impl RuntimeRobots {
    pub fn legacy(config: &SimConfig, htm_count: usize, vtm_count: usize) -> Self {
        Self {
            htm: vec![Arc::new(FloorRobotModel::legacy(config)); htm_count],
            vtm: vec![Arc::new(VtmMotion::from(config.vtm.cycle_times())); vtm_count],
            vtm_sources: vec![None; vtm_count],
        }
    }

    pub fn model_for_htm_index(&self, index: usize) -> Result<Arc<FloorRobotModel>, String> {
        self.htm.get(index).cloned().ok_or_else(|| format!("HTM {index} has no resolved robot model"))
    }

    pub fn vtm_for_robot(&self, index: usize) -> Result<Arc<VtmMotion>, String> {
        self.vtm.get(index).cloned().ok_or_else(|| format!("VTM {index} has no resolved robot model"))
    }

    pub fn unique_htm_models(&self) -> Vec<Arc<FloorRobotModel>> {
        // ordered: candidate evaluation must not depend on hash iteration.
        self.htm.iter().map(|model| (model.id, model.clone())).collect::<BTreeMap<_, _>>().into_values().collect()
    }

    pub fn validate_counts(&self, htm_count: usize, vtm_count: usize) -> Result<(), String> {
        if self.htm.len() != htm_count || self.vtm.len() != vtm_count || self.vtm_sources.len() != vtm_count {
            return Err(format!(
                "robot model bindings do not match physical fleet: HTM {}/{htm_count}, VTM {}/{vtm_count}, VTM sources {}",
                self.htm.len(),
                self.vtm.len(),
                self.vtm_sources.len()
            ));
        }
        let mut identities = FxHashMap::default();
        for model in &self.htm {
            if let Some(previous) = identities.insert(model.id, model) {
                if !Arc::ptr_eq(previous, model) {
                    return Err(format!("robot model {:?} names different immutable definitions", model.id));
                }
            }
        }
        Ok(())
    }
}
