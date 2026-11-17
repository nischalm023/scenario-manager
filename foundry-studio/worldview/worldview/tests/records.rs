//! Every record's bytes, pinned: a positional array of a fixed arity, with no
//! field names anywhere.
//!
//! These are the assertions a change to the vocabulary has to walk through.
//! The layouts are normative — a second implementation codes from them — so
//! the arities are written out here as the numbers they are rather than
//! derived from the code that produces them, and the one place they are also
//! written in prose is `docs/designs/worldview-format.md`.

mod support;

use desgraph::types::{CellId, StationId};
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use rmp_serde::Deserializer;
use serde::{Deserialize, Serialize};
use worldview::frames::declare::Declare;
use worldview::frames::heat::{HeatDelta, HeatKeyframe, HeatLayer, HeatLayerDelta};
use worldview::frames::motion::{MotionDelta, MotionProfile, MotionSpan, RobotMotion};
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RelayTransition, RobotClass, RobotRecord, StationChange,
    StationState, Step, StepKind, ToteContents, ToteCreation, ToteLocation, ToteRecord, ToteTransit, ToteTransition,
};
use worldview::frames::scalars::{
    AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit, ScalarsDelta, ScalarsKeyframe,
};
use worldview::frames::scenario::{Document, HAND_BUILT_FRAME, RobotEntry, Scenario};
use worldview::frames::state::state_at;
use worldview::frames::tables::{
    ColumnEncoding, ColumnValueType, TableColumn, TableDescriptor, TableId, TableRows, TablesDelta,
};
use worldview::frames::{Delta, HEAT, Keyframe, MOTION, PLAYBACK, SCALARS, TABLES, WriteFrames};
use worldview::{End, Error, Ms, Opaque, Payload, Reader, Sections, Tag, VERSION, Writer};

use support::vocabulary::{RunShape, SLICE_WIDTH_MS, build, metric, picks_table, whole_delta, whole_keyframe};

/// A keyframe a file can hold, carrying the scalars a test is about.
fn scalars_keyframe(values: Vec<Option<f64>>) -> Keyframe {
    Keyframe { scalars: Some(ScalarsKeyframe { values }), ..whole_keyframe(PlaybackKeyframe::default()) }
}

/// A delta a file can hold, carrying the scalars a test is about.
fn scalars_delta(values: Vec<Option<f64>>) -> Delta {
    Delta { scalars: Some(ScalarsDelta { values }), ..whole_delta(PlaybackDelta::default()) }
}

/// How many positions a MessagePack array holds, refusing anything that is
/// not an array — a map, which is what a record with field names would be.
fn positions(bytes: &[u8]) -> usize {
    match bytes[0] {
        marker @ 0x90..=0x9f => usize::from(marker & 0x0f),
        0xdc => usize::from(u16::from_be_bytes([bytes[1], bytes[2]])),
        marker => panic!("a record is a positional array and this one begins {marker:#04x}"),
    }
}

fn encoded<T: Serialize>(record: &T) -> Vec<u8> {
    rmp_serde::to_vec(record).expect("encodes")
}

fn location() -> ToteLocation {
    ToteLocation::InRack { cell: CellId(4), level: 5 }
}

fn robot_entry() -> RobotEntry {
    RobotEntry { robot: RobotId(1), name: "htm-1".to_owned(), class: RobotClass::Htm, class_entity: None }
}

fn pinned_metric() -> MetricDescriptor {
    MetricDescriptor {
        name: "m".to_owned(),
        label: "M".to_owned(),
        description: String::new(),
        semantics: MetricSemantics {
            unit: MetricUnit::Count,
            denominator_description: None,
            aggregation: AggregationClass::SnapshotGauge,
        },
    }
}

fn pinned_table() -> TableDescriptor {
    TableDescriptor {
        id: TableId(1),
        name: "t".to_owned(),
        columns: vec![TableColumn {
            name: "at".to_owned(),
            value_type: ColumnValueType::SimTime,
            encoding: ColumnEncoding::Positional,
        }],
        instant_column: Some(0),
    }
}

fn rows() -> TableRows {
    TableRows { table: TableId(1), row_count: 2, rows: vec![0x91, 3] }
}

fn robot_record() -> RobotRecord {
    RobotRecord {
        robot: RobotId(1),
        class: RobotClass::Vtm,
        at: Position::new(3, 4, 5),
        heading_mrad: Some(6),
        carrying: Some(ToteId(7)),
        cell: CellId(8),
        toward: None,
    }
}

fn tote_record() -> ToteRecord {
    ToteRecord { tote: ToteId(1), tote_type: 2, designation: 3, location: location(), contents_units: 6 }
}

fn creation() -> ToteCreation {
    ToteCreation { offset: SimDuration::ms(1), tote: ToteId(2), tote_type: 3, designation: 4, location: location() }
}

fn transition() -> ToteTransition {
    ToteTransition { offset: SimDuration::ms(1), tote: ToteId(2), location: location() }
}

fn heat_keyframe() -> HeatKeyframe {
    HeatKeyframe { layers: vec![HeatLayer { name: "nav".to_owned(), counts: vec![(CellId(1), 2)] }] }
}

fn heat_delta() -> HeatDelta {
    HeatDelta { layers: vec![HeatLayerDelta { name: "nav".to_owned(), increments: vec![(CellId(1), 2)] }] }
}

fn motion() -> MotionDelta {
    MotionDelta(vec![RobotMotion { robot: RobotId(1), spans: vec![span()] }])
}

fn end_block() -> End {
    End {
        stop_reason: "s".to_owned(),
        stop_detail: None,
        result: Opaque(vec![9]),
        summary: Opaque(vec![1, 2]),
        cumulative_scalars: vec![Some(1.0)],
        tables: Vec::new(),
        frames_written: 7,
    }
}

fn transit() -> ToteTransit {
    ToteTransit {
        tote: ToteId(1),
        from_cell: CellId(2),
        to_cell: CellId(3),
        start: SimTime::ms(10),
        end: SimTime::ms(20),
    }
}

fn mover() -> MoverSteps {
    MoverSteps {
        robot: RobotId(1),
        start_heading_mrad: Some(1_571),
        steps: vec![Step {
            at: Position::new(4, 5, 6),
            dt: SimDuration::ms(900),
            kind: StepKind::Move,
            cell: CellId(7),
        }],
        cell: CellId(8),
    }
}

fn span() -> MotionSpan {
    MotionSpan {
        source: CellId(1),
        destination: CellId(2),
        start: SimTime::ms(0),
        end: SimTime::ms(1_000),
        distance_mm: 500.0,
        profile: Some(MotionProfile {
            entry_velocity_mm_s: 0.0,
            exit_velocity_mm_s: 0.0,
            peak_velocity_mm_s: 1_000.0,
            accel_time_s: 0.5,
            cruise_time_s: 0.0,
            decel_time_s: 0.5,
            accel_distance_mm: 250.0,
            cruise_distance_mm: 0.0,
            decel_distance_mm: 250.0,
            total_time_s: 1.0,
            total_distance_mm: 500.0,
            acceleration_mm_s2: 2_000.0,
            deceleration_mm_s2: 2_000.0,
        }),
    }
}

/// Every record, with the bytes the format says it becomes.
///
/// Each field of a record gets a value of its own, so two fields swapped
/// changes the bytes, and the literals are written from the layout rather
/// than from what the code produced. Where a record ends in floating point or
/// in a run's own strings, the literal covers everything up to that.
fn pinned() -> Vec<(&'static str, Vec<u8>, Vec<u8>, usize)> {
    let one = |value: f64| vec![0xcb].into_iter().chain(value.to_be_bytes()).collect::<Vec<u8>>();
    vec![
        // The block opens with a twenty-four element array and the container
        // version; what follows is the run's own strings and documents.
        ("scenario block", encoded(&Scenario::default()), vec![0xdc, 0x00, 24, 1], 24),
        ("robot entry", encoded(&robot_entry()), vec![0x94, 1, 0xa5, b'h', b't', b'm', b'-', b'1', 1, 0xc0], 4),
        ("declare", encoded(&Declare::Metric(pinned_metric())), vec![0x92, 1, 0x94, 0xa1, b'm'], 2),
        ("metric descriptor", encoded(&pinned_metric()), vec![0x94, 0xa1, b'm', 0xa1, b'M', 0xa0, 0x93, 1, 0xc0, 4], 4),
        ("metric semantics", encoded(&pinned_metric().semantics), vec![0x93, 1, 0xc0, 4], 3),
        (
            "table descriptor",
            encoded(&pinned_table()),
            vec![0x94, 1, 0xa1, b't', 0x91, 0x93, 0xa2, b'a', b't', 6, 1, 0],
            4,
        ),
        ("table column", encoded(&pinned_table().columns[0]), vec![0x93, 0xa2, b'a', b't', 6, 1], 3),
        ("table rows", encoded(&rows()), vec![0x93, 1, 2, 0xc4, 2, 0x91, 3], 3),
        ("playback keyframe", encoded(&PlaybackKeyframe::default()), vec![0x95, 0x90, 0x90, 0x90, 0x90, 0x90], 5),
        ("robot record", encoded(&robot_record()), vec![0x97, 1, 2, 0x93, 3, 4, 5, 6, 7, 8, 0xc0], 7),
        (
            "robot record, heading for a cell",
            encoded(&RobotRecord { toward: Some(CellId(9)), ..robot_record() }),
            vec![0x97, 1, 2, 0x93, 3, 4, 5, 6, 7, 8, 9],
            7,
        ),
        ("position", encoded(&Position::new(2, 3, 4)), vec![0x93, 2, 3, 4], 3),
        ("position, west and below the origin", encoded(&Position::new(-1, 3, -2)), vec![0x93, 0xff, 3, 0xfe], 3),
        ("tote record", encoded(&tote_record()), vec![0x95, 1, 2, 3, 0x93, 2, 4, 5, 6], 5),
        ("tote location", encoded(&location()), vec![0x93, 2, 4, 5], 3),
        ("station state", encoded(&StationState { station: StationId(1), pipeline_depth: 2 }), vec![0x92, 1, 2], 2),
        ("tote transit", encoded(&transit()), vec![0x95, 1, 2, 3, 10, 20], 5),
        (
            "playback delta",
            encoded(&PlaybackDelta::default()),
            vec![0x99, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0xc4, 0],
            9,
        ),
        (
            "mover steps",
            encoded(&mover()),
            vec![0x94, 1, 0xcd, 0x06, 0x23, 0x91, 0x94, 0x93, 4, 5, 6, 0xcd, 0x03, 0x84, 1, 7, 8],
            4,
        ),
        ("heading", encoded(&(RobotId(1), Some(1_571))), vec![0x92, 1, 0xcd, 0x06, 0x23], 2),
        ("step", encoded(&mover().steps[0]), vec![0x94, 0x93, 4, 5, 6, 0xcd, 0x03, 0x84, 1, 7], 4),
        ("tote creation", encoded(&creation()), vec![0x95, 1, 2, 3, 4, 0x93, 2, 4, 5], 5),
        ("tote transition", encoded(&transition()), vec![0x93, 1, 2, 0x93, 2, 4, 5], 3),
        (
            "tote contents",
            encoded(&ToteContents { offset: SimDuration::ms(1), tote: ToteId(2), units: 3 }),
            vec![0x93, 1, 2, 3],
            3,
        ),
        (
            "relay transition",
            encoded(&RelayTransition { cell: CellId(1), offset: SimDuration::ms(2), reserved: true }),
            vec![0x93, 1, 2, 0xc3],
            3,
        ),
        (
            "station change",
            encoded(&StationChange { offset: SimDuration::ms(1), station: StationId(2), pipeline_depth: 3 }),
            vec![0x93, 1, 2, 3],
            3,
        ),
        (
            "scalars keyframe",
            encoded(&ScalarsKeyframe { values: vec![Some(1.0), None] }),
            [vec![0x91, 0x92], one(1.0), vec![0xc0]].concat(),
            1,
        ),
        ("scalars delta", encoded(&ScalarsDelta { values: vec![None] }), vec![0x91, 0x91, 0xc0], 1),
        (
            "heat keyframe",
            encoded(&heat_keyframe()),
            vec![0x91, 0x91, 0x92, 0xa3, b'n', b'a', b'v', 0x91, 0x92, 1, 2],
            1,
        ),
        ("heat layer", encoded(&heat_keyframe().layers[0]), vec![0x92, 0xa3, b'n', b'a', b'v', 0x91, 0x92, 1, 2], 2),
        ("heat delta", encoded(&heat_delta()), vec![0x91, 0x91, 0x92, 0xa3, b'n', b'a', b'v', 0x91, 0x92, 1, 2], 1),
        ("heat layer delta", encoded(&heat_delta().layers[0]), vec![0x92, 0xa3, b'n', b'a', b'v', 0x91, 0x92, 1, 2], 2),
        ("tables delta", encoded(&TablesDelta(vec![rows()])), vec![0x91, 0x93, 1, 2, 0xc4, 2, 0x91, 3], 1),
        ("motion delta", encoded(&motion()), vec![0x91, 0x92, 1, 0x91, 0x96, 1, 2, 0, 0xcd, 0x03, 0xe8, 0xcb], 1),
        ("robot motion", encoded(&motion().0[0]), vec![0x92, 1, 0x91, 0x96, 1, 2, 0, 0xcd, 0x03, 0xe8, 0xcb], 2),
        ("motion span", encoded(&span()), vec![0x96, 1, 2, 0, 0xcd, 0x03, 0xe8, 0xcb], 6),
        ("motion profile", encoded(&span().profile.expect("a profile")), vec![0x9d, 0xcb], 13),
        (
            "end block",
            encoded(&end_block()),
            [vec![0x97, 0xa1, b's', 0xc0, 0xc4, 1, 9, 0xc4, 2, 1, 2, 0x91], one(1.0), vec![0x90, 7]].concat(),
            7,
        ),
    ]
}

#[test]
fn every_record_is_the_positional_array_of_bytes_the_format_gives() {
    for (record, bytes, expected, arity) in pinned() {
        assert_eq!(positions(&bytes), arity, "{record}: how many positions");
        assert!(
            bytes.len() >= expected.len(),
            "{record}: {} bytes, short of the layout's {}",
            bytes.len(),
            expected.len()
        );
        assert_eq!(&bytes[..expected.len()], expected, "{record}: the bytes the layout gives");
    }
}

#[test]
fn an_id_and_a_quantity_travel_as_the_integers_they_wrap() {
    assert_eq!(encoded(&CellId(7)), vec![7], "a cell id is its integer, with nothing around it");
    assert_eq!(encoded(&SimDuration::ms(300)), vec![0xcd, 0x01, 0x2c], "a duration is its milliseconds");
    assert_eq!(encoded(&SimTime::ms(300)), vec![0xcd, 0x01, 0x2c]);
    assert_eq!(encoded(&TableId(2)), vec![2]);
    assert_eq!(encoded(&StepKind::Rotate), vec![4], "an enum inside a record is one byte");
    assert_eq!(encoded(&Document(vec![1, 2])), vec![0xc4, 0x02, 1, 2], "a document is a byte string");
}

/// The three frames that bracket a run go in and come back the same, through
/// the container that carries them.
#[test]
fn the_scenario_block_a_declare_and_the_end_block_round_trip_through_a_file() {
    let scenario = Scenario {
        run: "round-trip".to_owned(),
        slice_width: SimDuration::ms(SLICE_WIDTH_MS),
        keyframe_cadence_slices: 30,
        world: Document(b"{\"cells\":[]}".to_vec()),
        frame: HAND_BUILT_FRAME.to_owned(),
        data: Some(vec![("racks.csv".to_owned(), Document(vec![1, 2, 3]))]),
        sim_config: Opaque::of(&42u32, "sim config").expect("encodes"),
        producer: Some(Opaque::of(&"grid bounds", "producer state").expect("encodes")),
        metrics: vec![metric(0)],
        tables: vec![picks_table()],
        ..Scenario::default()
    };
    let declared = Declare::Table(TableDescriptor {
        id: TableId(2),
        name: "robots.htm_legs".to_owned(),
        columns: Vec::new(),
        instant_column: Some(0),
    });
    let ending = End {
        stop_reason: "reached the horizon".to_owned(),
        stop_detail: Some("robot 4".to_owned()),
        summary: Opaque::of(&vec![1u8, 2], "summary").expect("encodes"),
        cumulative_scalars: vec![Some(7.0)],
        tables: vec![TableRows { table: TableId(9), row_count: 1, rows: vec![0x91, 0x02] }],
        frames_written: 4,
        ..End::default()
    };

    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&scenario).expect("opens");
        writer.append_declare(SimTime::ZERO, &declared).expect("declares");
        writer.append_keyframe(SimTime::ZERO, &whole_keyframe(PlaybackKeyframe::default())).expect("a keyframe");
        writer.append_delta(SimTime::ZERO, &whole_delta(PlaybackDelta::default())).expect("a delta");
        writer.finish_with(SimTime::ms(SLICE_WIDTH_MS), &ending).expect("closes");
    }

    let mut reader = Reader::from_bytes(&bytes).expect("reads");
    assert_eq!(Scenario::read_from(&mut reader).expect("the scenario block reads"), scenario);
    let records = reader.index().records().to_vec();
    let declare_record = records.iter().find(|record| record.tag == Tag::Declare).expect("a declare");
    assert_eq!(reader.payload_at::<Declare>(declare_record).expect("reads"), declared);
    let end_record = records.iter().find(|record| record.tag == Tag::End).expect("an end frame");
    assert_eq!(reader.payload_at::<End>(end_record).expect("reads"), ending);
}

/// A server sending a browser less than the file holds takes sections out of
/// a frame and lays the rest out again. What it sends still decodes, with
/// nothing in the place of what it dropped.
#[test]
fn a_delta_with_sections_taken_out_of_it_decodes_with_none_in_their_place() {
    let delta = Delta {
        playback: Some(PlaybackDelta { movers: vec![mover()], ..PlaybackDelta::default() }),
        scalars: Some(ScalarsDelta { values: vec![Some(2.0)] }),
        heat: Some(HeatDelta::default()),
        tables: Some(TablesDelta(vec![TableRows::new(TableId(1))])),
        motion: Some(MotionDelta(vec![RobotMotion { robot: RobotId(1), spans: vec![span()] }])),
        robot_state: None,
    };
    let whole = delta.encode().expect("encodes");
    assert_eq!(whole.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![PLAYBACK, SCALARS, HEAT, TABLES, MOTION]);

    let kept: Vec<(u8, &[u8])> = whole
        .iter()
        .filter(|(id, _)| *id == PLAYBACK || *id == MOTION)
        .map(|(id, body)| (*id, body.as_slice()))
        .collect();
    let sections = Sections::decode(Sections::encode(&kept), 0).expect("the filtered frame lays out");
    let filtered = Delta::decode(&sections, 0).expect("decodes");
    assert_eq!(filtered.playback, delta.playback, "what was kept is byte for byte what it was");
    assert_eq!(filtered.motion, delta.motion);
    assert_eq!((filtered.scalars, filtered.heat, filtered.tables), (None, None, None));
    assert_eq!(filtered.robot_state, None);
}

/// A run declares a metric when the thing it measures first exists. The slot
/// appears at the end of the array, the frames written before it stay
/// shorter, and a reader that seeks past the declaration reads both.
#[test]
fn a_metric_declared_mid_run_extends_the_slots_and_the_shorter_frames_before_it_still_read() {
    let scenario = Scenario {
        slice_width: SimDuration::ms(SLICE_WIDTH_MS),
        keyframe_cadence_slices: 1,
        metrics: vec![metric(0)],
        frame: HAND_BUILT_FRAME.to_owned(),
        ..Scenario::default()
    };
    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&scenario).expect("opens");
        writer.append_keyframe(SimTime::ZERO, &scalars_keyframe(vec![Some(1.0)])).expect("a keyframe of one slot");
        writer.append_delta(SimTime::ZERO, &scalars_delta(vec![Some(2.0)])).expect("a delta of one slot");

        let second = SimTime::ms(SLICE_WIDTH_MS);
        writer.append_declare(second, &Declare::Metric(metric(2))).expect("a metric appears");
        writer.append_keyframe(second, &scalars_keyframe(vec![Some(3.0), Some(9.0)])).expect("a keyframe of two slots");
        writer.append_delta(second, &scalars_delta(vec![Some(4.0), Some(5.0)])).expect("a delta of two slots");
        writer
            .append_delta(SimTime::ms(2 * SLICE_WIDTH_MS), &scalars_delta(vec![None, None]))
            .expect("the slice after them");
        writer.finish_with(SimTime::ms(3 * SLICE_WIDTH_MS), &End::default()).expect("closes");
    }

    let mut reader = Reader::from_bytes(&bytes).expect("reads");
    let (early, _) = state_at(&mut reader, SimTime::ms(1)).expect("seeks into the first slice");
    assert_eq!(early.scalars.values, vec![Some(1.0)], "the frames before the declaration hold one slot");
    let (late, _) = state_at(&mut reader, SimTime::ms(2 * SLICE_WIDTH_MS)).expect("seeks past the declaration");
    // The second metric is a gauge, so its slot is the newest reading and
    // not the sum of the two the run stated.
    assert_eq!(late.scalars.values, vec![Some(7.0), Some(5.0)]);
}

/// A run states some metrics only while the thing they measure exists, and a
/// reader has to rebuild exactly the ones that were stated.
#[test]
fn a_slot_nothing_has_stated_stays_absent_through_a_keyframe_and_a_delta() {
    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer
            .append_scenario_block(&Scenario {
                metrics: vec![metric(0), metric(1)],
                slice_width: SimDuration::ms(SLICE_WIDTH_MS),
                frame: HAND_BUILT_FRAME.to_owned(),
                ..Scenario::default()
            })
            .expect("opens");
        writer
            .append_keyframe(SimTime::ZERO, &scalars_keyframe(vec![Some(1.0), None]))
            .expect("a keyframe with one slot unstated");
        writer.append_delta(SimTime::ZERO, &scalars_delta(vec![None, None])).expect("a delta that states neither");
        writer.append_delta(SimTime::ms(SLICE_WIDTH_MS), &scalars_delta(vec![None, None])).expect("the slice after it");
        writer.finish_with(SimTime::ms(2 * SLICE_WIDTH_MS), &End::default()).expect("closes");
    }
    let (state, _) =
        state_at(&mut Reader::from_bytes(&bytes).expect("reads"), SimTime::ms(SLICE_WIDTH_MS)).expect("seeks");
    assert_eq!(state.scalars.values, vec![Some(1.0), None], "a nil slot is absent, not zero");
}

/// A slice in which nothing happened still gets a delta, and that delta has
/// the shape every other one has: five sections, each present and empty. A
/// consumer reading the run never has to tell a section that says nothing
/// from one that was never written.
#[test]
fn an_empty_slice_is_a_delta_of_five_present_and_empty_sections() {
    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&Scenario::default()).expect("opens");
        writer.append_keyframe(SimTime::ZERO, &whole_keyframe(PlaybackKeyframe::default())).expect("a keyframe");
        writer.append_delta(SimTime::ZERO, &whole_delta(PlaybackDelta::default())).expect("a slice with no news");
        writer.finish_with(SimTime::ms(SLICE_WIDTH_MS), &End::default()).expect("closes");
    }
    let mut reader = Reader::from_bytes(&bytes).expect("reads");
    let delta = *reader.index().records().iter().find(|record| record.tag == Tag::Delta).expect("a delta");
    let sections = reader.sections_at(&delta).expect("the delta reads");
    assert_eq!(sections.ids(), vec![PLAYBACK, SCALARS, HEAT, TABLES, MOTION], "every section of a delta is there");
    let read = Delta::decode(&sections, delta.offset).expect("decodes");
    assert_eq!(read, whole_delta(PlaybackDelta::default()), "and each of them says nothing");
}

/// Records are fixed in arity on the way in as well as on the way out: a
/// reader counts positions, so a record with one more would leave two
/// consumers reading different fields.
#[test]
fn a_record_with_an_element_too_many_is_refused() {
    let place = encoded(&(1i32, 2i32, 3i32, 9i32));
    assert!(rmp_serde::from_slice::<Position>(&place).is_err(), "a position is three coordinates");
    let short = encoded(&(1i32, 2i32));
    assert!(rmp_serde::from_slice::<Position>(&short).is_err(), "and never two — every position states its height");
    let declared = encoded(&(1u8, pinned_metric(), 9u8));
    assert!(rmp_serde::from_slice::<Declare>(&declared).is_err(), "a declaration is a kind and a descriptor");
    let tote = encoded(&(ToteId(1), 2u8, 3u8, location(), 4u64, 9u8));
    assert!(rmp_serde::from_slice::<ToteRecord>(&tote).is_err(), "a tote record is five positions");
}

/// A keyframe states what is in the run. A tote that has left it is in no
/// keyframe: the encoder drops it and the decoder refuses one that says
/// otherwise.
#[test]
fn an_extracted_tote_is_left_out_of_a_keyframe_and_refused_in_one() {
    let gone = ToteRecord { location: ToteLocation::Extracted, ..tote_record() };
    let keyframe = PlaybackKeyframe { totes: vec![gone, tote_record()], ..PlaybackKeyframe::default() };
    let read = PlaybackKeyframe::from_bytes(&keyframe.to_bytes().expect("encodes"), 0).expect("decodes");
    assert_eq!(read.totes, vec![tote_record()], "the tote that left the run is not in the frame");

    let stated = encoded(&(ToteId(1), 2u8, 3u8, ToteLocation::Extracted, 4u64));
    assert!(rmp_serde::from_slice::<ToteRecord>(&stated).is_err(), "and one stated anyway is damage");
}

/// The block's id tables are state, so two producers that gathered the same
/// run in different orders write the same bytes; its descriptor lists are
/// not, because their order is what fixes a metric's slot.
#[test]
fn the_scenario_block_sorts_its_id_tables_and_leaves_its_descriptors_alone() {
    let scenario = Scenario {
        cells: vec![(CellId(2), "b".to_owned()), (CellId(1), "a".to_owned())],
        stations: vec![(StationId(2), "b".to_owned()), (StationId(1), "a".to_owned())],
        robots: vec![RobotEntry { robot: RobotId(2), ..robot_entry() }, robot_entry()],
        metrics: vec![metric(1), metric(0)],
        frame: HAND_BUILT_FRAME.to_owned(),
        ..Scenario::default()
    };
    let read = Scenario::from_bytes(&scenario.to_bytes().expect("encodes"), 0).expect("decodes");
    assert_eq!(read.cells, vec![(CellId(1), "a".to_owned()), (CellId(2), "b".to_owned())]);
    assert_eq!(read.stations.first().expect("a station").0, StationId(1));
    assert_eq!(read.robots.first().expect("a robot").robot, RobotId(1));
    assert_eq!(read.metrics, vec![metric(1), metric(0)], "the declaration order is what a slot means");
}

/// The magic ahead of the block says which version the file is. A block
/// saying another would leave a reader with two answers, so it is refused on
/// the way out and on the way in.
#[test]
fn a_block_disagreeing_with_the_magic_about_the_version_is_refused() {
    let wrong = Scenario { container_version: VERSION + 1, ..Scenario::default() };
    assert!(wrong.to_bytes().is_err(), "the encoder will not write it");

    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        let body = rmp_serde::to_vec(&(VERSION + 1, "", 0u32, "", "", false, "", "", SimDuration::ZERO, 0u32))
            .expect("a short block with a version of its own");
        writer.append_scenario(Ms::ZERO, &body).expect("the container carries what it is given");
        writer.finish_with(SimTime::ZERO, &End::default()).expect("closes");
    }
    let mut reader = Reader::from_bytes(&bytes).expect("the file still opens: the magic is right");
    let error = Scenario::read_from(&mut reader).expect_err("the block is not this version's");
    assert!(matches!(error, Error::UnsupportedVersion { .. } | Error::PayloadShape { .. }), "got {error}");
}

/// A recording's rows are byte-spliceable: a consumer that wants a whole
/// table concatenates the blocks the slices carry, in file order.
#[test]
fn a_tables_section_carries_only_the_rows_of_its_own_slice_and_they_splice() {
    let run = build(RunShape { slices: 4, cadence: 2, ..RunShape::default() });
    let mut reader = run.reader();
    let deltas: Vec<_> = reader.index().records().iter().filter(|record| record.tag == Tag::Delta).copied().collect();
    let mut whole = TableRows::new(TableId(1));
    for record in &deltas {
        let sections = reader.sections_at(record).expect("a delta reads");
        let tables = Delta::decode(&sections, record.offset).expect("decodes").tables.expect("a tables section");
        let rows = tables.get(TableId(1)).expect("the run writes one row per slice");
        whole.splice(rows).expect("one table's blocks splice");
    }
    assert_eq!(whole.row_count, deltas.len() as u32);
    let mut rows = Deserializer::new(whole.rows.as_slice());
    for slice in 0..deltas.len() as u64 {
        let row = <(u64, u64)>::deserialize(&mut rows).expect("a row reads");
        assert_eq!(row.0, slice * SLICE_WIDTH_MS, "each slice's rows carry that slice's instants");
    }
}
