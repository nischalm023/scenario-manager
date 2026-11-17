//! Within the Map Creator adapter, `export` is **the site becoming a dump**: run
//! every lens over a graph of the site's entities, hand the objects to the
//! serializers, and assemble the files a Map Creator export is.
//!
//! A host holds a site's live entities rather than a loaded world, so the graph
//! is built from entities alone (`Graph::of_entities`), and nothing here reads
//! anything an entity does not state: no traversability arc, no resolved
//! position. The record order a file states rides in each record's
//! `import_formatting` entry, as its `index`, and is restored here, once,
//! because a lens list comes back in identity order and the identities are
//! random.

use crate::dump::{Dump, Error};
use crate::identity::Existing;
use crate::kinds::FileKind;
use crate::format::steps::is_step;
use crate::references;
use crate::serialize::{component, formatting, Projection};
use crate::site::{owned, Held};
use rustc_hash::FxHashMap;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;
use worldadapt_sdk::strip_comments;
use worldlens::{Graph, Library, Matcher, Object};
use worldprotocol_core::{Entity, EntityId};

/// Every lens the adapter ships, in the order the record kinds are written.
///
/// The list is here rather than read off the library because the serializers name
/// these lenses: a library that has gained a lens the adapter does not serialize
/// is fine, and one that has lost a lens the adapter needs is a defect the export
/// names rather than a file that quietly comes out empty.
pub const LENSES: &[&str] = &[
    "mapcreator_export",
    "mapcreator_frame",
    "mapcreator_cell",
    "mapcreator_location_profile",
    "mapcreator_zone",
    "mapcreator_sector",
    "mapcreator_aisle",
    "mapcreator_block",
    "mapcreator_station",
    "mapcreator_presentation_point",
    "mapcreator_station_point",
    "mapcreator_charger",
    "mapcreator_dock_point",
    "mapcreator_ods_exclusion",
    "mapcreator_conveyor",
    "mapcreator_conveyor_mapping",
    "mapcreator_tote_storage",
    "mapcreator_rack_type",
    "mapcreator_sector_preference",
    "mapcreator_pps_bin",
    "mapcreator_pps_bin_type",
    "mapcreator_pps_seat",
    "mapcreator_pptl",
];

/// Write the site out as the dump it was ingested from.
///
/// Only this adapter's entities go into the graph, each with every `$comment`
/// annotation stripped: an entity carrying none of its lookup keys is never
/// written out, whatever it carries, and an author's note is never part of a
/// file. Two entities under one key are refused, since the files would state
/// one record twice and the next ingest could not say which entity it is.
pub fn of_site(held: &[Held<'_>]) -> Result<Dump, String> {
    let owned = owned(held)?;
    if let Some((key, entities)) = Existing::of_entities(held)?.ambiguities().into_iter().next() {
        let named: Vec<String> = entities.iter().map(Uuid::to_string).collect();
        return Err(format!(
            "entities {} carry one Map Creator key, {key}, so the files would state that record more than once",
            named.join(" and ")
        ));
    }
    let mut entities: BTreeMap<EntityId, Entity> = BTreeMap::new();
    for (held, _) in owned {
        let uuid = Uuid::parse_str(held.id)
            .map_err(|_| format!("the host handed over entity {:?}, whose identity is not a UUID", held.id))?;
        let id = EntityId::declared(uuid);
        let mut entity = Entity::new(id.clone());
        entity.name = held.name.map(str::to_string);
        entity.components = held.components.iter().map(|(name, value)| (name.clone(), strip_comments(value))).collect();
        entities.insert(id, entity);
    }
    let graph = Graph::of_entities(entities);
    let library = Library::parse(crate::lenses()).map_err(|error| error.to_string())?;
    Ok(export(&graph, &library)?)
}

/// Write the whole dump back out of a graph.
///
/// A site an export was ingested into carries a `mapcreator.export` entity — the
/// file inventory, which nothing else can state — and is written back file by
/// file, exactly. A site that carries none was never ingested from a Map Creator
/// export, and this adapter writes out nothing else (`project` refuses it).
pub fn export(graph: &Graph<'_>, library: &Library) -> Result<Dump, Error> {
    let projection = project(graph, library, LENSES)?;
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    for kind in FileKind::ALL {
        if let Some(value) = projection.file(kind) {
            files.insert(kind.name().to_string(), value?);
        }
    }
    let mut dump = Dump::from_files(files)?;
    for kind in dump.present() {
        dump.set_spacing(kind, projection.spacing(kind));
    }
    dump.set_arrangement(projection.arrangement());
    Ok(dump)
}

/// Run the named lenses over one graph and index what they project, every
/// reference the serializers will read resolved in the index first.
fn project(graph: &Graph<'_>, library: &Library, names: &[&'static str]) -> Result<Projection, Error> {
    let matcher = Matcher::over(graph);
    let mut objects: FxHashMap<&'static str, Vec<Object>> = FxHashMap::default();
    for name in names {
        let lens = library.lens(name).ok_or_else(|| {
            Error::Lens(format!("the lens library carries no lens `{name}`, which this export writes a file out of"))
        })?;
        let mut projected = matcher.project(lens);
        // ordered: a projection comes back in identity order and the identities
        // are random, so the record order rides in the formatting (spec §5) and
        // is restored here, once, for every serializer downstream.
        projected.sort_by_cached_key(|object| record_index(object, name));
        objects.insert(name, projected);
    }
    if objects.get("mapcreator_export").is_none_or(Vec::is_empty) {
        return Err(Error::Document(
            "this site holds no Map Creator export entity, and this adapter writes out only a site that was \
             ingested from a Map Creator export"
                .to_string(),
        ));
    }
    // One export is one inventory, one site frame and one floor: of two, the
    // files would be written from one and the other's facts left out.
    let frames = objects.get("mapcreator_frame").map(Vec::as_slice).unwrap_or_default();
    let role = |object: &&Object, role: &str| {
        component(object, "mapcreator.frame").get("role").and_then(Value::as_str) == Some(role)
    };
    for (what, held) in [
        ("export inventory, `mapcreator.export`,", objects["mapcreator_export"].iter().collect::<Vec<_>>()),
        ("site frame, `mapcreator.frame.role` site,", frames.iter().filter(|frame| role(frame, "site")).collect()),
        ("floor frame, `mapcreator.frame.role` floor,", frames.iter().filter(|frame| role(frame, "floor")).collect()),
    ] {
        if let [_, _, ..] = held.as_slice() {
            let named: Vec<String> = held.iter().map(|object| object.anchor.id.to_string()).collect();
            return Err(Error::Document(format!(
                "entities {} each carry this adapter's {what} and an export has one: the files would be written \
                 from one of them and the others' facts left out",
                named.join(" and ")
            )));
        }
    }
    // What one step of the site frame is worth, read off the adapter's frame
    // whose role is `site`: the frame an ingest reads the held unit off too, so
    // the two directions cannot read two different units. Every position is
    // written back through it, so a site without it is not written at all.
    let unit_mm = objects
        .get("mapcreator_frame")
        .into_iter()
        .flatten()
        .find(|object| component(object, "mapcreator.frame").get("role").and_then(Value::as_str) == Some("site"))
        .and_then(|object| component(object, "frame").get("unit_mm").and_then(Value::as_f64))
        .ok_or_else(|| {
            Error::Document(
                "this site holds no Map Creator site frame stating `frame.unit_mm` as a number, which says what one \
                 step is worth in millimetres, so no position could be written back"
                    .to_string(),
            )
        })?;
    // The ingest counts in these steps and no other, since only from these is
    // a count carried back into the very millimetres the export wrote.
    if !is_step(unit_mm) {
        return Err(Error::Document(format!(
            "this site's Map Creator site frame states `frame.unit_mm` as {unit_mm}, and a count of steps is carried \
             back into the millimetres a Map Creator file states exactly only in steps of 1, 0.5, 0.25 or 0.1 mm, \
             the ones an ingest counts in, so no position could be written back as the length it counts"
        )));
    }
    let projection = Projection::index(objects, unit_mm);
    // Every record is in a file the projection writes records into, and every
    // reference a serializer reads names a record it can write back, or
    // nothing is written: a serializer that could not spell one would leave it
    // out, or write an identity where the file keeps a key.
    projection.inventory()?;
    references::check(&projection)?;
    Ok(projection)
}

/// One object's position in its file, off its formatting entry for the lens it
/// was projected through, the record's own, never off an `index` another entry
/// carries. Every record states one; a frame and the export header do not,
/// and they are single records anyway.
fn record_index(object: &Object, lens: &str) -> i64 {
    formatting(object, lens).get("index").and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::LENSES;
    use worldlens::Library;

    /// Every lens the export runs is in the library it ships, so no file comes
    /// out empty for want of one.
    #[test]
    fn every_lens_the_export_runs_is_in_the_library() {
        let library = Library::parse(crate::lenses()).expect("the shipped library parses");
        for name in LENSES {
            assert!(library.lens(name).is_some(), "the library carries no `{name}`");
        }
    }
}
