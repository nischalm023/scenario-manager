//! `site-model-check` — read a scenario, derive its world, and say what came out.
//!
//! This is the smallest useful consumer of the protocol, and it exists to make
//! the loader's judgements visible to a person: what refused to load and why,
//! what loaded but looks unintended, and how many of each thing the documents
//! actually produced. The counts matter as much as the errors — a rack row that
//! silently derives half the slots someone expected is a bug the load
//! itself cannot see, and a table of per-generator counts is what makes it
//! obvious.
//!
//! `--expand` writes the expanded form: the same records with each generator's
//! expansion inline beneath it. It is written in the canonical projection, which
//! is compact rather than pretty on purpose — the artifact exists to be
//! re-read, diffed and compared byte for byte, and reading it back in is how a
//! second implementation gets checked against this one.
//!
//! `--report` writes the other kind of artifact: one HTML file that shows the
//! same world to a person who will never run this tool — the entities, their
//! components, which generator derived which, and what points at what. It opens
//! from a file path with nothing installed. Both flags can be given at once, and
//! a scenario that refuses to load still writes a report, because a refusal is
//! the thing most worth sending somebody.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use site_model::EntityId;
use site_model::pipeline::World;
use site_model::LayerSet;

fn main() -> ExitCode {
    let mut positional: Option<PathBuf> = None;
    let mut expand_to: Option<PathBuf> = None;
    let mut report_to: Option<PathBuf> = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--expand") => match args.next() {
                Some(out) => expand_to = Some(PathBuf::from(out)),
                None => {
                    eprintln!("--expand needs a path to write to");
                    return ExitCode::from(2);
                }
            },
            Some("--report") => match args.next() {
                Some(out) => report_to = Some(PathBuf::from(out)),
                None => {
                    eprintln!("--report needs a path to write to");
                    return ExitCode::from(2);
                }
            },
            Some(flag) if flag.starts_with("--") => {
                eprintln!("unknown option {flag}");
                return ExitCode::from(2);
            }
            _ if positional.is_none() => positional = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("expected exactly one scenario path");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = positional else {
        eprintln!(
            "usage: site-model-check [--expand <out.json>] [--report <out.html>] <scenario.json>"
        );
        return ExitCode::from(2);
    };

    match site_model::load(&path) {
        Err(error) => {
            println!("errors: 1");
            println!("  {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                println!("    caused by: {cause}");
                source = cause.source();
            }
            if let Some(out) = report_to {
                if !write(
                    &out,
                    "structure report",
                    &site_model::report::render_failure(&path, &error),
                ) {
                    return ExitCode::from(1);
                }
            }
            ExitCode::from(1)
        }
        Ok((set, world)) => {
            println!("errors: 0");
            report(&set, &world);
            if let Some(out) = expand_to {
                let document = site_model::expanded::emit(&set, &world);
                let text = site_model::expanded::to_canonical_string(&document);
                if !write(&out, "expanded form", &text) {
                    return ExitCode::from(1);
                }
            }
            if let Some(out) = report_to {
                let html = site_model::report::render(&path, &set, &world);
                if !write(&out, "structure report", &html) {
                    return ExitCode::from(1);
                }
            }
            ExitCode::SUCCESS
        }
    }
}

/// Write one artifact and say where it went, or say why it could not.
fn write(out: &Path, what: &str, text: &str) -> bool {
    match std::fs::write(out, text.as_bytes()) {
        Ok(()) => {
            println!("{what}: {} ({} bytes)", out.display(), text.len());
            true
        }
        Err(error) => {
            eprintln!("cannot write {}: {error}", out.display());
            false
        }
    }
}

fn report(set: &LayerSet, world: &World) {
    println!("layers: {}", set.layers.len());
    for document in &set.layers {
        println!(
            "  {} ({})",
            document.path.display(),
            document.meta.name.as_deref().unwrap_or("unnamed")
        );
    }

    println!("warnings: {}", world.warnings.len());
    for warning in &world.warnings {
        println!("  {warning}");
    }

    println!(
        "entities: {} ({} declared, {} derived)",
        world.entities.len(),
        world.entities.len() - world.derived_count(),
        world.derived_count()
    );
    println!("  by component:");
    for (component, count) in world.component_census() {
        println!("    {component:<20} {count:>6}");
    }

    let bidirectional = world.edges.iter().filter(|e| e.bidirectional).count();
    println!(
        "edges: {} ({} directed flow arcs, {} bidirectional stitch links)",
        world.edges.len(),
        world.edges.len() - bidirectional,
        bidirectional
    );

    println!("  expansion by generator:");
    for (generator, count) in &world.counts {
        let name = world
            .entities
            .get(&EntityId::from(*generator))
            .and_then(|e| e.name.as_deref())
            .unwrap_or("unnamed");
        println!("    {count:>6}  {generator}  {name}");
    }

    println!("suppressed: {}", world.suppressions.len());
}
