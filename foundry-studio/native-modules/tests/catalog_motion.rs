//! Catalog figures and model choice reach the concrete drives through the same
//! world/spec reader used by Studio and desvis. Legacy worlds remain readable,
//! but Studio refuses their unstated physical model.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use desgraph::types::{CellId, CellKind};
use dessim::SimTime;
use dessim::config::SimConfig;
use dessim::pathfinding::{
    Cached, Constant, DriveModel, MotionModel, Orientation, PathCacheKey, Velocity, fh_sipp_for_map,
};
use dessim::robots::require_catalog_motion;
use dessim::robots::runtime::RuntimeRobots;
use dessim::robots::vtm::model::{AxisMotion, VtmChassis};
use dessim::sim_spec::apply_cli_settings;
use dessim::world_spec;
use dessim::worldmap::{WorldMap, load_from_bytes};
use pathfinder::MotionKind;
use serde_json::{Value, json};

fn document(kind: Option<&str>) -> Value {
    let mut world: Value = serde_json::from_str(include_str!("fixtures/minimal_putaway.json")).unwrap();
    for entity in world["entities"].as_array_mut().unwrap() {
        if let Some(drive) = entity["components"].get_mut("floor_drive") {
            if let Some(kind) = kind {
                drive["kind"] = json!(kind);
                for load in ["loaded", "empty"] {
                    drive[load]["constant_speed_mm_s"] = json!(6000);
                }
            }
        }
    }
    world
}

fn load(document: &Value) -> WorldMap {
    load_from_bytes(&serde_json::to_vec(document).unwrap()).unwrap()
}

fn models(world: &WorldMap, config: &SimConfig) -> RuntimeRobots {
    let zones: Vec<_> = world
        .map
        .zones
        .iter()
        .flat_map(|zone| std::iter::repeat_n(BTreeSet::from([zone.id]), zone.vtms_deployed() as usize))
        .collect();
    world.racking.robot_deployment.resolve(config, world.map.fleet_config().htms, &zones).unwrap()
}

fn htm(document: &mut Value) -> &mut Value {
    document["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entity| entity.pointer("/components/robot_class/role").and_then(Value::as_str) == Some("htm"))
        .unwrap()
        .get_mut("components")
        .unwrap()
}

#[test]
fn catalog_kind_and_speed_price_the_actual_drive_without_settings() {
    let mut doc = document(Some("trapezoidal"));
    let world = load(&doc);
    require_catalog_motion(&world).unwrap();
    let config = SimConfig::default();
    assert_eq!(config.motion_model, MotionModel::ConstantSpeed);
    let robots = models(&world, &config);
    let drive = &robots.htm[0].execution[0];
    assert_eq!(drive.kind(), MotionKind::Kinematic);
    let ramped = drive.kinematic_cost(Velocity::ZERO, Velocity::ZERO, 10_000).0;
    assert_eq!(drive.max_speed(), Velocity(4500.0));

    htm(&mut doc)["floor_drive"]["kind"] = json!("constant");
    let robots = models(&load(&doc), &config);
    let drive = &robots.htm[0].execution[0];
    assert_eq!(drive.kind(), MotionKind::Constant);
    assert_eq!(drive.max_speed(), Velocity(6000.0), "constant mode uses its own catalog cruise");
    let constant = drive.kinematic_cost(Velocity::ZERO, Velocity::ZERO, 10_000).0;
    assert!(constant < ramped, "the chosen model changes executed travel cost");

    htm(&mut doc)["floor_drive"]["empty"]["max_speed_mm_s"] = json!(500);
    let robots = models(&load(&doc), &config);
    assert_eq!(robots.htm[0].execution[0].kinematic_cost(Velocity::ZERO, Velocity::ZERO, 10_000).0, constant);
    htm(&mut doc)["floor_drive"]["empty"]["constant_speed_mm_s"] = json!(500);
    let robots = models(&load(&doc), &config);
    assert_eq!(robots.htm[0].execution[0].kinematic_cost(Velocity::ZERO, Velocity::ZERO, 10_000).0.as_ms(), 20_000);
}

#[test]
fn studio_refuses_legacy_and_incomplete_motion_but_the_legacy_reader_still_works() {
    let legacy = load(&document(None));
    let error = require_catalog_motion(&legacy).unwrap_err();
    assert!(error.contains("floor_drive.kind"), "{error}");
    let config = SimConfig::default();
    assert_eq!(models(&legacy, &config).htm[0].kind, MotionModel::ConstantSpeed);

    let mut doc = document(Some("trapezoidal"));
    htm(&mut doc).as_object_mut().unwrap().remove("floor_drive");
    let error = load_from_bytes(&serde_json::to_vec(&doc).unwrap()).err().unwrap().to_string();
    assert!(error.contains("floor_drive"), "{error}");

    for entity in doc["entities"].as_array_mut().unwrap() {
        let components = entity["components"].as_object_mut().unwrap();
        for aspect in ["lineage", "body", "floor_drive", "rack_ladder", "lift", "end_effector"] {
            components.remove(aspect);
        }
    }
    let error = require_catalog_motion(&load(&doc)).unwrap_err();
    assert!(error.contains("no catalog motion"), "{error}");
}

#[test]
fn unknown_kind_is_refused_instead_of_becoming_constant_speed() {
    let error = load_from_bytes(&serde_json::to_vec(&document(Some("teleport"))).unwrap()).err().unwrap().to_string();
    assert!(error.contains("floor_drive") && error.contains("teleport"), "{error}");
}

#[test]
fn catalog_kind_and_cruise_cannot_be_overridden_in_spec_or_cli() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("spec.json");
    for kind in ["constant", "trapezoidal"] {
        let doc = document(Some(kind));
        fs::write(folder.path().join("world.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
        for (settings, key, value) in [
            (json!({"motion_model": "constant_speed"}), "motion_model", "constant_speed"),
            (json!({"htm": {"loaded_constant_speed": 9}}), "htm.loaded_constant_speed", "9"),
            (json!({"htm": {"empty_constant_speed": 9}}), "htm.empty_constant_speed", "9"),
        ] {
            fs::write(&path, serde_json::to_vec(&json!({"world":"world.json", "settings":settings})).unwrap()).unwrap();
            let error = match world_spec::load(&path) {
                Ok(_) => panic!("catalog override {key} was accepted for {kind}"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains(key), "{error}");
            let world = load(&doc);
            let error = apply_cli_settings(
                &mut SimConfig::default(),
                &[format!("{key}={value}")],
                &world.robot_catalog,
                Path::new("world.json"),
            )
            .unwrap_err();
            assert!(error.to_string().contains(key), "{error}");
        }
    }
}

#[test]
fn constant_models_require_cruise_and_trapezoidal_models_validate_it_only_when_present() {
    for kind in ["constant", "trapezoidal"] {
        for load in ["loaded", "empty"] {
            for value in [None, Some(json!(0)), Some(json!(-1))] {
                let mut doc = document(Some(kind));
                let profile = &mut htm(&mut doc)["floor_drive"][load];
                match value {
                    Some(value) => profile["constant_speed_mm_s"] = value,
                    None => {
                        profile.as_object_mut().unwrap().remove("constant_speed_mm_s");
                    }
                }
                let unstated = profile.get("constant_speed_mm_s").is_none();
                let result = load_from_bytes(&serde_json::to_vec(&doc).unwrap());
                if kind == "trapezoidal" && unstated {
                    assert!(result.is_ok(), "trapezoidal has no constant cruise dependency");
                } else {
                    let error = result.err().unwrap().to_string();
                    assert!(error.contains(&format!("floor_drive.{load}.constant_speed_mm_s")), "{error}");
                }
            }
        }
    }
}

fn same_drive(before: &dyn DriveModel, after: &dyn DriveModel) {
    assert_eq!(before.kind(), after.kind());
    assert_eq!(before.max_speed(), after.max_speed());
    assert_eq!(before.reversal_duration_for(), after.reversal_duration_for());
    for angle in [90_000, 180_000] {
        assert_eq!(before.rotation_cost(angle), after.rotation_cost(angle));
    }
    for distance in [850, 1000, 10_000] {
        assert_eq!(before.min_travel_time(distance), after.min_travel_time(distance));
        for entry in [Velocity::ZERO, Velocity(1000.0)] {
            assert_eq!(
                before.kinematic_cost(entry, Velocity::ZERO, distance),
                after.kinematic_cost(entry, Velocity::ZERO, distance)
            );
        }
    }
}

#[test]
fn catalog_migration_preserves_figures_and_paths_under_the_same_routing_policy() {
    let legacy = load(&document(None));
    let open: Vec<CellId> = legacy
        .map
        .graph
        .cells()
        .filter(|cell| cell.kind == CellKind::Open && cell.world_pos.is_some())
        .map(|cell| cell.id)
        .collect();
    assert!(open.len() > 20);
    for (kind, model) in [("constant", MotionModel::ConstantSpeed), ("trapezoidal", MotionModel::HtmKinematic)] {
        for (loaded, empty) in [(6.0, 6.0), (3.25, 5.75), (4.5, 4.5)] {
            let mut before = SimConfig::default();
            before.motion_model = model;
            before.htm.loaded_constant_speed = loaded;
            before.htm.empty_constant_speed = empty;
            let before_models = models(&legacy, &before);
            assert_eq!(
                (before_models.htm[0].figures.loaded_speed, before_models.htm[0].figures.empty_speed),
                (4.2, 4.5)
            );

            let mut doc = document(Some(kind));
            let drive = &mut htm(&mut doc)["floor_drive"];
            drive["loaded"]["constant_speed_mm_s"] = json!(loaded * 1000.0);
            drive["empty"]["constant_speed_mm_s"] = json!(empty * 1000.0);
            let catalog = load(&doc);
            require_catalog_motion(&catalog).unwrap();
            let after = SimConfig::default();
            let after_models = models(&catalog, &after);
            assert_eq!(before_models.htm[0].kind, after_models.htm[0].kind);

            let before_pathfinder = fh_sipp_for_map(&legacy.map, &before);
            let after_pathfinder = fh_sipp_for_map(&catalog.map, &after);
            let (before_drives, after_drives) = (&before_models.htm[0].execution, &after_models.htm[0].execution);
            let (before_estimates, after_estimates) = (&before_models.htm[0].estimate, &after_models.htm[0].estimate);
            for state in 0..2 {
                same_drive(before_drives[state].as_ref(), after_drives[state].as_ref());
                same_drive(before_estimates[state].as_ref(), after_estimates[state].as_ref());
                assert_eq!(after_estimates[state].max_speed(), Velocity(if state == 0 { 4500.0 } else { 4200.0 }));
                for i in 0..20 {
                    let (from, to) = (open[i], open[open.len() - i - 1]);
                    let original = before_pathfinder
                        .find_path(from, Orientation::East, to, before_estimates[state].as_ref(), SimTime::ZERO)
                        .unwrap();
                    let migrated = after_pathfinder
                        .find_path(from, Orientation::East, to, after_estimates[state].as_ref(), SimTime::ZERO)
                        .unwrap();
                    assert_eq!(original.waypoints, migrated.waypoints);
                    assert_eq!(original.travel_time, migrated.travel_time);
                    assert_eq!(
                        before_pathfinder.kinematic_travel_time(&original, before_drives[state].as_ref()),
                        after_pathfinder.kinematic_travel_time(&migrated, after_drives[state].as_ref())
                    );
                }
            }
        }
    }
}

/// D142 changes only the estimator's policy: legacy cruises remain execution
/// figures, while the catalog maxima price spatial routes in both models.
#[test]
fn route_estimates_use_catalog_maxima_without_retuning_execution() {
    for kind in ["constant", "trapezoidal"] {
        let world = load(&document(Some(kind)));
        let config = SimConfig::default();
        let robots = models(&world, &config);
        let original_config = serde_json::to_value(&config).unwrap();
        let original_execution = &robots.htm[0].execution;
        let figures = robots.htm[0].kinematics;
        let legacy = [Constant::htm(&figures, false), Constant::htm(&figures, true)];
        let corrected = &robots.htm[0].estimate;
        let execution = &robots.htm[0].execution;
        let pathfinder = fh_sipp_for_map(&world.map, &config);
        let cached =
            Cached::from_arc(pathfinder.clone(), robots.htm[0].estimate.clone(), robots.htm[0].execution.clone());
        let open: Vec<CellId> =
            world.map.graph.cells().filter(|cell| cell.kind == CellKind::Open).map(|cell| cell.id).collect();
        assert!(open.len() > 20);
        for state in 0..2 {
            assert_eq!(legacy[state].max_speed(), Velocity(6000.0));
            let maximum = if state == 0 { 4500.0 } else { 4200.0 };
            assert_eq!(corrected[state].max_speed(), Velocity(maximum));
            assert_eq!(corrected[state].min_travel_time(42_000).as_ms(), (42_000_000.0 / maximum) as u64);
            same_drive(original_execution[state].as_ref(), execution[state].as_ref());
            assert_eq!(execution[state].max_speed(), Velocity(if kind == "constant" { 6000.0 } else { maximum }));
            let mut changed = 0;
            for i in 0..20 {
                let (from, to) = (open[i], open[open.len() - i - 1]);
                let before = pathfinder.find_path(from, Orientation::East, to, &legacy[state], SimTime::ZERO).unwrap();
                let after = pathfinder
                    .find_path(from, Orientation::East, to, corrected[state].as_ref(), SimTime::ZERO)
                    .unwrap();
                changed += usize::from(before.travel_time != after.travel_time);
                assert!(after.travel_time >= before.travel_time);
                assert_eq!(
                    before.waypoints.iter().map(|w| (w.cell, w.orientation)).collect::<Vec<_>>(),
                    after.waypoints.iter().map(|w| (w.cell, w.orientation)).collect::<Vec<_>>(),
                    "these fixture routes keep their geometry while estimates change"
                );
                let quote = cached
                    .travel_time(PathCacheKey {
                        from,
                        from_orientation: Orientation::East,
                        to,
                        carrying_cargo: state == 1,
                    })
                    .unwrap();
                let expected = if kind == "constant" {
                    before.travel_time
                } else {
                    pathfinder.kinematic_travel_time(&before, original_execution[state].as_ref())
                };
                assert_eq!(quote, expected, "actual-drive quotes preserve their execution cost in both models");
            }
            assert!(changed > 0, "{kind}, load {state}: route costs must expose the corrected speed");
        }
        assert_eq!(serde_json::to_value(&config).unwrap(), original_config);
    }
}

#[test]
fn a_real_wps_export_supplies_both_fleets_and_the_physical_model() {
    let bytes = include_bytes!("../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json");
    let world = load_from_bytes(bytes).unwrap();
    require_catalog_motion(&world).unwrap();
    assert_eq!(world.map.fleet_config().htms, 24);
    assert_eq!(world.map.vtm_count(), 12);
    let config = SimConfig::default();
    let robots = models(&world, &config);
    assert_eq!(robots.htm.len(), 24);
    assert_eq!(robots.vtm.len(), 12);
    assert_eq!(robots.htm[0].kind, MotionModel::HtmKinematic);
    for drive in &robots.htm[0].execution {
        assert_eq!(drive.kind(), MotionKind::Kinematic);
        assert_eq!(drive.max_speed(), Velocity(4500.0));
        assert_eq!(drive.rotation_cost(90_000).as_ms(), 2100);
    }
    for drive in &robots.htm[0].estimate {
        assert_eq!(drive.kind(), MotionKind::Constant);
        assert_eq!(drive.max_speed(), Velocity(4500.0));
    }
    let htm = &robots.htm[0].figures;
    assert_eq!((htm.loaded_accel, htm.empty_accel), (2.0, 2.0));
    assert_eq!((htm.pickup_secs, htm.deposit_secs), (2.0, 2.0));
    let vtm = &robots.vtm[0];
    let VtmChassis::Ladder(ladder) = vtm.chassis else { panic!("Cencora VTM attaches to the rack") };
    assert_eq!((ladder.max_speed_m_s, ladder.accel_m_s2), (3.0, 2.0));
    assert_eq!(vtm.lift, AxisMotion::Trapezoidal { max_speed_m_s: 3.0, accel_m_s2: 2.0 });
    let handling = vtm.handling;
    assert_eq!(
        (handling.pick_secs, handling.drop_secs, handling.pick_alignment_secs, handling.drop_alignment_secs),
        (6.2, 5.0, 3.0, 3.0)
    );
}

/// The world reader must preserve model counts, not merely accept a manually
/// assembled runtime catalog. Entity order must not change physical bindings.
#[test]
fn world_cohorts_keep_distinct_models_and_counts_in_a_stable_order() {
    let mut doc = document(Some("trapezoidal"));
    let mut other = doc["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity.pointer("/components/robot_class/role").and_then(Value::as_str) == Some("htm"))
        .unwrap()
        .clone();
    let id = "00000000-0000-4000-8000-000000000009";
    other["id"] = json!(id);
    other["name"] = json!("slower second class");
    other["components"]["floor_drive"]["kind"] = json!("constant");
    other["components"]["floor_drive"]["empty"]["constant_speed_mm_s"] = json!(1000);
    other["components"]["end_effector"]["pick_secs"] = json!(7.0);
    doc["entities"].as_array_mut().unwrap().extend([
        other,
        json!({"id":"00000000-0000-4000-8000-000000000010", "components":{"cohort":{"robot_class":id,"count":2,"assignment":{"kind":"free"}}}}),
    ]);
    let world = load(&doc);
    require_catalog_motion(&world).unwrap();
    assert_eq!(world.map.fleet_config().htms, 6);
    let robots = models(&world, &SimConfig::default());
    assert_eq!(robots.htm.len(), 6);
    assert_eq!(robots.htm.iter().filter(|model| model.source.as_ref().unwrap().to_string() == id).count(), 2);
    assert_eq!(robots.htm[0].figures.pickup_secs, 7.0);
    assert_eq!(robots.htm[0].execution[0].max_speed(), Velocity(1000.0));
    assert_eq!(robots.htm[2].execution[0].max_speed(), Velocity(4500.0));
    let before = robots.htm.iter().map(|model| (model.id, model.source.clone())).collect::<Vec<_>>();
    doc["entities"].as_array_mut().unwrap().reverse();
    let reversed = models(&load(&doc), &SimConfig::default());
    assert_eq!(before, reversed.htm.iter().map(|model| (model.id, model.source.clone())).collect::<Vec<_>>());
}
