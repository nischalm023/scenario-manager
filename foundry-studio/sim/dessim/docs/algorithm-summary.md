Directory of algorithms per function:

## Order Choreography (Dispatch)

**Push-based, event-driven dispatch with coordinated VTM+HTM pipeline.** When an HTM departs a station, immediately score pending orders by proximity and dispatch the best match. The full fetch cycle coordinates both robots: VTM delivers tote to relay timed for HTM arrival, HTM delivers to station timed for pick completion. Physical bins and relay positions are claimed at arrival, not reserved at dispatch. Inventory is reserved at assignment time (Phase 1); HTMs committed at dispatch time (Phase 2, JIT). Queue factor (f64) controls arrival timing at station. 1.0 = next HTM arrives as current departs. Not count-based. Two-phase dispatch: Phase 1 assigns orders to stations and reserves inventory. Phase 2 dispatches HTMs at computed times (JIT). Multi-pick sequencing: Longest-Fetch-First (LFF) ordering, staggered dispatch.

See `specs/simulator/station-order-assignment.md` for full spec.

## Station Scoring (Proximity)

**Nearest-to-station per SKU.** For each (SKU, station) pair, find the storage space with minimum Manhattan distance that has available stock. Sum of min_manhattan for station assignment (total travel cost); max of min_manhattan for F_est (critical-path fetch time). Precompute at init; invalidate when `available() == 0` for a (SKU, space) pair after reservation.

See `docs/domain/station-order-scoring.md` for rationale and analysis.
