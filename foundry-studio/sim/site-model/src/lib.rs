//! `site-model` — Foundry's warehouse site model, read from world-protocol v0.7 documents.
//!
//! The protocol describes *worlds* (a site's physical and logical reality) and
//! *scenarios* (a world composed with a workload, a fleet, and policies for one
//! run). This crate is what turns those documents into something a consumer can
//! compute over: it reads them, runs SPEC §8's six-step pipeline to derive
//! everything the documents only describe how to generate, refuses documents
//! that cannot mean one thing, and writes the result back out in the expanded
//! form, where a second implementation can be checked against it byte for byte.
//! A site WPS has already generated arrives as its flagged export instead, and
//! [`flagged_export`] reads it into the same world without generating it again.
//!
//! It deliberately implements the spec rather than interpreting it. Where SPEC.md
//! is precise, this crate is precise; where it is silent, this crate takes the
//! most literal available reading and writes the ambiguity down in `SPEC-GAPS.md`
//! beside this file, which is an input to the next revision of the spec rather
//! than a list of excuses.
//!
//! ```no_run
//! use std::path::Path;
//! let (scenario, world) = site_model::load(Path::new("scenario.json")).unwrap();
//! println!("{} entities, {} edges", world.entities.len(), world.edges.len());
//! let expanded = site_model::expanded::emit(&scenario, &world);
//! println!("{}", site_model::expanded::to_canonical_string(&expanded));
//! ```

pub use worldprotocol_core::canonical;
pub mod components;
pub mod derivation;
pub mod document;
pub mod entity;
pub mod error;
pub mod expanded;
pub mod flagged_export;
pub mod frames;
pub mod ids;
pub mod pipeline;
mod placeholders;
pub mod references;
pub mod report;
pub mod units;

pub use derivation::DerivationKey;
pub use document::Document;
pub use entity::Entity;
pub use error::{LoadError, Warning};
pub use ids::{DeclaredEntityId, DerivedEntityId, DocId, EntityId, LocalHandle};
pub use pipeline::graph::Edge;
pub use pipeline::layers::LayerSet;
pub use pipeline::World;
pub use units::{Coord, Degrees, Secs, Steps};

use std::path::Path;

/// Read a scenario and derive its world: SPEC §8, all six steps, in order.
///
/// The layer set comes back alongside the world because a bake needs the
/// scenario's own metadata and settings, and because a caller reporting on a
/// load usually wants to name the documents that took part.
pub fn load(scenario: &Path) -> Result<(LayerSet, World), LoadError> {
    let set = pipeline::layers::resolve(scenario)?;
    let world = pipeline::derive(&set)?;
    Ok((set, world))
}
