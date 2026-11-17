//! Within the mock's flagged export, `generation` is THE STAND-IN FOR WPS'S GENERATION a test gives the mock
//! (`set_generation`): what it reads, the requested scopes' declarations and records and every declared entity
//! `referenced` brought, its pins stripped; and the rows it makes, each in the scope that holds its generator, given
//! back the pins its input held as WPS gives a generated row them; or the export refused where a row names nothing a
//! file could hold.

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;
use serde_json::{Map, Value, json};
use studio_protocol::{EntityRef, Pin, RefPin, ScopePath, Seq};

use super::requested::Request;
use super::{HeldScope, reached_twice};
use crate::error::WpsError;
use crate::export::strip_pins;
use crate::mock::{Inner, Latest, MockWps, refuse};
use crate::refusal::RouteKind;
use crate::walker::{Annotations, RefForm, annotated_pointers, walk};

/// A generation standing in for WPS's: it reads every entity an export's requested scopes declare and every record
/// they hold, and every declared entity `referenced` brought, spelled as a document's `entities` spells them, pins
/// stripped; and answers each entity it generates, `{ id, name, components }`, its records laid over it and a
/// suppressed one left out, or the sentence it refused with.
pub type StandInGeneration = Box<dyn Fn(Vec<Value>) -> Result<Vec<Value>, String> + Send + Sync>;

/// A row a stand-in generation makes: the scope that holds it, its identity, label and components.
pub(super) struct Generated {
    pub(super) scope: ScopePath,
    pub(super) id: String,
    pub(super) name: Option<String>,
    pub(super) components: Map<String, Value>,
}

/// What a stand-in generation reads, the scope that holds each declared entity among it, and each scope its pins
/// name with the changesets they name it at.
pub(super) struct Input {
    pub(super) read: Vec<Value>,
    owners: FxHashMap<String, ScopePath>,
    pinned: FxHashMap<ScopePath, Vec<Seq>>,
}

impl MockWps {
    /// Generate the derived entities of each flagged export with `generation`, as WPS generates a site's as each write
    /// lands; without one, an export holds only the derived entities a test plants.
    pub fn set_generation(&self, generation: StandInGeneration) {
        self.lock().generation = Some(generation);
    }
}

impl Inner {
    /// What a stand-in generation reads: every entity the requested scopes declare and every record they hold but a
    /// planted row's, and every declared entity `referenced` brought, its pins stripped as a generation reads through
    /// them; the scope that holds each declared entity read; and every pin they held, which `repin` gives back.
    ///
    /// WPS reads otherwise. It generates each scope as the scope is written, reading the scopes it pins, their declared
    /// and derived entities, as read-only input; the stand-in generates at export time, over what the export brings.
    /// With all three flags, which the client always asks for, that is every declared entity a reference from the
    /// requested scopes reaches, but never a pinned scope's derived entities. Without `referenced` it is nothing they
    /// pin, so the stand-in can refuse a site WPS exports: a template a stamp pins, say, is not among what it reads.
    pub(super) fn generation_input(
        &self,
        request: &Request,
        held: &BTreeMap<ScopePath, HeldScope>,
        latest: &Latest,
    ) -> Result<Input, WpsError> {
        let mut input = Input { read: Vec::new(), owners: FxHashMap::default(), pinned: FxHashMap::default() };
        for (path, one) in held {
            let (_, state, _) = self.state_at(path, Some(one.at))?;
            for (id, (_, name, components)) in &state.entities {
                if one.brought.as_ref().is_some_and(|brought| !brought.contains(id)) {
                    continue;
                }
                let mut entry = json!({ "id": id, "components": components });
                if let Some(name) = name {
                    entry["name"] = json!(name);
                }
                self.remember(&mut input.pinned, components, latest);
                input.read.push(stripped(entry, latest));
                input.owners.insert(id.clone(), path.clone());
            }
            if !request.whole.contains(path) {
                continue;
            }
            for (key, record) in &state.records {
                let target = key.rsplit_once('#').map_or(key.as_str(), |(target, _)| target);
                if state.planted.contains_key(target) {
                    continue;
                }
                if let Some(components) = record.get("components").and_then(Value::as_object) {
                    self.remember(&mut input.pinned, components, latest);
                }
                input.read.push(stripped(record.clone(), latest));
            }
        }
        Ok(input)
    }

    /// Note in `pinned` each pin `components` holds, under the scope that owns its target.
    fn remember(&self, pinned: &mut FxHashMap<ScopePath, Vec<Seq>>, components: &Map<String, Value>, latest: &Latest) {
        for (root, seq) in pins_of(components, latest) {
            if let Some(owner) = self.owners.get(&root) {
                pinned.entry(owner.clone()).or_default().push(seq);
            }
        }
    }

    /// What `generation` makes of `input`, each row in the scope that holds its generator, its references given back
    /// the pins the input held (`repin`); or the export refused, naming why, where a row names no entity a file could
    /// hold.
    pub(super) fn generated(
        &self,
        generation: &StandInGeneration,
        export: &ScopePath,
        input: &Input,
        latest: &Latest,
    ) -> Result<Vec<Generated>, WpsError> {
        let refused = |why: String| {
            let sentence = format!("the generation of {export}'s export {why}");
            refuse(RouteKind::Export, 422, "generation_refused", json!({ "scope": export }), sentence)
        };
        let made = generation(input.read.clone()).map_err(|why| refused(format!("refused it: {why}")))?;
        let mut rows = Vec::new();
        for entry in made {
            let Some(id) = entry.get("id").and_then(Value::as_str).map(str::to_owned) else {
                return Err(refused(format!("made a row without an `id`: {entry}")));
            };
            let root = id.split('/').next().unwrap_or_default();
            let Some(scope) = input.owners.get(root).cloned() else {
                return Err(refused(format!("made {id}, whose generator no entity it read declares")));
            };
            let Some(mut components) = entry.get("components").and_then(Value::as_object).cloned() else {
                return Err(refused(format!("made {id} without its components")));
            };
            self.repin(&scope, &mut components, &input.pinned, latest)?;
            let name = entry.get("name").and_then(Value::as_str).map(str::to_owned);
            rows.push(Generated { scope, id, name, components });
        }
        Ok(rows)
    }

    /// Give a generated row's references back the pins its generation's input held, as WPS leaves the rows its
    /// generation stores (`wp:server/server/src/storage/generation.rs`, `repin`, and a module's copy of a pinned
    /// reference, which stays pinned in its row): at each annotated place, a reference at head naming a scope of
    /// another unit than `holder`'s takes the changeset the input pinned that scope at. A scope the input pinned at
    /// two is refused as reached twice, as WPS refuses the write that would leave such a row.
    fn repin(
        &self,
        holder: &ScopePath,
        components: &mut Map<String, Value>,
        pinned: &FxHashMap<ScopePath, Vec<Seq>>,
        latest: &Latest,
    ) -> Result<(), WpsError> {
        let unit_of = |path: &ScopePath| self.scopes.get(path).map(|scope| &scope.unit);
        for (component, payload) in components.iter_mut() {
            for pointer in annotated_pointers(payload, latest.refs(component)) {
                let Some(slot) = payload.pointer_mut(&pointer) else { continue };
                let Some(Ok(reference)) = slot.as_str().map(str::parse::<EntityRef>) else { continue };
                let Some(owner) = self.owners.get(&reference.id().declared().to_string()) else { continue };
                if !matches!(reference.pin(), None | Some(RefPin::Head)) || unit_of(owner) == unit_of(holder) {
                    continue;
                }
                let Some(seqs) = pinned.get(owner) else { continue };
                let (Some(low), Some(high)) = (seqs.iter().min(), seqs.iter().max()) else { continue };
                if low != high {
                    return Err(reached_twice(owner, *low, *high));
                }
                *slot = Value::String(format!("{}@{low}", reference.id()));
            }
        }
        Ok(())
    }
}

/// An entry with every pin the walker reads stripped from its components.
fn stripped(mut entry: Value, annotations: &Latest) -> Value {
    strip_pins(&mut entry, annotations);
    entry
}

/// Each pinned reference of `components` the walker reads, nested ones included, as the identity its root was
/// declared as and the changeset: every pin a generation's input holds, which this mock strips before its generation
/// reads it (`stripped`) and gives back to the rows that name what it pinned (`repin`).
fn pins_of(components: &Map<String, Value>, annotations: &Latest) -> Vec<(String, Seq)> {
    let mut found = Vec::new();
    for (component, payload) in components {
        for occurrence in walk(component, payload, annotations) {
            let RefForm::Text(text) = occurrence.form else { continue };
            let Ok(reference) = text.parse::<EntityRef>() else { continue };
            if let Some(RefPin::Pin(Pin::Changeset(seq))) = reference.pin() {
                found.push((reference.id().declared().to_string(), *seq));
            }
        }
    }
    found
}
