//! Shared event output and cycle identities for one simulation. Task families
//! receive this handle without gaining access to unrelated warehouse state.
//! Every fact the world reports reaches the run's event stream through it, by
//! `emit` or through the sink it hands out, and it refuses one stamped behind
//! B, which the scheduler publishes to it as B advances.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{DispatchRejectionEvent, DispatchRejectionReason, EventSink, SimEvent};
use crate::model::{PresentationId, SimTime, VtmCycleId};
use desgraph::StationId;

#[derive(Clone)]
pub struct EventJournal(Arc<Inner>);

struct Inner {
    sink: Option<Arc<dyn EventSink>>,
    next_presentation: AtomicU64,
    next_vtm_cycle: AtomicU64,
    /// B as the scheduler last published it, in milliseconds, since an atomic
    /// holds no `SimTime`.
    barrier_ms: AtomicU64,
}

impl EventJournal {
    pub fn new(sink: Option<Arc<dyn EventSink>>) -> Self {
        Self(Arc::new(Inner {
            sink,
            next_presentation: AtomicU64::new(0),
            next_vtm_cycle: AtomicU64::new(0),
            barrier_ms: AtomicU64::new(0),
        }))
    }

    /// The scheduler advanced B to `at`: every fact emitted after this call,
    /// in scheduler time, is stamped at or after it.
    pub fn barrier_advanced(&self, at: SimTime) {
        self.0.barrier_ms.store(at.as_ms(), Ordering::Relaxed);
    }

    /// B as the scheduler last published it (simtime).
    pub fn barrier(&self) -> SimTime {
        SimTime::ms(self.0.barrier_ms.load(Ordering::Relaxed))
    }

    /// A sink for the parts of the world that report through one rather than
    /// through the journal: the inventory and the stripe plan. It refuses a
    /// fact stamped behind B as `emit` does. None when the run has no stream.
    pub fn sink(&self) -> Option<Arc<dyn EventSink>> {
        self.0.sink.is_some().then(|| self.0.clone() as Arc<dyn EventSink>)
    }

    pub fn next_presentation_id(&self) -> PresentationId {
        PresentationId(self.0.next_presentation.fetch_add(1, Ordering::Relaxed))
    }

    pub fn next_vtm_cycle_id(&self) -> VtmCycleId {
        VtmCycleId(self.0.next_vtm_cycle.fetch_add(1, Ordering::Relaxed))
    }

    pub fn emit<T: Into<SimEvent>>(&self, event: T) {
        self.0.send(event.into());
    }

    pub fn emit_dispatch_rejection(
        &self,
        station_id: StationId,
        at: SimTime,
        reason: DispatchRejectionReason,
        count: u64,
        detail: impl FnOnce() -> Option<String>,
    ) {
        // The detail is spelled out only for a stream that will carry it.
        if self.0.sink.is_some() {
            self.emit(SimEvent::DispatchRejection(DispatchRejectionEvent {
                station_id,
                at,
                reason,
                count,
                detail: detail(),
            }));
        }
    }
}

impl EventSink for Inner {
    /// Send `event` on to the run's stream unless B has passed its stamp. A
    /// fact stamped behind B claims something happened in simtime the
    /// scheduler had already closed, so the code that emitted it is wrong and
    /// no caller could act on an error: it panics, naming the fact, its stamp
    /// and B. The scheduler turns a task's panic into the run's failure. The
    /// observer still refuses such a fact on its own
    /// (`StreamViolation::BehindBarrier`), as the stream's contract.
    fn send(&self, event: SimEvent) {
        let Some(sink) = &self.sink else { return };
        let barrier = SimTime::ms(self.barrier_ms.load(Ordering::Relaxed));
        if let Some(stamp) = event.stamp().filter(|stamp| *stamp < barrier) {
            panic!("{event} stamped at {stamp} was emitted after B had reached {barrier}");
        }
        sink.send(event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::events::OrderCompleteEvent;
    use crate::model::OrderId;

    fn order_complete(sim_time_ms: u64) -> SimEvent {
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms })
    }

    /// A fact stamped where B stands goes out; B is where the scheduler last
    /// advanced it.
    #[test]
    fn a_fact_stamped_where_b_stands_goes_out() {
        let (tx, facts) = mpsc::channel();
        let journal = EventJournal::new(Some(Arc::new(tx)));
        journal.barrier_advanced(SimTime::ms(20_000));
        journal.emit(order_complete(20_000));
        assert_eq!(facts.try_iter().count(), 1);
    }

    /// A fact stamped behind B is refused as it is emitted, and the panic
    /// names the fact, its stamp and B.
    #[test]
    #[should_panic(expected = "stamped at 5s was emitted after B had reached 20s")]
    fn a_fact_stamped_behind_b_is_refused_as_it_is_emitted() {
        let (tx, _facts) = mpsc::channel();
        let journal = EventJournal::new(Some(Arc::new(tx)));
        journal.barrier_advanced(SimTime::ms(20_000));
        journal.emit(order_complete(5_000));
    }

    /// A dispatch rejection stamped behind B is refused as any other fact is.
    #[test]
    #[should_panic(expected = "stamped at 5s was emitted after B had reached 20s")]
    fn a_dispatch_rejection_stamped_behind_b_is_refused() {
        let (tx, _facts) = mpsc::channel();
        let journal = EventJournal::new(Some(Arc::new(tx)));
        journal.barrier_advanced(SimTime::ms(20_000));
        let blocked = DispatchRejectionReason::OrderAdmissionBlocked;
        journal.emit_dispatch_rejection(StationId(1), SimTime::ms(5_000), blocked, 1, || None);
    }

    /// The sink the journal hands out refuses a fact stamped behind B as
    /// `emit` does, so what reports through it is held to B too.
    #[test]
    #[should_panic(expected = "stamped at 5s was emitted after B had reached 20s")]
    fn its_sink_refuses_a_fact_stamped_behind_b() {
        let (tx, _facts) = mpsc::channel();
        let journal = EventJournal::new(Some(Arc::new(tx)));
        journal.barrier_advanced(SimTime::ms(20_000));
        journal.sink().expect("the run has a stream").send(order_complete(5_000));
    }
}
