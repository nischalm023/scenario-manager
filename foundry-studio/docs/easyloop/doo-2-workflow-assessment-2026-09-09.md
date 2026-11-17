# DFW6 DoO 2.0 and the Walmart simulation

The next work is workflow fidelity, correct waiting and HTM release, and picks
per simulated hour. XLSX and temporary demo reporting are deferred. The existing
full run can continue independently of smaller controlled checks.

Source: `DFW6 Pilot Design of operations DoO Draft 2.0.docx`, supplied September 9,
2026, version 2.0, dated August 31, marked **Draft for Review**. SHA-256:
`f121c2f2faadbee194169ade2a1617abf2f23a36f2b16f653d926089f003837f`.
The document was extracted in full and rendered; the outbound flow diagrams and
their surrounding sections were visually checked. Its placeholders, exception
proposals and unreleased equipment counts are not confirmed site parameters.

## Workflow differences

| Area | DoO requirement | Current simulation and needed correction |
| --- | --- | --- |
| Preparation and admission, §§7.1–7.2 | Prepare sources and targets in parallel. Release source movement toward EasyLoop after the relevant target-ready trigger and station capacity check. | First retrieval overlaps target acquisition, but source delivery waits for target arrival in Waiting. This correction also overlaps the next required retrieval with the unfinished target's return to Waiting. Separating relay preparation from station admission and coordinating approaches remain. |
| Baggable orders, §§7.1.3, 7.2.1, 8.3 | Route-compatible singles may share a target tote; downstream bagging produces individual shipments. | Each order currently owns a separate target tote. Add explicit batching and per-order attribution, after route fields, batch limits, closure and bagging timings are defined. Inventory-tote quantity is a different issue. |
| SIOC, §§7.1.4, 7.3 | No EasyLoop target or Packsize carton. Retrieve the source tote directly to Flex, remove and label the item, return the source tote, and route the resulting parcel downstream. | The approved interim model uses an order tray at EasyLoop. Replace it with a distinct Flex flow. The current map has three generically named extraction stations and no declared Flex identity/queue. Physical handoffs and service capacity must be supplied; 100 UPH is a target, not measured service time. |
| Incomplete targets, §7.2.2 | Re-evaluate against the next source; target may remain Active, move to Queue, or return to Waiting. | Every unfinished visit currently sends the target through its return loop to Waiting. Completed orders no longer take this detour. Later work should retain an active target when justified by known source readiness and station scheduling, rather than imposing a return on every unfinished order. |
| Completed targets, §7.2.2, E13 | Leave only when the immediate downstream handoff is available and reserved. | Open-ended destination protection prevents dispatch into an occupied endpoint, but completed targets can wait at presentation for too long. Distinguish an actual downstream capacity constraint from avoidable scheduling delay. Do not clear physical reservations while the HTM remains present. |
| Boxes, §§8.1–8.2 | Pick into an open Packsize carton, hand off to Ranpak/box finish and SLAM, then sort 3P parcels or re-induct DTC/DTS parcels for buffering/JIT dispense. | A carton is presently buffered or extracted directly after picking; whole-box removal stands in for downstream handling. Add finish, label and re-induction stages with their own load custody and capacities. Preserve carton identity through finish and buffering instead of treating the first handoff as final shipment. |
| Auto-bag, §8.3 | Source target totes enter a conveyor station; contents are bagged and labeled, and empty totes return from the station. | Extraction currently removes contents while the tote stays on its delivery HTM, then that HTM returns it to storage. Model dropoff, independent processing and empty collection once station geometry and HTM release rules are clear. |
| Buffer and destinations, §§8.1, 8.4–8.5 | Route/store conditions and downstream capacity choose JIT versus storage; a later release triggers retrieval. | DS currently always buffers and is immediately retrieved; SC/3P go directly to extraction. CSV codes DS/SC/3P are not mapped to the DoO's DTC/DTS/3P channels yet. Do not infer that mapping from names or percentages. |
| Inbound, §§5.2–5.3 | Case induction and delivery to decant; select empty or top-off source totes, fill them, validate, then store through HTM/relay/VTM. | Initial stock is preloaded; Walmart inbound roots are absent. The DoO adds top-off totes and separate case-delivery work to the earlier empty-tote model. Arrival data, tote eligibility and station handoffs remain necessary. |

The [September 10 scheduling wave](scheduling-wave-2026-09-10.md) implements
the coordinated presentation approach and deferred carton-extraction Parking
described below. The original assessment and its earlier measurements remain
as the before-change record; site-dependent workflow differences remain open.

## Scheduling correction and remaining defects

`flows/fulfillment/orders.rs` formerly started the next `RetrieveTote` only after
the order HTM completed its return to Waiting. It now starts one next required
retrieval at pick completion, overlapping that return movement. This changes
readiness, not route geometry, fleet size, service time or conveyor capacity.
The last pick creates neither another retrieval nor a Waiting detour. The
physical-timing regression failed before the change: the second retrieval
started at 64.258 seconds, exactly when the target reached Waiting. It now starts
strictly earlier, while remaining at or after the preceding pick completion.

Carton extraction registers an empty HTM and immediately calls `vacate_cell`
before another task can claim it. This is an unconditional parking assignment,
contrary to Daniel's departure-ownership ruling. Simply deleting it would leave
unclaimed HTMs blocking extraction. The required replacement is a deferred
Parking proposal, cheap before acceptance, cancelable when productive work wins,
and fully planned only when selected. A next assignment owns its own departure;
the preceding workflow does not recursively plan unknown future assignments.

The local `direct-empties` work supplies proposal-family competition and
dissolution, but not this finished generic Parking task. Its arbitration groups
contenders through resource-manager claims; it does not automatically discover
all arbitrary overlapping intervals. Integration must include ordinary HTM
assignments and fallback liveness. Its presence in the current remote master has
not been established by this review.

The order HTM's presentation approach also waits until the inventory presentation
becomes physically ready. Future joint reservation should reduce this delay while
retaining fairness: a later-ready order must not monopolize a presentation ahead
of earlier-ready work. Current readiness gating has a regression for that case.
Review of the underlying claims found two reservations to coordinate: the
inventory presentation slot and the order HTM's physical destination hold, both
open-ended. Removing the readiness wait alone lets future work block an earlier
arrival; making only the conveyor booking finite does not fix the physical hold.
A station-owned selection of known source/target visits is the next design to
validate, including unequal approach times, changing traffic and use of either
side. It must not create a premature proposal that prevents the simulation clock
from reaching the selected visit.

## Calibration and open decisions

- **Pick service:** nine seconds is currently charged once per SKU transfer,
  regardless of its quantity. The DoO distinguishes UPH and presentations/hour
  but gives no per-unit/per-presentation service decomposition. Changing batching
  without resolving this can create an artificial units/hour gain.
- **Equipment:** §4.3 explicitly labels 230 HTMs/72 VTMs as an unreleased WIP
  BOM. Keep the currently approved 144 HTMs and 40 inventory plus eight provisional
  buffer VTMs until the site resolves the disagreement. Do not adopt full-scale
  rack counts from the same document as pilot-map dimensions.
- **Floor VTMs:** the existing library-ladder approximation does not reserve
  VTM floor traffic. The draft's separation-of-movements prose does not supply
  physical clearance, aisle passing or VTM/HTM interference rules.
- **Exceptions:** the draft leaves timeout, skip/requeue, ICQA and recovery rules
  open; some exception row titles do not match their response text. Do not turn
  those draft notes into invented retry counts or automatic order termination.
- **Source data:** the supplied arrival span is about 47 hours, while the DoO
  evaluates a 20-hour operating shift. Current preload remains provisional
  automatic placement with the approved 43-unit overlay, pending mock-put data.

The [site questions](questions.html) now reflect the direct Flex flow, batch
compatibility, physical downstream handoffs and pick-service interpretation.
Channel mapping and the Flex cells/queue have also been asked directly.

## Measured result of next-source overlap

The full-map control uses the same 500 source orders, all available at zero,
with 1,397,632 units of full warehouse stock, identical automatic placement,
144 HTMs, 48 VTMs, geometry, dwell and collision avoidance. It changes only the
subsequent-retrieval scheduling above. Both binaries were built in release mode
with the `walmart-demo` feature, and both runs completed without a time limit stop.

| Measure | Before (`c34aa22f6`) | With next-source overlap |
| --- | ---: | ---: |
| Completed orders / picked units | 500 / 576 | 500 / 576 |
| Physical SKU transfers | 564 | 564 |
| Complete-batch duration, seconds | 4,004.988 | 4,006.720 |
| Units per simulated hour, whole batch | 517.75 | 517.53 |
| VTM cycles / HTM navigation events | 2,268 / 9,533 | 2,268 / 9,533 |

There is **no throughput improvement in this sample**: completion is 1.732
seconds later, or 0.043% lower unit throughput. Retain the correction because it
removes an unnecessary dependency for known work; do not present it as a speedup.
It leaves the larger presentation-approach and downstream-departure issues open.
This finite cohort includes startup and drain and does not establish sustained
station capacity. Other simulations shared the host, so wall time is excluded
from the performance conclusion.

Validation passed: 17 fulfillment integration tests, plus the separate full-map
audit of all 500 orders and 576 units in 564 transfers, 246 complete carton
lifecycles and 316 physical buffer round trips. The audit checks source/target
custody, exact SKU quantities, inventory conservation, operator exclusion and
conveyor capacity. Split quantities, repeated SKU lines, zero-quantity lines,
bounded retrieval and no final-pick return to Waiting have explicit regressions.
The independent audit and the headless candidate both ended at 4,006.720 seconds.

Baseline binary SHA-256:
`bdbd4c0c13d520a588bec4b8ee3ad83e79c0ceb008a99edeeffbf435d177af82`.
Candidate binary SHA-256:
`b7f27585aa660eb922fd44f46e7a0c3244c04d18ad8da361f541e2d36b7e6770`.
The retained benchmark manifest includes input hashes and the source patch;
the profile build stamp alone does not distinguish these frozen binaries.

## Validation sequence

1. Prove each scheduling correction with physical event timing, inventory
   conservation, bounded conveyor occupancy, operator exclusion and normal
   collision avoidance. No additional final-pick waiting or speculative parking.
2. Compare an unchanged 500-order cohort before and after the correction,
   preserving full stock, placement, arrivals, fleet, geometry and dwell. Report
   units and actual transfers separately; retain startup and drain definitions.
3. Use sustained backlog to measure capacity, separately from source-arrival
   replay. Measure source/target readiness and downstream blocking rather than
   attributing all idle time to a single resource. Queue occupancy and elapsed
   waits must refer to the same population and time window.
4. After changing real flows, report Flex, EasyLoop, bagging and shipment
   throughput separately. Moving SIOC out of EasyLoop or batching several orders
   into one tote changes the work mix and cannot be called a scheduling speedup
   against the previous route model.

Retained extraction, render, benchmark logs and source checksums are under
`../notes/walmart-doo-2-2026-09-09/` relative to this worktree.
