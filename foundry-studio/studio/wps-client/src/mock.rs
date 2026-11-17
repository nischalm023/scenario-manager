//! Within `wps-client`, `mock` is the in-memory world-proto-server every consumer's tests use (spec 04 section 3.2):
//! scopes with their flags and units, a log per scope with the base check, declared entities and records at every
//! changeset, the owner index and spent ids, exports plain and flagged, imports, events and a registry whose families
//! keep heads, with the operation identifier (bodies compared canonically), exact replace and scopes phase 3's
//! changeset across a unit (its scope rules, bases, read bases of the tree or of a registry family, creations and
//! generator-set precondition, applied whole or refused whole under one commit place), as world-proto 092d695 has
//! them; only `phase_3` lists them as features, with b9d82a8's structured refusals, which the client's probe needs at
//! start, and the export's flags. Like WPS it reads references only at annotated places: it respells `@head` there,
//! indexes referrers there, resolves a reference under the pin rule, and follows only those references in a flagged
//! export, a declared entity's, a record's and a derived row's alike, as WPS's index holds them before complete
//! annotation traversal. It runs no generation of its own: a test gives it one with `set_generation`, or plants what
//! a generation would leave with `plant`; the flagged export is the child module `flagged_export`. It validates no
//! schema and holds no reference a changeset writes to the state it leaves: a dangling reference, a deletion still
//! referenced and a pin cycle all apply here, and are refused by the real server in the integration suite. Its
//! refusals are WPS's refusal objects, each code with the details WPS names under it (wp:server/DESIGN.md section 10
//! at cd4923516), read back by `refusal`, so a test exercises the reader as well.
//!
//! TODO(overstuffed): one module holds WPS's write path, its reads, its reference index and a fault injector; the
//! faults and the reference reading want modules of their own.

use std::collections::BTreeMap;
use std::mem;
use std::sync::{Mutex, MutexGuard};

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use rustc_hash::{FxHashMap, FxHashSet};
use semver::Version;
use serde_json::{Map, Number, Value, json};
use sha2::{Digest, Sha256};
use studio_protocol::{DeclaredId, EntityId, EntityRef, Pin, RefPin, ScopePath, Seq};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{Sender, channel};

use crate::draft::{Op, tally};
use crate::error::{WpsError, spoken};
use crate::kinship::ancestry;
use crate::refusal::{self, Reading, Refusal, RouteKind};
use crate::value::{CommitPlace, CreatedScope, LogEntry, Prologue};
use crate::walker::{Annotations, annotated_pointers, respell_heads};
use crate::wire::{
    EntityQuery, ExportFlags, RawExport, RefAnnotation, Resolution, Respelled, WireApplied, WireAppliedScope,
    WireChangeset, WireComponent, WireDefinition, WireEntity, WireEvent, WireGenerator, WireGeneratorVersion,
    WireGenerators, WireImport, WireListed, WireProtocol, WireRanGenerator, WireReferrer, WireSchemaVersion, WireScope,
    WireTrail,
};
use crate::wps::{EventStream, PostFailure, Wps};

mod flagged_export;

use flagged_export::FLAGGED_FILE_KEYS;
pub use flagged_export::StandInGeneration;

/// A declared entity as the mock holds it: its id, label and components.
type Held = (DeclaredId, Option<String>, Map<String, Value>);

/// The families of WPS's registry, which stand outside the tree and whose heads a changeset may state as read bases.
const FAMILIES: [&str; 3] = ["registry/adapters", "registry/generators", "registry/lenses"];

#[derive(Clone, Default)]
struct State {
    // ordered: an export lists entries by identity, as WPS's does.
    entities: BTreeMap<String, Held>,
    records: BTreeMap<String, Value>,
    /// What a generation left, as a test planted it (`plant`), before any record is laid over it.
    // ordered: a flagged export lists entities by identity, as WPS's does.
    planted: BTreeMap<String, (EntityId, Map<String, Value>)>,
}

struct Scope {
    parent: Option<ScopePath>,
    boundary: bool,
    unit: ScopePath,
    states: Vec<State>,
    log: Vec<LogEntry>,
    /// The commit place each changeset of the log took, in log order.
    places: Vec<CommitPlace>,
    prologue: Option<Prologue>,
    spent: FxHashMap<String, Seq>,
    created: FxHashMap<String, Seq>,
}

impl Scope {
    fn new(parent: Option<ScopePath>, boundary: bool, unit: ScopePath) -> Self {
        Self {
            parent,
            boundary,
            unit,
            states: vec![State::default()],
            log: Vec::new(),
            places: Vec::new(),
            prologue: None,
            spent: FxHashMap::default(),
            created: FxHashMap::default(),
        }
    }

    fn head(&self) -> Seq {
        Seq::new(self.log.len() as u64)
    }
}

/// What a changeset writes in one scope: the base its author read, its operations and its prologue.
struct Written {
    base: Seq,
    ops: Vec<Op>,
    prologue: Option<Prologue>,
}

impl Default for Written {
    fn default() -> Self {
        Self { base: Seq::new(0), ops: Vec::new(), prologue: None }
    }
}

/// What a changeset names, read off its body: the scopes it writes, those it creates with their flags, and those it
/// only reads, each at the base it states.
struct Plan {
    // ordered: scopes are written, created and answered in the byte order of their paths, as WPS does.
    written: BTreeMap<ScopePath, Written>,
    created: BTreeMap<ScopePath, bool>,
    read: Vec<(ScopePath, Seq)>,
}

/// Faults a test injects, each used once.
#[derive(Default)]
struct Faults {
    lose_answer: u32,
    drop_unapplied: u32,
    /// Changesets answered before the dropped ones begin.
    drop_after: u32,
    postgres_ends: u32,
    registry_moves: u32,
    extra_respelling: Vec<Respelled>,
    unreadable: FxHashSet<ScopePath>,
    /// Scopes another writer makes by name just before the next changeset applies.
    made_before: Vec<(ScopePath, bool)>,
    /// A refusal the next changeset meets whatever it holds, as a rule the server enforces and this client cannot know:
    /// its code and its sentence.
    refusal: Option<(String, String)>,
}

#[derive(Default)]
struct Inner {
    // ordered: `GET /scopes` lists scopes in path order, as WPS's one statement does.
    scopes: BTreeMap<ScopePath, Scope>,
    owners: FxHashMap<String, ScopePath>,
    // ordered: `GET /registry` lists definitions by name.
    registry: BTreeMap<String, WireComponent>,
    generators: BTreeMap<String, Vec<WireGeneratorVersion>>,
    /// Each registry family's head, one never moved standing at 0.
    families: FxHashMap<ScopePath, Seq>,
    /// Each operation identifier with the digest of the body sent under it, its answer and the scope it was
    /// initiated on.
    operations: FxHashMap<String, (String, WireApplied, ScopePath)>,
    events: FxHashMap<ScopePath, Sender<WireEvent>>,
    faults: Faults,
    features: Option<Vec<String>>,
    /// The last commit place taken, none before the first changeset: every applied changeset takes the next, which
    /// its events carry.
    commit: Option<CommitPlace>,
    referrer_page: usize,
    requests: u64,
    exports: u64,
    /// What stands in for WPS's generation in a flagged export, when a test gives one (`set_generation`).
    generation: Option<StandInGeneration>,
}

/// An in-memory world-proto-server.
#[derive(Default)]
pub struct MockWps {
    inner: Mutex<Inner>,
}

/// WPS's refusal object of `code` with its `details`, read back as the client reads a real one.
fn refuse(route: RouteKind, status: u16, code: &str, details: Value, sentence: String) -> WpsError {
    let body = json!({ "error": sentence, "code": code, "details": details }).to_string();
    match refusal::read(route, status, body.as_bytes(), 0) {
        Reading::Error(error) => error,
        Reading::EndedByPostgres { text } | Reading::GeneratorsMoved { text, .. } => {
            WpsError::ServerFault { status, text }
        }
    }
}

/// A changeset's 422.
fn refused(code: &str, details: Value, sentence: String) -> PostFailure {
    PostFailure::Answered(refuse(RouteKind::Other, 422, code, details, format!("changeset refused: {sentence}")))
}

/// WPS's refusal of an upload to import that holds any key only a flagged export writes, naming those it holds: a
/// scope such a file holds in part would read as deletions.
fn flagged_upload_refusal(document: &Value) -> Option<Refusal> {
    let keys: Vec<&str> = FLAGGED_FILE_KEYS.into_iter().filter(|key| document.get(*key).is_some()).collect();
    if keys.is_empty() {
        return None;
    }
    let text = format!(
        "the upload holds {}, which a flagged export writes and no document does — import never takes a flagged \
         export, since a scope it holds in part would read as deletions; import the scope's plain export instead",
        spoken(&keys)
    );
    Some(Refusal { code: "document_flagged_export".to_owned(), details: json!({ "keys": keys }), text })
}

/// An operation's own name, as WPS's refusals spell it.
fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Declare { .. } => "declare",
        Op::Patch { .. } => "patch",
        Op::Replace { .. } => "replace",
        Op::Delete { .. } => "delete",
        Op::Override { .. } => "override",
        Op::Suppress { .. } => "suppress",
    }
}

/// The changesets a stored operation identifier names, each scope it wrote with its changeset, as WPS's refusal of
/// another body lists them.
fn named_changesets(initiated: &ScopePath, applied: &WireApplied) -> Vec<(ScopePath, Seq)> {
    match (applied.scopes.as_slice(), applied.seq) {
        ([], Some(seq)) => vec![(initiated.clone(), seq)],
        (parts, _) => parts.iter().map(|part| (part.scope.clone(), part.seq)).collect(),
    }
}

/// The generator-set precondition's `differences`, each `{ generator, stated, runs }` with `null` for a side that has
/// none, or `None` when the stated set is what would run: every generator a definition names when a written scope
/// carries an anchor, and none otherwise (`storage/apply.rs:1305-1351`).
fn generator_differences(
    stated: &[WireRanGenerator],
    runs: &BTreeMap<String, (Version, String)>,
    generating: bool,
) -> Option<Value> {
    // ordered: the differences are named in the generators' name order, as WPS names them.
    let mut differences: BTreeMap<&str, Value> = BTreeMap::new();
    for one in stated {
        let name = one.generator.as_str();
        let runs = match runs.get(name) {
            Some((version, digest)) if generating && version.to_string() == one.version && *digest == one.digest => {
                continue;
            }
            Some((version, digest)) if generating => json!({ "version": version.to_string(), "digest": digest }),
            _ => Value::Null,
        };
        let stated = json!({ "version": one.version, "digest": one.digest });
        differences.insert(name, json!({ "generator": name, "stated": stated, "runs": runs }));
    }
    for (name, (version, digest)) in runs.iter().filter(|_| generating) {
        if !stated.iter().any(|one| one.generator == *name) {
            let runs = json!({ "version": version.to_string(), "digest": digest });
            differences.insert(name, json!({ "generator": name, "stated": null, "runs": runs }));
        }
    }
    (!differences.is_empty()).then(|| json!({ "differences": differences.into_values().collect::<Vec<_>>() }))
}

fn missing(route: RouteKind, scope: &ScopePath) -> WpsError {
    refuse(route, 404, "unknown_scope", json!({ "scope": scope }), format!("scope {scope} does not exist"))
}

fn generator_family() -> ScopePath {
    "registry/generators".parse().expect("a family's path")
}

impl MockWps {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("the mock's lock")
    }

    /// A mock of world-proto master cd4923516, world-proto#4's merge: the first cut, the conveniences, scopes phase 3's
    /// changeset across a unit, structured refusals and `export_flags`, each listed as a feature word, as the client's
    /// probe and its export need them.
    pub fn phase_3() -> Self {
        let mock = Self::default();
        mock.set_features(&[
            "generation",
            "scope_paths",
            "pin_rule",
            "registry_revision",
            "scope_listing_names",
            "exact_replace",
            "operation_id",
            "scope_changesets",
            "structured_refusals",
            "export_flags",
        ]);
        mock
    }

    /// `GET /protocol`'s `features`, as branch `server/conveniences` lists them.
    pub fn set_features(&self, features: &[&str]) {
        self.lock().features = Some(features.iter().map(|word| (*word).to_owned()).collect());
    }

    /// Define `component` and publish `schema` under it at `version`.
    pub fn publish(&self, component: &str, version: &str, schema: Value, refs: Vec<RefAnnotation>) {
        let mut inner = self.lock();
        let entry = inner.registry.entry(component.to_owned()).or_insert_with(|| WireComponent {
            component: WireDefinition {
                component: component.to_owned(),
                owner: "mock".to_owned(),
                status: "experimental".to_owned(),
                meaning: format!("the mock's {component}"),
                asserts: Some("has".to_owned()),
                generator: None,
            },
            versions: Vec::new(),
        });
        entry.versions.push(WireSchemaVersion {
            component: component.to_owned(),
            version: version.to_owned(),
            schema,
            refs,
        });
    }

    /// Publish a generator's module digest at `version`, moving the generator family's head.
    pub fn publish_generator(&self, generator: &str, version: &str, digest: &str) {
        let mut inner = self.lock();
        let head = inner.families.entry(generator_family()).or_insert(Seq::new(0));
        *head = Seq::new(head.get() + 1);
        let versions = inner.generators.entry(generator.to_owned()).or_default();
        versions.push(WireGeneratorVersion {
            generator: generator.to_owned(),
            version: version.to_owned(),
            digest: digest.to_owned(),
        });
    }

    /// The next changeset applies and its answer is lost.
    pub fn lose_next_answer(&self) {
        self.lock().faults.lose_answer += 1;
    }

    /// The next changeset is not applied and no answer comes.
    pub fn drop_next_unapplied(&self) {
        self.lock().faults.drop_unapplied += 1;
    }

    /// After `answered` more changesets, the next one is not applied and no answer comes.
    pub fn drop_unapplied_after(&self, answered: u32) {
        let faults = &mut self.lock().faults;
        faults.drop_after = answered;
        faults.drop_unapplied += 1;
    }

    /// The next applied changeset's answer reports `respelled` besides what it did respell, as a server owing no
    /// such respelling would.
    pub fn report_respelling(&self, respelled: Respelled) {
        self.lock().faults.extra_respelling.push(respelled);
    }

    /// State that `component` is an input of `generator`, as a definition anchored on a generator does.
    pub fn anchor_generator(&self, component: &str, generator: &str) {
        if let Some(entry) = self.lock().registry.get_mut(component) {
            entry.component.generator = Some(generator.to_owned());
        }
    }

    /// Another writer makes `path` by name, with `boundary`, just before the next changeset applies.
    pub fn make_scope_before_next_changeset(&self, path: &ScopePath, boundary: bool) {
        self.lock().faults.made_before.push((path.clone(), boundary));
    }

    /// The next changeset is answered as one Postgres ended, nothing written.
    pub fn end_next_by_postgres(&self) {
        self.lock().faults.postgres_ends += 1;
    }

    /// The next changeset is refused under `code` with `sentence`, nothing written, as world-proto-server refuses a
    /// body by a rule this client does not check.
    pub fn refuse_next(&self, code: &str, sentence: &str) {
        self.lock().faults.refusal = Some((code.to_owned(), sentence.to_owned()));
    }

    /// The next changeset is answered as one the registry kept moving under.
    pub fn move_registry_next(&self) {
        self.lock().faults.registry_moves += 1;
    }

    /// `scope`'s log answers 503 until further notice.
    pub fn make_log_unreadable(&self, scope: &ScopePath, unreadable: bool) {
        let faults = &mut self.lock().faults;
        if unreadable {
            faults.unreadable.insert(scope.clone());
        } else {
            faults.unreadable.remove(scope);
        }
    }

    /// How many requests the mock has answered.
    pub fn requests(&self) -> u64 {
        self.lock().requests
    }

    /// How many exports the mock has answered, plain and flagged.
    pub fn exports(&self) -> u64 {
        self.lock().exports
    }

    /// How many referrers one page of `GET …/referrers/<id>` holds (WPS's default is 100).
    pub fn set_referrer_page(&self, rows: usize) {
        self.lock().referrer_page = rows.max(1);
    }

    /// How many changesets `scope`'s log holds.
    pub fn head(&self, scope: &ScopePath) -> Option<Seq> {
        self.lock().scopes.get(scope).map(Scope::head)
    }

    fn count(&self) -> MutexGuard<'_, Inner> {
        let mut inner = self.lock();
        inner.requests += 1;
        inner
    }
}

fn listing_row(path: &ScopePath, scope: &Scope) -> WireScope {
    WireScope {
        scope: path.to_string(),
        parent: scope.parent.as_ref().map(ToString::to_string),
        boundary: Some(scope.boundary),
        unit: Some(scope.unit.to_string()),
        head: scope.head().get(),
        name: None,
    }
}

fn document(scope: &Scope, state: &State) -> Value {
    let doc = match &scope.prologue {
        Some(prologue) => {
            let mut doc = Map::new();
            doc.insert("id".to_owned(), json!(prologue.doc));
            if let Some(kind) = &prologue.kind {
                doc.insert("kind".to_owned(), json!(kind));
            }
            if let Some(name) = &prologue.name {
                doc.insert("name".to_owned(), json!(name));
            }
            if !prologue.depends_on.is_empty() {
                doc.insert("depends_on".to_owned(), json!(prologue.depends_on));
            }
            Value::Object(doc)
        }
        None => json!({ "kind": "world" }),
    };
    let mut entities: Vec<Value> = state
        .entities
        .values()
        .map(|(id, name, components)| match name {
            Some(name) => json!({ "id": id, "name": name, "components": components }),
            None => json!({ "id": id, "components": components }),
        })
        .collect();
    entities.extend(state.records.values().cloned());
    let mut document = json!({ "proto_version": "0.7.0", "doc": doc, "entities": entities });
    if let Some(frame) = scope.prologue.as_ref().and_then(|prologue| prologue.primary_frame.as_ref()) {
        document["primary_frame"] = json!(frame);
    }
    document
}

fn record_entry(target: &EntityId, components: Option<&Map<String, Value>>) -> Value {
    match components {
        None => json!({ "id": target, "suppress": true }),
        Some(components) => {
            let text = target.to_string();
            let (generator, key) = text.split_once('/').unwrap_or((text.as_str(), ""));
            json!({ "derivation": { "generator": generator, "key": key }, "components": components })
        }
    }
}

/// The annotations of each component's latest published version, which WPS reads references by.
struct Latest(FxHashMap<String, Vec<RefAnnotation>>);

impl Annotations for Latest {
    fn refs(&self, component: &str) -> &[RefAnnotation] {
        self.0.get(component).map(Vec::as_slice).unwrap_or_default()
    }
}

/// Each annotated reference of `components` naming `target`'s identity, with its pointer and pin.
fn references_to(
    components: &Map<String, Value>,
    target: &EntityId,
    annotations: &Latest,
    prefix: &str,
) -> Vec<(String, String, Option<u64>)> {
    let mut found = Vec::new();
    for (component, payload) in components {
        for pointer in annotated_pointers(payload, annotations.refs(component)) {
            let Some(reference) =
                payload.pointer(&pointer).and_then(Value::as_str).and_then(|text| text.parse::<EntityRef>().ok())
            else {
                continue;
            };
            if reference.id() != target {
                continue;
            }
            let pin = match reference.pin() {
                Some(RefPin::Pin(Pin::Changeset(seq))) => Some(seq.get()),
                _ => None,
            };
            found.push((component.clone(), format!("{prefix}{pointer}"), pin));
        }
    }
    found
}

/// A body as WPS digests an operation identifier's: keys in order (serde's map is sorted) and every number in one
/// spelling, so key order, whitespace and a number's spelling make no difference.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() && float.fract() == 0.0 && float.abs() < 9.0e15 => {
                Value::Number(Number::from(float as i64))
            }
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(fields) => {
            Value::Object(fields.iter().map(|(key, item)| (key.clone(), canonical(item))).collect())
        }
        other => other.clone(),
    }
}

fn body_digest(changeset: &WireChangeset) -> String {
    let value = canonical(&serde_json::to_value(changeset).unwrap_or_default());
    hex::encode(Sha256::digest(serde_json::to_vec(&value).unwrap_or_default()))
}

impl Inner {
    /// The head of `scope` where it is a family of the registry.
    fn family_head(&self, scope: &ScopePath) -> Option<Seq> {
        FAMILIES.contains(&scope.as_str()).then(|| self.families.get(scope).copied().unwrap_or(Seq::new(0)))
    }

    /// The annotations of every component's latest published version.
    fn latest(&self) -> Latest {
        let mut latest = FxHashMap::default();
        for (name, component) in &self.registry {
            let newest = component.versions.iter().max_by_key(|version| Version::parse(&version.version).ok());
            if let Some(newest) = newest {
                latest.insert(name.clone(), newest.refs.clone());
            }
        }
        Latest(latest)
    }

    /// The generators WPS would run: each one a definition names, at its highest published version and module, and
    /// the components naming one, whose carriers are its anchors.
    fn registered(&self) -> (BTreeMap<String, (Version, String)>, FxHashSet<String>) {
        let anchors: FxHashSet<String> = self
            .registry
            .values()
            .filter(|entry| entry.component.generator.is_some())
            .map(|entry| entry.component.component.clone())
            .collect();
        let mut runs = BTreeMap::new();
        for entry in self.registry.values() {
            let Some(generator) = &entry.component.generator else { continue };
            let published = self.generators.get(generator).into_iter().flatten();
            let highest = published
                .filter_map(|one| Some((Version::parse(&one.version).ok()?, one.digest.clone())))
                .max_by(|a, b| a.0.cmp(&b.0));
            if let Some(highest) = highest {
                runs.insert(generator.clone(), highest);
            }
        }
        (runs, anchors)
    }

    /// Read what a changeset names off its body, or refuse it, 422, as WPS's `changeset_scopes::plan` does: the
    /// scopes it writes, each with its base, operations and prologue, the scopes it creates and those it only reads.
    fn plan(&self, changeset: &WireChangeset) -> Result<Plan, PostFailure> {
        let initiating = &changeset.scope;
        let outside = |field: &str, index: Option<usize>, what: String, scope: &ScopePath| {
            let details = json!({ "scope": scope, "initiating": initiating, "field": field, "index": index });
            let sentence = format!(
                "{what} names scope {scope}, which is neither {initiating}, the scope this changeset is initiated on, \
                 nor below it: a changeset initiated on a scope writes that scope and its descendants, never a sibling \
                 or a parent (DESIGN.md §8)"
            );
            refused("outside_the_initiating_scope", details, sentence)
        };
        let mut created: BTreeMap<ScopePath, bool> = BTreeMap::new();
        for new in &changeset.create {
            if !initiating.holds(&new.path) {
                return Err(outside("create", None, "its creation of a scope".to_owned(), &new.path));
            }
            if created.insert(new.path.clone(), new.boundary).is_some() {
                let sentence = format!(
                    "this changeset lists scope {} twice among the scopes it creates, and a scope is created once",
                    new.path
                );
                return Err(refused("created_twice", json!({ "scope": new.path }), sentence));
            }
        }
        let mut written: BTreeMap<ScopePath, Written> = BTreeMap::new();
        for (index, wire) in changeset.ops.iter().enumerate() {
            let path = wire.scope.as_ref().unwrap_or(initiating);
            if !initiating.holds(path) {
                let what = format!("its operation {index} (`{}`)", op_name(&wire.op));
                return Err(outside("ops", Some(index), what, path));
            }
            written.entry(path.clone()).or_default().ops.push(wire.op.clone());
        }
        if let Some(prologue) = &changeset.prologue {
            written.entry(initiating.clone()).or_default().prologue = Some(prologue.clone());
        }
        for (path, prologue) in &changeset.prologues {
            if !initiating.holds(path) {
                return Err(outside("prologues", None, "its prologue of a scope".to_owned(), path));
            }
            let scope = written.entry(path.clone()).or_default();
            if scope.prologue.as_ref().is_some_and(|stated| stated != prologue) {
                let sentence = format!(
                    "this changeset states the prologue of scope {path} twice, and the two disagree: `prologue`, and \
                     the entry in `prologues`"
                );
                return Err(refused("stated_twice", json!({ "scope": path, "fact": "prologue" }), sentence));
            }
            scope.prologue = Some(prologue.clone());
        }
        if written.is_empty() && created.is_empty() {
            written.insert(initiating.clone(), Written::default());
        }
        let mut bases: BTreeMap<ScopePath, Seq> = BTreeMap::from([(initiating.clone(), changeset.base)]);
        for (scope, base) in &changeset.bases {
            if scope == initiating && *base != changeset.base {
                let sentence = format!(
                    "this changeset states the base of scope {scope} twice, and the two disagree: {} as `base`, and \
                     {base} in `bases`",
                    changeset.base
                );
                return Err(refused("stated_twice", json!({ "scope": scope, "fact": "base" }), sentence));
            }
            bases.insert(scope.clone(), *base);
        }
        for (scope, base) in &bases {
            if created.contains_key(scope) && *base != Seq::new(0) {
                let sentence = format!(
                    "this changeset creates scope {scope} and states its base as {base}: a scope a changeset creates \
                     stands at 0 when the changeset begins, so its base is 0 (DESIGN.md §8)"
                );
                return Err(refused("created_base_not_zero", json!({ "scope": scope, "base": base }), sentence));
            }
        }
        for (scope, part) in written.iter_mut().filter(|(scope, _)| !created.contains_key(*scope)) {
            let Some(base) = bases.get(scope) else {
                let sentence = format!(
                    "this changeset writes scope {scope} and states no base for it: every scope a changeset writes \
                     states the head its author read, in `bases`, or in `base` for the scope it is initiated on \
                     (DESIGN.md §8)"
                );
                return Err(refused("base_unstated", json!({ "scope": scope }), sentence));
            };
            part.base = *base;
        }
        let read: Vec<(ScopePath, Seq)> = bases
            .into_iter()
            .filter(|(scope, _)| !written.contains_key(scope) && !created.contains_key(scope))
            .collect();
        let mut named = FxHashSet::default();
        for one in changeset.generators.iter().flatten() {
            if !named.insert(one.generator.as_str()) {
                let sentence = format!(
                    "this changeset names generator `{}` twice in the set it expects to run, and a generator runs at \
                     one version",
                    one.generator
                );
                return Err(refused("expected_twice", json!({ "generator": one.generator }), sentence));
            }
        }
        Ok(Plan { written, created, read })
    }

    /// Apply one changeset to every scope it writes, creating the scopes it creates, or refuse it whole.
    fn apply(&mut self, changeset: &WireChangeset) -> Result<WireApplied, PostFailure> {
        if let Some(operation) = &changeset.operation {
            let digest = body_digest(changeset);
            if let Some((stored, applied, initiated)) = self.operations.get(operation) {
                if *stored == digest {
                    return Ok(WireApplied { replayed: true, ..applied.clone() });
                }
                let named = named_changesets(initiated, applied);
                let each: Vec<String> =
                    named.iter().map(|(scope, seq)| format!("changeset {seq} of scope {scope}")).collect();
                let sentence = format!(
                    "operation `{operation}` already names {}, which was sent with another body — an operation \
                     identifier names one changeset and is never reused, so nothing was applied",
                    each.join(", ")
                );
                let named: Vec<Value> =
                    named.iter().map(|(scope, seq)| json!({ "scope": scope, "changeset": seq })).collect();
                let details = json!({ "operation": operation, "initiated": initiated, "named": named });
                return Err(PostFailure::Answered(refuse(
                    RouteKind::Other,
                    409,
                    "operation_reused",
                    details,
                    sentence,
                )));
            }
        }
        let plan = self.plan(changeset)?;
        let mut units: BTreeMap<ScopePath, (ScopePath, bool)> = BTreeMap::new();
        for (path, boundary) in &plan.created {
            if self.scopes.contains_key(path) {
                let sentence = format!("scope {path} already exists");
                return Err(PostFailure::Answered(refuse(
                    RouteKind::Other,
                    409,
                    "scope_exists",
                    json!({ "scope": path }),
                    sentence,
                )));
            }
            units.insert(path.clone(), self.unit_of_new(path, *boundary, &units)?);
        }
        let mut stale = Vec::new();
        let written = plan.written.iter().map(|(scope, part)| (scope, part.base, false));
        for (scope, base, read) in written.chain(plan.read.iter().map(|(scope, base)| (scope, *base, true))) {
            if plan.created.contains_key(scope) {
                continue;
            }
            // A read base may name a family of the registry, which a changeset never writes.
            let head = match (self.scopes.get(scope), read) {
                (Some(found), _) => found.head(),
                (None, true) => {
                    self.family_head(scope).ok_or_else(|| PostFailure::Answered(missing(RouteKind::Other, scope)))?
                }
                (None, false) => return Err(PostFailure::Answered(missing(RouteKind::Other, scope))),
            };
            if head != base {
                stale.push((scope.clone(), base, head));
            }
        }
        if !stale.is_empty() {
            stale.sort();
            let each: Vec<String> = stale
                .iter()
                .map(|(scope, base, head)| format!("changeset base {base} is not scope {scope}'s head {head}"))
                .collect();
            let sentence = format!("{} — re-read and rebuild against what stands", each.join("; "));
            let stale: Vec<Value> =
                stale.iter().map(|(scope, base, head)| json!({ "scope": scope, "base": base, "head": head })).collect();
            let details = json!({ "stale": stale });
            return Err(PostFailure::Answered(refuse(RouteKind::Other, 409, "stale_base", details, sentence)));
        }
        let mut seen = FxHashSet::default();
        for wire in &changeset.ops {
            if !seen.insert(wire.op.key()) {
                let key = wire.op.key();
                let (id, record) = key.split_once('#').unwrap_or((key.as_str(), ""));
                let what = match record {
                    "override" => "override record",
                    "suppress" => "suppression record",
                    _ => "entity",
                };
                let sentence = format!(
                    "{id}: two operations in one changeset write the same {what} — a changeset carries no order, so it \
                     cannot say which wins"
                );
                return Err(refused("duplicate_op", json!({ "written": id, "what": what }), sentence));
            }
        }
        let latest = self.latest();
        let mut respelled = Vec::new();
        let mut claimed: FxHashMap<String, ScopePath> = FxHashMap::default();
        let mut parts = Vec::new();
        for (path, written) in &plan.written {
            let held = self.scopes.get(path);
            let mut state = held.and_then(|scope| scope.states.last().cloned()).unwrap_or_default();
            for op in &written.ops {
                let mut op = op.clone();
                let entity = op.entity().to_string();
                if let Op::Declare { components, .. }
                | Op::Patch { components, .. }
                | Op::Replace { components, .. }
                | Op::Override { components, .. } = &mut op
                {
                    respelled.extend(respell_heads(&entity, components, &latest));
                }
                self.apply_op(path, held, &mut state, op, &mut claimed)?;
            }
            parts.push((path.clone(), written.base, state, written.ops.clone(), written.prologue.clone()));
        }
        if let Some(stated) = &changeset.generators {
            let (runs, anchors) = self.registered();
            let generating = parts.iter().any(|(_, _, state, ..)| {
                state.entities.values().any(|(_, _, components)| components.keys().any(|name| anchors.contains(name)))
            });
            if let Some(details) = generator_differences(stated, &runs, generating) {
                return Err(PostFailure::Answered(refusal::generators_moved(&details, stated)));
            }
        }
        let mut created = Vec::new();
        for (path, (unit, boundary)) in units {
            let parent = ancestry(&path).get(1).cloned();
            created.push(CreatedScope { scope: path.clone(), parent: parent.clone(), boundary, unit: unit.clone() });
            self.scopes.insert(path, Scope::new(parent, boundary, unit));
        }
        let commit = self.commit.map_or(CommitPlace::new(1), CommitPlace::next);
        self.commit = Some(commit);
        let mut answered = Vec::new();
        for (path, base, state, ops, prologue) in parts {
            let next = Seq::new(base.get() + 1);
            let counted: Vec<&Op> = ops.iter().collect();
            let entry = LogEntry {
                seq: next,
                author: changeset.author.clone(),
                message: changeset.message.clone(),
                tally: tally(&counted),
                applied_at: Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
                operation: changeset.operation.clone(),
            };
            let scope = self.scopes.get_mut(&path).expect("written scopes exist or were created above");
            for op in &ops {
                match op {
                    Op::Delete { id } => {
                        scope.spent.insert(id.to_string(), next);
                    }
                    Op::Declare { id, .. } => {
                        scope.created.entry(id.to_string()).or_insert(next);
                    }
                    _ => {}
                }
            }
            if prologue.is_some() {
                scope.prologue = prologue;
            }
            scope.states.push(state);
            scope.log.push(entry.clone());
            scope.places.push(commit);
            if let Some(events) = self.events.get(&path) {
                let event =
                    WireEvent { seq: next, author: entry.author.clone(), tally: entry.tally, commit: Some(commit) };
                let _ = events.send(event);
            }
            answered.push(WireAppliedScope {
                scope: path,
                base,
                seq: next,
                tally: entry.tally,
                archived: Vec::new(),
                generation: None,
            });
        }
        for (id, owner) in claimed {
            self.owners.insert(id, owner);
        }
        let mut applied = WireApplied { respelled, operation: changeset.operation.clone(), ..WireApplied::default() };
        if let [only] = answered.as_slice() {
            (applied.seq, applied.tally) = (Some(only.seq), Some(only.tally));
        }
        (applied.scopes, applied.created) = (answered, created);
        if let Some(operation) = &changeset.operation {
            let stored = (body_digest(changeset), applied.clone(), changeset.scope.clone());
            self.operations.insert(operation.clone(), stored);
        }
        Ok(applied)
    }

    /// The unit a scope created by name joins or starts, as WPS places it: a boundary starts its own, and a scope that
    /// is none joins its parent's unit where that unit's root is a boundary; a missing parent is refused, 422.
    fn unit_of_new(
        &self,
        path: &ScopePath,
        boundary: bool,
        made: &BTreeMap<ScopePath, (ScopePath, bool)>,
    ) -> Result<(ScopePath, bool), PostFailure> {
        let ancestry = ancestry(path);
        let Some(parent) = ancestry.get(1) else { return Ok((path.clone(), boundary)) };
        let flagged = |unit: &ScopePath| {
            made.get(unit).map(|(_, boundary)| *boundary).or_else(|| self.scopes.get(unit).map(|scope| scope.boundary))
        };
        let parent_unit = made
            .get(parent)
            .map(|(unit, _)| unit.clone())
            .or_else(|| self.scopes.get(parent).map(|scope| scope.unit.clone()));
        let Some(unit) = parent_unit else {
            let first = ancestry
                .iter()
                .rev()
                .find(|prefix| !self.scopes.contains_key(*prefix) && !made.contains_key(*prefix))
                .unwrap_or(parent);
            let sentence = format!(
                "scope {path} cannot be created, because its parent does not exist: {first} is the first of its \
                 ancestors that does not"
            );
            let details = json!({ "scope": path, "missing": first });
            return Err(PostFailure::Answered(refuse(RouteKind::Other, 422, "parent_missing", details, sentence)));
        };
        match !boundary && flagged(&unit) == Some(true) {
            true => Ok((unit, boundary)),
            false => Ok((path.clone(), boundary)),
        }
    }

    /// One operation applied to `state`, the scope `path`'s as the changeset leaves it so far, refused as WPS's gates
    /// refuse it.
    fn apply_op(
        &self,
        path: &ScopePath,
        held: Option<&Scope>,
        state: &mut State,
        op: Op,
        claimed: &mut FxHashMap<String, ScopePath>,
    ) -> Result<(), PostFailure> {
        match op {
            Op::Declare { id, name, components } => {
                let key = id.to_string();
                let owner = self.owners.get(&key).or_else(|| claimed.get(&key));
                if let Some(owner) = owner.filter(|owner| *owner != path) {
                    let sentence = format!(
                        "{id}: declare names an identity scope {owner} owns — a declared identity belongs to the scope \
                         that first declared it, for good, so a reference to it names one entity (DESIGN.md §4); \
                         declare a new identity here instead"
                    );
                    let details = json!({ "scope": path, "entity": id, "owner": owner });
                    return Err(refused("owned_elsewhere", details, sentence));
                }
                if let Some(deleted) = held.and_then(|scope| scope.spent.get(&key)) {
                    let sentence = format!(
                        "{id}: declare names an entity deleted at changeset {deleted} — a deleted identity is not \
                         re-declared, it stays spent"
                    );
                    let details = json!({ "scope": path, "entity": id, "deleted": deleted });
                    return Err(refused("declare_on_deleted", details, sentence));
                }
                let entry = state.entities.entry(key.clone()).or_insert_with(|| (id, None, Map::new()));
                entry.2.extend(components);
                if name.is_some() {
                    entry.1 = name;
                }
                claimed.insert(key, path.clone());
            }
            Op::Patch { id, name, components } => {
                let Some(entry) = state.entities.get_mut(&id.to_string()) else {
                    let sentence = format!("{id}: patch names an entity that is not live — declare it first");
                    return Err(refused("patch_missing_target", json!({ "scope": path, "entity": id }), sentence));
                };
                entry.2.extend(components);
                if name.is_some() {
                    entry.1 = name;
                }
            }
            Op::Replace { id, name, components } => {
                let Some(entry) = state.entities.get_mut(&id.to_string()) else {
                    let sentence = format!(
                        "{id}: replace names an entity that is not live in this scope — a replace states what a live \
                         entity of its own scope carries, so declare it first, or replace it in the scope that holds \
                         it (DESIGN.md §8)"
                    );
                    return Err(refused("replace_missing_target", json!({ "scope": path, "entity": id }), sentence));
                };
                *entry = (id, name, components);
            }
            Op::Delete { id } => {
                if state.entities.remove(&id.to_string()).is_none() {
                    let sentence = format!("{id}: delete names an entity that is not live");
                    return Err(refused("delete_missing_target", json!({ "scope": path, "entity": id }), sentence));
                }
            }
            Op::Override { target, components } => {
                state.records.insert(format!("{target}#override"), record_entry(&target, Some(&components)));
            }
            Op::Suppress { target } => {
                state.records.insert(format!("{target}#suppress"), record_entry(&target, None));
            }
        }
        Ok(())
    }

    /// `POST /scopes`: one scope made by name, with its flag, under a parent that exists.
    fn create(&mut self, path: &ScopePath, boundary: bool) -> Result<WireScope, WpsError> {
        if self.scopes.contains_key(path) {
            let sentence = format!("scope {path} already exists");
            return Err(refuse(RouteKind::CreateScope, 409, "scope_exists", json!({ "scope": path }), sentence));
        }
        let ancestry = ancestry(path);
        let parent = ancestry.get(1).cloned();
        let unit = match &parent {
            Some(parent) => {
                let Some(above) = self.scopes.get(parent) else {
                    let first =
                        ancestry.iter().rev().find(|prefix| !self.scopes.contains_key(*prefix)).unwrap_or(parent);
                    let sentence = format!(
                        "scope {path} cannot be created, because its parent does not exist: {first} is the first of \
                         its ancestors that does not"
                    );
                    let details = json!({ "scope": path, "missing": first });
                    return Err(refuse(RouteKind::CreateScope, 422, "parent_missing", details, sentence));
                };
                let flagged = self.scopes.get(&above.unit).is_some_and(|unit| unit.boundary);
                if !boundary && flagged { above.unit.clone() } else { path.clone() }
            }
            None => path.clone(),
        };
        let scope = Scope::new(parent, boundary, unit);
        let row = listing_row(path, &scope);
        self.scopes.insert(path.clone(), scope);
        Ok(row)
    }

    fn state_at(&self, scope: &ScopePath, at: Option<Seq>) -> Result<(&Scope, &State, Seq), WpsError> {
        let found = self.scopes.get(scope).ok_or_else(|| missing(RouteKind::Other, scope))?;
        let at = at.unwrap_or(found.head());
        // WPS reads a scope's head for a changeset past it; the mock refuses that read, and only it.
        let state = found.states.get(at.get() as usize).ok_or_else(|| WpsError::NotFound {
            text: format!("scope {scope} has no changeset {at}; its head is {}", found.head()),
        })?;
        Ok((found, state, at))
    }
}

impl Wps for MockWps {
    async fn healthz(&self) -> Result<String, WpsError> {
        Ok("mock".to_owned())
    }

    async fn protocol(&self) -> Result<WireProtocol, WpsError> {
        let inner = self.count();
        Ok(WireProtocol { proto_version: "0.7.0".to_owned(), features: inner.features.clone() })
    }

    async fn protocol_defs(&self) -> Result<Value, WpsError> {
        let _inner = self.count();
        Ok(json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": {} }))
    }

    async fn scopes(&self) -> Result<Vec<WireScope>, WpsError> {
        let inner = self.count();
        Ok(inner.scopes.iter().map(|(path, scope)| listing_row(path, scope)).collect())
    }

    async fn scopes_under(&self, under: &ScopePath) -> Result<Vec<WireScope>, WpsError> {
        let inner = self.count();
        let rows = inner.scopes.iter().filter(|(path, _)| under.holds(path));
        Ok(rows.map(|(path, scope)| listing_row(path, scope)).collect())
    }

    async fn create_scope(&self, path: &ScopePath, boundary: bool) -> Result<WireScope, WpsError> {
        self.count().create(path, boundary)
    }

    async fn export_scope(&self, scope: &ScopePath, at: Option<Seq>) -> Result<RawExport, WpsError> {
        let mut inner = self.count();
        inner.exports += 1;
        let (found, state, at) = inner.state_at(scope, at)?;
        let bytes = serde_json::to_vec(&document(found, state)).expect("a document serializes");
        Ok(RawExport { base: at, bytes: bytes.into() })
    }

    async fn export_flagged(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        at: &BTreeMap<ScopePath, Seq>,
    ) -> Result<Bytes, WpsError> {
        let mut inner = self.count();
        inner.exports += 1;
        inner.flagged_export(scope, flags, at)
    }

    async fn entity(&self, scope: &ScopePath, id: &EntityId, at: Option<Seq>) -> Result<WireEntity, WpsError> {
        let inner = self.count();
        let (found, state, at) = inner.state_at(scope, at)?;
        let Some((_, name, components)) = state.entities.get(&id.to_string()) else {
            let sentence = format!("scope {scope} has no entity `{id}` live at {at}");
            let details = json!({ "scope": scope, "entity": id, "at": at });
            return Err(refuse(RouteKind::Other, 404, "entity_not_live", details, sentence));
        };
        Ok(WireEntity {
            id: id.clone(),
            provenance: "declared".to_owned(),
            name: name.clone(),
            components: components.clone(),
            component_versions: Map::new(),
            generation: None,
            created_seq: found.created.get(&id.to_string()).copied().unwrap_or(Seq::new(0)),
            deleted_seq: None,
        })
    }

    async fn trail(&self, scope: &ScopePath, id: &EntityId) -> Result<Option<WireTrail>, WpsError> {
        let inner = self.count();
        let found = inner.scopes.get(scope).ok_or_else(|| missing(RouteKind::Other, scope))?;
        let key = id.to_string();
        Ok(found
            .created
            .get(&key)
            .map(|created| WireTrail { created_seq: *created, deleted_seq: found.spent.get(&key).copied() }))
    }

    async fn entities(
        &self,
        scope: &ScopePath,
        query: &EntityQuery,
    ) -> Result<(Vec<WireListed>, Option<String>), WpsError> {
        let inner = self.count();
        let (_, state, _) = inner.state_at(scope, query.at)?;
        if query.provenance.as_deref() == Some("derived") {
            return Ok((Vec::new(), None));
        }
        let after = query.after.clone().unwrap_or_default();
        let limit = usize::from(query.limit.unwrap_or(100));
        let page: Vec<WireListed> = state
            .entities
            .iter()
            .filter(|(key, _)| key.as_str() > after.as_str())
            .take(limit)
            .map(|(_, (id, name, components))| WireListed {
                id: (*id).into(),
                name: name.clone(),
                component_names: components.keys().cloned().collect(),
            })
            .collect();
        let next = page.last().map(|listed| listed.id.to_string());
        Ok((page, next))
    }

    async fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        _at: Option<Seq>,
        after: Option<String>,
    ) -> Result<(Vec<WireReferrer>, Option<String>), WpsError> {
        let inner = self.count();
        inner.scopes.get(scope).ok_or_else(|| missing(RouteKind::Other, scope))?;
        let latest = inner.latest();
        let mut rows = Vec::new();
        for (path, holder) in &inner.scopes {
            let Some(state) = holder.states.last() else { continue };
            let row = |from: EntityId, (component, field_path, pin): (String, String, Option<u64>)| WireReferrer {
                scope: path.clone(),
                from,
                component,
                field_path,
                pin,
            };
            for (from, _, components) in state.entities.values() {
                for found in references_to(components, id, &latest, "") {
                    rows.push(row((*from).into(), found));
                }
            }
            for record in state.records.values() {
                let (Some(components), Some(derivation)) =
                    (record.get("components").and_then(Value::as_object), record.get("derivation"))
                else {
                    continue;
                };
                let generator = derivation.get("generator").and_then(Value::as_str).unwrap_or_default();
                let key = derivation.get("key").and_then(Value::as_str).unwrap_or_default();
                let Ok(target) = format!("{generator}/{key}").parse::<EntityId>() else { continue };
                for found in references_to(components, id, &latest, "override:") {
                    rows.push(row(target.clone(), found));
                }
            }
        }
        let cursor = |row: &WireReferrer| format!("{}\u{1f}{}\u{1f}{}", row.from, row.component, row.field_path);
        rows.sort_by_key(cursor);
        let page: Vec<WireReferrer> = rows
            .into_iter()
            .filter(|row| after.as_ref().is_none_or(|after| cursor(row) > *after))
            .take(if inner.referrer_page == 0 { 100 } else { inner.referrer_page })
            .collect();
        let next = page.last().map(cursor);
        Ok((page, next))
    }

    /// WPS's resolution (`wp:server/server/src/storage/resolve.rs:121-192`): the owner by the owner index, then the
    /// pin rule between the referring scope's unit and the owner's, then liveness at head or at the pin.
    async fn resolve(&self, scope: &ScopePath, reference: &str) -> Result<Resolution, WpsError> {
        let inner = self.count();
        let referrer = inner.scopes.get(scope).ok_or_else(|| missing(RouteKind::Resolve, scope))?;
        let parsed: EntityRef = reference.parse().map_err(|fault| WpsError::Invalid { text: format!("{fault}") })?;
        let root = parsed.id().declared().to_string();
        let refused = |status: u16, code: &str, details: Value, sentence: String| {
            refuse(RouteKind::Resolve, status, code, details, sentence)
        };
        let Some(owner) = inner.owners.get(&root) else {
            let sentence = format!("`{reference}` names `{root}`, which no scope has declared");
            return Err(refused(404, "resolve_undeclared", json!({ "reference": reference, "root": root }), sentence));
        };
        let held = &inner.scopes[owner];
        let ruled = json!({ "reference": reference, "owner": owner, "scope": scope });
        let pin = match (parsed.pin(), referrer.unit == held.unit) {
            (None | Some(RefPin::Head), true) => None,
            (Some(_), true) => {
                let sentence = format!(
                    "`{reference}` names scope `{owner}` within one unit with scope {scope}: within one unit a \
                     reference is at head, written bare or `@head`, and never pinned (DESIGN.md §5)"
                );
                return Err(refused(422, "resolve_pin_within_unit", ruled, sentence));
            }
            (None | Some(RefPin::Head) | Some(RefPin::Pin(Pin::Branch(_))), false) => {
                let sentence = format!(
                    "`{reference}` names scope `{owner}`, in another unit than scope {scope}, at head or through a \
                     branch: across units a reference names an immutable version, `@<changeset>` (DESIGN.md §5)"
                );
                return Err(refused(422, "resolve_moving_across_units", ruled, sentence));
            }
            (Some(RefPin::Pin(Pin::Tag(_))), false) => {
                let sentence = format!(
                    "`{reference}` names a tag, and scope `{owner}` holds none: no scope holds a tag until tags are \
                     built"
                );
                let details = json!({ "reference": reference, "owner": owner });
                return Err(refused(404, "resolve_tag_not_held", details, sentence));
            }
            (Some(RefPin::Pin(Pin::Changeset(seq))), false) => Some(*seq),
        };
        let live_at =
            |at: Seq| held.states.get(at.get() as usize).is_some_and(|state| state.entities.contains_key(&root));
        match pin {
            None if !live_at(held.head()) => {
                let sentence =
                    format!("`{reference}` names nothing live at the head of scope `{owner}`, which owns its target");
                let details = json!({ "reference": reference, "owner": owner });
                Err(refused(404, "resolve_dangling", details, sentence))
            }
            Some(seq) if seq > held.head() => {
                let sentence = format!(
                    "`{reference}` is pinned at {seq}, past scope `{owner}`'s head {} — a pin names history, and \
                     that scope has not written that changeset (DESIGN.md §5)",
                    held.head()
                );
                let details = json!({ "reference": reference, "pin": seq, "owner": owner, "head": held.head() });
                Err(refused(404, "resolve_pin_beyond_head", details, sentence))
            }
            Some(seq) if !live_at(seq) => {
                let sentence = format!(
                    "`{reference}` pinned at {seq} names nothing live at that changeset of scope `{owner}`, which \
                     owns `{root}`"
                );
                let details = json!({ "reference": reference, "pin": seq, "root": root, "owner": owner });
                Err(refused(404, "resolve_pinned_dangling", details, sentence))
            }
            _ => Ok(Resolution { scope: owner.clone(), id: parsed.id().clone(), pin }),
        }
    }

    async fn changesets(&self, scope: &ScopePath, before: Option<Seq>, limit: u16) -> Result<Vec<LogEntry>, WpsError> {
        let inner = self.count();
        if inner.faults.unreadable.contains(scope) {
            return Err(WpsError::Unavailable {
                attempts: 1,
                last: format!("503: the log of {scope} is not answering"),
            });
        }
        let found = inner.scopes.get(scope).ok_or_else(|| missing(RouteKind::Other, scope))?;
        let before = before.map_or(u64::MAX, Seq::get);
        Ok(found.log.iter().rev().filter(|entry| entry.seq.get() < before).take(usize::from(limit)).cloned().collect())
    }

    async fn changeset_named(&self, scope: &ScopePath, operation: &str) -> Result<Option<LogEntry>, WpsError> {
        let inner = self.count();
        let found = inner.scopes.get(scope).ok_or_else(|| missing(RouteKind::Other, scope))?;
        Ok(found.log.iter().find(|entry| entry.operation.as_deref() == Some(operation)).cloned())
    }

    async fn post_changeset(&self, changeset: &WireChangeset) -> Result<WireApplied, PostFailure> {
        let mut inner = self.count();
        if inner.faults.drop_unapplied > 0 && inner.faults.drop_after > 0 {
            inner.faults.drop_after -= 1;
        } else if inner.faults.drop_unapplied > 0 {
            inner.faults.drop_unapplied -= 1;
            return Err(PostFailure::Lost { detail: "the mock dropped the connection before applying".to_owned() });
        }
        if inner.faults.postgres_ends > 0 {
            inner.faults.postgres_ends -= 1;
            let text = format!(
                "Postgres ended this write of {} as a serialization failure, and nothing was written — send it again \
                 from its bases (DESIGN.md §8)",
                changeset.scope
            );
            return Err(PostFailure::EndedByPostgres { text });
        }
        if inner.faults.registry_moves > 0 {
            inner.faults.registry_moves -= 1;
            let sentence = format!(
                "the registry kept moving under this changeset of scope {}: after 2 attempts it still needed a \
                 generator's turn it had not taken, and nothing was applied — send it again",
                changeset.scope
            );
            let details = json!({ "scope": changeset.scope, "attempts": 2 });
            return Err(PostFailure::Answered(refuse(
                RouteKind::Other,
                409,
                "registry_kept_moving",
                details,
                sentence,
            )));
        }
        if let Some((code, sentence)) = inner.faults.refusal.take() {
            return Err(refused(&code, json!({}), sentence));
        }
        for (path, boundary) in mem::take(&mut inner.faults.made_before) {
            inner.create(&path, boundary).map_err(PostFailure::Answered)?;
        }
        let mut applied = inner.apply(changeset)?;
        applied.respelled.append(&mut inner.faults.extra_respelling);
        if inner.faults.lose_answer > 0 {
            inner.faults.lose_answer -= 1;
            return Err(PostFailure::Lost { detail: "the mock applied the changeset and lost its answer".to_owned() });
        }
        Ok(applied)
    }

    async fn import(&self, scope: &ScopePath, base: Seq, document: &Value) -> Result<WireImport, WpsError> {
        let inner = self.count();
        if let Some(Refusal { code, details, text }) = flagged_upload_refusal(document) {
            return Err(refuse(RouteKind::Other, 422, &code, details, text));
        }
        let (_, state, _) = inner.state_at(scope, Some(base))?;
        let invalid = |text: &str| WpsError::Invalid { text: format!("the uploaded document: {text}") };
        let mut ops = Vec::new();
        let mut stated = FxHashSet::default();
        for entry in document.get("entities").and_then(Value::as_array).into_iter().flatten() {
            let components = entry.get("components").and_then(Value::as_object).cloned();
            if let Some(derivation) = entry.get("derivation") {
                let generator = derivation.get("generator").and_then(Value::as_str).unwrap_or_default();
                let key = derivation.get("key").and_then(Value::as_str).unwrap_or_default();
                let target = format!("{generator}/{key}").parse().map_err(|_| invalid("a derivation"))?;
                ops.push(Op::Override { target, components: components.unwrap_or_default() });
                continue;
            }
            let id = entry.get("id").and_then(Value::as_str).ok_or_else(|| invalid("an entry has no id"))?;
            if entry.get("suppress") == Some(&Value::Bool(true)) {
                ops.push(Op::Suppress { target: id.parse().map_err(|_| invalid("a suppression"))? });
                continue;
            }
            let id: DeclaredId = id.parse().map_err(|_| invalid("an id is not a UUID"))?;
            let name = entry.get("name").and_then(Value::as_str).map(str::to_owned);
            let components = components.unwrap_or_default();
            stated.insert(id.to_string());
            match state.entities.get(&id.to_string()) {
                None => ops.push(Op::Declare { id, name, components }),
                Some((_, live_name, live)) => {
                    let differing: Map<String, Value> = components
                        .into_iter()
                        .filter(|(component, payload)| live.get(component) != Some(payload))
                        .collect();
                    let renamed = name.filter(|name| Some(name) != live_name.as_ref());
                    if !differing.is_empty() || renamed.is_some() {
                        ops.push(Op::Patch { id, name: renamed, components: differing });
                    }
                }
            }
        }
        for (key, (id, _, _)) in &state.entities {
            if !stated.contains(key) {
                ops.push(Op::Delete { id: *id });
            }
        }
        let prologue = document.pointer("/doc/id").and_then(Value::as_str).and_then(|doc| {
            Some(Prologue {
                doc: doc.parse().ok()?,
                kind: document.pointer("/doc/kind").and_then(Value::as_str).map(str::to_owned),
                name: document.pointer("/doc/name").and_then(Value::as_str).map(str::to_owned),
                primary_frame: document
                    .get("primary_frame")
                    .and_then(Value::as_str)
                    .and_then(|frame| frame.parse().ok()),
                depends_on: document
                    .pointer("/doc/depends_on")
                    .and_then(Value::as_array)
                    .map(|depends| depends.iter().filter_map(|doc| doc.as_str()?.parse().ok()).collect())
                    .unwrap_or_default(),
            })
        });
        let counted: Vec<&Op> = ops.iter().collect();
        let tally = tally(&counted);
        let changeset =
            WireChangeset { prologue, ..WireChangeset::of_one_scope(scope.clone(), base, "", "import", ops) };
        Ok(WireImport { changeset, tally, warnings: Vec::new() })
    }

    async fn components(&self) -> Result<Vec<WireDefinition>, WpsError> {
        let inner = self.count();
        Ok(inner.registry.values().map(|entry| entry.component.clone()).collect())
    }

    async fn component(&self, name: &str) -> Result<Option<WireComponent>, WpsError> {
        let inner = self.count();
        Ok(inner.registry.get(name).cloned())
    }

    async fn generators(&self) -> Result<WireGenerators, WpsError> {
        let inner = self.count();
        let generators = inner.generators.keys().map(|name| json!({ "generator": name })).collect();
        let at = inner.family_head(&generator_family()).map_or(0, Seq::get);
        Ok(WireGenerators { at, generators })
    }

    async fn generator(&self, name: &str) -> Result<WireGenerator, WpsError> {
        let inner = self.count();
        let versions = inner.generators.get(name).cloned().unwrap_or_default();
        Ok(WireGenerator { generator: name.to_owned(), versions })
    }

    async fn subscribe(&self, scope: &ScopePath) -> Result<EventStream, WpsError> {
        let mut inner = self.count();
        if !inner.scopes.contains_key(scope) {
            return Err(missing(RouteKind::Other, scope));
        }
        let receiver = inner.events.entry(scope.clone()).or_insert_with(|| channel(256).0).subscribe();
        let events = futures_util::stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => return Some((Ok(event), receiver)),
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                }
            }
        });
        Ok(Box::pin(events))
    }
}
