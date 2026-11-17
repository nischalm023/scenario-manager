//! Within `flows`, the `answer` is the flow compiler's result in spec 06 section 3.4's words: each node with its
//! sectors' and members' counts, its ports and its work as written; each edge, segment and return rule with its
//! label; an executor as the panel names it, an `assigned` one as a kind of cohort without a reference or a count
//! (ruling D105); the plan's roots, settings and digest; and every problem as spec 01's item. It is pure, so a route
//! runs it on the compute pool beside the compile.

use dessim::composition::flowgraph::{
    AllIn, Clock as CatalogueClock, Compiled, Parameter, Place, Plan, Problem as Finding, ResolvedExecutor, Served,
    Severity as Weight, Step, all_in as all_in_of, settings_object,
};
use dessim::worldmap::WorldMap;
use dessim::worldmap::flows::{FlowEntity, FlowScope};
use serde::Serialize;
use serde_json::{Value, json};
use studio_protocol::{
    Clock, EntityId, FlowAllIn, FlowChoice, FlowEdge, FlowExecutor, FlowNode, FlowParameter, FlowPlace, FlowPlan,
    FlowReturnRule, FlowRoot, FlowSector, FlowSegment, FlowServedBy, FlowSetting, FlowSetup, Key, ProblemItem,
    ScopePath, Severity, flow_plan_digest,
};

/// What one document compiles to, kept under its bytes' digest; the read's provenance is added per request.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub segments: Vec<FlowSegment>,
    pub return_rules: Vec<FlowReturnRule>,
    pub plan: Option<FlowPlan>,
    pub problems: Vec<ProblemItem>,
}

/// An identity as the protocol spells it: the world reader and the protocol's grammar both write a declared id as
/// its UUID and a derived one as its generator's UUID and key chain, so the text reads back.
pub fn id(text: &impl ToString) -> EntityId {
    text.to_string().parse().expect("the world reader and the protocol spell an entity id alike")
}

fn field(component: Option<&Value>, name: &str) -> Option<Value> {
    component?.get(name).cloned()
}

fn word(component: Option<&Value>, name: &str) -> Option<String> {
    component?.get(name)?.as_str().map(str::to_owned)
}

/// Whether a template made the entity: it carries a template source key in `external_ids` (D69 comments).
fn made(entity: Option<&FlowEntity>) -> bool {
    (entity.and_then(|entity| entity.components.get("external_ids")).and_then(Value::as_object))
        .is_some_and(|ids| ids.keys().any(|system| system.starts_with("template/")))
}

pub fn all_in_words(found: &AllIn) -> FlowAllIn {
    FlowAllIn {
        field: found.field.clone(),
        value: found.value.get(),
        unit: found.unit.to_owned(),
        text: found.text.clone(),
    }
}

pub fn body(world: &WorldMap, flows: &FlowScope, compiled: &Compiled, flow_scope: &ScopePath) -> Body {
    let (graph, labels, resolved) = (&compiled.graph, &compiled.labels, &compiled.resolved);
    let named = |at| world.world.entities.get(at).and_then(|entity| entity.name.clone());
    let raw = |at, name: &str| flows.get(at).and_then(|entity| entity.components.get(name));
    let nodes = (graph.nodes.iter().zip(&resolved.nodes))
        .map(|(node, members)| {
            let spec = raw(&node.id, "flow.node");
            FlowNode {
                ref_: id(&node.id),
                name: labels.of(&node.id),
                kind: word(spec, "kind").unwrap_or_default(),
                sectors: (members.by_sector.iter())
                    .map(|(sector, held)| FlowSector {
                        ref_: id(sector),
                        name: named(sector),
                        members: held.len() as u32,
                    })
                    .collect(),
                members: members.members.len() as u32,
                ports: members.ports.iter().map(|(port, count)| (port_word(port), *count)).collect(),
                process: field(spec, "process"),
                holds: word(spec, "holds"),
                demand: word(spec, "demand"),
                work: field(spec, "work"),
                all_in: all_in_of(&node.spec).as_ref().map(all_in_words),
                intake: field(spec, "intake"),
                deposit: word(spec, "deposit"),
                made_from_template: made(flows.get(&node.id)),
                diff: None,
            }
        })
        .collect();
    let edges = (graph.edges.iter())
        .map(|edge| {
            let spec = raw(&edge.id, "flow.material_edge");
            FlowEdge {
                ref_: id(&edge.id),
                label: labels.of(&edge.id),
                from: id(&edge.spec.from.id),
                to: id(&edge.spec.to.id),
                moves: word(spec, "moves").unwrap_or_default(),
                when: field(spec, "when"),
                alternative_to: edge.spec.alternative_to.as_ref().map(|other| id(&other.id)),
                bias: field(spec, "bias"),
                opens_at_sim_ms: edge.spec.opens_at_sim_ms,
                until_sim_ms: edge.spec.until_sim_ms,
                storage_scope: edge.spec.storage_scope.as_ref().map(|scope| scope.iter().map(|s| id(&s.id)).collect()),
                returns_by: edge.spec.returns_by.as_ref().map(|rule| id(&rule.id)),
                induction: field(spec, "induction"),
                served_by: match compiled.served_by(edge) {
                    Served::Root(root) => FlowServedBy::Root { task: root.name().into() },
                    Served::Clearance => FlowServedBy::Clearance,
                    Served::Completion => FlowServedBy::Completion,
                    Served::Nothing => FlowServedBy::Nothing,
                },
                made_from_template: made(flows.get(&edge.id)),
                diff: None,
            }
        })
        .collect();
    let segments = (graph.segments.iter().zip(&resolved.executors))
        .map(|(segment, executor)| {
            let spec = raw(&segment.id, "flow.agent_segment");
            let place = |at: &Place| FlowPlace { node: id(&at.node.id), port: port_word(&at.port) };
            let retained: Vec<EntityId> = segment.spec.retained_for.iter().map(|next| id(&next.id)).collect();
            FlowSegment {
                ref_: id(&segment.id),
                label: labels.of(&segment.id),
                edge: segment.spec.edge.as_ref().map(|edge| id(&edge.id)),
                step: segment.spec.step.map(Step::get),
                from: place(&segment.spec.from),
                to: place(&segment.spec.to),
                executor: executor_of(world, compiled, executor),
                carries: word(spec, "carries").unwrap_or_else(|| "edge".into()),
                at_end: word(spec, "at_end").unwrap_or_default(),
                retained_for: (!retained.is_empty()).then_some(retained),
                made_from_template: made(flows.get(&segment.id)),
                diff: None,
            }
        })
        .collect();
    let return_rules = (graph.rules.iter())
        .map(|rule| FlowReturnRule {
            ref_: id(&rule.id),
            label: labels.of(&rule.id),
            min_backlog: rule.spec.min_backlog.get(),
            destination: "nearest_eligible".into(),
            made_from_template: made(flows.get(&rule.id)),
            diff: None,
        })
        .collect();
    let problems = compiled.problems.iter().map(|finding| item(finding, flow_scope)).collect();
    Body { nodes, edges, segments, return_rules, plan: compiled.plan.as_ref().map(plan), problems }
}

fn port_word(port: &impl Serialize) -> String {
    serde_json::to_value(port).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Who carries, as the panel names it: a cohort by its name and count; the cohort assigned at a place by its kind
/// alone, since the plan, never the flow, names the per-aisle cohorts (ruling D105).
fn executor_of(world: &WorldMap, compiled: &Compiled, executor: &ResolvedExecutor) -> FlowExecutor {
    let entity = |at| world.world.entities.get(at);
    let role = |cohort| {
        let class = entity(cohort)?.raw("cohort")?.get("robot_class")?.as_str()?.split('@').next()?.to_owned();
        let class = world.world.entities.values().find(|e| e.id.to_string() == class)?;
        class.raw("robot_class")?.get("role")?.as_str().map(str::to_uppercase)
    };
    match executor {
        ResolvedExecutor::Cohort(cohort) => FlowExecutor {
            kind: "cohort".into(),
            ref_: Some(id(cohort)),
            name: entity(cohort).and_then(|e| e.name.clone()).unwrap_or_else(|| cohort.to_string()),
            count: entity(cohort).and_then(|e| e.raw("cohort")?.get("count")?.as_u64()).map(|count| count as u32),
        },
        ResolvedExecutor::Assigned { by_member, .. } => {
            let mut kinds: Vec<String> = by_member.iter().flat_map(|(_, cohorts)| cohorts).filter_map(role).collect();
            kinds.sort();
            kinds.dedup();
            let kind = match kinds.as_slice() {
                [one] => format!("{one}s"),
                _ => "robots".to_owned(),
            };
            FlowExecutor {
                kind: "assigned".into(),
                ref_: None,
                name: format!("the {kind} assigned there"),
                count: None,
            }
        }
        ResolvedExecutor::Conveyance(node) => FlowExecutor {
            kind: "conveyance".into(),
            ref_: Some(id(node)),
            name: compiled.labels.of(node),
            count: None,
        },
        ResolvedExecutor::External => {
            FlowExecutor { kind: "external".into(), ref_: None, name: "outside the site".into(), count: None }
        }
    }
}

pub fn plan(plan: &Plan) -> FlowPlan {
    FlowPlan {
        v: plan.v,
        compiler: plan.compiler.clone(),
        digest: flow_plan_digest(&json!(plan.composition), &settings_object(&plan.settings)),
        setup: plan
            .setup
            .map(|setup| FlowSetup { stock_at_start: setup.stock_at_start, order_trays: setup.order_trays }),
        roots: (plan.composition.tasks.iter())
            .map(|task| FlowRoot {
                type_: task.task_type.clone(),
                after: task.after.clone(),
                config: task.config.clone(),
            })
            .collect(),
        settings: (plan.settings.iter())
            .map(|entry| FlowSetting {
                setting: entry.setting.clone(),
                value: entry.value.clone(),
                from: id(&entry.from),
                field: entry.field.clone(),
            })
            .collect(),
    }
}

/// A finding as spec 01's problem item; every entity a finding names is a flow entity, so its scope is the flow's.
pub fn item(finding: &Finding, flow_scope: &ScopePath) -> ProblemItem {
    let code = serde_json::to_value(finding.code).ok().and_then(|code| code.as_str().map(str::to_owned));
    ProblemItem {
        code: code.unwrap_or_default(),
        severity: match finding.severity {
            Weight::Error => Severity::Error,
            Weight::Warning => Severity::Warning,
        },
        ref_: finding.reference.as_ref().map(|at| Key::Entity(id(at))),
        scope: finding.reference.as_ref().map(|_| flow_scope.clone()),
        field: finding.field.clone(),
        knob: None,
        text: finding.text.clone(),
    }
}

pub fn parameter(found: Parameter) -> FlowParameter {
    FlowParameter {
        ref_: id(&found.reference),
        pointer: found.pointer,
        label: found.label,
        short_label: found.short_label.to_owned(),
        value: found.value,
        unit: found.unit.map(str::to_owned),
        clock: found.clock.map(|clock| match clock {
            CatalogueClock::Sim => Clock::Sim,
        }),
        sweepable: found.sweepable,
        options: (found.options.into_iter())
            .map(|choice| FlowChoice { value: choice.value.to_owned(), label: choice.label.to_owned() })
            .collect(),
        all_in: found.all_in.as_ref().map(all_in_words),
    }
}
