# Multithreaded quiescence

Status, ruled 2026-08-27: the current-thread runtime is the sim's
deterministic mode. A future multithreaded mode owes NO determinism — it
would be a throughput mode, and reproducible results keep coming from the
current-thread mode. No third-party executor is adopted. None of the work
below is scheduled. This document exists so that the day someone wants a
multithreaded mode, its research is already done rather than redone.

The mechanism in use today is `docs/global-barrier.md`, section
"Quiescence detection is deterministic": `Scheduler::run` builds the
runtime with `Builder::on_thread_park`, tokio fires that hook immediately
before its sole worker parks, and every firing is the executor stating
that it drained its run queue. The supervise loop consumes nothing but
"the executor drained" as a monotone epoch, so a different detector swaps
in behind that surface without touching the acceptance round.

Source cites below are to the vendored `tokio-1.52.0` and
`nexosim-1.0.0`, checked against those crates. The four testing runtimes
in the prior-art section were read from their upstream repositories and
are not re-checkable from this checkout.

## Why the park hook does not generalize

Each fact below is on its own enough to sink an aggregated park-hook
scheme on tokio's multi-thread scheduler.

**The hook fires before the worker decides whether it parks.**
`Context::park` calls the `before_park` callback and only afterwards
consults `Core::transition_to_parked` (`worker.rs:802-807`), which
returns `false` — no park — whenever the worker still `has_tasks()`
(`worker.rs:1196-1199`). A wake landing in this worker's LIFO slot or
local run queue between its last failed steal and that check leaves the
hook fired and the worker busy. The error also runs the harmless way:
when the worker's own deferred-wake list is non-empty the run loop takes
`park_yield`, which never calls the hook (`worker.rs:591-596`) — a missed
signal, not a false one.

**The hook carries no worker id, so N firings cannot be aggregated.** The
callback's bound is `Fn() + Send + Sync + 'static`, and on the
multi-thread runtime the same closure is called concurrently from every
worker with nothing to tell them apart. Counting firings and acting at N
is counting unidentified events.

**Tokio's idle bookkeeping decrements first and re-checks after.**
`Idle::transition_worker_to_parked` decrements the unparked count and
pushes the worker onto `sleepers` (`idle.rs:86-102`); only after it
returns does the caller run `notify_if_work_pending()`, which rescans
every steal queue and the injector (`worker.rs:1205-1217`,
`worker.rs:1451-1462`). So there is a real interval in which the count
reads "everyone parked" while a task sits in the injector waiting for an
unpark that has been decided but not yet delivered. That order is right
for tokio's own liveness and wrong for an outside observer — NexoSim
inverts it, below.

**Runtime metrics do not repair any of this.**
`worker_park_unpark_count` is the best stable primitive — odd means
parked, even active, and it is gated on `cfg_64bit_metrics` rather than
`tokio_unstable` (`metrics/runtime.rs:240`) — but N reads are N `Relaxed`
loads of values published in batches, so they lag, and no read order
gives a consistent snapshot while tasks migrate global-to-local,
local-to-global on overflow, and local-to-local by stealing.
`worker_local_queue_depth` is `tokio_unstable`
(`metrics/runtime.rs:397`). `num_alive_tasks` counts tasks that have
spawned and not yet exited, rather than runnable ones, and its own doc
concedes it may lack strong consistency on the multi-thread runtime
(`metrics/runtime.rs:52-58`). Nothing here should become a decision
input, including as a diagnostic someone could later promote to one.

Why even a sound drain signal would not restore determinism: the
multi-thread scheduler derives how many local tasks to poll between
global-queue checks from a moving average of measured poll durations
(`multi_thread/stats.rs:55-67`), so its task-selection order is
wall-clock-dependent by design. The current-thread scheduler uses a fixed
interval (`current_thread/mod.rs:128`).

## The leading successor: per-task wake-state counting on stock tokio

The bar is that the signal must be a proof, not a heuristic: an
acceptance taken on a false quiescent chooses from an incomplete proposal
pool, which makes the run wall-clock-dependent. The one mechanism that
clears that bar without owning the executor is to stop asking the runtime
anything and instrument the tasks instead. This crate has one production
spawn site, so there is exactly one place to wrap.

### The state machine

Wrap the spawned future in a proxy that owns a per-task state and hands
out its own waker. Five states, not a flag:

- `Idle` — parked on a poll's `Pending`, no wake outstanding.
- `Queued` — woken; the runtime holds it in a run queue, not yet polled.
- `Polling` — inside `poll`, no wake seen since this poll started.
- `PollingWoken` — inside `poll`, and a wake arrived mid-poll.
- `Done` — the future returned `Ready`; terminal.

`PollingWoken` is the state a lone `AtomicBool` cannot express. A waker
fired from another thread while the task is mid-poll races the end of
that poll: with one flag the wake is either swallowed (the poll's
`Pending` clears it, the task is queued, and the count says idle — a
false quiescent) or counted twice. Splitting "polling" from "polling, and
already re-queued" makes the end-of-poll transition total: `Polling` plus
`Pending` becomes `Idle` and decrements; `PollingWoken` plus `Pending`
becomes `Queued` and stays counted; anything plus `Ready` becomes `Done`
and decrements.

### The count and its zero transition

In flight is the number of tasks in `Queued | Polling | PollingWoken`,
held in one global count. Increment on the `Idle` to `Queued` edge,
before the task becomes visible to any run queue; decrement only on an
edge after which the task can no longer produce work without a further
wake. Increment-before-publication and decrement-after-quiescing make the
count over-approximate at every instant, so a zero is a stable fact
rather than a momentary reading.

The zero transition is claimed by the decrementing thread and never
sampled by an observer: whichever thread's decrement takes the count from
one to zero is the thread that reports the drain. That is what makes the
scheme independent of the worker count, and it keeps the same shape as
the park hook today — a report pushed out of the executor, not a poll of
it.

### Waiting on the report without waiting on yourself

The supervise loop is itself a future. Were it counted, it would be in
flight whenever it was polling and the count could never reach zero while
it was looking. Two things settle that together. The observer is outside
the count by construction: on the multi-thread runtime the `block_on`
future runs on the calling thread while spawned tasks run on the pool
(`multi_thread/mod.rs:83-86`). And the report is a monotone `u64` epoch
consumed by a level-triggered wait, not an edge — the observer snapshots
the epoch, does its work, then waits for "epoch moved past my snapshot",
so a drain landing while it works is not lost. That is the surface the
supervise loop already consumes, which is why the swap stays local.

### Coverage conditions, and how this crate already meets them

Soundness reduces entirely to covering every source of runnable work.
Three conditions, all currently held:

1. **Every task goes through the wrapper.** One production spawn site,
   `launch` in `scheduler/mod.rs`; a task spawned any other way is
   invisible to the count. `sim/dessim/clippy.toml` holds this with a
   `disallowed-methods` entry for `tokio::spawn`.
2. **No work escapes to a blocking pool.** `spawn_blocking` and
   `block_in_place` run work the wrapper never sees; both are in the same
   `disallowed-methods` list.
3. **No wake is deferred behind tokio's back.** `yield_now` and
   coop-budget exhaustion both call `context::defer(waker)`, which pushes
   the waker onto a per-worker `Vec<Waker>` and returns `Pending`
   *without ever calling wake* (`task/yield_now.rs:49`,
   `task/coop/mod.rs:396-398`, `runtime/scheduler/defer.rs`). A proxy
   waker sees the poll return `Pending`, decrements, and can report a
   drain while that waker is still sitting in the defer list. The
   budget half is closed by construction: `launch` wraps the future in
   `tokio::task::coop::unconstrained`, `Budget::unconstrained()` is
   `Budget(None)` (`task/coop/mod.rs:120-122`), and `Budget::decrement`
   on `None` always succeeds (`task/coop/mod.rs:413-427`), so
   `poll_proceed` never reaches its deferring branch for sim code.

Condition 3 is not a hole for the park hook at all — tokio refuses to
park a worker whose defer list is non-empty, so a deferred wake
suppresses that signal rather than falsifying it. It becomes a hole only
for a wake-counting successor.

### The open design point

`yield_now` defers a wake by that same route, and it is not fully banned.
The `clippy.toml` entry waives it per site for one anticipated case: a
windowed pathfinder yielding between path windows, which is a ruled
fairness exception. A wake-counting detector must therefore do one of
two things.

- Treat a task that deferred its waker as still in flight. This means the
  proxy has to observe the defer, and tokio exposes no hook for it.
- Or give that pathfinder a different fairness valve — one that suspends
  on something the proxy waker owns, such as a scheduler-side signal
  resumed on the next drain, rather than on `yield_now`.

The second is the cheaper answer and the one to design first. This is the
single unsettled question in the scheme.

## Parallelism without any of this

Heavy computation inside a poll does not suspend the task. A synchronous
`rayon` `par_iter` over path planning or geometry runs to completion
inside one `poll`, the task never returns `Pending`, and no waker fires —
so the run queue, the park hook, and any wake-counting successor all see
exactly what they see today. Quiescence semantics are untouched and no
seam work is owed. This is the cheap throughput move, and it widens the
axis that is genuinely wide (per-task compute) instead of the one that
costs a determinism argument (task interleaving).

## Rejected mechanisms

| Mechanism | Verdict |
| --- | --- |
| Park hook per worker, unaggregated | Says only "this worker had nothing to do a moment ago" — never a runtime-wide statement. |
| Park hook plus a shared parked count, fire at N | Unsound: the hook fires before the park decision, and the count is not a snapshot. |
| The same, re-validated inside the hook from `RuntimeMetrics` | Unsound and unrepairable from the public surface: no worker id, queue depth is `tokio_unstable`, and independent atomic loads cannot snapshot migrating tasks. |
| Polling `RuntimeMetrics` from outside | Unsound, and reintroduces the wall-clock dependence the barrier exists to remove. |
| `tokio_unstable` task hooks (`on_before_task_poll` and friends) | Blind under `LocalSet`, which wires up no task hooks (`task/local.rs:1151-1156`); and poll boundaries are not wake events. |
| tokio-console / `tracing` instrumentation | Diagnostic only; offers no synchronous zero-crossing edge. |
| `TaskTracker` | Tracks task completion, not runnability: a live parked task and a live queued task look the same. |
| Owning the executor, or adopting a third-party one | Surveyed, not taken — below. |

The executor-owning protocols are worth knowing rather than adopting.
They share one shape: the last worker out does not clear its own idle bit
— it re-checks the injector queue first and only then declares the pool
inactive, so the "count hits zero while a task is about to be enqueued"
window closes by ordering instead of by a second observation. Paired with
it is a sent-minus-received message counter kept thread-local and folded
into a global atomic immediately before the idle handshake, telling
"finished" apart from "stuck" at no hot-path cost. Getting either means
owning the run queues, the work-stealing search, and the park protocol.
The ruling declines that: it buys a sound quiescence signal on a
work-stealing pool and does not buy determinism, the harder half.

## Prior art

**NexoSim** (`nexosim-1.0.0`) is the closest match: a discrete-event
simulator on a work-stealing multi-threaded executor, whose module doc
states this same run-to-quiescence-then-advance-time architecture and
"treats deadlocking as a normal occurrence". Idleness is one atomic
bitfield of active workers, and `pool_is_idle()` is a single `Acquire`
load of it (`pool_manager.rs:191-195`); `try_set_worker_inactive`
deliberately leaves the bit set when the caller looks like the last
active worker and returns `false`, obliging that worker to re-check the
injector and call `set_all_workers_inactive` itself
(`pool_manager.rs:119-130`). It claims a *causal messaging* partial order
over message delivery, not determinism — grepping the vendored crate for
determinism, reproducibility, or repeatability returns zero hits.
Reference, not adopted.

**madsim, turmoil, shuttle and loom** all buy determinism by deleting
multi-threading. madsim runs one executor thread over a seeded,
randomized ready queue and advances a virtual clock once it drains.
turmoil gives each simulated host its own current-thread runtime plus
`LocalSet` on a single OS thread, and steps by a fixed simulated tick
rather than by quiescence. shuttle replaces the executor with a
single-threaded coroutine engine driven by a pluggable `Scheduler`
oracle. loom runs green threads under a C11 model checker with a
five-thread ceiling and no task spawn at all. All four are testing tools
and none generalizes to a worker pool. Reference, not adopted.

**Tokio's own paused clock** is this design, implemented internally: with
time paused and the runtime idle, the clock auto-advances to the next
pending timer. The implementation sits inside the park path, parks with a
zero timeout, and suppresses the advance when `did_wake()` reports that
work arrived during that park (`runtime/time/mod.rs:263-275`) — the same
re-check-before-declaring shape. `time::pause()` is documented as
requiring the current-thread runtime and refuses at runtime otherwise
(`time/clock.rs:326`). Reference, and the plainest statement available
that quiescence-driven time advance is a current-thread mechanism in
tokio.
