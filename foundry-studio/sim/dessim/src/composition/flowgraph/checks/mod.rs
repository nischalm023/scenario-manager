//! Within `flowgraph`, `checks` is stage 3 of spec 06 section 3.2, the validation: every check of its table, each
//! problem naming the entity and field at fault in a sentence the panel and the agent show as written. All
//! problems are collected and none stops the others; a check skips what an earlier one already refused (a
//! reference that names nothing, a component that could not be read), so one fault reads as one problem.

mod chains;
mod executors;
mod fit;
mod guards;
mod members;
mod process;
mod readiness;
mod references;

use std::fmt::Display;

use site_model::ids::EntityId;

use super::consumers::Consumers;
use super::graph::Graph;
use super::label::Labels;
use super::problem::{Code, Problem};
use super::resolve::Resolved;
use super::site::Site;
use super::{Accepts, Mode};

/// What every check reads.
pub(super) struct Ctx<'a, 'w> {
    pub graph: &'a Graph,
    pub site: &'a Site<'w>,
    pub resolved: &'a Resolved,
    pub labels: &'a Labels,
    pub consumers: &'a Consumers,
    pub mode: &'a Mode,
    pub accepts: &'a Accepts,
}

impl Ctx<'_, '_> {
    pub fn label(&self, id: &EntityId) -> String {
        self.labels.of(id)
    }

    pub fn error(&self, code: Code, id: &EntityId, field: &str, text: impl Display) -> Problem {
        Problem::error(code, id, Some(field.to_owned()), text)
    }

    /// A map or fleet entity's name, or its identity where it states none.
    pub fn name(&self, id: &EntityId) -> String {
        self.site.name(id).map_or_else(|| id.to_string(), str::to_owned)
    }

    /// The names of up to three map entities and how many more there are, for a sentence.
    pub fn some_of(&self, ids: &[EntityId]) -> String {
        let named: Vec<String> = ids.iter().take(3).map(|id| self.name(id)).collect();
        match ids.len() {
            0..=3 => named.join(", "),
            more => format!("{} and {} more", named.join(", "), more - 3),
        }
    }
}

pub(super) fn run(ctx: &Ctx, problems: &mut Vec<Problem>) {
    references::check(ctx, problems);
    members::check(ctx, problems);
    chains::check(ctx, problems);
    executors::check(ctx, problems);
    process::check(ctx, problems);
    guards::check(ctx, problems);
    fit::check(ctx, problems);
    readiness::check(ctx, problems);
}
