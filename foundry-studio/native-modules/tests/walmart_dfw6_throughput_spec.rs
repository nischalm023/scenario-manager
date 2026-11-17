//! Checks that every figure the Walmart DFW6 throughput run takes from
//! docs/walmart-facts.md reaches it at the value the facts file gives, reading
//! it back from the concrete types a run uses after loading the spec the way a
//! run does. The robot figures come from the robot entities of the spec's
//! world, walmart_dfw6.json, which carry the components of the robot model
//! files in `tests/fixtures/robots/`; the station figures come from the spec.
//!
//! The config structs take `#[serde(default)]` and ignore keys they do not
//! know, which the specs' `$comment` entries rely on, so a misspelled or
//! renamed key would fall back to a compiled default while the run still
//! reported numbers. Each assertion names the facts row it pins.

mod common;

use dessim::composition::InductionTaskConfig;
use dessim::pathfinding::MotionModel;
use dessim::robots::runtime::RuntimeRobots;
use dessim::robots::vtm::model::{AxisMotion, VtmChassis};
use dessim::{SimConfig, SimDuration};
use native_modules as _;
use std::collections::BTreeSet;

/// Every order released at the start, on the 2026-09-21 bot specifications.
const THROUGHPUT: &str = "walmart_dfw6_throughput_spec.json";

fn throughput() -> SimConfig {
    let (_world, _scenario, config) = common::load_world_spec(THROUGHPUT);
    config
}

fn robots() -> RuntimeRobots {
    let (world, _, config) = common::load_world_spec(THROUGHPUT);
    let assignments = world
        .map
        .zones
        .iter()
        .flat_map(|zone| std::iter::repeat_n(BTreeSet::from([zone.id]), zone.vtms_deployed() as usize))
        .collect::<Vec<_>>();
    world.racking.robot_deployment.resolve(&config, config.num_htms, &assignments).unwrap()
}

#[test]
fn the_throughput_run_takes_the_directive_vtm_figures_from_the_world() {
    let robots = robots();
    let model = &robots.vtm[0];
    let cycle = model.handling;
    let expected = AxisMotion::Trapezoidal { max_speed_m_s: 1.5, accel_m_s2: 0.5 };
    let VtmChassis::Floor(chassis) = model.chassis else { panic!("Walmart uses floor-driving VTMs") };
    assert_eq!(chassis.empty, expected);
    assert_eq!(chassis.loaded, expected);
    assert_eq!(model.lift, expected);
    assert_eq!(cycle.pick_secs, 4.0, "section 6b 'VTM pick time' 4.0 s");
    assert_eq!(cycle.drop_secs, 4.0, "section 6b 'VTM drop time' 4.0 s");
    assert_eq!(cycle.pick_alignment_secs, 3.56, "section 6b 'VTM pick alignment time' 3.56 s");
    assert_eq!(cycle.drop_alignment_secs, 2.3, "section 6b 'VTM drop alignment time' 2.3 s");
}

#[test]
fn the_throughput_run_takes_the_directive_htm_figures_from_the_world() {
    let config = throughput();
    assert!(
        matches!(config.motion_model, MotionModel::HtmKinematic),
        "the HTM speed and accel rows govern a kinematic run"
    );
    let robots = robots();
    let htm = &robots.htm[0];
    let kinematics = htm.kinematics;
    for (load, params) in [("loaded", kinematics.loaded), ("empty", kinematics.unloaded)] {
        assert_eq!(params.max_velocity_mm_s, 4_000.0, "section 6b 'HTM speed (loaded = unloaded)' 4.0 m/s, {load}");
        assert_eq!(params.acceleration_mm_s2, 1_800.0, "section 6b 'HTM accel (loaded = unloaded)' 1.8 m/s², {load}");
        assert_eq!(params.deceleration_mm_s2, 1_800.0, "section 6b 'HTM accel (loaded = unloaded)', {load} braking");
    }
    for (load, carrying) in [("loaded", true), ("empty", false)] {
        let turn = kinematics.turn_duration_for(carrying);
        assert_eq!(turn, SimDuration::ms(2_160), "section 6b 'HTM 90 degree turn (loaded = unloaded)' 2.16 s, {load}");
        let reversal = kinematics.reversal_duration_for(carrying);
        let pinned = "section 6b 'HTM 180 degree turn (loaded = unloaded)' 2.88 s";
        assert_eq!(reversal, Some(SimDuration::ms(2_880)), "{pinned}, {load}");
    }
    assert_eq!(htm.figures.pickup_time(), SimDuration::seconds(2), "section 6b 'HTM pick time' 2.0 s, at a relay");
    assert_eq!(htm.figures.deposit_time(), SimDuration::seconds(2), "section 6b 'HTM drop time' 2.0 s, at a relay");
    assert_eq!(
        htm.figures.conveyor_pickup_time(),
        SimDuration::ms(4_000),
        "section 6b 'HTM pick time' 2.0 s + 'HTM lift, min to cantilever height 1' 2.0 s, at a station belt"
    );
    assert_eq!(
        htm.figures.conveyor_deposit_time(),
        SimDuration::ms(7_280),
        "section 6b 'HTM drop time' 2.0 s + 'HTM lower, max to min' 5.28 s, at a station belt"
    );
    config.validate().expect("the simulation can use every figure the run takes");
}

#[test]
fn the_throughput_spec_sets_the_station_operator_figures() {
    let config = throughput();
    assert_eq!(config.operator.time_per_pick_secs, 10.0, "section 7 'Operator time, EasyLoop station' 10 s");
    let definition = config.composition.resolve().expect("the spec's composition resolves");
    let induction = definition.tasks.iter().find(|task| task.name() == "PutawayInduction").expect("putaway induction");
    let induction: InductionTaskConfig =
        serde_json::from_value(induction.config.clone()).expect("induction parameters");
    assert_eq!(
        induction.max_presentations_per_hour,
        Some(60.0),
        "section 7 'Operator time, induction station' 60 s: one presentation a minute at each station"
    );
}

/// Three directive figures equal the placeholder robot's defaults, so the
/// values the tests above read back cannot tell a figure the world gives from a
/// default one. The world gives both fleets' figures, so no default reaches the
/// run.
#[test]
fn the_world_gives_both_fleets_their_figures() {
    let (world, _scenario, _config) = common::load_world_spec(THROUGHPUT);
    assert!(!world.robot_catalog.htm.is_empty(), "the HTM fleet's figures come from the world");
    assert!(!world.robot_catalog.vtm.is_empty(), "the VTM fleet's figures come from the world");
}
