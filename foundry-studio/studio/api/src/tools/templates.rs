//! Within `tools`, `templates` are the agent's template tools (spec 05 section 3.9; spec 01 section 3.9): the list
//! without its drawings, one template, starting a draft in the turn's New site conversation, setting its knobs as the
//! agent (integers only, ruling D74) and evaluating it again, each answering once the evaluation it started has
//! finished (60 s of wall clock at most); and `site.create_from_template`, which asks: its card pins the input as given
//! (an evaluation never changes) and, once approved, Create site writes it under the approval's id with the approver as
//! author, the tool answering when the write has landed, been refused, or stayed unknown past its window.

use std::time::Duration;

use agent_loop::{Card, Class, Tool, ToolFuture};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::{Extension, Json};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use studio_protocol::{
    ApprovalFacts, BranchName, CreateSite, DraftMode, EvaluationNumber, EvaluationState, FactRow, KnobValue, Location,
    Pin, Problem, ProblemType, SetKnobs, Severity, SitePath, StartDraft, TemplateDetail, TemplateDraftState,
    TemplateEvaluation, Ulid,
};
use tokio::time::{Instant, sleep};
use wps_client::wps::Wps;

use super::{ToolContext, input};
use crate::problem::{ProblemAnswer, internal};
use crate::templates::{KnobSetter, again, create_under, list, one, read_draft, read_evaluation, set_knobs_as, start};

/// How long a template tool waits for the evaluation it started, and how often it looks (wall clock).
const EVALUATION_WAIT: Duration = Duration::from_secs(60);
const LOOK_EVERY: Duration = Duration::from_millis(250);
/// How long Create site's tool waits for the write it started to settle; the write's own worker asks on past it.
const WRITE_WAIT: Duration = Duration::from_secs(300);

fn uri(path: &str) -> Uri {
    path.parse().unwrap_or_else(|_| Uri::from_static("/api/template-drafts"))
}

fn invalid(detail: impl Into<String>) -> Problem {
    Problem::new(ProblemType::ValidationFailed, detail)
}

/// Evaluation `n` of `draft` once it has finished, or as it stands when the wait runs out.
async fn finished<S: Wps>(
    cx: &ToolContext<S>,
    draft: &Ulid,
    n: EvaluationNumber,
) -> Result<TemplateEvaluation, ProblemAnswer> {
    let until = Instant::now() + EVALUATION_WAIT;
    loop {
        let address = uri(&format!("/api/template-drafts/{draft}/evaluations/{n}"));
        let read =
            read_evaluation(State(cx.gate.clone()), address, Ok(Path((draft.to_string(), n.to_string())))).await?.0;
        if read.state != EvaluationState::InProgress || Instant::now() >= until {
            return Ok(read);
        }
        cx.progress(&format!("Evaluating the template draft (evaluation {n})"));
        sleep(LOOK_EVERY).await;
    }
}

/// A template draft's revision, which grows by one with each accepted knob change; the template routes' types carry it
/// as a plain number, so it is converted only where a tool hands it to them or reads it from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct DraftRevision(u32);

/// An evaluation as the model reads it: its number, state and knob values, the census, and its problems and notes.
fn summary(evaluation: &TemplateEvaluation, revision: DraftRevision) -> Value {
    json!({
        "draft": evaluation.draft,
        "revision": revision,
        "evaluation": evaluation.n,
        "state": evaluation.state,
        "knobs": evaluation.knobs,
        "census": evaluation.census,
        "problems": evaluation.problems,
        "notes": evaluation.notes,
    })
}

/// "Evaluation 2 of the draft: ready, 192 relays and 12 VTMs, 1 problem. Nothing written."
fn spoken(result: &Value) -> String {
    let census: Vec<String> = (result["census"]["declared"].as_object().into_iter().flatten())
        .take(4)
        .map(|(kind, count)| format!("{count} {kind}"))
        .collect();
    let problems = result["problems"].as_array().map_or(0, Vec::len);
    let state = result["state"].as_str().unwrap_or("unknown");
    let census = if census.is_empty() { String::new() } else { format!(", {}", census.join(", ")) };
    let plural = if problems == 1 { "" } else { "s" };
    format!(
        "Evaluation {} of the draft: {state}{census}, {problems} problem{plural}. Nothing written.",
        result["evaluation"]
    )
}

struct TemplateList;

impl<S: Wps> Tool<ToolContext<S>> for TemplateList {
    fn name(&self) -> &'static str {
        "template.list"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "List the site templates a new site can be made from, with each one's knobs (key, label, unit, bounds, options) \
         and the census of its default evaluation; `filter` keeps those whose title or text holds its words. Returns \
         { items: [{ location, title, text, knobs, flows, thumbnail: { census, problems } }] }. Link a template in a \
         reply as [its title](ref:scope:{location}), and call a knob by its label, as the Template panel shows it; its \
         key is for template_set_knobs."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "additionalProperties": false, "properties": { "filter": { "type": "string" } } })
    }

    fn describe(&self, _input: &Value) -> String {
        "Reading the site templates".to_owned()
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        format!("Read {} site templates", result["items"].as_array().map_or(0, Vec::len))
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let filter = given["filter"].as_str().map(str::to_lowercase);
            let listed = list(State(cx.gate.clone()), uri("/api/templates")).await?.0;
            let items: Vec<Value> = (listed.items.iter())
                .filter(|card| {
                    filter.as_ref().is_none_or(|filter| {
                        card.title.to_lowercase().contains(filter) || card.text.to_lowercase().contains(filter)
                    })
                })
                .map(|card| {
                    let knobs: Vec<Value> = (card.knobs.iter())
                        .map(|knob| {
                            json!({ "key": knob.key, "label": knob.label, "unit": knob.unit, "min": knob.min,
                                            "max": knob.max, "options": knob.options, "default": knob.default })
                        })
                        .collect();
                    let location = Location::new(card.ref_.clone(), Pin::Branch(BranchName::main()));
                    json!({
                        "location": location, "title": card.title, "text": card.text, "built_from": card.built_from,
                        "tag": card.tag, "seqs": card.seqs, "knobs": knobs, "flows": card.flows,
                        "thumbnail": { "census": card.thumbnail.census, "problems": card.thumbnail.problems },
                    })
                })
                .collect();
            Ok(json!({ "items": items }))
        })
    }
}

struct TemplateRead;

impl<S: Wps> Tool<ToolContext<S>> for TemplateRead {
    fn name(&self) -> &'static str {
        "template.read"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "Read one site template at a pin (`template/site/{t}@main`): its description, knobs, flows and its default \
         evaluation's census and problems. Returns { location, tag, seqs, description, knobs, flows, default }. Link it \
         in a reply as [its title](ref:scope:{location}), and call a knob by its label, as the Template panel shows it."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "additionalProperties": false, "required": ["template"],
                "properties": { "template": { "type": "string", "description": "template/site/{t}@main" } } })
    }

    fn describe(&self, input: &Value) -> String {
        format!("Reading the template {}", input["template"].as_str().unwrap_or("named"))
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        let title = result["description"]["title"].as_str().unwrap_or("the template");
        format!("Read {title}: {} knobs", result["knobs"].as_array().map_or(0, Vec::len))
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let location = given["template"].as_str().unwrap_or_default().to_owned();
            let address = uri(&format!("/api/templates/{location}"));
            let asked = location.parse::<Location>();
            let pin = asked.map_or_else(|_| Pin::Branch(BranchName::main()), |asked| asked.pin().clone());
            let read = one(State(cx.gate.clone()), address, Ok(Path(location))).await?.0;
            let TemplateDetail { ref_, tag, seqs, description, knobs, flows, default } = read;
            Ok(json!({ "location": Location::new(ref_, pin), "tag": tag, "seqs": seqs, "description": description,
                       "knobs": knobs, "flows": flows, "default": default }))
        })
    }
}

struct StartTemplateDraft;

impl<S: Wps> Tool<ToolContext<S>> for StartTemplateDraft {
    fn name(&self) -> &'static str {
        "template.start_draft"
    }

    fn class(&self) -> Class {
        Class::Draft
    }

    fn description(&self) -> &'static str {
        "Start a template draft in this New site conversation from a template at `template/site/{t}@main`, no knob set, \
         and wait for its first evaluation. Writes nothing to any site. Returns { draft, revision, evaluation, state, \
         knobs, census, problems, notes }."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "additionalProperties": false, "properties": {
            "template": { "type": "string", "description": "template/site/{t}@main" },
            "site": { "type": "string", "description": "a made site, to update it (not built yet)" },
        } })
    }

    fn describe(&self, input: &Value) -> String {
        format!("Starting a template draft from {}", input["template"].as_str().unwrap_or("the template"))
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        format!("Started a template draft. {}", spoken(result))
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let template = given["template"].as_str().map(str::to_owned);
            let site = given["site"].as_str().map(str::to_owned);
            let browser_tab = None;
            let body =
                StartDraft { template, site, conversation: Some(cx.conversation.clone()), browser_tab, context: None };
            let body = Bytes::from(json!(body).to_string());
            let (_, Json(started)) =
                start(State(cx.gate.clone()), Extension(cx.identity.clone()), uri("/api/template-drafts"), body)
                    .await?;
            let evaluation = finished(cx, &started.draft, started.evaluation.n).await?;
            Ok(summary(&evaluation, DraftRevision(started.revision)))
        })
    }
}

#[derive(Deserialize)]
struct KnobsInput {
    draft: Ulid,
    revision: DraftRevision,
    knobs: FxHashMap<String, KnobValue>,
}

struct SetTemplateKnobs;

impl<S: Wps> Tool<ToolContext<S>> for SetTemplateKnobs {
    fn name(&self) -> &'static str {
        "template.set_knobs"
    }

    fn class(&self) -> Class {
        Class::Draft
    }

    fn description(&self) -> &'static str {
        "Set knobs of a template draft by key to integers (an option's value for a knob with options), read at the \
         draft's `revision`, and wait for the evaluation that follows. Writes nothing to any site. A knob the person \
         set after that revision answers knob-changed naming who set what. Returns the evaluation with the new revision. \
         The person knows a knob by its label, as the Template panel shows it: say that, never its key."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "additionalProperties": false, "required": ["draft", "revision", "knobs"],
                "properties": {
                    "draft": { "type": "string" },
                    "revision": { "type": "integer", "minimum": 1 },
                    "knobs": { "type": "object", "minProperties": 1, "additionalProperties": { "type": "integer" } },
                } })
    }

    fn describe(&self, input: &Value) -> String {
        let knobs: Vec<String> =
            (input["knobs"].as_object().into_iter().flatten()).map(|(key, value)| format!("{key} {value}")).collect();
        format!("Setting the draft's knobs: {}", knobs.join(", "))
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        format!("Set the knobs. {}", spoken(result))
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let KnobsInput { draft, revision, knobs } = input(&given)?;
            let knobs = knobs.into_iter().map(|(key, value)| (key, Some(value))).collect();
            let setter = KnobSetter::Agent { turn: cx.turn_n };
            let instance = format!("/api/template-drafts/{draft}");
            let set = set_knobs_as(
                &cx.gate,
                &cx.identity,
                draft.clone(),
                SetKnobs { revision: revision.0, knobs },
                setter,
                &instance,
            )
            .await?
            .0;
            let evaluation = finished(cx, &draft, set.evaluation.n).await?;
            Ok(summary(&evaluation, DraftRevision(set.revision)))
        })
    }
}

struct EvaluateTemplate;

impl<S: Wps> Tool<ToolContext<S>> for EvaluateTemplate {
    fn name(&self) -> &'static str {
        "template.evaluate"
    }

    fn class(&self) -> Class {
        Class::Draft
    }

    fn description(&self) -> &'static str {
        "Evaluate a template draft again at its knobs (after the registry's generators changed, say) and wait for it. \
         Writes nothing to any site."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "additionalProperties": false, "required": ["draft"],
                "properties": { "draft": { "type": "string" } } })
    }

    fn describe(&self, _input: &Value) -> String {
        "Evaluating the template draft again".to_owned()
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        spoken(result)
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let draft: Ulid = input(&given["draft"])?;
            let address = uri(&format!("/api/template-drafts/{draft}/evaluate"));
            let (_, Json(set)) = again(State(cx.gate.clone()), address, Ok(Path(draft.to_string()))).await?;
            let evaluation = finished(cx, &draft, set.evaluation.n).await?;
            Ok(summary(&evaluation, DraftRevision(set.revision)))
        })
    }
}

#[derive(Deserialize)]
struct CreateInput {
    draft: Ulid,
    evaluation: EvaluationNumber,
    customer: String,
    slug: String,
    name: String,
}

struct CreateSiteFromTemplate;

impl<S: Wps> Tool<ToolContext<S>> for CreateSiteFromTemplate {
    fn name(&self) -> &'static str {
        "site.create_from_template"
    }

    fn class(&self) -> Class {
        Class::Apply
    }

    fn description(&self) -> &'static str {
        "Create a new site from a template draft's latest ready evaluation without an error, at \
         customer/{customer}/site/{slug} with `name`, in one changeset. A person approves it first: your turn waits on \
         the card. Once made, this conversation belongs to the site. Returns { site, seqs }: link the new site in a \
         reply as [its name](ref:scope:{site}@main)."
    }

    fn input_schema(&self) -> Value {
        let slug = json!({ "type": "string", "pattern": "^[a-z0-9][a-z0-9-]*$" });
        json!({ "type": "object", "additionalProperties": false,
                "required": ["draft", "evaluation", "customer", "slug", "name"],
                "properties": {
                    "draft": { "type": "string" },
                    "evaluation": { "type": "integer", "minimum": 1 },
                    "customer": slug.clone(),
                    "slug": slug,
                    "name": { "type": "string", "minLength": 1 },
                } })
    }

    fn describe(&self, input: &Value) -> String {
        format!("Creating the site {}", input["name"].as_str().unwrap_or("named"))
    }

    fn summarize(&self, input: &Value, result: &Value) -> String {
        let site = result["site"].as_str().unwrap_or("the site");
        format!("Created the site {} at {site}", input["name"].as_str().unwrap_or_default())
    }

    fn card<'a>(&'a self, cx: &'a ToolContext<S>, given: &'a Value) -> ToolFuture<'a, Card> {
        Box::pin(async move {
            let CreateInput { draft, evaluation, customer, slug, name } = input(given)?;
            let name = name.trim().to_owned();
            let site: SitePath =
                format!("customer/{customer}/site/{slug}").parse().map_err(|why| invalid(format!("{why}")))?;
            let address = uri(&format!("/api/template-drafts/{draft}"));
            let read = read_draft(State(cx.gate.clone()), address, Ok(Path(draft.to_string()))).await?.0;
            if read.state != TemplateDraftState::Open || read.mode != DraftMode::Create {
                return Err(Problem::new(ProblemType::DraftNotOpen, "This template draft takes no Create site now."));
            }
            let latest = read.latest.as_ref().map(|latest| latest.n);
            let shown = finished(cx, &draft, evaluation).await?;
            let errors = shown.problems.iter().filter(|problem| problem.severity == Severity::Error).count();
            if latest != Some(evaluation) || shown.state != EvaluationState::Ready || errors > 0 {
                let detail =
                    format!("Evaluation {evaluation} is not this draft's latest ready evaluation without an error.");
                return Err(Problem::new(ProblemType::EvaluationStale, detail));
            }
            let mut display = vec![
                FactRow { label: "Template".into(), value: read.title.clone() },
                FactRow { label: "Site".into(), value: format!("{name}, {site}") },
                FactRow { label: "Evaluation".into(), value: evaluation.to_string() },
            ];
            for (key, value) in &shown.knobs {
                let label =
                    (read.declarations.iter()).find(|knob| &knob.key == key).map_or(key.as_str(), |knob| &knob.label);
                display.push(FactRow { label: "Knob".into(), value: format!("{label}: {value}") });
            }
            for (kind, count) in shown.census.iter().flat_map(|census| census.declared.iter()).take(6) {
                display.push(FactRow { label: "Census".into(), value: format!("{count} {kind}") });
            }
            let facts = ApprovalFacts {
                lead: format!("Create the site {name} at {site} from {}, evaluation {evaluation}.", read.title),
                display,
                notes: vec!["This conversation becomes the site's conversation.".to_owned()],
                action_label: "Create site".to_owned(),
                footnote: None,
            };
            let stored =
                json!({ "draft": draft, "evaluation": evaluation, "customer": customer, "slug": slug, "name": name });
            Ok(Card { summary: format!("Create the site {name}"), stored_input: stored, facts })
        })
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let Some(approval) = cx.approval.clone() else {
                return Err(Problem::new(
                    ProblemType::ApprovalRequired,
                    "Create site runs only once a person approves it.",
                ));
            };
            let CreateInput { draft, evaluation, customer, slug, name } = input(&given)?;
            let create = CreateSite { customer, slug, name, evaluation, browser_tab: None, context: None };
            let instance = format!("/api/template-drafts/{draft}/site");
            let (_, Json(accepted)) =
                create_under(&cx.gate, &cx.identity, draft, create, Some(approval), &instance).await?;
            Ok(settled(cx, &accepted.approval.id).await?)
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(WRITE_WAIT + Duration::from_secs(30))
    }
}

/// The write `approval` executes, once settled: what it made, or the problem that refused it; past the wait, a
/// problem saying its outcome is not yet known, the worker asking on.
async fn settled<S: Wps>(cx: &ToolContext<S>, approval: &Ulid) -> Result<Value, ProblemAnswer> {
    let until = Instant::now() + WRITE_WAIT;
    loop {
        let row: Option<(Option<String>, Option<Value>)> =
            sqlx::query_as("SELECT execution, outcome FROM approvals WHERE id = $1")
                .bind(approval.as_str())
                .fetch_optional(&cx.gate.app.pool)
                .await
                .map_err(|fault| internal("read the approval", fault, "/api/approvals"))?;
        let (execution, outcome) = row.unwrap_or_default();
        match execution.as_deref() {
            Some("ok") => {
                let made = outcome.unwrap_or_default();
                return Ok(json!({ "site": made["site"], "seqs": made["seqs"] }));
            }
            Some("failed" | "interrupted") => {
                let outcome = outcome.unwrap_or_default();
                let problem = serde_json::from_value::<Problem>(outcome.clone()).unwrap_or_else(|_| {
                    let words = outcome["sentence"].as_str().unwrap_or("Create site failed.").to_owned();
                    Problem::new(ProblemType::WpsFault, words)
                });
                return Err(problem.into());
            }
            Some("unknown") => cx.progress("Not yet known whether the site landed; checking again"),
            _ => cx.progress("Writing the site to world-proto-server"),
        }
        if Instant::now() >= until {
            let words = "Not yet known whether the site landed; Agent Studio keeps checking, and the card will say.";
            return Err(Problem::new(ProblemType::WpsOutcomeUnknown, words).into());
        }
        sleep(Duration::from_millis(500)).await;
    }
}

pub fn tools<S: Wps>() -> Vec<Box<dyn Tool<ToolContext<S>>>> {
    vec![
        Box::new(TemplateList),
        Box::new(TemplateRead),
        Box::new(StartTemplateDraft),
        Box::new(SetTemplateKnobs),
        Box::new(EvaluateTemplate),
        Box::new(CreateSiteFromTemplate),
    ]
}
