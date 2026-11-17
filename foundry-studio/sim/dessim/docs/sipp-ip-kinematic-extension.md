# Turn-Waypoint Encoding Contract — Phase 2 (HTM Trapezoidal Motion)

**Status**: Draft for review. Not blocked by Phase 1; designed to land on top of it.
**Predecessor doc**: `specs/putaway/turn-encoding-contract-phase1.md`.
**Hard prerequisite**: Phase 1 must land first. Phase 1 already ships full SIPP `(cell, orient, safe_interval)` dedup (no time quantization), so the dedup-axis prerequisite is satisfied. Phase 2 adds `speed_bucket` to the dedup key per the SIPP-IP precedent.

## Problem

HTM bots have trapezoidal motion: acceleration, cruise velocity, deceleration. Traversal time across one cell is no longer a constant `traverse_duration` — it's a function of:

- Entry velocity (carried from the previous cell)
- Exit velocity (set by what follows: cruise into the next cell, or decelerate to a stop)
- Cell length, max acceleration, max velocity

A bot decelerating to rest *inside* cell N takes longer to clear it than a bot cruising through. Same cell, same entry time, different exit time.

Three things break under variable traversal time:

1. **Phase 1's `Move { start, end }` window calculation** uses constant `traverse_duration` for `end - start`. Under trapezoidal motion this becomes `kinematic_cost(entry_v, exit_v, cell_length)` — a function of velocity, not a constant.
2. **The search state space is no longer correctly deduped on `(cell, orient, safe_interval)`** alone. Two arrivals at the same `(cell, orient, safe_interval)` with different inbound velocities have different reachable successor sets at different costs (Ali & Yakovlev, AAAI 2023, prove this is incomplete without velocity in state). Specifically, a high-velocity arrival cannot rotate in place without first decelerating; a zero-velocity arrival can rotate immediately. Different successor sets ⇒ different nodes. Add `speed_bucket` to the dedup key.
3. **The heuristic `h(cell, goal)` based on a constant `traverse_duration`** is no longer admissible. A kinematic-aware lower-bound (e.g., manhattan distance / `v_max`) is needed.

## Solution

Phase 1 designed for this — Phase 2 is purely additive on top, no interface changes to surrounding code.

### Struct shape changes

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct TimedState {
    pub cell: CellId,
    pub orientation: Orientation,
    pub interval: SafeIntervalId,        // From Phase 1.
    pub speed_bucket: SpeedBucket,       // NEW in Phase 2. Discretized entry velocity.
}

pub(super) struct NodeMeta {
    pub cell_step: u32,
    pub entry_velocity: Velocity,        // NEW in Phase 2. Real, not bucketed.
    pub exit_velocity: Velocity,         // NEW in Phase 2. Real, not bucketed.
}
```

`AStarEntry` is unchanged. The `Waypoint` type and `WaypointAction` enum (Phase 1) are unchanged in shape — Move/Wait/Rotate/PathStart/PathEnd variants carry `start: SimTime, end: SimTime` (or `time` for markers); `kinematic_cost` just produces different numeric values for `end - start` on Move actions.

This matches SIPP-IP's verified reference implementation (`github.com/PathPlanning/SIPP-IP`, `CLOSED[x][y][o][v]` 4D dedup with intervals stored per-bucket) — see `specs/putaway/investigations/quantization-literature-check.md` round 2 for the primary-source verification.

### `SpeedBucket` discretization

Fixed bucket size, centralized as a search-level constant. **Recommended: 10 cm/s** (per architect's analysis, sized so two physically-distinguishable velocities never share a bucket given cencora kinematics: ~1.5 m/s top, ~1 m/s² accel, ~0.5 m cells, accel-step over a cell ≈ 0.7 m/s).

```rust
const SPEED_BUCKET_SIZE_CM_S: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpeedBucket(u32);  // velocity floor in 10 cm/s units

impl SpeedBucket {
    pub fn from_velocity(v: Velocity) -> Self { /* floor */ }
}
```

Do NOT derive bucket size from per-graph kinematics — search-state shape must be invariant across graphs.

### Velocity carriage

- **Bucketed** (`SpeedBucket`) goes in `TimedState` — used as a dedup-axis in the search state space. Hashable, equatable.
- **Real** (`Velocity`) goes in `NodeMeta` — used by `reconstruct_waypoints` to emit accurate waypoint timing for BotPlan playback. Not part of dedup.

The bucket is a coarse-grained search hash; the real value is the playback fidelity.

### Successor cost computation

Replace the per-edge `traverse_cost` constant with a kinematic call:

```rust
fn kinematic_cost(
    entry_v: Velocity,
    exit_v: Velocity,
    cell_length: f64,
    max_accel: f64,
    max_velocity: f64,
) -> (SimDuration, Velocity) {
    // Trapezoidal profile: accelerate from entry_v to peak, hold, decelerate to exit_v.
    // Returns (traversal_time, actual_exit_velocity_after_kinematic_clamp).
}
```

Successor emission (per Phase 1's `g_score = action-ready time` contract):

```rust
for next_action in possible_actions {
    let target_exit_v = velocity_for(next_action);  // 0 for rotate-next, cruise for move-next
    let (traverse_time, exit_v) = kinematic_cost(current.entry_velocity, target_exit_v, ...);

    let intervals = oracle.entry_intervals(neighbor, sim_start + current_g, horizon, predecessor);
    for (free_start, free_end) in intervals {
        // Move waypoint window: candidate_move_start may be delayed past
        // current_g if the neighbor's safe interval starts later. Recompute
        // action_start AND action_end after clipping.
        let candidate_move_start = max(free_start, sim_start + current_g);
        let m_in_start = candidate_move_start;
        let m_in_end = m_in_start + traverse_time;
        let tentative_g = m_in_end;

        // Implied reservation window on neighbor = m_in.start to earliest
        // Move-out-of-neighbor end. Minimum (immediate transit) = entry slice
        // + immediate exit slice, both kinematic.
        let next_traverse_time = kinematic_cost(exit_v, ...);  // immediate transit
        let implied_window_end = m_in_end + next_traverse_time;
        let implied_window = m_in_start .. implied_window_end;

        if oracle.earliest_available(neighbor, m_in_start, implied_window.duration()) == Some(m_in_start) {
            // emit successor with interval = (free_start, free_end) as dedup key.
            // node_meta records action_start = m_in_start, action_end = m_in_end,
            // entry_velocity = current.entry_velocity, exit_velocity = exit_v.
        }
    }
}
```

Note: under Phase 1's contract, `Move { start, end }` has `end - start = traverse_time` (the bot's own traversal). The composed reservation on the destination cell extends from `Move.start` to `Move-out.end` (= action-ready at the cell *after* the destination). Phase 2 derives both `traverse_time` terms kinematically. Neither is a hardcoded constant; both are properties of the specific Move instances composed.

### Heuristic

`h(cell, goal)` must remain admissible (under-approximate true cost-to-go) for A\*'s optimality. With variable traversal time, the cheapest possible time to traverse one cell is `cell_length / v_max` (zero-accel case). Use:

```rust
fn h(cell: CellId, goal: CellId) -> SimDuration {
    manhattan_distance_cells(cell, goal) * (cell_length / v_max)
}
```

This is admissible regardless of acceleration profile. Tight enough not to wreck branching factor in practice — verify empirically.

### Rotate semantics under trapezoidal

Rotates require speed = 0. Translating to the search:

- A `Rotate` successor is emitted only if `current.speed_bucket == SpeedBucket(0)`.
- `Move` successors targeting a same-cell-same-orientation Rotate (i.e., the next state is a Rotate) must have `target_exit_v = 0`. The kinematic computation includes the deceleration time within the cell.
- This naturally makes high-velocity arrivals "pay" the deceleration cost when a rotate follows, while zero-velocity arrivals rotate immediately.

(Note: Phase 1 renamed `WaypointAction::Turn` → `Rotate` to disambiguate from "turn at a corner" — which is a Move with an orientation change. Phase 2 inherits the rename.)

### Wait semantics

Already correct under Phase 1's explicit Wait waypoint mechanism (Wait is always its own waypoint; never embedded in Move or Rotate). A wait at speed = 0 is straightforward. A wait at speed > 0 is physically impossible (the bot would coast); the search must decelerate to 0 first. The kinematic cost already models this.

### Goal acceptance

The bot must arrive at the goal stationary (the dispatcher's `dest_dwell` semantics assume zero velocity, and physically the bot can't be "at the goal" while still moving through it). `expand_move` forces goal-cell Move successors to use `target_exit_v = 0`, but the trapezoidal Case 3a clamp can leave `actual_exit_v` non-zero when the goal cell is too short to brake fully (e.g. v_max=4500, decel=2000 over 1 m clamps to ~4031 mm/s). `try_accept_goal` therefore mirrors the Rotate/Wait gates and rejects any goal-arrival state whose `speed_bucket != ZERO`. The search either routes through additional decel cells before the goal or reports no-path if no such route fits the horizon. VTMs and `MotionModel::ConstantSpeed` are unaffected (their states stay at `SpeedBucket::ZERO` everywhere).

## Open questions

1. **Heuristic admissibility under rotate costs.** The `manhattan / v_max` bound assumes straight-line cruise. If the path requires a rotate, actual cost ≥ bound + `rotate_duration`. The bound stays admissible (under-approximates), but is it *tight* enough for reasonable branching? **TODO: empirical test on cencora topology.**
2. **`SpeedBucket` granularity tuning.** 10 cm/s is the architect's first call. If branching factor blows up, bucket coarser. If physical distinctness leaks across buckets, finer. **Empirical decision after Phase 2 lands.**
3. **Full SIPP-IP waiting-interval migration.** Phase 1 ships full SIPP `(cell, orient, safe_interval)`. Phase 2 adds `speed_bucket` additively → `(cell, orient, speed_bucket, safe_interval)`. SIPP-IP (Ali & Yakovlev 2023) folds velocity *into* the interval ("waiting interval" = continuous set of `(time, speed)` pairs feasible from a given vertex), giving stronger completeness guarantees under bounded acceleration. The verified SIPP-IP reference impl uses 4D `CLOSED[x][y][o][v]` with multi-interval per bucket — equivalent shape to our `(cell, orient, speed_bucket, safe_interval)`, just different storage layout. **Migration to true waiting-interval semantics deferred until profiling shows the additive shape has gaps.**
4. **Newtype wrappers.** Phase 1 deferred `EntryTime` / `OccupancyDuration` newtypes. Under Phase 2, `Velocity` / `SpeedBucket` / `Acceleration` are already typed; the surrounding type density is higher. Reconsider introducing time-side newtypes here. **Open design call.**
5. **HTM-only initially?** Trapezoidal motion is HTM-specific. VTMs may stay constant-speed for some time. The contract must not break if `kinematic_cost` returns a constant for VTMs. **No structural change needed; just ensure VTMs use a constant-velocity kinematic profile.**
6. **Reverse traversal.** HTMs at relay cells today fudge turns with in-place rotation; the lead-resume notes "needs reverse edge traversal." Reverse traversal is a separate kinematic action (back-up). **Likely a separate concern from Phase 2's encoding contract — track separately.**

## Verification

- All Phase 1 tests still green.
- New tests under `tests_kinematic.rs` (or similar):
  - Trapezoidal traversal time matches expected accel/decel profile.
  - `Rotate` successor only emitted when `speed_bucket == 0`.
  - `Move` successor targeting a Rotate-next decelerates to exit_v = 0 within the cell.
  - Two arrivals at same `(cell, orient, safe_interval)` with different `speed_bucket` are distinct nodes; both expanded independently.
- `cencora_10_robots_same_station` and `minimal_putaway2.json` continue to pass.
- A new fixture exercising acceleration/deceleration through tight reservation gaps (TBD).
- Empirical heuristic-admissibility check: run a few representative searches, assert `g_score[goal] >= h(start, goal)`.

## References

Same set as Phase 1, plus:

- `specs/putaway/investigations/mapf-mrta-state-space.md` — Ali & Yakovlev 2023 (SIPP-IP), Andreychuk et al. 2021 (PSIPP), specifically the velocity-in-state completeness proof.
- `specs/putaway/investigations/quantization-literature-check.md` round 2 — primary-source verification that SIPP-IP's reference impl uses 4D `CLOSED[x][y][o][v]` dedup, the shape Phase 2 lands on additively over Phase 1's SIPP.
- `specs/putaway/investigations/architect-turn-fix-shape.md` § "Cross-review revisions" — concession on velocity-in-state, bucket-size analysis (10 cm/s).
- `specs/putaway/investigations/algo-expert-turn-encoding.md` § "Cross-review response" — affirmation that velocity belongs in `TimedState`, not `NodeMeta`.

## Implementation phasing within Phase 2

Within Phase 2 itself, suggested landing order:

1. Add `SpeedBucket` newtype + `Velocity` newtype (if not already present) + `kinematic_cost` function (initially returning constant traverse_duration to preserve Phase 1 behavior).
2. Add `speed_bucket` to `TimedState` with default `SpeedBucket(0)` everywhere; add `entry_velocity`/`exit_velocity` to `NodeMeta` with constants. Phase 1 tests stay green (no behavior change).
3. Wire actual `kinematic_cost` to use real trapezoidal profile. Update heuristic. Phase 1 tests adjust if traversal times shift; new kinematic tests exercise the variable-time machinery.
4. Address rotate-only-at-speed-zero constraint. Adjust rotate-successor emission and existing tests as needed.
5. Profile expansion counts. Migrate to true SIPP-IP waiting-intervals if profiling shows the additive `(cell, orient, speed_bucket, safe_interval)` shape has gaps.

Each step keeps the codebase compilable and most tests green; failures surface tightly to the step that introduced them.

## Out of scope (Phase 3+)

- True SIPP-IP waiting-interval migration (folding velocity into interval identity).
- Reverse traversal at relay cells.
- Multi-robot coordinated optimization (CCBS-style multi-agent CBS).
- Anything beyond the single-robot temporal A\* contract.
