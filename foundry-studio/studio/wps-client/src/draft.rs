//! Within `wps-client`, `draft` is spec 04 section 3.4's write side: a changeset draft with a base for every scope of
//! the cut it was built against, WPS's own operations, what an apply answers, and `DraftBuilder`, the one way a
//! consumer builds a draft: it mints new ids, merges every edit of one entity into one operation (WPS refuses two on
//! one entity, `wp:server/server/src/storage/apply.rs:1310-1336`), and spells references.

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq};

use crate::error::WpsError;
use crate::snapshot::UnitSnapshot;
use crate::value::{ArchivedRecord, Author, CreatedScope, GenerationRan, GeneratorSet, OpTally, Prologue, Reference};

/// One of WPS's operations (`wp:server/core/src/changeset.rs`), `replace` being exact replace (`server/first-cut`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Declare {
        id: DeclaredId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        components: Map<String, Value>,
    },
    Patch {
        id: DeclaredId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        components: Map<String, Value>,
    },
    Replace {
        id: DeclaredId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        components: Map<String, Value>,
    },
    Delete {
        id: DeclaredId,
    },
    Override {
        target: EntityId,
        components: Map<String, Value>,
    },
    Suppress {
        target: EntityId,
    },
}

impl Op {
    /// What WPS counts one operation as against a duplicate: the entity, or one kind of record of a target.
    pub fn key(&self) -> String {
        match self {
            Self::Declare { id, .. } | Self::Patch { id, .. } | Self::Replace { id, .. } | Self::Delete { id } => {
                id.to_string()
            }
            Self::Override { target, .. } => format!("{target}#override"),
            Self::Suppress { target } => format!("{target}#suppress"),
        }
    }

    pub fn components(&self) -> Option<&Map<String, Value>> {
        match self {
            Self::Declare { components, .. }
            | Self::Patch { components, .. }
            | Self::Replace { components, .. }
            | Self::Override { components, .. } => Some(components),
            Self::Delete { .. } | Self::Suppress { .. } => None,
        }
    }

    /// The entity an operation writes, with its label when it states one.
    pub fn entity(&self) -> EntityId {
        match self {
            Self::Declare { id, .. } | Self::Patch { id, .. } | Self::Replace { id, .. } | Self::Delete { id } => {
                (*id).into()
            }
            Self::Override { target, .. } | Self::Suppress { target } => target.clone(),
        }
    }
}

/// The tally WPS answers for these operations.
pub fn tally(ops: &[&Op]) -> OpTally {
    let mut tally = OpTally::default();
    for op in ops {
        match op {
            Op::Declare { .. } => tally.declared += 1,
            Op::Patch { .. } => tally.patched += 1,
            Op::Replace { .. } => tally.replaced += 1,
            Op::Delete { .. } => tally.deleted += 1,
            Op::Override { .. } => tally.overridden += 1,
            Op::Suppress { .. } => tally.suppressed += 1,
        }
    }
    tally
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedOp {
    pub scope: ScopePath,
    #[serde(flatten)]
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewScope {
    pub path: ScopePath,
    pub boundary: bool,
}

/// A change to a site, stored verbatim by spec 01 and applied against the bases it was built on (ruling D43).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangesetDraft {
    pub initiated_on: ScopePath,
    // ordered: bases and prologues are walked, sent and stored in scope path order.
    pub bases: BTreeMap<ScopePath, Seq>,
    pub creates: Vec<NewScope>,
    pub prologues: BTreeMap<ScopePath, Prologue>,
    pub author: Author,
    pub message: String,
    pub ops: Vec<ScopedOp>,
    pub expect_generators: Option<GeneratorSet>,
    /// Spec 01's operation id, sent as the body's `operation` and never in the message: every write names one, of 1
    /// to 128 characters, or is refused before anything is sent.
    pub operation: String,
    /// The ids this draft minted, which `check` knows unspent without reading their trail.
    #[serde(default)]
    pub minted: Vec<DeclaredId>,
}

impl ChangesetDraft {
    /// The scopes the draft writes, as WPS reads its changeset: each scope an operation or a prologue names, or the
    /// initiating scope when it names none and creates none (`wp:server/DESIGN.md` section 8 at 092d695).
    pub fn written(&self) -> Vec<ScopePath> {
        let mut written: Vec<ScopePath> = self.ops.iter().map(|op| op.scope.clone()).collect();
        written.extend(self.prologues.keys().cloned());
        if written.is_empty() && self.creates.is_empty() {
            written.push(self.initiated_on.clone());
        }
        written.sort();
        written.dedup();
        written
    }

    pub fn ops_of<'a>(&'a self, scope: &'a ScopePath) -> impl Iterator<Item = &'a Op> + 'a {
        self.ops.iter().filter(move |op| &op.scope == scope).map(|op| &op.op)
    }
}

/// What an apply wrote, all in one changeset: each written scope's new changeset and what its generation ran (a
/// scope missing from `generation` when that went unread), and each scope the changeset created.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    pub seqs: BTreeMap<ScopePath, Seq>,
    pub generation: BTreeMap<ScopePath, Option<GenerationRan>>,
    pub archived: Vec<ArchivedRecord>,
    #[serde(default)]
    pub created: Vec<CreatedScope>,
    pub warnings: Vec<String>,
    /// Nothing was applied again: WPS answered the first sending's whole answer, stored under the operation
    /// identifier.
    pub replayed: bool,
}

/// What WPS's import proposes for a document against a base, as a draft that applies it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportProposal {
    pub draft: ChangesetDraft,
    pub tally: OpTally,
    pub warnings: Vec<String>,
}

/// Builds one draft against the snapshot it was read from.
pub struct DraftBuilder<'a> {
    base: &'a UnitSnapshot,
    draft: ChangesetDraft,
    by_key: FxHashMap<String, usize>,
}

impl<'a> DraftBuilder<'a> {
    pub fn new(
        base: &'a UnitSnapshot,
        initiated_on: ScopePath,
        author: Author,
        message: impl Into<String>,
        operation: impl Into<String>,
    ) -> Self {
        let draft = ChangesetDraft {
            initiated_on,
            bases: base.cut.seqs.clone(),
            creates: Vec::new(),
            prologues: BTreeMap::new(),
            author,
            message: message.into(),
            ops: Vec::new(),
            expect_generators: None,
            operation: operation.into(),
            minted: Vec::new(),
        };
        Self { base, draft, by_key: FxHashMap::default() }
    }

    fn push(&mut self, scope: ScopePath, op: Op) {
        self.by_key.insert(op.key(), self.draft.ops.len());
        self.draft.ops.push(ScopedOp { scope, op });
    }

    /// Declare a new entity in `scope` under a freshly minted id, which it returns.
    pub fn declare(&mut self, scope: &ScopePath, name: Option<String>, components: Map<String, Value>) -> DeclaredId {
        let id = DeclaredId::mint();
        self.draft.minted.push(id);
        self.push(scope.clone(), Op::Declare { id, name, components });
        id
    }

    fn owner(&self, id: &DeclaredId) -> Result<ScopePath, WpsError> {
        let pending = self.by_key.get(&id.to_string()).map(|&at| self.draft.ops[at].scope.clone());
        pending.or_else(|| self.base.owner_of(&(*id).into()).cloned()).ok_or_else(|| WpsError::NotFound {
            text: format!("no scope of {} holds {id} at the changesets this draft was built on", self.base.cut.unit),
        })
    }

    /// Set the value at `pointer` inside `component` of entity `id`, as one whole-component patch per entity.
    pub fn patch_pointer(
        &mut self,
        id: DeclaredId,
        component: &str,
        pointer: &str,
        value: Value,
    ) -> Result<(), WpsError> {
        let scope = self.owner(&id)?;
        let at = match self.by_key.get(&id.to_string()) {
            Some(&at) => at,
            None => {
                self.push(scope, Op::Patch { id, name: None, components: Map::new() });
                self.draft.ops.len() - 1
            }
        };
        let current = self.base.get(&id).and_then(|(_, entity)| entity.components.get(component).cloned());
        let components = match &mut self.draft.ops[at].op {
            Op::Declare { components, .. } | Op::Patch { components, .. } | Op::Replace { components, .. } => {
                components
            }
            other => {
                let text = format!("{id} is {} in this draft, so it takes no patch", op_word(other));
                return Err(WpsError::Invalid { text });
            }
        };
        let target =
            components.entry(component.to_owned()).or_insert_with(|| current.unwrap_or(Value::Object(Map::new())));
        set_pointer(target, pointer, value)
            .map_err(|why| WpsError::Invalid { text: format!("{id}'s `{component}` at `{pointer}`: {why}") })
    }

    /// A reference as the payload holds it (spec 04 section 3.7's spelling table).
    pub fn reference(&self, reference: &Reference) -> String {
        reference.spelled()
    }

    /// State that kept entity `id` carries exactly `components` and `name`: a `replace`, WPS's exact replace.
    pub fn overwrite(
        &mut self,
        id: DeclaredId,
        name: Option<String>,
        components: Map<String, Value>,
    ) -> Result<(), WpsError> {
        let scope = self.owner(&id)?;
        let op = Op::Replace { id, name, components };
        match self.by_key.get(&id.to_string()) {
            Some(&at) => self.draft.ops[at].op = op,
            None => self.push(scope, op),
        }
        Ok(())
    }

    pub fn delete(&mut self, id: DeclaredId) -> Result<(), WpsError> {
        let scope = self.owner(&id)?;
        match self.by_key.get(&id.to_string()) {
            Some(&at) => self.draft.ops[at].op = Op::Delete { id },
            None => self.push(scope, Op::Delete { id }),
        }
        Ok(())
    }

    pub fn record_override(&mut self, scope: &ScopePath, target: EntityId, components: Map<String, Value>) {
        self.push(scope.clone(), Op::Override { target, components });
    }

    pub fn suppress(&mut self, scope: &ScopePath, target: EntityId) {
        self.push(scope.clone(), Op::Suppress { target });
    }

    pub fn prologue(&mut self, scope: &ScopePath, prologue: Prologue) {
        self.draft.prologues.insert(scope.clone(), prologue);
    }

    /// A scope this draft creates, at base 0, inside its one changeset (section 3.4).
    pub fn create(&mut self, scope: NewScope) {
        self.draft.bases.insert(scope.path.clone(), Seq::new(0));
        self.draft.creates.push(scope);
    }

    pub fn expect_generators(&mut self, generators: GeneratorSet) {
        self.draft.expect_generators = Some(generators);
    }

    pub fn build(self) -> ChangesetDraft {
        self.draft
    }
}

fn op_word(op: &Op) -> &'static str {
    match op {
        Op::Declare { .. } => "declared",
        Op::Patch { .. } => "patched",
        Op::Replace { .. } => "replaced",
        Op::Delete { .. } => "deleted",
        Op::Override { .. } => "an override",
        Op::Suppress { .. } => "a suppression",
    }
}

/// Set `value` at the JSON pointer `pointer` (RFC 6901) inside `target`, making the objects on the way.
pub fn set_pointer(target: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
    if pointer.is_empty() {
        *target = value;
        return Ok(());
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err("a JSON pointer starts with `/`".to_owned());
    };
    let tokens: Vec<String> = rest.split('/').map(|token| token.replace("~1", "/").replace("~0", "~")).collect();
    let (last, path) = tokens.split_last().expect("split yields one token at least");
    let mut at = target;
    for token in path {
        at = match at {
            Value::Object(map) => map.entry(token.clone()).or_insert_with(|| Value::Object(Map::new())),
            Value::Array(items) => {
                let index: usize = token.parse().map_err(|_| format!("`{token}` is not an array index"))?;
                items.get_mut(index).ok_or_else(|| format!("index {index} is past the array's end"))?
            }
            _ => return Err(format!("`{token}` steps into a value that is neither an object nor an array")),
        };
    }
    match at {
        Value::Object(map) => {
            map.insert(last.clone(), value);
        }
        Value::Array(items) if last == "-" || last.parse::<usize>().ok() == Some(items.len()) => items.push(value),
        Value::Array(items) => {
            let index: usize = last.parse().map_err(|_| format!("`{last}` is not an array index"))?;
            *items.get_mut(index).ok_or_else(|| format!("index {index} is past the array's end"))? = value;
        }
        _ => return Err(format!("`{last}` names a field of a value that is neither an object nor an array")),
    }
    Ok(())
}
