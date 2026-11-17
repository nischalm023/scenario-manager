# First inventory retrieval alongside order-container acquisition

After an order is admitted and its SKU quantities are reserved, `FulfillOrder`
starts the first `RetrieveTote` for inventory alongside acquisition of the order
container. The parent consumes that completion for its first inventory visit.
Subsequent visits retain their existing dependencies.

This starts at most one early inventory retrieval per admitted order, using the
existing berth/fleet admission limit. Both retrievals still contend for actual
HTMs, VTMs and relay capacity. Inventory movement from the relay to conveyor
dropoff still follows the order HTM's arrival at its waiting berth. Coordinating
those approaches and preparing subsequent picks remain separate changes.

The regression uses one carton order, two HTMs and a long pickup dwell. Before
the change, inventory pickup began at 160.009 seconds, after carton pickup ended
at 130.288 seconds. It now asserts that inventory pickup begins before carton
pickup finishes, that picking waits for both loads, and that the order completes.
All 14 fulfillment orchestration tests pass with collision avoidance enabled;
the suite's two remaining fidelity overrides were changed to the normal mode.

## Full-map comparison

The same 500-order cohort, fleet, geometry, stock and dwell settings complete
with all physical custody, capacity, SKU-conservation and extraction checks.
Both sides of this comparison use capacity 100 with the original demand-only
stock: 576 units in 559 inventory totes, not full 100-unit batches.

| Measurement | Before | First retrieval overlaps |
| --- | ---: | ---: |
| Final completion, simulated seconds | 4,118.252 | 4,069.112 |
| Whole-batch units/hour | 503.51 | 509.60 |
| First pick start, simulated seconds | 102.115 | 74.203 |
| Last pick end, simulated seconds | 3,976.629 | 3,944.042 |
| Units picked during the first hour | 532 | 547 |
| Total physical transfers | 562 | 562 |
| Completed orders / extractions | 500 | 500 |
| Cartons / buffer round trips | 246 / 316 | 246 / 316 |
| VTM cycles | 2,264 | 2,264 |
| Scheduler accepts | 14,458 | 14,458 |
| Invalidated proposals | 14,517 | 14,298 |
| Direct-order final-pick clearance, mean seconds | 50.058 | 63.765 |
| Direct-order final-pick clearance, p95 seconds | 176.372 | 261.828 |

Completion is 49.140 seconds earlier: a 1.21% increase in finite-batch unit
throughput. This is a modest improvement, not evidence of sustained capacity
near the operator ceiling. The first pick starts 27.912 seconds earlier; the
interval from first pick start to last pick end shortens by only 4.675 seconds.
Waiting by completed direct orders at presentation worsens, so the result does
not establish a remedy for downstream blocking. Both direct-order summaries
cover the same 184 orders and use nearest-rank p95.

The 4,000-order control has not been rerun with this change. Release runner
wall time was 260.658 seconds, versus 265.496 in the
earlier control; host load was not controlled, so this is not a CPU benchmark.

Evidence lives in `../notes/walmart-order-preparation-2026-09-09/` relative to
the worktree, including a frozen release test binary, input hashes, before/after
comparison, complete log and physical JSON journals. An initial unoptimized
trial reached the 300-second wall timeout after 72 units; it is retained as
`debug-500.log` and excluded from the release comparison.

## Departure ownership: Daniel's September 9 ruling

A task does not plan the unknown assignment after its own chain. The next task
claiming the HTM, or a parking task, owns departure planning. The intended fallback
is a cheap parking proposal at dwell-end plus 1 ms, revocable before scheduling.
It competes with useful assignments for the exact HTM availability instance.
Only a parking winner selects a parking destination and computes a route.

The #457 worktree (`direct-empties`, branch `race/457-codex`) supplies proposal
competition and pre-acceptance dissolution. Ordinary HTM claim participation and
fallback liveness need integration before Walmart can replace eager parking and
`ParkingRelief`. A proposal winning or disappearing from availability does not
physically release its cell; the selected task must reserve the actual departure.

This retrieval change is independent of that work. The current eager carton
extraction parking and accepted outbound tasks that wait while owning their HTM
remain follow-up work; no departure reservation is shortened here.
