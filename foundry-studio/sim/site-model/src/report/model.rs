//! The view model: a loaded world reduced to what a reader is shown, in the
//! shape the page's script reads it back in.
//!
//! Every judgement about *what* the report says lives here rather than in the
//! script — which entities exist, what derived what, which field of which
//! component points where — so the script has nothing left to decide and only
//! draws. Two things shape the layout of these structs, and both come from
//! scale: entities are addressed by their position in one array instead of by
//! UUID, because a world of 21,000 entities repeats each 36-byte identifier
//! everywhere it is mentioned; and the arrays that repeat once per entity use
//! one-letter field names, while everything bounded — the summary, the
//! documents, the census — is spelled out.
//!
//! Counts here are read off the same [`World`] fields the text output prints
//! from, never recomputed a second way, so the two cannot disagree.

use crate::entity::Entity;
use crate::ids::{DeclaredEntityId, EntityId};
use crate::pipeline::layers::LayerSet;
use crate::pipeline::World;
use crate::references;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

/// Everything one report shows.
#[derive(Debug, Serialize)]
pub struct Page {
    pub scenario: ScenarioView,
    pub summary: Summary,
    pub documents: Vec<DocumentView>,
    /// How many entities carry each component name.
    pub census: Vec<(String, usize)>,
    /// Warnings and errors as the text output prints them, word for word.
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    /// `component.field` for every reference kind any entity uses, interned:
    /// an entity names one by position rather than by spelling it out.
    #[serde(rename = "referenceKinds")]
    pub reference_kinds: Vec<String>,
    pub entities: Vec<EntityView>,
    /// The UUIDs of entities the world no longer holds — a suppressed
    /// generator, an entity an exclusion removed — addressed by continuing the
    /// entity array's index space past its end.
    #[serde(rename = "absentIds")]
    pub absent_ids: Vec<String>,
    /// Indices of the declared entities that survived, in identity order: the
    /// top level of the tree.
    pub declared: Vec<usize>,
    pub generators: Vec<GeneratorView>,
    /// Flat groups of four — owner, from, to, 1 when the arc is bidirectional —
    /// because 30,000 arcs written as objects is a megabyte of field names.
    pub edges: Vec<usize>,
    pub suppressions: SuppressionsView,
}

/// What was loaded, named the way the documents name it.
#[derive(Debug, Serialize)]
pub struct ScenarioView {
    /// The path as the command line gave it.
    pub input: String,
    pub name: Option<String>,
    #[serde(rename = "protoVersion")]
    pub proto_version: String,
    #[serde(rename = "primaryFrame")]
    pub primary_frame: Option<usize>,
    pub settings: Option<Value>,
    /// The version of `site-model-check` that wrote the report. There is no
    /// timestamp on purpose: two runs over one scenario produce byte-identical
    /// reports, so a diff between them is a change in the world.
    pub tool: String,
}

/// The numbers the text output prints, in the order it prints them.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    /// The layers the scenario composes — the scenario document itself is not
    /// one of them, which is what the text output's `layers:` line counts.
    pub layers: usize,
    pub entities: usize,
    pub declared: usize,
    pub derived: usize,
    pub edges: usize,
    #[serde(rename = "flowArcs")]
    pub flow_arcs: usize,
    #[serde(rename = "stitchLinks")]
    pub stitch_links: usize,
    pub suppressed: usize,
    pub generators: usize,
    pub components: usize,
}

/// One document taking part in the load.
#[derive(Debug, Serialize)]
pub struct DocumentView {
    pub path: String,
    pub name: Option<String>,
    pub id: String,
    pub kind: Option<String>,
    /// `layer` or `scenario` — the scenario is the last word, so it is worth
    /// telling apart from the documents it composes.
    pub role: &'static str,
    pub required: Vec<String>,
    /// How many records of each kind, by the word the loader calls the kind.
    pub records: Vec<(&'static str, usize)>,
}

/// One entity, with the components that give it meaning.
#[derive(Debug, Serialize)]
pub struct EntityView {
    #[serde(rename = "i")]
    pub id: String,
    #[serde(rename = "n", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "c")]
    pub components: BTreeMap<String, Value>,
    /// Authored rather than derived. Absent means derived, which is the common
    /// case by two orders of magnitude.
    #[serde(rename = "d", skip_serializing_if = "is_false")]
    pub declared: bool,
    /// The entity one step up the chain, which is not always the entity whose
    /// subtree this one hangs in.
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    /// Reference kind and target, alternating.
    #[serde(rename = "r", skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<usize>,
}

/// One declared entity's expansion: how many entities it derived, of what
/// kinds, and the chain each one hangs on.
#[derive(Debug, Serialize)]
pub struct GeneratorView {
    pub id: String,
    pub name: Option<String>,
    /// The generator's own entity, absent when a suppression removed it.
    pub entity: Option<usize>,
    /// Surviving derived entities, straight from `World::counts`.
    pub count: usize,
    /// Component membership across derived entities; counts may overlap.
    pub components: Vec<(String, usize)>,
    pub children: Vec<Node>,
}

/// One step of a derivation chain: the key segment, whatever entity the chain
/// reaches at that step, and the steps below it. A step can carry no entity —
/// a stamp's `instance:0` names a level of the chain whether or not anything
/// stands at it.
#[derive(Debug, Serialize)]
pub struct Node {
    #[serde(rename = "s")]
    pub segment: String,
    #[serde(rename = "e", skip_serializing_if = "Option::is_none")]
    pub entity: Option<usize>,
    #[serde(rename = "c", skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

/// What was deleted, and by what.
///
/// The split is the one SPEC §5 draws: somebody wrote a suppression record, or
/// a tiling carved a hole out of its own cells and every cell inside it went.
/// Grouping the second kind under the tiling that carved it is the difference
/// between a finding — this cell_grid dropped 312 of its cells — and 6,428
/// lines to scroll past.
#[derive(Debug, Serialize)]
pub struct SuppressionsView {
    pub total: usize,
    pub authored: Vec<SuppressedView>,
    #[serde(rename = "byExclusion")]
    pub by_exclusion: Vec<ExclusionView>,
}

#[derive(Debug, Serialize)]
pub struct ExclusionView {
    /// The tiling entity whose `exclusions` removed these cells, as an index
    /// into the entity array.
    #[serde(rename = "x")]
    pub exclusion: usize,
    pub count: usize,
    pub items: Vec<SuppressedView>,
}

/// One deleted target: the generator it was derived from plus the key that
/// addressed it, or — when a record suppressed a declared entity — just the
/// entity, which is no longer anywhere to be linked to.
#[derive(Debug, Serialize)]
pub struct SuppressedView {
    #[serde(rename = "a")]
    pub anchor: usize,
    #[serde(rename = "k", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

fn is_false(flag: &bool) -> bool {
    !*flag
}

/// Reduce a loaded world to the page's view of it.
pub fn build(input: &Path, set: &LayerSet, world: &World) -> Page {
    let index = Index::over(world);
    let mut absent = AbsentIds::default();
    let mut reference_kinds = KindTable::default();

    let entities: Vec<EntityView> = world
        .entities
        .values()
        .map(|entity| EntityView {
            id: entity.id.to_string(),
            name: entity.name.clone(),
            components: entity.components.clone(),
            declared: !entity.is_derived(),
            parent: entity.parent.as_ref().and_then(|id| index.of(id)),
            references: reference_row(entity, &index, &mut reference_kinds),
        })
        .collect();

    // The top of the tree, in name order. A name is not identity and nothing
    // may be looked up by one (SPEC §2) — but a person browsing 327 declared
    // entities reads the names, and identity order puts "rack row: zone 8 left
    // face" between a sector and a clip for no reason a reader can see.
    let mut declared: Vec<usize> = world
        .entities
        .values()
        .filter(|e| !e.is_derived())
        .filter_map(|e| index.of(&e.id))
        .collect();
    declared.sort_by(|a, b| {
        let key = |at: &usize| (entities[*at].name.is_none(), entities[*at].name.clone());
        key(a).cmp(&key(b)).then(a.cmp(b))
    });

    let generators = generators(world, &index);
    let bidirectional = world.edges.iter().filter(|e| e.bidirectional).count();

    Page {
        scenario: ScenarioView {
            input: input.display().to_string(),
            name: set.scenario.meta.name.clone(),
            proto_version: set.scenario.proto_version.clone(),
            primary_frame: world.frames.primary().and_then(|id| index.of(id)),
            settings: set.scenario.settings.clone(),
            tool: format!("site-model-check {}", env!("CARGO_PKG_VERSION")),
        },
        summary: Summary {
            errors: 0,
            warnings: world.warnings.len(),
            layers: set.layers.len(),
            entities: world.entities.len(),
            declared: world.entities.len() - world.derived_count(),
            derived: world.derived_count(),
            edges: world.edges.len(),
            flow_arcs: world.edges.len() - bidirectional,
            stitch_links: bidirectional,
            suppressed: world.suppressions.len(),
            generators: world.counts.len(),
            components: world.component_census().len(),
        },
        documents: documents(set),
        census: world.component_census().into_iter().collect(),
        warnings: world.warnings.iter().map(ToString::to_string).collect(),
        errors: Vec::new(),
        reference_kinds: reference_kinds.names,
        entities,
        declared,
        generators,
        edges: edges(world, &index),
        suppressions: suppressions(world, &index, &mut absent),
        absent_ids: absent.uuids,
    }
}

/// Everything the report shows about a scenario, as one JSON value.
pub fn to_json(page: &Page) -> String {
    serde_json::to_string(page).expect("the view model is plain data and serializes")
}

/// The page for a scenario that refused to load: the refusal and its causes,
/// and nothing else, because there is no world to describe.
pub fn failure(input: &Path, errors: Vec<String>) -> Page {
    Page {
        scenario: ScenarioView {
            input: input.display().to_string(),
            name: None,
            proto_version: String::new(),
            primary_frame: None,
            settings: None,
            tool: format!("site-model-check {}", env!("CARGO_PKG_VERSION")),
        },
        summary: Summary {
            errors: 1,
            warnings: 0,
            layers: 0,
            entities: 0,
            declared: 0,
            derived: 0,
            edges: 0,
            flow_arcs: 0,
            stitch_links: 0,
            suppressed: 0,
            generators: 0,
            components: 0,
        },
        documents: Vec::new(),
        census: Vec::new(),
        warnings: Vec::new(),
        errors,
        reference_kinds: Vec::new(),
        entities: Vec::new(),
        absent_ids: Vec::new(),
        declared: Vec::new(),
        generators: Vec::new(),
        edges: Vec::new(),
        suppressions: SuppressionsView {
            total: 0,
            authored: Vec::new(),
            by_exclusion: Vec::new(),
        },
    }
}

/// Where each entity sits in the array the page addresses entities by.
struct Index(BTreeMap<EntityId, usize>);

impl Index {
    fn over(world: &World) -> Index {
        Index(
            world
                .entities
                .keys()
                .enumerate()
                .map(|(position, id)| (id.clone(), position))
                .collect(),
        )
    }

    fn of(&self, id: &EntityId) -> Option<usize> {
        self.0.get(id).copied()
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Identities the report has to name that the entity array does not hold.
#[derive(Default)]
struct AbsentIds {
    uuids: Vec<String>,
    positions: BTreeMap<EntityId, usize>,
}

impl AbsentIds {
    /// The index to address `id` by: its place in the entity array when it is
    /// still in the world, or a place past the end of that array when it is
    /// not.
    fn resolve(&mut self, id: &EntityId, index: &Index) -> usize {
        if let Some(position) = index.of(id) {
            return position;
        }
        if let Some(position) = self.positions.get(id) {
            return *position;
        }
        let position = index.len() + self.uuids.len();
        self.uuids.push(id.to_string());
        self.positions.insert(id.clone(), position);
        position
    }
}

/// The `component.field` spellings, interned in first-seen order.
#[derive(Default)]
struct KindTable {
    names: Vec<String>,
    positions: BTreeMap<String, usize>,
}

impl KindTable {
    fn resolve(&mut self, component: &str, field: &str) -> usize {
        let name = format!("{component}.{field}");
        if let Some(position) = self.positions.get(&name) {
            return *position;
        }
        let position = self.names.len();
        self.positions.insert(name.clone(), position);
        self.names.push(name);
        position
    }
}

/// One entity's references, as kind and target indices in pairs.
///
/// Both kinds of arrow are here — a UUID in a field and a cell a queue or a
/// stitch addresses — because both are things a reader follows. A target the
/// world no longer holds cannot be drawn as a link, and cannot occur either:
/// the load refuses a dangling reference and a queue path through a removed
/// cell (SPEC §2, §8 step 6).
fn reference_row(entity: &Entity, index: &Index, kinds: &mut KindTable) -> Vec<usize> {
    let mut row = Vec::new();
    let Ok(references) = references::all(entity) else {
        // Unreachable on a loaded world: a component that does not parse fails
        // the load long before a report is written.
        return row;
    };
    for reference in references {
        let Some(target) = index.of(&reference.target) else {
            continue;
        };
        row.push(kinds.resolve(reference.component, reference.field));
        row.push(target);
    }
    row
}

fn documents(set: &LayerSet) -> Vec<DocumentView> {
    let scenario = set.scenario.meta.id;
    set.documents()
        .map(|document| {
            let mut records: BTreeMap<&'static str, usize> = BTreeMap::new();
            for record in &document.records {
                *records.entry(record.kind_name()).or_default() += 1;
            }
            DocumentView {
                path: document.path.display().to_string(),
                name: document.meta.name.clone(),
                id: document.meta.id.to_string(),
                kind: document.meta.kind.clone(),
                role: if document.meta.id == scenario {
                    "scenario"
                } else {
                    "layer"
                },
                required: document.required_components.clone(),
                records: records.into_iter().collect(),
            }
        })
        .collect()
}

/// The derivation tree, one root per generator that produced anything.
///
/// Nodes hang by their key segments in the same order the expanded form nests
/// them — the order of a sorted map over the segment strings — so a reader
/// comparing the report against the artifact walks the two the same way.
fn generators(world: &World, index: &Index) -> Vec<GeneratorView> {
    let mut roots: BTreeMap<DeclaredEntityId, Subtree> = BTreeMap::new();
    let mut components: BTreeMap<DeclaredEntityId, BTreeMap<String, usize>> = BTreeMap::new();
    for entity in world.entities.values() {
        let Some(address) = entity.id.as_derived() else {
            continue;
        };
        let mut cursor = roots.entry(address.root()).or_default();
        for segment in address.key().segments() {
            cursor = cursor.children.entry(segment.to_string()).or_default();
        }
        cursor.entity = index.of(&entity.id);
        for name in entity.components.keys() {
            *components.entry(address.root()).or_default().entry(name.clone()).or_default() += 1;
        }
    }

    world
        .counts
        .iter()
        .map(|(generator, count)| {
            let subtree = roots.remove(generator).unwrap_or_default();
            let id = EntityId::from(*generator);
            let named = world.entities.get(&id).or_else(|| world.declared.get(&id));
            GeneratorView {
                id: generator.to_string(),
                name: named.and_then(|e| e.name.clone()),
                entity: index.of(&id),
                count: *count,
                components: components.remove(generator).unwrap_or_default().into_iter().collect(),
                children: nodes(subtree),
            }
        })
        .collect()
}

/// One derived entity and whatever hangs beneath it, while the tree is being
/// built out of a flat entity set.
#[derive(Default)]
struct Subtree {
    entity: Option<usize>,
    children: BTreeMap<String, Subtree>,
}

fn nodes(subtree: Subtree) -> Vec<Node> {
    subtree
        .children
        .into_iter()
        .map(|(segment, child)| Node {
            segment,
            entity: child.entity,
            children: nodes(child),
        })
        .collect()
}

/// Arcs as index groups. Every endpoint is an entity that survived — the graph
/// is compiled over the post-suppression cell set (SPEC §8 step 5) — so
/// nothing is dropped here; the summary's edge counts come straight from the
/// world either way.
fn edges(world: &World, index: &Index) -> Vec<usize> {
    let mut out = Vec::with_capacity(world.edges.len() * 4);
    for edge in &world.edges {
        let (Some(owner), Some(from), Some(to)) = (
            index.of(&edge.owner),
            index.of(&edge.from),
            index.of(&edge.to),
        ) else {
            continue;
        };
        out.push(owner);
        out.push(from);
        out.push(to);
        out.push(usize::from(edge.bidirectional));
    }
    out
}

fn suppressions(world: &World, index: &Index, absent: &mut AbsentIds) -> SuppressionsView {
    let mut authored = Vec::new();
    let mut by_exclusion: BTreeMap<EntityId, Vec<SuppressedView>> = BTreeMap::new();
    for suppressed in &world.suppressions {
        // A deleted derived entity is shown under the generator that produced
        // it, with its key beside it; a deleted declared one has only itself to
        // be shown by, and is no longer anywhere to be linked to.
        let item = match &suppressed.target {
            EntityId::Derived(address) => SuppressedView {
                anchor: absent.resolve(&address.root().into(), index),
                key: Some(address.key().to_string()),
            },
            EntityId::Declared(_) => SuppressedView {
                anchor: absent.resolve(&suppressed.target, index),
                key: None,
            },
        };
        match &suppressed.excluded_by {
            Some(exclusion) => by_exclusion
                .entry(exclusion.clone())
                .or_default()
                .push(item),
            None => authored.push(item),
        }
    }
    // Deleted targets are a set, and the order expansion happened to record
    // them in is bookkeeping — so they read in the order the expanded form
    // nests them, which is the order of the keys themselves.
    authored.sort_by(address_order);
    SuppressionsView {
        total: world.suppressions.len(),
        authored,
        by_exclusion: by_exclusion
            .into_iter()
            .map(|(exclusion, mut items)| {
                items.sort_by(address_order);
                ExclusionView {
                    exclusion: absent.resolve(&exclusion, index),
                    count: items.len(),
                    items,
                }
            })
            .collect(),
    }
}

fn address_order(a: &SuppressedView, b: &SuppressedView) -> Ordering {
    (a.anchor, &a.key).cmp(&(b.anchor, &b.key))
}
