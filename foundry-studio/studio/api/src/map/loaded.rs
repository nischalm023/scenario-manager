//! Within `map`, `loaded` is a site's flagged export loaded with foundry's reader and read as the Map panel's routes
//! need it (spec 03 sections 3.5 and 3.6): each entity by id with its label, components, place and members, counted by
//! component set, which of the export's scopes holds each declared id (as the export's check found them), the frames
//! positions are given and drawn in, and the floor compiled beside it. Blocking work, run on the compute pool by
//! `read`.

use bytes::Bytes;
use dessim::worldmap::WorldMap;
use dessim::worldmap::census::EntityPlace;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use studio_protocol::{CountRow, DeclaredId, EntityCount, EntityId, EntityRef, FrameAxes, Mm, ScopePath, UnitMm};
use wps_client::CutExport;

use crate::floors::{self, FloorBytes};
/// What the loaded export holds, for the summary, find and marks.
pub struct Loaded {
    /// The site's name, as the site list gives it.
    pub name: Option<String>,
    pub primary: EntityId,
    pub unit_mm: UnitMm,
    pub axes: FrameAxes,
    pub counts: Vec<CountRow>,
    pub entities: FxHashMap<EntityId, Entry>,
    pub owners: FxHashMap<DeclaredId, ScopePath>,
    pub map_empty: bool,
    /// The floor document's size, or why the export has no floor.
    pub floor: Result<FloorBytes, String>,
}

/// One entity of the loaded export.
pub struct Entry {
    pub label: String,
    pub components: Vec<String>,
    pub place: Place,
    /// A sector's members, as its `sector` component lists them.
    pub members: Vec<EntityId>,
    /// Its `external_ids` values, which find matches.
    pub external: Vec<String>,
}

/// Where an entity stands, as spec 03 section 3.5 tells its marks apart: on floor cells, which outline; somewhere
/// else on the floor, listed only (a rack's steel, a slot, a region); or nowhere (a sector, a stitch, a cohort).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Cells([Mm; 2]),
    Elsewhere([Mm; 2]),
    Nowhere,
}

/// The export loaded and read (blocking: the compute pool's), with its floor document when it has one; `name` is the
/// site's.
pub(super) fn load(
    export: &CutExport,
    map: &ScopePath,
    name: Option<String>,
) -> (Result<Loaded, String>, Option<Bytes>) {
    let world = match floors::load(&export.bytes) {
        Ok(world) => world,
        Err(why) => return (Err(why.to_string()), None),
    };
    let floor = floors::compile(&world).and_then(|compiled| floors::encode(&compiled));
    let size = floor.as_ref().map(|bytes| FloorBytes::of(bytes)).map_err(ToString::to_string);
    let read = index(&world, export, map, name, size);
    (read, floor.ok())
}

fn index(
    world: &WorldMap,
    export: &CutExport,
    map: &ScopePath,
    name: Option<String>,
    floor: Result<FloorBytes, String>,
) -> Result<Loaded, String> {
    let frame =
        world.world.frames.primary().ok_or("no frame of the site states unit_mm, so it has no primary frame")?;
    let primary: EntityId =
        frame.to_string().parse().map_err(|fault| format!("the primary frame {frame} is no entity id: {fault}"))?;
    let owners = export.holders.clone();
    let map_empty = !owners.values().any(|scope| scope == map);
    let mut entities = FxHashMap::default();
    let mut sets: FxHashMap<Vec<String>, u64> = FxHashMap::default();
    for row in &world.entities {
        let Ok(id) = row.id.to_string().parse::<EntityId>() else { continue };
        let components: Vec<String> = row.components.iter().map(|name| name.to_string()).collect();
        *sets.entry(components.clone()).or_default() += 1;
        let declared = world.world.entities.get(&row.id);
        let component = |name: &str| declared.and_then(|entity| entity.components.get(name));
        let members = component("sector")
            .and_then(|sector| sector.get("members")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|member| Some(member.as_str()?.parse::<EntityRef>().ok()?.id().clone()))
            .collect();
        let external = component("external_ids")
            .and_then(Value::as_object)
            .map(|ids| {
                ids.values().map(|value| value.as_str().map_or_else(|| value.to_string(), str::to_owned)).collect()
            })
            .unwrap_or_default();
        let label = row.name.clone().unwrap_or_else(|| format!("{} {}", components.join(" + "), &id.to_string()[..8]));
        entities.insert(id, Entry { label, components, place: place_of(world, &row.place), members, external });
    }
    let mut counts: Vec<CountRow> = sets
        .into_iter()
        .map(|(components, count)| CountRow {
            label: components.join(" + "),
            components,
            count: EntityCount::new(count),
        })
        .collect();
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    let (unit_mm, axes) = frames(world, &primary)?;
    Ok(Loaded { name, primary, unit_mm, axes, counts, entities, owners, map_empty, floor })
}

/// The centre of where an entity stands, in whole millimetres of the primary frame.
fn place_of(world: &WorldMap, place: &EntityPlace) -> Place {
    let whole = |x: f64, y: f64| [Mm::new(x.round() as i64), Mm::new(y.round() as i64)];
    let centre = |[x, y, w, h]: [f64; 4]| whole(x + w / 2.0, y + h / 2.0);
    match place {
        EntityPlace::Cells(cells) => {
            let at: Vec<[f64; 2]> = cells
                .iter()
                .filter_map(|cell| world.map.graph.cell(*cell)?.world_pos)
                .map(|at| [at.x_mm, at.y_mm])
                .collect();
            if at.is_empty() {
                return Place::Nowhere;
            }
            let (lo, hi) = at.iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), [x, y]| {
                ([lo[0].min(*x), lo[1].min(*y)], [hi[0].max(*x), hi[1].max(*y)])
            });
            Place::Cells(centre([lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]]))
        }
        EntityPlace::Rect(rect) | EntityPlace::Prism { rect, .. } => Place::Elsewhere(centre(*rect)),
        EntityPlace::Point([x, y, _]) => Place::Elsewhere(whole(*x, *y)),
        EntityPlace::Nowhere => Place::Nowhere,
    }
}

/// What one step of the primary frame is worth in millimetres, as its `unit_mm` states it, refused when it states none
/// a step can be; and the axes drawn: of the frame most entities state their place in, pointing as the scales on its
/// way to the primary frame turn them (a negative scale is the protocol's mirror).
pub(crate) fn frames(world: &WorldMap, primary: &EntityId) -> Result<(UnitMm, FrameAxes), String> {
    let frame_of = |id: &EntityId| {
        let found = world.world.entities.values().find(|entity| entity.id.to_string() == id.to_string())?;
        Some((found.name.clone(), found.components.get("frame")?.clone()))
    };
    let unit_mm = frame_of(primary)
        .and_then(|(_, frame)| frame.get("unit_mm").and_then(Value::as_f64))
        .and_then(UnitMm::new)
        .ok_or(
            "the site's primary frame states no unit_mm above zero, what one of its steps is worth in millimetres",
        )?;
    let mut stated: FxHashMap<String, u32> = FxHashMap::default();
    for entity in world.world.entities.values() {
        for component in entity.components.values() {
            if let Some(frame) = component.get("frame").and_then(Value::as_str) {
                *stated.entry(frame.to_owned()).or_default() += 1;
            }
        }
    }
    // ordered: the most stated frame, ties to the primary and then by id, so one document always names one frame.
    let mut ranked: Vec<(String, u32)> = stated.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| (a.0 != primary.to_string()).cmp(&(b.0 != primary.to_string())))
            .then_with(|| a.0.cmp(&b.0))
    });
    let drawn: EntityId = ranked.first().and_then(|(id, _)| id.parse().ok()).unwrap_or_else(|| primary.clone());
    let (mut sign, mut at, mut seen) = ([1, 1], drawn.clone(), FxHashSet::default());
    while at != *primary && seen.insert(at.to_string()) {
        let Some((_, frame)) = frame_of(&at) else { break };
        for (axis, flip) in sign.iter_mut().enumerate() {
            let scale = frame.pointer(&format!("/transform_to_parent/scale/{axis}")).and_then(Value::as_f64);
            if scale.is_some_and(|scale| scale < 0.0) {
                *flip = -*flip;
            }
        }
        match frame.get("parent").and_then(Value::as_str).and_then(|parent| parent.parse().ok()) {
            Some(parent) => at = parent,
            None => break,
        }
    }
    let name = frame_of(&drawn).and_then(|(name, _)| name).unwrap_or_else(|| "primary frame".to_owned());
    Ok((unit_mm, FrameAxes { frame: drawn, name, x: [sign[0], 0], y: [0, sign[1]] }))
}

#[cfg(test)]
mod tests;
