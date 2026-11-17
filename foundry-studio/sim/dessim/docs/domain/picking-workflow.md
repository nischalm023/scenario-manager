# Picking Workflow

How customer orders become physical picks, how storage containers move through the warehouse, and how the relay handoff model works.

---

## Order Lifecycle

### 1. Orders Arrive

Orders enter the system as a batch. Each order specifies:
- A list of items (SKU + quantity) to pick
- A priority level (low, medium, high, urgent)
- A deadline (default: 8 hours from creation)

Orders are queued as **pending** until the bin management process selects them for processing.

### 2. Order Selection (Bin Management)

Every 5 seconds, a periodic process evaluates pending orders and selects a batch for processing. Selection uses one of three order selection strategies (see `order-strategies.md`). The goal is to maximize throughput by choosing orders that share storage containers, minimizing the number of physical trips robots must make.

For each selected order:
1. **Set cover** -- find the minimum set of storage containers (racks or totes) that cover all items in the order
2. **Inventory reservation** -- atomically reserve the required quantities across all containers (all-or-nothing: if any item cannot be reserved, the entire order is rejected)
3. **Station assignment** -- assign the order to a station and bind it to an order bin at that station
4. **Task generation** -- create pick tasks and attach them to the appropriate storage containers

After this step, racks/totes "know" they have work -- pick tasks are attached directly to the storage container. Robots discover this work when scanning for containers with pending tasks.

### 3. Robot Execution

Robots independently discover and execute pick tasks. The flow differs by system mode:

**RTP mode:**
- Robot finds a rack with tasks
- Claims and transports the rack to the assigned station
- Operator picks items from the rack into order bins
- Robot returns rack to storage
- If the rack had tasks for multiple stations, the robot visits each one

**Relay mode:**
- VTM finds a tote with tasks in its zone
- VTM extracts tote from rack, delivers to relay
- HTM finds tote at relay, delivers to station
- Operator picks items from the tote
- HTM returns tote to a relay position (potentially in a different zone)
- VTM returns tote from relay to a rack slot

### 4. Pick Processing at Station

When a robot arrives at a station with a storage container:
1. Robot enters the station queue (FIFO, capacity 1 per station)
2. When the operator is free, processing begins
3. For each pick task on the container assigned to this station:
   - Operator picks `quantity` units of the SKU
   - Pick time = `operator_time_per_pick * quantity` (default: 12 seconds per pick)
   - Inventory is consumed from the storage container
   - The pick task is marked complete
4. If all tasks for an order are complete, the order is marked complete and its bin is released

### 5. Order Completion

An order is complete when ALL its pick tasks across ALL storage containers have been processed. Since an order's items may be spread across multiple racks/totes that arrive at different times, order completion is tracked via a callback: each time a task completes, the system checks if the parent order has any remaining tasks.

---

## The Relay Handoff Model

The relay system implements a **producer-consumer buffer** pattern. VTMs produce (load totes onto relays), HTMs consume (drain totes from relays). Relay positions are the buffer.

```
Rack Slots       Relay Positions       Picking Stations
[Tote] --VTM-->  [Relay] --HTM-->     [Station]
[Tote] --VTM-->  [Relay] --HTM-->     [Station]
[Tote]           [    ]               [Station]
                                      [Station]

     PRODUCTION         CONSUMPTION
     (zone-bound)       (global)
```

### Why Two Robot Types?

A single robot type doing the full rack-to-station trip would need to navigate narrow aisles AND long highway distances. The relay system splits this into two specialized roles:

- **VTMs** are optimized for short-range, high-frequency aisle work (extracting totes from racks)
- **HTMs** are optimized for long-range highway travel (moving totes across the warehouse to stations)

The relay positions decouple these two concerns spatially and temporally. A VTM can deliver a tote to relay and immediately start the next extraction, without waiting for the HTM to deliver and return.

### Relay Capacity and Backpressure

Each relay position holds exactly 0 or 1 tote. The total relay capacity of a zone is the number of relay positions in that zone.

**Zone utilization** is the percentage of relay positions that are occupied or reserved:
```
utilization = (occupied + reserved) / total_relay_positions * 100
```

When zone utilization reaches **80%**, backpressure activates:
- VTMs stop producing (delivering new totes to relay) for that zone
- VTMs prioritize returning empty totes from relay back to racks (draining the buffer)
- If no return work exists, VTMs idle for 2 seconds before re-checking

This threshold-based flow control prevents relay saturation. Without it, VTMs would keep delivering totes faster than HTMs can drain them, eventually blocking all relays and deadlocking the zone.

**Two-phase congestion check:**
1. **Physical occupancy** (totes physically at relay, not counting in-transit): triggers return work (actively drain the buffer)
2. **Projected occupancy** (physical + reserved for incoming totes): triggers idling (stop producing, the buffer will fill when in-transit totes arrive)

Physical check fires first because actively clearing the buffer is more effective than passively waiting.

### Zone Load Balancing

When an HTM returns a tote after processing, it must choose which zone to return it to. The system uses **bucketed load balancing**:

1. Track how many active totes each zone has (its "load")
2. Group zones into buckets of size 5 by load level
3. Select from the lowest-load bucket

This prevents all HTMs from converging on a single least-loaded zone (herd behavior) while still distributing returns toward less-loaded zones.

---

## Inventory and Reservation

### Storage Containers

**Rack (RTP system):** Holds up to 80 items across multiple SKUs. The entire rack moves to the station. Default capacity: 80 items.

**Tote (Relay system):** A smaller container holding up to 4 items. Totes live inside racks (each rack has up to 12 tote slots, indexed vertically by z-position). Only the tote moves -- the rack stays in place. Default capacity: 4 items.

Both racks and totes serve as "storage spaces" -- the generic concept of a container that holds inventory and can have pick tasks assigned to it.

### Set Cover: Minimizing Storage Space Trips

Given an order with multiple items, the system must decide which storage containers to pull. The **set cover algorithm** minimizes the number of containers needed:

1. Build a map of remaining items (SKU -> quantity needed)
2. Greedily select the storage container that covers the most distinct SKUs
3. Storage containers already in transit (at relay, at station, being carried) get a massive bonus (+1000) -- piggybacking on an already-moving container is essentially free
4. Repeat until all items are covered or a limit is reached (default: 10 containers per order)

This piggybacking bonus is key to throughput. If a tote is already at a relay or station for one order, adding another order's tasks to it costs zero additional robot trips.

### All-or-Nothing Reservation

When an order is selected for processing, the system atomically reserves inventory for ALL its tasks. If any single item cannot be reserved (insufficient stock in the assigned container), the entire order is rejected -- no partial fulfillment. This prevents inventory conflicts where two orders both try to claim the last unit of a SKU.

### Tote Location Tracking

Every tote has exactly one location at any time:
- **In rack** -- stored at a specific rack and vertical slot
- **At relay** -- sitting at a relay position, waiting for HTM pickup
- **At station** -- at a picking station, being processed
- **With robot** -- being carried by a VTM or HTM

These locations are mutually exclusive. Movement operations atomically transition the tote from one location to another.

---

## Key Invariants

1. **Relay capacity = 1.** A relay position holds exactly 0 or 1 tote. No buffering at a single position.
2. **All-or-nothing reservation.** Inventory for an order is either fully reserved or not reserved at all.
3. **Tote claims prevent double-selection.** Once a robot claims a tote, no other robot of the same type will target it.
4. **Single-threaded execution.** All decision-making and state mutation happens in a single thread. Claims, reservations, and state transitions are inherently atomic -- no locks needed.
5. **Station operator capacity = 1.** One robot processed at a time per station. Others queue in FIFO order.
6. **Active space piggybacking.** Storage containers already in motion get priority in set cover to encourage task batching on containers that are already incurring travel cost.
