//! # worldview-render
//!
//! The wgpu renderer a warehouse floor is drawn with, shared by the two
//! viewers: desvis on the desktop, and the browser viewer that draws a
//! recording the Crucible server streams.
//!
//! **What it is for.** A fix to how a robot, a container or a rack is drawn is
//! made once and both viewers get it. The two had their own copies of this
//! code and had already drifted, and nothing but a screenshot could tell.
//!
//! **What is here.** [`scene`] — what there is to draw; [`frame`] — the
//! metres everything is drawn in; [`rack`] — the steel and the heights a
//! container rests at; [`camera`] — where the viewer stands, what a pointer
//! gesture means, and where on the screen a point in metres lands;
//! [`cell_index`] — which cell's square a millimetre point falls in;
//! [`recording`] — a run's own words read as the things above;
//! [`document`] — a scene written down, for the viewer that did not work it
//! out itself; and [`renderer`] — the pipelines, the instance rows they are fed, and the draw
//! calls, over a wgpu device, queue and surface format a host hands in.
//!
//! **What is not here.** Anything that knows how a run is read or how a
//! window is put on screen: no egui, no DOM, no simulator. A caller fills in
//! [`scene`]'s plain data from wherever it reads a run.

pub mod camera;
pub mod cell_index;
pub mod document;
pub mod frame;
pub mod rack;
pub mod recording;
pub mod renderer;
pub mod scene;
pub mod view;

pub use camera::{CameraState, CameraUniforms, PointerInput, Viewport};
pub use cell_index::CellIndex;
pub use document::{Census, FLOOR_VERSION, FloorAndRoll, FloorDoc, RollEntry};
pub use frame::{FLOOR_CLEARANCE_M, RenderFrame};
pub use rack::{BeltRoute, ContainerPose, EasyLoopBelt, RackBox, RackBoxKind, RackVisual};
pub use recording::overlays::{CellTraffic, SPEED_BINS, Traffic, VisitCounter, slice_visits};
pub use recording::{Drawn, from_recording, visit_counts};
pub use renderer::{
    BotGpuResources, BotInstance, CellInstance, DEPTH_FORMAT, MSAA_SAMPLES, Moment, RackFrameGpuResources,
    SceneRenderer, ToteGpuResources, ToteInstance, build_bot_instances, build_tote_instances, place_pos, place_z,
};
pub use scene::{
    BotHighlight, BotType, Cell, ContainerKind, Designation, Edge, Floor, FloorPatch, HeatmapData, HeatmapMode,
    Overlays, Place, Playback, Robot, Scene, Tile, Tote, ToteAppearance, TrafficTints, Underlay,
};
pub use view::{ArrowSet, Measured, ViewMode};
// The wgpu a host must hand this crate its device and queue from: the one
// version both viewers link, not whichever one they name themselves.
pub use wgpu;
/// Where a container is, in the recording's own words — the vocabulary both
/// viewers place containers by.
pub use worldview::frames::playback::{ToteLocation, ToteTransit};
