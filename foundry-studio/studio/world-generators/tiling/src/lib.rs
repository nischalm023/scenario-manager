//! The tiling generator: a region with a pitch becomes the cells that cover it,
//! one per tile, each anchored at its own centre.
//!
//! This is the first generator to live outside the reference reader, and what it
//! is FOR is settling whether a generator can: it derives exactly what
//! `site_model`'s own tiling expansion derives, so the two can be compared
//! address by address on a real site. Everything it knows it reads off one
//! object of the `cell_grid` lens — the region's frame, origin and extent, the
//! pitch, the components the tiling says its cells carry, and the carve-outs it
//! takes back out of itself.
//!
//! There is no floating point in it. Every length is a whole count of steps and
//! a cell centre is half a pitch into its tile, which is a ratio of integers and
//! rounds exactly — see `foundry_spatial`, where SPEC §5's half-up rule
//! lives so that no generator writes it out again.
//!
//! Carve-outs are subtracted here rather than by whoever collects the output,
//! because what a tiling derives is a function of the tiling alone: a cell one
//! tiling carves out of itself must never have existed, and must not disturb an
//! overlapping tiling that keeps the same tile.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use foundry_spatial::{centre_of, Steps};
use worldgen_sdk::{export_generator, strip_comments, Derived, Generator, Key, Object};

/// The view this generator wants its input through. It travels in the manifest,
/// so a host that has never heard of tilings can still find its objects.
const LENS: &str = r#"
// Census 41, floor piece: a region that tiles itself into cells.
pattern cell_grid { g: region, tiling; }
lens cell_grid { anchor g: cell_grid; }
"#;

/// A rectangle in a frame, anchored at its minimum corner, measured in that
/// frame's steps (SPEC §5) — `Steps` and not `i64`, so that an index cannot be
/// added to a length by accident and the rounding rule below has one place to
/// live. Unknown fields are ignored, which is what lets a document annotate one.
#[derive(Debug, Deserialize)]
struct Region {
    #[serde(default)]
    frame: Option<String>,
    origin: [Steps; 2],
    size: [Steps; 2],
}

/// What makes a region a cell grid: a pitch, the carve-outs it leaves untiled,
/// and the whole component map every cell it derives carries.
#[derive(Debug, Deserialize)]
struct Tiling {
    pitch: [Steps; 2],
    #[serde(default)]
    exclusions: Vec<Exclusion>,
    #[serde(default)]
    derives: Option<BTreeMap<String, Value>>,
}

/// A rectangle this tiling leaves untiled, stated relative to its region's
/// origin, in the region's frame's steps.
#[derive(Debug, Deserialize)]
struct Exclusion {
    origin: [Steps; 2],
    size: [Steps; 2],
}

struct TilingGenerator;

impl Generator for TilingGenerator {
    const NAME: &'static str = "tiling";
    const VERSION: &'static str = "0.1.0";
    const LENS: &'static str = LENS;
    // `pose` and `cell` are what this generator states itself. A tiling's
    // `derives` can name any component at all, so this list is what the
    // generator writes unconditionally, not everything its cells end up
    // carrying — see the spike report.
    const WRITES: &'static [&'static str] = &["pose", "cell"];

    fn generate(object: &Object) -> Result<Vec<Derived>, String> {
        let anchor = object.anchor();
        let region: Region = anchor.require("region")?;
        let tiling: Tiling = anchor.require("tiling")?;
        let Some(frame) = region.frame else {
            return Err(format!(
                "entity {}: a tiled region must state its frame; inside a template, a stamp \
                 supplies it",
                anchor.id()
            ));
        };

        let cols = region.size[0]
            .whole_tiles(tiling.pitch[0])
            .ok_or_else(|| not_divisible(anchor.id(), "x", region.size[0], tiling.pitch[0]))?;
        let rows = region.size[1]
            .whole_tiles(tiling.pitch[1])
            .ok_or_else(|| not_divisible(anchor.id(), "y", region.size[1], tiling.pitch[1]))?;

        let derives = tiling.derives.unwrap_or_default();
        // A derived cell's pose is this tiling's own output — the centre of the
        // tile it was produced at — so a `derives` entry of that name would
        // replace it on every cell at once, and nothing downstream could tell
        // the substitute from the real one. The carve-outs below read that pose
        // as a position in the region's own frame, which a hand-written one
        // need not be, so a silent replacement moves which cells a carve-out
        // removes too.
        if derives.contains_key("pose") {
            return Err(format!(
                "entity {}: a tiling's `derives` may not name `pose`: each derived cell is posed \
                 at the centre of its own tile, and a derived pose would silently stand in for \
                 that",
                anchor.id()
            ));
        }

        let mut out = Vec::new();
        for col in 0..cols {
            for row in 0..rows {
                let centre = [
                    centre_of(region.origin[0], col, tiling.pitch[0]).ok_or_else(|| out_of_range(anchor.id()))?,
                    centre_of(region.origin[1], row, tiling.pitch[1]).ok_or_else(|| out_of_range(anchor.id()))?,
                ];
                if carved_out(&tiling.exclusions, region.origin, centre).ok_or_else(|| out_of_range(anchor.id()))? {
                    continue;
                }
                let key = Key::new(format!("cell:{col},{row}"))?;
                let mut cell =
                    Derived::at(key).with("pose", json!({ "frame": frame, "position": centre, "heading_deg": 0 }));
                for (name, value) in &derives {
                    // A `$comment` among the derived components is an
                    // annotation on the rule, not a component the cells carry
                    // (SPEC §2).
                    if name.starts_with('$') {
                        continue;
                    }
                    cell = cell.with(name.clone(), strip_comments(value));
                }
                // Every cell answers to being a cell, whether or not `derives`
                // says anything about one (SPEC §5).
                out.push(cell.or_state("cell", json!({})));
            }
        }
        Ok(out)
    }
}

/// Whether any carve-out takes out the cell centred here. `None` says a
/// rectangle's far corner does not fit in a step count, which is a document
/// stating a rectangle no floor has.
///
/// Every carve-out is read, with no early exit on the first hit: one that cannot
/// be computed is a refusal whichever of its siblings already covered this
/// centre. Strictly inside is the test SPEC §5 gives a carve-out, so a cell
/// centred exactly on an edge survives.
fn carved_out(exclusions: &[Exclusion], origin: [Steps; 2], centre: [Steps; 2]) -> Option<bool> {
    let mut inside = false;
    for exclusion in exclusions {
        let mut covers = true;
        for axis in 0..2 {
            let min = origin[axis].checked_add(exclusion.origin[axis])?;
            let max = min.checked_add(exclusion.size[axis])?;
            covers &= centre[axis] > min && centre[axis] < max;
        }
        inside |= covers;
    }
    Some(inside)
}

fn not_divisible(entity: &str, axis: &str, size: Steps, pitch: Steps) -> String {
    format!(
        "entity {entity}: tiling extent {size} on the {axis} axis is not a positive whole \
         multiple of the pitch {pitch}"
    )
}

fn out_of_range(entity: &str) -> String {
    format!(
        "entity {entity}: an `exclusions` rectangle runs off the step lattice — its origin plus \
         its size does not fit in a step count"
    )
}

export_generator!(TilingGenerator);
