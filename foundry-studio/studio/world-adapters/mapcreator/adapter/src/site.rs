//! Within the Map Creator adapter, `site` is **the site as this adapter reads
//! it**: every entity the host handed over, borrowed, with its scope; which of
//! them are this adapter's — those carrying one of its lookup keys — and
//! whether each of those has what its kind requires.
//!
//! The two directions read the site the same way, so the rules that say which
//! entities are this adapter's live here once. An entity that carries none of
//! its lookup keys is never changed, deleted or written out, whatever it carries
//! besides, because nothing says it is a Map Creator record. One that carries a
//! key and lacks a component or a field its kind is written back from, or
//! carries the keys of two kinds, is refused by name in both directions: the
//! projection would drop it from the files, or write it short or reordered,
//! without a word, and the ingest would write over it.
//!
//! The view borrows rather than copies. A site of tens of thousands of
//! entities is the largest thing a call holds, and the two directions only read
//! it.

use crate::identity::{constructs_of, scalar};
use crate::import::station_kind;
use crate::records;
use crate::vocabulary::{declared_kinds, unplaced, unplaced_formatting};
use crate::written;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use worldadapt_sdk::Site;

/// One entity of the site, borrowed from what the host handed over.
#[derive(Debug, Clone, Copy)]
pub struct Held<'s> {
    pub id: &'s str,
    pub name: Option<&'s str>,
    /// The scope that owns it.
    pub scope: &'s str,
    /// Whether that scope is the site's own. An entity of any other scope belongs
    /// to a scope the site pins: it is read, and never changed.
    pub own: bool,
    pub components: &'s BTreeMap<String, Value>,
}

impl<'s> Held<'s> {
    /// Every entity of the site, in the order the host handed them over.
    pub fn all_of(site: &'s Site) -> Vec<Held<'s>> {
        site.entities()
            .iter()
            .map(|entity| Held {
                id: entity.id(),
                name: entity.name(),
                scope: entity.scope(),
                own: site.is_own(entity),
                components: entity.components(),
            })
            .collect()
    }

    pub fn external_ids(&self) -> Option<&'s Map<String, Value>> {
        self.components.get("external_ids").and_then(Value::as_object)
    }
}

/// One thing an entity has to carry to be written back: a component, a field
/// of one, one of two fields of one, or a field of its record's formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Component(&'static str),
    /// A field of a component; `outer.inner` reaches into an object the
    /// component nests.
    Field(&'static str, &'static str),
    /// One of two fields of a component: the reference the import resolved,
    /// or the value the file wrote where it named nothing.
    Either(&'static str, &'static str, &'static str),
    /// A field of the record's entry of `import_formatting`, under the key its
    /// kind's formatting rides under: how the file wrote the record, which
    /// the projection writes it back by.
    Written(&'static str, &'static str),
}

impl Need {
    /// Whether the components an entity carries, answered by name, meet it.
    pub fn met<'v>(self, component: impl Fn(&str) -> Option<&'v Value> + Copy) -> bool {
        match self {
            Need::Component(name) => component(name).is_some(),
            Need::Field(name, path) => field(component, name, path).is_some(),
            Need::Either(name, one, other) => {
                field(component, name, one).is_some() || field(component, name, other).is_some()
            }
            Need::Written(key, name) => {
                component(written::COMPONENT).and_then(|all| all.get(key)).and_then(|entry| entry.get(name)).is_some()
            }
        }
    }

    /// The sentence refusing entity `id`, which carries the key of a
    /// `construct` and does not meet this.
    pub fn refusal(self, id: &str, construct: &str) -> String {
        let carrying = format!("entity {id} carries the Map Creator key of a {construct}");
        match self {
            Need::Component(name) => {
                format!("{carrying} and has no `{name}` component, which a {construct} is written back from")
            }
            Need::Field(name, path) => format!("{carrying} and no `{name}.{path}`, which a {construct} is written back from"),
            Need::Either(name, one, other) => format!(
                "{carrying} and neither `{name}.{one}` nor `{name}.{other}`, one of which a {construct} is written back \
                 from"
            ),
            Need::Written(key, name) => format!(
                "{carrying} and no `{}.{key}.{name}`, which says how the file wrote a {construct} and which it is \
                 written back by",
                written::COMPONENT
            ),
        }
    }
}

/// One field of one component, following `.` into the objects it nests.
fn field<'v>(component: impl Fn(&str) -> Option<&'v Value>, name: &str, path: &str) -> Option<&'v Value> {
    path.split('.').try_fold(component(name)?, |value, step| value.get(step))
}

/// What an entity of one construct must carry for the projection to write its
/// record back as it went in: the components the lens for its kind matches on,
/// and every component and field the serializer reads the record from whose
/// absence would drop the record or write it short, out of its order, or as a
/// record the ingest refuses. A field whose absence is the record not stating
/// it is not here: the projection leaves it out, as the record did. For the two
/// kinds no lens reads, the one core component that says what the entity is.
///
/// One table, which both directions check an entity of the adapter's against,
/// so an entity the ingest accepts is one the projection can write. Some rows
/// also depend on the entity (see [`needs_of_this_one`]).
pub fn needs<'v>(construct: &str, component: impl Fn(&str) -> Option<&'v Value> + Copy) -> Vec<Need> {
    let mut needs = needs_of_every_one(construct);
    needs.extend(needs_of_this_one(construct, component));
    needs
}

/// The first thing an entity of `construct` needs and does not carry.
pub fn lacking<'v>(construct: &str, component: impl Fn(&str) -> Option<&'v Value> + Copy) -> Option<Need> {
    needs(construct, component).into_iter().find(|need| !need.met(component))
}

/// What every entity of a construct needs, whatever it states.
fn needs_of_every_one(construct: &str) -> Vec<Need> {
    use Need::{Component as C, Either as E, Field as F, Written as W};
    let mut needs = match construct {
        "frame" | "floor" => vec![C("frame"), C("mapcreator.frame"), F("mapcreator.frame", "role")],
        // Chargers, stations of the older dialect, presentation points,
        // obstacle-detection exclusions and the floor's vendor sample point name
        // a cell by its barcode, and an ingest refuses a cell stating none.
        "cell" => vec![
            C("cell"),
            C("external_ids"),
            C("pose"),
            F("pose", "position"),
            F("cell", "blocked"),
            F("external_ids", "barcode"),
        ],
        // The sector's key comes back in the JSON kind its formatting records
        // the cells spelling it in.
        "sector" => vec![C("external_ids"), C("sector"), F("sector", "members"), W(written::SECTOR, "key")],
        "aisle" => vec![C("aisle"), C("sector"), F("sector", "members")],
        "stitch" => vec![C("stitch")],
        "robot_class" => vec![C("robot_class")],
        "block" => vec![C("block"), F("block", "entity_type")],
        // A station's `pps_id` comes back from its formatting's copy, which
        // keeps the JSON kind its dialect spells it in.
        "station" => {
            vec![C("mapcreator.station"), W("mapcreator_station", "dialect_shape"), W("mapcreator_station", "pps_id")]
        }
        // A point is written inside its station's record, which folds it by
        // this reference: without it the point would be dropped from the file.
        "presentation_point" => vec![C("external_ids"), C("presentation_point"), F("presentation_point", "station")],
        "station_point" => vec![C("station_point"), F("station_point", "station")],
        "conveyor_mapping" => vec![
            C("conveyor_mapping"),
            F("conveyor_mapping", "conveyor_coordinates"),
            E("conveyor_mapping", "conveyor_coordinates.conveyor", "conveyor_coordinates.conveyor_unresolved"),
        ],
        "pps_bin" => vec![C("pps_bin"), E("pps_bin", "station", "station_unresolved")],
        "pps_seat" => vec![C("pps_seat"), E("pps_seat", "station", "station_unresolved")],
        // A light's group is its place in the file, which nothing else states.
        "pptl" => vec![C("pptl"), W("mapcreator_pptl", "group")],
        "export" => vec![C("mapcreator.export"), F("mapcreator.export", "files")],
        "sector_preference" => vec![C("sector_preference"), F("sector_preference", "prefers")],
        other => records::kind(other).map(|kind| vec![C(kind.component)]).unwrap_or_default(),
    };
    // Every record kind's records are written in the order of their ordering
    // keys, so one without a key would move. A sector's and a preference's are
    // keys of one JSON object instead, which comes back in its own key order.
    if let Some(kind) = records::kind(construct) {
        needs.push(W(kind.lens, "index"));
    }
    // A record kind's lens matches only an entity carrying the kind's own key,
    // since its component is shared and another tool may put it on any entity.
    if !needs.contains(&C("external_ids")) && !matches!(construct, "frame" | "floor" | "station" | "export") {
        needs.push(C("external_ids"));
    }
    needs
}

/// What one entity needs besides, by what it states: a floor frame its floor
/// id, and the site frame the step every position is written back in; a block
/// the list its kind keeps its cells in — a queue's path, or the cells on its
/// own component; and an exclusion whose tuple split into a barcode and a
/// heading, the two the tuple is written back from.
fn needs_of_this_one<'v>(construct: &str, component: impl Fn(&str) -> Option<&'v Value>) -> Vec<Need> {
    use Need::{Component as C, Either as E, Field as F};
    let stated = |name: &str, field: &str| component(name).and_then(|c| c.get(field));
    match construct {
        "frame" | "floor" => match stated("mapcreator.frame", "role").and_then(Value::as_str) {
            Some("floor") => vec![F("mapcreator.frame", "floor_id")],
            Some("site") => vec![F("frame", "unit_mm")],
            _ => Vec::new(),
        },
        "block" => match stated("block", "entity_type").and_then(Value::as_str) {
            Some(kind) if records::QUEUE_KINDS.contains(&kind) => vec![C("queue"), F("queue", "path")],
            _ => vec![F("block", "cells")],
        },
        "ods_exclusion" if stated("ods_exclusion", "tuple_unparsed").is_none() => {
            vec![F("ods_exclusion", "heading"), E("ods_exclusion", "cell", "cell_barcode_unresolved")]
        }
        _ => Vec::new(),
    }
}

/// The fields of one record kind's formatting entry every entity of the kind
/// needs, whatever it states, which the `import_formatting` schema states as
/// that entry's `required`.
pub fn formatting_always_needed(key: &str) -> Vec<&'static str> {
    let mut fields: Vec<&'static str> = crate::identity::SHAPES
        .iter()
        .flat_map(|shape| needs_of_every_one(shape.construct))
        .filter_map(|need| match need {
            Need::Written(owner, field) if owner == key => Some(field),
            _ => None,
        })
        .collect();
    fields.sort_unstable();
    fields.dedup();
    fields
}

/// The fields of one component of the vocabulary every entity carrying it needs,
/// whatever it states, which its JSON Schema states as `required`. A field
/// inside a nested object, and one of two fields, are checked by the table
/// and not stated there.
pub fn fields_always_needed(component: &str) -> Vec<&'static str> {
    let mut fields: Vec<&'static str> = crate::identity::SHAPES
        .iter()
        .flat_map(|shape| needs_of_every_one(shape.construct))
        .filter_map(|need| match need {
            Need::Field(owner, field) if owner == component && !field.contains('.') => Some(field),
            _ => None,
        })
        .collect();
    fields.sort_unstable();
    fields.dedup();
    fields
}

/// The components the import writes on every record of a construct whose
/// source fields are present. An entity of the construct that holds one of
/// these and that the import would no longer write it on is an entity the files
/// stopped giving it to, and no change can bring it back in line, because no
/// change removes a component.
///
/// A component the import writes for some shapes of record only — a station's
/// `station` where its modes decide one kind, a block's `queue` or `waiting` by
/// its kind, a robot class's `lineage` where its name carries a maker — is not
/// here: [`written_for_shape`] reads it off the shape the record had. What this
/// cannot tell apart is a component of this list that another tool added where
/// the files give none, such as a pose on a station whose location names no
/// cell: that is still read as one the files stopped giving.
pub fn writes(construct: &str) -> &'static [&'static str] {
    match construct {
        "frame" | "floor" => &["external_ids", "frame", "mapcreator.frame"],
        "cell" => &["cell", "external_ids", "import_formatting", "pose"],
        "robot_class" => &["external_ids", "robot_class"],
        "sector" => &["external_ids", "import_formatting", "sector"],
        "aisle" => &["aisle", "external_ids", "import_formatting", "sector"],
        "stitch" => &["external_ids", "stitch"],
        "block" => &["block", "external_ids", "import_formatting"],
        "station" => &["external_ids", "import_formatting", "mapcreator.station", "pose"],
        "presentation_point" => &["external_ids", "import_formatting", "pose", "presentation_point"],
        "station_point" => &["external_ids", "import_formatting", "pose", "presentation_point", "station_point"],
        "zone" => &["external_ids", "import_formatting", "zone"],
        "location_profile" => &["external_ids", "import_formatting", "location_profile"],
        "tote_storage" => &["external_ids", "import_formatting", "tote_storage"],
        "rack_type" => &["external_ids", "import_formatting", "rack_type"],
        "charger" => &["charger", "external_ids", "import_formatting"],
        "dock_point" => &["dock_point", "external_ids", "import_formatting"],
        "ods_exclusion" => &["external_ids", "import_formatting", "ods_exclusion"],
        "conveyor" => &["conveyor", "external_ids", "import_formatting"],
        "conveyor_mapping" => &["conveyor_mapping", "external_ids", "import_formatting"],
        "sector_preference" => &["external_ids", "import_formatting", "sector_preference"],
        "pps_bin" => &["external_ids", "import_formatting", "pps_bin"],
        "pps_bin_type" => &["external_ids", "import_formatting", "pps_bin_type"],
        "pps_seat" => &["external_ids", "import_formatting", "pps_seat"],
        "pptl" => &["external_ids", "import_formatting", "pptl"],
        "export" => &["external_ids", "mapcreator.export"],
        _ => &[],
    }
}

/// The components the import writes for the shape a record's own component
/// states, among those it writes for some shapes only: a block's `queue` or
/// `waiting` by its `entity_type`, and a station's `station` where its `type`
/// and `allowed_modes` decide one kind. A robot class's `lineage` follows its
/// name, which is its key, so no record of one changes shape.
///
/// Read off an entity the site holds, this says which of them this adapter
/// wrote for the record's earlier shape; one the entity holds that its shape
/// does not get, another tool put there.
pub fn written_for_shape<'v>(
    construct: &str,
    component: impl Fn(&str) -> Option<&'v Value>,
) -> &'static [&'static str] {
    match construct {
        "block" => match component("block").and_then(|block| block.get("entity_type")).and_then(Value::as_str) {
            Some(kind) if records::QUEUE_KINDS.contains(&kind) => &["queue"],
            Some(kind) if records::WAITING_KINDS.contains(&kind) => &["waiting"],
            _ => &[],
        },
        "station" => {
            let station = component("mapcreator.station");
            let kind = station.and_then(|station| station.get("role")).and_then(Value::as_str).unwrap_or_default();
            let modes: Vec<String> = station
                .and_then(|station| station.get("allowed_modes"))
                .and_then(Value::as_array)
                .map(|modes| modes.iter().map(scalar).collect())
                .unwrap_or_default();
            if station_kind(kind, &modes).is_some() {
                &["station"]
            } else {
                &[]
            }
        }
        _ => &[],
    }
}

/// The shape a record's own component states, for a sentence: a block's
/// `entity_type`, a station's `type` and `allowed_modes`.
pub fn shape_of<'v>(construct: &str, component: impl Fn(&str) -> Option<&'v Value>) -> String {
    let stated = |name: &str, field: &str| {
        component(name).and_then(|own| own.get(field)).map_or("none".to_string(), Value::to_string)
    };
    match construct {
        "block" => format!("`entity_type` {}", stated("block", "entity_type")),
        "station" => format!(
            "`type` {} with `allowed_modes` {}",
            stated("mapcreator.station", "role"),
            stated("mapcreator.station", "allowed_modes")
        ),
        _ => String::new(),
    }
}

/// Every entity of the site that is this adapter's, each with the one
/// construct its keys name — or a sentence naming the first that carries the
/// keys of two kinds, or lacks what [`needs`] says its kind is written back
/// from.
pub fn owned<'s>(held: &[Held<'s>]) -> Result<Vec<(Held<'s>, &'static str)>, String> {
    let declared = declared_kinds();
    let mut out = Vec::new();
    for entity in held {
        let Some(ids) = entity.external_ids() else { continue };
        let constructs = constructs_of(ids);
        let construct = match constructs.as_slice() {
            [] => continue,
            [(only, _)] => *only,
            [(one, _), (other, _), ..] => {
                return Err(format!(
                    "entity {} carries the Map Creator keys of a {one} and of a {other}, and one entity is one \
                     record of one kind",
                    entity.id
                ));
            }
        };
        if let Some(need) = lacking(construct, |name| entity.components.get(name)) {
            return Err(need.refusal(entity.id, construct));
        }
        // A field of a component this adapter writes that the export has no
        // place for would be dropped from the files and from the component
        // the next ingest rewrites, and so would a formatting entry under
        // another of this adapter's keys than the record's own; another
        // system's components, and another importer's formatting entries, are
        // its own.
        for (component, payload) in entity.components {
            let Some(fields) = declared.get(component.as_str()) else { continue };
            let Some(payload) = payload.as_object() else { continue };
            let unplaced = match component.as_str() {
                written::COMPONENT => unplaced_formatting(fields, payload, written::key_of(construct)),
                _ => unplaced(fields, payload, ""),
            };
            if let Some(field) = unplaced {
                return Err(format!(
                    "entity {} carries `{component}.{field}`, a field this adapter has no place for in a Map \
                     Creator file: the files would leave it out, and an ingest would write the component without it",
                    entity.id
                ));
            }
        }
        out.push((*entity, construct));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{needs, owned, Held, Need};
    use crate::records::KINDS;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use worldlens::Library;

    fn components(value: Value) -> BTreeMap<String, Value> {
        value.as_object().expect("an object").iter().map(|(name, value)| (name.clone(), value.clone())).collect()
    }

    fn held<'s>(id: &'s str, components: &'s BTreeMap<String, Value>) -> Held<'s> {
        Held { id, name: None, scope: "site", own: true, components }
    }

    /// Every component a kind's lens matches on is one the table needs, so the
    /// projection can never drop an entity this check passed.
    #[test]
    fn every_component_a_lens_matches_on_is_needed() {
        let library = Library::parse(crate::lenses()).expect("the lens library parses");
        let lens_of = |construct: &str| -> Option<&str> {
            match construct {
                "frame" | "floor" => Some("mapcreator_frame"),
                "sector" => Some("mapcreator_sector"),
                "sector_preference" => Some("mapcreator_sector_preference"),
                "export" => Some("mapcreator_export"),
                other => KINDS.iter().find(|kind| kind.construct == other).map(|kind| kind.lens),
            }
        };
        for shape in crate::identity::SHAPES {
            let needed: Vec<String> = needs(shape.construct, |_| None)
                .into_iter()
                .filter_map(|need| match need {
                    Need::Component(name) => Some(name.to_string()),
                    _ => None,
                })
                .collect();
            assert!(!needed.is_empty(), "{} needs nothing", shape.construct);
            let Some(lens) = lens_of(shape.construct) else { continue };
            let carried = library.lens(lens).expect("the lens exists").component_names();
            // The station lens folds its parts on, so its list names theirs
            // too, and only the station's own are the station's.
            let matched: Vec<&String> =
                carried.iter().filter(|name| shape.construct != "station" || *name == "mapcreator.station").collect();
            for name in matched {
                assert!(needed.contains(name), "{}'s lens matches `{name}`, which is not needed", shape.construct);
            }
        }
    }

    /// An entity without a lookup key is not this adapter's, whatever it
    /// carries — a fleet catalog's robot class keyed by `butler_bot_version`
    /// included; one with a key is, with its construct.
    #[test]
    fn only_an_entity_with_a_lookup_key_is_this_adapters() {
        let zone = components(json!({ "zone": {}, "import_formatting": { "mapcreator_zone": { "index": 0 } },
                                      "external_ids": { "mapcreator_zone_id": "Z1" } }));
        let lookalike = components(json!({ "zone": {}, "external_ids": { "other_system": "Z1" } }));
        let catalog = components(json!({ "robot_class": {}, "external_ids": { "butler_bot_version": "HTM_X" } }));
        let found = owned(&[held("a", &zone), held("b", &lookalike), held("c", &catalog)]).expect("nothing refused");
        assert_eq!(
            found.iter().map(|(entity, construct)| (entity.id, *construct)).collect::<Vec<_>>(),
            [("a", "zone")]
        );
    }

    /// A keyed entity that lacks what its kind is read back by, carries two
    /// kinds' keys, or is a station or frame without the field its writing
    /// needs, is refused by name.
    #[test]
    fn a_keyed_entity_that_cannot_be_read_back_is_refused_by_name() {
        for (value, says) in [
            (
                json!({ "cell": {}, "external_ids": { "mapcreator_floor_id": "1", "mapcreator_grid_coordinate": "[1,1]" } }),
                "has no `pose`",
            ),
            (
                json!({ "zone": {}, "block": {},
                     "external_ids": { "mapcreator_zone_id": "Z1", "mapcreator_block_id": "7" } }),
                "of a zone and of a block",
            ),
            (
                json!({ "mapcreator.station": {}, "external_ids": { "mapcreator_pps_id": "3" } }),
                "no `import_formatting.mapcreator_station.dialect_shape`",
            ),
            (
                json!({ "frame": {}, "mapcreator.frame": {}, "external_ids": { "mapcreator_frame": "site" } }),
                "no `mapcreator.frame.role`",
            ),
            (
                json!({ "frame": {}, "mapcreator.frame": { "role": "floor" }, "external_ids": { "mapcreator_floor": "1" } }),
                "no `mapcreator.frame.floor_id`",
            ),
            (
                json!({ "block": { "entity_type": "queue" }, "import_formatting": { "mapcreator_block": { "index": 0 } },
                        "external_ids": { "mapcreator_block_id": "7" } }),
                "has no `queue` component",
            ),
            (
                json!({ "block": { "entity_type": "waiting" }, "waiting": {},
                        "import_formatting": { "mapcreator_block": { "index": 0 } },
                        "external_ids": { "mapcreator_block_id": "9" } }),
                "no `block.cells`",
            ),
            (
                json!({ "pose": {},
                        "external_ids": { "mapcreator_of_pps": "3", "mapcreator_point_id": "P1" } }),
                "has no `presentation_point` component",
            ),
            (
                json!({ "frame": {}, "mapcreator.frame": { "role": "site" }, "external_ids": { "mapcreator_frame": "site" } }),
                "no `frame.unit_mm`",
            ),
            (
                json!({ "pps_bin": {}, "import_formatting": { "mapcreator_pps_bin": { "index": 0 } },
                        "external_ids": { "mapcreator_of_pps": "1", "mapcreator_bin_id": "B1" } }),
                "neither `pps_bin.station` nor `pps_bin.station_unresolved`, one of which",
            ),
            (
                json!({ "conveyor_mapping": { "conveyor_coordinates": {} },
                        "import_formatting": { "mapcreator_conveyor_mapping": { "index": 0 } },
                        "external_ids": { "mapcreator_of_conveyor": "3", "mapcreator_step_id": "S1" } }),
                "neither `conveyor_mapping.conveyor_coordinates.conveyor` nor",
            ),
            (
                json!({ "ods_exclusion": { "cell": "c1" },
                        "import_formatting": { "mapcreator_ods_exclusion": { "index": 0 } },
                        "external_ids": { "mapcreator_ods_tuple": "611.117--1" } }),
                "no `ods_exclusion.heading`",
            ),
        ] {
            let map = components(value);
            let refusal = owned(&[held("e1", &map)]).expect_err("a refusal");
            assert!(refusal.contains("e1") && refusal.contains(says), "{refusal}");
        }
    }
}
