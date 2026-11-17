# Pathfinder Test Conventions

This document specifies the conventions for the single-shot Phase 1
`FHSippPathfinder` test suite at
`sim/dessim/src/pathfinding/fh_sipp/tests/`. It is the canonical
reference; resume files and the migration punchlist defer to it.

## Goals

The suite is a regression guard for two surfaces:

1. **Search behavior** — every state expansion, every successor pushed,
   every reservation oracle query, in order.
2. **Path output** — the chronological waypoint chain returned by
   `find_path` / `find_path_reserved`, including initial `PathStart`,
   move/rotate/wait sequence, and trailing dwell.

Both surfaces are pinned exact-match. When the production code changes —
heuristic upgrade, tie-break shift, dwell semantics, oracle behavior —
the diff is the receipt that something moved. Loose presence checks
defeat the purpose; never write them.

## File layout

```
fh_sipp/tests/
├── mod.rs        suite-wide doc + module declarations
├── prelude.rs    single re-export manifest; the only thing test files import
├── helpers.rs    constructors, fixtures, TraceCollector, assert_* macros
├── basic.rs      CANONICAL — read this before adding a new test file
├── corridor.rs   delayed_move.rs   stacking.rs   ...one #[test] file per topic
└── mod-quarantined.rs  — pre-pivot tests.rs preserved as design provenance, not declared in mod.rs
```

Test files are flat under `tests/`. There is no `tests_*.rs` prefix.
Each file groups tests by topic, not by which dormant file they came
from.

## Hard rules

### Test file shape

```rust
//! <one-line description of what's in this file>

use super::prelude::*;

#[test]
fn descriptive_test_name() {
    // setup: graph + pathfinder via helpers
    // act:   one find_path / find_path_reserved call
    // assert: assert_trace!  →  assert_waypoints!  →  optional travel_time
}
```

- `use super::prelude::*;` is the only test-file import. Inline
  `use Orientation::{East, North, ...}` for whatever cardinal directions
  the test names is fine.
- Test files contain only `#[test]` functions and per-file graph
  fixtures (small `fn graph_name() -> WarehouseGraph` builders).
  Constructors, pathfinder fixtures, waypoint builders, and the
  assertion macros stay in `helpers.rs`.
- Each `#[test]` exercises **one scenario, one search call**. Splitting
  a multi-call dormant test into multiple `#[test]` units is the
  default; merging only when the comparative assertion (e.g. "loaded >
  unloaded") is the point.
- 300-LOC hard limit per file (`specs/coding-standards.md` §2), with
  the trace-data carve-out spelled out there: a `#[test]` whose body
  is dominated by an exact-match assertion against generated data
  (long `assert_trace!`, snapshot vector) does not count its data
  toward the limit. Carve-out applies when the file is a coherent set
  of canonical examples whose only overage is locked-in regression-
  guard data — `basic.rs` (42-event trace in `grid_path_with_turn`) is
  the reference case. The carve-out does **not** excuse topic sprawl:
  files that mix unrelated scenarios still split by topic.

### `assert_trace!` exact-match — non-negotiable

`assert_trace!` locks the search-decision sequence as a list of
`TraceEvent` payload structs:

- `Expand { cell, orientation, g, step }` — a state was popped from the
  open set.
- `Rotate(cell, from, to, g_after, wait_at_source)` — a rotation
  successor was pushed.
- `Move(from, to, g_after, wait_at_source)` — a move successor was
  pushed.
- `EntryIntervals { cell, intervals }` — the oracle was queried for
  free intervals on a neighbor cell.
- `RotateBlocked(...)`, `MoveReservationCheck { ... }`,
  `SourceHoldCheck { ... }`, `Commit { ... }`, `CommitEdge { ... }`,
  `Exhausted { ... }`, `Extension { ... }` — see
  `crate::pathfinding::trace`.

Each entry is `.into()`'d to `TraceEvent`. Imports come through the
prelude — never write `TraceEvent::` or `Successor::` prefixes (the
`Successor` enum is gone).

**Never relax to looser checks.** When migrating a test whose new
trace shape is unknown, write `assert_trace!(trace, [])`, run, paste
the diff back into the assertion. This rule has a recorded incident
behind it (`feedback_assert_trace.md` in user memory).

### `assert_waypoints!` exact-match

`assert_waypoints!` narrates the path chronologically. Each entry is
built via the `path_start / mv / rotate / wait / path_end` constructors
in `helpers.rs`. Same exact-match contract — paste-the-diff workflow,
no presence checks.

The trailing `Wait` waypoint after `find_path` is the dest-dwell baked
into the spatial-only path (search emits `Some(traverse_duration)` as
the terminal dwell). `travel_time` excludes that dwell.

### Error cases

For tests that assert `Err(...)`:

- Use `let result = pf.find_path(...);` (no `.unwrap()`).
- Pin the partial trace via `assert_trace!`.
- Match the variant via `assert!(matches!(result, Err(PathfinderError::Variant { .. })))`.
- No `assert_waypoints!` — there is no path.

Same-cell rejection emits no trace events (pre-flight rejects
`from == to`); `assert_trace!(trace, [])` is correct there.

## `helpers.rs` API surface

Don't expand without reason. The chimera-helper feedback memory
(`feedback_collision_config_chimera.md`) documents why visibility at
the call site is a forcing function for eventual deconstruction:
helpers that wrap `CollisionConfig` or `oracle.commit_path(...)` hide
the design smell instead of fixing it.

### Tiny constructors

| Helper | Returns | Reads as |
|---|---|---|
| `t(ms: u64)` | `SimTime` | a timestamp |
| `ms(value: u64)` | `SimDuration` | a duration |
| `c(id: u32)` | `CellId` | a cell |
| `r(id: u32)` | `RobotId` | a robot |

### Cell + graph builders

| Helper | Topology |
|---|---|
| `cell(id, kind, x, y)` | one `Cell` at grid `(x, y)`, no zone/sector/world |
| `corridor(n)` | n-cell linear `Open` corridor, 1000 mm bidi edges |
| `grid(w, h)` | w×h 4-connected `Open` grid, 1000 mm bidi edges |

Custom topologies (restricted edges, dest_only, directional lanes,
mixed cell kinds) build inline in the test file with
`WarehouseGraphBuilder::new()` + `cell()` + `b.add_edge[_bidi]`. When
two tests share a topology, factor a private `fn topology_name() ->
WarehouseGraph` into the test file (not into `helpers.rs`).

### Pathfinder fixtures

| Helper | Use when |
|---|---|
| `pathfinder(graph)` | default 1000 mm/s, 1 s turn, no parking, no trace |
| `pathfinder_speed(graph, mm_per_sec)` | custom speed |
| `pathfinder_parking(graph, parking_set)` | needs `dest_dwell = MAX` (idle parking) |
| `pathfinder_traced(graph) -> (pf, TraceCollector)` | trace required |
| `pathfinder_traced_parking(graph, parking)` | trace + parking |

Combinations beyond these (e.g. traced + custom speed) compose inline
at the call site:

```rust
let trace = TraceCollector::new();
let pf = pathfinder_speed(&graph, 1500).with_trace_hook(trace.hook());
```

Don't add a one-off helper for an inline composition. The matrix is
intentionally small.

### Waypoint constructors

| Helper | Maps to |
|---|---|
| `path_start(cell, orient, time)` | `WaypointAction::PathStart { time }` |
| `mv(cell, orient, start, end)` | `WaypointAction::Move { start, end }` |
| `rotate(cell, orient, start, end)` | `WaypointAction::Rotate { start, end }` |
| `wait(cell, orient, start, end)` | `WaypointAction::Wait { start, end }` |
| `path_end(cell, orient, time)` | `WaypointAction::PathEnd { time }` |

### `TraceCollector` + `assert_trace!` + `assert_waypoints!`

Both macros print side-by-side diffs on mismatch — paste the diff back
into the assertion to lock in a new behavior. The diff format is
`actual: Some(...)` / `expected: Some(...)`; convert via the
identity-style mapping (`CellId(N)` → `c(N)`, `SimDuration(N)` →
`ms(N)`, `SimTime(N)` → `t(N)`). Numeric separators (`ms(1_000)`,
`t(2_000)`) are mandatory at and above 1000.

## Style preferences

### Numeric separators

`ms(1_000)`, `t(2_000)`, `ms(99_999)`. Never `ms(1000)`.

### Inline `use` statements

`use Orientation::{East, North, South, West};` immediately before the
`assert_trace!` block. Pull only the cardinals the test names. Don't
import to top-of-file.

### Comments

A test name and its trace data carry the meaning. Add a short comment
when:

- The trace fan-out has a non-obvious cause (the L1 heuristic failing
  to prune row-first vs column-first L-paths) — explain so the next
  reader doesn't try to "fix" the trace.
- A tie-break or dwell decision is asymmetric — explain which winner
  the search picks and why the loser is equally valid.
- A test is a regression guard for a specific pre-pivot bug — name
  the bug ID (`bug_c891118_...`) and what regressed.

Otherwise no comment. The assertion list is self-documenting.

## Migration recipe

For migrating a dormant `tests/*.rs` file into the new shape:

1. Read the dormant file for the test list, not the test bodies. The
   pre-pivot bodies use windowed-era stats (`cells_beyond_depth`,
   `optimistic_tail_committed`) and graph helpers (`build_linear_graph`,
   `make_cell`) that no longer exist. The new bodies are written from
   scratch against the prelude.
2. Decide splits and merges. Multi-call tests usually split. Tests
   whose only assertion is comparative (loaded > unloaded, A waits
   longer than B) may stay merged or split — pick whichever lets each
   `#[test]` pin a single trace.
3. Overwrite the dormant file in place. Open with one-line doc + `use
   super::prelude::*;`. Add `mod <file>;` to `tests/mod.rs`.
4. Write each test body with placeholders:

   ```rust
   assert_trace!(trace, []);
   assert_waypoints!(result, []);
   ```

5. Run `cargo test --release -p dessim --lib --
   pathfinding::fh_sipp::tests::<file> --test-threads=1 --nocapture
   > /tmp/run.log 2>&1`.
6. Both macros emit a `=== PASTE-READY EXPECTED ===` block on
   mismatch — copy the lines between the `[`...`]` of that block
   directly into the failing assertion. Custom `Debug` on `SimTime`,
   `SimDuration`, `CellId`, `RobotId`, `TraceEvent`, `EntryIntervals`,
   and `Waypoint` is paste-ready test syntax (`t(N)` / `ms(N)` /
   `c(N)` / `r(N)` / `MAX` / `EOT`, no variant wrapper, constructor
   form for waypoints), so no sed transform is needed. **Do not
   simplify the lists in the process** — every event is load-bearing.
7. Re-run; verify green per-file, verify the full lib-test count went
   up by exactly the number of new tests (`cargo test --release -p
   dessim --lib`).
8. Checkpoint commit with message
   `test(fh_sipp): migrate <name>.rs to single-shot contract`.

## Things to never do

- Add a helper that wraps `CollisionConfig` or
  `oracle.commit_path(...)`. The struct-at-call-site visibility is
  intentional.
- Replace `assert_trace!` with looser presence checks
  (`trace.events().iter().any(...)`).
- Replace `assert_waypoints!` with a length check or a partial cell
  list.
- Merge two `#[test]`s into one because their traces happen to match.
  The trace coincidence is information ("under these distinct setups,
  the search exhausts identically"); collapsing the tests destroys
  that signal. Keep the duplication, name the tests for the scenarios.
- Pull pathfinder fixtures or graph builders into per-file helpers.
  Reuse from `helpers.rs`; if a fixture composition is needed for
  exactly one test, inline it.
- Run dessim tests in debug mode. `--release` is mandatory.

## Pointers

- Algorithm reference: `sim/dessim/docs/fh-sipp-pathfinder-algo-draft.md`.
- Phase 1 spec: `specs/putaway/turn-encoding-contract-phase1.md`.
- Trace event payload structs: `crate::pathfinding::trace`.
- The `feedback_assert_trace` and `feedback_collision_config_chimera`
  user-memory entries record the historical incidents behind the
  hardest rules in this doc.
