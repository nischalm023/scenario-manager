//! Expand a Foundry scenario using the native application reader and emit its
//! canonical expanded form. This local diagnostic command does not contact WPS;
//! it runs the generators compiled into Foundry's native reader.

use anyhow::{Context, Result};
use std::path::Path;

pub fn run(scenario: &Path, out: Option<&Path>) -> Result<()> {
    let (set, world) = site_model::load(scenario).with_context(|| format!("loading {}", scenario.display()))?;
    let expanded = site_model::expanded::emit(&set, &world);
    let canonical = site_model::expanded::to_canonical_string(&expanded);
    eprintln!(
        "{} entities ({} derived), {} edges, {} warnings",
        world.entities.len(),
        world.derived_count(),
        world.edges.len(),
        world.warnings.len()
    );
    match out {
        Some(path) => std::fs::write(path, canonical).with_context(|| format!("writing {}", path.display()))?,
        None => println!("{canonical}"),
    }
    Ok(())
}
