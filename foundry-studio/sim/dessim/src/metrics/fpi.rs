//! Fleet Pressure Index (FPI): the mean over relays of the nearest-station
//! one-way HTM delivery leg.
//!
//! Computed with a constant-speed 1.5 m/s HTM model via A* so it is a stable
//! *geometric* property of the map, independent of the run's motion model.
//! This is the single source of truth for the value the `desvis fpi`
//! diagnostic reports and for auto-deriving the putaway direct-chain bias
//! (`putaway.direct_bias_secs = auto`): empirically, the optimal bias tracks
//! one fetch leg, so FPI is a map-scaled default that beats a magic constant.

use std::collections::HashSet;
use std::sync::Arc;

use desmap::WarehouseMap;
use rayon::prelude::*;

use crate::pathfinding::{Constant, DriveModel};
use crate::{Cached, FHSippPathfinder, HtmKinematics, Orientation, PathCacheKey, SimDuration};

/// HTM travel speed in mm/s (1.5 m/s), matching the `desvis fpi` diagnostic.
const HTM_SPEED_MM_S: u64 = 1500;

/// Mean over relays of the nearest reachable station's one-way HTM-carry
/// fetch leg. Returns `SimDuration::ZERO` for maps with no relays or stations.
///
/// A (relay, station) pair counts if the carry leg (relay→station) is
/// reachable. The `desvis fpi` diagnostic additionally requires the return
/// leg to be reachable; we drop that check here — in a connected warehouse it
/// never excludes a pair, and skipping it halves the A* queries at solver
/// init. (Verified to leave the computed FPI unchanged on cencora_phase2.)
pub fn fleet_pressure_index(map: &WarehouseMap) -> SimDuration {
    if map.relays.is_empty() || map.stations.is_empty() {
        return SimDuration::ZERO;
    }

    // HTM carry legs cruise at a fixed HTM_SPEED_MM_S (1.5 m/s) via a uniform
    // bundle — NOT the M5F production constants. FPI is a stable geometric
    // reference and must not move when the loaded M5F kinematics are tuned
    // (e.g. the 2026-06 field calibration that raised loaded to 4.5 m/s);
    // HtmKinematics::uniform pins the carry-leg cost to 1.5 m/s regardless.
    // HTM_SPEED_MM_S is also the shared speed for the heuristic baseline;
    // VTM/RTP legs are never queried here.
    let htm = HtmKinematics::uniform(HTM_SPEED_MM_S, SimDuration::seconds(1));
    let pathfinder = FHSippPathfinder::new(map, HTM_SPEED_MM_S, SimDuration::seconds(1), HashSet::new());
    // The FPI runs at constant speed, so the same constant drives search and convert.
    let drives: [Arc<dyn DriveModel>; 2] = [Arc::new(Constant::htm(&htm, false)), Arc::new(Constant::htm(&htm, true))];
    let cache = Cached::new(pathfinder, drives.clone(), drives);

    // Per relay, the nearest reachable station's one-way carry leg. Run in
    // parallel: this is ~relays×stations A* queries (tens of thousands on a
    // real map), and a sequential pass adds ~20s of solver-init stall.
    let per_relay_ms: Vec<u64> = map
        .relays
        .par_iter()
        .filter_map(|relay| {
            let mut nearest: Option<SimDuration> = None;
            for station in &map.stations {
                if let Ok(carry) = cache.travel_time(PathCacheKey {
                    from: relay.htm_access,
                    from_orientation: Orientation::East,
                    to: station.delivery_cell(),
                    carrying_cargo: true,
                }) {
                    nearest = Some(nearest.map_or(carry, |n| n.min(carry)));
                }
            }
            nearest.map(|n| n.as_ms())
        })
        .collect();

    if per_relay_ms.is_empty() {
        SimDuration::ZERO
    } else {
        SimDuration::ms(per_relay_ms.iter().sum::<u64>() / per_relay_ms.len() as u64)
    }
}
