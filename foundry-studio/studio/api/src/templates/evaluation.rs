//! Within `templates`, an `evaluation` is spec 05 section 4.3's steps 6 and 7 as the API runs them, for one numbered
//! evaluation of a draft or for a template's default evaluation behind its thumbnail: the engine evaluates the
//! template read at its changesets on the compute pool, the API names the made document, loads it with foundry's
//! reader and compiles its floor (`floors`, spec 03 section 3.6's one compile) and counts what the compile derived;
//! for a draft, the client checks the site's changeset the plan would make (`check`) beside that load and compile, its
//! problems the evaluation's once a floor is drawn (step 6.12); then it keeps the plan and the floor document in the
//! run-object store, which the page's viewer draws (ruling D121: one renderer), and records the result under the
//! draft's row lock, so an evaluation a later change superseded is never recorded ready. The made document is also
//! compiled by spec 06's flow compiler in validation mode, each problem it finds `flow-invalid` naming its flow entity
//! (spec 06 section 8's request to spec 05). It reads the knobs the draft set and records every knob's value as the
//! engine took it, and where each came from (D120).
//! TODO(overstuffed): it computes an evaluation (`make`, `complete`, blocking), runs one (`run`, `attempt`, the check
//! beside the floor) and records it (`keep`, `finish`); the run and its record want a module of their own.

use std::collections::BTreeMap;
use std::mem::take;
use std::pin::pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use dessim::composition::flowgraph::{Request, Severity as FlowSeverity, compile};
use dessim::worldmap::WorldMap;
use dessim::worldmap::flows::read as read_flows;
use runstore::{RunStorage, RunStore};
use serde_json::{Value, json};
use site_template::{
    DocId, Evaluation, KnobValue, KnobValues, Mint, Prior, Problem as EngineProblem, ProblemCode,
    Severity as EngineSeverity, SourceKey, evaluate,
};
use sqlx::Error as SqlxError;
use studio_protocol::{
    Census, DeclaredId, EvaluationEngine, EvaluationEvent, EvaluationNote, EvaluationNumber, EvaluationProblem,
    EvaluationPrologue, EvaluationState, FrameAxes, GeneratorUsed, JournalKind, KeyChange, KeyChanges, KnobChange,
    KnobDeclaration, KnobSource, KnobsChanged, LiveItem, LiveKind, PanelEvent, PanelEventKind, ScopePath, Seq,
    Severity, Ulid,
};
use tokio::select;
use tokio::time::sleep;
use tracing::{Instrument, Span, info_span};
use worldview_floor::FLOOR_VERSION;
use wps_client::compute::run as on_pool;
use wps_client::wps::Wps;
use wps_client::{Compute, GeneratorSet, WorldStore, WpsError};

use super::check::checked;
use super::phases::Phases;
use super::plan::plan_of;
use super::read::{Read, read};
use super::words::changed_sentence;
use crate::GateState;
use crate::db::{evaluation_column, read_word, word};
use crate::floors;
use crate::journal::{Appended, NewEntry, append};
use crate::map::frames;

/// How often a computation the full pool refused is offered again, and how long it waits between (wall clock).
const POOL_ATTEMPTS: u32 = 30;
const POOL_WAIT: Duration = Duration::from_secs(2);

/// What one evaluation computed, before anything is stored.
pub struct Computed {
    pub ready: bool,
    // ordered: every knob's value and source, in key order as the answer echoes them.
    pub values: BTreeMap<String, KnobValue>,
    pub sources: BTreeMap<String, KnobSource>,
    pub prologue: EvaluationPrologue,
    pub engine: EvaluationEngine,
    pub census: Census,
    pub changes: KeyChanges,
    pub problems: Vec<EvaluationProblem>,
    pub notes: Vec<EvaluationNote>,
    /// Every made entity's source key and id, for the draft's ids.
    pub ids: Vec<(String, DeclaredId)>,
    pub plan: Vec<u8>,
    pub floor: Option<Vec<u8>>,
    /// The frame the template's wall knobs name and where its axes point, for the stage's corner and its words.
    pub axes: Option<FrameAxes>,
    /// Where the computation's wall clock went.
    pub phases: Phases,
}

/// What one computation reads: the template, the knob values set, the draft's ids and document id, the made
/// document's name ("{title}, evaluation {n}", or the title for a thumbnail) and the generators the registry runs.
pub struct Inputs {
    pub read: Arc<Read>,
    pub knobs: BTreeMap<String, KnobValue>,
    pub prior: Prior,
    pub name: String,
    pub generators: GeneratorSet,
}

/// What the engine made, before the API's work on it: the evaluation, with the plan and ids the client check reads.
struct Made {
    evaluation: Evaluation,
    plan: Vec<u8>,
    ids: Vec<(String, DeclaredId)>,
    phases: Phases,
}

/// The engine's evaluation and its plan (blocking: the compute pool's to run).
fn make(inputs: &Inputs) -> Made {
    let mut phases = Phases::default();
    let values = values_of(&inputs.knobs);
    let evaluate = || evaluate(&inputs.read.template, &values, &inputs.prior, &mut Mint);
    let mut evaluation = phases.time("evaluate", evaluate);
    evaluation.prologue.name = Some(inputs.name.clone());
    let plan = phases.time("plan", || plan_of(&evaluation));
    let ids = evaluation.ids().into_iter().map(|(key, id)| (key.to_string(), id)).collect();
    Made { evaluation, plan, ids, phases }
}

/// Whether the engine refused to make the site at all (`template-invalid`): such an evaluation has no document to
/// load, no floor and nothing for the client to check.
fn template_invalid(evaluation: &Evaluation) -> bool {
    evaluation.problems.iter().any(|problem| problem.code == ProblemCode::TemplateInvalid)
}

/// Evaluate and do everything the API adds (blocking: the compute pool's to run).
pub fn compute(inputs: &Inputs) -> Computed {
    let mut made = make(inputs);
    let phases = take(&mut made.phases);
    complete(inputs, &made, phases)
}

/// Everything the API adds to what the engine made, its time kept after `phases`: the document loaded, its flow
/// validated and its floor compiled, the census, and the changes against the template's defaults (blocking).
fn complete(inputs: &Inputs, made: &Made, mut phases: Phases) -> Computed {
    let Inputs { read, knobs, prior, generators, .. } = inputs;
    let Made { evaluation, plan, ids, .. } = made;
    let mut problems: Vec<EvaluationProblem> = evaluation.problems.iter().map(problem).collect();
    let notes: Vec<EvaluationNote> = evaluation
        .notes
        .iter()
        .map(|note| EvaluationNote { ref_: note.reference.clone(), text: note.text.clone() })
        .chain(read.notes.iter().map(|text| EvaluationNote { ref_: None, text: text.clone() }))
        .collect();
    let mut census = Census {
        by_knob: evaluation.census.by_knob.iter().map(|(knob, n)| (knob.to_string(), n.get())).collect(),
        declared: evaluation.census.declared.iter().map(|(name, n)| (name.clone(), n.get())).collect(),
        derived: BTreeMap::new(),
    };
    let values: BTreeMap<String, KnobValue> =
        evaluation.knobs.iter().map(|(key, value)| (key.to_string(), value)).collect();
    let sources = values
        .keys()
        .map(|key| {
            let source = if knobs.contains_key(key) {
                KnobSource::Set
            } else if evaluation.derived.contains(key.as_str()) {
                KnobSource::Derived
            } else {
                KnobSource::Default
            };
            (key.clone(), source)
        })
        .collect();
    let invalid = template_invalid(evaluation);
    let (mut floor, mut axes) = (None, None);
    if !invalid {
        let document = phases.time("document", || evaluation.document(&read.template).to_bytes());
        let loaded = phases.time("load", || floors::load(&document));
        // A site the reader refuses is already `floor-compile-failed`; its flow is checked only once it loads.
        if let Ok(world) = &loaded {
            problems.extend(phases.time("flow", || flow_problems(world)));
        }
        let compiled = loaded.and_then(|world| {
            let compiled = phases.time("compile", || floors::compile(&world))?;
            phases.time("encode", || floors::encode(&compiled)).map(|bytes| (world, bytes))
        });
        match compiled {
            Ok((world, bytes)) => {
                for row in world.entities.iter().filter(|row| row.id.is_derived()) {
                    for component in row.components.iter() {
                        *census.derived.entry(component.to_string()).or_default() += 1;
                    }
                }
                floor = Some(bytes.to_vec());
                let primary = evaluation.prologue.primary_frame.as_ref();
                axes = primary.and_then(|primary| frames(&world, primary).ok()).map(|(_, axes)| axes);
            }
            Err(why) => problems.push(EvaluationProblem {
                severity: Severity::Error,
                code: "floor-compile-failed".into(),
                knob: None,
                ref_: None,
                text: format!("Error: The site these knobs make cannot be drawn: {why}"),
            }),
        }
    }
    let changes = if knobs.is_empty() || invalid {
        KeyChanges::default()
    } else {
        let defaults = || evaluate(&read.template, &KnobValues::new(), &Prior::new(prior.doc), &mut Mint);
        key_changes(evaluation, &phases.time("defaults", defaults))
    };
    Computed {
        ready: floor.is_some(),
        values,
        sources,
        prologue: EvaluationPrologue {
            doc: evaluation.prologue.doc.to_string(),
            kind: evaluation.prologue.kind.clone(),
            name: evaluation.prologue.name.clone(),
            primary_frame: evaluation.prologue.primary_frame.clone(),
        },
        engine: EvaluationEngine {
            version: evaluation.engine.version.clone(),
            expansion: "foundry's world-protocol reader, as linked into this API".into(),
            registry: evaluation.engine.registry.clone(),
            generators: generators_used(generators),
        },
        census,
        changes,
        problems,
        notes,
        ids: ids.clone(),
        plan: plan.clone(),
        floor,
        axes,
        phases,
    }
}

/// Spec 06's validation of a made document, as the simulator's reader loaded it for its floor (one load serves
/// both): each problem the flow compiler finds, as `flow-invalid` naming its flow entity.
pub fn flow_problems(world: &WorldMap) -> Vec<EvaluationProblem> {
    let compiled = compile(world, &read_flows(&world.world), &Request::validation());
    (compiled.problems.iter())
        .map(|found| EvaluationProblem {
            severity: match found.severity {
                FlowSeverity::Error => Severity::Error,
                FlowSeverity::Warning => Severity::Warning,
            },
            code: "flow-invalid".into(),
            knob: None,
            ref_: found.reference.as_ref().map(ToString::to_string),
            text: found.text.clone(),
        })
        .collect()
}

/// The generators `set` names as an evaluation records them, in name order; Create site holds them to the registry's.
pub fn generators_used(set: &GeneratorSet) -> Vec<GeneratorUsed> {
    set.0
        .iter()
        .map(|(name, (version, digest))| GeneratorUsed {
            name: name.clone(),
            version: version.to_string(),
            digest: digest.clone(),
        })
        .collect()
}

/// The engine's knob values from a draft's.
pub fn values_of(knobs: &BTreeMap<String, KnobValue>) -> KnobValues {
    let mut values = KnobValues::new();
    for (key, value) in knobs {
        if let Ok(key) = key.parse() {
            values.set(key, *value);
        }
    }
    values
}

pub fn problem(found: &EngineProblem) -> EvaluationProblem {
    EvaluationProblem {
        severity: match found.severity {
            EngineSeverity::Error => Severity::Error,
            EngineSeverity::Warning => Severity::Warning,
        },
        code: serde_json::to_value(found.code)
            .ok()
            .and_then(|code| code.as_str().map(str::to_owned))
            .unwrap_or_default(),
        knob: found.knob.as_ref().map(ToString::to_string),
        ref_: found.reference.clone(),
        text: found.text.clone(),
    }
}

/// The made entities `evaluation` adds or leaves out against the template's defaults, by source key.
fn key_changes(evaluation: &Evaluation, defaults: &Evaluation) -> KeyChanges {
    let named = |made: &Evaluation| -> BTreeMap<String, (DeclaredId, Option<String>)> {
        made.plan
            .values()
            .flat_map(|plan| plan.declarations.iter())
            .map(|made| (made.key.to_string(), (made.id, made.name.clone())))
            .collect()
    };
    let (now, then) = (named(evaluation), named(defaults));
    let added = now
        .iter()
        .filter(|(key, _)| !then.contains_key(*key))
        .map(|(key, (id, name))| KeyChange { key: key.clone(), id: Some(id.to_string()), name: name.clone() })
        .collect();
    let removed = then
        .iter()
        .filter(|(key, _)| !now.contains_key(*key))
        .map(|(key, (_, name))| KeyChange { key: key.clone(), id: None, name: name.clone() })
        .collect();
    KeyChanges { added, removed }
}

/// Run `work` on the compute pool, offering it again while the pool's queue is full.
pub async fn pooled<T: Send + 'static>(
    pool: &dyn Compute,
    work: impl FnOnce() -> T + Send + Clone + 'static,
) -> Result<T, WpsError> {
    let mut attempt = 0;
    loop {
        match on_pool(pool, work.clone()).await {
            Err(WpsError::ComputeBusy { retry_after }) if attempt < POOL_ATTEMPTS => {
                attempt += 1;
                sleep(retry_after.max(POOL_WAIT)).await;
            }
            answer => return answer,
        }
    }
}

/// A finished evaluation's keys in the run-object store, and its floor's size.
#[derive(Default)]
pub struct Kept {
    pub plan: Option<String>,
    pub floor: Option<(String, u64)>,
}

/// Put what `computed` made under `prefix` (`template-drafts/{draft}/{n}`); a store that refuses is
/// `runstore-unavailable`, and nothing is kept.
pub async fn keep(store: &RunStore, prefix: &str, computed: &mut Computed) -> Kept {
    let mut kept = Kept::default();
    let mut put = Vec::new();
    put.push((format!("{prefix}/plan.json"), Bytes::from(computed.plan.clone())));
    if let Some(floor) = &computed.floor {
        put.push((format!("{prefix}/floor.msgpack"), Bytes::from(floor.clone())));
    }
    for (key, bytes) in put {
        let size = bytes.len() as u64;
        if let Err(fault) = store.put(&key, bytes).await {
            eprintln!("studio-api: the run-object store refused {key}: {fault}");
            computed.ready = false;
            computed.problems.push(EvaluationProblem {
                severity: Severity::Error,
                code: "runstore-unavailable".into(),
                knob: None,
                ref_: None,
                text: "Error: The evaluation could not be stored; it is tried again on your next change.".into(),
            });
            return Kept::default();
        }
        match key.rsplit('/').next() {
            Some("plan.json") => kept.plan = Some(key),
            _ => kept.floor = Some((key, size)),
        }
    }
    kept
}

/// Run evaluation `n` of `draft`, as its row states it, to its end; a fault records it failed. Whichever way it ends,
/// it logs its one timing line, with the outcome its row was committed with and the phases it completed.
pub async fn run<S: Wps>(gate: GateState<S>, draft: Ulid, n: EvaluationNumber) {
    let (started, mut phases) = (Instant::now(), Phases::default());
    let ending = async {
        match attempt(&gate, &draft, n, &mut phases).await {
            Ok(committed) => committed,
            Err(fault) => {
                eprintln!("studio-api: evaluation {n} of draft {draft} failed: {fault}");
                let mut computed = failed(format!("Error: This evaluation could not run: {fault}"), "template-invalid");
                let nothing = Kept::default();
                phases.time_async("finish", finish(&gate, &draft, n, &mut computed, &nothing)).await.ok()
            }
        }
    };
    let committed = ending.instrument(info_span!("template_evaluation", %draft, %n)).await;
    phases.log(&draft, n, committed, started.elapsed());
}

/// An evaluation that stops at `text`, with nothing made.
pub fn failed(text: String, code: &str) -> Computed {
    Computed {
        ready: false,
        values: BTreeMap::new(),
        sources: BTreeMap::new(),
        prologue: EvaluationPrologue { doc: String::new(), kind: None, name: None, primary_frame: None },
        engine: EvaluationEngine {
            version: String::new(),
            expansion: String::new(),
            registry: BTreeMap::new(),
            generators: Vec::new(),
        },
        census: Census::default(),
        changes: KeyChanges::default(),
        problems: vec![EvaluationProblem {
            severity: Severity::Error,
            code: code.into(),
            knob: None,
            ref_: None,
            text,
        }],
        notes: Vec::new(),
        ids: Vec::new(),
        plan: Vec::new(),
        floor: None,
        axes: None,
        phases: Phases::default(),
    }
}

/// Evaluation `n` of `draft` to its end, each phase kept in `phases`: the state its row holds once it ends (`None` when
/// it has no row), or the fault that stopped it before anything was recorded.
async fn attempt<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    n: EvaluationNumber,
    phases: &mut Phases,
) -> Result<Option<EvaluationState>, String> {
    let pool = &gate.app.pool;
    let column = evaluation_column(n).map_err(|fault| fault.to_string())?;
    let row: Option<(String, Value, String, String, Value, String)> = phases
        .time_async(
            "row",
            sqlx::query_as(
                "SELECT d.template, d.template_seqs, d.doc, d.title, e.knobs, e.state \
                 FROM template_drafts d JOIN template_evaluations e ON e.draft = d.id WHERE d.id = $1 AND e.n = $2",
            )
            .bind(draft.as_str())
            .bind(column)
            .fetch_optional(pool),
        )
        .await
        .map_err(|fault| fault.to_string())?;
    let Some((template, seqs, doc, title, knobs, state)) = row else { return Ok(None) };
    if state != word(EvaluationState::InProgress) {
        return read_word(&state).map(Some).map_err(|fault| fault.to_string());
    }
    let template: ScopePath = template.parse().map_err(|fault| format!("{fault}"))?;
    let seqs: BTreeMap<ScopePath, Seq> = serde_json::from_value(seqs).map_err(|fault| fault.to_string())?;
    let knobs: BTreeMap<String, KnobValue> = serde_json::from_value(knobs).map_err(|fault| fault.to_string())?;
    let doc: DocId = doc.parse()?;
    let read = match phases.time_async("read", read(&gate.app.templates, &gate.world, &template, &seqs)).await {
        Ok(read) => read,
        Err(fault) => {
            let mut computed = failed(format!("Error: This template cannot be evaluated: {fault}"), "template-invalid");
            let nothing = Kept::default();
            let finishing = finish(gate, draft, n, &mut computed, &nothing);
            return phases.time_async("finish", finishing).await.map(Some).map_err(|fault| fault.to_string());
        }
    };
    let ids = sqlx::query_as("SELECT key, id FROM template_draft_ids WHERE draft = $1").bind(draft.as_str());
    let ids: Vec<(String, String)> =
        phases.time_async("ids", ids.fetch_all(pool)).await.map_err(|fault| fault.to_string())?;
    let mut prior = Prior::new(doc);
    for (key, id) in ids {
        if let (Ok(key), Ok(id)) = (key.parse::<SourceKey>(), id.parse::<DeclaredId>()) {
            prior.ids.insert(key, id);
        }
    }
    let generators = gate.world.registry().snapshot().generators.clone();
    let name = format!("{title}, evaluation {n}");
    let inputs = Arc::new(Inputs { read, knobs, prior, name: name.clone(), generators });
    let (submitted, span) = (Instant::now(), Span::current());
    let (held, within) = (inputs.clone(), span.clone());
    let making = move || within.in_scope(|| (submitted.elapsed(), make(&held)));
    let (waited, mut made) = pooled(&*gate.app.compute, making).await.map_err(|fault| fault.to_string())?;
    phases.record("pool", waited);
    phases.append(take(&mut made.phases));
    // The client check reads only the plan, so it runs on another of the pool's threads beside the floor's load and
    // compile rather than after them; its problems count only for an evaluation that draws a floor, so a plan the
    // engine refused never starts one.
    let (made, queued) = (Arc::new(made), Instant::now());
    let held = made.clone();
    let completing = move || span.in_scope(|| (queued.elapsed(), complete(&inputs, &held, Phases::default())));
    let completing = pooled(&*gate.app.compute, completing);
    let mut beside = Phases::default();
    let completed = if template_invalid(&made.evaluation) {
        completing.await
    } else {
        with_check(completing, beside.time_async("check", checked(gate, &made.plan, &made.ids, &name))).await
    };
    let (waited, mut computed) = completed.map_err(|fault| fault.to_string())?;
    phases.record("pool", waited);
    phases.append(take(&mut computed.phases));
    phases.append(beside);
    let prefix = format!("template-drafts/{draft}/{n}");
    let kept = phases.time_async("keep", keep(&gate.app.runstore, &prefix, &mut computed)).await;
    let finishing = finish(gate, draft, n, &mut computed, &kept);
    phases.time_async("finish", finishing).await.map(Some).map_err(|fault| fault.to_string())
}

/// The floor's computation, `completing`, and the client check beside it, `checking`: an evaluation that draws a floor
/// carries the check's problems, and one whose floor failed ends as soon as that is known, its check dropped
/// unanswered, so a failure never waits on world-proto-server for problems it would not carry.
async fn with_check<E>(
    completing: impl Future<Output = Result<(Duration, Computed), E>>,
    checking: impl Future<Output = Vec<EvaluationProblem>>,
) -> Result<(Duration, Computed), E> {
    let (mut completing, mut checking) = (pin!(completing), pin!(checking));
    let (completed, found) = select! {
        completed = &mut completing => (completed, None),
        found = &mut checking => (completing.await, Some(found)),
    };
    let (waited, mut computed) = completed?;
    if computed.ready {
        let found = match found {
            Some(found) => found,
            None => checking.await,
        };
        computed.problems.extend(found);
    }
    Ok((waited, computed))
}

/// Record evaluation `n` as `computed` left it, unless a later change superseded it meanwhile: the draft's ids it
/// gave new keys, and for a person's change the `knob.changed` entry; then tell the draft's conversation. Answers the
/// state the row holds once this ends: the one recorded, or the one found when it was no longer in progress.
async fn finish<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    n: EvaluationNumber,
    computed: &mut Computed,
    kept: &Kept,
) -> Result<EvaluationState, SqlxError> {
    let column = evaluation_column(n)?;
    let mut tx = gate.app.pool.begin().await?;
    let (conversation, unjournaled, declarations): (Option<String>, Value, Value) =
        sqlx::query_as("SELECT conversation, unjournaled, declarations FROM template_drafts WHERE id = $1 FOR UPDATE")
            .bind(draft.as_str())
            .fetch_one(&mut *tx)
            .await?;
    let (state, by_person, revision): (String, bool, i32) =
        sqlx::query_as("SELECT state, by_person, revision FROM template_evaluations WHERE draft = $1 AND n = $2")
            .bind(draft.as_str())
            .bind(column)
            .fetch_one(&mut *tx)
            .await?;
    if state != word(EvaluationState::InProgress) {
        return read_word(&state);
    }
    let ended = if computed.ready { EvaluationState::Ready } else { EvaluationState::Failed };
    let floor = kept.floor.as_ref();
    let (values, sources) = if computed.values.is_empty() {
        (None, None)
    } else {
        (Some(json!(computed.values)), Some(json!(computed.sources)))
    };
    sqlx::query(
        "UPDATE template_evaluations SET state = $3, prologue = $4, engine = $5, census = $6, \
             changes_from_template = $7, problems = $8, notes = $9, plan_key = $10, floor_key = $11, \
             floor_bytes = $12, floor_version = $13, axes = $14, knob_values = $15, knob_sources = $16, \
             finished_at = now() \
         WHERE draft = $1 AND n = $2",
    )
    .bind(draft.as_str())
    .bind(column)
    .bind(word(ended))
    .bind(json!(computed.prologue))
    .bind(json!(computed.engine))
    .bind(json!(computed.census))
    .bind(json!(computed.changes))
    .bind(json!(computed.problems))
    .bind(json!(computed.notes))
    .bind(kept.plan.as_deref())
    .bind(floor.map(|(key, _)| key.as_str()))
    .bind(floor.map(|(_, bytes)| *bytes as i64))
    .bind(floor.map(|_| FLOOR_VERSION as i32))
    .bind(computed.axes.as_ref().map(|axes| json!(axes)))
    .bind(values)
    .bind(sources)
    .execute(&mut *tx)
    .await?;
    if computed.ready {
        let (keys, ids): (Vec<String>, Vec<String>) =
            computed.ids.iter().map(|(key, id)| (key.clone(), id.to_string())).unzip();
        sqlx::query(
            "INSERT INTO template_draft_ids (draft, key, id, evaluation) \
             SELECT $1, key, id, $4 FROM UNNEST($2::text[], $3::text[]) AS made (key, id) \
             ON CONFLICT DO NOTHING",
        )
        .bind(draft.as_str())
        .bind(&keys)
        .bind(&ids)
        .bind(column)
        .execute(&mut *tx)
        .await?;
    }
    let conversation: Option<Ulid> = conversation.and_then(|id| id.parse().ok());
    let mut written = None;
    let changes = merged(serde_json::from_value(unjournaled).unwrap_or_default());
    if by_person && !changes.is_empty() {
        sqlx::query("UPDATE template_drafts SET unjournaled = '[]' WHERE id = $1")
            .bind(draft.as_str())
            .execute(&mut *tx)
            .await?;
        if let Some(conversation) = &conversation {
            let declarations: Vec<KnobDeclaration> = serde_json::from_value(declarations).unwrap_or_default();
            let text = changed_sentence(&changes, &declarations, n, &computed.problems);
            let body =
                KnobsChanged { draft: draft.clone(), changes, evaluation: n, problems: computed.problems.clone() };
            let event = PanelEvent { kind: PanelEventKind::KnobChanged, text, body: json!(body) };
            let entry = NewEntry::new(JournalKind::PanelEvent, json!(event));
            if let Ok(Appended::Wrote(entry)) = append(&mut tx, conversation, entry).await {
                written = Some(entry);
            }
        }
    }
    tx.commit().await?;
    if let Some(conversation) = &conversation {
        gate.app.hub.announce(written);
        let event = EvaluationEvent {
            draft: draft.clone(),
            n,
            revision: revision as u32,
            state: ended,
            problems: computed.problems.clone(),
        };
        let item = LiveItem { kind: LiveKind::TemplateEvaluation, body: json!(event) };
        let _ = gate.app.hub.live(conversation, item);
    }
    Ok(ended)
}

/// One change per knob, from its first `from` to its last `to`, a knob set back where it was dropped.
fn merged(changes: Vec<KnobChange>) -> Vec<KnobChange> {
    let mut out: Vec<KnobChange> = Vec::new();
    for change in changes {
        match out.iter_mut().find(|held| held.knob == change.knob) {
            Some(held) => held.to = change.to,
            None => out.push(change),
        }
    }
    out.retain(|change| change.from != change.to);
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use dessim::worldmap::load_from_bytes;
    use serde_json::{Value, json};

    use super::flow_problems;

    /// Spec 06 section 8: the made L-Shape's flow compiles clean, and with its pick node's synchronization input gone
    /// the evaluation carries `flow-invalid` as an error naming that node.
    #[test]
    fn a_made_flow_that_does_not_compile_is_flow_invalid_naming_its_entity() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json");
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let problems_of =
            |document: &Value| flow_problems(&load_from_bytes(&serde_json::to_vec(document).unwrap()).unwrap());
        assert_eq!(problems_of(&document), []);
        let entities = document["entities"].as_array_mut().unwrap();
        let pick = entities
            .iter_mut()
            .find(|entity| entity["name"] == "Pick stations" && entity["components"]["flow.node"].is_object())
            .unwrap();
        pick["components"]["flow.node"]["process"] = json!({ "operation": "pick", "receives_in": "station_bins" });
        let pick = pick["id"].as_str().unwrap().to_owned();
        let found = problems_of(&document);
        let codes: Vec<(&str, Option<&str>)> = found.iter().map(|p| (p.code.as_str(), p.ref_.as_deref())).collect();
        assert_eq!(codes, [("flow-invalid", Some(pick.as_str()))]);
        assert!(found[0].text.starts_with("Error: Pick stations's pick process needs both"), "{}", found[0].text);
    }
}
