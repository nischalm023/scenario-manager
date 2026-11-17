# FHSippPathfinder Algorithm Draft

Status: draft. This document describes `FHSippPathfinder`, the intended
full-horizon SIPP pathfinder after the single-shot reservation-aware changes
are complete. It is written in present tense as a reference for the completed
design. Since foreshortening on queue cells, what it describes is one window
of a `reserve_path` call: while another robot holds the target, or a queue
cell feeding it, open-ended, the call commits a window that ends on a queue
cell, awaits (scheduler time) that hold's bounding, and continues
(`sim/pathfinder/src/fh_sipp/allocator.rs`). Phase 2 HTM trapezoidal kinematics is intentionally out of scope,
although the design preserves the state-space seams that Phase 2 uses.

## Scope

`FHSippPathfinder` computes a time-valid path for one robot through the
`WarehouseGraph` and commits the corresponding cell and edge reservations to
the reservation table. The planner runs inside an offline deterministic
discrete-event simulator. The reservation table is static for the duration of a
single pathfinding call, and no other robot commits concurrently while that
call is searching.

`FHSippPathfinder` is a single-agent prioritized planner: each call plans one
robot against all paths already reserved by previously planned robots.
Multi-robot coordination emerges from the reservation table ordering, not from
a joint multi-agent search.

`FHSippPathfinder`:

- uses full-horizon Safe Interval Path Planning (SIPP) for reservation-aware
  A* search;
- checks reservations for every searched cell, not only a prefix;
- commits the returned path all-or-nothing;
- emits explicit `WaypointAction` timing for move, rotate, wait, start, and
  end markers;
- derives cell and edge reservation requests from the waypoint chain;
- rejects infeasible paths cleanly with `Unreachable` or a commit-consistency
  error;
- does not use optimistic tails, frontier widening, or a hard-prefix/soft-tail
  commit split within a window; a foreshortened call commits several windows,
  each a whole search and one commit, never a speculative tail.

Historical note: the predecessor implementation used a `collision_depth`
window. A* validated only the first `C` composed cells against reservations,
then the allocator attempted to commit the remaining path optimistically and
extend from the last committed frontier on conflict. `FHSippPathfinder`
removes that architecture because the frontier cell cannot always be legally
held open for the next iteration. That is also why a foreshortened window
ends only on a queue cell: the map makes a queue's edges destination-only, so
a robot holding one open-ended stops no traffic bound elsewhere.

## Literature Basis

The core search model follows Phillips and Likhachev's SIPP work: a dynamic
environment is represented by maximal safe intervals, and search states are
identified by a spatial configuration plus the safe interval that admits the
robot at that configuration. The key property is that the number of safe
intervals for a configuration is finite and usually small, while the number of
safe timesteps can be unbounded. SIPP therefore avoids the horizon-dependent
state explosion of `(cell, time)` A* while preserving optimality and
completeness under its action assumptions. See Phillips and Likhachev, "SIPP:
Safe Interval Path Planning for Dynamic Environments", ICRA 2011:
https://www.ri.cmu.edu/publications/sipp-safe-interval-path-planning-for-dynamic-environments/

The reservation-table coordination model is related to Cooperative A*,
Hierarchical Cooperative A*, and WHCA*, but the simulator uses it in a
different regime. Silver's Cooperative Pathfinding paper describes
reservation-table planning and an abstract reservation-ignored heuristic for
space-time cooperative search. `FHSippPathfinder` keeps the useful single-agent
reservation-table idea and rejects the rolling window as an offline-simulator
requirement. See Silver, "Cooperative Pathfinding", AIIDE 2005:
https://ocs.aaai.org/Papers/AIIDE/2005/AIIDE05-020.pdf

The design deliberately does not implement CO-WHCA*, RHCR, PIE, or Real-Time
LaCAM semantics. Those algorithms address online planning and execution loops,
where computation is interleaved with physical movement, new information
arrives over time, and small commits preserve future flexibility. In this
simulator, the pathfinder has the current reservation table up front, but an
open-ended hold is a departure not yet scheduled (scheduler time): a journey
such a hold blocks cannot be planned whole, so `reserve_path` ends a window
before it, on a queue cell, which can be held open-ended. Beyond that, the
argument for windowing is wallclock cost. CO-WHCA* and RHCR remain useful
comparison points: Bnaya and Felner's CO-WHCA* explicitly focuses an online
WHCA* window around conflicts, and Li et al.'s RHCR decomposes lifelong
warehouse MAPF into repeated bounded-window instances. See:

- Bnaya and Felner, "Conflict-Oriented Windowed Hierarchical Cooperative A*",
  ICRA 2014: https://tzin.bgu.ac.il/~felner/2014/COWA6p.pdf
- Li et al., "Lifelong Multi-Agent Path Finding in Large-Scale Warehouses",
  AAAI 2021:
  https://www.ri.cmu.edu/publications/lifelong-multi-agent-path-finding-in-large-scale-warehouses/

The higher-level scheduler may still use parking, non-task endpoints, or dummy
paths as fallback behavior. That is a scheduler concern, not a replacement for
full-horizon low-level deconfliction. The MAPD literature formalizes this with
well-formed instances and non-task endpoints. See Ma et al., "Lifelong
Multi-Agent Path Finding for Online Pickup and Delivery Tasks", AAMAS 2017:
https://www.ri.cmu.edu/publications/lifelong-multi-agent-path-finding-for-online-pickup-and-delivery-tasks/

The Phase 2 kinematic extension follows the SIPP-IP family by adding velocity
state and kinodynamic feasibility. Phase 1 does not include this, but its
`SippState` / `SearchKey` and `NodeMeta` layout are compatible with adding
`speed_bucket` and real entry/exit velocities later. See Ali and Yakovlev,
"Safe Interval Path Planning With Kinodynamic Constraints", AAAI 2023:
https://arxiv.org/abs/2302.00776

## Public Model

`FHSippPathfinder` has two conceptual modes:

1. No-oracle motion-timed pathfinding. This computes movement and rotate timing
   through the static graph without consulting the reservation table. It is
   used for ghost-mode path estimates, static heuristics, and travel-time
   queries. It still has physical move and turn timings; it simply ignores
   dynamic reservations and waits caused by other robots.
2. Reservation-aware pathfinding. This runs full-horizon SIPP-A* against a
   `ReservationOracle`, produces an exact waypoint chain, composes a
   `CommitPlan`, and commits that plan atomically.

The main reservation-aware inputs are:

- `from`: starting cell;
- `from_orientation`: starting orientation;
- `to`: goal cell;
- `robot_type`: HTM or VTM traversal rules;
- `carrying_cargo`: cargo-sensitive graph traversability;
- `self_robot`: robot id used for reservation ownership;
- `sim_start`: action-ready time at `from`;
- `dest_dwell`: terminal hold policy;
- `max_wait`: maximum planning horizon for waiting;
- `max_iterations`: search expansion guard;
- optional `predecessor`: an existing open-ended hold at `from` that this path
  closes atomically when it leaves the starting cell.

The output is a `ReservedPath` containing:

- the `Path`, expressed as contiguous `Waypoint`s;
- `SearchStats`, including expansion and oracle-query counters;
- the committed `CellReservation`s.

## Time Semantics

Every SIPP search state has a `g_score`. In Phase 1:

```text
g_score[state] = action-ready time at state.cell
```

For a `Move`, the action starts when the robot's leading edge enters the
destination cell and ends when the robot is action-ready at the destination
cell center:

```text
Move.start = leading-edge entry into destination cell
Move.end   = action-ready at destination cell
```

For a `Rotate`, the action starts at the current action-ready time and ends
after `turn_duration`.

For a `Wait`, the action starts at the prior action end and ends when the robot
is action-ready again in the same cell. Waits do not change orientation.

`PathStart` and `PathEnd` are zero-width markers. `PathEnd` marks an
open-ended terminal hold; it is not itself a reservation end.

Phase 1 uses constant move duration per edge, derived from the graph edge
distance and robot speed. Phase 2 replaces that constant with trapezoidal
kinematic cost, but the meaning of `g_score`, `Move.start`, and `Move.end`
does not change.

## Safe Intervals

A safe interval is a maximal contiguous time window during which a cell is free
of conflicting reservations for this robot. Its identity is canonical:

```text
SafeIntervalId = (free_start, free_end)
```

`free_end = None` means the interval is open-ended.

The reservation oracle exposes:

```text
entry_intervals(cell, after, horizon, predecessor) -> Vec<SafeIntervalId>
```

The returned interval identity is not clipped by `after`. If the physical free
interval is `[1000, 5000)`, then queries with `after = 1000`, `after = 2000`,
and `after = 4500` all identify the same interval as `(1000, Some(5000))`.
Expansion-time timing is clipped separately:

```text
action_start = max(free_start, current_action_ready_time)
```

This stability is required for correct SIPP deduplication.

The oracle applies the same gap/overlap semantics to interval construction and
conflict checking. If reservations separated by less than the configured gap
are considered conflicting, the safe interval enumeration removes that gap from
the free interval as well.

## Search State

Reservation-aware A* uses a SIPP state plus a small wait label:

```text
SippState {
    cell: CellId,
    orientation: Orientation,
    interval: SafeIntervalId,
}

SearchKey {
    sipp: SippState,
    wait_label: Option<SimTime>,
}
```

The ordinary SIPP identity is `(cell, orientation, interval)`. Time is not a
general state axis. Two ordinary arrivals at the same `(cell, orientation,
interval)` are represented by one state; the earliest `g_score` dominates later
ordinary arrivals inside the same interval. The later arrival cannot produce a
successor earlier than the earlier arrival, and any wait needed to use a later
immediate-neighbor interval is available to the earlier arrival as long as the
current safe interval admits the wait.

`wait_label` is `None` for normal SIPP states. It is `Some(event_time)` only
for explicit event-driven wait states. This preserves a small, finite set of
later departure times that are not naturally generated by immediate-neighbor
safe interval enumeration. The label is event-driven, not a tunable time
bucket: it is created only from reservation event times selected by the wait
successor logic.

The search stores per-state metadata:

```text
NodeMeta {
    action_kind: Start | Move | Rotate | Wait,
    action_start: SimTime,
    action_end: SimTime,
    source_cell: Option<CellId>,
    interval: SafeIntervalId,
}
```

`NodeMeta` is the authoritative reconstruction record. Reconstructing the final
path does not infer action kind from g-score deltas; it reads the metadata
written when the successor was emitted.

Phase 1 includes explicit `Wait` metadata when the search intentionally pushes
an event-labeled wait state. It also emits reconstruction waits for gaps caused
by delayed move or rotate actions. Both forms produce the same final
`WaypointAction::Wait` shape.

## Heuristic

The heuristic is a lower bound on remaining action-ready time to the goal. The
baseline implementation uses:

```text
h(cell, goal) = L1_grid_distance(cell, goal) * traverse_duration
```

This is admissible but weak. It ignores turns, orientation, directed graph
constraints, robot-type traversability, zones, reservations, waits, and
destination dwell.

The preferred stronger heuristic is a no-oracle motion-timed distance field:

```text
h(cell, orientation, goal) =
    shortest remaining move+turn time from (cell, orientation) to goal
    on the static graph,
    ignoring reservations, waits, and dynamic obstacles
```

This is computed by reverse Dijkstra or reverse A* from the goal over the
static graph and cached per `(goal, robot_type, carrying_cargo)` configuration.
Lookup during SIPP is O(1). It is not computed by running a no-oracle path
search from every SIPP state, and it is not based on deviation from one
precomputed no-oracle path.

This heuristic is admissible because every reservation-aware path consists of a
static route plus non-negative waits. Ignoring reservations and waits relaxes
the problem, so the relaxed cost-to-go is never greater than the true
reservation-aware remaining cost.

An orientation-aware distance field is tighter because it includes turn costs.
A lower-dimensional 2D field that ignores orientation is still admissible and
matches the shape used in the original SIPP implementation, where the
lower-dimensional heuristic is intentionally cheaper and underestimates the
lattice planner's true cost. Silver's HCA* uses the same principle: compute
abstract distances in a domain that ignores the time dimension and reservation
table.

Optional tightening can add release-time floors, for example a lower bound
derived from the earliest goal safe interval that admits the terminal dwell.
Such time-dependent bounds remain lower bounds only if they are formulated
against the goal reservation semantics precisely. The static no-oracle
distance field is the first upgrade because it is simple, cacheable, and
directly literature-backed.

## Successor Expansion

The search pops the minimum `f = g + h` state from the open heap, skips stale
heap entries, and expands the state until it extracts a valid goal or reaches
`max_iterations`.

### Rotate Successors

A rotate successor stays in the same cell and safe interval:

```text
action_start = current_action_ready_time
action_end   = action_start + turn_duration
next_state   = (current.cell, target_orientation, current.interval)
```

The rotate is emitted only if:

- the target orientation faces a traversable neighbor, or is a perpendicular
  intermediate needed to synthesize a 90- or 180-degree rotation toward such
  a neighbor. The U-turn case is explicit: when the current cell has a
  traversable edge opposite the current orientation, perpendicular
  intermediates must be admitted even though the current orientation itself
  faces a reachable neighbor — otherwise a bot in a long bidi corridor
  facing the wrong way can never reach the opposite side;
- the rotate duration fits inside the current safe interval;
- the reservation oracle confirms the current cell is available for the rotate
  window, excluding the predecessor reservation when appropriate.

The rotate successor does not advance the robot to a new cell. Phase 1 no
longer uses `cell_step` for reservation gating; any remaining move-count stats
are diagnostic only.

### Move Successors

A move successor goes from the current cell into a traversable neighbor. The
neighbor must be reachable by the robot type and cargo state, and the current
orientation must face the neighbor.

For each canonical safe interval on the neighbor:

```text
m_in_start = max(neighbor.free_start, current_action_ready_time)
m_in_end   = m_in_start + traverse_duration
```

The destination cell reservation begins at `m_in_start`, not at `m_in_end`.
This models the leading edge entering the destination cell.

For non-goal intermediate cells, the search validates the minimum composed
reservation needed to enter the destination cell and immediately leave it:

```text
minimum_destination_hold = [m_in_start, m_in_end + next_traverse_duration)
```

With constant-speed Phase 1 motion, `next_traverse_duration` is the immediate
move-out duration. Phase 2 computes this from kinematics.

For the goal cell, validation uses `dest_dwell`:

- `dest_dwell = None` or `Some(MAX)`: the goal reservation is open-ended, so
  the goal safe interval must be open-ended.
- `dest_dwell = Some(d)` finite: the goal reservation is
  `[m_in_start, m_in_end + d)`, and that full interval must fit.

If `m_in_start` is later than the current action-ready time, the robot waits in
the source cell before moving. The search validates that the current safe
interval admits this source hold through `m_in_start`. Reconstruction emits an
explicit `Wait` waypoint for the gap.

The move successor also validates the directed edge reservation
`(current.cell, neighbor)` over `[m_in_start, m_in_end)` against the oracle.
If the edge is not admissible for that crossing window, the successor is
rejected during search rather than deferred to commit.

The emitted state is:

```text
next_state = (neighbor, required_orientation, neighbor_safe_interval)
next_g     = m_in_end
```

### Event-Driven Wait Successors

SIPP normally obtains waits implicitly when a successor cell's safe interval
starts later than the current time. That covers the common case where the
immediate neighbor is blocked until a known release time.

Phase 1 also supports explicit event-driven wait successors for cases where
the useful wait is motivated by a downstream non-neighbor constraint and the
immediate neighbor intervals are all open-ended. Without this, the earliest
arrival at `(cell, interval)` can dominate later arrivals even when a later
departure is needed to satisfy a future bottleneck.

An explicit wait successor stays in the same cell, orientation, and safe
interval, but advances `g` to a selected event time and records that event in
the search key:

```text
wait_start      = current_action_ready_time
wait_end        = event_time
next.sipp       = (current.cell, current.orientation, current.interval)
next.wait_label = Some(event_time)
```

The event label is the bounded mechanism that keeps this later departure state
distinguishable from the earliest ordinary SIPP arrival. The preferred form is
event-driven: the search generates waits only at concrete reservation-derived
event times, not at arbitrary fixed buckets. The planning horizon is a cap, not
a wait event. Candidate event times come from a bounded source. The initial
implementation does not run a full reservation-ignored A* from every expanded
SIPP state just to discover wait events. The initial source is:

- goal-cell safe-interval starts, derived from existing goal-cell reservation
  boundaries and gap semantics, that admit the requested terminal dwell.

If metrics show this minimal source is too weak, follow-up sources may add
reservation-derived release times for already-enumerated bottleneck cells or
cells on a cheap static route corridor.

Each explicit wait is emitted only if the current cell's safe interval admits
the full wait and the wait end does not exceed the planning horizon. This keeps
the extra state set finite and tied to reservation events rather than to a
tunable time quantum.

The implementation records wait-event metrics, including how many candidate
events are considered, how many Wait successors are emitted, and how many are
rejected by the source interval or horizon. These counters are part of the
transition Step 5 validation check that the event source is useful without becoming a hidden
full-graph scan.

Within a given `SearchKey`, normal A* dominance still applies: the lowest
`g_score` wins. The event label only prevents the intentionally selected later
departure from being collapsed into the ordinary earliest-arrival state.

Bucketed wait states are a fallback engineering compromise. They are bounded
by `max_wait`, but they reintroduce a time-quantization tradeoff that SIPP is
designed to avoid.

## Goal Acceptance

The search does not accept a state merely because `state.cell == goal`. It
first validates the terminal reservation shape.

For open-ended terminal dwell, the current goal interval must be open-ended and
the goal reservation must be admissible from the move-entry time onward.

For finite terminal dwell, the reservation window from move-entry through dwell
completion must fit the current goal interval and pass the oracle's exact
availability check.

If terminal validation fails, the search continues. Another goal state in a
later safe interval may still be valid.

For deterministic offline simulation, equal-priority goal candidates use the
same stable ordering as the open set: lowest `f`, then lowest `g`, then
lexicographic `SearchKey`. The tie-break does not change optimality, but it
keeps runs reproducible when several goal states have the same priority.

## Path Reconstruction

When A* accepts a goal, it walks `came_from` and `NodeMeta` backward to the
start state, reverses the chain, and emits `Waypoint`s.

The first waypoint is always:

```text
PathStart { time: sim_start }
```

For each successor:

- `Move` metadata emits a `Move` waypoint at the destination cell. If
  `action_start > prior_action_end`, reconstruction first emits a `Wait` on
  the source cell to cover the gap.
- `Rotate` metadata emits a `Rotate` waypoint at the current cell. If
  `action_start > prior_action_end`, reconstruction first emits a `Wait` on
  that cell.
- explicit `Wait` metadata emits a `Wait` waypoint in the same cell and
  orientation.

Terminal handling: every path ends with `PathEnd { time:
last_action_end }`. The path describes motion only; the terminal dwell
(`dest_dwell`) is a reservation property, applied at commit-plan
composition, not encoded in the path waypoints.

The reconstructed chain is contiguous. It never contains overlapping adjacent
waypoints, and zero-width move/rotate/wait actions are dropped.

## Reservation Composition

The allocator composes the waypoint chain into a `CommitPlan`. The composition
is deterministic and mirrors the physical occupancy semantics used during
search.

A `Move` into cell `B` creates or continues the reservation epoch for `B`:

```text
reservation(B).start = Move.start
```

The reservation end for `B` is determined by the next action that leaves or
terminates the epoch:

- if the next cell advance leaves `B`, `reservation(B).end` is the move-out
  `Move.end`, so the cell is held until the robot is fully action-ready in the
  next cell;
- if `B` is the terminal cell (the chain ends with `PathEnd` at `B`), the
  composer applies `dest_dwell`: `None` or `Some(MAX)` leaves the end open
  (`None`); `Some(d)` for any other duration produces
  `reservation(B).end = Some(PathEnd.time + d)`. `Some(SimDuration::ZERO)`
  is finite (degenerate, but valid), not open-ended.

This creates intentional overlap between adjacent cell reservations during
straddle:

```text
reservation(B) overlaps reservation(C) while the robot's body spans B and C
```

Same-robot cross-cell overlap during straddle is valid. Same-cell overlap with
another robot is not valid.

Each move also creates an edge reservation:

```text
edge(source_cell, dest_cell) is reserved over [Move.start, Move.end)
```

Edge reservations prevent opposing or incompatible edge traversals while the
robot is physically crossing the boundary. Same-cell actions do not create edge
reservations.

## Atomic Commit

Each window of a reservation-aware `reserve_path` is one A* search and one
atomic commit. There is no optimistic tail.

Commit steps:

1. Compose the waypoint chain into a complete `CommitPlan`.
2. Validate every cell reservation request against the current table.
3. Validate predecessor close, if a predecessor open hold exists.
4. Validate every edge reservation request.
5. If every validation passes, commit all cell and edge reservations.
6. If any validation fails, commit nothing and return an error.

Because the reservation table is static during a window's search, a post-search commit
conflict indicates a consistency problem:

- search validation and commit composition disagree;
- interval enumeration and conflict checking use different gap semantics;
- edge validation is missing from search;
- the table mutated concurrently despite the offline-planner contract;
- predecessor exclusion was applied incorrectly.

The normal "no path" outcome is `Unreachable`, not a partial commit.

## Predecessor Holds

A predecessor is an already-open reservation at the starting cell. It exists
when a higher-level task begins from a cell the robot already occupies
open-ended, such as a parked robot or a robot waiting at a previous task
endpoint.

The search excludes the predecessor from self-conflict checks on its own cell.
The commit closes the predecessor atomically with the first move out of that
cell:

```text
predecessor.end = first_move_out_of_start.end
```

The close is part of the same atomic operation that commits the new path. There
is no release/recreate gap in which another robot can claim the starting cell.

If the path does not leave the predecessor cell, the predecessor remains open
or is closed according to the explicit terminal action requested by the caller.
The planner does not create a duplicate same-cell reservation merely to record
task metadata. BotPlan records action-ready metadata for no-move legs
separately from reservation-table mutation.

## Failure Semantics

`FHSippPathfinder` returns `Unreachable` when no path satisfies the current graph,
reservation, dwell, and horizon constraints. Diagnostics distinguish three
cases:

- `UnreachableWithinHorizon`: a path may exist with a larger `max_wait` or a
  different scheduler decision.
- `NoStaticRoute`: no path exists on the static graph for the requested robot
  type, cargo state, and zone constraints.
- `TerminalUnsatisfiable`: no goal-cell interval in the current query admits
  the requested terminal reservation, regardless of route.

Diagnostic precedence is:

1. `NoStaticRoute` if the static graph has no route for the requested robot
   type, cargo state, and zone constraints.
2. `UnreachableWithinHorizon` if a feasible plan may require a later
   reservation event or action-ready time beyond `sim_start + max_wait`.
3. `TerminalUnsatisfiable` only when goal-cell interval enumeration within the
   configured horizon proves the requested terminal reservation cannot fit
   regardless of route timing. When unsure whether a longer horizon would
   expose a valid goal interval, prefer `UnreachableWithinHorizon`.

`Unreachable` means "no path within this query's model and horizon." It does
not necessarily mean "no path exists forever." A higher-level scheduler may
respond by:

- retrying after later commitments advance;
- increasing the planning horizon;
- routing the robot to a parking or non-task endpoint;
- changing task priority/order;
- declaring the task infeasible for the current scenario.

`FHSippPathfinder` returns a commit conflict only when a path it believed valid
cannot be committed. That is treated as a bug or stale-table violation, not a
normal partial-success path.

Zero-length paths (`from == to`) are handled explicitly by the caller or by a
direct hold path, depending on the task semantics. An `FHSippPathfinder` movement call
does not silently turn zero-length movement into a successful route.

## State-Space and Performance

SIPP's state count is bounded by:

```text
cells * orientations * safe_intervals_per_cell
```

It is not bounded by:

```text
cells * time_horizon / time_bucket
```

This is the key Phase 1 property. Waiting for 10 seconds and waiting for 100
seconds inside the same safe interval do not by themselves create 1000
different time-bucket states.

At Cencora scale the graph contains roughly 15.9K cells, but each A* call
normally reaches a smaller connected subgraph, often on the order of 3K-8K
cells. The whole-graph bound is therefore a pessimistic memory bound rather
than the expected expansion count. Expansion depends on:

- heuristic tightness;
- number of safe intervals on cells near the path;
- corridor bottlenecks;
- degree of static graph branching;
- number and placement of explicit wait events;
- terminal dwell constraints.

The expected normal-case per-call latency is milliseconds to tens of
milliseconds, not microseconds. `FHSippPathfinder` records enough stats to
validate this empirically:

- states expanded;
- states pushed;
- open-heap peak;
- interval enumerations;
- exact availability checks;
- delayed moves;
- explicit waits emitted;
- source-hold blocks;
- goal validation rejects;
- commit validation failures.

The completed design trades a larger per-window search for fewer calls per
task: a task makes one call per journey. A foreshortened call runs, on each
pass (the first, and each after a wake or a window; scheduler time):

- one search to the target, which with the retry below ends the call when it
  succeeds or when nothing is held ahead of the robot; when something is held
  ahead, it is not run while another robot holds the target open-ended and
  the stay asked for is open-ended, nor while every way into the target
  comes, directly or through queue cells, from a cell another robot holds
  open-ended from the search's simtime start, a queue cell or not;
- when that search fails in a way a hold can explain and the call has
  awaited a cell: one empty-floor search for the robot's route, one per
  awaited cell on it whose clearance is at or beyond the horizon, for the
  travel to it, then one more search to the target from the latest start
  that reaches such a cell at its clearance in less than `max_wait`, when
  that is later than the first search's start;
- when something is held ahead and those fail the same way or are not run,
  one search per free queue cell (one no other robot holds open-ended),
  nearest the target first, until one is reached; a cell sealed off the same
  way is not searched;
- one empty-floor search for the held cells on the robot's route; when there
  are none and no window moved the robot, one more search to the target,
  whose error ends the call.

## Module Responsibilities

`FHSippPathfinder` owns:

- graph traversal expansion;
- orientation-aware movement and rotate costs;
- SIPP state identity and deduplication;
- reservation-aware successor validation;
- wait synthesis;
- heuristic evaluation;
- waypoint reconstruction;
- composition of waypoint chains into reservation plans;
- atomic commit orchestration through the oracle.

The reservation table owns:

- canonical safe interval enumeration;
- exact overlap/conflict checks;
- predecessor exclusion by reservation identity;
- cell reservation mutation;
- edge reservation mutation;
- all-or-nothing commit semantics.

The scheduler owns:

- task ordering and priority;
- selecting robot/task pairs;
- deciding whether to retry, defer, park, or fail on `Unreachable` (the wait
  for an open-ended hold on the way is `reserve_path`'s own);
- choosing destination dwell semantics;
- limiting how many future tasks are injected into the reservation table.

## Phase 2 Boundary

Phase 1 assumes instantaneous acceleration/deceleration and a constant
traversal duration per edge. It does not model speed at state entry or exit.

Phase 2 adds HTM trapezoidal motion by extending the state and metadata:

- `SippState` or the Phase 2 search-state carrier gains `speed_bucket`;
- `NodeMeta` records real entry and exit velocity;
- move cost is computed by a kinematic profile;
- rotate successors require zero speed;
- the heuristic changes to a kinematic lower bound.

`FHSippPathfinder` does not require any partial-commit or frontier-stitching
machinery for Phase 2: a window's commit is atomic, and the next window starts
from the hold the last one ended in. The same SIPP shape continues to apply, window by
window, with a richer state identity.

## References

- Mike Phillips and Maxim Likhachev, "SIPP: Safe Interval Path Planning for
  Dynamic Environments", ICRA 2011.
  https://www.ri.cmu.edu/publications/sipp-safe-interval-path-planning-for-dynamic-environments/
  Defines safe intervals, SIPP state identity, completeness/optimality under
  instantaneous-stop assumptions, and lower-dimensional heuristic use.

- David Silver, "Cooperative Pathfinding", AIIDE 2005.
  https://ocs.aaai.org/Papers/AIIDE/2005/AIIDE05-020.pdf
  Defines reservation-table cooperative A*, HCA*, WHCA*, wait actions in
  space-time search, and reservation-ignored abstract distance heuristics.

- Zahy Bnaya and Ariel Felner, "Conflict-Oriented Windowed Hierarchical
  Cooperative A*", ICRA 2014.
  https://tzin.bgu.ac.il/~felner/2014/COWA6p.pdf
  Describes CO-WHCA* as an online windowed MAPF method focused around likely
  conflicts. Useful contrast for why the simulator does not need a planning
  window under static offline assumptions.

- Jiaoyang Li, Andrew Tinka, Scott Kiesel, Joseph W. Durham, T. K. Satish
  Kumar, and Sven Koenig, "Lifelong Multi-Agent Path Finding in Large-Scale
  Warehouses", AAAI 2021.
  https://www.ri.cmu.edu/publications/lifelong-multi-agent-path-finding-in-large-scale-warehouses/
  Introduces RHCR, a rolling-horizon framework for lifelong warehouse MAPF.
  Useful as the main windowed-warehouse comparison point.

- Hang Ma, Jiaoyang Li, T. K. Satish Kumar, and Sven Koenig, "Lifelong
  Multi-Agent Path Finding for Online Pickup and Delivery Tasks", AAMAS 2017.
  https://www.ri.cmu.edu/publications/lifelong-multi-agent-path-finding-for-online-pickup-and-delivery-tasks/
  Defines MAPD, Token Passing, and well-formed instances with non-task
  endpoints. Provides the higher-level parking/fallback context.

- Zain Alabedeen Ali and Konstantin Yakovlev, "Safe Interval Path Planning
  With Kinodynamic Constraints", AAAI 2023.
  https://arxiv.org/abs/2302.00776
  Shows why vanilla instantaneous-stop SIPP is incomplete under kinodynamic
  constraints and motivates the Phase 2 speed/velocity state extension.

- Anton Andreychuk, Konstantin Yakovlev, Eli Boyarski, and Roni Stern,
  "Improving Continuous-time Conflict Based Search", AAAI 2021 preprint.
  https://arxiv.org/abs/2101.09723
  Uses generalized SIPP as a low-level continuous-time planner inside CCBS.
  Relevant for continuous-time/no-time-discretization framing.

- Yue Zhang, Zhe Chen, Daniel Harabor, Pierre Le Bodic, and Peter J. Stuckey,
  "Planning and Execution in Multi-Agent Path Finding: Models and Algorithms",
  ICAPS 2024.
  https://mapf.info/index.php/Main.PStuc24b
  Introduces PIE. Useful contrast for online planning/execution objectives,
  but not a direct argument for small commits in this offline simulator.
