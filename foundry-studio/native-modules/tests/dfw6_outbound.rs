//! The northern handoffs and relocated decant stations must have distinct,
//! reachable approaches with no overlapping service cells.

mod common;

use std::path::Path;

use desgraph::StationType;
use desmap::OutboundRole;
use dessim::pathfinding::{Orientation, htm_estimate_drives};
use dessim::{SimConfig, SimTime, worldmap};
use rustc_hash::FxHashSet;

#[test]
fn northern_stations_have_independent_reachable_approaches() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json");
    let world = worldmap::load(&path).unwrap();
    let endpoints: Vec<_> = world
        .map
        .stations
        .iter()
        .filter(|s| s.outbound_role.is_some() || s.station_type == StationType::Putaway)
        .collect();
    for (role, expected) in [
        (OutboundRole::RouteDispense, 3),
        (OutboundRole::Bagging, 1),
        (OutboundRole::Flex, 1),
        (OutboundRole::SealerDropoff, 3),
        (OutboundRole::SealedBoxPickup, 1),
    ] {
        assert_eq!(endpoints.iter().filter(|s| s.outbound_role == Some(role)).count(), expected, "{role:?}");
    }
    let mut owned = FxHashSet::default();
    for station in &world.map.stations {
        for cell in station.presentation_points.iter().flat_map(|point| point.cells()) {
            // EasyLoop shared conveyor queues are outside this station group.
            if station.station_type == StationType::Pick {
                continue;
            }
            assert!(owned.insert(cell), "station {:?} shares a service/approach cell {cell:?}", station.id);
        }
    }
    let planner = common::fh_sipp_for_map(&world.map, &SimConfig::default());
    let [empty, laden] = htm_estimate_drives(&SimConfig::default());
    for (endpoint, point) in endpoints.iter().flat_map(|e| e.presentation_points.iter().map(move |point| (e, point))) {
        if endpoint.outbound_role.is_some_and(|role| role != OutboundRole::RouteDispense) {
            let at = world.map.graph.cell(point.presentation_cell).unwrap().world_pos.unwrap();
            let before = world.map.graph.cell(*point.queue_path.last().unwrap()).unwrap().world_pos.unwrap();
            let exits = world.map.graph.neighbors(point.presentation_cell);
            assert_eq!(exits.len(), 1);
            let after = world.map.graph.cell(exits[0].0).unwrap().world_pos.unwrap();
            assert_eq!(before.x_mm, at.x_mm);
            assert_eq!(after.x_mm, at.x_mm);
            assert!(
                before.y_mm < at.y_mm && at.y_mm < after.y_mm,
                "new handoffs enter and leave straight; turns occur outside the presentation"
            );
        }
        let p = world.map.graph.cell(point.presentation_cell).unwrap().world_pos.unwrap();
        assert!(p.y_mm > 80_000.0, "post-pick handoffs belong north of storage");
        for machine in &world.map.conveyor_machines {
            for presentation in &machine.presentations {
                let from = presentation.order_area.as_ref().unwrap().egress;
                planner
                    .find_path(from, Orientation::North, point.presentation_cell, laden.as_ref(), SimTime::ZERO)
                    .unwrap_or_else(|error| {
                        panic!("loaded HTM cannot reach {:?} from {from:?}: {error}", endpoint.outbound_role)
                    });
                planner
                    .find_path(
                        point.presentation_cell,
                        Orientation::North,
                        world.map.relays[0].htm_access,
                        empty.as_ref(),
                        SimTime::ZERO,
                    )
                    .unwrap_or_else(|error| panic!("empty HTM cannot leave {:?}: {error}", endpoint.outbound_role));
            }
        }
    }
}
