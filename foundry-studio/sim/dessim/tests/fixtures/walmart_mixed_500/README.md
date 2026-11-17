# Mixed-container throughput sample

This fixture samples 500 orders across the supplied Walmart outbound extract:
sort distinct order IDs and select index `floor(i * order_count / 500)` for
`i = 0..499`, retaining every line of each selected order. The on-hand file here
contains only the referenced SKUs, with their original quantities, and is kept
for historical runs. The current launch profile reads the complete
`walmart-data/onhand.csv` snapshot.

All sampled `order_drop_time` values are deliberately set to
`2026-09-07 00:00:00` to exercise throughput with demand available immediately.
Other fields, including packaging, destinations, quantities and cutoff labels,
retain their source values. This is not a representative arrival schedule or a
deadline test.

The sample has 562 lines and 576 units: 224 bag orders, 246 box orders and
30 SIOC orders; 316 DS, 91 SC and 93 3P orders. There is sufficient extracted
stock for every selected SKU.

`../walmart_dfw6_mixed_spec.json` runs this sample on the full reviewed map
with collisions and HTM kinematics. Its inline composition is the Walmart
root task set with an explicit **test population of 24 reusable order trays**
configured on `FillInventory`. That count is not a site measurement. Remaining
empty order containers stay totes. No map capacity is changed.

The run preloads all **1,397,589 source units across 162,060 inventory totes**,
independently of this 500-order sample. Automatic placement gives each tote
8 or 9 units, may mix SKUs, and preserves every SKU's quantity. The configured
100 units is a capacity cap. Vinay's pending mock-put file will provide actual
starting contents and positions. Earlier demand-only results retain their
original meaning; see [automatic preload](../../../../../docs/easyloop/automatic-preload-2026-09-09.md).

The run uses the approved provisional SIOC→tray policy,
buffers DS orders, and sends SC/3P directly to extraction. Buffered orders
are retrieved as soon as storage finishes; cutoffs do not delay retrieval.
See [site questions](../../../../../docs/easyloop/questions.html) for the
unconfirmed operational details.
