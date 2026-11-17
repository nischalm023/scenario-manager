//! Within `checks`, `chains` holds each material edge's agent segments together (spec 06 section 3.2, ruling D62:
//! the agent flow is cut into segments at each change of custody): `port-invalid`, `edge-unrealized`,
//! `chain-broken`, and `custody-gap`, which keeps custody passing only at handoff ports, retained robots starting
//! where they were kept, and every empty move kept by some segment.

use super::Ctx;
use crate::composition::flowgraph::components::{AtEnd, Executor, Port};
use crate::composition::flowgraph::graph::Segment;
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::quantity::Step;

const SEGMENT: &str = "/flow.agent_segment";

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    let graph = ctx.graph;
    for segment in &graph.segments {
        for (end, place) in [("from", &segment.spec.from), ("to", &segment.spec.to)] {
            let Some(node) = graph.node(&place.node.id) else { continue };
            if !Port::of(node.spec.kind).contains(&place.port) {
                let text = format!(
                    "{} names the {} port of {}, which a {} node lacks.",
                    ctx.label(&segment.id),
                    place.port.word(),
                    ctx.label(&node.id),
                    node.spec.kind.word()
                );
                problems.push(ctx.error(Code::PortInvalid, &segment.id, &format!("{SEGMENT}/{end}/port"), text));
            }
        }
        let carries_edge = !segment.spec.is_empty_move();
        if carries_edge != segment.spec.edge.is_some() || segment.spec.edge.is_some() != segment.spec.step.is_some() {
            let text =
                format!("{} must name an edge and a step, or carry nothing and name neither.", ctx.label(&segment.id));
            problems.push(ctx.error(Code::ChainBroken, &segment.id, &format!("{SEGMENT}/edge"), text));
        }
        custody(ctx, segment, problems);
    }
    for edge in &graph.edges {
        let chain = graph.chain(&edge.id);
        let Some(first) = chain.first() else {
            let text = format!(
                "{} has no agent segment, so nothing carries it (an external executor counts).",
                ctx.label(&edge.id)
            );
            problems.push(ctx.error(Code::EdgeUnrealized, &edge.id, "/flow.material_edge", text));
            continue;
        };
        let steps: Vec<Option<u32>> = chain.iter().map(|segment| segment.spec.step.map(Step::get)).collect();
        let fault = if steps.iter().enumerate().any(|(at, step)| *step != Some(at as u32 + 1)) {
            let steps: Vec<String> =
                steps.iter().map(|step| step.map_or_else(|| "none".to_owned(), |step| step.to_string())).collect();
            Some(format!("its steps run {} where they must run from 1 without a repeat or a gap", steps.join(", ")))
        } else if first.spec.from.node.id != edge.spec.from.id {
            Some(format!(
                "its first segment starts at {} instead of its own start",
                ctx.label(&first.spec.from.node.id)
            ))
        } else if chain.last().is_some_and(|last| last.spec.to.node.id != edge.spec.to.id) {
            Some("its last segment ends somewhere other than its own end".into())
        } else {
            chain
                .windows(2)
                .find(|pair| pair[0].spec.to != pair[1].spec.from)
                .map(|pair| format!("{} does not start where {} ends", ctx.label(&pair[1].id), ctx.label(&pair[0].id)))
        };
        if let Some(fault) = fault {
            let text = format!("{}'s segments do not chain: {fault}.", ctx.label(&edge.id));
            problems.push(ctx.error(Code::ChainBroken, &edge.id, "/flow.material_edge", text));
        } else {
            for pair in chain.windows(2) {
                let (handing, taking) = (pair[0], pair[1]);
                if handing.spec.executor != taking.spec.executor && !handing.spec.to.port.is_handoff() {
                    let text = format!(
                        "custody passes from {} to {} at a {} port; it passes only at a relay, a drop-off or a collection.",
                        ctx.label(&handing.id),
                        ctx.label(&taking.id),
                        handing.spec.to.port.word()
                    );
                    problems.push(ctx.error(Code::CustodyGap, &handing.id, &format!("{SEGMENT}/to/port"), text));
                }
            }
        }
    }
    for empty in graph.segments.iter().filter(|segment| segment.spec.is_empty_move()) {
        let kept =
            graph.segments.iter().any(|segment| segment.spec.retained_for.iter().any(|next| next.id == empty.id));
        if !kept {
            let text = format!("{} is an empty move that no segment keeps a robot for.", ctx.label(&empty.id));
            problems.push(ctx.error(Code::CustodyGap, &empty.id, &format!("{SEGMENT}/carries"), text));
        }
    }
}

/// One segment's own custody: what it does at its end, and where the segments it keeps its robot for begin.
fn custody(ctx: &Ctx, segment: &Segment, problems: &mut Vec<Problem>) {
    let spec = &segment.spec;
    let external = spec.executor == Executor::External;
    let boundary_end = matches!(spec.at_end, AtEnd::Arrives | AtEnd::Leaves);
    let fault = if external != boundary_end {
        Some("arrives and leaves belong to an external executor, and only they do".to_owned())
    } else if spec.at_end == AtEnd::KeepsThroughWork
        && ctx.graph.node(&spec.to.node.id).is_some_and(|node| node.spec.process.is_none())
    {
        Some(format!("keeps its carrier through work at {}, where no process runs", ctx.label(&spec.to.node.id)))
    } else {
        spec.retained_for
            .iter()
            .filter_map(|next| ctx.graph.segment(&next.id))
            .find(|next| next.spec.from != spec.to)
            .map(|next| {
                format!("keeps its robot for {}, which does not start where this one ends", ctx.label(&next.id))
            })
    };
    if let Some(fault) = fault {
        let text = format!("{} {fault}.", ctx.label(&segment.id));
        problems.push(ctx.error(Code::CustodyGap, &segment.id, &format!("{SEGMENT}/at_end"), text));
    }
}
