//! A synthetic run written in the vocabulary's own records, with the run's
//! truth kept beside it.
//!
//! The builder chooses what happens in each slice, writes the frames that
//! state it, and keeps the answer in plain ordered maps it updates itself —
//! coordinates and the cells they count against, headings, tote locations and
//! contents, reserved cells, station depths, metric values, grid counters,
//! robot states — plus, for three instants inside every slice, where it meant
//! everything to be and what it meant every robot to be doing. Nothing here calls the state rebuild, so a fault in that code cannot
//! hide by being used on both sides of a test.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use rustc_hash::FxHashMap;

use desgraph::types::{CellId, StationId};
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use worldview::frames::declare::Descriptors;
use worldview::frames::end::End;
use worldview::frames::heat::{HeatDelta, HeatKeyframe, HeatLayer, HeatLayerDelta};
use worldview::frames::motion::{MotionDelta, MotionProfile, MotionSpan, RobotMotion};
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RelayTransition, RobotClass, RobotRecord, StationChange,
    StationState, Step, StepKind, ToteContents, ToteCreation, ToteLocation, ToteRecord, ToteTransit, ToteTransition,
};
use worldview::frames::robot_state::{
    CellRole, RobotSpans, RobotState, RobotStateDelta, RobotStateKeyframe, StateSpan, WaitCause,
};
use worldview::frames::scalars::{
    AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit, ScalarsDelta, ScalarsKeyframe,
};
use worldview::frames::scenario::{Document, HAND_BUILT_FRAME, RobotEntry, Scenario};
use worldview::frames::state::RunState;
use worldview::frames::tables::{
    ColumnEncoding, ColumnValueType, TableColumn, TableDescriptor, TableId, TableRows, TablesDelta,
};
use worldview::frames::{Delta, Keyframe, WriteFrames, section_declarations};
use worldview::{Reader, Writer};

/// Ten seconds, the slice width the cencora runs use.
pub const SLICE_WIDTH_MS: u64 = 10_000;

/// The layer every run counts, and the one only busier slices count.
pub const HEAT_LAYER: &str = "nav_visits_htm";
pub const SECOND_HEAT_LAYER: &str = "nav_visits_vtm";

/// The one table the fixture writes rows into.
pub const PICKS: TableId = TableId(1);

/// What run to build.
#[derive(Clone, Copy, Debug)]
pub struct RunShape {
    /// Slices between keyframes. One means every slice carries a keyframe.
    pub cadence: u32,
    pub slices: u32,
    pub robots: u32,
    pub totes: u32,
    pub stations: u32,
    pub metrics: usize,
    pub seed: u64,
    /// Whether the run states the robot state channel.
    pub robot_state: bool,
}

impl Default for RunShape {
    fn default() -> Self {
        Self { cadence: 30, slices: 90, robots: 8, totes: 12, stations: 3, metrics: 5, seed: 7, robot_state: false }
    }
}

/// A repeatable sequence of small numbers. The fixture needs variety, not
/// randomness, and a named seed makes a failing case reproducible.
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 16
    }

    pub fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 { 0 } else { (self.next() % u64::from(bound)) as u32 }
    }

    pub fn flip(&mut self) -> bool {
        self.next().is_multiple_of(2)
    }
}

/// Where the builder put a mover: its coordinates, and the cell those
/// coordinates count against.
///
/// The builder works both out for itself, from the steps it chose, and a test
/// compares them against what the rebuild says — so a fault in the rebuild
/// cannot be shared by the answer it is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Where {
    pub at: Position,
    pub cell: CellId,
}

impl Where {
    pub fn standing(cell: CellId, z_mm: i32) -> Self {
        Self { at: centre_of(cell, z_mm), cell }
    }
}

/// The point the fixture puts a cell's centre at, and the one it puts a step
/// that arrives off-centre at. The format states no centres, so these are the
/// builder's own invention; what matters is that they are far enough apart
/// that a rebuild reaching for the wrong one fails rather than passing by
/// luck.
pub fn centre_of(cell: CellId, z_mm: i32) -> Position {
    Position::new(cell.0 as i32 * 1_000, cell.0 as i32 * 10, z_mm)
}

pub fn off_centre(cell: CellId, index: u32, z_mm: i32) -> Position {
    Position::new(cell.0 as i32 * 1_000 + 37 + index as i32, cell.0 as i32 * 10 - 11, z_mm)
}

/// How high off the root frame's floor a robot of this id stands. The VTMs
/// ride the rack and the rest stand on the ground, which is what gives the
/// vertical coordinate something to carry through a whole run.
pub fn standing_z(robot: u32) -> i32 {
    match class_of(robot) {
        RobotClass::Vtm => 2_500,
        _ => 0,
    }
}

/// Where everything is at one slice boundary, as the builder meant it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Truth {
    pub at: SimTime,
    // ordered: the keyframes written from these maps state their arrays by
    // ascending id, and a test compares them entry by entry.
    pub positions: BTreeMap<RobotId, Where>,
    /// The cell each mover is heading for at this boundary, when a slice's
    /// end cut its leg. Kept beside the positions rather than in `Where`
    /// because a scrub inside a slice is never asked for it.
    pub toward: BTreeMap<RobotId, Option<CellId>>,
    pub headings: BTreeMap<RobotId, Option<i32>>,
    pub totes: BTreeMap<ToteId, (u8, u8, ToteLocation, u64)>,
    pub reserved: BTreeSet<CellId>,
    pub depths: BTreeMap<StationId, u32>,
    pub transits: Vec<ToteTransit>,
    pub scalars: Vec<Option<f64>>,
    pub heat: BTreeMap<String, BTreeMap<CellId, u64>>,
    /// What each robot is doing at this boundary, for a run that states the
    /// channel: the state a span beginning here begins in, else the one it
    /// carries on.
    pub states: Option<BTreeMap<RobotId, RobotState>>,
    /// How many totes the run has created so far, which is what names the
    /// next one.
    pub born: u32,
}

/// Where the builder meant everything to be at one instant inside a slice.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub at: SimTime,
    pub positions: BTreeMap<RobotId, Where>,
    pub totes: BTreeMap<ToteId, ToteLocation>,
    pub states: Option<BTreeMap<RobotId, RobotState>>,
}

/// One run's bytes and everything a test needs to check them.
pub struct BuiltRun {
    pub bytes: Vec<u8>,
    pub shape: RunShape,
    pub scenario: Scenario,
    /// One entry per slice: its start, the truth there, and three instants
    /// inside it.
    pub boundaries: Vec<SimTime>,
    pub truth: Vec<Truth>,
    pub probes: Vec<Vec<Probe>>,
    pub end: SimTime,
}

impl BuiltRun {
    pub fn reader(&self) -> Reader<Cursor<&[u8]>> {
        Reader::from_bytes(&self.bytes).expect("the run reads")
    }

    pub fn descriptors(&self) -> Descriptors {
        Descriptors::from_scenario(&self.scenario)
    }
}

impl Truth {
    /// What the rebuilt state must hold at this boundary, checked field by
    /// field so a failure says which one drifted.
    pub fn check(&self, state: &RunState) {
        let at = self.at.as_ms();
        assert_eq!(state.reached, self.at, "the state stands at {at} ms");
        let positions: BTreeMap<RobotId, Where> = state
            .playback
            .robots
            .iter()
            .map(|(robot, held)| (*robot, Where { at: held.at, cell: held.cell }))
            .collect();
        assert_eq!(positions, self.positions, "positions at {at} ms");
        let toward: BTreeMap<RobotId, Option<CellId>> =
            state.playback.robots.iter().map(|(robot, held)| (*robot, held.toward)).collect();
        assert_eq!(toward, self.toward, "the cells movers are heading for at {at} ms");
        let headings: BTreeMap<RobotId, Option<i32>> =
            state.playback.robots.iter().map(|(robot, held)| (*robot, held.heading_mrad)).collect();
        assert_eq!(headings, self.headings, "headings at {at} ms");
        let totes: BTreeMap<ToteId, (u8, u8, ToteLocation, u64)> = state
            .playback
            .totes
            .iter()
            .map(|(tote, held)| (*tote, (held.tote_type, held.designation, held.location, held.contents_units)))
            .collect();
        assert_eq!(totes, self.totes, "totes at {at} ms");
        assert_eq!(state.playback.reserved_relay_cells.iter().copied().collect::<BTreeSet<_>>(), self.reserved);
        assert_eq!(state.playback.stations.iter().map(|(s, d)| (*s, *d)).collect::<BTreeMap<_, _>>(), self.depths);
        assert_eq!(state.playback.transits_in_flight, self.transits, "journeys under way at {at} ms");
        assert_eq!(state.scalars.values, self.scalars, "metric values at {at} ms");
        let heat: BTreeMap<String, BTreeMap<CellId, u64>> = state
            .heat
            .layers
            .iter()
            .map(|(name, counts)| (name.clone(), counts.iter().map(|(cell, count)| (*cell, *count)).collect()))
            .collect();
        assert_eq!(heat, self.heat, "grid counters at {at} ms");
        assert_eq!(ordered(state.robot_states.as_ref().map(|held| &held.robots)), self.states, "states at {at} ms");
    }

    /// Which robot carries a tote, read off the tote locations — the pairing
    /// a keyframe states from both sides.
    fn carrying(&self, robot: RobotId) -> Option<ToteId> {
        self.totes
            .iter()
            .find(|(_, (_, _, location, _))| *location == ToteLocation::OnBot(robot))
            .map(|(tote, _)| *tote)
    }
}

/// A rebuilt state's robot states in the builder's order, so a mismatch says
/// which robot drifted.
pub fn ordered(states: Option<&FxHashMap<RobotId, RobotState>>) -> Option<BTreeMap<RobotId, RobotState>> {
    states.map(|states| states.iter().map(|(robot, state)| (*robot, *state)).collect())
}

/// The class a robot has for the whole run.
pub fn class_of(robot: u32) -> RobotClass {
    match robot % 3 {
        0 => RobotClass::Htm,
        1 => RobotClass::Vtm,
        _ => RobotClass::Rtp,
    }
}

/// The heading every robot begins the run with, before a delta turns it.
pub fn start_heading(robot: u32) -> Option<i32> {
    Some(robot as i32 * 250)
}

pub fn metric(index: usize) -> MetricDescriptor {
    let gauge = index % 3 == 2;
    MetricDescriptor {
        name: format!("metric_{index}"),
        label: format!("Metric {index}"),
        description: String::new(),
        semantics: MetricSemantics {
            unit: if gauge { MetricUnit::Ratio } else { MetricUnit::Count },
            denominator_description: None,
            aggregation: if gauge { AggregationClass::SnapshotGauge } else { AggregationClass::Additive },
        },
    }
}

pub fn picks_table() -> TableDescriptor {
    TableDescriptor {
        id: PICKS,
        name: "fulfillment.picks".to_owned(),
        columns: vec![
            TableColumn {
                name: "at".to_owned(),
                value_type: ColumnValueType::SimTime,
                encoding: ColumnEncoding::Positional,
            },
            TableColumn {
                name: "units".to_owned(),
                value_type: ColumnValueType::UInt,
                encoding: ColumnEncoding::Positional,
            },
        ],
        instant_column: Some(0),
    }
}

/// A scenario block carrying nothing but what every recording must state:
/// the frame its positions are measured in, and a document declaring it.
pub fn hand_built_scenario() -> Scenario {
    Scenario {
        frame: HAND_BUILT_FRAME.to_owned(),
        world: Document(format!("{{\"primary_frame\":\"{HAND_BUILT_FRAME}\"}}").into_bytes()),
        ..Scenario::default()
    }
}

pub fn scenario_of(shape: RunShape) -> Scenario {
    Scenario {
        run: format!("seed {}", shape.seed),
        attempt: 1,
        run_name: "fixture".to_owned(),
        commit: "0000000".to_owned(),
        timestamp: "2026-09-17T00:00:00Z".to_owned(),
        hostname: "test".to_owned(),
        slice_width: SimDuration::ms(SLICE_WIDTH_MS),
        keyframe_cadence_slices: shape.cadence,
        cells: (0..40).map(|cell| (CellId(cell), format!("cell-{cell}"))).collect(),
        stations: (0..shape.stations).map(|station| (StationId(station), format!("station-{station}"))).collect(),
        robots: (0..shape.robots)
            .map(|robot| RobotEntry {
                robot: RobotId(robot),
                name: format!("robot-{robot}"),
                class: class_of(robot),
                class_entity: None,
            })
            .collect(),
        metrics: (0..shape.metrics).map(metric).collect(),
        tables: vec![picks_table()],
        sections: section_declarations(shape.robot_state),
        // A recording names the frame its positions are measured in, and the
        // document it carries declares that frame primary. A fixture has no
        // warehouse behind it, so it carries the smallest document that can
        // say so rather than being excused the rule it exercises.
        frame: HAND_BUILT_FRAME.to_owned(),
        world: Document(format!("{{\"primary_frame\":\"{HAND_BUILT_FRAME}\"}}").into_bytes()),
        ..Scenario::default()
    }
}

/// One tote move the builder planned, in the order it happens.
struct Planned {
    offset_ms: u64,
    rank: u8,
    tote: ToteId,
    location: ToteLocation,
    born: Option<(u8, u8)>,
}

/// A keyframe as a file that does not state the robot state channel carries
/// it: the sections a test does not care about are present and empty, which
/// is what a file states for "nothing here".
pub fn whole_keyframe(playback: PlaybackKeyframe) -> Keyframe {
    Keyframe {
        playback: Some(playback),
        scalars: Some(ScalarsKeyframe::default()),
        heat: Some(HeatKeyframe::default()),
        robot_state: None,
    }
}

/// A delta as such a file carries it, all five sections present.
pub fn whole_delta(playback: PlaybackDelta) -> Delta {
    Delta {
        playback: Some(playback),
        scalars: Some(ScalarsDelta::default()),
        heat: Some(HeatDelta::default()),
        tables: Some(TablesDelta::default()),
        motion: Some(MotionDelta::default()),
        robot_state: None,
    }
}

/// Write a whole run and hand back its bytes and its truth.
pub fn build(shape: RunShape) -> BuiltRun {
    let mut bytes = Vec::new();
    let mut rng = Seeded::new(shape.seed);
    let scenario = scenario_of(shape);
    let mut now = Truth {
        at: SimTime::ZERO,
        positions: (0..shape.robots)
            .map(|robot| (RobotId(robot), Where::standing(CellId(robot), standing_z(robot))))
            .collect(),
        toward: (0..shape.robots).map(|robot| (RobotId(robot), None)).collect(),
        headings: (0..shape.robots).map(|robot| (RobotId(robot), start_heading(robot))).collect(),
        totes: (0..shape.totes)
            .map(|tote| {
                (
                    ToteId(tote),
                    (
                        (tote % 3) as u8,
                        (tote % 2) as u8,
                        ToteLocation::InRack { cell: CellId(100 + tote), level: (tote % 4) as u16 },
                        u64::from(tote),
                    ),
                )
            })
            .collect(),
        depths: (0..shape.stations).map(|station| (StationId(station), 0)).collect(),
        scalars: vec![None; shape.metrics],
        states: shape.robot_state.then(BTreeMap::new),
        ..Truth::default()
    };
    let mut boundaries = Vec::new();
    let mut truth = Vec::new();
    let mut probes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&scenario).expect("the scenario block opens the file");

        for slice in 0..shape.slices {
            let at = SimTime::ms(u64::from(slice) * SLICE_WIDTH_MS);
            let ends = at.saturating_add(SimDuration::ms(SLICE_WIDTH_MS));
            now.at = at;
            boundaries.push(at);
            // The slice states the headings its own start has, so the truth
            // at that boundary — and the keyframe written from it — is only
            // known once the slice has been chosen.
            let (delta, slice_probes, at_start) = slice_of(&mut rng, &mut now, shape, at, ends);
            truth.push(at_start.clone());
            if slice.is_multiple_of(shape.cadence) {
                writer.append_keyframe(at, &keyframe_of(&at_start)).expect("a keyframe");
            }
            probes.push(slice_probes);
            writer.append_delta(at, &delta).expect("a delta");
        }

        let end = SimTime::ms(u64::from(shape.slices) * SLICE_WIDTH_MS);
        now.at = end;
        let ending = End {
            stop_reason: "reached the horizon".to_owned(),
            cumulative_scalars: now.scalars.clone(),
            frames_written: writer.frames_written(),
            ..End::default()
        };
        writer.finish_with(end, &ending).expect("the file closes");
    }
    let end = SimTime::ms(u64::from(shape.slices) * SLICE_WIDTH_MS);
    BuiltRun { bytes, shape, scenario, boundaries, truth, probes, end }
}

fn keyframe_of(now: &Truth) -> Keyframe {
    Keyframe {
        playback: Some(PlaybackKeyframe {
            robots: now
                .positions
                .iter()
                .map(|(robot, put)| RobotRecord {
                    robot: *robot,
                    class: class_of(robot.0),
                    at: put.at,
                    heading_mrad: now.headings[robot],
                    carrying: now.carrying(*robot),
                    cell: put.cell,
                    toward: now.toward[robot],
                })
                .collect(),
            totes: now
                .totes
                .iter()
                .map(|(tote, (tote_type, designation, location, contents_units))| ToteRecord {
                    tote: *tote,
                    tote_type: *tote_type,
                    designation: *designation,
                    location: *location,
                    contents_units: *contents_units,
                })
                .collect(),
            reserved_relay_cells: now.reserved.iter().copied().collect(),
            stations: now
                .depths
                .iter()
                .map(|(station, depth)| StationState { station: *station, pipeline_depth: *depth })
                .collect(),
            transits_in_flight: now.transits.clone(),
        }),
        scalars: Some(ScalarsKeyframe { values: now.scalars.clone() }),
        heat: Some(HeatKeyframe {
            layers: now
                .heat
                .iter()
                .map(|(name, counts)| HeatLayer {
                    name: name.clone(),
                    counts: counts.iter().map(|(cell, count)| (*cell, *count)).collect(),
                })
                .collect(),
        }),
        robot_state: now
            .states
            .as_ref()
            .map(|states| RobotStateKeyframe(states.iter().map(|(robot, state)| (*robot, *state)).collect())),
    }
}

/// Choose what one slice does, write the truth forward, and say where
/// everything is at three instants inside it.
fn slice_of(
    rng: &mut Seeded,
    now: &mut Truth,
    shape: RunShape,
    at: SimTime,
    ends: SimTime,
) -> (Delta, Vec<Probe>, Truth) {
    let mut playback = PlaybackDelta::default();
    let mut motion = Vec::new();
    let mut moved: BTreeMap<RobotId, (Where, MoverSteps)> = BTreeMap::new();

    // Robots that turned since the slice before, whether or not they go
    // anywhere in this one. They are true from this slice's start, so the
    // builder's truth takes them before the keyframe is written.
    playback.headings = turned(rng, now, shape);
    for (robot, heading) in &playback.headings {
        now.headings.insert(*robot, *heading);
    }
    // A state that begins with the slice is the one its start has, like a
    // heading, and so is the one the keyframe there states.
    let changes = now.states.as_ref().map(|held| state_changes(rng, held, shape)).unwrap_or_default();
    let robot_state = now.states.as_ref().map(|held| robot_state_delta(held, &changes));
    if let Some(held) = now.states.as_mut() {
        for (robot, began) in &changes {
            if let Some((0, state)) = began.first() {
                held.insert(*robot, *state);
            }
        }
    }

    for _ in 0..=rng.below(3) {
        let robot = RobotId(rng.below(shape.robots));
        if moved.contains_key(&robot) {
            continue;
        }
        let began = now.positions[&robot];
        let (mover, spans) = walk(rng, robot, began, at, now.headings[&robot]);
        moved.insert(robot, (began, mover.clone()));
        if !spans.is_empty() {
            motion.push(RobotMotion { robot, spans });
        }
        playback.movers.push(mover);
    }

    let (planned, transits) = plan_tote_moves(rng, now, shape, at, ends, now.born);
    playback.tote_transits = transits;
    for move_ in &planned {
        match (move_.born, move_.rank) {
            (Some((tote_type, designation)), _) => playback.tote_creations.push(ToteCreation {
                offset: SimDuration::ms(move_.offset_ms),
                tote: move_.tote,
                tote_type,
                designation,
                location: move_.location,
            }),
            // A journey's landing is stated by the journey itself, which is
            // already in the delta; only the other moves are transitions.
            (None, 1) => {}
            (None, _) => playback.tote_transitions.push(ToteTransition {
                offset: SimDuration::ms(move_.offset_ms),
                tote: move_.tote,
                location: move_.location,
            }),
        }
    }
    now.born += u32::from(planned.iter().any(|move_| move_.born.is_some()));
    if rng.below(3) == 0 && shape.totes > 0 {
        let tote = ToteId(rng.below(shape.totes));
        let record = ToteContents {
            offset: SimDuration::ms(u64::from(rng.below(9_000))),
            tote,
            units: u64::from(rng.below(30)),
        };
        // Only a container the run still holds: one it has extracted has left
        // it, and a contents record naming that container is refused.
        if now.totes.contains_key(&tote) {
            playback.tote_contents.push(record);
        }
    }
    for _ in 0..rng.below(3) {
        playback.relay_transitions.push(RelayTransition {
            cell: CellId(rng.below(24)),
            offset: SimDuration::ms(u64::from(rng.below(9_000))),
            reserved: rng.flip(),
        });
    }
    if rng.flip() && shape.stations > 0 {
        playback.station_changes.push(StationChange {
            offset: SimDuration::ms(u64::from(rng.below(9_000))),
            station: StationId(rng.below(shape.stations)),
            pipeline_depth: rng.below(6),
        });
    }

    let scalars = ScalarsDelta {
        values: (0..shape.metrics)
            .map(|index| match (index + at.as_ms() as usize / 1_000) % 4 {
                0 => None,
                _ => Some(f64::from(rng.below(10))),
            })
            .collect(),
    };
    let mut increments = vec![HeatLayerDelta {
        name: HEAT_LAYER.to_owned(),
        increments: (0..=rng.below(3)).map(|_| (CellId(rng.below(40)), u64::from(rng.below(5)) + 1)).collect(),
    }];
    if rng.below(4) == 0 {
        increments
            .push(HeatLayerDelta { name: SECOND_HEAT_LAYER.to_owned(), increments: vec![(CellId(rng.below(40)), 1)] });
    }
    let heat = HeatDelta { layers: increments };

    let probes = probes_of(now, &moved, &planned, &changes, at);
    let at_start = now.clone();
    step_truth(now, &moved, &planned, &playback, &scalars, &heat, ends);
    if let Some(held) = now.states.as_mut() {
        for (robot, changed) in &changes {
            if let Some((_, state)) = changed.last() {
                held.insert(*robot, *state);
            }
        }
    }

    let delta = Delta {
        playback: Some(playback),
        scalars: Some(scalars),
        heat: Some(heat),
        tables: Some(TablesDelta(vec![TableRows {
            table: PICKS,
            row_count: 1,
            rows: rmp_serde::to_vec(&(at.as_ms(), 3u64)).expect("a row encodes"),
        }])),
        motion: Some(MotionDelta(motion)),
        robot_state,
    };
    (delta, probes, at_start)
}

/// Every state the format names, waits and no task with a cell role and
/// without one, so a run exercises each kind byte and every nil.
const STATES: [RobotState; 12] = [
    RobotState::Pickup,
    RobotState::Drop,
    RobotState::Alignment,
    RobotState::EmptyTravel,
    RobotState::LadenTravel,
    RobotState::TravelToPark,
    RobotState::StationProcessing,
    RobotState::Waiting { cause: WaitCause::RouteHold, cell: Some(CellRole::StationQueue) },
    RobotState::Waiting { cause: WaitCause::AwaitingSlot, cell: Some(CellRole::Relay) },
    RobotState::Waiting { cause: WaitCause::AwaitingDeposit, cell: None },
    RobotState::NoTask { cell: Some(CellRole::Charging) },
    RobotState::NoTask { cell: None },
];

/// Which robots change state in one slice, and each change's offset into it
/// and the state it begins, in order. A robot holding no state yet — every
/// robot as the run begins — begins one at the slice's very start, and a few
/// others change there too: that is the boundary a keyframe written at the
/// slice's start must already state. Some change at the very instant a probe
/// reads, where the span beginning there is the answer.
fn state_changes(
    rng: &mut Seeded,
    held: &BTreeMap<RobotId, RobotState>,
    shape: RunShape,
) -> BTreeMap<RobotId, Vec<(u64, RobotState)>> {
    let mut changes: BTreeMap<RobotId, Vec<(u64, RobotState)>> = (0..shape.robots)
        .map(RobotId)
        .filter(|robot| !held.contains_key(robot))
        .map(|robot| (robot, vec![(0, STATES[rng.below(STATES.len() as u32) as usize])]))
        .collect();
    for _ in 0..rng.below(4) {
        let robot = RobotId(rng.below(shape.robots));
        if changes.contains_key(&robot) {
            continue;
        }
        let offsets: BTreeSet<u64> = (0..=rng.below(3))
            .map(|_| match rng.below(5) {
                0 => 0,
                1 => SLICE_WIDTH_MS / 4 * u64::from(1 + rng.below(3)),
                _ => u64::from(rng.below(SLICE_WIDTH_MS as u32 - 1)) + 1,
            })
            .collect();
        let states = offsets.into_iter().map(|offset| (offset, STATES[rng.below(STATES.len() as u32) as usize]));
        changes.insert(robot, states.collect());
    }
    changes
}

/// The spans a slice states for each robot that changes state in it, laid end
/// to end across the slice: one carrying on the held state up to the first
/// change, unless that change is at the slice's start, then one per change.
fn robot_state_delta(
    held: &BTreeMap<RobotId, RobotState>,
    changes: &BTreeMap<RobotId, Vec<(u64, RobotState)>>,
) -> RobotStateDelta {
    RobotStateDelta(
        changes
            .iter()
            .map(|(robot, changed)| {
                let continues = changed.first().is_some_and(|(offset, _)| *offset > 0);
                let mut spans = Vec::new();
                if let (true, Some((first, _))) = (continues, changed.first()) {
                    spans.push(StateSpan { dt: SimDuration::ms(*first), state: held[robot] });
                }
                for (index, (offset, state)) in changed.iter().enumerate() {
                    let until = changed.get(index + 1).map_or(SLICE_WIDTH_MS, |(next, _)| *next);
                    spans.push(StateSpan { dt: SimDuration::ms(until - offset), state: *state });
                }
                RobotSpans { robot: *robot, continues, spans }
            })
            .collect(),
    )
}

/// The robots whose heading at this slice's start is not the one the reader
/// was holding. A few of them lose their heading altogether, which is the
/// producer saying it no longer tracks one.
fn turned(rng: &mut Seeded, now: &Truth, shape: RunShape) -> Vec<(RobotId, Option<i32>)> {
    let mut turned: Vec<(RobotId, Option<i32>)> = Vec::new();
    for _ in 0..rng.below(3) {
        let robot = RobotId(rng.below(shape.robots));
        if turned.iter().any(|(already, _)| *already == robot) {
            continue;
        }
        let heading = match rng.below(8) {
            0 => None,
            _ => Some(rng.below(6_283) as i32),
        };
        if now.headings[&robot] != heading {
            turned.push((robot, heading));
        }
    }
    turned
}

/// One mover's slice: it walks a few legs, and the slice's end may cut its
/// last one part way.
fn walk(
    rng: &mut Seeded,
    robot: RobotId,
    began: Where,
    at: SimTime,
    heading: Option<i32>,
) -> (MoverSteps, Vec<MotionSpan>) {
    let z_mm = standing_z(robot.0);
    let mut steps: Vec<Step> = Vec::new();
    let mut spans = Vec::new();
    let mut standing = began.cell;
    let count = 1 + rng.below(3);
    let mut when = at;
    for index in 0..count {
        let travels = rng.below(4) != 0;
        let dt = SimDuration::ms(u64::from(rng.below(1_500)) + 500);
        let cell = match travels {
            true => CellId((standing.0 + 1 + rng.below(3)) % 40),
            false => standing,
        };
        // Every kind the format names: a move and a reverse travel the leg, a
        // wait and a rotate spend their time where the mover already stands.
        let kind = match (travels, rng.flip()) {
            (true, true) => StepKind::Move,
            (true, false) => StepKind::Reverse,
            (false, true) => StepKind::Wait,
            (false, false) => StepKind::Rotate,
        };
        // Two steps in three arrive off their cell's own centre, which is
        // what a curved or rasterised leg looks like in a recording: several
        // steps naming one cell, each at its own point on the arc.
        let arrives = match rng.below(3) {
            0 => centre_of(cell, z_mm),
            _ => off_centre(cell, index, z_mm),
        };
        steps.push(Step { at: arrives, dt, kind, cell });
        if travels {
            spans.push(MotionSpan {
                source: standing,
                destination: cell,
                start: when,
                end: when.saturating_add(dt),
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
            });
            standing = cell;
        }
        when = when.saturating_add(dt);
    }
    // A slice's end can cut a travelling last step part way; a mover that
    // spent its last step standing still ends the slice where it stood. The
    // cut step is an ordinary step whose coordinates say where the mover
    // stopped and whose cell says where it was going.
    let last = steps.len() - 1;
    let mut left_in = steps[last].cell;
    if steps[last].kind.travels() && rng.flip() {
        let (before, from_cell) = match last {
            0 => (began.at, began.cell),
            index => (steps[index - 1].at, steps[index - 1].cell),
        };
        let along = f64::from(100 + rng.below(800)) / 1_000.0;
        let axis = |from: i32, to: i32| (f64::from(from) + (f64::from(to) - f64::from(from)) * along).round() as i32;
        steps[last].at = Position::new(
            axis(before.x_mm, steps[last].at.x_mm),
            axis(before.y_mm, steps[last].at.y_mm),
            axis(before.z_mm, steps[last].at.z_mm),
        );
        left_in = from_cell;
    }
    let mover = MoverSteps {
        robot,
        // A quarter of the movers state no heading, which says nothing and
        // leaves the robot facing where it already faced.
        start_heading_mrad: if rng.below(4) == 0 { None } else { heading },
        steps,
        cell: left_in,
    };
    (mover, spans)
}

/// What happens to the totes in one slice: a few of them move, one may be
/// born, one may leave the run, and one may set off along a conveyor —
/// which lands in this slice or in a later one.
///
/// The three kinds go into three arrays of the delta, and the order they
/// happen in is the one a reader has to put them back into: a birth, then a
/// journey coming to rest, then a move.
fn plan_tote_moves(
    rng: &mut Seeded,
    now: &Truth,
    shape: RunShape,
    at: SimTime,
    ends: SimTime,
    born: u32,
) -> (Vec<Planned>, Vec<ToteTransit>) {
    let mut births: Vec<Planned> = Vec::new();
    let mut landings: Vec<Planned> = Vec::new();
    let mut moves: Vec<Planned> = Vec::new();
    let mut transits: Vec<ToteTransit> = Vec::new();
    let mut touched: Vec<ToteId> = Vec::new();

    if rng.below(4) == 0 {
        // A tote born in this slice, which no keyframe before it states.
        let tote = ToteId(shape.totes + born);
        touched.push(tote);
        births.push(Planned {
            offset_ms: u64::from(rng.below(9_000)),
            rank: 0,
            tote,
            location: ToteLocation::AtCell(CellId(rng.below(40))),
            born: Some((rng.below(3) as u8, rng.below(2) as u8)),
        });
    }
    if rng.below(5) == 0 && shape.totes > 0 {
        // A journey along a conveyor. One that reaches past this slice has
        // not landed, and the tote stays in flight rather than at its
        // destination.
        let tote = ToteId(rng.below(shape.totes));
        // A container the run has extracted has left it, and no producer names
        // one again without creating it afresh — which the reader refuses, so
        // the builder must not plan it either.
        if !touched.contains(&tote) && now.totes.contains_key(&tote) {
            touched.push(tote);
            let start = at.saturating_add(SimDuration::ms(u64::from(rng.below(4_000))));
            let end = start.saturating_add(SimDuration::ms(u64::from(rng.below(12_000)) + 1_000));
            let to_cell = CellId(rng.below(40));
            transits.push(ToteTransit { tote, from_cell: CellId(rng.below(40)), to_cell, start, end });
            if end <= ends {
                landings.push(Planned {
                    offset_ms: end.as_ms() - at.as_ms(),
                    rank: 1,
                    tote,
                    location: ToteLocation::OnConveyor(to_cell),
                    born: None,
                });
            }
        }
    }
    for _ in 0..rng.below(3) {
        let tote = ToteId(rng.below(shape.totes));
        if touched.contains(&tote) || !now.totes.contains_key(&tote) {
            continue;
        }
        touched.push(tote);
        let robot = RobotId(rng.below(shape.robots));
        let riding = now.totes.values().any(|(_, _, location, _)| *location == ToteLocation::OnBot(robot))
            || moves.iter().any(|move_| move_.location == ToteLocation::OnBot(robot));
        let location = match rng.below(6) {
            0 => ToteLocation::AtCell(CellId(rng.below(40))),
            1 => ToteLocation::InRack { cell: CellId(rng.below(40)), level: rng.below(4) as u16 },
            2 => ToteLocation::AtRelay(CellId(rng.below(24))),
            // One tote to a robot: a keyframe states the pairing from both
            // sides, and two totes on one robot would leave it ambiguous.
            3 if !riding => ToteLocation::OnBot(robot),
            // A tote leaving the run altogether, which no keyframe after
            // this slice states.
            4 => ToteLocation::Extracted,
            _ => ToteLocation::OnConveyor(CellId(rng.below(40))),
        };
        moves.push(Planned { offset_ms: u64::from(rng.below(9_000)), rank: 2, tote, location, born: None });
    }

    let mut planned: Vec<Planned> = births;
    planned.extend(landings);
    planned.extend(moves);
    // Stable and by nothing else, the way a reader merges the three arrays.
    planned.sort_by_key(|move_| (move_.offset_ms, move_.rank));
    (planned, transits)
}

/// Where everything is at a quarter, a half and three quarters of the way
/// through the slice.
fn probes_of(
    now: &Truth,
    moved: &BTreeMap<RobotId, (Where, MoverSteps)>,
    planned: &[Planned],
    changes: &BTreeMap<RobotId, Vec<(u64, RobotState)>>,
    at: SimTime,
) -> Vec<Probe> {
    [SLICE_WIDTH_MS / 4, SLICE_WIDTH_MS / 2, 3 * SLICE_WIDTH_MS / 4]
        .into_iter()
        .map(|elapsed_ms| {
            let mut positions = now.positions.clone();
            for (robot, (began, mover)) in moved {
                positions.insert(*robot, walked_to(*began, mover, elapsed_ms));
            }
            let mut totes: BTreeMap<ToteId, ToteLocation> =
                now.totes.iter().map(|(tote, (_, _, location, _))| (*tote, *location)).collect();
            for move_ in planned.iter().filter(|move_| move_.offset_ms <= elapsed_ms) {
                match move_.location {
                    ToteLocation::Extracted => totes.remove(&move_.tote),
                    location => totes.insert(move_.tote, location),
                };
            }
            let states = now.states.as_ref().map(|held| {
                let mut states = held.clone();
                for (robot, changed) in changes {
                    if let Some((_, state)) = changed.iter().rev().find(|(offset, _)| *offset <= elapsed_ms) {
                        states.insert(*robot, *state);
                    }
                }
                states
            });
            Probe { at: at.saturating_add(SimDuration::ms(elapsed_ms)), positions, totes, states }
        })
        .collect()
}

/// Where a mover has got to `elapsed_ms` into its slice, worked out from the
/// steps the builder chose rather than from the code that reads them. A mover
/// part-way along a step still counts against the cell it has not left.
fn walked_to(began: Where, mover: &MoverSteps, elapsed_ms: u64) -> Where {
    let mut before = began.at;
    let mut cell = began.cell;
    let mut walked_ms = 0;
    for step in &mover.steps {
        if elapsed_ms < walked_ms + step.dt.as_ms() {
            if !step.kind.travels() {
                return Where { at: step.at, cell: step.cell };
            }
            let along = (elapsed_ms - walked_ms) as f64 / step.dt.as_ms() as f64;
            let axis =
                |from: i32, to: i32| (f64::from(from) + (f64::from(to) - f64::from(from)) * along).round() as i32;
            let at = Position::new(
                axis(before.x_mm, step.at.x_mm),
                axis(before.y_mm, step.at.y_mm),
                axis(before.z_mm, step.at.z_mm),
            );
            return Where { at, cell };
        }
        walked_ms += step.dt.as_ms();
        before = step.at;
        cell = step.cell;
    }
    end_of_slice(began, mover)
}

/// Where a slice leaves a mover.
fn end_of_slice(began: Where, mover: &MoverSteps) -> Where {
    mover.steps.last().map_or(began, |step| Where { at: step.at, cell: mover.cell })
}

/// Take the builder's own truth through one slice.
fn step_truth(
    now: &mut Truth,
    moved: &BTreeMap<RobotId, (Where, MoverSteps)>,
    planned: &[Planned],
    playback: &PlaybackDelta,
    scalars: &ScalarsDelta,
    heat: &HeatDelta,
    ends: SimTime,
) {
    for (robot, (began, mover)) in moved {
        now.positions.insert(*robot, end_of_slice(*began, mover));
        now.toward.insert(*robot, mover.heading_for());
    }
    for move_ in planned {
        match move_.location {
            ToteLocation::Extracted => {
                now.totes.remove(&move_.tote);
            }
            location => match move_.born {
                // A creation states the container's two permanent facts as it
                // places it; nothing else introduces one, which is what the
                // reader refuses a stream for doing.
                Some((tote_type, designation)) => {
                    let entry = now.totes.entry(move_.tote).or_insert((tote_type, designation, location, 0));
                    *entry = (tote_type, designation, location, entry.3);
                }
                None => {
                    if let Some(entry) = now.totes.get_mut(&move_.tote) {
                        entry.2 = location;
                    }
                }
            },
        }
    }
    for contents in &playback.tote_contents {
        if let Some(entry) = now.totes.get_mut(&contents.tote) {
            entry.3 = contents.units;
        }
    }
    for relay in &playback.relay_transitions {
        match relay.reserved {
            true => now.reserved.insert(relay.cell),
            false => now.reserved.remove(&relay.cell),
        };
    }
    for change in &playback.station_changes {
        now.depths.insert(change.station, change.pipeline_depth);
    }
    now.transits.retain(|transit| transit.end > ends);
    now.transits.extend(playback.tote_transits.iter().copied().filter(|transit| transit.end > ends));
    for (slot, stated) in scalars.values.iter().enumerate() {
        let Some(value) = stated else { continue };
        let gauge = metric(slot).semantics.aggregation == AggregationClass::SnapshotGauge;
        let running = if gauge { None } else { now.scalars[slot] };
        now.scalars[slot] = Some(running.unwrap_or(0.0) + value);
    }
    for layer in &heat.layers {
        let counts = now.heat.entry(layer.name.clone()).or_default();
        for (cell, increment) in &layer.increments {
            *counts.entry(*cell).or_insert(0) += increment;
        }
    }
}
