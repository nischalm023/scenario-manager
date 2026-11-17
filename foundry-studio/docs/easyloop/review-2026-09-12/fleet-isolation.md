# Fleet utilization and VTM isolation

September 12, 2026 · implementation checkpoint `1054bde36`

The matched eight-hour CONWIP run completed 4,981 single-unit picks, or
622.625 picks/hour across four EasyLoops. The preceding relay-corrected baseline
completed 4,468 (558.5/hour): an 11.48% improvement. The operator-only ceiling is
1,440 single-unit picks/hour; the current rate reaches 43.24% of that ceiling.
This is still substantially below the target.

The hourly totals are 593, 627, 629, 623, 624, 633, 632 and 620. This run does not
show a late collapse; it sustains an inadequate rate. It uses the complete source
order file with T0 eligibility and direct extraction, 10-second picks and
9-second extraction. The source remains uncapped in the scenario.

## HTM utilization is not productive-motion utilization

The 144-HTM fleet averages **70.71% busy and 29.29% idle** over the full horizon.
The busiest robot is 83.3% busy; the least busy is 59.9%.

| Recorded state | Share of total HTM fleet time |
|---|---:|
| Traveling, including in-route turns/holds | 21.589% |
| Loading/unloading, including loaded holds | 46.775% |
| Processing | 2.281% |
| Waiting for operator | 0.065% |
| Idle, the complement | 29.290% |

The independently recorded **wait-loaded overlay is 45.58%** of fleet time.
It overlaps the state rows and must not be added to them. `HtmPickup` deliberately
keeps a completed loaded pickup in `LoadingUnloading` until the next state
transition; this is not all mechanical fork work. Increasing robot count might
relieve shortages, but the headline busy percentage alone cannot establish that
the fleet is too small.

The inventory lifecycle records show each EasyLoop almost continuously at its
42/43/42/42 logical CONWIP ceiling. The per-machine mean counts are:

| EasyLoop | Before HTM loading | Loaded, before conveyor boarding | Boarded, before matching pick |
|---|---:|---:|---:|
| 0 | 22.22 | 16.96 | 2.82 |
| 1 | 21.89 | 19.21 | 1.90 |
| 2 | 22.80 | 17.31 | 1.89 |
| 3 | 22.66 | 16.67 | 2.67 |

These are timestamp-integrated counts, including unfinished visits clipped at
the eight-hour boundary. The last column excludes post-pick return/collection
occupancy and therefore is not total conveyor occupancy. More released work is
not automatically more ready work at the operator.

## VTM utilization

Across all 48 configured VTMs, mean busy time is **22.21%**, with **77.79% idle**.
The eight buffer/depot VTMs receive no cycles in this direct-extraction run.
Restricting the denominator to the 40 VTMs that do work gives **26.66% busy**.
The busiest individual VTM is 57.7% busy. These figures do not demonstrate
fleet-wide VTM saturation, but local availability and retrieval latency can still
delay matching inventory and order containers.

The existing `vtm.isolation=teleport` diagnostic replaces each scheduled VTM
cycle with approximately 1 ms and removes the per-VTM capacity frontier. It keeps
the rack/relay custody transitions. Its VTM geometric traces are hypothetical;
their displayed duty is not actual constrained VTM utilization.

## Which HTM count do the documents specify?

The supplied **Walmart Dallas — Simulation Design Specs** specifies:

- Flow 1, Putaway + Pick: **144 HTMs**.
- Flow 2, Pick Only: **80 HTMs**.
- Flow 3, Putaway + Pick + Order Extraction: **144 HTMs**.

The newer **DFW6 Pilot Design of operations DoO Draft 2.0**, §4.3, lists
**230 HTMs and 72 VTMs**, but labels the bill of materials WIP and says it is not
a released procurement baseline. Daniel subsequently directed the standard Walmart scenario to use **230 active
HTMs** on September 12. The map cohort and its loader checks now use that count. The [questions document](../questions.html#htm-fleet-confirmation)
now asks which count governs and whether it includes charging/spare robots.

## Controlled experiments

All three cases use the same frozen release binary, map, inventory, full workload,
seed, HTM motion, operator service and extraction behavior. Their logical inventory
limits are checked in the startup log. Each experiment changes one setting.

| Case | HTMs | VTM service | Eight-hour result |
|---|---:|---|---|
| Baseline | 144 | Normal | 4,981 picks; 622.625/hour |
| VTM isolation | 144 | Approximately 1 ms; capacity ignored | 5,083 picks; 635.375/hour (+2.05%) |
| Walmart default | 230 | Normal | 4,811 picks; 601.375/hour (-3.41%) |

The isolation case tests the combined timing/capacity cost of VTM service; it is
not a realistic replacement model. Removing that cost improved the single-seed
result by only 102 picks, so normal VTM service was not the dominant throughput
limit in this comparison. The 230-HTM result is also one seed, and its hourly
totals were **595, 635, 638, 631, 632, 614, 576 and 490**. That late decline means
the run must not be described as stable. Daniel approved 230 HTMs as the Walmart
default; this controlled result says only that adding robots did not improve this
workload under the current ownership and queueing behavior.

## The larger fleet moved waiting downstream

The 230-HTM run averaged **78.5774% busy and 21.4226% idle**. Its mutually
exclusive recorded states account for fleet time as follows:

| Recorded state | Share of total HTM fleet time |
|---|---:|
| Traveling | 13.28994% |
| Loading/unloading | 63.87805% |
| Processing | 1.37956% |
| Waiting for operator | 0.02988% |
| Idle, the complement | 21.42257% |

The inventory phase audit shows what that higher busy percentage represents.
Released-to-loaded time fell to **46.65–49.76 seconds** across the four
EasyLoops, while loaded-to-boarded time rose to **843.39–883.01 seconds**.
Mean loaded-before-boarding inventory grew to **36.79–37.83 visits per
EasyLoop**, compared with **16.67–19.21** in the 144-HTM baseline. The logical
CONWIP ceilings remained 42/43/42/42 in both cases.

This is an ownership and queueing shift: more retrievals obtained totes and
then held loaded HTMs while waiting to board. It is not evidence that useful
work increased. Throughput instead fell by 170 picks over eight hours. The
next diagnosis should therefore explain the loaded-to-boarded interval before
using fleet busy percentage or rapid loading as evidence for more HTM capacity.

Logs and launch provenance are under the surrounding workspace’s
`notes/easyloop-layout-2026-09-12/`: `conwip-control-8h`,
`conwip-vtm-isolation-8h`, and `conwip-230-htm-8h`. The baseline profile is
`conwip-control-8h/walmart_1054bde.desprofile`. Each directory's
`summary-audit.json` and `pipeline-audit.json` was independently decoded from
its saved profile. These are controlled single-seed results from the frozen
`1054bde36` behavior; the finalized comparison profiles were recorded at
`8df10ea` after the fleet/default documentation update.
