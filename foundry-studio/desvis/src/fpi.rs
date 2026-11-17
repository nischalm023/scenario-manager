//! FPI -- Fetch Pressure Index computation using A* pathfinding.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use desgraph::{RelayId, StationId};
use dessim::pathfinding::{Constant, DriveModel};
use dessim::robots::reference_htm;
use dessim::{Cached, FHSippPathfinder, Orientation, PathCacheKey, SimDuration};
use rayon::prelude::*;

/// HTM travel speed in mm/s (1.5 m/s).
const HTM_SPEED_MM_S: u64 = 1500;

/// Compute FPI and HTM saturation metrics via parallel A* pathfinding.
pub(crate) fn run_fpi(map_path: &str, cli_pick_time: Option<f64>) {
    let loaded = crate::world_cli::load(map_path);

    let pick_time_secs = cli_pick_time.unwrap_or(loaded.settings.operator.time_per_pick_secs);
    let pick_time = SimDuration::ms((pick_time_secs * 1000.0) as u64);

    let map = loaded.world.map.clone();

    let pathfinder = FHSippPathfinder::new(&map, HTM_SPEED_MM_S, SimDuration::seconds(1), HashSet::new());
    // The FPI runs at constant speed, so the same constant drives search and convert.
    let htm = reference_htm();
    let drives: [Arc<dyn DriveModel>; 2] = [Arc::new(Constant::htm(&htm, false)), Arc::new(Constant::htm(&htm, true))];
    let path_cache = Cached::new(pathfinder, drives.clone(), drives);

    let pairs: Vec<_> = map
        .relays
        .iter()
        .flat_map(|relay| {
            map.stations.iter().map(move |station| (relay.id, relay.htm_access, station.id, station.delivery_cell()))
        })
        .collect();

    let n_relays = map.relays.len();
    let n_stations = map.stations.len();
    let n_pairs = pairs.len();

    let results: Vec<_> = pairs
        .par_iter()
        .map(|&(relay_id, relay_cell, station_id, station_cell)| {
            let f_est = path_cache.travel_time(PathCacheKey {
                from: relay_cell,
                from_orientation: Orientation::East,
                to: station_cell,
                carrying_cargo: true,
            });
            let f_return = path_cache.travel_time(PathCacheKey {
                from: station_cell,
                from_orientation: Orientation::East,
                to: relay_cell,
                carrying_cargo: false,
            });
            (relay_id, station_id, f_est, f_return)
        })
        .collect();

    let reachable: Vec<_> =
        results.iter().filter(|(_, _, f_est, f_return)| f_est.is_ok() && f_return.is_ok()).collect();

    let n_reachable = reachable.len();

    // Per-station stats (one-way fetch: relay -> station).
    let mut station_stats: BTreeMap<StationId, Vec<SimDuration>> = BTreeMap::new();
    for (_, station_id, f_est, _) in &reachable {
        station_stats.entry(*station_id).or_default().push(*f_est.as_ref().unwrap());
    }

    let map_name = Path::new(map_path).file_name().unwrap_or_default().to_string_lossy();
    println!("=== Fetch Pressure Index (FPI) ===");
    println!("Map: {}", map_name);
    println!("Relays: {}  Stations: {}  Pairs computed: {}  Reachable: {}", n_relays, n_stations, n_pairs, n_reachable);

    println!();
    println!("Per-station (relay -> station, one-way):");
    for (station_id, times) in &station_stats {
        let n = times.len() as u64;
        let sum = times.iter().fold(SimDuration::ZERO, |acc, &t| acc + t);
        let mean = SimDuration::ms(sum.as_ms() / n);
        let min = *times.iter().min().unwrap();
        let max = *times.iter().max().unwrap();
        let mean_ms = mean.as_ms() as f64;
        let variance = times.iter().map(|t| (t.as_ms() as f64 - mean_ms).powi(2)).sum::<f64>() / n as f64;
        let sigma = SimDuration::ms(variance.sqrt() as u64);
        println!("  Station {:>2}:  mean={}  min={}  max={}  \u{03c3}={}", station_id.0, mean, min, max, sigma);
    }

    // Per-relay: min over stations for F_est and F_cycle.
    let mut relay_min_f_est: BTreeMap<RelayId, SimDuration> = BTreeMap::new();
    let mut relay_min_f_cycle: BTreeMap<RelayId, SimDuration> = BTreeMap::new();
    for (relay_id, _, f_est, f_return) in &reachable {
        let est = *f_est.as_ref().unwrap();
        let ret = *f_return.as_ref().unwrap();
        let cycle = est + ret;
        let e = relay_min_f_est.entry(*relay_id).or_insert(SimDuration::MAX);
        if est < *e {
            *e = est;
        }
        let c = relay_min_f_cycle.entry(*relay_id).or_insert(SimDuration::MAX);
        if cycle < *c {
            *c = cycle;
        }
    }

    let fpi = if relay_min_f_est.is_empty() {
        SimDuration::ZERO
    } else {
        let sum = relay_min_f_est.values().fold(SimDuration::ZERO, |acc, &t| acc + t);
        SimDuration::ms(sum.as_ms() / relay_min_f_est.len() as u64)
    };

    let f_cycle_mean = if relay_min_f_cycle.is_empty() {
        SimDuration::ZERO
    } else {
        let sum = relay_min_f_cycle.values().fold(SimDuration::ZERO, |acc, &t| acc + t);
        SimDuration::ms(sum.as_ms() / relay_min_f_cycle.len() as u64)
    };
    let f_cycle_sigma = if relay_min_f_cycle.is_empty() {
        SimDuration::ZERO
    } else {
        let mean_ms = f_cycle_mean.as_ms() as f64;
        let var = relay_min_f_cycle.values().map(|t| (t.as_ms() as f64 - mean_ms).powi(2)).sum::<f64>()
            / relay_min_f_cycle.len() as f64;
        SimDuration::ms(var.sqrt() as u64)
    };

    let f_max = reachable.iter().map(|(_, _, f_est, _)| *f_est.as_ref().unwrap()).max().unwrap_or(SimDuration::ZERO);

    println!();
    println!("Overall:");
    println!("  FPI (mean one-way to nearest station):  {}", fpi);
    println!("  F_cycle_mean (mean round-trip):         {}  +/- {}", f_cycle_mean, f_cycle_sigma);
    println!("  F_max (worst-case one-way):             {}", f_max);

    // HTM saturation: ceil(F_cycle / P).
    let per_station = ((f_cycle_mean + pick_time - SimDuration::ms(1)) / pick_time).max(1) as usize;
    let total_htms_needed = n_stations * per_station;
    // The spec's settings arrive with the document fleet already defaulted
    // (world_spec::load), mirroring the launch paths.
    let fleet_htms = loaded.settings.num_htms;

    println!();
    println!("D30 Saturation (P={}):", pick_time);
    println!("  Per station: ceil({} / {}) = {} D30s", f_cycle_mean, pick_time, per_station);
    println!("  Total ({} stations): {} D30s", n_stations, total_htms_needed);
    if fleet_htms > 0 {
        let headroom = fleet_htms as f64 / total_htms_needed as f64;
        println!("  Fleet available: {} D30s", fleet_htms);
        println!("  Headroom: {:.1}x", headroom);
    }
}
