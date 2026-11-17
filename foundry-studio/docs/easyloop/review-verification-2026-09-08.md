range: 58b16d1636f9cf4f8fa868cbe07ecdff91100d01..2e9b86064

# Walmart review: verified findings and next work

The independent review found real scheduling and reporting problems, but several
of its numerical claims and causal conclusions do not survive verification.
Extraction destination exclusion and serialized order/inventory work are the
best-supported next throughput investigations. Neither supports increasing the
EasyLoop conveyor capacity or fleet on the present evidence.

## Review scope and verification

Daniel requested Claude Fable 5.1 with a one-million-token context. The exact CLI
model `claude-fable-5-1[1m]` was verified to expose a 1,000,000-token context and
ran the review without a fallback model. The shorthand `fable-5.1[1m]` failed
before that review; its error output is not a review.

The [original review](fable-review-2026-09-08.md) covers the pre-rendering
checkpoint above. Its stated line-by-line scope excludes the already-merged
task-model-v2 commits and starts at `3ec77ec67`. The raw output remains at
`~/.claude/reviews/greymatter-foundry/walmart/2026-09-08-1623-main.md`; the repository
copy adds only the missing `range: ` prefix to its first line. Read the original
with this disposition, rather than treating its conclusions as accepted facts.

Verification used current code at `a833b63c3`, the retained run logs and physical
JSON traces, the actual workbook, source CSVs, and commit preimages. Independent
Codex passes recalculated the workbook and workload claims; the parent checked
the scheduling call paths, reservation allocator, counts, hourly completion data,
floor boundary spot checks and every finding's disposition. This is verification
of that review, not a claim of a second complete line-by-line branch review.

Evidence is retained under
`../notes/walmart-floor-alignment-2026-09-08/` relative to the worktree. Crucially,
`headed-500.xlsx` identifies **123513982** on its Cover sheet and covers
**5,340.411 seconds**. It is not the later `2e9b86064` acceptance run. The final
4,000-order evidence is `source-4000-final.log` and `source-4000-final/*.json`.

## Disposition of every numbered finding

### 1. Extraction reservations — mechanism confirmed; measurements corrected

[`Navigate::available_destination`](../../sim/dessim/src/flows/transport/navigate.rs)
excludes any destination with another robot's open-ended hold, even when its
start is in the future. If every port is excluded, the assigned robot waits at
its present cell. `DispatchOrder` reaches this step from the EasyLoop waiting
berth for direct orders and from the buffer retrieval endpoint for buffered
orders. This is a credible major source of outbound blocking.

The [path allocator](../../sim/dessim/src/pathfinding/fh_sipp/allocator.rs)
starts a destination-cell reservation at the **final inbound move's start** and
closes its predecessor at the **first outbound move's end**. The review instead
describes arrival and departure-start boundaries. Scheduler commitment happens
before the emitted journey record and is not directly timestamped in the
workbook. Journey coverage is therefore a proxy for exclusion, not a measured
commit-to-release utilization.

Recalculation from the retained workbook:

| Port | Extractions | Mean complete inbound route | Dwell duty | Inbound-route-start → outbound-start coverage |
| --- | ---: | ---: | ---: | ---: |
| 17 | 161 | 22.449 s | 27.133% | 94.811% |
| 18 | 174 | 19.935 s | 29.324% | 94.276% |
| 19 | 165 | 21.278 s | 27.807% | 93.548% |

All-three-port proxy coverage is **92.36%**. The review's 91–92% individual
values and 85.65% simultaneous coverage use only the final inbound Travel
segment, omitting earlier path segments. Its 36% dwell duty does not reproduce.
Its 171-second mean direct berth delay also does not reproduce: reconstructing
184 direct trips from return-travel endpoints gives 196.775 seconds, with
imperfect physical milestone attribution.

The later 4,000-order physical trace does independently confirm direct
last-pick-end → extraction-start mean **277.077 s**, nearest-rank p95
**797.289 s**, and buffer HTM pickup → extraction-start mean **264.472 s**, p95
**810.086 s**. These delays include travel and waiting; they are not service
times. No intervention yet establishes extraction exclusion as the unique
binding resource.

**Disposition:** instrument actual exclusion, arrival, service and clearance
boundaries before changing dispatch. The [site questions](questions.html#extraction-approach)
now ask for real approach/queue/egress cells and shared equipment. Do not end a
reservation after nine seconds while the HTM still physically occupies the port.

### 2. Serialized inventory work — confirmed, with an important timing correction

[`FulfillOrder::schedule`](../../sim/dessim/src/flows/fulfillment/orders.rs)
finishes acquiring and planning the order container to its waiting berth before
creating the inventory retrieval. It supplies `order.action_ready_at` to that
retrieval, then processes inventory visits one at a time. `PresentTote` waits for
inventory readiness at the scheduler barrier before the order HTM's approach is
created. Both dependencies prevent useful overlap.

However, `after` is **not always a floor on VTM cycle start**, as the review
claims. [`VtmService`](../../sim/dessim/src/model/relay/vtm/mod.rs) in normal
mode may start at `relay_start - cycle_duration`, bounded by robot availability
and the admission floor. Thus the entire VTM cycle need not occur after berth
arrival. Nor do the 495 laden station waits prove every second was exclusively
waiting for inventory; the reporting attribution is insufficient.

**Disposition:** start with a controlled first-inventory-retrieval overlap
experiment alongside order-container acquisition, then consider next-line
prefetch. Keep physical readiness, tote ownership, berth admission, conveyor
capacity and paired-transfer checks. Measure before claiming a gain; do not
simply remove the presentation barrier that protects earlier-ready work.

### 3. A deeper parking obstruction — credible risk, not an observed new failure

[`htm_departure_is_blocked`](../../sim/dessim/src/model/navigation.rs) explicitly
checks only the first legal step. A free neighbor can lead into an occupied
dead end. `InductContainer` then passes its accepted HTM to `Navigate`, whose
`reserve_path` error propagates. Unlike `HtmPickup`, it has no route-holder
relief on `UnreachableWithinHorizon`.

The code permits this failure mechanism, but the review's post-fix parking
occupancy example was not reproduced by a test or observed in the completed
4,000-order run. Its heading, “still aborts a run,” overstates the evidence.

**Disposition:** add a realistic multi-step obstruction regression and then
bounded parking relief/reselection. Keep static infeasibility distinct from a
temporary obstruction; blindly waiting on every pathfinding error would mask
invalid maps and violate the standing error rule.

### 4. Reporting attribution — confirmed; task ownership must remain explicit

`Navigate` emits `Idle` on arrival while its parent retains the robot.
[`observer/handlers.rs`](../../sim/dessim/src/observer/handlers.rs) and
[`nav_handler.rs`](../../sim/dessim/src/observer/nav_handler.rs) convert the
interval into `NoTask`. The workbook contains **495 laden “No task assigned”
rows attributed to Pick Station**, totaling **64,714.165 robot-seconds**,
mean 130.736 s and median 82.441 s. The labels are demonstrably misleading;
the intervals do not by themselves identify their actual waiting causes.

Journey segments inherit trip-level semantic endpoints rather than identifying
each actual berth/induction/presentation cell. The workbook's source-extract
type reads `Discriminant(4)`, reproduced in `Cover!B53` and the generic fallback
in [`xlsx_export/mod.rs`](../../desvis/src/xlsx_export/mod.rs). That fallback
predates this branch. Two presentation identities are legitimate detail, but
operator capacity must aggregate each pair into its one shared operator.

The prior broker handoff records work to remove `StateTransitionEvent`-based
state derivation. Extending this branch's label vocabulary is consequently the
wrong long-term remedy; reconcile the event/interval work at integration. It is
not necessary to reopen an already recorded direction as a new site question.

**Disposition:** derive assigned waiting, travel, handling and service from
task/physical intervals and add actual per-order milestones to UI and XLSX.
Preserve physical identifiers and source provenance. These remain work to do.

### 5. Order mix and historical tables — confirmed, corrected and clarified

Source IDs are grouped lexicographically and ready-time ties use the resulting
dense IDs. That shapes the ready-backlog experiment; the full-source default
also respects actual arrivals. Sorting IDs numerically would change policy,
not remove the need to report workload mix.

In the 4,000-order cohort there are **289 multi-unit orders** at dense IDs
2140–2815 and **228 multi-line orders** at 2232–2815. Some numeric source IDs
are below 20000. The review confuses these populations. In the 6–7-hour window,
**601 physical transfers** coincide with **231 orders finishing picking**
(216 cartons, 14 totes, one tray) and **241 extractions**. This is not a sudden
601-order/hour improvement. The proposed two-parts-order-overhead/one-part-pick
model is unmeasured and confounds packaging, routing and unit count.

The completed aligned-run figures had already been added by `e9e794e2b`;
calling that rerun unfinished is stale. Older tables remain useful historical
controls. [Throughput](throughput.md) now identifies them explicitly and adds
current hourly picks, orders finishing picking and extractions.

### 6. Site-capacity arithmetic — several claims rejected

Three independent nine-second extraction servers give **1,200 orders/hour**
before clearance, travel and other constraints, not 900. The site figures are
**1,280 units/hour** and **1,160 presentations/hour**, not automatically order
rates. At the full file's 1.15636 units/order, 1,280 units/hour would require
about **1,106.92 extractions/hour**, or **92.24%** of that dwell-only ceiling.
Shared packing equipment, nonzero clearance or a different mix changes it.

The workbook confirms one buffer VTM's 48.157% duty and 12.131-second mean
cycle, but its zone receives **33.54%**, not 37%, of buffer cycles. Only four
robots perform buffer-zone cycles there; treating all eight buffer/depot VTMs
as freely interchangeable requires a topology and assignment argument.

For 24 admitted-order tokens at observed output 340.446 orders/hour,
`24 / X = 253.785 seconds` is a bound assuming WIP ≤24, not a measured mean
berth residence. The review's 251-second residence is not established. Four
nine-second operators require 72.5% duty at 1,160 transfers/hour and 80% at
1,280 transfers/hour; those conditional calculations are sound.

The **22,160 orders, 25,625 units and 47h16m42s arrival span** reproduce. The
review's roughly 20,000-order backlog is unsupported: even a simplistic
constant-output fluid calculation at the observed finite-batch order rate
gives about 10,945 peak backlog, not 20,000. Its approximately 70-hour finish
is an extrapolation, not a simulated prediction. Stock shortages and changing
mix make applying a constant rate especially uncertain.

**Disposition:** add correctly dimensioned service-demand checks to the spec
assessment. Do not use these calculations as a substitute for a representative
arrival-driven run, floor-VTM interference, or the actual extraction model.

### 7. Tests — meaningful custody checks; throughput protection remains weak

[`common/queueing.rs`](../../sim/dessim/tests/common/queueing.rs) compares
independent calendar and physical visit histories, including pointwise occupancy,
operator exclusion and window carry-in/out. Those checks are not tautological.
The displayed full-run WIP and Little's-law form subsequently express the same
sample-path identity; their agreement is not a capacity prediction.

The source gates enforce completion, custody and conservation, but ordinary
duration regressions can pass under their 48-hour ceiling. “No full hour without
a pick” is a weak liveness check. The 3,600-second wall budget is now committed
and leaves room for the already measured 1,852-second run; it changes no physical
timing.

**Disposition:** retain the physical checks and add an explicit duration
regression budget for the fixed 500-order control, with deliberate review when
geometry, fleet, workload or service assumptions change. No timing gate has
been added or passed merely by writing this recommendation.

### 8. Factorization and terminology — distinguish compatibility from defects

The large `FulfillOrder::schedule` and transit lookup through a synthetic
`Station`, including its zero fallback, are confirmed. Project equipment timing
and capacity directly and separate acquisition, inventory-visit execution and
dispatch. Eight is the currently approved conveyor capacity; its code constant
is a factorization issue, not incorrect equipment behavior.

The legacy roots remain registered and exercised by compatibility callers.
The named Walmart composition uses `FulfillOrders` instead. Selecting it is
still a valid remedy for the composition error, but the wording implied it
supplied the old roots. The diagnostic is clarified to distinguish replacing
the workflow from explicitly retaining `PickDemand` and its legacy children.
No legacy roots were deleted without auditing their callers.

The private worldlens `Machine` is an actual predicate-based adapter, not an
aggregate state manager like the deleted `ModelState`. More specific equipment
projection and naming remain useful; its existence is not evidence that
worldlens was bypassed. The existing projection audit remains the work plan.

### 9. Map provenance — missing patcher step, recoverable exact history

Commit `a32b3cce0` changed exactly the HTM cohort count **122 → 144**, entity
`56ec28ec-6616-5ffd-bcee-78722d8345fd`. Replacing that one value in the commit
preimage reproduces the full postimage byte-for-byte. Its SHA-256 is
`344b4f14f3a04685593aa476d7d873626511eacbd91715d76da8f9e07e426092`, the input
pinned by the subsequent floor patcher. Provenance is not lost, although this
hop should have been part of the audited patcher chain.

**Disposition:** retain this explicit provenance record and use the named,
byte-audited patcher for future edits. The generator header's hand-managed
wording remains a separate recorded question; changing document identity is
unnecessary. No wholesale map regeneration is justified.

### 10. Master integration — confirmed gap, not an already broken merge

The inspected `origin/master` is `87ba106fd`; its Cencora fixture uses
`vtm.isolation = off`, whereas this branch retains `teleport`. The VTM service
explicitly ignores per-robot capacity in teleport mode. Shared relay,
navigation, loader and reporting changes warrant Cencora validation.

**Disposition:** integrate current master and establish comparative Cencora
evidence before landing. The draft PR already exists, so “fix before PR” is not
the right milestone. Do not infer that every Cencora run is broken, or merge
away measured checkpoints while attributing their effects to a throughput fix.
No merge/rebase was performed as part of this review.

### 11. Cencora parity — validation gap confirmed

`minimal_putaway1_no_violations` stalls at barrier **1,160.183 s**, after three
picks and 43 putaways, and the same failure existed before the latest departure
change. That isolates this change; it does not prove parity with master.
The broader desvis test also stalls in
`no_bot_cell_collisions_minimal_putaway_orders_0` after 60 putaways, before its
renderer collision assertions. The exact latter failure has not been baselined
against master. Neither failed check should be presented as a collision pass.

**Disposition:** compare the same Cencora cases on master and this branch,
including the real fixture with VTM isolation off, before claiming preservation.
No new Cencora runtime baseline was run during this review-verification pass.

## Next implementation sequence

1. Record explicit berth, inventory-ready, approach, port exclusion, service and
   clearance intervals in production metrics. Separate assigned waiting from
   unassigned availability, and expose the same quantities in UI and XLSX.
2. Overlap first inventory retrieval with order-container preparation under
   existing admission/custody rules; compare the fixed 500-order control and then
   the 4,000-order backlog with collisions enabled.
3. Resolve and implement physically declared extraction approach/egress space,
   then test whether reducing exclusion improves completed orders rather than
   merely moving waiting elsewhere. Add the multi-step parking regression.
4. Finish the [remaining floor boundaries](floor-boundaries-2026-09-08.md),
   integrate master/Cencora validation, and implement actual floor-VTM occupancy
   before interpreting fleet sensitivity as a site result.

The requested [container/conveyor visual changes](rendering-2026-09-08.md) and
full-source default selection are committed separately from the reviewed
checkpoint. Headed iteration 02 completes 500 orders with 576 picks in
5,350.287 simulated seconds, matching the preceding physical run. All 22,160
source orders load in the real-extract test. The full default still has a
43-unit deficit across 40 SKUs; Daniel's temporary stock-policy answer remains
pending, and no stock, arrivals or orders were silently fabricated or removed.
