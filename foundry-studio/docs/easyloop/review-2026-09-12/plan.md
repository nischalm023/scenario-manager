# EasyLoop workflow and throughput plan

September 12, 2026 · layout accepted at `494a2581f` (iteration 15) · reconciled with the inventory CONWIP checkpoint

**Implementation update:** Daniel moved item 5 first. Relay forward-entry,
backing-exit, no-turn, initial-facing and playback work is implemented, with
independent checks that all 2,132 Walmart stands are under their shelves and
rendered boards. The main Walmart spec now specifies 10-second operator service
and independent 9-second extraction. A separate throughput-control spec preserves
the full source workload and adds only T0 release and direct extraction. See
[the change record](relay-maneuvers.md). The inventory CONWIP checkpoint also adds per-side area admission and exact
station diagnostics. Full-map 100- and 500-order runs completed, alongside focused
workflow and shared-service checks. See [the inventory change record](inventory-conwip.md).
The eight-hour comparison now completes with 4,981 picks (622.625/hour), up
11.48% over the 4,468-pick baseline. That is still only 43.24% of the operator
ceiling; [fleet and VTM isolation experiments](fleet-isolation.md) follow.

Daniel's [complete message](user-message.txt) is preserved verbatim. Three Sol agents reviewed the full call transcript, current fulfillment implementation, and all three wiki pages. The root checked the consequential code paths and source statements and corrected the reviewers' overstatements before this consolidation.

## Confirmed per-block topology

Implementation: both the minimal and full Walmart fixtures declare two independent
six-cell waiting blocks per EasyLoop, with one fixed presentation per block.
Iteration 15 removes the four turn-helper cells and the lateral jog: the complete
entry/return lanes meet 700 mm centreline-radius turns on straight tangents.
Presentation positions and 135°/45° phases remain unchanged. The full-map apron
is aligned with the moved order doors. Daniel inspected and accepted this layout.
See [the checkpoint and validation record](direct-curves-experiment.md).

Loaded metadata assigns each waiting cell to its sole internally reachable
presentation. Order streams are presentation-owned rather than waiting-cell-owned;
an accepted order receives a slot from its side's area calendar. Cross-side
presentation choice is not allowed. Temporal area admission and inventory
pipeline sizing are implemented in the inventory CONWIP checkpoint.
33 map/motion/flow checks pass, including all 12 minimal-map orders (24 picks)
completing with both presentations used and every order remaining on its side.
Iteration 15 was launched without automatic startup; Daniel subsequently pressed
Run. It loads the canonical main spec and therefore retains order-drop timing.

**Ongoing throughput-run rule:** release the full order cohort at T0 for throughput
testing, retain original source timestamps, and state the timing override in the
sequential window label and run provenance. The main spec retains its real arrival
curve. Short internal samples are allowed; the canonical scenario remains uncapped.

**Known unnecessary first visit:** `FulfillOrder::schedule` still unconditionally
moves the acquired container to its waiting cell before preparing presentation.
The next-in-sequence order should instead pass the block's entry/admission boundary
and proceed directly to its fixed presentation when the paired schedule permits.
Owning a return-berth right must not require physically visiting that berth.
Use waiting only when actual sequencing/readiness requires it. This direct-first
route is pending. Retain no return
visit after the final pick; unfinished orders may need to recirculate for later
inventory, with the existing one-way graph respected.

**Final-pick egress ruling, reconfirmed after iteration 15:** once its final
item is picked, the order HTM continues around the curve and up the egress lane
to its outbound destination. It must not be dispatched to or routed through a
waiting cell. The current task already returns to waiting only when items remain.
Regression checks now inspect every waypoint of the first outbound leg and
require that leg to leave the machine, for reusable totes, cartons and trays
through direct and buffer routes, and for split/repeated order lines. All 22
fulfillment orchestration tests pass. The old trip-attribution test requiring
side switching now verifies both presentations while keeping each order on its
assigned side. No production scheduling behavior changed in this check.


Daniel now specifies that each waiting block owns a WIP order pool and one fixed order-HTM presentation point. The two blocks do not share a waiting pool or permit opportunistic switching between presentation points. His schematic is:

```text
[Enter]          [egress]
[ ⏷ ► ] [◀ wait] [ ◀ ▲  ]
[ ⏷ ► ] [◀ wait] [ ◀ ▲  ]
[ ⏷ ► ] [◀ wait] [ ◀ ▲  ]
[ ⏷ ► ] [◀ wait] [ ◀ ▲  ]
[ ⏷ ► ] [◀ wait] [ ◀ ▲  ]
[ ⏷ ► ] [◀ wait] [ ◀ ▲  ]
  ~ conveyor underpass ~
  ~ conveyor underpass ~
  ⤷ present(angled) ⤴
```

Each descending-lane row allows continuation toward presentation or a lateral move into its waiting cell. Each waiting cell returns to that descending lane. The independent ascending lane takes HTMs away from the curved presentation maneuver toward egress and permits a leftward move into any waiting cell on its row. A returning HTM can therefore rejoin its waiting cell directly, then later leave leftward into the descending lane. Waiting cells have incoming links from both lanes and an outgoing link only to the descending lane; no north/south traversal through waiting cells is implied. Preserve two under-conveyor crossings and the angled presentation dwell on a continuous U-turn. The two sides retain their existing different presentation angles unless separately revised; the shared operator remains exclusive.

Daniel resolved the return connection by adding a leftward arrow to every ascending-lane row. The earlier top-crossover proposal is withdrawn: recirculation is ascending lane → waiting cell → descending lane, wholly within the block. An unfinished order retains its admission and return-berth right through that cycle and does not pass the new-order admission gate again. Final departure continues up to egress and closes membership at its committed area-clear time. New upstream arrival can hold the dedicated external entry cell until its minimum admission sim-time.

## Source interpretation before Daniel's topology confirmation

Daniel's newly supplied [diagram](two-waiting-blocks.png) visibly labels PPS Waiting Block 1 and 2, each with six drawn waiting positions, plus PPS Queue Points n-1/n-2 and two Active Bins. This superseded the earlier shared six-cell interpretation. The current map metadata and admission service implement two independent six-order areas with a fixed presentation per side. The image itself does not explicitly label the external entry cells or every order-HTM travel direction; those details come from the accepted map and Daniel's later routing rulings.

The orchestration draft supports a sector-specific mapping: it requests order-container transport from a racetrack to its corresponding sector's active bin and resolves the destination sector from the source sector. This supports, but does not independently prove, Daniel's proposed one-block/one-presentation relationship. If adopted, each order HTM is assigned to a block/side and cannot opportunistically choose the other presentation. Inventory routing must respect the block's matching presentation. One operator and the inventory conveyor remain shared by both sides.

The proposed two-stage route handoff is valid: upstream delivery ends with an open-ended reservation at the block's dedicated external entry cell; an admission child plans from that cell into the block no earlier than the area calendar allows, closing the entry reservation as part of committed departure. The cell must be an actual holdable entry location outside the counted resident area, with separate resident return/exit paths. Its open-ended hold must not occupy a required through/exit cell. This reservation handoff adds no deliberate dwell; admission is based on planned sim-time exit/entry intervals, not elapsed wallclock. Multiple external entry cells could also share a single capacity calendar, so two blocks are a physical-layout decision, not a software prerequisite for admission.

`AreaAdmission` now owns two independent calendars per EasyLoop, keyed by the two declared order areas, each with capacity six. At most twelve unfinished order containers are therefore members of one EasyLoop, six on each side, plus separately bounded upstream work. A presenting or recirculating order retains its membership and return right until its committed route actually exits the area. The n-1/n-2 and Active Bin positions shown in the drawing are distinct physical positions; they are not extra admission credits. This does not double the shared operator's service capacity.

## What the evidence changes

- **The operator already exists as an exclusive resource.** One `Conveyance` owns one operator calendar shared by both sides; `TransferItems` starts after the matching inventory and order presentations are ready and keeps them occupied through the transfer. Walmart now configures 10-second service independently of 9-second extraction. The remaining timing ambiguity is a transfer containing multiple units.
- **Current inventory sequencing is not physical FIFO.** `selected_quote` ranks intents by readiness, skips an infeasible earlier intent, and may select later work. It has no physical conveyor sequence or transfer-step conflicts. Model ordered conveyor movement and choose order-HTM movements around it.
- **Assignment now belongs to fixed-presentation streams.** Streams are not statically attached to waiting cells; `AreaAdmission` chooses a slot on the stream's side. Exact events expose assignment, preparation, dispatch, admission, waiting, presentation, final pick and area exit. Inventory preparation can begin before order-container acquisition, but each logical order still prepares only its next inventory visit and the order-container pipeline does not prepare before claim.
- **Relay coordinates and movement are now checked together.** Iteration 16 puts the presentation stand under the board, removes through traversal, and permits forward entry followed by reverse exit with constant chassis heading. All 2,132 full-map stands pass the alignment check; Walmart and Cencora maneuver checks pass. Daniel inspected and accepted the result.
- **Direct extraction is an experiment control.** It removes completed-order buffer storage/retrieval and release timing from the causal path. It still includes travel, extraction service, empty-container return, and HTM reuse. It does not remove real extraction backpressure.

The latest completed eight-hour baseline is `b54de8226`, before this CONWIP wave: **4,468 picks, 558.5/hour**, with 10-second service, all orders eligible at T0 and direct extraction. The theoretical four-operator ceiling is 1,440 single-unit picks/hour, so this baseline reaches 38.8%. The earlier iteration-10 result was 5,397 picks at 9-second service and is not a controlled comparison. This wave’s matched eight-hour run delivered 4,981 picks (622.625/hour), an 11.48% gain. Its last seven hourly totals range from 620 to 633, so the remaining gap is persistent underfeeding rather than a late collapse.

## Proposed work order

| Order | Work chunk | Acceptance evidence |
|---|---|---|
| 1 | Establish the isolation scenario: all eligible orders at T0 for throughput tests, completed containers directly to extraction, 10-second picking, independently specified extraction service. Preserve original order destination and record the routing override. | No completed-order buffer round trips; every completed container follows a physical extraction path; box removal and reusable-container return remain correct. No accidental change to extraction dwell when pick time changes. |
| 2 | **Implemented in the inventory CONWIP checkpoint:** exact station and inventory lifecycle events, profile storage and Desvis time series. Operator busy comes from the existing shared cycles; unknown causes remain explicitly unknown. | Focused tests cover exact timestamps, matched visit identity, simultaneous deltas and old profiles. Terminal summaries and a full-run diagnostic review remain pending. |
| 3 | **Partly implemented in the inventory CONWIP checkpoint:** separate logical streams, per-side six-member area admission, incumbent return rights and release at actual area exit; bounded inventory preparation uses a derived CONWIP cap. | Focused mixed direct/buffer tests cover the physical boundary. Full-map 500-order completion exercises the full derived limits. The eight-hour comparison reaches 622.625/hour. Direct first presentation and an independent order-container preparation pipeline remain pending; the fleet-isolation experiments distinguish resource delay from orchestration. |
| 4 | Make the station follow an explicit inventory conveyor sequence. Represent penultimate staging, transfer/changeover conflict, two inventory APPs and exit capacity; coordinate order arrivals by actual travel and side availability. Keep operator service separate. | A blocked head tote cannot disappear from the sequence or be silently overtaken. Both sides can stage/change over while the one operator picks on the other. Matched readiness governs pick start. Delayed mates, repeated picks, both APPs occupied, full conveyor, and exit blocking all remain live or expose a specific unmet condition. |
| 5 | **Completed first, at Daniel’s direction:** add an explicit forward-in/reverse-out relay maneuver and prohibit in-place rotation there. Remove along-rack links from the relay cells while keeping the adjacent travel lane usable. Independently express no-in-place-rotation at other presentation cells. | No relay through routes or Rotate actions. Chassis heading remains constant while backing out. Path timing, reservations, carried-tote orientation and playback agree. Curved EasyLoop presentation approaches remain valid. Initial relay residents have valid headings and can depart. Check Walmart and Cencora. |
| 6 | Add switchable wait-duration and speed heatmaps with units on hover. This can proceed alongside the station work once measurement definitions are agreed. | Same result live, at completed run, and in playback at the same observation time; no future or cancelled movements counted; queue/presentation exclusions applied semantically; bounded memory. |

Chunks 1–4 are today's throughput priority. The completed relay correction is included in the eight-hour baseline, keeping this comparison separate from the effect of removing illegal shortcuts. Open-floor reorganization stays deferred to later this week; floor-driving aisle-pinned VTMs stay deferred to next week.

## Assignment, dispatch, admission: the architectural decision

**Follow-up clarification from Daniel:** chunks 1 and 2 are agreed. Diagnostics primarily means profile/UI time series, with terminal summaries as a secondary output. His retrieval-latency question supersedes arbitrary 6/7/8 tuning as the main pipeline-sizing recommendation below. Six remains a comparison control, not the intended total in-flight limit.

The order-container pipeline must cover VTM retrieval, empty HTM approach to the relay, handoff, loaded HTM travel, and local circulation/readiness. Size upstream work separately from the six physical waiting positions. A successor should already be undergoing retrieval/travel before its predecessor's final pick; final-pick completion and physical clearance must not be the trigger to begin that retrieval. Replenishing a larger maintained pipeline at completion is compatible with this lead time.

For the inventory boundary, the implemented `AdmissionLaw` sets
`cap = ceil(queue_factor × F_max / pick_period) + 2`. `F_max` combines the
configured worst VTM retrieval, empty-HTM approach and loaded delivery bounds.
The token spans accepted release before `RetrieveTote` through the matching
paired-pick completion. This is a shared per-EasyLoop logical CONWIP ceiling,
not arrival pacing or a throughput guarantee. The conveyor's physical capacity
of eight is independent and remains occupied from unload until collection.

Illustration only: one completed order every 10 seconds and 120 seconds of upstream preparation imply approximately 12 orders in that upstream pipeline before variability allowance, independently of the local waiting positions. That does not call for 12 additional floor cells. The pipeline spans retrieval, relay handoff and travel, with timed entry to declared destinations. More waiting cells become necessary only if local residence/return rights require them, not simply because rack retrieval is slow. With multi-pick orders, use the actual order completion period; the operator's pick period is not the order arrival period.

Both inventory and order streams need bounded upstream work and physical admission. `SupplyConveyor` now acquires the logical CONWIP token before retrieval and separately waits for physical conveyor admission at unload. The eight-tote conveyor bound and retained-HTM queue constrain physical custody through collection. The logical stream target derives from the same inventory cap; the former half-fleet worker limit is removed. Logical inventory visits may exceed the robot fleet because they include work released before robot acquisition. Relay and robot acceptance separately preserve resources needed to finish admitted work. No finite WIP count alone proves zero operator idle; paired supply, motion, changeover and downstream schedules must be feasible.

The current stream count acts as an **assigned-order target `A`**, independent of each side's **physical membership capacity `W=6`**. Count an order as assigned from station allocation through its final pick. On final-pick completion the stream can claim its successor at the same simulation timestamp while outbound work retains its own area membership and reservations through actual exit. Exhausted or not-yet-eligible demand remains an explicit exception.

The former shared-block A=6/7/8 experiment is superseded by two independently admitted blocks and latency-based preparation sizing. Each block has W=6 physical membership credits. Its larger upstream assignment target is derived from measured completion rate and preparation lead time, with explicit variability allowance. Bounded preparation must include stock, HTM and conveyor commitments; increasing logical assignment alone does not feed the operator.

Little's law applies to a defined boundary: `mean assigned orders = completed orders/second × mean assignment-to-final-pick seconds`, measured over a stable interval with censored work handled consistently. For single-pick orders at a 10-second service ceiling, six assigned orders cover only **60 seconds of average assignment-to-final-pick time**. If measured preparation and station residence exceed that, six cannot sustain the operator ceiling. More logical work can cover the lead time without creating more physical waiting cells.

For upstream preparation alone, a starting estimate of the additional pipeline is `order completion rate × preparation lead time`; variability then needs measured headroom. This is not a proof that `W+2` is sufficient or that maximum WIP is desirable. Increase the target only while it reduces operator starvation without increasing station blockage or downstream queues.

Early HTM dispatch above current physical occupancy can be useful. Permit it only with a feasible timed route to a declared destination whose earlier reservation will actually end. Do not dispatch an extra robot merely because a logical slot exists and then leave it blocking an approach. Preserve unfinished orders' return-berth rights until a tested admission policy can safely reuse them. A count limit alone does not prove deadlock freedom; arrival, presentation, return and exit claims must be compatible.

Implement this as a small station assignment/admission service used by the long-running fulfillment task, reusing the existing acquisition, supply, navigation, transfer and dispatch tasks. Keep conveyor sequence/capacity and operator calendars in their existing resource owners. Avoid both a new `ModelState` and a collection of six new state services without a demonstrated ownership need. Each task plans its own known work; the next task or deferred `Parking` owns departure, without recursively planning all future tasks.

## Admission and circular-wait clarification

**Sim-time admission ruling:** the next order HTM's minimum boundary-entry time comes from a previous order HTM's committed boundary-exit time. Scheduling can book that future entry immediately; it must not wait for wallclock passage or even for the simulation barrier to reach that exit. Half-open membership intervals `[entry, exit)` permit replacement at the same simulation timestamp, subject to the ordinary cell/edge clearance reservations.

Implemented owner: a reusable `AreaAdmission` state-management service with a capacity calendar and identity-bearing membership claims per declared order area. Capacity W and boundary/cell membership come from the world layout; fulfillment and navigation/dispatch tasks retain typed shared handles. Inventory conveyor/retained-HTM capacity remains in `Conveyance`; upstream inventory CONWIP remains a separate `PipelineAdmission`. Presentation-owned streams replace static waiting-cell workers.

A new-order route obtains the service's earliest legal entry, plans subject to that boundary time, and finalizes membership at the committed crossing. The area right may name a future entry before simulation wallclock reaches it. Order-container acquisition begins only after this right is accepted, avoiding a resource cycle while the order waits for entry. Departure planning records committed area-clear time rather than pick completion or extraction. Existing members retain membership through waiting-to-presentation-to-waiting circulation. The order area excludes the inventory HTM path, so its cap does not gate inventory supply.

Daniel accepts ordinary F_max/queue-factor CONWIP and route Wait waypoints for inventory supply, including modest stacking at the conveyor entrance. Do not introduce an additional external waiting-block abstraction or redesign inventory admission merely to eliminate that stacking.

For order containers, distinguish assignment (logical work), dispatch (a physical retrieval/travel commitment) and membership in the EasyLoop order subsystem. More than W logical assignments does not itself cause a deadlock. A conservative initial rule admits at most W order containers and gives each unfinished admitted container a return-berth right that persists while it presents or recirculates. Returning an already admitted order must not require another new-order admission token. Completed outbound HTMs retain physical reservations until clear; logical completion alone must not expose their occupied route as free.

In the current one-container/one-HTM order model, finishing an admitted order needs its already-admitted order HTM and subsequent inventory totes, not a different order HTM from outside. Therefore the W gate must apply only to new order-container membership, never to inventory supply or an existing member's return. This breaks the apparent dependency on more order HTMs entering, but does not prove the physical paths are safe.

Two concrete circular-wait cases still need tests before this policy is claimed safe:

1. New order HTMs waiting outside obstruct inventory access, incumbent recirculation or completed-order exit, preventing the capacity release they are waiting for.
2. FIFO inventory at the head serves only an unadmitted order; that order cannot enter until an incumbent finishes, while inventory needed by every incumbent sits behind that head. Assignment above W must not release future-order inventory into such an unresolvable position.

The second case requires coordinated order eligibility and inventory sequencing, while retaining ordinary inventory CONWIP. Inventory for an unadmitted order can be prepared ahead, but its irreversible conveyor release needs a feasible matching order presentation/admission, rather than assuming a future place will open. A tote also serving admitted orders is not automatically blocked merely because it serves future orders too. An explicit, timed physical recirculation policy could provide another way out, if adopted; free logical skipping cannot.

Upstream order dispatch may exceed W when ordinary scheduling finds a feasible arrival/wait/entry plan that preserves incumbent progress. No second-tier waiting block is introduced. The current admission service uses committed future area exits for replacement membership, but a finite inside count alone is not proof of safe spillback. Full-bay occupancy, late inventory, a long multi-pick incumbent, simultaneous returns/exits, new arrivals, and the FIFO future-order case still need load validation.

## Pairing and conveyor fidelity

The [high-level wiki](https://greyorange-work.atlassian.net/wiki/spaces/GRYMTTR/pages/72724213/Easy+Loop+Station+-+High+level+Details) defines **APP = Active Presentation Point**, specifically the inventory presentation on the conveyor. The corresponding order-container position is an **Active Bin/induct point** in the [orchestration draft](https://greyorange-work.atlassian.net/wiki/spaces/GRYMTTR/pages/72723168/Walmart+PoT+-+Easy+loop+Station+Orchestration+Design). Keep that distinction in code-facing documentation and use inventory/order presentation in the UI where clearer.

The next inventory tote is selected by its physical conveyor sequence. For its required order container, use its block’s fixed presentation and compare feasible order arrival time, inventory APP readiness/changeover, and shared operator availability. Dispatch the order HTM soon enough to meet that tote; do not start the operator clock while either mate is still travelling. Prepare the other side during service. After a partial pick, retain the order at presentation only if its next usable source makes that appropriate; otherwise circulate it through its return path. A completed order goes outbound immediately.

The wiki specifies a penultimate position that must not obstruct an APP exit and a transfer mechanism that cannot push laterally and pull longitudinally simultaneously. These need explicit resource conflicts even if animation remains approximate. An ordered sequence plus a few phase/resource calendars is enough for the first useful model; detailed roller physics is unnecessary.

The page gives 8-second drop/changeover budgets. Treat these as calibration evidence, not numbers to blindly add on top of the existing 45-second circuit abstraction. Define which physical actions each timing includes before replacing that abstraction. The operator's 10 seconds is a separate resource service.

Physical inventory recirculation is described in the wiki. It is compatible with FIFO if a tote really travels to the requeue point and rejoins the sequence; it is not permission to skip an infeasible intent for free. The late-mate policy—hold versus explicit recirculation—needs a named choice and evidence. Begin with observable head waiting unless the recirculation action and timing are included together.

## Heatmap decision

Use accumulated **HTM waiting seconds** on eligible floor cells and **moving mean speed in m/s** as the two selectable views. Speed is distance divided by moving time; waiting and pure rotation are accounted separately. Hover exposes the value, units, exposure, and exclusions. Add a small duration-weighted speed histogram to reveal fast/slow populations; standard deviation alone is not an indicator of healthy traffic.

A cumulative histogram still cannot distinguish alternating waves from interference. Later, a selected-cell time strip with travel directions, motion, waiting and rotation will answer that question. Keep cumulative totals sparse and histogram storage bounded. The [traffic review](traffic-review.md) specifies source data, cancellation/time clipping, spatial attribution and validation.

## Capacity and honest comparisons

Four operators at 10 seconds provide **1,440 completed pick tasks/hour**, before starvation or blocking. Whether that equals units/hour depends on multi-unit transfer service. Report pick tasks, units, inventory presentations and extracted orders separately.

Three extraction ports at a separately retained 9-second dwell provide at most **1,200 orders/hour**, before travel and clearance. If extraction unintentionally inherits 10 seconds, that falls to 1,080. Thus direct extraction removes buffer effects but can expose a different capacity limit. Do not remove physical extraction constraints to obtain an attractive pick rate. Measure station-only service saturation separately from sustainable end-to-end output if downstream capacity is limiting.

Use the same input order cohort, inventory, fleet, map and timing policy within each comparison. Report full-run and explicitly bounded interior windows, per-station utilization and idle causes, maximum and mean WIP, actual completed extraction, and whether pending work continues progressing. Change controls separately enough to identify causality. A better eight-hour pick count with an ever-growing outbound queue is not a sustainable-throughput improvement.

## Decisions and unresolved source details

1. **Recommended architecture:** separate assignment target and bounded preparation from physical admission; use `A=6` as the control and test 7/8. Retain explicit return rights and use existing task/resource owners.
2. **Timing clarification:** does a multi-unit transfer use 10 seconds total or 10 seconds per unit? Keep extraction independently configured either way.
3. **Late source/order matching:** implement visible FIFO head waiting first; add physical recirculation with its time/capacity cost as a separate policy, rather than adopting the draft's skip behavior implicitly.
4. **Conflicting inventory-HTM exit rules:** the wiki permits empty exits under pressure or timeout, whereas Daniel previously required inventory HTMs to remain under the conveyor. Preserve the explicit ruling and record the discrepancy; do not change it silently.

The product page also describes HAI and QT hardware variants with different inventory-bot maneuvers. Those are not grounds to universalize EasyLoop conveyor entry/exit behavior. The earlier curved order-HTM U-turn remains a valid continuous-motion maneuver, while relay back-out is a separate maneuver.

## Review trail and limits

- [Transcript review](transcript-review.md): all 4,667 transcript lines; timestamp/line references for source statements and omissions.
- [Implementation review](architecture-review.md): operator, presentation custody, assignment, FIFO gap, direct dispatch and reuse of existing tasks.
- [Wiki and geometry review](wiki-geometry-review.md): all three pages, dates, draft status, differences from project rulings, under-board geometry and reverse-motion gap.
- [Traffic review](traffic-review.md): root proposal with independent Sol critique.

Root checks confirmed the shared operator calendar and transfer validation, the skipped-head path in presentation intents, the extraction/pick default coupling, the iteration-10 completion line, the relay alignment assertions, and relevant wiki statements. The root rejected the initial review's 125-per-loop baseline, its blanket prohibition on early dispatch, and its proposed six-service split. Those corrections are reflected here.

All three wiki texts were read through the Chrome extension. Embedded images were unavailable or uninspected; an attempted background asset bundle failed for all six high-level diagrams. No image-derived layout dimensions or step-timing tables are claimed as verified. No simulation code, scenario, map, running process, or branch history was changed by this review.

## Full remaining docket after iteration 15 acceptance

1. Establish the T0-arrival, direct-extraction throughput control; use 10-second
   operator transfers with quantity interpretation explicitly provisional and
   extraction dwell configured independently. Record all overrides and source IDs.
2. Review the implemented profile/UI diagnostics in a full run. Exact sim-time
   events now drive logical assigned-order and physical resident series, matched
   order/inventory readiness, shared operator busy intervals, and inventory
   Released/Loaded/Boarded/PickCompleted series. Add terminal summaries and
   blockage attribution only where evidence is available; unknown stays unknown.
3. Complete the partially implemented assignment/admission design. Presentation-owned
   streams, per-side W=6 membership, future entry and actual-exit release exist.
   Add direct first presentation and an independently sized order-container
   preparation pipeline; current preparation-ahead applies only to inventory.
4. Coordinate both streams around the physical FIFO conveyor and fixed order
   presentation sides. Prepare inventory and successor containers ahead; model
   lateral-transfer/changeover conflicts and prevent a future-order head tote
   from blocking inventory needed by incumbent orders.
5. Correct relay enter/back-out motion, forbid through travel and in-place
   rotation, and give initial relay residents valid departure headings. Validate
   Walmart and Cencora. Do not add compulsory parking or waiting legs.
6. Validate extraction approach, service and HTM release/reuse without serializing
   travel unnecessarily. Preserve real endpoint capacity and keep the next task
   or deferred idle relocation responsible for departure.
7. Add wait-duration and speed heatmaps, value/units on hover, and bounded speed
   distributions. Audit missing metrics against real workflow events; a zero
   inbound-putaway rate is currently expected because Walmart has no inbound roots.
   XLSX parity remains secondary to live/profile visibility and correct flows.
8. Resolve and implement DoO-specific flows: direct-source SIOC/Flex, compatible
   bag-order batching, box finishing/re-induction, bagging/dispense endpoints,
   and confirmed buffer eligibility/release. Add inbound putaway once its workload
   and service specification are available. Import Vinay’s mock-put stock when supplied.
9. Later: floor-driving aisle-pinned VTMs, confirmed fleet/handling parameters,
   open-floor organization and charging behavior. Active Walmart fleet is now 230 HTMs, per Daniel’s September 12 direction,
   plus 40 inventory and eight provisional buffer/depot VTMs.
10. Throughout: minimal-map deadlock/sequence tests, full-map sustained-backlog
    runs, conservation and physical-custody checks, memory and proposal-cost
    measurements, Cencora regressions, updated diagrams/questions, WIP checkpoints
    and refreshed Mac demo bundles after a validated milestone.

11. Deferred from the original dual mandate: reconcile the remaining worldlens/format
    migration scope and unanswered format rulings, retain the Cencora parity gates,
    and prepare the eventual reviewed landing. This does not precede Walmart flow
    and throughput work.

No new site answer blocks assignment/admission and diagnostic implementation.
Multi-unit operator timing, conveyor phase timing and downstream equipment facts
remain calibration questions in the HTML questions document; they must not be
silently inferred from throughput targets.
