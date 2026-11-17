//! Within `worldview-render`, `scene` is WHAT THERE IS TO DRAW and WHAT IS
//! HAPPENING IN IT: the floor's cells, the optional layers of geometry that
//! stand on the floor, and the robots, containers and overlays at one instant.
//!
//! It exists so that a viewer hands over a run's facts and nothing else. The
//! base floor is what the server sends a browser — cell ids, centres, kinds
//! and one cell size — and a viewer that knows more, as desvis does from the
//! simulator's map, adds the optional layers beside it. Everything a viewer
//! reads about a cell that is not drawn (which zone it belongs to, what its
//! tooltip says) stays with the viewer.

use desgraph::types::{CellKind, HtmTraversal};
use desunits::{RobotId, ToteId};
use serde::{Deserialize, Serialize};
use worldview::frames::playback::{ToteLocation, ToteTransit};

use crate::frame::RenderFrame;
use crate::rack::RackVisual;

/// Whether the DOCUMENT stated a cell's extent, or something downstream chose
/// one so the cell could be drawn.
///
/// The distinction exists because the two stack differently. A document that
/// places a cell as a POINT — a port's Exchange cell, a presentation point, a
/// minted rack or aisle cell — says nothing about floor, and the point stands
/// INSIDE floor some other cell declares. That is what a point is for, and it
/// is not a defect; but two quads at one height are, so the drawn square goes
/// a depth tier IN FRONT OF the floor it stands in rather than into it.
///
/// It must be read from PROVENANCE and never from the size, because a stated
/// size and a chosen one are the same number by the time they reach the
/// renderer: a minted cell arrives as `[0, 0]`, but an unsized document cell
/// arrives at SPEC §5's 900 mm default, which reads exactly like declared
/// floor. Whoever reads the document is the last place that knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tile {
    /// Floor the document declares, at the extent the document states — its
    /// own `size`, or the pitch of the tiling it belongs to.
    Declared,
    /// A stand-in square for a cell the document places as a point: the
    /// reader's 900 mm default where the document declares the cell but no
    /// extent, and a nominal square where the simulator minted the cell.
    Marker,
}

/// One cell of the floor, as it is DRAWN: where its tile lies, which way it is
/// turned, what kind of place it is, and whether the document stated its
/// extent. Nothing a panel merely says about a cell belongs here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub id: u32,
    pub kind: CellKind,
    /// Unrotated tile in millimetres: min corner and size. `heading_rad`
    /// rotates it about its centre; drawing, picking and outlines use that
    /// same shape.
    pub tile_mm: [f64; 4],
    /// Rotation about the tile centre, in world radians.
    pub heading_rad: f64,
    pub tile: Tile,
    /// Whether waiting here is ordinary navigation. A station's queue, a
    /// relay's board, a charging bay and a presentation point are places a
    /// robot waits because that is the work, so the waiting overlay leaves
    /// them out rather than painting them as the busiest floor in the
    /// building. Only the compiled map knows which cells those are, so a
    /// floor stated by centres alone says false for every cell.
    pub wait_excluded: bool,
}

impl Cell {
    /// A cell a floor states by its CENTRE alone: the square of the floor's own
    /// cell size, centred there and unturned.
    pub fn at_centre(id: u32, kind: CellKind, centre_mm: [f64; 2], cell_size_mm: f64) -> Self {
        Self {
            id,
            kind,
            tile_mm: [centre_mm[0] - cell_size_mm / 2.0, centre_mm[1] - cell_size_mm / 2.0, cell_size_mm, cell_size_mm],
            heading_rad: 0.0,
            tile: Tile::Declared,
            wait_excluded: false,
        }
    }

    pub fn center_mm(&self) -> [f64; 2] {
        [self.tile_mm[0] + self.tile_mm[2] / 2.0, self.tile_mm[1] + self.tile_mm[3] / 2.0]
    }

    pub fn corners_mm(&self) -> [[f64; 2]; 4] {
        let [cx, cy] = self.center_mm();
        let [_, _, w, h] = self.tile_mm;
        let (sin, cos) = self.heading_rad.sin_cos();
        [[-w / 2.0, -h / 2.0], [w / 2.0, -h / 2.0], [w / 2.0, h / 2.0], [-w / 2.0, h / 2.0]]
            .map(|[x, y]| [cx + x * cos - y * sin, cy + x * sin + y * cos])
    }

    pub fn bounds_mm(&self) -> [f64; 4] {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in self.corners_mm() {
            for axis in 0..2 {
                lo[axis] = lo[axis].min(p[axis]);
                hi[axis] = hi[axis].max(p[axis]);
            }
        }
        [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]]
    }

    pub fn contains_mm(&self, x: f64, y: f64) -> bool {
        let [cx, cy] = self.center_mm();
        let (sin, cos) = self.heading_rad.sin_cos();
        let dx = x - cx;
        let dy = y - cy;
        let local = [dx * cos + dy * sin, -dx * sin + dy * cos];
        local[0] >= -self.tile_mm[2] / 2.0
            && local[0] < self.tile_mm[2] / 2.0
            && local[1] >= -self.tile_mm[3] / 2.0
            && local[1] < self.tile_mm[3] / 2.0
    }
}

/// One traversable hop between two cells, as an arrow draws it. An optional
/// layer: a floor that states no edges draws no arrows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    /// Endpoint positions in world mm, straight off the cells — an arrow
    /// draws from these alone, no index anywhere.
    pub from_mm: [f64; 2],
    pub to_mm: [f64; 2],
    pub vtm_traversal: bool,
    pub htm_traversal: HtmTraversal,
}

/// One rectangle of ground under the cells: a piece of the building's floor,
/// or an equipment strip standing on it. A tiling that carves holes out of
/// itself arrives as several patches, so nothing paints where the document
/// says nothing stands.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FloorPatch {
    /// `[min_x_mm, min_y_mm, width_mm, height_mm]`, axis-aligned.
    pub rect_mm: [f64; 4],
    /// The kind the patch is painted as — the same palette the cells use.
    pub kind: CellKind,
}

/// What lies UNDER the cells, composed into one disjoint layer before it is
/// drawn: rack footprints, equipment strips, the building's ground. Empty is
/// a floor with nothing beneath it, which is what a browser starts with.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Underlay {
    /// Rack footprints in millimetres. Height belongs to the bay frames.
    pub steel_mm: Vec<[f64; 4]>,
    pub equipment: Vec<FloorPatch>,
    pub building: Vec<FloorPatch>,
}

/// The floor itself: every cell by id, dense, and the size a cell is where the
/// floor states one size for all of them.
#[derive(Debug, Clone, Default)]
pub struct Floor {
    cells: Vec<Option<Cell>>,
    cell_size_mm: f64,
}

impl Floor {
    /// The floor a browser is sent: cell ids, centres, kinds and one cell size.
    /// Each cell becomes the square of that size around its centre.
    pub fn from_centres(cells: impl IntoIterator<Item = (u32, CellKind, [f64; 2])>, cell_size_mm: f64) -> Self {
        let mut dense: Vec<Option<Cell>> = Vec::new();
        for (id, kind, centre_mm) in cells {
            let at = id as usize;
            if dense.len() <= at {
                dense.resize(at + 1, None);
            }
            dense[at] = Some(Cell::at_centre(id, kind, centre_mm, cell_size_mm));
        }
        Self { cells: dense, cell_size_mm }
    }

    /// A floor whose cells state their own tiles, as a world document's do.
    /// The list is dense by cell id; a missing id draws nothing.
    pub fn from_tiles(cells: Vec<Option<Cell>>, cell_size_mm: f64) -> Self {
        Self { cells, cell_size_mm }
    }

    pub fn cells(&self) -> &[Option<Cell>] {
        &self.cells
    }

    pub fn cell(&self, id: u32) -> Option<&Cell> {
        self.cells.get(id as usize)?.as_ref()
    }

    /// The size a cell is where the floor states one size for all of them.
    pub fn cell_size_mm(&self) -> f64 {
        self.cell_size_mm
    }
}

/// Everything one map draws: the floor, and the layers a viewer that knows
/// more than the floor can add beside it.
///
/// The floor alone draws: cells, their kinds, the overlays over them. Each
/// layer is what a viewer holding the simulator's map can add — arrows from
/// the edges, steel from the rack model, ground from the underlay patches —
/// and a viewer without it simply leaves it out.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    floor: Floor,
    frame: RenderFrame,
    edges: Vec<Edge>,
    rack: RackVisual,
    underlay: Underlay,
}

impl Scene {
    /// The floor on its own, framed by the tiles its cells carry.
    pub fn new(floor: Floor) -> Self {
        let frame = RenderFrame::from_tiles(floor.cells());
        Self { floor, frame, edges: Vec::new(), rack: RackVisual::default(), underlay: Underlay::default() }
    }

    /// The hops arrows are drawn along.
    pub fn with_edges(mut self, edges: Vec<Edge>) -> Self {
        self.edges = edges;
        self
    }

    /// The rack as it is drawn: the bay frames, and the heights a container
    /// rests at.
    pub fn with_rack(mut self, rack: RackVisual) -> Self {
        self.rack = rack;
        self
    }

    /// Frame the scene by a stated extent rather than by its cells — a
    /// session with no map, whose cells are not the floor's outline.
    pub fn with_frame(mut self, frame: RenderFrame) -> Self {
        self.frame = frame;
        self
    }

    /// What lies under the cells.
    pub fn with_underlay(mut self, underlay: Underlay) -> Self {
        self.underlay = underlay;
        self
    }

    pub fn floor(&self) -> &Floor {
        &self.floor
    }

    pub fn cells(&self) -> &[Option<Cell>] {
        self.floor.cells()
    }

    /// The metres everything in this scene is drawn in, and the extent the
    /// camera frames.
    pub fn frame(&self) -> &RenderFrame {
        &self.frame
    }

    pub fn rack(&self) -> &RackVisual {
        &self.rack
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn underlay(&self) -> &Underlay {
        &self.underlay
    }
}

/// The physical container a load rides in. The discriminants reach the shader
/// through `ToteInstance::physical_type`, which sizes and colours the shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContainerKind {
    /// Walled container with open top.
    #[default]
    Tote,
    /// Reusable shallow support.
    Tray,
    /// Shipping carton carried directly by an HTM.
    Carton,
}

/// Which flow a container belongs to: stock waiting to be picked, or an
/// order's picks on their way to extraction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Designation {
    #[default]
    Inventory,
    Order,
}

/// What a container looks like, as distinct from where it is: the two are
/// independent, and a caller that knows neither gets the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToteAppearance {
    pub container: ContainerKind,
    pub designation: Designation,
    pub contents_units: u64,
}
/// How much a viewer is calling attention to a robot. Mirrors the shader's
/// `highlight` field: the primary is the selected robot, the secondary are the
/// ones a selected station is waiting on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BotHighlight {
    #[default]
    None,
    Primary,
    Secondary,
}

impl BotHighlight {
    pub(crate) fn to_u32(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Primary => 1,
            Self::Secondary => 2,
        }
    }
}

/// Where a robot is: a position in world millimetres, plus the cell it counts
/// as standing on.
///
/// Every source of a robot — a recording, a live poll, a test — states the
/// position, so the renderer never turns a cell into one. The cell rides
/// alongside because it is what decides whether the robot is over the rack's
/// steel on maps that state no steel rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub cell: u32,
    pub pos_mm: [f64; 2],
}

/// Which machine a robot is. The discriminants reach the shader through
/// `BotInstance::bot_type`, which is why the order is not free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotType {
    Vtm,
    Htm,
    Rtp,
}

/// One robot at the instant being drawn.
#[derive(Debug, Clone)]
pub struct Robot {
    pub id: RobotId,
    pub class: BotType,
    pub at: Place,
    /// Facing in radians: 0 points along +x, turning toward +y. A viewer that
    /// sweeps it across a rotation is seen to turn.
    pub heading_rad: f32,
    /// Whether this viewer is calling attention to the robot.
    pub highlight: BotHighlight,
}

/// One container at the instant being drawn: where it is, and what it looks
/// like. Custody is stated here and nowhere else — a robot is laden because
/// some container says it is carrying it.
#[derive(Debug, Clone)]
pub struct Tote {
    pub id: ToteId,
    pub at: ToteLocation,
    pub appearance: ToteAppearance,
}

/// What paints over the cells at one instant: the overlay a viewer has
/// switched on, and which relay cells are reserved.
pub struct Overlays<'a> {
    pub heatmap: Option<&'a HeatmapData>,
    pub mode: HeatmapMode,
    pub traffic: &'a dyn TrafficTints,
    /// Reserved flag per cell id, as dense as the cell list itself.
    pub reserved_cells: &'a [bool],
}

impl Default for Overlays<'_> {
    fn default() -> Self {
        Self { heatmap: None, mode: HeatmapMode::Off, traffic: &(), reserved_cells: &[] }
    }
}

/// The run, at the instant being drawn: where the movers are, which travels
/// are under way, and what is painted over the floor.
///
/// This is plain state, not geometry: turning it into the rows a GPU is fed —
/// including a container part-way along a conveyor — is the renderer's job, so
/// that two viewers of the same run cannot disagree about it.
pub struct Playback<'a> {
    /// The simulated instant, in milliseconds, that the travels are read at.
    pub at_ms: u64,
    pub robots: &'a [Robot],
    pub totes: &'a [Tote],
    /// Conveyor travels a container may be part-way along right now.
    pub transits: &'a [ToteTransit],
    pub overlays: Overlays<'a>,
}

/// Which overlay tints the cells, if any. `HtmWait` and `HtmSpeed` are
/// measured from the run's motion rather than counted per visit, which is why
/// they arrive through [`TrafficTints`] and the others through [`HeatmapData`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatmapMode {
    Off,
    VtmOnly,
    HtmOnly,
    Combined,
    HtmWait,
    HtmSpeed,
}

impl std::fmt::Display for HeatmapMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeatmapMode::Off => write!(f, "Off"),
            HeatmapMode::VtmOnly => write!(f, "VTM visits"),
            HeatmapMode::HtmOnly => write!(f, "HTM visits"),
            HeatmapMode::Combined => write!(f, "Combined visits"),
            HeatmapMode::HtmWait => write!(f, "HTM waiting time"),
            HeatmapMode::HtmSpeed => write!(f, "HTM moving speed"),
        }
    }
}

/// Visit counts per cell id, and the run's own maxima to normalise against.
pub struct HeatmapData {
    pub vtm_counts: Vec<u32>,
    pub htm_counts: Vec<u32>,
    pub vtm_max: u32,
    pub htm_max: u32,
}

/// Where the two measured overlays get a cell's tint. A viewer keeps its own
/// running measurement of the run's motion — how long robots waited on a cell,
/// how fast they crossed it — and answers here; −1 means nothing was measured
/// on that cell, which the shader draws as no tint at all.
pub trait TrafficTints {
    fn intensity(&self, cell_id: u32, mode: HeatmapMode) -> f32;
}

/// Nothing measured anywhere: what a viewer with no motion to report hands in.
impl TrafficTints for () {
    fn intensity(&self, _cell_id: u32, _mode: HeatmapMode) -> f32 {
        -1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A floor of centres draws the same squares a floor of tiles does, and
    /// frames to the same extent — the browser's floor and desvis's agree.
    #[test]
    fn a_floor_of_centres_tiles_and_frames_like_a_floor_of_tiles() {
        let compact =
            Floor::from_centres([(0, CellKind::Highway, [425.0, 425.0]), (2, CellKind::Aisle, [1275.0, 425.0])], 850.0);
        assert!(compact.cell(1).is_none(), "a floor states the cells it has, and the rest are holes");
        assert_eq!(compact.cell(2).unwrap().tile_mm, [850.0, 0.0, 850.0, 850.0]);
        assert_eq!(compact.cell_size_mm(), 850.0);

        let scene = Scene::new(compact);
        assert_eq!(scene.frame().extent(), (1.7, 0.85), "the floor reaches the far edge of its last cell");
        assert!(scene.edges().is_empty() && scene.rack().frames.is_empty(), "a floor alone carries no layers");
    }
}
