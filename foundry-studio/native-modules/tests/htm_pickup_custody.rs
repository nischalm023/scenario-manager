//! Real Cencora and Walmart relay pickups transfer physical and visual tote
//! custody only when the fork finishes, without shortening the relay hold.

mod common;

use std::sync::{Arc, mpsc};

use dessim::composition;
use dessim::events::SimEvent;
use dessim::model::bot::plan::Leg;
use dessim::model::relay::RelayService;
use dessim::model::{BotKind, BotService, InventoryService, ToteLocation};
use dessim::observer::time_slice::ToteVisualState;
use dessim::runner::resolve_scenario_with_racking;
use dessim::{FidelityMode, ScenarioConfig, SimDuration};
use native_modules as _;

fn check_pickups(fixture: &str) {
    let (world, mut scenario, mut config) = common::load_world_spec(fixture);
    if fixture == "minimal_pick_only_spec.json" {
        let ScenarioConfig::Generated(generated) = &mut scenario else { unreachable!() };
        generated.num_skus = 1;
        generated.total_inventory_units = 20;
        generated.num_orders = 3;
        generated.total_order_picks = 3;
    }
    config.fidelity = FidelityMode::CollisionDes;
    let pickup_time = SimDuration::seconds(7);
    config.htm.pickup_secs = 7.0;
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let inventory = reports.existing::<InventoryService>().unwrap();
    let bots = reports.existing::<BotService>().unwrap();
    let relays = reports.existing::<RelayService>().unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    let pickups: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::InventoryMove(moved)
                if matches!(moved.from, ToteLocation::AtRelay { .. })
                    && matches!(moved.to, ToteLocation::WithRobot { robot_id }
                        if bots.bot_kind(robot_id) == Some(BotKind::Htm)) =>
            {
                Some(moved)
            }
            _ => None,
        })
        .collect();
    assert!(!pickups.is_empty(), "fixture must exercise real HTM relay pickups");
    for moved in &pickups {
        let ToteLocation::AtRelay { relay_id } = moved.from else { unreachable!() };
        let ToteLocation::WithRobot { robot_id } = moved.to else { unreachable!() };
        let pickup = events
            .iter()
            .find_map(|event| match event {
                SimEvent::HtmRelayPickup(fork)
                    if fork.robot_id == robot_id
                        && fork.relay_id == relay_id
                        && fork.start <= moved.at
                        && moved.at <= fork.end =>
                {
                    Some(fork.start)
                }
                _ => None,
            })
            .expect("pickup must publish its actual fork start");
        let end = pickup + pickup_time;
        assert_eq!(moved.at, end, "inventory custody changed before fork completion");
        let tote = inventory.tote(moved.tote_id).unwrap();
        assert!(tote.location_is_through(pickup, end - SimDuration::ms(1), moved.from));
        assert_eq!(tote.location_at(end), moved.to);
        assert!(inventory.tote_on_robot(robot_id, end - SimDuration::ms(1)).is_none());
        assert_eq!(inventory.tote_on_robot(robot_id, end).unwrap().0, moved.tote_id);
        let relay_cell = relays.htm_access_cell(relay_id).unwrap();
        assert_eq!(bots.position_at(robot_id, pickup), Some(relay_cell));
        assert_eq!(bots.position_at(robot_id, end - SimDuration::ms(1)), Some(relay_cell));
        assert!(events.iter().any(|event| matches!(event, SimEvent::RelayStateChange(change)
            if change.cell_id == relay_cell.0 && !change.reserved && change.time == end)));
        let visual = events
            .iter()
            .find_map(|event| match event {
                SimEvent::ToteMoved(visual)
                    if visual.tote_id == moved.tote_id
                        && visual.new_state == (ToteVisualState::OnBot { bot_id: robot_id.0 })
                        && visual.time >= pickup
                        && visual.time <= end =>
                {
                    Some(visual)
                }
                _ => None,
            })
            .expect("physical pickup needs its corresponding visual transition");
        assert_eq!(visual.time, end);
        // The fork-Get bounds the custody change on both sides; the bot is
        // empty at `pickup` and carrying at `end` by construction, so the
        // operation's own window is the whole statement.
        assert!(
            events.iter().any(|event| matches!(event, SimEvent::HtmRelayPickup(fork)
                if fork.robot_id == robot_id && fork.start == pickup && fork.end == end)),
            "the fork-Get window must bound the custody change"
        );
        let mut matching_leg = false;
        bots.for_each(BotKind::Htm, |id, plan| {
            if id == robot_id {
                matching_leg =
                    plan.legs().iter().any(|leg| leg.kind == Leg::RelayApproach && leg.action_ready_arrival == end);
            }
        });
        assert!(matching_leg, "the committed pickup leg must remain action-ready at fork completion");
    }
    eprintln!("{fixture}: verified {} completed HTM relay pickups", pickups.len());
}

#[test]
fn cencora_htm_relay_pickups_keep_custody_until_fork_completion() {
    check_pickups("minimal_pick_only_spec.json");
}

#[test]
fn walmart_htm_relay_pickups_keep_custody_until_fork_completion() {
    check_pickups("minimal_easyloop_flow_spec.json");
}
