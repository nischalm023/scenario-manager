# Plan owned departures before incoming journeys need their cells

The 230-HTM control completed 4,811 picks in eight hours (601.375/hour), below
the 144-HTM control's 622.625/hour. VTM isolation improved the latter by only
2.05%. See [the controlled fleet results](fleet-isolation.md).

## What the saved movement trace shows

Joining the 230-HTM profile's inventory visit identities, tote custody changes
and actual HTM movement steps gives these means for all 4,811 completed visits:

| Part of loaded inventory delivery | Mean seconds |
|---|---:|
| Pickup complete to first physical move | 840.338 |
| First physical move to final arrival at dropoff | 20.611 |
| Dropoff arrival to boarding | 2.973 |

The first interval includes both order-area eligibility and waiting for a
route to be planned; this change does not eliminate the entire interval.
For example, order 0 already has an order container in waiting at 30.521s.
Its inventory is loaded at 20.809s, but does not leave the relay until 269.196s.
The source-side delay therefore cannot all be attributed to preparing that order.

The independent audit is `notes/easyloop-layout-2026-09-12/audit_inventory_motion.py`
in the surrounding workspace. It reads the saved profile, adds each slice's
start time to tote transition offsets, joins the inventory tote to its carrying
HTM at pickup completion, and measures the actual movement interval before
boarding. Results are in the profile directory's `inventory-motion-audit.json`.

## Change

`Navigate` previously waited at its source while its destination had an
open-ended hold. Even when the holder's next navigation task already existed,
that task had no way to advertise the future departure for an incoming journey
to request. The scheduler waited for the physical departure time before planning
it, losing the opportunity to overlap travel toward the cell.

Navigation now uses the same exact-reservation departure advertisements already
used by `HtmStore`. Incoming work can request an earlier decision about an
existing departure. This changes `resolve_by`, while physical `start_time`
continues to respect readiness, admission, dwell and the settled simulation time.
Only the holder's committed route closes its source reservation. No additional
successor task is invented, and there is no assumption that requesting departure
makes the cell free.

Advertisements are removed on acceptance or cancellation. Same-cell navigation
and destination alternatives containing the source publish no departure promise.
Planned-departure requests, change notifications and handling of distant known
clearance work without an idle-fleet registry; that optional registry is still
used for parking requests.

## Validation

The new two-HTM regression failed before the change because both journeys were
planned too late. It now proves both routes can be committed before the holder's
future dwell ends, while the holder stays through its dwell and the incoming
HTM arrives only after actual clearance. Reservations do not overlap.

Further checks cover deadline versus movement time, advertisement cancellation,
no-op journeys, distant clearance and the existing return/parking interactions.
The library suite passed 962 tests (one ignored; four unrelated profiling
benchmarks excluded), plus six Cencora parity checks, the early conveyor
collection regression and all 24 fulfillment orchestration checks.

## First-hour full-map result

The frozen release build at `7c182c4fb` completed **860 single-unit picks in one
simulated hour**, compared with **595** in the first hour of the preceding
230-HTM control: **44.54% more picks**. Both use normal VTM timing, the uncapped
source workload, T0 eligibility and direct extraction. This measures the
scheduling change separately from the fleet-count change. It reaches 59.72% of
the four operators' single-unit ceiling, so a substantial gap remains.

The new first-hour profile is
`notes/easyloop-layout-2026-09-12/nav-overlap-230-1h/walmart_7c182c4.desprofile`
in the surrounding workspace. Its completion gate reports 860 picks/orders at
3,600,000 ms. HTMs average 77.03% busy; the high loaded-hold component still
makes that different from productive movement. Order 0's inventory now departs
at pickup completion (20.809s), rather than 269.196s in the control.

The intended eight-hour validation reached its 45-minute wall budget at
6.414 simulated hours, with **5,687 picks (886.71/hour)**. Its six complete
hours produced **879, 929, 882, 872, 883 and 920 picks**; the remaining partial
hour produced 322. This sustains the improvement through six hours, but does
not establish an eight-hour completion. Iteration 19 was launched for inspection
without automatic simulation startup.

The longer profile is `notes/easyloop-layout-2026-09-12/nav-overlap-230-8h/walmart_2a82e82.desprofile`.
Its filename reflects Git HEAD at export; the run metadata pins the frozen
binary to `7c182c4fb` (SHA-256 `4ed4d5bddbdc6823ec008c41b201909d73951ddb1ed10054492c2fc61a572b08`).
The one-hour run's 860 pick records exactly match the longer run's first 860;
the longer run additionally commits 19 picks before the one-hour boundary.
Compare complete interior hours when assessing sustained rate.

## Induction follow-up

`InductContainer::select` excludes approaching robots while a loaded HTM holds
the induction cell open-ended. Since no incoming `Navigate` exists yet, it
cannot request an already-advertised departure. The follow-up extends the request
to the selector's pending proposal, retaining exact source identity and
cancellation. An available empty resident is still selected for induction
without asking it to leave. Requests change planning deadlines only; physical
dwell, arrival, custody and fleet-headroom checks are preserved.

The new flow regression failed before this change and now proves overlap of an
incoming induction journey with the previous container's pickup dwell, while
requiring source clearance before entry. Additional checks cover cancellation
without claiming fleet/inventory and selection of an empty resident without
requesting departure. The library suite passes 965 tests, plus all 31 Cencora
parity and fulfillment integration checks. The full-map run then failed after
337 picks on a relay pickup from cell 12091. The destination was free, but the
selected HTM's sole exit had continuous finite reservations beyond the pickup
route's 300-second horizon. See [the selection correction](first-exit-selection.md).
This induction change is not included in either preceding navigation-overlap result.
It was subsequently backed out after the selection correction exposed another
full-map route failure; the selection report records both failures.

Relay retrieval has a similar access-holder gate. Broader requests for route
blockers should remain separate from these exact destination cases.

## Assigned work versus physically resident orders

The 860-pick run maintains roughly 42/43/42/42 logical assignments, including
upstream preparation. Physical admission remains six per side, twelve per
EasyLoop. Mean physical residents are only 4.48, 5.00, 4.72 and 4.60 across the
four machines; assigned work is not equivalent to ready order containers.
The 230-HTM fleet averages 77.03% busy and 22.97% idle, split into 28.58% travel,
46.31% loading/unloading (including loaded holds), and 2.14% processing/operator
wait. Follow-up comparisons must check resident supply as well as pick totals.

Over the longer 6.414-hour run, logical assignments average 41.99, 42.99, 42.00
and 42.00; physical residents average **4.38, 4.97, 4.90 and 4.65**. Only the
first machine ever reaches twelve residents, for 0.0123% of the measured period.
Fleet time is **83.59% busy / 16.41% idle**: 32.05% traveling, 49.32%
loading/unloading including loaded holds, and 2.22% processing/operator wait.
These measures come from timestamp-integrated order phases and robot duty
totals, not a final-frame count. `audit_order_occupancy.py` beside the saved runs
writes `order-occupancy-audit.json` for each comparison.

## Extraction turnover in the longer run

The three extraction ports have nine-second configured dwells. Mean intervals
between those dwells are 3.181, 3.025 and 3.138 seconds, giving about 891
containers/hour combined at the measured turnover. Their dwell utilization is
73.81%, 74.81% and 74.09%. All completed orders in this measured cohort have
one picked unit, so container and pick rates can be compared directly here.

The shortest observed gap is 2.846 seconds. Even sustaining that gap at every
port would yield about 912 containers/hour total. The dwell-only limit, with
zero clearance time, is 1,200 containers/hour across three ports, below the four
operators' 1,440 single-unit picks/hour limit. These are three-port aggregate
rates, not per-port rates.

The movement trace contains departures starting exactly at extraction dwell
completion. For example, one dwell at port 18 ends at 118.875 seconds and its
outbound travel starts then; the next extraction begins at 122.105 seconds.
This 3.230-second gap includes physical clearance and the next HTM's final
approach. The evidence does not justify treating the entire gap as a scheduler
delay. The nine-second dwell and three-port layout remain unchanged pending
workflow calibration.
