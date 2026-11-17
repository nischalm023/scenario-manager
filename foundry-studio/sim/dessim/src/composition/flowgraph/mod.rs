//! Within `composition`, the `flowgraph` is the flow compiler (spec 06 sections 3.2 and 3.3; BACKLOG task 17): a pure
//! function from a loaded world and its flow sub-scope to the plan a run starts with, its root tasks and the
//! settings they read, or the problems that refuse it before the clock starts, each naming the entity at fault. The
//! API links it for the panel, the agent's tools, every draft check and the check at prepare; the runner links it
//! to compile what it fetched (ruling D77). It does no I/O, reads no clock, draws no random number, and lets no hash
//! map's order reach the plan or the problems.
//!
//! Its stages, in order: `graph` reads the four `flow.` components (`components`); `resolve` resolves nodes to map members and
//! executors to cohorts; `checks` validates; `roots` infers the roots; `plan` emits the plan.

mod all_in;
mod checks;
mod components;
mod consumers;
mod graph;
mod label;
mod parameters;
mod patch;
mod plan;
mod problem;
mod quantity;
mod resolve;
mod roles;
mod roots;
mod site;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::PickSequencing;
use crate::model::ExitPath;
use crate::worldmap::WorldMap;
use crate::worldmap::flows::FlowScope;
use checks::Ctx;
use consumers::Consumers;
use site::Site;

pub use all_in::{AllIn, all_in};
pub use components::{
    AtEnd, Bias, BiasName, Carries, Container, Deposit, Destination, EdgeSpec, End, Executor, Guard, Holds, Induction,
    Intake, MapFigures, Moves, NodeKind, NodeSpec, Operation, Phase, Place, Port, Process, ReceivesIn, Ref, RuleSpec,
    SCHEMA, SegmentSpec, Work,
};
pub use consumers::seconds;
pub use graph::{Edge, Graph, Item, Node, ReturnRule, Segment};
pub use label::{Labels, moves};
pub use parameters::{CATALOGUE, Choice, Clock, Field, Parameter, lookup, parameters};
pub use patch::{Patch, sweepable};
pub use plan::settings_object;
pub use plan::{MemberCohorts, Plan, PlanEdge, PlanExecutor, PlanFlows, PlanNode, PlanRule, PlanSegment, SettingEntry};
pub use problem::{Code, Problem, Severity};
pub use quantity::{
    AdmissionFactor, Backlog, Fairness, InFlightCap, LayerSize, OpenOrderCap, PerHour, Step, UnitsPerTote,
};
pub use resolve::{Resolved, ResolvedExecutor, ResolvedNode};
pub use roles::{Root, Served};

/// The compiler's own version (section 3.3): patch, the same plan for every input; minor, new inputs accepted and the
/// same plan for old ones; major, some existing input compiles to a different plan. Before 1.0 the minor stands for
/// the major, as it does when the compiler reads a schema's version.
pub const COMPILER: &str = "0.2.0";

/// The changeset a scope was read at (spec 04's `UnitCut.seqs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Changeset(pub u64);

/// A scenario's setup (section 3.2, run mode): FillInventory's preload fraction and order trays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RunSetup {
    pub stock_at_start: f64,
    pub order_trays: u64,
}

/// One kind of order the workload holds, as spec 07's mapping crate states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderKind {
    pub container: Container,
    pub exit_path: ExitPath,
    #[serde(default)]
    pub routing: Option<String>,
}

/// Validation, with no scenario (the template evaluation, the panel, drafts, Apply's gate), or a run's, with the
/// scenario's setup and the order kinds its demand holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Validation,
    Run { setup: RunSetup, order_kinds: Vec<OrderKind> },
}

/// The closed values the chosen runner image accepts (spec 08's registry, ruling D78), met with this build's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepts {
    sequencing: Vec<String>,
}

/// The sequencing modes this build of dessim runs, in its own spelling (`config/subsystems.rs`).
fn built_sequencing() -> Vec<&'static str> {
    PickSequencing::ALL
        .iter()
        .map(|mode| match mode {
            PickSequencing::AllTotes => "all_totes",
            PickSequencing::Sequenced => "sequenced",
        })
        .collect()
}

impl Accepts {
    /// What a registered image states it accepts.
    pub fn image(sequencing: Vec<String>) -> Self {
        Self { sequencing }
    }

    /// The first cut's one local image, whose `accepts` are this build's own (ruling D109).
    pub fn this_build() -> Self {
        Self { sequencing: built_sequencing().into_iter().map(str::to_owned).collect() }
    }

    /// The sequencing modes both the image and this build run, in alphabetical order.
    pub fn sequencing(&self) -> Vec<&'static str> {
        let mut both: Vec<&'static str> = built_sequencing()
            .into_iter()
            .filter(|mode| self.sequencing.iter().any(|accepted| accepted == mode))
            .collect();
        both.sort_unstable();
        both
    }

    pub fn allows_sequencing(&self, mode: &str) -> bool {
        self.sequencing().contains(&mode)
    }
}

/// One compile's inputs besides the world and its flows: the mode, the sweep variant to apply first, and the read's
/// provenance and registry versions, which the plan copies beside its composition and settings, never into them.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub mode: Mode,
    pub variant: Vec<Patch>,
    pub seqs: BTreeMap<String, Changeset>, // ordered: the plan's bytes
    pub exact: bool,
    pub accepts: Accepts,
    /// The registry version each `flow.` component was read at; one left out reads as this compiler's own.
    pub schemas: BTreeMap<String, String>, // ordered: the plan's bytes
}

impl Request {
    pub fn validation() -> Self {
        Self {
            mode: Mode::Validation,
            variant: Vec::new(),
            seqs: BTreeMap::new(),
            exact: true,
            accepts: Accepts::this_build(),
            schemas: BTreeMap::new(),
        }
    }

    pub fn run(setup: RunSetup, order_kinds: Vec<OrderKind>) -> Self {
        Self { mode: Mode::Run { setup, order_kinds }, ..Self::validation() }
    }
}

/// A compile's result: the plan when no problem is an error, every problem in one order, and the typed graph and
/// its resolution, which the API's flow read draws from.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub plan: Option<Plan>,
    pub problems: Vec<Problem>,
    pub graph: Graph,
    pub resolved: Resolved,
    pub labels: Labels,
}

impl Compiled {
    /// What serves `edge`, one of this compile's material edges, in a run (section 3.2's root inference).
    pub fn served_by(&self, edge: &Edge) -> Served {
        roles::served_by(&self.graph, edge)
    }
}

/// Compile `flows`, a world's flow sub-scope as `worldmap::flows::read` reads it (with a draft's operations already
/// applied, where there is one), against the `world` it was read with.
pub fn compile(world: &WorldMap, flows: &FlowScope, request: &Request) -> Compiled {
    let mut problems = Vec::new();
    let mut flows = flows.clone();
    problems.extend(patch::apply(&mut flows, &request.variant));
    let site = Site::read(world);
    if flows.is_empty() {
        problems.push(Problem::warning(Code::FlowEmpty, None, None, "This site has no flow graph at this read."));
        let (graph, resolved, labels) = Default::default();
        return Compiled { plan: None, problems, graph, resolved, labels };
    }
    let graph = graph::read(&flows, &request.schemas, &mut problems);
    let resolved = resolve::resolve(&graph, &site, &mut problems);
    let labels = Labels::build(&graph, &site);
    let consumers = Consumers::gather(&graph);
    let ctx = Ctx {
        graph: &graph,
        site: &site,
        resolved: &resolved,
        labels: &labels,
        consumers: &consumers,
        mode: &request.mode,
        accepts: &request.accepts,
    };
    checks::run(&ctx, &mut problems);
    problem::sort(&mut problems);
    let plan = (!problems.iter().any(Problem::is_error)).then(|| {
        let inference = roots::infer(&graph, &consumers, &request.mode);
        plan::emit(&graph, &resolved, inference, request)
    });
    Compiled { plan, problems, graph, resolved, labels }
}
