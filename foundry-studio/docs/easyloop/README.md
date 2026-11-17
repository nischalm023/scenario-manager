# EasyLoop

[**Open the task composition diagrams**](tasks.html)

[**Open the printable site questions**](questions.html)

[**Checkout and run the Walmart baselines**](../walmart-baselines.md) provides
portable headed-run commands for the measured partial/putaway and full-preload cases.

[**September 14 throughput matrix**](overnight-2026-09-14/report.html) and
[**per-variant time-series charts**](overnight-2026-09-14/variants.html) contain
the preload, putaway and order-timing comparisons. The
[full report-generation guide](../report-generation.md) covers experiment design,
recording, audits, both HTML formats, refinement and portable packaging. The
[tool reference](../../scripts/walmart_matrix/README.md) lists the commands. The reports work
offline; the variant viewer uses embedded JavaScript.

[**Post-pick handoffs: map, routing and remaining workflow**](outbound.html)
records the September 13 top-edge station additions and recovered client drawing.

[**Buffer release, capacity and full-flow validation**](review-2026-09-12/buffer-review.md)
records the buffer-enabled baseline, its hypotheses and the remaining workflow gaps.

[**DoO 2.0: workflow differences and next corrections**](doo-2-workflow-assessment-2026-09-09.md)
compares the September 9 source document with the current implementation. The
next priority is correct workflow and higher pick throughput: direct SIOC/Flex,
bag-order batching, downstream handoffs, and removing unnecessary task waits.
Physical endpoint and routing questions are recorded before those flows change.

[**Presentation approaches and deferred parking: September 10 scheduling wave**](scheduling-wave-2026-09-10.md)
records the next independent corrections and their controlled validation.

[**Independent inventory supply and proposal diagnostics**](inventory-supply-2026-09-10.md)
records the subsequent delivery/unloading correction and its 500-order comparison.

[**Simulation memory: shared inventory indexes and sparse playback history**](memory-2026-09-10.md)
records the iteration-08 diagnosis, measured corrections and remaining storage decisions.

The task document contains the parent–child task trees, reusable movement and
custody tasks, scheduling constraints, and EasyLoop configuration. The questions
document separates unresolved site details from Daniel's approved provisional
choices. Both HTML files open directly in a browser and require no build step,
JavaScript, or network access. [The Butler terminology mapping](butler-terminology.md)
records the source revision and scope of each naming choice.

The HTML's current tree records the implemented composition; the other trees
describe further task decomposition. [Named JSON compositions and shared state
resolution](../compositions.md) are implemented, and `ModelState` has been removed.
Bag orders use reusable order totes; box orders use inducted cardboard cartons;
SIOC uses reusable order trays for this pass. The mixed-container integration
tests cover all three through direct extraction and buffering on the minimal
and full maps, including a 500-order source-file sample. Cartons leave
whole, while reusable totes and trays return empty to storage. DS uses the
buffer, and SC/3P go directly to extraction. Cutoffs are ignored for the
throughput pass. The buffered throughput preset now adds a synthetic one-hour
minimum hold after storage, pending the route/truck release schedule. The
September 13 zero-hold baseline keeps its frozen input for comparison.

See [remaining flows, implemented behavior and provisional rules](remaining-flows.md),
[workload evidence](workload-checks.md), [queueing and throughput validation](throughput.md), and the [earlier implementation checkpoint](implementation.md).
Inbound putaway comes after the outbound flows; automatic preload distributes the full
1,397,589-unit snapshot plus the explicit 43-unit demo top-up across 162,060 inventory totes, with 8 or 9 units per
tote and mixed SKUs allowed. See [automatic preload](automatic-preload-2026-09-09.md).
Daniel's physical brief
is the configuration authority. The map geometry stays at its reviewed
checkpoint while task structure takes priority over conveyor animation.
The broader world-protocol migration remains parked.

The document records its code baseline and links to relevant implementation.
[The earlier EasyLoop design](../../specs/easyloop/DESIGN.md) is historical;
conflicting assumptions about waiting, operator grouping, and tote handling
are superseded by the current brief.

Temporary Mac demo packaging: [distribution instructions](mac-distribution.md).
