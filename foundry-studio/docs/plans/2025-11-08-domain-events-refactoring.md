# Domain Events Refactoring Implementation Plan (REVISED)

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace error-prone 7-parameter `emitEvent()` method with type-safe domain event records using Java 21 sealed interfaces.

**Architecture (REVISED):** Three-level sealed interface hierarchy (DomainEvent → Category → Concrete Records). **Consolidate into 4 files** instead of 18+ to reduce cognitive load. Use top-level records grouped by category for clean usage. Dual-API support during migration (new `emit(DomainEvent)` + deprecated `emitEvent(...)` for compatibility).

**Tech Stack:** Java 21 (sealed interfaces, records, pattern matching), Lombok, Spring Boot, Desmo-J

**Testing Strategy (REVISED):** Integration-focused tests only. Skip unit tests for individual event validation (rely on compile-time safety). Focus on end-to-end behavior: emit → collector → metrics.

---

## Problem Statement

### Current Pain Points

1. **Error-prone 7-parameter signature:**
   ```java
   emitEvent(eventType, timestamp, robotId, rackId, taskId, orderId, data)
   ```
   - Easy to mix up parameter order (robotId vs rackId)
   - Critical bugs where `null` was passed instead of actual IDs
   - Comments warning "CRITICAL: Pass IDs as direct parameters, NOT in data map"

2. **Redundant data in multiple places:**
   - `robotId` passed as parameter AND in data map
   - No single source of truth

3. **Boilerplate helper methods everywhere:**
   - Each class wraps `emitEvent()` to reduce verbosity
   - Varargs key-value construction not type-safe

4. **Manual Map construction with string keys:**
   ```java
   Map<String, Object> data = new HashMap<>();
   data.put("total_picks", 5);  // Typo-prone, no autocomplete
   ```

5. **No event-specific structure:**
   - All events use generic `Map<String, Object>`
   - No documentation of required fields per event type

### Solution Benefits

- ✅ **Type safety:** IDE autocomplete, compile-time checking
- ✅ **No parameter confusion:** Named fields in records
- ✅ **Self-documenting:** Each event declares its schema
- ✅ **Validation:** Compact constructors validate data at construction time
- ✅ **Pattern matching:** Collectors use `instanceof` for clean handling
- ✅ **Less boilerplate:** No manual Map construction

**Example Improvement:**
```java
// BEFORE: Error-prone, unclear
metricsManager.emitEvent(
    MetricEvent.TASK_COMPLETED,
    now(),
    robot.getId(),
    null,  // What is this?
    task.getTaskId(),
    task.getOrderId(),
    Map.of("total_picks", 5)
);

// AFTER: Type-safe, clear
metricsManager.emit(new TaskCompleted(
    now(),
    robot.getId(),
    task.getTaskId(),
    task.getOrderId(),
    stationId,
    5
));
```

---

## Architecture Overview

### 1. Event Hierarchy (REVISED - 4 Files)

**File Structure:**
```
events/
├── DomainEvent.java        // Base + all category sealed interfaces
├── TaskEvents.java         // TaskStarted + TaskCompleted (2 records)
├── RobotEvents.java        // RobotIdle + 3 cycle events (4 records)
└── MiscEvents.java         // Order, System, Operator, Inventory, Relay (8 records)
```

**Type Hierarchy:**
```
DomainEvent (sealed interface)
├── TaskEvent (sealed interface)
│   ├── TaskStarted (record)
│   └── TaskCompleted (record)
├── RobotEvent (sealed interface)
│   ├── RobotIdle (record)
│   ├── RackCycleCompleted (record)
│   ├── HtmCycleCompleted (record)
│   └── VtmCycleCompleted (record)
├── OrderEvent (sealed interface)
│   ├── OrderCompleted (record)
│   ├── BatchOrdersProcessed (record)
│   └── AllOrdersCompleted (record)
├── SystemEvent (sealed interface)
│   ├── SimulationStarted (record)
│   └── SimulationCompleted (record)
├── OperatorEvent (sealed interface)
│   └── OperatorProcessing (record)
├── InventoryEvent (sealed interface)
│   └── InventoryConsumed (record)
└── RelayEvent (sealed interface)
    └── RelayPositionBlocked (record)
```

**Note:** All category interfaces in DomainEvent.java, concrete records grouped by file.

### 2. Key Design Decisions (REVISED)

1. **Sealed interfaces:** Exhaustive pattern matching, compiler-enforced completeness
2. **Records:** Immutable data carriers with automatic equals/hashCode/toString
3. **No optional fields:** Required fields only, use separate event types for variants
4. **Minimal validation:** Only business rules (timestamp >= 0, quantity > 0), let NPE happen naturally for null
5. **Top-level records:** Clean usage (`new TaskCompleted(...)` not `TaskEvent.Completed(...)`)
6. **4 files only:** DomainEvent.java + 3 category files (TaskEvents, RobotEvents, MiscEvents)
7. **Keep MetricEvent enum:** Use for handler registration, provides backward compatibility
8. **Dual API during migration:** Support both `emit(DomainEvent)` and `emitEvent(...)` temporarily
9. **Integration tests only:** Skip unit tests for validation, focus on end-to-end behavior

### 3. Migration Strategy (REVISED - Simplified)

**Phase 1:** Add new infrastructure (4 files: base + 3 category files, no breaking changes)
**Phase 2:** Update MetricsManager with dual API support
**Phase 3:** Migrate collectors to handle both old and new events (pattern matching)
**Phase 4:** Migrate call sites incrementally (file by file)
**Phase 5:** Remove deprecated methods after full migration

**Reduced from 10 tasks to 6-7 tasks** by consolidating file creation and simplifying testing.

---

## Event Catalog (24 Events)

### Task Events (2)
1. **TaskStarted** - When task assigned to rack/tote
   - Fields: timestamp, taskId, orderId, rackId
2. **TaskCompleted** - When operator finishes pick
   - Fields: timestamp, robotId, taskId, orderId, stationId, totalPicks

### Robot Events (4)
3. **RobotIdle** - When robot has no work
   - Fields: timestamp, robotId
4. **RackCycleCompleted** - RTP robot completes full cycle
   - Fields: timestamp, robotId, rackId, cycleStartTime, cycleTime, totalPicks, tasksProcessed, travelTime, processingTime, stationsVisited, isMultiStation
5. **HtmCycleCompleted** - HTM robot completes delivery/return cycle
   - Fields: timestamp, robotId, toteId, cycleStartTime, cycleTime, totalPicks, stationsVisited
6. **VtmCycleCompleted** - VTM robot completes delivery/return cycle
   - Fields: timestamp, robotId, toteId, cycleStartTime, cycleTime, rackId, relayPositionId

### Order Events (3)
7. **OrderCompleted** - When all tasks for order complete
   - Fields: timestamp, orderId, totalTasks, completionTime
8. **BatchOrdersProcessed** - Batch orders loaded
   - Fields: timestamp, batchSize, totalOrders
9. **AllOrdersCompleted** - All orders finished
   - Fields: timestamp, totalOrders, duration

### System Events (2)
10. **SimulationStarted** - Simulation begins
    - Fields: timestamp
11. **SimulationCompleted** - Simulation ends
    - Fields: timestamp, duration

### Operator Events (1)
12. **OperatorProcessing** - When operator processes picks
    - Fields: timestamp, stationId, taskId, processingTime, pickCount

### Inventory Events (1)
13. **InventoryConsumed** - When items picked from storage
    - Fields: timestamp, skuId, quantity, storageSpaceId

### Relay Events (1)
14. **RelayPositionBlocked** - When VTM blocks waiting for relay capacity
    - Fields: timestamp, robotId, relayPositionId, waitTime

---

## Implementation Tasks (REVISED - 6 Tasks)

### Task 1: Create Event Infrastructure (All 4 Files)

**Goal:** Create complete sealed interface hierarchy with all event records in 4 files.

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/DomainEvent.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/TaskEvents.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/RobotEvents.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/MiscEvents.java`

**Step 1: Create the base sealed interface with all category interfaces**

Create DomainEvent.java with base interface and all category interfaces:

```java
package com.greyorange.dessimulator.simulator.metrics.events;

import com.greyorange.dessimulator.simulator.metrics.MetricEvent;

/**
 * Base sealed interface for all domain events in the metrics system.
 *
 * <p>Domain events represent business actions (TaskCompleted, RobotIdle) and are strongly typed
 * using Java 21 records. This replaces the error-prone 7-parameter emitEvent() method.
 *
 * <p>All domain events must:
 * - Implement this sealed interface (directly or via category interface)
 * - Be implemented as immutable records
 * - Validate their data in compact constructors
 * - Map to a MetricEvent enum value for handler routing
 *
 * @see MetricEvent
 */
public sealed interface DomainEvent
    permits TaskEvent, RobotEvent, OrderEvent, SystemEvent, OperatorEvent, InventoryEvent, RelayEvent {

  /**
   * Get the simulation timestamp when this event occurred.
   *
   * @return Simulation time in seconds
   */
  double timestamp();

  /**
   * Get the metric event type for handler routing.
   *
   * @return MetricEvent enum value
   */
  MetricEvent eventType();
}

/**
 * Category interface for task-related events.
 *
 * <p>Task events track the lifecycle of picking tasks from assignment to completion.
 */
sealed interface TaskEvent extends DomainEvent permits TaskStarted, TaskCompleted {}

/**
 * Category interface for robot-related events.
 *
 * <p>Robot events track robot activity including idle states and cycle completions.
 */
sealed interface RobotEvent extends DomainEvent
    permits RobotIdle, RackCycleCompleted, HtmCycleCompleted, VtmCycleCompleted {}

/**
 * Category interface for order-related events.
 *
 * <p>Order events track order processing from batch loading to completion.
 */
sealed interface OrderEvent extends DomainEvent
    permits OrderCompleted, BatchOrdersProcessed, AllOrdersCompleted {}

/**
 * Category interface for system lifecycle events.
 *
 * <p>System events mark major simulation state transitions.
 */
sealed interface SystemEvent extends DomainEvent permits SimulationStarted, SimulationCompleted {}

/**
 * Category interface for operator activity events.
 *
 * <p>Operator events track human operator actions at stations.
 */
sealed interface OperatorEvent extends DomainEvent permits OperatorProcessing {}

/**
 * Category interface for inventory consumption events.
 *
 * <p>Inventory events track item depletion from storage.
 */
sealed interface InventoryEvent extends DomainEvent permits InventoryConsumed {}

/**
 * Category interface for relay system events.
 *
 * <p>Relay events track relay position blocking and tote handoffs.
 */
sealed interface RelayEvent extends DomainEvent permits RelayPositionBlocked {}
```

**Step 2: Create TaskEvents.java with 2 event records**

```java
package com.greyorange.dessimulator.simulator.metrics.events;

import com.greyorange.dessimulator.simulator.metrics.MetricEvent;

/** Category interface for task-related events. */
sealed interface TaskEvent extends DomainEvent permits TaskStarted, TaskCompleted {}

/**
 * Event emitted when a task is assigned to a rack or tote for processing.
 */
public record TaskStarted(
    double timestamp,
    String taskId,
    String orderId,
    int rackId
) implements TaskEvent {

  public TaskStarted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    // Let NPE happen for null taskId/orderId
    if (rackId < 0) {
      throw new IllegalArgumentException("rackId must be >= 0, got: " + rackId);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.TASK_STARTED;
  }
}

/**
 * Event emitted when an operator finishes picking items from a task.
 */
public record TaskCompleted(
    double timestamp,
    int robotId,
    String taskId,
    String orderId,
    int stationId,
    int totalPicks
) implements TaskEvent {

  public TaskCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
    // Let NPE happen for null taskId/orderId
    if (stationId < 0) {
      throw new IllegalArgumentException("stationId must be >= 0, got: " + stationId);
    }
    if (totalPicks < 0) {
      throw new IllegalArgumentException("totalPicks must be >= 0, got: " + totalPicks);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.TASK_COMPLETED;
  }
}
```

**Step 3: Create RobotEvents.java with 4 event records**

```java
package com.greyorange.dessimulator.simulator.metrics.events;

import com.greyorange.dessimulator.simulator.metrics.MetricEvent;
import java.util.List;

/** Category interface for robot-related events. */
sealed interface RobotEvent extends DomainEvent
    permits RobotIdle, RackCycleCompleted, HtmCycleCompleted, VtmCycleCompleted {}

/**
 * Event emitted when a robot becomes idle (no work available).
 */
public record RobotIdle(
    double timestamp,
    int robotId
) implements RobotEvent {

  public RobotIdle {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.ROBOT_IDLE;
  }
}

/**
 * Event emitted when an RTP robot completes a full rack cycle.
 *
 * <p>Cycle: Retrieve rack → Transport to station(s) → Operator picks → Return rack to storage
 */
public record RackCycleCompleted(
    double timestamp,
    int robotId,
    int rackId,
    double cycleStartTime,
    double cycleTime,
    int totalPicks,
    int tasksProcessed,
    double travelTime,
    double processingTime,
    List<Integer> stationsVisited,
    boolean isMultiStation
) implements RobotEvent {

  public RackCycleCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
    if (rackId < 0) {
      throw new IllegalArgumentException("rackId must be >= 0, got: " + rackId);
    }
    if (cycleTime < 0) {
      throw new IllegalArgumentException("cycleTime must be >= 0, got: " + cycleTime);
    }
    if (totalPicks < 0) {
      throw new IllegalArgumentException("totalPicks must be >= 0, got: " + totalPicks);
    }
    if (tasksProcessed < 0) {
      throw new IllegalArgumentException("tasksProcessed must be >= 0, got: " + tasksProcessed);
    }
    // Let NPE happen for null stationsVisited
    stationsVisited = List.copyOf(stationsVisited); // Defensive copy
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.RACK_CYCLE_COMPLETED;
  }
}

/**
 * Event emitted when an HTM (Horizontal Tote Mover) robot completes a cycle.
 *
 * <p>Cycle: Pick tote from relay → Transport to station → Operator picks → Return to relay
 */
public record HtmCycleCompleted(
    double timestamp,
    int robotId,
    String toteId,
    double cycleStartTime,
    double cycleTime,
    int totalPicks,
    List<Integer> stationsVisited
) implements RobotEvent {

  public HtmCycleCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
    if (cycleTime < 0) {
      throw new IllegalArgumentException("cycleTime must be >= 0, got: " + cycleTime);
    }
    if (totalPicks < 0) {
      throw new IllegalArgumentException("totalPicks must be >= 0, got: " + totalPicks);
    }
    // Let NPE happen for null toteId/stationsVisited
    stationsVisited = List.copyOf(stationsVisited);
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.HTM_CYCLE_COMPLETED;
  }
}

/**
 * Event emitted when a VTM (Vertical Tote Mover) robot completes a cycle.
 *
 * <p>Cycle: Retrieve tote from rack → Transport to relay position → Return (or reverse)
 */
public record VtmCycleCompleted(
    double timestamp,
    int robotId,
    String toteId,
    double cycleStartTime,
    double cycleTime,
    int rackId,
    int relayPositionId
) implements RobotEvent {

  public VtmCycleCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
    if (cycleTime < 0) {
      throw new IllegalArgumentException("cycleTime must be >= 0, got: " + cycleTime);
    }
    if (rackId < 0) {
      throw new IllegalArgumentException("rackId must be >= 0, got: " + rackId);
    }
    if (relayPositionId < 0) {
      throw new IllegalArgumentException("relayPositionId must be >= 0, got: " + relayPositionId);
    }
    // Let NPE happen for null toteId
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.VTM_CYCLE_COMPLETED;
  }
}
```

**Step 4: Create MiscEvents.java with 8 event records**

```java
package com.greyorange.dessimulator.simulator.metrics.events;

import com.greyorange.dessimulator.simulator.metrics.MetricEvent;

/** Category interfaces for remaining event types. */
sealed interface OrderEvent extends DomainEvent
    permits OrderCompleted, BatchOrdersProcessed, AllOrdersCompleted {}
sealed interface SystemEvent extends DomainEvent permits SimulationStarted, SimulationCompleted {}
sealed interface OperatorEvent extends DomainEvent permits OperatorProcessing {}
sealed interface InventoryEvent extends DomainEvent permits InventoryConsumed {}
sealed interface RelayEvent extends DomainEvent permits RelayPositionBlocked {}

// ========== Order Events ==========

/**
 * Event emitted when all tasks for an order are completed.
 */
public record OrderCompleted(
    double timestamp,
    String orderId,
    int totalTasks,
    double completionTime
) implements OrderEvent {

  public OrderCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (totalTasks < 0) {
      throw new IllegalArgumentException("totalTasks must be >= 0, got: " + totalTasks);
    }
    if (completionTime < 0) {
      throw new IllegalArgumentException("completionTime must be >= 0, got: " + completionTime);
    }
    // Let NPE happen for null orderId
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.ORDER_COMPLETED;
  }
}

/**
 * Event emitted when a batch of orders is loaded into the simulation.
 */
public record BatchOrdersProcessed(
    double timestamp,
    int batchSize,
    int totalOrders
) implements OrderEvent {

  public BatchOrdersProcessed {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (batchSize < 0) {
      throw new IllegalArgumentException("batchSize must be >= 0, got: " + batchSize);
    }
    if (totalOrders < 0) {
      throw new IllegalArgumentException("totalOrders must be >= 0, got: " + totalOrders);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.BATCH_ORDERS_PROCESSED;
  }
}

/**
 * Event emitted when all orders in the simulation have been completed.
 */
public record AllOrdersCompleted(
    double timestamp,
    int totalOrders,
    double duration
) implements OrderEvent {

  public AllOrdersCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (totalOrders < 0) {
      throw new IllegalArgumentException("totalOrders must be >= 0, got: " + totalOrders);
    }
    if (duration < 0) {
      throw new IllegalArgumentException("duration must be >= 0, got: " + duration);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.ALL_ORDERS_COMPLETED;
  }
}

// ========== System Events ==========

/**
 * Event emitted when the simulation begins.
 */
public record SimulationStarted(double timestamp) implements SystemEvent {

  public SimulationStarted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.SIMULATION_STARTED;
  }
}

/**
 * Event emitted when the simulation ends.
 */
public record SimulationCompleted(
    double timestamp,
    double duration
) implements SystemEvent {

  public SimulationCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (duration < 0) {
      throw new IllegalArgumentException("duration must be >= 0, got: " + duration);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.SIMULATION_COMPLETED;
  }
}

// ========== Operator Events ==========

/**
 * Event emitted when an operator processes picks at a station.
 */
public record OperatorProcessing(
    double timestamp,
    int stationId,
    String taskId,
    double processingTime,
    int pickCount
) implements OperatorEvent {

  public OperatorProcessing {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (stationId < 0) {
      throw new IllegalArgumentException("stationId must be >= 0, got: " + stationId);
    }
    if (processingTime < 0) {
      throw new IllegalArgumentException("processingTime must be >= 0, got: " + processingTime);
    }
    if (pickCount < 0) {
      throw new IllegalArgumentException("pickCount must be >= 0, got: " + pickCount);
    }
    // Let NPE happen for null taskId
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.OPERATOR_PROCESSING;
  }
}

// ========== Inventory Events ==========

/**
 * Event emitted when items are picked from storage (inventory consumption).
 */
public record InventoryConsumed(
    double timestamp,
    String skuId,
    int quantity,
    int storageSpaceId
) implements InventoryEvent {

  public InventoryConsumed {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (quantity <= 0) {
      throw new IllegalArgumentException("quantity must be > 0, got: " + quantity);
    }
    if (storageSpaceId < 0) {
      throw new IllegalArgumentException("storageSpaceId must be >= 0, got: " + storageSpaceId);
    }
    // Let NPE happen for null skuId
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.INVENTORY_CONSUMED;
  }
}

// ========== Relay Events ==========

/**
 * Event emitted when a VTM robot blocks waiting for relay position capacity.
 *
 * <p>This indicates congestion at relay handoff points.
 */
public record RelayPositionBlocked(
    double timestamp,
    int robotId,
    int relayPositionId,
    double waitTime
) implements RelayEvent {

  public RelayPositionBlocked {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
    if (relayPositionId < 0) {
      throw new IllegalArgumentException("relayPositionId must be >= 0, got: " + relayPositionId);
    }
    if (waitTime < 0) {
      throw new IllegalArgumentException("waitTime must be >= 0, got: " + waitTime);
    }
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.RELAY_POSITION_BLOCKED;
  }
}
```

**Step 5: Verify compilation**

Run:
```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Compilation SUCCESS

**Step 6: Commit all event infrastructure**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/
git commit -m "feat(metrics): add complete domain event infrastructure (4 files)

- DomainEvent.java: Base sealed interface + all category interfaces
- TaskEvents.java: TaskStarted + TaskCompleted (2 records)
- RobotEvents.java: RobotIdle + 3 cycle events (4 records)
- MiscEvents.java: Order, System, Operator, Inventory, Relay events (8 records)

Benefits:
- Type-safe event emission with compile-time checks
- Minimal validation (business rules only, let NPE happen for nulls)
- Top-level records for clean usage: new TaskCompleted(...)
- 14 event types consolidated into 4 files for reduced cognitive load
"
```

---

### Task 2: Update MetricsManager with Dual API Support

**Goal:** Add new `emit(DomainEvent)` method while keeping old `emitEvent(...)` methods deprecated for backward compatibility.

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricsManager.java`

**Step 1: Add import for domain events**

Add at top of file:
```java
import com.greyorange.dessimulator.simulator.metrics.events.*;
```

**Step 2: Add new emit() method and conversion helper**

Add after existing `emitEvent()` methods:

```java
  /**
   * Emit a domain event (NEW TYPE-SAFE API).
   *
   * <p>This is the preferred way to emit events. Domain events are strongly typed records that
   * prevent parameter confusion and validate data at construction time.
   *
   * @param event The domain event to emit
   */
  public void emit(DomainEvent event) {
    // Convert to internal MetricEventData for backward compatibility with collectors
    MetricEventData legacyEvent = convertToLegacyEvent(event);

    // Log event
    eventsLog.add(legacyEvent);

    // Route to handlers
    List<HandlerInfo> handlers = handlerMap.get(event.eventType());
    if (handlers != null) {
      for (HandlerInfo handlerInfo : handlers) {
        try {
          // Handlers can accept either MetricEventData or DomainEvent
          Class<?> paramType = handlerInfo.method.getParameterTypes()[0];
          if (paramType.equals(DomainEvent.class) || DomainEvent.class.isAssignableFrom(paramType)) {
            // New-style handler: pass domain event directly
            handlerInfo.method.invoke(handlerInfo.collector, event);
          } else {
            // Old-style handler: pass converted legacy event
            handlerInfo.method.invoke(handlerInfo.collector, legacyEvent);
          }
        } catch (Exception e) {
          log.error(
              "Error invoking handler {} for event {}: {}",
              handlerInfo.method.getName(),
              event.eventType(),
              e.getMessage(),
              e);
        }
      }
    }

    log.trace("Emitted domain event: {} at t={}", event.eventType(), event.timestamp());
  }

  /**
   * Convert a domain event to legacy MetricEventData format.
   *
   * <p>This enables backward compatibility during the migration period.
   */
  private MetricEventData convertToLegacyEvent(DomainEvent event) {
    var builder = MetricEventData.builder()
        .eventType(event.eventType())
        .timestamp(event.timestamp());

    Map<String, Object> data = new HashMap<>();

    // Convert domain event fields to legacy format based on event type
    switch (event) {
      case TaskStarted e -> {
        builder.taskId(e.taskId()).orderId(e.orderId()).rackId(e.rackId());
      }
      case TaskCompleted e -> {
        builder.robotId(e.robotId()).taskId(e.taskId()).orderId(e.orderId());
        data.put("station_id", e.stationId());
        data.put("total_picks", e.totalPicks());
      }
      case RobotIdle e -> {
        builder.robotId(e.robotId());
      }
      case RackCycleCompleted e -> {
        builder.robotId(e.robotId()).rackId(e.rackId());
        data.put("cycle_start_time", e.cycleStartTime());
        data.put("cycle_time", e.cycleTime());
        data.put("total_picks", e.totalPicks());
        data.put("tasks_processed", e.tasksProcessed());
        data.put("travel_time", e.travelTime());
        data.put("processing_time", e.processingTime());
        data.put("stations_visited", e.stationsVisited());
        data.put("is_multi_station", e.isMultiStation());
      }
      case HtmCycleCompleted e -> {
        builder.robotId(e.robotId());
        data.put("tote_id", e.toteId());
        data.put("cycle_time", e.cycleTime());
        data.put("total_picks", e.totalPicks());
        data.put("stations_visited", e.stationsVisited());
      }
      case VtmCycleCompleted e -> {
        builder.robotId(e.robotId()).rackId(e.rackId());
        data.put("tote_id", e.toteId());
        data.put("cycle_time", e.cycleTime());
        data.put("relay_position_id", e.relayPositionId());
      }
      case OrderCompleted e -> {
        builder.orderId(e.orderId());
        data.put("total_tasks", e.totalTasks());
        data.put("completion_time", e.completionTime());
      }
      case BatchOrdersProcessed e -> {
        data.put("batch_size", e.batchSize());
        data.put("total_orders", e.totalOrders());
      }
      case AllOrdersCompleted e -> {
        data.put("total_orders", e.totalOrders());
        data.put("duration", e.duration());
      }
      case SimulationStarted e -> {
        // No additional data
      }
      case SimulationCompleted e -> {
        data.put("duration", e.duration());
      }
      case OperatorProcessing e -> {
        builder.taskId(e.taskId());
        data.put("station_id", e.stationId());
        data.put("processing_time", e.processingTime());
        data.put("pick_count", e.pickCount());
      }
      case InventoryConsumed e -> {
        data.put("sku_id", e.skuId());
        data.put("quantity", e.quantity());
        data.put("storage_space_id", e.storageSpaceId());
      }
      case RelayPositionBlocked e -> {
        builder.robotId(e.robotId());
        data.put("relay_position_id", e.relayPositionId());
        data.put("wait_time", e.waitTime());
      }
    }

    return builder.data(data).build();
  }
```

**Step 3: Deprecate old emitEvent methods**

Add `@Deprecated` annotation to the three existing `emitEvent()` methods:

```java
  /** @deprecated Use {@link #emit(DomainEvent)} instead for type safety */
  @Deprecated(since = "2025-11-08", forRemoval = true)
  public void emitEvent(MetricEvent eventType, double timestamp) {
    emitEvent(eventType, timestamp, null, null, null, null, null);
  }

  /** @deprecated Use {@link #emit(DomainEvent)} instead for type safety */
  @Deprecated(since = "2025-11-08", forRemoval = true)
  public void emitEvent(
      MetricEvent eventType,
      double timestamp,
      Integer robotId,
      Integer rackId,
      Map<String, Object> data) {
    emitEvent(eventType, timestamp, robotId, rackId, null, null, data);
  }

  /** @deprecated Use {@link #emit(DomainEvent)} instead for type safety */
  @Deprecated(since = "2025-11-08", forRemoval = true)
  public void emitEvent(
      MetricEvent eventType,
      double timestamp,
      Integer robotId,
      Integer rackId,
      String taskId,
      String orderId,
      Map<String, Object> data) {
    // ... existing implementation ...
  }
```

**Step 4: Verify compilation**

Run:
```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Compilation SUCCESS (deprecation warnings expected)

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricsManager.java
git commit -m "feat(metrics): add dual API support to MetricsManager

- Add emit(DomainEvent) method for type-safe event emission
- Add convertToLegacyEvent() for backward compatibility
- Deprecate old emitEvent() methods (marked for removal)
- Support both old and new handler signatures during migration

Migration path: call sites can switch to emit() incrementally
"
```

---

### Task 3: Update Collectors with DomainEvent Handlers

**Goal:** Add new handler methods that accept `DomainEvent` and use pattern matching, while keeping old handlers for compatibility.

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java`

**Step 1: Add domain event handlers to TaskCollector**

In `MetricsCollectors.java`, add these new handler methods to `TaskCollector` class (after existing handlers):

```java
    // NEW: Type-safe handlers using pattern matching
    @MetricHandler({MetricEvent.TASK_COMPLETED})
    public void handleTaskCompletedDomain(DomainEvent event) {
      if (event instanceof TaskCompleted tc) {
        if (tc.taskId() != null) {
          completedTasks.add(tc.taskId());
          taskCompletionTimes.add(tc.timestamp());
        }
        recordEvent(convertToLegacy(event));
        log.trace("Task completed (domain): {} at t={}", tc.taskId(), tc.timestamp());
      }
    }

    @MetricHandler({MetricEvent.ORDER_COMPLETED})
    public void handleOrderCompletedDomain(DomainEvent event) {
      if (event instanceof OrderCompleted oc) {
        if (oc.orderId() != null) {
          completedOrders.add(oc.orderId());
          orderCompletionTimes.add(oc.timestamp());
        }
        recordEvent(convertToLegacy(event));
      }
    }

    @MetricHandler({MetricEvent.SIMULATION_STARTED})
    public void handleSimulationStartedDomain(DomainEvent event) {
      if (event instanceof SimulationStarted ss) {
        simulationStartTime = ss.timestamp();
        recordEvent(convertToLegacy(event));
      }
    }

    @MetricHandler({MetricEvent.SIMULATION_COMPLETED})
    public void handleSimulationCompletedDomain(DomainEvent event) {
      if (event instanceof SimulationCompleted sc) {
        simulationEndTime = sc.timestamp();
        recordEvent(convertToLegacy(event));
      }
    }

    private MetricEventData convertToLegacy(DomainEvent event) {
      return MetricEventData.builder()
          .eventType(event.eventType())
          .timestamp(event.timestamp())
          .build();
    }
```

**Step 2: Add domain event handlers to remaining collectors**

Add similar pattern-matching handlers to:
- `RobotCollector` (for RobotIdle, RackCycleCompleted, HtmCycleCompleted, VtmCycleCompleted)
- `StorageSpaceCollector` (for RackCycleCompleted)
- `StationCollector` (for TaskCompleted, RackCycleCompleted)
- `OperatorCollector` (for OperatorProcessing)
- `InventoryCollector` (for InventoryConsumed)
- `RelayPositionCollector` (for RelayPositionBlocked)

Example for RobotCollector:
```java
    @MetricHandler({MetricEvent.ROBOT_IDLE})
    public void handleRobotIdleDomain(DomainEvent event) {
      if (event instanceof RobotIdle ri) {
        idleEvents.compute(ri.robotId(), (k, v) -> (v == null ? 0 : v) + 1);
        recordEvent(convertToLegacy(event));
      }
    }

    @MetricHandler({MetricEvent.RACK_CYCLE_COMPLETED})
    public void handleRackCycleCompletedDomain(DomainEvent event) {
      if (event instanceof RackCycleCompleted rcc) {
        cycleTimes.add(rcc.cycleTime());
        totalCyclesCompleted++;
        multiStationVisits.compute(rcc.stationsVisited().size(), (k, v) -> (v == null ? 0 : v) + 1);
        recordEvent(convertToLegacy(event));
      }
    }
```

**Step 3: Verify compilation**

Run:
```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Compilation SUCCESS

**Step 4: Run existing tests**

Run:
```bash
cd apps/backend
mvn test
```

Expected: All tests PASS (dual handler support working)

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat(metrics): add DomainEvent handlers to all collectors

- Add pattern-matching handlers for all event types
- Use instanceof for type-safe field access
- Keep old MetricEventData handlers for backward compatibility
- Dual-API support enables incremental migration

Next: Migrate call sites to use new emit(DomainEvent) API
"
```

---

### Task 4: Migrate Call Sites to Domain Events

**Goal:** Systematically migrate all call sites to use `emit(DomainEvent)` instead of deprecated `emitEvent(...)`.

**Files to modify:**
- `DesmojStationHelper.java`
- `DesmojRtpRobotProcess.java`
- `DesmojBinManagementProcess.java`
- `DesmojMetricMonitoringProcess.java`
- `DesmojSimulationEngine.java`
- Any other files found with grep

**Step 1: Find all call sites**

Run:
```bash
cd apps/backend
grep -r "metricsManager.emitEvent" src/main/java/ --include="*.java" | grep -v "MetricsManager.java" | cut -d: -f1 | sort -u
```

**Step 2: Migrate DesmojStationHelper.java**

Replace `emitTaskCompletionEvent` method:

```java
  private static void emitTaskCompletionEvent(
      MetricsManager metricsManager,
      Robot robot,
      PickTask task,
      StorageSpaceId storageSpaceId,
      SimProcess callingProcess) {

    if (metricsManager == null) {
      return;
    }

    // NEW: Type-safe domain event
    metricsManager.emit(new TaskCompleted(
        callingProcess.presentTime().getTimeAsDouble(TimeUnit.SECONDS),
        robot.getId(),
        task.getTaskId(),
        task.getOrderId(),
        task.getStationId().value(),
        task.getQuantity()
    ));
  }
```

**Step 3: Migrate DesmojRtpRobotProcess.java**

Replace idle event emission:
```java
// OLD: metricsManager.emitEvent(MetricEvent.ROBOT_IDLE, now(), robotId, null, Map.of());
// NEW:
metricsManager.emit(new RobotIdle(now(), robotId));
```

Replace cycle event emission in `emitRtpCycle()`:
```java
metricsManager.emit(new RackCycleCompleted(
    cycleEndTime,
    robotId,
    rackId,
    cycleStartTime,
    cycleEndTime - cycleStartTime,
    totalPicks,
    tasksProcessed,
    travelTime,
    processingTime,
    stationsVisited.stream().map(StationId::value).toList(),
    stationsVisited.size() > 1
));
```

**Step 4: Migrate DesmojSimulationEngine.java**

Replace all event emissions:
```java
// Batch orders
metricsManager.emit(new BatchOrdersProcessed(0.0, loadedOrders.size(), loadedOrders.size()));

// Task started
metricsManager.emit(new TaskStarted(0.0, task.getTaskId(), task.getOrderId(), rackId.value()));

// Order completed
metricsManager.emit(new OrderCompleted(currentTime, orderId, tasksForOrder.size(), currentTime));

// Simulation completed
metricsManager.emit(new SimulationCompleted(endTime, duration));

// All orders completed
metricsManager.emit(new AllOrdersCompleted(currentTime, totalOrders, currentTime));
```

**Step 5: Migrate remaining files**

Apply similar transformations to:
- `DesmojBinManagementProcess.java`
- `DesmojMetricMonitoringProcess.java`
- Any other files identified in Step 1

**Step 6: Verify compilation**

Run after each file migration:
```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Compilation SUCCESS

**Step 7: Run full test suite**

Run:
```bash
cd apps/backend
mvn test
```

Expected: All tests PASS (including DesmojIntegrationTest)

**Step 8: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/
git commit -m "refactor(metrics): migrate all call sites to domain events

Migrated files:
- DesmojStationHelper: TaskCompleted
- DesmojRtpRobotProcess: RobotIdle, RackCycleCompleted
- DesmojBinManagementProcess: BatchOrdersProcessed
- DesmojMetricMonitoringProcess: AllOrdersCompleted
- DesmojSimulationEngine: TaskStarted, OrderCompleted, SimulationCompleted

Benefits:
- No more 7-parameter confusion
- No more string keys in Maps
- Type-safe field access throughout
- All integration tests passing
"
```

---

### Task 5: Add Integration Tests

**Goal:** Create integration tests that verify end-to-end behavior: emit → collector → metrics.

**Files:**
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/simulator/metrics/DomainEventsIntegrationTest.java`

**Step 1: Create test class**

```java
package com.greyorange.dessimulator.simulator.metrics;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.*;
import com.greyorange.dessimulator.simulator.metrics.events.*;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

/**
 * Integration tests for domain events system.
 *
 * <p>Tests end-to-end flow: emit → collector → metrics aggregation.
 */
class DomainEventsIntegrationTest {

  private MetricsManager metricsManager;
  private SimulationConfig config;

  @BeforeEach
  void setUp() {
    config = new SimulationConfig(
        SystemType.RTP,
        "test-scenario",
        5,
        null,
        null,
        2,
        100,
        1.0,
        5.0
    );
    metricsManager = new MetricsManager(config, Map.of());
  }

  @Test
  void taskEventFlow_emitToCollector_aggregatesCorrectly() {
    // Emit task events
    metricsManager.emit(new SimulationStarted(0.0));
    metricsManager.emit(new TaskStarted(1.0, "task-1", "order-1", 5));
    metricsManager.emit(new TaskCompleted(10.0, 1, "task-1", "order-1", 2, 5));
    metricsManager.emit(new OrderCompleted(10.0, "order-1", 1, 10.0));
    metricsManager.emit(new SimulationCompleted(100.0, 100.0));

    // Verify events logged
    assertEquals(5, metricsManager.getEventCount());

    // Verify task metrics collected
    var taskMetrics = metricsManager.getMetrics().taskMetrics();
    assertEquals(1, taskMetrics.tasksCompleted());
    assertEquals(1, taskMetrics.ordersCompleted());
    assertNotNull(taskMetrics.avgTaskCompletionTime());
    assertTrue(taskMetrics.avgTaskCompletionTime() > 0);
  }

  @Test
  void robotEventFlow_emitToCollector_aggregatesCorrectly() {
    // Emit robot events
    metricsManager.emit(new RobotIdle(5.0, 0));
    metricsManager.emit(new RobotIdle(10.0, 0));
    metricsManager.emit(new RackCycleCompleted(
        20.0, 1, 5, 10.0, 10.0, 15, 3, 6.0, 4.0, List.of(1, 2), true
    ));

    // Verify events logged
    assertEquals(3, metricsManager.getEventCount());

    // Verify robot metrics collected
    var robotMetrics = metricsManager.getMetrics().robotMetrics();
    assertEquals(1, robotMetrics.cyclesCompleted());
    assertNotNull(robotMetrics.avgCycleTime());
    assertEquals(10.0, robotMetrics.avgCycleTime());
  }

  @Test
  void orderEventFlow_emitToCollector_aggregatesCorrectly() {
    // Emit order processing events
    metricsManager.emit(new BatchOrdersProcessed(0.0, 10, 50));
    metricsManager.emit(new OrderCompleted(120.0, "order-1", 5, 115.0));
    metricsManager.emit(new OrderCompleted(130.0, "order-2", 3, 128.0));
    metricsManager.emit(new AllOrdersCompleted(300.0, 50, 295.0));

    // Verify events logged
    assertEquals(4, metricsManager.getEventCount());

    // Verify order metrics collected
    var taskMetrics = metricsManager.getMetrics().taskMetrics();
    assertEquals(2, taskMetrics.ordersCompleted());
  }

  @Test
  void multipleEventCategories_emitConcurrently_allCollectedCorrectly() {
    // Simulate realistic event sequence
    metricsManager.emit(new SimulationStarted(0.0));
    metricsManager.emit(new BatchOrdersProcessed(0.0, 5, 5));
    metricsManager.emit(new TaskStarted(1.0, "task-1", "order-1", 10));
    metricsManager.emit(new RobotIdle(2.0, 0));
    metricsManager.emit(new TaskCompleted(5.0, 1, "task-1", "order-1", 1, 8));
    metricsManager.emit(new RackCycleCompleted(
        15.0, 1, 10, 5.0, 10.0, 8, 1, 7.0, 3.0, List.of(1), false
    ));
    metricsManager.emit(new OrderCompleted(15.0, "order-1", 1, 15.0));
    metricsManager.emit(new AllOrdersCompleted(100.0, 5, 100.0));
    metricsManager.emit(new SimulationCompleted(100.0, 100.0));

    // Verify all events logged
    assertEquals(9, metricsManager.getEventCount());

    // Verify all metric categories populated
    var metrics = metricsManager.getMetrics();
    assertNotNull(metrics.taskMetrics());
    assertNotNull(metrics.robotMetrics());
    assertNotNull(metrics.storageSpaceMetrics());

    // Verify specific values
    assertEquals(1, metrics.taskMetrics().tasksCompleted());
    assertEquals(1, metrics.taskMetrics().ordersCompleted());
    assertEquals(1, metrics.robotMetrics().cyclesCompleted());
  }

  @Test
  void patternMatching_unknownEventType_handledGracefully() {
    // This shouldn't happen due to sealed interfaces, but test robustness
    metricsManager.emit(new SimulationStarted(0.0));

    // Should not throw exception
    assertEquals(1, metricsManager.getEventCount());
  }
}
```

**Step 2: Run tests**

Run:
```bash
cd apps/backend
mvn test -Dtest=DomainEventsIntegrationTest
```

Expected: All tests PASS

**Step 3: Verify with existing integration test**

Run:
```bash
cd apps/backend
mvn test -Dtest=DesmojIntegrationTest
```

Expected: Test PASSES with domain events

**Step 4: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/simulator/metrics/DomainEventsIntegrationTest.java
git commit -m "test(metrics): add integration tests for domain events

- Test task event flow: emit → collector → metrics
- Test robot event flow: idle and cycle events
- Test order event flow: batch processing to completion
- Test multi-category event sequences
- Verify pattern matching and error handling

All tests pass, domain events working end-to-end
"
```

---

### Task 6: Remove Deprecated Code and Complete Migration

**Goal:** Clean up deprecated methods after successful migration.

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricsManager.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java`
- Delete: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEventData.java`

**Step 1: Remove deprecated emitEvent methods from MetricsManager**

Delete the three deprecated `emitEvent()` methods.

**Step 2: Simplify emit() method**

Remove dual-API support, keep only domain events:

```java
  /**
   * Emit a domain event.
   *
   * <p>Domain events are strongly typed records that prevent parameter confusion and validate data
   * at construction time.
   *
   * @param event The domain event to emit
   */
  public void emit(DomainEvent event) {
    // Log event
    eventsLog.add(event);

    // Route to handlers
    List<HandlerInfo> handlers = handlerMap.get(event.eventType());
    if (handlers != null) {
      for (HandlerInfo handlerInfo : handlers) {
        try {
          handlerInfo.method.invoke(handlerInfo.collector, event);
        } catch (Exception e) {
          log.error(
              "Error invoking handler {} for event {}: {}",
              handlerInfo.method.getName(),
              event.eventType(),
              e.getMessage(),
              e);
        }
      }
    }

    log.trace("Emitted event: {} at t={}", event.eventType(), event.timestamp());
  }
```

**Step 3: Remove convertToLegacyEvent() method**

Delete the conversion method (no longer needed).

**Step 4: Update eventsLog type**

Change:
```java
private final List<MetricEventData> eventsLog = ...;
```

To:
```java
private final List<DomainEvent> eventsLog = Collections.synchronizedList(new ArrayList<>());
```

Update getter:
```java
public List<DomainEvent> getEventsLog() {
  return Collections.unmodifiableList(eventsLog);
}
```

**Step 5: Remove old handlers from collectors**

In `MetricsCollectors.java`, delete all old handlers that accept `MetricEventData`.

Rename new handlers (remove "Domain" suffix):
- `handleTaskCompletedDomain` → `handleTaskCompleted`
- etc.

**Step 6: Update BaseCollector**

Change eventsLog type:
```java
private final List<DomainEvent> eventsLog;

protected void recordEvent(DomainEvent event) {
  eventsLog.add(event);
}

public List<DomainEvent> getEventsLog() {
  return Collections.unmodifiableList(eventsLog);
}
```

**Step 7: Delete MetricEventData.java**

Run:
```bash
rm apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEventData.java
```

**Step 8: Verify compilation**

Run:
```bash
cd apps/backend
mvn clean compile
```

Expected: Compilation SUCCESS (no deprecation warnings)

**Step 9: Run full test suite**

Run:
```bash
cd apps/backend
mvn test
```

Expected: All tests PASS

**Step 10: Format code**

Run:
```bash
cd apps/backend
mvn spotless:apply
```

**Step 11: Final commit**

```bash
git add -A
git rm apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEventData.java
git commit -m "refactor(metrics): complete domain events migration

- Remove deprecated emitEvent() methods from MetricsManager
- Remove convertToLegacyEvent() compatibility layer
- Delete MetricEventData.java (replaced by DomainEvent hierarchy)
- Remove old MetricEventData handlers from collectors
- Simplify emit() method (no more dual-API support)
- Update eventsLog type to List<DomainEvent>
- Code formatted with Spotless

Migration complete! Type-safe domain events now fully operational.

Benefits achieved:
✅ Type safety with compile-time checking
✅ No parameter confusion
✅ Self-documenting event schemas
✅ Validation at construction time
✅ Pattern matching in collectors
✅ 4 files instead of 18+ (reduced cognitive load)
✅ Integration tests verify end-to-end behavior
"
```

---

## Migration Summary (REVISED)

### What Changed

**Before:**
```java
// Error-prone 7-parameter method
metricsManager.emitEvent(
    MetricEvent.TASK_COMPLETED,
    now(),
    robot.getId(),      // Easy to mix up
    null,               // What is this?
    task.getTaskId(),
    task.getOrderId(),
    Map.of("total_picks", 5, "station_id", 3)  // String keys, no type safety
);
```

**After:**
```java
// Type-safe domain event
metricsManager.emit(new TaskCompleted(
    now(),
    robot.getId(),
    task.getTaskId(),
    task.getOrderId(),
    stationId,
    5
));
```

### Benefits Achieved

1. ✅ **Type safety:** Compile-time checking, IDE autocomplete
2. ✅ **No parameter confusion:** Named fields in records
3. ✅ **Self-documenting:** Each event declares its schema
4. ✅ **Validation:** Compact constructors catch errors at construction time
5. ✅ **Pattern matching:** Clean collector handlers with `instanceof`
6. ✅ **Reduced cognitive load:** 4 files instead of 18+
7. ✅ **Integration tests:** Verify real behavior, not just validation
8. ✅ **Clean usage:** `new TaskCompleted(...)` not `TaskEvent.Completed(...)`

### Files Created (4 files)

**Event Infrastructure:**
- `DomainEvent.java` - Base sealed interface + all category interfaces
- `TaskEvents.java` - TaskStarted + TaskCompleted (2 records)
- `RobotEvents.java` - RobotIdle + 3 cycle events (4 records)
- `MiscEvents.java` - Order, System, Operator, Inventory, Relay events (8 records)

**Tests:**
- `DomainEventsIntegrationTest.java` - End-to-end integration tests

### Files Modified (8 files)

- `MetricsManager.java` - Added `emit(DomainEvent)`, removed deprecated methods
- `MetricsCollectors.java` - Updated all collectors with pattern matching handlers
- `DesmojStationHelper.java` - Migrated to domain events
- `DesmojRtpRobotProcess.java` - Migrated to domain events
- `DesmojBinManagementProcess.java` - Migrated to domain events
- `DesmojMetricMonitoringProcess.java` - Migrated to domain events
- `DesmojSimulationEngine.java` - Migrated to domain events
- Other orchestrator files

### Files Deleted (1 file)

- `MetricEventData.java` - Replaced by `DomainEvent` hierarchy

### Task Count Reduction

- **Original plan:** 10 tasks with 18+ files and many unit tests
- **Revised plan:** 6 tasks with 4 files and integration tests only
- **Reduction:** 40% fewer tasks, 78% fewer files, more valuable tests
