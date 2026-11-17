# State Assertions

Invariants that must hold throughout scheduling. Each is enforced by a
`assert!` or `try_*` check in the code and verified by tests.

## Temporal ordering

| ID | Assertion | Location | Status |
|----|-----------|----------|--------|
| T1 | HTM relay pickup ≥ VTM relay deposit | `assign_rack_path` | **DONE** |
| T2 | VTM Return pickup ≥ HTM relay dropoff (Task B) | `schedule_tote_return` | **DONE** |
| T3 | Tote content set_contents(T) has T > all prior content slices for that tote | tote timeline | Implicit via sorted insert |
| T4 | Tote location move_to(T) has T consistent with bot task timing | tote timeline | **TODO** |

## Physical realism (no teleportation)

| ID | Assertion | Location | Status |
|----|-----------|----------|--------|
| P1 | Every tote on an HTM was either picked up at a relay or was already carried | Task A entry | Implicit via prescreen filter |
| P2 | Every tote returned to rack goes through relay (Task B + VTM Return) | post-planning | **TODO** — pick-only mode skips this |
| P3 | HTM cannot pick up a tote while carrying another | Task A entry | Implicit via carrying filter |
| P4 | Tote location transitions are physical: InRack→Traveling→AtRelay→WithRobot→AtStation (not InRack→AtStation) | tote timeline | **TODO** |

## Resource consistency

| ID | Assertion | Location | Status |
|----|-----------|----------|--------|
| R1 | Relay not double-booked: no overlapping reservation windows for same relay | `try_reserve` / `is_available` | **DONE** — check-then-commit with recheck |
| R2 | Station not double-booked: no overlapping presentation slots | `station_add_slot()` | **DONE** — window-aware `station_next_free` |
| R3 | Bot not double-booked: task queue times are non-overlapping | `assign_task()` | **DONE** — non-monotonic end_ms asserted |
| R4 | Tote content never goes negative | `try_remove_item()` | **DONE** — returns false, check phase uses `sku_qty_at` |

## Completeness

| ID | Assertion | Location | Status |
|----|-----------|----------|--------|
| C1 | Every HTM ends with either a task queued or parked | post-planning | **TODO** |
| C2 | Every pick-mode tote that was retrieved is returned (Task B + VTM Return) | `schedule_tote_return` | **PARTIAL** — returns false on failure, logged as WARN |
| C3 | Every putaway manifest item is scheduled | post-planning | **DONE** (manifest.is_empty()) |
| C4 | Every order line is scheduled for picking | post-planning | **DONE** (pick_count == total_picks) |
| C5 | Putaway demand = order demand (manifest covers all needed SKUs) | manifest construction | **TODO** — verify at build time |

## Commit discipline

All assign functions follow check-then-commit:

1. **Compute** — derive timing from evaluate's chain cost (pure)
2. **Check** — read-only validation of all resources (`is_available`, `sku_qty_at`)
3. **Commit** — `try_reserve` (recheck-on-commit) + `try_remove_item` + bot tasks + tote timeline
4. **Rollback** — on partial commit failure, `release` relay reservations

Zero side effects on failure. Relay `is_available` takes read lock;
`try_reserve` takes write lock and rechecks (handles TOCTOU between
check phase and another station's assign in the same pass).
