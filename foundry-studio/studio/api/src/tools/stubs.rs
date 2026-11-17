//! Within `tools`, `stubs` are the catalogue's tools whose tasks are not built (brief I1.03 section 3): each keeps its
//! name, class and input as spec 01 section 3.9 lists them, so the model meets the whole catalogue, and answers
//! `not-built` naming the task that builds it; an asking one refuses at its card, so no approval is ever asked for an
//! act nothing can carry out. `site.tag` answers `wps-not-ready`: world-proto-server has no tags yet.

use agent_loop::{Card, Class, Tool, ToolFuture};
use serde_json::{Map, Value, json};
use studio_protocol::{Problem, ProblemType};
use wps_client::wps::Wps;

use super::ToolContext;

struct Stub {
    name: &'static str,
    class: Class,
    description: &'static str,
    /// The task that builds the tool, or `None` for one that waits on world-proto-server.
    task: Option<&'static str>,
    properties: &'static [&'static str],
}

impl Stub {
    fn problem(&self) -> Problem {
        match self.task {
            Some(task) => Problem::new(
                ProblemType::NotBuilt,
                format!("{} is not built yet: BACKLOG task {task} builds it.", self.name),
            ),
            None => {
                Problem::new(ProblemType::WpsNotReady, format!("{} waits on tags in world-proto-server.", self.name))
            }
        }
    }
}

impl<S: Wps> Tool<ToolContext<S>> for Stub {
    fn name(&self) -> &'static str {
        self.name
    }

    fn class(&self) -> Class {
        self.class
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn input_schema(&self) -> Value {
        let properties: Map<String, Value> =
            self.properties.iter().map(|name| ((*name).to_owned(), json!({}))).collect();
        json!({ "type": "object", "properties": properties })
    }

    fn describe(&self, _input: &Value) -> String {
        format!("Calling {}", self.name)
    }

    fn summarize(&self, _input: &Value, _result: &Value) -> String {
        format!("Called {}", self.name)
    }

    fn card<'a>(&'a self, _cx: &'a ToolContext<S>, _input: &'a Value) -> ToolFuture<'a, Card> {
        let problem = self.problem();
        Box::pin(async move { Err(problem) })
    }

    fn execute<'a>(&'a self, _cx: &'a ToolContext<S>, _input: Value) -> ToolFuture<'a, Value> {
        let problem = self.problem();
        Box::pin(async move { Err(problem) })
    }
}

const fn stub(
    name: &'static str,
    class: Class,
    task: Option<&'static str>,
    properties: &'static [&'static str],
    description: &'static str,
) -> Stub {
    Stub { name, class, description, task, properties }
}

/// Every tool whose task is not built, in spec 01 section 3.9's order.
pub fn tools<S: Wps>() -> Vec<Box<dyn Tool<ToolContext<S>>>> {
    let site = &["site", "with_draft"];
    let stubs = [
        stub(
            "site.update_from_template",
            Class::Apply,
            Some("37"),
            &["draft", "evaluation"],
            "Not built yet (task 37): write a made site again from its template draft at other knobs.",
        ),
        stub(
            "flow.read",
            Class::Read,
            Some("22"),
            site,
            "Not built yet (task 22): read a site's flow graph: nodes, edges, segments and the compiled plan.",
        ),
        stub(
            "flow.resolve",
            Class::Read,
            Some("22"),
            &["site", "node", "with_draft"],
            "Not built yet (task 22): resolve a flow node's sectors to their members.",
        ),
        stub(
            "flow.validate",
            Class::Read,
            Some("22"),
            site,
            "Not built yet (task 22): compile a site's flows in validation mode and list the problems.",
        ),
        stub(
            "flow.draft",
            Class::Draft,
            Some("22"),
            &["site", "summary", "ops"],
            "Not built yet (task 22): stage edits to a site's flow in the turn's draft changeset.",
        ),
        stub(
            "fleet.draft",
            Class::Draft,
            Some("27"),
            &["site", "summary", "ops"],
            "Not built yet (task 27): stage agent overrides in a site's fleet in the turn's draft changeset.",
        ),
        stub(
            "changeset.apply",
            Class::Apply,
            Some("22"),
            &["draft"],
            "Not built yet (task 22): apply the turn's draft changeset to the site, after a person approves.",
        ),
        stub("scenario.list", Class::Read, Some("27"), &[], "Not built yet (task 27): list the site's scenarios."),
        stub(
            "scenario.read",
            Class::Read,
            Some("27"),
            &["scenario", "version"],
            "Not built yet (task 27): read a scenario version with its readiness.",
        ),
        stub(
            "scenario.create",
            Class::Draft,
            Some("27"),
            &["name", "description", "workload", "run_controls"],
            "Not built yet (task 27): create a scenario on the conversation's site.",
        ),
        stub(
            "scenario.update",
            Class::Draft,
            Some("27"),
            &["scenario", "version", "revision", "set"],
            "Not built yet (task 27): set fields of a scenario version by pointer.",
        ),
        stub(
            "scenario.delete",
            Class::Destroy,
            Some("27"),
            &["scenario"],
            "Not built yet (task 27): delete a scenario, after a person approves.",
        ),
        stub(
            "experiment.create",
            Class::Draft,
            Some("27"),
            &["scenario", "version", "revision", "varies", "repeats", "run_overrides"],
            "Not built yet (task 27): prepare one run, repeats or a sweep of a scenario.",
        ),
        stub(
            "experiment.read",
            Class::Read,
            Some("27"),
            &["experiment"],
            "Not built yet (task 27): read an experiment and its runs.",
        ),
        stub(
            "run.submit",
            Class::Submit,
            Some("32"),
            &["experiment"],
            "Not built yet (task 32): submit an experiment's runs, after a person approves.",
        ),
        stub(
            "run.retry",
            Class::Submit,
            Some("32"),
            &["run"],
            "Not built yet (task 32): run a failed run again, after a person approves.",
        ),
        stub(
            "run.cancel",
            Class::Cancel,
            Some("32"),
            &["run", "submission"],
            "Not built yet (task 32): cancel a run or a submission's runs.",
        ),
        stub("run.list", Class::Read, Some("32"), &["filter", "state", "limit"], "Not built yet (task 32): list runs."),
        stub("run.status", Class::Read, Some("32"), &["run"], "Not built yet (task 32): read a run's state."),
        stub("run.inputs", Class::Read, Some("32"), &["run"], "Not built yet (task 32): read what a run read."),
        stub(
            "run.compare",
            Class::Read,
            Some("36"),
            &["runs", "metrics"],
            "Not built yet (task 36): compare metrics across 2 to 12 runs.",
        ),
        stub(
            "site.tag",
            Class::Publish,
            None,
            &["site", "tag"],
            "Not available yet: tag a site's changesets, once world-proto-server has tags.",
        ),
    ];
    stubs.into_iter().map(|stub| Box::new(stub) as Box<dyn Tool<ToolContext<S>>>).collect()
}
