//! A document, and the four kinds of record it can contain.
//!
//! SPEC §2 makes the *shape* of a record its kind: an `id` declares, an `id`
//! plus `patch` modifies something an earlier layer declared, a `derivation`
//! reaches a derived entity, and `suppress` deletes. Reading those four
//! apart is this module's whole job, and it is deliberately strict — a record
//! that could be read two ways is refused rather than guessed at, because every
//! confusion here becomes a half-formed entity somewhere downstream.

use crate::components;
use crate::derivation::RawDerivation;
use crate::error::LoadError;
use crate::ids::{DeclaredEntityId, DerivedEntityId, DocId, EntityId};
use crate::placeholders;
use crate::units::{Coord, Steps};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The metadata block every document opens with.
///
/// The expanded form re-serializes this block verbatim, so the same rule the
/// re-serialized component views follow applies here: a field the document left
/// unstated serializes to nothing — not to an explicit null, and not to an empty
/// list. SPEC §9 holds two implementations to the canonical projection of that
/// artifact, and a scenario that names no `name` and depends on nothing should
/// not cost them a byte of disagreement over how to spell the absence.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DocMeta {
    pub id: DocId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Documents this one references into. A document loads standalone only if
    /// all of them are present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<DocId>,
    /// The scopes a server assembled this document from (SPEC §8): where its
    /// records were read from, carried so the expanded form can write it back,
    /// and never read for what the document means.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assembled_from: Vec<SourceScope>,
}

/// One scope an assembled document holds, at the changeset its records were
/// read at. Every field is the assembling server's word, kept as it was written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SourceScope {
    pub scope: String,
    pub changeset: u64,
    /// Stated only when the read reached the changeset through a tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// The `expect` pin: where the author believes an ordinal-keyed address sits.
/// Tiling keys are ordinal, so a pitch change re-aims an address at a different
/// patch of floor; the pin turns that into a loud failure (SPEC §3).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Expect {
    pub position: [Coord; 2],
    pub tolerance: Steps,
}

/// One entry of a document's `entities` array.
#[derive(Debug, Clone)]
pub enum Record {
    /// Introduces an entity, or replaces components of one an earlier layer
    /// introduced.
    Declaration(Declaration),
    /// Modifies an entity some earlier layer declared, and fails the load if no
    /// such entity exists — so a mistyped UUID in an overlay cannot mint a
    /// half-formed entity.
    Patch(Declaration),
    /// Replaces components of one derived entity.
    Override(Override),
    /// Deletes the addressed entity.
    Suppression(Suppression),
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub index: usize,
    /// A UUID, never an address. Writing an entity down is what mints its
    /// identity; an address would claim a generator produced it (SPEC §3).
    pub id: DeclaredEntityId,
    pub name: Option<String>,
    pub components: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct Override {
    pub doc: DocId,
    pub index: usize,
    /// A record may state both `id` and `derivation`; the loader compares them
    /// and rejects a mismatch.
    pub stated_id: Option<EntityId>,
    pub address: DerivedEntityId,
    pub expect: Option<Expect>,
    pub components: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct Suppression {
    pub index: usize,
    pub target: EntityId,
    pub expect: Option<Expect>,
}

impl Record {
    pub fn index(&self) -> usize {
        match self {
            Record::Declaration(d) | Record::Patch(d) => d.index,
            Record::Override(o) => o.index,
            Record::Suppression(s) => s.index,
        }
    }

    /// The identity a record addresses, and therefore what makes two records in
    /// one document duplicates of each other. One entity is one target however
    /// it was reached: a record naming a derived entity by its `derivation` and
    /// one naming it by its address in `id` address the same thing, because the
    /// address is the identity.
    pub fn target(&self) -> EntityId {
        match self {
            Record::Declaration(d) | Record::Patch(d) => d.id.into(),
            Record::Override(o) => o.address.clone().into(),
            Record::Suppression(s) => s.target.clone(),
        }
    }

    /// The entity a record declares, if it declares one. Only a declaration owns
    /// an inline expansion: an override or a suppression addresses something a
    /// generator produced, and that entity's expansion nests under the record
    /// that started the chain.
    pub fn declared_id(&self) -> Option<DeclaredEntityId> {
        match self {
            Record::Declaration(d) | Record::Patch(d) => Some(d.id),
            _ => None,
        }
    }

    /// The word this kind of record is called by, for error messages.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Record::Declaration(_) => "declaration",
            Record::Patch(_) => "patch",
            Record::Override(_) => "override",
            Record::Suppression(_) => "suppression",
        }
    }
}

/// A parsed document: metadata, records, and — for a scenario — the ordered
/// layers it composes.
#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub proto_version: String,
    pub meta: DocMeta,
    pub primary_frame: Option<EntityId>,
    pub required_components: Vec<String>,
    pub records: Vec<Record>,
    /// Paths as written, resolved relative to this document's own directory.
    pub layers: Vec<String>,
    pub settings: Option<Value>,
    /// Inline expansions, by the record that owns them. A document may carry the
    /// entities its generators derive as a `derived` subtree; because expansion
    /// is deterministic, that subtree is a cache and never a second source of
    /// truth — a loader may trust it or re-derive, and the two must agree.
    pub expansions: BTreeMap<DeclaredEntityId, RawExpansion>,
}

/// One record's inline expansion, exactly as written. Parsing waits until
/// [`crate::expanded`] compares it against what the source derives, so a
/// malformed cache is reported beside the divergence it causes rather than as a
/// shape error three steps earlier.
#[derive(Debug, Clone, Default)]
pub struct RawExpansion {
    /// Segment string to node, so two nodes cannot claim one segment: the shape
    /// itself rules out the duplicate rather than a check having to.
    pub derived: Map<String, Value>,
    pub edges: Vec<Value>,
}

impl Document {
    /// Read and classify a document. Everything that can be decided by looking
    /// at one file alone is decided here; anything needing the other layers
    /// waits for the pipeline.
    pub fn load(path: &Path) -> Result<Document, LoadError> {
        let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|source| LoadError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        Document::from_value(path, &value)
    }

    pub fn from_value(path: &Path, value: &Value) -> Result<Document, LoadError> {
        let shape = |message: String| LoadError::Shape {
            path: path.to_path_buf(),
            message,
        };
        let obj = value
            .as_object()
            .ok_or_else(|| shape("a document must be a JSON object".into()))?;

        let proto_version = obj
            .get("proto_version")
            .and_then(Value::as_str)
            .ok_or_else(|| shape("`proto_version` is missing".into()))?
            .to_string();
        check_proto_version(path, &proto_version)?;

        let meta_value = obj
            .get("doc")
            .ok_or_else(|| shape("`doc` metadata is missing".into()))?;
        let meta: DocMeta = serde_json::from_value(meta_value.clone())
            .map_err(|e| shape(format!("`doc` metadata does not parse: {e}")))?;

        let primary_frame = match obj.get("primary_frame") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                serde_json::from_value(v.clone())
                    .map_err(|e| shape(format!("`primary_frame` is not a UUID: {e}")))?,
            ),
        };

        let required_components: Vec<String> = match obj.get("required_components") {
            None => Vec::new(),
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| shape(format!("`required_components` does not parse: {e}")))?,
        };
        for component in &required_components {
            if !components::is_known(component) {
                return Err(LoadError::UnknownRequiredComponent {
                    doc: meta.id,
                    component: component.clone(),
                });
            }
        }

        let layers: Vec<String> = match obj.get("layers") {
            None => Vec::new(),
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| shape(format!("`layers` does not parse: {e}")))?,
        };

        // A document once announced "I carry my own derived entities" with a
        // top-level flag — `materialized` through v0.3, `baked` briefly in v0.4.
        // The expanded form replaced both: the expansion rides inline on the
        // records that own it, so its presence is the announcement and a flag
        // has nothing left to say. §9's rule that a consumer ignores what it does
        // not understand covers unknown fields *inside a known component*, not a
        // retired flag at document scope, so these are refused by name rather
        // than read as false.
        for retired in ["materialized", "baked"] {
            if obj.contains_key(retired) {
                return Err(LoadError::RetiredField {
                    path: path.to_path_buf(),
                    field: retired,
                    replacement: "an inline `derived` subtree on the records that own one",
                });
            }
        }

        let mut records = Vec::new();
        let mut expansions: BTreeMap<DeclaredEntityId, RawExpansion> = BTreeMap::new();
        if let Some(entities) = obj.get("entities") {
            let array = entities
                .as_array()
                .ok_or_else(|| shape("`entities` must be an array".into()))?;
            for (index, entry) in array.iter().enumerate() {
                records.push(classify(meta.id, index, entry, path)?);
                if let (Some(id), Some(object)) = (records[index].declared_id(), entry.as_object())
                {
                    let expansion = RawExpansion {
                        derived: object_field(object, "derived", index, path)?,
                        edges: array_field(object, "edges", index, path)?,
                    };
                    if !expansion.derived.is_empty() || !expansion.edges.is_empty() {
                        expansions.insert(id, expansion);
                    }
                }
            }
        }
        check_duplicate_targets(meta.id, &records)?;

        Ok(Document {
            path: path.to_path_buf(),
            proto_version,
            meta,
            primary_frame,
            required_components,
            records,
            layers,
            settings: obj.get("settings").cloned(),
            expansions,
        })
    }
}

fn array_field(
    object: &Map<String, Value>,
    field: &str,
    index: usize,
    path: &Path,
) -> Result<Vec<Value>, LoadError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(_) => Err(LoadError::Shape {
            path: path.to_path_buf(),
            message: format!("record #{index}: `{field}` must be an array"),
        }),
    }
}

fn object_field(
    object: &Map<String, Value>,
    field: &str,
    index: usize,
    path: &Path,
) -> Result<Map<String, Value>, LoadError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err(LoadError::Shape {
            path: path.to_path_buf(),
            message: format!("record #{index}: `{field}` must be an object keyed by segment"),
        }),
    }
}

/// `proto_version` is semver in full. Component schemas evolve additively within
/// a major version, but 0.x has no such promise between minors, and every minor
/// so far has proved it: v0.3 changed what a tiled region with no `flow` means,
/// v0.4 renamed the baked flag and retired bare-key queue paths, v0.5 replaced
/// `access_side` with the access component, v0.6 turned `origin_mm` into
/// `origin` and a rack's `boards` into relay levels — which an ignore-unknown
/// rule would have swallowed, loading a rack with no shelves — and v0.7 spells
/// every derived identity as its address and turns a relay level's `positions`
/// into two-number offsets, so a v0.6 document's one-number positions would read
/// as half a coordinate. So this implementation reads the current minor — the
/// `0.7` below, which moves when the crate does — and refuses every other rather
/// than guess (SPEC §9).
fn check_proto_version(path: &Path, found: &str) -> Result<(), LoadError> {
    let mut parts = found.split('.');
    let major = parts.next().and_then(|p| p.parse::<u32>().ok());
    let minor = parts.next().and_then(|p| p.parse::<u32>().ok());
    let patch = parts.next().and_then(|p| p.parse::<u32>().ok());
    match (major, minor, patch, parts.next()) {
        (Some(0), Some(7), Some(_), None) => Ok(()),
        _ => Err(LoadError::ProtoVersion {
            path: path.to_path_buf(),
            found: found.to_string(),
        }),
    }
}

/// Read one entry's kind off its shape (SPEC §2).
fn classify(doc: DocId, index: usize, entry: &Value, path: &Path) -> Result<Record, LoadError> {
    let shape = |message: String| LoadError::Shape {
        path: path.to_path_buf(),
        message,
    };
    let obj = entry
        .as_object()
        .ok_or_else(|| shape(format!("record #{index} is not a JSON object")))?;

    // `id` states an identity, and both arms are spellable: a UUID names a
    // declared entity, an address names a derived one (SPEC §3). Which arms a
    // record kind accepts is decided below, where the kind is known.
    let id: Option<EntityId> = match obj.get("id") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| shape(format!("record #{index}: `id` does not parse: {e}")))?,
        ),
    };
    let derivation: Option<DerivedEntityId> = match obj.get("derivation") {
        None | Some(Value::Null) => None,
        Some(v) => {
            // A record stands in no template, but the schema shares one key
            // definition between a record's address and a template's, so a
            // parameter passes it here, and is refused by name.
            if let Some(placeholder) = v.get("key").and_then(placeholders::describe) {
                return Err(shape(format!(
                    "record #{index}: `derivation.key` still holds {placeholder}, which only a stamp can resolve"
                )));
            }
            let raw: RawDerivation = serde_json::from_value(v.clone())
                .map_err(|e| shape(format!("record #{index}: `derivation` does not parse: {e}")))?;
            Some(raw.parse()?)
        }
    };
    let is_patch = obj.get("patch").and_then(Value::as_bool).unwrap_or(false);
    let is_suppress = obj
        .get("suppress")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let components = read_components(index, obj, path)?;
    let expect: Option<Expect> = match obj.get("expect") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| shape(format!("record #{index}: `expect` does not parse: {e}")))?,
        ),
    };
    let name = obj.get("name").and_then(Value::as_str).map(str::to_string);

    if is_suppress && !components.is_empty() {
        return Err(LoadError::SuppressWithComponents { doc, index });
    }

    match (id, derivation, is_patch, is_suppress) {
        (_, Some(address), _, true) => Ok(Record::Suppression(Suppression {
            index,
            target: address.into(),
            expect,
        })),
        (Some(id), None, _, true) => Ok(Record::Suppression(Suppression {
            index,
            target: id,
            expect,
        })),
        (stated_id, Some(address), _, false) => {
            // The one record shape that folds in after the retired-component
            // scan has run: an override lands its components on an already
            // expanded entity, so this is where `exclusion` must be refused
            // for it. Declarations and patches merge before expansion and the
            // scan there catches them.
            if components.contains_key("exclusion") {
                return Err(LoadError::ExclusionComponentRemoved {
                    entity: address.into(),
                });
            }
            Ok(Record::Override(Override {
                doc,
                index,
                stated_id,
                address,
                expect,
                components,
            }))
        }
        (Some(id), None, patch, false) => Ok({
            let declaration = Declaration {
                index,
                id: declares(doc, index, id)?,
                name,
                components,
            };
            if patch {
                Record::Patch(declaration)
            } else {
                Record::Declaration(declaration)
            }
        }),
        (None, None, _, _) => Err(LoadError::RecordWithoutTarget { doc, index }),
    }
}

/// The identity a declaration or a patch may carry: a UUID, because writing an
/// entity down is what mints its identity. An address in that position would say
/// a generator produced the entity, which is the one thing a hand-written
/// declaration cannot be — and it would be reachable by an override anyway
/// (SPEC §3).
fn declares(doc: DocId, index: usize, id: EntityId) -> Result<DeclaredEntityId, LoadError> {
    id.as_declared().ok_or(LoadError::DeclaredIdIsAnAddress {
        doc,
        index,
        address: id.to_string(),
    })
}

fn read_components(
    index: usize,
    obj: &Map<String, Value>,
    path: &Path,
) -> Result<BTreeMap<String, Value>, LoadError> {
    let mut out = BTreeMap::new();
    match obj.get("components") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (name, value) in map {
                // `$comment` sits among components as freely as it sits inside
                // one, and is not a component.
                if name.starts_with('$') {
                    continue;
                }
                out.insert(name.clone(), value.clone());
            }
        }
        Some(_) => {
            return Err(LoadError::Shape {
                path: path.to_path_buf(),
                message: format!("record #{index}: `components` must be an object"),
            })
        }
    }
    Ok(out)
}

/// At most one record per resolved target per document (SPEC §2).
fn check_duplicate_targets(doc: DocId, records: &[Record]) -> Result<(), LoadError> {
    let mut seen: BTreeMap<EntityId, usize> = BTreeMap::new();
    for record in records {
        let target = record.target();
        if let Some(&first) = seen.get(&target) {
            return Err(LoadError::DuplicateTarget {
                doc,
                first,
                second: record.index(),
                target: target.to_string(),
            });
        }
        seen.insert(target, record.index());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expanded;
    use crate::pipeline::derive;
    use crate::pipeline::layers::LayerSet;
    use std::collections::BTreeSet;
    use serde_json::json;

    fn parse(entities: Value) -> Result<Document, LoadError> {
        let doc = json!({
            "proto_version": "0.7.0",
            "doc": { "id": "da000000-0000-4000-8000-000000000001", "kind": "world" },
            "entities": entities
        });
        Document::from_value(Path::new("<test>"), &doc)
    }

    #[test]
    fn shape_alone_decides_the_record_kind() {
        let d = parse(json!([
            { "id": "a0000000-0000-4000-8000-000000000001", "components": { "cell": {} } },
            { "id": "a0000000-0000-4000-8000-000000000002", "patch": true,
              "components": { "flow": { "rule": "two_way" } } },
            { "derivation": { "generator": "a0000000-0000-4000-8000-000000000001",
                              "key": "cell:1,7" },
              "components": { "cell": { "blocked": true } } },
            { "derivation": { "generator": "a0000000-0000-4000-8000-000000000001",
                              "key": "cell:2,7" }, "suppress": true },
            { "id": "a0000000-0000-4000-8000-000000000003", "suppress": true }
        ]))
        .unwrap();
        let kinds: Vec<&str> = d.records.iter().map(Record::kind_name).collect();
        assert_eq!(
            kinds,
            [
                "declaration",
                "patch",
                "override",
                "suppression",
                "suppression"
            ]
        );
    }

    #[test]
    fn suppress_with_components_is_refused() {
        let err = parse(json!([
            { "derivation": { "generator": "a0000000-0000-4000-8000-000000000001",
                              "key": "cell:1,7" },
              "suppress": true, "components": { "cell": {} } }
        ]))
        .unwrap_err();
        assert!(
            matches!(err, LoadError::SuppressWithComponents { index: 0, .. }),
            "{err}"
        );
    }

    #[test]
    fn a_record_addressing_nothing_is_refused() {
        let err = parse(json!([{ "components": { "cell": {} } }])).unwrap_err();
        assert!(
            matches!(err, LoadError::RecordWithoutTarget { index: 0, .. }),
            "{err}"
        );
    }

    /// Overrides fold in after expansion's retired-component scan has run, so
    /// the parse is where an override smuggling `exclusion` onto a derived
    /// entity gets stopped. It would remove nothing — it never did, subtraction
    /// always ran first — but a retired name must refuse loudly everywhere,
    /// not ride along inert.
    #[test]
    fn an_override_carrying_the_retired_exclusion_component_is_refused() {
        let err = parse(json!([
            { "derivation": { "generator": "a0000000-0000-4000-8000-000000000001",
                              "key": "cell:1,7" },
              "components": { "exclusion": {} } }
        ]))
        .unwrap_err();
        assert!(
            matches!(err, LoadError::ExclusionComponentRemoved { ref entity }
                     if entity.to_string().ends_with("cell:1,7")),
            "{err}"
        );
    }

    #[test]
    fn two_records_on_one_target_are_refused() {
        let err = parse(json!([
            { "id": "a0000000-0000-4000-8000-000000000001", "components": { "cell": {} } },
            { "id": "a0000000-0000-4000-8000-000000000001", "patch": true,
              "components": { "cell": { "blocked": true } } }
        ]))
        .unwrap_err();
        assert!(
            matches!(
                err,
                LoadError::DuplicateTarget {
                    first: 0,
                    second: 1,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn an_override_and_a_suppression_of_one_address_are_duplicates() {
        let err = parse(json!([
            { "derivation": { "generator": "a0000000-0000-4000-8000-000000000001",
                              "key": "cell:1,7" },
              "components": { "cell": { "blocked": true } } },
            { "derivation": { "generator": "a0000000-0000-4000-8000-000000000001",
                              "key": "cell:1,7" }, "suppress": true }
        ]))
        .unwrap_err();
        assert!(matches!(err, LoadError::DuplicateTarget { .. }), "{err}");
    }

    #[test]
    fn an_unknown_required_component_refuses_the_document() {
        let doc = json!({
            "proto_version": "0.7.0",
            "doc": { "id": "da000000-0000-4000-8000-000000000001" },
            "required_components": ["frame", "conveyor_mechanics"],
            "entities": []
        });
        let err = Document::from_value(Path::new("<test>"), &doc).unwrap_err();
        assert!(
            matches!(err, LoadError::UnknownRequiredComponent { ref component, .. }
                     if component == "conveyor_mechanics"),
            "{err}"
        );
    }

    /// A consumer accepts the current minor and refuses every other (SPEC §9),
    /// in both directions: a minor from the future says things this reader has
    /// no rules for, and 0.6 says a relay level's `positions` as one number
    /// each, which this reader would half-read into shelves with no depth.
    #[test]
    fn every_minor_but_the_current_one_is_refused() {
        for version in ["0.8.0", "0.6.0", "0.3.0", "1.0.0"] {
            let doc = json!({
                "proto_version": version,
                "doc": { "id": "da000000-0000-4000-8000-000000000001" },
                "entities": []
            });
            assert!(
                matches!(
                    Document::from_value(Path::new("<test>"), &doc),
                    Err(LoadError::ProtoVersion { .. })
                ),
                "{version} should be refused"
            );
        }
        let current = json!({
            "proto_version": "0.7.4",
            "doc": { "id": "da000000-0000-4000-8000-000000000001" },
            "entities": []
        });
        assert!(Document::from_value(Path::new("<test>"), &current).is_ok());
    }
    #[test]
    fn an_assembled_documents_sources_survive_the_expanded_form() {
        let doc = json!({
            "id": "da000000-0000-4000-8000-000000000001",
            "kind": "world",
            "assembled_from": [
                { "scope": "customer/acme/site/dfw6/map", "changeset": 41, "tag": "run-41" },
                { "scope": "module/go-catalog", "changeset": 0 }
            ]
        });
        let document = json!({ "proto_version": "0.7.0", "doc": doc, "entities": [] });
        let scenario = Document::from_value(Path::new("<test>"), &document).unwrap();
        assert_eq!(
            scenario.meta.assembled_from[1],
            SourceScope { scope: "module/go-catalog".to_string(), changeset: 0, tag: None }
        );
        let set = LayerSet { scenario, layers: Vec::new() };
        let world = derive(&set).unwrap();
        assert_eq!(expanded::emit(&set, &world)["doc"], doc);
    }

    #[test]
    fn a_malformed_source_refuses_the_document() {
        for source in [
            json!({ "changeset": 41 }),
            json!({ "scope": "module/go-catalog", "changeset": "41" }),
            json!({ "scope": "module/go-catalog", "changeset": -1 }),
        ] {
            let document = json!({
                "proto_version": "0.7.0",
                "doc": { "id": "da000000-0000-4000-8000-000000000001", "assembled_from": [source] },
                "entities": []
            });
            let err = Document::from_value(Path::new("<test>"), &document).unwrap_err();
            assert!(matches!(err, LoadError::Shape { .. }), "{err}");
        }
    }

    #[test]
    fn the_doc_block_the_schema_states_is_the_one_read() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application/schema/document.json");
        let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let names = |object: &Value, keyword: &str| -> BTreeSet<String> {
            match &object[keyword] {
                Value::Object(map) => map.keys().filter(|key| !key.starts_with('$')).cloned().collect(),
                Value::Array(items) => items.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                _ => BTreeSet::new(),
            }
        };
        let read = [("doc_meta", schemars::schema_for!(DocMeta)), ("source_scope", schemars::schema_for!(SourceScope))];
        for (definition, derived) in read {
            let (stated, derived) = (&schema["$defs"][definition], serde_json::to_value(derived).unwrap());
            assert_eq!(names(stated, "properties"), names(&derived, "properties"), "the keys of `{definition}`");
            assert_eq!(names(stated, "required"), names(&derived, "required"), "the keys `{definition}` requires");
        }
    }

}

#[cfg(test)]
mod retired_field_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_retired_baked_flag_name_is_refused_rather_than_ignored() {
        let doc = json!({
            "proto_version": "0.7.0",
            "doc": { "id": "3cbd200e-580a-40d8-b5ee-edf406aa1c88" },
            "materialized": true,
            "entities": []
        });
        let err = Document::from_value(Path::new("<test>"), &doc).unwrap_err();
        assert!(
            matches!(
                err,
                LoadError::RetiredField {
                    field: "materialized",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn the_current_spelling_parses() {
        let doc = json!({
            "proto_version": "0.7.0",
            "doc": { "id": "3cbd200e-580a-40d8-b5ee-edf406aa1c88" },
            "entities": [
                { "id": "272542c8-2ea2-441f-8807-8a37aafc8f10",
                  "components": { "cell": {} },
                  "derived": { "cell:0,0": { "components": {} } },
                  "edges": [] }
            ]
        });
        let parsed = Document::from_value(Path::new("<test>"), &doc).unwrap();
        assert_eq!(parsed.expansions.len(), 1);
    }
}
