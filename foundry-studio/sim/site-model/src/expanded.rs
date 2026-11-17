//! The expanded form: a document carrying, inline, the entities its own
//! generators derive.
//!
//! Because expansion is deterministic, an inline expansion is a *cache* and
//! never a second source of truth. The records stay authoritative and stay
//! readable; a consumer that expands may re-derive and check the cache byte for
//! byte, and a consumer that does not may read the cache and skip the work. That
//! is the whole reason the shape nests: a derived entity hangs under the record
//! whose generator started its chain, keyed by its own segment, so provenance is
//! the position in the tree rather than a field repeating what the tree already
//! says. And because a derived node is not a record, none of §2's record rules
//! apply to it — which is what lets the authored half and the derived half share
//! one array instead of needing separate containers.
//!
//! A stale cache is refused. Not repaired, not preferred, not silently ignored:
//! a document whose inline expansion disagrees with what its own records derive
//! is a document that means two things, and the load fails naming the first
//! address where the two disagree.

use crate::canonical;
use crate::document::{Document, RawExpansion, Record};
use crate::entity::Entity;
use crate::error::LoadError;
use crate::ids::{DeclaredEntityId, DocId, EntityId};
use crate::pipeline::graph::Edge;
use crate::pipeline::layers::LayerSet;
use crate::pipeline::World;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// Build the expanded form of a scenario and the world it derives into.
///
/// The `entities` array carries the authored records — merged declarations, then
/// overrides, then the suppressions somebody wrote — each stating its target
/// once. A record whose entity generates anything gains a `derived` subtree, and
/// a record or node that owns arcs gains an `edges` array. Exclusion-driven
/// removals are *not* written back as suppression records: re-derivation reads
/// each tiling's own `exclusions` from source and subtracts them again, so a
/// tombstone would be a second way of saying what the source already says.
pub fn emit(set: &LayerSet, world: &World) -> Value {
    let scenario = &set.scenario;
    let mut document = Map::new();
    document.insert("proto_version".into(), json!(scenario.proto_version));
    document.insert(
        "doc".into(),
        serde_json::to_value(&scenario.meta).expect("document metadata serializes"),
    );
    if let Some(frame) = world.frames.primary() {
        document.insert("primary_frame".into(), json!(frame));
    }
    // The expanded form stands alone, so it must state everything its layers
    // required between them.
    let required: BTreeSet<&str> = set
        .documents()
        .flat_map(|d| d.required_components.iter().map(String::as_str))
        .collect();
    if !required.is_empty() {
        document.insert("required_components".into(), json!(required));
    }
    if let Some(settings) = &scenario.settings {
        document.insert("settings".into(), settings.clone());
    }
    document.insert("entities".into(), Value::Array(records(world)));
    Value::Object(document)
}

/// One derived entity and whatever hangs beneath it.
#[derive(Default)]
struct Subtree<'a> {
    entity: Option<&'a Entity>,
    children: BTreeMap<String, Subtree<'a>>,
}

fn subtrees(world: &World) -> BTreeMap<DeclaredEntityId, Subtree<'_>> {
    let mut roots: BTreeMap<DeclaredEntityId, Subtree> = BTreeMap::new();
    for entity in world.entities.values() {
        let Some(address) = entity.id.as_derived() else {
            continue;
        };
        let mut cursor = roots.entry(address.root()).or_default();
        for segment in address.key().segments() {
            cursor = cursor.children.entry(segment.to_string()).or_default();
        }
        cursor.entity = Some(entity);
    }
    roots
}

fn edges_by_owner(world: &World) -> BTreeMap<EntityId, Vec<&Edge>> {
    let mut out: BTreeMap<EntityId, Vec<&Edge>> = BTreeMap::new();
    for edge in &world.edges {
        out.entry(edge.owner.clone()).or_default().push(edge);
    }
    out
}

fn records(world: &World) -> Vec<Value> {
    let mut roots = subtrees(world);
    let owned = edges_by_owner(world);
    let mut out = Vec::new();

    for entity in world.declared.values() {
        let mut record = Map::new();
        record.insert("id".into(), json!(entity.id));
        if let Some(name) = &entity.name {
            record.insert("name".into(), json!(name));
        }
        record.insert("components".into(), components_of(entity));
        if let Some(subtree) = entity.id.as_declared().and_then(|id| roots.remove(&id)) {
            if let Some(children) = emit_children(&subtree, &owned) {
                record.insert("derived".into(), children);
            }
        }
        if let Some(edges) = owned.get(&entity.id) {
            record.insert(
                "edges".into(),
                Value::Array(edges.iter().map(edge).collect()),
            );
        }
        out.push(Value::Object(record));
    }

    for (target, components) in &world.overrides {
        let mut record = Map::new();
        addressed(&mut record, target);
        if let Some(pin) = world.expects.get(target) {
            record.insert("expect".into(), json!(pin));
        }
        record.insert("components".into(), json!(components));
        out.push(Value::Object(record));
    }

    for suppressed in &world.suppressions {
        // Only what somebody wrote. An exclusion is already in the source, in
        // its tiling's own `exclusions` list, and re-derivation subtracts it
        // again.
        if suppressed.excluded_by.is_some() {
            continue;
        }
        let mut record = Map::new();
        addressed(&mut record, &suppressed.target);
        if let Some(pin) = world.expects.get(&suppressed.target) {
            record.insert("expect".into(), json!(pin));
        }
        record.insert("suppress".into(), json!(true));
        out.push(Value::Object(record));
    }
    out
}

/// How a record names the entity it acts on. The identity decides the field: a
/// UUID is what `id` holds, and an address is what `derivation` spells out in
/// its two parts — the same address the entity is named by, written the way a
/// person authors one (SPEC §3).
fn addressed(record: &mut Map<String, Value>, target: &EntityId) {
    match target {
        EntityId::Declared(id) => {
            record.insert("id".into(), json!(id));
        }
        EntityId::Derived(address) => {
            record.insert(
                "derivation".into(),
                json!({ "generator": address.root(), "key": address.key().to_string() }),
            );
        }
    }
}

fn emit_children(node: &Subtree<'_>, owned: &BTreeMap<EntityId, Vec<&Edge>>) -> Option<Value> {
    if node.children.is_empty() {
        return None;
    }
    let mut map = Map::new();
    for (segment, child) in &node.children {
        map.insert(segment.clone(), emit_node(child, owned));
    }
    Some(Value::Object(map))
}

fn emit_node(node: &Subtree<'_>, owned: &BTreeMap<EntityId, Vec<&Edge>>) -> Value {
    let mut out = Map::new();
    if let Some(entity) = node.entity {
        // The id is the address the tree above it already spells, written out
        // flat so a reader has it in one piece — and so a cache that disagrees
        // with the position it sits at has one more place to be caught.
        out.insert("id".into(), json!(entity.id));
        if let Some(name) = &entity.name {
            out.insert("name".into(), json!(name));
        }
        out.insert("components".into(), components_of(entity));
        if let Some(edges) = owned.get(&entity.id) {
            out.insert(
                "edges".into(),
                Value::Array(edges.iter().map(edge).collect()),
            );
        }
    }
    if let Some(children) = emit_children(node, owned) {
        out.insert("derived".into(), children);
    }
    Value::Object(out)
}

fn components_of(entity: &Entity) -> Value {
    let mut map = Map::new();
    for (name, value) in &entity.components {
        map.insert(name.clone(), value.clone());
    }
    Value::Object(map)
}

fn edge(edge: &&Edge) -> Value {
    json!({
        "key": edge.key,
        "from": edge.from,
        "to": edge.to,
        "bidirectional": edge.bidirectional,
    })
}

// ---------------------------------------------------------------------------
// Cache verification
// ---------------------------------------------------------------------------

/// What a node says about itself, reduced to the one form both sides can be
/// compared in: the canonical projection (SPEC §9).
///
/// Comparing raw JSON would be wrong, not merely strict. A derived cell centre is
/// computed as `5750.0` and written to the canonical projection as `5750`, so a
/// cache that has been through a file is *equal* to the world it came from and
/// unequal as `serde_json::Value`. §9 already says what "the same" means for two
/// implementations, and it means the same thing for one implementation reading
/// back its own artifact.
#[derive(Debug, PartialEq)]
struct Snapshot {
    id: Option<EntityId>,
    /// Absent on a record's own entry: a record's components are the source, and
    /// the source is not something the cache gets to disagree with.
    components: Option<String>,
    edges: BTreeMap<String, String>,
}

/// A value in the projection two implementations are held to agree in.
fn projected(value: &Value) -> String {
    canonical::to_canonical_string(value)
}

/// Re-derive and compare: every document that carries an inline expansion has it
/// checked against what its own records produce, node for node.
///
/// All or nothing per document. A document that caches part of its expansion is
/// not partly trusted — the roots to check are every generator that document
/// declares *plus* every root it wrote a subtree for, so a missing subtree reads
/// as the divergence it is.
pub fn verify(set: &LayerSet, world: &World) -> Result<(), LoadError> {
    for document in set.documents() {
        if document.expansions.is_empty() {
            continue;
        }
        verify_document(document, world)?;
    }
    Ok(())
}

fn verify_document(document: &Document, world: &World) -> Result<(), LoadError> {
    let derived_roots: BTreeSet<DeclaredEntityId> = world
        .entities
        .values()
        .filter_map(|e| e.id.as_derived().map(|address| address.root()))
        .collect();
    let mut roots: BTreeSet<DeclaredEntityId> = document.expansions.keys().copied().collect();
    for record in &document.records {
        if let Record::Declaration(d) = record {
            if derived_roots.contains(&d.id)
                || world.edges.iter().any(|e| e.owner == EntityId::from(d.id))
            {
                roots.insert(d.id);
            }
        }
    }

    let mut cached: BTreeMap<String, Snapshot> = BTreeMap::new();
    for (root, expansion) in &document.expansions {
        read_cache(document.meta.id, *root, expansion, &mut cached)?;
    }
    let mut actual: BTreeMap<String, Snapshot> = BTreeMap::new();
    for root in &roots {
        read_world(*root, world, &mut actual);
    }

    let addresses: BTreeSet<&String> = cached.keys().chain(actual.keys()).collect();
    for address in addresses {
        let stale = |detail: String| LoadError::StaleExpansion {
            doc: document.meta.id,
            address: address.clone(),
            detail,
        };
        match (cached.get(address), actual.get(address)) {
            (Some(_), None) => {
                return Err(stale(
                    "the inline expansion has a node the records do not derive".into(),
                ))
            }
            (None, Some(_)) => {
                return Err(stale(
                    "the records derive a node the inline expansion is missing".into(),
                ))
            }
            (Some(want), Some(got)) => {
                if let Some(stated) = &want.id {
                    // A node may omit its id — the tree it hangs in already
                    // spells the address — but if it states one it has to be the
                    // address of the position it sits at.
                    if Some(stated) != got.id.as_ref() {
                        return Err(stale(format!(
                            "the inline expansion states id {stated}, the records derive {}",
                            got.id
                                .as_ref()
                                .map_or_else(|| "nothing".to_string(), |i| i.to_string())
                        )));
                    }
                }
                if want.components.is_some() && want.components != got.components {
                    return Err(stale("components differ".into()));
                }
                if want.edges != got.edges {
                    return Err(stale("edges differ".into()));
                }
            }
            (None, None) => unreachable!("the address came from one of the two maps"),
        }
    }
    Ok(())
}

fn read_cache(
    doc: DocId,
    root: DeclaredEntityId,
    expansion: &RawExpansion,
    out: &mut BTreeMap<String, Snapshot>,
) -> Result<(), LoadError> {
    out.insert(
        root.to_string(),
        Snapshot {
            id: Some(root.into()),
            components: None,
            edges: read_edges(doc, &root.to_string(), &expansion.edges)?,
        },
    );
    for (segment, node) in &expansion.derived {
        read_cache_node(doc, &root.to_string(), segment, node, out)?;
    }
    Ok(())
}

fn read_cache_node(
    doc: DocId,
    parent: &str,
    segment: &str,
    node: &Value,
    out: &mut BTreeMap<String, Snapshot>,
) -> Result<(), LoadError> {
    let address = format!("{parent}/{segment}");
    let stale = |detail: String| LoadError::StaleExpansion {
        doc,
        address: address.clone(),
        detail,
    };
    let object = node
        .as_object()
        .ok_or_else(|| stale("a derived node must be a JSON object".into()))?;

    let id: Option<EntityId> = match object.get("id") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            serde_json::from_value(value.clone())
                .map_err(|e| stale(format!("`id` is not an entity identity: {e}")))?,
        ),
    };
    let edges = match object.get("edges") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => return Err(stale("`edges` must be an array".into())),
    };
    out.insert(
        address.clone(),
        Snapshot {
            id,
            components: Some(projected(object.get("components").unwrap_or(&json!({})))),
            edges: read_edges(doc, &address, &edges)?,
        },
    );

    match object.get("derived") {
        None | Some(Value::Null) => {}
        Some(Value::Object(children)) => {
            for (child_segment, child) in children {
                read_cache_node(doc, &address, child_segment, child, out)?;
            }
        }
        Some(_) => return Err(stale("`derived` must be an object keyed by segment".into())),
    }
    Ok(())
}

fn read_edges(
    doc: DocId,
    address: &str,
    edges: &[Value],
) -> Result<BTreeMap<String, String>, LoadError> {
    let mut out = BTreeMap::new();
    for entry in edges {
        let key =
            entry
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| LoadError::StaleExpansion {
                    doc,
                    address: address.to_string(),
                    detail: "an edge must state its `key`".into(),
                })?;
        out.insert(key.to_string(), projected(entry));
    }
    Ok(out)
}

fn read_world(root: DeclaredEntityId, world: &World, out: &mut BTreeMap<String, Snapshot>) {
    let owned = |id: &EntityId| -> BTreeMap<String, String> {
        world
            .edges
            .iter()
            .filter(|e| &e.owner == id)
            .map(|e| (e.key.clone(), projected(&edge(&e))))
            .collect()
    };
    let root_id = EntityId::from(root);
    out.insert(
        root.to_string(),
        Snapshot {
            id: Some(root_id.clone()),
            components: None,
            edges: owned(&root_id),
        },
    );
    for entity in world.entities.values() {
        if entity.id.as_derived().map(|a| a.root()) != Some(root) {
            continue;
        }
        // The address a node is filed under is the identity, spelled the same
        // way the cache spells the path down to it.
        out.insert(
            entity.id.to_string(),
            Snapshot {
                id: Some(entity.id.clone()),
                components: Some(projected(&components_of(entity))),
                edges: owned(&entity.id),
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Canonical projection
// ---------------------------------------------------------------------------

/// The canonical projection of an expanded document (SPEC §9): every array whose
/// order is bookkeeping rather than data is sorted, then the value is written in
/// the one spelling both implementations must agree on.
///
/// Only this function knows which arrays those are. `queue.path` and a relay
/// level's `positions` are data and keep their order; `entities` and `edges` are
/// sets that happen to be written as lists. `derived` needs no help — it is a
/// map, and the canonical writer already orders object keys.
pub fn to_canonical_string(document: &Value) -> String {
    let mut value = document.clone();
    sort_bookkeeping(&mut value);
    canonical::to_canonical_string(&value)
}

fn sort_bookkeeping(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(items)) = object.get_mut("entities") {
        items.sort_by_key(record_sort_key);
        for item in items.iter_mut() {
            sort_bookkeeping(item);
        }
    }
    if let Some(Value::Object(children)) = object.get_mut("derived") {
        // A map needs no sorting: the canonical writer orders object keys, which
        // is the whole reason the shape is a map.
        for child in children.values_mut() {
            sort_bookkeeping(child);
        }
    }
    if let Some(Value::Array(items)) = object.get_mut("edges") {
        items.sort_by_key(edge_sort_key);
    }
}

/// A record sorts by the target it addresses: its id when it has one, otherwise
/// its derivation address. The leading digit keeps the three record kinds in
/// stable blocks even when a suppression and a declaration share nothing else.
fn record_sort_key(record: &Value) -> String {
    let id = record.get("id").and_then(Value::as_str);
    let derivation = record.get("derivation").map(|d| {
        format!(
            "{}/{}",
            d.get("generator").and_then(Value::as_str).unwrap_or(""),
            d.get("key").and_then(Value::as_str).unwrap_or("")
        )
    });
    let suppressed = record
        .get("suppress")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rank = if suppressed {
        2
    } else if id.is_some() {
        0
    } else if derivation.is_some() {
        1
    } else {
        3
    };
    format!(
        "{rank}:{}",
        id.map(str::to_string).or(derivation).unwrap_or_default()
    )
}

fn edge_sort_key(edge: &Value) -> String {
    edge.get("key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
