//! Within dessim, `worldmap` is this crate's READING of a site: its flagged
//! export, the whole site as WPS generated it with every entity it references,
//! or an authored world-protocol document, which it loads and expands through
//! the `site_model` pipeline; and it builds from either world the
//! `WarehouseMap` every other module here already computes over.
//!
//! It exists so the simulator stops compiling maps out of `WarehouseDesign`,
//! whose integer grid is the substrate of everything it says. A world document
//! positions things in millimetres and names each of them, so the loader's job
//! is no longer to *derive* structure from cell adjacency — the document states
//! which run a relay belongs to, which way serves which face, which cohort works
//! which sector — but to mint the dense indices dessim's inner machinery is
//! written against and keep the mapping back to the protocol's identities.
//! `sim/dessim/PORT-NOTES.md` records the seam and every behavioural change
//! that came with it.

pub mod cells;
pub mod census;
pub mod flows;
mod machine;
pub mod racking;
pub mod stations;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use smallvec::SmallVec;

use desgraph::DestGate;
use desgraph::Edge;
use desgraph::graph::{WarehouseGraph, WarehouseGraphBuilder};
use desgraph::{BlockId, CardinalDirection, CellId, CellKind, RelayId, StationId, SystemType, WorldPos, ZoneId};
use desmap::GridBounds;
use desmap::map::queue_edges::{
    CorridorAccess, CorridorConflict as QueueCorridorConflict, QueueCorridor, restrict_queue_corridors,
};
use desmap::map::relay_access::restrict_relay_access;
use desmap::{RelayPosition, VtmPosition, WarehouseMap, WarehouseMapBuilder, Zone};
use thiserror::Error;
use site_model::components::fleet::Carrier;
use site_model::components::spatial::{Region, Tiling};
use site_model::document::Document;
use site_model::entity::Entity;
use site_model::flagged_export;
use site_model::ids::EntityId;
// Re-exported: `WorldMap::world` hands the retained document to readers
// (desvis's entity explorer), and they should name its type through the
// module that loaded it rather than reach for the pipeline crate themselves.
use site_model::LoadError;
pub use site_model::pipeline::World;
use site_model::pipeline::layers::{self, LayerSet};
use site_model::units::{Coord, Steps};

use crate::model::RackId;
use crate::model::SimDuration;
use crate::model::conveyor::{ConveyorId, ConveyorLoops};
use crate::robots::{RobotCatalog, RobotError, resolve_successors};
use crate::robots::catalog::RobotDeployment;
use crate::scenario_gen::{RackColumn, RackUse, Racking};

use cells::CellTable;
use census::EntityRow;
use racking::Survey;

/// What one step of a document's primary frame is worth in millimetres — the
/// one place a protocol length becomes a dessim length.
///
/// The protocol counts whole steps of a declared frame and only the primary
/// frame says what a step is worth (SPEC §4, §5); dessim measures in
/// millimetres throughout. Every length crosses here exactly once, after the
/// frame chain has carried it into the primary frame, so a document authored at
/// a tenth of a millimetre and one authored at whole millimetres arrive at the
/// same simulator in the same units. A height crosses on this factor and
/// nothing else: a 2-D frame under a 3-D parent carries a two-element scale, so
/// no frame in the format stretches a vertical length.
#[derive(Clone, Copy)]
pub(crate) struct StepMm(f64);

impl StepMm {
    /// The document's own step. A world whose primary frame states no `unit_mm`
    /// cannot be loaded — the protocol refuses that document itself, so this is
    /// belt and braces for a `World` assembled by hand.
    fn of(world: &World) -> Result<StepMm, WorldMapError> {
        world.frames.unit_mm().map(StepMm).ok_or(WorldMapError::NoUnit)
    }

    pub(crate) fn length(self, steps: Steps) -> f64 {
        steps.0 as f64 * self.0
    }

    pub(crate) fn point(self, p: [Steps; 2]) -> [f64; 2] {
        [self.length(p[0]), self.length(p[1])]
    }
}

/// A world document read into the shape dessim runs on.
///
/// The map is the simulator's own; the rest is what a caller needs in order to
/// start a run and what a reader needs in order to say which protocol entity a
/// `CellId` in a trace stands for.
/// One bay's steel, for drawing: where it stands and how many shelves it has.
#[derive(Debug, Clone, Copy)]
pub struct Steel {
    /// `[min_x_mm, min_y_mm, width_mm, height_mm]`, axis-aligned.
    pub rect_mm: [f64; 4],
    pub levels: u32,
}

/// One piece of tiled floor, for drawing: a rectangle and the kind its cells
/// carry. A renderer that paints these gets solid surfaces -- lanes, aisles,
/// band, charging -- instead of thousands of per-cell tiles with seams. A
/// region whose tiling carves holes out of itself arrives as several pieces,
/// so nothing paints where the document says nothing stands.
#[derive(Debug, Clone, Copy)]
pub struct Floor {
    /// `[min_x_mm, min_y_mm, width_mm, height_mm]`, axis-aligned.
    pub rect_mm: [f64; 4],
    pub kind: CellKind,
}

/// One piece of installed equipment as a surveyed rectangle: an UNTILED
/// region on an entity that carries a vendor station component (today:
/// `go.easyloop` with a `part`). No cells, no traversal semantics of its
/// own -- the same standing as `building`, per part. `htm_traversable`
/// records what the component *states* about the floor under it (the
/// EasyLoop's staging area is drivable, its conveyor is not); it grants
/// nothing -- traversal comes only from tiled cells.
#[derive(Debug, Clone)]
pub struct Equipment {
    /// `[min_x_mm, min_y_mm, width_mm, height_mm]`, axis-aligned.
    pub rect_mm: [f64; 4],
    /// The component's `part` name: `envelope`, `staging`, ...
    pub part: String,
    pub htm_traversable: bool,
}

/// How many carriers a document declares, per pool.
///
/// The designation is permanent per carrier: a tote is an order tote or a
/// stock tote for its whole life, and the two populations never substitute for
/// each other (the anti-starvation ruling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarrierCensus {
    pub inventory: u32,
    pub order: u32,
}

impl CarrierCensus {
    pub fn total(&self) -> u32 {
        self.inventory + self.order
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

pub struct WorldMap {
    pub map: WarehouseMap,
    /// Every bay's steel: an axis-aligned millimetre rectangle
    /// `[min_x, min_y, w, h]` and its shelf count -- the equipment layer a
    /// renderer draws. Positions come from the same survey the solver
    /// schedules against, so what you see is what it runs.
    pub steel: Vec<Steel>,
    /// Every tiled region as solid rectangles with its cell kind -- one per
    /// region, or several where the tiling's own `exclusions` carve it up.
    pub floors: Vec<Floor>,
    /// Installed equipment rectangles (untiled, vendor-declared): the
    /// EasyLoop envelopes and their staging areas. Drawing and audit data,
    /// never traversal.
    pub equipment: Vec<Equipment>,
    /// The building envelope `[min_x, min_y, w, h]` in millimetres, when the
    /// document states one (an untiled region carrying the `building`
    /// component). No cells, no traversal -- where the walls are, for
    /// renderers.
    pub building: Option<[f64; 4]>,
    /// Each cell's true tile in millimetres, indexed by `CellId.0`. `[0, 0]`
    /// for the minted exchange cells (points, not floor). A renderer that
    /// draws cells at these sizes tiles the floor solid instead of scattering
    /// nominal squares.
    pub cell_sizes_mm: Vec<[f64; 2]>,
    /// Pose headings for drawing and inspecting cell footprints, keyed by CellId.
    pub cell_headings_rad: Vec<f64>,
    /// Whether the DOCUMENT fixed each cell's tile, indexed alongside
    /// `cell_sizes_mm`. A size alone cannot say: a cell the document places as
    /// a bare point states no `size` and inherits no tiling pitch, and the
    /// reader hands it SPEC §5's 900 mm default, so it arrives here looking
    /// exactly like declared floor. Anything deciding whether a cell IS floor
    /// -- desvis choosing a cell's coplanar depth tier, the DFW6 floor census
    /// counting overlapping tiles -- reads this rather than the size.
    pub cell_footprint_stated: Vec<bool>,
    /// Every station's belt, built from the `go.easyloop.loop_transit_secs`
    /// each station declares. Empty for a document whose stations present to
    /// an operator directly, which is every document but DFW6's.
    pub conveyors: ConveyorLoops,
    /// The tote population the document declares, counted per pool.
    ///
    /// `carrier_pool` is a generator of one `carrier` entity per tote, which
    /// WPS generates for a flagged export and the reader expands for an
    /// authored document, so this counts the carriers rather than the pool's
    /// `count`. A document that declares a pool of 200 and then overrides or
    /// hand-declares individual carriers is counted as it actually is.
    ///
    /// Empty when the document declares no carriers, which is every document
    /// but Walmart's today.
    pub carriers: CarrierCensus,
    /// Each deployed catalog class, without reducing it to fleet-wide figures.
    pub robot_catalog: Arc<RobotCatalog>,

    /// How the document's storage is laid out: one rack per `slot` entity, each
    /// with its own shelf height and its own place along the run. Hand this to
    /// `solver::resolve_scenario_with_racking` together with the map.
    pub racking: Racking,
    pub index: WorldIndex,
    /// Every entity the document holds, declared and derived, with what each
    /// is made of and where to point at it. Nothing in the simulator reads it
    /// -- it is for a reader studying what a conversion produced. See
    /// `census`.
    pub entities: Vec<EntityRow>,
    /// The world itself, as WPS generated it or as an authored document
    /// expands. Nothing in the simulator reads it — it is retained beside
    /// `entities` so a reader can open any entity's components without
    /// loading the file a second time.
    pub world: Arc<World>,
    /// What the protocol loader said about the document, carried so a caller
    /// can print it rather than swallow it.
    pub warnings: Vec<String>,
}

/// The mapping between the protocol's identities and the dense indices dessim
/// minted for them.
///
/// Nothing in the simulator reads this; it is for reports, traces, and the next
/// milestone, where a metric keyed by `CellId` wants to say which cell of which
/// document it counted.
#[derive(Debug, Default)]
pub struct WorldIndex {
    /// Every dessim cell's source entity, including rack anchors.
    pub cell_entities: BTreeMap<CellId, EntityId>,
    /// The reverse.
    pub cells_by_entity: BTreeMap<EntityId, CellId>,
    /// Each zone's aisle: the `along` region of the access that defines it.
    pub zone_aisles: BTreeMap<ZoneId, EntityId>,
    /// Which protocol relays each dessim relay gathers: one per shelf of the
    /// stack standing at that exchange point, bottom-up.
    pub relay_shelves: BTreeMap<RelayId, Vec<EntityId>>,
    pub station_entities: BTreeMap<StationId, EntityId>,
    /// Which protocol `slot` each storage position is. A `StorageSpaceId` can
    /// finally name a thing the document declares instead of an index of an
    /// index.
    pub storage_slots: BTreeMap<RackId, EntityId>,
}

/// Why a world document could not become a warehouse map.
///
/// Every variant names something the document says that dessim has no reading
/// for, rather than a shape the loader failed to guess — guessing is what
/// `PORT-NOTES.md` exists to forbid.
#[derive(Debug, Error)]
pub enum WorldMapError {
    #[error("invalid relay entry/back-out maneuver: {message}")]
    RelayManeuver { message: String },
    #[error("loading the world document failed: {0}")]
    Protocol(#[from] LoadError),

    #[error(
        "the run {run} is served by {count} accesses; a rack reachable from two sides is one rack \
         with two accesses in the protocol, and dessim's zone model has no reading for it yet"
    )]
    RunWithTwoServingFaces { run: EntityId, count: usize },

    #[error(
        "the run {run} holds {slots} storage positions and no handoff shelf any lane reaches, so \
         nothing can ever be put in it or taken out of it; the conversion ledgers call this a run \
         with no serving lane"
    )]
    StorageWithNoHandoff { run: EntityId, slots: usize },

    #[error(
        "rack {rack} says it holds `{holds}`, which this simulator does not model. SPEC fixes the \
         vocabulary as `inventory` or `order`; reading an unknown value as inventory would seed \
         SKUs into racking that is not for them"
    )]
    UnknownHolds { rack: EntityId, holds: String },

    #[error(
        "the run {run} is made of bays that do not agree on what they hold: some say inventory \
         and some say order. A run is one stamp of one template, so its bays state one answer; \
         an AISLE may mix them, which is how a row of order racking crosses a block"
    )]
    MixedHolds { run: EntityId },

    #[error(
        "the aisle {aisle} is assigned VTMs of two configurations, `{first}` and `{second}`. A \
         gripper cannot do a vacuum head's job or the reverse, so one aisle is worked by one kind \
         of machine"
    )]
    MixedVtmConfiguration { aisle: EntityId, first: String, second: String },

    #[error("station {station} is a `{kind}` station, and dessim runs pick and put stations only")]
    UnrunnableStationKind { station: EntityId, kind: String },

    #[error("station {station} names no presentation point, and dessim presents every carrier at a cell")]
    StationWithoutPresentation { station: EntityId },

    #[error(
        "station {station}'s presentation cell {cell} has no arc out: a robot that reaches it can never \
         leave, so the station is unusable. A station stamped into a document without a stitch to the \
         floor it stands on lands exactly here"
    )]
    StationCellIsolated { station: StationId, cell: CellId },

    #[error(
        "carrier {carrier} is designated `{designation}`, and this simulator runs `inventory` and \
         `order` carriers only"
    )]
    UnknownCarrierDesignation { carrier: EntityId, designation: String },

    #[error("the {what} of entity {entity} names {target}, which the world does not contain")]
    Dangling { what: &'static str, entity: EntityId, target: EntityId },

    #[error("{entity}: {message}")]
    Unreadable { entity: EntityId, message: String },

    #[error("{0}")]
    Robot(#[from] RobotError),

    #[error(
        "the lane {lane} states {steps} cell(s). A lane is an approach AND the place it serves, so it needs at \
         least two: one step short of that it cuts no corridor into the graph and still tells the loader not to \
         infer an approach for the cell it names"
    )]
    LaneWithoutPath { lane: EntityId, steps: usize },

    #[error(
        "the lane {lane} states a step at the position {at:?}. A lane names its cells by identity: the place it \
         serves is the last cell of its path, so a step written as a position would put a nearest-cell search on \
         the binding rule itself and a terminus mistyped by a metre would bind to whatever floor lay closest"
    )]
    LaneStepIsAPosition { lane: EntityId, at: [i64; 2] },

    #[error(
        "the lane {lane} runs over cells of machine {first} and of machine {second}. A lane belongs to one \
         machine; a lane that ends in another machine's cell is a mistyped terminus, and letting the other \
         machine claim it moves the approach and suppresses the first machine's own"
    )]
    LaneCrossesMachines { lane: EntityId, first: EntityId, second: EntityId },

    #[error(
        "the lane {lane} belongs to no machine and ends at cell {cell}, where no station presents. A lane serves \
         the place its path ends at, so one ending anywhere else serves nothing and would simply disappear"
    )]
    LaneEndsAtUnclaimedCell { lane: EntityId, cell: CellId },

    #[error(
        "the lane {lane} runs over machine {machine}'s cells and ends at cell {cell}, which is neither that \
         machine's handover nor one of its pick places. A lane belongs to the machine its cells lie in and may \
         serve only that machine's own places: one ending elsewhere opens the machine's floor to robots that \
         have no business inside it"
    )]
    LaneEndsOutsideItsMachine { lane: EntityId, machine: EntityId, cell: CellId },

    #[error(
        "the lane {lane} ends at cell {cell}, and no station presents there. The cell carries a \
         `presentation_point`, which is why the terminus was admitted, but a point no station selects binds the \
         lane to nothing and it disappears the way an unclaimed terminus did"
    )]
    LaneServesNoStation { lane: EntityId, cell: CellId },

    #[error(
        "the lane {lane} ends at cell {cell}, where both station {first} and station {second} present. A lane is \
         ONE station's approach; two install its corridor twice under two owners, and nothing here can say which \
         station the document means"
    )]
    LaneServesTwoStations { lane: EntityId, first: EntityId, second: EntityId, cell: CellId },

    #[error(
        "the lanes {first} and {second} both end at cell {cell}. One place is served by one lane: two install \
         two corridors in the graph while the machine keeps one of them as its inventory lane, so routing and \
         the machine disagree about which cells lead there"
    )]
    LaneTerminusClaimedTwice { first: EntityId, second: EntityId, cell: CellId },

    #[error(
        "the entity {entity} carries a `queue` and is also {also}. The cells a robot queues on to reach a cell \
         are a lane, an entity of its own carrying `queue` whose path ends at the cell it feeds"
    )]
    QueueNotOnItsOwnEntity { entity: EntityId, also: &'static str },

    #[error(
        "the lane {lane} runs through cell {cell}, where station {station} presents, before the cell it ends at. \
         The lane's corridor gates every arc onto that cell to robots bound for the lane's end, so a robot bound \
         for the cell could never reach it"
    )]
    LaneRunsThroughPresentation { lane: EntityId, station: EntityId, cell: CellId },

    #[error(
        "the lane {lane} ends at cell {cell}, a port, where station {station} presents without a belt of its own. \
         A lane into a port is a belt's through route, gated to the cells where the belt hands carriers over, and \
         a station presenting at the port itself states no such belt, so the lane has no reading here"
    )]
    LaneEndsAtPortWithoutBelt { lane: EntityId, station: EntityId, cell: CellId },

    #[error(
        "the port {port}, which belongs to {owner}, is not a cell. A port is where an HTM stands to hand a \
         carrier over, so it IS a declared cell; binding a posed port to the nearest floor could put a \
         machine's handover on another machine's lane. The owner is the machine whose frame the port is posed \
         in, or else whatever its `into`/`from` names -- a port that is posed nowhere and names nobody is the \
         one case where it can only be reported as its own"
    )]
    PortIsNotACell { port: EntityId, owner: EntityId },

    #[error(
        "machine {machine} states two `{direction}` ports, {first} and {second}. One belt has one way in and \
         one way out, and a second overwrote the first without a word"
    )]
    MachineWithTwoPorts { machine: EntityId, direction: String, first: EntityId, second: EntityId },

    #[error(
        "machine {inner}'s frame is nested under machine {outer}'s. Membership is the frame, and a frame chain \
         reaches every ancestor, so every cell, port and pick place of the inner machine would belong to both \
         and the outer machine's ports would overwrite the inner one's"
    )]
    NestedMachines { inner: EntityId, outer: EntityId },

    #[error(
        "the pick place {place} states the heading {heading_deg}°, which its frame chain carries into the \
         direction {primary:?} of the primary frame -- not a cardinal. An exit direction is which way an HTM \
         leaves the machine, and rounding a direction that lies between axes sends it out the wrong side"
    )]
    ExitHeadingIsNotCardinal { place: EntityId, heading_deg: f64, primary: [f64; 2] },

    #[error(
        "the lane into machine cell {cell} names no handover for its door to admit. A machine's through route \
         is gated to the cells a carrier changes hands at, and a machine with none of those has no reading here"
    )]
    MachineLaneWithoutHandover { cell: CellId },

    #[error("{conflict}")]
    LaneArcsConflict { conflict: QueueCorridorConflict },

    #[error("the document derives no cells at all, so there is nothing to simulate on")]
    NoCells,

    #[error(
        "the embedded world document composes the layers {layers:?}, and a recording carries no directory to \
         resolve them against. Record the run from a self-contained document"
    )]
    EmbeddedDocumentComposesLayers { layers: Vec<String> },

    #[error(
        "the document's primary frame says nothing about what one step is worth, so no length in \
         it can be turned into millimetres"
    )]
    NoUnit,

    #[error(
        "the primary frame {frame} is posed inside {parent}. Every position a run records is stated \
         in the primary frame, so that frame is the one the document measures from; one with a \
         parent is measured from somewhere else, and a converted point lands above the frame it \
         claims to be in"
    )]
    PrimaryFrameHasAParent { frame: EntityId, parent: EntityId },

    #[error(
        "the frame {frame} climbs to the root {root}, not to the primary frame {primary}. \
         Conversion walks a frame's parents until one has none, and labels wherever it arrives the \
         primary frame, so a chain ending at a second root turns positions measured from that root \
         into false claims about the primary one"
    )]
    FrameOutsideThePrimaryFrame { frame: EntityId, root: EntityId, primary: EntityId },
}

/// The robot entities `ids` names. The cohort survey refuses a cohort whose
/// robot entity the world does not hold before this reads it.
fn robot_entities<'w>(world: &'w World, ids: &[EntityId]) -> Result<Vec<&'w Entity>, WorldMapError> {
    let entity = |id: &EntityId| {
        let dangling = || WorldMapError::Dangling { what: "cohort's robot class", entity: id.clone(), target: id.clone() };
        world.entities.get(id).ok_or_else(dangling)
    };
    ids.iter().map(entity).collect()
}

/// Refuse a document whose frames do not all measure from the primary frame.
///
/// `FrameGraph::to_primary` reaches the primary frame by climbing parents until
/// a frame has none, and calls whatever it arrives at the primary frame. A
/// document with a second root, or with a primary frame that is itself posed
/// inside something, therefore converts a position measured from one origin and
/// hands back a claim about another. The protocol library admits both
/// documents; a dessim run, whose every recorded position names the primary
/// frame, cannot.
fn frames_measure_from_the_primary(world: &World) -> Result<(), WorldMapError> {
    let Some(primary) = world.frames.primary() else { return Ok(()) };
    if let Some(parent) = world.frames.get(primary).and_then(|frame| frame.parent.clone()) {
        return Err(WorldMapError::PrimaryFrameHasAParent { frame: primary.clone(), parent });
    }
    for id in world.entities.keys().filter(|id| world.frames.is_frame(id)) {
        let mut root = id.clone();
        while let Some(parent) = world.frames.get(&root).and_then(|frame| frame.parent.clone()) {
            root = parent;
        }
        if &root != primary {
            let (frame, primary) = (id.clone(), primary.clone());
            return Err(WorldMapError::FrameOutsideThePrimaryFrame { frame, root, primary });
        }
    }
    Ok(())
}

/// What a world file is to this module, which builds a map from either: a
/// site's flagged export, the whole site as WPS generated it, or an authored
/// world-protocol document, which it expands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldFile {
    FlaggedExport,
    Authored,
}

impl WorldFile {
    /// What `file`, read as JSON from `path`, is: a flagged export by the keys
    /// only one holds, its `scope` beside its flags, an authored document by
    /// its `proto_version`, or neither. A file marked both ways is refused
    /// rather than read either way.
    pub fn of(path: &Path, file: &Value) -> Result<Option<WorldFile>, WorldMapError> {
        if flagged_export::is_flagged_export(path, file)? {
            return Ok(Some(WorldFile::FlaggedExport));
        }
        Ok(file.get("proto_version").is_some().then_some(WorldFile::Authored))
    }
}

/// Read a world file and build the map it describes: a flagged export as WPS
/// generated it, or an authored scenario, whose layers `site_model` reads from
/// beside it and expands.
pub fn load(path: &Path) -> Result<WorldMap, WorldMapError> {
    let bytes = fs::read(path).map_err(|source| LoadError::Io { path: path.to_path_buf(), source })?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|source| LoadError::Json { path: path.to_path_buf(), source })?;
    load_parsed(path, &value)
}

/// `load`, for a world file its caller has already read from `path` as JSON
/// to tell what it is (`WorldFile::of`), so the file is not read twice.
pub fn load_parsed(path: &Path, file: &Value) -> Result<WorldMap, WorldMapError> {
    if WorldFile::of(path, file)? == Some(WorldFile::FlaggedExport) {
        return from_world(flagged_export::world(path, file)?);
    }
    let set = layers::resolve_from(Document::from_value(path, file)?)?;
    from_world(site_model::pipeline::derive(&set)?)
}

/// Build the map from a world document held in memory, as a recording carries
/// it.
///
/// A recording embeds the document byte for byte and nothing beside it, so
/// there is no directory to resolve a layer against: a document that composes
/// layers is refused here rather than silently losing them. Every document a
/// run has been recorded from is self-contained, which is what lets the
/// recording carry it at all; a flagged export is self-contained by what it
/// is, the whole site as WPS generated it with every entity it references.
pub fn load_from_bytes(document: &[u8]) -> Result<WorldMap, WorldMapError> {
    let here = Path::new("<recording>");
    let value: Value = serde_json::from_slice(document)
        .map_err(|source| WorldMapError::Protocol(LoadError::Json { path: here.to_path_buf(), source }))?;
    if WorldFile::of(here, &value)? == Some(WorldFile::FlaggedExport) {
        return from_world(flagged_export::world(here, &value)?);
    }
    let scenario = Document::from_value(here, &value)?;
    if !scenario.layers.is_empty() {
        return Err(WorldMapError::EmbeddedDocumentComposesLayers { layers: scenario.layers.clone() });
    }
    from_world(site_model::pipeline::derive(&LayerSet { scenario, layers: Vec::new() })?)
}

/// Build the map from a world that already holds its derived entities,
/// generated by WPS or expanded by the reader. Takes the world by value
/// because the result retains it (`WorldMap::world`). Agent overrides are
/// resolved first, so every fleet reader below reads an override as it reads
/// the item it succeeds.
pub fn from_world(world: World) -> Result<WorldMap, WorldMapError> {
    let world = Arc::new(resolve_successors(world)?);
    frames_measure_from_the_primary(&world)?;
    let unit = StepMm::of(&world)?;
    let survey = racking::survey(&world, unit)?;
    let mut table = cells::build(&world, &survey, unit)?;
    let zones = build_zones(&survey, &table);
    let relays = build_relays(&survey, &table);
    let stations = stations::build(&world, &table, unit)?;
    table.decorate(&stations.decorations);
    table.stamp_relay_access(relays.iter().map(|relay| relay.htm_access));
    let fleet = racking::floor_fleet(&world)?;
    let robot_catalog = Arc::new(RobotCatalog::read(&robot_entities(&world, &fleet.htm_classes)?, &robot_entities(&world, &survey.vtm_classes())?)?);
    let (mut plan, storage_slots) = build_racking(&survey, &table);
    plan.robot_deployment = RobotDeployment {
        catalog: robot_catalog.clone(),
        htm: fleet.htm_model_counts,
        vtm: survey.zones.iter().enumerate().map(|(i, zone)| (ZoneId(i as u32 + 1), zone.vtm_model_counts.clone())).collect(),
    };
    let entities = census::build(&world, &survey, &table, &stations, unit)?;

    let system_type = if relays.is_empty() { SystemType::Rtp } else { SystemType::Relay };
    let mut builder = WarehouseMapBuilder::new(system_type)
        .grid_bounds(GridBounds {
            width: table.floor_columns,
            depth: table.floor_rows,
            // The step a robot most often takes (the modal arc), because the
            // header's one nominal number feeds traverse estimates and
            // kinematics; the tightest gap would make every nominal traversal
            // far too quick on a banded floor.
            cell_size_mm: table.nominal_step_mm.unwrap_or(table.tightest_step_mm),
        })
        .graph(table.graph());
    if let Some(counts) = fleet.counts {
        builder = builder.fleet(counts);
    }
    for zone in zones {
        builder = builder.add_zone(zone);
    }
    for relay in relays {
        builder = builder.add_relay(relay);
    }
    for station in &stations.stations {
        builder = builder.add_station(station.clone());
    }

    let steel: Vec<Steel> = survey
        .zones
        .iter()
        .flat_map(|z| z.bays.iter().map(|b| Steel { rect_mm: b.rect_mm, levels: z.levels as u32 }))
        .collect();
    let cell_sizes_mm = table.cell_sizes_mm.clone();
    let cell_headings_rad = table.cell_headings_rad.clone();
    let cell_footprint_stated = table.cell_footprint_stated.clone();
    let floors = floor_regions(&world, unit)?;
    let equipment = equipment_regions(&world, unit)?;
    let building = building_rect(&world, unit)?;
    let carriers = carrier_census(&world)?;
    Ok(WorldMap {
        carriers,
        robot_catalog,
        steel,
        floors,
        equipment,
        building,
        cell_sizes_mm,
        cell_headings_rad,
        cell_footprint_stated,
        conveyors: ConveyorLoops::new(
            stations.stations.iter().filter_map(|s| Some((s.id, ConveyorId(s.conveyor_group?)))).collect(),
            stations
                .stations
                .iter()
                .filter_map(|s| Some((ConveyorId(s.conveyor_group?), SimDuration::ms(s.conveyor_transit_ms?))))
                .collect(),
            // The map alone does not know the operator's dwell; a run supplies
            // it when it creates the conveyor service. This copy exists for map-level
            // inspection, so it carries the catalog default.
            SimDuration::from_secs(9.0),
        ),
        map: {
            let mut map = builder.build();
            map.graph = restrict_relay_access(&map.graph, &map.relays)
                .map_err(|message| WorldMapError::RelayManeuver { message })?;
            // The queue discipline v3 maps get from the compiler
            // (queue_edges::restrict_queue_htm): one-way corridors through
            // each station's queue into its presentation, dest_only-gated,
            // with a single entry and a single exit edge. Without this an
            // HTM approaches a presentation cell from any direction and
            // drives THROUGH queues -- floor, not a station.
            let corridors = queue_corridors(&map)?;
            // The cell each lane is ENTERED from, keyed by the cell it serves.
            // A machine's inventory lane is entered from its dropoff, which is
            // not a step of the lane, so `handoff_approaches` cannot see it in
            // the path and would otherwise guess an approach through it by
            // nearest-cell search -- taking the dropoff's arcs with it. One lane
            // per terminus is refused upstream (`LaneTerminusClaimedTwice`), so
            // this is single-valued.
            let lane_doors: FxHashMap<CellId, CellId> =
                corridors.iter().filter_map(|c| Some((c.presentation, c.entry_from?))).collect();
            // The same answer, kept where a consumer of the MAP can find it. A
            // flow asking what a robot must drive to reach a handover needs the
            // cell the lane is entered from as well as the lane's own steps,
            // and the lane does not name it: on an EasyLoop that cell is the
            // dropoff, which the layout ruling put outside the path. Reading
            // the dropoff off the station instead is right for every document
            // in the tree and wrong in general -- a lane may be entered from
            // any cell with an arc into its head -- so the resolved cell
            // travels with the lane rather than being guessed again downstream.
            for station in &mut map.stations {
                for lane in &mut station.handoff_queues {
                    lane.entry_from = lane_doors.get(&lane.into).copied();
                }
            }
            if !corridors.is_empty() {
                map.graph = restrict_queue_corridors(&map.graph, &corridors)
                    .map_err(|conflict| WorldMapError::LaneArcsConflict { conflict })?;
            }
            let approaches = handoff_approaches(&map, &lane_doors);
            if !approaches.is_empty() {
                map.graph = restrict_handoff_approach(&map.graph, &approaches);
            }
            // Checked after the corridor restriction, which is the last thing
            // that can remove an edge.
            refuse_isolated_stations(&map)?;
            map.conveyor_machines =
                stations.machines.iter().map(|(group, machine)| machine::resolved(machine, *group, &map)).collect();
            map
        },
        racking: plan,
        entities,
        // A clone, not a move: `warnings` below still reads through the Arc.
        world: Arc::clone(&world),
        index: WorldIndex {
            cell_entities: table.cell_entities.clone(),
            cells_by_entity: table.cells_by_entity.clone(),
            zone_aisles: survey.zone_aisles(),
            relay_shelves: table.relay_shelves.clone(),
            station_entities: stations.by_id.clone(),
            storage_slots,
        },
        warnings: world.warnings.iter().map(|w| w.to_string()).collect(),
    })
}

/// Count the carriers a document declares, per pool.
///
/// `carrier_pool` is a generator, so the map already holds one `carrier`
/// entity per tote, generated by WPS or expanded by the reader. Counting the
/// carriers rather than the pool's `count` means a document that hand-declares
/// or overrides individual carriers is counted as it actually is.
///
/// An unknown designation is a load error, not a silently-dropped tote.
fn carrier_census(world: &World) -> Result<CarrierCensus, WorldMapError> {
    let mut census = CarrierCensus::default();
    for entity in world.entities.values() {
        let Some(carrier) = racking::optional::<Carrier>(entity, "carrier")? else { continue };
        match carrier.designation.as_str() {
            "inventory" => census.inventory += 1,
            "order" => census.order += 1,
            other => {
                return Err(WorldMapError::UnknownCarrierDesignation {
                    carrier: entity.id.clone(),
                    designation: other.to_string(),
                });
            }
        }
    }
    Ok(census)
}

/// Refuse a station any of whose presentation cells has no arc out.
///
/// Every pin over a document asserts structure — slot counts, board positions,
/// a station census — and none of them notices that a station is not attached
/// to the graph. DFW6 shipped four Easy Loops with degree-0 presentation cells;
/// the symptom was a pathfinder exhausting its search space per candidate, not
/// an error.
fn refuse_isolated_stations(map: &WarehouseMap) -> Result<(), WorldMapError> {
    for station in &map.stations {
        for cell in station.presentation_cells() {
            if map.graph.neighbors(cell).is_empty() {
                return Err(WorldMapError::StationCellIsolated { station: station.id, cell });
            }
        }
    }
    Ok(())
}

/// One dessim zone per region an access serves: the run (or the pair of runs)
/// that aisle is cut between, the relays on them, and the VTMs assigned to work
/// them.
fn build_zones(survey: &Survey, table: &CellTable) -> Vec<Zone> {
    survey
        .zones
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            let id = ZoneId(index as u32 + 1);
            let mut cells: Vec<CellId> = Vec::new();
            let mut relays: Vec<RelayId> = Vec::new();
            for bay in &draft.bays {
                if let Some(cell) = table.rack_cells.get(&bay.rack) {
                    cells.push(*cell);
                }
                for stand in &bay.stands {
                    if let Some(relay_id) = table.relays_by_stand.get(&(bay.rack.clone(), stand.cell.clone())) {
                        relays.push(*relay_id);
                    }
                }
            }
            cells.sort_unstable();
            relays.sort_unstable();
            Zone {
                id,
                name: draft.name.clone().or_else(|| Some(format!("zone_{}", id.0))),
                cells,
                relays,
                vtm_count: draft.vtm_count,
                holds: draft.holds,
                vtm_configuration: draft.vtm_configuration.clone(),
                zone_height_mm: draft.height_mm.round().max(0.0) as u64,
                zone_width_mm: Some(draft.width_mm()),
                real_shelf_count: draft.levels.min(u16::MAX as usize) as u16,
                slots: None,
            }
        })
        .collect()
}

/// One relay per exchange point: the bay it sits on, the floor cell an HTM
/// stands on to reach it, and the aisle position a VTM meets it from.
fn build_relays(survey: &Survey, table: &CellTable) -> Vec<RelayPosition> {
    let mut out = Vec::new();
    for (index, draft) in survey.zones.iter().enumerate() {
        let zone = ZoneId(index as u32 + 1);
        for bay in &draft.bays {
            let Some(&position) = table.rack_cells.get(&bay.rack) else {
                continue;
            };
            for stand in &bay.stands {
                let (Some(&id), Some(&htm_access)) = (
                    table.relays_by_stand.get(&(bay.rack.clone(), stand.cell.clone())),
                    table.cells_by_entity.get(&stand.cell),
                ) else {
                    continue;
                };
                out.push(RelayPosition {
                    id,
                    position,
                    htm_access,
                    vtm_access: VtmPosition::At {
                        position: WorldPos { x_mm: bay.aisle_centre[0], y_mm: bay.aisle_centre[1] },
                        anchor: position,
                    },
                    block: BlockId(zone.0),
                    side: bay.side,
                });
            }
        }
    }
    out.sort_by_key(|r| r.id.0);
    out
}

/// The storage layout the document states, in the shape scenario generation
/// consumes.
///
/// One dessim rack per protocol `slot`, holding one tote: a slot *is* a storage
/// position, so the two line up one for one and `StorageSpaceId` finally
/// addresses something the document names. That also retires the uniform shelf
/// model — every rack carries the height of its own shelf — and lets bays of a
/// run differ in how many slots they hold, which a single vertical capacity
/// could not express.
fn build_racking(survey: &Survey, table: &CellTable) -> (Racking, BTreeMap<RackId, EntityId>) {
    let mut columns = Vec::new();
    let mut slots = BTreeMap::new();
    let mut relay_exchange_pos = BTreeMap::new();
    let mut board_floors_m = BTreeMap::new();
    let mut vtm_start_positions = FxHashMap::default();

    for (index, draft) in survey.zones.iter().enumerate() {
        let zone = ZoneId(index as u32 + 1);
        if !draft.board_floors_m.is_empty() {
            board_floors_m.insert(zone, draft.board_floors_m.clone());
        }
        let mut starts = Vec::new();
        for bay in &draft.bays {
            let Some(&cell) = table.rack_cells.get(&bay.rack) else { continue };
            starts.push((WorldPos { x_mm: bay.aisle_centre[0], y_mm: bay.aisle_centre[1] }, cell));
            for slot in &bay.slots {
                slots.insert(RackId(columns.len() as u32), slot.entity.clone());
                columns.push(RackColumn {
                    // The bay's own statement read against its aisle's: an
                    // order bay in an order aisle is the buffer, an order bay
                    // in an inventory aisle is the depot of empty order totes.
                    uses: RackUse::read(bay.holds, draft.holds),
                    cell,
                    world_pos: Some(WorldPos { x_mm: slot.position[0], y_mm: slot.position[1] }),
                    floor_m: Some(slot.floor_m),
                    side: Some(bay.side),
                });
            }
            for stand in &bay.stands {
                let Some(&id) = table.relays_by_stand.get(&(bay.rack.clone(), stand.cell.clone())) else { continue };
                relay_exchange_pos.insert(id, WorldPos { x_mm: stand.exchange[0], y_mm: stand.exchange[1] });
            }
        }
        // Preserve the former aisle-cell ordering when distributing the fleet.
        starts.sort_by_key(|(pos, _)| (pos.y_mm.round() as i64, pos.x_mm.round() as i64));
        vtm_start_positions
            .insert(zone, starts.into_iter().map(|(position, anchor)| VtmPosition::At { position, anchor }).collect());
    }
    (Racking { columns, relay_exchange_pos, board_floors_m, vtm_start_positions, vertical_tote_capacity: 1, robot_deployment: RobotDeployment::default() }, slots)
}

/// One entity's `region` as an axis-aligned millimetre rectangle
/// `[min_x, min_y, w, h]`, carried into the primary frame.
///
/// This is the one place a declared rectangle becomes a drawable one, so every
/// reader below -- floor, equipment, building, census -- gets the same corners
/// from the same arithmetic. `None` means the entity states no region, or
/// states one in no frame, which is a thing to skip rather than to refuse; a
/// frame chain that will not carry the corner IS refused, because a rectangle
/// nobody can place is not a rectangle.
fn region_rect(world: &World, entity: &Entity, unit: StepMm) -> Result<Option<[f64; 4]>, WorldMapError> {
    let Ok(Some(region)) = entity.component::<Region>("region") else { return Ok(None) };
    let Some(frame) = region.frame.clone() else { return Ok(None) };
    let a = [region.origin[0].0, region.origin[1].0];
    let b = [a[0] + region.size[0].0, a[1] + region.size[1].0];
    Ok(Some(rect_between(world, entity, &frame, a, b, unit)?))
}

/// The millimetre rectangle between two step-count corners of one frame,
/// normalized to min-corner form. The one corner arithmetic `region_rect` and
/// the carve-outs share, so a hole lands by exactly the sums its region did.
fn rect_between(
    world: &World,
    entity: &Entity,
    frame: &EntityId,
    a: [i64; 2],
    b: [i64; 2],
    unit: StepMm,
) -> Result<[f64; 4], WorldMapError> {
    let corner = |p: [i64; 2]| -> Result<[f64; 2], WorldMapError> {
        world
            .frames
            .to_primary(frame, [Coord(p[0]), Coord(p[1])])
            .map(|steps| [unit.length(steps[0]), unit.length(steps[1])])
            .map_err(|source| WorldMapError::Unreadable { entity: entity.id.clone(), message: source.to_string() })
    };
    let a = corner(a)?;
    let b = corner(b)?;
    Ok([a[0].min(b[0]), a[1].min(b[1]), (b[0] - a[0]).abs(), (b[1] - a[1]).abs()])
}

/// One handoff cell and the cells an HTM must come from and leave to.
///
/// A QUEUE was tried here and reverted. Making the approach a 3-deep
/// `dest_only`-gated corridor let collision mode run further, but it
/// serialised the handoff under Ghost as well: 5,275 picks became 610 and the
/// planner deadlocked. Only a robot already bound for the handoff may enter a
/// gated corridor, so eight handoffs each admitted one robot at a time where
/// before the approach was simply open. Any retry needs the Ghost A/B run
/// BEFORE the collision one, which is the mistake that hid it.
struct HandoffApproach {
    cell: CellId,
    from: CellId,
    to: CellId,
}

/// Every belt handoff cell, with the cell an HTM comes from and the one it
/// leaves to.
///
/// The two run OPPOSITE ways, which is what makes the pair a circulation
/// rather than two dead ends. A robot drops off heading SOUTH, off the highway
/// and into the machine; it collects heading NORTH, out of the machine and
/// back onto the highway. Ruled by the user, 2026-08-28.
///
/// Measured off world positions rather than a lattice, because a handoff cell
/// is minted by its port and stands in a hole carved out of the station floor:
/// it shares no lattice with either neighbour. North of DFW6's is the south
/// highway (892mm up); south is the loop's own floor (850mm down).
fn handoff_approaches(map: &WarehouseMap, lane_doors: &FxHashMap<CellId, CellId>) -> Vec<HandoffApproach> {
    let at = |c: CellId| map.graph.cell(c).and_then(|cell| cell.world_pos);
    let mut out = Vec::new();
    // A machine's TWO stations name the SAME pair of handoff cells, because the
    // belt and its two cells are shared. Without this the pair is described
    // twice and each approach edge is added twice.
    let mut seen: FxHashSet<CellId> = FxHashSet::default();
    for station in &map.stations {
        // A port THIS MACHINE'S OWN declared lane runs through needs no
        // inference: the lane already says which cell an HTM comes from and
        // which it leaves to, and `restrict_queue_corridors` has already cut
        // the graph to exactly those. Guessing a second pair on top would
        // re-add the edges the lane removed. Read per machine and not across
        // the whole map, because a lane belonging to some OTHER machine that
        // happened to run over this cell used to suppress this machine's
        // inference and leave its handover with no way in at all.
        // The inference stays for a document with no lane -- a document that
        // states its machine as a tiled block -- which is the only reason it is
        // still here.
        //
        // A lane's ENTRY cell counts as lane-bound too. A machine's dropoff is
        // the entry of its inventory lane and no longer a step of it (the
        // 2026-09-04 layout ruling), so the path alone does not name it; left
        // out, the inference would synthesise a north-south pair through the
        // dropoff by nearest-cell search and `restrict_handoff_approach` would
        // delete every other arc touching it -- the arc into the lane, and the
        // stitch back onto the apron the ruling exists to keep.
        let lane_bound: FxHashSet<CellId> = station
            .handoff_queues
            .iter()
            .flat_map(|q| q.path.iter().copied().chain(std::iter::once(q.into)).chain(lane_doors.get(&q.into).copied()))
            .collect();
        for cell in [station.dropoff_cell, station.collection_cell].into_iter().flatten() {
            if !seen.insert(cell) {
                continue;
            }
            if lane_bound.contains(&cell) {
                continue;
            }

            let Some(_here) = at(cell) else { continue };
            // Nearest cell each way, within a cell-ish band either side so a
            // lattice offset does not disqualify the obvious neighbour.
            let pick = |north: bool, from: CellId| {
                let here = at(from)?;
                map.graph
                    .cells()
                    .filter(|c| c.id != from)
                    .filter_map(|c| {
                        let p = c.world_pos?;
                        let dy = p.y_mm - here.y_mm;
                        let ok = if north { dy > 100.0 } else { dy < -100.0 };
                        (ok && (p.x_mm - here.x_mm).abs() < 1_000.0 && dy.abs() < 1_500.0).then_some((
                            dy.abs() as u64,
                            (p.x_mm - here.x_mm).abs() as u64,
                            c.id,
                        ))
                    })
                    .min()
                    .map(|(_, _, id)| id)
            };
            // Dropoff runs north to south; collection runs south to north.
            let southbound = Some(cell) == station.dropoff_cell;
            // Southbound: come FROM the north, go TO the south.
            //
            // An authored queue names the approach outright, and then the
            // guess yields to it: the handover is entered from the queue's last
            // cell. This synthesis is what MAKES the handover's through-edges
            // -- the cell is minted by its port and joined to the map by one
            // lateral contact stitch, so before this it has no way in or out at
            // all, and `restrict_queue_corridors` can only keep edges that
            // already exist. Without this the queue would island the handover.
            let queued = station
                .handoff_queues
                .iter()
                .find(|q| q.into == cell && !q.path.is_empty())
                .and_then(|q| q.path.last().copied());
            let (from, to) = (queued.or_else(|| pick(southbound, cell)), pick(!southbound, cell));
            if let (Some(from), Some(to)) = (from, to) {
                out.push(HandoffApproach { cell, from, to });
            }
        }
    }
    out
}

/// An HTM crosses a belt handoff one way only, and the two cells run opposite
/// ways: south through the dropoff, north through the collection.
///
/// Both are one-way by construction: a handoff cell is a single robot's
/// standing spot, and a machine whose robots can enter it from any side has no
/// queue discipline at all -- two HTMs meeting head-on at an ingress is a
/// deadlock, not a race. Opposed directions make the pair a circulation: in
/// off the highway, through the machine, back out to it.
///
/// This ADDS the two edges as well as removing the rest. A handoff cell is
/// minted by its port and joined to the map by one `contact` stitch, so before
/// this it had exactly one neighbour and that neighbour was lateral.
fn restrict_handoff_approach(graph: &WarehouseGraph, approaches: &[HandoffApproach]) -> WarehouseGraph {
    let handoffs: FxHashSet<CellId> = approaches.iter().map(|a| a.cell).collect();
    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        builder.add_cell(cell.clone());
    }
    for (src, tgt, edge) in graph.edges() {
        // An edge touching no handoff is none of this pass's business.
        if !handoffs.contains(&src) && !handoffs.contains(&tgt) {
            builder.add_edge(src, tgt, edge.clone());
        }
    }
    let span = |a: CellId, b: CellId| {
        let (pa, pb) = (graph.cell(a).and_then(|c| c.world_pos), graph.cell(b).and_then(|c| c.world_pos));
        match (pa, pb) {
            (Some(pa), Some(pb)) => ((pb.x_mm - pa.x_mm).hypot(pb.y_mm - pa.y_mm)) as u64,
            _ => 1,
        }
    };
    for a in approaches {
        // ADDED, not merely re-tagged: a handoff cell is minted by its port and
        // joined to the map by one lateral `contact` stitch, so the edges along
        // its approach do not exist to be gated.
        builder.add_edge(a.from, a.cell, Edge::uniform(span(a.from, a.cell)));
        builder.add_edge(a.cell, a.to, Edge::uniform(span(a.cell, a.to)));
    }
    builder.build()
}

/// A tiled entity's carve-outs as absolute millimetre rectangles: each
/// `exclusions` entry offset from the region's origin and carried through the
/// same corner arithmetic as the region itself.
fn exclusion_rects_mm(world: &World, entity: &Entity, unit: StepMm) -> Result<Vec<[f64; 4]>, WorldMapError> {
    let Ok(Some(tiling)) = entity.component::<Tiling>("tiling") else { return Ok(Vec::new()) };
    if tiling.exclusions.is_empty() {
        return Ok(Vec::new());
    }
    let Ok(Some(region)) = entity.component::<Region>("region") else { return Ok(Vec::new()) };
    let Some(frame) = region.frame.clone() else { return Ok(Vec::new()) };
    tiling
        .exclusions
        .iter()
        .map(|hole| {
            let a = [region.origin[0].0 + hole.origin[0].0, region.origin[1].0 + hole.origin[1].0];
            let b = [a[0] + hole.size[0].0, a[1] + hole.size[1].0];
            rect_between(world, entity, &frame, a, b, unit)
        })
        .collect()
}

/// `rect` minus `holes`, as axis-aligned pieces: the horizontal strips between
/// hole edges, each split around the holes crossing it. Holes are clipped to
/// the rectangle first, so a carve-out overhanging the region (a negative
/// localized origin, a sliver past the far edge) subtracts only what it
/// covers. Deterministic: cuts and spans are sorted, so the pieces come out
/// bottom-to-top, left-to-right regardless of hole order.
fn subtract_rects(rect: [f64; 4], holes: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let [rx, ry, rw, rh] = rect;
    let (rx1, ry1) = (rx + rw, ry + rh);
    let holes: Vec<[f64; 4]> = holes
        .iter()
        .filter_map(|&[hx, hy, hw, hh]| {
            let x0 = hx.max(rx);
            let y0 = hy.max(ry);
            let x1 = (hx + hw).min(rx1);
            let y1 = (hy + hh).min(ry1);
            (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0])
        })
        .collect();
    if holes.is_empty() {
        return vec![rect];
    }
    let mut cuts: Vec<f64> = vec![ry, ry1];
    for &[_, y0, _, hh] in &holes {
        cuts.push(y0);
        cuts.push(y0 + hh);
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut out = Vec::new();
    for pair in cuts.windows(2) {
        let (y0, y1) = (pair[0], pair[1]);
        if y1 <= y0 {
            continue;
        }
        let mut spans: Vec<(f64, f64)> = holes
            .iter()
            .filter(|&&[_, hy, _, hh]| hy < y1 && hy + hh > y0)
            .map(|&[hx, _, hw, _]| (hx, hx + hw))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = rx;
        for (sx0, sx1) in spans {
            if sx0 > x {
                out.push([x, y0, sx0 - x, y1 - y0]);
            }
            x = x.max(sx1);
        }
        if x < rx1 {
            out.push([x, y0, rx1 - x, y1 - y0]);
        }
    }
    out
}

/// The building envelope, when the document states one: an untiled region
/// carrying the `building` marker component. Same rectangle as
/// `floor_regions`; the difference is what it means -- walls, not floor.
fn building_rect(world: &World, unit: StepMm) -> Result<Option<[f64; 4]>, WorldMapError> {
    for entity in world.entities.values() {
        if !entity.has("building") {
            continue;
        }
        if let Some(rect) = region_rect(world, entity, unit)? {
            return Ok(Some(rect));
        }
    }
    Ok(None)
}

/// Queue corridors from world positions: the entry cell is the graph
/// neighbour of the first queue cell most opposite the queue's forward
/// direction, the exit cell the presentation's neighbour most along the
/// station's exit direction. Grid maps derive the same two cells by index
/// arithmetic; positions are what a gridless map has instead.
fn queue_corridors(map: &WarehouseMap) -> Result<Vec<QueueCorridor>, WorldMapError> {
    let pos = |c: CellId| map.graph.cell(c).and_then(|cell| cell.world_pos);

    // Cells with an arc INTO each queue head, collected in one pass.
    //
    // The entry cell must be found among q0's PREDECESSORS, not its
    // neighbours: `WarehouseGraph::neighbors` yields outgoing arcs only, and
    // on a one-way `flow` lane q0 has no arc back toward its tail. Searching
    // outgoing arcs there returns None, `restrict_queue_corridors` then
    // deletes the real inbound arc as unlisted, and the station is sealed off
    // with no error -- zero cycles for the whole run. (v3's rule asked only
    // whether the tail CELL existed, never about arc direction.)
    // Every authored approach lane, whichever cell it feeds. The queues in a
    // station's `presentation_points` feed the cells it presents at; a
    // `handoff_queue` serves a belt handover metres away. They are the same
    // object -- an ordered lane ending at one robot's standing spot -- so each
    // earns the same one-way-in, one-way-out corridor, and a machine's three
    // handovers get three.
    struct Lane {
        path: Vec<CellId>,
        feeds: CellId,
        /// Which way out of `feeds` the corridor keeps when no arc out of it is
        /// curved. `Some` for a lane into a cell a station presents at, whose
        /// exit the document states as a cardinal. `None` for a handoff queue:
        /// `exit_direction` describes where an HTM leaves the PRESENTATION, and
        /// a belt handover is somewhere else entirely -- DFW6's dropoffs exit
        /// north while their stations state `+x`. Rather than ask a document to
        /// state a second direction it has no reason to know, the lane keeps
        /// whatever onward edge the handover already has.
        exit: Option<CardinalDirection>,
        /// Who the corridor admits, and which of its edges says so; see
        /// `HandoffQueue::approach`. An approach gates its whole chain to
        /// `feeds`; a machine gates only the LAST arc -- the one out of the lane
        /// and onto `feeds` -- to the cells its belt hands carriers over at, so
        /// a robot may drive in to wait for the belt as well as to meet it.
        /// `Err` for a machine lane whose station names no handover at all,
        /// carried this far so the refusal names the cell.
        access: Result<CorridorAccess, WorldMapError>,
    }
    let lanes: Vec<Lane> = map
        .stations
        .iter()
        .flat_map(|s| {
            // The cells this station's belt hands carriers over at: what the
            // arc out of a machine's lane admits. Sorted by construction
            // (dropoff first), so the gate is the same object across two runs of
            // one document.
            let handovers: Vec<CellId> = [s.dropoff_cell, s.collection_cell].into_iter().flatten().collect();
            // Each cell the station presents at is a standing spot entered
            // from its own lane, and left by its curved exit when it has one,
            // else by the onward neighbour in the station's exit direction.
            let presentations =
                s.presentation_points.iter().filter(|point| !point.queue_path.is_empty()).map(move |point| Lane {
                    path: point.queue_path.clone(),
                    feeds: point.presentation_cell,
                    exit: Some(s.exit_direction),
                    access: Ok(CorridorAccess::Approach),
                });
            let handoffs = s.handoff_queues.iter().filter(|q| !q.path.is_empty()).map(move |q| Lane {
                path: q.path.clone(),
                feeds: q.into,
                exit: None,
                access: if q.approach {
                    Ok(CorridorAccess::Approach)
                } else {
                    match DestGate::of(&handovers) {
                        Some(gate) => Ok(CorridorAccess::Machine { handovers: gate }),
                        None => Err(WorldMapError::MachineLaneWithoutHandover { cell: q.into }),
                    }
                },
            });
            presentations.chain(handoffs)
        })
        .collect();

    let heads: FxHashSet<CellId> = lanes.iter().filter_map(|l| l.path.first().copied()).collect();
    let mut into_head: FxHashMap<CellId, SmallVec<[CellId; 4]>> = FxHashMap::default();
    if !heads.is_empty() {
        for cell in map.graph.cells() {
            for (n, _) in map.graph.neighbors(cell.id) {
                if heads.contains(&n) {
                    into_head.entry(n).or_default().push(cell.id);
                }
            }
        }
    }
    // The neighbour of `of` whose direction from it best matches `dir`
    // (unit-agnostic: cosine > 0.7 keeps diagonals of the same general
    // heading out).
    // `dir` is measured FROM `of` toward the candidate, whichever list the
    // candidates come from.
    let toward = |of: CellId, dir: [f64; 2], candidates: &[CellId]| -> Option<CellId> {
        let origin = pos(of)?;
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        if len == 0.0 {
            return None;
        }
        let mut best: Option<(f64, CellId)> = None;
        for &n in candidates {
            let Some(np) = pos(n) else { continue };
            let (dx, dy) = (np.x_mm - origin.x_mm, np.y_mm - origin.y_mm);
            let nlen = (dx * dx + dy * dy).sqrt();
            if nlen == 0.0 {
                continue;
            }
            let cos = (dx * dir[0] + dy * dir[1]) / (nlen * len);
            if cos > 0.7 && best.map_or(true, |(b, _)| cos > b) {
                best = Some((cos, n));
            }
        }
        best.map(|(_, c)| c)
    };

    lanes
        .into_iter()
        .filter_map(|lane| {
            let access = match lane.access {
                Ok(access) => access,
                Err(refusal) => return Some(Err(refusal)),
            };
            let q0 = lane.path[0];
            let next = lane.path.get(1).copied().unwrap_or(lane.feeds);
            let (q0p, nextp) = (pos(q0)?, pos(next)?);
            let forward = map
                .graph
                .edge_between(q0, next)
                .and_then(|edge| edge.curve)
                .map_or([nextp.x_mm - q0p.x_mm, nextp.y_mm - q0p.y_mm], |arc| arc.start_tangent(q0p));
            // Predecessors for the entry (see `into_head`); the exit is an
            // outgoing arc, so it searches neighbours as before.
            let tail_candidates = into_head.get(&q0).map(|v| v.as_slice()).unwrap_or(&[]);
            let entry_from = toward(q0, [-forward[0], -forward[1]], tail_candidates);
            let exit_neighbors: SmallVec<[CellId; 4]> =
                map.graph.neighbors(lane.feeds).into_iter().map(|(n, _)| n).collect();
            let curved_exit =
                map.graph.neighbors(lane.feeds).into_iter().find_map(|(cell, edge)| edge.curve.map(|_| cell));
            let exit_to = if curved_exit.is_some() {
                curved_exit
            } else {
                match lane.exit {
                    // The loader maps "-y" -> North, so North is -y in document mm.
                    Some(dir) => {
                        let exit_dir = match dir {
                            CardinalDirection::North => [0.0, -1.0],
                            CardinalDirection::South => [0.0, 1.0],
                            CardinalDirection::East => [1.0, 0.0],
                            CardinalDirection::West => [-1.0, 0.0],
                        };
                        toward(lane.feeds, exit_dir, &exit_neighbors)
                    }
                    // Keep the onward edge the handover already has: anything that
                    // is not back into the queue it was entered from.
                    None => exit_neighbors.iter().copied().find(|n| !lane.path.contains(n)),
                }
            };
            Some(Ok(QueueCorridor { queue: lane.path.clone(), presentation: lane.feeds, entry_from, exit_to, access }))
        })
        .collect()
}

/// Untiled regions on entities carrying `go.easyloop` with a `part`: the
/// vendor-declared equipment rectangles (§floor_regions handles the tiled
/// ones). Untiled is load-bearing -- a tiling would mint cells, and an
/// EasyLoop envelope contains the conveyor no HTM may cross; the region is
/// survey truth for drawing and audit only.
fn equipment_regions(world: &World, unit: StepMm) -> Result<Vec<Equipment>, WorldMapError> {
    let mut out = Vec::new();
    for entity in world.entities.values() {
        if entity.has("tiling") {
            continue;
        }
        let Some(easyloop) = entity.components.get("go.easyloop") else { continue };
        let Some(part) = easyloop.get("part").and_then(|v| v.as_str()) else { continue };
        let Some(rect_mm) = region_rect(world, entity, unit)? else { continue };
        let htm_traversable = easyloop
            .get("traversable_by")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.iter().any(|c| c.as_str() == Some("htm")));
        out.push(Equipment { rect_mm, part: part.to_string(), htm_traversable });
    }
    // Deterministic order for pins and diffs: by part, then x, then y.
    out.sort_by(|a, b| {
        a.part.cmp(&b.part).then(a.rect_mm[0].total_cmp(&b.rect_mm[0])).then(a.rect_mm[1].total_cmp(&b.rect_mm[1]))
    });
    Ok(out)
}

/// The document's tiled regions as drawable rectangles. Kind mirrors
/// `cells::way_kind`: charging marker wins, then flow makes a highway, VTM-only
/// floor is an aisle, everything else is open floor.
fn floor_regions(world: &World, unit: StepMm) -> Result<Vec<Floor>, WorldMapError> {
    let mut out = Vec::new();
    for entity in world.entities.values() {
        if !entity.has("tiling") {
            continue;
        }
        let Some(rect_mm) = region_rect(world, entity, unit)? else { continue };
        let derives = entity.components.get("tiling").and_then(|t| t.get("derives"));
        let has = |name: &str| derives.is_some_and(|d| d.get(name).is_some());
        let classes: Vec<String> = derives
            .and_then(|d| d.get("cell"))
            .and_then(|c| c.get("traversable_by"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let kind = if has("charging") {
            CellKind::Charging
        } else if entity.has("queue") {
            // Drawing tint only: a tiled region that IS a queue paints as
            // queue floor, so an authored approach layout is verifiable by
            // eye. The derived cells stay Open -- QueueCell semantics attach
            // when a station binds the queue, not from the paint.
            CellKind::QueueCell
        } else if has("highway") {
            CellKind::Highway
        } else if entity.has("flow") {
            CellKind::Highway
        } else if classes.iter().any(|c| c == "vtm") && !classes.iter().any(|c| c == "htm") {
            CellKind::Aisle
        } else {
            CellKind::Open
        };
        // The paint honors the carve-outs the way the cells already do: the
        // slab is the region minus its tiling's exclusions, so a hole shows
        // whatever is really there -- usually nothing.
        let holes = exclusion_rects_mm(world, entity, unit)?;
        for piece in subtract_rects(rect_mm, &holes) {
            out.push(Floor { rect_mm: piece, kind });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::subtract_rects;

    fn area(pieces: &[[f64; 4]]) -> f64 {
        pieces.iter().map(|[_, _, w, h]| w * h).sum()
    }

    /// No piece may overlap a hole or another piece; together with the area
    /// check this pins the decomposition without caring how it slices.
    fn assert_disjoint(pieces: &[[f64; 4]], holes: &[[f64; 4]]) {
        let overlap = |a: &[f64; 4], b: &[f64; 4]| {
            a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
        };
        for (i, p) in pieces.iter().enumerate() {
            for hole in holes {
                assert!(!overlap(p, hole), "piece {p:?} overlaps hole {hole:?}");
            }
            for q in &pieces[i + 1..] {
                assert!(!overlap(p, q), "pieces {p:?} and {q:?} overlap");
            }
        }
    }

    #[test]
    fn no_holes_returns_the_rectangle_itself() {
        assert_eq!(subtract_rects([0.0, 0.0, 10.0, 10.0], &[]), vec![[0.0, 0.0, 10.0, 10.0]]);
    }

    #[test]
    fn a_hole_strictly_inside_leaves_a_ring_of_pieces() {
        let rect = [0.0, 0.0, 10.0, 10.0];
        let holes = [[4.0, 4.0, 2.0, 2.0]];
        let pieces = subtract_rects(rect, &holes);
        assert_eq!(area(&pieces), 100.0 - 4.0);
        assert_disjoint(&pieces, &holes);
    }

    #[test]
    fn a_hole_overhanging_the_edge_subtracts_only_what_it_covers() {
        // The shipped case: a carve-out with a negative localized origin, or a
        // sliver past the far edge, clips to the region before subtracting.
        let rect = [0.0, 0.0, 10.0, 10.0];
        let holes = [[-5.0, -5.0, 8.0, 8.0]];
        let pieces = subtract_rects(rect, &holes);
        assert_eq!(area(&pieces), 100.0 - 9.0);
        assert_disjoint(&pieces, &holes);
    }

    #[test]
    fn a_hole_covering_everything_leaves_nothing() {
        assert!(subtract_rects([0.0, 0.0, 10.0, 10.0], &[[-1.0, -1.0, 12.0, 12.0]]).is_empty());
    }

    #[test]
    fn a_hole_entirely_outside_changes_nothing() {
        let rect = [0.0, 0.0, 10.0, 10.0];
        assert_eq!(subtract_rects(rect, &[[20.0, 20.0, 5.0, 5.0]]), vec![rect]);
    }

    #[test]
    fn edge_touching_holes_subtract_no_area() {
        // Flush against the boundary: open-interval overlap says no shared
        // area, mirroring the reader's rule that such a carve-out removes no
        // cell.
        let rect = [0.0, 0.0, 10.0, 10.0];
        assert_eq!(subtract_rects(rect, &[[10.0, 0.0, 5.0, 10.0]]), vec![rect]);
    }

    #[test]
    fn overlapping_holes_do_not_double_subtract() {
        let rect = [0.0, 0.0, 10.0, 10.0];
        let holes = [[0.0, 4.0, 6.0, 2.0], [4.0, 4.0, 6.0, 2.0]];
        let pieces = subtract_rects(rect, &holes);
        // One 10x2 band carved by the union of the two: 100 - 20 = 80.
        assert_eq!(area(&pieces), 80.0);
        assert_disjoint(&pieces, &holes);
    }

    #[test]
    fn the_cencora_end_cap_shape_splits_the_band_into_strip_and_teeth() {
        // The lower blanket band: full-width, with 3-column bites out of its
        // last two rows at each rack run -- the shape that must NOT paint over
        // the end caps.
        let rect = [0.0, 0.0, 130050.0, 17000.0];
        let holes: Vec<[f64; 4]> = (0..24).map(|i| [850.0 + i as f64 * 5375.0, 15300.0, 2550.0, 1700.0]).collect();
        let pieces = subtract_rects(rect, &holes);
        assert_eq!(area(&pieces), 130050.0 * 17000.0 - 24.0 * 2550.0 * 1700.0);
        assert_disjoint(&pieces, &holes);
        // One solid strip below the bite row, then the surviving teeth.
        assert_eq!(pieces.len(), 1 + 25);
    }
}
