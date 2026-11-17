//! Within `site-template`, `evaluate` is the engine's one entry point and the steps of spec 05 section 4.3, step 6
//! that follow the layout: keys and ids (9), the reference rewrite (10), records re-rooted (11), and source keys,
//! the plan's validation and the prologue (12). Steps 2 to 8 live in `template`, `values` and `layout`.

use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value, json};
use studio_protocol::{DeclaredId, EntityId, EntityRef};

use crate::count::Count;
use crate::evaluation::{
    Census, Declaration, EngineStamp, Evaluation, Note, Problem, ProblemCode, RelativeScope, ScopePlan,
};
use crate::knob::KnobValues;
use crate::layout::{Layout, Placed, fault_problem};
use crate::read::{DocId, Prologue, Record, TemplateRead};
use crate::references::{self, Position, Rewrite};
use crate::source_key::SourceKey;
use crate::template::{Names, PREFIX, Template};
use crate::values::Knobs;

/// Where new ids come from: the API passes [`Mint`], spec 04's `DeclaredId::mint`.
pub trait IdSource {
    fn fresh(&mut self) -> DeclaredId;
}

/// Random v4 ids, never derived from anything.
pub struct Mint;

impl IdSource for Mint {
    fn fresh(&mut self) -> DeclaredId {
        DeclaredId::mint()
    }
}

/// What a draft carries from one evaluation to the next: the id each source key was first given, and the draft's
/// document id (the site's own on update).
#[derive(Debug, Clone)]
pub struct Prior {
    pub ids: FxHashMap<SourceKey, DeclaredId>,
    pub doc: DocId,
}

impl Prior {
    pub fn new(doc: DocId) -> Self {
        Self { ids: FxHashMap::default(), doc }
    }

    /// The draft's ids after `evaluation`: every key it was given, and every key evaluated earlier.
    pub fn after(mut self, evaluation: &Evaluation) -> Self {
        self.ids.extend(evaluation.ids());
        self
    }
}

/// Evaluate `read` at the knobs a person or the agent `set`, every other knob at its default (an expression default
/// worked out at the others), each made entity keeping the id `prior` gives its source key and every other drawing a
/// fresh one from `ids`.
pub fn evaluate(read: &TemplateRead, set: &KnobValues, prior: &Prior, ids: &mut dyn IdSource) -> Evaluation {
    let mut evaluation = Evaluation {
        knobs: set.clone(),
        derived: FxHashSet::default(),
        prologue: Prologue {
            doc: prior.doc,
            kind: Some("world".into()),
            name: None,
            primary_frame: None,
            depends_on: Vec::new(),
        },
        plan: FxHashMap::default(),
        problems: Vec::new(),
        notes: Vec::new(),
        census: Census::default(),
        engine: EngineStamp {
            version: env!("CARGO_PKG_VERSION").into(),
            registry: read.registry.versions().into_iter().collect(),
        },
    };
    let template = match Template::read(read) {
        Ok(template) => template,
        Err(problems) => {
            evaluation.problems = problems;
            return evaluation;
        }
    };
    evaluation.prologue.name = Some(template.title.clone());
    let Some(values) = Knobs::read(&template, set, &mut evaluation.problems) else { return evaluation };
    evaluation.knobs = values.values.clone();
    evaluation.derived = values.derived.clone();
    values.check(&mut evaluation.problems);

    let mut layout = Layout::new(&template, &values);
    layout.lay_out();
    evaluation.problems.append(&mut layout.problems);
    if evaluation.problems.iter().any(|problem| problem.code == ProblemCode::TemplateInvalid) {
        return evaluation;
    }

    // Step 9: each source key keeps the id the draft first gave it.
    let index: FxHashMap<String, (DeclaredId, RelativeScope)> = layout
        .made
        .iter()
        .map(|made| {
            let id = prior.ids.get(&made.key).copied().unwrap_or_else(|| ids.fresh());
            (made.key.to_string(), (id, made.scope.clone()))
        })
        .collect();
    let resolver =
        Resolver { template: &template, index: &index, absent: &layout.absent, placements: &layout.placements };

    // Steps 10 and 12: references rewritten, the source key written, every component validated.
    let system = read.template.to_string();
    for made in &mut layout.made {
        resolver.rewrite(&mut made.components, &made.scope, &made.key.to_string(), &mut evaluation.problems);
        made.components.insert("external_ids".into(), json!({ system.as_str(): made.key.to_string() }));
        for (name, value) in &made.components {
            if let Err(why) = read.registry.validate(name, value) {
                let text = format!("This template cannot be evaluated: in what it makes, {why}");
                evaluation.problems.push(Problem::error(ProblemCode::TemplateInvalid, text).at(&made.key));
            }
        }
    }

    // Step 11: records follow the generators they aim at.
    let mut records: Vec<(RelativeScope, Record)> = Vec::new();
    for (_, snapshot) in &template.scopes {
        for record in &snapshot.records {
            match resolver.reroot(record, &values, &mut evaluation.problems) {
                Ok(Some(rerooted)) => records.push(rerooted),
                Ok(None) => {}
                Err(note) => evaluation.notes.push(note),
            }
        }
    }

    let primary = index.get(SourceKey::entity(template.primary_frame).as_str()).map(|(id, _)| EntityId::from(*id));
    if primary.is_none() {
        evaluation.problems.push(Problem::error(
            ProblemCode::TemplateInvalid,
            "This template cannot be evaluated: its primary frame is absent at these knobs",
        ));
    }
    // A template-invalid problem leaves the plan empty, whichever step found it.
    if evaluation.problems.iter().any(|problem| problem.code == ProblemCode::TemplateInvalid) {
        return evaluation;
    }
    evaluation.prologue.primary_frame = primary;
    evaluation.census = census(&layout);
    let mut plan: FxHashMap<RelativeScope, ScopePlan> = FxHashMap::default();
    let mut made = layout.made;
    made.sort_by(|a, b| a.key.cmp(&b.key));
    for made in made {
        let (id, _) = index[made.key.as_str()];
        let declaration = Declaration { id, key: made.key, name: made.name, components: made.components };
        plan.entry(made.scope).or_default().declarations.push(declaration);
    }
    for (scope, record) in records {
        plan.entry(scope).or_default().records.push(record);
    }
    evaluation.plan = plan;
    evaluation
}

fn census(layout: &Layout<'_>) -> Census {
    let mut declared: BTreeMap<String, Count> = BTreeMap::new(); // ordered: user-visible
    for made in &layout.made {
        for name in made.components.keys().filter(|name| *name != "external_ids" && !name.starts_with('$')) {
            *declared.entry(name.clone()).or_default() += Count::ONE;
        }
    }
    Census { by_knob: layout.by_knob.clone(), declared, derived: BTreeMap::new() }
}

/// What a reference into the template comes to in the made site.
enum Found {
    Made { id: DeclaredId, scope: RelativeScope, rest: String },
    Placement(String),
    Absent,
}

struct Resolver<'a> {
    template: &'a Template<'a>,
    index: &'a FxHashMap<String, (DeclaredId, RelativeScope)>,
    absent: &'a FxHashSet<String>,
    placements: &'a FxHashMap<String, Placed>,
}

impl Resolver<'_> {
    /// Step 10 for one made entity or record, standing in `scope`, named `at` in problems.
    fn rewrite(
        &self,
        components: &mut Map<String, Value>,
        scope: &RelativeScope,
        at: &str,
        problems: &mut Vec<Problem>,
    ) {
        references::rewrite(components, &self.template.read.registry, &mut |position| {
            self.resolve(&position, scope, at, problems)
        });
    }

    fn resolve(
        &self,
        position: &Position<'_>,
        scope: &RelativeScope,
        at: &str,
        problems: &mut Vec<Problem>,
    ) -> Rewrite {
        // A placement's parameter can bring a reference into a product past step 2's reading of rule 6, so every
        // pin is held to the rule again before a template reference becomes a made one.
        if let Some(why) = self.template.pin_fault(position.value) {
            let text =
                format!("This template cannot be evaluated: `{}` at `{}`: {why}", position.component, position.path);
            problems.push(Problem::error(ProblemCode::TemplateInvalid, text).at(at));
            return Rewrite::Keep;
        }
        let (address, object) = match position.value {
            Value::String(text) => match text.parse::<EntityRef>() {
                Ok(reference) => (reference.id().to_string(), false),
                Err(_) => return Rewrite::Keep,
            },
            Value::Object(map) => {
                match (map.get("generator").and_then(Value::as_str), map.get("key").and_then(Value::as_str)) {
                    (Some(generator), Some(key)) => (format!("{generator}/{key}"), true),
                    _ => return Rewrite::Keep,
                }
            }
            _ => return Rewrite::Keep,
        };
        let Ok(root) = address[..address.len().min(36)].parse::<DeclaredId>() else { return Rewrite::Keep };
        let problem = |code: ProblemCode, why: String| Problem::error(code, why).at(at);
        match self.template.names(root) {
            Names::Catalog => Rewrite::Keep,
            Names::Elsewhere => {
                let why = format!(
                    "`{}` at `{}` names {address}, outside the template and the catalog it pins",
                    position.component, position.path
                );
                problems.push(problem(ProblemCode::ReferenceLeavesTemplate, why));
                Rewrite::Keep
            }
            Names::Template => match self.find(&address) {
                Found::Made { id, scope: lands, rest } => Rewrite::Replace(spell(id, &rest, *scope == lands, object)),
                Found::Placement(key) => {
                    let products = self.stands_for(&key);
                    if position.in_list {
                        Rewrite::Splice(
                            products.into_iter().map(|(id, lands)| spell(id, "", *scope == lands, object)).collect(),
                        )
                    } else if let [(id, lands)] = products.as_slice() {
                        Rewrite::Replace(spell(*id, "", *scope == *lands, object))
                    } else {
                        let why = format!(
                            "`{}` at `{}` names the placement {key}, which made {} entities, in a field that holds one",
                            position.component,
                            position.path,
                            products.len()
                        );
                        problems.push(problem(ProblemCode::PlacementInSingleReference, why));
                        Rewrite::Keep
                    }
                }
                Found::Absent if position.in_list => Rewrite::Splice(Vec::new()),
                Found::Absent => {
                    let why = format!(
                        "`{}` at `{}` names {address}, which is absent at these knobs",
                        position.component, position.path
                    );
                    problems.push(problem(ProblemCode::ReferenceToAbsentEntity, why));
                    Rewrite::Keep
                }
            },
        }
    }

    /// The longest prefix of `address` that is a made entity, a placement or an absent entity.
    fn find(&self, address: &str) -> Found {
        let segments: Vec<&str> = address.split('/').collect();
        for n in (1..=segments.len()).rev() {
            let prefix = segments[..n].join("/");
            if let Some((id, scope)) = self.index.get(&prefix) {
                return Found::Made { id: *id, scope: scope.clone(), rest: segments[n..].join("/") };
            }
            if n == segments.len() && self.placements.contains_key(&prefix) {
                return Found::Placement(prefix);
            }
            if self.absent.contains(&prefix) {
                return Found::Absent;
            }
        }
        Found::Absent
    }

    /// What a placement named in a list stands for: its products of the `stands_for` handles (all, without one),
    /// instance by instance in source-key order, a nested placement standing for its own.
    fn stands_for(&self, key: &str) -> Vec<(DeclaredId, RelativeScope)> {
        let placed = &self.placements[key];
        let mut out = Vec::new();
        for product in &placed.products {
            let handle = product.handle().unwrap_or_default();
            if placed.stands_for.as_ref().is_some_and(|handles| !handles.iter().any(|h| h == handle)) {
                continue;
            }
            if let Some((id, scope)) = self.index.get(product.as_str()) {
                out.push((*id, scope.clone()));
            } else if self.placements.contains_key(product.as_str()) {
                out.extend(self.stands_for(product.as_str()));
            }
        }
        out
    }

    /// Step 11 for one record: present, re-rooted onto its made generator, its references rewritten and what it
    /// carries over validated; a note when its generator is absent.
    fn reroot(
        &self,
        record: &Record,
        values: &Knobs<'_>,
        problems: &mut Vec<Problem>,
    ) -> Result<Option<(RelativeScope, Record)>, Note> {
        let (Record::Override { target, .. } | Record::Suppress { target }) = record;
        if let Record::Override { components, .. } = record {
            match values.present(components) {
                Ok(true) => {}
                Ok(false) => return Ok(None),
                Err(fault) => {
                    problems.push(fault_problem(fault, "the record's presence condition").at(target));
                    return Ok(None);
                }
            }
        }
        let address = target.to_string();
        let Found::Made { id, scope, rest } = self.find(&address) else {
            return Err(Note {
                reference: Some(address),
                text: "A record was left out: the generator it aims at is absent at these knobs.".into(),
            });
        };
        let rerooted: EntityId = format!("{id}/{rest}").parse().map_err(|_| Note {
            reference: Some(address.clone()),
            text: "A record was left out: it aims at a made entity itself, not at an entity it derives.".into(),
        })?;
        Ok(Some(match record {
            Record::Override { components, .. } => {
                let mut components = components.clone();
                components.retain(|name, _| !name.starts_with(PREFIX));
                self.rewrite(&mut components, &scope, &address, problems);
                for (name, value) in &components {
                    if let Err(why) = self.template.registry().validate(name, value) {
                        let text = format!("This template cannot be evaluated: in a record it carries over, {why}");
                        problems.push(Problem::error(ProblemCode::TemplateInvalid, text).at(&address));
                    }
                }
                (scope, Record::Override { target: rerooted, components })
            }
            Record::Suppress { .. } => (scope, Record::Suppress { target: rerooted }),
        }))
    }
}

/// A made reference: bare within one made scope, `@head` across the made site's scopes (spec 04's `Local` and
/// `Kin`); a derivation address keeps its two-field shape, which holds no pin.
fn spell(id: DeclaredId, rest: &str, same_scope: bool, object: bool) -> Value {
    if object && !rest.is_empty() {
        return json!({ "generator": id.to_string(), "key": rest });
    }
    let base = if rest.is_empty() { id.to_string() } else { format!("{id}/{rest}") };
    Value::String(if same_scope { base } else { format!("{base}@head") })
}
