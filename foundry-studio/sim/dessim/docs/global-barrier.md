# Global barriers

Status: trailing barrier BUILT as the acceptance trigger (the
"barrier-only" mode). Resource-specific pull and the combined mode are
roadmap; the leading barrier is PUNTED to the roadmap.

## Why a barrier exists

The scheduler drives a full-preplan simulation: no global sim clock
advances while it runs, and every task plans as far ahead as its inputs allow,
immediately. That destroyed the cencora runs (2026-08-26 diagnosis):
picks embargoed until t=7200s planned their claims at wall-start with
the embargo as a mere selection floor, and those far-future claims
ratcheted every frontier-shaped resource for the whole gap until the
congestion tripwires halted the run. It also left acceptance greedy:
a candidacy was granted the wall-instant its resource freed, so almost
every acceptance saw a choice pool of one.

The trailing global barrier gives the run a present, and — as the
acceptance trigger — a reason to wait: candidacies accumulate until
their instant arrives, so each acceptance chooses among whatever
competition built up while the barrier approached it.

## The trailing barrier

`Barrier` is owned by the `Scheduler` (it is scheduler state: a generic
clock over opaque proposals, with no domain types). Tasks and demand
modules reach it through `SchedCtx::barrier()`. Three faces:

- **Clock**: `floor()` is the settled frontier B, a `SimTime` starting
  at zero. It only advances at executor quiescence (below).
- **Floor**: no new claim may start before B. The accept surface
  enforces it generically — a proposal whose `start_time` sits behind B
  is a scheduler bug and halts the run. An advance can never strand a
  pending candidacy: both advance rules below bound it by the earliest
  live decision deadline (asserted in `advance_to`).
- **Await**: `wait_until(t)` suspends the caller until B >= t,
  registering t as an advance target while it waits. Level-triggered:
  the caller
  re-checks on every resume. With the barrier disabled, a `wait_until`
  strictly above B is an error — it would never wake.

### Barrier-triggered acceptance

At executor quiescence the scheduler runs one acceptance round:

1. Evict every pending proposal whose `poisoned()` is true — a
   conflicting state change killed it; its owner re-selects. Because a
   winner's acceptance poisons its same-resource competitors
   synchronously, a stale tie can never double-book behind an earlier
   winner.
2. Otherwise take the earliest of: the pending proposals' decision
   deadlines (`resolve_by`: the `start_time`, unless an entrant's
   need-by asks for the decision sooner) and the registered `wait_until`
   targets, a target at that instant first. The advance never passes
   any live candidacy's deadline: a due decision whose contenders are
   all inadmissible (`admissible()` false — e.g. zone inbound headroom
   exhausted) holds the clock and waits for in-flight work to free its
   family.
3. Advance B there and decide: accept the due proposal,
   or, when its family ranks a contest for the same claim
   (`ranked_competitors`), the best-ranked admissible contender, which
   may start later. Exactly one per round, so the next round sees the
   acceptance's consequences before choosing again.

If nothing is admissible and nothing is awaited while pending work
remains, B stays put and the deadlock alarm owns the situation.

All three faces read proposals only through the generic surfaces on
`Proposal` (`start_time`, `resolve_by`, `ranked_competitors`,
`poisoned`, `admissible`); the scheduler never touches a resource
family. The consumer split is a ruling: `Proposal` is pulled ONLY by the
trailing global barrier, on its decision deadline — a claim's start or
earlier, never an arrival or completion, since B advances to the
reported instant and no new claim may start behind B.
`ProposedReservation` is pulled ONLY by a resource demand (pull mode,
roadmap), ranked by the concrete type's own `Ord` — arrival, start,
whatever that family means by best — with the winner's whole Proposal
accepted. In barrier mode a family compares its own claims only inside
a contest it ranks: the HTM pool ranks a productive claim on a stay
that departs no later than the station's clearance ahead of it, and one
that would depart later behind it (`AvailableHtmRegistry`'s
`ranked_claims`). Both start at the operator's release, so that contest
is decided when B reaches the release (sooner if an entrant's need-by
asks). A delivery waiting in the station's queue for the cell has its
window onto the cell committed once a path off the cell is committed,
at that same B: the winner's drive, whose own candidacy is accepted in
a later round, or, when that drive is refused, the clearance's. Equal
deadlines outside a contest fall to task-creation order, deterministic
and merit-blind.

### Quiescence detection is deterministic

An acceptance must never ride the 50ms watchdog into a still-moving
system, so the scheduler does not judge quiescence for itself: it takes
the executor's own word for it.

- `Scheduler::run` builds the runtime with `Builder::on_thread_park`.
  tokio runs that hook immediately before the sole worker parks, and it
  reaches a park only when no task is runnable, no wake is deferred, and
  the supervise future is not already woken. Every firing is therefore
  the executor reporting one drained run queue.
- The hook bumps a park count and wakes the supervise loop, which runs
  one acceptance round on that wake. Waking from inside the hook cannot
  deadlock: work the hook schedules aborts the park instead of sleeping
  through it.
- The loop arms that wait only when a round could do something — after a
  round that found nothing to do it stops asking until the graph moves —
  so a quiet executor's parks never busy-wake the scheduler.
- Determinism: a park is a structural fact about the run queue, not a
  timeout. The wall clock still carries only the stop-signal check and
  the stall alarm; the sim-time limit is checked only on graph and
  drained wakes.

This replaced a fingerprint probe that snapshotted an activity counter,
yielded once, and compared. One `yield_now` covers at most 61 task polls
(tokio's per-round `LocalSet` budget), so the probe could call a run
quiescent while woken tasks were still queued; and any wake that skipped
the `notified()` funnel — a raw waker, a barrier target registration —
never reached the counter at all.

### Configuration

`barrier.enabled` (default true). Disabled pins B at zero: `wait_until`
beyond zero refuses — which is why a run configured with
`pick.embargo_secs > 0` and the barrier disabled is refused at startup
(an embargo requires a present) — and no acceptance ever happens, so a
disabled barrier is only meaningful for compositions whose tasks
publish no proposals.

## Consumers

`PickDemand::schedule` opens with `ctx.barrier().wait_until(embargo)`:
picks are not built, started, or spawned until the embargo instant is in
the settled past. The demand's per-order embargo floors remain as local
honesty; after the gate opens they are subsumed by B.

The observer hears each advance in the event stream: the runner hands
`Scheduler::with_advance_listener` a closure that sends the current world
time, `Barrier::current_world_time`, as `SimEvent::BarrierAdvanced` each
time the scheduler advances B, before any acceptance at the new instant, so
the marker follows every fact sent before it. The current world time is
`floor()` read as the floor for what is emitted rather than for what is
admitted.

**The barrier rule.** No fact may be reported stamped earlier than B stands
when it is reported; a fact stamped exactly at B is allowed.
`SimEvent::stamp` gives each fact's instant. Every fact the world reports
reaches the stream through the run's `EventJournal`, by `emit` or through
the sink `EventJournal::sink` hands out, and the same closure tells the
journal of each advance before it sends the marker. The journal refuses a
fact stamped behind B as the fact is emitted: it panics, naming the fact,
its stamp and B, and the scheduler turns a task's panic into the run's
failure. The observer keeps its own check, as the stream's contract. The
marker travels in the stream with the facts, so a fact the observer reads
after a marker and stamped before it was reported after B had passed it. The
observer stops the run on it, through the run's stop signal, and the run
fails with `StreamViolation::BehindBarrier`, which names the fact, its stamp
and B. A bot's plan snapshot, reported once the scheduler stops, has no
instant of its own: its legs are the plan's future, and it is filed where B
stands.

**Each slice once, final.** A slice is final once B has passed its end. The
observer makes it final when it reads the advance of B that passes the
slice's end. It derives the slice's D30 duty, folds in its cell-visit share,
stamps its stock with the physical replay, makes it drawable, stamps its
wall-clock duration (the wall time between B passing the slice's start and B
passing its end) and gives it its relay busy time: how long the relays'
access cells were held across it, read from the holds navigation emits as
it commits each path (`SimEvent::CellHeld`, `SimEvent::CellReleased`). A
hold's end is reported with the path that ends it, which B has not reached,
so a hold still open fills the slice. Holds that contradict each other stop
the run as they are read: a hold of a cell an open hold still holds, as
`StreamViolation::CellHeldTwice`; a release of a cell no open hold holds, as
`StreamViolation::ReleaseWithoutHold`; a hold that ends before it begins, as
`StreamViolation::HoldEndsBeforeItBegins`; and a release before the hold it
ends began, as `StreamViolation::ReleaseBeforeHoldBegan`. It then hands the
slice to the run's recording, once, which is what anything watching the run
reads, with the journal rows due with it: those whose instant of finality B
has passed, which for a leg or a VTM cycle is its end, not its start. It adds
the slice's picks and completed orders to the progress figures a caller polls,
which then reach the slice's end. A fact that writes into a slice B has
passed, whether or not the slice has gone out yet, stops the run with
`StreamViolation::WriteIntoPassedSlice`. A record that describes simtime B
has passed, such as the bot inspector's record of an interval built once B
passed its close, or a putaway's station-cycle record reported at its fill,
goes in the later of the slices its start and B stand in. B's marker makes
no slice of its own: what is filed makes slices, records filed as B advances
included, so a slice nothing had made when B passed it is made final at the
first advance after one makes it. The slices B never passed are made final,
the same way, when the stream closes; in them a hold still open counts up to
the latest instant a hold on a relay's access cell or a station's
presentation cell begins or ends.

## Roadmap (design frame, not built)

- **Resource-specific pull.** A demand generator or family service pulls
  the best reservation for ITS resource when its condition fires (CONWIP
  below threshold at a pick station pulls the best-arriving candidacy
  for that station's presentation cell): it isolates the live
  `ProposedReservation`s for that one resource — all one concrete type —
  sorts them by that type's own `Ord` (each family legislates its own
  merit: a deliver by arrival, a VTM cycle by start), and accepts the
  winner's whole Proposal through the `SchedCtx::accept_proposal` /
  `evict_proposal` surfaces. This needs each family service to index its
  own live candidacies per resource — typed, domain-side. The combined
  mode runs pull with the barrier as cleanup, with per-acceptance
  attribution — every barrier-triggered acceptance is then a localized
  pull-hole bug report, and the test harness asserts barrier accepts are
  zero. Details to be settled once barrier-only mode is proven working.
- **A drain detector for a multithreaded executor.** The park hook is a
  current-thread mechanism: one worker, one run queue, one park. Ruled
  2026-08-27: the current-thread runtime is the deterministic mode;
  multithreaded mode owes NO determinism, and no third-party executor is
  adopted. The supervise loop consumes nothing but "the executor
  drained" as a monotone epoch, so the detector swaps behind that
  surface. `docs/multithreaded-quiescence.md` holds the research: why
  aggregated park hooks are unsound on tokio's pool, the per-task
  wake-state counting successor and its one open design point, the
  rayon-inside-polls move that needs no detector at all, and the
  rejected and surveyed alternatives.
- **Leading barrier**: "thou shalt not plan any thing after this
  simtime" — a B+W cap on plan-ahead. Punted: any W both under-serves
  some demands and still pulls the timeline forward by up to W at
  admission.
- **Interval-aware (gap-fillable) availability** for frontier-shaped
  families — LNS-style gap filling / history rewriting. Behind the
  barrier on the roadmap, not in front of it.

## Measurement

The retrieve family records its own laziness audit at acceptance
(`model/relay/acceptance_stats.rs`): choice depth — the winner plus its
live rivals on the accepted relay — the family-wide tentative pool,
and per-hour breakdowns. The scheduler's done line carries its
accept and poison-eviction counts; the barrier reports its advances.
Pre-barrier baselines (2026-08-27, choice depth avg 1.19–1.45, 73–87%
singleton) are the comparison floor.
