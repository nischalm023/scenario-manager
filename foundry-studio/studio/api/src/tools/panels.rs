//! Within `tools`, `panels` are spec 03 section 3.3's panel commands as the agent calls them: `panel.show`,
//! `panel.highlight`, `panel.select` and `panel.clear`, each checked and resolved by the API before anything is
//! journaled (the location's pin to its cut, every reference to its marks through `POST /api/resolve`'s rules, a
//! reference that resolves to nothing failing the whole call), then journaled as one `panel.command` entry with the
//! sentence the announcer speaks, and answered with the route, the cut and the marks. One cut serves a site's command:
//! the one read for its location gives its references' marks, its `resolved`, its route and its `as_it_was`. `show`
//! carries the Flow panel's layers and draft (checked against that cut), one run's playback instant, the runs compared
//! and the metrics to headline. A command without a location acts on what the turn's last `show` opened, a list
//! included, kept on the turn's row across its waits, else on the page the turn's message came from with its `kin`.
//! The browser acts on the entry; nothing waits on it.

use agent_loop::{Class, Tool, ToolFuture};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::Uri;
use desunits::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Error as SqlxError;
use studio_protocol::{
    ChangesetOp, DraftState, FlowLayer, GrammarError, GroupMarkCounts, HighlightGroup, HighlightGroupInput,
    JournalKind, JournalSeq, Location, MarkCount, ObjectKind, Panel, PanelAction, PanelCommandEntry,
    PanelCommandResult, PanelShowInput, PanelTarget, Pin, Problem, ProblemType, Ref, Resolution, ResolveRequest,
    ScopePath, ScopeSeq, SitePath, Ulid,
};
use url::form_urlencoded;
use wps_client::wps::Wps;

use super::ToolContext;
use crate::agent::journal;
use crate::db::word;
use crate::drafts;
use crate::map::{SiteAt, map_marks, resolve, site_at};
use crate::problem::ProblemAnswer;

/// The panel commands, one tool each.
struct Command(PanelAction);

/// What a command acts on, once its panel, location and kin are settled.
struct Target {
    panel: Panel,
    location: Option<PanelTarget>,
    kin: Option<String>,
}

/// What a turn's last `show` opened, which its later commands without a location act on: the panel, its location
/// (none for a list) and the kin it was read with; kept on the turn's row (`turns.panel_context`) across its waits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    pub panel: Panel,
    pub location: Option<String>,
    pub kin: Option<String>,
}

fn panel_word(panel: Panel) -> &'static str {
    match panel {
        Panel::Map => "Map",
        Panel::Flow => "Flow",
        Panel::Template => "Template",
        Panel::Scenario => "Scenario",
        Panel::Runs => "Runs",
    }
}

fn problem(type_: ProblemType, detail: impl Into<String>) -> ProblemAnswer {
    Problem::new(type_, detail).into()
}

fn groups_schema() -> Value {
    json!({ "type": "array", "minItems": 1, "maxItems": 8, "items": {
        "type": "object", "additionalProperties": false, "required": ["refs", "note"],
        "properties": {
            "refs": { "type": "array", "minItems": 1, "maxItems": 200, "items": { "type": "string" } },
            "note": { "type": "string", "maxLength": 160, "description": "why, in your words; the announcer reads it" },
        },
    } })
}

impl<S: Wps> Tool<ToolContext<S>> for Command {
    fn name(&self) -> &'static str {
        match self.0 {
            PanelAction::Show => "panel.show",
            PanelAction::Highlight => "panel.highlight",
            PanelAction::Select => "panel.select",
            PanelAction::Clear => "panel.clear",
        }
    }

    fn class(&self) -> Class {
        Class::Page
    }

    fn description(&self) -> &'static str {
        match self.0 {
            PanelAction::Show => {
                "Open a panel for the person: map or flow at a site's location (customer/demo/site/lshape-6/map@main), \
                 template (a template-draft:ULID, or the list), scenario or runs; optionally select one thing and \
                 highlight up to 8 groups of references with a note each. Returns { journaled, route, resolved, groups }."
            }
            PanelAction::Highlight => {
                "Replace your highlights on a panel with up to 8 groups of references, each with a short note saying why. \
                 Without a location it acts on your last panel.show, else on the page the person was on."
            }
            PanelAction::Select => "Select one thing on a panel, as the person would by clicking it.",
            PanelAction::Clear => "Remove your highlights from a panel; the selection stays.",
        }
    }

    fn input_schema(&self) -> Value {
        let panel = json!({ "enum": ["map", "flow", "template", "scenario", "runs"] });
        let location = json!({ "type": "string", "description": "customer/demo/site/lshape-6/map@main, template-draft:01K…, scenario:01K…, run:01K…" });
        let mut properties = json!({ "panel": panel, "location": location });
        let required: Vec<&str> = match self.0 {
            PanelAction::Show => {
                properties["select"] = json!({ "type": "string" });
                properties["highlight"] = groups_schema();
                properties["layers"] = json!({ "type": "array", "minItems": 1, "uniqueItems": true,
                    "items": { "enum": ["material", "agent"] }, "description": "the Flow panel's layers; absent: both" });
                properties["draft"] = json!({ "type": "string", "description": "for the flow panel: a changeset \
                    draft's ULID, of the location's site, drafting or ready, written against the changesets the \
                    location resolves to" });
                properties["sim_ms"] = json!({ "type": "integer", "minimum": 0,
                    "description": "for one run on the runs panel: the playback instant, in simtime milliseconds" });
                properties["compare"] = json!({ "type": "array", "minItems": 2, "maxItems": 12,
                    "items": { "type": "string" }, "description": "for the runs panel: the runs to compare, by ULID" });
                properties["metrics"] = json!({ "type": "array", "minItems": 1, "maxItems": 8,
                    "items": { "type": "string" }, "description": "for the runs panel: the metrics to headline when \
                    the project pins none, by name" });
                vec!["panel"]
            }
            PanelAction::Highlight => {
                properties["highlight"] = groups_schema();
                vec!["highlight"]
            }
            PanelAction::Select => {
                properties["select"] = json!({ "type": "string", "description": "an entity id or an object key" });
                vec!["select"]
            }
            PanelAction::Clear => Vec::new(),
        };
        json!({ "type": "object", "additionalProperties": false, "required": required, "properties": properties })
    }

    fn describe(&self, input: &Value) -> String {
        let panel = input["panel"].as_str().unwrap_or("current");
        match self.0 {
            PanelAction::Show => format!("Opening the {panel} panel"),
            PanelAction::Highlight => format!("Highlighting on the {panel} panel"),
            PanelAction::Select => {
                format!("Selecting {} on the {panel} panel", input["select"].as_str().unwrap_or("one thing"))
            }
            PanelAction::Clear => format!("Clearing the highlights on the {panel} panel"),
        }
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        result["announce"].as_str().map_or_else(|| "Sent the panel command".to_owned(), str::to_owned)
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, input: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move { Ok(command(cx, self.0, input).await?) })
    }
}

/// Settle the panel, location and kin a command acts on: its own location; else, for all but `show`, what the turn's
/// last `show` opened, a list included, kept on the turn's row across its waits; else the page the turn's message came
/// from, with that route's `kin`.
fn target<S: Wps>(cx: &ToolContext<S>, action: PanelAction, input: &Value) -> Result<Target, ProblemAnswer> {
    let named_panel: Option<Panel> = input
        .get("panel")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|why| problem(ProblemType::ValidationFailed, why.to_string()))?;
    let named = input["location"].as_str().map(str::to_owned);
    let shown = cx.shown.lock().ok().and_then(|shown| shown.clone());
    let paged = cx.page.as_ref().and_then(|page| paged(&page.route));
    let (location, kin, remembered) = match (action, named) {
        (_, Some(location)) => (Some(location), None, None),
        (PanelAction::Show, None) => (None, None, None),
        (_, None) => match (shown, paged) {
            (Some(shown), _) => (shown.location, shown.kin, Some(shown.panel)),
            (None, Some((location, kin))) => (Some(location), kin, None),
            (None, None) => (None, None, None),
        },
    };
    let location: Option<PanelTarget> = location
        .map(|text| text.parse())
        .transpose()
        .map_err(|why: GrammarError| problem(ProblemType::RefMalformed, why.to_string()))?;
    let panel = match (named_panel.or(remembered), &location) {
        (Some(panel), _) => panel,
        (None, Some(PanelTarget::Location(location))) if location.scope().slugs().last() == Some("flow") => Panel::Flow,
        (None, Some(PanelTarget::Location(_))) => Panel::Map,
        (None, Some(PanelTarget::Object(object))) => match object.kind() {
            ObjectKind::Run | ObjectKind::Submission => Panel::Runs,
            ObjectKind::Scenario | ObjectKind::Experiment => Panel::Scenario,
            _ => Panel::Template,
        },
        (None, None) => {
            let detail = "There is no panel to act on: name a location, or show a panel first.";
            return Err(problem(ProblemType::NoPanelContext, detail));
        }
    };
    if action != PanelAction::Show && location.is_none() && matches!(panel, Panel::Map | Panel::Flow) {
        return Err(problem(ProblemType::NoPanelContext, "There is no site location to act on: name one."));
    }
    Ok(Target { panel, location, kin })
}

/// A page route's world location and its `kin`, when the page showed one (`/w/…`).
fn paged(route: &str) -> Option<(String, Option<String>)> {
    let shown = route.strip_prefix("/w/")?;
    let (location, query) = shown.split_once('?').unwrap_or((shown, ""));
    let kin = form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == "kin").map(|(_, kin)| kin.into_owned());
    Some((location.to_owned(), kin))
}

/// The site root and pin of a world location, and the view's scope (`map` or `flow`) under it.
fn world(location: &Location, panel: Panel) -> Result<(SitePath, Pin), ProblemAnswer> {
    let slugs: Vec<&str> = location.scope().slugs().collect();
    let site = slugs.get(..4).map(|root| root.join("/")).and_then(|root| root.parse::<SitePath>().ok());
    match (site, slugs.len()) {
        (Some(site), 4) => Ok((site, location.pin().clone())),
        (Some(site), 5) if slugs[4] == "map" || slugs[4] == "flow" => Ok((site, location.pin().clone())),
        _ => Err(problem(
            ProblemType::PanelMismatch,
            format!(
                "The {} panel shows a site's scopes, and {} is not one of them.",
                panel_word(panel),
                location.scope()
            ),
        )),
    }
}

/// `show`'s panel-specific inputs, read (spec 03 section 3.3's schema): the Flow panel's layers and draft, one run's
/// playback instant, the runs compared and the metrics to headline.
#[derive(Default)]
struct Extras {
    layers: Option<Vec<FlowLayer>>,
    draft: Option<Ulid>,
    sim_ms: Option<SimTime>,
    compare: Option<Vec<Ulid>>,
    metrics: Option<Vec<String>>,
}

impl Extras {
    fn read(input: &Value) -> Result<Self, ProblemAnswer> {
        let shown: PanelShowInput = serde_json::from_value(input.clone())
            .map_err(|why| problem(ProblemType::ValidationFailed, why.to_string()))?;
        let ulid = |text: &String| {
            text.parse::<Ulid>().map_err(|why| problem(ProblemType::RefMalformed, format!("`{text}`: {why}")))
        };
        Ok(Self {
            layers: shown.layers,
            draft: shown.draft.as_ref().map(ulid).transpose()?,
            sim_ms: shown.sim_ms,
            compare: shown.compare.map(|runs| runs.iter().map(ulid).collect()).transpose()?,
            metrics: shown.metrics,
        })
    }
}

/// Each panel-specific input on the panel it is for, else `panel-mismatch`.
fn fits(target: &Target, extras: &Extras) -> Result<(), ProblemAnswer> {
    let one_run = matches!(&target.location, Some(PanelTarget::Object(object)) if object.kind() == ObjectKind::Run);
    let why = match () {
        _ if target.panel != Panel::Flow && (extras.layers.is_some() || extras.draft.is_some()) => {
            "`layers` and `draft` are the Flow panel's"
        }
        _ if extras.sim_ms.is_some() && !(target.panel == Panel::Runs && one_run) => {
            "`sim_ms` is one run's playback instant, on the Runs panel at that run"
        }
        _ if target.panel != Panel::Runs && (extras.compare.is_some() || extras.metrics.is_some()) => {
            "`compare` and `metrics` are the Runs panel's"
        }
        _ if extras.compare.is_some() && target.location.is_some() => {
            "`compare` opens the comparison, which takes no location"
        }
        _ => return Ok(()),
    };
    Err(problem(
        ProblemType::PanelMismatch,
        format!("{why}, and this command shows the {} panel.", panel_word(target.panel)),
    ))
}

/// Spec 03 section 3.3, step 3: the draft a Flow `show` overlays exists, is the shown site's, is drafting or ready, and
/// was written against the changesets `at` resolved on every scope its operations touch (all of its cut's, when the
/// map places none of their targets).
async fn drafted<S: Wps>(cx: &ToolContext<S>, draft: &Ulid, site: &SitePath, at: &SiteAt) -> Result<(), ProblemAnswer> {
    let read = drafts::load(&cx.gate.app.pool, draft).await;
    let found = read.map_err(|fault| problem(ProblemType::Internal, format!("The draft was not read: {fault}")))?;
    let Some(found) = found else {
        return Err(problem(ProblemType::LocationUnresolved, format!("There is no changeset draft {draft}.")));
    };
    if &found.site != site {
        let detail = format!("The draft {draft} is a draft of {}, not of {site}.", found.site);
        return Err(problem(ProblemType::PanelMismatch, detail));
    }
    if !matches!(found.state, DraftState::Drafting | DraftState::Ready) {
        let detail = format!("The draft {draft} is {}, so it is not drawn.", word(found.state));
        return Err(problem(ProblemType::StaleDraft, detail));
    }
    let owners = at.loaded.as_ref().as_ref().ok().map(|loaded| &loaded.owners);
    let touched: Vec<&ScopePath> = (found.ops.iter())
        .filter_map(|op| match op {
            ChangesetOp::Patch { target, .. } => owners.and_then(|owners| owners.get(&target.declared())),
        })
        .collect();
    for (scope, base) in &found.cut.seqs {
        let read = at.cut.seqs.get(scope);
        if (touched.is_empty() || touched.contains(&scope)) && read != Some(base) {
            let read = read.map_or_else(|| "no changeset".to_owned(), |seq| format!("changeset {seq}"));
            let detail = format!(
                "The draft {draft} was written against {scope} at changeset {base}; this command reads {read}."
            );
            return Err(problem(ProblemType::PanelMismatch, detail));
        }
    }
    Ok(())
}

async fn command<S: Wps>(cx: &ToolContext<S>, action: PanelAction, input: Value) -> Result<Value, ProblemAnswer> {
    let target = target(cx, action, &input)?;
    let groups: Vec<HighlightGroupInput> = match input.get("highlight") {
        Some(groups) => serde_json::from_value(groups.clone())
            .map_err(|why| problem(ProblemType::ValidationFailed, why.to_string()))?,
        None => Vec::new(),
    };
    let extras = match action {
        PanelAction::Show => Extras::read(&input)?,
        _ => Extras::default(),
    };
    fits(&target, &extras)?;
    let select = input["select"].as_str().map(str::to_owned);
    let instance = "/api/resolve";
    let (mut resolved, mut name, mut frozen, mut cut) = (Vec::new(), String::new(), None, None);
    let path = match (&target.panel, &target.location) {
        (Panel::Map | Panel::Flow, Some(PanelTarget::Location(location))) => {
            let (site, pin) = world(location, target.panel)?;
            let view = if target.panel == Panel::Map { "map" } else { "flow" };
            let at = site_at(&cx.gate, &format!("{site}@{pin}"), target.kin.as_deref(), instance).await?;
            if let Some(draft) = &extras.draft {
                drafted(cx, draft, &site, &at).await?;
            }
            resolved = at.cut.seqs.iter().map(|(scope, seq)| ScopeSeq { scope: scope.clone(), seq: *seq }).collect();
            name = at
                .loaded
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|loaded| loaded.name.clone())
                .unwrap_or_else(|| site.to_string());
            let shown_seq = resolved.iter().find(|seen| seen.scope.slugs().last() == Some(view)).map(|seen| seen.seq);
            if let Some(seq) = shown_seq {
                name = format!("{name}, {view} at changeset {seq}");
            }
            if extras.draft.is_some() {
                name.push_str(", with its draft");
            }
            if let (Pin::Branch(_), Some(seq)) = (&pin, shown_seq) {
                let kin: Vec<String> = (resolved.iter())
                    .filter(|seen| seen.scope.slugs().count() == 5 && seen.scope.slugs().last() != Some(view))
                    .map(|seen| format!("{}@{}", seen.scope.slugs().last().unwrap_or_default(), seen.seq))
                    .collect();
                let kin = (!kin.is_empty()).then(|| format!("kin={}", kin.join(",")));
                frozen = Some((format!("/w/{site}/{view}@{seq}"), kin));
            }
            cut = Some(at);
            format!("/w/{site}/{view}@{pin}")
        }
        (Panel::Map | Panel::Flow, _) => {
            return Err(problem(
                ProblemType::NoPanelContext,
                "The Map and Flow panels show a site: name its location.",
            ));
        }
        (Panel::Template, Some(PanelTarget::Object(object))) => format!("/template-drafts/{}", object.id()),
        (Panel::Template, Some(PanelTarget::Location(location))) => format!("/w/{location}"),
        (Panel::Template, None) => "/templates".to_owned(),
        (Panel::Scenario, Some(PanelTarget::Object(object))) => format!("/scenarios/{}", object.id()),
        (Panel::Runs, _) if extras.compare.is_some() => "/runs/compare".to_owned(),
        (Panel::Runs, Some(PanelTarget::Object(object))) => format!("/runs/{}", object.id()),
        (Panel::Scenario, _) => {
            cx.site.as_ref().map_or_else(|| "/scenarios".to_owned(), |site| format!("/scenarios?site={site}"))
        }
        (Panel::Runs, _) => "/runs".to_owned(),
    };
    let refs: Vec<String> = groups.iter().flat_map(|group| group.refs.clone()).chain(select.clone()).collect();
    let resolution = match (&cut, target.panel) {
        _ if refs.is_empty() => Resolution { resolved: Vec::new(), refs: Vec::new() },
        (Some(at), Panel::Map) => {
            let parsed: Result<Vec<Ref>, GrammarError> = refs.iter().map(|text| text.parse()).collect();
            let parsed = parsed.map_err(|why| problem(ProblemType::RefMalformed, why.to_string()))?;
            map_marks(at, parsed, instance)?
        }
        _ => {
            let request = ResolveRequest {
                location: target.location.as_ref().map(ToString::to_string),
                kin: target.kin.clone(),
                draft: extras.draft.as_ref().map(ToString::to_string),
                panel: target.panel,
                refs: refs.clone(),
            };
            let body = Bytes::from(json!(request).to_string());
            let address: Uri = instance.parse().expect("a route");
            resolve(State(cx.gate.clone()), address, body).await?.0
        }
    };
    let marks_of = |text: &str| {
        resolution.refs.iter().find(|found| found.ref_.to_string() == *text || found.key.to_string() == *text)
    };
    let mut highlight = Vec::new();
    for group in &groups {
        let mut refs = Vec::new();
        let (mut marks, mut on_floor) = (0, 0);
        for text in &group.refs {
            let parsed: Ref =
                text.parse().map_err(|why: GrammarError| problem(ProblemType::RefMalformed, why.to_string()))?;
            if let Some(found) = marks_of(text) {
                marks += found.marks.len() as u32;
                on_floor += found.marks.iter().filter(|mark| mark.on_floor).count() as u32;
            }
            refs.push(parsed);
        }
        highlight.push(HighlightGroup {
            refs,
            note: group.note.clone(),
            marks: MarkCount::new(marks),
            on_floor: MarkCount::new(on_floor),
        });
    }
    let selected = match &select {
        Some(text) => {
            Some(text.parse::<Ref>().map_err(|why: GrammarError| problem(ProblemType::RefMalformed, why.to_string()))?)
        }
        None => None,
    };
    let selected_label = select.as_deref().and_then(marks_of).map(|found| found.label.clone());
    let word = panel_word(target.panel);
    let all_marks: u32 = highlight.iter().map(|group| group.marks.get()).sum();
    let notes: Vec<&str> = highlight.iter().map(|group| group.note.as_str()).collect();
    let lit = (!highlight.is_empty()).then(|| format!(" Highlighted {all_marks} entities: {}.", notes.join("; ")));
    let announce = match action {
        PanelAction::Show if name.is_empty() => {
            format!("The agent opened the {word} panel.{}", lit.unwrap_or_default())
        }
        PanelAction::Show => format!("The agent opened the {word} panel: {name}.{}", lit.unwrap_or_default()),
        PanelAction::Highlight => {
            format!("The agent highlighted {all_marks} entities on the {word} panel: {}.", notes.join("; "))
        }
        PanelAction::Select => format!(
            "The agent selected {} on the {word} panel.",
            selected_label.unwrap_or_else(|| select.clone().unwrap_or_default())
        ),
        PanelAction::Clear => format!("The agent cleared its highlights on the {word} panel."),
    };
    let counts: Vec<GroupMarkCounts> =
        highlight.iter().map(|group| GroupMarkCounts { marks: group.marks, on_floor: group.on_floor }).collect();
    let mut before = Vec::new();
    if let Some([only]) = extras.layers.as_deref() {
        before.push(format!(
            "layers={}",
            match only {
                FlowLayer::Material => "material",
                FlowLayer::Agent => "agent",
            }
        ));
    }
    before.extend(extras.draft.as_ref().map(|draft| format!("draft={draft}")));
    let mut after: Vec<String> = extras.sim_ms.iter().map(|at| format!("sim_ms={}", at.as_ms())).collect();
    if let Some(runs) = &extras.compare {
        after.push(format!("ids={}", runs.iter().map(Ulid::as_str).collect::<Vec<_>>().join(",")));
    }
    let remembered = (action == PanelAction::Show).then(|| Shown {
        panel: target.panel,
        location: input["location"].as_str().map(str::to_owned),
        kin: None,
    });
    let entry = PanelCommandEntry {
        action,
        panel: target.panel,
        location: target.location,
        resolved: resolved.clone(),
        select: selected,
        layers: extras.layers,
        draft: extras.draft,
        sim_ms: extras.sim_ms,
        compare: extras.compare,
        metrics: extras.metrics,
        highlight: (!highlight.is_empty()).then_some(highlight),
        route: String::new(),
        as_it_was: None,
        announce: announce.clone(),
    };
    let route = Route { path, before, after, frozen };
    let journaled = journal_command(cx, entry, route, remembered.as_ref())
        .await
        .map_err(|fault| problem(ProblemType::Internal, format!("The command was not journaled: {fault}")))?;
    if let (Some(remembered), Ok(mut shown)) = (remembered, cx.shown.lock()) {
        *shown = Some(remembered);
    }
    let result = PanelCommandResult { journaled: journaled.0, route: journaled.1, resolved, groups: counts };
    let mut answer = json!(result);
    answer["announce"] = json!(announce);
    Ok(answer)
}

/// Where a command's entry sends a browser (spec 03 section 3.2's canonical order): its path, the parameters written
/// before `select` (`layers`, `draft`) and after `highlights` (`sim_ms`, `ids`), and for `as_it_was` the path at the
/// changeset it resolved with the `kin` that cut needs.
struct Route {
    path: String,
    before: Vec<String>,
    after: Vec<String>,
    frozen: Option<(String, Option<String>)>,
}

/// Journal `entry` under the number it is about to take, which its routes name in `highlights`; the conversation's row
/// lock holds that number until the entry is written. A `show` keeps what it opened on the turn's row in the same
/// commit, for the turn's later commands, after a wait as before it.
async fn journal_command<S: Wps>(
    cx: &ToolContext<S>,
    mut entry: PanelCommandEntry,
    route: Route,
    remembered: Option<&Shown>,
) -> Result<(JournalSeq, String), SqlxError> {
    let app = &cx.gate.app;
    let mut tx = app.pool.begin().await?;
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM conversations WHERE id = $1 FOR UPDATE")
        .bind(cx.conversation.as_str())
        .fetch_one(&mut *tx)
        .await?;
    let seq = last + 1;
    let mut query = route.before;
    if let Some(select) = &entry.select {
        query.push(format!("select={}", select.key()));
    }
    if entry.highlight.is_some() {
        query.push(format!("highlights={seq}"));
    }
    query.extend(route.after);
    let joined = |path: &str, query: &[String]| match query.is_empty() {
        true => path.to_owned(),
        false if path.contains('?') => format!("{path}&{}", query.join("&")),
        false => format!("{path}?{}", query.join("&")),
    };
    entry.route = joined(&route.path, &query);
    entry.as_it_was = route.frozen.map(|(frozen, kin)| {
        let held: Vec<String> = kin.into_iter().chain(query.iter().cloned()).collect();
        joined(&frozen, &held)
    });
    if let Some(shown) = remembered {
        sqlx::query("UPDATE turns SET panel_context = $2 WHERE id = $1")
            .bind(cx.turn.as_str())
            .bind(json!(shown))
            .execute(&mut *tx)
            .await?;
    }
    let route = entry.route.clone();
    let written =
        journal(&mut tx, &cx.conversation, Some(&cx.turn), JournalKind::PanelCommand, entry, Some(cx.call.to_string()))
            .await?;
    tx.commit().await?;
    let seq = written.entry().seq;
    app.hub.announce([written]);
    Ok((seq, route))
}

pub fn tools<S: Wps>() -> Vec<Box<dyn Tool<ToolContext<S>>>> {
    [PanelAction::Show, PanelAction::Highlight, PanelAction::Select, PanelAction::Clear]
        .into_iter()
        .map(|action| Box::new(Command(action)) as Box<dyn Tool<ToolContext<S>>>)
        .collect()
}
