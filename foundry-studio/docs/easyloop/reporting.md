# Walmart reporting audit

For the current profile exporter, JSON schema and HTML regeneration workflow,
see [the full report-generation guide](../report-generation.md) and
[the reporting tools](../../scripts/walmart_matrix/README.md). The audit
below records the September 8 state; later putaway and reporting work is covered
by the [September 14 matrix](overnight-2026-09-14/report.html).

The September 8 audit separates absent workflows from lost or misclassified
measurements. UI and XLSX should consume the same reporting facts and time windows.

## Putaway

The Walmart composition currently has no inbound putaway task. `FillInventory`
seeds inventory at startup; it does not represent an operator filling a tote.
Returning inventory containers, storing filled orders in the buffer and returning
empty order containers to their depot all generate storage movements. None is an
inbound putaway. Their throughput must be reported without inflating putaway/hr.

Actual Walmart putaway remains dependent on the site answers in
[the questions document](questions.html): empty inventory tote supply, products
and quantities loaded per tote, arrival demand and operator dwell.

## Reporting corrections

| Finding | Required behavior |
| --- | --- |
| Extraction counters were accumulated but omitted from export and chart selection | Show completed extraction count/rate and extracted units, with station totals and an operation journal in UI/XLSX |
| Pick and extraction completion counted at dwell start | Count at physical dwell end; retain start and duration for occupancy |
| Multiple inventory visits reused an order HTM presentation identity | Count the inventory visit identity, while retaining the order trip identity for robot processing |
| Filled order-buffer containers appeared empty | Track physical contents independently of available inventory stock |
| Future events aggregated occupancy in emission order | Integrate chronologically ordered physical content/location changes |
| Several physical racks shared a drawing cell, collapsing their occupied slots | Count `(RackId, level)` from inventory custody; a reservation still occupies its rack slot until pickup |
| Revised historical slices could rewind the live robot display | Replace metrics by slice identity while preserving the newest live robot positions and the paused timeline cursor |
| Extraction station IDs missing from trip export | Preserve the actual extraction endpoint |
| Every VTM return labelled putaway support | Use storage retrieval/return direction labels; infer business purpose only from explicit facts |
| Picked units/hour labelled presentations/hour | Name the measured unit correctly |
| Fully picked orders labelled as if outbound work was finished | Distinguish picking completion from extraction completion |

New extraction records use the existing descriptor-backed stream container.
An absent historical stream means unavailable; an existing empty stream means
measured zero. A legacy profile cannot supply a container split that it never
recorded.

## Validation and limitations

The full-map physical gate completed 4,000 source orders over 11h47m of simulated
time with collisions enabled. The reporting pass separately checks observer
reconstruction, shared report/profile round trips, UI slice revisions, exported
worksheet contents and an actual headed 500-order run. These checks distinguish
reporting correctness from the physical throughput gate; see
[throughput validation](throughput.md) for the latter's measurements.

The final headed run retained the exact 5,378.043-second simulation duration and
completed 576 picks, 500 orders and 500 extractions. The exported workbook has
500 chronologically ordered extraction operations totaling 576 units, with the
same totals and covered-time rates on its summary and extraction sheets.
Inventory aisles' first-hour occupied-slot fractions are 0.997636–0.999695 rather
than the erroneous 0.004555 produced by drawing-cell aliasing. Occupied storage
equals empty plus non-empty storage in every exported aisle/window. These are
occupied tote slots, not a claim that every tote contains saleable inventory.

The final observer pass has 108 passing tests and one existing ignored test.
Earlier checks of the shared observer/inventory/relay/reporting/profile path
passed 209 tests; UI/export checks passed 19; live polling/export checks passed
two; fulfillment orchestration passed 11. The separate task-configured putaway
timing test also passes. The latter uses its existing small fixture and is not a
Walmart collision acceptance test. The completed headed run and workbook are an
end-to-end export check; automated native visual inspection was unavailable.

The broader Cencora check `minimal_putaway1_no_violations` exposed an existing
end-of-stock termination failure. Both the pre-reporting binary from `095847f21`
and the current code stop with the same barrier at 1,160.183 seconds after three
picks and 43 putaways. The finite empty-tote supply has drained while putaway
tasks remain waiting. This is not a successful Cencora parity gate, nor evidence
that Walmart's absent putaway metric should be fabricated from storage returns.
The reproduction logs are retained with the September 8 reporting run.

## Remaining visibility

These need additional production facts; the current generic storage and robot
reports are not substitutes for them:

- Box induction completed at HTM pickup, distinct from box creation.
- Extraction split by reusable tote, tray and disposable carton.
- Filled-order buffer entry, storage, retrieval and residence by order/container.
- Per-conveyor physical tote occupancy, retained inventory-HTM queue occupancy,
  admission blocking and boarding-to-collection time.
- Productive utilization of each shared operator, combining its two presentation
  sides with one denominator; separately, time blocked in presentation cells.
- Remaining demand and work in progress alongside throughput, so startup and
  batch drain can be distinguished from starvation or deadlock.

The full-map physical tests already measure several of these for validation.
They must be emitted through the production reporting path before they can be
claimed as available in saved profiles, the UI or XLSX. See
[throughput validation](throughput.md) for window and conservation conventions.
