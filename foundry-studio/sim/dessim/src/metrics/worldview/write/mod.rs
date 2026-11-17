//! Within `worldview`, `write` is THE RECORDING AS THE RUN GOES: each slice
//! the observer makes final becomes frames of one `.worldview` file, and the
//! run's summary closes it.
//!
//! **What it is for.** A run's numbers have to outlive the process that made
//! them, and every consumer downstream reads the same file: the viewer scrubs
//! it, the workbook tabulates it, `compare` reads two of them. So the file
//! has to state the whole run — where everything was at every instant, every
//! metric, every journal row, and the documents the run was built from — with
//! no reference to the machine that produced it.
//!
//! **How a slice is written.** The scenario block goes before the first
//! slice, once the observer knows the fleet (`scenario`). The producer holds
//! the running truth (`producer`): where every robot and tote stands, what
//! every robot is doing, which relay cells are held, how deep each station's
//! queue is, what every metric adds up to, what every cell has been counted
//! for. Every thirtieth slice, starting at slice 0, it states all of it as a
//! keyframe before that slice's delta, so a reader reaches any instant from
//! one keyframe and at most twenty-nine deltas. A slice's own records become
//! the delta's (`delta`). A metric the run first states in a later slice is
//! declared just before it.

mod delta;
mod error;
mod producer;
mod scenario;
mod tap;

use std::io::Write;
use std::path::Path;

use desunits::SimTime;
use worldview::container::error::Error as ContainerError;
use worldview::container::writer::{AppendSink, Writer, create_file};
use worldview::frames::declare::Declare;
use worldview::frames::end::End;
use worldview::frames::scalars::ScalarsDelta;
use worldview::frames::tables::{TableDescriptor, TablesDelta};
use worldview::frames::{Delta, WriteFrames};

use crate::metrics::profile::{ScalarClass, SummaryStats, classed_scalars, grid_to_dynamic};
use crate::metrics::streams::StreamDescriptor;
use crate::metrics::worldview::summary::{document, summary_document};
use crate::observer::finality::recording::{FinalSlice, RecordingFailed, SliceRecorder, SliceRows};
use crate::observer::output::MetricsOutput;
use crate::sim_types::{SimResult, StopReason};
use delta::{Reach, heat_delta, motion_delta, playback_delta, robot_state_delta, rows_of, table_rows};
use producer::Producer;
use scenario::{journal_tables, scenario_block};
use tap::Tapped;

pub use error::RecordingError;
pub use scenario::{ProducerDocument, RunScenario};

/// Slices between keyframes. The file states it in its scenario block and
/// every reader takes it from there, never from here.
pub const DEFAULT_KEYFRAME_CADENCE_SLICES: u32 = 30;

/// One run's recording, written as the run goes: the scenario block before
/// the first slice, a keyframe every thirtieth slice and a delta for each,
/// and the end block once the run's summary is known.
///
/// The recording is the run's output. A slice that cannot be written ends
/// it: the failure is kept for [`RunRecording::finish`] to return, and the
/// observer, told of it, stops the run.
pub struct RunRecording {
    writer: Writer<Box<dyn AppendSink + Send>>,
    /// What the scenario block states, until the first slice writes it.
    scenario: Option<Box<RunScenario>>,
    tables: Vec<TableDescriptor>,
    streams: Vec<StreamDescriptor>,
    producer: Producer,
    /// The journal rows the run reported after its last slice went out, for
    /// the end block to state.
    rows_left: SliceRows,
    slices_written: u32,
    /// Where the last slice written ends, which is where the end block is stamped: the run's stop.
    last_ends: Option<SimTime>,
    failed: Option<RecordingError>,
}

impl RunRecording {
    /// Open the recording at `path` for a run about to start, and hand
    /// `tap`, when there is one, every byte the file takes as it takes it.
    pub fn create(
        path: &Path,
        tap: Option<Box<dyn Write + Send>>,
        scenario: RunScenario,
    ) -> Result<Self, RecordingError> {
        let file = create_file(path).map_err(RecordingError::Open)?;
        let sink: Box<dyn AppendSink + Send> = match tap {
            Some(tap) => Box::new(Tapped::new(file, tap)),
            None => Box::new(file),
        };
        Self::new(sink, scenario)
    }

    /// Open a recording into `sink`, which holds nothing yet, for a run about
    /// to start.
    ///
    /// Every position in the recording is measured in the world document's
    /// primary frame and the scenario block is the one place that says which
    /// frame that is. A run that names none would be measured from nowhere,
    /// so it is refused here, before it runs.
    pub fn new(sink: Box<dyn AppendSink + Send>, scenario: RunScenario) -> Result<Self, RecordingError> {
        if scenario.ids.frame.is_empty() {
            return Err(RecordingError::NoPrimaryFrame);
        }
        let (tables, streams) = journal_tables()?;
        Ok(Self {
            writer: Writer::new(sink),
            scenario: Some(Box::new(scenario)),
            tables,
            streams,
            producer: Producer::new(),
            rows_left: SliceRows::default(),
            slices_written: 0,
            last_ends: None,
            failed: None,
        })
    }

    /// Write one slice the observer has made final: before the first, the
    /// scenario block; a declaration for each metric it is the first to
    /// state; a keyframe on the cadence; its delta, with the journal rows
    /// due with it.
    fn write_slice(&mut self, final_slice: FinalSlice<'_>) -> Result<(), RecordingError> {
        let FinalSlice { slice, next, ends, last, rows, robot_spans, roster } = final_slice;
        let at = SimTime::ms(slice.start_ms);
        let metrics = classed_scalars(&slice.scalars);
        let grid = grid_to_dynamic(&slice.grid);
        let unslotted = self.producer.unslotted(&metrics);
        match self.scenario.take() {
            Some(scenario) => {
                for (name, class) in unslotted {
                    self.producer.slot(name, class);
                }
                let block = scenario_block(&scenario, &self.producer, &self.tables, &self.streams, roster)?;
                self.writer.append_scenario_block(&block).map_err(written("the scenario block", at))?;
                self.producer.seed_reservations(slice);
            }
            None => self.declare(at, unslotted)?,
        }
        let headings = self.producer.take_headings(slice);
        self.producer.seed_from_slice(slice)?;
        let robot_state = robot_state_delta(&robot_spans);
        self.producer.enter_robot_states(at, &robot_state)?;
        if self.slices_written.is_multiple_of(DEFAULT_KEYFRAME_CADENCE_SLICES) {
            let keyframe = self.producer.keyframe(at)?;
            self.writer.append_keyframe(at, &keyframe).map_err(written("a keyframe", at))?;
        }
        let width_ms = (ends - at).as_ms();
        let reach = Reach { width_ms, through_its_end: last };
        let delta = Delta {
            playback: Some(playback_delta(slice, next, reach, headings)?),
            scalars: Some(ScalarsDelta { values: self.producer.slice_values(&metrics) }),
            heat: Some(heat_delta(&grid)),
            tables: Some(TablesDelta(table_rows(&rows, &self.tables)?)),
            motion: Some(motion_delta(&slice.bot_locations)),
            robot_state: Some(robot_state),
        };
        self.writer.append_delta(at, &delta).map_err(written("a delta", at))?;
        let playback = delta.playback.as_ref().expect("the delta this writer just built");
        self.producer.apply_slice(slice, &metrics, &grid, playback)?;
        let robot_state = delta.robot_state.as_ref().expect("the delta this writer just built");
        self.producer.leave_robot_states(robot_state);
        self.slices_written += 1;
        self.last_ends = Some(ends);
        Ok(())
    }

    /// Declare each metric in `unslotted`, in order, before the slice at `at`.
    fn declare(&mut self, at: SimTime, unslotted: Vec<(String, ScalarClass)>) -> Result<(), RecordingError> {
        for (name, class) in unslotted {
            let declared = Declare::Metric(self.producer.slot(name, class).clone());
            self.writer.append_declare(at, &declared).map_err(written("a metric's declaration", at))?;
        }
        Ok(())
    }

    /// Close the recording with the run's end, stamped at the run's stop: its
    /// result, its summary, every metric's final value, the run-level tables
    /// and the journal rows the run reported after its last slice went out.
    /// `output` is what the observer returned and `summary` what the run's
    /// journals moved into. A recording a slice could not be written into
    /// returns that failure, and so does one whose last slice does not end at
    /// the stop, since a reader ends that slice at the end block.
    ///
    /// The final values are the run's totals, which the observer carries from
    /// the slices this recording states: each figure the run adds up is the
    /// sum of those slices, and each level the last one's reading.
    pub fn finish(
        mut self,
        output: &MetricsOutput,
        summary: &mut SummaryStats,
        result: &SimResult,
    ) -> Result<(), RecordingError> {
        if let Some(failure) = self.failed {
            return Err(failure);
        }
        let cumulative = classed_scalars(&output.cumulative_scalars);
        let unslotted = self.producer.unslotted(&cumulative);
        let at = SimTime::ms((summary.sim_time_secs * 1000.0).round().max(0.0) as u64);
        if let Some(last_ends) = self.last_ends.filter(|last_ends| *last_ends != at) {
            return Err(RecordingError::EndOffTheLastSlice { stop: at, last_ends });
        }
        match self.scenario.take() {
            Some(scenario) => {
                for (name, class) in unslotted {
                    self.producer.slot(name, class);
                }
                let block =
                    scenario_block(&scenario, &self.producer, &self.tables, &self.streams, &output.robot_types)?;
                self.writer.append_scenario_block(&block).map_err(written("the scenario block", at))?;
            }
            None => self.declare(at, unslotted)?,
        }
        let mut tables = table_rows(&self.rows_left, &self.tables)?;
        for stream in &summary.log_streams {
            tables.push(rows_of(stream, &self.tables)?);
        }
        // The end block counts the frames the producer made, its own included;
        // the index block after it is the container's bookkeeping.
        let frames_written = self.writer.frames_written() + 1;
        let end = End {
            stop_reason: format!("{:?}", result.stop_reason),
            stop_detail: stop_detail(&result.stop_reason),
            result: document(result, "simulation result")?,
            summary: summary_document(summary)?,
            cumulative_scalars: self.producer.slice_values(&cumulative),
            tables,
            frames_written,
        };
        self.writer.finish_with(at, &end).map_err(written("the end block", at))?;
        Ok(())
    }
}

impl SliceRecorder for RunRecording {
    /// The first slice that cannot be written ends the recording; the
    /// observer hands it no more.
    fn record(&mut self, slice: FinalSlice<'_>) -> Result<(), RecordingFailed> {
        self.write_slice(slice).map_err(|failure| {
            let told = RecordingFailed(failure.to_string());
            self.failed = Some(failure);
            told
        })
    }

    fn keep_rows_left(&mut self, rows: SliceRows) {
        self.rows_left = rows;
    }
}

/// A sink's refusal of `frame`, stamped `at`, as the recording's failure.
fn written(frame: &'static str, at: SimTime) -> impl FnOnce(ContainerError) -> RecordingError {
    move |source| RecordingError::Write { frame, at, source }
}

/// What a stop reason points at, when it points at something: the watchdog's
/// complaint and the solver's error carry the only text a run leaves about
/// why it ended, and dropping it would leave a reader with the word
/// `Watchdog` and nothing else.
fn stop_detail(stop_reason: &StopReason) -> Option<String> {
    match stop_reason {
        StopReason::Watchdog(detail) | StopReason::Error(detail) => Some(detail.clone()),
        StopReason::Complete | StopReason::UserStop | StopReason::WallTimeout | StopReason::SimTimeLimit => None,
    }
}
