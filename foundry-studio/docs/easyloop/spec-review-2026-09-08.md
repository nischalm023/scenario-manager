# Walmart document assessment September 8

The full-workflow HTM fleet is now 144. Keep 40 inventory VTMs plus eight buffer
VTMs provisionally, as Daniel directed; Vinay still needs to confirm whether the
buffer machines are additional to the document's fleet. The larger remaining
gap is physical: Walmart declares floor-running VTMs, but the current simulation
still gives them the rack-attached VTM execution model.

## Sources and authority

Reviewed the supplied DOCX exports, including all rendered pages:

- **Walmart Dallas — Simulation Design Specs**, pages 1–2 for figures and
  directionality; page 3 contains layout images.
  [Linked source document](https://docs.google.com/document/d/1f982ItN8-5uhJocar6JULzm80CKyqsl0zwluoW0W3lU/edit).
  Export SHA-256: `e3549de79502dc62f587a0486ccfac5846dbf607d6a4d8b0b80cc46547d95352`.
- **Simulation Objectives**, page 1 for targets and primary KPIs, page 2 for
  validation metrics.
  [Linked source document](https://docs.google.com/document/d/1OA-IByVGgQaRVQJt6245gpdyA1RkWT14QLd_24t9DpI/edit).
  Export SHA-256: `090410e8de41d4056702eff685d7022b32a7560298896823cc310a947ffa2c2b`.

Vinay had not confirmed that these exports contain his latest revisions. They
are assessment inputs, not permission to replace later rulings about EasyLoop
geometry, container routing or the parked world-format migration. The questions
page records unresolved interpretations, including the fleet clarification.

## Fleet and workload targets

| Item | Document | Current decision or gap |
| --- | --- | --- |
| HTMs, putaway + pick or full flow including extraction | 144 | Changed full Walmart map cohort from 122 to 144 |
| HTMs, pick-only experiment | 80 | Separate flow variant; do not relabel the full outbound workflow as pick-only |
| Inventory VTMs | 49 installed, assume 40 working and nine charging | Keep two active VTMs in each of 20 inventory aisles; charging is not additional productive capacity |
| Order-buffer VTMs | Not separately specified | Retain eight dedicated VTMs provisionally; Vinay must confirm |
| Picking | Four stations × 320 units/hour = 1,280 units/hour | Target, not a measured result; four shared operators, not eight independent presentation servers |
| Tote presentations | Four × 290/hour = 1,160/hour, approximately 1.1 units/presentation | Confirm source versus target visit definition and workload batching |
| Decant | Six stations × 250 presentations/hour = 1,500/hour | Inbound tasks, SKU/quantity arrivals and dwell remain unspecified |
| Duration | 20 hours plus fleet sensitivity | Existing 11h47m finite-backlog gate is insufficient for this objective |

At target output, a 20-hour run needs 25,600 picked units and approximately 23,200
source presentations if that is the intended presentation definition. Merely
raising the simulation timeout does not provide sustained demand or replenishment.
The source feed, usable stock, arrival schedule, warmup and measurement window
must agree. The current finite cohorts remain regression tests.

Do not turn 250 decant presentations/hour into a 14.4-second operator dwell by
assumption. It is a throughput requirement. Likewise 320/290 is approximately
1.10 units per visit, not permission to manufacture fractional picks. Under the
current nine-second shared-operator dwell per transfer, 1,160 transfers/hour
would require 72.5% productive utilization across four operators before
considering travel and synchronization, if one source presentation corresponds
to one such transfer. The implementation currently passes the same dwell for
multi-unit transfers too; that timing rule needs calibration. At one unit per
transfer, as in the source regression, 1,280 units/hour instead requires 80%.
These are necessary service-demand checks, not predictions of achievable
throughput or proof that source visits and transfer events are interchangeable.

## Changes to prioritize

The [checked September 8 review](review-verification-2026-09-08.md) adds an
extraction service-demand check. Three independent endpoints at the current
provisional nine-second dwell have a **1,200 extractions/hour dwell-only upper
bound**, before clearance or shared downstream equipment. At the full source
mix of 25,625 units in 22,160 orders, 1,280 units/hour implies about 1,106.92
extractions/hour, already 92.24% of that bound. Presentation targets and unit
targets are not automatically extraction rates. The current dispatcher also
excludes ports while inbound travel is being planned, so physical service duty
alone hides a substantial access constraint. Approach cells, exclusion timing
and egress must be measured and specified before a capacity claim.

1. **Implement floor VTM occupancy before interpreting congestion or fleet
   sensitivity as site results.** Start with two robots confined to each aisle,
   which the design explicitly allows for V1. Aisle confinement still requires
   collision avoidance and stationary service occupancy.
2. **Make KPI populations and windows explicit in UI and XLSX.** Add distinct
   inventory-source and order-target presentation journals, endpoint tote
   throughput, arrival/departure dwell and conveyor residence. Report productive
   shared-operator time separately from blocked presentation occupancy. Build
   speeds, stops, rotations, fetch and fork timings from actual motion/handling
   facts. Existing extraction, occupancy, transit and heatmap reports are useful
   but do not supply every requested KPI.
3. **Build the representative 20-hour experiment.** Add confirmed inbound and
   outbound workloads and initial free storage, then conduct controlled HTM/VTM
   sensitivity with identical work. Track remaining demand, WIP, completed work
   and carry-in/out for each window; keep the finite-batch gates too.
4. **Reconcile layout revisions before changing geometry.** The new document's
   bay/tunnel arithmetic and incomplete highway notes conflict with existing
   layout rulings. Changing counts alone would conceal the underlying mismatch.
5. **Resolve routing, batching and induction semantics.** The objectives mention
   3P sorting versus direct-to-store buffering; direct 3P extraction remains the
   approved temporary behavior. “Four totes batch picking for one order” and
   “put wall” are too ambiguous to implement. The new targets do not answer
   putaway units per tote, arrival times or equipment/operator dwell.

The document's 80% relay and 95% storage figures are stated operating concerns,
not universal deadlock thresholds. Report physical occupied-slot time, distinct
positions used, reservation occupancy and blocked time separately. A busy-position
count over an hour is not its time-weighted occupancy. Sweep these conditions
with the actual topology and workload rather than forcing those percentages into
the admission logic. The current source fixture starts nearly every inventory
slot holding a tote, many empty; it is not a calibrated 95%-occupied site model.

## Floor VTM implementation boundary

The world already has `robot_class.locomotion = "floor"` for both Walmart VTM
classes and actual aisle cells admitting VTMs. The loader ignores that distinction
when assigning VTM cohorts. This can be fixed within the current world format;
no docking-marker geometry or format migration is needed.

Today `VtmService` reserves each robot's availability frontier, not floor cells.
`BotService` rejects VTMs, initial fleet setup reserves only HTM positions, and
VTM events describe straight rail moves. Consequently existing `CollisionDes`
results establish HTM collision behavior, not VTM–VTM or VTM–HTM interference.
Travel is projected onto one aisle axis, with a universal same-side rack/relay
constraint; floor access must instead obey actual approach/reachability rules.

The smallest coherent change is to:

- Preserve locomotion in the runtime fleet projection and retain Cencora's
  rack-attached implementation separately.
- Register each floor VTM with its own actual starting cell and heading, using
  the existing floor navigation reservation service. Generalize the existing
  navigation primitive's robot type and motion parameters rather than duplicating
  retrieval/storage parent tasks.
- Execute rack/relay work as floor navigation, alignment/lift/fork, navigation
  and transfer. Retain floor occupancy while stationary and keep fork height
  separate from chassis position. Confirm whether lifting can overlap driving.
- Preserve physical tote custody and existing VTM operation journals, deriving
  movement and blocked time from the actual reserved paths. Replace blanket DA50
  labels with VTM or the declared, confirmed equipment model.
- Test two VTMs meeting, stationary lift blocking, HTM conflicts where floor is
  shared, transfer timing and Cencora parity. Add cross-aisle relocation later;
  a multi-zone assignment must preserve one robot identity rather than duplicate
  the cohort into each zone.

Relevant code: `worldmap/racking.rs`, `model/relay/vtm/`, `model/bot/`,
`flows/ttp/setup/initialize_fleet.rs`, `flows/transport/navigate.rs`,
`model/relay/config.rs`, `scenario_gen/vtm_placement.rs` and `robots/vtm/motion.rs`
under `sim/dessim/src/`.

Calibration also needs a correction: the run banner reports `vtm.speed = 6.0 m/s`,
but active VTM cycles use hardcoded horizontal/vertical maxima of 3.0 m/s and
acceleration of 2.0 m/s², with simultaneous horizontal/lift timing. The inherited
world-class speed declarations differ again. Neither attachment supplies the
actual machine values. Preserve the current benchmark while making these inputs
explicit in the floor implementation; do not quietly select a different number.

## Layout discrepancy to resolve

The design text specifies 19 bays, 23 levels, three single-sided aisles and
161,690 inventory locations. Current map tests preserve a prior 21-bay ruling,
including two tunnel bays: 170,570 positions in the main block, of which 8,510
are assigned to the empty order-container depot, leaving 162,060 inventory slots.
The separate buffer adds 3,450, for 174,020 total positions. Inventory-only totals
therefore differ by 370, but that small difference hides different bay/tunnel
geometry. It is not justification for deleting 370 arbitrary slots.

Both specify 2,072 inventory relay positions; the map additionally has 60 buffer
relays. The document's tunnel starting height of 18.17 m exceeds its stated
32-foot total storage height. Request the corrected dimensions and revision,
including what the unfinished five-superhighway note means, before editing steel
or traversal.

## Validation of the fleet change

The source-sample physical gate with 144 HTMs and the retained 48 VTMs completes
all 500 orders, 576 paired picks, 246 carton inductions/extractions and 316 filled
buffer round trips in 5,449.938 simulated seconds. The same workload at 122 HTMs
took 5,378.043 seconds. This single deterministic comparison is about 1.34% longer,
so the larger fleet is conformance to the stated full-flow design, not a measured
throughput improvement. The motion/geometry/workload parameters were unchanged.
Custody, conveyor capacity, operator exclusion and HTM collision checks pass.
This does not validate the still-missing floor VTM collision model or the 20-hour
site objective.
