# Independent inventory supply

This comparison starts at `c7267374a`, after the known extraction-departure
correction. Fleet, map, initial stock, arrivals, conveyor capacity, operator and
fork dwell, and collision avoidance stay unchanged. The main scenario still
admits all 22,160 source orders; benchmark cohorts are separate inputs.

## Changed dependency

Inventory retrieval already overlapped order-container acquisition and an
unfinished order's return to Waiting. Its next task, the loaded HTM journey
from relay to conveyor dropoff, was still created only after the order HTM's
trip to Waiting finished planning. Delayed order-side work could therefore
keep a ready inventory HTM at its relay.

`SupplyConveyor` now owns the known inventory visit independently: it creates
`RetrieveAndDeliverTote`, admits the arriving tote within conveyor and retained-
HTM capacity, runs `UnloadTote` with its full dwell, and starts `AdvanceQueue`.
It returns without waiting for queue arrival, so picking and collection can
release work ahead of that robot. The order parent starts only the first
required visit or the next visit after an unfinished pick. Split and repeated
SKU lines retain their existing demand accounting.

The children receive specific service handles. Retrieval/delivery returns the
same carrying handoff; unloading consumes it and returns explicit conveyor
custody, a retained HTM, boarding time, source provenance and a queue completion.
The parent retains paired presentation, picking, collection and return work.
Quantity is read from the actual conveyed tote at boarding, before any pick.
No stale carrying handoff survives unloading and no idle HTM is advertised.

## Physical regression and validation

An independent 300-second hold on box induction exposes the old dependency:
inventory delivery did not start until **303.922 seconds**. The regression fails
before independent delivery and passes with it. Extending it through unloading
also fails on the delivery-only candidate: boarding waits until **304.922
seconds** for unrelated order induction. It checks positive-distance
movement, actual arrival at dropoff and the finite end of the exact relay-cell
hold before induction clears. The inventory tote remains on its HTM through
the full unload dwell; picking cannot precede order acquisition. The final
regression also requires actual conveyor boarding and the retained HTM to
reach its queue while induction is still blocked.

The delivery-only candidate passes all 32 release composition and fulfillment
integration tests. Coverage includes Cencora composition, actual paired arrivals,
delayed extraction, repeated and split demand, mixed-SKU inventory, full Parking
failure, carton lifecycles, buffer custody and no final-pick return to Waiting.
The independent-supply candidate also passes all 32 tests, including the
stronger unloading-and-queue regression, plus all 12 scheduler tests. The release
application builds successfully. Physical source-workload audits are separate
from these compact regressions.

## Proposal diagnostics

Existing logs report total poisoned proposals but do not identify the dominant
task type or cause. The scheduler now counts successful poison evictions by
static task type and prints deterministic sorted totals. It allocates no display
strings per eviction and changes no ranking, acceptance or invalidation rule.
A focused regression distinguishes static types from instance labels, counts
retired attempts once and verifies fresh proposals still accept normally.

Do not infer that all retries are avoidable. Pickup selection passes include
unpublished retries, and source retrieval already observes the chosen relay,
tote and headroom. Removing an unselected HTM can change the selector's refined
candidate batch; ignoring that removal is not demonstrably ranking-preserving.

## Controlled results

The delivery-only 500-order benchmark and physical audit both complete at
**3,382.036 seconds**, with identical event counts: 576 units, 564 physical
transfers, 500 extractions, 246 cartons, 316 buffer round trips and 2,268 VTM
cycles. The preceding binary finishes in **3,322.889 seconds**, at 624.04
units/hour over complete cleanup. Delivery alone is **1.75% lower** in unit
rate (613.12/hour), while poisoned proposals fall 23,466 → 22,934. Its changed
source selections add one physical visit; demand is identical. This is not a
throughput improvement. The subsequent supply candidate includes independent
unloading to remove the additional demonstrated dependency.

Binary/input hashes, source patch, failing-before test log and audit evidence
are retained under `../notes/walmart-next-wave-2026-09-10/`, with the
`delivery-overlap` and `supply-overlap` prefixes.

The final supply benchmark and physical audit both complete at **3,298.235
seconds**, with identical event counts: all **576 units, 563 physical transfers,
500 extractions, 246 cartons, 316 buffer round trips and 2,266 VTM cycles** pass.
That is **628.70 units/hour**, **0.75% above** the preceding 624.04/hour, and
24.654 simulated seconds less for the complete batch. The correction is useful
workflow independence with a modest measured gain, not an approach to the
operator ceiling. The 4,000-order benchmark has not been repeated for this
increment; the preceding 669.17/hour longer result remains a separate checkpoint.

In the same interior clock window, **790.467–2,360.370 seconds**, mean operator
duty changes **41.03% → 41.30%** and mean conveyor WIP per EasyLoop changes
**2.652 → 2.722**. Both presentation sides holding completed orders remains
zero. Full-cohort residence increases **58.214 → 59.134 seconds** while conveyor
visit throughput increases **641.01 → 644.01 visits/hour**. Summed occupancy
across four conveyors, **10.366 → 10.579**, independently equals visit throughput
multiplied by mean residence. Earlier supply can increase time on the belt;
occupancy is not itself a capacity gain. This window is descriptive, not a
steady-state claim. Calculations and trace hashes are in
`verified-supply-overlap-500-comparison.json` under the notes directory above.

### Observed proposal evictions

The final benchmark and audit agree on these counts. The sum is 23,215, down
1.07% from 23,466 in the preceding run. These classify retired proposals, not
all proposal work or CPU cost, and do not establish which retries are avoidable.

| Task type | Poison evictions |
| --- | ---: |
| HtmPickup | 13,939 |
| SelectPresentation | 3,860 |
| VtmReturn | 2,006 |
| ClaimNextOrder | 1,354 |
| VtmForward | 1,201 |
| ClaimReturnedTote | 549 |
| InductContainer | 214 |
| RetrieveTote | 91 |
| Parking | 1 |

HtmPickup accounts for 60.04% and SelectPresentation for 16.63%. Before changing
their invalidation rules, distinguish a changed selected resource, a changed
ranking and an unrelated notification. A larger-cohort follow-up and physical
attribution of remaining operator idle are still needed; this change does not
increase admission limits or the provisional buffer fleet.
