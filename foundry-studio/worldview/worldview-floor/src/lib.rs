//! # worldview-floor
//!
//! The compile from a loaded world document to the scene a viewer draws: every
//! cell's tile, the hops between them, the rack steel, the ground underneath,
//! and the roll of which cells each entity of the document turned into.
//!
//! **What it is for.** A world document states entities and components; it
//! states no floor. Only the simulator's map compiler turns the one into the
//! other, and only this crate turns the compiler's map into the plain scene
//! types [`worldview_render`] draws. Both viewers come through here — desvis
//! from the map it just loaded, the Crucible server on behalf of a browser that
//! has no simulator in it — so the two cannot draw a cell in two places.
//!
//! **Where it sits.** Between `dessim`'s `worldmap` and `worldview-render`: it
//! is the only crate that depends on both. That is also why it is not inside
//! either — `worldview-render` compiles to wasm and the simulator does not,
//! and a simulator that linked a GPU renderer would be an odd thing indeed.
//!
//! **What is not here.** Anything a panel says rather than draws: which zone a
//! cell is in, a relay's diagnostics, a station's tooltip. Those stay with the
//! viewer that shows them.

pub mod cells;
pub mod easyloop;
pub mod props;
pub mod rack;

/// The scene types a compiled floor is made of, re-exported so a NATIVE
/// consumer — desvis, and the Crucible server compiling on a browser's behalf —
/// names one crate rather than two. A browser links [`worldview_render`]
/// directly and never sees this crate, which is why the types themselves live
/// there: this crate holds the simulator, and the simulator has no wasm build.
pub use worldview_render::{
    Cell, Census, Edge, FLOOR_VERSION, FloorDoc, FloorPatch, RackVisual, RollEntry, Tile, Underlay,
};

use dessim::worldmap::WorldMap;
use dessim::worldmap::census::EntityPlace;
use worldview_render::{Floor, RenderFrame};

use crate::easyloop::Placement;

/// A document compiled: what to draw, and what each drawn thing came from.
pub struct Compiled {
    pub floor: FloorDoc,
    pub census: Census,
    /// One per census row: whether that entity IS the one cell it stands on,
    /// a cell of the floor, rather than a thing standing on cells.
    pub is_cell: Vec<bool>,
}

impl Compiled {
    /// The roll a floor document carries (`worldview_render::FloorAndRoll`):
    /// the census with each cell of the floor marked, so a pick can name what
    /// stands on a cell before the cell itself.
    pub fn roll(&self) -> Vec<RollEntry> {
        let rows = self.census.iter().zip(&self.is_cell);
        rows.map(|((key, cells), &is_cell)| RollEntry { key: key.clone(), cells: cells.clone(), is_cell }).collect()
    }
}

/// What a document can fail to be compiled into a scene.
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("the map states no grid bounds, so a cell has no size")]
    NoGridBounds,
    #[error("its conveyor geometry: {0}")]
    Conveyor(String),
}

/// Compile a loaded document into everything a viewer draws from a RECORDING:
/// the cells' tiles, the hops between them, the rack steel with the conveyor
/// props standing on it, the ground under it, and the census.
///
/// This is the one compile. desvis runs it when it opens a recording and the
/// Crucible server runs it on a browser's behalf, so a cell, a bay and a belt
/// are in one place for both. The layers are assembled in the order a viewer
/// must see them in — bay geometry first, then the props extending it — because
/// the rack's box list reaches a GPU as it stands.
pub fn compile(world: &WorldMap) -> Result<Compiled, CompileError> {
    let bounds = world.map.grid_bounds.ok_or(CompileError::NoGridBounds)?;
    let drawn = cells::tiles(
        &world.map,
        &world.cell_sizes_mm,
        &world.cell_footprint_stated,
        &world.cell_headings_rad,
        bounds.cell_size_mm,
    );
    let mut rack = rack::from_worldmap(&world.map, &world.racking, &world.steel);
    let frame = RenderFrame::from_tiles(&drawn);
    for placement in Placement::load(world).map_err(CompileError::Conveyor)? {
        placement.add_to_scene(&world.map, &frame, &mut rack);
    }
    // What lies under the cells is the bays' steel and nothing else. A
    // document also states a building envelope, and a viewer holding the
    // document paints it; a recording's viewer does not, because desvis's
    // own recording path never has, and the two must draw one picture.
    let underlay = Underlay { steel_mm: world.steel.iter().map(|bay| bay.rect_mm).collect(), ..Underlay::default() };
    let floor = FloorDoc::of(&Floor::from_tiles(drawn, bounds.cell_size_mm), cells::edges(&world.map), rack, underlay);
    let (census, is_cell) = census(world);
    Ok(Compiled { floor, census, is_cell })
}

/// Every entity standing on cells, and whether it is a cell itself: the one
/// the cell table minted its cell for (`WorldIndex::cells_by_entity`), standing
/// on that cell alone.
fn census(world: &WorldMap) -> (Census, Vec<bool>) {
    world
        .entities
        .iter()
        .filter_map(|row| match &row.place {
            EntityPlace::Cells(cells) => {
                let is_cell =
                    matches!(cells.as_slice(), [only] if world.index.cells_by_entity.get(&row.id) == Some(only));
                Some(((row.id.to_string(), cells.iter().map(|cell| cell.0).collect()), is_cell))
            }
            _ => None,
        })
        .unzip()
}
