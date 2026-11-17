# Walmart/EasyLoop call transcript review

Source: `Walmart DES 2026-09-12 12_16(GMT-7_00).txt`, read in full (4,667 lines). Daniel's preserved user message is authoritative where it corrects or reprioritizes the call. This report records transcript evidence; it does not infer current implementation state.

## Current truths and priority

1. **Model the EasyLoop operator and pick correctly.** Each station has one exclusive operator. A pick takes 10 seconds and can begin only when the matching order container and inventory tote are both presented on the selected left/right side. Only one side can be picked at once. A ready pair on the other side may wait, but its pick clock has not started. The containers remain presented through completion. This is a pick task, not a fixed per-container dwell. [13:17:09–13:21:47, lines 2731–2975; 13:22:33–13:24:58, lines 3016–3119]
2. **Keep the station fed.** For the current model, define `W` as the number of waiting cells and cap concurrent station order assignments at `W`; with six waiting cells, assignment count should remain exactly six until the global order supply is exhausted. Record a time series of assigned-order count per station so any starvation interval is visible. [13:26:19–13:30:05, lines 3172–3326]
3. **Orchestrate two differently constrained streams.** Inventory totes are effectively serial/FIFO on the main conveyor, with only the two reversible APP branches providing a small final routing choice. Order HTMs in the independently navigable waiting block can be resequenced; their travel distances to presentation differ by waiting position. Therefore the inventory sequence is the controlling sequence and order-container selection/timing should follow it. [13:25:02–13:26:07, lines 3121–3161; 13:31:59–13:36:54, lines 3424–3659]
4. **Temporarily route completed orders directly to extraction.** Preserve the real EasyLoop internal flow; do not make containers evaporate after ten seconds. Bypass the order buffer and drive every completed order container directly to the extraction station to remove buffer capacity and delayed-extraction timing as experimental variables. Daniel's user note promotes this from the call's “interim setting” to a requested immediate change. [13:12:35–13:16:28, lines 2548–2687; 13:37:03–13:38:07, lines 3667–3713]
5. **Fix relay/diving-board geometry and motion.** The access cell is underneath the diving board, rather than in front of it. The relay/presentation cell has one-sided entry/egress: the HTM enters, drops or collects, and backs out. It cannot traverse north/south or rotate/turn there. The adjacent under-rack travel cell remains longitudinally traversable. Daniel's user note is the authoritative clarification where the live screen discussion had uncertain naming. [12:27:44–12:32:06, lines 487–728]
6. **Defer floor reorganization and floor-driving VTMs.** The call explored open-floor/highway redesign and aisle-pinned VTMs, but Daniel's later note explicitly says existing open-floor HTM traversability is acceptable for now; layout work is later this week, and floor-driving VTM types later next week. The incomplete VTM sentence in the note must not be completed by inference. The transcript's short-term aisle pinning rationale remains useful future design evidence. [13:07:57–13:11:33, lines 2368–2480; 13:38:08–13:52:23, lines 3718–4397]

## EasyLoop workflow specification

- An order is assigned to a station; its order tote or box is treated generically as an **order container**. The container is inbounded on an HTM and queued in the waiting block if needed. Required inventory totes are inbounded onto the station conveyor. The order container and each inventory tote are paired for presentation. [13:14:37–13:16:09, lines 2620–2672]
- When a matching pair is available at an APP and the operator is free, a 10-second pick task starts. Completion represents the operator moving the unit and confirming completion; explicit button mechanics need not be modeled. Both containers stay presented until completion. [13:17:40–13:20:41, lines 2753–2933]
- If another inventory tote for the same order can follow immediately, the order can remain in the presentation flow; otherwise its HTM cycles back through the waiting block to clear the APP for other work. [13:15:23–13:16:22, lines 2656–2678]
- The two sides are meant to hide presentation motion behind operator work: while one side is being picked, the other can enter, leave, or wait fully ready. The theoretical station ceiling is therefore 360 picks/hour, subject to keeping one side ready whenever the operator becomes free. [13:21:24–13:24:58, lines 2965–3119]
- Vinay described production orchestration tentatively: summon order containers in sequence, estimate their dispatch/arrival cadence, then call inventory totes to match. Daniel questioned delaying inventory dispatch. The settled invariant is coordinated readiness, not that exact dispatch algorithm. [13:25:02–13:26:18, lines 3121–3169]

### Queue and saturation semantics

- Current physical/model facts supplied in the task context: six waiting positions per station and inventory-conveyor capacity eight. Do not conflate the two capacities.
- The call noticed that the reference diagram may show **two waiting blocks**, not one. Daniel deliberately deferred validating the physical waiting-block layout and abstracted it as `W`. This needs documentation/wiki reconciliation before using the diagram to change geometry. [13:26:37–13:27:40, lines 3181–3218]
- `W=6` is a present simplification, not a proved optimum. Vinay initially answered `W+2`; Daniel chose `W` to avoid “hot racking” waiting slots. Daniel's user note reopens whether more than W orders should be assigned/dispatched to cover travel latency, perhaps with a CONWIP/admission split. [13:27:58–13:29:24, lines 3238–3296]
- Separate three counts in metrics/design: orders **assigned** to a station, order HTMs **dispatched/in flight**, and HTMs **admitted to/occupying** the waiting/presentation subsystem. The transcript only settles assigned=`W` for the current simplified experiment. It does not settle whether dispatched may exceed W.
- Starvation has at least two causes: insufficient system vehicles/work supply, and poor synchronization of serial inventory arrivals with reorderable order HTMs. [13:31:29–13:32:35, lines 3394–3452]

### Inventory ordering versus order resequencing

- Treat the main inventory conveyor as serial/FIFO for scheduling purposes. At the end, a tote can be directed to either of two active presentation points through reversible mini-conveyors; that local choice does not make the upstream sequence generally reorderable. [13:34:14–13:36:54, lines 3538–3659]
- Order HTMs can enter/exit the waiting block independently and thus can be selected in the order needed to marry with the next inventory tote. Their release time must account for differing travel distance from each waiting position. Daniel suggested exact timing might safely fall out of cell occupancy once sequence is correct; this is tentative and should be tested rather than embedded as a requirement. [13:32:38–13:34:10, lines 3457–3530]
- The transcript does not specify tie-breaking, lookahead depth, handling of an inventory tote whose order HTM is delayed, or whether a tote may circulate/retry. These are open scheduler decisions.

## Relay/drop/collect behavior

- A relay/diving-board presentation cell is a dead-end service location. An HTM enters from its access/travel cell, performs a drop **or collection**, then reverses out along the entry edge. No north/south through movement and no in-cell rotation are allowed. Daniel's post-call note is authoritative on collection and no rotation; the transcript explicitly discusses drop/back-out and restricted traversal. [12:28:25–12:31:24, lines 523–683]
- The cell beneath the diving board is the access/travel cell and may support longitudinal under-rack travel. The call's labels were repeatedly reversed while inspecting the screen, so names alone are unsafe; use physical relation and traversability. [12:28:38–12:32:06, lines 535–728]
- Whether paired under-rack travel lanes should be one-way or bidirectional was deliberately left as an experiment: both designs were considered valid, with throughput deciding. In the one-way design, an HTM may have to back two cells before joining the desired direction. This is distinct from the settled no-through/no-rotation rule inside the relay cell. [12:35:40–12:41:28, lines 934–1166]

## Throughput definitions and targets

- **Station theoretical ceiling:** one pick/10 seconds = 360 picks/hour/station. With four stations, the ideal ceiling is 1,440 picks/hour. The task context says the current configured pick is 9 seconds, yielding 400/station/hour or 1,600/hour total; that is inconsistent with the authoritative 10-second workflow and should be corrected before comparing runs.
- **Observed baseline supplied in task context:** iteration 10 produced 5,397 picks in eight hours, about 675 picks/hour total. Preserve both total and normalized rate.
- Early in the call, numbers were muddled: Daniel recalled 1,400, Vinay mentioned “300, 1200” and “290 presentations,” and speech recognition rendered “49.” Later arithmetic explicitly uses 360 picks/hour/station and “reach 1400,” which is the coherent four-station interpretation. Treat 1,200/290 as target/expected performance possibilities, not the theoretical cap. [12:23:18–12:24:48, lines 250–347; 13:30:28–13:31:09, lines 3355–3374]
- Vinay cautioned that the dense field may prevent reaching the ceiling and said not to force 1,400. The Monday deliverable was reframed as maximum possible throughput for pick-only, put-away-disabled, full-horizon SIP, with windowed SIP likely later. [12:24:27–12:24:51, lines 322–347; 13:12:18–13:12:42, lines 2530–2555; 13:52:32–13:53:14, lines 4408–4436]
- Report at minimum: completed picks, picks/hour (whole run and steady interval if defined), per-station picks/hour, operator busy/idle fraction and idle reason, assigned-order count over time, readiness gaps at each APP, and inventory/order arrival mismatch. “Presentation” must not be counted as throughput unless it maps one-to-one to completed picks.

The multi-unit case remains unresolved: the call uses pick and presentation inconsistently, and current TransferItems accepts a quantity with one service duration. Confirm whether the new 10 seconds applies per unit or to the whole matched transfer before interpreting the operator ceiling as units/hour.

## Heatmaps and traffic diagnostics

### Settled first views

1. **Cumulative waiting duration per cell** over the whole run. Exclude queue/waiting cells and all presentation/access cells because their intentional dwell would dominate the color scale. Hover must show the cell's numeric duration and units. [12:42:19–12:43:19, lines 1210–1253]
2. **HTM speed per cell** over the whole run, selectable in the UI, with the numeric speed and units on hover. Time slicing is desirable later. [12:43:11–12:44:04, lines 1252–1286]

### Statistic is unresolved

The call moved from average to median, then considered standard deviation because a healthy cell may be bimodal: fast through traffic and slow turning/crossing traffic can be time-separated without interfering. A single mean or median can hide that distinction. Turning-only and turning-plus-waiting layers were also floated, but explicitly remained uncertain. [12:44:07–12:48:33, lines 1291–1487]

Recommended decision for planning: store enough event-level aggregates to expose count, mean, median (or quantiles), variance, stopped time, turning time, and straight-through time; initially render a simple selectable statistic. Do not claim standard deviation alone proves healthy wave separation. The diagnostic purpose is to localize a hotspot and explain why two map variants differ, rather than rely only on global throughput. [12:48:33–12:50:58, lines 1489–1607]

## Contradictions, tentative claims, and open decisions

- **Operator implementation:** the transcript establishes required behavior, but Daniel said he needed to check current modeling. User suspicion that the resource is missing is not evidence of code state; await code inspection. [13:18:34–13:18:43, lines 2806–2807]
- **10 seconds vs current 9 seconds:** workflow truth is 10 seconds; current configuration/theoretical 1,600/hour is a known model mismatch.
- **Assignment cap:** current experiment says `W`; `W+2`, dispatch-above-W, admission control, and CONWIP targets remain design work.
- **Waiting geometry:** transcript says the diagram appears to have two waiting blocks; current model/task context says six waiting positions per station. Reconcile with wiki before geometry changes.
- **APP/PPP terminology:** Vinay was unsure (“APP and PPP, something like that”) and referred Daniel to documentation. Use wiki as terminology evidence; do not treat the expansion as settled solely from this transcript. [13:35:34–13:36:15, lines 3601–3629]
- **Order destination:** no evaporation. Direct extraction is a temporary experiment that bypasses buffer routing; it does not alter station internals.
- **Drop and collect:** transcript examples emphasize drop; Daniel's authoritative note explicitly requires both drop and collect with the same back-out/no-rotation movement.
- **Floor/highways:** the call contains long exploratory and contradictory directionality/naming discussion. Daniel's user note defers it and confirms existing HTM traversability is acceptable. Do not implement the call's proposed six single-cell “highway entities” now.
- **Path-planning scope:** maximum-throughput evidence is initially for full-horizon SIP. Robustness to speed variability and windowed planning was discussed as important, but was not promised for the immediate run. [12:58:37–12:59:27, lines 1963–1982; 13:52:32–13:53:14, lines 4408–4436]

## Proposed task order (planning only)

1. Verify current operator/pick behavior in code and configuration; implement only if inspection confirms the gap. Align pick duration to 10 seconds and prove mutual exclusion across APP sides.
2. Add station observability: assigned/dispatched/admitted order counts, operator state and idle reason, APP readiness, arrival mismatch, and completed-pick rate.
3. Implement/validate scheduler saturation at assigned=`W=6` for the current experiment; then separately evaluate dispatch lookahead above W without exceeding physical admission capacity.
4. Implement inventory-led pairing: FIFO inventory sequence, reorderable order HTMs, position-aware readiness/release, and explicit behavior when the mate is late.
5. Temporarily bypass the order buffer and route completed order containers directly to extraction.
6. Correct relay access geometry and enforce enter-service-reverse semantics for drop/collect with no relay traversal or rotation.
7. Add whole-run wait-duration and speed heatmaps with selector and unit-bearing hover values; preserve richer distributions for later diagnostic views.
8. Run pick-only/full-horizon comparisons against iteration 10, reporting normalized throughput and starvation breakdown. Do not use the stale 9-second/1,600-hour ceiling as the acceptance target.
9. Later: reconcile waiting-block geometry from documentation, reorganize open floor/highways, then implement floor-driving aisle-pinned VTMs and revisit dynamic VTM allocation/thrashing.

