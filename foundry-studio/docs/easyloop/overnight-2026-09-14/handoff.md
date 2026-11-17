# Overnight Walmart matrix — September 14

**Report complete as of 05:20 PDT.** All 12 runnable experiments finished and
passed the flow, inventory, extraction, buffer and motion audits. Four expected
stock-preflight rejections and two no-inbound equivalents complete the 18-row
matrix. No simulation or analysis jobs remain. See the final section below;
earlier progress snapshots are historical.

Daniel requested a morning HTML report with the full discussed workload matrix,
pick and putaway throughput, source-file timing/stock observations, congestion
findings and simulator limitations. He permits up to four concurrent headless
desvis runs conditional on RAM and at most two headed instances. Do not open
windows, disable collision avoidance, or contact branch coordination.

Worktree `/Users/daniel/vt/greyorange/walmart`, branch `walmart`. Existing draft
PR 468 stays draft. WIP checkpoints and pushes to origin/walmart and
origin/agent/walmart-map-corrections are authorized.

## Files and execution

- Matrix artifacts: `/Users/daniel/vt/greyorange/notes/walmart-matrix-2026-09-14`.
- `manifest.json` freezes source/binary/input hashes and all case specifications.
- Each case has `state.json`, `run.log`, a profile on completion, and audits.
- `scripts/walmart_matrix/run.py --prepare` creates the matrix exactly once.
- `--supervise` manages a queue with a four-headless process ceiling including
  unrelated desvis runs. A file lock prevents duplicate supervisors. Check
  existing running case PIDs before recovery; do not launch duplicates.
- Report destination: `docs/easyloop/overnight-2026-09-14/report.html`.
- Hourly heartbeat `finish-walmart-overnight-throughput-report` should be paused
  once the final report is delivered. Do not stop an actively working parent turn.

## Matrix meaning

Full / partial / empty preload × putaway off/on × source timing / all orders T0.
Relevant inbound cases additionally compare inherited `sku_demand` sequencing
with new `order_release` sequencing. Two source-timed cases test a synthetic
one-hour inbound lead; this is an assumption, never a customer timestamp.

Common comparison horizon is 16 simulated hours. Full-preload/source timing and
empty/order-aligned/source timing extend to 52 hours to cover the original
approximately 47-hour arrival span. Use each recorded prefix for the common
comparison, and show extended results separately. Never compare a partial run's
whole-run mean with a complete run's mean as if the horizons matched.

Full preload consumes the finite supplied on-hand stock, so enabling putaway
creates zero inbound lots. Those rows are explicitly equivalent/no inbound,
not evidence of simultaneous putaway capacity. Empty/off cannot fulfill orders;
partial/off may fail the stock preflight. Preserve these outcomes and do not
waive the check to manufacture a passing cell in the matrix.

## Fixed controls and caveats

230 HTMs, 40 inventory + 8 buffer VTMs, 128 diagnostic reusable trays (the site
count is unconfirmed; repo main uses 24), queue factor 3, four EasyLoops with one
operator each, 10 seconds per paired transfer, six decant stations at provisional
60 fills/hour/station with at most six units/lot. Ordinary box induction and
extraction, DS buffer hold one hour; SC direct extraction remains provisional.
No charging or floor-driving VTMs yet. Boxes disappear at sealer handoff by the
user's temporary simplification. Multi-unit service timing is not yet calibrated:
do not treat 1,440 units/hour as a universal ceiling for multi-unit transfers.

`onhand.csv` is a snapshot with on_hand_qty and no receiving/induction times;
`outbound.csv` includes order_drop_time and a time-of-day cutoff_time. No real
inbound schedule is supplied. Stock totals include the explicitly approved
43-unit top-up. Partial preload rounds per SKU upward and uses half the eligible
tote positions; it is not exactly half of all units. Empty start still contains
empty totes in racks. New order-priority induction respects source release order
(or order-ID priority under T0), consumes preload/queued stock credits once, and
preserves exact inbound totals. It is static sequencing, not a dynamic inbound
optimization algorithm. Source order timestamps remain unchanged in the files.

## Validation and reporting contract

New sequencing tests (3) and the shared putaway integration test (both policies,
partial/empty) passed; release build passed. Simulation source checkpoint dd6ef9096. The frozen binary hash is in the
manifest; profile Git metadata is obtained at runtime and is not a substitute
for that hash. Do not relabel a run based on later report or UI-only commits.

Use physical completion timestamps through each horizon. Separate inducted
units, filled totes, and completed filled-tote rack stores. Validate stock
conservation, buffer slot occupancy and canonical movement vs playback. The
Python flow/buffer audits exist in scripts/; the streaming traffic exporter is
now `target/release/traffic_report` (promoted from an example to a binary;
source in `sim/dessim/src/bin/traffic_report.rs`). Heatmap waiting includes work/resource waiting; excludes queues,
presentation/access, exchange and charging cells. It is NOT a pure congestion
measure. Speed uses canonical movement, duration-weighted bins and link halves.
Show cell IDs/locations, exposure and normalization; do not infer causal traffic
interference from fleet idle percentages or a single whole-run histogram.

The prior empty start (iteration36) produced 28 picks in8h due to mismatched
synthetic SKU priority, not a measured layout capacity. Prior full-preload T0
iteration32 produced 9,549 picks/8h, peak full hour 1,246. Partial iteration35
finished with 9,651 picks/8h (1,206.375/h), 10,719 inducted units, 2,058 fills
and 2,052 rack stores. All audits passed. HTM idle 27.4007%, VTM 40.1633%.
Different preload placement prevents attributing the entire difference to
putaway alone. Older source-timed seven-hour 144-HTM run peaked 523 picks/h;
it is historical, not current capacity evidence.

## Live handoff at 23:55 PDT

- Supervisor PID 46530; `supervisor.json` records authoritative current children,
  memory and queue. Max four active headless; currently zero headed.
- Analyzer PID 73905 (restarted from 60523 to use integrated duty counters rather
  than instantaneous state snapshots). `analysis-status.json` and analysis.log
  show health. It updates the HTML every 30 seconds and audits finished cases.
- `caffeinate -i -w 46530` PID 65934 prevents idle system sleep only while the
  supervisor runs. Display sleep is unaffected.
- Launch RAM gate: conservative reclaimable headroom >=22 GiB, compressor<12 GiB.
  Check runtime pressure too; this gate does not evict already-active jobs.
- First matrix case finished: iteration39, empty/on/T0/order-priority, 16h,
  exit0: 3,660 picks (228.75/h), peak full hour239, best rolling60min247;
  12,499 inducted units, 3,514 filled totes, 3,507 rack stores. Stock conservation
  0 + 12,499 - 3,660 = 8,839; flow/buffer/motion audits passed. HTM idle61.9329%,
  VTM86.5884%. Buffer mean148.03 slots, peak176/3450, all three occupancy
  integrations agree. This workload is supply-limited, not saturated pick-capacity
  evidence. Judge the matched partial/full and original-priority cases when ready.
- Heatmap verification discovered that declared order waiting berths have Open
  traversal kind. They were leaking into the travel-wait color scale. UI metadata
  and exporter now exclude their semantic equipment membership plus inventory
  holding cells; six traffic UI tests including a real-map regression pass.
  This does NOT change the frozen simulation binary or run physics. Old headed
  builds have the older color-scale exclusion; do not claim they are fixed.
- Report idle uses per-slice integrated busy durations with the full roster
  denominator. Aggregate state-count snapshots do not reconstruct a time-weighted
  duty percentage. Empty case integrated duty differs from final summary by only
  0.000254 percentage points HTM and zero VTM. Don't use the discarded snapshot
  calculation as a VTM bottleneck diagnosis.
- Cold-start bound: 21,161 distinct demanded SKUs require at least21,182 fills at
  six units/single-SKU lot; six stations *60 fills/hour =>58.8 ideal hours to
  supply all requested units. Whole-stock minimum329,814 fills=>916.2 ideal hours.
  Conditional on current synthetic one-SKU/fill model; mixed-SKU inventory is
  supported but mixed-SKU decant fills are not generated. Not a peak-rate forecast.
- HTML QA: `scripts/walmart_matrix/check_html.cjs` uses separate headless Chromium,
  never foreground/user tabs. Execute with bundled Node:
  `/Users/daniel/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node`.
  Browser launch needs normal execution escalation. Screenshots and
  `html-validation.json` go to the artifact base. Desktop1440/mobile390 widths,
  18 matrix rows and five SVGs passed at23:55; screenshots inspected. Rerun
  after all final data. Corrected empty-case ordinary travel waiting is
  5,446.367 HTM-seconds (1.513 HTM-hours); moving mean2.86m/s, 18.9% below1m/s.
  Largest hotspot is an apron cell c862:261.416s over16h (0.454% of window).
  This low-load case does not establish saturated layout capacity.
- Supervisor source now also gates launch when more than2 headed processes
  exist; the current running supervisor predates this tiny guard. There have
  been zero headed processes throughout, and its headless/RAM gates are active.
  Do not restart it just to load this source change; avoid orphan/duplicate runs.

## Follow-up at 00:18 PDT

No new completed cases yet; four headless, zero headed, roughly24GiB conservative
headroom. No incremental swapouts. Supervisor/analyzer remain healthy.

Read-only policy review found an important mismatch to assess once iteration42
finishes. `ClaimNextOrder::propose` in flows/fulfillment/pending_orders.rs ranks
by `(max(current availability/barrier, order release), OrderId)`. Already-released
backlog therefore ranks by ID; it does not retain release-time priority. The new
synthetic `order_release` induction sorts once by `(source release, OrderId)`.
Thus T0 priorities align, but timed backlog can diverge. Report now states this
explicitly. Do NOT silently change the frozen experiments to repair it. Check
iteration42 order phases, missing-stock waits and inducted SKU sequence before
attributing low picks to this hypothesis or to layout congestion. A future
matched diagnostic could dynamically prioritize assigned shortages or retain
FIFO order-release priority, but it must be a separate controlled variation.

## Follow-up at 01:28 PDT

- Iteration42 (empty/source/order_release) finished52h:1,696picks overall32.615/h;
  first16h1,450picks90.625/h; peak full hour293 atT9–10. Inducted38,478units,
  11,113fills,11,105rackstores. Whole HTM idle70.7223%, VTM93.8113%.
- Flow audit exit1 is now localized: it includes `start == horizon`, a zero-width
  bucket. A six-unit fill completes at187206977ms,6.977s beyond52h. That terminal
  bucket contains6units/1fill and inventory36788; the preceding complete bucket
  reports36782, which equals38478inducted−1696picked. Summary/journal are correct.
  The audit currently treats the raw terminal bucket as observed. Preserve the
  discrepancy and explain it; do not infer physical stock fabrication or claim
  UI visual validation from it. Exact-boundary events must be handled deliberately
  if fixing the auditor, because completions_through includes events exactly atT.
- Buffer audit and canonical-motion audit pass. Buffer mean27.33slots, peak296
  of3450; residence3618.56s; final10stored. Ordinary traffic remains low-load.
- New `scripts/walmart_matrix/backlog.py` reconstructs unreserved stock from the
  frozen source-ID mapping, timestamped fills and PreparationStarted allocations.
  It passed nonnegative-stock checks for empty T0 and timed runs. Artifacts are
  `backlog.json`; it is now called by the analysis worker for new cases.
- Timed-run evidence at52h:169assigned-but-unprepared orders, all169need a SKU
  never inducted; medianassignmentwait22.2075h. Meanwhile20,354released orders
  unassigned, of which11,079are individuallystock-ready (not jointlyguaranteed).
  At16h169waiters/167never-inductedSKU,2,090readyunassigned; at10h156waiters,
  828readyunassigned. This directly demonstrates assignment/induction blocking.
  Preserve baseline; do not silently rewrite the frozen policy experiment.
- Analyzer restarted while idle to PID41234 for backlog integration. Supervisor
  remains46530. Four headless/zero headed; last memory~26.8GiB available,
  compressor6.5GiB, no incremental swapouts. Report renders background-only.

## Follow-up at 02:40 PDT

- Nine recordings analyzed; all flow, buffer and traffic audits now pass.
  Four expected stock-preflight rejections and two no-inbound aliases remain
  explicitly distinct. Three partial-preload sensitivities are still running:
  `partial-on-t0-sku_demand` PID74389,
  `partial-on-source-sku_demand` PID76444, and
  `partial-on-source-order_release-lead1h` PID88642. No queued or headed runs.
- Supervisor remains46530. Analyzer was restarted while idle to **88564** to
  load the added station occupancy breakdown. One-off refresh worker88565 has
  finished. Do not act on the older analyzer PIDs above.
- Last conservative memory headroom47.7GiB, compressor8.35GiB; swapouts unchanged
  at7310421. High-volume runs sampled individual RSS peaks around18–21GiB near
  export; report now includes wall time and sampled peak RSS per case.
- **The cutoff-audit issue described above is fixed in the auditor.** It retains
  raw bucket counts/inventory, subtracts only journal-confirmed future events
  within the final bucket, and includes events exactly at the cutoff. This also
  handles partially elapsed buckets. No simulation or recorded event was changed.
  Ten Python tests pass, including exact-time completions, future fills/stores/
  extractions, zero-time and a corrupted-count case that must still fail. The
  flow audit now independently checks extraction counts and units too. Old audit
  JSON is archived as `flow-audit-before-cutoff-fix.json`; current audits all pass.
  Do not repeat the old claim that a discrepancy still remains.
- Full/T0/no-putaway16h: **19,922 picks,1245.125/h**, peak1415. Transfer tasks
  19,174=1198.375/h (83.22% of the1440tasks/h service bound). HTM idle36.10%,
  VTM49.76%. Buffermean675.38,peak892/3450. Travel wait73.666HTM-hours.
- Partial/T0/order-priority putaway16h: **19,993 picks,1249.5625/h**, peak1391;
  **20,939 inducted units=1308.6875/h**,4093fills,4087stores=255.4375/h.
  Transfers19,372=1210.75/h (84.08% of service bound). HTM idle29.46%,VTM44.32%.
  Buffermean673.38,peak881. Travel wait74.621HTM-hours. Different preload/placement
  prevents attributing the +0.36% pick difference solely to putaway.
- High-load travel moving means2.585 and2.574m/s, about23.7% of moving time below
  1m/s and33% at4–4.5m/s. Dominant apron hotspots are c874 andc882 at
  (47.33,13.17)m and(54.15,13.17)m. Partial c874 waited22,444s (6.23HTM-hours,
  39.0% of16h). These are explicit waits but reasons can include station admission;
  do not label them all traffic interference. No general fleet-wide collapse in
  this comparison; physical floor-driving VTM traffic remains unmodeled.
- Inventory pipeline occupancy~42–43 per station includes permits awaiting
  consumer permission to start retrieval. It is NOT loaded HTM count. Added
  separate means: about3 loaded inventory HTMs awaiting boarding and4.3 inventory
  totes on conveyor before picking; mean order residents~7 of12 physical capacity.
  Completed loaded→boarding means36–38s; finalpick→exit~4s. HTML labels clarify.
- **Full preload/source timing completed all22,160 order extractions and25,625
  picked/extracted units at47.317114h**, before its52h limit. Whole mean541.559/h;
  peak full hour1290 atT20–21. First16h10,018picks=626.125/h,peak1286. All audits
  pass, bufferpeak994/3450 and no containers remain stored. It is a completed
  workload, not a52h observation. Charts now omit incomplete hourly/bin exposure.
- Partial/source/order-priority16h:9988picks=624.25/h,peak1279;
  19,546inductedunits,3771fills,3764stores. HTM idle34.17%,VTM67.88%.
- Empty/T0/legacySKU policy16h:32picks=2/h,peak21,19,128inductedunits.
  Empty/source/legacy16h:36picks=2.25/h,peak6,19,039inductedunits. Both show
  assigned missing-SKU orders while many stock-ready orders stay unassigned.
- Empty/source/order-priority withsynthetic1hlead16h:1237picks=77.3125/h,
  peak370 atT6–7,9363inductedunits. At16h169assigned
  waiters all neednever-inductedSKUs;1526stock-readyunassigned;medianwait4.327h.
  This is worse sustained throughput than no-eligibility-gate control90.625/h,
  despite a higher peak. Do not call the peak improvement a sustained gain.
- Source log preload:full1,397,632units/162,060stockedtotes=8.62units/tote;
  partial747,243/81,030=9.22;174,020source storage positions.100units/tote is
  the packing limit, not each tote's actual contents. `source-audit.json` records
  these parsed and stock-checked initialization facts.
- The128configuredtrays are specifically SIOC trays; ordinary bagging order totes
  remain a separate seeded pool. This does not cap total reusable containers at128.
  `FillInventory` retypes128existingemptyordercontainers toTray; it does not
  create128additionalcontainers. Maincomposition uses24trays. Site count remains
  unvalidated.
- HTML now includes measured comparative congestion, completed-transfer capacity,
  physical-vs-logicalWIP distinction, source-timed completion, lead sensitivities,
  execution/RSS evidence and recommended next controlled experiments. Background
  QA at02:39:18matrixrows,23SVGs,noJavaScript errors,no desktop/mobile overflow;
  desktop/mobile screenshots inspected. Two policy plots were added afterward;
  rerun final QA after the remaining results land.

Next: inspect the three remaining partial cases and compare SKU/lead policies;
check the final report's claims against all audits, refresh final QA, update and
push the final HTML. The hourly heartbeat remains ACTIVE until final delivery.

## Follow-up at 03:21 PDT

No new completed cases yet; the same three partial sensitivities are advancing.
T0/legacy is around6.5h; source/legacy around14.3h; source/lead around13.8h.
Ignore a transient very long ETA on the lead case: subsequent samples confirmed
advancing simtime and picks. Supervisor46530/analyzer88564 healthy; three
headless, zero headed;41.3GiB headroom,8.0GiB compressor, no new swapouts.

Report generator has an uncommitted explanatory addition: the actual CONWIP
formula and a stage-specific Little's-law comparison (3.07 loaded HTMs /
301.1boardings/h ≈36.7s, agreeing with36.6s measured completed-visit delay).
It distinguishes logical pipeline credits from loaded HTMs. Include this in
the next results checkpoint. Last pushed checkpoint remains340b7e4eb.

## Follow-up at 04:26 PDT

Eleven recordings now pass all three audits. Only partial/T0/legacy SKU induction
remains (PID74389), around14h of16h and advancing. No queued/headed processes;
supervisor46530/analyzer88564 healthy. Headroom52GiB, compressor7.65GiB, no new
swapouts. Frozen binary, four inputs and all18specification hashes were verified;
see `integrity-validation.json`.

New matched source-timed partial results:
- Legacy SKU:9902picks/16h=618.875/h,peak1272;19,130inductedunits,3765fills,
  3759stores;HTM idle33.95%,VTM67.99%;travelwait27.884HTM-hours. At16h46assigned
  orders remain unprepared,43lack sufficient unreservedstock,medianwait2.73h;
  66stock-readyorders are unassigned. Similar headline rate hides order starvation.
- Order-priority with1hlead:9858picks=616.125/h,peak1286;only139inductedunits,
  108fills/102stores;HTM idle46.60%,VTM72.90%;travelwait28.443HTM-hours. Noassigned
  stockwaiters at16h. Priority covers only shortages left after preload. Bulk
  surplus remains behind future priority lots in the FIFO queue. This is a
  very light inbound workload, not evidence of simultaneous full-rate putaway.
- T0-eligible order-priority control:624.25picks/h,19,546inductedunits.
  All three pick means within1.3%, with very different replenishment workloads.

Report now explains these comparisons and will add the final T0/legacy comparison
automatically when its facts arrive. Source inventory balance explicitly excludes
units already transferred to order containers. Background HTML QA at04:22 passed:
18rows,29SVGs,noJSerrors/no desktop/mobile overflow. Final QA still needed after
the last result.

Sleep guards: original65934 waits for supervisor46530. **69858** waits for analyzer
88564, so auditing can finish after the simulation supervisor exits. **72685** is
a bounded90-minute delivery guard started04:26 (expires~05:56). Its record is
`delivery-sleep-guard.json`. Stop it after final report delivery, verifying its
PID command is `/usr/bin/caffeinate -i -t 5400` first. No display sleep is blocked.

At next heartbeat, finish/check/push the last result and final report, stop the
delivery guard, and pause the hourly automation. Do not leave it running after
delivery. No further simulations are needed for this matrix if the last case
completes its16h horizon.

## Final result at 05:20 PDT

The final partial/T0/demand-ranked case finished its full 16h:
19,849 picks (1,240.5625/hour), peak full hour 1,386;
19,979 inducted units, 4,102 fills and 4,095 rack stores.
HTM idle 28.85%, VTM idle 44.40%; travel-cell waiting 72.885 HTM-hours,
moving mean 2.569m/s. Buffer mean 673.90 slots, peak 870, with 682 still stored
at the boundary. Twelve assigned orders were still unprepared; nine lacked
sufficient unreserved stock, while 5,550 unassigned orders were individually
stock-ready. All audits pass. These waits remain a policy issue despite a pick
rate close to the 1,249.6/hour order-priority control.

Final deliverable: `docs/easyloop/overnight-2026-09-14/report.html` (self-contained,
about 2.6MB). All 18 matrix rows and 48 numerical result cells were checked
against the first-16h profile facts. Desktop/mobile HTML checks passed with
31 SVGs, no JavaScript errors and no page overflow; screenshots were inspected.
Audit and integrity evidence is in `final-validation.json`,
`html-validation.json` and `integrity-validation.json` under the artifact base.

The full-preload/source-timed run completed all 22,160 orders and extracted all
25,625 units at 47.317h. All other measured cases cover 16h except the deliberately
extended empty/order-priority/source-timed run, which covers 52h and exposes
severe inventory/assignment starvation. No timed-out run is treated as a full
16h comparison. Normal box induction/extraction and the synthetic one-hour DS
buffer hold stayed enabled. The frozen simulation executable was not changed.

Next simulation work, outside this completed reporting task: fix stock-aware
assignment/induction coordination, validate receiving/decant assumptions, and
attribute operator idle time and apron waits before tuning release limits.
The report distinguishes logical preparation credits from loaded HTMs and
documents the conditional cold-start capacity bounds. A rebuilt headed app still
needs visual acceptance of the corrected waiting-cell heatmap exclusion; no
foreground windows were opened overnight.

The overnight heartbeat should be retired now, and its bounded delivery sleep
guard stopped after the final checkpoint is pushed and the report delivered.
