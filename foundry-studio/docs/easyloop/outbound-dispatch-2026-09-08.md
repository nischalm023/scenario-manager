# Completed-order dispatch and opaque containers — September 8

Completed order containers no longer receive a navigation task back to the
EasyLoop waiting block. Their existing HTMs continue from their final
presentation toward buffer storage or extraction. Returning to a waiting berth
is reserved for orders with another pick to perform.

The waiting-space allocation ends at final pick completion, after checking that
the exact last waiting-cell reservation has physically cleared. An outbound
delay therefore does not reserve an empty sequencing berth. The completed
container's separate presentation reservation remains valid until its actual
departure clears that cell. Downstream congestion can still delay departure;
this change does not invent an exit queue or release occupied cells early.

Carton extraction also plans the empty HTM's real parking departure as soon as
service is scheduled, with travel starting no earlier than service completion.
That makes the extraction reservation finite soon enough for another HTM to
plan an overlapping approach. If parking cannot yet be planned, the original
hold and availability remain intact until service finishes; an intervening
assignment may take ownership, otherwise parking is retried. Failure still
reports the blocked port without losing the robot's hold.

Successful early parking commits the empty HTM to declared parking rather than
leaving it at extraction for possible direct reassignment. That is a dispatch
tradeoff, not a change to service time or collision clearance. Reusable totes
and trays retain their empty-container storage workflow.

## Opacity correction

Container materials already had opaque output and depth writes. The remaining
defect was draw order: the robot pass intended to show robots through racks ran
after containers and therefore painted through tote floors and box walls too.
The renderer now draws rack-occluded robots, opaque containers, and finally
visible robots that respect depth. Robot number labels can still overlay the
container, as requested.

The previous container-only GPU preview could not catch this interaction. The
new actual-pipeline GPU regression includes HTMs, VTMs, highlighted robots,
container contents, carton flaps, tilt and a rack beam. It verifies that covered
container pixels are identical with and without the hidden robots, while the
rack still shows robot silhouettes. This check passes.

## Validation

All twelve fulfillment orchestration checks pass, including collision-enabled
carton/tote/tray routes through both direct extraction and buffer storage. The
first laden route after each final pick must not end in a waiting berth. That
assertion checks the unwanted scheduled stop; it does not claim a route can
never traverse or encounter collision waiting in any such cell.

The full-parking regression preserves the robot and port hold, and verifies
that unsuccessful early planning retries at service completion. Both existing
waiting-berth clearance tests pass. The GPU opacity regression passes.

The final full-map control passes in **4,193.720 simulated seconds (69m53.720s)**
for 500 orders, 576 transfers and 500 extractions, versus **5,350.287 seconds**
before this change: **21.62% shorter**, or **27.58% more completed orders/hour**
for the same finite workload. All 246 carton lifecycles, 316 buffer round trips,
2,292 VTM cycles, conveyor capacity, paired custody and final conservation checks
pass. Geometry, 144 HTMs, 40 inventory plus eight buffer/depot VTMs, dwell,
workload and HTM collision settings are unchanged. Floor-VTM collisions remain
unimplemented. Runner wall time was 251.22 seconds with concurrent compilation;
this is not a simulator-speed benchmark.

The independent event audit exposes a remaining downstream blockage. Direct
orders reach extraction sooner, but now occupy their final presentation while
awaiting an extraction route. Removing the waiting-block detour did not remove
that wait:

| Physical interval (seconds) | Before mean / p95 | After mean / p95 |
| --- | ---: | ---: |
| Direct final pick → extraction start | 237.401 / 595.236 | 82.760 / 216.709 |
| Buffer HTM pickup → extraction start | 195.762 / 584.872 | 69.953 / 202.735 |
| Direct final pick → presentation clearance | 1.253 / 1.521 | 45.068 / 177.153 |
| Buffered final pick → presentation clearance | 1.271 / 1.521 | 1.190 / 1.442 |
| Inventory conveyor boarding → collection | 64.174 / 66.264 | 78.754 / 175.934 |

The longest post-pick presentation occupancy is 323.837 seconds. Both
presentation cells hold completed orders simultaneously for 8.53–15.20% of
each machine's conveyor measurement horizon. This is downstream blocking,
not productive two-sided picking. An explicit exit movement or better
downstream admission must preserve real floor reservations; freeing the
presentation on paper would hide the blockage.

Over identical clock boundaries, 1,036.525–3,109.577 seconds, physical units
picked increase from 228 to 302 and mean productive operator duty increases
from 24.75% to 32.60%. Both runs still have unstarted demand. Nevertheless,
their order mixes differ within this interval, so this is a bounded workload
comparison, not a steady-state capacity estimate. These measurements use the
older one-unit inventory totes. Later 100-unit stock experiments must be
reported separately.

All 104 desvis unit checks also pass. Headed **iteration 03**, checkpoint
`bf961b50b`, includes both corrections and completed the same 500-order cohort
in 4,193.720 seconds with collisions enabled and XLSX output. The separate 4,000-order acceptance run also passes in **32,988.036 seconds
(9h09m48s)**, versus 42,297.482 seconds: **22.01% shorter**, or **28.22% higher
finite-batch throughput**. It completes all 4,594 transfers and 4,000 extractions,
with 530 orders still unpicked at eight hours and a 107.674-second longest
physical pick gap. The same presentation-blocking pattern persists; the
[latest throughput assessment](throughput.md#distance-from-the-operator-ceiling)
quantifies it without treating occupied presentation cells as productive work.

The independent pre-change review and checked findings remain available in
[the verification report](review-verification-2026-09-08.md). Its observations
about completed orders returning to waiting berths describe the older code.
