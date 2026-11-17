//! Within `worldview`, `read` is THE READING END: a recording becomes the run
//! model again, whether it is a finished `.worldview` file or bytes still
//! coming in from a run being written ([`incoming`]).
//!
//! **What it is for.** Everything downstream of a run is written against the
//! run model, so a recording is only as useful as the rebuild that puts one
//! back together. This takes a recording's frames one at a time in file
//! order — the scenario block, then each keyframe, declaration and delta —
//! and assembles the [`PerfProfile`] the viewer, the workbook, `compare` and
//! `repeat` already read. A file and the bytes of a run being written go
//! through the same intake, so they cannot be read two ways.
//!
//! **What comes back exactly, and what does not.** Every number a consumer
//! reads comes back as it went in. Positions are the one approximation, and a
//! stated one: a recording carries whole millimetres, so a position comes back
//! within half a millimetre of the one the run computed rather than on it.
//!
//! **A run that died still opens.** The container rebuilds the index by
//! scanning the frames when no trailer describes the file, so every slice
//! written before the break reads; what is missing is the end block, and
//! [`Recording::truncated`] says so rather than the read failing.
//!
//! TODO(module): two jobs share this file. One walks the frames and refuses
//! what a file may not say — a delta missing a section, a keyframe whose
//! counters disagree with the deltas, a cell the world does not place. The
//! other assembles the run model out of what survives that walk. The second
//! is the mirror of `write.rs`'s `Producer` and belongs beside it; the first
//! is the format's own reading rules and belongs nearer `worldview::frames`.

mod incoming;

use std::collections::HashMap;
use std::fmt::Display;
use std::path::Path;

use desgraph::types::{RobotType, WorldPos};
use desunits::{RobotId, SimDuration, SimTime};
use rustc_hash::FxHashMap;
use serde_json::Value;
use site_model::LoadError;
use site_model::flagged_export::{self, FlaggedExport};
use smallvec::SmallVec;
use worldview::container::index::IndexRecord;
use worldview::container::payload::Payload;
use worldview::container::reader::Reader;
use worldview::container::sections::Sections;
use worldview::frames::declare::{Declare, Descriptors};
use worldview::frames::end::End;
use worldview::frames::heat::HeatKeyframe;
use worldview::frames::motion::{MotionDelta, MotionSpan as SpanRecord};
use worldview::frames::playback::{MoverSteps, PlaybackDelta, Position};
use worldview::frames::scenario::Scenario;
use worldview::frames::state::RunState;
use worldview::frames::tables::{TableId, TableRows};
use worldview::frames::{Delta, Keyframe};
use worldview::{Frame, Tag};

use crate::config::SimConfig;
use crate::metrics::profile::{
    DynamicGrid, IdTables, PerfProfile, ProfileMetadata, RobotIdentity, RunDocuments, ScenarioSource, SliceRecord,
    SummaryStats,
};
use crate::metrics::streams::{Entries, EntryEncoding, LogStream, StreamDescriptor, StreamError};
use crate::metrics::worldview::journals;
use crate::metrics::worldview::summary::read_document;
use crate::metrics::worldview::wire::{bot_step_kind, designation_of, radians, tote_type_of, visual_state};
use crate::metrics::worldview::write::ProducerDocument;
use crate::observer::station_diag::StationDiagRecord;
use crate::observer::time_slice::{
    BotLocation, BotStep, RelayStateRecord, ToteCreationRecord, ToteStateRecord, ToteTransitRecord, ToteVisualState,
};
use crate::observer::traffic::MotionSpan;
use crate::pathfinding::TrapezoidalProfile;
use crate::sim_types::SimResult;

pub use incoming::IncomingRecording;

/// The floor-plane part of a recorded position, which is what the run model's
/// two-dimensional `WorldPos` holds. The height travels beside it.
fn floor_plane(at: Position) -> WorldPos {
    WorldPos { x_mm: f64::from(at.x_mm), y_mm: f64::from(at.y_mm) }
}

/// One mover's steps as the run model holds them. Every number comes from the
/// file: the recording states where each step left the robot, so nothing here
/// consults the floor to find out.
fn steps(mover: Option<&MoverSteps>) -> SmallVec<[BotStep; 4]> {
    let Some(mover) = mover else { return SmallVec::new() };
    mover
        .steps
        .iter()
        .map(|step| BotStep {
            cell_id: step.cell.0,
            pos: floor_plane(step.at),
            z_mm: f64::from(step.at.z_mm),
            delta_ms: step.dt.as_ms(),
            kind: bot_step_kind(step.kind),
        })
        .collect()
}

/// A run model read back from a file, and whether the file had an end.
pub struct Recording {
    pub profile: PerfProfile,
    /// The run crashed or is still going: every slice ahead of the break is
    /// here, and the run's totals, summary and result are not.
    pub truncated: bool,
}

/// Read one `.worldview` file into the run model: the container finds the
/// frames, through the trailer or by scanning a file that has none, and the
/// intake takes each one the index lists, in order.
pub fn read_worldview(path: &Path) -> Result<Recording, String> {
    let in_file = |e: String| format!("{}: {e}", path.display());
    let mut reader = Reader::open(path).map_err(|e| in_file(e.to_string()))?;
    let scenario = Scenario::read_from(&mut reader).map_err(|e| in_file(e.to_string()))?;
    let mut intake = FrameIntake::open(scenario).map_err(in_file)?;
    let records: Vec<IndexRecord> = reader.index().records().to_vec();
    // The first record is the scenario block the intake opened on.
    for record in records.iter().skip(1) {
        let frame = reader.frame_at(record.offset).map_err(|e| in_file(frame_error(record.tag, record.offset, e)))?;
        intake.take(record.offset, &frame).map_err(in_file)?;
    }
    intake.into_recording().map_err(in_file)
}

/// A recording taken a frame at a time, in file order: the run model the
/// frames taken so far rebuild, which becomes the [`Recording`] once the
/// last of them is in.
struct FrameIntake {
    scenario: Scenario,
    /// The scenario block's descriptors and every declaration taken since.
    descriptors: Descriptors,
    build: Build,
    /// How many orders the run's scenario resolved to, from the block.
    orders: usize,
    end: Option<End>,
}

impl FrameIntake {
    /// Open on the scenario block, which states what every frame after it is
    /// read against.
    fn open(scenario: Scenario) -> Result<Self, String> {
        let build = Build::new(&scenario)?;
        let orders = order_total(&build.producer, &build.sim_config);
        Ok(Self { descriptors: Descriptors::from_scenario(&scenario), scenario, build, orders, end: None })
    }

    /// Take the frame that begins at `offset`, the one after the last taken.
    /// The index block is the container's bookkeeping and passes by.
    fn take(&mut self, offset: u64, frame: &Frame) -> Result<(), String> {
        self.take_frame(offset, frame).map_err(|e| frame_error(frame.header.tag, offset, e))
    }

    fn take_frame(&mut self, offset: u64, frame: &Frame) -> Result<(), String> {
        let at = SimTime::ms(frame.header.t.0);
        let body = || frame.contents(offset).map_err(|e| e.to_string());
        let sections = || Sections::decode(body()?, offset).map_err(|e| e.to_string());
        // A slice ends where the next frame begins: the next slice's, or the
        // end block's, stamped at the run's stop, which may cut the last slice
        // short of the scenario's width.
        if frame.header.tag != Tag::Index {
            self.build.close_pending(at)?;
        }
        match frame.header.tag {
            Tag::Keyframe => {
                let keyframe = Keyframe::decode(&sections()?, offset).map_err(|e| e.to_string())?;
                self.build.take_keyframe(at, &keyframe, &self.descriptors)?;
            }
            Tag::Declare => {
                // A run states a metric or a table when the thing it measures
                // first exists, so the descriptors a slice is read against
                // are the block's plus every declaration made by then.
                let declared = Declare::from_bytes(&body()?, offset).map_err(|e| e.to_string())?;
                self.descriptors.push(&declared);
                self.build.state.scalars.describe(&self.descriptors);
            }
            Tag::Delta => {
                let delta = Delta::decode(&sections()?, offset).map_err(|e| e.to_string())?;
                whole_delta(&delta)?;
                self.build.take_delta(at, delta, &self.descriptors)?;
            }
            Tag::End => self.end = Some(End::from_bytes(&body()?, offset).map_err(|e| e.to_string())?),
            Tag::Index => {}
            Tag::Scenario => return Err("a recording opens with its one scenario block, and this is a second".into()),
        }
        Ok(())
    }

    fn slice_width(&self) -> SimDuration {
        self.scenario.slice_width
    }

    fn slices(&self) -> &[SliceRecord] {
        &self.build.slices
    }

    fn cumulative_grid(&self) -> &DynamicGrid {
        &self.build.raised
    }

    fn has_ended(&self) -> bool {
        self.end.is_some()
    }

    fn robots(&self) -> Vec<RobotIdentity> {
        roster(&self.scenario)
    }

    fn orders(&self) -> usize {
        self.orders
    }

    /// The run model, with the run's totals, summary and result when the end
    /// block came in, and [`Recording::truncated`] when it did not.
    fn into_recording(self) -> Result<Recording, String> {
        self.build.into_recording(&self.scenario, self.end, &self.descriptors)
    }
}

/// How many orders a run resolved its scenario to: a loaded scenario's own
/// list or the workload a parametric one resolved to, cut as the run cut them
/// to its configuration's `max_orders`. A loaded scenario's block states the
/// whole list, uncut.
fn order_total(producer: &ProducerDocument, config: &SimConfig) -> usize {
    let listed = match &producer.scenario_source {
        ScenarioSource::Loaded { orders, .. } => orders.len(),
        ScenarioSource::Parametric(_) => producer.workload.orders.len(),
    };
    config.max_orders.map_or(listed, |cap| listed.min(cap))
}

/// Every section a delta of a FILE carries. A wire may send fewer — that is
/// what a browser asking for less gets — but a file states all five in every
/// slice, so one missing here is damage rather than a filter, and reading
/// past it would silently drop a slice's movers or its counters.
fn whole_delta(delta: &Delta) -> Result<(), String> {
    let missing: Vec<&str> = [
        ("playback", delta.playback.is_some()),
        ("scalars", delta.scalars.is_some()),
        ("heat", delta.heat.is_some()),
        ("tables", delta.tables.is_some()),
        ("motion", delta.motion.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, present)| (!present).then_some(name))
    .collect();
    match missing.is_empty() {
        true => Ok(()),
        false => Err(format!("a file's delta carries all five sections and this one has no {}", missing.join(", "))),
    }
}

fn frame_error(tag: Tag, offset: u64, error: impl Display) -> String {
    format!("the {tag} frame at {offset}: {error}")
}

/// The frame a recording's world document measures from, empty when it names
/// none: a flagged export's one frame that states `unit_mm`, or an authored
/// document's `primary_frame`; a document marked both ways, and a flagged
/// export without all three of its flags, are refused.
fn primary_frame_of(world: &Value) -> Result<String, String> {
    let here = Path::new("<recording>");
    let unreadable = |e: LoadError| format!("the recording's world document does not read: {e}");
    if !flagged_export::is_flagged_export(here, world).map_err(unreadable)? {
        return Ok(world.get("primary_frame").and_then(Value::as_str).unwrap_or_default().to_owned());
    }
    let export = FlaggedExport::read(here, world).map_err(unreadable)?;
    Ok(export.primary_frame().map_err(unreadable)?.map(|frame| frame.to_string()).unwrap_or_default())
}

/// The running state a walk of the frames leaves behind: where everything
/// stands right now, and the slices, journal rows and counters assembled so
/// far.
struct Build {
    /// Where everything stands at the start of the newest slice taken, which
    /// `pending` holds until the next frame says where the slice ends.
    state: RunState,
    pending: Option<(SimTime, Delta)>,
    slices: Vec<SliceRecord>,
    /// The ids designated ORDER, which the run states once, at its start.
    order_totes: Vec<u32>,
    /// Each table's rows, spliced in the order the file states them.
    rows: Vec<(TableId, TableRows)>,
    table_names: FxHashMap<TableId, String>,
    /// Each journal's own descriptor, from the producer document, so a stream
    /// comes back describing itself as it went in.
    streams: FxHashMap<String, StreamDescriptor>,
    producer: ProducerDocument,
    world: Value,
    sim_config: SimConfig,
    /// The run's cell counters, summed from the DELTAS alone.
    ///
    /// A keyframe states the same totals, and taking them from there would
    /// make the answer depend on which keyframe was read last — a file with a
    /// keyframe that disagrees would be read two ways by two callers. Summing
    /// the deltas is the one reading, and each keyframe is checked against
    /// it, so a disagreement is named instead of chosen between.
    raised: DynamicGrid,
}

impl Build {
    fn new(scenario: &Scenario) -> Result<Self, String> {
        // A recording a server filtered for a browser states nil here; it is
        // not a file this reader can rebuild a run model from, and saying so
        // is what tells a caller they were handed a wire capture.
        let stated = scenario
            .producer
            .as_ref()
            .ok_or("this recording states no producer document; it was filtered for a browser")?;
        let producer: ProducerDocument = read_document(stated, "producer document")?;
        let world: Value = serde_json::from_slice(&scenario.world.0)
            .map_err(|e| format!("the recording's world document is not JSON: {e}"))?;
        // The block names the frame every position in the file is measured in
        // and the document beside it names the frame it measures from. A file
        // that says two different things has no reading: the millimetres in it
        // belong to one frame or the other, and nothing here can say which.
        let declared = primary_frame_of(&world)?;
        if scenario.frame != declared {
            return Err(format!(
                "this recording states its positions in the frame {}, and the world document it carries declares \
                 {} as its primary frame",
                scenario.frame,
                match declared.is_empty() {
                    true => "no primary frame",
                    false => &declared,
                }
            ));
        }
        let sim_config: SimConfig = read_document(&scenario.sim_config, "simulation config")?;
        Ok(Self {
            state: RunState::default(),
            pending: None,
            slices: Vec::new(),
            order_totes: Vec::new(),
            rows: Vec::new(),
            table_names: scenario.tables.iter().map(|table| (table.id, table.name.clone())).collect(),
            streams: producer.tables.iter().map(|stream| (stream.id.clone(), stream.clone())).collect(),
            producer,
            world,
            sim_config,
            raised: DynamicGrid::new(),
        })
    }

    /// A keyframe states everything outright, so the state starts again from
    /// it. The first one also states which containers are order containers,
    /// which the run model keeps once for the whole run.
    fn take_keyframe(&mut self, at: SimTime, keyframe: &Keyframe, descriptors: &Descriptors) -> Result<(), String> {
        if let Some(heat) = &keyframe.heat {
            self.agrees_with_the_deltas(heat)?;
        }
        self.state = RunState::from_keyframe(at, keyframe, descriptors).map_err(|e| e.to_string())?;
        if self.order_totes.is_empty() && self.slices.is_empty() {
            let mut order: Vec<u32> = self
                .state
                .playback
                .totes
                .iter()
                .filter(|(_, tote)| tote.designation == 1)
                .map(|(tote, _)| tote.0)
                .collect();
            order.sort_unstable();
            self.order_totes = order;
        }
        Ok(())
    }

    /// A keyframe states the counters the deltas before it have raised. One
    /// that says otherwise is damage: the file would then carry two readings
    /// of the same run, and a viewer that seeked and one that replayed would
    /// draw different heat.
    fn agrees_with_the_deltas(&self, heat: &HeatKeyframe) -> Result<(), String> {
        for layer in &heat.layers {
            let raised = self.raised.get(&layer.name);
            for (cell, stated) in &layer.counts {
                let summed = raised.and_then(|counts| counts.get(&cell.0)).copied().unwrap_or(0);
                if *stated != summed {
                    return Err(format!(
                        "the keyframe says cell {} has been counted {stated} times in layer {}, and the deltas \
                         before it raise it {summed}",
                        cell.0, layer.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Take the slice the next frame, at `ends`, closes: the state moves to
    /// where it ends, and its robot spans must lie across it exactly.
    fn close_pending(&mut self, ends: SimTime) -> Result<(), String> {
        let Some((at, delta)) = self.pending.take() else { return Ok(()) };
        self.state.apply(at, ends, &delta).map_err(|e| format!("taking in the slice at {at}: {e}"))
    }

    /// One slice, as the run model records it: every robot's position at the
    /// slice's start with the steps it took inside it, the relay cells held at
    /// the start with the changes inside, the tote records, the metrics the
    /// slice stated, and what it added to each cell's counters. The state
    /// takes the slice in once the next frame says where it ends.
    fn take_delta(&mut self, at: SimTime, delta: Delta, descriptors: &Descriptors) -> Result<(), String> {
        let playback = delta.playback.clone().unwrap_or_default();
        let motion = delta.motion.clone().unwrap_or_default();
        let first = self.slices.is_empty();
        // The headings this slice begins with are true from its first instant,
        // so the state takes them before anything inside the slice is read.
        self.state.enter_slice(at, &delta).map_err(|e| format!("entering the slice at {at}: {e}"))?;

        let record = SliceRecord {
            start_ms: at.as_ms(),
            metrics: metrics_of(&delta, descriptors),
            grid: heat_of(&delta),
            bot_locations: self.bot_locations(&playback, &motion)?,
            relay_state: RelayStateRecord {
                start_reserved: sorted(self.state.playback.reserved_relay_cells.iter().map(|cell| cell.0)),
                transitions: playback
                    .relay_transitions
                    .iter()
                    .map(|transition| (transition.cell.0, transition.offset.as_ms(), transition.reserved))
                    .collect(),
            },
            tote_state: self.tote_state(&playback, first)?,
            station_diag: match playback.station_display.is_empty() {
                true => StationDiagRecord::default(),
                false => read_document(&playback.station_display, "station diagnostics")?,
            },
        };
        self.slices.push(record);

        for rows in delta.tables.iter().flat_map(|tables| tables.0.iter()) {
            self.splice(rows)?;
        }
        for (layer, counts) in heat_of(&delta) {
            let into = self.raised.entry(layer).or_default();
            for (cell, count) in counts {
                *into.entry(cell).or_insert(0) += count;
            }
        }
        self.pending = Some((at, delta));
        Ok(())
    }

    /// Every robot in the run, whether or not it moved: the ones the slice
    /// names carry its steps, the rest stand where the state has them with no
    /// steps at all.
    fn bot_locations(
        &self,
        playback: &PlaybackDelta,
        motion: &MotionDelta,
    ) -> Result<FxHashMap<u32, BotLocation>, String> {
        let movers: FxHashMap<RobotId, &MoverSteps> =
            playback.movers.iter().map(|mover| (mover.robot, mover)).collect();
        let mut locations = FxHashMap::default();
        for (&robot, state) in &self.state.playback.robots {
            let spans = motion.get(robot).map(|record| spans_of(&record.spans)).unwrap_or_default();
            locations.insert(
                robot.0,
                BotLocation {
                    motion: Some(spans),
                    start_cell: state.cell.0,
                    start_pos: floor_plane(state.at),
                    start_z_mm: f64::from(state.at.z_mm),
                    start_toward: state.toward.map(|cell| cell.0),
                    start_heading_rad: state.heading_mrad.map(radians),
                    robot_type: RobotType::from(state.class),
                    steps: steps(movers.get(&robot).copied()),
                },
            );
        }
        Ok(locations)
    }

    fn tote_state(&self, playback: &PlaybackDelta, first: bool) -> Result<ToteStateRecord, String> {
        Ok(ToteStateRecord {
            creations: playback
                .tote_creations
                .iter()
                .map(|birth| {
                    Ok(ToteCreationRecord {
                        offset_ms: birth.offset.as_ms(),
                        tote_id: birth.tote.0,
                        tote_type: tote_type_of(birth.tote_type)
                            .map_err(|e| format!("tote {}'s birth: {e}", birth.tote.0))?,
                        designation: designation_of(birth.designation)
                            .map_err(|e| format!("tote {}'s birth: {e}", birth.tote.0))?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
            contents: playback
                .tote_contents
                .iter()
                .map(|contents| (contents.offset.as_ms(), contents.tote.0, contents.units))
                .collect(),
            transits: playback
                .tote_transits
                .iter()
                .map(|transit| ToteTransitRecord {
                    tote_id: transit.tote.0,
                    from_cell: transit.from_cell.0,
                    to_cell: transit.to_cell.0,
                    start_ms: transit.start.as_ms(),
                    end_ms: transit.end.as_ms(),
                })
                .collect(),
            start_states: if first { self.start_states() } else { Vec::new() },
            transitions: playback
                .tote_transitions
                .iter()
                .map(|transition| (transition.offset.as_ms(), transition.tote.0, visual_state(transition.location)))
                .collect(),
            order_totes: if first { self.order_totes.clone() } else { Vec::new() },
            carries_previous: !first,
        })
    }

    fn start_states(&self) -> Vec<(u32, ToteVisualState)> {
        let mut states: Vec<(u32, ToteVisualState)> =
            self.state.playback.totes.iter().map(|(tote, state)| (tote.0, visual_state(state.location))).collect();
        states.sort_unstable_by_key(|(tote, _)| *tote);
        states
    }

    fn splice(&mut self, rows: &TableRows) -> Result<(), String> {
        match self.rows.iter_mut().find(|(table, _)| *table == rows.table) {
            Some((_, held)) => held.splice(rows).map_err(|e| e.to_string()),
            None => {
                self.rows.push((rows.table, rows.clone()));
                Ok(())
            }
        }
    }

    /// Every table's rows as the streams the run model keeps them in.
    fn streams(&self) -> Result<Vec<LogStream>, String> {
        self.rows
            .iter()
            .map(|(table, rows)| {
                let name = self
                    .table_names
                    .get(table)
                    .ok_or_else(|| format!("the file carries rows for table {}, which nothing declares", table.0))?;
                let descriptor = self
                    .streams
                    .get(name)
                    .ok_or_else(|| format!("table {name} has no descriptor in the producer document"))?
                    .clone();
                Ok(LogStream {
                    descriptor,
                    entries: Entries {
                        encoding: EntryEncoding::RowsV1,
                        entry_count: u64::from(rows.row_count),
                        bytes: rows.rows.clone(),
                    },
                })
            })
            .collect()
    }

    fn into_recording(
        mut self,
        scenario: &Scenario,
        end: Option<End>,
        descriptors: &Descriptors,
    ) -> Result<Recording, String> {
        let mut streams = self.streams()?;
        let mut summary = SummaryStats::default();
        let mut sim_result = SimResult::default();
        let mut cumulative_scalars = HashMap::new();
        let truncated = end.is_none();
        if let Some(end) = &end {
            summary = read_document(&end.summary, "summary statistics")?;
            sim_result = read_document(&end.result, "simulation result")?;
            // A slot past the descriptors has no metric to name it, so its
            // number means nothing and dropping it would hide the damage.
            if end.cumulative_scalars.len() > descriptors.metrics.len() {
                return Err(format!(
                    "the end block states {} metric values and the run declared {}",
                    end.cumulative_scalars.len(),
                    descriptors.metrics.len(),
                ));
            }
            cumulative_scalars = end
                .cumulative_scalars
                .iter()
                .enumerate()
                .filter_map(|(slot, value)| Some((descriptors.metrics.get(slot)?.name.clone(), (*value)?)))
                .collect();
            for rows in &end.tables {
                self.splice(rows)?;
            }
            streams = self.streams()?;
        }
        take_journals(streams, &mut summary).map_err(|e| e.to_string())?;

        Ok(Recording {
            profile: PerfProfile {
                name: self.producer.name.clone(),
                metadata: ProfileMetadata {
                    branch: self.producer.branch.clone(),
                    commit: scenario.commit.clone(),
                    commit_short: self.producer.commit_short.clone(),
                    dirty: scenario.dirty,
                    timestamp: scenario.timestamp.clone(),
                    hostname: scenario.hostname.clone(),
                    grid_width: self.producer.grid_width,
                    grid_depth: self.producer.grid_depth,
                    slice_width_ms: scenario.slice_width.as_ms(),
                },
                world: self.world,
                documents: RunDocuments {
                    spec: scenario.scenario.0.clone(),
                    world: scenario.world.0.clone(),
                    data: scenario.data.iter().flatten().map(|(name, doc)| (name.clone(), doc.0.clone())).collect(),
                },
                ids: IdTables {
                    frame: scenario.frame.clone(),
                    cells: scenario.cells.clone(),
                    stations: scenario.stations.clone(),
                    robots: roster(scenario),
                },
                sim_config: self.sim_config,
                scenario: self.producer.scenario_source,
                workload: self.producer.workload,
                vtm_fleet: self.producer.vtm_fleet,
                summary,
                sim_result,
                time_slices: self.slices,
                cumulative_grid: self.raised,
                cumulative_scalars,
            },
            truncated,
        })
    }
}

/// The robots the scenario block says the run holds, as the run model names
/// them.
fn roster(scenario: &Scenario) -> Vec<RobotIdentity> {
    scenario
        .robots
        .iter()
        .map(|robot| RobotIdentity {
            robot: robot.robot,
            name: robot.name.clone(),
            class: robot.class.into(),
            class_entity: robot.class_entity.clone(),
        })
        .collect()
}

/// Exactly the metrics a slice stated: a nil slot is a metric the run said
/// nothing about in that slice, which is not the same as zero.
fn metrics_of(delta: &Delta, descriptors: &Descriptors) -> HashMap<String, f64> {
    let Some(scalars) = &delta.scalars else { return HashMap::new() };
    scalars
        .values
        .iter()
        .enumerate()
        .filter_map(|(slot, value)| Some((descriptors.metrics.get(slot)?.name.clone(), (*value)?)))
        .collect()
}

/// What one slice added to each cell's counters, as the run model's layers.
fn heat_of(delta: &Delta) -> DynamicGrid {
    let Some(heat) = &delta.heat else { return DynamicGrid::new() };
    heat.layers
        .iter()
        .map(|layer| (layer.name.clone(), layer.increments.iter().map(|(cell, count)| (cell.0, *count)).collect()))
        .collect()
}

fn spans_of(spans: &[SpanRecord]) -> Vec<MotionSpan> {
    spans
        .iter()
        .map(|span| MotionSpan {
            source: span.source.0,
            destination: span.destination.0,
            start_ms: span.start.as_ms(),
            end_ms: span.end.as_ms(),
            distance_mm: span.distance_mm,
            profile: span.profile.map(|p| TrapezoidalProfile {
                entry_velocity_mm_s: p.entry_velocity_mm_s,
                exit_velocity_mm_s: p.exit_velocity_mm_s,
                peak_velocity_mm_s: p.peak_velocity_mm_s,
                accel_time_s: p.accel_time_s,
                cruise_time_s: p.cruise_time_s,
                decel_time_s: p.decel_time_s,
                accel_distance_mm: p.accel_distance_mm,
                cruise_distance_mm: p.cruise_distance_mm,
                decel_distance_mm: p.decel_distance_mm,
                total_time_s: p.total_time_s,
                total_distance_mm: p.total_distance_mm,
                acceleration_mm_s2: p.acceleration_mm_s2,
                deceleration_mm_s2: p.deceleration_mm_s2,
            }),
        })
        .collect()
}

fn sorted(ids: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut ids: Vec<u32> = ids.collect();
    ids.sort_unstable();
    ids
}

/// Put the journal rows a file carried back where the run model keeps them,
/// and leave the rest as the streams the summary carries.
fn take_journals(streams: Vec<LogStream>, summary: &mut SummaryStats) -> Result<(), StreamError> {
    let mut untyped = Vec::new();
    for stream in streams {
        match stream.descriptor.id.as_str() {
            journals::HTM_LEGS => summary.htm_leg_log = journals::read_htm_legs(&stream)?,
            journals::VTM_LEGS => summary.vtm_leg_log = journals::read_vtm_legs(&stream)?,
            journals::VTM_CYCLES => summary.vtm_cycle_log = journals::read_vtm_cycles(&stream)?,
            journals::VTM_OPERATIONS => summary.vtm_operation_log = journals::read_vtm_operations(&stream)?,
            journals::PICKS => summary.pick_log = journals::read_picks(&stream)?,
            journals::PUTAWAYS => summary.putaway_log = journals::read_putaways(&stream)?,
            journals::COMPLETED_ORDERS => summary.completed_order_log = journals::read_completed_orders(&stream)?,
            journals::AISLE_STATES => summary.aisle_state_log = journals::read_aisle_states(&stream)?,
            journals::AISLE_FLUX => summary.aisle_flux_log = journals::read_aisle_flux(&stream)?,
            _ => untyped.push(stream),
        }
    }
    summary.log_streams = untyped;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::primary_frame_of;

    /// A recording's world document is read by the reader its keys name: an authored document's primary frame is the
    /// one its prologue names, a flagged export's the frame that states `unit_mm`; one carrying a flagged export's
    /// `scope` beside an authored prologue is refused rather than read either way, and so is a flagged export without
    /// all three of its flags.
    #[test]
    fn a_recordings_world_is_read_by_its_keys() {
        let frame = "f0000000-0000-4000-8000-000000000001";
        let doc = json!({ "id": "d0000000-0000-4000-8000-000000000001" });
        let authored = json!({ "proto_version": "0.7.0", "doc": doc, "primary_frame": frame, "entities": [] });
        assert_eq!(primary_frame_of(&authored).as_deref(), Ok(frame));
        let unit = json!({ "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } });
        let row = json!({ "components": unit, "id": frame, "scope": "customer/c/site/s" });
        let mut export = json!({
            "derived": true,
            "entities": [row],
            "nested": true,
            "referenced": true,
            "scope": "customer/c/site/s",
            "scopes": [{ "changeset": 1, "scope": "customer/c/site/s", "whole": true }],
        });
        assert_eq!(primary_frame_of(&export).as_deref(), Ok(frame));
        let mut both = authored;
        both["scope"] = json!("customer/c/site/s");
        let refused = primary_frame_of(&both).expect_err("a document marked both ways");
        assert!(refused.contains("`scope`") && refused.contains("`proto_version`"), "{refused}");
        export["referenced"] = json!(false);
        let refused = primary_frame_of(&export).expect_err("a flagged export without `referenced`");
        assert!(refused.contains("`referenced`"), "{refused}");
    }
}
