# EasyLoop workload checks

Current source profiles allow 100 units per inventory tote while retaining demand-only stock. The [inventory-batch note](inventory-batches-2026-09-08.md) distinguishes this packing change from the pending full-batch stock choice and records the latest 500-order check.

`native-modules/tests/walmart_flow.rs` runs the named Walmart composition against
the reviewed minimal and DFW6 maps. Both maps have Ghost and CollisionDes cases;
each case uses HTM kinematics so its curved presentation links participate
in routing. Runs have a 60-second wall-clock limit. Every test requires natural completion.

The measured throughput baseline below is generated, prefilled, tote-only and
direct-extraction-only. Mixed-container tests now cover box induction, reusable
trays, physical buffering and distinct extraction outcomes. Full-map mixed
acceptance is tracked separately below; these workloads do not calibrate the
whole Walmart operation. Inbound putaway and site-specific extraction/release
rules remain [open work](remaining-flows.md).

## Inputs

The default Walmart launch uses **all 22,160 delivered source orders**, with
24,789 order lines and 25,625 units. Run `scripts/run_walmart.sh` from the
repository, or open `walmart_dfw6_spec.json` directly. Both retain the source
arrival times; the first and last drops span 47h16m42s. A seven-day simulation
safety horizon allows arrivals and cleanup; it is not a seven-day site operating
schedule. The simulator can finish earlier when the work drains.

The source on-hand snapshot is short by **43 units across 40 SKUs**. No default
silently removes those orders or disables the inventory check. The exact source
deficits are emitted by the preparation manifest; the shortage treatment must
be explicit before a full run can complete. Among the complete order set, 126
orders reference a globally short SKU; this does not mean all 126 orders are
unfulfillable, because they compete for the remaining stock.

Smaller populations below remain explicit internal tests. They are not the
default Walmart workload. For a bounded diagnostic that retains all source
orders, use `scripts/run_walmart.sh --headless --simtime 20h`; its time limit can
leave work pending and does not establish full completion.

| Spec in `sim/dessim/tests/fixtures/` | Workload | Purpose |
| --- | --- | --- |
| `minimal_easyloop_flow_spec.json` | 8 orders, 24 one-unit picks, 12 HTMs | Repeated picks, both presentation sides, shared waiting, retained inventory HTMs, collection and extraction |
| `walmart_dfw6_smoke_spec.json` | 5 orders, 10 picks, 200 stock units | The same task flow on all four machine bindings, including mirrored geometry |
| `walmart_dfw6_spec.json` | All 22,160 source orders, 25,625 units | Default full workload; source arrivals, collisions and HTM kinematics |
| `walmart_dfw6_generated_spec.json` | 500 orders, 2,000 picks, 50,000 stock units | Explicit generated regression workload after smoke checks pass |
| `walmart_dfw6_mixed_spec.json` | 500 sampled source orders, 576 units / 562 transfers at capacity 100, demand-only stock | Real packaging/routing labels, all demand released at time zero; explicit test population of 24 order trays |
| `dfw6_real_spec.json` | `walmart-data/outbound.csv` and `onhand.csv` | Historical four-hour, Ghost-mode diagnostic with a stock waiver; superseded for normal launches by `walmart_dfw6_spec.json` |

The new minimal spec changes workload and fleet count, not geometry. It allows
four simulated hours so the initial serial task baseline can drain all orders. Its map
already contains ten empty order totes, thirty stock positions, fifteen order
buffer positions, eleven relays, three VTMs, extraction and induction endpoints.
Twelve HTMs leave order-tote robots available if eight inventory HTMs are retained.
The test also checks the resolved stock covers demand for each SKU.

The current composition assigns distinct waiting berths to concurrent order
workers. Unloaded inventory HTMs advance through the queue independently, and
collection/storage cleanup runs separately from the next pick. Temporal capacity
and competing collection selection are also checked directly in
`model::conveyance::tests`.

The earlier serial full generated workload completed in both Ghost and CollisionDes
with HTM kinematics. `native-modules/tests/walmart_workload.rs` preserves the 500
orders, 2,000 requested picks and 50,000 initial stock units. Its expensive
tests are ignored by default and run explicitly in release mode with a 48-hour
simulation horizon and a 180-second wall-clock guard. They require natural
completion, all 500 extractions, 2,000 paired transfers, nonoverlapping operator
intervals, correct tote custody throughout each transfer, and no unfinished or
abandoned conveyor custody.

| Earlier serial baseline, 2026-09-07 | Ghost | CollisionDes |
| --- | ---: | ---: |
| Stop reason | Complete | Complete |
| Picks / orders / extractions | 2,000 / 500 / 500 | 2,000 / 500 / 500 |
| Retrieval / return VTM cycles | 2,500 / 2,500 | 2,500 / 2,500 |
| Simulated duration | 83,430.654 s | 84,231.595 s |
| Measured wall time | 68.821 s | 95.394 s |

The concurrent CollisionDes workload, rerun after the relay pressure fix,
completed naturally with all 2,000 picks, 500 orders, 500 extractions and 2,500
retrieval/return pairs. It took 17,029.804 simulated seconds (4 h 43 m 49.804 s)
and 78.249 measured wall seconds.
That workload completed about 4.95 times faster in simulation than the serial
collision baseline, averaging 422.79 picks per simulated hour over the entire
run, including startup and final cleanup. The concurrent full Ghost workload
has not yet been rerun.

Each of the four machines reached eight inventory totes on its conveyor. The
audit also observed eight order HTMs inside each machine's declared cells and
proved simultaneous presence of multiple order HTMs and multiple conveyor totes
at that same machine. Inventory maxima are checked at every boarding/collection
boundary; order HTM counts are observations at those boundaries and pick times,
verified against actual tote custody and robot positions. Operator intervals
remained exclusive and all conveyor custody drained by completion.

These results establish this workload's completion and actual station overlap;
they are not a measured maximum-throughput rating. Wall times are measurements
of the local optimized test run, not limits on other machines.

## Shared dispatch and circuit timing correction

Two further changes were measured separately on the same full generated
CollisionDes workload, with unchanged fleet, stock and motion settings:

| September 7 checkpoint | Simulated duration | Picks/hour | Measured wall time |
| --- | ---: | ---: | ---: |
| Concurrent workers with fixed order batches | 17,029.804 s | 422.79 | 78.249 s |
| Shared pending work, awarded by simulated berth availability | 14,437.316 s | 498.71 | 74.233 s |
| Shared work plus corrected full-circuit timing | 13,871.701 s | 519.04 | 73.956 s |

Every run completed 2,000 picks, 500 orders, 500 extractions and 2,500 retrieval/
return VTM pairs. The combined improvement is 22.77% in picks/hour. Shared
dispatch reduces the spread between machine pick-completion frontiers from
about 7,369 seconds to 436 seconds; after the timing correction it is about
252 seconds. Work moves to available stations instead of remaining in fixed
batches. Released orders retain order-ID priority, and future arrivals cannot
be claimed early.

The timing correction stops charging the declared circuit duration twice.
It restores the existing equal-leg approximation: the configured 45-second
circuit contributes 22.5 seconds before presentation and 22.5 seconds after
picking. The physical value and split remain uncalibrated. A caller-level
regression checks that unloading, picking, final queue travel and loading still
take their additional time. The minimal CollisionDes case completes in
619.149 seconds after both changes; both minimal fidelity modes pass.

Six scheduler checks cover simulated-time claim ordering, competing claims,
arrival gating, deterministic priority, faster berth reuse and cancellation
before acceptance. The full-workload custody/capacity checks remain in force.

## Mixed containers and physical buffering

The generated 500-order acceptance case also runs with 167 order totes,
167 inducted cartons and 166 reusable-tray orders. It alternates destination
groups independently, producing 168 buffered orders; SC and 3P take direct
extraction. The workload retains its 2,000 requested picks, 50,000 initial
stock units, fleet, collision model and kinematics. `FillInventory` assigns
24 of the existing empty order containers as trays, an explicit test input.

At checkpoint `d42fd33c4`, this case completed all 500 orders and extractions,
all 2,000 picks and 5,002 VTM cycles in 12,184.655 simulated seconds
(111.353 measured wall seconds). The audit verified 167 distinct cartons
ended extracted and empty; all conveyor work drained; operator intervals
did not overlap; and each machine simultaneously held multiple inventory
totes and order HTMs. Observed conveyor maxima were 8, 7, 8 and 8.

This changes the container/route mix, so its rate is not a like-for-like
performance improvement over the earlier tote-only table.

After the destination-candidate invalidation and pickup wakeup fixes, the
same mixed workload passed again with identical completion/custody totals
in 12,217.351 simulated seconds (107.147 measured wall seconds).

The separate [500-order source sample](../../sim/dessim/tests/fixtures/walmart_mixed_500/README.md)
exercises source packaging and destination labels with all arrivals at zero.
Its full-map acceptance test additionally checks that each of the 316 DS
orders physically enters and leaves the buffer with its contents. The source
sample exposed a stale pickup candidate that pinned the clock after another
HTM occupied its destination. Such candidates now expire and reselect; pickup
also wakes when reservation changes alone record another robot's departure.

After those fixes, the source sample passed every audit: 500 orders, 576 paired
picks, 500 extractions, 246 distinct box births/removals, 24 seeded trays and
316 filled buffer round trips. The buffer audit checks authoritative contents
at storage, actual VTM pickup, relay deposit and HTM collection; it supports
immediate retrieval without mistaking a reservation for physical removal.
The run completed in 7,744.366 simulated seconds, with 2,292 VTM cycles and
124.372 measured runner wall seconds. The headed build reproduced the same
simulated duration, event counts, 576 picks and 500 completed orders.

Focused regressions cover a newly occupied induction source whose departure
requires a later task acceptance, and both interior/destination pickup holders
that leave without a fleet-registry update. They preserve collision reservations
and scheduler acceptance ordering.

## Running

### Current queueing and extraction checkpoint

The same full-map mixed cohorts now pass with next-ready inventory collection,
physical-readiness presentation admission, completed-order waiting-berth clearance,
reservation-aware storage waits and extraction destination selection at actual
HTM readiness. Geometry, fleet, stock and physical dwell are unchanged within
each comparison.

| CollisionDes + HTM kinematics | Baseline at `4c73c945f` | Current reported batch duration | Current orders/hour | Current transfers/hour |
| --- | ---: | ---: | ---: | ---: |
| Source sample: 500 orders, 576 transfers | 7,744.366 s | **5,378.043 s** | 334.69 | 385.57 |
| Generated mixed: 500 orders, 2,000 transfers | 12,217.351 s | **9,441.955 s** | 190.64 | 762.55 |

Both gates complete naturally with all 500 extractions and the same physical
container/custody totals: 2,292 VTM cycles and 246 cartons for the source sample;
5,002 cycles and 167 cartons for generated mixed work. The source audit again
verifies all 316 filled buffer round trips. The tests took 126.418 and 107.707
measured runner wall seconds, respectively.

Queueing checks independently integrate actual capacity-calendar transitions and
visit-history intervals, verify authoritative inventory custody before boarding,
throughout conveyance and at collection, require every capacity calendar to drain,
and enforce one shared operator per machine. Full-cohort occupancy agrees with
`L = X W`; clipped middle windows include boundary-crossing visits. No
steady-state assumption is made. Optional raw exports include physical visit
times, presentation holds and audited source-order outbound milestones.

Focused checks also pass for alternate-destination navigation (8), physical
presentation readiness (2), berth clearance (2), returned-tote selection (3),
reversed-ready FIFO collection, reservation-only storage wakeups, queue accounting
(9) and observer metrics (105). The final integration pass covers fulfillment
orchestration (11), Cencora putaway, Walmart fixture readiness and minimal/full
smoke flows (5). Historical Cencora parity is still separate unfinished work.

Reported batch duration is the observer's latest accounted event timestamp;
final empty-HTM parking can continue afterward. Resource utilization has its own
station/relay reservation cutoff. See [throughput validation](throughput.md) for
the denominator definitions, rejected WIP reductions, local blocking changes,
tail distributions and controlled results.

### Commands

From the repository root, run the fixture readiness check independently:

```sh
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target cargo test -p native-modules --test walmart_flow fixtures_supply -- --nocapture
```

Run all four flow cases (the test runner can execute them concurrently):

```sh
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target cargo test -p native-modules --test walmart_flow -- --nocapture
```

Run the full-workload cases sequentially:

```sh
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target cargo test --release -p native-modules --test walmart_workload -- --ignored --nocapture --test-threads=1
```

Run the sampled source-file workflow and its physical buffer audit:

```sh
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target cargo test --release -p native-modules --test walmart_site_sample -- --ignored --nocapture
```

For a saved full-map smoke profile:

```sh
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target cargo run -p desvis -- sim/dessim/tests/fixtures/walmart_dfw6_smoke_spec.json --headless --wall-timeout 60s --output-dir /private/tmp/walmart-flow-smoke --name walmart-flow-smoke
```

Use `--fidelity collision-des --settings motion_model=htm_kinematic` for the
corresponding collision run. Do not add `--allow-incomplete` to a correctness run.

## Evidence required by the flow checks

Completion and extraction counts must match the resolved workload, with no
watchdog or headroom alarm. Every pick must also have a committed transfer in
the run's `Conveyances` manager. A test-only audit task runs after `FulfillOrders`
and before teardown, reading the same inventory and robot managers. It preserves
observer shutdown by allowing the runner to release its registry and event senders.

- The inventory tote is on the conveyor while the order tote remains on its
  named HTM at the actual presentation cell.
- Stock leaves the inventory tote and the same SKU enters the order tote at
  transfer completion.
- One machine's operator intervals do not overlap. Conveyor occupancy never
  exceeds eight; presentation occupancy never exceeds two.
- Each delivering inventory HTM stays inside the one-way inventory lane until
  collection and traverses all eight holding cells in order.
- Each order HTM enters and leaves presentation through the declared curved links.
- Recorded playback places that same order HTM at presentation at both ends of
  the pick dwell, even after later navigation events have been processed.
- The minimal workload includes an order that returns through shared waiting
  and presents on the other side.
- All committed tote and robot custody finishes; no abandoned custody remains.

Every resolved machine must perform work; the full-map check exercises both
mirrored layouts as well as the unmirrored pair.
