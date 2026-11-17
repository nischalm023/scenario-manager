//! Robot definitions and deployment counts read from a world. Source ids stay
//! attached through scenario generation; each catalog class materializes once
//! and its physical instances share that model. Only a wholly absent aspect
//! set uses legacy settings, and ambiguous mixed-model count overrides refuse.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use desgraph::{RobotType, ZoneId};
use site_model::entity::Entity;
use site_model::ids::EntityId;

use super::aspects::COMPONENTS;
use super::figures::{HtmFigures, fleet_of_figure_key};
use super::materialize::{RobotError, htm_figures, vtm_figures};
use super::runtime::{FloorRobotModel, RobotModelId, RuntimeRobots};
use super::vtm::model::VtmMotion;
use crate::config::SimConfig;

#[derive(Clone, Debug, Default)]
pub struct RobotCatalog {
    // ordered: source order assigns stable runtime model ids.
    pub htm: BTreeMap<EntityId, HtmFigures>,
    // ordered: deterministic source traversal and diagnostics.
    pub vtm: BTreeMap<EntityId, VtmMotion>,
}

impl RobotCatalog {
    pub fn read(htm: &[&Entity], vtm: &[&Entity]) -> Result<Self, RobotError> {
        fn models<T>(
            entities: &[&Entity],
            read: fn(&Entity) -> Result<T, RobotError>,
        ) -> Result<BTreeMap<EntityId, T>, RobotError> {
            if !entities.iter().any(|entity| COMPONENTS.iter().any(|name| entity.has(name))) {
                return Ok(BTreeMap::new());
            }
            entities.iter().map(|entity| Ok((entity.id.clone(), read(entity)?))).collect()
        }
        Ok(Self { htm: models(htm, htm_figures)?, vtm: models(vtm, vtm_figures)? })
    }

    /// A settings override must not rewrite any deployed catalog definition.
    /// Missing selectors in pre-catalog worlds retain their explicit legacy
    /// compatibility behavior; absence in a trapezoidal model does not make
    /// constant cruise a runtime dependency.
    pub fn states(&self, key: &str) -> bool {
        match key {
            "motion_model" => return self.htm.values().any(|model| model.kind.is_some()),
            "htm.loaded_constant_speed" => {
                return self.htm.values().any(|model| model.kind.is_some() || model.loaded_constant_speed.is_some());
            }
            "htm.empty_constant_speed" => {
                return self.htm.values().any(|model| model.kind.is_some() || model.empty_constant_speed.is_some());
            }
            _ => {}
        }
        match fleet_of_figure_key(key) {
            Some(RobotType::Htm) => !self.htm.is_empty(),
            Some(RobotType::Vtm) => !self.vtm.is_empty(),
            _ => false,
        }
    }
}

/// Counts retain the class each cohort instantiates. Sorted maps make
/// bindings independent of entity declaration and hash iteration order.
#[derive(Clone, Debug, Default)]
pub struct RobotDeployment {
    pub catalog: Arc<RobotCatalog>,
    // ordered: source order binds physical instances deterministically.
    pub htm: BTreeMap<EntityId, usize>,
    // ordered: zone and source order determine physical VTM bindings.
    pub vtm: BTreeMap<ZoneId, BTreeMap<EntityId, usize>>,
}

impl RobotDeployment {
    pub fn resolve(
        &self,
        config: &SimConfig,
        htm_count: usize,
        vtm_zones: &[BTreeSet<ZoneId>],
    ) -> Result<RuntimeRobots, String> {
        let mut robots = RuntimeRobots::legacy(config, htm_count, vtm_zones.len());
        if !self.catalog.htm.is_empty() {
            let definitions: BTreeMap<_, _> = self
                .catalog
                .htm
                .iter()
                .enumerate()
                .map(|(i, (source, figures))| {
                    (
                        source.clone(),
                        Arc::new(FloorRobotModel::from_catalog(
                            RobotModelId(i as u32 + 1),
                            source.clone(),
                            figures.clone(),
                            config,
                        )),
                    )
                })
                .collect();
            robots.htm = expand(&self.htm, htm_count, "HTM fleet")?
                .iter()
                .map(|source| {
                    definitions.get(source).cloned().ok_or_else(|| format!("HTM catalog has no model for {source}"))
                })
                .collect::<Result<_, _>>()?;
        }
        if !self.catalog.vtm.is_empty() {
            let definitions: BTreeMap<_, _> =
                self.catalog.vtm.iter().map(|(source, model)| (source.clone(), Arc::new(*model))).collect();
            // ordered: deterministic zone binding and refusal order.
            let mut indices: BTreeMap<ZoneId, Vec<usize>> = BTreeMap::new();
            for (index, zones) in vtm_zones.iter().enumerate() {
                if zones.len() != 1 {
                    return Err(format!("VTM {index} must have exactly one assigned zone to bind its catalog model"));
                }
                indices.entry(*zones.first().expect("one checked zone")).or_default().push(index);
            }
            for (zone, indices) in indices {
                let counts = self.vtm.get(&zone).ok_or_else(|| format!("zone {zone:?} has no VTM cohort binding"))?;
                let sources = expand(counts, indices.len(), &format!("zone {zone:?}"))?;
                for (index, source) in indices.into_iter().zip(sources) {
                    robots.vtm[index] = definitions
                        .get(&source)
                        .cloned()
                        .ok_or_else(|| format!("VTM catalog has no model for {source}"))?;
                    robots.vtm_sources[index] = Some(source);
                }
            }
        }
        robots.validate_counts(htm_count, vtm_zones.len())?;
        Ok(robots)
    }
}

fn expand(counts: &BTreeMap<EntityId, usize>, actual: usize, fleet: &str) -> Result<Vec<EntityId>, String> {
    let active: Vec<_> = counts.iter().filter(|(_, count)| **count > 0).collect();
    let declared: usize = active.iter().map(|(_, count)| **count).sum();
    if actual != declared {
        if let [(source, _)] = active.as_slice() {
            return Ok(vec![(*source).clone(); actual]);
        }
        return Err(format!(
            "{fleet} declares {declared} robots across {} models but the run requests {actual}; state per-model counts instead of a total-only override",
            active.len()
        ));
    }
    Ok(active.into_iter().flat_map(|(source, count)| std::iter::repeat_n(source.clone(), *count)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::{MotionModel, Velocity};
    use crate::robots::aspects::FloorDriveKind;
    use crate::robots::vtm::model::{AxisMotion, FloorVtmMotionModel, VtmChassis};

    fn source(n: u32) -> EntityId {
        format!("00000000-0000-4000-8000-{n:012}").parse().unwrap()
    }

    fn deployment() -> RobotDeployment {
        let config = SimConfig::default();
        let mut slow = HtmFigures::from_legacy(&config.htm);
        slow.kind = Some(FloorDriveKind::Constant);
        slow.empty_constant_speed = Some(1.0);
        slow.pickup_secs = 7.0;
        let mut fast = slow.clone();
        fast.kind = Some(FloorDriveKind::Trapezoidal);
        fast.empty_speed = 4.0;
        fast.empty_accel = 2.0;
        fast.pickup_secs = 2.0;
        let ladder = VtmMotion::from(config.vtm.cycle_times());
        let mut floor = ladder;
        floor.chassis = VtmChassis::Floor(FloorVtmMotionModel {
            empty: AxisMotion::Constant { speed_m_s: 1.0 },
            loaded: AxisMotion::Constant { speed_m_s: 0.5 },
        });
        RobotDeployment {
            catalog: Arc::new(RobotCatalog {
                htm: BTreeMap::from([(source(2), fast), (source(1), slow)]),
                vtm: BTreeMap::from([(source(3), ladder), (source(4), floor)]),
            }),
            htm: BTreeMap::from([(source(2), 1), (source(1), 2)]),
            vtm: BTreeMap::from([(ZoneId(1), BTreeMap::from([(source(4), 1), (source(3), 1)]))]),
        }
    }

    #[test]
    fn each_instance_keeps_its_model_handling_and_catalog_identity() {
        let config = SimConfig::default();
        let before = serde_json::to_value(&config).unwrap();
        let zones = [BTreeSet::from([ZoneId(1)]), BTreeSet::from([ZoneId(1)])];
        let robots = deployment().resolve(&config, 3, &zones).unwrap();
        assert!(Arc::ptr_eq(&robots.htm[0], &robots.htm[1]), "one model shared by two physical robots");
        assert!(!Arc::ptr_eq(&robots.htm[0], &robots.htm[2]));
        assert_eq!(
            robots.htm.iter().map(|model| model.source.clone()).collect::<Vec<_>>(),
            vec![Some(source(1)), Some(source(1)), Some(source(2))]
        );
        assert_eq!(robots.htm[0].kind, MotionModel::ConstantSpeed);
        assert_eq!(robots.htm[2].kind, MotionModel::HtmKinematic);
        assert_eq!(robots.htm[0].execution[0].max_speed(), Velocity(1000.0));
        assert_eq!(robots.htm[2].execution[0].max_speed(), Velocity(4000.0));
        assert_eq!(robots.htm[0].figures.pickup_secs, 7.0);
        assert_eq!(robots.htm[2].figures.pickup_secs, 2.0);
        assert_eq!(
            robots.unique_htm_models().iter().map(|model| model.id).collect::<Vec<_>>(),
            vec![RobotModelId(1), RobotModelId(2)]
        );
        assert!(matches!(robots.vtm[0].chassis, VtmChassis::Ladder(_)));
        assert!(matches!(robots.vtm[1].chassis, VtmChassis::Floor(_)));
        assert_eq!(robots.vtm_sources, vec![Some(source(3)), Some(source(4))]);
        assert_eq!(serde_json::to_value(config).unwrap(), before, "catalog never rewrites run settings");
    }

    #[test]
    fn mixed_model_count_overrides_refuse_while_single_model_scaling_is_explicit() {
        let mut deployment = deployment();
        let config = SimConfig::default();
        assert!(deployment.resolve(&config, 4, &[]).unwrap_err().contains("per-model counts"));
        let zones = [BTreeSet::from([ZoneId(1)])];
        assert!(deployment.resolve(&config, 3, &zones).unwrap_err().contains("per-model counts"));
        deployment.htm.remove(&source(2));
        deployment.vtm.get_mut(&ZoneId(1)).unwrap().remove(&source(4));
        let robots = deployment.resolve(&config, 5, &zones).unwrap();
        assert_eq!(robots.htm.len(), 5);
        assert!(robots.htm.iter().all(|model| model.source == Some(source(1))));
        assert_eq!(robots.vtm_sources, vec![Some(source(3))]);
    }

    #[test]
    fn absent_bindings_and_missing_catalog_models_are_not_settings_fallbacks() {
        let mut deployment = deployment();
        deployment.htm.insert(source(9), 1);
        assert!(deployment.resolve(&SimConfig::default(), 4, &[]).unwrap_err().contains("no model"));
        let robots = RuntimeRobots::default();
        assert!(robots.model_for_htm_index(0).is_err());
        assert!(robots.vtm_for_robot(0).is_err());
        assert!(robots.validate_counts(1, 0).is_err());
        let mut deserialized = RuntimeRobots::legacy(&SimConfig::default(), 1, 1);
        deserialized.vtm_sources.clear();
        assert!(deserialized.validate_counts(1, 1).is_err());
    }
}
