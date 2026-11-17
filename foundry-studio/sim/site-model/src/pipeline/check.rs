//! Step 6 of SPEC §8: the questions that can only be asked once everything
//! exists.
//!
//! Two different jobs share this step, and the split between them is deliberate.
//! Identity and reference resolution are *refusals*: a duplicate id or a
//! reference to an entity nobody declared means the document cannot be read at
//! all, and the format's whole case against display-name keys (925 cells keyed to
//! a zone string with a load-bearing trailing space) rests on failing here rather
//! than best-effort matching. Physics-versus-policy disagreements are *warnings*:
//! a lane pointed through floor a robot cannot occupy is very probably a mistake,
//! but it is a judgement about intent, and SPEC §7 is explicit that the validator
//! warns in that direction and never the other.

use crate::components;
// Documents still carry the deprecated `robot_class`, so the checks keep reading it.
#[allow(deprecated)]
use crate::components::fleet::{Cohort, RobotClass};
use crate::components::product::{CellAddress, LocalRef, Queue, Sector};
use crate::components::spatial::{Cell, Pose, Region, Tiling};
use crate::components::structure::{Access, AccessCell, Rack, Stitch};
use crate::derivation::RawLocalDerivation;
use crate::document::{Document, Record};
use crate::entity::Entity;
use crate::error::{GridCollision, LoadError, Warning};
use crate::frames::FrameGraph;
use crate::ids::EntityId;
use crate::pipeline::graph::{CellGeometry, Edge};
use crate::references;
use crate::placeholders;
use crate::units::{Coord, Steps};
use std::collections::{BTreeMap, BTreeSet};

/// Run every post-expansion check. Refusals return; judgements accumulate.
pub fn check(
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
    cells: &BTreeMap<EntityId, CellGeometry>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    produced: &BTreeSet<EntityId>,
    edges: &[Edge],
) -> Result<Vec<Warning>, LoadError> {
    check_references(entities, frames)?;
    check_queue_paths(entities, produced)?;
    check_relay_bindings(entities)?;
    let mut warnings = Vec::new();
    warn_flow_over_uninhabitable(entities, &mut warnings)?;
    warn_cohorts_assigned_out_of_reach(entities, contributions, &mut warnings)?;
    warn_classes_taller_than_the_steel_above_them(entities, frames, cells, &mut warnings)?;
    warn_touching_regions_without_stitch(entities, frames, &mut warnings)?;
    warn_lattice_projection_collisions(entities, frames, cells, edges, &mut warnings)?;
    warn_floor_served_along_is_no_cell_grid(entities, contributions, &mut warnings)?;
    Ok(warnings)
}

// SPEC §8's step 6 opened with a global uniqueness pass, and there is nothing
// left for it to ask. An identity is either a UUID somebody minted or the
// address a generator produced an entity at (SPEC §3), so a declared identity
// and a derived one can never be the same string, two generators cannot reach
// one address, and no derived identity can disagree with the chain that made it
// — the chain *is* the identity. What one generator could still do is emit one
// segment twice, and expansion refuses that where it happens, naming the
// generator rather than reporting a collision two steps later.

/// Every UUID a known component uses as a reference must resolve, and a field
/// that wants a frame must get one.
///
/// Which fields are references is part of each component's schema and not
/// guessable from shape (SPEC §2), so the list is written out by hand in
/// [`crate::references`] — that is the cost of the design, and the alternative
/// is guessing that any UUID-shaped string is a pointer.
fn check_references(
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
) -> Result<(), LoadError> {
    for entity in entities.values() {
        for reference in references::of(entity)? {
            let resolved = entities.contains_key(&reference.target)
                && (!reference.must_be_frame || frames.is_frame(&reference.target));
            if !resolved {
                return Err(LoadError::DanglingReference {
                    entity: entity.id.clone(),
                    component: reference.component.to_string(),
                    field: reference.field.to_string(),
                    target: reference.target.clone(),
                });
            }
        }
    }
    Ok(())
}

/// A routing decision over cells nothing can stand on.
fn warn_flow_over_uninhabitable(
    entities: &BTreeMap<EntityId, Entity>,
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    for entity in entities.values() {
        if !entity.has("flow") || !entity.has("tiling") {
            continue;
        }
        let mut any = false;
        let mut habitable = false;
        for other in entities.values() {
            if other.parent.as_ref() != Some(&entity.id) || !other.has("cell") {
                continue;
            }
            any = true;
            if let Some(cell) = other.component::<Cell>("cell")? {
                if !cell.traversable_by.is_empty() {
                    habitable = true;
                    break;
                }
            }
        }
        if any && !habitable {
            warnings.push(Warning::FlowOverUninhabitableCells {
                region: entity.id.clone(),
            });
        }
    }
    Ok(())
}

/// A document that leans on a component without declaring that it does.
///
/// `required_components` is how a document says "refuse me rather than
/// mis-simulate me". It only works if it is complete, and completeness is
/// exactly the kind of bookkeeping an author forgets — so the loader, which can
/// see every component every record actually uses, says which load-bearing ones
/// the list is missing.
pub fn required_components_gaps(documents: &[&Document]) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for document in documents {
        let declared: BTreeSet<&str> = document
            .required_components
            .iter()
            .map(String::as_str)
            .collect();
        let mut missing: BTreeSet<String> = BTreeSet::new();
        for record in &document.records {
            let components = match record {
                Record::Declaration(d) | Record::Patch(d) => &d.components,
                Record::Override(o) => &o.components,
                Record::Suppression(_) => continue,
            };
            for name in components.keys() {
                if components::LOAD_BEARING.contains(&name.as_str())
                    && !declared.contains(name.as_str())
                {
                    missing.insert(name.clone());
                }
            }
        }
        if !missing.is_empty() {
            warnings.push(Warning::LoadBearingComponentNotRequired {
                doc: document.meta.id,
                components: missing.into_iter().collect(),
            });
        }
    }
    warnings
}

/// Every step of every queue path must land on a cell that exists.
///
/// A queue is an ordered walk over floor, so a step that resolves to nothing is
/// the same failure a dangling override is, and gets the same answer: refuse and
/// name the record (SPEC §2, §3). The removed-cell case is called out separately
/// because it reads differently to whoever has to fix it — the address was
/// right when it was written and an exclusion or a suppression later took the
/// floor out from under it, and a queue whose third step is a hole is an
/// authoring mistake to correct now rather than a surprise for a simulator to
/// discover at runtime.
fn check_queue_paths(entities: &BTreeMap<EntityId, Entity>, produced: &BTreeSet<EntityId>) -> Result<(), LoadError> {
    for entity in entities.values() {
        let Some(queue) = entity.component::<Queue>("queue")? else {
            continue;
        };
        for (index, step) in queue.path.iter().enumerate() {
            let (target, address) = match step {
                CellAddress::Derived(raw) => {
                    let address = raw.parse()?;
                    (EntityId::from(address.clone()), address.to_string())
                }
                CellAddress::Id(id) => (id.clone(), id.to_string()),
                // Only a stamp turns a handle into an identity, and the
                // placeholder walk refuses one before this check can meet it.
                CellAddress::Local(RawLocalDerivation { local_ref, .. })
                | CellAddress::Handle(LocalRef { local_ref, .. }) => {
                    return Err(placeholders::unresolved_handle(
                        &entity.id,
                        "queue",
                        format!("path[{index}]"),
                        local_ref,
                    ))
                }
                // A position names no entity, so there is no reference to
                // resolve and nothing here can dangle. Whether a cell actually
                // lies near it is the consumer's binding question, answered the
                // way a relay's is.
                CellAddress::At(_) => continue,
            };
            match entities.get(&target) {
                Some(cell) if cell.has("cell") => {}
                Some(_) => return Err(LoadError::QueuePathNotACell { entity: entity.id.clone(), index, address }),
                None if produced.contains(&target) => {
                    return Err(LoadError::QueuePathThroughRemovedCell { entity: entity.id.clone(), index, address })
                }
                None => return Err(LoadError::QueuePathDangling { entity: entity.id.clone(), index, address }),
            }
        }
    }
    Ok(())
}

/// Every relay's binding must name a cell this world holds.
///
/// The search can only ever pick a live cell, so this is about the other half of
/// SPEC §5's parity: a binding an override wrote by hand names a cell address
/// like any other, and a reference that points at nothing is a load error
/// (SPEC §2) rather than a relay quietly serving a hole.
fn check_relay_bindings(entities: &BTreeMap<EntityId, Entity>) -> Result<(), LoadError> {
    for entity in entities.values() {
        let Some(binding) = entity.component::<AccessCell>("access_cell")? else {
            continue;
        };
        let (target, address) = match &binding.cell {
            CellAddress::Derived(raw) => {
                let address = raw.parse()?;
                (EntityId::from(address.clone()), address.to_string())
            }
            CellAddress::Id(id) => (id.clone(), id.to_string()),
            CellAddress::Local(RawLocalDerivation { local_ref, .. })
            | CellAddress::Handle(LocalRef { local_ref, .. }) => {
                return Err(placeholders::unresolved_handle(&entity.id, "access_cell", "cell".into(), local_ref))
            }
            // Nothing to resolve: the binder treats a position as "search from
            // here", exactly as it does an unstated binding.
            CellAddress::At(_) => continue,
        };
        if !entities.get(&target).is_some_and(|cell| cell.has("cell")) {
            return Err(LoadError::AccessCellNotALiveCell { entity: entity.id.clone(), address });
        }
    }
    Ok(())
}

/// A cohort tied to somewhere its own robots cannot go.
///
/// "Cannot go" means the class fits nowhere in the target's expansion, and a
/// rack counts as somewhere a VTM goes: climbing a rack face is rack-internal
/// motion, not graph traversal, so a VTM cohort assigned to a rack sector is
/// doing exactly what it was built for even though every relay cell there is
/// HTM-only (SPEC §7, v0.3).
#[allow(deprecated)]
fn warn_cohorts_assigned_out_of_reach(
    entities: &BTreeMap<EntityId, Entity>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    for entity in entities.values() {
        let Some(cohort) = entity.component::<Cohort>("cohort")? else {
            continue;
        };
        let Some(assignment) = &cohort.assignment else {
            continue;
        };
        if !assignment.is_hard() {
            continue;
        }
        let Some(class) = entities
            .get(&cohort.robot_class)
            .and_then(|e| e.component::<RobotClass>("robot_class").transpose())
            .transpose()?
        else {
            continue;
        };
        for target in &assignment.to {
            let mut reachable = Vec::new();
            gather_cells(target, entities, contributions, &mut reachable)?;
            let admits_a_cell = reachable.iter().any(|id| {
                entities
                    .get(id)
                    .and_then(|e| e.component::<Cell>("cell").ok().flatten())
                    .is_some_and(|cell| cell.traversable_by.contains(&class.role))
            });
            let served = served_by_an_access(target, &cohort.robot_class, entities)?;
            if !admits_a_cell && !served {
                warnings.push(Warning::CohortAssignedWhereClassCannotGo {
                    cohort: entity.id.clone(),
                    robot_class: cohort.robot_class.clone(),
                    assigned_to: target.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Whether an access over this target's expansion serves the class.
///
/// This replaces the older heuristic that any rack in a sector counted as
/// habitat for any VTM. A rack is not habitat in itself — a rack somebody
/// declared no access onto is steel nobody reaches — and an access is the
/// declaration that turns it into somewhere a class works: it names the run, the
/// face, and who serves from it. `serves` absent means unrestricted.
fn served_by_an_access(
    target: &EntityId,
    class: &EntityId,
    entities: &BTreeMap<EntityId, Entity>,
) -> Result<bool, LoadError> {
    let mut scope: Vec<EntityId> = vec![target.clone()];
    if let Some(entity) = entities.get(target) {
        if let Some(sector) = entity.component::<Sector>("sector")? {
            scope.extend(sector.members.iter().cloned());
        }
    }
    for entity in entities.values() {
        let Some(access) = entity.component::<Access>("access")? else {
            continue;
        };
        if !scope
            .iter()
            .any(|root| covers(root, &access.rack, entities))
        {
            continue;
        }
        match &access.serves {
            None => return Ok(true),
            Some(classes) if classes.contains(class) => return Ok(true),
            Some(_) => {}
        }
    }
    Ok(false)
}

/// A face somebody drives up to that has no cells in front of it.
///
/// Floor robots need surviving cells, whether the serving region tiles them
/// directly or stamps several tiled sections. Ladder robots need neither.
/// Reading the expansion also catches a tiling whose exclusions remove every
/// cell; the presence of a generator alone does not provide anywhere to stand.
#[allow(deprecated)]
fn warn_floor_served_along_is_no_cell_grid(
    entities: &BTreeMap<EntityId, Entity>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    for entity in entities.values() {
        let Some(access) = entity.component::<Access>("access")? else {
            continue;
        };
        if !entities.contains_key(&access.along) {
            continue;
        }
        if contributions.get(&access.along).is_some_and(|cells| !cells.is_empty()) {
            continue;
        }
        // Absent `serves` is unrestricted, so it resolves to every class there
        // is — including every floor class.
        let mut offenders: Vec<EntityId> = Vec::new();
        for candidate in entities.values() {
            let Some(class) = candidate.component::<RobotClass>("robot_class")? else {
                continue;
            };
            let named = access
                .serves
                .as_ref()
                .is_none_or(|list| list.contains(&candidate.id));
            if named && class.is_floor() {
                offenders.push(candidate.id.clone());
            }
        }
        for robot_class in offenders {
            warnings.push(Warning::FloorServedAlongIsNoCellGrid {
                access: entity.id.clone(),
                along: access.along.clone(),
                robot_class,
            });
        }
    }
    Ok(())
}

/// The cells an assignment target stands for: its own expansion, plus — when the
/// target is a sector — the expansions of everything it names, because SPEC §10
/// says a generator member stands for its whole expansion.
fn gather_cells(
    target: &EntityId,
    entities: &BTreeMap<EntityId, Entity>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    out: &mut Vec<EntityId>,
) -> Result<(), LoadError> {
    if let Some(cells) = contributions.get(target) {
        out.extend(cells.iter().cloned());
    }
    if let Some(entity) = entities.get(target) {
        if let Some(sector) = entity.component::<Sector>("sector")? {
            for member in &sector.members {
                if let Some(cells) = contributions.get(member) {
                    out.extend(cells.iter().cloned());
                }
            }
        }
    }
    Ok(())
}

/// A robot class taller than the steel it would have to drive under.
///
/// A tunnel is not a kind of rack and not a field: it is a rack whose first
/// level starts high, with a cell_grid tiled beneath it. So the quantity to
/// check against is the lowest level of any rack whose footprint that cell_grid
/// crosses (SPEC §7). The association is still geometric — footprint
/// containment, not a declared reference — which is the half of SPEC-GAPS 29
/// that stands.
#[allow(deprecated)]
fn warn_classes_taller_than_the_steel_above_them(
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
    cells: &BTreeMap<EntityId, CellGeometry>,
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    let classes: Vec<(EntityId, RobotClass)> = entities
        .values()
        .filter_map(|e| {
            e.component::<RobotClass>("robot_class")
                .transpose()
                .map(|c| c.map(|c| (e.id.clone(), c)))
        })
        .collect::<Result<_, _>>()?;
    if classes.is_empty() {
        return Ok(());
    }

    for entity in entities.values() {
        let Some(rack) = entity.component::<Rack>("rack")? else {
            continue;
        };
        let Some(lowest_level) = rack.lowest_level() else {
            continue;
        };
        let Some(pose) = entity.component::<Pose>("pose")? else {
            continue;
        };
        let (Some(frame), Some(anchor)) = (pose.frame.as_ref(), pose.xy()) else {
            continue;
        };
        // The footprint at heading 0 runs from the anchor into +x/+y; the heading
        // turns it about the anchor.
        let mut min = [i64::MAX, i64::MAX];
        let mut max = [i64::MIN, i64::MIN];
        for corner in [
            [0, 0],
            [rack.footprint[0].0, 0],
            [0, rack.footprint[1].0],
            [rack.footprint[0].0, rack.footprint[1].0],
        ] {
            let r = pose
                .heading_deg
                .rotate([corner[0] as f64, corner[1] as f64]);
            let p = frames.to_primary(
                frame,
                [
                    Coord(Steps::round_half_up(anchor[0].0 as f64 + r[0]).0),
                    Coord(Steps::round_half_up(anchor[1].0 as f64 + r[1]).0),
                ],
            )?;
            for axis in 0..2 {
                min[axis] = min[axis].min(p[axis].0);
                max[axis] = max[axis].max(p[axis].0);
            }
        }

        let beneath: BTreeSet<String> = cells
            .values()
            .filter(|geometry| {
                geometry.centre[0].0 > min[0]
                    && geometry.centre[0].0 < max[0]
                    && geometry.centre[1].0 > min[1]
                    && geometry.centre[1].0 < max[1]
            })
            .filter_map(|geometry| entities.get(&geometry.id))
            .filter_map(|e| e.component::<Cell>("cell").ok().flatten())
            .flat_map(|cell| cell.traversable_by)
            .collect();

        for (class_id, class) in &classes {
            let Some(height) = class.height else {
                continue;
            };
            if height.0 > lowest_level.0 && beneath.contains(&class.role) {
                warnings.push(Warning::ClassTallerThanLowestLevel {
                    rack: entity.id.clone(),
                    robot_class: class_id.clone(),
                    lowest_level,
                    height,
                });
            }
        }
    }
    Ok(())
}

/// Two cells that a declared integer frame cannot tell apart.
///
/// The lattice frame is nominal and lossy by declaration (§4) — real sites'
/// lattices are not affine images of the measured world — so a projection
/// folding two cells into one square is expected *where the author already said
/// the two are one seam*, and a surprise everywhere else. Two cell_grids that
/// deliberately overlap and are stitched together are the first case: a legacy
/// exporter wrote one square there and was right to. Two that merely happen to
/// land within half a lattice pitch of each other are the second: whatever a
/// legacy consumer does with that square, it will be wrong about one of them.
fn warn_lattice_projection_collisions(
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
    cells: &BTreeMap<EntityId, CellGeometry>,
    edges: &[Edge],
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    let lattices: Vec<EntityId> = frames.integer_only().collect();
    if lattices.is_empty() || cells.is_empty() {
        return Ok(());
    }

    // A declared seam: an arc a stitch owns. Not an arc a flow rule compiled
    // between neighbours inside one cell_grid.
    let mut linked: BTreeSet<(EntityId, EntityId)> = BTreeSet::new();
    for edge in edges {
        let declared_seam = entities.get(&edge.owner).is_some_and(|e| e.has("stitch"));
        if declared_seam {
            linked.insert(ordered_pair(&edge.from, &edge.to));
        }
    }

    for frame in lattices {
        let mut claims: BTreeMap<(i64, i64), Vec<EntityId>> = BTreeMap::new();
        for geometry in cells.values() {
            // §4: a projection into an integer frame rounds half-up, which
            // `from_primary` has already done.
            let projected = frames.from_primary(&frame, geometry.centre)?;
            claims
                .entry((projected[0].0, projected[1].0))
                .or_default()
                .push(geometry.id.clone());
        }
        let mut folded: Vec<GridCollision> = Vec::new();
        let mut cells_involved = 0usize;
        // `claims` is keyed by coordinate, so this walks the squares in a fixed
        // order and the first three samples are the same on every run.
        for (square, mut claimants) in claims {
            if claimants.len() < 2 {
                continue;
            }
            claimants.sort();
            let every_pair_stitched = claimants.iter().enumerate().all(|(i, a)| {
                claimants[i + 1..]
                    .iter()
                    .all(|b| linked.contains(&ordered_pair(a, b)))
            });
            if every_pair_stitched {
                continue;
            }
            cells_involved += claimants.len();
            folded.push(GridCollision {
                coordinate: [square.0, square.1],
                cells: claimants.iter().map(EntityId::to_string).collect(),
            });
        }
        if !folded.is_empty() {
            warnings.push(Warning::GridProjectionCollisions {
                frame: frame.clone(),
                squares: folded.len(),
                cells: cells_involved,
                samples: folded.into_iter().take(SAMPLE_SQUARES).collect(),
            });
        }
    }
    Ok(())
}

/// How many folded squares a summary shows before it stops. Enough to see the
/// shape of the loss, few enough that the count stays the headline.
const SAMPLE_SQUARES: usize = 3;

fn ordered_pair(a: &EntityId, b: &EntityId) -> (EntityId, EntityId) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

/// Two cell_grids that share a boundary with nothing declared between them.
///
/// SPEC §5 calls this an error. It ships as a warning because "touch" has no
/// numeric definition in the spec and deliberate overlaps exist — a cell_grid
/// tiled under a rack, a stamped station standing in the floor it opens onto —
/// so a refusal here would refuse the format's own fixtures. See SPEC-GAPS.
fn warn_touching_regions_without_stitch(
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    let mut tiled: Vec<(EntityId, [i64; 2], [i64; 2])> = Vec::new();
    for entity in entities.values() {
        if !entity.has("tiling") {
            continue;
        }
        let Some(region) = entity.component::<Region>("region")? else {
            continue;
        };
        let Some(frame) = region.frame.as_ref() else {
            continue;
        };
        let a = frames.to_primary(
            frame,
            [Coord(region.origin[0].0), Coord(region.origin[1].0)],
        )?;
        let far = region.max_corner();
        let b = frames.to_primary(frame, [Coord(far[0].0), Coord(far[1].0)])?;
        tiled.push((
            entity.id.clone(),
            [a[0].0.min(b[0].0), a[1].0.min(b[1].0)],
            [a[0].0.max(b[0].0), a[1].0.max(b[1].0)],
        ));
        let _: Option<Tiling> = entity.component("tiling")?;
    }

    let mut seams: Vec<(EntityId, EntityId)> = Vec::new();
    for entity in entities.values() {
        let Some(stitch) = entity.component::<Stitch>("stitch")? else {
            continue;
        };
        seams.push((stitch.a.clone(), stitch.b.clone()));
    }

    for i in 0..tiled.len() {
        for j in (i + 1)..tiled.len() {
            let (ai, amin, amax) = &tiled[i];
            let (bi, bmin, bmax) = &tiled[j];
            let mut separations = [0i64; 2];
            for axis in 0..2 {
                separations[axis] = amin[axis].max(bmin[axis]) - amax[axis].min(bmax[axis]);
            }
            let touching = (separations[0] == 0 && separations[1] < 0)
                || (separations[1] == 0 && separations[0] < 0);
            if !touching {
                continue;
            }
            // A stitch covers the pair when each of its endpoints is one of the
            // two regions or something the region hangs beneath — a stitch named
            // against a rack-row stamp speaks for every region that stamp
            // derived.
            let covered = seams.iter().any(|(sa, sb)| {
                (covers(sa, ai, entities) && covers(sb, bi, entities))
                    || (covers(sa, bi, entities) && covers(sb, ai, entities))
            });
            if !covered {
                warnings.push(Warning::TouchingRegionsWithoutStitch {
                    a: ai.clone(),
                    b: bi.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Whether `root` is `node` itself or one of its ancestors.
fn covers(root: &EntityId, node: &EntityId, entities: &BTreeMap<EntityId, Entity>) -> bool {
    let mut cursor = Some(node.clone());
    while let Some(current) = cursor {
        if &current == root {
            return true;
        }
        cursor = entities.get(&current).and_then(|e| e.parent.clone());
    }
    false
}
