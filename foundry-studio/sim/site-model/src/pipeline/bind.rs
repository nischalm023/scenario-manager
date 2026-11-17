//! The other half of step 5 of SPEC §8: giving each relay the cell an HTM
//! occupies to exchange with it.
//!
//! A relay is a shelf on a bay, and the place somebody stands to work it is an
//! ordinary cell of the cell_grid that HTM travels — not something the rack
//! mints beside itself. So binding is a search rather than a derivation, and the
//! search is proximity and nothing else: the relay's own centre, the cell
//! centres, and the level's `reach` as the radius. There is no facing test, no
//! outwardness test, and no face-derived starting point, because who may stand
//! on a cell is the cell's own `traversable_by` to answer and never the relay's.
//! The result rides as an `access_cell` component, which is the same component
//! an override writes by hand; a relay that already carries one is not searched
//! for.
//!
//! Binding runs here, after overrides and after suppression, for the two reasons
//! SPEC §8 gives: a hand-stated binding can then name a cell a later layer
//! declared, and the automatic search sees exactly the cells that survived.
//!
//! Where the fault lies decides the severity (SPEC §7). A relay with no floor in
//! front of it while its neighbours on the same run have one is an authoring
//! mistake and refuses the load. A run that can reach nothing at all is a
//! statement about the site — 3,300 of the 3,609 runs in the converted fixture
//! set make it — and warns instead, because refusing those would refuse the
//! installed base. And two relays standing at *different* places on their bays
//! that end up on one cell warn too: SPEC §5 expects a cell under each relay
//! stack, so one cell serving two stacks says the floor and the steel are out of
//! step.

use crate::components::product::CellAddress;
use crate::components::spatial::Pose;
use crate::components::structure::{AccessCell, Rack, Relay};
use crate::entity::Entity;
use crate::error::{LoadError, Warning};
use crate::frames::FrameGraph;
use crate::ids::{DeclaredEntityId, EntityId};
use crate::pipeline::graph::CellGeometry;
use crate::units::{Coord, Steps};
use serde_json::json;
use std::collections::BTreeMap;

/// Bind every relay that does not already carry a binding, and say what could
/// not be bound and what came out misaligned.
pub fn relays(
    entities: &mut BTreeMap<EntityId, Entity>,
    cells: &BTreeMap<EntityId, CellGeometry>,
    frames: &FrameGraph,
) -> Result<Vec<Warning>, LoadError> {
    let mut per_run: BTreeMap<DeclaredEntityId, RunTally> = BTreeMap::new();
    // Bindings this stage found and will write. A hand-stated one is not in
    // here: rewriting it would respell an author's two-part address as a flat
    // one for no reason, and the two are the same address (SPEC §3).
    let mut found: Vec<(EntityId, EntityId)> = Vec::new();
    // Every relay standing on a cell, however it got there, which is what the
    // sharing check reads.
    let mut standing: Vec<(EntityId, EntityId)> = Vec::new();
    // Which stack each relay belongs to. A stack is one bay and one offset on
    // it: the two shelves of cencora's stacked relay levels are one place to
    // stand, and sharing a cell is what they are meant to do.
    let mut stacks: BTreeMap<EntityId, Stack> = BTreeMap::new();

    for entity in entities.values() {
        let Some(relay) = entity.component::<Relay>("relay")? else {
            continue;
        };
        // The run a relay belongs to: the declared entity its chain starts at —
        // the stamp whose instances are the bays — or the rack itself when
        // somebody declared one directly.
        let tally = per_run.entry(entity.id.root()).or_default();
        tally.relays += 1;
        stacks.insert(
            entity.id.clone(),
            Stack {
                rack: relay.rack.clone(),
                offset: relay.offset,
            },
        );
        // An overridden relay is not searched for: declared and derived
        // bindings are the same component either way (SPEC §5). It still counts
        // toward the sharing check below, because two shelves on one cell is the
        // same misalignment however the binding got there.
        if let Some(stated) = entity.component::<AccessCell>("access_cell")? {
            tally.bound += 1;
            if let Some(cell) = stated_cell(&stated) {
                standing.push((entity.id.clone(), cell));
            }
            continue;
        }
        let (point, reach) = reach_of(&entity.id, &relay, entities, frames)?;
        match nearest_in_reach(point, reach, cells) {
            Some(cell) => {
                tally.bound += 1;
                standing.push((entity.id.clone(), cell.clone()));
                found.push((entity.id.clone(), cell));
            }
            None => {
                tally.unbound.push((entity.id.clone(), reach));
            }
        }
    }

    let mut warnings = Vec::new();
    for (run, tally) in &per_run {
        if tally.unbound.is_empty() {
            continue;
        }
        if tally.bound > 0 {
            let (relay, reach) = &tally.unbound[0];
            return Err(LoadError::RelayUnbound {
                address: relay.to_string(),
                reach: *reach,
            });
        }
        warnings.push(Warning::SteelNobodyReaches {
            run: (*run).into(),
            relays: tally.relays,
        });
    }

    warnings.extend(shared_sideways(&standing, &stacks));

    for (relay, cell) in found {
        if let Some(entity) = entities.get_mut(&relay) {
            entity
                .components
                .insert("access_cell".to_string(), json!({ "cell": cell }));
        }
    }
    Ok(warnings)
}

/// How one run's relays fared, which is what decides between a refusal and a
/// warning.
#[derive(Debug, Default)]
struct RunTally {
    relays: usize,
    bound: usize,
    unbound: Vec<(EntityId, Steps)>,
}

/// The cell a hand-stated binding names, when it names one this stage can read.
/// The template-internal form is rewritten to a concrete address at stamp
/// expansion, long before binding runs, so it cannot arrive here — and if it
/// somehow did, the reference check is the stage that says so.
fn stated_cell(stated: &AccessCell) -> Option<EntityId> {
    match &stated.cell {
        CellAddress::Id(id) => Some(id.clone()),
        CellAddress::Derived(raw) => raw.parse().ok().map(EntityId::from),
        CellAddress::Local(_) | CellAddress::Handle(_) => None,
        // A position states where to stand, not which cell: that is what the
        // automatic search does anyway, so let it run.
        CellAddress::At(_) => None,
    }
}

/// Which shelves are the same place to stand: one bay, one offset on it,
/// however many heights.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Stack {
    rack: EntityId,
    offset: [Steps; 2],
}

/// One warning per pair of stacks that ended up on one cell (SPEC §5).
///
/// Stacked shelves sharing a cell is the expected shape and says nothing. Two
/// shelves at different places on the steel sharing one is the misalignment
/// signal: the floor was tiled without the relays in mind, or a relay stands
/// where no cell was laid for it, and the site's own drawings are where that
/// gets fixed. Both relays are named because neither one is at fault by itself.
fn shared_sideways(
    standing: &[(EntityId, EntityId)],
    stacks: &BTreeMap<EntityId, Stack>,
) -> Vec<Warning> {
    // Per cell, the lowest-identity relay of each distinct stack standing on it.
    let mut per_cell: BTreeMap<&EntityId, BTreeMap<&Stack, &EntityId>> = BTreeMap::new();
    for (relay, cell) in standing {
        let Some(stack) = stacks.get(relay) else {
            continue;
        };
        per_cell
            .entry(cell)
            .or_default()
            .entry(stack)
            .and_modify(|first| {
                if relay < *first {
                    *first = relay;
                }
            })
            .or_insert(relay);
    }

    let mut warnings = Vec::new();
    for (cell, on_it) in per_cell {
        let relays: Vec<&EntityId> = on_it.into_values().collect();
        for (index, a) in relays.iter().enumerate() {
            for b in &relays[index + 1..] {
                warnings.push(Warning::RelaysApartShareOneCell {
                    cell: (*cell).clone(),
                    a: (*a).clone(),
                    b: (*b).clone(),
                });
            }
        }
    }
    warnings
}

/// Where a relay stands in the primary frame — because that is where the cells
/// are — and how far from it a cell may sit and still be its own.
fn reach_of(
    relay_id: &EntityId,
    relay: &Relay,
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
) -> Result<([Steps; 2], Steps), LoadError> {
    let rack_entity = entities
        .get(&relay.rack)
        .ok_or_else(|| LoadError::DanglingReference {
            entity: relay_id.clone(),
            component: "relay".into(),
            field: "rack".into(),
            target: relay.rack.clone(),
        })?;
    let rack: Rack = rack_entity.require("rack")?;
    let level = rack
        .levels
        .get(relay.level)
        .and_then(|level| level.relay())
        .ok_or_else(|| LoadError::Unsupported {
            entity: relay_id.clone(),
            message: format!(
                "the relay names level {} of its rack, which is no relay level",
                relay.level
            ),
        })?;
    let pose: Pose = rack_entity.require("pose")?;
    let (Some(frame), Some(anchor)) = (pose.frame.clone(), pose.xy()) else {
        return Err(LoadError::Unsupported {
            entity: rack_entity.id.clone(),
            message: "a rack whose relays bind must be posed in a frame".into(),
        });
    };

    let place = |p: [i64; 2]| -> Result<[Steps; 2], LoadError> {
        let rotated = pose.heading_deg.rotate([p[0] as f64, p[1] as f64]);
        frames.to_primary(
            &frame,
            [
                Coord(Steps::round_half_up(anchor[0].0 as f64 + rotated[0]).0),
                Coord(Steps::round_half_up(anchor[1].0 as f64 + rotated[1]).0),
            ],
        )
    };
    let point = place([relay.offset[0].0, relay.offset[1].0])?;
    // The reach is a radius, so it is carried into the primary frame by
    // measuring one: a point that far away in the rack's own frame, pushed
    // through the same chain. A frame whose two axes scale differently has no
    // radius to speak of — the same assumption the rack's heading already makes.
    let span = place([relay.offset[0].0 + level.reach.0, relay.offset[1].0])?;
    Ok((point, Steps::distance(point, span)))
}

/// The nearest cell to a relay within its reach, ties broken toward the lower
/// identity (SPEC §5). Centre to centre, and nothing else asked.
fn nearest_in_reach(
    point: [Steps; 2],
    reach: Steps,
    cells: &BTreeMap<EntityId, CellGeometry>,
) -> Option<EntityId> {
    let limit = i128::from(reach.0) * i128::from(reach.0);
    let mut best: Option<(i128, &EntityId)> = None;
    for cell in cells.values() {
        let dx = i128::from(cell.centre[0].0 - point[0].0);
        let dy = i128::from(cell.centre[1].0 - point[1].0);
        let distance = dx * dx + dy * dy;
        if distance > limit {
            continue;
        }
        let candidate = (distance, &cell.id);
        if best.is_none_or(|current| candidate < current) {
            best = Some(candidate);
        }
    }
    best.map(|(_, id)| id.clone())
}
