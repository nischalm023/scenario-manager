# Dematic Metrics Reporting — Detailed Implementation Plan

Status: landed as the corrected sequential stack — see §14 for the record
Branches: PRs #293, #298, #299, #294–#297, plus the integration slice
  (the original `codex/dematic-kpis` cumulative lineage is superseded)
Umbrella: GitHub #254
Cases: #255–#262

## 1. Outcome

Deliver one auditable metrics reporting system whose canonical measurements drive:

1. Desvis interactive metric tables and charts.
2. XLSX normalized/tabular data.
3. Native XLSX charts.

The same run, scope, bucket, filters, numerators, and denominators must produce
the same values on all three surfaces. A UI widget or workbook formula must not
become an independent metric definition.

## 2. Confirmed product decisions

- “KPI” is stakeholder terminology for a metric, not a separate product
  concept. Do not create a separate
  `KPIs` top-level tab.
- Rename the current `Charts` bottom-panel tab to `Metrics`; keep
  `Utilization` beside it as the station/robot Gantt diagnostic.
- The `Metrics` surface contains the existing general time-series view plus
  the curated DA50, buffer/storage, and distribution views. Tables and charts
  are two representations of the same metrics, not separate product areas.
- User-facing terminology:
  - Tote, not Dematic “case”.
  - Order, not Dematic “pallet”.
  - Pick station, not customer-specific “Palletizer”.
  - DA50 and D30 are the only customer bot labels; do not expose VTM/HTM in
    product labels.
- Provide both full-run and post-embargo DA50 totals.
- “Alignment” is the DA50 motion model's buffer/alignment time.
- Apply alignment at every storage- and relay-side fork interaction.
- Parking and “travel to unblock another DA50” remain unsupported future
  states; do not report a misleading zero.
- Track storage slots occupied by any tote, an empty tote, and a non-empty tote.
- Trailing window and output resolution are independent selectors.
- Suppress leading output until a complete trailing window exists.
- A table-row hover drives a persistent composition detail:
  - aisle row → that aisle across pick stations;
  - DC row → that DC across pick stations.
- “Tote count” means completed tote presentations. Re-presenting the same
  physical tote counts again because the metric measures workload flow, not unique
  inventory identities.

Remaining confirmations must not block the foundational work:

- Whether alignment includes departure/de-alignment as well as approach.
- Whether diving-board views should default to instantaneous occupancy,
  capacity-time utilization, or show both.
- Whether trailing output alignment must be configurable beyond
  simulation-start alignment.

## 3. Current-system audit

### Existing strengths

- `VtmNavEvent` already records DA50 rail moves and fork interactions.
- `VTM Journey Legs` records duration-bearing travel, mechanical, and
  selected-plan wait rows.
- `VtmCycleRecord` records direction, empty/loaded travel time, rack/relay/tote
  identity, and pre-cycle selected-plan wait.
- Desvis already has a resizable bottom panel with `Charts` and `Utilization`
  tabs.
- The profile contains raw time slices, tote/relay transitions, cycle/journey
  logs, duty timelines, and map/config provenance.
- XLSX already emits stable raw/summary sheets.
- Per-aisle laden storage-slot count and capacity already exist.

### Gaps that affect the architecture

- The detailed DA50 motion model applies `BUFFER_APPROACH_SECS` only to the
  relay-side composite `buffer_secs`; storage-side `fork_secs` has no
  alignment component.
- Relay-side `buffer_secs` combines alignment and transfer, preventing an
  authoritative Alignment/Pickup/Drop split.
- DA50 travel distance is not split by load at the aggregate level.
- Wait reason counts exist, but the metric needs duration and full reconciliation
  to the Waiting state.
- Pick rows lack order ID, tote ID, source aisle, source relay, and presentation
  identity.
- Per-aisle storage state tracks only non-empty totes.
- Relay playback represents reservation state, not a distinct physical-tote
  occupancy interval.
- Current Desvis rolling sums are centered and emit at source-slice resolution.
- XLSX does not generate native charts.

## 4. Desvis information architecture

### Bottom-panel navigation

```text
Metrics | Utilization

Metrics:
Time series | DA50 state | DA50 throughput | Buffer & storage |
Aisle distribution | Destinations | D30 state
```

("DC distribution" was renamed "Destinations" with the destination-striping
slice — same axes, plus the planned stripe matrix and the pick-destination
throughput chart; destination = DC for Cencora. See
specs/putaway_pick_locality/plan.md.)

D30 implementation views were gated on #261 producing an approved taxonomy.
Daniel approved it 2026-07-29 (#261 comment), settling two rulings: Station
processing is the ninth exhaustive state — landed as the `D30 state` view
below (see §14) — and demand-shape summary cards lead with the mean, which
awaits the #274 demand-shape slice and is not part of the state view.

### Shared metric controls

The Metrics surface owns a typed, persistent view state:

- Scope: Full run | Post-embargo.
- Period: Overall | Hourly.
- Time range: follows the global playback cursor/range when a ranged view is
  introduced; initial delivery is run-scope.
- Measure: section-specific count/percentage/distance/duration selector.
- Window and resolution: visible only for trailing-capable time-series views.
- Unknown/unsupported state is displayed explicitly.

Changing a control invalidates only the affected canonical report cache.

### Main workspace

Use the wide bottom-panel shape:

- Primary area: table/matrix or dominant chart.
- Detail area: composition pie, legend, exact values, and denominator.
- At narrow width, stack detail below the primary area.
- Do not use a modal for row hover.

Section layouts:

1. DA50 state
   - Overall: full-run and post-embargo state pies plus exact duration table.
   - Hourly: 100% stacked bar plus hourly table.
   - Waiting row expands or switches the detail pane to selected-plan reasons.
   - Unsupported park/unblock rows remain visible and labeled unsupported.
2. DA50 throughput
   - Time-series chart and operation × payload table.
   - Operation filters: pickup storage, drop relay, pickup relay, drop storage.
   - Payload filters: all, empty, non-empty, unknown.
   - Independent trailing-window and resolution controls.
3. Buffer & storage
   - Aisle rows; columns for all totes, empty totes, non-empty totes, and
     diving-board occupancy.
   - Time-series detail for the selected row.
4. Aisle distribution
   - Rows = aisles; columns = pick stations.
   - Toggle Tote count | percentage within pick station.
   - Heat intensity applies to cells.
   - Row hover updates pick-station composition detail.
5. Destinations (né DC distribution; destination = DC for Cencora)
   - Rows = destinations (DCs); columns = pick stations.
   - Matrix source toggle: Completed (facts, below) | Assigned stripes (the
     run's stripe plan). Planned and completed are never mixed in one cell.
   - Completed: toggle Tote count | Order count | percentage within DC.
   - Row hover updates pick-station composition detail.
   - Below the matrix: pick-destination throughput time series (distinct
     presentations per bucket, one line per destination, legend annotated
     with the destination's primary stripe station).

### Interaction rules

- Hover is transient; click/focus pins a row for keyboard and careful analysis.
- Cell hover shows exact value, numerator, denominator, and unknown count.
- Row composition always uses the currently filtered fact set.
- A chart click may scrub the existing global time cursor where meaningful.
- UI labels come from metric definitions rather than being duplicated in
  render functions.

## 5. Canonical reporting architecture

### Ownership

Add a pure reporting layer under `dessim::metrics::reporting`.

- Observer/event code owns facts and physical intervals.
- `dessim::metrics::reporting` owns definitions, bucketing, scope filtering,
  reconciliation, and typed report tables.
- Desvis renders typed report tables.
- XLSX writes the same typed report tables and charts their cell ranges.

Do not make the dynamic string-keyed `SliceRecord.metrics` map the primary
source for new multi-dimensional metrics. Preserve it for generic charts until
the #300/#281 stream migration replaces it with descriptor-carrying metric
streams.

### Core types

Introduce:

- `MetricScope`: `FullRun` or `PostEmbargo`.
- `ReportPeriod`: `Overall`, `Fixed { width_ms }`, or
  `Trailing { window_ms, step_ms }`.
- `TimeBucket`: start, end, coverage, and partial flag.
- `SupportStatus`: measured, unknown, or unsupported.
- `Ratio`: raw numerator, denominator, and optional percentage.
- `MetricDefinition`: stable ID, label, description, unit, denominator
  description, aggregation class, and profile-schema version.
- Typed fact/aggregate rows for each metric family.
- `MetricsReport`: definitions plus the requested scoped/bucketed result
  tables.

Use integers for event counts, durations, and distance until final division.
Use half-open source buckets `[start, end)`. Trailing windows use
`(t - window, t]` as specified in #260.

### Aggregation invariants

- Ratios are computed from summed numerator/denominator pairs.
- Do not average percentages.
- Split intervals exactly at scope and bucket boundaries.
- Leading trailing windows are suppressed.
- Zero denominators yield N/A, never a fabricated 0%.
- Unknown values stay visible and participate in reconciliation.
- Unsupported means the simulator did not model the behavior.
- Old profiles missing a field yield unknown/unsupported, not guessed values.

### Cache boundary

Desvis caches `MetricsReport` by:

- profile/run identity;
- scope;
- period/window/step;
- section-specific filters.

UI state does not mutate profile data. XLSX builds reports through the same API
without an egui dependency.

## 6. Instrumentation and data-model changes

### 6.1 DA50 alignment and transfer phases — #262

Refactor the motion breakdown so every serviced endpoint has:

- alignment duration;
- pickup/drop transfer duration.

Forward sequence:

1. empty travel to storage;
2. storage alignment;
3. storage pickup;
4. laden travel to relay;
5. relay alignment;
6. relay drop.

Return sequence:

1. empty travel to relay;
2. relay alignment;
3. relay pickup;
4. laden travel to storage;
5. storage alignment;
6. storage drop.

Implementation:

- Replace ambiguous composite calculations in
  `sim/dessim/src/robots/vtm/motion.rs` with explicit endpoint-service
  components.
- Apply the same semantics to `task_time_full` and the detailed geometric
  cycle functions.
- Add an Alignment DA50 nav/event phase; keep Fork for transfer only.
- Ensure scheduler duration, availability frontier, tote transitions, relay
  finalization, emitted events, journey legs, and cycle totals share the same
  boundaries.
- Add fields with `#[serde(default)]`. (Superseded 2026-07-27: no alias or
  compatibility retention — nobody keeps profile files between builds; see §9.)

### 6.2 DA50 operation and payload facts — #249/#257

Add a stable payload classification:

- Empty.
- NonEmpty.
- Unknown.

Capture at each physical DA50 pickup/drop boundary. Emit one operation fact:

- timestamp;
- DA50 ID;
- operation kind;
- endpoint kind and ID;
- aisle;
- tote ID;
- payload state;
- cycle ID/presentation ID.

Do not reconstruct payload state from later inventory.

### 6.3 DA50 state and waiting — #256

Build state intervals from authoritative facts:

- Alignment event → Alignment.
- Fork Get → Pickup.
- Fork Put → Drop.
- Rail move with no tote → Empty travel.
- Rail move with tote → Laden travel.
- Selected-plan gap → Waiting with reason.

Parking/unblock remain unsupported.

Waiting reasons:

- AwaitingDemand.
- AwaitingTote.
- AwaitingSlot.
- AwaitingHtmDeposit, labeled “Awaiting D30 deposit”.
- NoTask/unattributed.
- Unknown.

The reason is selected-plan post-hoc attribution, never global congestion.
Derive unassigned leading/trailing gaps explicitly so Waiting reconciles to the
availability window.

### 6.4 Storage composition and relay occupancy — #258

Maintain per-aisle slot-state counts:

- unoccupied;
- empty tote;
- non-empty tote;
- unknown.

Emit end-of-slice snapshots plus enough transitions for exact interval
aggregation where required. Preserve the historical laden-fill columns.

Track physical relay occupancy from tote-at-relay transitions independently of
reservation state. Record:

- relay/zone;
- start/end;
- tote ID/payload when known;
- physical capacity.

### 6.5 Completed-work distribution fact — #259

Extend the pick completion fact with:

- order ID;
- tote ID;
- source aisle;
- source relay;
- presentation ID.

`Presentation` already has the order ID, tote ID, presentation ID, and source
zone in its committed chain. Thread the source relay through
`HtmCarryingTote` rather than re-looking it up after completion.

Record one tote presentation for each qualifying physical pick presentation.
Keep picked SKU quantity as a separate measure. Join `OrderCompleteEvent` to
the order's DC/station so one distinct order count is attributed at completion.

## 7. Trailing aggregation — #260

Implement a causal resampler independent from the existing centered smoothing:

- `window_ms` and `step_ms` are separate.
- First output is at the first aligned point with a complete window.
- Counts/durations/distances sum.
- Rates divide the trailing total by covered time.
- Ratios sum numerators and denominators, then divide.
- Snapshot gauges cannot select trailing sum unless explicitly supported.
- Cumulative series cannot select trailing sum.

Keep current centered mean/sum/min/max/median under the label “Smoothing”.
Add the new controls under “Trailing”.

Test boundary inclusion, exact hour/window edges, window ≠ step, and old
coarse-slice profiles.

## 8. XLSX reporting — #255

### Workbook structure

Add (names as landed):

- `Metric Descriptors` — a dedup of the descriptors attached to reports
  actually written into the workbook; never a maintained catalog.
- `Throughput` (renamed from `Trailing Aggregates`, 2026-07-29; per-row
  plumbing columns dropped — the once-per-sheet window/step/alignment
  metadata block remains the §7 disclosure).
- `DA50 State` (with per-DA50 drilldown tables).
- `DA50 Distance` (fleet + per-DA50).
- `DA50 Operations`.
- `DA50 Throughput`.
- `D30 State` (with per-D30 drilldown tables; Pickup/Drop/Alignment rows stay
  Unsupported until #275).
- `D30 Distance` (fleet + per-D30; empty/laden/parking split).
- `Aisle Occupancy` (renamed from "Aisle Utilization" — the values are
  occupancy ratios).
- `Completed Tote Work`.
- `Aisle x Station`.
- `DC x Station`.

Existing raw fact-log sheets remain.

Every percentage column is adjacent to its numerator and denominator. Include
scope, units, unknown count, and support status. Bucket start/end and
coverage appear per row only where they vary per row; values constant for
the whole sheet (a fixed trailing window's bounds and coverage, for
example) belong in the once-per-sheet metadata block instead (ruling
2026-07-29, the Throughput sheet trim).

### Native charts

Use `rust_xlsxwriter` native charts:

- 100% stacked column for hourly DA50 state.
- pie/donut for overall DA50 state.
- line charts for throughput and aisle utilization.
- grouped/stacked summaries for DA50 operations.
- conditional formatting for matrices.
- summary charts for selected/default matrix totals.

Charts reference aggregate sheet cells. They never reimplement a metric.

### Verification

- Save a deterministic test workbook.
- Inspect workbook ZIP/XML or supported chart metadata to assert sheet names,
  chart types, series ranges, and category ranges.
- Verify all chart source ranges remain valid for empty, one-row, and multi-row
  fixtures.

## 9. Compatibility and migration

Superseded 2026-07-27 by the descriptor-contract rulings (issues #280, #281,
#300). What holds now:

- New profile fields use `#[serde(default)]`; no field aliases are retained —
  profile files are not kept between builds, so there is nothing to migrate.
- The profile schema version stays 1. There are no profile-version gates;
  support derives from the facts a run actually captured. `SupportStatus`
  (Measured / Unknown / Unsupported) is producer-emitted, never inferred by
  consumers.
- Named profile fields for new fact families are explicitly temporary
  residence pending the #300 framed streaming container; every named log
  migrates together in the tenant-migration pass.
- Metric metadata travels as producer-owned descriptors on emitted reports.
  Consumers discover; any registry is derived. Definition-version numbers and
  compatibility matrices are repealed concepts.

## 10. Performance constraints

- Aggregate facts once per report cache key, not every frame.
- Use sorted vectors/BTreeMaps for deterministic table order.
- Avoid cloning full journals during hover.
- Pre-index by time, aisle, station, DC, DA50, and operation where it materially
  reduces repeated filtering.
- Benchmark Cencora-scale profiles before/after. Target:
  - no simulation throughput regression from observer instrumentation above 5%;
  - metric report generation under one second for the reference profile where
    practical;
  - no frame-time work proportional to all raw facts.

## 11. Test strategy

### Unit fixtures

Create a small deterministic fixture with:

- two DA50s;
- two aisles and two relays;
- two pick stations;
- two DCs;
- two orders;
- empty and non-empty totes;
- every DA50 endpoint operation;
- selected-plan waits;
- events crossing an hour and embargo boundary.

Assertions:

- states reconcile to fleet time;
- wait reasons reconcile to Waiting;
- distance split reconciles;
- operation payload split reconciles;
- storage composition reconciles to capacity;
- physical relay utilization never exceeds capacity;
- tote/order matrices reconcile to facts;
- percentages reconcile on the intended axis;
- full-run and post-embargo totals differ only by excluded intervals/events.

### UI/view-model tests

- Section/filter defaults.
- Scope and period invalidation.
- Row-hover/pin selection model.
- Matrix denominator direction.
- Unsupported/unknown rendering.
- Trailing selector validation.

Test view models rather than pixels; perform one manual Desvis smoke pass on a
reference profile for layout and interaction.

### Integration and regression

- `cargo test -p dessim`.
- `cargo test -p desvis`.
- Relevant workspace formatting/lint commands.
- Headless simulation with XLSX output.
- Load an older profile.
- Run a Cencora smoke scenario and reconcile UI/XLSX aggregates.

## 12. Ordered implementation and eventual stacked PRs

All development occurs on `codex/dematic-kpis`. Commits remain in stack order.
At the end, create one branch at each contiguous slice boundary and open stacked
PRs whose bases point to the preceding slice.

### Stack 1 — Reporting core and trailing aggregation

Issues: #254, #255 foundation, #260.

- Spec and metric definitions.
- Canonical bucket/scope/ratio/support types.
- Trailing resampler and tests.
- Metrics report cache contract.

### Stack 2 — XLSX native chart foundation

Issue: #255.

- Data dictionary and reusable table helpers.
- Native chart helpers.
- Workbook structure/source-range tests.

### Stack 3 — DA50 motion and facts

Issues: #262, #249, #257.

- Alignment at all fork interactions.
- Explicit alignment/pickup/drop phases.
- Payload-at-boundary classification.
- DA50 operation facts and distance split.

### Stack 4 — DA50 metric surfaces

Issue: #256/#257.

- DA50 state/wait/distance aggregates.
- DA50 throughput aggregates.
- Desvis Metrics surface and DA50 sections.
- DA50 XLSX tables/charts.

### Stack 5 — Buffer and storage

Issue: #258.

- Slot composition facts.
- Physical relay occupancy.
- Desvis and XLSX surfaces.

### Stack 6 — Distribution analytics

Issue: #259.

- Completed tote/order fact.
- Aisle/DC × station matrices.
- Row-hover composition interaction.
- Desvis and XLSX surfaces.

### Stack 7 — Reconciliation and migration hardening

Umbrella: #254.

- Cross-surface golden fixture.
- Old-profile behavior.
- Cencora performance/smoke validation.
- Documentation and issue acceptance audit.

Do not include D30 implementation in these stacks; #261 is discovery first.

### Commit discipline

- One behavior or mechanical migration per commit.
- Each commit compiles and its affected tests pass.
- No later-stack files in an earlier-stack commit unless required by the public
  interface.
- Do not squash or reorder after downstream slices depend on a commit.
- Record the final commit SHA for each stack boundary before creating PR
  branches.

## 13. Definition of done

- All #254 acceptance criteria are mapped to tests or an explicit deferred
  decision.
- Desvis, XLSX tables, and XLSX charts agree exactly for the golden fixture.
- DA50 full-run and post-embargo totals are available.
- Unsupported future states are honest.
- Waiting labels retain the selected-plan caveat.
- All new user-facing labels use Tote/Order/Pick station and DA50/D30.
- Leading trailing windows are suppressed.
- Existing profiles load safely.
- Stack branches/PRs are created from the pre-recorded contiguous commit
  boundaries.

## 14. Status record (2026-07-27)

The plan landed as small sequential PRs on the corrected architecture, not as
the original cumulative stacks of §12–13 (kept above as history):

- #293 native XLSX chart foundation (the central metric catalog was deleted
  before landing; producers own their descriptors).
- #298 `MetricDescriptor` contract; dead version fields dropped.
- #299 unified stream contract (`StreamDescriptor` core, `LogStream` /
  `MetricStream` families, `#[derive(StreamRecord)]`, streaming-capable row
  transport).
- #294 DA50 facts (operation journal, alignment activity, re-decomposed cycle
  timing — a deliberate, quantified physical change: +3.9% picks on the 4h
  cencora baseline).
- #295 DA50 state/throughput report builders with producer-owned descriptors;
  all profile-version gates deleted; canonical taxonomy constants exported
  with exhaustiveness guards.
- #296 aisle occupancy (renamed from utilization): per-aisle state snapshots,
  five unified Ratio descriptors, the return-side `ToteMoved(AtRelay)`
  emission (observational; proven non-perturbing by bit-identical parity).
- #297 completed-work distribution: aisle × station and DC × station
  matrices with reconciliation-by-dimension and explicit per-dimension
  unattributed counters.
- Integration slice: Desvis metrics workspace + drilldowns, the workbook
  Metric Descriptors sheet (dedup of what was written), Trailing Aggregates
  (renamed `Throughput` 2026-07-29) and DA50 Distance sheets, per-DA50
  tables, pre-run metric enumeration via
  the producers' descriptor functions.
- D30 state family (2026-07-29, after the #261 taxonomy approval): the
  `d30_state` report builder mirroring the DA50 family (state / waiting
  attribution / travel distance with per-D30 drilldown, built from
  `htm_leg_log`), the Desvis `D30 state` view, and the `D30 State` /
  `D30 Distance` workbook sheets. Recorded constraints: Station processing
  is measured from same-station journal gaps — pre-presentation residue,
  the presentation dwell, any post-presentation hold, and the return leg's
  rotation before its first move; station queue slack is NOT in it (the live model
  dispatch-serializes presentations, so that slack is explicit NoTask
  idle; #273's QueueCell facts will make queue holds measurable).
  Pickup/Drop/Alignment stay Unsupported pending #275's endpoint-service
  facts. Wait reasons without a per-interval journal fact render by their
  honest status: route hold, post-presentation return hold, and return
  slot are Unknown (modeled and measured in aggregate — duty overlays and
  per-reservation slot waits — but unattributable from journal facts);
  station queue hold is Unsupported (not modeled). Move-only leg durations
  put in-path rotation and dispatch seams in Waiting → Unknown (#272's
  recorded gap). The fleet denominator is the configured eligible count;
  facts naming more D30s fail hard, as do overlapping same-robot travel
  facts; a third check (distance split vs classified total) is unreachable
  while the kind partition stays exhaustive and exists to catch a future
  taxonomy edit.

Deliberate divergences from this plan's original text, all recorded in PR
bodies: no `Da50TravelLoad::Unknown` and no unknown-distance metric (no
producer can emit one — metadata lands with its first real producer); no
peak/final instantaneous snapshot values (an aggregation the descriptor
vocabulary cannot yet describe honestly); matrices retain unattributable
facts as explicit counters rather than visible Unknown rows.

Validation: metrics-only slices hold bit-identical pick parity (exactly 2,716
picks) on the same-seed 4h `cencora_phase2` baseline; per-slice agent reviews
(/rev + placement) plus headed desvis + xlsx runs.
