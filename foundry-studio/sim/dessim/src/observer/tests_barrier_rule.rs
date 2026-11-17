//! The barrier rule as the observer holds it: a fact read after an advance
//! of B and stamped before it was reported after B had passed it, and the
//! observer stops the run on it, naming the fact and B.

use std::sync::mpsc;

use crate::SimTime;
use crate::events::{OrderCompleteEvent, SimEvent};
use crate::model::OrderId;
use crate::observer::time_slice::MetricsConfig;
use crate::sim_types::{StopReason, StopSignal};

use super::{SimObserver, StreamViolation};

fn order_complete(sim_time_ms: u64) -> SimEvent {
    SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms })
}

/// Read `events` in order, with the run's stop signal attached.
fn observe(events: Vec<SimEvent>) -> (Result<(), StreamViolation>, Option<StopReason>) {
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    let stop = StopSignal::new();
    let observed = SimObserver::new(MetricsConfig::default(), rx).with_failure_stop(stop.clone()).run();
    (observed.map(|_| ()), stop.take_reason())
}

#[test]
fn a_fact_stamped_behind_b_stops_the_run_and_names_the_fact_and_b() {
    let (observed, stopped) =
        observe(vec![SimEvent::BarrierAdvanced(SimTime::ms(5_000)), order_complete(4_000), order_complete(6_000)]);
    let violation = StreamViolation::BehindBarrier {
        fact: "OrderComplete",
        stamp: SimTime::ms(4_000),
        barrier: SimTime::ms(5_000),
    };
    assert_eq!(observed, Err(violation.clone()));
    assert!(
        matches!(&stopped, Some(StopReason::Error(message)) if *message == violation.to_string()),
        "the scheduler is told to stop: {stopped:?}"
    );
}

/// A fact may happen exactly where B stands, and B's own advances may run
/// past facts already read.
#[test]
fn a_fact_stamped_at_b_is_read() {
    let (observed, stopped) = observe(vec![
        order_complete(3_000),
        SimEvent::BarrierAdvanced(SimTime::ms(5_000)),
        order_complete(5_000),
        SimEvent::BarrierAdvanced(SimTime::ms(9_000)),
    ]);
    assert_eq!(observed, Ok(()));
    assert!(stopped.is_none(), "nothing stops the run: {stopped:?}");
}
