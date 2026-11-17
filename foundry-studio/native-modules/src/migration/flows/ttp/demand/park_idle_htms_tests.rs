//! Idle relocation preserves occupied cells and screens out destinations that
//! cannot accept its open-ended stop before asking for a route.

use crate::robots::runtime::RobotModelId;
use desgraph::{BlockId, RelayId};
use desmap::{BlockSide, RelayPosition, VtmPosition};
use std::sync::{Arc, Mutex};

use super::*;
use crate::model::BotKind;
use crate::model::test_model::{TestModelConfig, block, build_model, register_idle_htm};
use crate::pathfinding::ReservationOracle;
use crate::pathfinding::{DriveModel, FHSippPathfinder, Orientation, Path, Pathfinder, ReservedPath, WindowSink};
use crate::simtime::TimeRange;
use async_trait::async_trait;

struct RecordingPathfinder {
    inner: FHSippPathfinder,
    destinations: Mutex<Vec<CellId>>,
}

#[async_trait(?Send)]
impl Pathfinder for RecordingPathfinder {
    fn find_path(
        &self,
        from: CellId,
        orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        start: SimTime,
    ) -> Result<Path, PathfinderError> {
        self.inner.find_path(from, orientation, to, drive, start)
    }

    async fn reserve_path(
        &self,
        from: CellId,
        orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
        windows: &mut dyn WindowSink,
    ) -> Result<ReservedPath, PathfinderError> {
        self.destinations.lock().unwrap().push(to);
        self.inner.reserve_path(from, orientation, to, drive, oracle, config, windows).await
    }

    fn traverse_duration(&self) -> SimDuration {
        self.inner.traverse_duration()
    }
}

#[test]
fn blocked_nearest_parking_bay_does_not_hide_a_reachable_alternative() {
    parking_destination_selection(false);
}

#[test]
fn a_relay_that_clears_within_the_horizon_remains_a_parking_candidate() {
    parking_destination_selection(true);
}

fn parking_destination_selection(finite_hold: bool) {
    let (model, oracle) = build_model(TestModelConfig::default());
    let mut map = model.navigation.map.as_ref().clone();
    map.relays = [CellId(8), CellId(40)]
        .into_iter()
        .map(|cell| RelayPosition {
            id: RelayId(cell.0),
            position: cell,
            htm_access: cell,
            vtm_access: VtmPosition::Cell(CellId(0)),
            block: BlockId(0),
            side: BlockSide::Left,
        })
        .collect();
    if finite_hold {
        map.relays.retain(|relay| relay.htm_access == CellId(8));
    }
    let parking = ParkingRegistry::from_map(&map);
    let pathfinder = Arc::new(RecordingPathfinder {
        inner: FHSippPathfinder::new(
            &map,
            1000,
            SimDuration::seconds(1),
            [CellId(8), CellId(40)].into_iter().collect(),
        ),
        destinations: Mutex::new(Vec::new()),
    });
    let navigation = Navigation::new(Arc::new(map), pathfinder.clone(), model.bot_service.clone(), oracle.clone());
    if !finite_hold {
        let travel = |to| {
            navigation
                .cache_for_model(RobotModelId(0))
                .unwrap()
                .travel_time(PathCacheKey {
                    from: CellId(10),
                    from_orientation: Orientation::default(),
                    to,
                    carrying_cargo: false,
                })
                .unwrap()
        };
        assert!(travel(CellId(8)) < travel(CellId(40)), "the blocked relay must win the unscreened route ranking");
    }
    let robot = RobotId(1);
    register_idle_htm(&model, &oracle, robot, CellId(10), SimTime::ZERO);
    // A robot working at the nearer bay holds its floor cell, without claiming
    // it as idle parking. The parking registry alone cannot see that conflict.
    model.bot_service.register(RobotId(2), BotKind::Htm, CellId(8)).unwrap();
    let blocked_until = SimTime::seconds(10);
    let window =
        if finite_hold { TimeRange::new(SimTime::ZERO, blocked_until) } else { TimeRange::open_ended(SimTime::ZERO) };
    let blocked = oracle.table().reserve_cell(RobotId(2), CellId(8), window).unwrap();
    assert!(
        block(vacate_cell(
            &navigation,
            &model.available_htm,
            &model.bot_service,
            &model.inventory,
            Some(&parking),
            &model.events,
            robot,
            CellId(10),
            SimTime::ZERO,
        ))
        .unwrap()
    );
    let parked = model.available_htm.peek_matching(|htm| htm.robot() == robot);
    assert_eq!(parked.len(), 1);
    let destination = if finite_hold { CellId(8) } else { CellId(40) };
    assert_eq!(parked[0].cell(), destination);
    assert!(parked[0].cell_reservation.is_open_ended());
    assert_eq!(
        *pathfinder.destinations.lock().unwrap(),
        vec![destination],
        "only search destinations that can accept parking"
    );
    assert_eq!(blocked.is_open_ended(), !finite_hold);
    if finite_hold {
        assert!(parked[0].cell_reservation.window().start() >= blocked_until);
    }
    assert!(oracle.find_overlaps().is_empty());
}

#[test]
fn random_idle_selection_is_not_reranked_to_the_nearest_relay() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let mut map = model.navigation.map.as_ref().clone();
    map.relays = [CellId(9), CellId(14)]
        .into_iter()
        .map(|cell| RelayPosition {
            id: RelayId(cell.0),
            position: cell,
            htm_access: cell,
            vtm_access: VtmPosition::Cell(CellId(0)),
            block: BlockId(0),
            side: BlockSide::Left,
        })
        .collect();
    let robot = RobotId(1);
    let registry = (0..100)
        .map(|seed| ParkingRegistry::with_policy(&map, IdleRelaySelection::Random, seed))
        .find(|registry| {
            registry.find_candidates(CellId(10), cell_point_mm(&map, CellId(10)), SimTime::ZERO, robot)[0] == CellId(14)
        })
        .expect("a seed selecting the farther relay");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &map,
        1000,
        SimDuration::seconds(1),
        [CellId(9), CellId(14)].into_iter().collect(),
    ));
    let navigation = Navigation::new(Arc::new(map), pathfinder, model.bot_service.clone(), oracle.clone());
    register_idle_htm(&model, &oracle, robot, CellId(10), SimTime::ZERO);
    assert!(
        block(vacate_cell(
            &navigation,
            &model.available_htm,
            &model.bot_service,
            &model.inventory,
            Some(&registry),
            &model.events,
            robot,
            CellId(10),
            SimTime::ZERO
        ))
        .unwrap()
    );
    let idle = model.available_htm.peek_matching(|entry| entry.robot() == robot);
    assert_eq!(idle[0].cell(), CellId(14), "random ordering must survive final path selection");
}
