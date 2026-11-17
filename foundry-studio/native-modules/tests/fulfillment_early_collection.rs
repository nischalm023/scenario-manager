//! A retained inventory HTM can collect another order's returned tote while
//! its own order presentation is blocked. Collection starts at conveyor supply.

mod common;

use std::sync::{Arc, mpsc};

use dessim::composition;
use dessim::events::SimEvent;
use dessim::model::conveyance::{Conveyances, TotePhase};
use dessim::model::navigation::Navigation;
use dessim::model::order::{ExitPath, OrderContainer};
use dessim::model::{BotKind, BotService, InventoryService, RobotId, ToteLocation};
use dessim::observer::time_slice::ToteVisualState;
use dessim::pathfinding::{CommitPlan, ReservationRequest};
use dessim::runner::resolve_scenario_with_racking;
use dessim::{FidelityMode, SimDuration, SimTime};
use native_modules as _;

#[test]
fn retained_collector_serves_another_return_while_its_order_presentation_is_blocked() {
    let (world, scenario, mut config) = common::load_world_spec("minimal_easyloop_flow_spec.json");
    config.fidelity = FidelityMode::CollisionDes;
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    data.orders.truncate(2);
    for order in &mut data.orders {
        order.lines.truncate(1);
        order.lines[0].quantity = 1;
        order.exit_path = ExitPath::Dock;
    }
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].arrival = Some(SimTime::ZERO);
    data.orders[1].container = OrderContainer::Tote;
    data.orders[1].arrival = Some(SimTime::seconds(60));
    let first_order = data.orders[0].id;
    let second_order = data.orders[1].id;
    let presentation_free = SimTime::seconds(200);
    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    let navigation = scheduler.states().get::<Navigation>().unwrap();
    let first_presentation = data.map.conveyor_machines[0].presentations[0].cell;
    navigation
        .oracle
        .commit_path(
            RobotId(1000),
            &CommitPlan::single(ReservationRequest::spanning(
                first_presentation,
                SimTime::ZERO,
                Some(presentation_free),
            )),
            None,
        )
        .unwrap();
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    let inventory = reports.existing::<InventoryService>().unwrap();
    let bots = reports.existing::<BotService>().unwrap();
    let conveyances = reports.existing::<Conveyances>().unwrap();
    let (_, conveyor) = &conveyances.machines()[0];
    let pick = |order_id| {
        events
            .iter()
            .find_map(|event| match event {
                SimEvent::CycleComplete(cycle)
                    if cycle.tote_presentation.is_some_and(|pick| pick.order_id == order_id) =>
                {
                    Some(cycle)
                }
                _ => None,
            })
            .unwrap()
    };
    let first_pick = pick(first_order);
    let second_pick = pick(second_order);
    let first_tote = first_pick.tote_presentation.unwrap().tote_id;
    let second_tote = second_pick.tote_presentation.unwrap().tote_id;
    let first_robot = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteMoved(moved) if moved.tote_id == first_tote => match moved.new_state {
                ToteVisualState::OnBot { bot_id } if bots.bot_kind(RobotId(bot_id)) == Some(BotKind::Htm) => {
                    Some(RobotId(bot_id))
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let visits = conveyor.visits();
    let first_visit = visits.iter().find(|visit| visit.tote == first_tote).unwrap();
    let second_visit = visits.iter().find(|visit| visit.tote == second_tote).unwrap();
    assert!(
        first_visit.boarded_at.unwrap() < second_visit.boarded_at.unwrap(),
        "fixture must put the delayed order's inventory HTM first in the queue"
    );
    assert!(first_visit.boarded_at.unwrap() < SimTime::seconds(60));
    assert!(SimTime::ms(first_pick.sim_time_ms) >= presentation_free);
    assert!(SimTime::ms(second_pick.sim_time_ms + second_pick.dwell_ms) < presentation_free);
    let TotePhase::Collected { at, robot } = second_visit.phase else { panic!("second tote was not collected") };
    assert_eq!(robot, first_robot, "the front inventory HTM must collect the other order's ready tote");
    assert!(
        at < presentation_free,
        "collection at {at} incorrectly waited for its original order's blocked presentation"
    );
    assert_eq!(inventory.tote(second_tote).unwrap().location_at(at), ToteLocation::WithRobot { robot_id: first_robot });
    assert!(inventory.tote(first_tote).unwrap().location_is_through(
        first_visit.boarded_at.unwrap(),
        presentation_free - SimDuration::ms(1),
        ToteLocation::OnConveyance { conveyor_id: conveyor.id() }
    ));
    assert_eq!(conveyor.unfinished(), 0, "root completion awaits both collectors and their rack returns");
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 2);
}
