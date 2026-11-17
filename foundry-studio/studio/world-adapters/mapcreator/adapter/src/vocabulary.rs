//! Within the Map Creator adapter, `vocabulary` is **what the adapter asks a
//! registry to register**: one definition per component the mapping writes that
//! the core seed does not define, each with the prose line that fixes its
//! meaning, its is/has reading, its rung on the ratification ladder, and its
//! JSON Schema.
//!
//! It exists so the graph server can hold these names without knowing them. The
//! server reads no component (DESIGN.md §9, ruled 2026-09-09) and validates every
//! write against a registered schema at a named version, so a document carrying
//! `zone` is refused until somebody registers `zone` — and this is the
//! somebody. The shape is what `scripts/derive-component-schemas.py`
//! produces for the core seed, so a seed script can walk this list exactly as it
//! walks the core vocabulary table.
//!
//! Most of these are **shared vocabulary**: an undotted name, ratified and
//! owned by Foundry, because Map Creator is the incumbent and a second
//! team should reuse its `zone` rather than mint a rival one; `import_formatting`,
//! how a file wrote each record, is shared too and enters at `candidate`. Three
//! stand under Map Creator's own name at `experimental`: the export's table of
//! contents, and the frame and station facts whose merge into the core's
//! `frame` and `station` is not obvious (`DECISIONS.md`). What Map Creator
//! states of a cell and a presentation point is optional fields of the
//! protocol's own components ([`EXTENDED`]), which this vocabulary does not
//! define.
//!
//! The **schemas are derived from `records`**, not written out again. A schema that
//! restated the field map by hand would drift from it the first time a field moved,
//! and a schema that disagrees with the converter is worse than no schema:
//! `every_component_schema_matches_its_field_map` is the assertion that keeps them
//! one thing.
//!
//! So are the **reference annotations** a schema version is published with. The
//! server indexes a reference only at a path a component's annotations name, and
//! refuses a delete that would leave a reference dangling only there; every field
//! the projection reads as a reference is annotated, with the component its
//! target carries, so a tool deleting a cell a charger stands on is refused at
//! the server before the projection ever meets the dangling reference.

use crate::ordering;
use crate::records::{Field, Shape, Slot};
use crate::{identity, records, site, written};
use rustc_hash::FxHashMap;
use serde_json::{json, Map, Value};

/// One component definition, in the shape the registry's `POST /registry` takes
/// plus the schema its first version publishes and that version's reference
/// annotations.
#[derive(Debug, Clone)]
pub struct ComponentDefinition {
    /// The name: undotted for shared vocabulary, and under Map Creator's own
    /// `mapcreator.` for the three whose merge is not obvious.
    pub component: &'static str,
    /// Who answers for it: Foundry for both shared and Map Creator names.
    pub owner: &'static str,
    /// The rung: `ratified` for a shared name but the formatting, at
    /// `candidate`, and `experimental` for a dotted one.
    pub status: &'static str,
    /// `is` or `has`: whether carrying the component says the entity IS that
    /// thing, or that it HAS that relation. SPEC §2 requires a definition to fix
    /// this and it is not derivable from the schema.
    pub asserts: &'static str,
    /// The prose line SPEC §2 requires: what the component means, in one sentence
    /// a person can disagree with.
    pub meaning: &'static str,
    /// JSON Schema 2020-12, derived from the field map.
    pub schema: Value,
    /// Every field that holds a reference, in path order.
    pub refs: Vec<Reference>,
}

/// One field of a component that holds references, as a schema version's
/// `refs` states it to the server: a JSON pointer into the payload, ending
/// `/*` for each element of a list, and the component the target carries.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    pub path: String,
    pub expects: &'static str,
}

/// The meaning and the is/has reading of each component, which no field map can
/// derive. One row per record kind, plus the four the mapping writes that no
/// record kind stands for.
///
/// These sentences are the deliverable a registry stores. Written to be
/// disagreed with: each says what the thing IS relative to the site, not what the
/// converter does with it.
const MEANINGS: &[(&str, &str, &str)] = &[
    (
        "location_profile",
        "is",
        "the entity is one named bundle of Map Creator bot permissions: per bot version and per heading, \
         which lift heights are legal loaded and unloaded and how much ground clearance there is. Every \
         cell that names it carries cell.traversable_by read off this, which is the flat and lossy form",
    ),
    (
        "zone",
        "is",
        "the entity is one Map Creator zone: the e-stop and light-curtain domain every cell belongs to \
         exactly one of, which the orchestrator pauses and blocks whole on a physical safety event",
    ),
    (
        "aisle",
        "has",
        "the sector IS a Map Creator aisle: a run of io-point cells a vertical transport works, with the \
         operator-facing name and the vendor classification the export states and no extent, no rack and \
         no direction",
    ),
    (
        "block",
        "has",
        "the entity is one Map Creator block — a named cell group the orchestrator runs as a queue or a \
         waiting area — carrying the kind, the sequenced-queue extras, and the cells where the core \
         component has no way to name them",
    ),
    (
        "mapcreator.station",
        "has",
        "the station has the Map Creator facts about it the core cannot carry: the product type and \
         the allowed modes, the queues that serve it, the internal transport graph over its own parts, \
         and the docking bays it offers",
    ),
    (
        "station_point",
        "has",
        "the presentation point has what the older Map Creator dialect states about it instead: the \
         relay-or-RTP kind it serves, and its position number at the station",
    ),
    (
        "charger",
        "is",
        "the entity is one Map Creator charger: the cell it stands at, the cell a robot approaches from, \
         the re-initialization cell it sends a robot to, the heading at each, and its operating mode",
    ),
    (
        "dock_point",
        "is",
        "the entity is one Map Creator docking point: a fixed floor position where a carrier is parked at \
         a station, with its heading, its operator-facing name and the bay it belongs to",
    ),
    (
        "ods_exclusion",
        "is",
        "the entity is one Map Creator obstacle-detection exclusion: a cell and a heading at which a \
         robot's obstacle sensor is suppressed, because a fixture there would otherwise read as an \
         obstacle",
    ),
    (
        "conveyor",
        "is",
        "the entity is one Map Creator conveyor: an ordered chain of steps with entry and exit ports, \
         each port naming the cell a robot stands on, the handoff height and the heading",
    ),
    (
        "conveyor_mapping",
        "is",
        "the entity is one row of Map Creator's three-frame join table, relating a cell to a conveyor \
         step and, where it has one, the vendor's own coordinate for the same place",
    ),
    (
        "tote_storage",
        "is",
        "the entity is one Map Creator storage position: the cell a robot stands on to reach it, the \
         heading it stands at, the direction the position lies in, its depth index and its height above \
         the floor — a position with no rack anywhere in the export to derive it from",
    ),
    (
        "rack_type",
        "is",
        "the entity is one Map Creator rack and carrier geometry template: the outer and inter-leg \
         extents, the mass and centre of mass, and the shelf tree down to each slot's label. Nothing in \
         the export places one anywhere",
    ),
    (
        "sector_preference",
        "is",
        "the entity is one Map Creator storage preference: a carrier type, and the sectors it prefers in \
         order",
    ),
    (
        "pps_bin",
        "is",
        "the entity is one bin of a Map Creator station's put wall: its place in the wall, its \
         station-local centre, which way it opens, what may go in it, and the two operator seats that \
         reach it",
    ),
    (
        "pps_bin_type",
        "is",
        "the entity is one Map Creator bin geometry type: the extents every bin of that type has, stated \
         in centimetres where the rest of the export is millimetres",
    ),
    (
        "pps_seat",
        "is",
        "the entity is one operator seat at a Map Creator station: which side of the put wall it works \
         from, and the name the bins reference it by",
    ),
    (
        "pptl",
        "is",
        "the entity is one put-to-light module at a Map Creator station, bound one to one to a bin, with \
         its own label",
    ),
    (
        "mapcreator.frame",
        "has",
        "the frame has the role Map Creator gave it: the site frame, the floor, the nominal legacy grid \
         declared for projection only, or the robot vendor's — plus, on the floor, the floor id and the \
         barcode of the one cell the export offers as a vendor-frame sample point",
    ),
    (
        "import_formatting",
        "has",
        "the entity has how the file it was imported from wrote it, apart from the components that state \
         what that file says about the site: for each record the entity carries facts from, keyed by the \
         lens its kind is written back through, the record's place among its file's records, the order its \
         objects state their keys in where the file's usual order is not it, the whole numbers it wrote with \
         a decimal point, and what else its importer needs to write the record back byte for byte",
    ),
    (
        "mapcreator.export",
        "is",
        "the entity is one Map Creator export's own table of contents: which of the nineteen files the \
         site shipped, which of them were empty and in which form, the per-file headers that are \
         envelopes rather than model data, and how each file's text was written beyond its values",
    ),
];

impl ComponentDefinition {
    /// The definition as one JSON object, in the shape a registry's `POST
    /// /registry` takes, with the schema its first version publishes and that
    /// version's reference annotations beside it: what each file under
    /// `vocabulary/` holds.
    pub fn to_json(&self) -> Value {
        let refs: Vec<Value> =
            self.refs.iter().map(|one| json!({ "path": one.path, "expects": one.expects })).collect();
        json!({
            "component": self.component,
            "owner": self.owner,
            "status": self.status,
            "asserts": self.asserts,
            "meaning": self.meaning,
            "schema": self.schema,
            "refs": refs,
        })
    }
}

/// Every component the mapping writes that the core registry does not define,
/// each with its meaning and its schema.
pub fn vocabulary() -> Vec<ComponentDefinition> {
    MEANINGS
        .iter()
        .map(|(component, asserts, meaning)| {
            // An undotted name is shared vocabulary, which the world team answers
            // for and which starts out ratified, but for the formatting, which
            // enters at `candidate` as a shape only one importer has written so
            // far; a dotted one, whose merge into a core component is not
            // obvious, stands at `experimental`.
            let (owner, status) = rung_of(component);
            ComponentDefinition {
                component,
                owner,
                status,
                asserts,
                meaning,
                schema: schema_for(component),
                refs: references_of(component),
            }
        })
        .collect()
}

/// The JSON kinds each field takes of each component this adapter writes its
/// records' fields into — its vocabulary's, and the protocol's two that carry
/// Map Creator's fields as optional ones ([`EXTENDED`]) — read off the schema
/// each is published with once per call, so that checking a payload costs a
/// lookup per field. Of the protocol's two, the fields it states on its own
/// account are known, and take any kind: the core's mapping, not a field map,
/// decides what each holds.
pub fn declared_kinds() -> FxHashMap<&'static str, FxHashMap<String, Declared>> {
    MEANINGS
        .iter()
        .map(|(component, _, _)| *component)
        .chain(EXTENDED)
        .map(|component| {
            let mut fields = declared(Some(&properties_of(component)));
            for field in protocol_fields(component) {
                fields.insert(field.to_string(), Declared { kinds: Vec::new(), nested: FxHashMap::default() });
            }
            (component, fields)
        })
        .collect()
}

/// The fields the protocol states of one of [`EXTENDED`] on its own account,
/// beside the ones this adapter's records add there; none for any other
/// component. `every_field_written_into_a_core_component_is_one_the_protocol_states_and_the_seed_annotates`
/// holds this list to the protocol's schema.
fn protocol_fields(component: &str) -> &'static [&'static str] {
    match component {
        "cell" => &["blocked", "legacy_grid_coordinate", "size", "traversable_by"],
        "presentation_point" => &["for"],
        _ => &[],
    }
}

/// One field's declared JSON kinds, as the schema types it names, none for a
/// field whose schema states no kind (a legacy value, a reference's kept
/// spelling), which takes any; and the fields of the object it nests.
pub struct Declared {
    kinds: Vec<&'static str>,
    nested: FxHashMap<String, Declared>,
}

fn declared(properties: Option<&Map<String, Value>>) -> FxHashMap<String, Declared> {
    let kind_of = |name: &str| TYPES.iter().copied().find(|kind| *kind == name);
    properties
        .into_iter()
        .flatten()
        .map(|(field, property)| {
            let kinds = match property.get("type") {
                Some(Value::String(one)) => kind_of(one).into_iter().collect(),
                Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).filter_map(kind_of).collect(),
                _ => Vec::new(),
            };
            let nested = declared(property.get("properties").and_then(Value::as_object));
            (field.clone(), Declared { kinds, nested })
        })
        .collect()
}

/// The JSON Schema types a field may declare.
const TYPES: [&str; 7] = ["string", "number", "integer", "boolean", "array", "object", "null"];

/// The first field of a payload whose JSON kind its declared kinds do not
/// take, nested objects and `runtime` included: its path, its value and the
/// kinds declared. Nothing is allocated for a field that passes.
pub fn mistyped<'v>(
    declared: &FxHashMap<String, Declared>,
    payload: &'v Map<String, Value>,
    path: &str,
) -> Option<(String, &'v Value, Vec<&'static str>)> {
    let at = |field: &str| if path.is_empty() { field.to_string() } else { format!("{path}.{field}") };
    for (field, value) in payload {
        let Some(field_kinds) = declared.get(field) else { continue };
        if !field_kinds.kinds.is_empty() && !field_kinds.kinds.iter().any(|kind| is_kind(value, kind)) {
            return Some((at(field), value, field_kinds.kinds.clone()));
        }
        if let (false, Some(nested)) = (field_kinds.nested.is_empty(), value.as_object()) {
            if let Some(found) = mistyped(&field_kinds.nested, nested, &at(field)) {
                return Some(found);
            }
        }
    }
    None
}

/// The first field of a payload its component has no place for, nested
/// objects and `runtime` included, as its path: a field the export cannot
/// write, which both directions refuse (`site::owned`). An object a field
/// keeps whole is not looked into, and a note is no field.
pub fn unplaced(declared: &FxHashMap<String, Declared>, payload: &Map<String, Value>, path: &str) -> Option<String> {
    let at = |field: &str| if path.is_empty() { field.to_string() } else { format!("{path}.{field}") };
    for (field, value) in payload {
        if field == "$comment" {
            continue;
        }
        let Some(known) = declared.get(field) else { return Some(at(field)) };
        if let (false, Some(nested)) = (known.nested.is_empty(), value.as_object()) {
            if let Some(found) = unplaced(&known.nested, nested, &at(field)) {
                return Some(found);
            }
        }
    }
    None
}

/// The first field of a record's formatting the export has no place for, as
/// its path: an entry under one of this adapter's keys but `own`, the key the
/// record's kind is written back from — another kind's, which no file writes
/// for this record and the next ingest, rewriting this adapter's entries,
/// would drop — or a field of its own entry that the entry's schema does not
/// state. Another importer's entry is its own, and the ingest keeps it as it
/// keeps another system's `external_ids` key.
pub fn unplaced_formatting(
    declared: &FxHashMap<String, Declared>,
    payload: &Map<String, Value>,
    own: Option<&str>,
) -> Option<String> {
    for (key, entry) in payload {
        if key == "$comment" || !written::keys().any(|mine| mine == key) {
            continue;
        }
        if Some(key.as_str()) != own {
            return Some(key.clone());
        }
        let (Some(known), Some(entry)) = (declared.get(key), entry.as_object()) else { continue };
        if let Some(found) = unplaced(&known.nested, entry, key) {
            return Some(found);
        }
    }
    None
}

/// Whether a value is of one JSON Schema type.
fn is_kind(value: &Value, kind: &str) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    }
}

/// Whether the mapping's own vocabulary defines `name`: the components a
/// record's own fields land in, and its formatting.
pub fn defines(name: &str) -> bool {
    MEANINGS.iter().any(|(component, _, _)| *component == name)
}

/// Who answers for one name of the vocabulary, and the rung it enters at. The
/// three under `mapcreator.` enter at `experimental`; the dot finds them because
/// this vocabulary dots exactly those three (`only_three_names_are_still_prefixed`),
/// and not because the registry ties a rung to a dot.
fn rung_of(component: &str) -> (&'static str, &'static str) {
    if component == written::COMPONENT {
        ("foundry-studio", "candidate")
    } else if component.contains('.') {
        ("foundry-studio", "experimental")
    } else {
        ("foundry-studio", "ratified")
    }
}

/// The component every cell reference's target carries.
const CELL: &str = "cell";

/// The core components this adapter's records write fields into besides the
/// core's own: the protocol defines them, and states these fields among their
/// optional ones (`proto/schema/components/`), so the vocabulary writes no
/// definition of them.
pub const EXTENDED: [&str; 2] = ["cell", "presentation_point"];

/// What this adapter's fields add to one of [`EXTENDED`], derived from its
/// field maps exactly as a definition of its own is: the properties the
/// protocol's schema of the component has to state, and the reference
/// annotations the seed has to publish with it.
pub fn extension(component: &str) -> (Map<String, Value>, Vec<Reference>) {
    let schema = schema_for(component);
    let properties = schema["properties"].as_object().cloned().unwrap_or_default();
    (properties, references_of(component))
}

/// Every field of one component that holds references: each cell, cell list,
/// join and block list of its field maps, nested objects included, and the
/// references the import writes that no field map states.
fn references_of(component: &str) -> Vec<Reference> {
    let mut out = Vec::new();
    for kind in records::KINDS.iter().filter(|kind| kind.component == component) {
        for map in std::iter::once(kind.fields).chain(kind.dialects.iter().copied()) {
            annotate(map, "", &mut out);
        }
    }
    let extra: &[(&str, &str)] = match component {
        "block" => &[("/cells/*", CELL)],
        "mapcreator.station" => &[("/served_by/*", "block")],
        "presentation_point" | "station_point" => &[("/station", "mapcreator.station")],
        "ods_exclusion" => &[("/cell", CELL)],
        "sector_preference" => &[("/prefers/*", "sector")],
        _ => &[],
    };
    out.extend(extra.iter().map(|(path, expects)| Reference { path: path.to_string(), expects }));
    // ordered: a definition is read by a person and diffed as a file, and a
    // field both dialect tables state is one field.
    out.sort();
    out.dedup();
    out
}

/// The references one field map places, under the path of the object it is
/// read inside.
fn annotate(map: &[Field], under: &str, out: &mut Vec<Reference>) {
    for field in map {
        let path = format!("{under}/{}", field.target);
        let (path, expects) = match field.slot {
            Slot::Cell(_) => (path, CELL),
            Slot::Cells(_) => (format!("{path}/*"), CELL),
            Slot::Join(join, _) => (path, join.component()),
            Slot::Blocks => (format!("{path}/*"), "block"),
            Slot::Nested(inner) => {
                annotate(inner, &path, out);
                continue;
            }
            Slot::Key { .. }
            | Slot::Verbatim(_)
            | Slot::Runtime(_)
            | Slot::Written(_)
            | Slot::Echo { .. }
            | Slot::Elsewhere(_) => continue,
        };
        out.push(Reference { path, expects });
    }
}

/// The URN every one of these schemas takes its `$id` under.
///
/// The namespace and version these shapes were first published under, by the
/// crate this adapter was ported from; an identifier, kept as it was. Not `urn:worldproto:…`: that names the protocol's
/// schema layer, which Kristina owns and which carries the core vocabulary
/// alone.
pub const SCHEMA_URN: &str = "urn:mapcreator-compat:0.1:components";

/// The draft a JSON Schema here is written against, stated the way the seed's
/// own derived schemas state it — a compiler is entitled to know which dialect
/// it is reading before it reads it.
pub const SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// A component's properties as its schema states them, `runtime`'s among
/// them: what its schema is built around, and all a check of a payload's
/// fields needs, without the definitions the schema carries beside them. The
/// formatting's are its entries, one per key this adapter writes.
fn properties_of(component: &str) -> Map<String, Value> {
    if component == written::COMPONENT {
        return formatting_entries();
    }
    let mut properties = Map::new();
    let mut runtime = Map::new();
    for kind in records::KINDS {
        if kind.component != component {
            continue;
        }
        collect(kind.fields, &mut properties, &mut runtime);
        // One component, two dialect tables: the union of the two is the shape,
        // and a field both state in different JSON kinds accepts either.
        for dialect in kind.dialects {
            collect(dialect, &mut properties, &mut runtime);
        }
    }
    for (name, property) in extras(component) {
        merge(&mut properties, name, property);
    }
    if !runtime.is_empty() {
        properties.insert(
            "runtime".into(),
            json!({
                "description": "What was happening at the site when the export was written, as the export wrote it. SPEC §1 separates a world from what is happening in it, so this is here to be dropped by a policy that decides to rather than lost by a converter that forgot — the round trip writes it back.",
                "type": "object",
                "properties": Value::Object(runtime)
            }),
        );
    }
    properties
}

/// One component's JSON Schema, derived from its field map.
///
/// The root's shape is the one `scripts/derive-component-schemas.py`
/// emits for the core seed — the dialect marker, an `$id`, the body at the top
/// level, and a `$defs` carrying every definition the body reaches — so a seed
/// script can register these exactly as it registers the core vocabulary, and
/// the server's `jsonschema` compile has nothing to resolve off the machine.
fn schema_for(component: &str) -> Value {
    if component == written::COMPONENT {
        return rooted(component, formatting_entries(), Vec::new());
    }
    rooted(component, properties_of(component), demanded_by_the_export(component))
}

/// A schema's root around its properties, in the shape the seed's derived
/// schemas take.
fn rooted(component: &str, properties: Map<String, Value>, demanded: Vec<&'static str>) -> Value {
    let meaning = MEANINGS.iter().find(|(name, _, _)| *name == component).map(|(_, _, meaning)| *meaning);
    let mut root = json!({
        "$schema": SCHEMA_DIALECT,
        "$id": format!("{SCHEMA_URN}:{component}"),
        "title": component,
        "description": meaning.unwrap_or("A Map Creator component."),
        "type": "object",
        "properties": Value::Object(properties),
        // Left open on purpose, which is the reading SPEC §9 requires of a
        // consumer: a field a newer producer added is carried, not refused. The
        // converter's own completeness check is elsewhere and is stricter — a key
        // path in a dump with no disposition fails the run.
        "$comment": "Open to fields a newer producer adds (SPEC §9). Derived from `records.rs`'s field map by `vocabulary()`; edit the map, not this."
    });
    if !demanded.is_empty() {
        root.as_object_mut().expect("a JSON object").insert("required".into(), json!(demanded));
    }
    let reached = definitions_reached(&root);
    root.as_object_mut().expect("a JSON object").insert("$defs".into(), Value::Object(reached));
    root
}

/// The properties of `import_formatting`'s schema: one per key this adapter
/// writes an entry under, each the formatting of one record kind — its ordering
/// key, where its objects' key order and its whole numbers' spelling depart
/// from the file's usual, and what the kind's field maps and its import record
/// besides of how the file wrote it. Open to the keys another importer writes.
fn formatting_entries() -> Map<String, Value> {
    let mut entries = Map::new();
    // How a text spells what it states: an object's key order, a whole number
    // written `40.0`, a zero written `-0.0`, each by its pointer.
    let spelled = |properties: &mut Map<String, Value>| {
        properties.insert(
            "key_order".into(),
            json!({
                "description": KEY_ORDER_MEANING,
                "type": "object",
                "additionalProperties": { "type": "array", "items": { "type": "string" } }
            }),
        );
        properties.insert(
            "written_with_point".into(),
            json!({ "description": WITH_POINT_MEANING, "type": "array", "items": { "type": "string" } }),
        );
        properties.insert(
            "written_negative_zero".into(),
            json!({ "description": NEGATIVE_ZERO_MEANING, "type": "array", "items": { "type": "string" } }),
        );
    };
    for kind in records::KINDS {
        let mut properties = Map::new();
        properties.insert("index".into(), json!({ "description": INDEX_MEANING, "$ref": "#/$defs/record_index" }));
        spelled(&mut properties);
        for map in std::iter::once(kind.fields).chain(kind.dialects.iter().copied()) {
            collect_written(map, &mut properties);
        }
        for (name, property) in formatting_extras(kind.lens) {
            merge(&mut properties, name, property);
        }
        let description = format!(
            "How `{}` wrote the {} record this entity is, beyond what the record says about the site.",
            kind.file.name(),
            kind.construct.replace('_', " ")
        );
        entries.insert(kind.lens.to_string(), entry_schema(&description, properties, kind.lens));
    }
    for (key, description) in [
        (
            written::SECTOR,
            "How the cells' `sector` field and the sector index write the sector this entity is: its place among \
             the index's keys, and the JSON kind the cells spell its key in.",
        ),
        (
            written::SECTOR_PREFERENCE,
            "How `sectorMxUPreferences.json` wrote the preference this entity is: its place among the file's \
             carrier types, and how the file wrote its list of sectors — a preference's record is its list, so \
             each pointer here runs from the list — so that the list is written so under whatever carrier type \
             the preference has.",
        ),
    ] {
        let mut properties = Map::new();
        properties.insert("index".into(), json!({ "description": INDEX_MEANING, "$ref": "#/$defs/record_index" }));
        if key == written::SECTOR_PREFERENCE {
            spelled(&mut properties);
        }
        for (name, property) in formatting_extras(key) {
            merge(&mut properties, name, property);
        }
        entries.insert(key.to_string(), entry_schema(description, properties, key));
    }
    entries
}

/// One record kind's entry of `import_formatting`, requiring what the
/// projection cannot write such a record back without (`site::needs`).
fn entry_schema(description: &str, properties: Map<String, Value>, key: &str) -> Value {
    let mut entry = json!({ "description": description, "type": "object", "properties": Value::Object(properties) });
    let demanded = site::formatting_always_needed(key);
    if !demanded.is_empty() {
        entry.as_object_mut().expect("a JSON object").insert("required".into(), json!(demanded));
    }
    entry
}

/// The fields one field map copies into a record's formatting, as written.
fn collect_written(fields: &[Field], properties: &mut Map<String, Value>) {
    for field in fields {
        if let Slot::Written(shape) = field.slot {
            merge(properties, field.target, verbatim(field.source, shape));
        }
    }
}

/// The fields this adapter's own export refuses to write a record without.
///
/// A schema's `required` is what a producer is told at the door, and the honest
/// list is the one the reader of these components cannot do without — otherwise a
/// server accepts a document and the export that is supposed to write it back
/// refuses it, which is a refusal a site finds out about after the write rather
/// than during it.
///
/// So the list is read off the table both directions check an entity against
/// (`site::needs`): the fields of the component every entity carrying it needs,
/// whatever it states. A field only some entities need — a floor frame's floor
/// id, a waiting block's cells — a field inside a nested object, and a need of
/// one of two fields are checked there and not stated here. Every other field
/// the export leaves out where it is absent, and says so by not being here.
fn demanded_by_the_export(component: &str) -> Vec<&'static str> {
    site::fields_always_needed(component)
}

/// The descriptions `key_order`, `written_with_point`, `written_negative_zero`
/// and `index` carry in every record's formatting entry, all but the last on
/// the export entity's too.
const KEY_ORDER_MEANING: &str = "The order the file wrote the keys of an object of this record in, for each object whose order is not the one the export entity's `key_orders` gives an object with the same keys at the same place, by its JSON pointer from the record's own object (`\"\"` for that object; on the export entity, which holds what lies outside every record, the file's name and its pointer in the file). JSON gives key order no meaning; the export writes each file back byte for byte, so the order is a fact this document carries.";

const WITH_POINT_MEANING: &str = "The JSON pointers, from the record's own object (on the export entity, the file's name and the pointer in the file), of the whole numbers the file wrote with a decimal point (`40.0`), which a reader spells as integers, so that the export writes them back with one.";

const NEGATIVE_ZERO_MEANING: &str = "The JSON pointers, from the record's own object (on the export entity, the file's name and the pointer in the file), of the zeros the file wrote with a minus sign (`-0.0`), which a reader holds as the zero without one, so that the export writes them back with one.";

const INDEX_MEANING: &str = "The record's ordering key in its Map Creator file: bigger means later. A lens list comes back in identity order and the identities are random, so the order a file states its records in is a fact this document carries. The values are gapped rather than consecutive so that inserting a record between two others takes a value between theirs and leaves every other record's key alone — a re-import is a diff, and renumbering a file would rewrite every entity of it.";

/// The definitions the shared shapes take, and what each one is.
///
/// Three, and every one of them is a shape more than one property has: an
/// identity, a list of identities, and the verbatim value an unresolved
/// reference keeps. They sit in `$defs` rather than inline so that a person
/// reading one schema reads each shape's sentence once, which is what the seed's
/// own derived schemas do with the protocol's value definitions.
fn definitions() -> Vec<(&'static str, Value)> {
    vec![
        (
            "entity_id",
            json!({
                "title": "entity_id",
                "description": "One entity of this document, by identity. A declared identity is a UUID; a derived one is its root UUID and the generator's key path, which nothing in a converted document produces.",
                "type": "string"
            }),
        ),
        (
            "entity_ids",
            json!({
                "title": "entity_ids",
                "description": "Entities of this document by identity, in the order the Map Creator file states them.",
                "type": "array",
                "items": { "$ref": "#/$defs/entity_id" }
            }),
        ),
        (
            "legacy_value",
            json!({
                "title": "legacy_value",
                "description": "A legacy value the dump wrote, kept verbatim in whatever JSON kind it wrote it. It states no type on purpose: the round trip writes back what the dump said, and correcting a kind here would be the converter deciding what the site meant."
            }),
        ),
        (
            "placed_legacy_values",
            json!({
                "title": "placed_legacy_values",
                "description": "Legacy values a Map Creator list holds that named nothing in the dump, each with `at`, its place in the list counting from zero, and `value`, the value verbatim.",
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "at": { "type": "integer", "minimum": 0 },
                        "value": { "$ref": "#/$defs/legacy_value" }
                    },
                    "required": ["at", "value"]
                }
            }),
        ),
        (
            "record_index",
            json!({
                "title": "record_index",
                "description": "An ordering key over the records of one Map Creator file. A whole number, never negative, and never above 2^53 − 1: a key travels as a JSON number and is read back by consumers whose numbers are IEEE doubles, where every integer up to 2^53 is exact and the next one is not — so a larger key can come back as a different key, and two records can come back with one key between them.",
                "type": "integer",
                "minimum": 0,
                "maximum": ordering::MAX
            }),
        ),
    ]
}

/// Every definition one schema's own `$ref`s reach, transitively.
///
/// Only the ones reached: a `$defs` carrying a definition nothing points at is a
/// shape nobody can find a use for, and the seed's derive script collects the
/// same way.
fn definitions_reached(root: &Value) -> Map<String, Value> {
    let all = definitions();
    let mut out: Map<String, Value> = Map::new();
    let mut wanted: Vec<String> = refs_in(root);
    while let Some(name) = wanted.pop() {
        if out.contains_key(&name) {
            continue;
        }
        let Some((_, body)) = all.iter().find(|(defined, _)| *defined == name) else {
            continue;
        };
        out.insert(name, body.clone());
        wanted.extend(refs_in(body));
    }
    out
}

/// The local definition names one value's `$ref`s point at.
fn refs_in(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => {
            let mut out = Vec::new();
            for (key, child) in map {
                if key == "$ref" {
                    if let Some(name) = child.as_str().and_then(|r| r.strip_prefix("#/$defs/")) {
                        out.push(name.to_string());
                    }
                    continue;
                }
                out.extend(refs_in(child));
            }
            out
        }
        Value::Array(items) => items.iter().flat_map(refs_in).collect(),
        _ => Vec::new(),
    }
}

/// Put one property in, widening it where a name is already there.
///
/// Two dialect tables of one component can state the same field in two JSON
/// kinds — a station's `pps_id` is a number in v3 and a string in v2 — and the
/// component has to accept both, because both are real records of real sites.
/// Everything else about a repeated property has to agree, and the assertion is
/// the next four lines rather than this sentence: a second table carrying the same
/// name with a different meaning would be two facts under one key, which is the
/// thing the field map exists to prevent, and a doc comment claiming a check
/// nobody makes is how that goes unnoticed.
fn merge(properties: &mut Map<String, Value>, name: &str, property: Value) {
    let Some(present) = properties.get(name) else {
        properties.insert(name.to_string(), property);
        return;
    };
    if *present == property {
        return;
    }
    assert_eq!(
        without_type(present),
        without_type(&property),
        "the field map states `{name}` twice and the two say different things about it, so one \
         component cannot carry both: widening the JSON kind is the only difference two dialect \
         tables may have"
    );
    let mut widened = present.clone();
    let types = union_of_types(present.get("type"), property.get("type"));
    if let (Some(object), Some(types)) = (widened.as_object_mut(), types) {
        object.insert("type".into(), types);
    }
    properties.insert(name.to_string(), widened);
}

/// One property with its JSON kind removed — everything it says about itself
/// except which kinds it accepts, which is the one thing [`merge`] may widen.
fn without_type(property: &Value) -> Value {
    let mut out = property.clone();
    if let Some(object) = out.as_object_mut() {
        object.remove("type");
    }
    out
}

/// The union of two `type` statements, or `None` when either states none.
fn union_of_types(left: Option<&Value>, right: Option<&Value>) -> Option<Value> {
    let names = |value: Option<&Value>| -> Vec<String> {
        match value {
            Some(Value::String(one)) => vec![one.clone()],
            Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        }
    };
    let mut out = names(left);
    if out.is_empty() {
        return None;
    }
    for name in names(right) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    // ordered: a schema is read by a person and diffed as a file, so the type
    // list is written in one order rather than in the order two tables happened
    // to be walked.
    out.sort();
    Some(json!(out))
}

/// Walk one field map, putting each carried field where its slot says it goes.
fn collect(fields: &[Field], properties: &mut Map<String, Value>, runtime: &mut Map<String, Value>) {
    for field in fields {
        match field.slot {
            Slot::Key { .. } | Slot::Elsewhere(_) | Slot::Written(_) => {}
            Slot::Verbatim(shape) => merge(properties, field.target, verbatim(field.source, shape)),
            Slot::Runtime(shape) => merge(runtime, field.target, verbatim(field.source, shape)),
            Slot::Cell(_) => {
                merge(properties, field.target, reference(field.source, "cell", Shape::Reference));
                merge(
                    properties,
                    &format!("{}_unresolved", field.target),
                    unresolved(field.source, "the cell it names is not in this dump"),
                );
            }
            Slot::Cells(_) => {
                merge(properties, field.target, reference(field.source, "cells, in order", Shape::References));
                merge(
                    properties,
                    &format!("{}_unresolved", field.target),
                    placed_unresolved(field.source, "a cell that is not in this dump"),
                );
            }
            Slot::Join(join, _) => {
                merge(properties, field.target, reference(field.source, join.what(), Shape::Reference));
                merge(
                    properties,
                    &format!("{}_unresolved", field.target),
                    unresolved(
                        field.source,
                        &format!(
                            "it names no {} of this dump, or is not spelled in the JSON kind the export writes that \
                             key back in",
                            join.what()
                        ),
                    ),
                );
            }
            Slot::Blocks => {
                merge(properties, field.target, reference(field.source, "blocks, in order", Shape::References));
                merge(
                    properties,
                    &format!("{}_unresolved", field.target),
                    placed_unresolved(field.source, "a block that is not in this dump"),
                );
            }
            Slot::Echo { of } => merge(
                properties,
                field.target,
                json!({
                    "description": format!(
                        "Map Creator's `{}`, present only where it differs from the `{of}` it repeats in every \
                         row of every dump seen. Absent means the two agree and the export writes this one from \
                         the other.",
                        field.source
                    ),
                    "type": "string"
                }),
            ),
            Slot::Nested(inner) => {
                // A nested object carries no ordering key of its own: a record's
                // `index` orders the records of a file, and the objects inside one
                // record are not records. The import writes it in the record's
                // formatting only, and this says the same.
                let mut nested = Map::new();
                let mut nested_runtime = Map::new();
                collect(inner, &mut nested, &mut nested_runtime);
                merge(
                    properties,
                    field.target,
                    json!({
                        "description": format!("Map Creator's `{}` object, field by field.", field.source),
                        "type": "object",
                        "properties": Value::Object(nested)
                    }),
                );
            }
        }
    }
}

fn verbatim(source: &str, shape: Shape) -> Value {
    json!({
        "description": format!("Map Creator's `{source}`, carried as the export states it."),
        "type": shape.json_types()
    })
}

fn reference(source: &str, what: &str, shape: Shape) -> Value {
    let description = format!(
        "The {what} Map Creator's `{source}` names, by entity identity. The export writes the legacy \
         spelling back from the identity, so no display string is load-bearing here (SPEC §2)."
    );
    match shape {
        Shape::References => json!({ "description": description, "$ref": "#/$defs/entity_ids" }),
        _ => json!({ "description": description, "$ref": "#/$defs/entity_id" }),
    }
}

fn unresolved(source: &str, why: &str) -> Value {
    json!({
        "description": format!(
            "Map Creator's `{source}` verbatim, present only when {why}. Never corrected and never \
             dropped: the round trip writes back what the dump said, and the ledger counts it as a \
             dangling source reference."
        ),
        "$ref": "#/$defs/legacy_value"
    })
}

/// The fields a record does not state that the export would otherwise write
/// back, each by the name its file gives it.
fn absent_fields(fields: &[&str]) -> Value {
    json!({
        "description": "The Map Creator fields this record does not state and the export would otherwise write back — an empty list, or a value copied from another field — each by the name its file gives it. Absent when the record states them all.",
        "type": "array",
        "items": { "type": "string", "enum": fields }
    })
}

/// The entries of a list that named nothing, each kept with its place in the
/// list.
fn placed_unresolved(source: &str, what: &str) -> Value {
    json!({
        "description": format!(
            "The entries of Map Creator's `{source}` that name {what}, each verbatim with its place in the list the \
             file states. The entries that resolved are references in their own order, and the export writes each \
             of these back at its place among them. Present only when an entry named nothing."
        ),
        "$ref": "#/$defs/placed_legacy_values"
    })
}

/// The fields one component carries that no field map states: a reference the
/// import writes because the record's own file does not state it, and the handful
/// of components that have no record kind at all.
fn extras(component: &str) -> Vec<(&'static str, Value)> {
    let identity_field = |what: &'static str, why: &'static str| json!({ "description": format!("The {what} this record belongs to, by entity identity. {why}"), "$ref": "#/$defs/entity_id" });
    match component {
        "block" => vec![(
            "entity_type",
            json!({
                "description": "Map Creator's `entity_type` for the block — `queue`, `sequenced_queue`, `waiting` or a conveyor touch point — which decides which core component sits beside this one.",
                "type": "string"
            }),
        ), (
            "cells",
            json!({
                "description": "The cells this block covers, in order, by entity identity — for a block whose kind gives it no core component to state them in. SPEC §10 makes a waiting area's capacity its cell count and gives it no way to name its cells.",
                "$ref": "#/$defs/entity_ids"
            }),
        ), (
            "coordinates_unresolved",
            placed_unresolved("entity_details.coordinates", "a cell that is not in this dump"),
        )],
        "mapcreator.station" => vec![
            ("role", json!({ "description": "Map Creator's `type`: the product identity of the station, which `station.role` beside this repeats where a core kind was derivable. Absent when the record states no `type`.", "type": "string" })),
            ("served_by", json!({
                "description": "The blocks that serve this station, by entity identity, in the order `entities.blocks` lists them. The core's station component names nothing.",
                "$ref": "#/$defs/entity_ids"
            })),
            ("served_by_unresolved", placed_unresolved("entities.blocks", "a block that is not in this dump")),
        ],
        "presentation_point" => vec![
            (
                "station",
                identity_field("station", "A point sits inside its station's own record rather than naming it, so the reference is the import's and the export rebuilds the nesting from it."),
            ),
            ("world_coordinates_unresolved", json!({
                "description": "Map Creator's `attributes.world_coordinates` verbatim, present only when `attributes.barcode` names no cell of this dump or the point states no barcode: the export otherwise writes the coordinates back from the cell the barcode names, and here no cell can give them back.",
                "$ref": "#/$defs/legacy_value"
            })),
        ],
        "station_point" => vec![(
            "station",
            identity_field("station", "A point sits inside its station's own record rather than naming it, so the reference is the import's and the export rebuilds the nesting from it."),
        )],
        "ods_exclusion" => vec![
            ("cell", json!({ "description": "The cell the exclusion's tuple names, by entity identity. The tuple itself is `external_ids.mapcreator_ods_tuple`, and the export rebuilds it from this cell's barcode and the heading.", "$ref": "#/$defs/entity_id" })),
            ("heading", json!({ "description": "The heading half of Map Creator's `ods_tuple`, as the export spells it.", "type": "string" })),
            ("cell_barcode_unresolved", unresolved("ods_tuple", "the barcode half names no cell of this dump — 24 of one site's 180 do not")),
            ("tuple_unparsed", unresolved("ods_tuple", "it does not split into a barcode and a heading at all")),
        ],
        "aisle" => vec![(
            "io_point_coordinates_unresolved",
            placed_unresolved("io_point_coordinates", "a cell that is not in this dump"),
        )],
        "sector_preference" => vec![
            (
                "prefers",
                json!({
                    "description": "The sectors this carrier type prefers, by entity identity, in the order the file lists them.",
                    "$ref": "#/$defs/entity_ids"
                }),
            ),
            (
                "prefers_unresolved",
                placed_unresolved("sectorMxUPreferences.json", "a sector no cell of this dump states in that spelling"),
            ),
        ],
        "mapcreator.frame" => vec![
            ("role", json!({ "description": "Which frame this is: `site`, `floor`, `legacy_grid` or `vendor_vda5050`.", "type": "string", "enum": ["site", "floor", "legacy_grid", "vendor_vda5050"] })),
            ("floor_id", json!({ "description": "The floor id `map.json` states, on the floor frame. One export is one floor.", "type": "integer" })),
            ("vda5050_reference_barcode", json!({
                "description": "The barcode of the one cell `map.json` offers as a vendor-frame sample point. SPEC §4 names a sample point in place of a transform as the anti-pattern; the transform is declared on the vendor frame instead, and this is the sample the export stated. Its other half — the vendor coordinate — is that cell's own field, so the export rebuilds the pair from the cell.",
                "type": "string"
            })),
        ],
        "mapcreator.export" => vec![
            ("files", json!({
                "description": "Each file the site shipped, and the form it arrived in: `array`, `object`, `empty_array` or `empty_object`. A file the site never shipped is absent from this map. All four are different facts — five of the nineteen files are empty on every site seen, three of them as `{}` and the rest as `[]` — and the export reproduces whichever it found rather than guessing.",
                "type": "object",
                "additionalProperties": { "type": "string", "enum": ["array", "object", "empty_array", "empty_object", "other"] }
            })),
            ("conveyor_header", json!({ "description": "`conveyor.json`'s own dialect version and its vendor port classification, which sit beside the conveyor list rather than in it.", "type": "object" })),
            ("pps_bin_envelope", json!({ "description": "The captured HTTP request `pps_bin.json` is stored as — url, method and headers. Not model data, and carried because the export has to write the file back as it found it.", "type": "object" })),
            ("pps_bin_type_envelope", json!({ "description": "As `pps_bin_envelope`, for `pps_bintype.json`.", "type": "object" })),
            ("key_orders", json!({
                "description": "How the export's files usually order an object's keys: for each place an object stands at in a file, with list positions left out (`map.json[].map_values[]` is every cell), and for each set of keys an object there states, the order most objects with that set state it in when the set first appears, which a later import keeps while any object there still states the set; the orders kept first, then the sets most objects state. A record whose object departs records its own order (`key_order`); for an object no file stated, the place's first order serves.",
                "type": "object",
                "additionalProperties": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } }
            })),
            ("spacing", json!({
                "description": "How each file's text lays its value out, where it is not one compact line with nothing around it: `separators`, the one between two items and the one between a key and its value; `indent`, the unit each line is indented by per level, where every item stands on a line of its own; `before` and `after`, the whitespace around the value. A file whose text no such spacing writes is refused at ingest, naming what in it cannot be written back.",
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        "separators": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2 },
                        "indent": { "type": "string" },
                        "before": { "type": "string" },
                        "after": { "type": "string" }
                    }
                }
            })),
            ("pptl_empty_groups", json!({
                "description": "The places in `pptl.json`, counting from zero, of the groups that hold no device. A group is rebuilt from its devices, so one with none has nothing else to say it stood there.",
                "type": "array",
                "items": { "type": "integer", "minimum": 0 }
            })),
            ("location_profiles_empty", json!({
                "description": "`true` when `map.json` states `location_profiles` as an empty list, which no profile entity can say: a floor object stating no list and one stating an empty list are two texts. Absent otherwise.",
                "type": "boolean"
            })),
            ("key_order", json!({
                "description": KEY_ORDER_MEANING,
                "type": "object",
                "additionalProperties": { "type": "array", "items": { "type": "string" } }
            })),
            ("written_with_point", json!({ "description": WITH_POINT_MEANING, "type": "array", "items": { "type": "string" } })),
            ("written_negative_zero", json!({ "description": NEGATIVE_ZERO_MEANING, "type": "array", "items": { "type": "string" } })),
        ],
        _ => Vec::new(),
    }
}

/// What one entry of `import_formatting` records that no field map states, by
/// the key the entry rides under: what the import notes of how the file wrote
/// a record beyond its ordering key, its key orders and its whole numbers.
fn formatting_extras(key: &str) -> Vec<(&'static str, Value)> {
    match key {
        "mapcreator_station" => vec![
            ("absent", absent_fields(&["entities.presentation_points", "pps_points"])),
            (
                "dialect_shape",
                json!({
                    "description": "Which of pps.json's two dialects this record was read by: `v3`, where `location` is a raw grid pair and the parts hang under `entities`, or `v2`, where `location` is a barcode and the points are `pps_points`. The export cannot write a station back without it, and it is a fact about the record's shape rather than about the site — `dialect` beside it is the `version` value the file states.",
                    "type": "string", "enum": ["v2", "v3"]
                }),
            ),
        ],
        "mapcreator_presentation_point" => vec![("absent", absent_fields(&["attributes.world_coordinates"]))],
        "mapcreator_dock_point" => vec![("absent", absent_fields(&["dock_barcode"]))],
        "mapcreator_pptl" => vec![(
            "group",
            json!({
                "description": "The module's group in `pptl.json`, counting from zero. A group is not one station's — one site lists ten stations in twenty groups, the same ten twice over with different peripheral ids — so the grouping is the module's place in the file, which no field of a record determines.",
                "$ref": "#/$defs/record_index"
            }),
        )],
        "mapcreator_aisle" => vec![(
            "io_point_coordinates_as_pairs",
            json!({
                "description": "The places in Map Creator's `io_point_coordinates`, counting from zero, of the entries that name a cell by a raw grid pair such as `[6, 1]` rather than by the coordinate string `\"[6,1]\"`, so the export writes each back as the file spelled it. Absent when every entry is a string.",
                "type": "array",
                "items": { "type": "integer", "minimum": 0 }
            }),
        )],
        written::SECTOR => vec![(
            "key",
            json!({
                "description": "The sector's key in the JSON kind the cell list spells it — a number for a numbered sector, the string `undefined` for the cells assigned to none — while the sector index keys on the string form of both, which `external_ids.mapcreator_sector` holds. The export writes each back as its own file spells it.",
                "type": ["string", "number"]
            }),
        )],
        _ => Vec::new(),
    }
}

/// The `external_ids` keys the mapping writes, with what each one is. Not a
/// component definition — SPEC §2 fixes `external_ids`'s meaning and fixes no keys,
/// since every key is another system's spelling — but a registry's operator wants
/// the list, and so does anybody reading a converted document.
pub fn external_id_keys() -> Vec<(&'static str, String)> {
    let mut keys: Vec<(&'static str, String)> = Vec::new();
    for shape in identity::SHAPES {
        for field in shape.fields {
            keys.push((field, format!("part of the lookup key for a Map Creator {}", shape.construct)));
        }
    }
    keys.push(("barcode", "the floor sticker on a cell, written as plain data beside the cell's own key".to_string()));
    keys.push(("vda_world_coordinate", "a cell's recorded position in the robot vendor's frame".to_string()));
    keys.push((
        "butler_bot_version",
        "a robot class's bot type as butler names it, written as plain data beside the class's own key".to_string(),
    ));
    keys.sort();
    keys.dedup_by(|a, b| a.0 == b.0);
    keys
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{extension, external_id_keys, merge, protocol_fields, vocabulary, Reference, EXTENDED, MEANINGS};
    use crate::records::{Field, Slot};
    use crate::{identity, records, written};
    use serde_json::{json, Map, Value};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    /// The files under `vocabulary/` are what the tables generate, one per
    /// component, and nothing else is there: the tables in `records.rs` are the
    /// source, and a file that disagrees with them is a definition a registry
    /// would be handed out of date. With `MAPCREATOR_WRITE_VOCABULARY=1` set, the
    /// test writes the files afresh instead of comparing.
    #[test]
    fn every_committed_vocabulary_file_is_what_the_tables_generate() {
        let directory = PathBuf::from(env!("FOUNDRY_APPLICATION_ROOT")).join("data/seed/application/mapcreator/components");
        let write = std::env::var("MAPCREATOR_WRITE_VOCABULARY").as_deref() == Ok("1");
        let mut generated = BTreeSet::new();
        for definition in vocabulary() {
            let name = format!("{}.json", definition.component);
            let text = serde_json::to_string_pretty(&definition.to_json()).expect("JSON") + "\n";
            let path = directory.join(&name);
            if write {
                fs::create_dir_all(&directory).expect("the vocabulary directory");
                fs::write(&path, &text).expect("the file writes");
            } else {
                let committed = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(
                    committed == text,
                    "{name} is not what the tables generate; MAPCREATOR_WRITE_VOCABULARY=1 writes it afresh"
                );
            }
            generated.insert(name);
        }
        let present: BTreeSet<String> = fs::read_dir(&directory)
            .expect("the vocabulary directory")
            .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(present, generated, "a file under vocabulary/ that the tables do not generate, or the reverse");
    }

    /// Two dialect tables may differ on a field's JSON kind, and the union of the
    /// two is the shape the component publishes.
    #[test]
    fn one_name_in_two_dialects_accepts_both_kinds() {
        let mut properties = Map::new();
        let says = |kind: &str| json!({ "description": "Map Creator's `pps_id`.", "type": kind });
        merge(&mut properties, "pps_id", says("number"));
        merge(&mut properties, "pps_id", says("string"));
        assert_eq!(properties["pps_id"]["type"], json!(["number", "string"]));
        assert_eq!(properties["pps_id"]["description"], json!("Map Creator's `pps_id`."));
    }

    /// And may differ on nothing else. Two tables carrying one name with two
    /// meanings would be two facts under one key: the widening kept the first
    /// meaning and dropped the second silently, and a doc comment said otherwise.
    #[test]
    #[should_panic(expected = "states `pps_id` twice")]
    fn one_name_meaning_two_things_is_refused() {
        let mut properties = Map::new();
        merge(&mut properties, "pps_id", json!({ "description": "the station's own id.", "type": "number" }));
        merge(&mut properties, "pps_id", json!({ "description": "the bin's station id.", "type": "number" }));
    }

    #[test]
    fn every_record_kinds_component_is_defined() {
        let defined: BTreeSet<&str> = vocabulary().iter().map(|definition| definition.component).collect();
        for kind in records::KINDS {
            let extended = EXTENDED.contains(&kind.component);
            assert!(defined.contains(kind.component) != extended, "`{}` has no definition, or two", kind.component);
        }
    }

    /// Every field this adapter writes into a core component is one the
    /// protocol's schema of that component states, as the same kind of value —
    /// a reference where it holds one, the same JSON kinds otherwise — and every
    /// reference among them is annotated in the test fixture for
    /// the component, at the same path. The protocol's schema is where these
    /// fields are defined now, so this is what keeps it and the field maps one
    /// statement.
    #[test]
    fn every_field_written_into_a_core_component_is_one_the_protocol_states_and_the_seed_annotates() {
        let repository = PathBuf::from(env!("FOUNDRY_APPLICATION_ROOT"));
        let bytes = fs::read(repository.join("data/seed/application/core-components.json"))
            .expect("the core component definitions");
        let references: Value = serde_json::from_slice(&bytes).expect("the core component definitions");
        for component in EXTENDED {
            let (properties, refs) = extension(component);
            let protocol = protocol_definition(&repository, component);
            agree(component, &properties, &protocol["properties"]);
            // Beyond these, the protocol states only the fields the checks of
            // a payload count as its own.
            let stated: BTreeSet<&str> =
                protocol["properties"].as_object().into_iter().flatten().map(|(name, _)| name.as_str()).collect();
            let known: BTreeSet<&str> =
                properties.keys().map(String::as_str).chain(protocol_fields(component).iter().copied()).collect();
            assert_eq!(
                stated, known,
                "the protocol's `{component}` and this adapter's reading of it name other fields"
            );
            let published = fixture_annotations(&references, component);
            for one in refs {
                let found = published.iter().find(|(path, _)| *path == one.path);
                let (_, expects) = found.unwrap_or_else(|| panic!("the seed publishes no `{component}{}`", one.path));
                // A shared component's annotation names the target's component
                // only where every producer's target carries the same one.
                assert!(
                    expects.as_deref().is_none_or(|expects| expects == one.expects),
                    "the seed says `{component}{}` names a `{expects:?}`, and this adapter a `{}`",
                    one.path,
                    one.expects
                );
            }
        }
    }

    /// One core component's definition in the protocol's schema layer.
    fn protocol_definition(repository: &Path, component: &str) -> Value {
        let directory = repository.join("data/seed/application/schema/components");
        for entry in fs::read_dir(&directory).expect("the component schemas") {
            let file: Value =
                serde_json::from_slice(&fs::read(entry.expect("an entry").path()).expect("a schema")).expect("JSON");
            if let Some(definition) = file["$defs"].get(component) {
                return definition.clone();
            }
        }
        panic!("the protocol defines no `{component}`")
    }

    /// Whether every property this adapter states is one the protocol states as
    /// the same kind of value, nested objects walked into.
    fn agree(component: &str, ours: &Map<String, Value>, theirs: &Value) {
        let reference = |property: &Value| {
            let named = property.get("$ref").or_else(|| property.get("items").and_then(|items| items.get("$ref")));
            named.and_then(Value::as_str).is_some_and(|name| {
                ["entity_id", "entity_ids", "entity_ref", "cell_address"].iter().any(|kind| name.ends_with(kind))
            })
        };
        let kinds = |property: &Value| -> BTreeSet<String> {
            match property.get("type") {
                Some(Value::String(one)) => [one.clone()].into(),
                Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                _ => BTreeSet::new(),
            }
        };
        for (name, property) in ours {
            let stated = &theirs[name];
            assert!(stated.is_object(), "the protocol's `{component}` states no `{name}`");
            assert_eq!(reference(property), reference(stated), "`{component}.{name}` is a reference on one side only");
            if !reference(property) {
                assert_eq!(kinds(property), kinds(stated), "`{component}.{name}` takes other JSON kinds");
            }
            if let Some(inner) = property.get("properties").and_then(Value::as_object) {
                agree(&format!("{component}.{name}"), inner, &stated["properties"]);
            }
        }
    }

    /// The test vocabulary's reference annotations for one component.
    fn fixture_annotations(references: &Value, component: &str) -> Vec<(String, Option<String>)> {
        references[component]["refs"]
            .as_array()
            .expect("an annotation list")
            .iter()
            .map(|one| (one["path"].as_str().expect("a path").to_string(), one["expects"].as_str().map(str::to_string)))
            .collect()
    }

    /// Every definition says the four things DESIGN.md §7 requires of one: the
    /// name, the rung, the is/has reading, and a meaning a person can disagree
    /// with. A blank on any of them is a name nobody can govern.
    #[test]
    fn every_definition_states_its_meaning_and_its_rung() {
        for definition in vocabulary() {
            let (owner, status) = match definition.component.split_once('.') {
                Some(("mapcreator", _)) => ("foundry-studio", "experimental"),
                Some(_) => panic!("`{}` is in a namespace not this adapter's", definition.component),
                None if definition.component == written::COMPONENT => ("foundry-studio", "candidate"),
                None => ("foundry-studio", "ratified"),
            };
            assert_eq!(definition.owner, owner, "`{}`", definition.component);
            assert_eq!(definition.status, status, "`{}`", definition.component);
            assert!(
                definition.asserts == "is" || definition.asserts == "has",
                "`{}` asserts `{}`",
                definition.component,
                definition.asserts
            );
            assert!(definition.meaning.len() > 60, "`{}`'s meaning is too short to be one", definition.component);
        }
    }

    /// Three names are still prefixed: the export's record of its own files,
    /// and the frame and the station, whose undotted names the core holds and
    /// which `DECISIONS.md` says do not merge into them obviously. Every other
    /// name is shared vocabulary, and this list only shrinks.
    #[test]
    fn only_three_names_are_still_prefixed() {
        let prefixed: BTreeSet<&str> =
            vocabulary().iter().map(|definition| definition.component).filter(|name| name.contains('.')).collect();
        let expected = ["mapcreator.export", "mapcreator.frame", "mapcreator.station"];
        assert_eq!(prefixed, expected.into_iter().collect());
    }

    #[test]
    fn no_component_is_defined_twice() {
        let mut names: Vec<&str> = MEANINGS.iter().map(|(name, _, _)| *name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "a component is defined twice");
    }

    /// The whole reason the schemas are derived: every field the converter writes
    /// has a property, and every property is a field the converter writes. A
    /// schema that disagrees with the converter is worse than no schema.
    #[test]
    fn every_component_schema_matches_its_field_map() {
        for definition in vocabulary().into_iter().filter(|definition| definition.component != written::COMPONENT) {
            let properties = definition.schema["properties"].as_object().expect("properties");
            let mut expected: BTreeSet<String> = BTreeSet::new();
            for kind in records::KINDS.iter().filter(|kind| kind.component == definition.component) {
                walk(kind.fields, &mut expected);
                for dialect in kind.dialects {
                    walk(dialect, &mut expected);
                }
            }
            for (name, _) in super::extras(definition.component) {
                expected.insert(name.to_string());
            }
            let got: BTreeSet<String> = properties.keys().cloned().collect();
            assert_eq!(got, expected, "`{}`'s schema and its field map disagree", definition.component);
        }
    }

    fn walk(fields: &[Field], into: &mut BTreeSet<String>) {
        for field in fields {
            match field.slot {
                Slot::Key { .. } | Slot::Elsewhere(_) | Slot::Written(_) => {}
                Slot::Runtime(_) => {
                    into.insert("runtime".to_string());
                }
                Slot::Cell(_) | Slot::Cells(_) | Slot::Join(..) | Slot::Blocks => {
                    into.insert(field.target.to_string());
                    into.insert(format!("{}_unresolved", field.target));
                }
                _ => {
                    into.insert(field.target.to_string());
                }
            }
        }
    }

    /// The formatting has one entry per key this adapter writes, and each
    /// record kind's entry holds its ordering key, its key orders and its whole
    /// numbers written with a decimal point, the fields its field maps copy as
    /// written, and what its import notes besides: nothing a record states
    /// about the site, and so no reference.
    #[test]
    fn the_formatting_has_one_entry_per_key_and_each_holds_how_its_file_wrote_it() {
        let formatting = vocabulary()
            .into_iter()
            .find(|definition| definition.component == written::COMPONENT)
            .expect("the formatting is defined");
        let entries = formatting.schema["properties"].as_object().expect("properties");
        let keys: BTreeSet<&str> = entries.keys().map(String::as_str).collect();
        assert_eq!(keys, written::keys().collect::<BTreeSet<&str>>());
        for kind in records::KINDS {
            let mut expected: BTreeSet<String> = ["index", "key_order", "written_with_point", "written_negative_zero"]
                .into_iter()
                .map(str::to_string)
                .collect();
            for map in std::iter::once(kind.fields).chain(kind.dialects.iter().copied()) {
                let as_written = map.iter().filter(|field| matches!(field.slot, Slot::Written(_)));
                expected.extend(as_written.map(|field| field.target.to_string()));
            }
            expected.extend(super::formatting_extras(kind.lens).into_iter().map(|(name, _)| name.to_string()));
            let got: BTreeSet<String> =
                entries[kind.lens]["properties"].as_object().expect("an entry's properties").keys().cloned().collect();
            assert_eq!(got, expected, "the formatting of `{}`", kind.construct);
            let required = entries[kind.lens]["required"].as_array().cloned().unwrap_or_default();
            assert!(required.contains(&json!("index")), "the formatting of `{}` requires no index", kind.construct);
        }
        assert!(formatting.refs.is_empty(), "the formatting names an entity");
    }

    /// Every property says what it is and what shape it takes, because a registry
    /// hands these to a validator and to a person, and a property with neither is
    /// no better than an untyped bag.
    #[test]
    fn every_property_says_what_it_is() {
        for definition in vocabulary() {
            check(&definition.schema, definition.component);
        }
    }

    fn check(schema: &Value, component: &str) {
        assert_eq!(schema["type"], "object", "`{component}`");
        for (name, property) in schema["properties"].as_object().expect("properties") {
            assert!(property["description"].is_string(), "`{component}`.{name} says nothing about itself");
            assert!(
                property["description"].as_str().unwrap_or_default().len() > 20,
                "`{component}`.{name}'s description is too short to be one"
            );
            // A property states its shape outright or names a definition that
            // does. `legacy_value` is the definition that deliberately states no
            // type: it is whatever kind the dump wrote.
            assert!(
                !property["type"].is_null() || property["$ref"].is_string(),
                "`{component}`.{name} states neither a type nor a definition"
            );
            if property["type"] == "object" && property["properties"].is_object() {
                check(property, component);
            }
        }
    }

    /// Every `$ref` a schema writes resolves inside that schema's own `$defs`,
    /// and every definition in there is one a `$ref` reaches.
    ///
    /// Both halves matter to the server: `schema_defs::compile` resolves nothing
    /// off the machine except the server's own definitions document, so a `$ref`
    /// pointing at a definition this schema does not carry is a schema that will
    /// not compile — and a definition nothing points at is a shape nobody can
    /// find a use for.
    #[test]
    fn every_definition_a_schema_names_is_in_its_own_defs() {
        for definition in vocabulary() {
            let component = definition.component;
            let defs = definition.schema["$defs"].as_object().expect("every schema carries $defs").clone();
            let mut body = definition.schema.clone();
            body.as_object_mut().expect("an object").remove("$defs");
            let mut reached: BTreeSet<String> = super::refs_in(&body).into_iter().collect();
            let mut frontier: Vec<String> = reached.iter().cloned().collect();
            while let Some(name) = frontier.pop() {
                let held = defs.get(&name).unwrap_or_else(|| panic!("`{component}` names `{name}` and $defs has none"));
                for further in super::refs_in(held) {
                    if reached.insert(further.clone()) {
                        frontier.push(further);
                    }
                }
            }
            let carried: BTreeSet<String> = defs.keys().cloned().collect();
            assert_eq!(carried, reached, "`{component}`'s $defs and its references disagree");
        }
    }

    /// The root of every schema says which dialect it is and what it is called,
    /// which is what the seed's own derived schemas say.
    #[test]
    fn every_schema_states_its_dialect_and_its_id() {
        for definition in vocabulary() {
            assert_eq!(definition.schema["$schema"], super::SCHEMA_DIALECT, "`{}`", definition.component);
            assert_eq!(
                definition.schema["$id"],
                Value::from(format!("{}:{}", super::SCHEMA_URN, definition.component)),
                "`{}`",
                definition.component
            );
        }
    }

    /// The station is one component over two export dialects, so a field both of
    /// them state in a different JSON kind accepts either — and a field only the
    /// older one states is there at all.
    ///
    /// This is the check that would have caught a schema derived from the v3
    /// table alone: it says `pps_id` is a number, and site-c's 20 stations and
    /// site-e's 10 spell it `"1"`.
    #[test]
    fn the_station_schema_spans_both_export_dialects() {
        let definitions = vocabulary();
        let station = definitions
            .iter()
            .find(|definition| definition.component == "mapcreator.station")
            .expect("the station is defined");
        let properties = station.schema["properties"].as_object().expect("properties");
        // How the file spells the station's key and states its version is its
        // formatting, and both dialects' kinds are accepted there.
        let formatting = definitions
            .iter()
            .find(|definition| definition.component == written::COMPONENT)
            .expect("the formatting is defined");
        let as_written =
            formatting.schema["properties"]["mapcreator_station"]["properties"].as_object().expect("properties");
        for (field, kinds) in [("pps_id", ["number", "string"]), ("dialect", ["number", "string"])] {
            let properties = as_written;
            let stated = properties[field]["type"].as_array().unwrap_or_else(|| panic!("{field} states one type only"));
            for kind in kinds {
                assert!(stated.iter().any(|t| t == kind), "a station's `{field}` refuses a {kind}");
            }
        }
        for v2_only in [
            "eligible_system",
            "queue_cells",
            "pick_cell",
            "pick_direction",
            "put_docking_positions",
            "docking_stations",
        ] {
            assert!(properties.contains_key(v2_only), "the v2 field `{v2_only}` has no property");
        }
    }

    /// Every join field of every record kind is annotated as a reference to the
    /// component the joined record carries: a bin's type, its two seats and its
    /// station among them, which the server then refuses to delete from under
    /// the bin.
    #[test]
    fn every_join_field_is_annotated_as_a_reference_to_what_it_names() {
        fn joins(map: &[Field], under: &str, out: &mut Vec<Reference>) {
            for field in map {
                let path = format!("{under}/{}", field.target);
                match field.slot {
                    Slot::Join(join, _) => out.push(Reference { path, expects: join.component() }),
                    Slot::Nested(inner) => joins(inner, &path, out),
                    _ => {}
                }
            }
        }
        let definitions = vocabulary();
        let mut annotated = 0;
        for kind in records::KINDS {
            let refs = match definitions.iter().find(|one| one.component == kind.component) {
                Some(definition) => definition.refs.clone(),
                None => extension(kind.component).1,
            };
            let mut named = Vec::new();
            for map in std::iter::once(kind.fields).chain(kind.dialects.iter().copied()) {
                joins(map, "", &mut named);
            }
            for join in named {
                assert!(refs.contains(&join), "`{}` does not annotate {join:?}", kind.component);
                annotated += 1;
            }
        }
        assert_eq!(annotated, 11, "the number of join fields across the record kinds moved");
    }

    /// Every property whose schema is an entity identity, or a list of them, is
    /// annotated, at its path, and no other property is: the server reads a
    /// legacy value at an annotated path as a reference and refuses the write,
    /// and it never protects a reference field left out. Each names a component
    /// this vocabulary defines as what its target carries.
    #[test]
    fn every_reference_property_is_annotated_and_no_other_is() {
        fn paths(properties: &Value, under: &str, out: &mut Vec<String>) {
            for (name, property) in properties.as_object().into_iter().flatten() {
                let path = format!("{under}/{name}");
                match property.get("$ref").and_then(Value::as_str) {
                    Some("#/$defs/entity_id") => out.push(path.clone()),
                    Some("#/$defs/entity_ids") => out.push(format!("{path}/*")),
                    _ => {}
                }
                if let Some(inner) = property.get("properties") {
                    paths(inner, &path, out);
                }
            }
        }
        for definition in vocabulary() {
            let mut typed = Vec::new();
            paths(&definition.schema["properties"], "", &mut typed);
            typed.sort();
            let annotated: Vec<String> = definition.refs.iter().map(|one| one.path.clone()).collect();
            assert_eq!(annotated, typed, "`{}`'s annotations and its reference fields disagree", definition.component);
            for one in &definition.refs {
                assert!(
                    MEANINGS.iter().any(|(name, _, _)| *name == one.expects) || CORE_TARGETS.contains(&one.expects),
                    "`{}` annotates {} as expecting `{}`, which this vocabulary does not define",
                    definition.component,
                    one.path,
                    one.expects
                );
            }
        }
    }

    /// The core components a reference of this vocabulary may expect its target
    /// to carry: what the import writes on every cell and every sector it
    /// declares.
    const CORE_TARGETS: [&str; 2] = ["cell", "sector"];

    /// The external-id keys a converted document carries, listed. Every lookup key
    /// field is one of them, which is what makes the list complete rather than
    /// remembered.
    #[test]
    fn every_lookup_key_field_is_listed() {
        let listed: BTreeSet<&str> = external_id_keys().iter().map(|(key, _)| *key).collect();
        for shape in identity::SHAPES {
            for field in shape.fields {
                assert!(listed.contains(field), "`{field}` is a lookup key field and is not listed");
            }
        }
        for (_, what) in external_id_keys() {
            assert!(!what.is_empty());
        }
    }
}
