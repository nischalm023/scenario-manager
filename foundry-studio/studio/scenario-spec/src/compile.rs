//! Within `scenario-spec`, `compile` joins the site's flow and the generated workload to the executable run spec.
//! Readiness, plan preparation and the runner use this same run-mode compiler and digest. A missing or unsupported
//! flow produces problems and no executable spec; it cannot fall through to dessim's default composition.

use dessim::composition::flowgraph::{self, Code, Problem, Request, RunSetup, Severity as Weight, settings_object};
use dessim::worldmap::WorldMap;
use dessim::worldmap::flows::read;
use serde_json::{Value, json};
use studio_protocol::{Key, ProblemItem, RunOverrides, ScenarioContent, Severity, flow_plan_digest};

use crate::{Generated, SpecFiles, order_kinds, setup, spec_fragment};

pub use dessim::composition::flowgraph::COMPILER;

/// The executable part of one variant's flow plan, including every setting the composition reads elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub digest: String,
    pub compiler: String,
    pub composition: Value,
    pub settings: Value,
}

/// Compiler findings accompany a usable plan, or explain why the site cannot run.
#[derive(Debug, Clone, PartialEq)]
pub struct Compilation {
    pub compiled: Option<Compiled>,
    pub problems: Vec<ProblemItem>,
}

impl Compiled {
    pub fn spec(&self, content: &ScenarioContent, overrides: &RunOverrides, files: SpecFiles<'_>) -> Value {
        let mut spec = spec_fragment(content, overrides, files);
        spec["composition"] = self.composition.clone();
        merge(&mut spec["settings"], &self.settings);
        spec
    }
}

/// Compile after applying the variant and generating its workload, so FillInventory starts with that workload's
/// preload fraction and the scenario's order trays and the guards cover every generated order kind.
pub fn compile(world: &WorldMap, content: &ScenarioContent, workload: &Generated) -> Compilation {
    let setup = setup(content, workload);
    let request = Request::run(
        RunSetup { stock_at_start: setup.stock_at_start.0, order_trays: u64::from(setup.order_trays.0) },
        order_kinds(),
    );
    let result = flowgraph::compile(world, &read(&world.world), &request);
    let problems: Vec<ProblemItem> = result.problems.iter().map(item).collect();
    let plan = result.plan.filter(|_| !problems.iter().any(|problem| problem.severity == Severity::Error));
    let compiled = plan.map(|plan| {
        let composition = json!(plan.composition);
        let settings = settings_object(&plan.settings);
        Compiled { digest: flow_plan_digest(&composition, &settings), compiler: plan.compiler, composition, settings }
    });
    Compilation { compiled, problems }
}

fn merge(into: &mut Value, from: &Value) {
    if let (Some(into), Some(from)) = (into.as_object_mut(), from.as_object()) {
        for (key, value) in from {
            merge(into.entry(key).or_insert(Value::Null), value);
        }
    } else {
        *into = from.clone();
    }
}

fn item(problem: &Problem) -> ProblemItem {
    let empty = problem.code == Code::FlowEmpty;
    ProblemItem {
        code: serde_json::to_value(problem.code).expect("a compiler code is JSON").as_str().unwrap().to_owned(),
        severity: match problem.severity {
            Weight::Error => Severity::Error,
            Weight::Warning if empty => Severity::Error,
            Weight::Warning => Severity::Warning,
        },
        ref_: problem
            .reference
            .as_ref()
            .map(|id| Key::Entity(id.to_string().parse().expect("a compiler entity is a protocol entity"))),
        scope: None,
        field: problem.field.clone(),
        knob: None,
        text: if empty {
            "This site needs a configured flow before it can run.".to_owned()
        } else {
            problem.text.clone()
        },
    }
}
