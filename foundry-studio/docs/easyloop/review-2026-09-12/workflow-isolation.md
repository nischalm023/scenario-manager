# Isolate outbound workflow capacity

The matched one-hour control at `78beb6265` completed 866 single-unit orders.
Three extraction ports each spent nine seconds in service and approximately
three seconds between services. Their measured combined turnover of about
895 containers/hour motivated removing the entire extraction stage, rather
than merely changing its dwell.

## Supported controls

`FulfillOrders` accepts independent boolean task parameters
`bypass_extraction` and `bypass_box_induction`. Both default to false. The
[solutions-team guide](../isolation.html) describes the behavior and links four
presets whose only differences are these parameters.

Extraction bypass preserves the loaded order HTM's route through its area's
egress and releases its admission right at actual clearance. Contents disappear
there with no service dwell. Cartons disappear; reusable totes and trays retain
custody until their normal empty-container return finishes. It removes buffer
storage as well as extraction travel/service. It emits no extraction service
cycles, so those rates read zero. It does not report a fictitious extraction
station at an EasyLoop.

Box-induction bypass selects an available empty HTM using its travel to the
assigned block as the proximity criterion. A box appears and becomes its cargo
at the HTM's current location without induction travel or pickup dwell. Its
normal loaded journey to the EasyLoop remains. It still competes for a finite
robot, respects its future availability and empty-fleet headroom, and requires
an area admission right. Reusable-container retrieval is unaffected.

Both controls are present in the saved composition and logged when enabled.
These are isolation experiments, not changes to the faithful scenario defaults.
They remove physical constraints and therefore cannot demonstrate attainable
full-workflow throughput on their own.

## Verification

The final library suite passes 970 tests (one ignored, four profiling benchmarks
excluded), plus 36 Cencora parity, fulfillment, collection and custody integration
checks. Integration tests remove the actual port definitions, exercise the
switches separately and together, and require genuine picks, area clearance,
carton removal, reusable-container returns and final empty-HTM conservation.
The small-map flow with box induction disabled completes with two HTMs and retains normal extraction dwell.

The first induction variants at `9a1f91a36` failed on `CreateContainer` custody
validation. The selected HTM was physically at its idle relay, but its completed
parking leg still advertised the old origin as cancellable availability. A new
unit regression reproduced the mismatch. Instant supply now uses the existing
same-cell navigation handoff before creation: it establishes productive work
without moving, rotating, replaying parking or adding dwell. The regression
passes and checks both the position and timestamp. The correction is
`f33b678b8`; failed runs are excluded from the comparison.

## Extraction disabled; normal box induction

| Measure | Normal extraction | Extraction bypass |
| --- | ---: | ---: |
| Picks in one simulated hour | 866 | 1,264 |
| Fraction of four operators' 1,440 single-unit/hour ceiling | 60.1% | 87.8% |
| Fleet idle fraction | 39.4% | 34.2% |
| Mean physically resident orders per EasyLoop | 4.2–4.7 | 7.5–7.7 |

The bypass's four quarter-hour counts are 290, 321, 327 and 326. Its gain is
46.0% over the matched control. This identifies the removed outbound travel,
service and associated traffic as a major constraint. It does not establish
that nine seconds is the correct real extraction dwell, nor that all turnover
is avoidable scheduling delay.

The extraction-disabled frozen binary is iteration 24, source `2fcab7d80`, SHA-256
`bb49946b8fa6f0479b873459de555f4a37c0fe80349ce431192c7ac3e5dcb18d`.
The profile filename `walmart_9a1f91a.desprofile` reflects the checkout at export,
not the frozen source. Use the run metadata for provenance.

The run with box induction disabled and normal extraction at `f33b678b8` stopped
after 560 picks. Its committed event horizon was 2,361.317 seconds; the settled
barrier was 1,812.832 seconds. `HtmPickup` could not plan from cell 11754 to 7675
within the existing 300-second route-search horizon. No valid completed-hour
throughput is reported for this run. This is a temporal routing failure distinct
from the corrected parked-HTM handoff. It needs a reservation/selection audit;
the first-exit feasibility guard does not prove a complete route is feasible.

The run with both extraction and box induction disabled completed the hour:
1,313 picks, 35.84% fleet idle time, and mean physical order-area occupancy
9.29–9.61 per EasyLoop. Its quarter-hour counts are 310, 331, 337 and 335.
It reaches 91.18% of the four operators' ceiling and adds 49 picks (3.88%)
compared with extraction disabled and normal box induction. All measured orders
still contain one picked unit. These results do not validate the remaining
multi-line mix or concurrent putaway.

That frozen binary is iteration 26, source `f33b678b8`, SHA-256
`f9fc87a89cfb8a5d37a21524f2ae3506861a899d848402bf843222761d6a99fc`.
Its one-hour gate exited successfully at the requested 3,600-second limit.
A separate eight-hour run with both stages disabled is being started to check
whether the improvement persists. The normal-workflow eight-hour validation
of the inventory-release change remains separate.
All runs use the full map, 230 HTMs, 48 normal VTMs, T0 orders, queue factor three,
ten-second operator picks and the same inventory preload. Normal extraction
retains its nine-second dwell. Putaway is absent from these comparison tasks.

Artifacts are under the surrounding workspace's
`notes/easyloop-layout-2026-09-12/`: `consumer-release-230-1h`,
`no-extraction-230-1h`, `no-induction-230-1h-v2`, and
`no-induction-or-extraction-230-1h-v2`. `compare_workflow_isolation.py` reads the
profile summaries and writes `workflow-isolation-comparison.json`.
