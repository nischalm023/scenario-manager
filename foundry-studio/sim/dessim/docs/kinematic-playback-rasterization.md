# Kinematic Playback Rasterization

Status: spec, not implemented. Successor doc to
`sim/dessim/docs/sipp-ip-kinematic-extension.md`.

## Problem

The SIPP-IP kinematic mode (`MotionModel::HtmKinematic`) computes physically
correct per-cell traversal times for HTMs, including acceleration from rest,
cruise at v_max, and deceleration into rotates and goal cells. The waypoint
chain reflects these timings — a kinematic move into a goal can take 1.4s
while a cruise move takes 0.7s on the same 1m cell.

Despite correct timings, the visualizer renders robots moving at constant
velocity within each cell because `desvis/src/app/playback.rs` linearly
interpolates position between waypoint endpoints. A 1.4s decel-into-goal and
a 1.4s accel-from-rotate look identical: both render as a uniform pace through
the cell, with no visible velocity ramp.

The user-visible signal that exists today is "this cell took longer than
that cell" — coarse and easy to miss. There is no within-cell ease-in,
ease-out, or cruise plateau.

## Goal

Make trapezoidal motion visible in playback without polluting the search
graph or the playback layer with kinematic-profile concerns. The search
already owns `(entry_v, exit_v, total_time)`; the visualization layer should
own the smooth curve.

## Non-goals

- No changes to `NodeMeta`, the SIPP-A* search runner, or any
  reservation-aware logic. The search must remain headless and free of
  rendering metadata.
- No changes to the `BotStep` struct or the recording format. (The container
  this was written against has since been replaced by `.worldview`, which
  carries the same steps and the millimetres each one stands at.)
- No changes to `playback.rs` interpolation. Linear position blending stays.
- No new kinematic logic in playback or `BotStep`-consuming code. The
  rendering side stays unaware of kinematic params.

## Architecture

```
SEARCH LAYER (untouched)
    NodeMeta { entry_velocity, exit_velocity, ... }
        ↓ reconstruct.rs
PATH OUTPUT LAYER (minimal extension)
    WaypointAction::Move { start, end, entry_velocity_mm_s, exit_velocity_mm_s }
        ↓ NavEvent → SimObserver
OBSERVER LAYER (new responsibility — rasterization)
    nav_handler reads (entry_v, exit_v, distance, kinematic_params)
    Computes trapezoidal profile internally
    Discretizes Move waypoints into N micro-BotSteps along the curve
        ↓ time_slice records
RENDERING LAYER (untouched)
    playback.rs linear-interpolates between dense micro-BotSteps
    The polyline approximation of the trapezoidal curve produces
    visible ease-in, cruise, and ease-out in the rendered motion.
```

The trapezoidal curve is *rasterized* into a polyline of dense sample points
at the observer layer, the same way a smooth curve is rendered as a polyline
in graphics. Linear interpolation between dense enough samples approximates
any smooth curve to arbitrary fidelity.

## Phase 1: Surface velocities through Path

### `WaypointAction::Move` extension

In `sim/dessim/src/pathfinding/path.rs`:

```rust
WaypointAction::Move {
    start: SimTime,
    end: SimTime,
    entry_velocity_mm_s: f64,
    exit_velocity_mm_s: f64,
}
```

Both velocities are in mm/s, real (not bucketed). They describe the robot's
velocity at the move's start (= source cell center) and end (= destination
cell center). Other `WaypointAction` variants (`PathStart`, `PathEnd`,
`Rotate`, `Wait`) are unchanged.

### `reconstruct.rs` populates the velocities

In `sim/dessim/src/pathfinding/fh_sipp/reconstruct.rs`, when emitting a
Move waypoint, copy from the corresponding `NodeMeta`:

```rust
WaypointAction::Move {
    start: meta.action_start,
    end: meta.action_end,
    entry_velocity_mm_s: meta.entry_velocity.mm_per_sec(),
    exit_velocity_mm_s: meta.exit_velocity.mm_per_sec(),
}
```

### ConstantSpeed mode shape

For `MotionModel::ConstantSpeed`, the search's `NodeMeta.entry_velocity` and
`exit_velocity` are `Velocity::ZERO` (we never wired them for the constant
path). The reconstruction needs a fallback: when both are zero AND the
action is a Move, write `entry = exit = pathfinder.speed_mm_per_sec`. This
preserves the rasterization layer's ability to detect "is this a kinematic
move or a constant-velocity move" by checking `entry == exit`.

### Test sites

`grep -rn 'WaypointAction::Move' sim/dessim/src/` should turn up roughly
15 sites — production reconstruct, plus tests under
`sim/dessim/src/pathfinding/fh_sipp/tests/` and assorted
unit-test fixtures. Each construction site needs the two new fields. Most
test sites are happy with `entry = exit = some_default` since they use
ConstantSpeed mode.

## Phase 2: Observer-layer rasterization

### `MetricsConfig` extension

In `sim/dessim/src/observer/state.rs` (or wherever `MetricsConfig` lives,
matching the current location of `speed_mm_per_sec` and `cell_spacing_mm`):

```rust
pub struct MetricsConfig {
    // ...existing fields...
    pub htm_kinematics_loaded: Option<KinematicParams>,
    pub htm_kinematics_unloaded: Option<KinematicParams>,
    /// Sample period for trapezoidal rasterization. Default 100ms.
    /// Smaller = smoother visualization, more memory.
    pub motion_step_ms: u32,
}
```

`solver.rs` populates these from the same `KinematicParams` that the
pathfinder uses. When `SimConfig.motion_model == ConstantSpeed`, the
`htm_kinematics_*` fields are `None` and rasterization is skipped — the
observer emits one Move `BotStep` per Move waypoint as today.

### Rasterization in `nav_handler.rs`

When processing a `WaypointAction::Move`:

```rust
let needs_rasterization = (entry_v - exit_v).abs() > VELOCITY_EQ_EPSILON
    && self.config.htm_kinematics_loaded.is_some();

if needs_rasterization {
    let params = if carrying_cargo {
        self.config.htm_kinematics_loaded.unwrap()
    } else {
        self.config.htm_kinematics_unloaded.unwrap()
    };
    let profile = compute_trapezoid(entry_v, exit_v, distance_mm, &params);
    let step_ms = self.config.motion_step_ms;
    let mut t = 0u64;
    while t < move_duration_ms {
        let t_end = (t + step_ms as u64).min(move_duration_ms);
        let pos_at_t_end = trapezoid_position(&profile, t_end as f64 / 1000.0);
        intervals.push(OccupancyInterval {
            start_ms: move_start + t,
            end_ms: move_start + t_end,
            cell: cell_key,
            pos: pos_at_t_end,  // world-space position at this micro-step's end
            kind: BotStepKind::Move,
        });
        t = t_end;
    }
} else {
    // single BotStep, current behavior
    intervals.push(OccupancyInterval { ... });
}
```

The `pos` field is the WORLD-SPACE position at the end of each micro-step,
NOT the destination cell center. Playback's existing linear interpolation
between consecutive `BotStep` positions then traces a polyline that matches
the trapezoidal curve.

### Trapezoidal helpers

Reuse the existing math in `sim/dessim/src/pathfinding/fh_sipp/kinematics.rs`.
The current `trapezoidal_cost` function already computes the full profile
internally (peak velocity, accel/cruise/decel phase distances and times) but
returns only `(total_time, exit_velocity)`. Either:

- (Preferred) Extract a `pub struct TrapezoidalProfile { peak_v, accel_time,
  cruise_time, accel_distance, cruise_distance }` and a public
  `trapezoidal_profile(...)` function returning it. `trapezoidal_cost` becomes
  a thin wrapper that returns just `(total_time, actual_exit_v)`.
- Add `trapezoid_position(&profile, t)` and `trapezoid_velocity(&profile, t)`
  helpers in `kinematics.rs` operating on the profile. Used by the observer.

This keeps the trapezoidal math in one place — search and observer both
consume the same canonical helpers. `kinematics.rs` becomes a small
self-contained module with no rendering or playback awareness.

### Slice-boundary handling

The existing `append_interval` logic in `nav_handler.rs:321` already splits
intervals at slice boundaries. Micro-step intervals (typically ≤100ms) will
rarely cross slice boundaries — and when they do, the existing per-step
splitting still works correctly because each micro-step has its own
canonical end position (computed kinematically before the split). No
changes to `append_interval` required.

## Phase 3: Verification

- All existing 58 fh_sipp tests pass unchanged.
- The new Move waypoint fields populate correctly:
  - HtmKinematic mode: `entry_v` and `exit_v` reflect actual NodeMeta values.
  - ConstantSpeed mode: both equal `pathfinder.speed_mm_per_sec`.
- `nav_handler` rasterization unit tests:
  - A kinematic accel-from-rest Move emits N micro-BotSteps with positions
    that follow the trapezoidal accel curve (quadratic position growth in
    accel phase).
  - A kinematic decel-to-zero Move emits the symmetric ease-out curve.
  - A constant-speed Move (entry_v == exit_v) emits exactly 1 BotStep.
- Visual smoke test in desvis: `--settings motion_model=htm_kinematic` shows
  HTMs visibly easing in after rotates and easing out before goals/rotates.
  Compare against `motion_model=constant_speed` — should look like uniform
  motion (linear interpolation).
- Recording round-trip: record a kinematic run, replay via
  `desvis --recording <path>`. Should render identically to a live kinematic
  run.

## Phase 4: The recording

`BotStep` is unchanged, so a `.worldview` recording needs nothing new: a run
under `motion_model=htm_kinematic` simply states more steps per HTM Move, and
the playback section is a list whose length nothing depends on. Each step also
carries the millimetres it stands at, so a rasterised curve comes back point
for point rather than as a line between cell centres.

The `motion_step_ms` rasterization granularity is fixed when the run is
recorded. Replaying a recording made at one granularity at another would mean
re-running: the recording is the canonical record of what playback drew.

## Memory budget

A 1.5s kinematic Move at 100ms granularity becomes 15 BotSteps instead of
1. Each BotStep is roughly 24 bytes. So a kinematic Move costs ~360 bytes
of profile storage per occurrence.

For a typical run:
- ~1000 HTM Moves per simulation hour
- ~50% are kinematic (decel/accel), ~50% are cruise (single BotStep)
- ~500 kinematic Moves × 360 bytes = ~180KB extra per sim hour

Bounded, well below the typical recording size (tens of MB). No memory
cliff.

## Rejected alternatives

### Alt 1: Extend `BotStep` with profile fields, do trapezoidal interpolation in playback

```
BotStep {
    cell_id, pos, delta_ms, kind,
    entry_velocity_mm_s: f32,
    peak_velocity_mm_s: f32,
    exit_velocity_mm_s: f32,
    accel_phase_ms: u16,
    cruise_phase_ms: u16,
}
```

Playback computes position via piecewise integration of the trapezoidal
profile.

Rejected because:
- Adds 16 bytes per BotStep (Wait/Rotate steps waste them).
- Bumps `BENCHMARK_PROFILE_FORMAT_VERSION` — old profiles fail to load.
- Couples `playback.rs` to kinematic profile semantics.
- Couples `BotStep` to a single motion model. Adding e.g. an exponential
  acceleration profile later would require another schema change.
- The visual fidelity gain over dense rasterization is negligible for
  reasonable sample rates (100ms produces visually smooth motion).

### Alt 2: Extend `NodeMeta` with full profile (peak_v, phase times)

Rejected because:
- Pollutes the search graph with rendering concerns.
- The search does not consume peak_v or phase times for any decision.
- Increases `NodeMeta` cache footprint during the hot search loop.
- Couples the search to a particular profile representation.

### Alt 3: Linear-velocity quadratic-position approximation in playback

Replace linear `progress` with a quadratic curve based on `(entry_v,
exit_v, total_time)`. Cheap, no profile fields needed.

Rejected because:
- Visually wrong for moves with a cruise plateau (over/undershoots
  mid-cell).
- The error is bounded but noticeable on longer cells.
- We have the kinematic params at the observer; we may as well use them.

## Future extensions

- **Configurable `motion_step_ms`** via `SimConfig.kinematic_playback_step_ms`
  with a default of 100ms. Allows users to trade memory for visual smoothness
  for specific debugging needs.
- **Velocity-aware overlay** in desvis: render robots with a colored ring
  whose hue/intensity reflects current velocity. Requires the playback layer
  to know velocity, which would push us toward Alt 1. Defer until clearly
  needed.
- **VTM kinematics**: the architecture already accommodates VTM trapezoidal
  motion (just add `KinematicParams::a5_vtm()` and route it to VTM Moves).
  Currently VTMs are constant-speed; can be added later without changing
  the architecture.

## Implementation order

Suggested split into commits:

1. **Add velocity fields to `WaypointAction::Move` and `reconstruct`**.
   Update all test construction sites. ConstantSpeed mode fills with
   `speed_mm_per_sec`. All 58 search tests still green.
2. **Extract `TrapezoidalProfile` from `trapezoidal_cost`**, add
   `trapezoid_position` and `trapezoid_velocity` helpers in
   `kinematics.rs`. Unit tests for the helpers. No behavior change.
3. **Extend `MetricsConfig`** with `htm_kinematics_*` and `motion_step_ms`.
   `solver.rs` populates from `SimConfig`. ConstantSpeed mode passes None.
4. **Rasterization branch in `nav_handler.rs`**. Unit tests for
   rasterization (count, positions, slice-boundary behavior).
5. **End-to-end visual smoke** test or fixture.

Each commit keeps the build and test suite green. The visual change is
gated entirely on `MotionModel::HtmKinematic`; ConstantSpeed mode produces
identical playback to the current behavior.

## References

- `sim/dessim/docs/sipp-ip-kinematic-extension.md` — Phase 2 SIPP-IP
  kinematic search spec (predecessor doc).
- `sim/dessim/docs/fh-sipp-pathfinder-algo-draft.md` — Phase 1 FH-SIPP
  algorithm.
- `sim/dessim/src/pathfinding/fh_sipp/kinematics.rs` — current
  trapezoidal_cost implementation.
- `sim/dessim/src/observer/nav_handler.rs:130-193` — current Move →
  BotStep handling.
- `crates/desvis/src/app/playback.rs:182-212` — current playback
  interpolation logic.
