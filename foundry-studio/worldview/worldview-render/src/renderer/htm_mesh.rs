//! Within `renderer`, `htm_mesh` is THE HTM'S DRAWN BODY: the low-poly stand-in
//! for the real machine that every HTM instance draws.
//!
//! The vendor model is 21 MB of Draco-compressed, 760k-triangle detail — too
//! big to keep in the repo and far too heavy to draw once per bot per frame.
//! `scripts/bake-htm-mesh.js` reduces it offline to the blob embedded here,
//! already turned into the bot's own frame: metres, +x forward, +y left, +z up,
//! origin centred on the footprint with the wheels on z = 0, each vertex
//! carrying the paint of the vendor material it came from. Unpacking that blob
//! is all this module does. Being longer than it is wide is what lets a viewer
//! see an HTM turn.

/// Baked by `scripts/bake-htm-mesh.js`, whose header documents the layout.
const BAKED: &[u8] = include_bytes!("../../assets/htm-mesh.bin");
const MAGIC: &[u8; 4] = b"HTMM";
const HEADER_LEN: usize = 16;
const VERTEX_LEN: usize = 28;
const COLOR_AT: usize = 24;

/// Height of the tote deck above the floor, in metres — the top of the baked
/// body, and so where a carried tote rides. `baked_asset_matches_deck_height`
/// keeps it honest against the asset.
pub(super) const DECK_Z: f32 = 0.341;

pub(super) struct HtmVertex {
    /// Metres in the bot's own frame.
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// The paint the vendor material wears, already sRGB-encoded so the
    /// shader can hand it to the framebuffer untouched. Alpha is always 255.
    pub color: [u8; 4],
}

pub(super) struct HtmMesh {
    pub vertices: Vec<HtmVertex>,
    pub indices: Vec<u16>,
}

/// The embedded body, or `None` if the blob does not parse — which sends HTMs
/// back to the cylinder glyph rather than dropping them from the scene.
pub(super) fn baked() -> Option<HtmMesh> {
    parse(BAKED)
}

fn parse(blob: &[u8]) -> Option<HtmMesh> {
    if blob.len() < HEADER_LEN || &blob[..4] != MAGIC {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes([blob[at], blob[at + 1], blob[at + 2], blob[at + 3]]) as usize;
    if word(4) != 2 {
        return None;
    }
    let (vertex_count, index_count) = (word(8), word(12));
    let index_start = HEADER_LEN.checked_add(vertex_count.checked_mul(VERTEX_LEN)?)?;
    let index_end = index_start.checked_add(index_count.checked_mul(2)?)?;
    if index_count % 3 != 0 || blob.len() < index_end {
        return None;
    }

    // Decoded field by field: `include_bytes!` yields byte-aligned data, so a
    // bytemuck cast to f32 would fail on alignment.
    let vertices: Vec<HtmVertex> = blob[HEADER_LEN..index_start]
        .chunks_exact(VERTEX_LEN)
        .map(|v| {
            let mut coords = [0.0f32; 6];
            for (slot, raw) in coords.iter_mut().zip(v[..COLOR_AT].chunks_exact(4)) {
                *slot = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            }
            HtmVertex {
                position: [coords[0], coords[1], coords[2]],
                normal: [coords[3], coords[4], coords[5]],
                color: [v[COLOR_AT], v[COLOR_AT + 1], v[COLOR_AT + 2], v[COLOR_AT + 3]],
            }
        })
        .collect();
    let indices: Vec<u16> =
        blob[index_start..index_end].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();

    if indices.iter().any(|&i| i as usize >= vertex_count) {
        return None;
    }
    Some(HtmMesh { vertices, indices })
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::{DECK_Z, baked};

    fn extent() -> ([f32; 3], [f32; 3]) {
        let mesh = baked().expect("the baked htm-mesh.bin must parse");
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &mesh.vertices {
            for axis in 0..3 {
                lo[axis] = lo[axis].min(v.position[axis]);
                hi[axis] = hi[axis].max(v.position[axis]);
            }
        }
        (lo, hi)
    }

    /// The shape claim the rotation animation rests on: an HTM drawn as a body
    /// longer than it is wide reads as turning; a rotationally symmetric one
    /// would not.
    #[test]
    fn baked_asset_is_longer_than_it_is_wide() {
        let (lo, hi) = extent();
        let (length, width) = (hi[0] - lo[0], hi[1] - lo[1]);
        assert!((length - 0.7).abs() < 0.01, "baked to the drawn bot footprint, got {length} m");
        assert!(width < length * 0.9, "length {length} m vs width {width} m leaves the turn invisible");
    }

    /// The bake that shipped before this one dropped every material, and the
    /// renderer painted the whole body in the bot-type colour. Three things
    /// separate real vendor paint from that: many distinct colours rather than
    /// one, at least one strongly coloured vertex (the machine's light bar),
    /// and a mid-range median — an unencoded linear byte for this model's dark
    /// greys lands near 10, a doubly gamma-encoded one near 129.
    #[test]
    fn baked_asset_carries_the_vendor_paint() {
        let mesh = baked().expect("the baked htm-mesh.bin must parse");
        let distinct: FxHashSet<[u8; 4]> = mesh.vertices.iter().map(|v| v.color).collect();
        assert!(distinct.len() > 8, "only {} distinct colours — the bake dropped materials", distinct.len());
        assert!(mesh.vertices.iter().all(|v| v.color[3] == 255), "the body draws solid, so every vertex is opaque");

        let chroma = |c: [u8; 4]| c[..3].iter().max().unwrap() - c[..3].iter().min().unwrap();
        assert!(mesh.vertices.iter().any(|v| chroma(v.color) > 64), "no vertex is strongly coloured");

        let mut luma: Vec<u16> = mesh
            .vertices
            .iter()
            .map(|v| ((2126 * v.color[0] as u32 + 7152 * v.color[1] as u32 + 722 * v.color[2] as u32) / 10000) as u16)
            .collect();
        luma.sort_unstable();
        let median = luma[luma.len() / 2];
        assert!((30..=120).contains(&median), "median luminance {median} is not sRGB-encoded vendor paint");
    }

    #[test]
    fn baked_asset_matches_deck_height() {
        let (lo, hi) = extent();
        assert!(lo[2] >= 0.0, "the body must stand on the floor, not below it");
        assert!((hi[2] - DECK_Z).abs() < 0.01, "DECK_Z {DECK_Z} m disagrees with the asset's {} m", hi[2]);
    }
}
