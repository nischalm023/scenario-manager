//! Within `finality`, `at_the_stop` is the facts filed exactly at the run's stop when the stop is a slice boundary:
//! their instant opens the slice after the run's last, so the last slice, which the stop ends, takes them in. A run
//! stopped on its simtime limit stops at an instant known before any fact, but the slice beginning there also holds
//! the work the scheduler committed past it, so a fact exactly at a limit on a slice boundary waits apart until the
//! stream closes. Any other run stops at the latest instant a fact names, so the slice beginning at its stop holds
//! only facts at the stop. Motion, the cell visits it makes and the spans of time clipped into slices begin at the
//! stop at the earliest, so they lie past it and are not filed apart.

use std::mem;

use crate::SimTime;
use crate::observer::state::SimObserverState;
use crate::observer::station_diag::StationDiagRecord;
use crate::observer::time_slice::{RelayStateRecord, TimeSliceData, ToteStateRecord};

impl SimObserverState {
    /// The slice a fact at `at` writes into: the one `at` falls in, made to exist with every slice before it, but for
    /// a fact exactly at the run's simtime limit when that is a slice boundary, which waits apart until the stream
    /// closes, to join the slice ending at the limit if the run stops there and the one beginning there otherwise.
    pub(in crate::observer) fn slice_at(&mut self, at: SimTime) -> &mut TimeSliceData {
        let width = self.config.slice_width_ms;
        let index = (at.as_ms() / width) as usize;
        if self.config.sim_time_limit != Some(at) || at == SimTime::ZERO || !at.as_ms().is_multiple_of(width) {
            return self.slice_to_write(index);
        }
        self.note_write_into(index);
        self.extend_slices_through(index);
        let at_the_limit = || TimeSliceData { start_ms: at.as_ms(), ..TimeSliceData::default() };
        self.finality.at_the_limit.get_or_insert_with(at_the_limit)
    }

    /// The stream has closed and the run stopped at `stop`, with every slice up to it made to exist: join the facts
    /// at the run's simtime limit to the slice they belong in, if the run reached the limit, and when the stop is a
    /// slice boundary, give the run's last slice the facts filed at the stop.
    pub(super) fn take_in_the_facts_at(&mut self, stop: SimTime) {
        let width = self.config.slice_width_ms;
        if let Some(at_the_limit) = self.finality.at_the_limit.take() {
            let limit = at_the_limit.start_ms;
            let opens = (limit / width) as usize;
            if stop.as_ms() == limit {
                take_in(self.slice_to_finalize(opens - 1), at_the_limit);
            } else if stop.as_ms() > limit {
                take_in(self.slice_to_finalize(opens), at_the_limit);
            }
        }
        let opens = (stop.as_ms() / width) as usize;
        let on_a_boundary = stop > SimTime::ZERO && stop.as_ms().is_multiple_of(width);
        if !on_a_boundary || self.config.sim_time_limit == Some(stop) || opens >= self.slices().len() {
            return;
        }
        // The slice beginning at the stop is no slice of the run's. It keeps the motion it begins, which the
        // recording reads the cell of each robot moving at the stop from.
        let opening = &mut self.finality.slices[opens];
        let motion = mem::take(&mut opening.bot_locations);
        let kept = TimeSliceData { start_ms: opening.start_ms, bot_locations: motion, ..TimeSliceData::default() };
        let facts = mem::replace(opening, kept);
        take_in(self.slice_to_finalize(opens - 1), facts);
    }
}

/// Take into `slice` `facts`, all filed at the instant `facts` starts at, which is `slice`'s end or its start: counts
/// add, and changes and records join the slice's own at that instant, after them at its end and before them at its
/// start. What else `facts` holds stays out: a slice's opening state and the readings it was stamped with, the stock
/// the replay stamps, conveyor journeys, each filed in the slice it lands in, and motion and the cell visits it
/// makes. Every field is named, so one added later does not compile until its rule is chosen here.
fn take_in(slice: &mut TimeSliceData, facts: TimeSliceData) {
    let TimeSliceData { start_ms, scalars, grid: _, bot_locations: _, relay_state, tote_state, station_diag } = facts;
    let shift = start_ms - slice.start_ms;
    let at_start = shift == 0;
    slice.scalars.add_counts(&scalars);
    let RelayStateRecord { start_reserved: _, transitions } = relay_state;
    let transitions = transitions.into_iter().map(|(cell, offset, reserved)| (cell, offset + shift, reserved));
    join(&mut slice.relay_state.transitions, transitions, at_start);
    let ToteStateRecord {
        creations,
        contents: _,
        transits: _,
        start_states: _,
        transitions,
        order_totes: _,
        carries_previous: _,
    } = tote_state;
    let creations = creations.into_iter().map(|mut birth| {
        birth.offset_ms += shift;
        birth
    });
    join(&mut slice.tote_state.creations, creations, at_start);
    let transitions = transitions.into_iter().map(|(offset, tote, state)| (offset + shift, tote, state));
    join(&mut slice.tote_state.transitions, transitions, at_start);
    let StationDiagRecord {
        assignments,
        arrivals,
        rejections,
        pipeline_depths,
        activity,
        vtm_idle_reasons,
        htm_relay_pickups,
        station_cycles,
        order_phases,
        presentation_readiness,
        inventory_visit_phases,
        plan_legs,
    } = station_diag;
    let diag = &mut slice.station_diag;
    join(&mut diag.assignments, assignments, at_start);
    join(&mut diag.arrivals, arrivals, at_start);
    join(&mut diag.pipeline_depths, pipeline_depths, at_start);
    join(&mut diag.activity, activity, at_start);
    join(&mut diag.vtm_idle_reasons, vtm_idle_reasons, at_start);
    join(&mut diag.htm_relay_pickups, htm_relay_pickups, at_start);
    join(&mut diag.station_cycles, station_cycles, at_start);
    join(&mut diag.order_phases, order_phases, at_start);
    join(&mut diag.presentation_readiness, presentation_readiness, at_start);
    join(&mut diag.inventory_visit_phases, inventory_visit_phases, at_start);
    join(&mut diag.plan_legs, plan_legs, at_start);
    for counted in rejections {
        diag.join_rejections(counted);
    }
}

/// Join `facts` to a slice's `own` records: before them when the facts are at the slice's start, after them otherwise.
fn join<T>(own: &mut Vec<T>, facts: impl IntoIterator<Item = T>, at_start: bool) {
    if at_start {
        own.splice(0..0, facts);
    } else {
        own.extend(facts);
    }
}

#[cfg(test)]
mod tests {
    use desgraph::StationId;

    use super::super::tests::{order_complete, pick, state};
    use super::*;
    use crate::events::{
        DispatchRejectionEvent, DispatchRejectionReason, RelayStateChangeEvent, SimEvent, ToteCreatedEvent,
    };
    use crate::model::{ToteDesignation, ToteId, ToteType};
    use crate::observer::time_slice::{MetricsConfig, ToteVisualState};

    fn relay_change(at_ms: u64, reserved: bool) -> SimEvent {
        SimEvent::RelayStateChange(RelayStateChangeEvent { cell_id: 7, reserved, time: SimTime::ms(at_ms) })
    }

    fn limited_to(limit_ms: u64) -> SimObserverState {
        let sim_time_limit = Some(SimTime::ms(limit_ms));
        SimObserverState::new(MetricsConfig { slice_width_ms: 1_000, sim_time_limit, ..MetricsConfig::default() })
    }

    /// A run stopped at its last fact, at 1 s, the end of its first slice: the slice the stop's instant opens is no
    /// slice of the run's, and the run's one slice states the order, the pick cycle, the relay change, the tote's
    /// birth and the station's refusal filed at the stop, the changes at its end.
    #[test]
    fn a_run_stopped_on_a_slices_end_states_the_facts_filed_there_in_its_last_slice() {
        let mut state = state(1_000);
        let (stop, at_cell) = (SimTime::ms(1_000), ToteVisualState::AtCell { cell_id: 4 });
        let (tote_type, designation) = (ToteType::default(), ToteDesignation::default());
        let birth = ToteCreatedEvent { tote_id: ToteId(3), tote_type, designation, initial_state: at_cell, time: stop };
        let (station_id, reason) = (StationId(2), DispatchRejectionReason::ConwipCapSaturated);
        let refusal = DispatchRejectionEvent { station_id, at: stop, reason, count: 1, detail: None };
        for fact in [order_complete(500), order_complete(1_000), pick(800, 3), relay_change(1_000, true)] {
            state.dispatch_event(&fact);
        }
        state.dispatch_event(&SimEvent::ToteCreated(birth));
        state.dispatch_event(&SimEvent::DispatchRejection(refusal));
        state.finalize_remaining_slices(stop);

        assert_eq!(state.slices().len(), 1, "one slice, ending at the stop");
        let last = &state.slices()[0];
        assert_eq!((last.scalars.orders_completed, last.scalars.picks_completed), (2, 3), "both orders and the pick");
        assert_eq!(last.relay_state.transitions, [(7, 1_000, true)], "the relay change, at the slice's end");
        let births: Vec<(u64, u32)> = last.tote_state.creations.iter().map(|b| (b.offset_ms, b.tote_id)).collect();
        assert_eq!(births, [(1_000, 3)], "the tote's birth");
        assert_eq!(last.tote_state.transitions.iter().map(|(at, ..)| *at).collect::<Vec<_>>(), [1_000], "its move");
        let refused: Vec<(StationId, u64)> =
            last.station_diag.rejections.iter().map(|r| (r.station_id, r.count)).collect();
        assert_eq!(refused, [(StationId(2), 1)], "the station's refusal");
        let totals = &state.output.cumulative_scalars;
        assert_eq!((totals.orders_completed, totals.picks_completed), (2, 3), "and the run's totals count them");
    }

    /// A run stopped on its simtime limit at a slice's end, 2 s: B passing the limit makes no slice ending there
    /// final, and as the stream closes the last slice takes in the order and the relay change at the limit. The order
    /// and the change that work committed before the stop files at 2.4 s, in the slice the limit opens, stay out.
    #[test]
    fn a_run_stopped_on_its_limit_at_a_slices_end_states_the_facts_at_the_limit_and_none_past_it() {
        let mut state = limited_to(2_000);
        let facts = [order_complete(1_500), order_complete(2_000), relay_change(2_000, true)];
        for fact in facts.iter().chain(&[order_complete(2_400), relay_change(2_400, false)]) {
            state.dispatch_event(fact);
        }
        state.finalize_passed_slices(SimTime::ms(2_500));
        assert_eq!(state.finality.final_upto, 1, "B past the limit makes final only the slice before it");
        state.finalize_remaining_slices(SimTime::ms(2_000));

        assert_eq!(state.slices().len(), 2, "two slices, the last ending at the limit");
        let last = &state.slices()[1];
        assert_eq!(last.scalars.orders_completed, 2, "the orders at 1.5 s and at the limit");
        assert_eq!(last.relay_state.transitions, [(7, 1_000, true)], "the relay change at the limit, at its end");
        assert_eq!(state.output.cumulative_scalars.orders_completed, 2, "the run's total");
    }

    /// A run whose simtime limit, 1 s, is a slice's end, but which stops past it, at its last fact at 1.5 s: the
    /// facts at the limit join the slice the limit opens, before the slice's own.
    #[test]
    fn the_facts_at_a_limit_the_run_stops_past_join_the_slice_the_limit_opens() {
        let mut state = limited_to(1_000);
        for fact in [relay_change(1_500, false), relay_change(1_000, true), order_complete(1_000)] {
            state.dispatch_event(&fact);
        }
        state.finalize_remaining_slices(SimTime::ms(1_500));

        let slices = state.slices();
        assert_eq!(slices.len(), 2, "two slices, the last cut at the stop");
        assert_eq!(slices[0].scalars.orders_completed, 0, "the first ends before the limit");
        assert_eq!(slices[1].scalars.orders_completed, 1, "the second begins at it");
        assert_eq!(slices[1].relay_state.transitions, [(7, 0, true), (7, 500, false)], "the change at the limit first");
    }
}
