//! Within `results`, `compare` is `GET /api/runs/compare` (spec 08 section 3.12; brief I1.10, task 33), which exists
//! so that two runs are never read as differing in one input when they differ in three: every input that differs is
//! stated first, each marked as the experiment's design (its varied field, its repeats' seeds, and the workload a
//! varied generator field makes) or as differing outside it, which warns, but for a read's exactness and the
//! compiler's version, provenance that never warns; then what is the same; then the metrics, combined only where every
//! finished run reports one over one window with one unit, aggregation and denominator. Inputs are classified on their
//! canonical values and said only after, in words that tell differing values apart. Runs whose latest attempt did
//! not finish are listed apart and never combined. The two halves are pure, so synthetic rows test them.

use std::ops::RangeInclusive;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::Uri;
use desunits::SimTime;
use run_invocation::Fault;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use serde_json::Value;
use studio_protocol::{
    CompareReply, ComparedMetric, ComparedRun, ComparedValue, InputField, InputRow, InputSection, InputValue,
    MetricRow, NotCompared, Problem, ProblemType, RunInputs, RunState, SameField, SimWindow, Ulid,
};

use crate::AppState;
use crate::problem::{ProblemAnswer, internal, validation_failed};
use crate::scenarios::catalogue;

use super::headline::{choose, pins};
use super::inputs::{Gathered, gather};
use super::stored::attempt;
use super::values::row;
use super::words::{field_value_exact, sim};

/// How many runs a comparison holds, and how many metrics it may name (spec 08 sections 3.7 and 3.8).
const RUNS: RangeInclusive<usize> = 2..=12;
const METRICS: usize = 8;

/// The fields that are provenance a difference in never warns of: a read's exactness and the compiler's version.
const NEVER_WARNS: [&str; 2] = ["exact", "compiler"];

/// The canonical value of `plan-differs` on an attempt without that finding.
const NO_FINDING: &str = "null";

/// What the compared runs' experiment varied, when every run comes from one: its varied fields and whether its
/// repeats vary the seed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Design {
    pub varied: Vec<String>,
    pub repeats: bool,
}

#[derive(Deserialize)]
pub struct CompareQuery {
    ids: Option<String>,
    metrics: Option<String>,
    reference: Option<String>,
}

/// `GET /api/runs/compare?ids=&metrics=&reference=`.
pub async fn compare(
    State(app): State<AppState>,
    uri: Uri,
    query: Result<Query<CompareQuery>, QueryRejection>,
) -> Result<Json<CompareReply>, ProblemAnswer> {
    let instance = uri.path();
    let Query(query) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let ids: Vec<&str> = query.ids.as_deref().unwrap_or_default().split(',').filter(|id| !id.is_empty()).collect();
    if !RUNS.contains(&ids.len()) {
        return Err(validation_failed(format!("`ids` names 2 to 12 runs; this names {}", ids.len()), instance));
    }
    let ids: Vec<Ulid> = ids
        .iter()
        .map(|id| id.parse().map_err(|_| validation_failed(format!("{id} is no run's id"), instance)))
        .collect::<Result<_, _>>()?;
    if let Some(twice) = (1..ids.len()).find(|&at| ids[..at].contains(&ids[at])) {
        return Err(validation_failed(format!("{} is named twice", ids[twice]), instance));
    }
    let named: Option<Vec<String>> = query.metrics.map(|names| names.split(',').map(str::to_owned).collect());
    if named.as_ref().is_some_and(|names| names.is_empty() || names.len() > METRICS) {
        return Err(validation_failed(format!("`metrics` names 1 to {METRICS} metrics"), instance));
    }
    let asked: Option<Ulid> = match query.reference {
        Some(reference) => {
            Some(reference.parse().map_err(|_| validation_failed("`reference` is no run's id", instance))?)
        }
        None => None,
    };
    if asked.as_ref().is_some_and(|reference| !ids.contains(reference)) {
        return Err(validation_failed("`reference` is one of the runs `ids` names", instance));
    }
    let fault = |fault: Fault| internal("compare the runs", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(|failure| fault(failure.into()))?;
    let mut runs: Vec<Gathered> = Vec::with_capacity(ids.len());
    for id in &ids {
        let gathered = gather(&app, &mut conn, id).await.map_err(fault)?;
        let missing = || Problem::new(ProblemType::NotFound, format!("No run has the id {id}.")).at(instance);
        runs.push(gathered.ok_or_else(missing)?);
    }
    let (compared, left): (Vec<&Gathered>, Vec<&Gathered>) =
        runs.iter().partition(|run| run.summary.state == RunState::Finished);
    let reference = asked
        .filter(|asked| compared.iter().any(|run| run.summary.id == *asked))
        .or_else(|| compared.first().map(|run| run.summary.id.clone()))
        .unwrap_or_else(|| ids[0].clone());
    let one_experiment = compared.windows(2).all(|pair| pair[0].experiment.id == pair[1].experiment.id);
    let design = compared.first().filter(|_| one_experiment).map(|run| Design {
        varied: run.experiment.sweep.iter().map(|sweep| sweep.varies.pointer.clone()).collect(),
        repeats: run.experiment.seeds.len() > 1,
    });
    let inputs: Vec<(Ulid, &RunInputs)> = compared.iter().map(|run| (run.summary.id.clone(), &run.inputs)).collect();
    let (differs, same) = compare_inputs(&inputs, design.as_ref());

    let mut results: Vec<(Ulid, Result<Vec<MetricRow>, String>)> = Vec::new();
    let mut declared_by_reference = Vec::new();
    for run in &compared {
        let read = attempt(&mut conn, &run.summary.id, Some(run.summary.attempt))
            .await
            .map_err(|failure| fault(failure.into()))?;
        let Some(read) = read else { continue };
        let instants = read.instants();
        let window = &run.summary.report_window;
        let rows = match instants.window(window.from_sim_ms, window.to_sim_ms) {
            Ok(window) => Ok(read
                .declared
                .iter()
                .map(|metric| row(metric, read.stored.get(&metric.metric), window, instants.whole()))
                .collect()),
            Err(gap) => Err(format!("its report window's end at {} of simtime is not recorded", sim(gap.asked))),
        };
        if run.summary.id == reference {
            declared_by_reference = read.declared.clone();
        }
        results.push((run.summary.id.clone(), rows));
    }
    let declared = |name: &String| {
        results.iter().any(|(_, rows)| rows.as_ref().is_ok_and(|rows| rows.iter().any(|row| &row.metric == name)))
    };
    if let Some(missing) =
        named.as_ref().and_then(|names| names.iter().find(|name| !declared(name))).filter(|_| !results.is_empty())
    {
        return Err(validation_failed(format!("no compared run declared a metric named {missing}"), instance));
    }
    let names = match named {
        Some(names) => names,
        None => {
            let project = compared.iter().find(|run| run.summary.id == reference).map(|run| run.project.clone());
            let pinned = match project {
                Some(project) => pins(&mut conn, &project).await.map_err(|failure| fault(failure.into()))?,
                None => Vec::new(),
            };
            choose(&declared_by_reference, &pinned, None)
        }
    };
    Ok(Json(CompareReply {
        runs: runs
            .iter()
            .map(|run| ComparedRun {
                id: run.summary.id.clone(),
                label: run.summary.label.clone(),
                state: run.summary.state,
                attempt: run.summary.attempt,
                stub: run.stub,
            })
            .collect(),
        differs,
        same,
        not_compared: left
            .iter()
            .map(|run| NotCompared {
                run: run.summary.id.clone(),
                state: run.summary.state,
                stop_reason: run.summary.stop_reason.clone(),
            })
            .collect(),
        reference,
        metrics: compare_metrics(&names, &results),
    }))
}

/// Section 3.12's inputs rules over the compared runs' rows, in the first run's order: a field is the same only when
/// every run states it with one canonical value, else it differs, a run without it reading null; a difference is the
/// experiment's when `design` names its field, else it warns, provenance aside; a run that differs from its check
/// always shows; and a scope read at other changesets names the differing inputs read from it.
pub fn compare_inputs(runs: &[(Ulid, &RunInputs)], design: Option<&Design>) -> (Vec<InputField>, Vec<SameField>) {
    let mut order: Vec<&str> = Vec::new();
    let mut seen: FxHashSet<&str> = FxHashSet::default();
    for (_, inputs) in runs {
        for row in &inputs.rows {
            if seen.insert(row.field.as_str()) {
                order.push(row.field.as_str());
            }
        }
    }
    let mut differs = Vec::new();
    let mut same = Vec::new();
    for field in order {
        let rows: Vec<_> =
            runs.iter().map(|(run, inputs)| (run, inputs.rows.iter().find(|row| row.field == field))).collect();
        let first = rows.iter().find_map(|(_, row)| *row).expect("a field one run states");
        // Equal words can hide unequal values (3.0001 and 3.0002 both say 3): only the canonical values classify.
        let alike = rows.iter().all(|(_, row)| row.is_some_and(|row| row.canonical == first.canonical));
        let flagged =
            field == "plan-differs" && rows.iter().any(|(_, row)| row.is_some_and(|row| row.canonical != NO_FINDING));
        if alike && !flagged {
            same.push(SameField {
                field: field.to_owned(),
                section: first.section,
                label: first.label.clone(),
                value: first.value.clone(),
                clock: first.clock,
            });
            continue;
        }
        let by_experiment = design.is_some_and(|design| {
            design.varied.iter().any(|varied| varied == field)
                || (field == "/run_controls/seed" && design.repeats)
                || (field == "workload"
                    && design.varied.iter().any(|varied| varied.starts_with("/workload/generator/")))
        });
        differs.push(InputField {
            field: field.to_owned(),
            section: first.section,
            label: first.label.clone(),
            by_experiment,
            warns: !by_experiment && !NEVER_WARNS.contains(&field),
            values: rows
                .iter()
                .zip(told_apart(field, &rows))
                .map(|((run, row), value)| InputValue {
                    run: (*run).clone(),
                    value,
                    number: row.and_then(|row| row.number),
                    changeset: row.and_then(|row| row.changeset),
                })
                .collect(),
            changed: Vec::new(),
            clock: first.clock,
            unit: first.unit.clone(),
        });
    }
    let scoped: FxHashMap<String, Vec<String>> = differs.iter().fold(FxHashMap::default(), |mut by_scope, field| {
        let scope = runs.iter().find_map(|(_, inputs)| {
            inputs
                .rows
                .iter()
                .find(|row| row.field == field.field && row.section != InputSection::Provenance)?
                .scope
                .clone()
        });
        if let Some(scope) = scope {
            by_scope.entry(scope.to_string()).or_default().push(field.label.clone());
        }
        by_scope
    });
    for field in &mut differs {
        if let Some(scope) = field.field.strip_prefix("read:") {
            field.changed = scoped.get(scope).cloned().unwrap_or_default();
        }
    }
    (differs, same)
}

/// Each run's value of a differing field in words that tell the values apart: each row's own words, unless two runs
/// whose canonical values differ say the same, when every value is said exactly, a scenario field's number unrounded
/// in its unit and any other row as its canonical figures; a run without the field says none.
fn told_apart(field: &str, rows: &[(&Ulid, Option<&InputRow>)]) -> Vec<Option<String>> {
    let stated: Vec<&InputRow> = rows.iter().filter_map(|(_, row)| *row).collect();
    let blurred = stated.iter().enumerate().any(|(at, one)| {
        stated[at + 1..].iter().any(|other| other.value == one.value && other.canonical != one.canonical)
    });
    let entry = catalogue().items.iter().find(|entry| entry.pointer == field);
    let exactly = |row: &InputRow| match (entry, serde_json::from_str::<Value>(&row.canonical)) {
        (Some(entry), Ok(value)) => field_value_exact(entry, &value),
        _ => row.canonical.clone(),
    };
    rows.iter().map(|(_, row)| row.map(|row| if blurred { exactly(row) } else { row.value.clone() })).collect()
}

/// Section 3.12's windows and kinds over each compared run's metric rows, or why its window could not be read: a
/// metric is comparable only when every run reports it over one window, with one unit, clock, aggregation and
/// denominator.
pub fn compare_metrics(names: &[String], runs: &[(Ulid, Result<Vec<MetricRow>, String>)]) -> Vec<ComparedMetric> {
    let short = |run: &Ulid| run.as_str().chars().take(8).collect::<String>();
    let mut compared = Vec::new();
    for name in names {
        let rows: Vec<(&Ulid, Result<Option<&MetricRow>, &String>)> = runs
            .iter()
            .map(|(run, rows)| (run, rows.as_ref().map(|rows| rows.iter().find(|row| &row.metric == name))))
            .collect();
        let held = rows.iter().find_map(|(run, row)| row.as_ref().ok().copied().flatten().map(|row| (*run, row)));
        let Some((first_run, first)) = held else { continue };
        let mut reason = None;
        for (run, row) in &rows {
            let why = match row {
                Err(why) => Some(format!("run {}: {why}", short(run))),
                Ok(None) => Some(format!("run {} did not declare it", short(run))),
                Ok(Some(row)) if row.window != first.window => Some(format!(
                    "the runs report it over different windows: {} on run {}, {} on run {}",
                    span(first.window),
                    short(first_run),
                    span(row.window),
                    short(run)
                )),
                Ok(Some(row)) if row.unit != first.unit => Some("the runs state it in different units".to_owned()),
                Ok(Some(row)) if row.clock != first.clock => Some("the runs state it on different clocks".to_owned()),
                Ok(Some(row)) if row.aggregation != first.aggregation => {
                    Some("the runs aggregate it differently".to_owned())
                }
                Ok(Some(row)) if row.denominator != first.denominator => {
                    Some("the runs divide it by different denominators".to_owned())
                }
                Ok(Some(_)) => None,
            };
            if why.is_some() {
                reason = why;
                break;
            }
        }
        let empty = SimWindow { from_sim_ms: SimTime::ZERO, to_sim_ms: SimTime::ZERO };
        compared.push(ComparedMetric {
            metric: name.clone(),
            label: first.label.clone(),
            unit: first.unit,
            clock: first.clock,
            aggregation: first.aggregation,
            comparable: reason.is_none(),
            reason,
            values: rows
                .iter()
                .map(|(run, row)| {
                    let row = row.as_ref().ok().copied().flatten();
                    ComparedValue {
                        run: (*run).clone(),
                        value: row.and_then(|row| row.value),
                        window: row.map_or(empty, |row| row.window),
                    }
                })
                .collect(),
        });
    }
    compared
}

fn span(window: SimWindow) -> String {
    format!("{} to {} of simtime", sim(window.from_sim_ms), sim(window.to_sim_ms))
}
