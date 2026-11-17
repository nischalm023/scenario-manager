//! Within `checks`, `references` is `ref-unresolved` (spec 06 section 3.2): a reference that names nothing of the
//! kind its field expects at this read, or names a pinned version, where within a site every reference reads at the
//! head (rulings D60, D76). WPS refuses dangling annotated references at write; a read may combine scopes, so the
//! compiler checks again. A segment's executor references are `executor-unresolved`'s, as section 3.2 words them.

use site_model::ids::EntityId;

use super::Ctx;
use crate::composition::flowgraph::components::{EDGE, NODE, RULE, Ref, SEGMENT};
use crate::composition::flowgraph::problem::{Code, Problem};

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    let mut expect = |holder: &EntityId, field: String, reference: &Ref, expects: &str| {
        // An entity whose component could not be read is `schema-unknown`'s; naming it is not a second fault.
        let unread = ctx.graph.unread.contains(&reference.id);
        let found = unread
            || reference.at_head()
                && match expects {
                    NODE => ctx.graph.node(&reference.id).is_some(),
                    EDGE => ctx.graph.edge(&reference.id).is_some(),
                    SEGMENT => ctx.graph.segment(&reference.id).is_some(),
                    RULE => ctx.graph.rule(&reference.id).is_some(),
                    component => ctx.site.entity(&reference.id).is_some_and(|entity| entity.has(component)),
                };
        if !found {
            let pinned = reference.pin.as_deref().filter(|_| !reference.at_head());
            let why = match pinned {
                Some(pin) => format!("is pinned at {pin}, and a reference within a site reads at the head"),
                None => format!("names no {expects} at this read"),
            };
            let text = format!("{}'s {field} {why} ({}).", ctx.label(holder), reference.id);
            problems.push(ctx.error(Code::RefUnresolved, holder, &field, text));
        }
    };
    for node in &ctx.graph.nodes {
        for (at, sector) in node.spec.sectors.iter().enumerate() {
            expect(&node.id, format!("/{NODE}/sectors/{at}"), sector, "sector");
        }
        if let Some(process) = &node.spec.process {
            for (at, edge) in process.gives_by.iter().enumerate() {
                expect(&node.id, format!("/{NODE}/process/gives_by/{at}"), edge, EDGE);
            }
            for (at, edge) in process.receives_by.iter().enumerate() {
                expect(&node.id, format!("/{NODE}/process/receives_by/{at}"), edge, EDGE);
            }
        }
    }
    for edge in &ctx.graph.edges {
        let spec = &edge.spec;
        expect(&edge.id, format!("/{EDGE}/from"), &spec.from, NODE);
        expect(&edge.id, format!("/{EDGE}/to"), &spec.to, NODE);
        if let Some(primary) = &spec.alternative_to {
            expect(&edge.id, format!("/{EDGE}/alternative_to"), primary, EDGE);
        }
        if let Some(rule) = &spec.returns_by {
            expect(&edge.id, format!("/{EDGE}/returns_by"), rule, RULE);
        }
        for (at, sector) in spec.storage_scope.iter().flatten().enumerate() {
            expect(&edge.id, format!("/{EDGE}/storage_scope/{at}"), sector, "sector");
        }
    }
    for segment in &ctx.graph.segments {
        let spec = &segment.spec;
        if let Some(edge) = &spec.edge {
            expect(&segment.id, format!("/{SEGMENT}/edge"), edge, EDGE);
        }
        expect(&segment.id, format!("/{SEGMENT}/from/node"), &spec.from.node, NODE);
        expect(&segment.id, format!("/{SEGMENT}/to/node"), &spec.to.node, NODE);
        for (at, next) in spec.retained_for.iter().enumerate() {
            expect(&segment.id, format!("/{SEGMENT}/retained_for/{at}"), next, SEGMENT);
        }
    }
}
