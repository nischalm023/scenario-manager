# Direct presentation links — iterations 14 and 15

Rollback checkpoint: `a1944e245` (`walmart`, pushed). This experiment is a separate
commit so it can be reverted without undoing the two independent waiting blocks.

Removed the four `turn:{side}:in/out` helper cells from every EasyLoop in the
full and minimal maps. Each descending lane's bottom cell now connects directly
to its presentation; the presentation connects directly to the bottom return cell.
For EasyLoop 1's left block, this is `26fa61da → 005dd14c → 452f1cc5`.
Each machine now has 52 declared cells instead of 56.

Presentation positions, headings, circular radius and dwell points are unchanged.
The two inner travel lanes are 226.6 mm off their circular tangents. The new
opt-in `smooth_leads` curve setting joins those endpoints with a gentle lateral
transition whose endpoint headings match the circle. Leads contribute their actual
curved length to routing and playback; older straight leads retain their meaning.

Removed the two order-approach queue declarations. Only the entering edge of a
curved pick presentation is destination-gated, so the last circulation row retains
its waiting-cell access. The inventory queue, twelve waiting cells, fixed
presentation assignment, apron geometry and surrounding floor are unchanged.

Validation: 33 map/motion/flow integration tests, seven curve-math tests, seven
protocol curve tests and 85 protocol unit/schema checks pass. The small workload
completes all twelve orders, uses both presentations, and keeps every order on its
assigned side. This validates the layout experiment; it does not establish a full-map
throughput improvement. Temporal area admission is still pending.

Iteration 14 loads the main Walmart spec without `--run`. Iteration 13 remains
available for visual comparison.

## Iteration 15: straight approaches

Daniel rejected the lateral jog in iteration 14 and accepted the recommended
slightly wider turn. The centreline radius is now 700 mm; the conservative
700 mm square robot sweeps an outer diameter of approximately 2.214 m.
Presentation positions and 135°/45° phases remain unchanged. Both complete
entry/return columns now meet their circular links on straight tangents.

The full map's apron is retiled around all 26 existing door connections,
including box induction, preserving independent straight approaches. It uses
112 columns (previously 104), each at least 695 mm wide. Door references are
remapped explicitly. Machine envelopes extend 26.7 mm toward the operator;
all unrelated floor, relay, fleet and workflow entities remain unchanged.
The minimum tile gap between the inventory queue and the adjacent order lane
is 13.1 mm; the 700 mm robot envelope is a conservative modelling estimate.

The `smooth_leads` extension has been removed from the protocol, graph math,
authors and fixtures. Ordinary circular links with straight tangent leads
express the entire path. Direct presentation links, destination gates, all
52 machine cells and the two independent waiting blocks remain.

Iteration 14 is preserved at `bfc823ad6`; the earlier helper-cell layout is at
`a1944e245`. Each experiment is a separate WIP commit for easy rollback.

Iteration 15 validation: 33 map/motion/flow integration tests, two curve-math
tests, six protocol curve tests and 85 protocol unit/schema checks pass.
The fixture updater is idempotent, unrelated entities remain byte-identical,
and the Desvis release build succeeds. The main-spec inspection launch has
no `--run`; throughput has not been measured for this geometry revision.
