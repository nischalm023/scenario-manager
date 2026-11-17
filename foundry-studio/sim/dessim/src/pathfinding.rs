//! dessim's face on `pathfinder`: the crate's pathfinder, path and oracle
//! vocabulary at the paths the simulator has always used, plus what stays
//! the simulator's own — the spatial estimate cache, nearest-cell selection,
//! the holding-cell rule, and the one place a run assembles its pathfinder
//! and its HTMs' drives.

mod cache;
pub mod estimate;
mod holding_cells;
mod nearest;

use std::sync::Arc;

use desmap::WarehouseMap;

use crate::config::SimConfig;

pub use crate::model::reservation::oracle::TableOracle;
pub use crate::model::reservation::table::{ReservationConflict, ReservationTable};
pub use cache::{Cached, PathCacheKey};
pub use holding_cells::holding_cells;
pub(crate) use nearest::NearestCellSelector;
pub use nearest::PathSelectionError;
pub use nearest::{CellCandidate, CellSelection};
pub use path::{CommitPlan, MoveOp, ReservationRequest, ReservedPath, Waypoint, WaypointAction};
pub use pathfinder::{
    CollisionConfig, Constant, DriveModel, FHSippPathfinder, Heading, HtmKinematics, KinematicParams, MotionModel,
    Orientation, Path, Pathfinder, PathfinderError, ReservationOracle, SearchStats, Trapezoidal, Velocity, WindowSink,
    is_journey_end,
};
pub(crate) use pathfinder::{TrapezoidalProfile, arrival_orientation, trapezoid_position, trapezoidal_profile};
pub use pathfinder::{path, pathfinder_trace, stats};

/// The pathfinder a run gets: the shared speed read off the run's own
/// configuration, indefinite holds permitted where a robot is allowed to stand
/// (relay access and charging cells). The HTM's figures go to its drives
/// instead ([`htm_estimate_drives`], [`htm_drives`]).
///
/// One place, because every caller that assembles a run — desvis, the Crucible
/// server, the integration tests — has to hand the composition the same
/// pathfinder, and three spellings of it were three chances to differ.
pub fn fh_sipp_for_map(map: &WarehouseMap, config: &SimConfig) -> Arc<dyn Pathfinder> {
    let speed_mm_per_sec = (config.vtm.speed * 1000.0) as u64;
    let holding = holding_cells(map);
    let pathfinder = FHSippPathfinder::new(map, speed_mm_per_sec, SimDuration::seconds(1), holding);
    #[cfg(feature = "pathfinder-trace")]
    let pathfinder = pathfinder.with_trace_hook(pathfinder_trace::stderr_trace_hook);
    Arc::new(pathfinder)
}

/// The HTM's drives every estimate runs with, `[empty, laden]` (index by
/// `carrying_cargo as usize`). Every estimate (`find_path`, the path cache's
/// searches) runs at the catalog maximum for that load state. Constant-mode
/// execution has its own cruise; it does not set these spatial estimates.
pub fn htm_estimate_drives(config: &SimConfig) -> [Arc<dyn DriveModel>; 2] {
    let figures = HtmKinematics {
        loaded_constant_speed_mm_s: config.htm.loaded_speed * 1000.0,
        empty_constant_speed_mm_s: config.htm.empty_speed * 1000.0,
        ..config.htm.to_kinematics()
    };
    [Arc::new(Constant::htm(&figures, false)), Arc::new(Constant::htm(&figures, true))]
}

/// The drives an HTM's reservations are priced with under the run's
/// `motion_model`, `[empty, laden]` (index by `carrying_cargo as usize`):
/// constant under `ConstantSpeed`, trapezoidal under `HtmKinematic`. The drive
/// follows the bot's config: its figures are read off the run's HTM
/// configuration.
pub fn htm_drives(config: &SimConfig) -> [Arc<dyn DriveModel>; 2] {
    let figures = config.htm.to_kinematics();
    match config.motion_model {
        MotionModel::ConstantSpeed => {
            [Arc::new(Constant::htm(&figures, false)), Arc::new(Constant::htm(&figures, true))]
        }
        MotionModel::HtmKinematic => {
            [Arc::new(Trapezoidal::htm(&figures, false)), Arc::new(Trapezoidal::htm(&figures, true))]
        }
    }
}

use crate::model::SimDuration;
