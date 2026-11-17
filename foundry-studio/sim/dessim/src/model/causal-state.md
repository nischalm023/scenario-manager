# Causal State Management in DES

## Original Design Note

> I'm creating this file as a design note to frame dump some thoughts about the fundamental issues with state management in discrete event simulation.

> The actor model does implement causal state management, but its awkward, because we really like to use shared state to simplify the mental model.

Yes. The actor model enforces causality through message ordering, but it forces you to express every data dependency as a message. Look at `Htm::loaded` (htm/mod.rs:122): to move a tote from relay to robot, the HTM sends a `MoveToteReq` through a `UniRequestor` to the Dispatcher, which updates `ToteState` in its god-object `BTreeMap<ToteId, ToteState>`. The HTM doesn't care about actors or messages -- it cares about one thing: "the tote is now with me." The `UniRequestor` / `Output` / `Mailbox` ceremony is pure friction. The data dependency is between the HTM's logical clock and the tote's state, not between two actors.

The Dispatcher struct (state.rs) is the smoking gun. It has 20+ fields spanning totes, relays, zones, rack claims, idle queues, visual state -- not because these things are conceptually related, but because the actor model forces all shared state into actor-owned buckets. The result is a god object that exists purely to satisfy the framework's ownership model.

> This is problematic when you are doing multithreaded simulation however. I believe that the actor-based des processes events for all threads monotonically (albeit nondeterministically in terms of ordering between threads) - meaning that no actor receives its event for a given time step until all other actors have received and processed their events for prior time steps. IE there's a global clock, and all actors step forward with that clock, albeit by different amounts, with commensurately different wakeups scheduled.

Correct for nexosim's model. nexosim 1.0 processes all events at time T before advancing to T+1. Within a single timestep, actor execution order is nondeterministic, but all actors at time T complete before any actor at time T+1 begins. The global clock is a synchronization barrier at every timestep.

This is **conservative synchronization** in the PDES literature. It's safe (never violates causality) but pessimistic: it forces all 472 robots to synchronize at every timestep boundary even when 460 of them have no causal relationship to each other. Two HTMs navigating to different stations on opposite sides of the warehouse share zero state, yet the global barrier forces them to march in lockstep.

> This strikes me as a sloppy way to model causality. I feel like perhaps a better way would be to expressly model that causality in the state management system, and to build out a new DES that uses async, but not actors, and whenever a piece of state is accessed, it is done so with a causal context which causes the state access to be suspended until the causality is satisfied

## Analysis: Is this just the actor model with more steps?

> is this a good model? is this just the actor model with more steps?

No. It's a fundamentally different execution model. Here's the distinction:

**Actor model:** Causality is implicit in message ordering. The framework ensures messages arrive in causal order by controlling the execution schedule. State lives inside actors; access is mediated by the framework's port/mailbox mechanism. The programmer thinks in messages.

**Causal state model (what you're describing):** Causality is explicit in the state primitives. Each piece of shared state knows its logical timestamp. Any task can access any state at any time, but the access blocks until the state's timestamp catches up. The framework's only job is advancing time and waking blocked tasks. The programmer thinks in state reads and writes.

The key semantic difference: actors serialize *all* interactions between two entities (because everything goes through a mailbox), while causal state serializes only the *actual data dependencies*. If HTM-5 and HTM-200 both read the ReservationTable but touch different cells, an actor model serializes those reads through the Dispatcher. Causal state lets them proceed concurrently because there's no data conflict.

This is a well-studied model. What you're describing is **Chandy-Misra-Bryant (CMB) conservative parallel DES**, expressed as Rust async primitives rather than message-passing channels. The core insight of CMB is identical to yours: processes advance independently and block only on actual data dependencies.

### What existing systems does this map to?

**Closest match: Chandy-Misra-Bryant (1979).** Each logical process (robot, station, dispatcher subsystem) has its own logical clock and advances independently. When it needs to read state produced by another process, it blocks until that state is available at the required timestamp. The framework detects when no process can advance (deadlock/quiescence) and advances the global clock.

**MVCC databases** are the data-structure analogy. Each "read" specifies a timestamp; the storage engine returns the version visible at that timestamp. Writes create new versions. PostgreSQL's MVCC is doing essentially what you want, but for database rows instead of simulation state.

**NOT Time Warp / optimistic PDES.** Time Warp lets processes speculate ahead and rolls back on causality violations. You've explicitly ruled out rollback, which is the right call -- rollback in a warehouse sim would mean un-picking totes and un-navigating robots, which is nonsensical.

**NOT Software Transactional Memory (STM).** STM provides atomicity and isolation but doesn't have a notion of logical time. You could build causal state on top of STM primitives, but STM alone doesn't give you what you want.

**Differential/timely dataflow** (Frank McSherry) is adjacent but operates on a different abstraction. Timely dataflow tracks logical timestamps through a dataflow graph and notifies operators when all input at a timestamp has arrived. The "notification" mechanism is analogous to your "state blocks until timestamp is satisfied." But timely dataflow is designed for streaming computation, not discrete event simulation. The programming model is dataflow operators, not imperative async -- the opposite direction from what you want.

## The Clock Advancement Problem

This is the hard part you've identified, and it's where CMB historically gets stuck.

**The deadlock problem:** If process A is blocked waiting for state X to reach T=5000, and the only process that can advance state X is process B, but process B is blocked waiting for state Y which only process A can advance -- classic deadlock. In pure CMB, this is solved with **null messages**: every process periodically announces "I have nothing to produce until at least T=N," which lets downstream processes unblock.

**In your model, this maps to:** each causal state cell needs a way to declare "I won't change before T=N." Without this, a reader at T=5000 has no way to know whether the state *will* be written at T=4999 or not. It blocks forever.

There are three approaches:

### Approach 1: Lookahead declarations

Each writer declares its minimum next write time. For a robot, this is computable: "I'm navigating to cell X, which takes 3.2 seconds; I won't touch any relay state until T_now + 3200ms." This lets all relay state readers at timestamps before T_now + 3200ms proceed without blocking.

This is exactly what the existing `ReservationTable` does -- `CellReservation` declares "robot R occupies cell C during [enter_ms, exit_ms)." That's a lookahead declaration. The reservation table is already a causal state primitive, just not generalized.

### Approach 2: Epoch-based advancement

Group state updates into epochs. Within an epoch, all writes happen; then the epoch closes and all reads for that epoch can proceed. This degenerates toward the global barrier approach you're trying to escape, but the epochs can be per-resource rather than global.

### Approach 3: Topology-driven advancement

The causal dependency graph in the warehouse sim is relatively static. HTMs depend on: relay state, operator state, tote state. VTMs depend on: rack state, relay state. The dispatcher depends on: idle queues. If you can statically declare these dependencies, the runtime can compute safe advancement frontiers without null messages.

**My recommendation: Approach 1 (lookahead) for the warehouse sim.** The domain has natural lookahead: travel time is deterministic, dropoff/collection times are fixed, processing time is known at operator-grant time. A robot always knows the minimum time before its next state interaction. This is a gift -- most DES domains don't have this property.

## Concrete Sketch: What This Looks Like in Rust

### The core primitive: `SimCell<T>`

```rust
/// A piece of simulation state with causal timestamp tracking.
/// Readers block until the cell's timestamp reaches their requested time.
/// Writers advance the cell's timestamp.
pub struct SimCell<T> {
    inner: Arc<SimCellInner<T>>,
}

struct SimCellInner<T> {
    value: RwLock<T>,
    /// The timestamp through which this cell's value is guaranteed stable.
    /// Readers at t <= horizon can proceed; readers at t > horizon block.
    horizon: AtomicU64,
    /// Wakers for tasks blocked on this cell.
    waiters: Mutex<BTreeMap<u64, Vec<Waker>>>,
}

impl<T> SimCell<T> {
    /// Read the value. Blocks (yields) until horizon >= requested_time_ms.
    pub async fn read(&self, at_ms: u64) -> SimReadGuard<'_, T> {
        // Fast path: horizon already past our timestamp
        if self.inner.horizon.load(Ordering::Acquire) >= at_ms {
            return SimReadGuard(self.inner.value.read().unwrap());
        }
        // Slow path: register waker and yield
        poll_fn(|cx| {
            let h = self.inner.horizon.load(Ordering::Acquire);
            if h >= at_ms {
                Poll::Ready(SimReadGuard(self.inner.value.read().unwrap()))
            } else {
                let mut waiters = self.inner.waiters.lock().unwrap();
                waiters.entry(at_ms).or_default().push(cx.waker().clone());
                Poll::Pending
            }
        }).await
    }

    /// Write a new value and advance the horizon.
    /// Wakes all tasks waiting for timestamps <= new_horizon.
    pub fn write(&self, at_ms: u64, f: impl FnOnce(&mut T)) {
        {
            let mut val = self.inner.value.write().unwrap();
            f(&mut *val);
        }
        self.advance_horizon(at_ms);
    }

    /// Declare "I won't modify this cell before `until_ms`."
    /// This is the lookahead / null-message equivalent.
    pub fn advance_horizon(&self, until_ms: u64) {
        let prev = self.inner.horizon.fetch_max(until_ms, Ordering::AcqRel);
        if until_ms > prev {
            let mut waiters = self.inner.waiters.lock().unwrap();
            // Wake all waiters whose timestamp is now satisfied
            let satisfied: Vec<u64> = waiters.range(..=until_ms).map(|(&k, _)| k).collect();
            for ts in satisfied {
                if let Some(wakers) = waiters.remove(&ts) {
                    for w in wakers { w.wake(); }
                }
            }
        }
    }
}
```

### The DES runtime: SimClock

```rust
/// Minimal DES runtime. Owns the event queue and drives the async executor.
pub struct SimClock {
    /// Priority queue of (timestamp_ms, task_id).
    /// Tasks are woken at their scheduled time.
    queue: BinaryHeap<Reverse<(u64, TaskId)>>,
    now_ms: u64,
}

impl SimClock {
    /// Sleep until `duration` has elapsed in simulation time.
    /// The calling task yields and is re-woken when the clock reaches now + duration.
    pub async fn sleep(&self, duration_ms: u64) { /* register in queue, yield */ }

    /// Current simulation time for the calling task.
    pub fn now_ms(&self) -> u64 { self.now_ms }
}
```

### HTM pick cycle: before and after

**Current nexosim actor model** (htm/mod.rs, ~200 lines of ceremony):

```rust
// 12 schedulable methods, each scheduling the next via cx.schedule_event()
// 7 UniRequestor ports wired to Dispatcher/OperatorManager mailboxes
// State machine enum (HtmState) to track position in the cycle
// HtmCycleContext threaded through every scheduled event

#[nexosim(schedulable)]
async fn loaded(&mut self, ctx: HtmCycleContext, cx: &Context<Self>) {
    // Send message to Dispatcher to update tote location
    self.move_tote.send(MoveToteReq {
        tote_id: ctx.tote_id,
        location: ToteLocation::WithRobot { robot_id: self.id },
    }).await;
    self.carrying_tote = Some(ctx.tote_id);
    self.state = HtmState::NavToStation;
    let tr = self.travel(self.pos, ctx.target_station_access, true, sim_ms);
    // Schedule next phase as a separate event
    cx.schedule_event(tr.duration, schedulable!(Self::arrive_at_station), ctx).unwrap();
}
```

**Causal state model** (what the same logic would look like):

```rust
/// An HTM's entire pick cycle as a single imperative async function.
/// No state machine enum. No port wiring. No scheduled callbacks.
async fn htm_cycle(
    id: RobotId,
    assignment: HtmAssignment,
    totes: &SimCell<BTreeMap<ToteId, ToteState>>,
    relays: &SimCell<BTreeMap<RelayId, Relay>>,
    operators: &SimCell<BTreeMap<StationId, OperatorState>>,
    clock: &SimClock,
    pathfinder: &Pathfinder,
) {
    let now = clock.now_ms();

    // Navigate to relay (pure computation, no shared state needed)
    let travel = pathfinder.find_path(pos, assignment.relay_access, ...);
    clock.sleep(travel.duration_ms).await;

    // Load tote: write tote state, advance its horizon
    clock.sleep(LOAD_TIME_MS).await;
    let now = clock.now_ms();
    totes.write(now, |t| {
        t.get_mut(&assignment.tote_id).unwrap().location =
            ToteLocation::WithRobot { robot_id: id };
    });

    // Navigate to station
    let travel = pathfinder.find_path(pos, assignment.station_access, ...);
    clock.sleep(travel.duration_ms).await;

    // Request operator: read operator state, block until our timestamp
    let now = clock.now_ms();
    loop {
        let guard = operators.read(now).await;
        if guard[&assignment.station_id].is_available() {
            drop(guard);
            operators.write(now, |ops| {
                ops.get_mut(&assignment.station_id).unwrap().grant(id);
            });
            break;
        }
        drop(guard);
        // Wait for operator state to change (horizon advance)
        operators.read(now + 1).await;  // blocks until next write
    }

    // Process picks
    clock.sleep(processing_time_ms).await;

    // Release operator, return tote to relay...
    // (same pattern: write shared state, sleep for travel/unload)
}
```

The cycle drops from ~200 lines across 12 methods with a state machine enum and 7 port wirings to ~40 lines of straight-line async code. The state machine is gone because the async function *is* the state machine -- Rust's async transform does what you were doing by hand with `HtmState`.

### What about contention?

The operator queue example above is naive -- the `loop` with `read(now + 1)` is a busy-wait in simulation time. A real implementation needs a `SimCondVar` or `SimChannel`:

```rust
/// A simulation-time condition variable.
/// Waiters block until the condition is signaled at their timestamp.
pub struct SimSignal {
    inner: Arc<SimSignalInner>,
}

impl SimSignal {
    /// Wait until signaled at or after `at_ms`.
    pub async fn wait(&self, at_ms: u64) { ... }
    /// Signal all waiters at timestamp `at_ms`.
    pub fn signal(&self, at_ms: u64) { ... }
}
```

The operator becomes:

```rust
// In htm_cycle:
let grant = operator_signal.wait(clock.now_ms()).await;
// ... processing ...
operators.write(clock.now_ms(), |ops| { ops.release(id); });
operator_signal.signal(clock.now_ms()); // wake next waiter
```

## How Arc<RwLock<>> Relates

The current `Arc<RwLock<>>` pattern in `ReservationTable` (reservation.rs) is a degenerate `SimCell` with no timestamp awareness. It provides shared access and mutual exclusion but has no concept of "this read is at time T" or "this state is stable through time T."

`ReservationTable` happens to work correctly because it encodes time *in the data* (each `CellReservation` has `enter_ms` / `exit_ms`) rather than in the access primitive. But this is ad-hoc -- every consumer has to manually reason about timestamps. The pattern doesn't generalize to the Dispatcher's `relay_states`, `totes`, `zones`, etc., where the time dimension is implicit in the execution order.

`SimCell<T>` is `Arc<RwLock<T>>` plus a horizon timestamp plus waiter notification. The existing `Arc<RwLock<>>` usage is halfway there structurally, but missing the key piece: temporal awareness.

## Concurrency By Default, Barriers When Needed

> my goal here is to enable concurrency by default, and only enforce barriers when expressly needed

In the warehouse sim, the natural concurrency/barrier structure is:

**Fully concurrent** (no shared state touched):
- HTM navigating to station + HTM navigating to different station
- VTM navigating to rack + any HTM doing anything
- All pathfinding computation (reads reservation table but via immutable snapshot)
- Processing time at station (timer only, no state interaction)

**Barrier points** (shared state mutation):
- Relay reservation: two VTMs can't reserve the same relay. This is a write to `SimCell<BTreeMap<RelayId, Relay>>` -- the write serializes, readers at later timestamps block.
- Tote state transitions: `move_tote` changes tote location. One writer at a time.
- Operator grant: one station presentation slot. FIFO queue serializes access.
- Rack claims: VTM claiming a rack cell for pickup. Write serializes.
- Idle queue registration: HTM/VTM registering as idle for dispatch. Write serializes.

The key insight: **most simulation time is spent in navigation (sleep) and pathfinding (pure computation).** The barrier points are brief writes that happen at discrete instants. Between barriers, hundreds of robots advance concurrently. This is exactly the workload profile where causal state shines -- the barrier-to-compute ratio is very low.

## Relaxing Strict Determinism

> I will say that strict determinism is a non-goal, but causal consistency is a goal.

The current sim is deterministic: `BTreeMap` everywhere, sorted iteration, fixed seeds. Relaxing this:

**What you gain:**
- `HashMap` instead of `BTreeMap` for hot paths (relay_states, totes) -- measurable speedup at 400+ robots
- No need to sort idle queues before matching -- first-come-first-served in simulation time, with ties broken arbitrarily
- Multi-threaded execution without deterministic scheduling overhead
- Simpler code: no artificial ordering constraints that exist only for reproducibility

**What you lose:**
- Exact reproducibility across runs. Two runs with the same input may produce slightly different throughput numbers because tie-breaking at the same timestamp is nondeterministic.
- Easier debugging: "run it again and set a breakpoint" only works if the same execution path repeats.

**What you keep (causal consistency guarantees):**
- If event A causally precedes event B, A always happens before B.
- No robot reads stale state -- every read sees all writes that causally precede it.
- Aggregate metrics (throughput, utilization) converge to the same distribution across runs. Individual event orderings within the same timestamp may differ, but the statistical properties are stable.

For a throughput simulator, causal consistency without strict determinism is the right tradeoff. You're computing aggregate statistics, not proving that a specific event sequence occurs. The small nondeterminism within a timestep is noise in a system processing 4,000 picks/hour.

## Practical Recommendation

Given where the codebase is heading (thin actors, shared model with `Arc<RwLock<>>`), here's the minimum viable path:

### Step 1: Build `SimCell<T>` and `SimClock`

These are ~200 lines total. `SimCell<T>` wraps any state with a timestamp horizon and waiter notification. `SimClock` is a priority queue that drives async task scheduling.

This is a new crate -- call it `causalsim` or `simrt`. It has no dependency on nexosim, the warehouse domain, or any business logic. Pure simulation runtime.

### Step 2: Port one robot type as a proof of concept

Take the VTM (simpler cycle than HTM: rack -> relay -> relay -> rack). Write it as a single `async fn vtm_cycle(...)` that uses `SimCell` reads/writes and `SimClock::sleep()`. Compare the code clarity and line count against the current nexosim VTM actor.

### Step 3: Solve the clock advancement problem for your domain

The warehouse sim has a crucial property: **all delays are computable in advance.** Travel time is deterministic (A* gives exact duration). Load/unload/processing times are known. This means every robot can declare its lookahead ("I won't touch shared state until T_now + travel_time_ms") immediately after starting navigation. This makes horizon advancement trivial and avoids the CMB deadlock problem entirely.

The advancement rule is: after every `clock.sleep()`, advance the horizon on all `SimCell`s that the robot *could* write to by the robot's earliest possible next interaction time. In practice, robots interact with at most 2-3 state cells per cycle phase (totes, one relay, one operator). The lookahead graph is sparse.

### Step 4: Worry about multi-threading later

The `SimCell` design supports multi-threading (it's `Arc`-based with atomic horizons and waker notification), but start single-threaded. The concurrency win at 472 robots comes from not having the global timestep barrier, even on a single thread -- tasks that don't share state don't need to synchronize.

Multi-threaded execution is an optimization for when single-threaded causal simulation isn't fast enough. The same `SimCell` / `SimClock` primitives work either way.

### What NOT to build

- **Don't build a general-purpose PDES framework.** You need causal state for a warehouse sim, not a general simulation kernel. Keep it domain-aware. The fact that all delays are deterministic is a massive simplification -- exploit it, don't abstract it away.

- **Don't build MVCC / versioned state.** You don't need multiple versions of relay state -- there's only one version, and readers either see it (horizon satisfied) or block. MVCC is for concurrent reads at different timestamps, which doesn't apply when your reads are "give me the current value as of time T" rather than "give me a consistent snapshot at time T while writes at T+1 are in progress."

- **Don't build a `CausalCell<T>` that tracks read/write sets for conflict detection.** That's STM territory. The warehouse sim's conflict structure is simple enough to express with `SimCell` read/write and `SimSignal` for notifications. You don't need automatic conflict detection.

## Summary

The idea is good. It's not the actor model with more steps -- it's a genuinely different execution model that eliminates the god-object problem, the port-wiring ceremony, and the global timestep barrier, while giving you imperative async code that reads like sequential business logic.

It maps to Chandy-Misra-Bryant conservative PDES with lookahead, which is well-understood theory. The warehouse domain is particularly favorable because delays are deterministic (no speculative execution or rollback needed) and the dependency graph is sparse (robots are mostly independent).

The implementation is small: `SimCell<T>` (~100 lines), `SimClock` (~100 lines), `SimSignal` (~50 lines). The hard work is porting the business logic from actor callbacks to imperative async, but each port *deletes* code rather than adding it.
