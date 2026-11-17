//! Simulation observer — event fanout.
//!
//! The observer runs on its own thread, consuming [`SimEvent`]s and fanning
//! them out to independent consumers:
//!
//! - **State/metrics** — cumulative and per-slice counters, bot positions
//! - **Watchdog** — stall detection (separate struct, notified of progress)
//! - **Final slices** — each slice once, final, as B passes it, to the run's
//!   recording, which is what reads a live run, and to the progress figures a
//!   caller polls
//!
//! Idle is derived, never received: a robot executes scheduled tasks, each
//! with an extent, and idle is the gap between them. `activity` builds the
//! intervals from the milestones the stream reports, so every gap between
//! them, the trailing one included, is idle by construction.

pub mod activity;
mod completions;
pub mod counters_v2;
pub mod digest;
pub mod duty;
pub mod finality;
mod handlers;
mod nav_handler;
mod nav_kinematic;
pub mod output;
pub use completions::FlowCompletions;

mod resource_handler;
pub mod robot_state;
mod state;
pub mod station_diag;
mod station_diag_handler;
pub mod time_slice;
pub mod traffic;
mod vtm_legs;
pub mod watchdog;

#[cfg(test)]
mod test_models;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_barrier_rule;
#[cfg(test)]
mod tests_duty;
#[cfg(test)]
mod tests_embargo;
#[cfg(test)]
mod tests_hold_journal;
#[cfg(test)]
mod tests_htm_kpi_duty;
#[cfg(test)]
mod tests_kinematic_rasterization;
#[cfg(test)]
mod tests_navigation_commit;

use std::sync::{Arc, mpsc};

use desgraph::CellId;
use thiserror::Error;

use crate::SimTime;
use crate::events::SimEvent;
use crate::model::RobotId;
use crate::observer::activity::ActivityViolation;
use crate::observer::digest::{EventDigest, kind_name};
use crate::observer::finality::recording::SharedRecorder;
use crate::observer::output::MetricsOutput;
use crate::observer::robot_state::CommittedSpansOverlap;
use crate::observer::time_slice::MetricsConfig;
use crate::observer::watchdog::Watchdog;
use crate::sim_types::Counters;
use crate::sim_types::{StopReason, StopSignal};

use state::SimObserverState;

// ---------------------------------------------------------------------------
// SimObserver
// ---------------------------------------------------------------------------

/// Event fanout: consumes SimEvents and distributes to consumers.
///
/// Constructed with the receiver end of the event channel, then `run()` blocks
/// until the channel closes (all senders dropped). Returns [`MetricsOutput`].
pub struct SimObserver {
    config: MetricsConfig,
    rx: mpsc::Receiver<SimEvent>,
    recorder: Option<SharedRecorder>,
    watchdog: Option<Watchdog>,
    counters: Option<Counters>,
    /// The run's stop signal, raised when the facts break a rule of the event
    /// stream (`StreamViolation`) or the run's recording fails.
    failure_stop: Option<Arc<StopSignal>>,
    /// Event subscribers: each receives a clone of every event.
    subscribers: Vec<mpsc::Sender<SimEvent>>,
    /// Print each event to stderr inline (no separate thread).
    print_events: bool,
}

impl SimObserver {
    /// Create a new simulation observer.
    pub fn new(config: MetricsConfig, rx: mpsc::Receiver<SimEvent>) -> Self {
        Self {
            config,
            rx,
            recorder: None,
            watchdog: None,
            counters: None,
            failure_stop: None,
            subscribers: Vec::new(),
            print_events: false,
        }
    }

    /// Hand each slice to the run's recording as it is made final.
    pub fn with_recorder(mut self, recorder: SharedRecorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Show the run's progress in `counters` as each slice is made final.
    pub fn with_counters(mut self, counters: Counters) -> Self {
        self.counters = Some(counters);
        self
    }

    /// Subscribe to the event stream. The subscriber receives a clone
    /// of every event as the observer processes it. Can be called multiple
    /// times for multiple subscribers.
    pub fn subscribe_events(mut self, tx: mpsc::Sender<SimEvent>) -> Self {
        self.subscribers.push(tx);
        self
    }

    /// Print each event to stderr as processed (inline, no separate thread).
    pub fn with_print_events(mut self, enabled: bool) -> Self {
        self.print_events = enabled;
        self
    }

    /// Attach a watchdog for stall detection.
    pub fn with_watchdog(mut self, watchdog: Watchdog) -> Self {
        self.watchdog = Some(watchdog);
        self
    }

    /// Stop the run through `signal` as soon as the facts break a rule of the
    /// event stream, or the run's recording cannot take a slice, rather than
    /// simulate on into output nothing can be read from.
    pub fn with_failure_stop(mut self, signal: Arc<StopSignal>) -> Self {
        self.failure_stop = Some(signal);
        self
    }

    /// Run the event loop until the channel closes. Returns final output, or
    /// the first rule of the stream the facts broke.
    ///
    /// Each slice goes out once, final, when the observer reads the advance of
    /// B that passes the slice's end; the slices B never passed go out when
    /// the stream closes.
    pub fn run(mut self) -> Result<MetricsOutput, StreamViolation> {
        let mut state = SimObserverState::new(self.config);
        if let Some(recorder) = self.recorder {
            state.record_into(recorder);
        }
        if let Some(counters) = &self.counters {
            state.show_progress_in(counters.clone());
        }
        // T=0 aisle baseline through the sole producer (see stamp_all_aisles).
        state.stamp_all_aisles(0);
        let mut digest = EventDigest::from_env();
        let failure_stop = self.failure_stop.clone();
        let stop = |violation| stop_run(failure_stop.as_deref(), violation);

        for event in self.rx.iter() {
            // The scheduler's frontier marker is not a warehouse fact: it
            // moves the current world time on, settles the work behind it and
            // makes final the slices it passes, and nothing prints, forwards,
            // digests or counts it.
            if let SimEvent::BarrierAdvanced(world_time) = event {
                state.settle_activity(world_time).map_err(|violation| stop(violation.into()))?;
                check_committed_spans(&mut state).map_err(stop)?;
                state.finalize_passed_slices(world_time);
                check_late_write(&mut state, "BarrierAdvanced").map_err(stop)?;
                stop_on_failed_recording(&mut state, failure_stop.as_deref());
                continue;
            }
            // The marker travels in the same stream as the facts, so a fact
            // stamped behind the last B read here was reported after B had
            // passed it.
            let barrier = state.current_world_time();
            if let Some(stamp) = event.stamp().filter(|stamp| *stamp < barrier) {
                return Err(stop(StreamViolation::BehindBarrier { fact: kind_name(&event), stamp, barrier }));
            }
            if self.print_events {
                eprintln!("{}", event);
            }
            for sub in &self.subscribers {
                let _ = sub.send(event.clone());
            }
            digest.record(&event);
            state.total_events += 1;
            state.dispatch_event(&event);
            if let Some(violation) = state.take_contradicting_hold() {
                return Err(stop(violation));
            }
            check_late_write(&mut state, kind_name(&event)).map_err(stop)?;

            // Fan out to watchdog.
            if let Some(ref mut wd) = self.watchdog {
                if let SimEvent::CycleComplete(ref data) = event {
                    wd.notify_progress(data.sim_time_ms);
                }
                wd.check(state.latest_sim_time.as_ms());
            }
        }

        // B had reached the current world time, so the run did too, even with
        // no fact stamped that late: a visit the run stopped inside stands
        // until it.
        state.latest_sim_time = state.latest_sim_time.max(state.current_world_time());
        // The interval pass first: it is what duty, the per-slice
        // distribution and the journal's no-task rows are read from, and it
        // needs the run's final instant to close the trailing gap.
        state.build_activity_intervals()?;
        check_committed_spans(&mut state).map_err(stop)?;
        // The run's stop, where its summary ends it: its simtime limit when it
        // stopped on that, though work committed before the stop reaches past
        // it, and otherwise the latest instant it reached.
        let on_limit = self.failure_stop.as_deref().is_some_and(StopSignal::stopped_on_sim_time_limit);
        let run_stop = state.config.sim_time_limit.filter(|_| on_limit).unwrap_or(state.latest_sim_time);
        state.finalize_remaining_slices(run_stop);
        check_late_write(&mut state, "the stream's close").map_err(stop)?;
        // Rewrite Teleport VTM duty as the clamped union of geometric cycle
        // windows. After the last event, before `sim_elapsed_ms` is read.
        state.finalize_vtm_union_duty();
        state.finalize_station_gaps();

        state.output.event_digest = digest.summary();
        state.output.total_events = state.total_events;
        state.output.total_slices = state.slices().len();
        state.output.time_slices = state.finality.into_slices();
        state.output.sim_elapsed_ms = state.latest_sim_time.as_ms();
        // Per-aisle capacity is a scenario constant; surface it for the
        // "Rack" xlsx sheet's fill-% denominator. (Global capacity rides the
        // per-slice `rack_capacity` scalar instead.)
        state.output.rack_zone_capacity = (*state.config.zone_capacity).clone();
        Ok(state.output)
    }
}

/// Fail with the first two spans facts committed one robot to over one instant, if any did: read before B's advance
/// makes a slice final from them, and after the activity build, which names two overlapping tasks first.
fn check_committed_spans(state: &mut SimObserverState) -> Result<(), StreamViolation> {
    match state.robot_state_log.take_overlap() {
        Some(overlap) => Err(StreamViolation::CommittedSpansOverlap(overlap)),
        None => Ok(()),
    }
}

/// Fail with the write `writer` made into a slice B had passed, if it made one.
fn check_late_write(state: &mut SimObserverState, writer: &'static str) -> Result<(), StreamViolation> {
    match state.take_late_write() {
        Some(slice_start) => Err(StreamViolation::WriteIntoPassedSlice { writer, slice_start }),
        None => Ok(()),
    }
}

/// Stop the run if its recording could not take a slice B passed. The
/// recording is the run's output, so a run that cannot write it has nothing
/// left to run for; the facts already sent are still read, as for any stopped
/// run. A slice the stream's close makes final comes after the run has
/// stopped, and its failure is the recording's to return.
fn stop_on_failed_recording(state: &mut SimObserverState, signal: Option<&StopSignal>) {
    if let Some(failure) = state.take_recording_failure()
        && let Some(signal) = signal
    {
        signal.stop(StopReason::Error(failure.to_string()));
    }
}

/// A rule of the event stream that the facts the observer read break. Nothing
/// read from such a stream can be trusted, so the run stops on it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StreamViolation {
    /// A fact reported after B had passed its instant: it claims something
    /// happened in simtime the scheduler had already closed.
    #[error("{fact} stamped at {stamp} was reported after B had reached {barrier}")]
    BehindBarrier { fact: &'static str, stamp: SimTime, barrier: SimTime },
    /// Something the observer read wrote into a slice B had passed: the slice
    /// is final from then on, so a viewer holding it would never see the
    /// write.
    #[error("{writer} wrote into the slice from {slice_start} after B had passed it")]
    WriteIntoPassedSlice { writer: &'static str, slice_start: SimTime },
    /// Navigation held a relay's access cell from `from` while its open hold
    /// from `open_from` still held it: no cell is held twice at once.
    #[error("navigation held {cell:?} for {robot:?} from {from} while an open hold from {open_from} still held it")]
    CellHeldTwice { robot: RobotId, cell: CellId, from: SimTime, open_from: SimTime },
    /// Navigation ended an open hold of a relay's access cell that no open
    /// hold held.
    #[error("navigation released {robot:?}'s hold of {cell:?} at {at}, but no open hold held the cell")]
    ReleaseWithoutHold { robot: RobotId, cell: CellId, at: SimTime },
    /// Navigation held a cell over a span that ends before it begins.
    #[error("navigation held {cell:?} for {robot:?} from {from} until {until}, before the hold began")]
    HoldEndsBeforeItBegins { robot: RobotId, cell: CellId, from: SimTime, until: SimTime },
    /// Navigation ended the open hold of a relay's access cell before the
    /// hold began.
    #[error("navigation released {robot:?}'s hold of {cell:?} at {at}, before the hold began at {from}")]
    ReleaseBeforeHoldBegan { robot: RobotId, cell: CellId, from: SimTime, at: SimTime },
    /// Two facts committed one robot to two spans over one instant: a robot
    /// is in one state at a time, so the robot state channel cannot state
    /// both.
    #[error(
        "{:?} is committed to {:?} from {} to {} and to {:?} from {} to {}",
        .0.robot, .0.first.2, .0.first.0, .0.first.1, .0.second.2, .0.second.0, .0.second.1
    )]
    CommittedSpansOverlap(CommittedSpansOverlap),
    #[error(transparent)]
    Activity(#[from] ActivityViolation),
}

/// Stop the run on `violation` while the scheduler is still going, and hand
/// the violation on as the observer's result.
fn stop_run(signal: Option<&StopSignal>, violation: StreamViolation) -> StreamViolation {
    if let Some(signal) = signal {
        signal.stop(StopReason::Error(violation.to_string()));
    }
    violation
}
