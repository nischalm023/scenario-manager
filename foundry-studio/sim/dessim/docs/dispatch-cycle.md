# Full Dispatch Cycle

End-to-end flow from order arrival to tote return.

## Actors

| Actor | Code | Role |
|-------|------|------|
| **OrderManager** | `actors/order/` | Order scoring, inventory reservation, pick scheduling |
| **Dispatcher** | `actors/dispatcher/` | VTM/HTM matching, relay management, tote tracking |
| **VTM** | `robots/vtm/` | Zone-bound tote mover (rack <-> relay) |
| **HTM** | `native-modules/src/migration/flows/ttp/task/htm_*.rs` | Floor-roaming tote carrier (relay <-> station) |
| **OperatorManager** | `actors/operator/` | Station operator queue |

## The Cycle

```
  ORDER ARRIVAL
       │
       v
  1. Order Assignment ─── OrderManager scores orders, reserves inventory,
       │                   generates PickTasks, schedules into station pipelines.
       │
       v
  2. VTM Matching ──────── Dispatcher scores idle VTMs against totes-in-rack.
       │                   Selects relay. Pushes VtmPushAssignment.
       │
       v
  3. VTM Forward ───────── VTM navigates to rack, loads tote, delivers to relay.
       │                   Registers idle on completion.
       │
       v
  4. HTM Matching ──────── Dispatcher scores idle HTMs against totes-at-relay.
       │                   Pushes HtmPushAssignment (relay pre-selected).
       │
       v
  5. HTM Forward ───────── HTM navigates to relay, loads tote, delivers to station.
       │                   Operator picks. Tasks completed.
       │
       v
  6. Return Relay ──────── HTM requests return relay from Dispatcher.
       │                   Least-loaded zone, first available.
       │
       v
  7. HTM Return ────────── HTM navigates to return relay, unloads tote.
       │                   Triggers VTM return matching (back to step 2).
       │
       v
  8. HTM Parking ───────── HTM selects parking cell, navigates there, parks
       │                   with indefinite reservation. Registers idle.
       │
       v
  9. VTM Return ────────── VTM picks up tote at return relay, returns to rack.
                           Registers idle. Tote back InRack. Cycle complete.
```

## Tote Lifecycle

```
  InRack ──[VTM forward]──> WithRobot(VTM) ──[unload]──> AtRelay
    ^                                                        │
    │                                            [HTM picks up]
    │                                                        v
    │                                                   WithRobot(HTM)
    │                                              [station picks]
    │                                                        v
    │                                                   WithRobot(HTM)
    │                                              [HTM returns tote]
    │                                                        v
    └──────────[VTM return]──── WithRobot(VTM) <── AtRelay(return)
```

## Selection Methods Summary

| Step | What is selected | Method | A* reachability? |
|------|-----------------|--------|-----------------|
| 1 | Order -> Station | score_order_for_station (manhattan) | No |
| 2 | VTM for tote | find_closest_robot (manhattan + A* top-5) | Yes |
| 2d | Relay for VTM cycle | select_optimal_relay_for_vtm (manhattan) | **No** |
| 4 | HTM for relay | find_closest_robot (manhattan + A* top-5) | Yes |
| 6 | Return relay | select_relay_for_return (zone load) | **No** |
| 8 | Parking cell | select_parking_cell (manhattan) | **No** |

## Triggering

The cycle is self-sustaining via idle registration:

- VTM completes -> `register_idle_vtm` -> Dispatcher runs `try_match_vtm`
- HTM completes -> `register_idle_htm` -> Dispatcher runs `try_match_htm_work`
- `complete_tasks_batch` -> may trigger `assign_orders_to_stations`

Initial kickoff:
1. `OrderManager::process_orders()` dispatches first batch
2. VTMs and HTMs call `start()` -> register idle
3. Dispatcher matching activates on first idle registration

---

## Step Details

### 1. Order Assignment (OrderManager)

**Entry:** `assign_orders_to_stations()` [assignment.rs]

Two-phase dispatch per station:
- Phase 1: breadth-first fill to `num_bins` distinct orders per station.
  Overshoot capped at `max_inflight_picks + num_bins`.
- Phase 2: speculative pre-dispatch with hard cap at `max_inflight_picks`.

For each order dispatched:
1. Score pending orders for station [dispatch_order.rs:91] --
   proximity (manhattan to nearest SKU storage)
2. Reserve inventory [dispatch_order.rs:16] --
   greedy set-cover across storage spaces (all-or-nothing)
3. Generate PickTasks [dispatch_order.rs:66]
4. Schedule picks in LFF order [dispatch_order.rs:206] --
   Longest-Fetch-First, `queue_factor` target arrival, P-spaced pipeline

Output: PickTasks with `target_station` and `storage_space_id`.

### 2. VTM Matching (Dispatcher)

**Entry:** `try_match_vtm()` [matching/vtm.rs]

Unified forward + return scoring in one pass:

**Forward candidates:** totes InRack with unclaimed tasks.
Deadlock-blocked zones excluded (raw occupied+reserved within
`deadlock_headroom` of capacity).

**Return candidates:** totes at relays with no remaining tasks.

Scoring (parallel rayon):
1. Filter idle VTMs to candidate's zone
2. `find_closest_robot` -- manhattan prescreen, A* on top-5 candidates
3. `score_vtm_work` -- composite of proximity, zone pressure, forward
   vs return priority, in-flight VTM count

Sequential claim of best-scored candidates. For forward:
- **Select relay** via `select_optimal_relay_for_vtm_inner` [relay_ops.rs:106]:
  `0.6 * vtm_manhattan + 0.4 * htm_manhattan`. No A* reachability check.
- Reserve relay atomically, claim tote, push `VtmPushAssignment`

### 3. VTM Forward Cycle (VTM actor)

**Entry:** `receive_vtm_assignment()` [vtm/mod.rs:83]

State machine:
```
NavToRack -> arrive_at_rack -> LoadingRack -> loaded -> NavToRelay
  -> arrive_at_relay -> Unloading -> unloaded -> Idle (register)
```

- NavToRack: A* path VTM pos -> `rack_vtm_access` (reservation-aware,
  zone-gated)
- LoadingRack: `load_time` duration, `move_tote: InRack -> WithRobot`
- NavToRelay: A* path rack -> `relay_vtm_access` (carrying cargo)
- Unloading: `unload_time`, `move_tote: WithRobot -> AtRelay`,
  release relay + tote claim, register idle

### 4. HTM Matching (Dispatcher)

**Entry:** `try_match_htm_work()` [matching/htm.rs]

Candidates: totes AtRelay with unclaimed tasks.

Parallel scoring: `find_closest_robot` to relay's `htm_access`
(manhattan prescreen, A* on top-5). Scores HTM-to-relay distance but
does NOT verify the relay is reachable from the HTM.

Sequential assignment: claim tote, push `HtmPushAssignment` with
pre-selected relay. The HTM does not choose its relay.

### 5. HTM Forward Cycle (HTM actor)

**Entry:** `receive_htm_assignment()` [htm/mod.rs:110]

State machine:
```
[release parking] -> NavToRelay -> arrive_at_relay -> Loading -> loaded
  -> NavToStation -> arrive_at_station -> [bin gate] -> [operator queue]
  -> Picking -> picks_complete -> [release operator]
```

- Release parking cell if parked. Parking reservation shortened
  automatically by `reserve_path` first-cell replacement.
- NavToRelay: A* path HTM pos -> `source_relay_htm_access`
  (reservation-aware, not carrying cargo)
- Loading: `load_time`, `move_tote: AtRelay -> WithRobot`
- NavToStation: A* path relay -> `station_access` (carrying cargo)
- Bin gate: `notify_arrival` -> OrderManager. May wait for bin.
- Operator: `request_operator` -> OperatorManager queue
- Picking: `processing_time`, then `process_tasks` -> Dispatcher,
  `complete_tasks_batch` -> OrderManager, `release_operator`

### 6. Return Relay Selection (HTM -> Dispatcher)

**Entry:** `select_return_relay` [htm_ops.rs:110]

1. Sort zones by load (`occupied + reserved*discount / capacity`)
2. For least-loaded zone, find first unreserved, empty relay
3. Reserve atomically, return `ReturnReservation`

No distance or A* reachability check. On failure: retry with
exponential backoff (navigate toward source relay on first failure).

### 7. HTM Return Cycle (HTM actor)

**Entry:** `select_return_relay` callback [htm/mod.rs:248]

State machine:
```
NavToReturnRelay -> arrive_at_return_relay -> Unloading -> unloaded
```

- NavToReturnRelay: A* path station -> `return_relay_htm_access`
  (carrying cargo)
- Unloading: `unload_time`, `move_tote: WithRobot -> AtRelay`,
  release relay + tote claim

Tote is now at return relay with no tasks. Triggers VTM return
matching (step 2).

### 8. HTM Parking (HTM actor)

**Code:** [htm/mod.rs:320-361]

Immediately after unloading at return relay:
1. `select_parking_cell` from `ParkingRegistry` (nearest unoccupied
   candidate by manhattan distance)
2. If already on a good cell: park immediately
3. Otherwise: `NavToParking -> arrive_at_parking`
4. `reserve_parking`: indefinite hold (`exit_ms = u64::MAX`)
5. `register_idle` -> available for next HTM matching

### 9. VTM Return Cycle (VTM actor)

**Entry:** `receive_vtm_assignment(Return)` [vtm/mod.rs:97]

Triggered by VTM matching (step 2) when it finds a tote at a relay
with no remaining tasks.

State machine:
```
NavToRelayReturn -> arrive_at_relay_return -> PickingUpReturn
  -> picked_up_return -> NavToRackReturn -> arrive_at_rack_return
  -> UnloadingReturn -> unloaded_return -> Idle (register)
```

- NavToRelayReturn: A* path VTM pos -> `relay_vtm_access`
- PickingUpReturn: `load_time`, `move_tote: AtRelay -> WithRobot`
- NavToRackReturn: A* path relay -> `rack_vtm_access` (carrying cargo)
- UnloadingReturn: `unload_time`, `move_tote: WithRobot -> InRack`,
  release tote claim, register idle

Tote is back InRack. Cycle complete.
