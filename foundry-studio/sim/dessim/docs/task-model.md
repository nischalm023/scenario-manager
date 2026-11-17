# Task Model

Every bot action is a `BotTask` with one or more `TaskLeg`s. Each leg
has a destination, estimated duration, and a `LegAction`. The task's
`carrying_at_end` and `end_pos` define the bot's state after completion.

## Resource binding: three mechanisms

Tasks receive resources through exactly three mechanisms, each with
distinct semantics for **when** the binding happens and **who** decides.

### 1. Constructor args (demand generator decides)

Static intent decided at chain creation time. These define WHAT the task
wants to accomplish — not specific bots, totes, or cells.

Examples: `station_id`, `PresentationOp::Pick { sku, qty }`, `zone_id`
(placeholder). The demand generator knows the intent but not the specific
resources that will fulfill it.

A task's constructor args never include a specific `RobotId` or `ToteId`.
Those are always bound at schedule time (mechanisms 2 or 3 below).

### 2. Upstream results (the parent task decides)

A parent task awaits one child's result (`ctx.add_task(child).await`, the
child's `schedule` output, decided when that child is scheduled) and passes
it to the next child's constructor. The task doesn't know or care whether
the value came from an upstream task's work or from the parent's own
choice.

**Peek-then-take discipline**: a task's `propose` peeks the value and does
speculative work (path estimation, scoring); its `schedule` takes it, once
the proposal is accepted. An evicted proposal leaves the value in place for
the next one.

Examples:
- `RelayReservation<ToteId>` — relay info from VtmForward, consumed by
  HtmPickup.
- `HtmCarryingTote` — HTM + tote from HtmPickup (standard chain) OR from
  a productive claim on an HTM standing at a station (direct delivery).
  HtmDeliver doesn't know which case it is.

### 3. Registry peek/take (this task decides)

The task itself selects a resource at schedule time from a shared pool.
Used when the selection depends on information only available at schedule
time — e.g., which HTM is nearest to the relay just discovered from the
upstream result.

**Only one task uses this**: `UnladenHtmInbound`. It takes an HTM from
the `AvailableHtmRegistry` because HTM selection is coupled to the relay
location (from the upstream result). The demand generator can't
pre-select the HTM because VtmForward hasn't run yet.

On Decline, the task restores the entry to the registry.

All other tasks receive their resources as upstream results (mechanism
2). Demand generators that know which HTM to use (e.g., direct
delivery) take from the registry themselves and hand it to the child as
its upstream result.

### Summary table

| Mechanism | Who decides | When bound | Examples |
|-----------|------------|------------|----------|
| Constructor | Demand generator | Chain creation | station_id, PresentationOp |
| Upstream result | Upstream task or demand generator | Upstream's schedule time | RelayReservation, HtmCarryingTote |
| Registry | This task | This task's schedule time | UnladenHtmInbound selects HTM |

## Scheduling model: Eager VTM Dispatch, JIT HTM Arrival

The FullPreplan solver schedules all tasks at T=0 using a work queue
(UOW accumulator) that decouples VTM and HTM scheduling symmetrically.
This approximates how a reactive real-world dispatcher issues independent
work items to VTMs and HTMs — computed all at once for wall-clock efficiency.

### Inventory starting state

Inventory starting state depends on configuration:

- **Putaway disabled**: All totes are pre-populated with SKUs and placed
  in rack at T=0. Pick demand can find any SKU immediately.

- **Putaway enabled**: Totes start EMPTY in rack. The putaway manifest
  defines which SKUs need to be loaded onto totes. Putaway must complete
  the full cycle (empty tote → putaway station → operator loads SKU →
  tote returns to rack) before a pick can find that SKU. Pick and putaway
  scheduling are interleaved — there is no separate "putaway phase."

### Task graph

The TaskGraph (`scheduler/mod.rs`) is the authoritative source for task
topology and lifecycle: a task added with `add_task` publishes a proposal,
and the global barrier accepts one proposal per acceptance round
(`docs/global-barrier.md`).

### Task chains

Chains are built incrementally by demand modules, not as monolithic
sequences. The chain topology depends on the demand module's decision
at creation time:

**Full chain (tote from rack via relay):**
```
VtmForward → UnladenHtmInbound → HtmDeliver → Presentation
```

**Direct chain (HTM already carrying a tote):**
```
HtmDeliver → Presentation
```

**Return chain (post-presentation tote to rack):**
```
HtmStore → VtmReturn
```

The full chain and direct chain are created by demand modules
(`FeedPutawayStations`, `OrderPickDemand`). The return chain is created
by the presentation's clearance (`ClearPresentation`, `native-modules/src/migration/flows/ttp/task/clearance/`)
when no productive claim takes the HTM. Between chains, the `AvailableHtmRegistry`
holds idle HTMs with their tote state. Presentation registers the HTM,
and its operation's owner spawns the clearance that contests the HTM's
stay with any productive claim (a demand module's direct chain).

#### Task roles

- **VtmForward**: VTM retrieves tote from rack → deposits at relay.
  Root task (no predecessor). Selects tote + relay at schedule time.
  Sends `RelayReservation<ToteId>` downstream as its result.

- **UnladenHtmInbound**: Empty HTM travels to relay, picks up tote.
  Receives relay info as its upstream result. Selects nearest HTM from
  `AvailableHtmRegistry` at schedule time (mechanism 3 — registry take).
  Sends `HtmCarryingTote` downstream as its result.

- **HtmDeliver**: HTM carries tote to station. Receives `HtmCarryingTote`
  from its parent (mechanism 2 — from HtmPickup, or from a productive
  claim for direct delivery). Commits its reservation-aware path in one
  `reserve_path` call from the handoff's ready instant. While another HTM
  holds the station's cell open-ended, the pathfinder ends a window on the
  free queue cell nearest the station that its search reaches, the HTM
  drives there and waits
  (simtime), and the call awaits the holder's departure being committed
  (scheduler time) before it commits the move up. Returns the
  `HtmCarryingTote` at the station to its parent.

- **Presentation**: Inventory mutation at station (pick or putaway). No
  pathfinding. Receives `HtmCarryingTote` as its upstream result.
  Registers the HTM's `CellReservation` + tote state into
  `AvailableHtmRegistry` on completion.

- **HtmStore**: HTM carries tote from station → relay for VTM return.
  Its parent reserves the deposit relay before handing the HTM to it: a
  putaway fill reserves its tote's relay before the fill is committed and its
  clearance carries it; another clearance's return (`ReturnClearedTote`)
  reserves one when it is decided.

- **VtmReturn**: VTM picks tote from relay → returns to rack slot.

### Demand modules (Phase 7)

Two demand modules create task chains:

- **FeedPutawayStations**: keep putaway stations fed with empty totes.
  Direct delivery (post-pick HTM with empty tote) preferred; falls back
  to full chain from rack.
- **OrderPickDemand**: bring SKU-containing totes to pick stations.

A finished presentation's tote goes back to the rack through the
clearance its operation's owner spawns, not through a demand module.

### Scheduling loop

Tasks publish proposals, and the scheduler accepts one per acceptance
round as the global barrier advances, each at its decision deadline or in
a contest its resource family ranks (`docs/global-barrier.md`).

Putaway retrievals are generated first: they create supply (laden totes)
that pick retrievals consume. Both compete for VTMs in the same zones
but not for the same totes (empty vs laden).

### Four-wall inventory and SKU reservation

InventoryService maintains two location-independent SKU-level counters.
These track what exists in the warehouse and what is spoken for,
regardless of where individual totes are physically located.

**`sku_inventory[sku]`** — total physical units of SKU in the warehouse.
Does not track WHERE (in rack, on VTM, at relay, on HTM, at station).
Just that the unit has been received (putaway) and not yet consumed
(picked from a tote).

- Putaway completes (tote.set_contents at presentation): **+1**
- Pick consumed (tote.try_remove_item at presentation): **-1**
- Tote movement (VtmForward, HtmDeliver, HtmStore, VtmReturn): **no change**

Initialized at BotDispatch construction from the inventory map. With
putaway disabled: count = pre-populated tote contents. With putaway
enabled: all counts start at 0 (totes are empty).

**`sku_reserved[sku]`** — units claimed by in-flight orders that have
not yet been picked. This is a priority mechanism: in-flight orders
get first dibs on inventory, preventing new orders from consuming
inventory that in-flight orders need to complete.

- Order enters picking state (first pick dispatches): **+N** for each
  SKU the order needs (all lines reserved at once)
- Pick consumed (tote.try_remove_item): **-1** (reservation fulfilled)

**Available for new orders** = `sku_inventory[sku] - sku_reserved[sku]`.
An order can only enter picking state when this is >= needed for every
line in the order. This prevents two orders from splitting scarce
inventory — one order cannot start if completing it would starve an
already-in-flight order.

Both counters are temporal step functions (same pattern as `content_log`
and `location_log` on ToteData). This is required because the system is
an offline solver — there is no "now." Different task chains are at
different points in sim time simultaneously: a putaway may complete at
T=300s while a pick targets T=150s. Scalar counters would conflate
these, reporting inventory that doesn't exist at the query time.

Each counter is a sorted `Vec<(from_ms, delta)>` per SKU. Point queries
use binary search to compute the cumulative count at any sim time.
Insertions maintain sorted order (same `insert_sorted` pattern as
existing temporal logs). Queried at the order's commitment time (first
VtmForward's `after_ms`) when checking order readiness — all lines
checked at the same T, all reservations placed atomically at that T.

### Order lifecycle and dispatch

Orders progress through a lifecycle that ensures completion priority:

```
Blocked → Ready → Picking (in-flight) → Complete
```

**Blocked**: at least one SKU in the order has insufficient available
inventory (`sku_inventory - sku_reserved < needed`). The order is
indexed by its blocking SKUs for efficient re-check when inventory
changes.

**Ready**: all SKUs have sufficient available inventory. The order is
in the dispatch queue, waiting for a scheduling turn.

**Ready → Picking transition**: the scheduler finds a candidate tote
for line 0 (heaviest item) and determines when its VtmForward would
start (VTM availability in the tote's zone). That sim time is T. At T,
the scheduler checks `sku_available_at(sku, T) >= needed` for EVERY
line in the order. If ALL pass, all reservations are placed atomically
at T and the order enters Picking. If ANY fail, the order stays Ready
and retries next pass. No partial reservation — the order does not
start until all inventory can be guaranteed.

**Picking (in-flight)**: all SKU reservations have been placed
(sku_reserved incremented for every line at T). Picks dispatch one
at a time as totes become available, in arrival-time sequence (by
weight — heaviest first). In-flight orders' picks are dispatched
before any new order is allocated at the same station.

**Complete**: all picks have been consumed (try_remove_item for every
line). SKU reservations are fully decremented by individual pick
consumptions.

### Arrival-time sequencing

Within an order, picks must ARRIVE at the station in line sequence
(ordered by item weight — heaviest first for packing). This is
controlled by `after_ms` on each VtmForward, not by dispatch ordering.

Multiple picks for the same order can be in-flight simultaneously —
they dispatch with staggered `after_ms` values back-calculated from
their target arrival slots:

```
target_arrival[i] = station_next_free + i * P
after_ms[i] = target_arrival[i] - estimated_travel_time[i]
```

Longest-Fetch-First (LFF) assigns totes to slots: the tote with the
longest estimated travel time gets slot 0 (most lead time). Spatial
estimates rank candidates; Pathfinder validates actual times at commit
(see coding-standards.md §8).

### Tote availability for dispatch

A tote is available for pick dispatch at two points in its lifecycle:

- **In rack**: available until claimed by a VtmForward. The standard
  retrieval path (VTM → relay → HTM → station).
- **Post-presentation on HTM**: the HTM is holding the tote after a
  station presentation. Available for direct delivery to another
  station, skipping the VTM/relay path entirely.

Intermediate states (on VTM, at relay, on HTM en route) are NOT
available. The tote is committed to its current task chain.

Tote selection happens at dispatch time (lazy), not at SKU reservation
time. The SKU reservation guarantees inventory EXISTS for the order.
Tote selection finds the best REACHABLE tote when it's time to
dispatch each line.

### Scheduling priority

Each pass, the scheduling loop processes stations in round-robin. For
each station:

1. **In-flight orders first**: for each in-flight order with
   undispatched lines, check if the next line's tote is available and
   the arrival timing is correct for its slot. If so, dispatch it.
2. **New orders**: if the station can accept more in-flight orders,
   pop the next ready order from the queue. Check
   `sku_inventory - sku_reserved >= needed` for all lines. If yes,
   place SKU reservations and start dispatching.

This ensures in-flight orders complete before new orders consume
inventory. Completing orders frees station bins for new work.

### Spatial estimates vs Pathfinder (hard rule)

Spatial estimates (`estimate_travel_secs`, `travel_ms`, manhattan
distance) are used ONLY for prescreening — ranking candidates cheaply.
All claims, reservations, and scheduled arrival times must use the
Pathfinder (`get_path` / `reserve_path`).

Prescreening narrows the candidate set. The Pathfinder validates and
commits. If the Pathfinder rejects the top candidate (longer path,
conflict, no route), try the next one. See coding-standards.md §8.

### VTM scheduling (eager)

VTMs work through the queue in ASAP order. When multiple VtmForward
items are actionable, the VTM chains them back-to-back with zero idle
gaps. VtmReturn items are only drained when `after_ms <= vtm_available`
— if the VTM is free before the tote arrives at the relay, it does
Forwards instead. This matches the Java reference (`DesmojVtmProcess.java`).

### HTM scheduling (JIT arrival)

HTMs are scored by `queue_factor` for smooth presentation rates:
`target_arrival = station_next_free - (QF-1) * P`. The evaluate funnel
picks the HTM that arrives closest to the target time, with asymmetric
cost (3x penalty for late vs early).

### Relay as bounded buffer (with Holds model)

The relay absorbs timing mismatch between VTM deposit and HTM pickup.
Two hold types govern relay lifecycle:

**SlotHold** (tote presence on relay): two-phase lifecycle preserving
VTM/HTM decoupling.

- **Pick relay**: VTM deposits laden tote → SlotHold opens at
  `vtm_deposit`. HTM picks up → SlotHold finalized at `htm_pickup`.
  Uses `ReservationEnd::PendingPickup { consumer_task }` →
  `ReservationEnd::Exact(htm_pickup)`.

- **Store relay**: HTM deposits tote → SlotHold opens at `htm_deposit`.
  VTM picks up → SlotHold finalized at `vtm_pickup`. Same lifecycle.

**CellHold** (bot presence at relay HTM access cell): universal
deferred-close reservation. See §CellHold above.

SlotHold and CellHold are on different physical cells (relay slot vs
relay HTM access cell) and track different resources (tote vs bot).
VTM uses a separate VTM access cell — not affected by CellHold.

Relay holds use `ReservationBound::Unfinalized` (not sentinel
`SimTime::MAX`) at the planner state level. Oracle conversion via
`to_oracle_end_ms()` maps `Unfinalized` → `SimTime::MAX` for A*
performance. See `specs/putaway/planner-state-holds-spec.md`.

Relay occupancy self-regulates via the quadratic pressure curve and
`deadlock_headroom=2` (see `orchestration-model.md`). The headroom
check runs per-task in the dispatch pool scanner.

### Direct delivery

When an HTM already carries a matching tote (empty for putaway, laden
with needed SKU for pick), it bypasses the VtmForward → relay path
entirely. Direct delivery enters the chain at HtmDeliver with no relay
involved. The cost model naturally prefers direct routes because they
avoid VTM cycles.

## CellHold: bot-at-cell between tasks (Phase 6A)

Every task ends with an open-ended CellHold at its final cell. The bot
holds position with a `ReservationBound::Unfinalized` oracle reservation
until the scheduler commits the next task. This replaces the prior
"default parking leg" model.

**The Unfinalized reservation exists only in solver/real time.** Once the
next task commits, the reservation narrows to `Finite(departure_ms)`.
In sim-time the bot was never idle — just the normal task dwell. A* sees
the Unfinalized as `SimTime::MAX` and routes around it, which is correct
because the bot IS there until it leaves.

**Availability:** The bot reports availability at its CellHold cell (where
real work ended). It is immediately retaskable — the next scheduling pass
commits new work starting from that cell, which narrows the CellHold.

**Parking.** Between tasks a bot holds position via CellHold. An empty
HTM drives to a parking bay only when its presentation's clearance wins
the contest for its stay (`ParkClaimedHtm`), when it stands idle on a
relay access cell another task needs (`Parking`), or when it is still
standing idle on a belt's drop-off cell when the loop deposit's next round
comes (`LoopDeposit`). At end-of-plan, any open CellHolds are finalized in
place (bot stays where it is).

**Carrying state:** HTMs may hold a CellHold while carrying a tote (empty
after a pick, laden after a putaway). The tote remains visible to the
prescreen via content-based filtering — an HTM holding a matching tote at
a CellHold can be claimed as a Direct chain candidate (no VTM needed).

See `specs/putaway/planner-state-holds-spec.md` for the full Hold model.

## HTM Task A: Deliver to station

**Start**: HTM anywhere, carrying nothing or matching payload.

Matching payload = what the destination station needs:
- Pick station needs SKU S → matching = tote containing S
- Putaway station needs empty tote → matching = empty tote

Carrying nothing is not a match — it means the HTM must acquire the
payload via steps 1-2. Carrying a non-matching tote is invalid —
Task B must come first to store it.

**Steps** (maximal form):
1. Travel to relay *(skip if carrying matching payload)*
2. Pickup tote at relay *(skip if carrying matching payload)*
3. Travel to station
4. Present tote, dwell P seconds
5. Leave the station's cell when the operator releases the HTM (simtime),
   on the next task: a productive claim's, or the clearance's

Steps 1-2 acquire the payload when the HTM doesn't have it. Skipped
when the HTM already carries what the station needs (direct delivery).
When not skipped, the VTM has already deposited the target tote at the
relay via the VtmForward work item.

**End**: HTM at station, carrying tote, until its next task departs.
- After pick station: `EmptyTote` (operator removed items; `LadenTote`
  if items remain — depends on pick quantity vs tote contents)
- After putaway station: `LadenTote` (operator loaded items)

`estimated_end_ms` includes dwell — the station is occupied until
presentation completes.

### Post-presentation routing decision

After station presentation, the HTM carries a tote, and claims contest
its stay:
- **Direct delivery** to another station (empty → putaway),
  a productive claim
- **Store** at relay (HTM Task B) followed by VtmReturn, through the
  presentation's clearance (`ClearPresentation`)

A productive claim whose departure is the operator's release (simtime)
wins over the clearance, and one that would depart later loses to it.
Both claims start at the release, so the contest is decided when the
barrier reaches the release (scheduler time), and the winner's path is
committed departing at the release (simtime); a delivery waiting in the
station's queue moves up as soon as that path is committed. A direct
delivery that cannot depart then, its way held open-ended with no queue
cell free to drive onto, is refused, and in the same scheduler-time instant
the clearance takes the HTM home, departing at the release. This decision
point is where the system routes totes to their best next use.

## HTM Task B: Store tote

**Start**: HTM anywhere, carrying a tote (empty or laden).

**Steps**:
1. Travel to relay in the tote's zone (zone from task context, propagated
   through the task chain — totes are nomadic, no "home zone")
2. Drop off tote at relay
3. CellHold opens on relay HTM access cell (`Unfinalized`)
   — HTM stays at relay until next task commits

**End**: HTM at relay access cell, carrying nothing. CellHold open.

Two holds are created:
- **SlotHold** on the relay (tote presence): finalized when VtmReturn
  picks up the tote. Uses `ReservationEnd::PendingPickup { consumer_task }`
  → `ReservationEnd::Exact(vtm_pickup)`.
- **CellHold** on the relay HTM access cell (bot presence): finalized
  when the HTM departs for its next task (same-relay HtmDeliver preferred).

VtmReturn successor created in TaskGraph during apply-transition.

**Precondition**: HTM is carrying a tote.

## HTM Parking

HTMs hold position at their current cell via CellHold until the
scheduler assigns new work, and park only as **Parking** above says:
a clearance's `ParkClaimedHtm`, or `Parking` for an idle empty HTM on a
relay access cell another task needs (`TaskKind::Park`).

HTMs at CellHold may carry totes. An HTM carrying a matching tote is
a supply source visible to the prescreen (Direct chain candidate).

## VTM Forward: Retrieve from rack

**Start**: VTM in its zone, carrying nothing.

**Legs**:
1. `VtmTravel` — travel to rack shelf (unladen, diagonal X+Z movement)
2. `VtmForkOp` — withdraw tote from shelf (extend forks, lift, retract)
3. `VtmTravel` — travel to relay (laden, diagonal X+Z movement)
4. `VtmForkOp` — insert tote into relay (extend forks, lower, retract)

**End**: VTM at relay (aisle end), carrying nothing. Tote at relay.

**Precondition**: tote is in rack in the VTM's zone.

The relay hold starts as
`ReservationEnd::PendingPickup { consumer_task }` at `vtm_deposit`, then
is finalized to `ReservationEnd::Exact(htm_pickup)` when HtmDeliver is
scheduled. In the current queue implementation, an HtmDeliver work item is
pushed as a side effect; in TaskGraph migration, this becomes successor
node/edge creation during apply-transition.

## VTM Return: Store to rack

**Start**: VTM in its zone, carrying nothing.

**Legs**:
1. `VtmForkOp` — withdraw tote from relay (extend forks, lift, retract)
2. `VtmTravel` — travel to rack shelf (laden, diagonal X+Z movement)
3. `VtmForkOp` — insert tote into shelf (extend forks, lower, retract)

**End**: VTM at rack cell (in aisle), carrying nothing. Tote in rack.

Starts after HTM deposits tote at relay (the VtmReturn work item's
`after_ms` is the HTM deposit time).

**Precondition**: tote has been deposited at a relay in the VTM's zone.

VTM Returns are drained only when the VTM is available at or after the
deposit time. If the VTM is free before the tote arrives at the relay,
it does Forwards instead — no idle waiting. As relay pressure rises,
the quadratic scoring curve shifts priority toward Returns
(see `orchestration-model.md`).

## Task sequencing

Which tasks can follow which, constrained by end-state carrying:

| End state | Carrying | Valid next tasks |
|-----------|----------|-----------------|
| Task A (pick station) | EmptyTote | A (direct to putaway), B |
| Task A (putaway station) | LadenTote | B |
| Task B | Nothing | A (via relay), parking empty |
| Parking (empty) | Nothing | A (via relay) |

Direct delivery (Task A skipping steps 1-2) is the preferred path when
the HTM already carries what a station needs. The cost model naturally
selects direct delivery over relay round-trips because direct routes
avoid VTM cycles entirely.

## Putaway→pick tote lifecycle

When putaway is enabled, a tote goes through two full cycles to deliver
a pick. The second cycle depends on the first completing — this is the
temporal dependency that drives the multi-pass scheduling loop:

```
--- Putaway cycle (creates supply) ---
1. VtmForward      — empty tote: rack → relay (VTM eager, ASAP)
     → pushes HtmDeliver
2. HtmDeliver      — empty tote: relay → putaway station (HTM JIT)
                      operator fills tote with SKU (dwell P)
                      sku_inventory[sku] += 1 (new unit in warehouse)
                      blocked orders for this SKU re-checked
     → pushes HtmStore
3. HtmStore        — laden tote: station → relay
     → pushes VtmReturn
4. VtmReturn       — laden tote: relay → rack

--- Pick cycle (consumes supply) ---
5. Order enters picking state:
     sku_reserved[sku] += qty for ALL lines (priority claim)
     first pick dispatched:
6. VtmForward      — laden tote: rack → relay (VTM eager, ASAP)
     → pushes HtmDeliver
7. HtmDeliver      — laden tote: relay → pick station (HTM JIT)
                      operator picks item (dwell P)
                      sku_inventory[sku] -= 1 (unit consumed)
                      sku_reserved[sku] -= 1 (reservation fulfilled)
     → pushes HtmStore
8. HtmStore        — empty tote: station → relay
     → pushes VtmReturn
9. VtmReturn       — empty tote: relay → rack
```

Steps 6-9 repeat for each line in the order, dispatched one at a time
in arrival-time sequence (by weight). The SKU reservation at step 5
ensures all inventory is spoken for before any picks dispatch.

At steps 2 and 7, the post-presentation routing decision may choose
direct delivery instead of Store — sending the tote straight to another
station and skipping the Store/VtmReturn steps entirely.

Putaway and pick work are interleaved — there is no separate "putaway
phase." All work competes for resources simultaneously, with relay
backpressure providing natural flow control. The four-wall inventory
count ensures pick orders are only allocated when their SKUs physically
exist in the warehouse.

## Leg types

| Leg | Bot | Description |
|-----|-----|-------------|
| `RelayApproach` | HTM | Travel to relay for tote pickup |
| `StationApproach` | HTM | Travel from relay/current pos to station |
| `RelayDeposit` | HTM | Travel to relay to deposit tote (Store) |
| `Parking { cell }` | HTM | Travel to parking cell (default, cancelable) |
| `VtmTravel` | VTM | Travel between shelf and relay (diagonal X+Z) |
| `VtmForkOp` | VTM | Fork operation: withdraw or insert tote |

## Future considerations

### UOW model as general decoupling pattern

The work queue model generalizes beyond relays. Any physical handoff
point where one bot deposits and another picks up is a potential UOW
boundary with the same two-phase reservation pattern:
`ReservationEnd::PendingPickup { consumer_task }` ->
`ReservationEnd::Exact(pickup)`:

- **Station queue cells**: HTM deposits tote in queue, operator picks up.
  Could decouple HTM arrival from operator availability.
- **Charging stations**: bot parks at charger, dispatcher reclaims when
  charged. The "after_ms" is charge-complete time.
- **Staging areas**: cross-dock or consolidation buffers where totes
  accumulate before batch transport.

The key insight: any leg boundary where a bot waits for an external
precondition is a natural UOW boundary. The work queue doesn't care
what the buffer is — it needs an `after_ms` and a bot type.

### HTM leg decomposition

The current HTM legs (RelayApproach, StationApproach, RelayDeposit)
could similarly become UOW items if finer-grained decoupling is needed.
For example, separating relay pickup from station delivery would allow
the system to re-route an HTM mid-task if a closer station develops
demand. This is not needed now but the pattern supports it.

### Precomputed candidates on WorkItems

Work items could carry precomputed candidate lists (tote, relay, bot
shortlists from the prescreen phase) to amortize and deduplicate the
CO-WHCA* pathfinding work across passes. A candidate becomes stale when
the underlying reservation state changes, requiring invalidation. This
optimization reduces wall-clock scheduling cost without changing the
scheduling semantics.
