# Order buffering and outbound capacity — 13 September

The objective is sustained picking near the operator ceiling with normal box
induction, all outbound processes, and order buffering operating. Removing
those stages remains a diagnostic comparison, not the target workflow.

## Eight-hour baseline completed

The eight-hour headless run `post-pick-buffered-230-8h` started at 11:42 a.m.
Pacific on September 13. It uses the frozen iteration-27 program and map,
230 HTMs, 40 inventory VTMs plus eight buffer VTMs, all 22,160 orders eligible
at T0, ten-second picks, and nine-second outbound handoffs. Extraction and
box induction are enabled. Buffering is enabled for the provisional DS route.
Putaway is not part of this composition.

The frozen spec uses zero buffer dwell, the previously approved throughput
approximation. The repository's [buffered throughput preset](../../../sim/dessim/tests/fixtures/walmart_dfw6_throughput_buffered_spec.json)
now uses a one-hour synthetic hold for the next comparison, as described below;
that completed process used its frozen zero-hold input. The main scenario's
real arrival curve is unchanged. The run is an assessment
of the current implementation, not evidence that all DoO workflows are complete.
The run reached its eight-hour simulation limit and exited successfully at
12:53 p.m. Pacific: **10,013 picks, 1,251.625 picks/hour, 9,967 extractions**.
Hourly pick counts were 1,232, 1,251, 1,255, 1,256, 1,258, 1,264, 1,261 and
1,236. This is 86.9% of the four operators' 1,440 single-unit picks/hour
ceiling, with no sustained late-run collapse. The source-ordering caveat below
still applies: this prefix does not validate multi-unit order performance.

**The occupancy disagreement is a confirmed rack-slot allocation defect.**
The baseline's 6,625 deposits include **3,331 deposits into an already occupied
slot**. Per-container storage integrates to 541,137,878 container-ms; counting
distinct occupied `(rack_id, level)` pairs gives 341,036,143 slot-ms, exactly
the aisle snapshots' integral. Nineteen containers remain in thirteen slots.
The chart counted physical slots correctly; the simulation allowed multiple
containers to occupy them. The recorded pick count is real output of that
implementation, but is not a validated warehouse capacity result.

The independent reconstruction joins each storage operation to that tote's
visual arrival at the exact same timestamp to recover the level missing from
older operation rows. It uses the operation's rack ID, not the drawing cell,
because different racks may share a drawing cell. Both profiles have complete
matches. `buffer-slot-audit.json` beside each profile preserves every conflict.

Frozen build: `33452fd27`, binary SHA-256
`dab90516fc9c46d52fd2accf0549d4eb025fc08fcf020c6c06ebeb82e51b753f`.
The surrounding workspace's `notes/easyloop-layout-2026-09-12/` contains
`post-pick-buffered-230-8h-inputs.json`, the frozen spec, process metadata and log.

## Hypotheses to test

1. Separate post-pick handoffs reduce competition for the former three shared
   ports. The predicted mechanism is shorter final-pick-to-area-exit delay,
   followed by better replenishment of each EasyLoop's order pool and less
   operator starvation. A higher short-run pick count alone does not establish
   that mechanism or sustained capacity.
2. Buffer deposit can decouple EasyLoop clearance from extraction readiness:
   a completed container can leave its order area and its delivery HTM can be
   released at a buffer relay. This may help the pick loop even while the buffer
   adds storage/retrieval work. The countervailing cost is additional VTM cycles,
   HTM journeys and relay traffic; assess the eight buffer VTMs separately from
   the 40 inventory VTMs.
3. Uncontrolled retrieval can move the queue from storage onto loaded HTMs and
   relays. Low buffer fill would then coexist with poor fleet availability or
   blocked buffer handoffs. Measure the post-retrieval population and its time
   to extraction, not just rack fill.

## Findings verified in code and source material

### Release is independent of route and downstream capacity

`DispatchOrder` in `sim/dessim/src/flows/fulfillment/orders.rs` awaits physical
storage and computes `due = stored_at + buffer_dwell`. `StoreTote` returns after
the VTM has placed the container in its rack slot. The retrieval therefore
has a simulation-time lower bound tied to actual storage, not wall-clock
completion of a task.

With the configured zero dwell, it immediately creates a named-container
`RetrieveTote`. No route-release controller, downstream admission permission,
or extraction-capacity gate is attached to that retrieval. The extraction
destination is subsequently selected by `Navigate`. The normal planner still
controls transport conflicts; the missing mechanism is a policy deciding when
the stored container should become outbound work.

The CSV reader preserves a cutoff interpretation in `OrderSpec.deadline`;
dispatch does not use it. The order file has `cutoff_time` as an undated time
of day, plus `wave`, `subnode` and `sort_id`. It has no explicit buffer hold,
buffer-release timestamp or dated truck schedule. The grouping fields are not
yet retained by the loader. Their operational meanings and the cutoff's
next-day interpretation remain unconfirmed.

DoO 2.0 §8.4 specifies route/store organisation, release by route cutoff or
dispense availability, and JIT bypass when route conditions and station capacity
allow. §8.7's extraction exception says to withhold release before extraction
starts when capacity is unavailable. Those policies are not implemented by
an unconditional store-then-retrieve cycle.

### Interim one-hour storage hold

Following Daniel's September 13 suggestion, the next buffered throughput
preset sets `settings.order.buffer_dwell_secs` to `3600`. This is a minimum
hold measured from physical rack placement, after the delivery HTM is released
at the relay. Retrieval remains subject to normal resource availability after
that hour; no HTM waits with the order for the duration of the hold.

This synthetic policy creates stored inventory without claiming to reproduce
the site's dispatch schedule. It staggers eligibility by each order's storage
time, whereas a truck/route release can make a group eligible together. Do not
interpret the resulting peak occupancy as calibrated buffer capacity. The site
questions request a dated schedule, its order grouping key, retrieval lead time
and downstream admission policy. The current eight-hour zero-hold run is kept
as the comparison baseline.

Iteration 28, `post-pick-buffered-1h-hold-230-8h`, started headless at
12:11 p.m. Pacific on September 13. It uses the same frozen iteration-27
binary, world and input CSVs as the zero-hold baseline. The only simulation
configuration change is `buffer_dwell_secs: 0 → 3600`; all orders remain
eligible at T0. The input hashes, launch arguments and completion/audit records
are saved under that run name in `notes/easyloop-layout-2026-09-12/` beside the
baseline. It ended at 2:11 p.m. Pacific after its two-hour wall-clock limit,
having reached only **59 minutes 32.670 seconds of simulation**: 843 picks,
282 extractions, 555 buffer deposits and no buffer retrievals. This is not an
eight-hour result. Of those deposits, 345 reused occupied slots, leaving 555
containers in 210 distinct slots. Their distinct-slot integral is
429,461,569 slot-ms, again exactly matching the aisle snapshots. The hold's
throughput and wall-clock behavior need reassessment after the slot fix.

### Exclusive destination slots

Relay selection previously reserved only the diving board. Its recommended
rack slot stayed available to other deposits until `VtmReturn` wrote the
container's arrival. Multiple HTM chains could therefore commit to the same
destination while still traveling through different relays. Selection also
discarded the vacancy timestamp, allowing reuse before a scheduled pickup.

The correction claims a destination across every relay serving it, retains
that claim through the HTM/VTM handoff, and releases it only after inventory
records occupied custody. An abandoned reservation releases the claim without
changing physical inventory. The previous occupant's departure time now bounds
deposit admission. This applies to inventory, empty-order-container and buffer
racking through their existing shared service.

VTM operation rows now retain the rack level. The audit checks slot exclusivity
and compares container-time, distinct-slot-time and aisle snapshots; old
recordings can still be checked using their visual arrivals. Validation passed:
120 relay tests, 127 observer tests, the full and minimal Walmart flow checks,
116 viewer tests, and four independent slot-accounting cases.

Iteration 29's completed 30-minute check (`slot-reservations-0s-hold-230-30m`,
build `54d09416e`) recorded **575 picks (1,150/hour), 535 extractions, 382 buffer
deposits and 367 retrievals**. All three occupancy integrals agree exactly at
25,577,738 slot-ms, with zero overlapping deposits and fifteen containers
remaining in fifteen slots. Mean buffer occupancy was 14.210 slots, peak 24.
This validates the repair over this window; it is not an eight-hour capacity
estimate. Normal box induction and extraction remain enabled, with all source
orders eligible at T0 and zero added buffer dwell for this check.

### Reusable tray availability constrains the hold experiment

The preset's `FillInventory.order_trays` is only 24. The old one-hour-hold
profile records exactly 24 tray creations and 24 trays stored in the buffer,
the last at 1,823,670 ms (30m23.670s); none was retrieved before its horizon.
This is a second constraint, independent of the slot overbooking. The first
1,000 source orders contain 54 DS/SIOC orders. At the single-unit operator
ceiling of 1,440 orders/hour, Little's law gives 77.76 trays held for one hour,
before adding retrieval, picking and empty-return time. The full source mix
has 989 DS/SIOC orders in 22,160, which still implies 64.27 held trays at that
order rate. A 128-tray experiment can isolate this shortage; it is not a site
quantity. The [site questions](../questions.html#reusable-tray-supply) request
the actual supports, quantities and return process, especially as DoO 2.0
replaces the provisional EasyLoop/tray path with direct-source Flex handling.

Iteration 30 started headless at 4:21 p.m. Pacific on September 13 using the
same frozen build as iteration 29. The run is
`slot-reservations-3600s-hold-230-128trays-8h`: eight simulated hours requested,
one-hour buffer hold, 230 HTMs, 48 VTMs and 128 reusable trays in its diagnostic
specification only. Normal induction/extraction and all orders at T0 remain
enabled. The repository scenarios retain their existing tray setting while
the site quantity is unresolved. This experiment changes both hold duration
and tray supply; it is not a one-variable estimate of the hold's effect.
Iteration 30 was manually stopped at 4:42 p.m. after identifying the retry
defect below. Its last progress report was **20m17s simulated, 345 picks,
20m55s wall time**. No finalized profile or eight-hour result was produced.
The process log and explicit stop record retain that incomplete status.

### Failed relay clearance can wake its own storage retry

A two-second CPU sample of iteration 30 at 4:26 p.m. put 1,425 of 1,442 main
thread samples in `StoreTote`'s pressure-relief path planning. The sample
identifies where this interval's CPU time went, not whole-run utilisation.
The declined-relay path took the fleet version before trying to relocate idle
HTMs. A failed relocation takes and restores an HTM; restoration advances that
version without changing any floor reservation. Waiting on the old version
therefore immediately retries the same unavailable relays.

The correction observes the fleet version after synchronous failed relief,
then waits for subsequent fleet, relay, inventory or floor-reservation changes.
It preserves the captured oracle version so an actual committed clearance
still wakes storage. A regression exercises failed relief while another
assigned HTM is scheduled to clear the needed cell at a later simulation time.
This changes retry behavior; it does not cancel any accepted task or travel.
The regression fails immediately with the old version capture and passes with
the correction. Both storage wakeup tests and the minimal/full Walmart workflow
tests pass. The test counts failed restorations so a future recurrence fails
promptly instead of hanging the test process.

### Skip idle destinations that cannot accept the stop

Iteration 31 started at 4:48 p.m. with exactly the same frozen scenario inputs
as iteration 30, using `0926db583`. A subsequent one-second sample found
706 of 710 main-thread samples in an accepted `Parking` task's route search.
Correcting the storage retry does not eliminate the cost of those searches.

The idle-destination registry does not include every working HTM's floor hold.
The relocation helper previously tried a full route search even when the
navigation reservations already proved the destination could not accept an
open-ended stop within its 120-second planning horizon. It now checks that
necessary condition first. A cell with a finite hold that clears within the
horizon remains eligible; route planning still establishes reachability and
commits the actual movement.

The regression records route-search destinations: without the check it searches
the occupied relay and then the available alternative; with the check it
searches only the alternative. The finite-hold and randomized-selection tests
also pass. This is a reduction in wasted computation, not a change to robot
speed or a cancellation of accepted travel.
Validation: all 27 parking-related unit tests and both minimal/full Walmart
workflow tests pass.

### Iteration 32 — corrected eight-hour validation completed

Started September 13 at **5:01 p.m. Pacific**, source `3a3f4bfd4`, using frozen
binary `desvis-iteration-32` (SHA-256
`855c894c1041e2895ac1591abffd6f7739794fb2dc9173cb690550b0d481f718`).
Its inputs are identical to iterations 30 and 31: 230 HTMs, 48 VTMs, all 22,160
orders eligible at T0, ordinary induction/extraction, a one-hour DS buffer hold,
128 diagnostic SIOC trays, and no putaway. The repository tray setting remains
24 pending clarification; 128 is an experimental quantity.

The destination check substantially reduces calculation time. At one observation
iteration 32 reached 27m32s simulated in 2m33s wall time, while iteration 31 had
reached 17m05s simulated in 15m29s wall time. These are progress observations,
not a completed benchmark or evidence of higher warehouse pick throughput.
Iteration 31 was stopped at 5:04 p.m.; its final reported horizon was 18m32s,
306 picks, and 16m31s wall time. It has no finalized profile.

The run completed the full eight simulated hours at **6:03 p.m. Pacific**,
in 61m48s wall time. It produced **9,549 picks (1,193.625/hour overall)** and
**8,673 extractions**. The final five hours averaged **1,235 picks/hour**, or
85.76% of the 1,440/hour operator ceiling. There was no late-run collapse.

| Simulation hour | Picks | Extractions |
| --- | ---: | ---: |
| 1 | 1,067 | 353 |
| 2 | 1,125 | 946 |
| 3 | 1,182 | 1,201 |
| 4 | 1,246 | 1,272 |
| 5 | 1,239 | 1,208 |
| 6 | 1,231 | 1,257 |
| 7 | 1,235 | 1,226 |
| 8 | 1,224 | 1,210 |

The finalized profile passes the independent buffer audit: **zero overlapping
deposits**, with all three occupancy calculations agreeing exactly at
22,153,788,097 slot-ms. There were 6,332 deposits, 5,480 retrievals, and
852 containers still stored at the cutoff. Mean occupancy was 769 slots
(22.30% of capacity), with a peak of 892. Completed buffer visits averaged
62.58 minutes: the configured 60-minute hold plus 2.58 minutes to pickup,
including handling. Retrieved orders then averaged 36.98 seconds to extraction.

The recording reports 66.52% aggregate HTM utilisation and 53.89% aggregate
VTM utilisation. The latter combines inventory and buffer equipment and does
not establish whether the eight buffer VTMs are a bottleneck. The earlier
1,251.625/hour result used double-booked slots and zero buffer hold; it is not
a valid controlled baseline for measuring this run's change in capacity.

The completed audit and process metadata are saved beside the profile under
`notes/easyloop-layout-2026-09-12/slot-idle-prescreen-3600s-hold-230-128trays-8h/`
in the surrounding workspace. As noted below, this order-file prefix still
does not exercise the full multi-unit order mix.

Compare hourly picks and physical extractions, separating startup and the final
hour's held work from sustained behaviour. The hold should increase stored WIP
by approximately buffer arrival rate × one hour once flow stabilises; it adds
no new transport steps. Measure whether it reduces the retrieved-but-not-yet-
extracted population or merely shifts that workload later. Report stored,
eligible-stored and retrieved outbound orders separately, with the eight buffer
VTMs' demand, rather than treating warehouse-wide idle time as their utilisation.

### Sealed-box lifecycle — deliberately deferred

Daniel's September 13 ruling: retain whole-box removal at the sealer handoff
while the other flow and scheduling problems are resolved. Queue the complete
sealed-box lifecycle for later this week, not today's work. The difference
below is an accepted simplification for the current throughput comparisons.

Current DS carton path:

`EasyLoop → buffer storage → retrieval → sealer handoff → removal from model`

DoO §§8.2–8.5 describe:

`EasyLoop → void fill/seal/label → re-induction → buffer or eligible JIT → dispense`

The new sealed-box pickup cell is declared but unused. A sealer handoff still
runs `ExtractOrder`, which removes the box and its contents from the simulated
system. Full sealed-box return and separate dispense therefore remain required.
Re-enabling buffering does not repair that sequence.

### Buffer capacity and utilisation need separate scrutiny

The map has 15 buffer bays × 230 positions = **3,450 slots**, across eight
dedicated aisles. The map invariant and the recorded per-aisle capacities agree.
DoO 2.0 §6.1's table states **27,600 buffer locations**. The scope of that draft
quantity must be confirmed before changing the pilot map; it is not safe to
silently multiply capacity.

The current “Buffer & storage” UI reports aisle occupancy and a
warehouse-wide capacity-weighted mean. Its arithmetic integrates physical
slot-time, but the headline mixes inventory, empty-container storage and order
buffer capacity. A dedicated order-buffer aggregate, with aisle breakdown and
backlog stages, is still needed for this question.

### Eight hours does not cover the order-file mix

The source has 22,160 orders and 25,625 units. Under the provisional destination
mapping, 14,626 orders (66.0%) are DS/buffer-bound: 6,674 bags, 6,963 boxes and
989 SIOC orders.

The loader assigns internal order IDs in source-ID string order; T0 dispatch
then follows those IDs. There are **11,841 single-unit orders before the first
multi-unit order**. At the four operators' 1,440 single-unit/hour ceiling,
eight hours can pick at most 11,520 such orders, even without startup loss.
Thus this run cannot validate later multi-unit sequencing. A separate mixed
cohort or a longer complete-file run is needed. The running baseline retains
its original order sequence.

## Measurement method

[`scripts/audit_buffer_profile.py`](../../../scripts/audit_buffer_profile.py)
reads the saved profile's physical VTM storage operations and extraction stream.
For the current DFW6 map, pass buffer aisles `21,22,23,24,25,26,27,28`.

It reports deposits, retrievals, physical mean/peak fill, completed residence
times, the ages of containers still stored, and storage-pickup-to-extraction
times. Eligibility is inferred from the current fixed-dwell policy, not from
a recorded release decision. Pickup-to-extraction includes relay work, HTM
acquisition, travel and service; it is not labelled pure waiting time.

Occupancy is independently reconstructed from aisle-state transitions and from
each container's storage interval. The sums must agree. Unfinished visits are
clipped at the horizon and retained in the slot-time integral; dropping them
would understate WIP. Little's-law reasoning must use the same population and
time window for inventory, throughput and residence, with censored work stated
separately. A short or draining window is not a steady-state capacity estimate.

The audit passes a synthetic case with overlapping and unfinished visits. The
iteration-27 buffer-bypassed control produces zero physical buffer visits and
zero buffer occupancy, with matching integrals. The enabled run will provide
the first validation of its buffer joins on this map.

## Next decisions after the baseline

- Implement release tied to route eligibility and downstream admission, with
  explicit policies for throughput experiments and real order timing.
- Confirm buffer capacity and the CSV's DS/SC/3P mapping to DTC/DTS/3P-CR.
- Expose buffer storage, eligible backlog and retrieved outbound WIP separately
  in the saved metrics and UI. Retain pick completion and physical extraction
  as different outcomes.
- Validate a cohort containing multi-unit orders and, subsequently, simultaneous
  putaway before treating the result as the complete warehouse workflow.

## Queued for later this week — not September 13

- Correct sealed-box custody and the seal/re-induct/buffer/dispense sequence
  after the current throughput, buffer and scheduling issues are worked out.
  Establish service times and shared equipment capacity without guessing them.
  Until then, preserve box removal at the sealer handoff.
