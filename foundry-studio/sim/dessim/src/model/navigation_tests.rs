//! The departure screen rejects only an already occupied first step; routing
//! still owns finite timing conflicts, downstream obstacles and static errors.

use super::*;
use crate::model::BotKind;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{TestModelConfig, build_model};
use crate::pathfinding::{CellCandidate, Orientation};
use crate::simtime::TimeRange;
use desgraph::{DestGate, Edge, WarehouseGraphBuilder};
use desmap::{SystemType, WarehouseMapBuilder};

fn navigation(edges: &[(u32, u32, Edge)]) -> (Navigation, Arc<TableOracle>) {
    let (model, oracle) = build_model(TestModelConfig::default());
    let mut graph = WarehouseGraphBuilder::new();
    for id in 0..=3 {
        graph.add_cell(model.navigation.map.graph.cell(CellId(id)).unwrap().clone());
    }
    for &(from, to, edge) in edges {
        graph.add_edge(CellId(from), CellId(to), edge);
    }
    let map = Arc::new(WarehouseMapBuilder::new(SystemType::Relay).graph(graph.build()).build());
    let navigation =
        Navigation::new(map, model.navigation.pathfinder.clone(), model.bot_service.clone(), oracle.clone());
    (navigation, oracle)
}

#[test]
fn departure_screen_preserves_finite_and_future_holds_for_pathfinding() {
    let edges = [(0, 1, Edge::uniform(1000)), (0, 2, Edge::uniform(1000))];
    for (window, expected_blocked) in [
        (TimeRange::open_ended(SimTime::ZERO), true),
        (TimeRange::open_ended(SimTime::seconds(21)), false),
        (TimeRange::open_ended(SimTime::seconds(20)), true),
        (TimeRange::new(SimTime::ZERO, SimTime::seconds(50)), false),
    ] {
        let (navigation, oracle) = navigation(&edges);
        oracle.table().reserve_cell(RobotId(2), CellId(1), TimeRange::open_ended(SimTime::ZERO)).unwrap();
        oracle.table().reserve_cell(RobotId(3), CellId(2), window).unwrap();
        assert_eq!(
            navigation.htm_departure_is_blocked(RobotId(1), CellId(0), CellId(3), false, SimTime::seconds(20)),
            expected_blocked,
            "neighbor hold {window:?}"
        );
    }
}

#[test]
fn departure_screen_uses_outgoing_edges_and_the_actual_destination_gate() {
    let gated = Edge { dest_only: Some(DestGate::Cell(CellId(3))), ..Edge::uniform(1000) };
    let (navigation, oracle) = navigation(&[(1, 0, Edge::uniform(1000)), (0, 2, Edge::uniform(1000)), (0, 3, gated)]);
    oracle.table().reserve_cell(RobotId(2), CellId(2), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    assert!(navigation.htm_departure_is_blocked(RobotId(1), CellId(0), CellId(2), false, SimTime::ZERO));
    assert!(!navigation.htm_departure_is_blocked(RobotId(1), CellId(0), CellId(3), false, SimTime::ZERO));
    assert!(!navigation.htm_departure_is_blocked(RobotId(1), CellId(0), CellId(0), false, SimTime::ZERO));
}

#[test]
fn departure_screen_respects_htm_load_and_does_not_wait_for_missing_static_edges() {
    let mut unladen = Edge::uniform(1000);
    unladen.traversability.htm_traversal = HtmTraversal::UnladenOnly;
    let mut vtm_only = Edge::uniform(1000);
    vtm_only.traversability.htm_traversal = HtmTraversal::None;
    let (navigation, oracle) = navigation(&[(0, 1, unladen), (0, 2, vtm_only), (3, 0, Edge::uniform(1000))]);
    oracle.table().reserve_cell(RobotId(2), CellId(1), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    assert!(navigation.htm_departure_is_blocked(RobotId(1), CellId(0), CellId(3), false, SimTime::ZERO));
    assert!(
        !navigation.htm_departure_is_blocked(RobotId(1), CellId(0), CellId(3), true, SimTime::ZERO),
        "no legal laden exit must reach the static pathfinder error rather than wait for an oracle change"
    );
    assert!(!navigation.htm_departure_is_blocked(RobotId(1), CellId(2), CellId(3), false, SimTime::ZERO));
    assert!(
        !navigation.htm_departure_is_blocked(RobotId(2), CellId(0), CellId(3), false, SimTime::ZERO),
        "the acting robot's own holds do not block it"
    );
}

#[test]
fn horizon_screen_rejects_continuous_finite_traffic_but_preserves_other_exits() {
    let (navigation, oracle) = navigation(&[(0, 1, Edge::uniform(1000)), (0, 2, Edge::uniform(1000))]);
    for cell in [CellId(1), CellId(2)] {
        for i in 0..4 {
            oracle
                .table()
                .reserve_cell(
                    RobotId(10 + i),
                    cell,
                    TimeRange::new(SimTime::seconds(i as u64 * 100), SimTime::seconds((i as u64 + 1) * 100)),
                )
                .unwrap();
        }
    }
    let blocked = |from, target, after| {
        navigation.htm_departure_exceeds_horizon(RobotId(1), from, target, false, after, SimDuration::seconds(300))
    };
    assert!(blocked(CellId(0), CellId(3), SimTime::ZERO));
    assert!(!blocked(CellId(0), CellId(3), SimTime::seconds(101)), "a later opening remains for actual pathfinding");
    assert!(!blocked(CellId(0), CellId(0), SimTime::ZERO), "resident work does not need to depart");
    assert!(!blocked(CellId(3), CellId(0), SimTime::ZERO), "missing edges must reach static route validation");
}

#[test]
fn horizon_screen_respects_destination_gates_and_available_alternatives() {
    let gated = Edge { dest_only: Some(DestGate::Cell(CellId(3))), ..Edge::uniform(1000) };
    let (navigation, oracle) = navigation(&[(0, 1, Edge::uniform(1000)), (0, 2, gated)]);
    oracle.table().reserve_cell(RobotId(2), CellId(1), TimeRange::new(SimTime::ZERO, SimTime::seconds(400))).unwrap();
    assert!(navigation.htm_departure_exceeds_horizon(
        RobotId(1),
        CellId(0),
        CellId(1),
        false,
        SimTime::ZERO,
        SimDuration::seconds(300)
    ));
    assert!(!navigation.htm_departure_exceeds_horizon(
        RobotId(1),
        CellId(0),
        CellId(3),
        false,
        SimTime::ZERO,
        SimDuration::seconds(300)
    ));
}

#[derive(Clone)]
struct ModelCandidate {
    robot: RobotId,
    cell: CellId,
}

impl CellCandidate for ModelCandidate {
    fn cell(&self) -> CellId {
        self.cell
    }
}

fn mixed_model_navigation() -> Navigation {
    use crate::config::SimConfig;
    use crate::model::BotService;
    use crate::pathfinding::{FHSippPathfinder, MotionModel};
    use crate::robots::runtime::{FloorRobotModel, RobotModelId, RuntimeRobots};
    let (fixture, oracle) = build_model(TestModelConfig::default());
    let model = |id, speed| {
        let mut config = SimConfig::default();
        config.motion_model = MotionModel::ConstantSpeed;
        config.htm.empty_speed = speed;
        config.htm.loaded_speed = speed;
        config.htm.empty_constant_speed = speed;
        config.htm.loaded_constant_speed = speed;
        let mut model = FloorRobotModel::legacy(&config);
        model.id = RobotModelId(id);
        Arc::new(model)
    };
    let slow = model(1, 0.1);
    let fast = model(2, 10.0);
    let mut models = RuntimeRobots::default();
    models.htm = vec![slow; 12];
    models.htm.push(fast);
    let bots = BotService::from_runtime(&models, 0);
    for id in 0..=12 {
        bots.register(RobotId(id), BotKind::Htm, CellId(if id == 12 { 20 } else { id + 1 })).unwrap();
    }
    let pathfinder =
        Arc::new(FHSippPathfinder::new(&fixture.navigation.map, 1000, SimDuration::seconds(1), Default::default()));
    Navigation::new(fixture.navigation.map.clone(), pathfinder, bots, oracle)
}

#[test]
fn heterogeneous_dispatch_refines_the_faster_candidate_beyond_the_old_top_ten() {
    let navigation = mixed_model_navigation();
    let mut candidates: Vec<_> =
        (0..12).map(|id| ModelCandidate { robot: RobotId(id), cell: CellId(id + 1) }).collect();
    candidates.push(ModelCandidate { robot: RobotId(12), cell: CellId(20) });
    let selected = navigation
        .select_htm_to(
            candidates,
            CellId(0),
            |candidate| candidate.robot,
            |_| Orientation::West,
            false,
            |_, time| Some(SimTime::ZERO + time),
        )
        .unwrap();
    assert_eq!(selected.candidate.robot, RobotId(12));
    assert_eq!(selected.travel_time, SimDuration::seconds(2));
}

#[test]
fn an_unassigned_quote_uses_the_slowest_eligible_model_and_unknown_candidates_refuse() {
    use crate::pathfinding::{Orientation, PathCacheKey, PathSelectionError};
    let navigation = mixed_model_navigation();
    let key =
        PathCacheKey { from: CellId(20), from_orientation: Orientation::West, to: CellId(0), carrying_cargo: true };
    let slow = navigation.cache(RobotId(0)).unwrap().travel_time(key).unwrap();
    let fast = navigation.cache(RobotId(12)).unwrap().travel_time(key).unwrap();
    assert!(slow > fast);
    assert_eq!(navigation.unassigned_travel_time(key).unwrap(), slow);
    let error = navigation.select_htm_to(
        [ModelCandidate { robot: RobotId(99), cell: CellId(0) }],
        CellId(0),
        |candidate| candidate.robot,
        |_| Orientation::West,
        false,
        |_, time| Some(SimTime::ZERO + time),
    );
    assert!(
        matches!(error, Err(PathSelectionError::ModelBinding(_))),
        "even a zero-distance candidate needs an actual robot binding"
    );
}
