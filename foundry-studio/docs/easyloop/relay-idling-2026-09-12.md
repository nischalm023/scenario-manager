# Relay idling — September 12, 2026

Daniel's ruling after speaking with Vinay applies to every scenario: parking cells do not exist. HTMs start at randomly chosen relay presentation cells or explicit charging cells. Later idle relocation uses available relay cells; charge-dependent dispatch is future work.

## Implemented

- Removed the parking cell kind, markers, authoring tool and automatic generation. Existing parking strips remain ordinary floor; no chargers were inferred from them. Explicit charging infrastructure has a charging marker and editor support.
- Startup samples without replacement using `settings.rng_seed`. Distinct usable relay access cells must strictly outnumber HTMs. Charging cells join the initial sample but do not relax relay capacity. Zero HTMs needs no relay capacity.
- `settings.htm.idle_selection` accepts `nearest` (default) or `random`. Local selection refines nearby candidates with travel estimates; random selection preserves its seeded ordering through physical route attempts. Both honor floor reservations and retain the original hold if no move can be committed.
- The deferred `Parking` task remains an action. Productive claims take priority before it plans a route. Removed the unused `ClearBufferAisles` task, whose policy of clearing idle robots from relays contradicts the new resting policy.
- Reporting recognizes relay access through relay metadata even when the underlying floor cell is Open. Chargers remain identifiable separately.
- Reduced the small EasyLoop flow fixture from 12 to 10 HTMs because its 11 distinct relay access cells cannot accommodate the former fleet plus a spare. Full Walmart remains 144 HTMs and 48 VTMs.

The usable-cell count checks positions and incoming/outgoing HTM edges. Global route connectivity is checked separately by map and pathfinding validation; the count alone does not prove every destination is reachable.

## Validation at the first WIP checkpoint

- Simulation library: 917 tests passed; two existing ignored.
- Small EasyLoop: 22 fulfillment checks plus both lane/workflow checks passed, including all 24 picks and 12 extractions.
- Cencora pick sample: 20/20 picks. Used a temporary 60-unit preload matching its 60 slots; the existing 200-unit/one-unit-per-tote fixture otherwise fails inventory completeness before starting.
- Map, graph and editor tests: 188 passed, three existing ignored. Converter: 27 passed, including heavy golden checks. Frozen Cencora conversion ledger preserved.
- Map/parity tests: 31 passed. Former floor-strip cell identities preserved; Cencora's old parking-related traversal differences disappear. Its existing 74 station-footprint differences remain.
- Scenario generation: 46 passed, one existing external-extract test ignored.
- World-protocol and worldlens marker/schema suites passed. Six stale geometry census assertions were independently recounted and corrected; all ten affected census/pattern tests passed.
- Release Desvis build and default Crucible check passed. The optional legacy Windowed pathfinder still has pre-existing compile errors from an outdated oracle call and missing trait method; it is not used by these runs. Crucible UI dependencies are absent locally, so its TypeScript build was not run.

## Full-map issue under investigation

The 100-order full-map check stalled after 94 picks/orders at about 13.5 simulated minutes. Headed iteration 11, all 22,160 orders available at T0, also stalled (113 picks/orders at about 18.5 minutes). Both use collision avoidance.

The scheduler reports accepted tasks waiting locally, with no live proposal competition. `HtmStore` waits for an occupied relay access cell without requesting relocation when the occupant is an available idle HTM. Relay-based startup and idling expose that missing handoff. A focused reproduction and correction are in progress; this checkpoint is not a validated throughput improvement.

Local run evidence: `/Users/daniel/vt/greyorange/notes/relay-idling-2026-09-12/`.

## Storage handoff correction

`HtmStore` now creates a deferred `Parking` child when its selected relay is occupied by an empty, available HTM. Productive claims still compete first; the loaded returning HTM retains its source reservation while waiting. Failed relocation snapshots notifications after restoring the idle claim, preventing its own take/restore from causing repeated retries.

The new regression failed before the change and passes afterward: the idle resident relocates to another relay, the tote reaches the rack, the source hold closes in the oracle, and there are no overlapping reservations. All four storage tests, 22 parking/competition tests and 24 EasyLoop workflow/lane tests pass.

Repeating the exact 100-order full-map check now completes **100 picks and 100 orders at 821.614 simulated seconds** (79.955 wall seconds). This confirms the reproduced deadlock is resolved; it does not establish sustained eight-hour throughput. Headed iteration 12 runs the corrected build with all 22,160 orders available at T0 for an eight-hour limit.
