# Walmart inventory tote fill — September 8 (historical capacity-only change)

**Superseded September 9:** Daniel confirmed that inventory totes may hold
multiple SKUs and multiple units per SKU. Automatic preload now distributes the
full on-hand snapshot evenly among inventory totes, while preserving every SKU
quantity. Vinay's tote-addressed mock-put file is pending. See
[automatic preload](automatic-preload-2026-09-09.md) for current behavior.
The results below describe the earlier demand-only stock configuration.

Daniel requested approximately 100 units per inventory tote. The full-source
and 500-order launch profiles previously set `units_per_tote` to **1**; both
now set it to **100**. This setting controls packing capacity. It does not
multiply stock, change order quantities, or fill order totes and inducted boxes.

The current Walmart composition seeds inventory at time zero through
`FillInventory`; it does not yet run inbound replenishment. The same batch-size
ruling must apply when that physical putaway flow is implemented.

## Earlier stock policy

Both profiles currently retain `stock: "cover_demand"`: for each requested SKU,
seed the smaller of its total order demand and its supplied on-hand quantity.
Consequently, capacity 100 alone does **not** create full 100-unit totes.

| Source cohort | Requested SKUs | Demand units | Seeded units under existing policy | Stocked totes at capacity 100 |
| --- | ---: | ---: | ---: | ---: |
| All 22,160 orders | 21,161 | 25,625 | 25,582 | 21,161 |
| 500-order sample | 559 | 576 | 576 | 559 |

The full cohort has 18,652 SKUs with only one seeded unit under this policy;
the sample has 547. Rounding demand up to full 100-unit batches would instead
seed **2,116,100 units** and **55,900 units**, respectively. That would be an
explicit synthetic-stock choice, not a reading of the on-hand snapshot.

The question then pending with Daniel was whether to use those full batches for
throughput work or preserve source quantities and allow partial totes. That question is now resolved by the September 9 ruling above. The original
source quantities remain intact; the full-source 43-unit shortage across 40 SKUs
still remains an input-validation issue.

## Physical behavior and checks

The focused CollisionDes regression serves two concurrent orders of three and
four units from one 100-unit inventory tote. Both reported transfers match the
actual stock deltas, and the same physical tote returns to storage twice with
**93 units remaining** at completion. A separate packing check distributes
250 units into 100, 100 and 50, preserving a different SKU's single unit in its
own tote. All five packing checks pass.

The full-map **500-order capacity-only check also passes**: 576 units in
562 physical transfers, 500 extractions, 246 carton lifecycles, 316 buffer
round trips and 2,264 VTM cycles. Reported completion is 4,118.252 simulated
seconds (runner wall time 265.496 seconds alongside the long control).
This seeds the original 576 stock units into 559 totes; it is **not** the
55,900-unit full-batch experiment. The prior one-unit control took 4,193.720
seconds. Packing changes placement and reduces transfer count, while the
multi-unit dwell remains uncalibrated, so this difference is not evidence
of a scheduling improvement.

The strengthened full-map audit independently derives each physical transfer's
SKU and quantity from source and order contents. Event quantities, each
order's per-SKU demand and extraction units must match; initial inventory
minus transferred units must equal final physical stock and the SKU ledger,
with no remaining order stock claims. Its two helper checks and all thirteen
fulfillment orchestration tests pass.

The current task tree returns an inventory tote to storage after each
presentation, even if it retains stock. Another order retrieves it after the
return; inventory cannot currently remain on the conveyor for a second order.
More units in a tote therefore do not automatically reduce VTM or conveyor
trips. They can also concentrate repeated demand on one circulating tote.

Multi-unit transfers currently receive one fixed pick dwell. Report **units**
and **physical paired transfers** separately; the appropriate multi-unit
service-time rule remains a site-calibration question. Existing one-unit
throughput baselines must not be compared as if stock packing were unchanged.

The previous dispatch-only 4,000-order run uses its frozen one-unit spec and
binary. It completed in 32,988.036 simulated seconds and remains a control for [the completed-order dispatch change](outbound-dispatch-2026-09-08.md).
