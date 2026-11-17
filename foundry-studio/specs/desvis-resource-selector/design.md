# Resource Selector + Station Inspector

Click a station (or bot) on the desvis State-view map and inspect its
dispatch state: which HTMs are en-route/assigned, and what blocked dispatch
for that station recently. Spans dessim (instrumentation) and desvis (UI).

## Event schema (dessim)

Three new `SimEvent` variants (`sim/dessim/src/events/station_diag.rs`):

| Event | Fields | Emitted at |
|---|---|---|
| `HtmAssignment` | robot, station, tote, `StationArrivalSource`, dispatch_time, quoted_arrival | Where the HTM identity is first bound: `HtmPickup` commit (pick + putaway from-rack chains) and the direct-putaway feed commit (`create_direct_chain`) |
| `HtmArrival` | robot, station, arrival | `HtmDeliver` commit, alongside `StationArrivalDiagnostics::record` (the committed path arrival at the presentation cell) |
| `DispatchRejection` | station, at, `DispatchRejectionReason`, count, optional detail | Station-attributable rejection points (below) |

**Deviation from the brief:** assignments are not emitted at the pick
dispatch commit point because no HTM exists there — from-rack chains select
their HTM later, in `HtmPickup`. Emitting at HTM-binding time is the earliest
moment a truthful `robot_id` exists. The quoted arrival still carries the
dispatch-commit quote via `StationArrivalPlan`.

`DispatchRejectionReason` and emission sites:

- `ConwipCapSaturated` — pick `try_schedule_station`, cap floor `None`.
- `OrderAdmissionBlocked` / `OrderInventoryUnavailable` — pick
  `start_new_order_for_topup`; aggregated per pass (one event per reason
  with a count) so a deep blocked ready-queue emits O(1) events.
- `TopupNoDispatch` — top-up order started but no line dispatched.
- `CounterRace` — post-commit WIP-cap re-check cancelled the reservation.
- `NoProposal` — pick: candidate lines existed but every relay quote came
  back empty (`quote_line` folds no-candidates and headroom into "no
  quote"); putaway: `stage_one` found no admissible source.
- `RelayNoCandidates` / `RelayHeadroomExhausted` — putaway
  `from_rack_proposal` quote errors (station threaded in from the caller;
  relay internals untouched).
- `AccessCellConflict` — commit-time `reserve_selected_relay_for_retrieve`
  failure (pick `commit_line`, putaway `create_from_rack_chain`): the quoted
  selection went stale between quote and commit (slot/access cell taken,
  availability moved). The relay error's reason string rides in `detail`.

Rejections with no station in scope (e.g. inside relay selection) are not
emitted — no station is invented. Emission is gated on an attached event
sink (`ModelState::emit_dispatch_rejection` builds the detail string lazily);
existing `trace_event!` calls are untouched.

## Data flow (dessim → desvis)

Observer (`observer/station_diag_handler.rs`) folds events into a per-slice
`StationDiagRecord` (`observer/station_diag.rs`) on `TimeSliceData`:

- `assignments` / `arrivals` — verbatim, bucketed by their sim timestamp.
- `rejections` — aggregated per (station, reason) within the slice: count +
  most recent occurrence + its detail. Bounds hot rejection paths.
- `pipeline_depths` — per-station depth from `PipelineSnapshotEvent`
  (currently only emitted by the uncompiled quarantine tree, so live runs
  show "n/a"; the plumbing is ready if the event goes live).

Timestamps are absolute `SimTime`s, not slice offsets — the slice is only a
transport bucket. The field is serde-defaulted and skipped when empty on both
`TimeSliceData` and `SliceRecord`, so old `.desprofile`s load and new
profiles carry the payload. Live polling and profile playback therefore see
identical data.

## desvis store + queries

`crates/desvis/src/station_diag/mod.rs` — `StationDiagStore`, a time-sorted
flattening of every slice's record, cached on `DesvisApp` and invalidated
wherever `chart_cache` is (the observer re-sends dirty slices in place, so
the store is rebuilt, never appended). All queries are as-of a cursor:

- `inbound_htms_at(station, cursor)` — assignments ≤ cursor with no matching
  arrival ≤ cursor; arrivals close each robot's earliest open assignment.
- `rejections_in_window(station, cursor, REJECTION_WINDOW)` — trailing 60s
  (single constant), grouped by reason, count-sorted, last occurrence +
  detail. Window accuracy is one slice width (per-slice buckets).
- `active_assignment_for_bot(robot, cursor)` — the bot inspector's view.
- `latest_pipeline_depth(station, cursor)`.

Unit-tested in `station_diag/tests.rs` (assignment/arrival matching across
re-assignment, window edges, per-station isolation).

## UI behavior

- **Selection** (`app/central_panel.rs`): single click in the State view
  resolves via `screen_to_grid` — bot marker on the cell wins, else any
  station-owned cell (position, presentation, queue path — lookup built once
  from the map in `grid_data::build_station_metas`, also in playback mode
  where `self.map` is `None`), else clear. Double-click fly-to unchanged.
- **Highlight**: selected station's cells outlined / selected bot ringed via
  egui painter over the wgpu canvas (`grid_to_screen`); no pipeline changes.
- **Inspector** (`app/inspector.rs`): right-anchored closable `egui::Window`
  (closing clears the selection). Renders as-of the playback cursor when
  scrubbing is engaged, else the live edge (matching what the map shows).
  Station: type / presentation cell / queue length / bins, pipeline depth
  (or "n/a"), inbound HTMs (source, tote, dispatch time, ETA countdown,
  overdue in red), recent blockers (reason × count, last seen, detail).
  Empty state says "no dispatch activity in window" explicitly. Bot: id,
  type, current cell; HTMs additionally show their active assignment + ETA.

## Known gaps (Phase 1)

- `PipelineSnapshotEvent` is not emitted on the live solver path, so the
  pipeline-depth row reads "n/a" today.
- `cargo test -p dessim --lib` does not compile on the merge-base
  (pre-existing `htm_store.rs` test-fixture breakage, stale
  `FHSippPathfinder::new` signature) — flagged separately; the new store
  test lives in desvis and passes.
- Rejection-window counts are slice-granular: a bucket straddling the 60s
  edge contributes its whole slice count.

---

# Phase 2 — bot/relay depth, in-scene highlight, path overlay

> **28 October 2026:** #596 replaced the robot state that this section, Phase
> 4's bot strip and Phase 6's HTM idle descriptor build on, and turned the
> relay pickup waits into the fork operations. The slices now carry `activity`
> records (`RobotActivityRecord { robot_id, kind, start, end }`) in place of
> `state_transitions`, and `htm_relay_pickups`, from the `HtmRelayPickup` event
> (`HtmRelayPickupEvent`), in place of `relay_pickup_waits`, from
> `RelayPickupWait` (`RelayPickupWaitEvent`). The inspector
> reads them through `activity_at`, `activity_spans` and
> `htm_relay_pickups_in_window`, which replace `bot_state_at`,
> `bot_state_intervals` and `relay_pickup_waits_in_window`, and
> `*_activity_counts` replace `*_state_counts`. `RobotStateLabel`,
> `StateTransitionEvent` and `handle_state_transition` are gone. What follows
> is the design as first built, and its measurements are of that design.

## New per-slice payloads (dessim)

`StationDiagRecord` (kept its Phase-1 name; now covers bots + relays) gains
three serde-defaulted vecs, all timestamped with absolute `SimTime`:

- `state_transitions: [(robot_id, RobotStateLabel, at)]` — folded from the
  existing per-bot `StateTransitionEvent` (which previously only fed
  aggregate `*_state_counts` + duty and never reached desvis). Recorded from
  `handle_state_transition`.
- `vtm_idle_reasons: [(robot_id, VtmIdleReasonKind, gap_ms, relay_cell, at)]`
  — folded from `VtmIdleReasonEvent`; `Continuous` (no gap) is skipped.
- `relay_pickup_waits: [(robot_id, relay_cell, wait_ms, at)]` — folded from
  `RelayPickupWaitEvent`.

`HtmAssignmentEvent`/`HtmAssignmentRecord` gain `relay_cell: Option<u32>` (the
relay HTM-access cell the chain fetches through; `None` for direct putaway),
so the relay inspector can list HTMs routed through a clicked relay.

**Relay cell resolution.** `VtmIdleReason`/`RelayPickupWait` carry `RelayId`,
but desvis keys relays by clicked *cell*. Rather than widen those two events
(other consumers), the observer resolves `RelayId → htm_access cell` via a new
`MetricsConfig::relay_htm_access_cells` map (built in `solver.rs` from
`data.map.relays`), matching how `RelayStateChangeEvent` already keys relays
by htm_access cell.

## No BotPlanSnapshot event — plan data already flows

The brief allowed skipping a per-slice plan snapshot if the plan is
recoverable from existing events. It is: `handle_nav` splits each committed
Nav waypoint chain across **all** slices it spans, so a bot's future path is
already in `bot_locations[bot].steps` of current + subsequent slices the
moment the leg commits (even ahead of the sim clock — the observer buckets by
waypoint time). The path overlay reads these directly; the bot inspector's
"current task / next waypoints" reads the cursor slice's steps. This avoids
the flagged per-slice-×-bots snapshot cost. The only genuinely new per-slice
data is the small metadata above (state label, idle reason, pickup wait) —
not geometry.

## desvis store (Phase 2 queries)

`StationDiagStore` gains: `bot_state_at`, `latest_vtm_idle_reason`,
`relay_pickup_waits_in_window`, `inbound_htms_via_relay`, and
`recent_{station,bot,relay}_events` (newest-first, capped at
`RECENT_EVENTS_MAX = 10`, windowed by `RECENT_EVENTS_WINDOW = 120s`). A
second unit test covers bot-state-at-cursor, VTM idle reason, relay window,
relay routing, and recent-event ordering.

## UI (Phase 2)

- **In-scene highlight** (wgpu): `BotInstance` gains a `highlight` field
  (0 none / 1 primary / 2 secondary) threaded through `build_bot_instances`,
  the vertex layout, and `bots.wgsl` — primary (selected bot) tints warm
  yellow + scales 1.35×; secondary (a selected station's/relay's inbound
  HTMs) tints cyan. The occluded pass tints too and bumps highlighted-bot
  alpha (0.25→0.55) so the selection reads through racks.
- **Path overlay**: egui polyline over the canvas of the selected bot's
  remaining committed steps from the cursor, bounded by
  `PATH_OVERLAY_HORIZON = 60s` / `PATH_OVERLAY_MAX_POINTS = 120`. Occlusion
  ignored (diagnostic).
- **Relay selection**: click a relay's position / htm-access / vtm-access
  cell → `SelectedResource::Relay { htm_access_cell }`. Priority: bot >
  station > relay. `RelayMeta`/`relay_by_cell` built once from the map.
- **Inspector** split into `app/inspector/{mod,station,bot,relay}.rs`.
  Bot: current cell, state (+ since), current task/leg + next waypoints (from
  committed steps), blocked/idle (VTM idle reason; a Wait step spanning the
  cursor → "waiting at cell since t"; else labeled unknown), HTM active
  assignment + ETA. Relay: id/block/cells, reserved-vs-free at cursor (from
  `relay_state`), pickup-wait window, HTMs via relay. All resources: a
  "Recent events" feed. A shared `resource_cursor()` keeps inspector +
  overlays on the same "now".

## Known gaps (Phase 2)

- The path overlay's re-plan safety currently rests on `NavCanceledEvent`
  being emitted only from the uncompiled `quarantine/` tree: committed Nav
  waypoints are bucketed into future slices at ingest, and
  `strip_bot_steps_from` removes stale future steps on cancel but does NOT
  re-dirty slices already flushed to desvis. If a live `NavCanceled` emit
  site ever appears, a re-planned bot can briefly show a ghost path (a route
  it will no longer take) until the affected slices are re-emitted. Fix at
  that point: re-dirty (or delta-patch) flushed slices on cancel.
- `state_transitions` is the highest-volume new payload (every bot, every
  transition). See the completion report's slice-size assessment; if it grows
  uncomfortably the mitigation is to record only currently-selected bots —
  but that breaks scrub-back for a newly-selected bot, so it is kept
  unconditional for now.

---

# Phase 3 — "Why is this station idle?"

## New dispatch rejection reasons (dessim)

Two previously-silent dispatch outcomes in `pick/dispatch.rs
try_schedule_station` now emit a `DispatchRejection` (aggregated per pass,
lazy detail, sink-gated, station-attributable — same discipline as Phase 1):

- `NoReadyOrders` — the station had capacity to open another order but the
  ready queue held nothing eligible for it (demand-starved). Emitted once per
  pass, driven by a new `TopupOutcome::NoReadyOrder` return from
  `start_new_order_for_topup` (which distinguishes "no eligible ready order"
  from "a ready order existed but was blocked", the latter already covered by
  `OrderAdmissionBlocked` / `OrderInventoryUnavailable`).
- `NoOpenLines` — open orders exist but every line is already dispatched (in
  flight); the station is waiting on inbound arrivals, not a fresh decision.
  Emitted from `dispatch_soonest` when `build_candidates` yields nothing but
  the open-order list was non-empty.

**Inventory missing vs inaccessible — not faked.** The dispatch layer already
carries this split across two existing reasons, so no speculative third reason
was added: `OrderInventoryUnavailable` fires at the ledger level
(`reserve_for_order` — the SKU's unreserved units don't exist), while
`RelayNoCandidates` fires at the fetch level (a tote with the SKU exists but
no relay/HTM can bring it). The plain-language mapper groups the fetch-side
reasons under "fetch capacity (relay / HTM side)".

**Putaway PRD_HR gate — deliberately not per-station-attributed.** The gate
lives on the single shared arrival queue in `PutawayInduction::try_schedule`
(`select_fill(net_time)` picks whichever station has a presenting empty), so a
net-time deferral is a global-pipeline condition, not attributable to a
particular station at its decision point. The station-attributable putaway
signal remains `NoProposal` from the supply loop's `stage_one`.

## Per-station cycle records (dessim)

`StationDiagRecord` gains `station_cycles: [(station, start, dwell)]`, folded
from the already-handled `CycleCompleteEvent` (which carries dwell-start
`sim_time_ms` + `dwell_ms`). A cycle spans `[start, start+dwell)`. Serde-
defaulted like the Phase-2 additions. Lets desvis decide Serving (a cycle
covers the cursor) vs Idle-since-last-completion.

## desvis synthesis

`StationDiagStore::station_idle_diagnosis(station, cursor, window)` returns a
`StationDiagnosis` with strict precedence:
1. `Serving { since }` — a cycle span covers the cursor.
2. `WaitingInbound { count, next_eta, overdue }` — assigned HTMs en-route;
   `overdue` is the "HTM not in time" case (quoted arrival already passed).
3. `Rejected { cause, reason, count, last_at }` — the dominant (highest-count)
   recent rejection, mapped to plain language via `rejection_cause`.
4. `IdleUnrecorded { since }` — no dispatch signal in the window; never a
   guess. Carries the last cycle completion if any.

Store split for the 300-line limit: core queries in `station_diag/mod.rs`,
the diagnosis + `rejection_cause` in `station_diag/diagnosis.rs`, the
recent-events builders in `station_diag/recent.rs` (both descendant modules
reading the store's private vecs directly). The station inspector renders a
prominent color-coded **"Status:"** line (green serving / amber waiting or
blocked / red overdue-inbound / grey unrecorded) with one-line evidence
beneath. A third store unit test covers the precedence and window edges.

## Per-pass event volume (idle stations)

An idle station now emits exactly one aggregated `DispatchRejection` per
planning pass (whichever single reason its `try_schedule_station` path hits),
never one per order. The observer folds same-(station,reason) rejections
within a slice into a single `RejectionAggregate` (count + last occurrence),
so the per-slice footprint is bounded by distinct (station × reason) pairs,
not by pass count — identical to the Phase-1 hot-path reasons. See the
completion report for the measured aggregate.

---

# Phase 4 — per-resource timeline strip (click-to-scrub)

## Timeline strip in the inspector

A compact (~46px) full-run `[0, max_ms]` gantt/histogram strip for the
selected resource, drawn at the top of its inspector body so idle stretches
are visible at a glance and clickable. All data comes from the per-slice diag
store + time series — no new slice payloads this round — so it works live and
in playback identically.

Strip content by resource:
- **Bot** (`bot_strip`): colored `state_transitions` intervals reusing the
  `theme::STATE_*` palette the utilization Gantt already uses. VTM idle
  intervals whose reason was recorded render a shade darker and name the
  reason in the hover tooltip (e.g. "Idle — awaiting demand").
- **Station** (`station_strip`): green serving spans from `station_cycles`
  over a pale idle track, with amber rejection ticks in the gaps so a
  "blocked" gap reads differently from a quiet/demand-starved one.
- **Relay** (`relay_strip`): blue reserved spans reconstructed from the
  per-slice `relay_state` transitions (walked into absolute-time spans, open
  reservations run to `max_ms`) over a free track, plus pickup-wait ticks.

## Cursor sync (both ways)

The global playback cursor is drawn as a vertical line in every strip. A
click or drag inside the strip returns `Some(sim_ms)` up through the
inspector body → `draw_inspector_window` → `engage_cursor` (the same
mechanism the bottom Gantt/chart panels use), so the map, inspector fields,
Status line, and strip all move together. Mirrors `gantt.rs`'s
`lane_x_to_time` + `interact_pointer_pos` pattern.

## Interval extraction + queries

New store queries in `station_diag/timeline.rs` (descendant module, reads the
store's private vecs): `bot_state_intervals` (contiguous state runs, last runs
to `run_end`, VTM idle intervals annotated with `vtm_idle_reason_at`),
`station_serving_spans`, `station_rejection_marks`, `relay_pickup_marks`.
Relay reserved spans are reconstructed inspector-side (`relay_reserved_spans`)
because the raw `relay_state` transitions live on `SliceRecord`, not the store.
A fourth store unit test covers interval extraction (state runs, idle
annotation, `run_end` tail, serving spans, `covers`).

## No zoom; performance

No zoom knob — the bottom Gantt/chart panel already covers detailed
multi-lane inspection; the strip is the glance-level "where is this one
resource idle" view. Interval extraction runs per frame for the single
selected resource (one filtered pass over the sorted vecs, a few thousand
comparisons); measured negligible at cencora scale, so no per-frame cache was
added. The store's existing invalidation point (`station_diag_store = None` on
time-series change) remains the seam to cache against if a future profile
makes it hot. *(Superseded in Phase 5: the cache was implemented.)*

---

# Phase 5 — full committed paths, per-HTM colored paths, committed pass, timeline cache

## Full committed path (was 60s window)

`gather_bot_path_points` no longer truncates at a 60s time horizon. Because
Nav commits per leg and the observer buckets each committed waypoint chain
into the slices it spans, walking a bot's committed steps from the cursor to
the end of the time series **is** the full committed plan — the next resource
plus any further committed legs. The path now **ends where the committed plan
ends**; only a large safety cap (`PATH_OVERLAY_MAX_POINTS = 2048`) bounds a
pathological run.

## Per-inbound-HTM colored paths (station selected)

When a station is selected, `paint_resource_paths` draws one full-committed
path per inbound HTM (`inbound_htms_at`), each colored by
`inbound_path_color(robot_id) = PALETTE[robot_id % 6]` — stable per robot so a
bot keeps its color as others come and go. The station inspector's inbound
rows carry a matching color chip. A selected *bot* still gets its single red
path (no station is selected then, so the two never conflict).

## Committed planner pass

`pass_id` is threaded into both `HtmAssignmentEvent` emit sites
(`HtmPickup::try_schedule` for from-rack; `create_direct_chain` via
`stage_pass`/`stage_one` for direct putaway) → `committed_pass` on the event +
`HtmAssignmentRecord` (`#[serde(default)]`, so pre-Phase-5 profiles load; live
passes are ≥ 1, so a defaulted 0 reads as "unrecorded"). Displayed in the bot
inspector assignment block ("committed in pass N") and the station inbound
rows ("pass N").

### HTM-field completeness sweep (deliverable 4)

The bot inspector's assignment block previously showed only station, source,
dispatched-at, and ETA. Missing stored fields now added: **tote** (`tote_id`),
**relay cell** (`relay_cell`, or "none (direct)"), and the new **committed
pass**. The station inbound rows already showed tote; they now also show relay
cell and pass. No other stored-but-undisplayed HTM fields remained — the
`HtmAssignmentRecord`/`InboundHtm` field set (robot, station, tote, source,
relay_cell, committed_pass, dispatch_time, quoted_arrival) is now fully
surfaced in at least one inspector view.

## Timeline cache (audit M3)

`ResourceTimeline` (bot intervals / station serving+rejections / relay
reserved+pickups) is cached on `DesvisApp` (`resource_timeline_cache`), built
once by `ensure_resource_timeline` when the selection changes or the cache is
empty, and invalidated at exactly the sites that clear `station_diag_store`
(polling ×2, sim start/reset). The strips read the cache instead of
re-extracting per frame — removing the per-frame bot idle-reason nested scan
and the relay all-slices walk+alloc+sort the audit flagged.

## Strip tooltip tolerance (audit L5)

Tick hover tolerance is now a fixed `TICK_HOVER_TOLERANCE_PX = 4.0` screen
pixels, converted to a ms tolerance per strip via `max_ms / strip_width_px`,
rather than ±1% of the run (which was ±6 min on a 10h run). Passed into the
`describe` closure so station/relay tick hovers use it.

---

# Phase 6 — cross-linking, jump-to, live pipeline depth, HTM idle descriptor, real leg semantics

## Inspector cross-linking + jump-to (pure desvis UI)

Inspector rows are clickable links that retarget the inspector. Bodies now
return an `InspectorAction { scrub, select }` bubbled out of the window body;
`draw_inspector_window` applies `select` (dropping the timeline cache so the
strip rebuilds). Links: station inbound-HTM row → that HTM; bot assignment's
station / relay-cell → that station / relay; relay "HTMs via relay" row →
that HTM. A "Jump to:" header (kind combo + id combo over
`station_metas`/`bot_positions`/`relay_metas`) selects any resource by id;
`draw_jump_to` returns the pick through the same `select` path (a jump
overrides a row click the same frame).

## Live pipeline depth (closes the "n/a")

`PipelineSnapshotEvent` was emitted only from the dead quarantine tree. Now
emitted from the live pick `commit_line`, once per committed chain, sampling
the station's CONWIP inflight counter (`count_at`) at the chain's
`relay_start`. **Why relay_start, not the planning barrier:** the offline
solver plans pick dispatch at a fixed `now` (≈ embargo / t=0), so sampling at
`now` collapses every snapshot to one timestamp — a static value. `relay_start`
spans the run, giving a time-varying depth profile. Bounded at one snapshot per
committed chain (≈ the assignment stream); sink-gated; no schema change
(populates the existing per-slice `pipeline_depths`). On cencora the depth
reads ~1 — a true low-overlap property of the well-paced JIT dispatch, not a
bug (chains rarely overlap at each other's start).

## HTM idle descriptor (honest, no new classifier)

HTMs record no idle *reason* (only Traveling/Loading/Processing/Idle;
WaitingForOperator is quarantine-dead). The bot inspector *describes* — does
not classify — from existing store data: when an HTM is Idle at the cursor,
"idle, no assignment" vs "assigned, not yet moving" (active-assignment
presence) + "idle since <t>" (state transition). Non-idle → no descriptor; no
transition recorded → labeled unknown. Pure display composition of
`bot_state_at` + `active_assignment_for_bot`; no new event/query.

## Real leg semantics — BotPlanSnapshot

The bot inspector's "current task" was derived from committed
Move/Wait/Rotate steps, not the planner's `Leg` kinds. New
`BotPlanSnapshotEvent` carries each bot's full committed legs (kind +
destination cell + action-ready time), capped at `PLAN_SNAPSHOT_MAX_LEGS = 32`,
mapped to a serializable `PlanLegKind`.

**Emission hook + deviation from "per-slice":** the offline solver has no
per-slice tick — the whole plan is known only at planning completion — so legs
are emitted **once per bot** from the existing post-run `for_each_bot` walk in
`solver.rs` (event channel still open), read via `plan.legs()` +
`waypoints_for_leg()`. desvis filters by cursor (`plan_legs_from`) to show the
"remaining legs from the frontier", so whole-plan-once is strictly cheaper AND
more useful than a per-slice snapshot. Bucketed into the slice of the first
leg's enter time (transport only; the desvis store flattens by robot_id).

The bot inspector shows the real leg label + destination ("current leg:
station approach → cell N") and the next few legs, above the finer step-level
detail. Serde-defaulted per-slice `plan_legs` (old profiles load).

**Payload cost (honesty):** measured +20,636 bytes / **+0.9%** profile size
(phase1 20m: 3161 legs, 400 bots, avg 7.9/bot, cap 32 hit for some) — far
cheaper than the per-slice `state_transitions` (+6.5%), because it is one
snapshot per bot rather than per-slice × fleet.

---

# Phase 7 — fuzzy jump, zoom-scaled paths, path truncation, putaway utilization

## Fuzzy jump-to search box

`jump.rs` replaced the kind+id dropdowns with a text search box + ranked
result list. `fuzzy_score(needle, haystack)` (both lowercased) scores
exact > prefix > contiguous-substring > subsequence, earlier positions
higher; `rank_jump_candidates` builds labels for all stations/bots/relays,
filters+ranks, caps at `MAX_JUMP_RESULTS = 8`. Click a row or Enter on the top
match; routes through the existing `InspectorAction.select`. No new deps;
`fuzzy_score` unit-tested.

## Zoom-scaled path stroke width

Path overlays used a fixed 4px stroke (a worm zoomed out, a thread zoomed in).
`path_stroke_width` measures on-screen cell size by projecting a unit grid edge
at the grid center (stable under perspective) and sets width =
`clamp(cell_px * PATH_WIDTH_FACTOR(0.12), MIN(1.5), MAX(6.0))`. Applied to the
selected-bot path and every per-inbound-HTM path; the selection outline and
bot ring are unchanged.

## Path truncation to one delivery cycle

`gather_bot_path_points` walked the full committed plan. Now bounded by a
`PathBound` (`path_bound_for`): **primary** — the bot's active assignment names
a target station, so the polyline stops right after the step entering that
station's presentation cell (for the multi-HTM station view, that is each
inbound HTM's approach into the selected station). **Fallback** — no station
target (returning empty / parking): stop at the next committed leg boundary
(`plan_legs_from` next-leg enter time). Else the `PATH_OVERLAY_MAX_POINTS`
safety cap. Inputs are store-tested; the truncation loop is app-level over
`time_series` geometry.

## Putaway HTM dwell counts as station utilization

**Investigation:** `CycleCompleteEvent` *does* fire for putaway (from
`PutawayInduction::fill`), but carried `dwell_ms = 0` ("present duration is
variable, not a fixed operator hold"), so `station_cycles` spans were
zero-width and putaway stations read all-idle — the pessimism the user flagged.

**Interval now counted (service-dwell — user's fidelity call):** the deposit
**service** window only — `dwell_ms = self.pacing_interval` (= `3600 /
max_presentations_per_hour`, the configured rate-ceiling service interval),
giving a busy span of `[fill_at - pacing_interval, fill_at)`. `sim_time_ms =
fill_at` is unchanged so the putaway-**rate** metric is untouched — only the
busy-span width changed. A kind-aware `cycle_busy_interval` reconciles the two
conventions in one place — pick fires at the dwell **start** (`[t, t+dwell)`,
`dwell = pick service time`), putaway at the dwell **end** (`[t-dwell, t)`,
`dwell = pacing_interval`) — used by both the Gantt `station_timelines` and the
inspector `station_cycles`. This makes pick and putaway utilization directly
comparable (both = service time / wall). Rate/log/gap metrics still key off the
raw `sim_time_ms` — no regression.

**Why service-dwell, not full occupancy (rejected option):** the first cut
counted `[present_start, fill_at)` — the empty tote's full present-until-filled
occupancy — which ran 0..~47 min because the empty stages early and then waits
for its PRD_HR net-time gate. That counts gate-waiting as utilization (the
robot deposited and left; the cell is occupied but no work happens), swinging
from all-idle pessimism to the opposite over-count. Rejected per the user's
call: only the actual deposit service counts busy; the gate-wait reads idle.

**Grumpy-hat finding:** the vestigial `dwell_secs` config field is read by
nothing on the timing path — the real service/rate quantity is
`pacing_interval = 3600 / max_presentations_per_hour`. phase2 does NOT override
`max_presentations_per_hour`, so it defaults (see decode for the live value);
the old "4.5s putaway dwell" note was the stale `dwell_secs` model. Using
`self.pacing_interval` is the faithful, non-hardcoded source of truth.
Pick stations verified unchanged (fixed 9000ms dwell); putaway serving spans
are now `pacing_interval`-wide (headless phase2 decode).

---

# Phase 8 — path overlay fixes + waypoint aggregation (live-GUI feedback)

## Flash / "too many waypoints" — root cause + fix

Confirmed chain: (1) `frame.rs` paints the central panel (paths) *before* the
inspector rebuilds the diag store; (2) polling clears `station_diag_store` on
every dirty-slice batch (~10×/sec live); (3) `bot_highlight_set` only rebuilt
the store for Station/Relay selections — not Bot; (4) `path_bound_for` with a
missing store fell back to `Capped` = the **whole committed plan**. Net: with
a bot selected in a live run, one frame per poll drew the full-plan path,
then snapped back to one cycle — a repeating short→huge flash. Secondary:
no-assignment + no-remaining-legs also fell back to whole-plan (plan legs
only stream at planning completion, so that was the live-run norm).

Fix: `bot_highlight_set` builds the store for Bot selections too, and the
fallback bound is now `PathBound::NearHorizon` — a short named budget
(`PATH_FALLBACK_MAX_CELLS = 32`) — so the overlay can never balloon to the
whole plan. A bound-source change at an assignment boundary still changes the
path once (legitimate); nothing oscillates because every frame resolves the
same bound for the same state.

## VTM paths bound to the current cycle

**Definition (documented):** a selected VTM's path stops at the end of its
current fetch-or-deposit motion — the first remaining `VtmForkOp` leg's end;
if the first remaining leg *is* a fork op (the VTM is completing it now), the
end of the **following** `VtmForkOp`, covering the next travel+fork pair. No
fork op in the `VTM_BOUND_LEG_LOOKAHEAD = 8` leg lookahead (e.g. live runs,
where legs stream only at completion) → near horizon. Factored as
`vtm_cycle_bound`, unit-tested.

## Waypoint list run-length aggregation

The "next waypoints" list showed one row per step (per cell). New
`inspector/steps.rs` aggregates for display: straight same-direction Move
runs collapse to `Move A→B (N cells, Xs)` (direction change starts a new
row; a missing grid mapping breaks the run rather than guessing);
consecutive Waits coalesce to `Wait (Xs)`; Rotates likewise. **Limitation:**
`BotStepKind` has no Dwell variant — a station/present dwell is
indistinguishable from any other Wait in step data, so dwells render as
`Wait`. The row cap applies to aggregated rows with an overflow count.
RLE unit-tested (straight run, direction change, wait coalescing, single
step).

## First-vertex anchoring

The polyline began at a cell center (slice-start or a step cell possibly
*behind* the bot) while the marker renders at the interpolated frac position
— the first segment swung/lagged and could point backward. Now
`paint_one_path` prepends the bot's marker position (same
`frac_col/frac_row + 0.5` the marker uses) as the anchor, and the gather
skips steps whose span ends ≤ cursor so the first emitted vertex is the
in-progress step's destination (the cell the bot is moving toward). Applies
to the selected bot and every per-inbound-HTM path (each anchored at its own
HTM marker).

Housekeeping: the path overlay concern (bounds, stroke, gather, palette)
moved from `selection.rs` to `app/path_overlay.rs` for the 300-line limit;
pure move.
