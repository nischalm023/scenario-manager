//! Within `checks`, `executors` holds each segment's executor to the fleet as the run will mint it (spec 06 section
//! 3.2): `executor-unresolved` for a cohort that is missing or counts zero, an `assigned` place with a member no
//! hard-assigned cohort reaches, or a conveyance that is no conveyance node; `executor-cannot-take` for a class at a
//! place it cannot take custody at, or a hard-assigned cohort outside its assignment. A class is read by
//! `robot_class.role`, as dessim's reader sorts cohorts into fleets (`worldmap/racking.rs`), not by `locomotion`,
//! which the seed's DA50 and DFW6's VTM classes both state as `floor`.

use site_model::ids::EntityId;

use super::Ctx;
use crate::composition::flowgraph::components::{NodeKind, Place, Port};
use crate::composition::flowgraph::graph::Segment;
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::resolve::ResolvedExecutor;
use crate::composition::flowgraph::site::CohortFacts;

const EXECUTOR: &str = "/flow.agent_segment/executor";

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    for (segment, executor) in ctx.graph.segments.iter().zip(&ctx.resolved.executors) {
        let unresolved = |why: String| {
            let text = format!("{} {why}.", ctx.label(&segment.id));
            ctx.error(Code::ExecutorUnresolved, &segment.id, EXECUTOR, text)
        };
        match executor {
            ResolvedExecutor::Cohort(id) => match ctx.site.cohort(id) {
                None => problems.push(unresolved(format!("names the cohort {id}, which the fleet does not hold"))),
                Some(cohort) if cohort.count == 0 => {
                    problems.push(unresolved(format!("names {}, which counts zero", ctx.name(id))))
                }
                Some(cohort) => {
                    for place in [&segment.spec.from, &segment.spec.to] {
                        take(ctx, segment, cohort, place, problems);
                    }
                }
            },
            ResolvedExecutor::Conveyance(node) => {
                if ctx.graph.node(node).is_none_or(|node| node.spec.kind != NodeKind::Conveyance) {
                    problems.push(unresolved(format!(
                        "names {} as its conveyance, which is no conveyance node",
                        ctx.label(node)
                    )));
                }
            }
            ResolvedExecutor::Assigned { by_member, .. } => {
                let orphans: Vec<EntityId> = by_member
                    .iter()
                    .filter(|(_, cohorts)| cohorts.is_empty())
                    .map(|(member, _)| member.clone())
                    .collect();
                if !orphans.is_empty() {
                    problems.push(unresolved(format!(
                        "is carried by the cohort assigned there, and no hard-assigned cohort above zero reaches {}",
                        ctx.some_of(&orphans)
                    )));
                }
                let mut cohorts: Vec<&EntityId> = by_member.iter().flat_map(|(_, cohorts)| cohorts).collect();
                cohorts.sort();
                cohorts.dedup();
                for cohort in cohorts.into_iter().filter_map(|id| ctx.site.cohort(id)) {
                    for place in [&segment.spec.from, &segment.spec.to] {
                        take_by_class(ctx, segment, cohort, place, problems);
                    }
                }
            }
            ResolvedExecutor::External => {}
        }
    }
}

/// A named cohort at one of its segment's places: its class must take custody there, and a hard assignment must
/// reach every member of the place's node.
fn take(ctx: &Ctx, segment: &Segment, cohort: &CohortFacts, place: &Place, problems: &mut Vec<Problem>) {
    take_by_class(ctx, segment, cohort, place, problems);
    let members = ctx.resolved.node(&place.node.id).map(|node| node.members.as_slice()).unwrap_or(&[]);
    let outside: Vec<EntityId> = if cohort.reach.is_some() {
        members.iter().filter(|member| !ctx.site.reaches(cohort, member)).cloned().collect()
    } else {
        Vec::new()
    };
    if !outside.is_empty() {
        let text = format!(
            "{} names {}, which is hard-assigned elsewhere and does not reach {} at {}.",
            ctx.label(&segment.id),
            ctx.name(&cohort.id),
            ctx.some_of(&outside),
            ctx.label(&place.node.id)
        );
        problems.push(ctx.error(Code::ExecutorCannotTake, &segment.id, EXECUTOR, text));
    }
}

/// A floor robot (an HTM) takes no carrier from a slot, and a climbing robot (a VTM) none at a presentation.
fn take_by_class(ctx: &Ctx, segment: &Segment, cohort: &CohortFacts, place: &Place, problems: &mut Vec<Problem>) {
    let cannot = match (cohort.role.as_deref(), place.port) {
        (Some("htm"), Port::Slot) => Some("an HTM cannot reach a slot"),
        (Some("vtm"), Port::Presentation | Port::OrderPresentation) => Some("a VTM cannot reach a presentation"),
        _ => None,
    };
    if let Some(why) = cannot {
        let text = format!(
            "{} has {} take custody at {}: {why}.",
            ctx.label(&segment.id),
            ctx.name(&cohort.id),
            ctx.label(&place.node.id)
        );
        problems.push(ctx.error(Code::ExecutorCannotTake, &segment.id, EXECUTOR, text));
    }
}
