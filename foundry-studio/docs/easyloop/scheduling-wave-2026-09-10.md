# Presentation approaches and known departure decisions

This wave changes known-work scheduling on the Walmart branch after
`0b99e53f1`. Geometry, fleet, stock, arrivals, dwell times and collision avoidance
remain fixed. The existing main scenario still admits all 22,160 source orders;
500 and 4,000 order cohorts are internal controls only.

The final 4,000-order physical audit completes all 4,594 units in **24,714.932
simulated seconds (6h51m55s)**, versus 31,734.127 before this wave. Whole-batch
unit throughput rises **28.40%, from 521.16 to 669.17 units/hour**. This includes
outbound cleanup; it is a finite-backlog comparison, not a site capacity claim.

A subsequent [inventory-supply correction](inventory-supply-2026-09-10.md) removes
the delivery/unloading dependency on order preparation and adds per-type eviction
counts. Its separate 500-order comparison does not replace this longer result.

## Changed behavior

- `PrepareOrderPresentation` coordinates known inventory readiness, estimated
  order HTM travel and the physical availability of either presentation cell.
  The order approach can start before inventory arrives. Earlier-ready known
  inventory visits retain priority; accepting a visit claims its inventory
  side. `Navigate` reserves the actual route, and `TransferItems` still waits
  for both actual arrivals and the shared operator.
- Carton extraction registers its empty HTM and creates `Parking`, whose cheap
  proposal starts one millisecond after the last dwell. A feasible published
  productive claim for the same idle period wins, including a later-start
  claim. Only accepted parking selects a bay and searches a route. Superseded
  parking ends before scheduling; failure to find parking restores the exact
  availability and physical hold and reports the existing explicit failure.
- An incoming extraction journey whose eligible ports are all blocked can ask
  one already-published Parking family to decide earlier. Productive contenders
  still take priority. The decision can precede the dwell's completion, while
  physical departure remains after it. Only actual route reservations expose
  the port's release; the request itself frees no cell or HTM.
- If an early accepted fallback finds all bays unavailable, it restores the
  original availability and waits until its physical departure time. Productive
  work may take the HTM during that wait. Otherwise one fresh Parking child
  re-enters ordinary competition; a second failure is terminal. This bounded
  retry avoids declaring future egress impossible from an earlier snapshot.
- The second chunk extends the incoming journey's request to the reusable
  container's existing `HtmStore` task. Its relay is already reserved and its
  HTM remains assigned. A Navigation-owned departure advertisement identifies
  the exact source hold and proposal attempt. Only live demand advances the
  decision; acceptance withdraws the advertisement and normal route commitment
  bounds the source hold. An occupied relay retains that hold while other
  tasks and the simulation clock can proceed.
- For an opted-in extraction approach whose known clearance lies beyond the
  route planner's 900-second limit, departure is shifted toward that clearance
  using the cached travel time. The limit stays unchanged. The clearance comes
  from the oracle's canonical free interval, including its safety gap; actual
  routing still reserves the full path and respects the previous extraction.
  Ordinary nine-second service does not enter this exceptional branch.
- The HTM state manager identifies competing claims by one availability
  incarnation. The scheduler checks each candidate's whole proposal, rather
  than deciding conflicts from task names or assuming generic interval overlap.
  Equal-time waiting producers wake before the due competition is decided.
  Productive-only contests keep their existing start/task ordering.
- Immutable HTM lookup caches are shared when cloning the service handle, so
  publishing a claim no longer copies the floor-position map and parking-cell
  set. This changes allocation cost, not scheduling decisions.

Parking currently replaces the forced departure at carton extraction. Other
flows retain existing pressure relief. This is not a claim that every idle-HTM
release has been migrated. No task recursively plans its unknown successor.

## Validation

The physical approach and productive-departure regressions both failed before
their respective integrations. The combined fulfillment suite passes all 20
tests, including occupied target cells, both actual arrival times, complete
pick dwell, mixed containers, repeated and split SKU demand, inventory custody,
and no final-pick return to Waiting. The source-selection tests exercise unequal
travel times and an earlier-ready visit whose nominal departure is later.

Focused checks also cover later-start productive competition, whole-proposal
admissibility, same-time publication, stale proposal attempts and HTM idle
periods, fallback execution and no-bay restoration. A panic-on-search test
proves superseded parking does not perform route planning.

The occupied-port regression checks actual movement: an incoming HTM starts
before the incumbent's 100-second dwell ends, and its destination hold starts
after the incumbent departs at 100.001 seconds. Other checks cover free
alternatives, choosing only one blocked holder, withdrawn requests and stale
availability periods. In this first chunk, earlier decision deadlines apply
only to existing Parking claims, preventing an inadmissible productive claim
from pinning the clock before the work needed to unblock it can run.
The full-bay integration regression remains unchanged: failed extraction egress
must retain the empty HTM and its physical hold, and cannot halt before the
preceding dwell ends. Focused cases also exercise a bay becoming free during
the wait and productive work winning the fresh contest.

Release controls completed all 500 orders and 576 picked units, with identical
inputs and full 1,397,632-unit stock:

| Variant | Batch completion, sim seconds | Units/hour over complete batch | Poison evictions |
| --- | ---: | ---: | ---: |
| Previous committed workflow (`0b99e53f1`) | 4,006.720 | 517.53 | 15,324 |
| Framework and source-selection helpers; old approach/parking call sites | 3,988.178 | 519.94 | 17,879 |
| Coordinated approach; forced extraction parking retained | 4,002.407 | 518.09 | 33,034 |
| Same approach, narrowed target-availability invalidation | 4,002.407 | 518.09 | 16,861 |
| First deferred Parking integration (regressed; not final) | 5,151.847 | 402.50 | 14,274 |
| First corrected departure request and bounded parking retry | 3,974.531 | 521.72 | 18,170 |

The control already contains the revised scheduler and source-selection
machinery. The preceding committed workflow completed this cohort in 4,006.720
seconds, so neither helper adoption nor equal-time ordering is hidden in the
approach comparison. The approach-only change is 0.36% lower in whole-batch unit
rate than its control. It fixes the dependency; this sample does not support a
throughput improvement claim. Physical SKU transfers change from 564 to 563
because source selection can change whether a quantity needs a split visit;
the unit demand and service-time rule are unchanged.

Review then removed global-traffic invalidation of presentation proposals.
Unrelated movements now leave their proposals valid; changes to either eligible
target cell still trigger reselection, with static travel estimates cached once.
The combined measurement includes this retry reduction as well as deferred
parking; its effects must not all be attributed to Parking alone.

The narrow-invalidation control has identical simulated duration and event
counts to the approach-only control, with 48.96% fewer proposal evictions.
The first deferred Parking integration passed the independent physical audit
but regressed to 5,151.847 seconds. Its 4,000-order audit was stopped while this
regression was corrected; it is not reported as a passing longer run.

Independent joins of the extraction journal to the next positive-distance HTM
journey show all 246 empty carton HTMs park in both the framework control and
the regressed integration. Their departure delay changes from zero to exactly
one millisecond; reusable-container departures remain at extraction completion.
No later productive assignment retained an extraction port in this cohort.
The regressed candidate delays planning the future port release, so incoming
navigation sees an open-ended hold longer even though actual departure remains
prompt. The correction must expose that release through an earlier decision on
existing work, preserving its physical departure time and resource ownership.

The corrected release and its independent audit both finish at **3,974.531
seconds**, with identical event counts. All 576 units, 564 physical SKU
transfers, 500 extractions, 246 cartons, 316 buffer round trips and 2,268 VTM
cycles pass the physical checks. Compared with the preceding committed workflow,
the complete batch finishes 32.189 seconds sooner: **0.81% higher unit rate**.
First-hour units are 554 versus 559 before; the shorter drain does not establish
a sustained capacity gain. Final proposal evictions are also higher than the
previous commit, despite the separate narrowing improvement demonstrated above.

The corrected 500-order observation window ends at the last conveyor collection,
3,827.530 seconds. Mean conveyor WIP is 3.42–3.52 totes per EasyLoop; productive
operator duty is 32.9–33.6%. The 184 direct orders retain their final presentation
for a mean 70.160 seconds after picking. Both sides hold completed orders for
28.17% of that window, averaged over the four machines. This remains substantial
downstream blocking. These figures describe this finite cohort and window,
not the earlier one-unit long run or calibrated station capacity.

The first chunk's 4,000-order comparison and custody audit also complete:

| Variant | Batch completion, sim seconds | Units/hour over complete batch | Poison evictions |
| --- | ---: | ---: | ---: |
| Previous committed workflow (`0b99e53f1`) | 31,734.127 | 521.16 | 115,534 |
| Demand-triggered Parking and coordinated presentation (`d7675a1f4`) | 31,763.015 | 520.68 | 137,726 |

All 4,594 units and 4,000 orders finish, including 1,871 cartons, 2,568 buffer
round trips and 18,308 VTM cycles in the physical audit. Complete hourly windows
contain 462–647 picked units, with demand still remaining after eight hours.
There is no late-run collapse in this finite cohort. The batch unit rate is
0.09% lower than the previous commit, so this first chunk establishes corrected
dependencies and continued flow, not a long-run throughput gain.

The middle half of the audit's conveyor observation window spans 7,909.628 to
23,728.886 seconds. Productive operator duty is 31.3–34.9%; both presentation
cells hold completed orders for 26.7–34.1% of that same window. This is still
substantial downstream blocking and motivates the separate reusable-container
departure correction below.

Benchmark artifacts, source patches and binary/input hashes are under
`../notes/walmart-next-wave-2026-09-10/` relative to this worktree. First-chunk release
binary SHA-256:
`9b1b41a4a617a9ca4d8abf4d5b3e5b27e33c7df88ef280a2f1a802d68c0cd013`.

The longer control contains 4,000 stock-feasible source orders and 4,594 units:
1,846 totes, 1,871 cartons and 283 trays; 2,568 orders take the buffer route.
All arrivals are set to zero for this internal backlog test. The existing
deterministic selector replaces one whole primary order (`3849`) with another
source order (`10`) against original on-hand feasibility; it changes no SKU or
quantity within an accepted order. Both binaries use that identical generated
input, full stock and explicit overlay. The canonical source-arrival scenario
is unchanged.

## Reusable-container departure correction

The first chunk's 500-order history identified another scheduling boundary.
`DispatchOrder` immediately created the reusable container's storage task, but
`ProposedHtmStore` published extraction completion as its decision time. The
source hold stayed open-ended until that decision. Incoming `Navigate` could
request an earlier Parking decision, but not this already-owned storage leg.

Joining every extraction to its incoming journey, then comparing consecutive
users of each port, gives:

| Previous container | Port transitions | Incoming movement starts before previous extraction ends | Starts exactly at that end | Mean gap between service dwells |
| --- | ---: | ---: | ---: | ---: |
| Carton | 245 | 231 | 0 | 5.94 s |
| Tote | 222 | 0 | 196 | 21.36 s |
| Tray | 30 | 0 | 26 | 23.70 s |

The independent review and a separate parent join agree on all counts and means.
The match uses robot, actual extraction port and trip identity, and selects the
first positive-distance Travel segment. It is not a claim that a port is
physically free throughout its service gap; future reservations and path
clearance still matter.

The existing `HtmStore` route owner now receives a source-release decision
request after its relay deposit reservation is committed. Relay selection,
headroom ownership and physical departure time are preserved. The extraction
hold closes only through the actual route commit. A held reusable container
never enters the idle-HTM registry to obtain this behavior.

All 20 focused checks pass: source and attempt replacement, independent request
withdrawal, no-demand behavior, blocked-relay clock progress, mixed Parking and
storage alternatives, and the existing Parking suite. All 31 release composition
and fulfillment integration tests pass, including Cencora and the two-order
reusable-container case. Actual movement starts before the first extraction
ends; both containers retain their full dwell and real cell-reservation
exclusion. The same assertion holds for a 1,000-second extraction. Exact
900-second clearance and a 100 ms safety gap crossing the horizon are covered
separately without raising the route-search limit.

The reproducible pre-change join and input hashes are retained in
`../notes/walmart-next-wave-2026-09-10/extraction-approach-join.json`.

The frozen second-chunk 500-order headless run and independent physical audit
both complete at **3,322.889 seconds**, with identical event counts. All 576
units, 563 physical SKU transfers, 500 extractions, 246 cartons, 316 buffer round
trips and 2,266 VTM cycles pass the physical checks. This is **624.04 units/hour**
over the complete batch: **19.61% above the first chunk** and **20.58% above the
preceding committed workflow**. These are finite-batch measurements.

The same 184 direct-extraction orders now clear their final presentation a mean
**1.15 seconds after the last pick**, versus **70.16 seconds** in the first
chunk. Both presentation cells holding completed orders drops to zero in this
sample. This is physical source-cell clearance through the committed next
journey, not removal of the extraction dwell or deletion of a reservation.

Independent extraction-journey joins find actual incoming movement before the
previous extraction ends in 197 of 222 tote-preceded visits and 24 of 30
tray-preceded visits, versus none before. Mean inter-dwell gaps change from
21.36 to 9.35 seconds after totes and 23.70 to 10.38 seconds after trays. Carton
gaps increase from 5.94 to 10.67 seconds as the flow changes; these observational
container groups are not isolated service-capacity experiments.

An independent interval sweep uses the same interior clock window for both
500-order runs, **956.882–2,371.401 seconds**. Mean operator duty across the four
EasyLoops rises from **32.43% to 41.01%**; both sides holding completed orders
falls from **36.16% to zero**; mean physical conveyor WIP per EasyLoop falls
from **4.080 to 2.658**. Remaining operator idle is not attributed to a particular
upstream resource by these measurements.

Across each complete conveyor cohort, mean boarded-to-collected residence falls
from **94.221 to 58.214 seconds**. Pick-end-to-collection stays at **25.683
seconds**. The independently integrated physical occupancy equals completed
visit rate multiplied by mean residence time: summed WIP across four conveyors
changes from 13.884 to 10.366 while completed visit rate changes from 530.47 to
641.01 visits/hour. Those rates use each cohort's last-collection horizon;
they differ from picked-unit rate over complete outbound cleanup. All visits
are included and initial/final physical WIP is zero. Order 272 needs one
inventory visit instead of two; its requested units remain unchanged.

Proposal evictions increase from 18,170 to 23,466, with zero inadmissible holds.
This chunk improves simulated throughput; it does not establish a reduction in
proposal churn. Shared-host wall times are not a controlled CPU-speed comparison.

### Final 4,000-order comparison

The same larger cohort completes with all physical assertions passing:

| Variant | Batch completion, sim seconds | Units/hour over complete batch | Poison evictions |
| --- | ---: | ---: | ---: |
| Previous committed workflow (`0b99e53f1`) | 31,734.127 | 521.16 | 115,534 |
| First scheduling chunk (`d7675a1f4`) | 31,763.015 | 520.68 | 137,726 |
| Owned reusable-container departure (`38ab5417e`) | 24,714.932 | 669.17 | 190,310 |

All **4,000 orders, 4,594 units, 4,457 physical SKU transfers, 1,871 cartons,
2,568 buffer round trips and 18,308 VTM cycles** complete. The final batch rate
is 28.40% above the preceding workflow and 28.52% above the first chunk.
Complete hourly windows contain **658, 661, 660, 716, 703 and 660 units**;
536 units remain after six hours. The remaining picks finish at 6h48m17s and
outbound cleanup finishes at 6h51m55s. No hour goes without picks while demand
remains; the longest pick-completion gap, including startup, is 71.033 seconds.
This cohort now drains before eight hours. It does not pass or replace the
separate eight-hour sustained-backlog gate, whose requirements remain unchanged.

The same 1,432 direct-extraction orders clear their final presentation a mean
**1.142 seconds after picking**, versus **91.267 seconds** in the first chunk;
the maximum falls from 1,008.443 to 1.521 seconds. No final-pick Waiting detour
or shortened extraction dwell accounts for that result. The next already-owned
return exposes its real port clearance soon enough for incoming work to plan.

A separate interval calculation compares the identical interior clock window
**7,909.628–18,391.976 seconds**, the intersection of both middle-half windows.
Mean operator duty across the four EasyLoops rises **32.06% → 40.57%**; both
presentation sides holding completed orders falls **33.18% → zero**; mean
conveyor WIP per EasyLoop falls **3.765 → 2.624**. This window is descriptive,
not asserted to be steady state. The remaining operator idle still needs
attribution before changing admission limits or fleet sizes.

Across each complete conveyor cohort, the visit count stays 4,457 and mean
boarded-to-collected residence falls **109.698 → 58.233 seconds**. Independently
integrated occupancy equals visit throughput multiplied by residence time:
summed WIP across all four conveyors is **15.453 → 10.584**, at **507.14 →
654.30 visits/hour**. Initial/final conveyor WIP is zero; the last-collection
horizon differs from whole-batch completion. This verifies Little's law for the
measured population rather than treating conveyor occupancy as capacity.

The final audit reports zero inadmissible holds. Proposal evictions rise
38.18% from the first chunk, so reducing invalid proposals remains separate
work. No controlled CPU-speed claim follows from shared-host wall times.
Final audit binary SHA-256:
`cbb3c49a0e9fc9b7adc500484d6a308f822ee09a4c15a3d0b901f995a3094eb2`.
Exact trace hashes, comparison windows and calculations are retained in
`../notes/walmart-next-wave-2026-09-10/verified-store-release-4000-comparison.json`.

## Remaining workflow work

### Next scheduling comparison

At this checkpoint, `FulfillOrder` starts inventory retrieval alongside
order-container acquisition but waits for the order HTM to reach Waiting before
inventory delivery. The [follow-on comparison](inventory-supply-2026-09-10.md)
removes that dependency through unloading and queue launch while retaining
actual admission limits and custody. Its measured gain is modest; remaining
operator idle still needs physical attribution.

Admission also assigns one worker to each waiting cell and keeps its return
berth through picking. Separate preparation-in-flight limits from physical
waiting-cell occupancy before testing a wider pipeline; adding workers without
an occupancy model would not be a valid capacity improvement. The 500-order
report's busiest VTM, A40, has 89% duty while fleet-average duty is much lower.
The canonical map and roster identify A40–A42 as the first three northern
completed-order buffer VTMs (buffer aisles 12–14, runtime zones 21–23), with
one hard-assigned gripper VTM each. They do not serve the empty order-container
depot. Their measured duties are 89.0%, 76.5% and 56.1%. Inspect buffer allocation
and per-zone waiting before changing fleet or routing: duty alone does not
establish a bottleneck, and deposit selection already considers VTM availability
and estimated work as well as HTM access timing.

### Site-dependent changes

Direct SIOC-to-Flex, route-compatible bag batches, downstream box finishing and
re-induction, and off-HTM auto-bag processing need the physical handoffs and
routing details recorded in the [DoO assessment](doo-2-workflow-assessment-2026-09-09.md)
and [site questions](questions.html). Mandatory return to Waiting for every
unfinished visit remains a separate scheduling limitation. The follow-on
inventory-supply task removes the delivery dependency recorded here; broader
admission changes remain separate work.

The nine-second pick assumption is per physical SKU transfer, not a calibrated
per-unit service curve. Report units and transfers separately. Finite-batch
completion rates include startup and drain; sustained capacity needs a defined
backlogged observation window. Little's law must use occupancy, departures and
residence time for that same population and window.

### Separate reservation-query defect

Review found a pre-existing nonzero-gap loop in
`ReservationTableInner::earliest_free` (`model/reservation/table.rs`). With a
100 ms reservation gap, one hold `[0, 899950)` and a query from zero for an
open-ended interval, its retry sets the candidate to `899950` repeatedly. That
candidate still conflicts with the required gap. The focused reproduction did
not return and its owned test process was stopped. This wave uses the canonical
`entry_intervals` API, which correctly exposes clearance at `900050`, and leaves
the table implementation unchanged. A separate table fix must cover both
finite-duration and open-ended queries before relying on `earliest_free` with
nonzero gaps. Current production zero-gap behavior is not this reproduction.
