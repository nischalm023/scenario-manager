# Walmart throughput validation

This is a finite-fleet, finite-capacity network: HTMs circulate through repeated
transport work; conveyors retain inventory totes and their empty HTMs; order
containers may revisit EasyLoop and may pass through the order buffer. Blocking,
travel and synchronization are part of the workload. We do not fit an M/M/c queue
or assume Poisson arrivals, exponential service, independent stations or unlimited
waiting space.

## Current acceptance and review

The subsequent [independent inventory supply change](inventory-supply-2026-09-10.md)
removes delivery, unloading and queue-entry waits on order preparation. Its
500-order benchmark and physical audit finish in **3,298.235 seconds**, versus
3,322.889 immediately before: **0.75% higher whole-batch unit rate**, at
**628.70 units/hour**. All custody checks, 32 composition/fulfillment integration
tests and 12 scheduler tests pass. New diagnostics attribute 60.04% of poison
evictions to HtmPickup and 16.63% to SelectPresentation; avoidability remains
unmeasured. This increment has not repeated the 4,000-order run below.

The September 10 [scheduling wave](scheduling-wave-2026-09-10.md) coordinates
presentation approaches and lets existing storage or Parking tasks plan an
extraction-port departure when incoming work needs that decision. The final
**4,000-order / 4,594-unit audit completes in 24,714.932 seconds (6h51m55s)**,
versus 31,734.127 before the wave: **28.40% higher whole-batch unit rate**
(669.17 versus 521.16 units/hour). All 4,457 physical SKU transfers, 4,000
extractions, 1,871 cartons, 2,568 buffer round trips and 18,308 VTM cycles pass
the physical checks. Six complete hourly windows contain 658–716 units, with
536 units still to pick at six hours. This cohort drains before eight hours;
it does not establish eight-hour sustained-backlog or full-site capacity.

The same 1,432 direct orders clear final presentation a mean 1.142 seconds
after picking, versus 91.267 seconds before the owned-return correction.
In the identical interior clock window, operator duty rises 32.06% → 40.57%,
both sides holding completed orders falls 33.18% → zero, and mean conveyor
WIP falls 3.765 → 2.624 totes per EasyLoop. Full-cohort occupancy, completed
visits and residence time independently satisfy Little's law. Remaining
operator idle is not yet attributed to a particular upstream resource.

The 500-order counterpart finishes in **3,322.889 seconds**, versus 4,006.720
before the wave: **20.58% higher whole-batch unit rate** (624.04 versus 517.53
units/hour). Its headless run and independent physical audit agree on completion
and all event counts. All 31 release composition/fulfillment integration tests
and 20 final focused checks pass. Proposal evictions increased in both cohorts;
this wave does not demonstrate reduced scheduling churn or a controlled CPU gain.
See the scheduling report for input provenance and current validation; the
measurements below describe preceding checkpoints.

The September 9 [DoO 2.0 assessment](doo-2-workflow-assessment-2026-09-09.md)
records the preceding controlled check with full stock and the approved 43-unit
overlay. Starting the next required inventory retrieval during the unfinished
target's return to Waiting passes all physical checks but leaves throughput
effectively unchanged: 4,004.988 → 4,006.720 seconds for the same 500 orders and
576 units (564 SKU transfers). This removes a dependency; it is not a measured
speedup. Source/target presentation coordination and forced parking were still
open at that checkpoint. The draft also establishes different real workflows for
SIOC, batch totes and downstream handoffs that require separate validation.

The following measurements retain earlier stock and scheduling checkpoints.

On September 9, the first inventory retrieval was moved alongside order-container
acquisition. The same capacity-100 **500-order check completes in 4,069.112 seconds**
versus 4,118.252 seconds: 49.140 seconds earlier, or 1.21% higher finite-batch
unit throughput. All 576 units, 562 transfers, 500 extractions, 246 cartons and
316 buffer round trips pass the physical checks. Most of the picking-horizon
gain is earlier startup; completed direct-order presentation waits worsen
(mean 50.058 → 63.765 seconds). The long run below has not been repeated with
this change. See the [comparison and task-boundary ruling](order-preparation-2026-09-09.md).

With 144 HTMs and 40 inventory plus eight provisional buffer/depot VTMs,
the final-dispatch **4,000-order control completes in 32,988.036 seconds
(9h09m48s)**, versus 42,297.482 seconds before that change: 22.01% shorter,
or 28.22% more completed work per hour. All 4,594 unit transfers, 4,000
extractions, 1,871 carton lifecycles, 2,568 buffer round trips and 18,582 VTM
cycles pass the physical custody/conservation checks. At eight hours, 530
orders remain unpicked; the longest physical pick-completion gap is 107.674
seconds. This remains a finite-backlog control, not a calibrated 20-hour site
capacity claim. HTM collisions are enabled; floor-VTM reservations remain absent.

The dispatch-only control keeps the old one-unit inventory packing. Its
500-order counterpart completes in 4,193.720 seconds. The newer
[capacity-100 packing check](inventory-batches-2026-09-08.md) preserves original
demand-only stock and completes 576 units in 562 physical transfers in
4,118.252 seconds. That capacity-only experiment did not fill totes to 100.
The September 9 [automatic preload](automatic-preload-2026-09-09.md) now uses
the full 1,397,589-unit snapshot and mixed-SKU totes with 8 or 9 units each.
Its placement and inventory differ from these controls; compare separately.

### Distance from the operator ceiling

One shared operator at nine seconds per transfer has a loose ceiling of
400 transfers/hour per EasyLoop. Over the latest long run's complete batch,
the observed average is **125.34 transfers/hour per EasyLoop**, or **31.33%**
of that ceiling. Across all four machines that is 501.35 transfers/hour.
This needs roughly 3.19 times the current rate to reach the operator-only
ceiling, or 2.55 times to reach 320 units/hour at one unit per transfer.

The physical measurement horizon is [0, 32,921.197) seconds, ending at the
last inventory conveyor collection rather than the final batch event. Its
middle half is [8,230.299, 24,690.897) seconds; neither window is asserted to
be steady state. Productive work comes from actual pick intervals, and
completed-container blocking comes from exact final-pick-to-clearance holds.

| EasyLoop | Full-horizon transfers/hour | Middle-half transfers/hour | Productive operator duty, full | Both sides held by completed orders, full | Mean conveyor totes / 8 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0 | 125.97 | 133.19 | 31.49% | 14.81% | 2.97 |
| 1 | 126.19 | 133.63 | 31.55% | 17.78% | 3.10 |
| 2 | 124.55 | 129.69 | 31.14% | 19.56% | 3.22 |
| 3 | 125.65 | 132.97 | 31.41% | 19.46% | 3.18 |

The middle window still reaches only about one-third of the ceiling, so
startup and drain do not explain most of the gap. All orders in this control
arrive at zero, and substantial demand remains beyond eight hours.

Two confirmed scheduling issues limit useful overlap:

1. **Completed orders block presentation while awaiting extraction.** The
   1,432 direct-extraction orders retain their final cell for a mean 63.009
   seconds, nearest-rank p95 248.560 seconds and maximum 512.087 seconds.
   Buffered orders clear in 1.218 seconds on average. Both sides hold completed
   orders for 17.90% of the full horizon on average (14.27% in the middle).
   These intervals do not overlap productive operator work. Removing the
   waiting-block detour helped total flow but moved downstream waiting onto
   presentation; see [the dispatch report](outbound-dispatch-2026-09-08.md).
2. **Inventory and order approaches are unnecessarily sequential.** At this
   long-run checkpoint, the task finished acquiring the order container and
   planning it to its waiting berth before creating its first inventory
   retrieval; the September 9 change removes that first dependency. It still waits for the inventory
   presentation's physical readiness before creating the order HTM approach,
   and it processes inventory visits one at a time. These dependencies are
   visible in `FulfillOrder::schedule`. A VTM cycle can sometimes be back-planned
   before its relay-ready time, so the entire VTM cycle is not necessarily
   serialized behind berth arrival. Remaining stock currently returns to the
   rack between separate order visits; on-conveyor reuse is unimplemented.

The remaining **50.70%** of full-horizon operator time is idle without both
sides being held by completed orders. Its division among inventory retrieval,
HTM approach, traffic, sequencing and partial blocking is not yet measured;
we cannot assign the entire deficit to one resource. Mean conveyor occupancy
of about three totes does not prove that burst capacity or circulation is
never binding in another workload. In this run, peak conveyor occupancy is six
at every machine; each conveyor is empty for only 0.56–1.12% of the horizon.
Totes being aboard does not mean the required source and order are both ready.
Little's law describes these observed intervals, not a promise
that adding WIP or fleet will yield a proportional improvement.

There is also a downstream design bound: three extraction ports with nine
seconds of service per order can serve at most 1,200 orders/hour before
clearance. At this cohort's 4,594/4,000 = 1.1485 units/order, that is 1,378.2
units/hour site-wide, or 344.55 per EasyLoop if balanced. Thus the 400-per-loop
operator ceiling cannot be sustained end to end under these downstream
assumptions. It does not explain today's much lower rate by itself: operator
and extraction service are frequently idle while matching, navigation and
reservation-dependent departures wait.

Departure planning follows Daniel's September 9 ruling: the next task claiming
an HTM, or a deferred parking task, owns its route. Extraction must not plan an
unknown successor. That work depends on integration with #457's proposal
competition and dissolution. Independent remaining changes are coordinated
inventory/order approaches and bounded preparation of subsequent picks. Measure
each separately with the custody, capacity and collision gates; adding robots
is not yet supported as the primary remedy.

The [independent review and verified dispositions](review-verification-2026-09-08.md)
retain the earlier checkpoint and its limitations. The [full-source default](workload-checks.md)
contains 22,160 orders and differs from these stock-feasible, all-arrivals-ready
controls. The sections below retain earlier interventions as history; “current”
within an older section means that checkpoint, not the latest revision.

## Earlier queue and extraction checkpoint

The comparisons in this checkpoint used 122 HTMs and 48 VTMs. On September 8,
the supplied full-workflow design changed the map to 144 HTMs; 40 inventory plus
eight buffer VTMs are retained provisionally pending site confirmation. The new
fleet's 500-source-order gate passes in 5,449.938 seconds, versus 5,378.043 below.
See [the document assessment](spec-review-2026-09-08.md) for that separate
comparison and the newly established 20-hour objective. Existing CollisionDes
gates cover HTM floor traffic; VTM floor collision reservations remain absent.

The subsequent [floor alignment](floor-alignment-2026-09-08.md) keeps that fleet
and, with the blocked-departure selection fix, completes the same 500-source-order
gate in 5,350.287 seconds, 1.83% shorter. These changes address port/relay alignment,
floor connectivity and idle-robot eligibility; workload, service dwell and
admission limits are unchanged.

Both full-map collision gates pass with the current queue and extraction changes.
The comparison uses unchanged fleet, geometry, stock, dwell and workload within
each row; it does not compare the two different order populations as equal work.

| Cohort | Baseline batch duration | Current batch duration | Duration reduction | Completed orders/hour | Transfers/hour |
| --- | ---: | ---: | ---: | ---: | ---: |
| 500 sampled source orders / 576 transfers | 7,744.366 s | **5,378.043 s** | 30.56% | 334.69 | 385.57 |
| 500 generated mixed orders / 2,000 transfers | 12,217.351 s | **9,441.955 s** | 22.72% | 190.64 | 762.55 |

The resulting increases in completed orders/hour are 44.00% and 29.39%,
respectively. All 500 extractions, container outcomes, physical buffer round
trips, VTM cycles, conveyor capacity/custody and operator exclusion checks pass.
This is measured batch performance under the current provisional site rules.

## Comparable workloads

Both acceptance cohorts run with collision avoidance and HTM kinematics. Compare
a change against the **same cohort**, preserving geometry, fleet, stock, task dwell
and container/routing mix. Physical custody and final conservation must still pass.

| Cohort at `4c73c945f` | Orders | Item transfers | VTM cycles | Reported batch duration | Orders/hour | Transfers/hour |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 500 sampled source orders | 500 | 576 | 2,292 | 7,744.366 s | 232.43 | 267.76 |
| 500 generated mixed orders | 500 | 2,000 | 5,002 | 12,217.351 s | 147.33 | 589.33 |

Throughput divides completed batch counts by the runner’s reported simulation
duration: the observer’s latest accounted event timestamp. Inventory return and
container cleanup are included, but final empty-HTM parking can continue afterward.
The resource table uses a separate cutoff from station/relay reservation boundaries;
that is not the last arrival across all robot paths either. `OrderComplete` occurs
at the final pick end, before extraction and container cleanup. These milestones
must not be interchanged.

The source sample deliberately releases every order at time zero. Its orders/hour
is achieved batch-departure throughput, not an offered steady arrival rate or an
estimate of sustainable site capacity. Initial fill and final drain remain in the
full-batch result. Middle-window statistics are diagnostics, not automatically
steady-state estimates. The site's actual arrival pattern and sustained-load
capacity require a separate experiment.

## Boundaries and conservation

Use the same customer population and clock window on both sides of every law.
For physical conveyor occupancy a visit begins at completed unloading (`boarded_at`)
and ends at completed collection. Claiming admission is not physically unloading.
After all visits complete, the conveyance calendar has replaced provisional
admission starts with those physical boarding times.

For any measured window `[a,b)`, including visits crossing either boundary:

```
occupancy area = integral N(t) dt
               = sum length([visit entry, visit exit) intersect [a,b))
mean occupancy = occupancy area / (b-a)
```

For an empty-to-empty cohort, this gives `L = X W`, with `X = completed visits /
horizon` and `W = mean full visit duration`. In an interior window, completed-only
sojourn samples omit customers still present and can bias the result. Integrate
clipped intervals for all intersecting visits instead. This is the sample-path
accounting behind [Little's law](https://web.mit.edu/urban_or_book/www/book/chapter4/4.4.html).

The audit integrates the capacity calendar at its actual transition times and
compares that with the separately held visit history. It also checks each full
boarding-to-collection interval against the authoritative inventory custody
timeline, requires robot custody immediately before each post-start boarding,
and verifies the named collector at collection. All capacity calendars must be
empty at the final horizon. Full and clipped windows are checked, and one shared
operator must have no overlapping transfer intervals. An inventory visit's time
separates into boarding-to-pick-start, productive pick dwell, and pick-end-to-collection. The
first and last portions include physical transport and blocking; neither is
labelled pure queue wait without finer milestones.

## Service demand and capacity

Count actual resource visits by workload class. With this implementation, each
inventory exchange uses two VTM cycles; each reusable order container uses two;
each buffered order uses another two. Thus source work requires
`2*576 + 2*254 + 2*316 = 2,292` cycles; generated mixed work requires
`2*2000 + 2*333 + 2*168 = 5,002`. Transfers/hour alone hides this different demand.

For each physically interchangeable resource pool (for example, one VTM zone),
demand per completed order is its summed productive service time divided by
completed orders. Equivalently, `D_j = sum_c p_c * V_jc * S_jc` weights each
class's visit count and service time by its workload share. A pool with `m`
servers gives the necessary bound `X <= m / D_j`. Do not pool all VTMs across
the map when their zones are not interchangeable. This operational accounting uses measured work rather
than assumed service distributions; see
[Denning and Buzen, Operational Analysis of Queueing Networks](https://docs.lib.purdue.edu/cstech/165/).

Each EasyLoop's two presentation sides share **one operator**. Four operators at
nine seconds per transfer give a loose upper bound of 1,600 transfers/hour if
continuously supplied. Travel, pairing, blocking and the finite fleet reduce
achievable throughput. Blocking demand can change with admission WIP, so service
demand invariance must be established before extrapolating a result.
Station-cell reservation duty is occupied time, including
waiting; it is not productive operator utilization. The old station table also
lists each shared conveyor dropoff twice, once per presentation StationId.

## Admission experiment

`FulfillOrders.max_active_orders_per_machine` optionally caps the number of
orders being picked concurrently at each machine. It cannot exceed the declared
waiting berths; the existing whole-fleet cap still applies. Omission preserves
all declared berths (six per machine on the reviewed full map). Inventory return
and completed-order dispatch continue independently, so this is **not** a cap
on all outstanding orders or all HTMs.

The full-workload tests accept `WALMART_ACTIVE_ORDER_LIMIT` to override that root
parameter without changing the fixture. `WALMART_QUEUE_TRACE_DIR` optionally
saves per-visit physical timestamps and window measurements for inspection.
The parameter changes admission policy, not the map's waiting/conveyor capacity.

For a reproducible source-sample capture:

```sh
WALMART_QUEUE_TRACE_DIR=/private/tmp/walmart-source-queueing \
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target \
cargo test --release -p native-modules --test walmart_site_sample source_sample_completes -- --ignored --nocapture
```

The JSON contains each visit's physical milestones, full/middle window
integrals, and actual order-presentation cell holds. A companion order file
identifies final picks and outbound routes. Set `WALMART_ACTIVE_ORDER_LIMIT=4`
only for the corresponding controlled admission experiment; leave it unset
for the default six-berth workload.

## Sustained source backlog

The 500-order run starts draining near minute 79: 558 of its 576 physical picks
have completed by then. Its last pick finishes at 81m47s and last extraction at
89m04s. That tail does not establish a loss of sustained service capacity.

Generate a larger finite backlog with `scripts/prepare_walmart_throughput.py`.
The explicit internal cohort selects 4,000 whole orders evenly across the source order IDs,
checks their combined demand against original on-hand stock, and replaces any
infeasible selection deterministically. It preserves packaging, destinations,
quantities and stock; only drop times become T0. The manifest records input hashes,
selected orders and replacements. This deliberately models a ready backlog, not
the site's arrival or replenishment schedule.

```sh
python3 scripts/prepare_walmart_throughput.py --orders 4000 --arrivals ready --output-dir /absolute/run/folder
WALMART_LONG_SPEC=/absolute/run/folder/world_spec.json \
WALMART_QUEUE_TRACE_DIR=/absolute/run/folder/physical-audit \
CARGO_TARGET_DIR=/private/tmp/codex-walmart-map-target \
cargo test --release -p native-modules --test walmart_site_sample sustained_source_backlog -- --ignored --nocapture
```

This gate keeps every source-sample physical custody check, requires natural
completion with collisions and HTM kinematics, and requires demand to last past
eight hours. It exports hourly physical pick completions and remaining demand
alongside the conveyor and order traces. Startup and drain still belong to the
batch; a partial final hour must not be compared with a full hour by raw count.

The first 4,000-order control at `095847f21` exposed a relay admission deadlock
after 32 picks. An unaccepted retrieval retained its old start after another
relay consumed its aisle's last inbound headroom. It then pinned the scheduler
before accepted work could release that capacity. The correction invalidates
unaccepted same-aisle retrieval quotes when inbound headroom fills. It preserves
accepted physical reservations and the headroom limit.

The pre-alignment corrected run passes all physical gates: **4,000 orders, 4,594 paired picks,
1,871 inducted/extracted boxes, 2,568 filled-order buffer round trips and 18,582
VTM cycles**. The reported batch duration is **42,421.609 seconds (11h47m02s)**;
the last pick ends at 42,039.454 seconds and last extraction at 42,409.626 seconds.
At eight hours, 1,297 orders have not begun picking, versus 24 picking berths.
The maximum gap between physical pick completions, including startup, is 114.214
seconds. The run completes naturally with collisions and HTM kinematics enabled.

| Full simulated hour | Physical picks completed | Picks remaining at end |
| --- | ---: | ---: |
| 0–1 | 380 | 4,214 |
| 1–2 | 359 | 3,855 |
| 2–3 | 353 | 3,502 |
| 3–4 | 347 | 3,155 |
| 4–5 | 361 | 2,794 |
| 5–6 | 350 | 2,444 |
| 6–7 | 600 | 1,844 |
| 7–8 | 456 | 1,388 |
| 8–9 | 423 | 965 |
| 9–10 | 354 | 611 |
| 10–11 | 359 | 252 |

The final 40m39s of picking completes the remaining 252 units. Full-batch rates
are 389.86 picks/hour and 339.45 extractions/hour, including startup and drain.
They are measurements of this finite source backlog, not site-capacity estimates.

Over the common boarding-to-last-collection horizon, the four conveyors average
1.72–1.79 physical totes aboard and their four shared operators spend 24.24–25.12%
of time in productive pick dwell. Independent visit and capacity histories satisfy
the full and clipped-window occupancy identities. These observations motivate
measuring supply and order-container transport delays before adding conveyor
capacity or fleet; they do not by themselves identify which upstream resource
limits throughput. All three extraction ports are used (1,337 / 1,385 / 1,278 orders).

The persisted run inputs, manifest, failed control log and successful physical
traces are under `../notes/walmart-throughput-2026-09-08-4000` relative to the
worktree. The successful gate took 1,234 wall-clock seconds; that timing includes
concurrent reporting compilation and is not a simulator speed benchmark.

## Investigation record

Three physical intake cells (664, 666, 669) have maximum holds of roughly
865, 642 and 814 seconds in the source baseline; cell 671 clears in about 4.5
seconds. Their internal inventory queue graphs have the same geometry and
connectivity. This motivated measuring admission, presentation and collection
delays before changing dispatch. Static nearest-extraction selection was another
hypothesis, subsequently tested below. Site-specific VTM technology remains
separate work; these experiments do not establish a need for more VTMs.

### Controlled source-sample admission sweep

| Picking-order limit per machine | Reported batch duration | Decision |
| --- | ---: | --- |
| 6 (existing default) | 7,744.366 s | Baseline reproduced exactly |
| 4 | 8,713.228 s | Rejected: 12.51% longer batch completion |
| 2 | 10,808.724 s | Rejected: 39.57% longer batch completion |

With limit four, group 2 averages 153.381 seconds from boarding to pick start
and 401.150 seconds from pick end to collection, with 5.676 inventory totes aboard
on average over the common 0–8,538.545-second conveyor horizon. In comparison,
group 1 averages 24.424 and 26.150 seconds for those respective phases. These phases
include transit and waiting; the measured difference motivates investigating
return-collection sequencing, rather than assuming all remaining time is operator
service. The full and clipped-window queue identities pass.

### Original-HTM matching explains the return delay

The instrumented six-order baseline reproduces 7,744.366 seconds exactly. Its
physical HTMs collect in their boarding order. Inventory totes finish their
picks in a different order. With the original assignment, an HTM cannot collect
its ready tote until all earlier HTMs collect their own totes.

For each visit `i`, let `r_i = pick_end_i + 22.5 s` be the model's return-ready
time. Original-HTM matching forces at least
`max(0, max(r_j for j before i in HTM queue) - r_i)` additional delay, even with
instantaneous collection travel and forks. This is a lower bound for that
specific assignment rule, not for a work-conserving collector that can load a
different ready tote.

| Machine group / intake | Visits | Mean return-ready→collection | Mean forced delay from original assignment |
| --- | ---: | ---: | ---: |
| 0 / 664 | 84 | 265.13 s | 253.50 s |
| 1 / 671 | 273 | 9.80 s | 5.12 s |
| 2 / 669 | 116 | 183.60 s | 172.55 s |
| 3 / 666 | 103 | 176.94 s | 166.58 s |

The assignment rule explains most excess return delay on the three blocked
machines. Collection now keeps the HTM queue FIFO and selects the next ready
inventory tote when the queue-head HTM is available. No early HTM exit,
additional conveyor capacity, shorter physical dwell or additional fleet is
introduced. Selection remains revocable until acceptance, so publication of
an earlier-ready tote can replace a future selection.

### Removing one queue does not guarantee greater throughput

| Six-order policy, same fleet and physical timings | Source sample | Generated mixed |
| --- | ---: | ---: |
| Original-HTM matching | 7,744.366 s | 12,217.351 s |
| Next-ready tote collection | 8,110.872 s | 12,093.793 s |
| Next-ready collection plus physical-readiness presentation gate | 8,076.041 s | Not measured separately |

Next-ready collection brings mean pick-end-to-collection down to 25.683 seconds
at every source-sample machine: 22.5 seconds of outbound conveyor travel plus
collection travel and fork dwell. Nevertheless, the source batch takes 4.73%
longer, while the generated batch takes 1.02% less time. The waiting has moved
upstream. This physically appropriate correction is not, by itself, a demonstrated
throughput improvement on the source workload.

Presentation selection also waits until the inventory tote is physically ready;
a future unload schedule cannot claim a presentation ahead of earlier-ready work.
The source result improves only slightly. Actual order-HTM cell reservations show
where a much larger delay remains:

| Machine group | Mean presentation occupancy before pick start | Mean occupancy after pick end |
| --- | ---: | ---: |
| 0 | 1.493 s | 199.632 s |
| 1 | 1.867 s | 14.696 s |
| 2 | 1.510 s | 81.770 s |
| 3 | 1.464 s | 148.055 s |

These are physical cell-hold durations around each transfer, not operator service.
The medians are only about 1.1–1.4 seconds; rare holds lasting roughly
2,482–3,124 seconds dominate the means on groups 0, 2 and 3. Reporting only
average operator duty would hide this blocking pattern.
In that checkpoint, completed order dispatch waits for downstream relay/storage
capacity from the presentation cell.

### Clear presentations while retaining physical waiting capacity

Completed orders now return to their owned waiting berths before dispatch. A
worker does not admit another order until the departing HTM's exact reservation
has a finite physical clearance time. The downstream child continues separately;
the worker does not wait for extraction and empty-container storage to finish.
Storage waits also observe reservation-only departures and refresh their
departure time after waking, preventing missed wakeups and backdated travel.

The combined source run completes all physical audits in **8,648.460 seconds**.
Mean post-pick presentation holds fall to 1.236, 1.308, 1.333 and 1.264 seconds.
Mean boarding-to-pick-start falls to approximately 29 seconds on every machine.
The last conveyor collection is at 7,373.233 seconds, but outbound work continues
for another 1,275.227 seconds. This is **not an overall throughput win** against
the 7,744.366-second starting checkpoint. Clearing the local queue has exposed
downstream blocking; reported batch completion remains the comparison measure.

The run uses extraction ports 17 and 18 for 268 and 232 orders, respectively;
port 19 receives none. This imbalance is a diagnostic observation, not proof
that adding extraction capacity would help. Station-cell occupancy includes
travel and waiting, and destination selection currently uses path distance.

### Follow the downstream custody milestones

Reranking the relay parking-relief fallback produced exactly the same simulated
duration and event counts. That experimental change is excluded from this work
chunk. Audited per-order milestones locate the downstream delay more precisely:

| Buffered-order phase | Mean | Median | 95th percentile | Maximum |
| --- | ---: | ---: | ---: | ---: |
| Last pick → buffer rack storage | 55.81 s | 54.01 s | 69.12 s | 93.41 s |
| Rack storage → VTM pickup | 17.18 s | 7.20 s | 57.45 s | 94.33 s |
| VTM pickup → relay deposit | 4.83 s | 4.83 s | 4.83 s | 4.83 s |
| Relay deposit → HTM pickup | 2.08 s | 2.00 s | 2.00 s | 14.83 s |
| HTM pickup → extraction start | 708.89 s | 57.79 s | 3,795.69 s | 4,128.86 s |

Each phase includes its physical travel/service as well as waiting. These are
not all queue-only durations. Over `[0,8648.460 s)`, the 316 buffered orders'
HTM-pickup-to-extraction intervals account for an average of approximately
25.9 loaded HTMs. This is another closed-cohort `L = X W` calculation using
the same stage and window, not a steady-state capacity estimate.

### Choose an available extraction destination when the HTM is ready

The generic `Navigate` child now optionally accepts several declared destinations.
It selects at the HTM's actual departure readiness, excludes destinations held
indefinitely by another robot, and ranks the others by a loaded-motion travel
quote combined with known finite destination holds. If every reachable choice
is blocked, it waits for reservation changes and reselects. The chosen route
still goes through the existing collision planner. This is a local dispatch
estimate, not a globally optimal route or a new capacity claim.

`DispatchOrder` supplies all extraction ports and uses the returned physical
destination for extraction and empty-container return. An earlier nearest-port
hint may help buffer retrieval, but no longer permanently assigns the final
extraction destination before the HTM is ready.

The source sample completes in **5,378.043 seconds**, versus 8,648.460 for the
immediately preceding control and 7,744.366 at the starting checkpoint. Ports
17, 18 and 19 receive 172, 169 and 159 orders. The four machines perform
151, 147, 133 and 145 transfers. The last conveyor collection is at 4,933.118
seconds and the last extraction at 5,344.162 seconds.

| Buffered-order HTM pickup → extraction start | Previous control | Available-port selection |
| --- | ---: | ---: |
| Mean | 708.89 s | 260.01 s |
| Median | 57.79 s | 62.43 s |
| 95th percentile | 3,795.69 s | 866.89 s |
| Maximum | 4,128.86 s | 1,091.12 s |
| Mean HTMs in this stage over each cohort's 0→last-extraction window | 25.90 | 15.37 |

The median did not improve; the gain comes from reducing the long blocking tail.
Upstream buffer stages remain short relative to that tail. This supports changing
destination selection rather than increasing fleet or pretending station-cell
occupancy is productive extraction service. Significant outbound blocking remains.

The separate generated mixed gate also passes in **9,441.955 seconds**, with
2,000 paired transfers, 167 carton extractions, 168 buffer round trips and
5,002 VTM cycles. Across the common 0–9,356.533-second conveyor horizon, mean
conveyor occupancy is 3.365, 3.547, 3.370 and 3.428 totes; measured productive
operator fractions are 47.33%, 49.63%, 47.33% and 48.09%. These are full finite-run
fractions, not steady-state utilization estimates. Physical post-pick
presentation holds average about 1.4 seconds, and pick-end-to-collection remains
25.683 seconds on all four machines. Independent calendar integration agrees
with visit-history occupancy in both full and clipped windows.

### Next measurements

Keep these two cohorts as controls. Next separate loaded outbound travel from
reservation blocking, including extraction egress and empty-container storage,
before changing dispatch further. Check the distribution by route and zone as
well as the mean; local queue reduction can move the bottleneck elsewhere.

The current destination choice checks static reachability and destination holds,
then collision-plans the selected route. A collision-planning failure on an
interior corridor still propagates instead of trying the other free ports.
Neither full acceptance cohort encounters this; a blocked-corridor fallback
regression is a follow-up robustness check.

There is also a known scheduling opportunity at presentation: `PresentTote`
currently waits for inventory readiness before the order HTM starts its approach.
Safe overlap could recover approach travel time, provided a future inventory tote
cannot claim a slot ahead of earlier-ready work and actual paired custody remains
the transfer prerequisite. Current tests preserve the full circuit, order travel,
pick dwell and loading time; no timing is removed to obtain the results above.

### September 8 aligned-floor long-run acceptance

The final 4,000-source-order collision gate passes in **42,297.482 simulation
seconds**, with all 4,594 paired picks, 4,000 extractions, 1,871 carton orders,
2,568 buffer round trips and 18,582 VTM cycles. Custody, eight-tote conveyor
capacity, operator exclusion and final conservation checks pass. The longest
physical pick-completion gap, including startup, is 107.674 seconds; 1,288
orders are still unpicked at eight hours. Runner wall time is 1,852.130 seconds.
The test allows 3,600 wall seconds after an earlier 1,800-second guard expired
during final returns; physical timing and required outcomes are unchanged.
See [the floor-alignment evidence](floor-alignment-2026-09-08.md) for the
finite-cohort comparison and known Cencora/inbound-putaway failures.

### Presentation occupancy in the 4,000-order checkpoint

Physical order-HTM holds overlap at both presentation cells for only 1.447,
34.753, 19.559 and 50.079 seconds on machines 0–3 over the same 41,904.503-second
conveyor horizon (all below 0.13%). Their pick splits between first and second
slots are 961/132, 1,018/159, 972/188 and 1,007/157. Each order's local pick
sequence starts at zero, biasing the many single-pick orders toward the first
slot. Balancing this preference would distribute visits; it would not establish
a throughput gain. The current order HTM also starts its approach only after
inventory presentation readiness. Readiness overlap and upstream/downstream
waiting need measurement before increasing equipment or claiming saturation.

### Hourly mix in the aligned 4,000-order run

These counts use physical transfer ends, each order's last pick end, and physical
extraction ends, respectively. The windows are `(start, end]`; an order finishing
its picks may be extracted in a later window. Do not equate a pick completion
with an extracted order.

| Simulated hour | Physical transfers | Orders finishing picks | Orders extracted |
| --- | ---: | ---: | ---: |
| 0–1 | 387 | 387 | 347 |
| 1–2 | 357 | 357 | 364 |
| 2–3 | 357 | 357 | 356 |
| 3–4 | 347 | 347 | 352 |
| 4–5 | 361 | 361 | 357 |
| 5–6 | 354 | 341 | 355 |
| 6–7 | 601 | 231 | 241 |
| 7–8 | 451 | 331 | 306 |
| 8–9 | 428 | 337 | 340 |
| 9–10 | 355 | 355 | 350 |
| 10–11 | 362 | 362 | 359 |

The remaining 234 transfers finish by 11h37m58.820s; extraction and cleanup
continue afterward. The 6–7-hour window's 231 orders finishing picks comprise
216 cartons, 14 totes and one tray. The ready-backlog cohort processes source-ID
ties lexicographically, concentrating multi-unit orders in this part of the run.
The extra transfers per container coincide with the higher pick rate; this is
not evidence of an equivalent increase in extraction capacity. Unit count,
packaging and routing mix change together, so an isolated per-order overhead
model cannot be inferred from this table alone.
