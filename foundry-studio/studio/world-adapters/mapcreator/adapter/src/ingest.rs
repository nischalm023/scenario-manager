//! Within the Map Creator adapter, `ingest` is **the import's entities set
//! against the site**: what the files say the site should hold, compared entity
//! by entity with what it holds, and turned into the changes that close the
//! gap.
//!
//! The crate this adapter was ported from wrote a whole document and left the
//! comparison to whoever loaded it. A module is handed the site and answers
//! with changes, so the comparison is here, and it follows four rules:
//!
//! - A record nothing in the site answers for is declared, under the identity
//!   the host issued for it. An entity the import would write differently is
//!   patched, naming only the components whose values differ, each whole, and
//!   its label when the label differs. An entity the import would write the
//!   same is left alone. An entity of this adapter's that no record claimed is
//!   deleted.
//! - Two components are compared as the host spells them, with every `$comment`
//!   annotation stripped, so a note somebody added never causes a change by
//!   itself, and a whole number the host wrote as an integer is the number the
//!   import computed as a float. A component the adapter does rewrite is
//!   written as the import computes it, so a note on it is replaced.
//! - `external_ids` and `import_formatting` are shared: other systems keep
//!   their keys for an entity in the one, and other importers their entries of
//!   how a file wrote it in the other. Only this adapter's own keys and entries
//!   are compared and rewritten, and every other one, with its note, is written
//!   back as it was.
//! - What no change can say is refused with a sentence: a pinned entity the
//!   files would patch or delete, a component the import would stop writing,
//!   and an entity to delete that another system still knows by a key of its
//!   own, or whose formatting another importer still keeps an entry in. No
//!   change removes a component, so the adapter cannot detach from either. And an entity the import would write without something its kind
//!   is written back from (`site::needs`) is refused, since the projection
//!   would refuse the site it left.
//!
//! The changes come out in an order that depends on the input alone: the
//! declares and patches in the order the import wrote the entities, then the
//! deletes in the order the host handed the site over.

use crate::identity::{constructs_of, Existing};
use crate::records;
use crate::site::{lacking, shape_of, writes, written_for_shape, Held};
use crate::vocabulary::{declared_kinds, external_id_keys, mistyped};
use crate::written;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value};
use uuid::Uuid;
use worldadapt_sdk::{Change, EntityWrite};
use worldcanonical::to_canonical_string;

/// The changes that make the site hold what the import says it should.
///
/// `imported` is the import's entities, each an `{id, name, components}`
/// record; `site` is every entity the host handed over; `unclaimed` is this
/// adapter's entities no record claimed, in the order the host handed them over.
pub fn changes(
    imported: Vec<Value>,
    site: &[Held<'_>],
    existing: &Existing,
    unclaimed: &[Uuid],
) -> Result<Vec<Change>, String> {
    let by_id: FxHashMap<&str, &Held<'_>> = site.iter().map(|entity| (entity.id, entity)).collect();
    let pinned: FxHashMap<&str, &Held<'_>> = site.iter().filter(|held| !held.own).map(|held| (held.id, held)).collect();
    let declared = declared_kinds();
    let own_keys: FxHashSet<&'static str> = external_id_keys().into_iter().map(|(key, _)| key).collect();
    let own_entries: FxHashSet<&'static str> = written::keys().collect();
    let mut written: FxHashSet<String> = FxHashSet::default();
    let mut out = Vec::new();
    for entity in imported {
        let Value::Object(mut record) = entity else {
            return Err("the import wrote an entity that is not a record".to_string());
        };
        let id = record.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let name = record.get("name").and_then(Value::as_str).map(str::to_string);
        let Some(Value::Object(components)) = record.remove("components") else {
            return Err(format!("the import wrote entity {id} with no components"));
        };
        let construct = ids_of(&components)
            .and_then(|ids| constructs_of(ids).first().map(|(construct, _)| *construct))
            .unwrap_or_default();
        if let Some(need) = lacking(construct, |name| components.get(name)) {
            return Err(format!(
                "the import would write an entity the projection refuses: {}",
                need.refusal(&id, construct)
            ));
        }
        // A field the file states in a JSON kind its component's published
        // schema does not take is refused here, by the record, rather than by
        // a server after the adapter has answered with changes.
        for (component, payload) in &components {
            let Some(fields) = declared.get(component.as_str()) else { continue };
            let Some(payload) = payload.as_object() else { continue };
            if let Some((at, value, kinds)) = mistyped(fields, payload, "") {
                // A record's formatting is keyed by its kind's lens, whose file
                // wrote the field.
                let file = match component.as_str() {
                    written::COMPONENT => {
                        let key = at.split('.').next().unwrap_or_default();
                        records::KINDS.iter().find(|kind| kind.lens == key)
                    }
                    _ => records::KINDS.iter().find(|kind| kind.component == component),
                };
                return Err(format!(
                    "{} states the field it carries in `{component}.{at}` as {}, where that component's published \
                     schema takes {}: a server would refuse the change, so the ingest does, for entity {id}{}",
                    file.map_or("the export".to_string(), |kind| kind.file.name().to_string()),
                    shown(value),
                    kinds.join(" or "),
                    key_of(&components).map(|key| format!(" (the {key})")).unwrap_or_default()
                ));
            }
        }
        if !written.insert(id.clone()) {
            return Err(format!(
                "the files state two records that the site holds as one entity, {id}{}: each record of an export \
                 is one entity, so one of the two is a record the files repeat",
                key_of(&components).map(|key| format!(" (the {key})")).unwrap_or_default()
            ));
        }
        // The record's key, for a sentence refusing a reference into a pinned
        // scope; read only where the site pins one, since it is read for
        // every entity an ingest writes.
        let named = match pinned.is_empty() {
            true => String::new(),
            false => key_of(&components).map(|key| format!(" (the {key})")).unwrap_or_default(),
        };
        match by_id.get(id.as_str()) {
            None => {
                let mut write = EntityWrite::new(id);
                if let Some(name) = name {
                    write = write.named(name);
                }
                for (component, value) in components {
                    write = write.with(component, value);
                }
                naming_no_pinned_entity(&write, &named, &pinned)?;
                out.push(Change::Declare(write));
            }
            Some(held) => {
                if let Some(patch) = patch(held, construct, name, components, (&own_keys, &own_entries))? {
                    if !held.own {
                        return Err(format!(
                            "the files would change entity {}{}, which belongs to the pinned scope {}: this site \
                             reads a pinned scope's entities and never changes them",
                            held.id,
                            held.external_ids()
                                .and_then(key_of_ids)
                                .map(|key| format!(" (the {key})"))
                                .unwrap_or_default(),
                            held.scope
                        ));
                    }
                    naming_no_pinned_entity(&patch, &named, &pinned)?;
                    out.push(Change::Patch(patch));
                }
            }
        }
    }
    for id in unclaimed {
        let text = id.to_string();
        let Some(held) = by_id.get(text.as_str()) else { continue };
        let key = held.external_ids().and_then(key_of_ids).unwrap_or_default();
        if !held.own {
            return Err(format!(
                "the files hold no record for entity {} (the {key}), which belongs to the pinned scope {}: this \
                 site reads a pinned scope's entities and never deletes them",
                held.id, held.scope
            ));
        }
        for (component, own) in [("external_ids", &own_keys), (written::COMPONENT, &own_entries)] {
            if let Some(foreign) = held
                .components
                .get(component)
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|shared| shared.keys())
                .find(|key| !own.contains(key.as_str()) && *key != "$comment")
            {
                let knows = match component {
                    "external_ids" => "another system still knows it by its `external_ids` key",
                    _ => "another importer still keeps how its file wrote it, in the `import_formatting` entry",
                };
                return Err(format!(
                    "the files hold no record for entity {} (the {key}), and {knows} `{foreign}`: deleting it \
                     would end that system's entity too, and no change can detach this adapter from it, because \
                     none removes a component",
                    existing.describe(&text)
                ));
            }
        }
        out.push(Change::Delete(text));
    }
    Ok(out)
}

/// The patch that makes one held entity what the import says, or none when it
/// already is; or a sentence when the import would stop writing a component
/// the entity holds, one it writes for every record of the kind or one it
/// wrote for the record's earlier shape.
///
/// `own` is this adapter's own keys of the two shared components: its
/// `external_ids` keys, and the keys of its `import_formatting` entries.
fn patch(
    held: &Held<'_>,
    construct: &str,
    name: Option<String>,
    components: Map<String, Value>,
    own: (&FxHashSet<&'static str>, &FxHashSet<&'static str>),
) -> Result<Option<EntityWrite>, String> {
    for component in writes(construct) {
        if held.components.contains_key(*component) && !components.contains_key(*component) {
            return Err(format!(
                "entity {} (the {}) holds a `{component}` component the files no longer give it, and no change \
                 can remove a component",
                held.id,
                key_of(&components).unwrap_or_default()
            ));
        }
    }
    // A component the import writes for one shape of record only, which the
    // record's own component shows this adapter wrote for the shape it had,
    // and which the shape the files now give it does not get: left on, it would
    // say the record is still what it was. One another tool put on a record of
    // a shape that gets none is left alone.
    let held_component = |name: &str| held.components.get(name);
    for component in written_for_shape(construct, held_component) {
        if held.components.contains_key(*component) && !components.contains_key(*component) {
            return Err(format!(
                "entity {} (the {}) holds a `{component}` component this adapter wrote for the record's earlier \
                 shape, {}, and the files now give it {}, for which it writes none: no change can remove a component",
                held.id,
                key_of(&components).unwrap_or_default(),
                shape_of(construct, held_component),
                shape_of(construct, |name| components.get(name))
            ));
        }
    }
    let mut write = EntityWrite::new(held.id);
    let mut differs = false;
    if let Some(name) = name {
        if held.name != Some(name.as_str()) {
            write = write.named(name);
            differs = true;
        }
    }
    for (component, value) in components {
        let stated = held.components.get(&component);
        let shared = match component.as_str() {
            "external_ids" => Some(own.0),
            written::COMPONENT => Some(own.1),
            _ => None,
        };
        let value = if let Some(own) = shared {
            let Some(merged) = merge_shared(stated, value, own) else { continue };
            merged
        } else {
            if stated.is_some_and(|stated| same(stated, &value)) {
                continue;
            }
            value
        };
        write = write.with(component, value);
        differs = true;
    }
    Ok(differs.then_some(write))
}

/// A shared component — `external_ids`, or `import_formatting` — as it should
/// stand: the import's own keys, and every other key the entity holds, another
/// system's or another importer's, with its note, as it was. `None` when the
/// import's keys already stand as they should.
fn merge_shared(stated: Option<&Value>, imported: Value, own_keys: &FxHashSet<&'static str>) -> Option<Value> {
    let none = Map::new();
    let stated = stated.and_then(Value::as_object).unwrap_or(&none);
    let imported = match imported {
        Value::Object(imported) => imported,
        _ => Map::new(),
    };
    let mine = |ids: &Map<String, Value>| ids.iter().filter(|(key, _)| own_keys.contains(key.as_str())).count();
    let unchanged = mine(stated) == mine(&imported)
        && stated
            .iter()
            .filter(|(key, _)| own_keys.contains(key.as_str()))
            .all(|(key, value)| imported.get(key).is_some_and(|other| same(value, other)));
    if unchanged {
        return None;
    }
    let mut merged: Map<String, Value> = stated
        .iter()
        .filter(|(key, _)| !own_keys.contains(key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    merged.extend(imported);
    Some(Value::Object(merged))
}

/// Whether two values are one value as the host spells them, annotations aside:
/// the comparison `to_canonical_string` would make of the two, walked rather
/// than spelled out, because spelling every component of a site of thirty
/// thousand entities as text costs an ingest more than its import does.
fn same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let noted = |map: &Map<String, Value>| map.keys().filter(|key| *key != "$comment").count();
            noted(left) == noted(right)
                && left
                    .iter()
                    .filter(|(key, _)| *key != "$comment")
                    .all(|(key, value)| right.get(key).is_some_and(|other| same(value, other)))
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(one, other)| same(one, other))
        }
        (Value::Number(one), Value::Number(other)) => match (one.as_i64(), other.as_i64()) {
            (Some(one), Some(other)) => one == other,
            _ => to_canonical_string(left) == to_canonical_string(right),
        },
        (left, right) => left == right,
    }
}

/// The `external_ids` a record's components state, if they state one.
fn ids_of(components: &Map<String, Value>) -> Option<&Map<String, Value>> {
    components.get("external_ids").and_then(Value::as_object)
}

/// The lookup key a record's components spell, for a sentence: "cell 1 [7,1]".
fn key_of(components: &Map<String, Value>) -> Option<String> {
    ids_of(components).and_then(key_of_ids)
}

/// A value for a sentence: itself where it is short, and its JSON kind where
/// quoting it would bury the sentence.
fn shown(value: &Value) -> String {
    let text = value.to_string();
    if text.len() <= 60 {
        return text;
    }
    match value {
        Value::Array(_) => "a list".to_string(),
        Value::Object(_) => "an object".to_string(),
        Value::String(_) => "a long string".to_string(),
        _ => text,
    }
}

/// A change that names an entity of a pinned scope is refused, naming the
/// change's entity, where it names it, the entity and its scope. The server's
/// reference gate takes a reference into another scope only with the pin it
/// is read at, and this adapter writes a bare identity, so the host would be
/// handed a change the server refuses. An entity of this site naming one the
/// import found in a pinned scope is refused until pinned references are
/// written.
fn naming_no_pinned_entity(
    write: &EntityWrite,
    named: &str,
    pinned: &FxHashMap<&str, &Held<'_>>,
) -> Result<(), String> {
    if pinned.is_empty() {
        return Ok(());
    }
    for (component, value) in write.components() {
        let mut at = format!("{component}");
        if let Some(held) = named_in(value, &mut at, pinned) {
            return Err(format!(
                "the files would have entity {}{named} name entity {}{} at `{at}`, which belongs to the pinned scope \
                 {}: a reference into a pinned scope carries the pin the server's reference gate reads it at, and \
                 this adapter writes a bare identity, so the server would refuse the change",
                write.id(),
                held.id,
                held.external_ids().and_then(key_of_ids).map(|key| format!(" (the {key})")).unwrap_or_default(),
                held.scope
            ));
        }
    }
    Ok(())
}

/// The first pinned entity a value names anywhere in it, with `at` extended
/// to the place it is named at.
fn named_in<'h>(value: &Value, at: &mut String, pinned: &FxHashMap<&str, &'h Held<'h>>) -> Option<&'h Held<'h>> {
    match value {
        Value::String(text) => pinned.get(text.as_str()).copied(),
        Value::Array(items) => items.iter().enumerate().find_map(|(index, item)| {
            let stem = at.len();
            at.push_str(&format!("[{index}]"));
            let found = named_in(item, at, pinned);
            if found.is_none() {
                at.truncate(stem);
            }
            found
        }),
        Value::Object(fields) => fields.iter().find_map(|(key, item)| {
            let stem = at.len();
            at.push('.');
            at.push_str(key);
            let found = named_in(item, at, pinned);
            if found.is_none() {
                at.truncate(stem);
            }
            found
        }),
        _ => None,
    }
}

fn key_of_ids(ids: &Map<String, Value>) -> Option<String> {
    let (construct, key) = constructs_of(ids).into_iter().next()?;
    Some(format!("{construct} {}", key.values.join(" ")))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::changes;
    use crate::identity::tests::Stated;
    use crate::identity::Existing;
    use serde_json::json;
    use uuid::Uuid;
    use worldadapt_sdk::Change;

    const ZONE: &str = "b0000000-0000-4000-8000-000000000001";

    /// How `pps.json` wrote a station of the current dialect numbered 3.
    fn station_formatting() -> Value {
        json!({ "mapcreator_station": { "index": 0, "dialect_shape": "v3", "pps_id": 3 } })
    }

    fn zone(name: &str, paused: bool, extra: Value) -> Value {
        let mut ids = json!({ "mapcreator_zone_id": "Z1" });
        for (key, value) in extra.as_object().expect("an object") {
            ids[key] = value.clone();
        }
        json!({ "id": ZONE, "name": name, "components": {
            "zone": { "runtime": { "blocked": false, "paused": paused } },
            "import_formatting": { "mapcreator_zone": { "index": 0 } },
            "external_ids": ids
        } })
    }

    fn run(site: &Stated, imported: Vec<Value>, unclaimed: &[Uuid]) -> Result<Vec<Change>, String> {
        let held = site.held();
        let existing = Existing::of_entities(&held).expect("a site");
        changes(imported, &held, &existing, unclaimed)
    }

    /// The same record written the same way asks for nothing, a note on it
    /// included; a changed field is one patch naming that component alone,
    /// which says only what the site states: a paused zone's patch carries its
    /// `runtime` and nothing of how `zone.json` wrote it.
    #[test]
    fn a_record_written_the_same_asks_for_nothing_and_a_changed_one_is_one_patch() {
        let mut noted = zone("zone Z1", false, json!({}));
        noted["components"]["zone"]["$comment"] = json!("checked on site");
        let site = Stated::of(json!([noted]));
        assert!(run(&site, vec![zone("zone Z1", false, json!({}))], &[]).expect("changes").is_empty());
        let changed = run(&site, vec![zone("zone Z1", true, json!({}))], &[]).expect("changes");
        let [Change::Patch(patch)] = changed.as_slice() else { panic!("{changed:?}") };
        assert_eq!(patch.components().keys().collect::<Vec<_>>(), ["zone"]);
        assert_eq!(patch.components()["zone"], json!({ "runtime": { "blocked": false, "paused": true } }));
        assert_eq!(patch.name(), None);
    }

    /// Another system's key in `external_ids` never causes a change, and a
    /// change to this adapter's keys writes it back beside them.
    #[test]
    fn another_systems_external_id_is_kept() {
        let site = Stated::of(json!([zone("zone Z1", false, json!({ "wms": "W-9", "$comment": "from the WMS" }))]));
        assert!(run(&site, vec![zone("zone Z1", false, json!({}))], &[]).expect("changes").is_empty());
    }

    /// Another importer's entry of `import_formatting` never causes a change,
    /// a change to this adapter's own entry writes it back beside it, and an
    /// entity it is on is refused rather than deleted.
    #[test]
    fn another_importers_formatting_is_kept() {
        let mut held = zone("zone Z1", false, json!({}));
        held["components"]["import_formatting"]["other_importer_zone"] = json!({ "index": 91 });
        let site = Stated::of(json!([held]));
        assert!(run(&site, vec![zone("zone Z1", false, json!({}))], &[]).expect("changes").is_empty());
        let mut moved = zone("zone Z1", false, json!({}));
        moved["components"]["import_formatting"]["mapcreator_zone"]["index"] = json!(5);
        let changed = run(&site, vec![moved], &[]).expect("changes");
        let [Change::Patch(patch)] = changed.as_slice() else { panic!("{changed:?}") };
        assert_eq!(
            patch.components()["import_formatting"],
            json!({ "mapcreator_zone": { "index": 5 }, "other_importer_zone": { "index": 91 } })
        );
        let id = Uuid::parse_str(ZONE).expect("a UUID");
        let refusal = run(&site, vec![], &[id]).expect_err("a refusal");
        assert!(refusal.contains(ZONE) && refusal.contains("`other_importer_zone`"), "{refusal}");
    }

    /// An entity of this adapter's that no record claims is deleted, unless
    /// another system still knows it by a key of its own — whatever that key
    /// is spelled like. A note is not a key.
    #[test]
    fn an_unclaimed_entity_is_deleted_unless_another_system_knows_it() {
        let id = Uuid::parse_str(ZONE).expect("a UUID");
        let noted = Stated::of(json!([zone("zone Z1", false, json!({ "$comment": "from the survey" }))]));
        assert_eq!(run(&noted, vec![], &[id]).expect("changes"), [Change::Delete(ZONE.to_string())]);
        for (key, value) in [("wms", "W-9"), ("$erp", "E1")] {
            let known = Stated::of(json!([zone("zone Z1", false, json!({ key: value }))]));
            let refusal = run(&known, vec![], &[id]).expect_err("a refusal");
            assert!(refusal.contains(ZONE) && refusal.contains(&format!("`{key}`")), "{refusal}");
        }
    }

    /// A component the import would stop writing is refused, naming it.
    /// Two values are the same exactly when the host would spell them alike
    /// with their notes stripped: key order, a note at any depth and a whole
    /// number written as a float make no difference; anything else does.
    #[test]
    fn values_compare_as_the_host_spells_them() {
        use super::same;
        use worldcanonical::to_canonical_string;
        let cases = [
            (json!({ "a": 1, "b": [2.0, { "$comment": "x", "c": null }] }), json!({ "b": [2, { "c": null }], "a": 1 })),
            (json!({ "a": 1 }), json!({ "a": 1, "b": 2 })),
            (json!([1, 2]), json!([2, 1])),
            (json!(0.1), json!(0.10000000000000001)),
            (json!(-0.0), json!(0)),
            (json!("1"), json!(1)),
            (json!({ "$comment": "only a note" }), json!({})),
        ];
        for (left, right) in cases {
            let spelled = to_canonical_string(&left) == to_canonical_string(&right);
            assert_eq!(same(&left, &right), spelled, "{left} against {right}");
        }
    }

    /// An entity the import would write without something its kind is written
    /// back from is refused, naming the entity and what it lacks, rather than
    /// left for the projection to refuse.
    #[test]
    fn an_entity_the_import_would_leave_short_is_refused() {
        let mut short = zone("zone Z1", false, json!({}));
        short["components"]["import_formatting"]["mapcreator_zone"].as_object_mut().expect("an object").remove("index");
        let refusal = run(&Stated::of(json!([])), vec![short], &[]).expect_err("a refusal");
        assert!(
            refusal.contains(ZONE) && refusal.contains("no `import_formatting.mapcreator_zone.index`"),
            "{refusal}"
        );
    }

    /// A component the import writes for every record of the kind and would
    /// stop writing is refused, naming it; one it writes for some shapes of
    /// record only, another tool may put there, and it is left alone.
    #[test]
    fn a_component_the_import_would_stop_writing_is_refused() {
        let held = json!([{ "id": ZONE, "components": {
            "pose": { "frame": ZONE, "position": [0, 0] }, "station": { "kind": "pick" },
            "mapcreator.station": {}, "import_formatting": station_formatting(),
            "external_ids": { "mapcreator_pps_id": "3" } } }]);
        let site = Stated::of(held);
        let without_pose = json!({ "id": ZONE, "components": {
            "station": { "kind": "pick" }, "mapcreator.station": {}, "import_formatting": station_formatting(),
            "external_ids": { "mapcreator_pps_id": "3" } } });
        let refusal = run(&site, vec![without_pose], &[]).expect_err("a refusal");
        assert!(refusal.contains("`pose`") && refusal.contains(ZONE), "{refusal}");
        let without_station = json!({ "id": ZONE, "components": {
            "pose": { "frame": ZONE, "position": [0, 0] }, "mapcreator.station": {}, "import_formatting": station_formatting(),
            "external_ids": { "mapcreator_pps_id": "3" } } });
        assert!(run(&site, vec![without_station], &[]).expect("changes").is_empty());
    }

    /// A component the import writes for one shape of record only, which the
    /// record's own component shows this adapter wrote for its earlier shape,
    /// is refused when the files give the record a shape that gets none,
    /// naming the entity, the component and the two shapes.
    #[test]
    fn a_component_written_for_a_records_earlier_shape_is_refused_when_the_shape_changes() {
        let station = |modes: Value, core: bool| {
            let mut components = json!({
                "pose": { "frame": ZONE, "position": [0, 0] },
                "mapcreator.station": { "role": "manual", "allowed_modes": modes },
                "import_formatting": station_formatting(),
                "external_ids": { "mapcreator_pps_id": "3" } });
            if core {
                components["station"] = json!({ "kind": "pick", "role": "manual" });
            }
            json!({ "id": ZONE, "components": components })
        };
        let site = Stated::of(json!([station(json!(["pick"]), true)]));
        let refusal = run(&site, vec![station(json!(["pick", "put"]), false)], &[]).expect_err("a refusal");
        for says in [ZONE, "`station` component", "`allowed_modes` [\"pick\"]", "`allowed_modes` [\"pick\",\"put\"]"] {
            assert!(refusal.contains(says), "{says:?} in {refusal}");
        }
    }
}
