//! Part of step 5 of SPEC §8: deriving the arcs, which is the only way an arc
//! ever exists.
//!
//! Two mechanisms and no third. A tiling compiles arcs *inside* its own
//! surviving cells, and a `stitch` entity compiles arcs *between* two
//! expansions. The two differ in what silence means. Inside one cell_grid,
//! `flow` is a restriction and never a permission: one with no flow rule is
//! two-way, because floor an author tiled is floor an author meant to be
//! travelled. Between expansions, silence means nothing at all — SPEC §5 rules
//! out global adjacency scans precisely so a seam is something an author
//! asserted and a loader can refuse. That is why a contact stitch producing zero
//! links fails the load, while many-to-one matching is fine: relay pitch and
//! cell pitch are incommensurate in real buildings.
//!
//! A relay's binding to the floor is not an arc and is not here: it names a cell
//! that already exists, which is [`crate::pipeline::bind`]'s job.
//!
//! A contact stitch resolves in one of two geometries, chosen by where the two
//! expansions sit rather than by anything an author writes: *embedded*, when one
//! expansion stands inside the other's extent, and *separated*, when there is a
//! seam between them. They are different questions — "which of these cells am I
//! standing on" against "which cells face me across this gap" — and answering
//! the first with the second's machinery is what left a six-bay rack row with a
//! single link into the floor it serves.
//!
//! Every comparison below is exact integer arithmetic on half-steps
//! ([`crate::components::spatial::HalfSteps`]), because a cell's edge sits half
//! its footprint from its centre and an odd footprint would otherwise put a
//! boundary between two steps.

use crate::components::product::{CellAddress, LocalRef};
use crate::components::spatial::{Bounds, Cell, HalfSteps, Pose, Region, Tiling};
use crate::components::structure::{Direction, Flow, FlowRule, Stitch, StitchRule};
use crate::derivation::{KeySegment, RawLocalDerivation};
use crate::entity::Entity;
use crate::error::LoadError;
use crate::frames::FrameGraph;
use crate::ids::EntityId;
use crate::placeholders;
use crate::units::{Coord, Steps};
use std::collections::BTreeMap;

/// One arc of the traversability graph. Edges are not entities in v0; they are
/// derived data with stable keys, so a record can still name one to suppress it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The generator or stitch the key hangs under.
    pub owner: EntityId,
    /// `edge:{fromkey}>{tokey}` for a flow arc, `link:{i}` for a stitch link.
    pub key: String,
    pub from: EntityId,
    pub to: EntityId,
    /// A bidirectional link also permits the reversed traversal.
    pub bidirectional: bool,
    pub curve: Option<ArcGeometry>,
}

/// Circular link geometry in primary-frame steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcGeometry {
    pub center: [Steps; 2],
    pub clockwise: bool,
    pub start: Option<[Steps; 2]>,
    pub end: Option<[Steps; 2]>,
}

/// A cell as the geometric steps see it: an identity, a centre, and an extent,
/// all in the primary frame's steps.
#[derive(Debug, Clone)]
pub struct CellGeometry {
    pub id: EntityId,
    pub centre: [Steps; 2],
    pub footprint: [Steps; 2],
    /// Whether the DOCUMENT fixed this extent or the reader supplied SPEC §5's
    /// default for it. True when the cell states its own `size` or inherits a
    /// tiling's pitch; false when nothing states an extent and
    /// `default_cell_footprint` filled one in.
    ///
    /// It exists because the footprint alone cannot say which happened, and a
    /// consumer that must tell a tile from a point needs to know: a cell the
    /// document places as a bare POINT — a port's exchange cell, a
    /// presentation point — arrives carrying a 900 mm square it never asked
    /// for, and a consumer reading only the number treats it as declared
    /// floor. desvis did, and drew those points as floor tiles coplanar with
    /// the floor they stand in.
    pub footprint_stated: bool,
}

impl CellGeometry {
    pub fn bounds(&self) -> Bounds {
        Bounds::around(self.centre, self.footprint)
    }

    /// Squared centre-to-centre distance, in steps squared. Nothing needs the
    /// root: it is only ever compared against another of the same kind.
    fn distance_squared(&self, other: &CellGeometry) -> i128 {
        let dx = i128::from(self.centre[0].0 - other.centre[0].0);
        let dy = i128::from(self.centre[1].0 - other.centre[1].0);
        dx * dx + dy * dy
    }
}

/// Compile every flow rule, then every stitch, over the post-suppression cells.
pub fn compile(
    entities: &BTreeMap<EntityId, Entity>,
    cells: &BTreeMap<EntityId, CellGeometry>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    frames: &FrameGraph,
) -> Result<Vec<Edge>, LoadError> {
    let mut edges = Vec::new();
    for entity in entities.values() {
        if entity.has("region") && entity.has("tiling") {
            compile_flow(entity, entities, &mut edges)?;
        }
    }
    for entity in entities.values() {
        let Some(stitch) = entity.component::<Stitch>("stitch")? else {
            continue;
        };
        compile_stitch(
            entity,
            &stitch,
            entities,
            cells,
            contributions,
            frames,
            &mut edges,
        )?;
    }
    Ok(edges)
}

/// Arcs among one cell_grid's surviving cells. A cell_grid that declares no
/// `flow` compiles two-way, since flow restricts travel rather than granting it
/// (SPEC §5); the alternative would leave a tiled station spine as a dust of
/// isolated nodes that nothing can move along. A cell an exclusion or a
/// suppression removed is simply absent from the lookup, so no arc can touch it.
fn compile_flow(
    entity: &Entity,
    entities: &BTreeMap<EntityId, Entity>,
    edges: &mut Vec<Edge>,
) -> Result<(), LoadError> {
    let flow: Option<Flow> = entity.component("flow")?;
    let region: Region = entity.require("region")?;
    let tiling: Tiling = entity.require("tiling")?;
    let cols =
        region.size[0]
            .whole_tiles(tiling.pitch[0])
            .ok_or(LoadError::TilingNotDivisible {
                entity: entity.id.clone(),
                axis: "x",
                size: region.size[0],
                pitch: tiling.pitch[0],
            })?;
    let rows =
        region.size[1]
            .whole_tiles(tiling.pitch[1])
            .ok_or(LoadError::TilingNotDivisible {
                entity: entity.id.clone(),
                axis: "y",
                size: region.size[1],
                pitch: tiling.pitch[1],
            })?;

    let alive = |col: usize, row: usize| -> Option<EntityId> {
        let id = EntityId::from(entity.id.derive(KeySegment::parse(&format!("cell:{col},{row}")).expect("cell ordinals form a key segment")));
        entities.contains_key(&id).then_some(id)
    };
    let key = |from: (usize, usize), to: (usize, usize)| {
        format!("edge:cell:{},{}>cell:{},{}", from.0, from.1, to.0, to.1)
    };
    let mut arc = |from: (usize, usize), to: (usize, usize), a: EntityId, b: EntityId| {
        edges.push(Edge {
            owner: entity.id.clone(),
            key: key(from, to),
            from: a,
            to: b,
            bidirectional: false,
            curve: None,
        });
    };

    let rule = flow.as_ref().map_or(FlowRule::TwoWay, |f| f.rule);
    let direction =
        match rule {
            FlowRule::TwoWay => None,
            _ => Some(flow.as_ref().and_then(|f| f.direction).ok_or_else(|| {
                LoadError::Unsupported {
                    entity: entity.id.clone(),
                    message: format!("the flow rule {rule:?} needs a `direction`"),
                }
            })?),
        };

    for col in 0..cols {
        for row in 0..rows {
            let Some(here) = alive(col, row) else {
                continue;
            };
            for (dc, dr) in [(1usize, 0usize), (0, 1)] {
                let (nc, nr) = (col + dc, row + dr);
                if nc >= cols || nr >= rows {
                    continue;
                }
                let Some(there) = alive(nc, nr) else {
                    continue;
                };
                let step_axis = if dc == 1 { 0 } else { 1 };
                let (forward, backward) = permitted(rule, direction, step_axis, col, row);
                if forward {
                    arc((col, row), (nc, nr), here.clone(), there.clone());
                }
                if backward {
                    arc((nc, nr), (col, row), there, here.clone());
                }
            }
        }
    }
    Ok(())
}

/// Which of the two directions across one neighbour pair the rule permits.
/// `step_axis` is the axis the pair straddles; `col`/`row` are the lower cell's
/// indices, which `alternating_rows` needs in order to know whose turn it is.
fn permitted(
    rule: FlowRule,
    direction: Option<Direction>,
    step_axis: usize,
    col: usize,
    row: usize,
) -> (bool, bool) {
    match rule {
        FlowRule::TwoWay => (true, true),
        FlowRule::OneWay => {
            let d = direction.expect("one_way carries a direction");
            if step_axis == d.axis() {
                (d.forward(), !d.forward())
            } else {
                // Crossing from one row to the next stays two-way unless a
                // suppression takes one out.
                (true, true)
            }
        }
        FlowRule::AlternatingRows => {
            let d = direction.expect("alternating_rows carries a direction");
            if step_axis == d.axis() {
                // A "row" is a line of cells running along the flow axis, so it
                // is indexed on the *other* axis: even rows run the stated way.
                let row_index = if d.axis() == 0 { row } else { col };
                let forward = if row_index % 2 == 0 {
                    d.forward()
                } else {
                    !d.forward()
                };
                (forward, !forward)
            } else {
                (true, true)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stitches
// ---------------------------------------------------------------------------

/// Every surviving cell's geometry in the primary frame, which is where SPEC §5
/// says stitch rules are evaluated.
pub fn cell_geometry(
    entities: &BTreeMap<EntityId, Entity>,
    frames: &FrameGraph,
) -> Result<BTreeMap<EntityId, CellGeometry>, LoadError> {
    let mut out = BTreeMap::new();
    for entity in entities.values() {
        if !entity.has("cell") {
            continue;
        }
        let Some(pose) = entity.component::<Pose>("pose")? else {
            continue;
        };
        let (Some(frame), Some(xy)) = (pose.frame.as_ref(), pose.xy()) else {
            continue;
        };
        let centre = frames.to_primary(frame, xy)?;
        let stated = footprint_of(entity, entities)?;
        let footprint = match stated {
            Some(local) => {
                let scale = frames.extent_scale(frame)?;
                [
                    Steps::round_half_up(local[0].0 as f64 * scale[0]),
                    Steps::round_half_up(local[1].0 as f64 * scale[1]),
                ]
            }
            // The default is 900 mm rather than a step count, so it arrives in
            // the primary frame's steps already and is not scaled again.
            None => frames.default_cell_footprint(&entity.id)?,
        };
        out.insert(
            entity.id.clone(),
            CellGeometry {
                id: entity.id.clone(),
                centre,
                footprint,
                footprint_stated: stated.is_some(),
            },
        );
    }
    Ok(out)
}

/// A cell's extent in its own frame: its own `size` if it states one, otherwise
/// the tile it came from. `None` means nothing states it and SPEC §5's 900 mm
/// default applies.
fn footprint_of(
    cell: &Entity,
    entities: &BTreeMap<EntityId, Entity>,
) -> Result<Option<[Steps; 2]>, LoadError> {
    if let Some(component) = cell.component::<Cell>("cell")? {
        if let Some(size) = component.size {
            return Ok(Some(size));
        }
    }
    if let Some(parent) = cell.parent.as_ref().and_then(|p| entities.get(p)) {
        if let Some(tiling) = parent.component::<Tiling>("tiling")? {
            return Ok(Some(tiling.pitch));
        }
    }
    Ok(None)
}

/// Which cells each entity's expansion contributes. A stitch endpoint may be any
/// entity whose expansion contributes cells — a region, a whole rack-row stamp —
/// so this walks each cell up its parent chain and files it under every
/// ancestor, plus under itself when the endpoint *is* a cell.
pub fn cell_contributions(
    entities: &BTreeMap<EntityId, Entity>,
    cells: &BTreeMap<EntityId, CellGeometry>,
) -> BTreeMap<EntityId, Vec<EntityId>> {
    let mut out: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
    for id in cells.keys() {
        let mut cursor = Some(id.clone());
        while let Some(current) = cursor {
            cursor = entities.get(&current).and_then(|e| e.parent.clone());
            out.entry(current).or_default().push(id.clone());
        }
    }
    for list in out.values_mut() {
        list.sort();
    }
    out
}

fn compile_stitch(
    entity: &Entity,
    stitch: &Stitch,
    entities: &BTreeMap<EntityId, Entity>,
    cells: &BTreeMap<EntityId, CellGeometry>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    frames: &FrameGraph,
    edges: &mut Vec<Edge>,
) -> Result<(), LoadError> {
    if stitch.rule == StitchRule::Contact && (!stitch.bidirectional || stitch.curve.is_some()) {
        return Err(LoadError::Unsupported {
            entity: entity.id.clone(),
            message: "direction and curvature require explicit stitch pairs".into(),
        });
    }
    match stitch.rule {
        StitchRule::Explicit => compile_explicit(entity, stitch, entities, cells, frames, edges),
        StitchRule::Contact => compile_contact(entity, stitch, cells, contributions, edges),
    }
}

fn compile_explicit(
    entity: &Entity,
    stitch: &Stitch,
    entities: &BTreeMap<EntityId, Entity>,
    cells: &BTreeMap<EntityId, CellGeometry>,
    frames: &FrameGraph,
    edges: &mut Vec<Edge>,
) -> Result<(), LoadError> {
    let pairs = stitch.pairs.as_deref().unwrap_or(&[]);
    for (index, pair) in pairs.iter().enumerate() {
        if pair.len() != 2 {
            return Err(LoadError::MalformedStitchPair {
                stitch: entity.id.clone(),
                index,
            });
        }
        let a = resolve_address(&pair[0], &entity.id, (index, 0), entities)?;
        let b = resolve_address(&pair[1], &entity.id, (index, 1), entities)?;
        let curve = stitch
            .curve
            .as_ref()
            .map(|arc| {
                let refuse =
                    |message: &str| LoadError::Unsupported { entity: entity.id.clone(), message: message.into() };
                let scale = frames.direction_to_primary(&arc.frame, [1.0, 1.0])?;
                if scale[0].abs() <= f64::EPSILON || (scale[0].abs() - scale[1].abs()).abs() > 1e-9 {
                    return Err(refuse("a circular link requires a nonzero uniform frame scale"));
                }
                let center = frames.to_primary(&arc.frame, arc.center.map(|v| Coord(v.0)))?;
                let first = cells.get(&a).ok_or_else(|| refuse("a curved link endpoint is not a live cell"))?;
                let last = cells.get(&b).ok_or_else(|| refuse("a curved link endpoint is not a live cell"))?;
                let transform = |p: [Steps; 2]| frames.to_primary(&arc.frame, p.map(|v| Coord(v.0)));
                let start = arc.start.map(transform).transpose()?;
                let end = arc.end.map(transform).transpose()?;
                let a = start.unwrap_or(first.centre);
                let b = end.unwrap_or(last.centre);
                let radius = |p: [Steps; 2]| ((p[0].0 - center[0].0) as f64).hypot((p[1].0 - center[1].0) as f64);
                // Cell and centre coordinates are rounded to primary steps. Two
                // steps cover that quantization, not an arbitrary fitting radius.
                if radius(a) <= 2.0 || (radius(a) - radius(b)).abs() > 2.0 || a == b {
                    return Err(refuse("curved link endpoints must be distinct points on the stated circle"));
                }
                let clockwise = arc.clockwise ^ (scale[0] * scale[1] < 0.0);
                for (join, from, to) in [(a, first.centre, a), (b, b, last.centre)] {
                    let d = [(to[0].0 - from[0].0) as f64, (to[1].0 - from[1].0) as f64];
                    let length = d[0].hypot(d[1]);
                    if length <= 2.0 {
                        continue;
                    }
                    let sign = if clockwise { -1.0 } else { 1.0 };
                    let tangent = [
                        -sign * (join[1].0 - center[1].0) as f64 / radius(join),
                        sign * (join[0].0 - center[0].0) as f64 / radius(join),
                    ];
                    if (d[0] / length - tangent[0]).hypot(d[1] / length - tangent[1]) > 1e-4 {
                        return Err(refuse(
                            "a curve's straight lead must meet it tangentially in the traversal direction",
                        ));
                    }
                }
                Ok(ArcGeometry { center, clockwise, start, end })
            })
            .transpose()?;
        edges.push(Edge {
            owner: entity.id.clone(),
            key: format!("link:{index}"),
            from: a,
            to: b,
            bidirectional: stitch.bidirectional,
            curve,
        });
    }
    Ok(())
}

fn resolve_address(
    address: &CellAddress,
    stitch: &EntityId,
    (pair, end): (usize, usize),
    entities: &BTreeMap<EntityId, Entity>,
) -> Result<EntityId, LoadError> {
    let id = match address {
        CellAddress::Id(id) => id.clone(),
        CellAddress::Derived(raw) => raw.parse()?.into(),
        // Only a stamp turns a handle into an identity, and the placeholder
        // walk refuses one before a stitch is compiled — the same refusal a
        // queue path gets, because it is the same address vocabulary.
        CellAddress::Local(RawLocalDerivation { local_ref, .. }) | CellAddress::Handle(LocalRef { local_ref, .. }) => {
            return Err(placeholders::unresolved_handle(stitch, "stitch", format!("pairs[{pair}][{end}]"), local_ref));
        }
        // An explicit pair is used where contact would be wrong, so it has to
        // name both cells outright. Refused rather than guessed at.
        CellAddress::At(_) => {
            return Err(LoadError::PositionAddressUnsupported { entity: stitch.clone(), field: "stitch.pairs" });
        }
    };
    if !entities.contains_key(&id) {
        return Err(LoadError::DanglingReference {
            entity: stitch.clone(),
            component: "stitch".into(),
            field: "pairs".into(),
            target: id,
        });
    }
    Ok(id)
}

/// Contact matching, in the two geometries SPEC §5 distinguishes.
///
/// Which one applies is a fact about the bounding boxes, not a choice. When the
/// two expansions overlap on *both* axes, neither faces the other across
/// anything — one is embedded in the other, the way a stamped station's cells
/// stand inside the floor strip they open onto — and asking which edge faces
/// which is meaningless: it picks an axis by a centre distance that means
/// nothing and then admits only the outermost cell of the line. When the boxes
/// are apart on at least one axis there is a real seam, and the facing machinery
/// is exactly right for it. Both geometries filter before choosing, allow
/// many-to-one, and break ties toward the lower UUID; a stitch that produces no
/// link at all under either fails the load.
fn compile_contact(
    entity: &Entity,
    stitch: &Stitch,
    cells: &BTreeMap<EntityId, CellGeometry>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
    edges: &mut Vec<Edge>,
) -> Result<(), LoadError> {
    let tolerance = stitch.tolerance.unwrap_or(Steps(0));
    let a = endpoint_cells(&entity.id, &stitch.a, cells, contributions)?;
    let b = endpoint_cells(&entity.id, &stitch.b, cells, contributions)?;

    let pairs = match_cells(&a, &b, tolerance);

    if pairs.is_empty() {
        return Err(LoadError::ZeroLinkStitch {
            stitch: entity.id.clone(),
        });
    }
    for (index, (from, to)) in pairs.into_iter().enumerate() {
        edges.push(Edge {
            owner: entity.id.clone(),
            key: format!("link:{index}"),
            from,
            to,
            bidirectional: true,
            curve: None,
        });
    }
    Ok(())
}

fn match_cells(
    a: &[CellGeometry],
    b: &[CellGeometry],
    tolerance: Steps,
) -> Vec<(EntityId, EntityId)> {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let small_bounds = set_bounds(small);
    let large_bounds = set_bounds(large);
    if is_embedded(small_bounds, large_bounds) {
        embedded_pairs(small, large, tolerance)
    } else {
        facing_pairs(small, large, small_bounds, large_bounds, tolerance)
    }
}

/// One expansion sits inside the other's extent: the bounding boxes overlap on
/// both axes, so there is no axis across which the two could face each other.
fn is_embedded(small: Bounds, large: Bounds) -> bool {
    (0..2).all(|axis| small.separation(large, axis).0 < 0)
}

/// The embedded geometry: no facing axis, no boundary layer. Every cell of the
/// smaller expansion looks for the nearest cell of the larger one that is within
/// tolerance on *both* axes, so a line of cells standing on floor cells links
/// along its whole length instead of at its tip.
fn embedded_pairs(
    small: &[CellGeometry],
    large: &[CellGeometry],
    tolerance: Steps,
) -> Vec<(EntityId, EntityId)> {
    let allowance = HalfSteps::of(tolerance);
    let mut pairs = Vec::new();
    for cell in small {
        let bounds = cell.bounds();
        let mut best: Option<(i128, &EntityId)> = None;
        for other in large {
            let other_bounds = other.bounds();
            let within =
                (0..2).all(|axis| bounds.separation(other_bounds, axis).0.max(0) <= allowance.0);
            if !within {
                continue;
            }
            let candidate = (cell.distance_squared(other), &other.id);
            if best.is_none_or(|current| candidate < current) {
                best = Some(candidate);
            }
        }
        if let Some((_, other)) = best {
            pairs.push((cell.id.clone(), other.clone()));
        }
    }
    pairs
}

/// The separated geometry: a real seam, matched nearest-facing across it. Only
/// the smaller expansion's boundary layer participates — the cells whose facing
/// edge is the whole set's facing edge — and among the candidates that clear
/// both the normal-gap and the tangential test, the nearest wins. A ragged
/// boundary therefore links only its outermost layer; every seam in the fixtures
/// is flat, and SPEC-GAPS 36 keeps the question open.
fn facing_pairs(
    small: &[CellGeometry],
    large: &[CellGeometry],
    small_bounds: Bounds,
    large_bounds: Bounds,
    tolerance: Steps,
) -> Vec<(EntityId, EntityId)> {
    let axis = facing_axis(small_bounds, large_bounds);
    let tangent = 1 - axis;
    let allowance = HalfSteps::of(tolerance);

    // Which way the other endpoint lies, and therefore which edge of the smaller
    // set faces it.
    let toward_plus = large_bounds.centre()[axis] >= small_bounds.centre()[axis];
    let facing_edge = small_bounds.edge(axis, toward_plus);

    let mut pairs = Vec::new();
    for cell in small {
        let bounds = cell.bounds();
        if bounds.edge(axis, toward_plus) != facing_edge {
            continue;
        }
        let mut best: Option<(i64, i64, &EntityId)> = None;
        for other in large {
            let gap = bounds.separation(other.bounds(), axis).0.max(0);
            let offset = (cell.centre[tangent].0 - other.centre[tangent].0).abs() * 2;
            let widest = cell.footprint[tangent].0.max(other.footprint[tangent].0);
            if gap > allowance.0 || offset > allowance.0 + widest {
                continue;
            }
            let candidate = (gap, offset, &other.id);
            // Nearest by normal gap, then by tangential offset, then by the
            // lower identity so a tie resolves the same way in every
            // implementation.
            if best.is_none_or(|current| candidate < current) {
                best = Some(candidate);
            }
        }
        if let Some((_, _, other)) = best {
            pairs.push((cell.id.clone(), other.clone()));
        }
    }
    pairs
}

fn endpoint_cells(
    stitch: &EntityId,
    endpoint: &EntityId,
    cells: &BTreeMap<EntityId, CellGeometry>,
    contributions: &BTreeMap<EntityId, Vec<EntityId>>,
) -> Result<Vec<CellGeometry>, LoadError> {
    let empty = || LoadError::StitchEndpointHasNoCells {
        stitch: stitch.clone(),
        endpoint: endpoint.clone(),
    };
    let ids = contributions.get(endpoint).ok_or_else(empty)?;
    let out: Vec<CellGeometry> = ids.iter().filter_map(|id| cells.get(id)).cloned().collect();
    if out.is_empty() {
        return Err(empty());
    }
    Ok(out)
}

fn set_bounds(cells: &[CellGeometry]) -> Bounds {
    cells
        .iter()
        .map(CellGeometry::bounds)
        .reduce(Bounds::union)
        .expect("an endpoint with no cells is refused before this point")
}

/// Which axis the two expansions face each other across.
///
/// Two sets can only face each other along an axis they are actually apart on —
/// you cannot look at something through ground you are standing on — so an axis
/// that separates the two bounding boxes wins outright. When both axes separate
/// them, as a diagonal neighbour does, the tie goes to the axis whose
/// bounding-box centres are farther apart. Only [`facing_pairs`] calls this, so
/// the case where *neither* axis separates them never arrives here: that is the
/// embedded geometry, and it has no facing axis. SPEC §5 states this rule as
/// written since v0.3.
fn facing_axis(a: Bounds, b: Bounds) -> usize {
    let separated: Vec<usize> = (0..2).filter(|&ax| a.separation(b, ax).0 >= 0).collect();
    if separated.len() == 1 {
        return separated[0];
    }
    let ca = a.centre();
    let cb = b.centre();
    let dx = (ca[0].0 - cb[0].0).abs();
    let dy = (ca[1].0 - cb[1].0).abs();
    if dx >= dy { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use uuid::Uuid;

    fn id(s: &str) -> EntityId {
        EntityId::declared(Uuid::from_str(s).unwrap())
    }

    fn geometry(idstr: &str, centre: [i64; 2], footprint: [i64; 2]) -> CellGeometry {
        CellGeometry {
            id: id(idstr),
            centre: [Steps(centre[0]), Steps(centre[1])],
            footprint: [Steps(footprint[0]), Steps(footprint[1])],
            // Every cell these tests build states its own extent, and nothing
            // in stitch geometry reads the flag.
            footprint_stated: true,
        }
    }

    #[test]
    fn a_separating_axis_beats_a_farther_pair_of_centres() {
        // The fixture's approach corridor and station spine: they share the
        // boundary x = 22100, but the spine is so much longer north-south that
        // its centre sits 4500 steps away in y and only 2250 away in x.
        let corridor = set_bounds(&[
            geometry(
                "11111111-0000-4000-8000-000000000001",
                [18950, 5750],
                [900, 900],
            ),
            geometry(
                "11111111-0000-4000-8000-000000000002",
                [21650, 5750],
                [900, 900],
            ),
        ]);
        let spine = set_bounds(&[
            geometry(
                "22222222-0000-4000-8000-000000000001",
                [22550, 3050],
                [900, 900],
            ),
            geometry(
                "22222222-0000-4000-8000-000000000002",
                [22550, 17450],
                [900, 900],
            ),
        ]);
        assert_eq!(facing_axis(corridor, spine), 0);
    }

    #[test]
    fn centres_decide_when_neither_axis_separates() {
        // A station cell sitting beside the middle of a long spine overlaps it on
        // both axes; the larger centre distance is the only signal left.
        let station = set_bounds(&[geometry(
            "11111111-0000-4000-8000-000000000001",
            [22000, 12000],
            [900, 900],
        )]);
        let spine = set_bounds(&[
            geometry(
                "22222222-0000-4000-8000-000000000001",
                [22550, 3050],
                [900, 900],
            ),
            geometry(
                "22222222-0000-4000-8000-000000000002",
                [22550, 17450],
                [900, 900],
            ),
        ]);
        assert_eq!(facing_axis(station, spine), 1);
    }

    #[test]
    fn one_way_keeps_row_changes_two_way() {
        // Along the flow axis only the stated direction survives; across it both.
        assert_eq!(
            permitted(FlowRule::OneWay, Some(Direction::PlusY), 1, 0, 0),
            (true, false)
        );
        assert_eq!(
            permitted(FlowRule::OneWay, Some(Direction::PlusY), 0, 0, 0),
            (true, true)
        );
        assert_eq!(
            permitted(FlowRule::OneWay, Some(Direction::MinusX), 0, 0, 0),
            (false, true)
        );
    }

    #[test]
    fn alternating_rows_turns_every_other_row_around() {
        // Flow along x, so a "row" is indexed by its y position.
        assert_eq!(
            permitted(FlowRule::AlternatingRows, Some(Direction::PlusX), 0, 3, 0),
            (true, false)
        );
        assert_eq!(
            permitted(FlowRule::AlternatingRows, Some(Direction::PlusX), 0, 3, 1),
            (false, true)
        );
    }

    /// A line of cells standing inside a larger field: six cells on six floor
    /// cell centres, which is the shape a stamped row inside an open floor has.
    /// Every one of them must link, and each to the floor cell it stands on.
    #[test]
    fn an_embedded_line_links_every_cell_to_the_one_it_stands_on() {
        let line: Vec<CellGeometry> = (0..6)
            .map(|i| {
                geometry(
                    &format!("11111111-0000-4000-8000-{i:012}"),
                    [5500, 2500 + 1000 * i64::from(i)],
                    [900, 900],
                )
            })
            .collect();
        let mut field = Vec::new();
        for col in 0..8 {
            for row in 0..18 {
                field.push(geometry(
                    &format!("22222222-0000-4000-8000-{:012}", col * 100 + row),
                    [500 + 1000 * i64::from(col), 500 + 1000 * i64::from(row)],
                    [1000, 1000],
                ));
            }
        }

        let line_bounds = set_bounds(&line);
        let field_bounds = set_bounds(&field);
        assert!(is_embedded(line_bounds, field_bounds));

        // Coincident centres mean the gap is zero on both axes, so even a
        // tolerance of zero links them.
        let pairs = embedded_pairs(&line, &field, Steps(0));
        assert_eq!(pairs.len(), 6);
        for (i, (from, to)) in pairs.iter().enumerate() {
            assert_eq!(*from, line[i].id);
            let target = field.iter().find(|c| c.id == *to).expect("a field cell");
            assert_eq!(target.centre, line[i].centre, "link {i} left its own tile");
        }

        // What the embedded rule exists for: the facing machinery admits only
        // the extreme cell of the line, because the boundary layer of a line
        // buried in a field is one cell.
        assert_eq!(
            facing_pairs(&line, &field, line_bounds, field_bounds, Steps(100)).len(),
            1
        );
    }

    /// A cell far enough out that no candidate clears the tolerance links to
    /// nothing, embedded or not — the rule loosens which cells may participate,
    /// never how close they must be.
    #[test]
    fn an_embedded_cell_out_of_tolerance_still_links_to_nothing() {
        let stray = vec![geometry(
            "11111111-0000-4000-8000-000000000001",
            [5500, 2500],
            [100, 100],
        )];
        let field = vec![
            geometry(
                "22222222-0000-4000-8000-000000000001",
                [500, 500],
                [1000, 1000],
            ),
            geometry(
                "22222222-0000-4000-8000-000000000002",
                [9500, 9500],
                [1000, 1000],
            ),
        ];
        assert!(is_embedded(set_bounds(&stray), set_bounds(&field)));
        assert!(embedded_pairs(&stray, &field, Steps(100)).is_empty());
    }

    /// A real seam is still matched across its facing axis, unchanged: the
    /// fixture's highway and aisle are 200 steps apart in x and overlap in y, so
    /// the aisle's two minimum-x cells each find a highway partner.
    #[test]
    fn a_separated_seam_keeps_the_facing_geometry() {
        let highway: Vec<CellGeometry> = (0..33)
            .map(|row| {
                geometry(
                    &format!("22222222-0000-4000-8000-{row:012}"),
                    [1350, 450 + 900 * i64::from(row)],
                    [900, 900],
                )
            })
            .collect();
        let aisle: Vec<CellGeometry> = (0..2)
            .map(|row| {
                geometry(
                    &format!("11111111-0000-4000-8000-{row:012}"),
                    [2450, 5750 + 900 * i64::from(row)],
                    [900, 900],
                )
            })
            .collect();
        let aisle_bounds = set_bounds(&aisle);
        let highway_bounds = set_bounds(&highway);
        assert!(!is_embedded(aisle_bounds, highway_bounds));
        assert_eq!(facing_axis(aisle_bounds, highway_bounds), 0);
        assert_eq!(
            facing_pairs(&aisle, &highway, aisle_bounds, highway_bounds, Steps(250)).len(),
            2
        );
        // The 200-step seam is real: halve the tolerance and nothing reaches.
        assert!(
            facing_pairs(&aisle, &highway, aisle_bounds, highway_bounds, Steps(100)).is_empty()
        );
    }

    /// A cell whose footprint is an odd number of steps has edges halfway
    /// between two steps, and the comparison is still exact: 901-wide cells
    /// 901 apart touch, and one step more apart do not.
    #[test]
    fn an_odd_footprint_keeps_its_edges_exact() {
        let a = geometry("11111111-0000-4000-8000-000000000001", [0, 0], [901, 901]);
        let touching = geometry("22222222-0000-4000-8000-000000000001", [901, 0], [901, 901]);
        let apart = geometry("22222222-0000-4000-8000-000000000002", [902, 0], [901, 901]);
        assert_eq!(a.bounds().separation(touching.bounds(), 0), HalfSteps(0));
        assert_eq!(a.bounds().separation(apart.bounds(), 0), HalfSteps(2));
    }

    #[test]
    fn two_way_needs_no_direction() {
        assert_eq!(permitted(FlowRule::TwoWay, None, 0, 0, 0), (true, true));
    }
}
