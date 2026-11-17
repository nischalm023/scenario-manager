# Walmart flows and remaining site questions

September 7 checkpoint. Daniel prioritizes throughput and the whole simulation.
The provisional choices below authorize the current implementation; the
[printable site questions](questions.html) collect the facts still needed from
the site team. The broader world-protocol migration and detailed conveyor
animation remain deferred.

**September 13 priority ruling:** deliberately retain whole-box removal at the
sealer handoff. The complete seal/re-induct/buffer/dispense lifecycle is queued
for later this week, after the current throughput, buffer and scheduling issues
are resolved; it is not today's work. See the [current buffer review](review-2026-09-12/buffer-review.md).

| Flow | Implemented behavior and evidence | Remaining work or site confirmation |
| --- | --- | --- |
| EasyLoop picking | Concurrent orders own distinct waiting berths and draw from shared pending work. Inventory HTMs stay in the under-conveyor queue; inventory totes use the shared eight-tote conveyor, two presentations and one operator. Every transfer requires the correct inventory and order containers together. | Detailed shared-belt stops, lateral transfers and transit calibration remain open. The task diagram shows the proposed further decomposition. |
| Bag orders | Retrieve an empty reusable order tote of physical type `Tote`. It remains on its HTM through picking. At extraction, remove its contents after the dwell, retain the empty tote on the HTM and return it to OrderDepot. | The extraction dwell stands in for downstream packing/bagging. Actual endpoint capacities and processing times need confirmation. There is no auto-bagger at EasyLoop. |
| Box induction | `InductContainer` acquires an empty HTM, navigates to a declared induction presentation and creates a new `Carton` there at its actual arrival time. Pickup moves custody onto the HTM after the configured fork dwell. The HTM holds the source throughout creation and pickup. | Immediate box supply is approved for this pass. Assembly time, source replenishment, endpoint capacity and any separate tray/support need site confirmation. |
| Box extraction | `ExtractOrder` retains the box and its contents through the extraction dwell, then removes the whole box, marks it `Extracted` and releases an empty HTM. The carton identity and history remain available for audit. | Carton workloads require reachable declared parking egress. If runtime clearance cannot find free reachable parking, the run reports that failure while retaining the empty HTM and its source hold. Actual sealing, sorting and downstream equipment remain unmodeled. |
| SIOC orders | Provisional physical type is reusable `Tray`. Typed retrieval keeps trays distinct from reusable totes. Extraction removes the shipped contents and the HTM returns the empty tray to OrderDepot. | Confirm SIOC's actual packaging, support, storage compatibility and tray handling. Configure the initial tray count explicitly with `FillInventory.order_trays`; the default is zero. |
| Order buffer | Store the same filled container in OrderBuffer, release its delivery HTM, then retrieve that named container with another available HTM. The mixed-container test covers totes, cartons and trays through this route. | DS uses the buffer provisionally. Its actual eligibility, capacity and release/wave policy need confirmation. |
| Direct outbound | SC and 3P go directly to the extraction presentation. SC's `Sorter` and 3P's `Dock` labels remain in the order data. | Confirm the meanings and physical routes of DS, SC and 3P, including the effect of `delivery_method`. Dedicated sorter/dock behavior is deferred. |
| Inbound putaway | The shared tasks already supply an empty inventory tote, fill it at a putaway station, and store it through HTM → relay → VTM. Walmart declares six putaway stations. Daniel selected this sequence. | Walmart's composition does not yet start inbound roots. Units per tote, SKU mix, arrivals and service times remain unspecified. Daniel put this flow last. |

## Approved choices for this pass

- **Bag:** reusable order tote; downstream extraction removes contents and
  returns the empty tote to storage.
- **Box:** immediate box appearance plus the HTM pickup/fork action; exclusive
  order cargo; whole-box removal at extraction leaves an empty HTM.
- **SIOC:** reusable order tray, emptied at extraction and returned to storage.
- **Routing:** DS buffers; SC and 3P go directly to extraction.
- **Cutoffs:** retain source data but ignore cutoffs for the throughput pass.
  `deadline` no longer acts as a retrieval not-before time. Buffer release is
  `stored_at + buffer_dwell_secs`; its default is zero, so the approved pass
  retrieves immediately after storage. Explicit nonzero dwell remains available
  for configured scenarios and the existing timing regression.
- **Initial inventory:** distribute all 1,397,589 supplied units across 162,060
  inventory totes (8 or 9 units each), preserving per-SKU quantities and allowing
  mixed SKUs. Order samples retain the same full preload. This automatic
  placement stands in for Vinay's pending tote-addressed mock-put file.
  Implement inbound putaway last.
- **Conveyor timing:** preserve the prior full-circuit interpretation of the
  declared 45 seconds, split into equal outbound and return legs. Operator,
  queue and resource delays remain additional. The number and split still need
  calibration.

The existing `walmart_dfw6_putaway_spec.json` suppresses initial stock when
putaway is enabled, but its Walmart composition has no inbound roots yet. It
is not a working replenishment scenario.

## Verification and configuration

`fulfillment_orchestration` exercises a six-order CollisionDes matrix:
reusable tote, carton and tray, each through direct extraction and the buffer.
It checks the actual physical container at the pick, named-container buffering,
removal only at extraction completion, reusable-container return and carton
creation at declared induction cells. A separate constrained-fleet test checks
that extracted cartons release HTMs for subsequent induction. Dedicated tests
cover competing induction requests and no mutation when a tentative claim is
discarded.

Initial trays are configured on the short-lived `FillInventory` root through
`order_trays`. The root assigns that many seeded empty order-container IDs to
physical type `Tray` before registration, so existing and late-created relay
indexes agree. Remaining seeded order containers retain physical type `Tote`.
The source adapter's explicit `unknown_container: "as_tray"` policy maps SIOC
orders to this provisional tray workflow.

Live and recorded visualization receive a timestamped container-creation event
with physical type and designation. Cartons render brown, reusable order totes
remain teal, and trays render shallow. Types persist through later slices;
containers do not appear before creation or remain visible after extraction.
Older profiles retain their original order-tote designation through the
slice-zero roster.

The full-map generated mixed-container gate completes 500 orders and 2,000
picks. A separate 500-order source sample completes 576 picks, including
246 box extractions and 316 physically audited filled buffer round trips.
See [workload evidence](workload-checks.md) for inputs and measured results.
The earlier all-tote throughput comparisons below do not establish mixed-flow
throughput.

## Questions and source evidence

[Questions for the site team](questions.html) cover destination meanings and
buffer eligibility; cutoff date, timezone and milestone; release waves;
SIOC and box/tray handling; induction/extraction capacities and dwell; putaway
data; conveyor timing; and later floor-VTM calibration. Observed fields include
`shipping_protection` (`bag`, `box`, `SIOC`), `destination_type` (`DS`, `SC`,
`3P`), `wave`, `cutoff_time`, `delivery_method` and `order_drop_time`.

- Daniel's September 7 brief governs geometry, shared waiting, curved
  presentations, eight-tote capacity, retained inventory HTMs and the one-cell
  box source.
- The archived August 7 walkthrough describes a box assembled onto a tray.
  Its older operator/queue descriptions are superseded by Daniel's later
  rulings; actual box support remains a site question. The local excerpt is
  `~/vt/greyorange/notes/dual-mandate-2026-09-02/easyloop-DESIGN-v0.1.md`.
- [The Walmart source adapter](../../sim/dessim/src/scenario_gen/walmart.rs)
  retains source labels and explicit interpretation policies. Provisional
  routing approval does not establish the operational definitions of those
  labels or the correct use of cutoff times.
- Butler at `7064a99964` contains whole-tote induction and extraction examples
  in `apps/mhs/test/ct/gmc_relay/manual_tote_{induction,extraction}_flow_SUITE.erl`.
  Those examples provide vocabulary and mechanisms; site-specific rates and
  rules come from the site discussion.

## Earlier all-tote throughput comparison

The concurrent baseline completes 2,000 picks in 17,029.804 simulated seconds,
about 423 picks/hour including startup and cleanup. All generated orders
arrive at zero. Fixed per-berth batches leave one machine finished at about
9,518 seconds while the slowest picks until about 16,887 seconds. Shared
pending work with claims ordered by simulated berth availability cuts the run
to 14,437.316 seconds.

Restoring the existing full-circuit conveyor convention removes the erroneous
charge of 45 seconds on each leg. The combined run completes in 13,871.701
seconds: 519.04 picks/hour, 22.77% above the concurrent baseline. This restores
the prior model interpretation; actual transit calibration remains open.
