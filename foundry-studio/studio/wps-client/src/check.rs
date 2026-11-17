//! Within `wps-client`, `check` is the client's own pre-flight of a draft (spec 04 sections 3.4, 3.5 and 3.7): the
//! invariants of a draft, each payload against the registry's schema, and every reference under ruling D76's
//! kinship and WPS's pin rule, over the reference graph the draft would leave (surviving components and records,
//! after WPS's archive rule), before anything is sent. It reads WPS only for what the base cannot answer (a target
//! outside the site, a head, a spent id, a delete's referrers), so a site-local draft sends no request at all. It
//! cannot run generation, so a generator's refusal, a vanish that strands a referrer and a record aimed at an address
//! nothing derives still arrive at apply time.

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value};
use studio_protocol::{DeclaredId, EntityId, EntityRef, Kinship, Pin, RefPin, ScopePath, Seq};

use crate::apply::body_of;
use crate::client::Client;
use crate::draft::{ChangesetDraft, NewScope, Op};
use crate::error::{Problem, ProblemKind, WpsError};
use crate::kinship::{ScopeTree, classify};
use crate::refusal::{BODY_LIMIT, Unresolved, unresolved};
use crate::schema::RegistrySnapshot;
use crate::site::{is_loadable_root, is_root};
use crate::snapshot::{Record, UnitSnapshot};
use crate::value::Capability;
use crate::walker::{RefForm, components_read, walk};
use crate::wire::WireReferrer;
use crate::wps::Wps;

/// Where a reference sits: its scope, entity, component and path.
struct Site<'a> {
    scope: &'a ScopePath,
    entity: EntityId,
    component: &'a str,
    path: &'a str,
}

impl Site<'_> {
    fn problem(&self, kind: ProblemKind, text: String) -> Problem {
        Problem::new(kind, text).in_scope(self.scope).on(self.entity.clone()).at_field(self.component, self.path)
    }
}

/// What the references a draft leaves standing pin: each (referrer's scope, target's scope) at its changesets.
type Pins = FxHashMap<(ScopePath, ScopePath), Vec<(Seq, String)>>;

/// What every reference of one draft is checked against: the base, the scopes it creates, what it declares and
/// deletes, the scope tree once read, and the pins found so far.
struct Context<'a> {
    base: &'a UnitSnapshot,
    creates: &'a [NewScope],
    declared_here: FxHashMap<DeclaredId, ScopePath>,
    deleted_here: FxHashSet<DeclaredId>,
    tree: Option<ScopeTree>,
    pins: Pins,
}

/// Where WPS says a reference's target lives, or the problem its resolution names.
enum Owner {
    Scope(ScopePath),
    Problem(ProblemKind, String),
}

fn holds_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(holds_nul),
        Value::Object(fields) => fields.iter().any(|(key, item)| key.contains('\0') || holds_nul(item)),
        _ => false,
    }
}

/// The draft's operation on each declared entity it declares, patches, replaces or deletes.
fn entity_ops(draft: &ChangesetDraft) -> FxHashMap<DeclaredId, &Op> {
    draft
        .ops
        .iter()
        .filter(|op| matches!(op.op, Op::Declare { .. } | Op::Patch { .. } | Op::Replace { .. } | Op::Delete { .. }))
        .map(|op| (op.op.entity().declared(), &op.op))
        .collect()
}

/// Whether WPS archives a live record aimed under `target` when the draft applies without restating it: the draft
/// deletes or replaces the target's anchor, or writes a component of it a generator reads (288463c).
fn archived(target: &EntityId, ops: &FxHashMap<DeclaredId, &Op>, registry: &RegistrySnapshot) -> bool {
    match ops.get(&target.declared()) {
        None => false,
        Some(Op::Delete { .. } | Op::Replace { .. }) => true,
        Some(op) => op.components().is_some_and(|written| written.keys().any(|name| registry.is_generator_input(name))),
    }
}

/// Whether `payload` of `component` still names the entity `id`, at any place the walker reads.
fn names(component: &str, payload: &Value, id: DeclaredId, registry: &RegistrySnapshot) -> bool {
    walk(component, payload, registry).iter().any(|occurrence| match &occurrence.form {
        RefForm::Text(text) => text.parse::<EntityRef>().is_ok_and(|reference| reference.id().declared() == id),
        RefForm::TwoPart { generator, .. } => generator.parse::<DeclaredId>().is_ok_and(|anchor| anchor == id),
        _ => false,
    })
}

/// Whether a referrer of `id` at head still names it once the draft applies: the entity or record holding it survives,
/// and the component holding it is one the draft leaves alone or rewrites still naming `id`.
fn still_names(
    referrer: &WireReferrer,
    id: DeclaredId,
    draft: &ChangesetDraft,
    ops: &FxHashMap<DeclaredId, &Op>,
    registry: &RegistrySnapshot,
) -> bool {
    if referrer.pin.is_some() || matches!(ops.get(&referrer.from.declared()), Some(Op::Delete { .. })) {
        return false;
    }
    let rewritten = |components: &Map<String, Value>| {
        components.get(&referrer.component).is_some_and(|payload| names(&referrer.component, payload, id, registry))
    };
    if referrer.field_path.starts_with("override:") {
        let restated = draft.ops.iter().find_map(|op| match &op.op {
            Op::Override { target, components } if op.scope == referrer.scope && *target == referrer.from => {
                Some(components)
            }
            _ => None,
        });
        return match restated {
            Some(components) => rewritten(components),
            None => !archived(&referrer.from, ops, registry),
        };
    }
    if referrer.from.segments().next().is_some() {
        return true;
    }
    match ops.get(&referrer.from.declared()) {
        Some(Op::Replace { components, .. }) => rewritten(components),
        Some(Op::Declare { components, .. } | Op::Patch { components, .. })
            if components.contains_key(&referrer.component) =>
        {
            rewritten(components)
        }
        _ => true,
    }
}

impl<S: Wps> Client<S> {
    pub(crate) async fn check_draft(
        &self,
        draft: &ChangesetDraft,
        base: &UnitSnapshot,
    ) -> Result<Vec<Problem>, WpsError> {
        let mut problems = Vec::new();
        let mut read = FxHashSet::default();
        for (component, payload) in draft.ops.iter().filter_map(|op| op.op.components()).flatten() {
            read.extend(components_read(component, payload));
        }
        for name in &read {
            self.schemas.ensure(&*self.wps, name).await?;
        }
        let registry = self.schemas.snapshot();
        self.check_shape(draft, base, &mut problems);
        let mut cx = Context {
            base,
            creates: &draft.creates,
            declared_here: draft
                .ops
                .iter()
                .filter_map(|op| match &op.op {
                    Op::Declare { id, .. } => Some((*id, op.scope.clone())),
                    _ => None,
                })
                .collect(),
            deleted_here: draft
                .ops
                .iter()
                .filter_map(|op| match &op.op {
                    Op::Delete { id } => Some(*id),
                    _ => None,
                })
                .collect(),
            tree: None,
            pins: FxHashMap::default(),
        };
        let minted: FxHashSet<DeclaredId> = draft.minted.iter().copied().collect();
        for scoped in &draft.ops {
            let scope = &scoped.scope;
            let entity = scoped.op.entity();
            match &scoped.op {
                Op::Declare { id, .. } => match base.get(id) {
                    Some((owner, _)) if owner != scope => problems.push(
                        Problem::new(ProblemKind::OwnedElsewhere, format!("{id} is declared by {owner}, not {scope}"))
                            .in_scope(scope)
                            .on(entity.clone()),
                    ),
                    Some(_) => {}
                    None if minted.contains(id) => {}
                    None => {
                        if let Some(trail) = self.wps.trail(scope, &entity).await?
                            && let Some(deleted) = trail.deleted_seq
                        {
                            let text = format!("{id} was deleted from {scope} at changeset {deleted}, and stays spent");
                            problems.push(Problem::new(ProblemKind::SpentId, text).in_scope(scope).on(entity.clone()));
                        }
                    }
                },
                Op::Patch { id, .. } | Op::Replace { id, .. } | Op::Delete { id } => {
                    if base.get(id).is_none_or(|(owner, _)| owner != scope) {
                        let text = format!("{id} is not live in {scope} at the changeset this draft was built on");
                        problems.push(Problem::new(ProblemKind::NotLive, text).in_scope(scope).on(entity.clone()));
                    }
                }
                Op::Override { .. } | Op::Suppress { .. } => {}
            }
            if let Op::Declare { name: Some(name), .. }
            | Op::Patch { name: Some(name), .. }
            | Op::Replace { name: Some(name), .. } = &scoped.op
                && name.contains('\0')
            {
                let text = format!("{entity}'s label holds U+0000, which world-proto-server cannot store");
                problems.push(Problem::new(ProblemKind::Nul, text).in_scope(scope).on(entity.clone()));
            }
            for (component, payload) in scoped.op.components().into_iter().flatten() {
                let at = Site { scope, entity: entity.clone(), component, path: "" };
                if holds_nul(payload) {
                    let text = format!("{entity}'s `{component}` holds U+0000, which world-proto-server cannot store");
                    problems.push(at.problem(ProblemKind::Nul, text));
                }
                if let Err(problem) = registry.validate(component, payload) {
                    problems.push((*problem).in_scope(scope).on(entity.clone()).at_field(component, ""));
                }
                for occurrence in walk(component, payload, &*registry) {
                    let at = Site { scope, entity: entity.clone(), component, path: &occurrence.path };
                    if let Some(problem) = self.check_reference(&at, &occurrence.form, &mut cx).await? {
                        problems.push(problem);
                    }
                }
            }
        }
        self.standing_pins(draft, &registry, &mut cx).await?;
        for ((from, to), at) in &cx.pins {
            let mut seqs: Vec<Seq> = at.iter().map(|(seq, _)| *seq).collect();
            seqs.sort();
            seqs.dedup();
            if seqs.len() > 1 {
                let named: Vec<&str> = at.iter().map(|(_, text)| text.as_str()).collect();
                let text = format!(
                    "{from} would pin {to} at changesets {seqs:?} ({}); generation reads one",
                    named.join(", ")
                );
                problems.push(Problem::new(ProblemKind::PinnedTwice, text).in_scope(from));
            }
        }
        problems.extend(self.left_dangling(draft, &registry, &cx).await?);
        check_prologues(draft, &cx, &mut problems);
        let size = serde_json::to_vec(&body_of(draft)).map_or(0, |body| body.len() as u64);
        if size > BODY_LIMIT {
            let text = format!("the changeset is {size} bytes, past world-proto-server's {BODY_LIMIT}");
            problems.push(Problem::new(ProblemKind::TooLarge, text).in_scope(&draft.initiated_on));
        }
        Ok(problems)
    }

    /// Bases for every scope, operations inside the initiating scope's subtree, one operation per entity or record.
    fn check_shape(&self, draft: &ChangesetDraft, base: &UnitSnapshot, problems: &mut Vec<Problem>) {
        let mut needing: Vec<ScopePath> = base.cut.seqs.keys().cloned().collect();
        needing.extend(draft.written());
        needing.sort();
        needing.dedup();
        for scope in needing.iter().filter(|scope| !draft.bases.contains_key(*scope)) {
            let text = format!("the draft states no base for {scope}, which it reads or writes");
            problems.push(Problem::new(ProblemKind::MissingBase, text).in_scope(scope));
        }
        for created in draft.creates.iter().filter(|created| draft.bases.get(&created.path) != Some(&Seq::new(0))) {
            let text = format!("{} is created by the draft, so its base is 0", created.path);
            problems.push(Problem::new(ProblemKind::MissingBase, text).in_scope(&created.path));
        }
        let outside = draft
            .ops
            .iter()
            .map(|op| (&op.scope, Some(op.op.entity())))
            .chain(draft.prologues.keys().map(|scope| (scope, None)));
        for (scope, entity) in outside.filter(|(scope, _)| !draft.initiated_on.holds(scope)) {
            let text = format!("{scope} is outside {}, the scope this draft was initiated on", draft.initiated_on);
            let problem = Problem::new(ProblemKind::OutsideSubtree, text).in_scope(scope);
            problems.push(match entity {
                Some(entity) => problem.on(entity),
                None => problem,
            });
        }
        let mut seen = FxHashSet::default();
        for op in &draft.ops {
            if !seen.insert(op.op.key()) {
                let text = format!(
                    "two operations of this draft write {}, and a changeset cannot say which wins",
                    op.op.key()
                );
                problems.push(Problem::new(ProblemKind::DuplicateWrite, text).in_scope(&op.scope).on(op.op.entity()));
            }
        }
    }

    async fn check_reference(
        &self,
        at: &Site<'_>,
        form: &RefForm,
        cx: &mut Context<'_>,
    ) -> Result<Option<Problem>, WpsError> {
        let (reference, written) = match form {
            RefForm::Text(text) => match text.parse::<EntityRef>() {
                Ok(reference) => (reference, text.clone()),
                Err(fault) => return Ok(Some(at.problem(ProblemKind::BadReference, fault.to_string()))),
            },
            RefForm::TwoPart { generator, key } => {
                if generator.contains('@') || key.contains('@') {
                    let text = format!(
                        "a derivation address in two parts carries no pin, and `{generator}`/`{key}` holds `@`"
                    );
                    return Ok(Some(at.problem(ProblemKind::PinInTwoParts, text)));
                }
                match format!("{generator}/{key}").parse::<EntityId>() {
                    Ok(id) => (EntityRef::new(id, None), format!("{{{generator}, {key}}}")),
                    Err(fault) => return Ok(Some(at.problem(ProblemKind::BadReference, fault.to_string()))),
                }
            }
            RefForm::Local { known: true, .. } => return Ok(None),
            RefForm::Local { handle, .. } => {
                let text = format!("`{handle}` names no sibling inside its template");
                return Ok(Some(at.problem(ProblemKind::Dangling, text)));
            }
            RefForm::Malformed(value) => {
                return Ok(Some(at.problem(ProblemKind::BadReference, format!("{value} is not a reference"))));
            }
        };
        let anchor = reference.id().declared();
        if cx.deleted_here.contains(&anchor) {
            return Ok(Some(
                at.problem(ProblemKind::Dangling, format!("`{written}` names {anchor}, which this draft deletes")),
            ));
        }
        let target = match cx.declared_here.get(&anchor).or_else(|| cx.base.owner_of(reference.id())) {
            Some(scope) => scope.clone(),
            None => {
                // A two-part address carries no pin, so WPS reads it as written bare.
                let asked =
                    if matches!(form, RefForm::TwoPart { .. }) { reference.id().to_string() } else { written.clone() };
                match self.owner_from_wps(at.scope, &asked, &mut cx.tree).await? {
                    Owner::Scope(scope) => scope,
                    Owner::Problem(kind, text) => return Ok(Some(at.problem(kind, text))),
                }
            }
        };
        let two_part = matches!(form, RefForm::TwoPart { .. });
        let kinship = if cx.base.cut.seqs.contains_key(at.scope) && cx.base.cut.seqs.contains_key(&target) {
            Kinship::Kin
        } else {
            let tree = read_tree(self, &mut cx.tree).await?;
            let created = |scope: &ScopePath| cx.creates.iter().any(|new| new.boundary && new.path == *scope);
            classify(at.scope, &target, |scope| tree.is_boundary(scope) || created(scope))
        };
        let problem = match (kinship, reference.pin()) {
            (Kinship::Kin, None | Some(RefPin::Head)) => None,
            (Kinship::Kin, Some(RefPin::Pin(_))) => Some((
                ProblemKind::PinnedInsideBoundary,
                format!(
                    "`{written}` names an entity of {target}, under the same boundary, so it is at head and unpinned"
                ),
            )),
            (Kinship::Lateral, _) if two_part => Some((
                ProblemKind::LateralNeedsVersion,
                format!("`{written}` names {target} under another boundary in two parts, which carry no version"),
            )),
            (Kinship::Lateral, Some(RefPin::Pin(Pin::Changeset(seq)))) => {
                let head = read_tree(self, &mut cx.tree).await?.head(&target).unwrap_or(Seq::new(0));
                cx.pins.entry((at.scope.clone(), target.clone())).or_default().push((*seq, written.clone()));
                (*seq > head)
                    .then(|| (ProblemKind::PinBeyondHead, format!("`{written}` pins {target} past its head {head}")))
            }
            (Kinship::Lateral, Some(RefPin::Pin(Pin::Tag(tag)))) if !self.capabilities.has(Capability::Tags) => Some((
                ProblemKind::TagNotHeld,
                format!("`{written}` names tag {tag}, and no scope holds a tag until world-proto-server builds tags"),
            )),
            (Kinship::Lateral, Some(RefPin::Pin(Pin::Tag(_)))) => None,
            (Kinship::Lateral, _) => Some((
                ProblemKind::LateralNeedsVersion,
                format!(
                    "`{written}` names {target} under another boundary, so it needs `@<changeset>`, never a branch, `@head` or no pin"
                ),
            )),
        };
        Ok(problem.map(|(kind, text)| at.problem(kind, text)))
    }

    /// The scope WPS's owner index names for a reference read from `referrer`, or the problem its resolution names:
    /// a miss, a pin past the head, a tag no scope holds (spec 04 section 3.7, step 2). A pin-rule refusal still names
    /// the owner, and the rule is this check's own to word. A referrer the draft creates is asked about from a scope
    /// that exists, for the owner alone.
    async fn owner_from_wps(
        &self,
        referrer: &ScopePath,
        written: &str,
        tree: &mut Option<ScopeTree>,
    ) -> Result<Owner, WpsError> {
        let nothing = || Owner::Problem(ProblemKind::Dangling, format!("`{written}` names nothing any scope holds"));
        let mut asking = referrer.clone();
        loop {
            let fault = match self.wps.resolve(&asking, written).await {
                Ok(resolution) => return Ok(Owner::Scope(resolution.scope)),
                Err(fault) => fault,
            };
            let stand_in = asking != *referrer;
            match unresolved(&fault) {
                Some((Unresolved::Owned { problem: Some(kind), .. }, text)) if !stand_in => {
                    return Ok(Owner::Problem(kind, text));
                }
                Some((Unresolved::Owned { owner, .. }, _)) => return Ok(Owner::Scope(owner)),
                Some((Unresolved::Undeclared, _)) => return Ok(nothing()),
                Some((Unresolved::NoReferrer, _)) if !stand_in => match read_tree(self, tree).await?.stand_in(referrer)
                {
                    Some(scope) if scope != *referrer => asking = scope,
                    _ => return Ok(nothing()),
                },
                Some((Unresolved::NoReferrer, _)) | None => return Err(fault),
            }
        }
    }

    /// The pins the references a draft leaves standing hold, in each scope it writes: the base's components the draft
    /// neither rewrites nor removes, and the base's live overrides it neither restates nor leaves to be archived.
    /// What the draft writes is pinned by `check_reference`.
    async fn standing_pins(
        &self,
        draft: &ChangesetDraft,
        registry: &RegistrySnapshot,
        cx: &mut Context<'_>,
    ) -> Result<(), WpsError> {
        let ops = entity_ops(draft);
        let restated: FxHashSet<(ScopePath, EntityId)> = draft
            .ops
            .iter()
            .filter(|op| matches!(op.op, Op::Override { .. }))
            .map(|op| (op.scope.clone(), op.op.entity()))
            .collect();
        for scope in draft.written() {
            let Some(snapshot) = cx.base.scopes.get(&scope) else { continue };
            let mut standing: Vec<(&str, &Value)> = Vec::new();
            for entity in &snapshot.entities {
                for (component, payload) in &entity.components {
                    let kept = match ops.get(&entity.id) {
                        None | Some(Op::Override { .. } | Op::Suppress { .. }) => true,
                        Some(Op::Delete { .. } | Op::Replace { .. }) => false,
                        Some(Op::Declare { components, .. } | Op::Patch { components, .. }) => {
                            !components.contains_key(component)
                        }
                    };
                    if kept {
                        standing.push((component, payload));
                    }
                }
            }
            for record in &snapshot.records {
                let Record::Override { target, components } = record else { continue };
                if restated.contains(&(scope.clone(), target.clone())) || archived(target, &ops, registry) {
                    continue;
                }
                standing.extend(components.iter().map(|(component, payload)| (component.as_str(), payload)));
            }
            for (component, payload) in standing {
                for occurrence in walk(component, payload, registry) {
                    let RefForm::Text(text) = &occurrence.form else { continue };
                    let Ok(reference) = text.parse::<EntityRef>() else { continue };
                    let Some(RefPin::Pin(Pin::Changeset(seq))) = reference.pin() else { continue };
                    let anchor = reference.id().declared();
                    let target = match cx.declared_here.get(&anchor).or_else(|| cx.base.owner_of(reference.id())) {
                        Some(target) => target.clone(),
                        None => match self.owner_from_wps(&scope, text, &mut cx.tree).await? {
                            Owner::Scope(target) => target,
                            Owner::Problem(..) => continue,
                        },
                    };
                    cx.pins.entry((scope.clone(), target)).or_default().push((*seq, text.clone()));
                }
            }
        }
        Ok(())
    }

    /// For every entity the draft deletes, each referrer at head, every page of them, that still names it once the
    /// draft applies: WPS's one `LeftDangling` refusal (spec 04 section 3.7, step 6).
    async fn left_dangling(
        &self,
        draft: &ChangesetDraft,
        registry: &RegistrySnapshot,
        cx: &Context<'_>,
    ) -> Result<Vec<Problem>, WpsError> {
        let ops = entity_ops(draft);
        let mut problems = Vec::new();
        let mut deleted: Vec<DeclaredId> = cx.deleted_here.iter().copied().collect();
        deleted.sort_by_key(DeclaredId::to_string);
        for id in deleted {
            let Some((scope, _)) = cx.base.get(&id) else { continue };
            let mut after = None;
            loop {
                let (page, next) = self.wps.referrers(scope, &id.into(), None, after.take()).await?;
                for referrer in page.iter().filter(|referrer| still_names(referrer, id, draft, &ops, registry)) {
                    let text = format!(
                        "deleting {id} would leave {}.{}{} in {} naming nothing",
                        referrer.from, referrer.component, referrer.field_path, referrer.scope
                    );
                    problems.push(Problem::new(ProblemKind::LeftDangling, text).in_scope(scope).on(id.into()));
                }
                match next {
                    Some(next) if !page.is_empty() => after = Some(next),
                    _ => break,
                }
            }
        }
        Ok(problems)
    }
}

/// The scope tree, read once and only when a check needs it.
async fn read_tree<'t, S: Wps>(client: &Client<S>, tree: &'t mut Option<ScopeTree>) -> Result<&'t ScopeTree, WpsError> {
    if tree.is_none() {
        *tree = Some(client.tree().await?);
    }
    Ok(tree.as_ref().expect("read above"))
}

/// A created root states a prologue; a site's or a template's root names its primary frame, an entity the draft
/// declares or the base holds and the draft does not delete (spec 04 sections 3.3 and 3.4).
fn check_prologues(draft: &ChangesetDraft, cx: &Context<'_>, problems: &mut Vec<Problem>) {
    let base = cx.base;
    let created_roots = draft.creates.iter().filter(|created| created.boundary).map(|created| created.path.clone());
    let boundary = |scope: &ScopePath| is_root(scope) || (*scope == base.cut.unit && base.cut.seqs.len() > 1);
    let first_writes =
        draft.written().into_iter().filter(|scope| boundary(scope) && draft.bases.get(scope) == Some(&Seq::new(0)));
    let mut roots: Vec<ScopePath> = created_roots.chain(first_writes).collect();
    roots.sort();
    roots.dedup();
    for root in roots.iter().filter(|root| !draft.prologues.contains_key(*root)) {
        let text = format!("{root} is a root written for the first time, and states no prologue, so it would not load");
        problems.push(Problem::new(ProblemKind::MissingPrologue, text).in_scope(root));
    }
    for (scope, prologue) in &draft.prologues {
        let Some(frame) = &prologue.primary_frame else {
            if is_loadable_root(scope) {
                let text = format!("{scope}'s prologue names no primary frame, so no position in its export resolves");
                problems.push(Problem::new(ProblemKind::MissingPrologue, text).in_scope(scope));
            }
            continue;
        };
        let held = cx.declared_here.contains_key(&frame.declared()) || base.owner_of(frame).is_some();
        if !held || cx.deleted_here.contains(&frame.declared()) {
            let text =
                format!("{scope}'s prologue names {frame} as its primary frame, and the draft leaves no such entity");
            problems.push(Problem::new(ProblemKind::Dangling, text).in_scope(scope).on(frame.clone()));
        }
    }
}
