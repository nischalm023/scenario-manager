//! Within `worldview-render`, `document` is A SCENE WRITTEN DOWN: the floor's
//! cells and the layers standing on them, in a form that travels.
//!
//! It exists because the program that works a scene out and the program that
//! draws it are not always the same one. Turning a world document's entities
//! into cells with tiles, kinds and hops is the simulator's map compiler; a
//! browser has no simulator in it. So a server compiles the scene once per run
//! and sends this, and the browser reads it back into a [`Scene`] and draws
//! exactly what desvis draws from the map itself.
//!
//! The types are [`scene`](crate::scene)'s and [`rack`](crate::rack)'s own —
//! this module adds a container around them and the two conversions, and no
//! second spelling of a cell or a bay. They live HERE, in the crate that draws
//! them, rather than in the crate that compiles them, because the reader is a
//! browser: this crate builds for wasm32 and the simulator's map compiler
//! does not. [`FloorAndRoll`] is the same floor sent with no run around it,
//! as Agent Studio sends a site's (spec 03 section 3.6), with a roll that also
//! says which of its entities IS a cell.

use serde::{Deserialize, Serialize};
use worldview::{Error, Opaque};

use crate::rack::RackVisual;
use crate::scene::{Cell, Edge, Floor, Scene, Underlay};

/// The layout of a compiled floor AND of the roll that travels beside it,
/// one version for the two: a [`FloorDoc`] and its [`Census`] in a
/// `.worldview` scenario block or crucible's kept copy, a [`FloorDoc`] and its
/// [`RollEntry`] rows in a [`FloorAndRoll`]. Any change to either layout bumps
/// it, so a consumer that cached a floor compiled by an older build, or keyed
/// one by this number (Agent Studio's `FloorKey`), compiles it again. It says
/// nothing of how the container around them is laid out.
pub const FLOOR_VERSION: u32 = 1;

/// A scene as one document: which layout it is in, the floor, and each
/// optional layer on it.
///
/// `cells` is dense by cell id, so position is identity and a hole is a cell
/// id the floor does not have. Every layer may be empty, which is a floor with
/// nothing standing on it — the least a viewer can draw and still be right.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloorDoc {
    /// [`FLOOR_VERSION`] as the writer knew it.
    pub version: u32,
    pub cells: Vec<Option<Cell>>,
    /// The size a cell is where the floor states one size for all of them.
    pub cell_size_mm: f64,
    pub edges: Vec<Edge>,
    pub rack: RackVisual,
    pub underlay: Underlay,
}

impl Default for FloorDoc {
    fn default() -> Self {
        Self {
            version: FLOOR_VERSION,
            cells: Vec::new(),
            cell_size_mm: 0.0,
            edges: Vec::new(),
            rack: RackVisual::default(),
            underlay: Underlay::default(),
        }
    }
}

impl FloorDoc {
    /// Write a scene down. Takes the floor and the layers separately because a
    /// [`Scene`] keeps its own framing, which is derived from the cells and is
    /// therefore not worth carrying.
    pub fn of(floor: &Floor, edges: Vec<Edge>, rack: RackVisual, underlay: Underlay) -> Self {
        Self {
            version: FLOOR_VERSION,
            cells: floor.cells().to_vec(),
            cell_size_mm: floor.cell_size_mm(),
            edges,
            rack,
            underlay,
        }
    }

    /// Write down the scene a viewer already holds, layers and all — the one
    /// way out of a [`Scene`], since its layers are its own.
    pub fn of_scene(scene: &Scene) -> Self {
        Self {
            version: FLOOR_VERSION,
            cells: scene.cells().to_vec(),
            cell_size_mm: scene.floor().cell_size_mm(),
            edges: scene.edges().to_vec(),
            rack: scene.rack().clone(),
            underlay: scene.underlay().clone(),
        }
    }

    /// Read it back as the scene to draw.
    pub fn into_scene(self) -> Scene {
        Scene::new(Floor::from_tiles(self.cells, self.cell_size_mm))
            .with_edges(self.edges)
            .with_rack(self.rack)
            .with_underlay(self.underlay)
    }
}

/// One entity of the world document and the cells it turned into, as the wire
/// carries the roll: `[[entity id, [cell id…]]…]`.
///
/// Beside [`FloorDoc`] and for the same reason — a browser reads it, and this
/// is the crate a browser links. Only entities that ARE somewhere on the floor
/// appear: a stitch, an access, a cohort or a catalogue product states a
/// relation or a kind and stands nowhere.
pub type Census = Vec<(String, Vec<u32>)>;

/// One entity of a floor document's roll: its key, the cells it stands on,
/// and whether it IS the one cell it stands on (a cell of the floor, which the
/// compiler knows and a cell count cannot say), so a pick can name what stands
/// on a cell before the cell itself (spec 03 section 3.6). Positional on the
/// wire, `[entity id, [cell id…], is a cell]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollEntry {
    pub key: String,
    pub cells: Vec<u32>,
    pub is_cell: bool,
}

/// A floor and its roll as one document: what a browser drawing a site or an
/// evaluation is sent, where a run's browser finds the two in its scenario
/// block. `{ floor_version, floor, roll }`, positional like every worldview
/// document, the two inside carried as their own MessagePack as the scenario
/// block carries them, so the stamp is read, and a document of another layout
/// refused by name, before either is decoded; the floor's own stamp inside
/// must say the same.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloorAndRoll {
    pub floor_version: u32,
    pub floor: Opaque,
    pub roll: Opaque,
}

impl FloorAndRoll {
    /// The bytes a compiled floor and its roll travel as, refused for a floor
    /// whose own stamp is not this build's layout.
    pub fn write(floor: &FloorDoc, roll: &[RollEntry]) -> Result<Vec<u8>, Error> {
        if floor.version != FLOOR_VERSION {
            let detail = format!("its floor is layout {}, and this build writes layout {FLOOR_VERSION}", floor.version);
            return Err(Error::Encode { what: "floor document", detail });
        }
        let both = Self {
            floor_version: FLOOR_VERSION,
            floor: Opaque::of(floor, "compiled floor")?,
            roll: Opaque::of(&roll, "entity roll")?,
        };
        Ok(Opaque::of(&both, "floor document")?.0)
    }

    /// The floor and the roll back out of those bytes, or why not: a document
    /// of another layout is refused before its floor is read as this one, and
    /// a floor inside stamped otherwise than its document is refused too.
    pub fn read(bytes: &[u8]) -> Result<(FloorDoc, Vec<RollEntry>), Error> {
        let both: Self = Opaque(bytes.to_vec()).read("floor document")?;
        let refused = |layout: u32| {
            let detail = format!("its floor is layout {layout}, and this build reads layout {FLOOR_VERSION}");
            Error::OpaqueShape { what: "floor document", detail }
        };
        if both.floor_version != FLOOR_VERSION {
            return Err(refused(both.floor_version));
        }
        let floor: FloorDoc = both.floor.read("compiled floor")?;
        if floor.version != FLOOR_VERSION {
            return Err(refused(floor.version));
        }
        Ok((floor, both.roll.read("entity roll")?))
    }
}

#[cfg(test)]
mod tests {
    use desgraph::types::CellKind;

    use super::*;
    use crate::rack::{RackBox, RackBoxKind};
    use crate::scene::{FloorPatch, Tile};

    /// A scene sent as a document draws the scene it was written from: the
    /// same tiles, the same extent, the same steel.
    #[test]
    fn a_scene_written_down_and_read_back_is_the_scene_it_came_from() {
        let floor =
            Floor::from_centres([(0, CellKind::Highway, [425.0, 425.0]), (2, CellKind::Aisle, [1275.0, 425.0])], 850.0);
        let rack = RackVisual {
            frames: vec![RackBox { min: [0.0; 3], size: [1.0, 2.0, 3.0], yaw: 0.5, kind: RackBoxKind::Upright }],
            ..RackVisual::default()
        };
        let underlay = Underlay {
            building: vec![FloorPatch { rect_mm: [0.0, 0.0, 10.0, 10.0], kind: CellKind::Highway }],
            ..Underlay::default()
        };
        let sent = FloorDoc::of(&floor, Vec::new(), rack, underlay);

        let bytes = rmp_serde::to_vec(&sent).expect("a floor encodes");
        let scene = rmp_serde::from_slice::<FloorDoc>(&bytes).expect("and reads back").into_scene();

        assert_eq!(scene.frame().extent(), (1.7, 0.85), "the drawn extent survives the trip");
        assert_eq!(scene.cells().len(), 3, "dense by cell id, hole included");
        assert!(scene.cells()[1].is_none());
        let cell = scene.floor().cell(2).expect("cell 2");
        assert_eq!((cell.tile_mm, cell.tile), ([850.0, 0.0, 850.0, 850.0], Tile::Declared));
        assert_eq!(scene.rack().frames.len(), 1, "and the steel with it");
        assert_eq!(rmp_serde::from_slice::<FloorDoc>(&bytes).expect("reads").version, FLOOR_VERSION);
    }

    /// A floor sent with its roll reads back as the two it was written from,
    /// and a document stamped with another layout, outside or on the floor
    /// inside, is refused by that stamp.
    #[test]
    fn a_floor_and_its_roll_travel_together_and_a_foreign_layout_is_refused() {
        let floor = Floor::from_centres([(0, CellKind::Highway, [425.0, 425.0])], 850.0);
        let sent = FloorDoc::of(&floor, Vec::new(), RackVisual::default(), Underlay::default());
        let roll = [RollEntry { key: "b6a9a763-687c-4067-909b-460e986e2229".into(), cells: vec![0], is_cell: true }];

        let bytes = FloorAndRoll::write(&sent, &roll).expect("writes");
        let (floor, read_roll) = FloorAndRoll::read(&bytes).expect("reads");
        assert_eq!(rmp_serde::to_vec(&floor).unwrap(), rmp_serde::to_vec(&sent).unwrap(), "the same floor");
        assert_eq!(read_roll, roll);

        let foreign_layout = |at_floor: bool| {
            let mut foreign: FloorAndRoll = rmp_serde::from_slice(&bytes).expect("decodes");
            if at_floor {
                let older = FloorDoc { version: FLOOR_VERSION + 1, ..sent.clone() };
                foreign.floor = Opaque::of(&older, "compiled floor").unwrap();
            } else {
                foreign.floor_version = FLOOR_VERSION + 1;
            }
            FloorAndRoll::read(&rmp_serde::to_vec(&foreign).unwrap()).expect_err("another layout").to_string()
        };
        for at_floor in [false, true] {
            let refused = foreign_layout(at_floor);
            assert!(refused.contains(&format!("layout {}", FLOOR_VERSION + 1)), "{refused}");
        }
        let unwritable = FloorDoc { version: FLOOR_VERSION + 1, ..sent };
        assert!(FloorAndRoll::write(&unwritable, &roll).is_err(), "a floor of another layout is not written");
    }
}
