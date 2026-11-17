//! A synthetic run in the vocabulary's own records, with its truth written out
//! by hand beside it.
//!
//! It is shared by the tests that read a recording and the ones that serve one
//! over a socket, so both are talking about the same nine slices. Its world
//! document is a real fixture, because compiling a floor is the one thing here
//! that no hand-written stand-in could stand in for.

#![allow(dead_code)]

use std::fs::File;
use std::path::{Path, PathBuf};

use desgraph::types::{CellId, StationId};
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use serde_json::Value;
use worldview::frames::end::End;
use worldview::frames::heat::{HeatDelta, HeatKeyframe, HeatLayer, HeatLayerDelta};
use worldview::frames::motion::MotionDelta;
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RobotClass, RobotRecord, StationChange, StationState, Step,
    StepKind, ToteContents, ToteLocation, ToteRecord,
};
use worldview::frames::scalars::{
    AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit, ScalarsDelta, ScalarsKeyframe,
};
use worldview::frames::scenario::RobotEntry;
use worldview::frames::tables::TablesDelta;
use worldview::{Declare, Delta, Document, Keyframe, Opaque, Scenario, WriteFrames, Writer};

/// Ten-second slices, a keyframe every third, nine of them.
pub const SLICE_MS: u64 = 10_000;
pub const CADENCE: u32 = 3;
pub const SLICES: u64 = 9;

/// The metric whose slice values are one apiece, so its running total at the
/// end of slice k is k + 1 and nothing has to be derived to know it.
pub const PICKS: &str = "picks";
/// A gauge: each slice's value replaces the reading before it.
pub const DEPTH: &str = "depth";
/// Declared four slices in, which is what an attachment past that instant has
/// to find folded into its scenario block.
pub const LATE: &str = "late";
pub const DECLARED_AT: u64 = 4 * SLICE_MS;

pub fn metric(name: &str, aggregation: AggregationClass) -> MetricDescriptor {
    MetricDescriptor {
        name: name.to_owned(),
        label: name.to_owned(),
        description: String::new(),
        semantics: MetricSemantics { unit: MetricUnit::Count, denominator_description: None, aggregation },
    }
}

pub fn world_document() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_easyloop.json");
    std::fs::read(&path).unwrap_or_else(|e| panic!("the fixture world document at {}: {e}", path.display()))
}

/// The primary frame the fixture world document declares. A recording names
/// the frame its positions are measured in, and a reader holds the name and
/// the document to each other, so the two are stated from one place here.
fn world_primary_frame() -> String {
    let document: Value = serde_json::from_slice(&world_document()).expect("the fixture is JSON");
    document["primary_frame"].as_str().expect("the fixture declares a primary frame").to_owned()
}

/// Write a whole run: three keyframes, nine deltas, one declaration and an end
/// block, over a real world document.
pub fn write_run(into: &Path) {
    let scenario = Scenario {
        frame: world_primary_frame(),
        run: "01TEST".to_owned(),
        attempt: 1,
        run_name: "synthetic".to_owned(),
        slice_width: SimDuration::ms(SLICE_MS),
        keyframe_cadence_slices: CADENCE,
        world: Document(world_document()),
        // A data file and a producer document, so the filter has something to
        // take out and the test can see that it did.
        data: Some(vec![("items.csv".to_owned(), Document(vec![b'a'; 4_096]))]),
        producer: Some(Opaque::of(&vec![7u64; 512], "producer").expect("encodes")),
        cells: (0..8).map(|cell| (CellId(cell), format!("cell-{cell}"))).collect(),
        stations: vec![(StationId(0), "station-0".to_owned())],
        robots: (0..2)
            .map(|robot| RobotEntry {
                robot: RobotId(robot),
                name: format!("robot-{robot}"),
                class: RobotClass::Htm,
                class_entity: None,
            })
            .collect(),
        metrics: vec![metric(PICKS, AggregationClass::Additive), metric(DEPTH, AggregationClass::SnapshotGauge)],
        ..Scenario::default()
    };

    let file = File::create(into).expect("a file to write into");
    let mut writer = Writer::new(file);
    writer.append_scenario_block(&scenario).expect("the scenario block opens the file");
    for slice in 0..SLICES {
        let at = SimTime::ms(slice * SLICE_MS);
        if slice * SLICE_MS == DECLARED_AT {
            writer.append_declare(at, &Declare::Metric(metric(LATE, AggregationClass::Additive))).expect("declared");
        }
        if slice.is_multiple_of(u64::from(CADENCE)) {
            writer.append_keyframe(at, &keyframe_at(slice)).expect("a keyframe");
        }
        writer.append_delta(at, &delta_at(slice)).expect("a delta");
    }
    let ending = End {
        stop_reason: "reached the horizon".to_owned(),
        frames_written: writer.frames_written(),
        ..End::default()
    };
    writer.finish_with(SimTime::ms(SLICES * SLICE_MS), &ending).expect("the file closes");
}

/// Where everything stands at the start of slice `slice`: robot 0 has walked
/// one cell per slice, robot 1 has not moved, and the totals are the ones the
/// deltas below add up to.
pub fn keyframe_at(slice: u64) -> Keyframe {
    let robot = |robot: u32, cell: u32| RobotRecord {
        robot: RobotId(robot),
        class: RobotClass::Htm,
        at: Position::new(cell as i32 * 1_000, 0, 0),
        heading_mrad: Some(0),
        carrying: None,
        cell: CellId(cell),
        toward: None,
    };
    Keyframe {
        playback: Some(PlaybackKeyframe {
            robots: vec![robot(0, (slice % 8) as u32), robot(1, 7)],
            totes: vec![ToteRecord {
                tote: ToteId(0),
                tote_type: 0,
                designation: 0,
                location: ToteLocation::InRack { cell: CellId(3), level: 0 },
                contents_units: slice,
            }],
            reserved_relay_cells: Vec::new(),
            stations: vec![StationState { station: StationId(0), pipeline_depth: slice as u32 }],
            transits_in_flight: Vec::new(),
        }),
        scalars: Some(ScalarsKeyframe {
            values: match slice {
                0 => vec![Some(0.0), None],
                _ => vec![Some(slice as f64), Some((slice - 1) as f64)],
            },
        }),
        heat: Some(HeatKeyframe {
            layers: vec![HeatLayer { name: "visits".to_owned(), counts: vec![(CellId(1), slice)] }],
        }),
        robot_state: None,
    }
}

/// One slice: robot 0 steps to the next cell, the tote gains a unit, the
/// station's pipeline deepens by one, one pick lands, the gauge reads the slice
/// number, and one cell is visited once. Every one of those is what the next
/// keyframe states outright, so a rebuild from the frames and a rebuild from a
/// keyframe have to agree.
pub fn delta_at(slice: u64) -> Delta {
    Delta {
        playback: Some(PlaybackDelta {
            movers: vec![MoverSteps {
                robot: RobotId(0),
                start_heading_mrad: Some(0),
                steps: vec![Step {
                    at: Position::new((((slice + 1) % 8) as i32) * 1_000, 0, 0),
                    dt: SimDuration::ms(SLICE_MS),
                    kind: StepKind::Move,
                    cell: CellId(((slice + 1) % 8) as u32),
                }],
                cell: CellId(((slice + 1) % 8) as u32),
            }],
            tote_contents: vec![ToteContents { offset: SimDuration::ZERO, tote: ToteId(0), units: slice + 1 }],
            station_changes: vec![StationChange {
                offset: SimDuration::ZERO,
                station: StationId(0),
                pipeline_depth: (slice + 1) as u32,
            }],
            ..PlaybackDelta::default()
        }),
        scalars: Some(ScalarsDelta { values: vec![Some(1.0), Some(slice as f64)] }),
        heat: Some(HeatDelta {
            layers: vec![HeatLayerDelta { name: "visits".to_owned(), increments: vec![(CellId(1), 1)] }],
        }),
        tables: Some(TablesDelta::default()),
        motion: Some(MotionDelta::default()),
        robot_state: None,
    }
}

/// Write the run into `temp` under the key a server would hold it at, and hand
/// back the path. `damaged` cuts the tail off, which is what a reader meets
/// when a recording did not finish arriving.
pub fn written(temp: &Path, damaged: bool) -> PathBuf {
    let path = temp.join("synthetic.worldview");
    write_run(&path);
    if damaged {
        let whole = std::fs::read(&path).expect("the recording");
        std::fs::write(&path, &whole[..whole.len() / 3]).expect("truncated");
    }
    path
}
