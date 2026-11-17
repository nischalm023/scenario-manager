//! The rack generator: a bay of racking becomes the shelves it is made of — one
//! storage slot per storage position, and one handoff relay per position of
//! every relay level.
//!
//! What it is FOR is giving each shelf an identity of its own. Inventory sits in
//! a particular slot and a robot hands a carrier to a particular relay, so both
//! have to be entities a record can name and an inventory can fill. A bay five
//! wide and two deep over twenty-three storage levels is two hundred and thirty
//! of them, and a document that wrote those out by hand would be a document
//! nobody could check against the steel. So a bay states the steel — how wide,
//! how deep, and the height of every shelf bottom-up — and this derives the
//! positions that follow from it.
//!
//! One list, `levels`, carries both kinds. An entry that says where its shelves
//! stand is a relay level and derives relays; an entry that does not is storage
//! and derives slots. Both keys index that one list, so level 3 names the same
//! physical shelf whichever kind of entry sits below it.
//!
//! There is no geometry in it at all. A slot and a relay carry no pose: where a
//! shelf stands is the bay's own pose plus the offset its level records, and
//! nothing asks for that sum to be written down a second time. The relay's
//! binding to a floor cell is a later step's work (SPEC §8 step 5), so this
//! generator reads no cells and writes no `access_cell`.
//!
//! It does parse the whole `rack` component even so, including the two fields
//! it never reads. Parsing is half of what a generator says about a document:
//! a view narrowed to what the arithmetic needs would accept bays the reference
//! reader refuses, and a document would then pass here and fail there. So the
//! view below is the reader's view field for field, and the one deliberate
//! difference — the width of a count — is written down where it is made.

use serde::Deserialize;
use serde_json::json;
use foundry_spatial::Steps;
use worldgen_sdk::{export_generator, Derived, Generator, Key, Object};

/// The view this generator wants its input through. It travels in the manifest,
/// so a host that has never heard of racking can still find its objects.
const LENS: &str = r#"
// Census 17's bay without the run above it: every bay of racking, wherever it
// stands and whatever placed it. What a bay derives follows from its steel
// alone, so asking for the stamp as well would be asking for something the
// shelves do not depend on.
pattern racked { r: rack; }
lens racked { anchor r: racked; }
"#;

/// One bay of racking, as the reference reader reads one: every field of the
/// component, in the shape the reader demands it in.
///
/// Every field, and not only the three the shelves follow from, because a
/// generator that parsed less would accept bays the reader refuses — a bay
/// stating no `footprint`, or a `footprint` that is not two step counts, would
/// derive its slots here and refuse the whole document there. So `footprint`
/// and `slot_pitch` are parsed and then never read: what they are FOR here is
/// refusing exactly what the reader refuses. Where the steel stands and how
/// big a position is belong to whoever draws the bay.
///
/// The counts are `u64` and not `usize`, which is the one place this view must
/// NOT copy the reader's spelling: this is compiled for `wasm32-wasip2`, where
/// `usize` is 32 bits, and the reader's is 64. A `slots_deep` above four
/// billion parses there and derives nothing, and would be refused here while
/// deserialising — an accepted-input difference that no rack in any document
/// would show and that a document could still state.
#[derive(Debug, Deserialize)]
#[allow(dead_code, reason = "footprint and slot_pitch are parsed to refuse what the reader refuses")]
struct Rack {
    slots_wide: u64,
    slots_deep: u64,
    #[serde(default)]
    slot_pitch: Option<[Steps; 2]>,
    footprint: [Steps; 2],
    levels: Vec<Level>,
}

/// One shelf of a bay.
#[derive(Debug, Deserialize)]
struct Level {
    /// How far above the floor at the bay's pose this shelf sits.
    height: Steps,
    /// Where this level's shelves stand, each an `[along, out]` offset from the
    /// bay's own origin corner. Present makes this a relay level; absent makes
    /// it storage.
    #[serde(default)]
    positions: Option<Vec<[Steps; 2]>>,
    /// How far from a relay of this level a cell may sit and still be the one it
    /// binds. Read here only to insist that a relay level states one, since the
    /// binding itself is a later step's.
    #[serde(default)]
    reach: Option<Steps>,
}

struct RackGenerator;

impl Generator for RackGenerator {
    const NAME: &'static str = "rack";
    const VERSION: &'static str = "0.1.0";
    const LENS: &'static str = LENS;
    // The whole of what a shelf here can carry: a relay level's shelves get
    // `relay` and a storage level's get `slot`. Unlike a tiling, whose
    // `derives` can name any component at all, nothing a document writes adds
    // to this — so here the lower bound the contract asks for is also the
    // whole list.
    const WRITES: &'static [&'static str] = &["relay", "slot"];

    fn generate(object: &Object) -> Result<Vec<Derived>, String> {
        let anchor = object.anchor();
        let rack: Rack = anchor.require("rack")?;
        check_levels(anchor.id(), &rack)?;

        let mut out = Vec::new();
        for (level, entry) in rack.levels.iter().enumerate() {
            let Some(positions) = &entry.positions else {
                continue;
            };
            for (index, offset) in positions.iter().enumerate() {
                let key = Key::new(format!("relay:{level},{index}"))?;
                out.push(Derived::at(key).with(
                    "relay",
                    json!({
                        "rack": anchor.id(),
                        "level": level,
                        "index": index,
                        "height": entry.height,
                        "offset": offset,
                    }),
                ));
            }
        }

        for (level, entry) in rack.levels.iter().enumerate() {
            if entry.positions.is_some() {
                continue;
            }
            for col in 0..rack.slots_wide {
                for depth in 0..rack.slots_deep {
                    let key = Key::new(format!("slot:{level},{col},{depth}"))?;
                    out.push(
                        Derived::at(key)
                            .with("slot", json!({ "rack": anchor.id(), "level": level, "col": col, "depth": depth })),
                    );
                }
            }
        }
        Ok(out)
    }
}

/// The two things a `levels` list owes the addresses about to hang off it,
/// checked before any of them is built: heights rise down the list, so that
/// "the lowest level" and "level 3" each name one shelf; and a relay level says
/// how far its relays reach, without which nothing could later bind them to a
/// cell. Where the shelves stand needs no check of its own — a position is a
/// position.
fn check_levels(entity: &str, rack: &Rack) -> Result<(), String> {
    let mut below: Option<(usize, Steps)> = None;
    for (index, level) in rack.levels.iter().enumerate() {
        if let Some((previous, previous_height)) = below {
            if level.height <= previous_height {
                return Err(format!(
                    "rack {entity}: level {index} sits at {}, not above level {previous} at \
                     {previous_height} — a rack's levels run bottom-up and strictly increasing",
                    level.height
                ));
            }
        }
        if level.positions.is_some() && level.reach.is_none() {
            return Err(format!(
                "rack {entity}: level {index} states `positions` and so is a relay level, but \
                 states no `reach`, without which its relays cannot be bound to a cell"
            ));
        }
        below = Some((index, level.height));
    }
    Ok(())
}

export_generator!(RackGenerator);
