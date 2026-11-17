//! Within `observer`, `finality` holds the run's slices, makes each final once
//! B has passed its end, sends it once, and stops the run on a fact's write
//! into a slice B has passed. Making a slice final folds in its cell-visit
//! share, stamps its stock (`physical_inventory`), leaves out of the slice
//! the run's stop cuts what it files past the stop, makes it drawable
//! (`drawable`), times it and gives it its relay busy time (`relay_busy`),
//! then hands it to the run's recording (`recording`) with the journal rows
//! due with it (`rows_due`) and every robot's spans across it
//! (`robot_state`), the one way a live run's slices leave the
//! observer, carries it onto the run's totals (`run_totals`), and adds it to
//! the progress figures a caller polls. The slices B
//! never passed are made final the same way when the stream closes, up to the
//! run's stop: the last is cut there or ends there, takes in the facts filed
//! exactly at the stop (`at_the_stop`), and none after it is sent.

mod at_the_stop;
mod drawable;
pub(super) mod physical_inventory;
pub mod recording;
mod relay_busy;
mod rows_due;
mod run_totals;

use std::collections::VecDeque;
use std::time::Instant;

use rustc_hash::FxHashMap;

use crate::SimTime;
use crate::events::{CellHeldEvent, CellReleasedEvent};
use crate::sim_types::Counters;

use super::StreamViolation;
use super::state::SimObserverState;
use super::time_slice::{GridCounters, MetricsConfig, TimeSliceData, passed_slice_end, passed_slice_start};
use recording::{FinalSlice, RecordingFailed, SharedRecorder};
use relay_busy::RelayBusy;
use rows_due::{Due, RowsDue, drop_rows_filed_after};

/// The run's slices, which of them are final, and what making the next ones
/// final reads.
pub(super) struct Finality {
    /// The run's slices, in order. A fact writes into one through
    /// `slice_to_write`, and a step that makes one final through
    /// `slice_to_finalize`.
    slices: Vec<TimeSliceData>,
    /// Slices before this one are final and sent.
    final_upto: usize,
    /// The wall-clock instant B passed each slice boundary it has passed from
    /// the start of slice `final_upto` on, in order.
    boundaries_passed: VecDeque<Instant>,
    /// The next slice boundary B has not passed.
    next_boundary: usize,
    /// Navigation's holds on the relays' access cells, which give each slice
    /// its relay busy time.
    relay_busy: RelayBusy,
    /// The first slice a fact wrote into after B had passed it.
    late_write: Option<usize>,
    /// The facts filed exactly at the run's simtime limit when that is a slice boundary, until the stream closes
    /// (`at_the_stop`).
    at_the_limit: Option<TimeSliceData>,
    /// The run's recording, which takes each slice as it is made final, until
    /// one cannot be written.
    recorder: Option<SharedRecorder>,
    /// Which journal rows the recording has taken, and which wait to be due.
    rows_due: RowsDue,
    /// The recording's failure to take a slice, until the observer reads it.
    recording_failed: Option<RecordingFailed>,
    /// The progress figures a caller polls, which show each slice once it is
    /// final.
    progress: Option<Counters>,
}

impl Finality {
    pub(super) fn new(config: &MetricsConfig) -> Self {
        Self {
            slices: Vec::new(),
            final_upto: 0,
            boundaries_passed: VecDeque::new(),
            next_boundary: 0,
            relay_busy: RelayBusy::new(config),
            late_write: None,
            at_the_limit: None,
            recorder: None,
            rows_due: RowsDue::default(),
            recording_failed: None,
            progress: None,
        }
    }

    pub(super) fn push_slice(&mut self, slice: TimeSliceData) {
        self.slices.push(slice);
    }

    /// The run's slices, once the stream has closed and every one is final.
    pub(super) fn into_slices(self) -> Vec<TimeSliceData> {
        self.slices
    }
}

impl SimObserverState {
    pub(super) fn slices(&self) -> &[TimeSliceData] {
        &self.finality.slices
    }

    /// Hand each slice to `recorder` as it is made final.
    pub(super) fn record_into(&mut self, recorder: SharedRecorder) {
        self.finality.recorder = Some(recorder);
    }

    /// Show each slice in `counters` as it is made final.
    pub(super) fn show_progress_in(&mut self, counters: Counters) {
        self.finality.progress = Some(counters);
    }

    /// Slice `index`, made to exist with every slice before it, for a fact to
    /// write into. A slice B has passed is final, so a write into it is noted
    /// and stops the run.
    pub(super) fn slice_to_write(&mut self, index: usize) -> &mut TimeSliceData {
        self.note_write_into(index);
        self.extend_slices_through(index);
        &mut self.finality.slices[index]
    }

    /// Slice `index`, for a step that makes it final to write into: what B's
    /// passing derives into it, and its finalization. Each such write lands
    /// before the slice is final; one into a final slice is an observer bug.
    pub(super) fn slice_to_finalize(&mut self, index: usize) -> &mut TimeSliceData {
        assert!(index >= self.finality.final_upto, "slice {index} is final and takes no more writes");
        &mut self.finality.slices[index]
    }

    /// The cell-visit counters a fact books against slice `index`, which fold
    /// into the slice when it is made final.
    pub(super) fn grid_share(&mut self, index: usize) -> &mut GridCounters {
        self.note_write_into(index);
        self.slice_grids.entry(index).or_default()
    }

    /// Note a fact's write into slice `index` if B has passed the slice. The
    /// observer reads the note after each fact and stops the run on it, so
    /// handlers stay infallible.
    fn note_write_into(&mut self, index: usize) {
        if passed_slice_end(self.current_world_time(), index, self.config.slice_width()) {
            self.finality.late_write.get_or_insert(index);
        }
    }

    /// The first slice a fact wrote into after B had passed it, if one did.
    pub(super) fn take_late_write(&mut self) -> Option<SimTime> {
        let index = self.finality.late_write.take()?;
        Some(SimTime::ms(index as u64 * self.config.slice_width_ms))
    }

    /// The recording's failure to take a slice, if it failed. The observer
    /// reads it after each step that makes slices final and stops the run.
    pub(super) fn take_recording_failure(&mut self) -> Option<RecordingFailed> {
        self.finality.recording_failed.take()
    }

    /// Navigation held a handoff cell for a robot.
    pub(super) fn note_cell_held(&mut self, hold: &CellHeldEvent) {
        self.finality.relay_busy.held(hold);
    }

    /// A robot's open hold of a handoff cell ended.
    pub(super) fn note_cell_released(&mut self, release: &CellReleasedEvent) {
        self.finality.relay_busy.released(release);
    }

    /// The first hold or release navigation emitted that contradicts the
    /// holds before it, if one did. The observer reads it after each fact and
    /// stops the run on it, before an advance of B makes a slice final from
    /// the holds.
    pub(super) fn take_contradicting_hold(&mut self) -> Option<StreamViolation> {
        self.finality.relay_busy.take_contradiction()
    }

    /// B advanced to `at`, and activity has built what lies behind it: make
    /// final each slice whose end B has passed, but one ending at or past the
    /// run's simtime limit, which is made final only as the stream closes,
    /// once the stop and the facts filed exactly there are known
    /// (`at_the_stop`). A slice no fact had made when B passed it is made
    /// final at the first advance after one makes it.
    pub(super) fn finalize_passed_slices(&mut self, at: SimTime) {
        let width = self.config.slice_width();
        let passed_at = Instant::now();
        while passed_slice_start(at, self.finality.next_boundary, width) {
            self.finality.boundaries_passed.push_back(passed_at);
            self.finality.next_boundary += 1;
        }
        let limit = self.config.sim_time_limit;
        let within_limit = |index: usize| limit.is_none_or(|limit| SimTime::ZERO + width * (index as u64 + 1) < limit);
        while self.finality.final_upto < self.slices().len()
            && passed_slice_end(at, self.finality.final_upto, width)
            && within_limit(self.finality.final_upto)
        {
            self.finalize_next_slice(at, passed_at, None);
        }
    }

    /// The stream has closed and the run stopped at `stop`: make final every
    /// slice that begins before it, timing the part of each that B had not
    /// passed up to the wall-clock instant of the close. Every slice up to the
    /// stop is made to exist, so the slices sent cover the run, and the last
    /// ends at the stop and takes in the facts filed exactly there
    /// (`at_the_stop`). The run's own figures end there too: a slice beginning
    /// at or after the stop, which those facts make and so does work the
    /// scheduler committed past a simtime limit, is dropped with the cell
    /// visits it counted, and so is every journal row filed after the stop.
    /// The progress figures then reach the stop.
    ///
    /// What a run that stops on its simtime limit inside a slice still states
    /// past the stop: that last slice's playback, motion, robot states and
    /// stock end at the stop, but its counters and heat count what facts
    /// stamped between the stop and the slice's end added, its relay busy time
    /// runs to the slice's end, and a journal row filed before the stop keeps
    /// its whole duration though it ends after the stop (an HTM's travel row).
    /// A run stopped any other way stops at the latest instant a fact names,
    /// so nothing lies past it.
    pub(super) fn finalize_remaining_slices(&mut self, stop: SimTime) {
        let closed_at = Instant::now();
        let barrier = self.current_world_time();
        let before_the_stop = stop.as_ms().div_ceil(self.config.slice_width_ms) as usize;
        if let Some(last) = before_the_stop.checked_sub(1) {
            self.extend_slices_through(last);
        }
        assert!(self.finality.final_upto <= before_the_stop, "a slice past the run's stop at {stop} was sent");
        self.take_in_the_facts_at(stop);
        while self.finality.final_upto < before_the_stop {
            self.finalize_next_slice(barrier, closed_at, Some(stop));
        }
        self.forget_slices_from(before_the_stop);
        if let Some(recorder) = &self.finality.recorder {
            let left = self.finality.rows_due.take(&self.output, Due::FiledBy(stop));
            recorder.lock().expect("only the observer's thread holds the recorder during a run").keep_rows_left(left);
        }
        drop_rows_filed_after(&mut self.output, stop);
        if let Some(progress) = &self.finality.progress {
            progress.show_reached(stop);
        }
    }

    /// Drop slice `first` and every slice after it, and take the cell visits
    /// booked against them out of the run's totals, which count what the
    /// slices count.
    fn forget_slices_from(&mut self, first: usize) {
        self.finality.slices.truncate(first);
        let past: Vec<usize> = self.slice_grids.keys().copied().filter(|index| *index >= first).collect();
        for index in past {
            if let Some(share) = self.slice_grids.remove(&index) {
                unfold_grid(&mut self.output.cumulative_grid, &share);
            }
        }
    }

    /// Make final the first slice that is not final yet, with B at `barrier`,
    /// and send it. A boundary of it B has not passed is taken as passed at
    /// the wall-clock instant `fallback`. `stop` is the run's, once the stream
    /// has closed: a slice reaching past it ends there.
    fn finalize_next_slice(&mut self, barrier: SimTime, fallback: Instant, stop: Option<SimTime>) {
        let index = self.finality.final_upto;
        let width = self.config.slice_width();
        let begins = SimTime::ZERO + width * index as u64;
        let end = stop.map_or(begins + width, |stop| stop.min(begins + width));
        if let Some(grid) = self.slice_grids.remove(&index) {
            fold_grid(&mut self.slice_to_finalize(index).grid, &grid);
        }
        self.stamp_physical_replay(index, stop);
        if let Some(stop) = stop.filter(|stop| *stop == end) {
            leave_out_past(self.slice_to_finalize(index), stop);
        }
        self.make_drawable(index);
        let started = self.finality.boundaries_passed.pop_front().unwrap_or(fallback);
        let ended = self.finality.boundaries_passed.front().copied().unwrap_or(fallback);
        let relay_busy = self.finality.relay_busy.take_slice(index, barrier);
        let slice = self.slice_to_finalize(index);
        slice.scalars.wall_clock_duration_ms = (ended - started).as_secs_f64() * 1000.0;
        slice.scalars.relay_reserved_busy_ms = relay_busy.all.as_ms();
        slice.scalars.relay_reserved_busy_per_zone = relay_busy.per_zone;
        self.finality.final_upto = index + 1;
        let robot_spans = self.robot_state_log.state_slice(begins, end, &self.output.robot_types);
        if let Some(recorder) = &self.finality.recorder {
            let slices = &self.finality.slices;
            let last = stop == Some(end);
            // The run's last slice, the one the stop ends, takes every row left that the run filed by its stop.
            let due = if last { Due::FiledBy(end) } else { Due::FinalBefore(end) };
            let rows = self.finality.rows_due.take(&self.output, due);
            let slice = FinalSlice {
                slice: &slices[index],
                next: slices.get(index + 1),
                ends: end,
                last,
                rows,
                robot_spans,
                roster: &self.output.robot_types,
            };
            let recorded =
                recorder.lock().expect("only the observer's thread holds the recorder during a run").record(slice);
            if let Err(failure) = recorded {
                self.finality.recorder = None;
                self.finality.recording_failed = Some(failure);
            }
        }
        self.output.cumulative_scalars.carry(&self.finality.slices[index].scalars);
        if let Some(progress) = &self.finality.progress {
            let scalars = &self.finality.slices[index].scalars;
            progress.show_slice(scalars.picks_completed, scalars.orders_completed, end);
        }
    }
}

/// Leave out of `slice`, the one the run's stop at `stop` ends, every change it files past the stop: work the
/// scheduler committed before the stop files its tote, relay and station changes at the instants they will happen,
/// and a recording states nothing after the run's stop. A change at the stop stays, and so does a conveyor journey
/// landing by then; one landing later is left out, as no slice of a run that ended at the stop would state it. The
/// stock replay already stops the slice's tote contents at the stop.
fn leave_out_past(slice: &mut TimeSliceData, stop: SimTime) {
    let reach_ms = stop.as_ms() - slice.start_ms;
    let totes = &mut slice.tote_state;
    totes.creations.retain(|birth| birth.offset_ms <= reach_ms);
    totes.transitions.retain(|(offset_ms, ..)| *offset_ms <= reach_ms);
    totes.transits.retain(|transit| transit.end_ms <= stop.as_ms());
    slice.relay_state.transitions.retain(|(_, offset_ms, _)| *offset_ms <= reach_ms);
    slice.station_diag.pipeline_depths.retain(|depth| depth.at <= stop);
}

/// Take `share` out of `totals`, leaving no counter at zero.
fn unfold_grid(totals: &mut GridCounters, share: &GridCounters) {
    take_out(&mut totals.nav_visits_vtm, &share.nav_visits_vtm, u32::saturating_sub);
    take_out(&mut totals.nav_visits_htm, &share.nav_visits_htm, u32::saturating_sub);
    take_out(&mut totals.robot_presence_vtm_ms, &share.robot_presence_vtm_ms, u64::saturating_sub);
    take_out(&mut totals.robot_presence_htm_ms, &share.robot_presence_htm_ms, u64::saturating_sub);
}

fn take_out<V: Copy + Default + PartialEq>(
    total: &mut FxHashMap<u32, V>,
    part: &FxHashMap<u32, V>,
    minus: fn(V, V) -> V,
) {
    for (cell, count) in part {
        if let Some(counted) = total.get_mut(cell) {
            *counted = minus(*counted, *count);
            if *counted == V::default() {
                total.remove(cell);
            }
        }
    }
}

fn fold_grid(slice: &mut GridCounters, share: &GridCounters) {
    for (cell, count) in &share.nav_visits_vtm {
        *slice.nav_visits_vtm.entry(*cell).or_insert(0) += count;
    }
    for (cell, count) in &share.nav_visits_htm {
        *slice.nav_visits_htm.entry(*cell).or_insert(0) += count;
    }
    for (cell, ms) in &share.robot_presence_vtm_ms {
        *slice.robot_presence_vtm_ms.entry(*cell).or_insert(0) += ms;
    }
    for (cell, ms) in &share.robot_presence_htm_ms {
        *slice.robot_presence_htm_ms.entry(*cell).or_insert(0) += ms;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use desgraph::{CellId, RobotType, StationId, StationType, ZoneId};
    use rustc_hash::FxHashMap;

    use super::recording::{SliceRecorder, SliceRows};
    use super::*;
    use crate::events::{
        CellHeldEvent, CellReleasedEvent, CycleCompleteEvent, CycleKind, OrderCompleteEvent, SimEvent,
    };
    use crate::model::ids::SkuId;
    use crate::model::{OrderId, RobotId, SimDuration, StationArrivalSource, ToteDesignation, ToteType};
    use crate::observer::SimObserver;
    use crate::observer::output::CompletedOrderRecord;
    use crate::observer::station_diag::PipelineDepthRecord;
    use crate::observer::time_slice::{ToteCreationRecord, ToteTransitRecord, ToteVisualState};
    use crate::sim_types::{StopReason, StopSignal};

    pub(super) fn order_complete(sim_time_ms: u64) -> SimEvent {
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms })
    }

    pub(super) fn state(width: u64) -> SimObserverState {
        SimObserverState::new(MetricsConfig { slice_width_ms: width, ..Default::default() })
    }

    /// Each slice a recorder was handed: its start, and the start of the
    /// slice after it as B left it.
    #[derive(Default)]
    pub(super) struct Handed(pub(super) Vec<(u64, Option<u64>)>);

    impl SliceRecorder for Handed {
        fn record(&mut self, slice: FinalSlice<'_>) -> Result<(), RecordingFailed> {
            self.0.push((slice.slice.start_ms, slice.next.map(|next| next.start_ms)));
            Ok(())
        }

        fn keep_rows_left(&mut self, _: SliceRows) {}
    }

    /// The recording takes each slice once, when B passes its end, and the
    /// slices before it never again.
    #[test]
    fn the_recording_takes_a_slice_once_b_passes_its_end() {
        let handed = Arc::new(Mutex::new(Handed::default()));
        let mut state = state(1_000);
        state.record_into(handed.clone());
        state.dispatch_event(&order_complete(2_500));
        let taken = || -> Vec<u64> { handed.lock().unwrap().0.drain(..).map(|(start, ..)| start).collect() };
        state.finalize_passed_slices(SimTime::ms(1_000));
        assert_eq!(taken(), Vec::<u64>::new(), "B at a slice's end has not passed it");
        state.finalize_passed_slices(SimTime::ms(1_001));
        assert_eq!(taken(), [0]);
        state.finalize_passed_slices(SimTime::ms(2_400));
        assert_eq!(taken(), [1_000]);
        state.finalize_remaining_slices(SimTime::ms(2_500));
        assert_eq!(taken(), [2_000], "the slice B never passed goes out when the stream closes");
    }

    /// The recording takes each slice once, in order, as it is made final,
    /// with the slice after it as B leaves it; the last slice made final when
    /// the stream closes is the run's last.
    #[test]
    fn the_recording_takes_each_slice_once_as_it_is_made_final() {
        let handed = Arc::new(Mutex::new(Handed::default()));
        let mut state = state(1_000);
        state.record_into(handed.clone());
        state.dispatch_event(&order_complete(2_500));
        state.finalize_passed_slices(SimTime::ms(1_001));
        assert_eq!(handed.lock().unwrap().0, [(0, Some(1_000))]);
        state.finalize_passed_slices(SimTime::ms(1_500));
        state.dispatch_event(&order_complete(3_500));
        state.finalize_passed_slices(SimTime::ms(2_001));
        state.finalize_remaining_slices(SimTime::ms(3_500));
        let expected = [(0, Some(1_000)), (1_000, Some(2_000)), (2_000, Some(3_000))];
        assert_eq!(handed.lock().unwrap().0[..3], expected);
        assert_eq!(handed.lock().unwrap().0[3..], [(3_000, None)], "the stream's close makes the last final");
    }

    /// A slice ending past the run's simtime limit is made final only as the stream closes, though B has passed it: B
    /// can run past the limit while the planned frontier is short of it, and the run that then stops on the limit ends
    /// there.
    #[test]
    fn a_slice_ending_past_the_simtime_limit_is_made_final_as_the_stream_closes_and_ends_at_the_stop() {
        let handed = Arc::new(Mutex::new(Handed::default()));
        let config =
            MetricsConfig { slice_width_ms: 1_000, sim_time_limit: Some(SimTime::ms(2_500)), ..Default::default() };
        let mut state = SimObserverState::new(config);
        state.record_into(handed.clone());
        state.dispatch_event(&order_complete(3_500));
        state.finalize_passed_slices(SimTime::ms(3_001));
        let taken = || -> Vec<u64> { handed.lock().unwrap().0.drain(..).map(|(start, ..)| start).collect() };
        assert_eq!(taken(), [0, 1_000], "slice 2 ends at 3 s, past the limit");
        state.finalize_remaining_slices(SimTime::ms(2_500));
        assert_eq!(taken(), [2_000], "the stop at the limit cuts it, and the slice past the stop goes nowhere");
        assert_eq!(state.slices().len(), 3, "and the run's own slices end with it");
    }

    /// The completed orders due with each slice a recorder was handed, and
    /// those it was handed as left when the stream closed.
    #[derive(Default)]
    struct Orders {
        due: Vec<Vec<u64>>,
        left: Option<Vec<u64>>,
    }

    impl SliceRecorder for Orders {
        fn record(&mut self, slice: FinalSlice<'_>) -> Result<(), RecordingFailed> {
            self.due.push(slice.rows.completed_orders.iter().map(|row| row.sim_time_ms).collect());
            Ok(())
        }

        fn keep_rows_left(&mut self, rows: SliceRows) {
            assert!(self.left.is_none(), "rows are left once, as the stream closes");
            self.left = Some(rows.completed_orders.iter().map(|row| row.sim_time_ms).collect());
        }
    }

    /// A row reported when its slice is no longer the one being made final, as
    /// a gap the run's end closes is, goes with the next slice made final; the
    /// run's last slice takes every row left that the run filed by its stop,
    /// and one filed after it, work committed past the stop, is never stated.
    #[test]
    fn the_runs_last_slice_takes_every_row_left_filed_by_its_stop() {
        let orders = Arc::new(Mutex::new(Orders::default()));
        let mut state = state(1_000);
        state.record_into(orders.clone());
        state.dispatch_event(&order_complete(500));
        state.dispatch_event(&order_complete(1_500));
        state.finalize_passed_slices(SimTime::ms(1_001));
        for sim_time_ms in [1_200, 2_600, 3_200] {
            let order = CompletedOrderRecord { sim_time_ms, order_id: OrderId(2), station_id: None, dc_id: None };
            state.output.completed_order_log.push(order);
        }
        state.finalize_remaining_slices(SimTime::ms(2_600));
        let orders = orders.lock().unwrap();
        assert_eq!(orders.due, [vec![500], vec![1_500, 1_200], vec![2_600]]);
        assert_eq!(orders.left, Some(Vec::new()), "the last slice took them all, but the one past the stop");
    }

    /// A row reported once every slice has gone out is handed over as the
    /// stream closes, for the recording's end to state.
    #[test]
    fn a_row_reported_after_every_slice_went_out_is_handed_over_at_the_close() {
        let orders = Arc::new(Mutex::new(Orders::default()));
        let mut state = state(1_000);
        state.record_into(orders.clone());
        state.dispatch_event(&order_complete(500));
        state.finalize_passed_slices(SimTime::ms(1_001));
        state.output.completed_order_log.push(CompletedOrderRecord {
            sim_time_ms: 800,
            order_id: OrderId(2),
            station_id: None,
            dc_id: None,
        });
        state.finalize_remaining_slices(SimTime::ms(1_000));
        let orders = orders.lock().unwrap();
        assert_eq!(orders.due, [vec![500]]);
        assert_eq!(orders.left, Some(vec![800]));
    }

    /// A recording that takes no slice: each one it is handed, it refuses.
    #[derive(Default)]
    struct Refuses(Vec<u64>);

    impl SliceRecorder for Refuses {
        fn record(&mut self, slice: FinalSlice<'_>) -> Result<(), RecordingFailed> {
            self.0.push(slice.slice.start_ms);
            Err(RecordingFailed("the disk is full".to_owned()))
        }

        fn keep_rows_left(&mut self, _: SliceRows) {}
    }

    /// A slice B passes that the recording cannot take stops the run while it
    /// goes, before the stream closes, and the recording is handed no slice
    /// after it: not the next one B passes, nor the last at the close.
    #[test]
    fn a_slice_the_recording_cannot_take_stops_the_run() {
        let refuses = Arc::new(Mutex::new(Refuses::default()));
        let stop = StopSignal::new();
        let (tx, rx) = mpsc::channel();
        let observer = SimObserver::new(MetricsConfig { slice_width_ms: 1_000, ..Default::default() }, rx)
            .with_recorder(refuses.clone())
            .with_failure_stop(stop.clone());
        let observer = thread::spawn(move || observer.run());
        tx.send(order_complete(2_500)).unwrap();
        tx.send(SimEvent::BarrierAdvanced(SimTime::ms(1_001))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !stop.is_stopped() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(stop.is_stopped(), "the run is told to stop while the stream is still open");
        tx.send(SimEvent::BarrierAdvanced(SimTime::ms(2_001))).unwrap();
        drop(tx);
        assert!(observer.join().unwrap().is_ok(), "the observer reads on to the close, as for any stopped run");
        let reason = stop.take_reason();
        assert!(
            matches!(&reason, Some(StopReason::Error(message)) if message == "the run's recording failed: the disk is full"),
            "the reason is the recording's failure: {reason:?}"
        );
        assert_eq!(refuses.lock().unwrap().0, [0], "only slice 0 was handed to it");
    }

    pub(super) fn pick(sim_time_ms: u64, qty: u32) -> SimEvent {
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(0),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty },
            station_id: StationId(0),
            sim_time_ms,
            dwell_ms: 200,
            arrival_source: None,
            dc_id: None,
            presentation_id: None,
            tote_presentation: None,
        })
    }

    /// The progress figures show a slice's picks and completed orders once B
    /// has passed its end, and then reach that end; facts read ahead of B show
    /// nothing. The stream's close shows the rest, up to the run's end.
    #[test]
    fn a_progress_figure_shows_only_once_its_slice_is_final() {
        let counters = Counters::new();
        let mut state = state(1_000);
        state.show_progress_in(counters.clone());
        state.dispatch_event(&pick(1_500, 3));
        state.dispatch_event(&order_complete(1_500));
        state.dispatch_event(&order_complete(2_500));
        let shown = || (counters.shown_picks(), counters.shown_orders_completed(), counters.shown_sim_time());
        assert_eq!(shown(), (0, 0, SimTime::ZERO), "every fact is ahead of B");
        state.finalize_passed_slices(SimTime::ms(1_001));
        assert_eq!(shown(), (0, 0, SimTime::ms(1_000)), "slice 0 is final and states neither");
        state.finalize_passed_slices(SimTime::ms(2_001));
        assert_eq!(shown(), (3, 1, SimTime::ms(2_000)));
        state.finalize_remaining_slices(SimTime::ms(2_500));
        assert_eq!(shown(), (3, 2, SimTime::ms(2_500)), "the run stops inside slice 2, at its last fact");
    }

    /// A putaway cycle is reported at its fill, after B has passed the start
    /// of its dwell: the station-cycle record goes in the slice B stands in,
    /// stating the whole dwell, and nothing writes into the slices B passed.
    #[test]
    fn a_putaway_cycle_record_is_filed_where_b_stands() {
        let mut config = MetricsConfig::default();
        config.station_types.insert(StationId(0), StationType::Putaway);
        let mut state = SimObserverState::new(config);
        state.dispatch_event(&order_complete(15_000));
        state.settle_activity(SimTime::ms(25_000)).unwrap();
        state.finalize_passed_slices(SimTime::ms(25_000));
        state.dispatch_event(&SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(0),
            robot_type: RobotType::Htm,
            kind: CycleKind::Putaway { sku: SkuId(1), qty: 1 },
            station_id: StationId(0),
            sim_time_ms: 30_000,
            dwell_ms: 15_000,
            arrival_source: Some(StationArrivalSource::PutawayFromRack),
            dc_id: None,
            presentation_id: None,
            tote_presentation: None,
        }));
        assert_eq!(state.take_late_write(), None);
        assert!(
            state.slices()[1].station_diag.station_cycles.is_empty(),
            "the dwell's start slice went out without it"
        );
        let filed = &state.slices()[2].station_diag.station_cycles;
        assert_eq!(
            filed.iter().map(|cycle| (cycle.start, cycle.dwell)).collect::<Vec<_>>(),
            [(SimTime::ms(15_000), SimDuration::ms(15_000))]
        );
    }

    /// A slice is closed to facts once B has passed its end, whether or not it
    /// has gone out: one no fact had made when B passed it goes out at the
    /// next advance, and a write into it before then is noted.
    #[test]
    fn a_write_into_a_slice_b_has_passed_is_noted_before_the_slice_goes_out() {
        let mut state = state(1_000);
        state.settle_activity(SimTime::ms(2_500)).unwrap();
        state.finalize_passed_slices(SimTime::ms(2_500));
        assert!(state.slices().is_empty(), "no fact has made a slice yet");
        state.slice_to_write(1);
        assert_eq!(state.take_late_write(), Some(SimTime::ms(1_000)));
        state.slice_to_write(2);
        assert_eq!(state.take_late_write(), None, "B stands inside slice 2");
    }

    /// A step that makes a slice final writes into it before it is final; a
    /// write into a final slice is an observer bug and crashes.
    #[test]
    #[should_panic(expected = "slice 0 is final and takes no more writes")]
    fn a_finalizing_write_into_a_final_slice_panics() {
        let mut state = state(1_000);
        state.dispatch_event(&order_complete(1_500));
        state.finalize_passed_slices(SimTime::ms(1_001));
        state.slice_to_finalize(1);
        state.slice_to_finalize(0);
    }

    /// A slice's wall-clock duration runs from B passing its start to B
    /// passing its end; a slice B passes whole in one advance took no time.
    #[test]
    fn a_slice_is_timed_from_b_passing_its_start_to_b_passing_its_end() {
        let mut state = state(1_000);
        state.dispatch_event(&order_complete(2_500));
        state.finalize_passed_slices(SimTime::ms(1_500));
        thread::sleep(Duration::from_millis(20));
        state.finalize_passed_slices(SimTime::ms(2_001));
        assert_eq!(state.slices()[0].scalars.wall_clock_duration_ms, 0.0, "B passed slice 0 whole at once");
        assert!(state.slices()[1].scalars.wall_clock_duration_ms >= 20.0, "B passed slice 1 over at least 20 ms");
    }

    fn relay_state(width: u64) -> SimObserverState {
        let mut config = MetricsConfig { slice_width_ms: width, ..Default::default() };
        config.relay_htm_access_cells.insert(0, 7);
        config.station_cells.insert(StationId(0), CellId(9));
        SimObserverState::new(config)
    }

    fn held(cell: u32, from: u64, until: Option<u64>) -> SimEvent {
        let (from, until) = (SimTime::ms(from), until.map(SimTime::ms));
        SimEvent::CellHeld(CellHeldEvent { robot_id: RobotId(1), cell: CellId(cell), from, until })
    }

    /// B advances to `at`, as the observer's loop reads the marker.
    fn advance(state: &mut SimObserverState, at: u64) {
        state.settle_activity(SimTime::ms(at)).unwrap();
        state.finalize_passed_slices(SimTime::ms(at));
    }

    fn relay_busy(state: &SimObserverState) -> Vec<u64> {
        state.slices().iter().map(|slice| slice.scalars.relay_reserved_busy_ms).collect()
    }

    /// A slice's relay busy time is how long navigation's holds kept the
    /// relays' access cells across it. In a slice B never passed, an open hold
    /// counts up to the latest instant a hold on an access or presentation
    /// cell begins or ends.
    #[test]
    fn a_slice_carries_how_long_the_relays_access_cells_were_held_across_it() {
        let mut state = relay_state(1_000);
        state.dispatch_event(&order_complete(3_500));
        state.dispatch_event(&held(7, 500, None));
        state.dispatch_event(&held(8, 0, Some(5_000)));
        advance(&mut state, 1_001);
        let released = CellReleasedEvent { robot_id: RobotId(1), cell: CellId(7), at: SimTime::ms(1_700) };
        state.dispatch_event(&SimEvent::CellReleased(released));
        state.dispatch_event(&held(7, 2_200, Some(2_400)));
        state.dispatch_event(&held(7, 2_500, None));
        state.dispatch_event(&held(9, 3_200, Some(3_600)));
        advance(&mut state, 2_001);
        state.finalize_remaining_slices(SimTime::ms(3_600));
        assert_eq!(relay_busy(&state), [500, 700, 200 + 500, 600], "cell 8 is no relay's; cell 9 is a station's");
    }

    /// An open hold fills each slice B passes, past the horizon too: nothing
    /// can end it before B.
    #[test]
    fn an_open_hold_fills_every_slice_b_passes_even_past_the_horizon() {
        let mut state = relay_state(1_000);
        state.dispatch_event(&order_complete(3_500));
        state.dispatch_event(&held(7, 500, None));
        advance(&mut state, 3_001);
        state.finalize_remaining_slices(SimTime::ms(3_500));
        assert_eq!(relay_busy(&state), [500, 1_000, 1_000, 0], "the horizon is where the hold began, 0.5 s");
    }

    /// A slice made final carries each relay zone's busy time beside all the
    /// relays', an idle zone's as zero.
    #[test]
    fn a_final_slice_carries_each_relay_zones_busy_time() {
        let mut config = MetricsConfig { slice_width_ms: 1_000, ..Default::default() };
        config.relay_htm_access_cells.extend([(0, 7), (1, 8)]);
        config.relay_cell_zone = Arc::new([(7, 3), (8, 4)].into_iter().collect());
        let mut state = SimObserverState::new(config);
        state.dispatch_event(&order_complete(500));
        state.dispatch_event(&held(7, 200, Some(600)));
        advance(&mut state, 1_001);
        let scalars = &state.slices()[0].scalars;
        assert_eq!(scalars.relay_reserved_busy_ms, 400);
        let zones = FxHashMap::from_iter([(ZoneId(3), SimDuration::ms(400)), (ZoneId(4), SimDuration::ZERO)]);
        assert_eq!(scalars.relay_reserved_busy_per_zone, zones);
    }

    /// Booking a cell visit must never bring a slice into existence.
    ///
    /// A waypoint of no duration sitting exactly on a boundary — a `PathEnd`
    /// at the last instant of a run, a rail move that stops where a slice
    /// begins — names the slice after the run's last. Creating it to hold the
    /// visit would give the run a slice master never had, and
    /// `extend_slices_through` stamps a new slice with SNAPSHOTS: the rack
    /// fill, the relay count. The run would then report a trailing slice of
    /// numbers nobody measured.
    #[test]
    fn a_visit_past_the_last_slice_makes_no_slice_and_is_dropped() {
        let mut state = state(1_000);
        state.extend_slices_through(1);
        let had = state.slices().len();

        // Two visits: one inside the run's last slice, one at the instant the
        // slice after it would begin.
        *state.grid_share(1).nav_visits_htm.entry(7).or_insert(0) += 1;
        *state.grid_share(2).nav_visits_htm.entry(9).or_insert(0) += 1;
        assert_eq!(state.slices().len(), had, "booking a visit creates no slice");

        state.finalize_remaining_slices(SimTime::ms(2_000));
        assert_eq!(state.slices().len(), had, "and neither does folding them in");
        assert_eq!(state.slices()[1].grid.nav_visits_htm.get(&7), Some(&1), "the visit inside the run is kept");
        assert!(
            state.slices().iter().all(|slice| !slice.grid.nav_visits_htm.contains_key(&9)),
            "and the one past its end belongs to no slice"
        );
    }

    /// The slice the run's stop cuts, 4 s into it, keeps each tote, relay and station change at the stop and each
    /// conveyor journey landing by then, and leaves out the ones work committed before the stop files past it.
    #[test]
    fn the_slice_the_stop_cuts_leaves_out_what_is_filed_past_the_stop() {
        let mut state = state(10_000);
        let slice = state.slice_to_write(0);
        for at_ms in [4_000, 6_000] {
            let tote_id = at_ms as u32;
            let (tote_type, designation) = (ToteType::default(), ToteDesignation::default());
            slice.tote_state.creations.push(ToteCreationRecord { offset_ms: at_ms, tote_id, tote_type, designation });
            slice.tote_state.transitions.push((at_ms, tote_id, ToteVisualState::Extracted));
            let journey = ToteTransitRecord { tote_id, from_cell: 1, to_cell: 2, start_ms: 1_000, end_ms: at_ms };
            slice.tote_state.transits.push(journey);
            slice.relay_state.transitions.push((7, at_ms, true));
            let depth = PipelineDepthRecord { station_id: StationId(3), depth: 1, at: SimTime::ms(at_ms) };
            slice.station_diag.pipeline_depths.push(depth);
        }
        state.finalize_remaining_slices(SimTime::ms(4_000));

        let cut = &state.slices()[0];
        let totes = &cut.tote_state;
        assert_eq!(totes.creations.iter().map(|birth| birth.offset_ms).collect::<Vec<_>>(), [4_000], "births");
        assert_eq!(totes.transitions.iter().map(|(at, ..)| *at).collect::<Vec<_>>(), [4_000], "tote moves");
        assert_eq!(totes.transits.iter().map(|journey| journey.end_ms).collect::<Vec<_>>(), [4_000], "journeys");
        assert_eq!(cut.relay_state.transitions, [(7, 4_000, true)], "relay changes");
        let depths: Vec<u64> = cut.station_diag.pipeline_depths.iter().map(|depth| depth.at.as_ms()).collect();
        assert_eq!(depths, [4_000], "station depths");
    }

    /// A run stopped on its simtime limit at a slice's end, 2 s, totals what its two slices state: eleven units picked
    /// in three cycles, ending at 0.7 s, 1.7 s and the stop, and two orders, completed at 1.5 s and the stop, the last
    /// slice taking in what is filed at the stop. The cycle and the order that work committed before the stop
    /// completes at 2.5 s belong to no slice, and so to no total.
    #[test]
    fn a_run_stopped_on_its_limit_at_a_slices_end_totals_only_what_its_slices_state() {
        let limit = Some(SimTime::ms(2_000));
        let mut state =
            SimObserverState::new(MetricsConfig { slice_width_ms: 1_000, sim_time_limit: limit, ..Default::default() });
        let before_the_stop = [pick(500, 1), pick(1_500, 2), order_complete(1_500)];
        let at_the_stop = [pick(1_800, 8), order_complete(2_000)];
        let past_it = [pick(2_500, 4), order_complete(2_500)];
        for fact in before_the_stop.iter().chain(&at_the_stop).chain(&past_it) {
            state.dispatch_event(fact);
        }
        state.finalize_remaining_slices(SimTime::ms(2_000));
        let totals = &state.output.cumulative_scalars;
        assert_eq!((totals.picks_completed, totals.presentations), (11, 3), "units picked and cycles, by the stop");
        assert_eq!(totals.orders_completed, 2, "orders completed by the stop");
    }

    /// The counters land in the slice they were booked against, and add onto
    /// whatever that slice already held.
    #[test]
    fn the_counters_land_in_the_slice_they_were_booked_against() {
        let mut state = state(1_000);
        state.extend_slices_through(2);
        state.slice_to_write(0).grid.nav_visits_vtm.insert(4, 2);
        *state.grid_share(0).nav_visits_vtm.entry(4).or_insert(0) += 3;
        *state.grid_share(0).robot_presence_htm_ms.entry(4).or_insert(0) += 500;
        *state.grid_share(2).nav_visits_htm.entry(5).or_insert(0) += 1;

        state.finalize_remaining_slices(SimTime::ms(3_000));
        assert_eq!(state.slices()[0].grid.nav_visits_vtm.get(&4), Some(&5), "the counts add");
        assert_eq!(state.slices()[0].grid.robot_presence_htm_ms.get(&4), Some(&500));
        assert_eq!(state.slices()[2].grid.nav_visits_htm.get(&5), Some(&1));
        assert!(state.slices()[1].grid.nav_visits_htm.is_empty(), "a slice nothing was booked against stays empty");
    }
}
