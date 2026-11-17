//! The barrier rule over a whole run: a fact reported after B has passed its
//! instant stops the run, refused by the journal as it is emitted, and the
//! run's error names the fact, its stamp and B. A fact that reaches the stream
//! past the journal's check is refused by the observer, which stops the run
//! too. Each run here holds one task that waits for B to reach an instant and
//! then reports one fact.

mod common;

use std::sync::Arc;

use dessim::events::{EventJournal, EventSink, OrderCompleteEvent, SimEvent};
use dessim::prelude::*;
use dessim::runner::RunnerHooks;

/// Waits for B to reach `wait_for`, then reports `fact`.
#[derive(Display)]
#[display("ReportOnceBReaches")]
struct ReportOnceBReaches {
    journal: EventJournal,
    wait_for: SimTime,
    fact: Option<SimEvent>,
}

#[async_trait(?Send)]
impl Task for ReportOnceBReaches {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(self.wait_for).await?;
        if let Some(fact) = self.fact.take() {
            self.journal.emit(fact);
        }
        Ok(())
    }
}

/// Run a run whose one task reports an order completed at `stamp_ms` once B
/// has reached 20 s, through the journal `reporter` picks given the run's own
/// journal and its event stream.
fn report_order_complete_at(
    stamp_ms: u64,
    reporter: impl FnOnce(EventJournal, Arc<dyn EventSink>) -> EventJournal,
) -> Result<(), String> {
    let fact = SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: stamp_ms });
    let task = |journal, stream| ReportOnceBReaches {
        journal: reporter(journal, stream),
        wait_for: SimTime::ms(20_000),
        fact: Some(fact),
    };
    common::run_one_task(task, RunnerHooks::default()).map(|_| ())
}

/// The run's own journal, which hears B advance.
fn the_runs_journal(journal: EventJournal, _: Arc<dyn EventSink>) -> EventJournal {
    journal
}

/// A journal of the task's own on the run's stream, which never hears B
/// advance, so only the observer stands between its facts and the run.
fn a_journal_deaf_to_b(_: EventJournal, stream: Arc<dyn EventSink>) -> EventJournal {
    EventJournal::new(Some(stream))
}

#[test]
fn a_fact_reported_behind_b_stops_the_run() {
    let error =
        report_order_complete_at(5_000, the_runs_journal).expect_err("a fact stamped behind B must stop the run");
    assert!(
        error.contains("ORDER_COMPLETE") && error.contains("stamped at 5s was emitted after B had reached 20s"),
        "the error names the fact, its stamp and B: {error}"
    );
}

/// The observer refuses a fact stamped behind B on its own, as the stream's
/// contract, when the fact reached the stream past the journal's check.
#[test]
fn the_observer_stops_a_run_when_a_fact_reaches_it_behind_b() {
    let error = report_order_complete_at(5_000, a_journal_deaf_to_b).expect_err("the observer must stop the run");
    assert!(
        error.starts_with("observer stopped the run")
            && error.contains("OrderComplete stamped at 5s was reported after B had reached 20s"),
        "the observer stopped the run, naming the fact, its stamp and B: {error}"
    );
}

#[test]
fn a_fact_reported_at_b_is_part_of_the_run() {
    report_order_complete_at(20_000, the_runs_journal).expect("a fact stamped where B stands is in the run");
}
