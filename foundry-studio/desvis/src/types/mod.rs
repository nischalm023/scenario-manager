//! Within desvis, `types` is the UI STATE VOCABULARY: the enums and state
//! structs that panels share (app/sim status, view selectors, metrics-panel
//! selections) so modules don't couple to each other's
//! internals.
//!
//! TODO(overstuffed): this is a bucket whose residents have drifted apart —
//! sim lifecycle status, metrics selections, and playback
//! types are different concerns. Split by owning surface when one of them
//! next grows.

mod metric_extract;
mod metrics;
pub use metrics::{MetricKind, RollingConfig, RollingKind};

use desgraph::types::{BlockSide, RackOrientation};

use desgraph::{StationId, StationType};
use desmap::PresentationPoint;
use dessim::SimResult;
use dessim::metrics::profile::PerfProfile;
use std::ops::Deref;
use worldview_floor::easyloop::Placement;

use desunits::RobotId;
// The renderer's own vocabulary, named here as well: a panel that puts a
// robot or an overlay in a tooltip reads the same type the renderer draws it
// from, and desvis has one word for it.
pub use worldview_render::{
    BotHighlight, BotType, Cell, HeatmapData, HeatmapMode, Place, Robot, Scene as DrawnScene, Tile,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SIDEBAR_WIDTH: f32 = 220.0;

// ---------------------------------------------------------------------------
// Traversal view mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    State,
    UnladenHtm,
    LadenHtm,
    Vtm,
    Simulation,
}

impl std::fmt::Display for ViewMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewMode::State => write!(f, "State"),
            ViewMode::UnladenHtm => write!(f, "Unladen D30"),
            ViewMode::LadenHtm => write!(f, "Laden D30"),
            ViewMode::Vtm => write!(f, "DA50"),
            ViewMode::Simulation => write!(f, "Simulation"),
        }
    }
}

// ---------------------------------------------------------------------------
// Bot markers for State view
// ---------------------------------------------------------------------------

/// One robot as DESVIS holds it: the cell the run last reported it on, the
/// position its playback interpolated between the steps, and the heading
/// `app::heading` swept across a rotation.
///
/// It is desvis's reading of the run, not the renderer's input: [`Self::robot`]
/// turns it into the row the renderer takes, once per frame, with whatever the
/// selection is highlighting.
#[derive(Debug, Clone)]
pub struct BotMarker {
    pub cell_id: u32,
    pub bot_type: BotType,
    pub bot_id: u32,
    /// True position in world millimetres. Every source of a marker states
    /// one, so a robot is never placed from its cell.
    pub pos_mm: [f64; 2],
    /// Facing in radians: 0 points along +x, turning toward +y.
    pub heading_rad: f32,
}

impl BotMarker {
    /// Where this robot stands, in the renderer's terms.
    pub fn place(&self) -> Place {
        Place { cell: self.cell_id, pos_mm: self.pos_mm }
    }

    /// The row the renderer draws this robot from.
    pub fn robot(&self, highlight: BotHighlight) -> Robot {
        Robot {
            id: RobotId(self.bot_id),
            class: self.bot_type,
            at: self.place(),
            heading_rad: self.heading_rad,
            highlight,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VtmAccessMarker {
    pub cell_id: u32,
    pub relay_position: u32,
    pub block_id: u32,
    pub side: BlockSide,
}

#[derive(Debug, Clone)]
pub struct HtmAccessMarker {
    pub cell_id: u32,
    pub station_id: u32,
}

// ---------------------------------------------------------------------------
// Resource selection (station inspector)
// ---------------------------------------------------------------------------

/// The map resource currently selected by a single click in the State view.
/// Drives the inspector window; cleared by clicking empty floor or closing
/// the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedResource {
    Station(StationId),
    /// Bot ids are only unique per type (VTMs and HTMs share numbering), so
    /// the type rides along.
    Bot {
        bot_type: BotType,
        bot_id: u32,
    },
    /// A relay, keyed by its HTM-access cell id (the cell the user clicks and
    /// the key relay diagnostics flow under).
    Relay {
        htm_access_cell: u32,
    },
}

/// Static station facts for click resolution and the inspector header.
/// Built once from the warehouse map (available in both live and playback
/// modes at construction, before the map is dropped in playback).
#[derive(Debug, Clone)]
pub struct StationMeta {
    pub process_label: Option<&'static str>,
    pub id: StationId,
    pub station_type: StationType,
    /// The cells the station presents at, each with the queue of the lane that
    /// feeds it, if one does.
    pub presentation_points: Vec<PresentationPoint>,
    /// The cell an HTM bound for this station drives to: the one the flows
    /// deliver to.
    pub delivery_cell: u32,
    /// Per-station bin override from the map; `None` = system default.
    pub bins: Option<usize>,
    /// Every cell id that selects this station (its position, and each
    /// presentation point's cell and queue path) — also the highlight outline set.
    pub cell_ids: Vec<u32>,
}

/// Static relay facts for click resolution and the inspector header. Built
/// once from the map. A relay is selectable by either its HTM-access or
/// VTM-access cell (both resolve to the same relay), but keyed by
/// HTM-access — the cell diagnostics flow under.
#[derive(Debug, Clone)]
pub struct RelayMeta {
    pub relay_id: u32,
    pub htm_access_cell: u32,
    pub vtm_access_cell: u32,
    pub block_id: u32,
    /// The relay's cell ids (position, htm-access, vtm-access) for the
    /// selection highlight.
    pub cell_ids: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Heatmap mode
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Simulation status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SimStatus {
    Idle,
    Running,
    Complete(SimResult),
    Error(String),
}

// ---------------------------------------------------------------------------
// Data source
// ---------------------------------------------------------------------------

pub enum DataSource {
    None,
    Live,
    Profile(Box<PerfProfile>),
}

// ---------------------------------------------------------------------------
// Bottom panel view (tab selector)
// ---------------------------------------------------------------------------

/// Which view the bottom panel renders: metrics or the utilization panel.
/// User-toggled via tabs at the top of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomPanelView {
    Metrics,
    Utilization,
}

/// Metric family rendered inside the Metrics bottom-panel view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsView {
    TimeSeries,
    Da50State,
    Da50Throughput,
    OrderExtraction,
    BufferStorage,
    AisleDistribution,
    Destinations,
    D30State,
}

impl MetricsView {
    pub const ALL: [Self; 8] = [
        Self::TimeSeries,
        Self::Da50State,
        Self::Da50Throughput,
        Self::OrderExtraction,
        Self::BufferStorage,
        Self::AisleDistribution,
        Self::Destinations,
        Self::D30State,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::TimeSeries => "Time series",
            Self::Da50State => "DA50 state",
            Self::Da50Throughput => "DA50 throughput",
            Self::OrderExtraction => "Order extraction",
            Self::BufferStorage => "Buffer & storage",
            Self::AisleDistribution => "Aisle distribution",
            Self::Destinations => "Destinations",
            Self::D30State => "D30 state",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::TimeSeries => "Simulation metrics over time",
            Self::Da50State => "DA50 time, waiting attribution, and travel distance",
            Self::Da50Throughput => "DA50 relay and storage operations by tote contents",
            Self::OrderExtraction => "Completed extraction operations and units at extraction stations",
            Self::BufferStorage => "Aisle fill, relay occupancy, and stored tote contents",
            Self::AisleDistribution => "Tote distribution by aisle and pick station",
            Self::Destinations => "Planned stripes and completed work by destination and pick station",
            Self::D30State => "D30 time, waiting attribution, and travel distance",
        }
    }
}

/// Persistent controls shared by the canonical Metrics report views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsPanelState {
    pub scope: MetricsScopeSelection,
    pub trailing_window_ms: u64,
    pub trailing_resolution_ms: u64,
    pub da50_payload: MetricsPayloadSelection,
    pub selected_da50: Option<u32>,
    pub selected_d30: Option<u32>,
    pub dc_measure: DcMeasureSelection,
    pub dc_display: MatrixDisplaySelection,
    /// Which matrix the Destinations tab shows: completed facts or the
    /// planned stripe map. Planned and completed are never mixed in one
    /// cell (specs/dematic_kpi_reporting/plan.md separation rule).
    pub destinations_source: DestinationsSourceSelection,
    /// Destination isolated in the throughput chart, as an index into the
    /// chart's row-ordered destination list; `None` shows all lines.
    pub chart_destination_index: Option<usize>,
    /// The timeline cursor: a sim time selected by clicking the docked
    /// throughput chart. `Some` snaps BOTH matrices to that instant (Picks
    /// = the bucket under the cursor, Assigned stripes = the governing
    /// snapshot); `None` = run totals / latest snapshot.
    pub chart_cursor_ms: Option<u64>,
    /// Pinned aisle in the distribution view; `None` means no row is pinned.
    pub selected_aisle: Option<u32>,
    pub selected_dc_index: Option<usize>,
}

impl Default for MetricsPanelState {
    fn default() -> Self {
        Self {
            scope: MetricsScopeSelection::FullRun,
            // Trailing 15m @ 5m is the sole temporal lens and its default
            // width (Daniel, 2026-07-30); the Period menu is retired — a
            // 60m trailing window reproduces Hourly when needed.
            trailing_window_ms: 15 * 60 * 1_000,
            trailing_resolution_ms: 5 * 60 * 1_000,
            da50_payload: MetricsPayloadSelection::All,
            selected_da50: None,
            selected_d30: None,
            dc_measure: DcMeasureSelection::Totes,
            dc_display: MatrixDisplaySelection::Count,
            destinations_source: DestinationsSourceSelection::Picks,
            chart_destination_index: None,
            chart_cursor_ms: None,
            selected_aisle: None,
            selected_dc_index: None,
        }
    }
}

/// Which data source the Destinations matrix renders. Measured pick facts
/// and the planned stripe map share axes (destination × pick station) but
/// are separate matrices by rule — never mixed in one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationsSourceSelection {
    /// Completed pick/order work (measured facts).
    Picks,
    /// The stripe plan the run was composed with (configuration, not
    /// measurement).
    Planned,
}

impl DestinationsSourceSelection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Picks => "Picks",
            Self::Planned => "Assigned stripes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsScopeSelection {
    FullRun,
    PostEmbargo,
}

impl MetricsScopeSelection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::FullRun => "Full run",
            Self::PostEmbargo => "Post-embargo",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsPayloadSelection {
    All,
    Empty,
    NonEmpty,
    Unknown,
}

impl MetricsPayloadSelection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All totes",
            Self::Empty => "Empty totes",
            Self::NonEmpty => "Non-empty totes",
            Self::Unknown => "Unknown contents",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DcMeasureSelection {
    Totes,
    Orders,
}

impl DcMeasureSelection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Totes => "Totes",
            Self::Orders => "Orders",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixDisplaySelection {
    Count,
    Percentage,
}

impl MatrixDisplaySelection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Count => "Count",
            Self::Percentage => "Percentage",
        }
    }
}

#[cfg(test)]
mod bottom_panel_tests {
    use std::collections::HashSet;

    use super::MetricsView;

    #[test]
    fn metric_view_labels_are_unique() {
        let labels: HashSet<_> = MetricsView::ALL.into_iter().map(MetricsView::label).collect();

        assert_eq!(labels.len(), MetricsView::ALL.len());
    }
}

// ---------------------------------------------------------------------------
// Edge info for drawing
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The drawable scene
// ---------------------------------------------------------------------------

/// What desvis derives from a world document at load, to draw and to talk
/// about: the renderer's scene — the floor, the hops, the steel and the
/// ground under them — and the conveyor drawings it places, and the notes its
/// panels read about each cell. A new drawable element is a new field HERE,
/// built where the map is loaded and consumed where the GPU buffers are built
/// -- never a new loose parameter threaded hand to hand.
/// Tiled regions are deliberately NOT here: a region is a rule, not a thing,
/// and nothing draws for it -- its cells carry the floor's color themselves.
pub struct Scene {
    pub drawn: DrawnScene,
    /// What a panel says about a cell, beside what the renderer draws of it.
    pub cell_infos: Vec<Option<CellInfo>>,
    /// Conveyor drawings placed in their dropoff cells' declared frames.
    pub easyloops: Vec<Placement>,
}

/// One cell as DESVIS holds it: the cell the renderer draws, and what this
/// viewer knows about it that nothing draws — which zone it is in, what its
/// tooltip says, why it is left out of the waiting overlay.
///
/// The deref is the composition said plainly: a `CellInfo` IS the drawn cell,
/// with desvis's notes beside it, so a panel reads `info.kind` and
/// `info.center_mm()` from the one place the renderer reads them.
#[derive(Clone)]
pub struct CellInfo {
    pub cell: Cell,
    /// Shared map-derived reason this cell is omitted from navigational waits.
    pub navigation_wait_exclusion: Option<&'static str>,
    pub zone_id: u32,
    pub rack_orientation: Option<RackOrientation>,
    pub outgoing_edges: u32,
    pub relay_info: Option<String>,
    pub station_info: Option<String>,
}

impl Deref for CellInfo {
    type Target = Cell;

    fn deref(&self) -> &Cell {
        &self.cell
    }
}

// ---------------------------------------------------------------------------
// Legacy time-slice heatmap data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TimeSlice {
    pub vtm_counts: Vec<u32>,
    pub htm_counts: Vec<u32>,
    /// Pre-computed max of vtm_counts to avoid linear scan on cursor move.
    pub vtm_max: u32,
    /// Pre-computed max of htm_counts to avoid linear scan on cursor move.
    pub htm_max: u32,
}
