//! Within `templates`, `phases` is where an evaluation's wall clock goes (spec 05 section 4.3, steps 6 and 7, as the
//! API runs them; the evaluation-latency chore): each phase runs in a `tracing` span that carries its milliseconds of
//! wall clock, and every evaluation, whichever way it ends, logs one line at info level naming it, the outcome its row
//! was committed with and each phase it completed in event order, so a slow or failed redraw is read from the log
//! phase by phase rather than guessed at.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use studio_protocol::{EvaluationNumber, EvaluationState, Ulid};
use tracing::field::Empty;
use tracing::{Instrument, Span, info, info_span};

/// One evaluation's phases in event order, each with the wall clock it took.
#[derive(Debug, Default)]
pub struct Phases(Vec<(&'static str, Duration)>);

impl Phases {
    /// Run `work` as phase `name`, inside its span, keeping its wall clock.
    pub fn time<T>(&mut self, name: &'static str, work: impl FnOnce() -> T) -> T {
        let (span, started) = (span_of(name), Instant::now());
        let out = span.in_scope(work);
        self.ended(&span, name, started.elapsed());
        out
    }

    /// Await `work` as phase `name`, inside its span, keeping its wall clock.
    pub async fn time_async<T>(&mut self, name: &'static str, work: impl Future<Output = T>) -> T {
        let (span, started) = (span_of(name), Instant::now());
        let out = work.instrument(span.clone()).await;
        self.ended(&span, name, started.elapsed());
        out
    }

    /// A phase measured elsewhere: the wait for a thread of the compute pool, which only that thread sees end.
    pub fn record(&mut self, name: &'static str, took: Duration) {
        self.ended(&span_of(name), name, took);
    }

    /// The phases `later` kept, after these.
    pub fn append(&mut self, later: Phases) {
        self.0.extend(later.0);
    }

    /// Log evaluation `n` of `draft`'s one line: the outcome its row was committed with (`committed`, the state it
    /// holds once the run ends; `None` when it has no row), its whole wall clock, and each phase's.
    pub fn log(&self, draft: &Ulid, n: EvaluationNumber, committed: Option<EvaluationState>, total: Duration) {
        let outcome = match committed {
            Some(EvaluationState::Ready) => "published",
            Some(EvaluationState::Failed) => "failed",
            Some(EvaluationState::Superseded) => "superseded",
            // Nothing this run computed was recorded: its row could not be read or written, or is gone.
            Some(EvaluationState::InProgress) | None => "unrecorded",
        };
        let mut phases = String::new();
        for (name, took) in &self.0 {
            let _ = write!(phases, "{}{name}={}", if phases.is_empty() { "" } else { " " }, took.as_millis());
        }
        let total = total.as_millis() as u64;
        let line = "template evaluation timed in wall-clock milliseconds";
        info!(%draft, %n, %outcome, wall_clock_ms = total, phases_wall_clock_ms = %phases, "{line}");
    }

    fn ended(&mut self, span: &Span, name: &'static str, took: Duration) {
        span.record("wall_clock_ms", took.as_millis() as u64);
        self.0.push((name, took));
    }
}

fn span_of(name: &'static str) -> Span {
    info_span!("evaluation_phase", phase = name, wall_clock_ms = Empty)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Phases;

    /// The phases keep event order across a computation's own and the wait measured around it.
    #[test]
    fn phases_keep_event_order() {
        let mut phases = Phases::default();
        phases.time("row", || ());
        phases.record("pool", Duration::from_millis(7));
        let mut computed = Phases::default();
        assert_eq!(computed.time("evaluate", || 42), 42);
        phases.append(computed);
        let names: Vec<&str> = phases.0.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["row", "pool", "evaluate"]);
        assert_eq!(phases.0[1].1, Duration::from_millis(7));
    }
}
