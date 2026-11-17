//! Physical rulings through September 12, checked on both loaded map graphs.
use desgraph::{CellId, HtmTraversal, StationType};
use desmap::{PresentationPoint, WarehouseMap};
use dessim::worldmap::{self, WorldMap};
use rustc_hash::FxHashSet;
use serde_json::Value;
use std::path::Path;

fn reachable(w: &WorldMap, from: CellId, to: CellId, members: &FxHashSet<CellId>) -> bool {
    let mut seen = FxHashSet::default();
    let mut todo = vec![from];
    while let Some(cell) = todo.pop() {
        if cell == to {
            return true;
        }
        if !seen.insert(cell) {
            continue;
        }
        todo.extend(w.map.graph.neighbors(cell).into_iter().filter_map(|(n, edge)| {
            (members.contains(&n)
                && edge.traversability.htm_traversal == HtmTraversal::Both
                && edge.dest_only.is_none_or(|gate| gate.admits(to)))
            .then_some(n)
        }));
    }
    false
}

#[test]
fn independent_waiting_blocks_and_direct_presentations_survive_loading() {
    for (fixture, count) in [("minimal_easyloop.json", 1), ("walmart_dfw6.json", 4)] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let w = worldmap::load(&path).expect("map loads");
        let g = &w.map.graph;
        for number in 1..=count {
            let prefix = format!("easyloop {number}: ");
            let members: FxHashSet<_> = w
                .index
                .cell_entities
                .iter()
                .filter_map(|(cell, id)| {
                    w.world
                        .entities
                        .get(id)
                        .and_then(|e| e.name.as_ref())
                        .is_some_and(|name| name.starts_with(&prefix))
                        .then_some(*cell)
                })
                .collect();
            assert_eq!(members.len(), 52);
            let named = |handle: &str| {
                members.iter().copied().find(|cell| {
                    w.world.entities[&w.index.cell_entities[cell]].name.as_deref() == Some(&format!("{prefix}{handle}"))
                })
            };
            for handle in ["return:0:corner", "return:0:under", "entry:1:corner", "entry:1:under"] {
                assert!(named(handle).is_none(), "intermediate connector {handle} was removed");
            }
            for side in 0..2 {
                let columns = if side == 0 { [0, 1] } else { [5, 6] };
                for row in 0..6 {
                    assert!(named(&format!("order:3/{row}")).is_none(), "shared column is gone");
                    let wait = named(&format!("wait:{side}/{row}")).unwrap();
                    let mut lanes = columns.map(|col| named(&format!("order:{col}/{}", row + 1)).unwrap());
                    lanes.sort_by(|a, b| {
                        g.cell(*a)
                            .unwrap()
                            .world_pos
                            .unwrap()
                            .x_mm
                            .total_cmp(&g.cell(*b).unwrap().world_pos.unwrap().x_mm)
                    });
                    let [descending, returning] = lanes;
                    for (lane, toward_operator) in [(descending, true), (returning, false)] {
                        let next = g.neighbors(lane).into_iter().find(|(next, _)| *next != wait).unwrap().0;
                        let dy = g.cell(next).unwrap().world_pos.unwrap().y_mm
                            - g.cell(lane).unwrap().world_pos.unwrap().y_mm;
                        assert_eq!(dy < 0.0, toward_operator, "the two lanes circulate in opposite directions");
                    }
                    let outgoing: Vec<_> = g.neighbors(wait).iter().map(|(next, _)| *next).collect();
                    assert_eq!(outgoing, [descending], "a wait dispatches only left, into the descending lane");
                    for lane in lanes {
                        assert!(g.neighbors(lane).iter().any(|(next, _)| *next == wait));
                        assert_eq!(g.neighbors(lane).len(), 2, "one longitudinal link and one waiting link");
                        let gap = (g.cell(lane).unwrap().world_pos.unwrap().x_mm
                            - g.cell(wait).unwrap().world_pos.unwrap().x_mm)
                            .abs()
                            - 695.0;
                        assert!((0.0..10.0).contains(&gap), "waiting block gap is {gap} mm");
                    }
                    let at = named(&format!("present:{side}")).unwrap();
                    let other = named(&format!("present:{}", 1 - side)).unwrap();
                    assert!(reachable(&w, wait, at, &members));
                    assert!(reachable(&w, at, wait, &members));
                    assert!(!reachable(&w, wait, other, &members), "no crossing into the other block");
                    assert!(!reachable(&w, other, wait, &members), "no return from the other block");
                }
                for col in columns {
                    let boundary = named(&format!("order:{col}/0")).unwrap();
                    let first = named(&format!("order:{col}/1")).unwrap();
                    let x = g.cell(boundary).unwrap().world_pos.unwrap().x_mm;
                    for row in 1..=6 {
                        let cell = named(&format!("order:{col}/{row}")).unwrap();
                        assert!(
                            (g.cell(cell).unwrap().world_pos.unwrap().x_mm - x).abs() < 0.1,
                            "the complete entry/return lane is straight"
                        );
                    }
                    if fixture == "walmart_dfw6.json" {
                        let outside: Vec<_> = g
                            .edges()
                            .filter_map(|(from, to, _)| {
                                if from == boundary && !members.contains(&to) {
                                    Some(to)
                                } else if to == boundary && !members.contains(&from) {
                                    Some(from)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        assert_eq!(outside.len(), 1, "each order door has one apron connection");
                        assert!(
                            (g.cell(outside[0]).unwrap().world_pos.unwrap().x_mm - x).abs() < 0.1,
                            "the apron connection stays straight after moving a lane"
                        );
                    }
                    let internal: Vec<_> = g
                        .neighbors(boundary)
                        .iter()
                        .filter(|(to, _)| members.contains(to))
                        .map(|(to, _)| *to)
                        .collect();
                    let enters = g.cell(boundary).unwrap().world_pos.unwrap().x_mm
                        < g.cell(named(&format!("wait:{side}/0")).unwrap()).unwrap().world_pos.unwrap().x_mm;
                    if enters {
                        assert_eq!(internal, [first], "dedicated entry has no crossover or lateral exit");
                    } else {
                        assert!(internal.is_empty(), "egress leaves the block");
                    }
                }
            }
            for side in 0..2 {
                for endpoint in ["in", "out"] {
                    assert!(named(&format!("turn:{side}:{endpoint}")).is_none(), "curve helpers are edge geometry");
                }
                let mut ends =
                    if side == 0 { [0, 1] } else { [5, 6] }.map(|col| named(&format!("order:{col}/6")).unwrap());
                ends.sort_by(|a, b| {
                    g.cell(*a).unwrap().world_pos.unwrap().x_mm.total_cmp(&g.cell(*b).unwrap().world_pos.unwrap().x_mm)
                });
                let present = named(&format!("present:{side}")).unwrap();
                let approach = g.neighbors(ends[0]).into_iter().find(|(to, _)| *to == present).unwrap().1;
                assert!(approach.dest_only.unwrap().admits(present));
                let wait = named(&format!("wait:{side}/5")).unwrap();
                assert!(!approach.dest_only.unwrap().admits(wait), "presentation cannot be used as a through route");
                assert_eq!(g.neighbors(present)[0].0, ends[1], "departure connects directly to the return lane");
            }
            let waiting: Vec<_> = members
                .iter()
                .copied()
                .filter(|cell| w.world.entities[&w.index.cell_entities[cell]].has("waiting"))
                .collect();
            assert_eq!(waiting.len(), 12);
            let mut places: Vec<_> = w
                .map
                .stations
                .iter()
                .filter(|s| {
                    s.station_type == StationType::Pick
                        && s.order_presentation_cell.is_some_and(|cell| members.contains(&cell))
                })
                .collect();
            places.sort_by(|a, b| {
                g.cell(a.order_presentation_cell.unwrap())
                    .unwrap()
                    .world_pos
                    .unwrap()
                    .x_mm
                    .total_cmp(&g.cell(b.order_presentation_cell.unwrap()).unwrap().world_pos.unwrap().x_mm)
            });
            assert_eq!(places.len(), 2);
            let conveyor = w.conveyors.of_station(places[0].id).expect("a shared conveyor");
            assert_eq!(w.conveyors.capacity(conveyor), Some(8), "eight totes over the entire conveyor");
            for (side, place) in places.iter().enumerate() {
                let at = place.order_presentation_cell.unwrap();
                assert!(!w.index.cell_entities[&at].is_derived(), "presentation is declared directly");
                let incoming: Vec<_> = g.edges().filter(|(_, to, _)| *to == at).collect();
                let outgoing = g.neighbors(at);
                assert_eq!((incoming.len(), outgoing.len()), (1, 1));
                let (entry, _, approach) = incoming[0];
                let (exit, departure) = outgoing[0];
                let point = |cell| g.cell(cell).unwrap().world_pos.unwrap();
                let arc = approach.curve.expect("approach is curved");
                let onward = departure.curve.expect("departure is curved");
                assert_eq!(arc.center, onward.center, "dwell splits one continuous circle");
                assert_eq!(arc.clockwise, onward.clockwise);
                assert_eq!(arc.end, None, "presentation is directly on the circle");
                assert_eq!(onward.start, None);
                let phase = arc.sweep(point(entry), point(at)).abs().to_degrees();
                assert!((phase - if side == 0 { 135.0 } else { 45.0 }).abs() < 0.01, "physical side {side}: {phase}");
                assert!(point(entry).x_mm < point(exit).x_mm, "enter left, return right on both physical sides");
                let remainder = onward.sweep(point(at), point(exit)).abs().to_degrees();
                assert!((phase + remainder - 180.0).abs() < 0.01);
                assert!(arc.start_tangent(point(entry))[1] < -0.999);
                assert!(onward.end_tangent(point(exit))[1] > 0.999);
                assert!(
                    (arc.start.unwrap().x_mm - point(entry).x_mm).abs() < 0.1,
                    "approach runs straight into the circle without a lateral jog"
                );
                assert!(
                    (onward.end.unwrap().x_mm - point(exit).x_mm).abs() < 0.1,
                    "departure runs straight out of the circle without a lateral jog"
                );
                let p = point(at);
                let radius = (p.x_mm - arc.center.x_mm).hypot(p.y_mm - arc.center.y_mm);
                let outer_radius = (radius + 350.0).hypot(350.0);
                assert!((699.9..700.1).contains(&radius), "centreline radius: {radius} mm");
                assert!((1106.5..1107.0).contains(&outer_radius), "700 mm square swept envelope: {outer_radius} mm");
            }
            for &a in &members {
                for &b in &members {
                    if a < b {
                        assert!(
                            !tiles_overlap(&w, a, b),
                            "rotated cell footprints overlap: {:?}, {:?}",
                            w.world.entities[&w.index.cell_entities[&a]].name,
                            w.world.entities[&w.index.cell_entities[&b]].name
                        );
                    }
                }
            }
        }
    }
}

fn tiles_overlap(w: &WorldMap, a: CellId, b: CellId) -> bool {
    let corners = |id: CellId| {
        let p = w.map.graph.cell(id).unwrap().world_pos.unwrap();
        let [width, height] = w.cell_sizes_mm[id.0 as usize];
        let (s, c) = w.cell_headings_rad[id.0 as usize].sin_cos();
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
            .map(|(x, y)| [p.x_mm + x * width * c - y * height * s, p.y_mm + x * width * s + y * height * c])
    };
    let (a, b) = (corners(a), corners(b));
    for polygon in [a, b] {
        for i in 0..2 {
            let d = [polygon[i + 1][0] - polygon[i][0], polygon[i + 1][1] - polygon[i][1]];
            let axis = [-d[1], d[0]];
            let project = |poly: [[f64; 2]; 4]| {
                poly.map(|p| p[0] * axis[0] + p[1] * axis[1])
                    .into_iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
            };
            let ((alo, ahi), (blo, bhi)) = (project(a), project(b));
            if ahi <= blo + 0.01 || bhi <= alo + 0.01 {
                return false;
            }
        }
    }
    true
}

#[test]
fn dfw6_box_induction_is_one_pickup_cell_each() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json");
    let w = worldmap::load(&path).unwrap();
    let stations: Vec<_> = w.map.stations.iter().filter(|s| s.station_type == StationType::BoxInduction).collect();
    assert_eq!(stations.len(), 2);
    for station in stations {
        assert_eq!(
            station.presentation_points,
            vec![PresentationPoint { presentation_cell: station.position, queue_path: vec![] }]
        );
        let name = w.world.entities[&w.index.cell_entities[&station.position]].name.as_ref().unwrap();
        let prefix = name.strip_suffix(": presentation cell").unwrap();
        let cells = w
            .index
            .cell_entities
            .values()
            .filter(|id| w.world.entities[*id].name.as_ref().is_some_and(|n| n.starts_with(prefix)))
            .count();
        assert_eq!(cells, 1, "{prefix} has no decorative standing or queue cells");
    }
}

#[test]
fn resolved_machine_metadata_carries_waiting_presentations_and_inventory_queue() {
    for (fixture, count) in [("minimal_easyloop.json", 1), ("walmart_dfw6.json", 4)] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let w = worldmap::load(&path).unwrap();
        assert_eq!(w.map.conveyor_machines.len(), count);
        for machine in &w.map.conveyor_machines {
            let members: FxHashSet<_> = machine.cells.iter().copied().collect();
            let declared_waiting: FxHashSet<_> = machine
                .cells
                .iter()
                .copied()
                .filter(|cell| w.world.entities[&w.index.cell_entities[cell]].has("waiting"))
                .collect();
            assert_eq!(machine.waiting_cells.iter().copied().collect::<FxHashSet<_>>(), declared_waiting);
            assert_eq!(machine.waiting_cells.len(), 12);
            assert_eq!(machine.presentations.len(), 2);
            for station in
                w.map.stations.iter().filter(|station| station.conveyor_group == Some(machine.conveyor_group))
            {
                assert!(
                    station
                        .handoff_queues
                        .iter()
                        .all(|queue| !machine.presentations.iter().any(|p| p.cell == queue.into)),
                    "circulation cells are not exclusive order-presentation queues"
                );
            }
            assert_eq!(machine.inventory_holding_capacity(), 8);
            let queue = machine.inventory_queue.as_ref().unwrap();
            assert_eq!(queue.entry_from, machine.dropoff_cell);
            assert_eq!(Some(queue.into), machine.collection_cell);
            assert!(!queue.approach, "the inventory queue is a through route");
            assert!(machine.waiting_cells.iter().all(|cell| !queue.path.contains(cell)));
            for presentation in &machine.presentations {
                let station = w.map.station(presentation.station_id).unwrap();
                assert_eq!(station.conveyor_group, Some(machine.conveyor_group));
                assert_eq!(station.order_presentation_cell, Some(presentation.cell));
                let approach = presentation.approach_cell.unwrap();
                let departure = presentation.departure_cell.unwrap();
                assert!(
                    w.map
                        .graph
                        .neighbors(approach)
                        .iter()
                        .any(|(to, edge)| *to == presentation.cell && edge.curve.is_some())
                );
                assert!(
                    w.map
                        .graph
                        .neighbors(presentation.cell)
                        .iter()
                        .any(|(to, edge)| *to == departure && edge.curve.is_some())
                );
                assert_eq!(presentation.waiting_cells.len(), 6);
                for &wait in &machine.waiting_cells {
                    let owns = presentation.waiting_cells.contains(&wait);
                    assert_eq!(reachable(&w, wait, presentation.cell, &members), owns);
                    assert_eq!(reachable(&w, presentation.cell, wait, &members), owns);
                }
            }
        }
        let serialized = serde_json::to_vec(&w.map).unwrap();
        let restored: WarehouseMap = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored.conveyor_machines, w.map.conveyor_machines);
    }
}

#[test]
fn waiting_capacity_follows_markers_without_reading_names() {
    let mut document: Value = serde_json::from_str(include_str!("fixtures/minimal_easyloop.json")).unwrap();
    let mut retained = 0;
    for entity in document["entities"].as_array_mut().unwrap() {
        entity.as_object_mut().unwrap().remove("name");
        let components = entity["components"].as_object_mut().unwrap();
        if components.contains_key("waiting") {
            if retained >= 2 {
                components.remove("waiting");
            }
            retained += 1;
        }
    }
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("easyloop-two-waiting-locations.json");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let w = worldmap::load(&path).unwrap();
    let machine = &w.map.conveyor_machines[0];
    assert_eq!(machine.waiting_cells.len(), 2);
    assert!(machine.presentations.iter().all(|presentation| presentation.waiting_cells.len() == 1));
    assert_eq!(machine.inventory_holding_capacity(), 8);
    let members = machine.cells.iter().copied().collect();
    for &wait in &machine.waiting_cells {
        for presentation in &machine.presentations {
            let owns = presentation.waiting_cells.contains(&wait);
            assert_eq!(reachable(&w, wait, presentation.cell, &members), owns);
            assert_eq!(reachable(&w, presentation.cell, wait, &members), owns);
        }
    }
}
