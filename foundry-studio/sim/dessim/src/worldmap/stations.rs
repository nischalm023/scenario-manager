//! Within `worldmap`, `stations` is the OPERATOR ENDPOINT reading: which cell
//! each workstation stands on, which cells a carrier is presented at, and which
//! cells each approach lane runs over.
//!
//! A world document says all three separately — the station has a pose, a floor
//! cell carries a `presentation_point`, and a lane, an entity of its own, names
//! its cells by identity — where desmap's authoring format packed them into one
//! record of grid coordinates. This module reads the three statements into
//! dessim's one `Station` and the cell *labels* (`Presentation`, `QueueCell`,
//! `StationArea`); `worldmap::load` then cuts each lane into the graph as the
//! one-way corridor `PORT-NOTES.md` §5.2 describes.
//!
//! An endpoint arrives here in one of two spellings, and the module reads both:
//!
//! - an entity carrying `station`, as in cencora, walmart-dallas, and DFW6's
//!   induction/extraction points;
//! - a PICK PLACE on a machine — a cell carrying the `pick` happening, found
//!   through `machine`. `station` is a lens word for the assemblage, not a
//!   component (Daniel, 2026-09-04), so a machine authored cell by cell carries
//!   none and the happening lives on the cell where it happens.
//!
//! Both produce the same `desmap::Station`, and `StationType` stays the engine's
//! derived classification of one.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};

use desgraph::{CardinalDirection, CellId, CellKind, StationId, StationType};
use desmap::{HandoffQueue, OutboundRole, PresentationPoint, Station};
use site_model::components::product::{CellAddress, Port, Queue, Station as WorldStation};
use site_model::components::spatial::Pose;
use site_model::entity::Entity;
use site_model::ids::{DeclaredEntityId, EntityId};
use site_model::pipeline::World;

use super::cells::CellTable;
use super::machine::{self, Machine};
use super::{StepMm, WorldMapError};
use crate::model::SimDuration;

/// The stations of a world plus the cell labels they claim.
pub(super) struct Stations {
    pub stations: Vec<Station>,
    pub machines: Vec<(u32, Machine)>,
    pub by_id: BTreeMap<StationId, EntityId>,
    /// Cells a station owns, with the kind that says how it owns them. Applied
    /// to the table after the fact, because resolving a station needs the cells
    /// to exist first.
    pub decorations: Vec<(CellId, CellKind, StationId)>,
}

/// One operator endpoint, before it has a `StationId`: either an entity the
/// document gave a `station` component, or a pick place on a machine.
enum Seat {
    Declared,
    /// Indices into the machine census and into that machine's pick places.
    PickPlace(usize, usize),
}

/// Read every operator endpoint into dessim's shape.
pub(super) fn build(world: &World, table: &CellTable, unit: StepMm) -> Result<Stations, WorldMapError> {
    let machines = machine::census(world, table, unit)?;
    let presentations = declared_presentation_points(world, table, unit);
    let lanes = lanes(world, table, &machines, &presentations)?;

    // ONE list, sorted once. A `StationId` is a position in it, so the two
    // spellings have to interleave by where each endpoint stands rather than by
    // which spelling it came from -- otherwise adding a machine to a document
    // renumbers every station that was already there. Use the axes of their
    // common enclosing frame: mirroring the whole floor keeps its numbering,
    // while a machine's local mirror or units cannot change the site's order.
    let mut found: Vec<(Seat, EntityId, [f64; 2])> = Vec::new();
    for entity in world.entities.values() {
        if !entity.has("station") {
            continue;
        }
        let pose = entity
            .component::<Pose>("pose")
            .map_err(|source| WorldMapError::Unreadable { entity: entity.id.clone(), message: source.to_string() })?;
        let Some((frame, xy)) = pose.and_then(|p| Some((p.frame.clone()?, p.xy()?))) else {
            return Err(WorldMapError::Unreadable {
                entity: entity.id.clone(),
                message: "a station must be posed in a frame for a robot to drive to it".into(),
            });
        };
        let position =
            unit.point(world.frames.to_primary(&frame, xy).map_err(|source| WorldMapError::Unreadable {
                entity: entity.id.clone(),
                message: source.to_string(),
            })?);
        found.push((Seat::Declared, entity.id.clone(), position));
    }
    for (index, machine) in machines.iter().enumerate() {
        for (place, seat) in machine.pick_places.iter().enumerate() {
            found.push((Seat::PickPlace(index, place), seat.entity.clone(), seat.at));
        }
    }
    let axes = ordering_axes(world, found.iter().map(|(_, id, _)| id))?;
    let key = |position: [f64; 2]| axes.map(|axis| position[0] * axis[0] + position[1] * axis[1]);
    found.sort_by(|a, b| {
        let (a_key, b_key) = (key(a.2), key(b.2));
        a_key[1].total_cmp(&b_key[1]).then(a_key[0].total_cmp(&b_key[0])).then(a.1.cmp(&b.1))
    });

    let ports = carrier_ports(world, table, &machines)?;
    // Every DECLARED machine half that states a belt, sorted so the index is
    // stable. This is the stamped spelling (DFW6's); a machine found through
    // `machine::census` numbers its belt after these, so the two never collide
    // in one document.
    let mut stamped: Vec<DeclaredEntityId> = found
        .iter()
        .filter(|(seat, id, _)| {
            matches!(seat, Seat::Declared) && world.entities.get(id).is_some_and(|e| conveyor_transit(e).is_some())
        })
        .map(|(_, id, _)| id.root())
        .collect();
    stamped.sort();
    stamped.dedup();
    let mut out =
        Stations { stations: Vec::new(), machines: Vec::new(), by_id: BTreeMap::new(), decorations: Vec::new() };

    for (index, (seat, entity_id, position)) in found.iter().enumerate() {
        let id = StationId(index as u32 + 1);
        let mut station = match seat {
            Seat::Declared => {
                declared_station(world, table, entity_id, *position, &presentations, &ports, &lanes, &stamped)?
            }
            Seat::PickPlace(machine, place) => machine_station(&machines[*machine], *place, stamped.len(), &lanes),
        };
        station.id = id;

        // A port cell keeps the kind `way_kind` already gave it: `Exchange`,
        // a belt handoff, which is not an operator presentation and must not
        // be painted as one. Only a station that presents directly claims the
        // Presentation kind for its cells.
        for cell in station.presentation_cells() {
            if Some(cell) != station.dropoff_cell {
                out.decorations.push((cell, CellKind::Presentation, id));
            }
        }
        // A lane's steps are labelled `QueueCell`, but a PORT that a lane runs
        // through keeps `Exchange`: what the cell IS is a belt handover, and
        // the label is what desvis paints it by and what a reader looks for
        // when asking where a machine's doorway is.
        //
        // No document in the tree reaches this any more. A machine's inventory
        // lane used to START on its dropoff and the skip is what stopped that
        // handover being repainted; since the 2026-09-04 layout ruling the
        // dropoff is the lane's ENTRY cell and the collection its TERMINUS, so
        // neither is a step. The check stays because nothing REFUSES a lane
        // that names a port mid-path, and a document that did would otherwise
        // repaint a handover without saying so.
        let handovers = [station.dropoff_cell, station.collection_cell];
        let steps = station.presentation_points.iter().flat_map(|point| &point.queue_path);
        for &cell in steps.chain(station.handoff_queues.iter().flat_map(|q| &q.path)) {
            if handovers.contains(&Some(cell)) {
                continue;
            }
            out.decorations.push((cell, CellKind::QueueCell, id));
        }
        if !station.presentation_and_queue_cells().any(|cell| cell == station.position) {
            let kind = if station.order_presentation_cell == Some(station.position) {
                CellKind::Presentation
            } else {
                CellKind::StationArea
            };
            out.decorations.push((station.position, kind, id));
        }

        out.by_id.insert(id, entity_id.clone());
        out.stations.push(station);
    }

    // A lane no machine owns is exactly one station's approach. `lanes`
    // admitted its terminus because a `presentation_point` sits on that cell;
    // WHICH station presents there is only known once the stations are
    // resolved, so a point no station selects -- or one two stations select --
    // is caught here and nowhere earlier. So is a lane over a cell a station
    // presents at: its corridor gates every arc onto its steps to the robots
    // bound for its end, which would close that cell to the robots bound there.
    for lane in lanes.iter().filter(|lane| lane.machine.is_none()) {
        for &step in &lane.path {
            let presenting =
                out.stations.iter().zip(out.by_id.values()).find(|(s, _)| s.presentation_cells().any(|c| c == step));
            if let Some((_, station)) = presenting {
                return Err(WorldMapError::LaneRunsThroughPresentation {
                    lane: lane.entity.clone(),
                    station: station.clone(),
                    cell: step,
                });
            }
        }
        let mut serving = out
            .stations
            .iter()
            .zip(out.by_id.values())
            .filter(|(station, _)| {
                station.handoff_queues.iter().any(|queue| queue.into == lane.terminus)
                    || station.presentation_cells().any(|cell| cell == lane.terminus)
            })
            .map(|(_, entity_id)| entity_id);
        let Some(first) = serving.next() else {
            return Err(WorldMapError::LaneServesNoStation { lane: lane.entity.clone(), cell: lane.terminus });
        };
        if let Some(second) = serving.next() {
            return Err(WorldMapError::LaneServesTwoStations {
                lane: lane.entity.clone(),
                first: first.clone(),
                second: second.clone(),
                cell: lane.terminus,
            });
        }
    }
    out.machines = machines.into_iter().map(|machine| (stamped.len() as u32 + machine.group, machine)).collect();
    Ok(out)
}

/// One basis for every endpoint, in physical units. The nearest shared frame
/// preserves floor orientation without letting station-local transforms assign
/// incomparable sort keys. All frame chains have already been checked to end
/// at the primary frame, so nonempty endpoint sets have a common ancestor.
fn ordering_axes<'a>(
    world: &World,
    endpoints: impl Iterator<Item = &'a EntityId>,
) -> Result<[[f64; 2]; 2], WorldMapError> {
    let mut common: Option<Vec<EntityId>> = None;
    for id in endpoints {
        let unreadable = |message: String| WorldMapError::Unreadable { entity: id.clone(), message };
        let frame = world.entities[id]
            .component::<Pose>("pose")
            .map_err(|source| unreadable(source.to_string()))?
            .and_then(|pose| pose.frame)
            .ok_or_else(|| unreadable("a station must be posed in a frame to order it".into()))?;
        let ancestry = world.frames.ancestry(&frame).map_err(|source| unreadable(source.to_string()))?;
        match &mut common {
            None => common = Some(ancestry),
            Some(frames) => frames.retain(|frame| ancestry.contains(frame)),
        }
    }
    let mut axes = [[1.0, 0.0], [0.0, 1.0]];
    if let Some(frame) = common.as_ref().and_then(|frames| frames.first()) {
        let unreadable = |message: String| WorldMapError::Unreadable { entity: frame.clone(), message };
        for axis in &mut axes {
            let direction =
                world.frames.direction_to_primary(frame, *axis).map_err(|source| unreadable(source.to_string()))?;
            let length = direction[0].hypot(direction[1]);
            if !length.is_finite() || length <= 0.0 {
                return Err(unreadable("the station ordering frame must have finite, nonzero axes".into()));
            }
            *axis = direction.map(|coordinate| coordinate / length);
        }
    }
    Ok(axes)
}

/// The endpoint an entity carrying `station` states.
///
/// This is the reading every document in the tree but the minimal EasyLoop map
/// uses today, DFW6's stamped machine halves included, and it stays exactly as
/// it was: the port pair is found by shared derivation root, the belt timing by
/// a `go.easyloop` block on the station itself.
#[allow(clippy::too_many_arguments)]
fn declared_station(
    world: &World,
    table: &CellTable,
    entity_id: &EntityId,
    position: [f64; 2],
    presentations: &[(CellId, Option<EntityId>, [f64; 2], Option<String>)],
    ports: &[(CellId, Option<EntityId>, [f64; 2], PortDirection)],
    lanes: &[Lane],
    stamped: &[DeclaredEntityId],
) -> Result<Station, WorldMapError> {
    let entity = &world.entities[entity_id];
    let declared: WorldStation = entity
        .require("station")
        .map_err(|source| WorldMapError::Unreadable { entity: entity_id.clone(), message: source.to_string() })?;
    let station_type = match declared.kind.as_str() {
        "pick" => StationType::Pick,
        "put" | "putaway" => StationType::Putaway,
        // The world-protocol catalog declares exactly `pick`, `induct`
        // and `extract` (data/seed/application-tests/native/fixtures/
        // walmart-dallas/catalog.json), so those are the spellings. Both
        // register as real stations -- they render, the pins count them,
        // the inspector finds them -- but they join neither the pick nor
        // the putaway demand list, so dispatch never targets them. Inert
        // by construction, not by being dropped at load. The order-box
        // flow itself is gaps-plan Gap 4.
        "induct" => StationType::BoxInduction,
        "extract" => StationType::Extraction,
        "pickup" => StationType::ContainerPickup,
        other => {
            return Err(WorldMapError::UnrunnableStationKind { station: entity_id.clone(), kind: other.to_string() });
        }
    };
    let outbound_role = declared
        .role
        .as_deref()
        .map(|role| {
            let parsed = match role {
                "route_dispense" => OutboundRole::RouteDispense,
                "bagging" => OutboundRole::Bagging,
                "flex" => OutboundRole::Flex,
                "sealer_dropoff" => OutboundRole::SealerDropoff,
                "sealed_box_pickup" => OutboundRole::SealedBoxPickup,
                _ => {
                    return Err(WorldMapError::Unreadable {
                        entity: entity_id.clone(),
                        message: format!("unsupported station role {role:?}"),
                    });
                }
            };
            let expected = if parsed == OutboundRole::SealedBoxPickup {
                StationType::ContainerPickup
            } else {
                StationType::Extraction
            };
            if station_type != expected {
                return Err(WorldMapError::Unreadable {
                    entity: entity_id.clone(),
                    message: format!("station role {role:?} requires {expected:?}"),
                });
            }
            Ok(parsed)
        })
        .transpose()?;

    // Where the carrier this station works on is handed over. A station
    // that declares a SINK port names that cell itself: an EasyLoop's
    // inventory tote arrives on the belt, and its `presentation_point`s
    // are for the ORDER carrier, a different thing at a different cell.
    // Stations with no port keep the presentation point, which is every
    // station in every other document.
    // A port belongs to the MACHINE, not to one of its stations. An
    // EasyLoop has one ingress-egress pair (Kalpesh: "there's an
    // ingress-egress pair for the conveyor itself") shared by both its
    // stations, so the match is on the declaring entity rather than on the
    // station: whichever station the port names, both halves get the cell.
    let port_cell = |want: PortDirection| {
        ports
            .iter()
            .find(|(_, owner, _, dir)| *dir == want && owner.as_ref().is_some_and(|o| o.root() == entity_id.root()))
            .map(|(cell, _, _, _)| *cell)
    };
    let dropoff_cell = port_cell(PortDirection::Sink);
    // What the belt accepts, stated by the port itself. Unread until now,
    // which is how an order tote came to be boarded onto the inventory
    // belt: the deposit took whatever stood on the cell.
    let conveyance_accepts = dropoff_cell.and_then(|_| port_designation(world, entity_id));
    let collection_cell = port_cell(PortDirection::Source);

    // The cells this station's points name, ordered by cell id. A station names
    // one point, or none and takes the nearest; Dematic's pick station names
    // two, one per lane. A point counts once per cell, and a point that names
    // its station is believed whatever carrier it is for.
    let mut named: Vec<CellId> = presentations
        .iter()
        .filter(|(_, owner, _, _)| owner.as_ref() == Some(entity_id))
        .map(|(cell, _, _, _)| *cell)
        .collect();
    named.dedup();
    let presentation_cell = dropoff_cell
        .or_else(|| named.first().copied())
        .or_else(|| nearest_presentation(presentations, position).map(|(cell, _, _, _)| *cell))
        .ok_or_else(|| WorldMapError::StationWithoutPresentation { station: entity_id.clone() })?;

    // Where an ORDER carrier presents, as distinct from where an
    // inventory tote is handed to the belt. An EasyLoop declares one per
    // station (`for: order_carrier`, the horseshoe head); the two are
    // metres apart, and close-out had been delivering to the belt's
    // dropoff cell because that is what `presentation_cell` resolves to.
    let order_presentation_cell = presentations
        .iter()
        .filter(|(_, _, _, role)| role.as_deref() == Some("order_carrier"))
        .min_by(|a, b| {
            let d = |p: &[f64; 2]| (p[0] - position[0]).hypot(p[1] - position[1]);
            d(&a.2).partial_cmp(&d(&b.2)).unwrap_or(Ordering::Equal)
        })
        .map(|(cell, _, _, _)| *cell);

    // The cells this station presents at. A belt's station presents at its
    // dropoff whatever points name it, and the lanes that serve its belt are
    // its handoff queues below.
    let (cells, feeders) = match dropoff_cell {
        Some(_) => (vec![presentation_cell], &[][..]),
        None if named.is_empty() => (vec![presentation_cell], lanes),
        None => (named, lanes),
    };
    let presentation_points = presentation_approaches(entity_id, &cells, feeders)?;
    // Only the cells this station OWNS. `order_presentation_cell` above is a
    // nearest-point match, so every station in the document resolves to some
    // machine's horseshoe head; a station with no belt does not own one, and
    // binding it the machine's lane would give three inert stations a copy of
    // an approach they never drive.
    let owns_order_presentation = conveyor_transit(entity).is_some().then_some(order_presentation_cell).flatten();
    let handoff_queues = lanes_serving(lanes, &[dropoff_cell, collection_cell, owns_order_presentation]);
    let station_cell = nearest_way_cell(table, position);

    Ok(Station {
        // Stamped by `build`, which is the one place that knows the order.
        id: StationId(0),
        outbound_role,
        position: station_cell.unwrap_or(presentation_cell),
        presentation_points,
        edge: None,
        handoff_queues,
        exit_direction: exit_direction(world, entity_id, entity)?,
        bins: bins(entity),
        station_type,
        dropoff_cell,
        collection_cell,
        order_presentation_cell,
        conveyance_accepts,
        conveyor_transit_ms: conveyor_transit(entity).map(|d| d.as_ms()),
        // Both stations stamped from one EasyLoop share its declared
        // entity, so the root is the machine.
        conveyor_group: conveyor_transit(entity).map(|_| machine_index(stamped, entity_id)),
    })
}

/// The lane into each cell a station presents at: the lane no machine owns
/// that ends there, or none, since a presentation cell need not have a queue.
/// A machine's lanes belong to the machine, and reach its halves through its
/// handover cells instead. A lane ending at a port is refused: it is a belt's
/// through route, whose door admits the robots bound for the belt's handovers,
/// and a station without a belt has none.
fn presentation_approaches(
    station: &EntityId,
    cells: &[CellId],
    lanes: &[Lane],
) -> Result<Vec<PresentationPoint>, WorldMapError> {
    let lane_into = |cell: CellId| lanes.iter().find(|lane| lane.machine.is_none() && lane.terminus == cell);
    cells
        .iter()
        .map(|&cell| match lane_into(cell) {
            Some(lane) if !lane.approach => Err(WorldMapError::LaneEndsAtPortWithoutBelt {
                lane: lane.entity.clone(),
                station: station.clone(),
                cell,
            }),
            lane => Ok(PresentationPoint {
                presentation_cell: cell,
                queue_path: lane.map(|lane| lane.path.clone()).unwrap_or_default(),
            }),
        })
        .collect()
}

/// One pick place of a machine, as dessim's station.
///
/// Everything but the seat itself belongs to the MACHINE and is shared with its
/// other halves: one belt, one dropoff, one collection cell. What the seat
/// contributes is where an order carrier is held out to the operator, which way
/// the HTM leaves afterwards, and the lane that serves it.
fn machine_station(machine: &Machine, place: usize, stamped: usize, lanes: &[Lane]) -> Station {
    let seat = &machine.pick_places[place];
    // The presentation cell is the belt handover, not the seat: what this
    // station's operator is handed is an inventory tote off the conveyor, and
    // the seat is where the ORDER carrier stands. Same split as a stamped
    // EasyLoop half's, reached without a `station` component.
    let presentation_cell = machine.dropoff.unwrap_or(seat.cell);
    Station {
        // Stamped by `build`, as above.
        id: StationId(0),
        outbound_role: None,
        position: seat.cell,
        edge: None,
        // A machine's halves have no approach of their own: every lane it
        // declares serves one of its handover cells, so all of them are
        // handoff queues.
        presentation_points: vec![PresentationPoint { presentation_cell, queue_path: Vec::new() }],
        handoff_queues: lanes_serving(lanes, &[machine.dropoff, machine.collect, Some(seat.cell)]),
        exit_direction: seat.exit,
        bins: None,
        station_type: StationType::Pick,
        dropoff_cell: machine.dropoff,
        collection_cell: machine.collect,
        order_presentation_cell: Some(seat.cell),
        conveyance_accepts: machine.accepts.clone(),
        conveyor_transit_ms: Some(machine.transit.as_ms()),
        conveyor_group: Some(stamped as u32 + machine.group),
    }
}

/// The declared lanes that terminate at one of these cells, as approach queues.
///
/// A lane says what it serves by ENDING there (the 2026-09-02 framing ruling),
/// so binding is by identity of the last cell and nothing else -- no `into`
/// field to keep in step with the path, and no nearest-cell search.
fn lanes_serving(lanes: &[Lane], cells: &[Option<CellId>]) -> Vec<HandoffQueue> {
    lanes.iter().filter(|lane| cells.contains(&Some(lane.terminus))).map(Lane::as_queue).collect()
}

/// An entity's pose in primary-frame millimetres, when it has one. Shared with
/// `census`, which points a reader at a posed entity the same way a station
/// binds to the floor under it.
pub(super) fn posed_at(world: &World, entity: &Entity, unit: StepMm) -> Option<[f64; 2]> {
    let pose = entity.component::<Pose>("pose").ok().flatten()?;
    let (frame, xy) = (pose.frame.clone()?, pose.xy()?);
    Some(unit.point(world.frames.to_primary(&frame, xy).ok()?))
}

/// Every carrier port cell, the station it belongs to, and its direction.
///
/// A port is where a carrier changes hands with fixed conveyance — an
/// EasyLoop's dropoff and collection cells. BOTH directions are collected: the sink
/// is the arrival, where an HTM sets its tote on the belt and leaves, and the
/// source is the return, where an HTM lifts it back off. Reading only the sink
/// (which this did until the dropoff/collection work) leaves the source cell bound to
/// nothing, so the belt has no far end.
///
/// `cells.rs` gives any cell whose entity carries `port` the kind
/// `CellKind::Exchange`, which is what makes one usable as a handover cell and
/// what desvis paints sea green.
fn carrier_ports(
    world: &World,
    table: &CellTable,
    machines: &[Machine],
) -> Result<Vec<(CellId, Option<EntityId>, [f64; 2], PortDirection)>, WorldMapError> {
    let mut out = Vec::new();
    for entity in world.entities.values() {
        let Ok(Some(port)) = entity.component::<Port>("port") else { continue };
        let direction = match port.direction.as_str() {
            "sink" => PortDirection::Sink,
            "source" => PortDirection::Source,
            // A direction the format grows later is skipped, not guessed at.
            _ => continue,
        };
        let owner = match direction {
            PortDirection::Sink => port.into.clone(),
            PortDirection::Source => port.from.clone(),
        };
        let cell = match port_cell(table, &entity.id) {
            Some(cell) => cell,
            None => {
                // WHOSE port this is, so a reader knows where to look. The
                // machine whose frame it is posed in answers first -- that is
                // membership, and it holds even for a port that names nothing
                // -- and the `into`/`from` it declares answers otherwise. Only
                // a port that is posed nowhere and names nobody falls back to
                // itself, and there the document has said nothing more.
                let owner = match enclosing_machine(world, machines, entity)? {
                    Some(index) => machines[index].entity.clone(),
                    None => owner.unwrap_or_else(|| entity.id.clone()),
                };
                return Err(WorldMapError::PortIsNotACell { port: entity.id.clone(), owner });
            }
        };
        let Some(centre) = table.centres.get(&cell) else { continue };
        out.push((cell, owner, *centre, direction));
    }
    out.sort_by_key(|(cell, _, _, _)| cell.0);
    Ok(out)
}

/// The cell a port entity IS.
///
/// One function, used wherever a port has to become a cell: `carrier_ports`
/// binding a machine's belt ends, `machine::census` reading the same two, and
/// the lane classification deciding whether a terminus is a handover. They
/// disagreed before -- one accepted a posed port and bound it to the nearest
/// floor, the other insisted the port entity be the cell -- so a through lane
/// ending at a resolved cell was marked an approach and closed to every robot
/// but one.
///
/// A port is a DECLARED CELL: it is a standing spot an HTM drives to and waits
/// on, and a nearest-cell search for one could put a machine's handover on
/// another machine's floor. `None` here is a refusal at both call sites, not a
/// fallback.
pub(super) fn port_cell(table: &CellTable, port: &EntityId) -> Option<CellId> {
    table.cells_by_entity.get(port).copied()
}

/// Which way a carrier crosses a port, from the robot's point of view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PortDirection {
    /// The HTM puts a carrier down here and leaves: an EasyLoop's DROPOFF cell.
    Sink,
    /// The HTM takes a carrier from here: an EasyLoop's COLLECTION cell.
    Source,
}

/// Every cell carrying a `presentation_point`, with the station it names when
/// it names one. SPEC §10 leaves explicit binding open and binds by proximity in
/// v0; a document that states the binding is believed, and one that does not
/// falls back to the nearest point.
fn declared_presentation_points(
    world: &World,
    table: &CellTable,
    unit: StepMm,
) -> Vec<(CellId, Option<EntityId>, [f64; 2], Option<String>)> {
    let mut out = Vec::new();
    for entity in world.entities.values() {
        let Some(raw) = entity.raw("presentation_point") else { continue };
        // Either the entity IS a cell -- the override form, where a document
        // names a derived cell by column and row -- or it is posed, and binds
        // to the nearest way cell the way a relay's stand point does. Prefer
        // posing: a column and row is a coordinate into a decomposition, so it
        // silently means somewhere else if the pitch ever changes.
        let cell = match table.cells_by_entity.get(&entity.id) {
            Some(&cell) => cell,
            None => match posed_at(world, entity, unit).and_then(|at| nearest_way_cell(table, at)) {
                Some(cell) => cell,
                None => continue,
            },
        };
        let Some(centre) = table.centres.get(&cell) else { continue };
        let owner = raw.get("station").and_then(|value| serde_json::from_value::<EntityId>(value.clone()).ok());
        // What the point presents. An EasyLoop declares `order_carrier` for
        // the horseshoe head, which is a different place from the inventory
        // handoff and had never been distinguished.
        let role = raw.get("for").and_then(|v| v.as_str()).map(str::to_owned);
        out.push((cell, owner, *centre, role));
    }
    out.sort_by_key(|(cell, _, _, _)| cell.0);
    out
}

fn nearest_presentation(
    points: &[(CellId, Option<EntityId>, [f64; 2], Option<String>)],
    position: [f64; 2],
) -> Option<&(CellId, Option<EntityId>, [f64; 2], Option<String>)> {
    points.iter().min_by(|a, b| {
        let distance = |p: &(CellId, Option<EntityId>, [f64; 2], Option<String>)| {
            (p.2[0] - position[0]).hypot(p.2[1] - position[1])
        };
        distance(a).total_cmp(&distance(b))
    })
}

/// One declared lane: an ordered walk over cells, the place it serves, and who
/// declared it.
///
/// A lane is an entity of its own carrying a `queue`, never a tiling generating
/// the floor it runs over -- the shape a machine authored cell by cell uses for
/// each of its approaches. What it serves is the LAST cell of its own path, so a
/// document states the target once, in the place a reader is already looking.
///
/// It keeps its DECLARING ENTITY so a refusal can name it, and the MACHINE it
/// belongs to so that binding is ownership rather than coincidence: a lane laid
/// over one machine's cells may not end at another machine's, and the loader
/// says which lane and which two machines rather than letting the second claim
/// it.
pub(super) struct Lane {
    /// The entity carrying the `queue`. Named by every refusal about the lane.
    pub entity: EntityId,
    /// The machine every cell of the lane belongs to. `None` for a plain
    /// station's approach, which runs over ordinary floor and belongs to the
    /// station its terminus presents at.
    pub machine: Option<EntityId>,
    pub terminus: CellId,
    /// The steps before the terminus, entry first.
    pub path: Vec<CellId>,
    /// Whether the lane is an approach into one standing spot; see
    /// `HandoffQueue::approach`.
    pub approach: bool,
}

impl Lane {
    /// The lane as the map's shape for a handoff queue: a lane into a belt's
    /// handover cell or into an order presentation cell.
    fn as_queue(&self) -> HandoffQueue {
        // `entry_from` is the graph's answer and the graph is not built yet;
        // `worldmap::load` fills it in from the resolved corridors.
        HandoffQueue { into: self.terminus, path: self.path.clone(), approach: self.approach, entry_from: None }
    }
}

/// Every lane a world declares, bound to the place it ends at and owned by
/// whoever declared it.
///
/// A queue on an entity that is also a station, a presentation point, a port, a
/// relay or a cell (a port is named as one, though it is a cell too) is
/// refused: the cells a robot queues on to reach a cell are a lane, an entity
/// of its own, the one way a document states them. A queue on a CELL GRID is
/// an older stamped form -- the Walmart Dallas catalog's order queue is a
/// tiled region that also carries a path -- and is left alone: re-authoring
/// Dallas is its own job.
///
/// Each of these is REFUSED here, where the older reading let it through
/// without a word:
///
/// - a `queue` that does not parse, which used to be skipped as if it were
///   absent;
/// - a lane whose path states fewer than two cells, which declares no approach
///   at all and yet still tells the handoff inference to stand down;
/// - a lane whose cells belong to two machines, which is what a mistyped
///   terminus looks like: the other machine used to simply claim it;
/// - a lane owned by a machine that ends anywhere but that machine's own
///   handovers and pick places, which used to become an ordinary station's
///   approach and open the machine's floor to that station's robots;
/// - a lane owned by no machine that ends at a cell no station presents at,
///   which used to be dropped on the floor;
/// - a lane ending where another lane already ends, which used to install two
///   corridors in the graph while the collect staging map kept exactly one.
fn lanes(
    world: &World,
    table: &CellTable,
    machines: &[Machine],
    presentations: &[(CellId, Option<EntityId>, [f64; 2], Option<String>)],
) -> Result<Vec<Lane>, WorldMapError> {
    // Which machine owns each cell. Disjoint by construction: `machine::census`
    // refuses a machine frame nested under another machine's, so a cell's frame
    // chain reaches at most one machine.
    let mut owner: FxHashMap<CellId, usize> = FxHashMap::default();
    // The cells of a machine a lane may END at: where a carrier changes hands
    // with its belt, and where an operator works a carrier an HTM holds.
    // Everything else in the machine is floor to drive over, not a destination.
    let mut bindable: FxHashMap<CellId, usize> = FxHashMap::default();
    for (index, machine) in machines.iter().enumerate() {
        for &cell in &machine.cells {
            owner.insert(cell, index);
        }
        // A handover and a pick place belong to the machine whether or not the
        // membership pattern also returned them, so OWNED and BINDABLE answer
        // over the same cells and "the terminus belongs to this lane's machine"
        // is a lookup rather than a guess.
        for cell in [machine.dropoff, machine.collect].into_iter().flatten() {
            bindable.insert(cell, index);
            owner.insert(cell, index);
        }
        for seat in &machine.pick_places {
            bindable.insert(seat.cell, index);
            owner.insert(seat.cell, index);
        }
    }
    // Every cell a carrier changes hands with fixed conveyance at: a lane
    // ending on one of these is a machine's through route rather than an
    // approach, and that is what decides where its gate sits.
    let handovers: FxHashSet<CellId> =
        world.entities.values().filter(|e| e.has("port")).filter_map(|e| port_cell(table, &e.id)).collect();
    let presented_at: FxHashSet<CellId> = presentations.iter().map(|(cell, _, _, _)| *cell).collect();

    let mut out: Vec<Lane> = Vec::new();
    for entity in world.entities.values().filter(|entity| entity.has("queue")) {
        let also = [
            ("station", "a station"),
            ("presentation_point", "a presentation point"),
            ("port", "a port"),
            ("relay", "a relay"),
        ]
        .into_iter()
        .find_map(|(component, what)| entity.has(component).then_some(what))
        .or_else(|| table.cells_by_entity.contains_key(&entity.id).then_some("a cell"));
        if let Some(also) = also {
            return Err(WorldMapError::QueueNotOnItsOwnEntity { entity: entity.id.clone(), also });
        }
        if entity.has("tiling") {
            continue;
        }
        let queue: Queue = entity
            .require("queue")
            .map_err(|source| WorldMapError::Unreadable { entity: entity.id.clone(), message: source.to_string() })?;
        let mut cells = resolve_lane_path(table, entity, &queue)?;
        // One cell is a destination with no approach: it declares nothing for
        // the graph to cut and still tells `handoff_approaches` to stand down.
        if cells.len() < 2 {
            return Err(WorldMapError::LaneWithoutPath { lane: entity.id.clone(), steps: cells.len() });
        }
        let terminus = cells.pop().expect("checked non-empty above");

        // The machine the lane belongs to: the one every cell of it is posed
        // in. Two means a lane laid across a boundary, which is a mistyped step
        // or a mistyped terminus and never a machine anybody drew.
        let mut owning: Option<usize> = None;
        for cell in cells.iter().copied().chain(std::iter::once(terminus)) {
            let Some(&index) = owner.get(&cell) else { continue };
            match owning {
                Some(first) if first != index => {
                    return Err(WorldMapError::LaneCrossesMachines {
                        lane: entity.id.clone(),
                        first: machines[first].entity.clone(),
                        second: machines[index].entity.clone(),
                    });
                }
                _ => owning = Some(index),
            }
        }
        // And the machine the lane is DECLARED in, when the document poses it
        // in one: a lane written inside machine A may not serve machine B.
        if let Some(declared) = enclosing_machine(world, machines, entity)? {
            if let Some(index) = owning.filter(|index| *index != declared) {
                return Err(WorldMapError::LaneCrossesMachines {
                    lane: entity.id.clone(),
                    first: machines[declared].entity.clone(),
                    second: machines[index].entity.clone(),
                });
            }
        }

        // The OWNER decides what the terminus may be, and the terminus never
        // decides the owner. A lane laid over machine A's cells is A's lane, so
        // it may end only where A hands a carrier over or seats an operator;
        // reading the machine off the terminus instead let a lane cross A's
        // floor, end at an ordinary station's presentation cell, come out
        // owned by nobody, and hand A's cells to every robot bound for that
        // station. A lane no machine owns runs over ordinary floor, and the
        // only place it can serve is a cell a station presents at.
        let machine = match owning {
            Some(index) if bindable.get(&terminus) != Some(&index) => {
                return Err(WorldMapError::LaneEndsOutsideItsMachine {
                    lane: entity.id.clone(),
                    machine: machines[index].entity.clone(),
                    cell: terminus,
                });
            }
            Some(index) => Some(machines[index].entity.clone()),
            None if presented_at.contains(&terminus) => None,
            None => {
                return Err(WorldMapError::LaneEndsAtUnclaimedCell { lane: entity.id.clone(), cell: terminus });
            }
        };
        out.push(Lane {
            entity: entity.id.clone(),
            machine,
            terminus,
            path: cells,
            approach: !handovers.contains(&terminus),
        });
    }
    // Deterministic: a lane's terminus and first step order it, never hash order.
    out.sort_by_key(|lane| (lane.terminus.0, lane.path.first().map(|c| c.0).unwrap_or(0)));
    // ONE lane per place. Two lanes ending at one collection or presentation
    // cell used to install two corridors in the graph while the collect staging
    // map kept whichever the scan reached last, so routing and reservations
    // disagreed about which cells a robot queues on.
    for pair in out.windows(2) {
        if pair[0].terminus == pair[1].terminus {
            return Err(WorldMapError::LaneTerminusClaimedTwice {
                first: pair[0].entity.clone(),
                second: pair[1].entity.clone(),
                cell: pair[0].terminus,
            });
        }
    }
    Ok(out)
}

/// The machine whose frame an entity's own pose sits in, when the document
/// poses it in one at all.
///
/// This is MEMBERSHIP by the same rule `machine::census` uses for cells, asked
/// of things that are not cells: a lane (whose pose, when it has one, says
/// which machine wrote it) and a port that failed to resolve to a cell (whose
/// pose is all the loader has left to say whose port it was). Neither needs a
/// pose to be well formed -- a lane's path says where it runs -- so `None` is
/// an ordinary answer and not a defect.
fn enclosing_machine(world: &World, machines: &[Machine], entity: &Entity) -> Result<Option<usize>, WorldMapError> {
    let Some(frame) = entity.component::<Pose>("pose").ok().flatten().and_then(|p| p.frame) else {
        return Ok(None);
    };
    let chain = world
        .frames
        .ancestry(&frame)
        .map_err(|source| WorldMapError::Unreadable { entity: entity.id.clone(), message: source.to_string() })?;
    Ok(machines.iter().position(|m| chain.contains(&m.entity)))
}

/// One LANE's path as dessim cells, in path order.
///
/// A lane names its cells by IDENTITY and nothing else. The place a lane serves
/// is the last cell of its path, so a step written as a bare position would put
/// a nearest-cell search on the binding rule itself: a terminus mistyped by a
/// metre would bind to whichever way cell happened to be closest — another
/// machine's port, another operator's standing spot — and the document would
/// load. Refused instead, naming the lane.
fn resolve_lane_path(table: &CellTable, entity: &Entity, queue: &Queue) -> Result<Vec<CellId>, WorldMapError> {
    let lane = &entity.id;
    let mut out = Vec::with_capacity(queue.path.len());
    for address in &queue.path {
        let target = match address {
            CellAddress::Id(id) => id.clone(),
            CellAddress::Derived(raw) => raw
                .parse()
                .map_err(|source| WorldMapError::Unreadable { entity: lane.clone(), message: source.to_string() })?
                .into(),
            // Expansion rewrites a template-local address before a consumer ever
            // sees it, so one surviving here is a document defect.
            CellAddress::Local(_) | CellAddress::Handle(_) => {
                return Err(WorldMapError::Unreadable {
                    entity: lane.clone(),
                    message: "the queue names a template-local cell outside any template".into(),
                });
            }
            CellAddress::At(xy) => {
                return Err(WorldMapError::LaneStepIsAPosition { lane: lane.clone(), at: [xy[0], xy[1]] });
            }
        };
        match table.cells_by_entity.get(&target) {
            Some(&cell) => out.push(cell),
            None => return Err(WorldMapError::Dangling { what: "queue path step", entity: lane.clone(), target }),
        }
    }
    Ok(out)
}

/// The way cell a station stands on: the nearest one to its pose. A station is
/// posed, not addressed, so this is the one place the loader matches by
/// geometry rather than by reference.
fn nearest_way_cell(table: &CellTable, position: [f64; 2]) -> Option<CellId> {
    table
        .way_cells
        .iter()
        .filter_map(|cell| table.centres.get(cell).map(|centre| (*cell, *centre)))
        .min_by(|a, b| {
            let distance = |c: &(CellId, [f64; 2])| (c.1[0] - position[0]).hypot(c.1[1] - position[1]);
            distance(a).total_cmp(&distance(b))
        })
        .map(|(cell, _)| cell)
}

/// Which way an HTM leaves after presenting. The protocol writes an axis and a
/// sign IN THE STATION'S FRAME; the chain of scales to the primary frame turns
/// that sign round under a mirrored floor, so the direction travels through the
/// chain first and only then becomes a compass point: row centres increase
/// along `+y` and desmap's north is row-decreasing, so primary `-y` is North.
fn exit_direction(world: &World, entity_id: &EntityId, entity: &Entity) -> Result<CardinalDirection, WorldMapError> {
    let local = match entity.raw("station").and_then(|raw| raw.get("exit_direction")).and_then(|v| v.as_str()) {
        Some("-y") => [0.0, -1.0],
        Some("+y") => [0.0, 1.0],
        Some("-x") => [-1.0, 0.0],
        _ => [1.0, 0.0],
    };
    let unreadable = |message: String| WorldMapError::Unreadable { entity: entity_id.clone(), message };
    let d = match entity.component::<Pose>("pose").ok().flatten().and_then(|pose| pose.frame) {
        Some(frame) => {
            world.frames.direction_to_primary(&frame, local).map_err(|source| unreadable(source.to_string()))?
        }
        None => local,
    };
    if d[0] == 0.0 && d[1] == 0.0 {
        return Err(unreadable("the frame chain flattens this station's exit to no direction at all".into()));
    }
    Ok(if d[0].abs() >= d[1].abs() {
        if d[0] < 0.0 { CardinalDirection::West } else { CardinalDirection::East }
    } else if d[1] < 0.0 {
        CardinalDirection::North
    } else {
        CardinalDirection::South
    })
}

/// The carrier designation a machine's sink port accepts, from its `of`.
///
/// `of` is a list of `{kind, designation}` and the format allows several; a
/// belt that accepted two populations would need a rule for which, so the
/// first is taken and a second would be a document worth questioning rather
/// than quietly merging.
fn port_designation(world: &World, station: &EntityId) -> Option<String> {
    world.entities.values().find_map(|entity| {
        let Ok(Some(port)) = entity.component::<Port>("port") else { return None };
        if port.direction != "sink" || port.into.as_ref()?.root() != station.root() {
            return None;
        }
        port.of.first()?.get("designation")?.as_str().map(str::to_owned)
    })
}

/// A stable index per declaring entity: the EasyLoop, not the station.
///
/// Sorted by the declared id so the number does not depend on hash order, and
/// so a document that adds a machine renumbers deterministically.
fn machine_index(machines: &[DeclaredEntityId], entity: &EntityId) -> u32 {
    let root = entity.root();
    machines.iter().position(|m| *m == root).expect("root collected before use") as u32
}

/// One circuit of this station's belt, from `go.easyloop.loop_transit_secs`.
///
/// `None` for a station with no `go.easyloop` timing block, which is every
/// station that presents to an operator directly. Seconds in the document
/// because that is how the GO catalog states it; a `SimDuration` from here on,
/// so the `_secs` spelling stops at the document boundary.
fn conveyor_transit(entity: &Entity) -> Option<SimDuration> {
    let secs = entity.raw("go.easyloop")?.get("loop_transit_secs")?.as_f64()?;
    if secs <= 0.0 {
        return None;
    }
    Some(SimDuration::from_secs(secs))
}

fn bins(entity: &Entity) -> Option<usize> {
    entity.raw("station").and_then(|raw| raw.get("bins")).and_then(|value| value.as_u64()).map(|count| count as usize)
}
