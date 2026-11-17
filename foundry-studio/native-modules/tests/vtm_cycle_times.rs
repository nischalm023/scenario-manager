//! DA50 cycle times come from the world's VTM robot entity, keyed by
//! operation, per the spec sheet.

use dessim::config::VtmConfig;
use dessim::robots::vtm::model::VtmMotion;
use dessim::robots::vtm::motion::{self, CycleTimes};

mod common;

/// DA50 spec sheet: row 1 Pick SD 6.2, row 2 Drop SD 5.0, row 8 Alignment 3.0;
/// the chassis and lift from Quicktron's A5 efficiency calculation tool. The
/// Cencora world's VTM entity states them all, from the A5 robot model file.
const SPEC: CycleTimes = CycleTimes {
    pick_secs: 6.2,
    drop_secs: 5.0,
    pick_alignment_secs: 3.0,
    drop_alignment_secs: 3.0,
    chassis_speed: 3.0,
    chassis_accel: 2.0,
    lift_speed: 3.0,
    lift_accel: 2.0,
};

/// The relay height of the zone these tests travel: the relay stands 0.39 m up.
const RELAY_HEIGHT_M: f64 = 0.39;

/// The tote height these tests travel to: the tote stands 5 m up.
const SHELF_HEIGHT_M: f64 = 5.0;

#[test]
fn fixture_matches_the_da50_spec_sheet() {
    let (world, _scenario, _config) = common::load_world_spec("cencora_v3_spec.json");
    assert!(!world.robot_catalog.vtm.is_empty());
    for model in world.robot_catalog.vtm.values() {
        assert_eq!(*model, VtmMotion::from(SPEC));
    }
    // T-06: (6.2 + 3) + (5 + 3) = 17.2 s fixed per cycle, excluding travel.
    assert!((SPEC.fixed_cycle_secs() - 17.2).abs() < 1e-9);
}

/// A run assembled without any world gets a placeholder robot: the
/// simulator's former compiled defaults, not a real robot's figures.
#[test]
fn defaults_are_the_placeholder_robot() {
    let d = CycleTimes::default();
    assert_eq!(d.pick_secs, 4.0);
    assert_eq!(d.drop_secs, 2.0);
    assert_eq!(d.pick_alignment_secs, 1.933);
    assert_eq!(d.drop_alignment_secs, 1.933);
    assert_eq!(d.chassis_speed, 3.0);
    assert_eq!(d.chassis_accel, 2.0);
    assert_eq!(d.lift_speed, 3.0);
    assert_eq!(d.lift_accel, 2.0);
    assert_eq!(VtmConfig::default().cycle_times(), d, "the settings' default is the same placeholder");
}

/// A pick costs `pick_secs` wherever it happens. Forward picks at the rack,
/// return picks at the relay, so the endpoint carrying the pick time swaps with
/// direction. Endpoint-keyed input got this wrong for every return cycle.
#[test]
fn pick_time_follows_the_operation_not_the_endpoint() {
    let fwd = motion::forward_cycle_with_heights(10.0, 10.0, SHELF_HEIGHT_M, RELAY_HEIGHT_M, SPEC);
    let ret = motion::return_cycle_with_heights(10.0, 10.0, SHELF_HEIGHT_M, RELAY_HEIGHT_M, SPEC);

    assert_eq!(fwd.rack_transfer_secs, 6.2, "forward picks at the rack");
    assert_eq!(fwd.relay_transfer_secs, 5.0, "forward drops at the relay");
    assert_eq!(ret.relay_transfer_secs, 6.2, "return picks at the relay");
    assert_eq!(ret.rack_transfer_secs, 5.0, "return drops at the rack");

    for (label, c) in [("forward", &fwd), ("return", &ret)] {
        assert_eq!(c.rack_alignment_secs, 3.0, "{label}: alignment at every fork insertion");
        assert_eq!(c.relay_alignment_secs, 3.0, "{label}");
        assert!((c.total_secs - c.travel_secs - 17.2).abs() < 1e-9, "{label}");
    }
    assert_eq!(fwd.travel_secs, ret.travel_secs, "travel is unaffected");
}

/// A run that states a slower chassis pays for it in travel and nowhere else:
/// handling at the two endpoints is a fixed cost per fork insertion, so halving
/// the speed and the acceleration has to leave it alone. This is what makes the
/// knob safe to hand an operator — it cannot silently retime the DA50's forks.
#[test]
fn a_slower_chassis_lengthens_travel_and_leaves_handling_alone() {
    let slow = CycleTimes { chassis_speed: SPEC.chassis_speed / 2.0, chassis_accel: SPEC.chassis_accel / 2.0, ..SPEC };

    // Ground-level slot, so the horizontal legs decide the travel outright.
    let fast_cycle = motion::forward_cycle_with_heights(10.0, 10.0, 0.0, RELAY_HEIGHT_M, SPEC);
    let slow_cycle = motion::forward_cycle_with_heights(10.0, 10.0, 0.0, RELAY_HEIGHT_M, slow);

    assert!(
        slow_cycle.first_travel_secs > fast_cycle.first_travel_secs,
        "first leg {:.3}s should exceed {:.3}s",
        slow_cycle.first_travel_secs,
        fast_cycle.first_travel_secs
    );
    assert!(
        slow_cycle.second_travel_secs > fast_cycle.second_travel_secs,
        "second leg {:.3}s should exceed {:.3}s",
        slow_cycle.second_travel_secs,
        fast_cycle.second_travel_secs
    );

    for (label, c) in [("fast", &fast_cycle), ("slow", &slow_cycle)] {
        assert_eq!(c.rack_alignment_secs, 3.0, "{label}");
        assert_eq!(c.relay_alignment_secs, 3.0, "{label}");
        assert_eq!(c.rack_transfer_secs, 6.2, "{label}");
        assert_eq!(c.relay_transfer_secs, 5.0, "{label}");
        assert!((c.total_secs - c.travel_secs - 17.2).abs() < 1e-9, "{label}");
    }
}

/// Vinay's 2026-09-21 directive gives 3.56 s to align for a pick and 2.3 s to
/// align for a drop. The endpoint where each one happens swaps with the
/// direction of the cycle, so a run that states the pair must see it swap.
#[test]
fn alignment_follows_the_operation_when_pick_and_drop_differ() {
    let walmart = CycleTimes { pick_alignment_secs: 3.56, drop_alignment_secs: 2.3, ..SPEC };

    let fwd = motion::forward_cycle_with_heights(10.0, 10.0, SHELF_HEIGHT_M, RELAY_HEIGHT_M, walmart);
    let ret = motion::return_cycle_with_heights(10.0, 10.0, SHELF_HEIGHT_M, RELAY_HEIGHT_M, walmart);

    assert_eq!(fwd.rack_alignment_secs, 3.56, "forward picks at the rack");
    assert_eq!(fwd.relay_alignment_secs, 2.3, "forward drops at the relay");
    assert_eq!(ret.relay_alignment_secs, 3.56, "return picks at the relay");
    assert_eq!(ret.rack_alignment_secs, 2.3, "return drops at the rack");

    // One pick and one drop per cycle either way, so the fixed cost is the
    // same in both directions and the travel is untouched.
    let fixed = 6.2 + 5.0 + 3.56 + 2.3;
    assert!((walmart.fixed_cycle_secs() - fixed).abs() < 1e-9);
    for (label, c) in [("forward", &fwd), ("return", &ret)] {
        assert!((c.total_secs - c.travel_secs - fixed).abs() < 1e-9, "{label}");
    }
    assert_eq!(fwd.travel_secs, ret.travel_secs, "travel is unaffected");
}

/// A spec that states only the scalar still governs both operations: that is
/// what keeps every existing map's cycle exactly what it was.
#[test]
fn a_lone_alignment_scalar_still_governs_both_operations() {
    let scalar_only = VtmConfig { alignment_secs: 5.5, ..VtmConfig::default() };
    let times = scalar_only.cycle_times();
    assert_eq!(times.pick_alignment_secs, 5.5);
    assert_eq!(times.drop_alignment_secs, 5.5);

    let pick_stated = VtmConfig { pick_alignment_secs: Some(3.56), ..scalar_only };
    let times = pick_stated.cycle_times();
    assert_eq!(times.pick_alignment_secs, 3.56, "a stated pick figure wins");
    assert_eq!(times.drop_alignment_secs, 5.5, "an unstated drop figure still falls back");
}

/// The lift axis is its own pair of figures. A tall slot over a short aisle is
/// decided by the vertical legs, so halving the lift figures lengthens travel
/// there while halving the chassis figures does not -- the one arrangement that
/// tells a wiring mistake (the vertical axis reading the chassis pair) from a
/// correct one.
#[test]
fn a_slower_lift_lengthens_a_tall_retrieval_and_a_slower_chassis_does_not() {
    let slow_lift = CycleTimes { lift_speed: SPEC.lift_speed / 2.0, lift_accel: SPEC.lift_accel / 2.0, ..SPEC };
    let slow_chassis =
        CycleTimes { chassis_speed: SPEC.chassis_speed / 2.0, chassis_accel: SPEC.chassis_accel / 2.0, ..SPEC };

    // One metre of aisle, nine metres of rack: the lift decides both legs.
    let fast = motion::forward_cycle_with_heights(1.0, 1.0, 9.0, RELAY_HEIGHT_M, SPEC);
    let lift = motion::forward_cycle_with_heights(1.0, 1.0, 9.0, RELAY_HEIGHT_M, slow_lift);
    let chassis = motion::forward_cycle_with_heights(1.0, 1.0, 9.0, RELAY_HEIGHT_M, slow_chassis);

    assert!(lift.first_travel_secs > fast.first_travel_secs, "{:.3} vs {:.3}", lift.first_travel_secs, fast.first_travel_secs);
    assert!(lift.second_travel_secs > fast.second_travel_secs, "{:.3} vs {:.3}", lift.second_travel_secs, fast.second_travel_secs);
    assert_eq!(chassis.first_travel_secs, fast.first_travel_secs, "a one-metre aisle leg never decides a nine-metre lift");
    assert_eq!(chassis.second_travel_secs, fast.second_travel_secs);
    for c in [&fast, &lift, &chassis] {
        assert!((c.total_secs - c.travel_secs - 17.2).abs() < 1e-9, "handling is untouched by either axis");
    }
}
