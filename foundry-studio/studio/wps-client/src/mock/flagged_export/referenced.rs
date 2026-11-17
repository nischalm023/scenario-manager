//! Within the mock's flagged export, `referenced` is WHAT `referenced=1` BRINGS INTO THE FILE: every entity that an
//! entity the file holds references, followed through each target's own references until nothing new turns up, each
//! such entity and none of the rest of its scope; or WPS's refusal of a scope the export would read at two changesets,
//! or of more scopes than one export holds.
//!
//! A pin brings its target at the pinned changeset, and the target's own references at head read the scopes beside it
//! as that changeset left them, by commit place; a reference at head from a requested scope reads where the request
//! reads. A scope the export reads is held at the changeset the request reads it at, and any other at the first the
//! walk places it at, so a target placed anywhere else is refused. Like WPS's index, the walk reads a reference only
//! at an annotated place, so one nested where the annotations do not reach, such as in a template's entities, is not
//! followed before complete annotation traversal lands.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use studio_protocol::{EntityId, EntityRef, Pin, RefPin, ScopePath, Seq};

use super::generation::Generated;
use super::requested::Request;
use super::{CLOSURE_LIMIT, HeldScope, Row, closure_too_large, reached_twice};
use crate::error::WpsError;
use crate::mock::{Inner, Latest};
use crate::value::CommitPlace;
use crate::walker::{Annotations, annotated_pointers};

/// Where the references at head of the entity a visit reads land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum HeadsAt {
    /// In the changesets the request reads.
    Requested,
    /// As the pinned changeset that took this commit place left each scope beside it.
    AsOf(CommitPlace),
}

/// One row of WPS's reference index: the scope that owns the reference's target, the target's identity, and its pin,
/// none for a reference at head.
struct IndexRow {
    to_scope: ScopePath,
    to_id: String,
    pin: Option<Seq>,
}

/// An entity a reference names, where the walk reads it: its scope at a changeset, and where its own references at
/// head land.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Target {
    scope: ScopePath,
    at: Seq,
    heads: HeadsAt,
    id: String,
}

impl Inner {
    /// Every scope the flagged export of `scope` holds entities from: the requested scopes, whole, and with
    /// `referenced` the entities their entities reference, followed through each target's own references until
    /// nothing new turns up; or WPS's refusal of a scope the export would read at two changesets, or of more scopes
    /// than one export holds. `generated` are the rows a stand-in generation made.
    pub(super) fn held(
        &self,
        scope: &ScopePath,
        request: &Request,
        generated: &[Generated],
        latest: &Latest,
    ) -> Result<BTreeMap<ScopePath, HeldScope>, WpsError> {
        // ordered: the file lists its scopes, and their entities, in path order.
        let mut held: BTreeMap<ScopePath, HeldScope> = request
            .whole
            .iter()
            .map(|path| (path.clone(), HeldScope { at: request.at[path], brought: None }))
            .collect();
        if !request.flags.referenced {
            return Ok(held);
        }
        let mut references = Vec::new();
        for path in &request.whole {
            for row in self.rows(path, request.at[path], request.flags.derived, generated)?.values() {
                references.extend(self.references_of(row, latest).into_iter().map(|one| (HeadsAt::Requested, one)));
            }
        }
        let (mut visited, mut scope_rows) = (FxHashSet::default(), FxHashMap::default());
        while !references.is_empty() {
            let mut targets: Vec<Target> =
                references.into_iter().filter_map(|(heads, one)| self.target_of(request, heads, one)).collect();
            // In one order, so that a refusal is the same refusal every time.
            targets.sort_unstable();
            targets.dedup();
            let mut next = Vec::new();
            for target in targets {
                let rows = match scope_rows.entry((target.scope.clone(), target.at)) {
                    Entry::Occupied(rows) => rows.into_mut(),
                    Entry::Vacant(rows) => {
                        rows.insert(self.rows(&target.scope, target.at, request.flags.derived, generated)?)
                    }
                };
                let Some(row) = rows.get(&target.id) else { continue };
                // A scope the export reads is held where the request reads it, and any other where the walk first
                // placed it, so a pin never overrides the request.
                let placed = request.at.get(&target.scope).or(held.get(&target.scope).map(|one| &one.at));
                if let Some(&at) = placed.filter(|&&at| at != target.at) {
                    return Err(reached_twice(&target.scope, at, target.at));
                }
                let full = held.len() == CLOSURE_LIMIT;
                match held.get_mut(&target.scope) {
                    Some(one) => {
                        if let Some(brought) = &mut one.brought {
                            brought.insert(target.id.clone());
                        }
                    }
                    None if full => return Err(closure_too_large(scope)),
                    None => {
                        let brought = Some(BTreeSet::from([target.id.clone()]));
                        held.insert(target.scope.clone(), HeldScope { at: target.at, brought });
                    }
                }
                let heads = target.heads;
                if visited.insert(target) {
                    next.extend(self.references_of(row, latest).into_iter().map(|one| (heads, one)));
                }
            }
            references = next;
        }
        Ok(held)
    }

    /// Where the walk reads the target of `row`, a reference made by an entity whose references at head land at
    /// `heads`: a pin's at the pinned changeset, its own references at head landing as that changeset left the scopes
    /// beside it; one at head's where its referrer's land. One at head that lands where the request reads, in a
    /// requested scope, lands in what the file holds whole, and is answered none; so is one naming a scope the request
    /// does not read, which WPS's write path refuses and the mock leaves unresolved, and a pin at no changeset its
    /// scope reached.
    fn target_of(&self, request: &Request, heads: HeadsAt, row: IndexRow) -> Option<Target> {
        let IndexRow { to_scope, to_id, pin } = row;
        let (at, heads) = match (pin, heads) {
            (Some(pin), _) => (pin, HeadsAt::AsOf(self.commit_place(&to_scope, pin)?)),
            (None, HeadsAt::AsOf(place)) => (self.stood_at(place, &to_scope), heads),
            (None, HeadsAt::Requested) if request.whole.contains(&to_scope) => return None,
            (None, HeadsAt::Requested) => (*request.at.get(&to_scope)?, heads),
        };
        Some(Target { scope: to_scope, at, heads, id: to_id })
    }

    /// The commit place changeset `seq` of `scope` took, if the scope reached that changeset.
    fn commit_place(&self, scope: &ScopePath, seq: Seq) -> Option<CommitPlace> {
        let index = usize::try_from(seq.get().checked_sub(1)?).ok()?;
        self.scopes.get(scope)?.places.get(index).copied()
    }

    /// The changeset `scope` stood at as the changeset that took `place` left it: its latest no later than it, or 0.
    fn stood_at(&self, place: CommitPlace, scope: &ScopePath) -> Seq {
        let places = self.scopes.get(scope).map(|held| held.places.as_slice()).unwrap_or_default();
        Seq::new(places.iter().take_while(|taken| **taken <= place).count() as u64)
    }

    /// The references WPS's index holds for `row`: each at an annotated place of its components, and of the components
    /// a planting left under an override, naming an identity some scope owns, one string with its pin or at head, or
    /// a derivation address in its two parts at head. A tag or a branch names no changeset the walk can read.
    fn references_of(&self, row: &Row, latest: &Latest) -> Vec<IndexRow> {
        let mut found = Vec::new();
        for components in [Some(&row.components), row.planted.as_ref()].into_iter().flatten() {
            for (component, payload) in components {
                for pointer in annotated_pointers(payload, latest.refs(component)) {
                    let (id, pin) = match payload.pointer(&pointer) {
                        Some(Value::String(text)) => {
                            let Ok(reference) = text.parse::<EntityRef>() else { continue };
                            let pin = match reference.pin() {
                                None | Some(RefPin::Head) => None,
                                Some(RefPin::Pin(Pin::Changeset(seq))) => Some(*seq),
                                Some(RefPin::Pin(_)) => continue,
                            };
                            (reference.id().clone(), pin)
                        }
                        Some(Value::Object(parts)) => {
                            let part = |key: &str| parts.get(key).and_then(Value::as_str);
                            let (Some(generator), Some(key)) = (part("generator"), part("key")) else { continue };
                            let Ok(id) = format!("{generator}/{key}").parse::<EntityId>() else { continue };
                            (id, None)
                        }
                        _ => continue,
                    };
                    if let Some(owner) = self.owners.get(&id.declared().to_string()) {
                        found.push(IndexRow { to_scope: owner.clone(), to_id: id.to_string(), pin });
                    }
                }
            }
        }
        found
    }
}
