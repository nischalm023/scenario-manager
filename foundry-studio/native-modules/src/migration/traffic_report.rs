//! Fold a recording into the same elapsed-motion cell aggregates desvis draws
//! from. This is a report, not a new metric definition and not a claim that
//! every stationary wait was caused by congestion.
//!
//! It holds the whole run model: the recording is read into one, then its
//! slices are folded in order. That is hundreds of megabytes for a long run —
//! the earlier reader walked one slice at a time, and this one does not.

use std::error::Error;
use std::fs::File;
use std::path::Path;

use desgraph::RobotType;
use dessim::metrics::profile::SliceRecord as Slice;
use dessim::metrics::worldview::read_worldview;
use dessim::observer::time_slice::BotStepKind;
use dessim::observer::traffic::CellTraffic;
use dessim::world_spec;
use native_modules as _;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};

#[path = "traffic_report/series.rs"]
mod series;
use series::{Series, Summary};

#[derive(Default)]
struct Aggregate {
    cells: FxHashMap<u32, CellTraffic>,
    implicit_idle_ms: FxHashMap<u32, f64>,
    width_ms: u64,
    until_ms: u64,
    actual_end_ms: u64,
    playback_moving_ms: f64,
    missing_motion_slices: u64,
    series: Option<Series>,
}

impl Aggregate {
    fn add(&mut self, record: &Slice) {
        let until = self.until_ms.min(record.start_ms + self.width_ms);
        if until <= record.start_ms {
            return;
        }
        if let Some(series) = &mut self.series {
            series.add(record, until);
        }
        let mut bots: Vec<_> = record.bot_locations.iter().collect();
        bots.sort_unstable_by_key(|&(id, _)| *id);
        for (_, loc) in bots {
            if loc.robot_type != RobotType::Htm {
                continue;
            }
            if let Some(motion) = &loc.motion {
                for span in motion {
                    span.accumulate(record.start_ms, until, &mut self.cells);
                }
            } else {
                self.missing_motion_slices += 1;
            }
            let mut time = record.start_ms;
            let mut cell = loc.start_cell;
            for step in &loc.steps {
                let end = time.saturating_add(step.delta_ms);
                let ms = end.min(until).saturating_sub(time) as f64;
                if ms > 0.0 {
                    match step.kind {
                        BotStepKind::Wait | BotStepKind::Rotate => {
                            let row = self.cells.entry(step.cell_id).or_default();
                            row.observed_ms += ms;
                            if step.kind == BotStepKind::Wait {
                                row.wait_ms += ms;
                            } else {
                                row.rotation_ms += ms;
                            }
                        }
                        BotStepKind::Move | BotStepKind::Reverse => {
                            self.playback_moving_ms += ms;
                        }
                    }
                }
                time = end;
                cell = step.cell_id;
                if time >= until {
                    break;
                }
            }
            if time < until {
                let ms = (until - time) as f64;
                let row = self.cells.entry(cell).or_default();
                row.observed_ms += ms;
                row.wait_ms += ms;
                *self.implicit_idle_ms.entry(cell).or_default() += ms;
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 && !(args.len() == 7 && args[5] == "--series-minutes") {
        return Err("usage: traffic_report profile spec until-hours output.json [--series-minutes minutes]".into());
    }
    let mut aggregate = Aggregate { until_ms: (args[3].parse::<f64>()? * 3_600_000.0) as u64, ..Default::default() };
    if args.len() == 7 {
        aggregate.series = Some(Series::new(args[6].parse::<u64>()? * 60_000)?);
    }
    let recording = read_worldview(Path::new(&args[1]))?;
    if recording.truncated {
        return Err("Incomplete recording".into());
    }
    let profile = recording.profile;
    aggregate.width_ms = profile.metadata.slice_width_ms;
    aggregate.actual_end_ms = (profile.summary.sim_time_secs * 1000.0).round() as u64;
    if aggregate.actual_end_ms == 0 {
        return Err("Incomplete recording".into());
    }
    // The cutoff is the earlier of what was asked for and what the run
    // reached. Without this a report over more hours than the run has divides
    // every rate by a window the run never ran, and says nothing about it.
    aggregate.until_ms = aggregate.until_ms.min(aggregate.actual_end_ms);
    // The series reads a handful of the summary's own columns; it takes them
    // straight off the summary rather than through a copy of the whole thing.
    if let Some(series) = &mut aggregate.series {
        series.summary = Summary::of(&profile.summary);
    }
    for slice in &profile.time_slices {
        aggregate.add(slice);
    }
    let loaded = world_spec::load(Path::new(&args[2]))?;
    let world = loaded.world;
    let map = &world.map;
    let exclusions = map.navigation_wait_exclusions();
    let mut rows: Vec<Value> = Vec::new();
    let mut total_distance = 0.0;
    let mut total_moving = 0.0;
    let mut total_wait = 0.0;
    let mut eligible_wait = 0.0;
    for cell in map.graph.cells() {
        let Some(t) = aggregate.cells.get(&cell.id.0) else {
            continue;
        };
        let pos = cell.world_pos;
        let entity = world.index.cell_entities.get(&cell.id);
        let name = entity.and_then(|id| world.world.entities.get(id)).and_then(|e| e.name.clone());
        total_distance += t.distance_mm;
        total_moving += t.moving_ms;
        total_wait += t.wait_ms;
        if !exclusions.contains_key(&cell.id.0) {
            eligible_wait += t.wait_ms;
        }
        rows.push(json!({"cell":cell.id.0,"entity":entity.map(ToString::to_string),"name":name,
            "kind":cell.kind,"zone":cell.zone.0,"x_mm":pos.map(|p|p.x_mm),"y_mm":pos.map(|p|p.y_mm),
            "size_mm":world.cell_sizes_mm.get(cell.id.0 as usize),"exclusion":exclusions.get(&cell.id.0),
            "wait_s":t.wait_ms/1000.0,"implicit_hold_s":aggregate.implicit_idle_ms.get(&cell.id.0).copied().unwrap_or(0.0)/1000.0,
            "rotate_s":t.rotation_ms/1000.0,"moving_s":t.moving_ms/1000.0,"distance_m":t.distance_mm/1000.0,
            "mean_m_s":t.moving_speed_m_s(),"speed_seconds":t.speed_ms.map(|ms|ms/1000.0)}));
    }
    rows.sort_by_key(|row| row["cell"].as_u64());
    let difference = total_moving - aggregate.playback_moving_ms;
    let result = json!({"profile":args[1],"until_ms":aggregate.until_ms,"recorded_end_ms":aggregate.actual_end_ms,
        "missing_motion_slices":aggregate.missing_motion_slices,"canonical_moving_ms":total_moving,
        "playback_moving_ms":aggregate.playback_moving_ms,"motion_difference_ms":difference,
        "distance_m":total_distance/1000.0,"all_wait_s":total_wait/1000.0,"eligible_wait_s":eligible_wait/1000.0,
        "cells":rows,"floors":world.floors.iter().map(|f|json!({"rect_mm":f.rect_mm,"kind":f.kind})).collect::<Vec<_>>()});
    let result = if let Some(series) = aggregate.series {
        series.finish(result, map, &exclusions, aggregate.until_ms)?
    } else {
        result
    };
    serde_json::to_writer(File::create(&args[4])?, &result)?;
    if difference.abs() > 0.1 || aggregate.missing_motion_slices != 0 {
        return Err("Traffic validation failed".into());
    }
    Ok(())
}
