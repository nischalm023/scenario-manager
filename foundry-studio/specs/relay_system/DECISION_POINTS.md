# Relay System: Algorithmic Decision Points

**Document Version:** 1.0  
**Last Updated:** November 2024  
**Purpose:** Comprehensive reference for all algorithmic and logical decision points in the Relay warehouse system

---

## Table of Contents

1. [Inventory Initialization (Put Algorithm)](#1-inventory-initialization-put-algorithm)
2. [VTM Zone Management](#2-vtm-zone-management)
3. [Order Processing & Task Generation](#3-order-processing--task-generation)
4. [VTM Task Selection & Execution](#4-vtm-task-selection--execution)
5. [Relay Position Management](#5-relay-position-management)
6. [HTM Task Selection & Execution](#6-htm-task-selection--execution)
7. [Task Lifecycle & State Management](#7-task-lifecycle--state-management)
8. [Station Processing](#8-station-processing)

---

## 1. Inventory Initialization (Put Algorithm)

### Decision Point 1.1: Put Algorithm Selection

**Location:** `domain/put/put.py` - `PutProcessor.iniialize_inventory()`

**Decision:** Which algorithm to use for initial inventory placement?

**Current Implementation:**

- **Options:** `greedy`, `popularity_based`, `random`
- **Default:** Configured per simulation
- **Greedy Strategy:**
  - Fill totes sequentially with popular items first
  - Prioritize high-demand SKUs near stations
  - `MAX_SKUS_PER_TOTE = 3` (prevents overloading small-capacity totes)

---

### Decision Point 1.2: SKU-to-Tote Assignment

**Location:** `domain/put/put.py` - `_initialize_inventory_greedy()`

**Decision:** Which totes receive which SKUs?

**Current Implementation:**

- **Popularity Scoring:** Items ranked by demand frequency
- **Capacity Constraint:** Max 3 SKUs per tote (relay-specific)
- **Quantity Allocation:**
  ```python
  allocated_quantity = min(
      remaining_quantity,
      tote.max_capacity - tote.current_quantity
  )
  ```
- **Rack Affinity:** Totes in same rack can have different SKU mixes

**Key Trade-offs:**

- Lower SKU diversity per tote → More tote movements
- Higher SKU diversity → Better order consolidation

---

### Decision Point 1.3: Popularity Calculation

**Location:** `domain/inventory/inventory.py` - `calculate_popularity_score()`

**Decision:** How to score tote/rack popularity?

**Current Implementation:**

```python
popularity_score = popular_item_quantity / total_quantity
```

- **Popular Items:** Top 20% by demand frequency (configurable)
- **Score Range:** [0.0, 1.0]
- **Usage:** Influences task prioritization and positioning

---

## 2. VTM Zone Management

### Decision Point 2.1: Zone Generation

**Location:** `domain/robot/zones.py` - `_generate_zones()`

**Decision:** How to partition warehouse into VTM service zones?

**Current Implementation:**

- **Zone Definition:** Each aisle = 1 zone
- **Coverage Area:** Empty aisle cells for VTM navigation
- **Serviced Racks:** Racks adjacent to aisle (left/right split at midpoint)
- **Left Aisle Zone:** Services left-side racks of block
- **Right Aisle Zone:** Services right-side racks of block

**Algorithm:**

```python
for each rack_block:
    left_aisle_col = block.start_col - 1
    right_aisle_col = block.end_col + 1

    # Split racks by proximity
    midpoint_col = (block.start_col + block.end_col) // 2

    left_zone.serviced_racks = [racks where col <= midpoint_col]
    right_zone.serviced_racks = [racks where col > midpoint_col]
```

---

### Decision Point 2.2: VTM-to-Zone Assignment (Load Balancing)

**Location:** `domain/robot/zones.py` - `balance_vtm_assignments()`

**Decision:** How to assign VTMs to zones?

**Current Implementation:**

- **Case 1: VTMs ≥ Zones**
  - Strategy: Multiple VTMs per zone (round-robin)
  - Example: 6 VTMs, 3 zones → Each zone gets 2 VTMs
- **Case 2: VTMs < Zones**
  - Strategy: Multiple zones per VTM (distribute evenly)
  - Example: 2 VTMs, 5 zones → VTM1 gets 3 zones, VTM2 gets 2 zones

**Algorithm:**

```python
if num_vtms >= num_zones:
    # Multiple VTMs per zone
    for i, vtm_id in enumerate(vtm_ids):
        zone_idx = i % num_zones
        assign_vtm_to_zones(vtm_id, [zones[zone_idx]])
else:
    # Multiple zones per VTM
    zones_per_vtm = ceil(num_zones / num_vtms)
    for i, vtm_id in enumerate(vtm_ids):
        start_idx = i * zones_per_vtm
        end_idx = min(start_idx + zones_per_vtm, num_zones)
        assign_vtm_to_zones(vtm_id, zones[start_idx:end_idx])
```

**Key Trade-offs:**

- Static assignment (set at initialization) vs dynamic rebalancing
- No workload-based rebalancing currently implemented

---

### Decision Point 2.3: VTM Initial Positioning

**Location:** `simulator/simulation_engine.py` - `_balance_vtm_assignments()`

**Decision:** Where to physically place VTMs after zone assignment?

**Current Implementation:**

- **Strategy:** Teleport to center of first assigned zone
- **Calculation:**
  ```python
  zone_center = GridCoordinate(
      row=mean(zone.coverage_area rows),
      col=mean(zone.coverage_area cols)
  )
  vtm.teleport_to(zone_center)
  ```
- **Rationale:** Minimize initial travel distance

---

## 3. Order Processing & Task Generation

### Decision Point 3.1: Order Selection Algorithm

**Location:** `domain/order/manager.py` - `process_orders()`

**Decision:** Which orders to process in each batch?

**Current Implementation:**

- **Options:**
  - `maximize_ppf_directly`: Maximize picks-per-face
  - `maximize_ppf_with_tiering`: PPF + priority tiers
  - `maintain_tpq`: Maintain tasks-per-quantity ratio
- **Batch Size:** Configurable (default: 100 orders)
- **Timing:** Based on SimPy simulation clock

---

### Decision Point 3.2: Task Quantity Allocation

**Location:** `domain/order/processor.py` - `_generate_tasks_for_order()`

**Decision:** How to split order items into tasks?

**Current Implementation:**

```python
available_qty = tote.get_quantity(sku)

if available_qty == 0:
    # Skip task creation entirely
    logger.warning("⚠️ SKIPPING TASK: No stock available")
    continue

task_quantity = min(requested_qty, available_qty)

if task_quantity < requested_qty:
    logger.warning("⚠️ PARTIAL FULFILLMENT")
```

**Key Constraints:**

- Max 6 items per tote (physical capacity)
- No task splitting across multiple totes
- Inventory reservation happens at task creation

---

### Decision Point 3.3: Station Assignment

**Location:** `domain/order/processor.py` - `_assign_station_to_tasks()`

**Decision:** Which station should process this order?

**Current Implementation:**

- **Strategy:** Random station selection
- **Future Consideration:** Load balancing, proximity-based, operator skill-based

```python
station_id = random.choice(available_stations)
```

---

## 4. VTM Task Selection & Execution

### Decision Point 4.1: Task Selection

**Location:** `simulator/orchestrator/vtm_orchestrator.py` - `_find_vtm_task()`

**Decision:** Which pending task should this VTM execute?

**Current Implementation:**

- **Stage Filter:** Only `PENDING` tasks with no assigned VTM
- **Zone Filter:** Task's source rack must be in VTM's assigned zones
- **State Validation:**
  ```python
  if tote.state != ToteState.IN_RACK:
      continue  # Skip totes not in racks
  ```
- **Selection:** First match wins (no optimization)

**Filtering Pipeline:**

```
All Tasks
  → Filter: stage == PENDING
  → Filter: assigned_vtm_id == None
  → Filter: source_rack_id != None
  → Filter: tote.state == IN_RACK
  → Filter: rack in any VTM zone
  → Select: First match
```

**Future Improvements:**

- Priority-based selection
- Distance-based optimization
- Predicted wait time minimization

---

### Decision Point 4.2: Relay Position Selection

**Location:** `domain/robot/relay_positions.py` - `RelayPositionManager.select_least_congested_position()`

**Decision:** Which relay position should receive this tote?

**Current Implementation:**

- **Strategy:** Least congested (fewest pending totes)
- **Tie-breaking:** First available (no secondary criteria)

```python
least_congested = min(
    positions,
    key=lambda p: len(p.pending_totes)
)
```

**Key Metrics:**

- `pending_totes`: Totes waiting for HTM pickup
- No consideration of: relay proximity, station affinity, HTM availability

**Future Improvements:**

- Distance from current VTM position
- Target station proximity
- HTM availability prediction

---

## 5. Relay Position Management

### Decision Point 5.1: High Utilization Threshold

**Location:** `domain/robot/relay_positions.py` - `RelayPosition.is_highly_utilized()`

**Decision:** When should totes be returned to racks?

**Current Implementation:**

```python
UTILIZATION_THRESHOLD = 0.8  # 80% capacity

def is_highly_utilized(self) -> bool:
    return len(self.pending_totes) / self.max_capacity >= UTILIZATION_THRESHOLD
```

**Return to Storage Logic:**

- **Trigger:** Relay position reaches 80% capacity
- **Action:** HTM returns tote to original source rack (not yet implemented)
- **Current Behavior:** Totes stay at relay indefinitely

**Design Intent:**

- Keep "hot" totes at relay for quick reuse
- Return "cold" totes when space is needed

---

## 6. HTM Task Selection & Execution

### Decision Point 6.1: Relay Position Selection

**Location:** `simulator/orchestrator/htm_orchestator.py` - `_find_relay_with_totes()`

**Decision:** Which relay position should HTM service?

**Current Implementation:**

- **Strategy:** First relay with pending totes
- **No optimization:** Does not consider distance, quantity, or priority

```python
for position in all_relay_positions:
    if len(position.pending_totes) > 0:
        return position  # First match
```

**Future Improvements:**

- Nearest relay first
- Most congested relay first
- Station-affinity based selection

---

### Decision Point 6.2: Tote-to-Task Matching

**Location:** `simulator/orchestrator/htm_orchestator.py` - HTM process loop

**Decision:** Which task corresponds to this tote at relay?

**Current Implementation:**

```python
relay_task = next(
    (t for t in relay_tasks
     if t.tote_id == tote.id
     and t.stage == RelayTaskStage.AT_RELAY),
    None
)
```

**Stage Requirements:**

- Task must be in `AT_RELAY` stage
- If no matching task: HTM waits (1 second idle)
- If task stage is `COMPLETED`: Ignored (waiting for new orders)

**Key Edge Case:**

- Tote at relay with only COMPLETED tasks → waits for new batch

---

## 7. Task Lifecycle & State Management

### Decision Point 7.1: Task Stage Transitions

**Location:** Multiple orchestrators

**Decision:** When do tasks transition between stages?

**Current Implementation:**

| Stage            | Trigger                             | Actor          | Next Stage                         |
| ---------------- | ----------------------------------- | -------------- | ---------------------------------- |
| `PENDING`        | Task created                        | OrderProcessor | → `VTM_RETRIEVAL`                  |
| `PENDING`        | Task created, tote already at relay | OrderProcessor | → Stays `PENDING` until VTM scan\* |
| `VTM_RETRIEVAL`  | VTM assigns to self                 | VTM            | → `VTM_TO_RELAY`                   |
| `VTM_TO_RELAY`   | VTM picks up tote                   | VTM            | → `AT_RELAY`                       |
| `AT_RELAY`       | VTM delivers to relay               | VTM            | → `HTM_TO_STATION`                 |
| `HTM_TO_STATION` | HTM assigns to self                 | HTM            | → `AT_STATION`                     |
| `AT_STATION`     | HTM picks up from relay             | HTM            | → `RETURNING`                      |
| `RETURNING`      | Station processing complete         | Station        | → `COMPLETED`                      |
| `COMPLETED`      | HTM returns to relay                | HTM            | (Terminal)                         |

**\*Special Case - Tote Already at Relay (NEW):**

```python
# In VTM task search loop:
if tote.state == ToteState.AT_RELAY_POSITION:
    task.stage = RelayTaskStage.AT_RELAY
    task.relay_position_id = tote.current_holding_entity
    # VTM skips this task, HTM picks it up directly
```

**Key Design Decision:**

- Tasks for totes already at relay start as `PENDING`
- VTM scan "promotes" them to `AT_RELAY` dynamically
- HTM can process immediately without VTM transport

---

### Decision Point 7.2: Tote State Transitions

**Location:** `domain/inventory/totes.py` + orchestrators

**Decision:** When do totes change state?

**Current State Machine:**

```
IN_RACK (initial)
  ↓ [VTM picks up]
BEING_TRANSPORTED_TO_RELAY
  ↓ [VTM delivers]
AT_RELAY_POSITION
  ↓ [HTM picks up]
BEING_TRANSPORTED_TO_STATION
  ↓ [Arrives at station]
AT_STATION
  ↓ [Processing complete, HTM picks up]
BEING_RETURNED_TO_RELAY
  ↓ [HTM delivers back]
AT_RELAY_POSITION (cycle repeats)
```

**Key Validation:**

- `move_to_relay()` accepts both `BEING_TRANSPORTED_TO_RELAY` and `BEING_RETURNED_TO_RELAY`
- State checked before VTM task assignment
- State must be `IN_RACK` for VTM pickup, `AT_RELAY_POSITION` for HTM pickup

---

## 8. Station Processing

### Decision Point 8.1: Task Processing Order

**Location:** `simulator/orchestrator/station_orchestrator.py` - `process_storage_space_tasks()`

**Decision:** In what order should tasks from a tote be processed?

**Current Implementation:**

- **Strategy:** Sequential FIFO (First In, First Out)
- **Station Filtering:** Only process tasks assigned to current station
- **Inventory Consumption:**

  ```python
  consumed = inventory.consume_item(
      storage_space_id=tote.id,
      sku=task.sku,
      quantity=task.quantity
  )

  if consumed < task.quantity:
      logger.error("❌ UNDERFULFILMENT")
  ```

**Key Constraint:**

- Tote may visit multiple stations if tasks assigned to different stations
- Not yet implemented in Relay system (RTP only)

---

### Decision Point 8.2: Operator Resource Allocation

**Location:** `simulator/orchestrator/station_orchestrator.py`

**Decision:** How are operators assigned to processing tasks?

**Current Implementation:**

- **Resource Model:** SimPy `Resource` with capacity = 1
- **Blocking:** Tasks wait if operator busy
- **Priority:** FIFO (no prioritization)

```python
with operator_resource.request() as request:
    yield request
    # Process task
    yield env.timeout(processing_time)
```

---

## Summary of Key Algorithmic Gaps & Future Work

### Immediate Optimization Opportunities:

1. **VTM Task Selection:** Currently first-match; could optimize by distance/priority
2. **HTM Relay Selection:** Currently first-available; could optimize by proximity
3. **Zone Rebalancing:** Static assignment; could dynamically rebalance based on workload
4. **Station Assignment:** Random; could use load balancing or proximity
5. **Relay Position Selection:** Least-congested only; could consider target station proximity

### Strategic Considerations:

1. **Tote Pool Management:** When to return totes to racks vs keep at relay
2. **Multi-Station Routing:** Not yet implemented for Relay (exists in RTP)
3. **Predictive Task Assignment:** Anticipate future demand patterns
4. **Dynamic Capacity Planning:** Adjust VTM/HTM count based on utilization

---

## Document Maintenance

**To Update This Document:**

1. When adding new decision logic → Add new section
2. When changing existing logic → Update "Current Implementation"
3. When implementing future work → Move from "Future Improvements" to "Current Implementation"

**Review Schedule:** After each major feature addition or algorithm change

**Contact:** Algorithm/Simulation team leads

---

_End of Document_
