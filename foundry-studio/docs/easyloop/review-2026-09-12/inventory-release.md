# Release inventory against a concrete order-container plan

The navigation-overlap control retained 42/43/42/42 logical inventory visits
across the four EasyLoops. Its analyzed 6.414-hour profile showed a mean
667.1 seconds from logical release to pick, 570.8 seconds from HTM pickup
completion to pick, and 475.8 seconds from pickup completion to conveyor
boarding. The 90th percentile of loaded-to-pick time was 767.4 seconds.

The old supply task acquired its CONWIP credit, retrieved and loaded inventory,
then waited for an order-area claim before dispatching the loaded HTM. Thus
logical lookahead could consume physical robots and relay positions for orders
whose containers were not yet prepared.

## Change

The logical pipeline credit is still acquired immediately when available.
Before scheduling `RetrieveTote`, the supply task now waits for its matching
order parent to publish a concrete readiness bound: the later of the acquired
order container's action-ready time and its accepted area-entry time.

Physical retrieval is eligible at:

```
max(original eligibility,
    logical pipeline start,
    order readiness - inventory fetch lead,
    current simulation barrier)
```

The readiness future may resolve while that timestamp is still in the future.
The timestamp governs physical work; waiting for planning knowledge does not
mean waiting for the container's physical arrival. The fetch lead remains the
existing conservative Fmax estimate, approximately 133–136 seconds on the full
map. Queue factor remains three; physical waiting rights remain six per side.

No specific inventory tote, relay, HTM, path or conveyor slot is acquired while
the supply task is waiting for its matching container plan. Order-container
acquisition preserves one empty HTM for inventory. Once the order robot is
owned, inventory can use the remaining robot, including in a two-HTM scenario.
Later inventory visits retain their original eligibility at the preceding pick.

This is a bounded lookahead experiment, not a prediction of the exact next
operator service time. Subsequent congestion can still delay either stream.
The existing `Released` event continues to denote logical CONWIP occupancy;
`Loaded` and `Boarded` retain their physical meanings.

## Basis and checks

CONWIP's WIP bound controls congestion; it does not prescribe immediate robot
commitment for every assigned order. See [Hopp and Spearman (2004)](https://pubsonline.informs.org/doi/pdf/10.1287/msom.1030.0028).
Coordinating inventory presentations with finite active-order capacity is also
central to [Justkowiak, Kovalyov and Pesch (2024)](https://onlinelibrary.wiley.com/doi/full/10.1002/net.22245).
Those sources motivate this experiment; they do not establish an optimal
Walmart lookahead or endorse our Fmax multiplier.

A full-flow, two-HTM regression blocks box induction until a known future time.
With the release gate removed, inventory retrieval starts at zero despite its
consumer-derived release of 147.659 seconds, and the test fails. With the gate,
retrieval respects the bound, begins before the box is ready, and the complete
pick/extraction/return flow finishes. A separate unit check covers pending
readiness, notification ordering and future-time subtraction.

The library suite passes 967 tests (one ignored, four unrelated profiling
benchmarks excluded), plus all 34 targeted parity, orchestration, collection
and custody integration checks. The collector regression now blocks an order's
presentation rather than requiring its inventory to be loaded before its
container exists. It still proves that the front retained HTM collects another
order's return before its own pick, closes its actual queue hold and completes
both rack returns.

## Matched full-map result

The one-hour comparison uses 230 HTMs, 48 VTMs, all orders eligible at time
zero, direct extraction, the same queue factor and fetch bounds, a ten-second
operator pick, and nine-second extraction dwell. The control binary is
`84e1a9aca`; the changed binary is `78beb6265`.

| Measure | Control | Consumer-timed release |
| --- | ---: | ---: |
| Picks in one simulated hour | 860 | 866 |
| Mean inventory pickup completion to pick | 540.7 s | 99.9 s |
| 90th percentile pickup completion to pick | 771.4 s | 164.3 s |
| Mean inventory pickup completion to conveyor boarding | 449.7 s | 33.5 s |
| Mean loaded inventory HTMs awaiting boarding, all stations | 124.9 | 8.5 |
| Fleet idle time | 23.0% | 39.4% |
| Fleet loading/unloading activity, including custody holds | 46.3% | 14.3% |

The change removes most premature loaded waiting without materially changing
pick throughput. The released fleet capacity should help when putaway competes
for HTMs, but this comparison has putaway disabled and does not validate that
combined workload. Reported traveling activity rises from 28.6% to 44.1%; that
activity classification can include route waits and is not a measure of purely
productive motion.

Physical order-area occupancy remains approximately 4.2–4.7 containers per
station on average, below the twelve available places. Mean logical release to
pick remains long: 614.7 seconds in the control and 582.9 seconds after the
change. Logical work is still prepared early; it now spends much less of that
time occupying loaded robots. Further throughput work must establish the next
binding constraint rather than merely increase the inventory credit limit.

## Long-run validation and provenance

The control completed eight simulated hours with 7,105 picks, or 888.1/hour.
Hourly counts were 879, 929, 882, 872, 883, 920, 879 and 861. It took 3,373.7
wall-clock seconds and ended at the requested simulation limit, with no late
throughput collapse. Its fleet idle fraction was 16.0% over the whole run.

The changed version's eight-hour validation is running separately. Iteration
23 is also available as a headed inspection window, launched without auto-run.
Do not treat the one-hour result as validation of the changed eight-hour flow.

Run artifacts live under
`/Users/daniel/vt/greyorange/notes/easyloop-layout-2026-09-12/`:

- `stable-selection-230-1h`: control one-hour profile and audits.
- `consumer-release-230-1h`: changed one-hour profile and audits.
- `stable-selection-230-8h`: completed control eight-hour profile and audits.
- `consumer-release-230-8h`: changed eight-hour validation in progress.

Use each run's process metadata and frozen binary checksum for provenance.
The control eight-hour profile is named `walmart_78beb62.desprofile` because
the exporter read the current checkout revision when it finished. Its actual
binary was built from `84e1a9aca`, with SHA-256
`c4cbe220b987e19193495670765e97f5eff7dec0945e94886998de7572da452f`.
Iteration 23 was built from `78beb6265`, with SHA-256
`8913421b66493a31e7a11a6ba66c600812e483a936803b03d1c55e0ac1661f90`.
