# Temporal State Surfaces

Both categories below are temporal. The distinction is whether a scheduler may
search interior time gaps, or must schedule monotonically from the resource's
current frontier.

## Rules

- **Full temporal** stores enough history to answer as-of or interval queries.
  Use it only where filling a finite gap is a valid scheduling move.
- **Frontier-only** stores the latest usable state plus a time frontier. It may
  remember `last_write_at`, but it cannot reconstruct interior gaps.
- **Open-ended holds are frontier-only while open.** Their eventual end is not
  known, so no scheduler may plan through the hold until the valid successor
  finalizes it.
- The offline planner's default posture is **go forward without creating
  gaps**. If scheduling would create a gap, abstain unless a local pressure rule
  explicitly permits an exception or motivates a legal unblocking action.

## Full Temporal: Gap Queries Allowed

- **`ReservationOracle` / `ReservationTable`**
  ([`model/reservation/`](../src/model/reservation/))
  - Full interval history for cell and edge reservations.
  - Pathfinder may search finite safe intervals and wait inside finite gaps.
  - Open-ended predecessor/destination holds are not gap-fillable until a
    successor closes them.

- **`InventoryService`**
  ([`model/inventory/`](../src/model/inventory/))
  - Tote location and contents are time-sliced step functions:
    `location_at(T)`, `contents_at(T)`, `is_empty_at(T)`, `has_sku_at(T)`.
  - SKU availability is temporal through `SkuLedger`.
  - Use this as the source of truth when a demand module must know whether
    inventory or an empty tote exists at a future `SimTime`.

- **`SkuLedger`**
  ([`model/inventory/sku_ledger.rs`](../src/model/inventory/sku_ledger.rs))
  - Temporal counters for four-wall inventory and soft reservations.
  - Supports future order-readiness checks such as
    `earliest_sku_qty_available`.

- **`TemporalCounter`**
  ([`utils/temporal_counter.rs`](../src/utils/temporal_counter.rs))
  - Step-function counter with `count_at(T)`, finite spans, and open spans.
  - Finite spans can support admission-window checks. Open spans behave like a
    frontier until closed.

- **`BotPlan`**
  ([`model/bot/plan.rs`](../src/model/bot/plan.rs))
  - Position is queryable at arbitrary `SimTime` for observation and path
    reasoning.
  - Allocation of new HTM work still goes through `AvailableHtmRegistry`; do
    not backfill work into historical bot-idle gaps from `BotPlan`.

- **`TemporalSet<K>`**
  ([`utils/temporal_set.rs`](../src/utils/temporal_set.rs))
  - General full-temporal set. Currently an upgrade target, not a primary
    production surface.

## Frontier-Only: Schedule Forward

- **`FrontierSortedSet<K, V>`**
  ([`utils/frontier_sorted_set.rs`](../src/utils/frontier_sorted_set.rs))
  - Lossy temporal index: latest active membership plus `last_write_at`.
  - Stale writes are ignored; interior membership history is unavailable.

- **`RelayInventoryIndex`**
  ([`model/relay/inventory_index/`](../src/model/relay/inventory_index/))
  - Built on `FrontierSortedSet` for `empty_totes`, `sku_totes`, and
    `empty_slots`.
  - Answers "what is the best current frontier candidate per relay?"
  - Cannot answer "what candidate is available inside an earlier/later gap?"
  - Schedulers using it must start at or after the candidate frontier, or
    abstain. If a future materialization must be considered, query
    `InventoryService` or upgrade this index.

- **`RelayService` open-ended relay reservations**
  ([`model/relay/`](../src/model/relay/))
  - Relay slots have timelines, but unfinalized reservations are frontier
    barriers.
  - The N-2 direction headroom rule is a hard safety gate, not a throughput or
    gap-filling policy.
  - `schedule_pressure` / `direction_pressure` should motivate consumers that
    finalize existing holds before the hard gate blocks new reservations.

- **`AvailableHtmRegistry`**
  ([`model/htm/registry.rs`](../src/model/htm/registry.rs))
  - Current claimable HTM frontier: open-ended cell hold, tote state, and
    `action_ready_at`.
  - New work starts from this frontier. Historical idle windows are not
    schedulable.

- **`OrderReadyQueue`**
  ([`native-modules/src/migration/flows/ttp/demand/order_queue.rs`](../../../native-modules/src/migration/flows/ttp/demand/order_queue.rs))
  - Frontier bookkeeping for ready, blocked, picking, and complete orders.
  - It may consult temporal inventory to promote orders, but the queue itself
    is not a temporal order-history index.

- **`ParkingRegistry`**
  ([`model/parking.rs`](../src/model/parking.rs))
  - Parking-cell frontier/occupancy registry for idle parking choices.
  - Do not use it as a generic gap-filling reservation table.

## Classification Tests

- Ask **"may a scheduler legally use a finite interior gap here?"**
  - Yes: full temporal surface or query required.
  - No: frontier-only surface is appropriate.
- Ask **"does an open-ended hold participate?"**
  - Yes: treat that resource as unavailable until its successor finalizes.
- Ask **"would this decision create an avoidable gap?"**
  - Yes: abstain under Normal pressure.
  - Under pressure: use a scoped exception or a legal unblocking successor, not
    broad recovery/rollback.

## Current Watch Items

- `RelayInventoryIndex.empty_slots` reports `available_at = SimTime::ZERO`.
  If deposit scheduling needs real future slot frontier times, this field must
  become meaningful or the scheduler must query `InventoryService` directly.
- Empty-tote recycling through picks is a policy decision:
  - If putaway must plan against future recycled totes, the relay empty-tote
    frontier is insufficient.
  - If putaway should only consume currently-frontier empty totes, then false
    completion with putaway backlog must report a clear blocked-by-empty-tote
    state rather than clean completion.
