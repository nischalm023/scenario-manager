//! The structure report: one self-contained HTML file that makes a loaded
//! world legible to somebody who will never run this tool.
//!
//! The text output says how many of each thing a scenario produced; this says
//! *what* they are. It answers the four questions a person asks of a world
//! they did not author — which entities exist, what components each one
//! carries, which generator derived which entities, and what points at what —
//! and it answers them from the loaded world rather than from the documents, so
//! what it shows is what a consumer would actually get.
//!
//! Three constraints shaped it. It has to open with nothing installed and no
//! network, so the stylesheet, the script and the data are all inline and the
//! file works from a `file://` path. It has to stay usable at the size real
//! sites reach — 21,000 entities in the Cencora conversion — so the entity tree
//! is drawn a branch at a time out of embedded data rather than laid out ahead
//! of being asked for, and no list is ever shortened without saying by how
//! much. And its numbers have to be the text output's numbers, which they are
//! by construction: both read the same fields of the same [`World`].

pub mod model;
mod page;

use crate::error::LoadError;
use crate::pipeline::layers::LayerSet;
use crate::pipeline::World;
use std::path::Path;

/// The report for a scenario that loaded.
pub fn render(input: &Path, set: &LayerSet, world: &World) -> String {
    page::render(&model::build(input, set, world))
}

/// The report for a scenario that refused to load: the refusal and its causes,
/// spelled the way the text output spells them.
///
/// A refusal produces no world, so there is nothing else to show — and that is
/// the case worth being able to send somebody, because "this document will not
/// load and here is the record that stopped it" is what they need to fix it.
pub fn render_failure(input: &Path, error: &LoadError) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        messages.push(format!("caused by: {cause}"));
        source = cause.source();
    }
    page::render(&model::failure(input, messages))
}
