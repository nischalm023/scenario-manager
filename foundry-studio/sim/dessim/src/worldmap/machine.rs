//! Within `worldmap`, `machine` is the ASSEMBLAGE reading: which entity IS one
//! piece of fixed equipment an HTM works at, and which cells belong to it.
//!
//! It exists because a machine is not a station and never was. An EasyLoop is
//! one belt, two pick places, two ports and a set of lanes, and the loader used
//! to find that assemblage by a shared derivation root — everything a stamp put
//! down — which only worked while the machine was stamped from a template. A
//! machine authored cell by cell has no stamp, so membership is stated
//! differently and stated once: the machine IS a frame, and every cell posed in
//! that frame is a member of it (Daniel, 2026-09-04).
//!
//! The predicates are not written here. They are `machine`, `machine_cell`,
//! `machine_pick_place` and `machine_port` in
//! `data/seed/application/lenses/world.lens`, and this module evaluates
//! them with `worldlens` — so what the simulator recognises and what the shipped
//! pattern library says a machine is are one statement rather than two that
//! drift. `machine_waiting_cell` identifies the standing locations separately
//! from travel cells and the inventory HTM holding queue. Directed reachability
//! in the resolved machine graph assigns each one to its presentation.
//!
//! What the patterns CANNOT say is "the nearest enclosing frame": `in_frame`
//! walks every ancestor by design, so a machine frame parented under another
//! machine's would put every cell in both. That is refused here rather than
//! narrowed — one cell, one machine — and the refusal names both machines.

use std::collections::{BTreeMap, VecDeque};
use std::sync::OnceLock;

use desgraph::{CardinalDirection, CellId, CircularArc, HtmTraversal, WorldPos};
use desmap::{ConveyorMachine, MachinePresentation, OrderHtmArea, WarehouseMap};
use worldlens::{Matcher, Library};
use site_model::components::product::Port;
use site_model::components::spatial::Pose;
use site_model::ids::EntityId;
use site_model::pipeline::World;

use rustc_hash::FxHashSet;

use super::cells::CellTable;
use super::stations::{self, posed_at};
use super::{StepMm, WorldMapError};
use crate::model::SimDuration;

/// The shipped pattern library, compiled into the simulator.
///
/// One text, read by the loader and by `worldlens`'s own pins. A pattern the
/// library cannot parse is a bug in a file this crate ships, not in a document,
/// so it panics rather than turning into a load error about somebody's map.
const PATTERNS: &str = include_str!("../../../../data/seed/application/lenses/world.lens");

fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| Library::parse(PATTERNS).expect("the shipped pattern library parses"))
}

/// One machine and the places on it a robot uses.
///
/// `group` is the dense index the conveyor model keys a belt by; it counts
/// machines in identity order, so adding one renumbers deterministically.
pub(super) struct Machine {
    pub entity: EntityId,
    pub group: u32,
    /// One circuit of the belt, from `go.easyloop.loop_transit_secs`.
    pub transit: SimDuration,
    /// Where an HTM sets a carrier onto the belt, and where it takes one back.
    pub dropoff: Option<CellId>,
    pub collect: Option<CellId>,
    /// What the belt admits, from the sink port's own `of`.
    pub accepts: Option<String>,
    /// The pick places: the cells carrying a `pick` happening, in reading order
    /// (north to south, then west to east), so a station index means the same
    /// seat across two runs of one document.
    pub pick_places: Vec<PickPlace>,
    /// Every cell posed in the machine's frame, sorted.
    pub cells: Vec<CellId>,
    pub waiting_cells: Vec<CellId>,
}

/// One place on a machine where an operator works a carrier an HTM holds.
pub(super) struct PickPlace {
    pub entity: EntityId,
    pub cell: CellId,
    pub at: [f64; 2],
    /// Straight departure direction, after completing a declared curve if any.
    /// The actual presentation heading remains on its pose and curve tangent.
    pub exit: CardinalDirection,
}

/// Every machine a world declares, with its cells and its pick places.
pub(super) fn census(world: &World, table: &CellTable, unit: StepMm) -> Result<Vec<Machine>, WorldMapError> {
    let library = library();
    let lens = Matcher::new(&world.entities);
    // ordered: keyed by machine identity, and the census below walks it in that
    // order to hand out the dense `conveyor_group` index. A hash map here would
    // renumber the belts between two runs of one document.
    let bound = |pattern: &str, var: &str, other: &str| -> BTreeMap<EntityId, Vec<EntityId>> {
        let parsed = library.get(pattern).unwrap_or_else(|| panic!("`{pattern}` is in the shipped library"));
        let mut out: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for binding in lens.matches(parsed) {
            let (Some(key), Some(value)) = (binding.get(var), binding.get(other)) else { continue };
            out.entry(key.clone()).or_default().push(value.clone());
        }
        out
    };

    let members = bound("machine_cell", "m", "c");
    let pick_places = bound("machine_pick_place", "m", "c");
    let ports = bound("machine_port", "m", "c");
    let waiting = bound("machine_waiting_cell", "m", "c");

    let mut out = Vec::new();
    let machine_pattern = library.get("machine").expect("`machine` is in the shipped library");
    let mut identities: Vec<EntityId> =
        lens.matches(machine_pattern).iter().filter_map(|b| b.get("m").cloned()).collect();
    identities.sort();
    identities.dedup();

    // ONE cell, ONE machine. The frame-reference walk is transitive, so a machine frame
    // parented under another machine's frame puts every one of the inner
    // machine's cells, ports and pick places in both machines at once, and the
    // outer machine's ports then overwrite the inner one's. A frame chain is a
    // path, so refusing the nesting is what makes the membership answer single.
    let is_machine: FxHashSet<&EntityId> = identities.iter().collect();
    for entity in &identities {
        let chain = world
            .frames
            .ancestry(entity)
            .map_err(|source| WorldMapError::Unreadable { entity: entity.clone(), message: source.to_string() })?;
        if let Some(outer) = chain.iter().skip(1).find(|id| is_machine.contains(id)) {
            return Err(WorldMapError::NestedMachines { inner: entity.clone(), outer: outer.clone() });
        }
    }

    for (group, entity) in identities.into_iter().enumerate() {
        let transit = transit(world, &entity).ok_or_else(|| WorldMapError::Unreadable {
            entity: entity.clone(),
            message: "the machine states a `go.easyloop` belt with no positive `loop_transit_secs`".into(),
        })?;
        let cell_of = |id: &EntityId| table.cells_by_entity.get(id).copied();

        let mut cells: Vec<CellId> = members.get(&entity).into_iter().flatten().filter_map(cell_of).collect();
        cells.sort_unstable();
        cells.dedup();
        let mut waiting_cells: Vec<CellId> = waiting.get(&entity).into_iter().flatten().filter_map(cell_of).collect();
        waiting_cells.sort_unstable();
        waiting_cells.dedup();

        let mut dropoff: Option<(EntityId, CellId)> = None;
        let mut collect: Option<(EntityId, CellId)> = None;
        let mut accepts = None;
        for id in ports.get(&entity).into_iter().flatten() {
            let Ok(Some(port)) = world.entities[id].component::<Port>("port") else { continue };
            // A port IS a cell: it is a standing spot an HTM drives to, and
            // resolving a posed one to whatever floor happens to lie nearest
            // could bind a machine's handover to another machine's lane.
            let cell = stations::port_cell(table, id)
                .ok_or_else(|| WorldMapError::PortIsNotACell { port: id.clone(), owner: entity.clone() })?;
            // One belt, one way in and one way out. Two sinks on one machine
            // used to leave whichever the scan reached last, silently.
            let held = match port.direction.as_str() {
                "sink" => {
                    accepts =
                        port.of.first().and_then(|o| o.get("designation")).and_then(|d| d.as_str()).map(str::to_owned);
                    &mut dropoff
                }
                "source" => &mut collect,
                // A direction the format grows later is skipped, not guessed at.
                _ => continue,
            };
            if let Some((first, _)) = held {
                return Err(WorldMapError::MachineWithTwoPorts {
                    machine: entity.clone(),
                    direction: port.direction.clone(),
                    first: first.clone(),
                    second: id.clone(),
                });
            }
            *held = Some((id.clone(), cell));
        }
        let dropoff = dropoff.map(|(_, cell)| cell);
        let collect = collect.map(|(_, cell)| cell);

        let mut seats: Vec<PickPlace> = Vec::new();
        for id in pick_places.get(&entity).into_iter().flatten() {
            let (Some(cell), Some(at)) = (cell_of(id), posed_at(world, &world.entities[id], unit)) else {
                return Err(WorldMapError::Unreadable {
                    entity: id.clone(),
                    message: "a pick place is a cell an HTM stands on, so it needs both a cell of its own and a \
                              pose in a frame"
                        .into(),
                });
            };
            seats.push(PickPlace { entity: id.clone(), cell, at, exit: exit_of(world, id)? });
        }
        seats.sort_by(|a, b| {
            a.at[1].total_cmp(&b.at[1]).then(a.at[0].total_cmp(&b.at[0])).then(a.entity.cmp(&b.entity))
        });

        out.push(Machine {
            entity,
            group: group as u32,
            transit,
            dropoff,
            collect,
            accepts,
            pick_places: seats,
            cells,
            waiting_cells,
        });
    }
    Ok(out)
}

/// One circuit of a machine's belt. `None` for an entity whose `go.easyloop`
/// states no positive transit, which is how a document says "this part of the
/// machine is scenery" rather than "this machine has an instantaneous belt".
fn transit(world: &World, entity: &EntityId) -> Option<SimDuration> {
    let secs = world.entities.get(entity)?.raw("go.easyloop")?.get("loop_transit_secs")?.as_f64()?;
    (secs > 0.0).then(|| SimDuration::from_secs(secs))
}

/// Which way an HTM leaves a pick place, read off the cell's own heading and
/// carried into the primary frame.
///
/// `heading_deg` absent means 0, which is `+x` OF THE CELL'S OWN FRAME. That
/// frame reaches the primary frame through a chain of scales, and a negative
/// scale mirrors: local 0 under `scale [-1, 1]` points along primary `-x`, which
/// is West and not East. So the heading travels as a DIRECTION -- the chain's
/// translations mean nothing to it, its scales mean everything -- and only then
/// becomes a compass point. Row centres increase along `+y` and desmap's north
/// is row-decreasing, so `+y` is South, the same mapping
/// `station.exit_direction`'s `-y` gets.
///
/// A direction that is not an axis after all that is REFUSED. A 45-degree
/// heading, or one a frame has sheared off the axes, has no cardinal to round
/// to, and rounding it to East silently sends the robot out the wrong side of
/// the machine.
fn exit_of(world: &World, entity: &EntityId) -> Result<CardinalDirection, WorldMapError> {
    let unreadable = |message: String| WorldMapError::Unreadable { entity: entity.clone(), message };
    let pose = world
        .entities
        .get(entity)
        .and_then(|e| e.component::<Pose>("pose").ok().flatten())
        .ok_or_else(|| unreadable("a pick place needs a pose for its exit heading to mean anything".into()))?;
    let frame = pose
        .frame
        .clone()
        .ok_or_else(|| unreadable("a pick place's pose states no frame, so its heading points nowhere".into()))?;
    let local = pose.heading_deg.rotate([1.0, 0.0]);
    let d = world.frames.direction_to_primary(&frame, local).map_err(|source| unreadable(source.to_string()))?;
    let length = d[0].hypot(d[1]);
    if length <= 0.0 {
        return Err(unreadable("the frame chain flattens this pick place's heading to no direction at all".into()));
    }
    let mut direction = [d[0] / length, d[1] / length];
    let departures: Vec<_> = world.edges.iter().filter(|edge| edge.from == *entity && edge.curve.is_some()).collect();
    if !departures.is_empty() {
        if departures.len() != 1 || departures[0].bidirectional {
            return Err(unreadable("a curved presentation departure must name one directed outgoing link".into()));
        }
        let edge = departures[0];
        let geometry = edge.curve.as_ref().expect("filtered curved departure");
        let arc = CircularArc {
            center: WorldPos { x_mm: geometry.center[0].0 as f64, y_mm: geometry.center[1].0 as f64 },
            clockwise: geometry.clockwise,
            start: geometry.start.map(|p| WorldPos { x_mm: p[0].0 as f64, y_mm: p[1].0 as f64 }),
            end: geometry.end.map(|p| WorldPos { x_mm: p[0].0 as f64, y_mm: p[1].0 as f64 }),
        };
        let point = |id: &EntityId| {
            world
                .cells
                .get(id)
                .map(|cell| WorldPos { x_mm: cell.centre[0].0 as f64, y_mm: cell.centre[1].0 as f64 })
                .ok_or_else(|| unreadable("curved departure endpoint has no cell geometry".into()))
        };
        let tangent = arc.start_tangent(point(entity)?);
        if (tangent[0] - direction[0]).hypot(tangent[1] - direction[1]) > 1e-4 {
            return Err(unreadable("presentation heading disagrees with its outgoing curve's tangent".into()));
        }
        // The old station field describes the straight departure after the
        // turn. The corridor binds to the explicit curved edge below; it must
        // never round the presentation's diagonal tangent to an axis.
        direction = arc.end_tangent(point(&edge.to)?);
    }
    let [x, y] = direction;
    // A unit vector is on an axis when its OTHER component is zero, and that
    // is the component the tolerance has to be read on: the one along the axis
    // is a cosine, which stays within 1e-6 of 1 for every heading inside about
    // a twentieth of a degree, so testing it accepted a heading of 0.05° as
    // due East. On the other component the tolerance is a sine -- the distance
    // off the axis itself -- and 1e-6 is about two ten-thousandths of a degree.
    // A unit vector cannot have both components that small, so at most one arm
    // matches.
    const AXIS_TOLERANCE: f64 = 1e-6;
    match (y.abs() <= AXIS_TOLERANCE, x.abs() <= AXIS_TOLERANCE) {
        (true, _) if x > 0.0 => Ok(CardinalDirection::East),
        (true, _) => Ok(CardinalDirection::West),
        (_, true) if y > 0.0 => Ok(CardinalDirection::South),
        (_, true) => Ok(CardinalDirection::North),
        _ => Err(WorldMapError::ExitHeadingIsNotCardinal {
            place: entity.clone(),
            heading_deg: pose.heading_deg.0,
            primary: [x, y],
        }),
    }
}

/// Bind semantic membership to the final graph, after corridor restrictions.
/// Missing curved anchors remain absent: a runtime requiring curved presentation
/// must reject incomplete equipment instead of guessing a nearby cell.
pub(super) fn resolved(machine: &Machine, conveyor_group: u32, map: &WarehouseMap) -> ConveyorMachine {
    let stations: Vec<_> =
        map.stations.iter().filter(|station| station.conveyor_group == Some(conveyor_group)).collect();
    let inventory_queue = stations
        .iter()
        .flat_map(|station| &station.handoff_queues)
        .find(|queue| Some(queue.into) == machine.collect)
        .cloned();
    let unique = |cells: Vec<CellId>| match cells.as_slice() {
        [cell] => Some(*cell),
        _ => None,
    };
    let mut presentations: Vec<_> = stations
        .iter()
        .filter_map(|station| {
            let cell = station.order_presentation_cell?;
            let waiting_cells: Vec<_> = machine
                .waiting_cells
                .iter()
                .copied()
                .filter(|&waiting| {
                    machine_reachable(map, &machine.cells, waiting, cell)
                        && machine_reachable(map, &machine.cells, cell, waiting)
                })
                .collect();
            let order_area = order_area(map, &machine.cells, cell, &waiting_cells);
            Some(MachinePresentation {
                station_id: station.id,
                inventory_cell: station.position,
                cell,
                waiting_cells,
                order_area,
                approach_cell: unique(
                    map.graph
                        .edges()
                        .filter_map(|(from, to, edge)| (to == cell && edge.curve.is_some()).then_some(from))
                        .collect(),
                ),
                departure_cell: unique(
                    map.graph
                        .neighbors(cell)
                        .into_iter()
                        .filter_map(|(to, edge)| edge.curve.is_some().then_some(to))
                        .collect(),
                ),
            })
        })
        .collect();
    presentations.sort_by_key(|presentation| presentation.station_id);
    ConveyorMachine {
        conveyor_group,
        cells: machine.cells.clone(),
        waiting_cells: machine.waiting_cells.clone(),
        dropoff_cell: machine.dropoff,
        collection_cell: machine.collect,
        inventory_queue,
        presentations,
    }
}

fn order_area(
    map: &WarehouseMap,
    members: &[CellId],
    presentation: CellId,
    waiting: &[CellId],
) -> Option<OrderHtmArea> {
    let mut resident_cells: Vec<_> = members
        .iter()
        .copied()
        .filter(|&cell| {
            machine_reachable(map, members, presentation, cell) && machine_reachable(map, members, cell, presentation)
        })
        .collect();
    resident_cells.sort_unstable();
    let residents: FxHashSet<_> = resident_cells.iter().copied().collect();
    if waiting.is_empty() || !waiting.iter().all(|cell| residents.contains(cell)) {
        return None;
    }
    let members: FxHashSet<_> = members.iter().copied().collect();
    let mut entries = FxHashSet::default();
    let mut exits = FxHashSet::default();
    for (from, to, edge) in map.graph.edges() {
        if edge.traversability.htm_traversal != HtmTraversal::Both || !members.contains(&from) || !members.contains(&to)
        {
            continue;
        }
        if !residents.contains(&from) && residents.contains(&to) {
            entries.insert(from);
        }
        if residents.contains(&from) && !residents.contains(&to) {
            exits.insert(to);
        }
    }
    if entries.len() != 1 || exits.len() != 1 {
        return None;
    }
    let entry = *entries.iter().next()?;
    let egress = *exits.iter().next()?;
    (entry != egress).then_some(OrderHtmArea { entry, egress, resident_cells })
}

fn machine_reachable(map: &WarehouseMap, members: &[CellId], from: CellId, to: CellId) -> bool {
    let members: FxHashSet<_> = members.iter().copied().collect();
    let mut seen = FxHashSet::default();
    let mut pending = VecDeque::from([from]);
    while let Some(cell) = pending.pop_front() {
        if cell == to {
            return true;
        }
        if !seen.insert(cell) {
            continue;
        }
        pending.extend(
            map.graph
                .neighbors(cell)
                .into_iter()
                .filter(|(_, edge)| {
                    edge.traversability.htm_traversal == HtmTraversal::Both
                        && edge.dest_only.is_none_or(|gate| gate.admits(to))
                })
                .map(|(next, _)| next)
                .filter(|next| members.contains(next) && !seen.contains(next)),
        );
    }
    false
}
