# Automatic inventory preload — September 9

Daniel confirmed that inventory totes may contain multiple SKUs and multiple
units of each SKU. The earlier single-SKU packing was a scenario choice, not a
general tote constraint. Until Vinay supplies the mock-put file specifying
actual tote contents and rack positions, automatic preload distributes the
complete on-hand inventory itself.

The original automatic-preload baseline used:

```json
{
  "stock": "as_extracted",
  "inventory_distribution": "even",
  "units_per_tote": 100
}
```

The preload preserves all **1,397,589 units of 174,997 SKUs**. Across the map's
**162,060 inventory totes**, **60,951 totes contain 8 units** and **101,109
contain 9**. Order-container depots and order buffers receive no inventory
stock. The 100-unit setting is a cap, not a request to invent extra units.

Slots are deterministically shuffled across the warehouse. Each receives its
share of the total; SKU batches fill these shares, splitting when necessary
and allowing another SKU to occupy the remainder. Every per-SKU quantity is
preserved. This balances unit counts, not volume, weight, velocity class or
travel distance; no physical slotting policy is inferred from the source.
Insufficient tote capacity or missing inventory positions are errors in this
mode, rather than reasons to drop stock.

The full source snapshot is also used for smaller order samples and for the
throughput preparation script. Changing the order limit therefore does not
change the warehouse inventory. Cencora and existing configurations retain
their default `single_sku` packing. Runtime tote storage, retrieval and picking
already support mixed-SKU contents.

This still uses the finite `FillInventory` setup task at time zero. It neither
implements inbound putaway nor invents a format for the pending mock-put file.
The source extract is **43 units short across 40 SKUs**. The original baseline
preserved that discrepancy; the 500-order sample had sufficient stock.

## Approved demo correction

After the full main-spec launch stopped at that stock check, Daniel authorized
exactly the missing 43 units on September 9. Both main and sample profiles now
explicitly reference `walmart_demo_stock_top_up.csv` through the optional
`scenario.stock_top_up_csv` setting. Its `item_id,additional_units` rows add only
each SKU's full-source demand deficit. Original on-hand and outbound CSVs remain
unchanged. The loader rejects unknown/duplicate SKUs, zero additions and overflow;
there is no implicit replenishment or completeness waiver.

Effective demo preload is **1,397,632 units** across the same 162,060 totes:
**60,908 totes × 8 units** and **101,152 × 9**. Order limits still leave preload
unchanged. This is a documented temporary correction, not a claim about measured
site inventory. Vinay's corrected snapshot/mock-put file is still needed.
The original-stock regression remains separate from the corrected main-spec test.
The benchmark below predates this correction; its results are historical.

Prior throughput measurements used demand-only inventory. They remain valid
for their frozen inputs, but a run with full stock is a different workload and
placement baseline.

## Validation

- Seven inventory-placement checks pass, covering per-SKU conservation,
  evenness, determinism, sparse/empty input, capacity refusal, order-pool
  exclusion and unchanged single-SKU packing.
- The full-source loader regression checks all 174,997 SKU quantities against
  the snapshot, the exact 8/9-unit tote counts, and identical stock after
  limiting the same scenario to zero orders.
- All 15 fulfillment orchestration tests pass. The new mixed-SKU regression
  serves two concurrent orders from one tote containing A×5, B×4 and C×2.
  Picking A×5 and B×3 leaves B×1 and C×2 after two storage returns, with no
  outstanding SKU claims and no changes to unrelated contents during a pick.
- Both physical-transfer audit helper checks pass. The complete full-map
  500-order audit passes with collision avoidance enabled: **576 units in
  564 paired transfers**, **246 cartons**, **316 buffer round trips** and
  **2,268 VTM cycles**. Final physical stock and the ledger both retain
  **1,397,013 units**. Completion is **4,075.772 simulated seconds**.
- The throughput preparation script's output retains every row of the original
  on-hand file even for the 500-order sample. Desvis builds successfully.

The first full-stock audit reached the same 4,075.772-second timeline and all
500 completed orders, but its five-minute wall-clock guard classified the run
as `WallTimeout` during completion. It was not counted as passing. The test
now allows ten minutes for the larger stock snapshot, event capture and final
checks; the rerun reached natural `Complete` in 247.259 runner wall seconds.
Only the test's wall budget changed. Neither simulation duration, collision
avoidance nor completion requirements were relaxed. These wall times also
include different host contention and are not a performance comparison.

Headed **iteration 05** uses the same full preload and 500-order sample. Run
logs, frozen binaries, checksums and physical timing exports are saved under
`../notes/walmart-preload-2026-09-09/` relative to the Walmart worktree.
