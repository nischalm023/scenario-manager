# Inventory CONWIP and timed order admission

September 12, 2026 · iteration 17 · baseline `b54de8226`

Inventory preparation is no longer limited to one next visit for each of the
twelve physically resident orders. Each EasyLoop derives its inventory ceiling
using the shared admission law:

`max(1, ceil(queue_factor × F_max / pick_period)) + 2`

With queue factor 3 and a 10-second pick period, the four full-map ceilings are
**42, 43, 42 and 42 inventory visits**. Their static F_max estimates are 132.8,
135.6, 133.3 and 133.0 seconds. F_max combines VTM retrieval and empty-HTM approach
in parallel, then relay pickup, loaded travel, unload and inbound conveyor travel.
It is a conservative design estimate, not a measured maximum under congestion.
The multiplier and two-visit margin do not guarantee continuous operator work.

An inventory visit occupies its logical CONWIP credit from accepted release,
before rack retrieval, through completion of its matching pick. It includes
retrieval, relay handoff, loaded travel and the inbound conveyor. The separate
eight-tote physical conveyor limit stays occupied from unloading until collection,
including the return side after the pick. The order waiting blocks each admit six
physical residents and do not determine the inventory pipeline limit.

## Scheduling changes

- Logical order streams use the derived inventory ceiling. The old half-fleet
  worker clamp is removed. Each logical order still prepares its next inventory
  visit; the count of logical orders can now exceed the twelve local residents.
- Each side has a temporal area calendar. An unfinished order keeps its return
  right while presenting or recirculating. A final pick releases logical assignment
  immediately, while physical membership ends at the committed area-exit time.
- Inventory preparation can begin before area admission. Delivery toward the
  conveyor and order-container acquisition require an accepted area-entry right,
  which may refer to a future simulation time.
- Speculative preparation preserves relay and robot capacity for productive
  work. Relay occupancy includes the loaded HTM's open-ended pickup hold until
  a departure is committed. Pair readiness determines how many empty robots
  must remain available; two-HTM scenarios can still acquire both halves.
- A retained inventory HTM begins its collection task as soon as unloading and
  queue launch finish. It may collect another order's returning inventory tote,
  even while its original tote's order container is not ready.

The service instances are specific, synchronous state managers passed through
typed task constructors. Generic area and pipeline calendars do not depend on
EasyLoop layout names. The existing shared operator remains exclusive.

## Diagnostics

Profile events preserve exact simulation times for assignment, preparation,
dispatch, admission, waiting, presentation, final pick and area exit. Matched
order/inventory readiness uses order and visit identity. Inventory events record
release, loading, boarding and pick completion.

Desvis shows logical assigned orders separately from physical residents, plus
total inventory CONWIP, upstream inventory and loaded inventory in transit.
Older profiles without these events show the measurements as unavailable.

## Validation

Both full-map batches used the throughput-control scenario: all sampled orders
eligible at T0, direct extraction, 10-second operator service and 9-second
extraction. The canonical scenario remains uncapped at 22,160 source orders and
retains its source release times. No order cap was added to either JSON file.

| Workload | Outcome | Simulation duration | Solver wall time |
|---|---|---|---|
| Full map, 100 orders | 100 picks and completed orders | 14m 1.597s | 28.776s |
| Full map, 500 orders | 500 picks and completed orders | 53m 45.264s | 157.647s |

The 500-order run exercised logical stream ceilings 42/43/42/42 and the physical
12-resident limit per station. This demonstrates end-to-end progress with the
larger pipeline. Its approximately 558 picks/hour includes startup and drain;
it is not evidence of a sustained throughput improvement.

The completed eight-hour baseline before this wave delivered **4,468 picks
(558.5/hour)**. The four-operator service ceiling is **1,440 single-unit picks/hour**.
The corresponding eight-hour run of this checkpoint completed with **4,981 picks
(622.625/hour), an 11.48% improvement**. The eight hourly totals were 593, 627,
629, 623, 624, 633, 632 and 620. This is 43.24% of the operator-only ceiling;
it does not establish acceptable site capacity. See [the fleet and VTM isolation
assessment](fleet-isolation.md) for the duty breakdown and follow-up experiments.

Checks completed:

- 960 library tests passed; one ignored and four unrelated scale benchmarks
  excluded from the bounded test run.
- 24 fulfillment orchestration checks, six Cencora map-parity checks, two actual
  HTM pickup-custody checks, and the minimal/full order-area layout check passed.
- Eight Desvis station-diagnostics checks passed, including simultaneous lifecycle
  transitions, matching visit identity and profile compatibility.
- A small scenario with two resident places and eight logical orders finishes
  24 picks with inventory WIP exceeding the former five-stream half-fleet clamp.
  Mixed totes, cartons and trays also complete within the per-side area bound.
- The early-collection regression blocks one order's carton until 600 seconds.
  Its retained inventory HTM collects another order's returning tote before that
  time. Restoring the old delayed-collector wiring makes this test fail.
- Qualified-type-path and whitespace checks passed.

The release binary used for both full-map checks is `desvis-iteration-17`, saved
under `notes/easyloop-layout-2026-09-12` in the surrounding GreyOrange workspace.
Its SHA-256 is
`2b16471a60dc7b34c7022a61a578be793cd8b4866c7d20e9ed9f639121fa65b6`.
It was compiled from this wave before its WIP commit. The 100/500-order
profiles report the then-current `b54de82`; profile metadata is collected at run
time, so the subsequent eight-hour profile correctly reports `1054bde36`. Launch
provenance records the source checkpoint and binary hash separately. Logs are `/private/tmp/walmart-conwip-100.log` and
`/private/tmp/walmart-conwip-500.log`; profiles are in the corresponding
`conwip-100` and `conwip-500` notes directories.

## Still pending

The first order-HTM route still visits its waiting cell unconditionally. It should
proceed directly to presentation when sequencing and paired readiness permit.
Final picks already leave directly without a waiting-cell detour.

The physical FIFO conveyor sequence, belt transfer/changeover conflicts and
penultimate staging are still incomplete. Increasing upstream WIP cannot repair
those scheduling details. Independent order-container preparation before area
acceptance, traffic heatmaps, and sustained throughput diagnosis remain on the
broader docket.
