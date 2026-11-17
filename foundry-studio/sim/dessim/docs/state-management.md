# State Management

How data is owned, shared, and indexed across the model and service layers.

## Structs at a glance

| Struct | Type | State it owns | Fundamental record |
|--------|------|--------------|-------------------|
| `InventoryService` | Service (`Arc<Inner>`) | All totes: contents timeline, location timeline | `ContentSlice { from_ms, contents: Vec<(SkuId, u32)> }`, `LocationSlice { from_ms, location: ToteLocation }` |
| `Tote` | Handle into `InventoryService` | None — reads/writes through `Arc<Inner>` | — |
| `BotService` | Service (`Arc<Inner>`) | All bots: task queues, position, carrying state | `BotTask { legs, estimated_end_ms, end_pos, carrying_at_end, target_station }` |
| `OrderService` | Service (`Arc<Inner>`) | All orders: status, station assignment, line progress | `OrderData { status, station, lines: Vec<LineData { sku, quantity, available }> }` |
| `Order` | Handle into `OrderService` | None — reads through `Arc<RwLock<State>>` | — |
| `RelayService` | Service (`Arc<Inner>`) | Relay reservations, precomputed distances | `Reservation { start_ms, end_ms, bot_id }` |
| `StationSet` | Immutable collection | Station identity and geometry (no mutable state) | `Station { id, station_type, presentation_cell, pos, queue_path, exit_direction }` |
| `Station` | Handle (`Arc<Inner>`, `Deref`) | None — immutable fields via Deref | — |
| `BotDispatch` | Orchestrator | Station presentation queue, scheduled work counts | `PresentationSlot { start_ms, end_ms }` |

All services are `Clone` via `Arc` bump. Handles (`Tote`, `Order`) are
ephemeral — constructed on demand, never stored.

## Temporal invariant

**No model may hold single-point state for any time-varying quantity.**
Every resource that changes over simulated time uses an append-only
temporal structure — a time-sliced step function or a window list.
Point queries at arbitrary sim time T use binary search or window scan.

| Resource | Temporal structure | Point query | Availability query |
|----------|--------------------|------------|-------------------|
| Tote contents | `Vec<ContentSlice>` step function | `contents_at(T)` | `find_nearest_with_sku(sku, near, T)` |
| Tote location | `Vec<LocationSlice>` step function | `location_at(T)` | `find_nearest_empty_tote(near, T)` |
| Relay occupancy | `Vec<Reservation>` windows | `relay_available_at(T)` | `find_nearest_available(station, T)` |
| Station occupancy | `Vec<PresentationSlot>` windows | — | `station_next_free(station, after_T)` |
| Bot trajectory | `VecDeque<BotTask>` queue | — | `availability(T)` — after last task |

Bot task queues are implicitly temporal — the ordered queue of future
tasks with `estimated_end_ms` defines the bot's trajectory. The task
queue IS the timeline.

### Temporal structure variants

**Tote content/location** use implicit-end step functions: each slice
ends when the next begins. Append-only, no back-mutation. Adding explicit
`end_ms` would enable gap-filling for replanning.

**Relay reservations and station slots** use explicit-window models:
`[start_ms, end_ms)` per reservation. This naturally supports gap-filling
— new reservations can be inserted between existing ones. The
`is_available` / `try_reserve` check-then-commit pattern validates
windows against existing reservations before committing.

---

## Indexes

| Index | On | Kind | Shape | Updated by |
|-------|------|------|-------|-----------|
| `sku_totes` | `InventoryService` | Maintained reverse | `BTreeMap<SkuId, BTreeSet<ToteId>>` — totes that *ever* contained a SKU | `add_item()`, `set_contents()` |
| `station_task_count` | `BotService` | Maintained counter | `HashMap<StationId, u32>` — tasks targeting each station | `assign_task()`, `complete_current_task()` |
| `station_last_arrival` | `BotService` | Maintained max | `HashMap<StationId, u64>` — latest `estimated_end_ms` per station | `assign_task()`, `complete_current_task()` |
| `zone_vtms` | `BotService` | Immutable | `HashMap<ZoneId, Vec<RobotId>>` — VTMs per zone | `register()` at init |
| `sku_demand` | `OrderService` | Immutable | `BTreeMap<SkuId, Vec<OrderId>>` — orders needing each SKU | `add_orders()` at init |
| `station_orders` | `OrderService` | Maintained reverse | `HashMap<StationId, BTreeSet<OrderId>>` — orders per station | `assign_orders_to_stations()`, `soft_allocate_item()` |
| `station_load` | `OrderService` | Maintained counter | `BTreeMap<StationId, u32>` — order count per station | `assign_orders_to_stations()`, `soft_allocate_item()` |
| `station_relay_dist` | `RelayService` | Immutable | `BTreeMap<StationId, Vec<StationRelayEntry>>` — relays sorted by distance | Init |
| `relay_station_dist` | `RelayService` | Immutable | `HashMap<(RelayId, StationId), u32>` — O(1) distance | Init |
| `pick` / `putaway` | `StationSet` | Immutable | `Vec<Station>` — stations by type | Init |

No eager `empty_totes_in_rack` index exists. `find_nearest_empty_tote`
scans all totes temporally. Future optimization candidate.

---

## Service details

### InventoryService

Single `RwLock<ToteState>`. Tote contents and location are time-sliced
step functions — each entry is a full snapshot from `from_ms` until the
next entry. Binary search for point queries.

Mutations: `add_item(sku, qty, at_ms)`, `remove_item(sku, qty, at_ms)`,
`set_contents(contents, at_ms)`, `clear_contents(at_ms)`,
`move_to(location, at_ms)`. All append-only.

`find_nearest_with_sku(sku, near, T)` walks `sku_totes[sku]`, checks
`sku_qty_at(T) > 0` and `location_at(T) == InRack`, scores by manhattan,
partial-sorts for top K.

### BotService

Single `RwLock<State>`, partitioned into `htms` and `vtms` (both
`BTreeMap<RobotId, BotData>`) so filtered searches scan only the
relevant population.

`availability(sim_time)` projects `(time, cell, grid_pos)` from the
last queued task. `carrying_at_available()` returns the last task's
`carrying_at_end`. The task queue IS the temporal model.

`LegAction` variants: `PickPresent`, `PutawayPresent`, `RelayDropoff`,
`RelayPickup`, `Park`, `VtmForward`, `VtmReturn`.

### OrderService

`Arc<RwLock<State>>` with `orders: BTreeMap<OrderId, OrderData>`.
Order lifecycle: `Unassigned → Assigned → Ready → Picking → Complete`.

`station_work_queue(station_id)` returns `Vec<(OrderId, SkuId, qty)>`
using the `station_orders` index.

### RelayService

Immutable relay metadata + distance tables. Mutable state is only
`reservations: BTreeMap<RelayId, Vec<Reservation>>` behind `RwLock`.

`reserve(relay, start, end, bot)` appends a window.
`relay_available_at(T)` walks windows, pushes past overlaps.

### BotDispatch

Holds cloned refs to all five services plus scheduling state:
- `station_queue`: temporal presentation slots per station
- `scheduled_counts`: work-item dedup counters (monotonic in FullPreplan)
- `pathfinder`: optional A* for travel times (manhattan fallback)
- `layout`: relay/station CellId mappings
