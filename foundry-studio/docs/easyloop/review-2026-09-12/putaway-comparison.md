# Concurrent putaway comparison

Checkpoint before this experiment: `7a180f9f9`, pushed to both Walmart refs on September 13, 2026. The production main spec still preloads all stock. Two explicit test presets add the shared putaway tasks to the Walmart composition:

- `walmart_dfw6_putaway_partial_spec.json`: `FillInventory.preload_fraction = 0.5`.
- `walmart_dfw6_putaway_empty_spec.json`: `FillInventory.preload_fraction = 0`.

The fraction applies to each SKU quantity, rounded up, and to the available preload tote positions. Unfilled positions still contain empty inventory totes. The remaining per-SKU stock becomes inbound work; it is not duplicated. The total remains 1,397,632 units including the approved 43-unit demo overlay. Initial plus future supply must pass the order-completability check. Individual picks still require stock to have arrived physically.

Both presets retain all 22,160 orders released at T=0, normal box induction and extraction, and the one-hour DS buffer hold. Long comparisons retain the previous diagnostic 128 reusable trays; the repo presets retain 24. Robot fleets stay at 230 HTMs and 48 VTMs.

The provisional inbound workload uses six stations, each with a 60-second fill (60 presentations/hour), and same-SKU lots of up to six units. Partial lots preserve the source quantity; the measured units/hour will therefore differ from the ceiling of 2,160 units/hour. Synthetic arrival ordering retains demand-prioritized SKU selection. This is not a customer-supplied inbound schedule or proof of why order release starts slowly. The DoO also cites 250 units/hour per station; confirmation is pending in the questions document.

`PutawaySupplyDemand` obtains empty totes through the shared VTM/relay/HTM workflow. `PutawayInduction` fills them. `ReturnTotesToRack` is restricted to putaway stations so it does not take ownership of Walmart's order containers or conveyor returns. The source and destination legs use the normal fleet services.

## Metrics and checks

- Existing fill counters are labelled **Inducted totes/hour** and **Inducted units/hour** in the UI.
- **Putaway totes stored/hour** counts the end of the filled tote's return VTM cycle. Returning unused empty supply at the end of demand does not count as completed putaway.
- XLSX preserves actual fill quantities in one row per fill and separates rack-store completions in `Putaway Rack Stores` and the Throughput table.
- The small shared-fleet regression completes all 24 picks for both startup modes, reconciles initial plus inducted stock, and verifies one physical rack return for every fill.
- The existing default preload regression retains exactly the original on-hand quantities and the approved overlay.
- Heatmap tests cover cursor clipping, repeated/revised delivery, reverse seek, legacy data, and MessagePack round trip; egui hover text is checked for units and the approximate-median label. Shader validation passes. These are background checks, not a visual acceptance of the final GPU picture.
- `scripts/audit_flow_profile.py` independently reconciles recorded flow journals with the time series and verifies elapsed canonical movement against playback. Iteration 32 reconciles 9,549 picks, 8,673 extractions, and 1,554,467,925 HTM-ms of movement with zero duration difference or motion overlaps. Station order phases, inventory visit phases, and paired readiness events are present.

## Results

The comparison baseline is iteration 32: 1,193.6 picks/hour overall and 1,235/hour over the final five hours. The partial run adds inbound pressure but also changes initial SKU placement/availability. The empty run measures startup and stock availability as well as transport capacity; its rate must not be attributed solely to fleet contention.

The first full-map attempts used checkpoint `ef815bf6f`:

| Run | Preloaded units | Outcome |
| --- | ---: | --- |
| Iteration 33, partial | 747,243 in 81,030 stocked totes | Stopped around 13 minutes with a route-search iteration-limit error; 216 picks and 364 inducted units reported by the failure gate. |
| Iteration 34, empty | 0 | Stopped around 6 hours 16 minutes with the same error; 28 picks and 12,119 inducted units reported by the failure gate. |

Neither failed run produced a finalized recording. These gate counts include committed plans and are not validated throughput measurements.

The failure reproducer showed an `HtmDeliver` accepted at 26.971 seconds still using that departure anchor after waiting until approximately minute 12. The search explored 4.9 million states across already-settled traffic. Resumed delivery now floors its unplanned departure at the current barrier. A task-level regression fails without that correction (it emits navigation at T=0 after a T=30 clearance) and passes with it; source occupancy remains reserved throughout.

A corrected 30-minute partial-start run completed: **523 picks (1,046/hour)**, **666 inducted units (1,332/hour)**, 125 filled totes and 119 completed rack stores. HTM idle time was 32.9%; VTM idle time was 55.6%. This is a startup interval, not a steady-state estimate. Both profile audits passed: stock reconciles to 747,386 units, buffer accounting agrees, and canonical motion matches playback exactly at 82,930,079 HTM-ms. The eight-hour retries use this correction.

The matching first 30 minutes of baseline iteration 32 contained 525 picks (1,050/hour), only two more picks than this concurrent-putaway case. Iterations **35 (partial)** and **36 (empty)** are the eight-hour retries from pushed checkpoint `be41b389c`.

Iteration 36 completed all eight hours without a planner failure: **28 picks (3.5/hour)**, **9,668 inducted units (1,208.5/hour)**, 1,709 filled totes and 1,702 completed rack stores. Reported HTM/VTM idle fractions were 79.3%/95.5%. Inventory reconciles exactly to 9,640 units; buffer accounting and canonical/playback motion agree (143,263,166 HTM-ms).

The original iteration 36 terminal summary included a six-unit fill committed at T=8h28s, reporting 9,674 rather than 9,668. Reporting now derives flow completions and rates from timestamped journals through the run cutoff. Raw accepted-plan facts remain available. XLSX pick/fill rows exclude later completions, and its coarse throughput windows require fully elapsed source slices. Boundary regressions cover a completion exactly at the cutoff, one just after it, out-of-order journal arrival, and future source slices. The stricter audit identifies the known six-unit discrepancy in iteration 36; it does not alter that recording.

Iteration **37**, a 30-minute empty-start validation from `68141c3a4`, completed with 636 inducted units, 112 filled totes and 103 rack stores. Summary counts and rates, physical inventory, time-series counters, buffer accounting and motion audits all agree. No picks had occurred by that startup cutoff. The reporting change does not alter scheduling; iteration 35 continues with its original frozen binary, and its results must be read through the audited eight-hour cutoff.

Cold-start sequencing is a separate limitation: synthetic inbound prioritizes global SKU demand, whereas `ClaimNextOrder` assigns in order-ID sequence without checking stock availability. Early station work can therefore wait for SKUs not yet inducted. The incoming workload and order-assignment policy need to be assessed together before interpreting this mode as a transport-capacity test.

An independent reconstruction of the input ordering, including the stock overlay, found that only **23 of the first 169 prepared orders** have their first required SKU lot within the first 2,880 inbound lots. That is the theoretical eight-hour lot budget at six stations × 60 totes/hour. The median first-lot rank is 5,385, requiring at least approximately 15 hours even with uninterrupted induction. This is an input-sequence bound, not a forecast of achieved throughput. The site rule is now requested in `questions.html`; these comparisons retain the existing policy.

Station order-WIP and inventory-CONWIP plots now clip to the playback cursor and extend the current value to that cursor, instead of exposing future planned phases or ending at the last transition. The flow audit also checks `initial stock + inducted units − picked units = final stock` and rejects missing nonzero flow counters.
