# System TPQ Metrics Design

## Problem

`TaskPoolQualityMetrics` is null because `ORDER_BATCH_SELECTED` events are never emitted. The TPQ collector exists and listens for these events, but no code emits them.

## Goal

Track and emit system-wide TPQ metrics for all order selection algorithms, enabling performance comparison across algorithms (not just the TPQ-optimizing algorithm).

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Scope of TPQ | All algorithms | Enables comparison across PPF, Greedy, TPQ algorithms |
| What to measure | System-wide TPQ (MVP), batch TPQ (future) | Start simple with system state; add batch quality later |
| Primary metric | System TPQ | Overall task pool quality after batch assignment |
| Travel distance | Per storage space to assigned station | Accurate measurement using actual coordinates |
| Travel calculation | One-way distance | Matches existing `TpqCalculator` convention |
| Relay robot speed | HTM speed | Simplified approximation (full model would track VTM→Relay→Station) |
| Position lookup | Query `StorageSpaceManager` directly | Avoids duplicating state, always current |
| Fleet management | `FleetManager` class | Follows pattern of StorageSpaceManager, StationManager, etc. with O(1) lookups |
| When to emit | After batch assigned to storage spaces | Measures state after tasks added to racks/totes, ~5s frequency |
| Code organization | Extend `TpqCalculator` class | Keep TPQ logic together, static methods unchanged |

## Design

### FleetManager Class

**Location:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/FleetManager.java`

Manages robot fleet with immutable access and fast lookups:

```java
@Slf4j
public class FleetManager {
    private final Map<Integer, Robot> robotsById;
    private final List<Robot> robots;

    public FleetManager(List<Robot> robots) {
        this.robots = new ArrayList<>(robots);
        this.robotsById = robots.stream()
            .collect(Collectors.toMap(Robot::getRobotId, r -> r));
        
        log.info("FleetManager initialized with {} robots", robots.size());
    }

    /**
     * Get robot by ID.
     * @param robotId The robot ID to lookup
     * @return Optional containing the robot if found, empty otherwise
     */
    public Optional<Robot> getRobot(int robotId) {
        return Optional.ofNullable(robotsById.get(robotId));
    }

    /**
     * Get immutable copy of all robots.
     */
    public List<Robot> getAllRobots() {
        return List.copyOf(robots);
    }

    /**
     * Get number of robots in fleet.
     */
    public int getFleetSize() {
        return robots.size();
    }
}
```

### WarehouseComponents Record

**Location:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/common/WarehouseComponents.java`

Container for warehouse entity managers needed by TPQ calculator:

```java
public record WarehouseComponents(
    StorageSpaceManager storageSpaceManager,
    StationManager stationManager,
    RelayPositionManager relayPositionManager,  // null for RTP system
    FleetManager fleetManager
) {}
```

### TpqCalculator Changes

**Location:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/TpqCalculator.java`

**New instance-based functionality:**

```java
public final class TpqCalculator {

    // ========== INSTANCE FIELDS (new) ==========
    private final WarehouseComponents warehouseComponents;
    private final SimulationConfig config;
    private final double robotSpeed;
    private final double navFactor;
    private final double operatorTimePerPick;

    // ========== CONSTRUCTOR (new) ==========
    public TpqCalculator(WarehouseComponents components, SimulationConfig config) {
        this.warehouseComponents = components;
        this.config = config;

        // Cache derived values
        if (config.systemType() == SystemType.RTP) {
            this.robotSpeed = config.getRtpRobotConfig().robotSpeed();
        } else {
            // For Relay: Use HTM speed since we measure delivery to station
            // Note: Full Relay TPQ should account for VTM→Relay→Station path,
            // but this simplified version uses HTM speed as approximation
            this.robotSpeed = config.getRelayRobotConfig().htmSpeed();
        }
        // navFactor accounts for grid navigation overhead (typically 1.2-1.5)
        this.navFactor = config.algorithmConfig().navFactor();
        this.operatorTimePerPick = config.operatorConfig().operatorTimePerPick();
    }

    // ========== INSTANCE METHODS (new) ==========
    
    /**
     * Calculate TPQ for all tasks currently on storage spaces (system-wide).
     * Measures overall system state after batch assignment.
     */
    public TpqSnapshot getCurrentSystemTpq() { ... }

    // ========== STATIC METHODS (unchanged) ==========
    public static TpqMetrics calculate(SimulationConfig config) { ... }
    public static double calculateAvgTravelDistance(...) { ... }
    public static double calculateTpq(double travelTime, double operatorTime) { ... }
}
```

**New result record (nested in TpqCalculator):**

```java
public record TpqSnapshot(
    double tpq,             // systemTime / operatorTime
    double systemTime,      // travelTime + operatorTime
    double operatorTime,    // sum of all task operator times
    double travelTime,      // sum of distances / speed * navFactor
    int taskCount           // number of tasks measured
) {}
```

### getCurrentSystemTpq() Implementation

Calculates TPQ for all tasks currently on storage spaces (system-wide state):

```java
public TpqSnapshot getCurrentSystemTpq() {
    double totalOperatorTime = 0.0;
    double totalTravelTime = 0.0;
    int taskCount = 0;

    StorageSpaceManager ssm = warehouseComponents.storageSpaceManager();
    StationManager stationManager = warehouseComponents.stationManager();

    if (config.systemType() == SystemType.RTP) {
        // RTP system - iterate racks
        for (Rack rack : ssm.getAllRacks()) {
            if (rack.getTasks().isEmpty()) continue;

            GridCoordinate rackPos = rack.getCurrentCoordinate();

            for (PickTask task : rack.getTasks()) {
                totalOperatorTime += task.getQuantity() * operatorTimePerPick;

                StationId stationId = task.getStationId();
                GridCoordinate stationPos = stationManager.getStation(stationId).getPosition().position();
                double distance = manhattanDistance(rackPos, stationPos);
                totalTravelTime += (distance / robotSpeed) * navFactor;

                taskCount++;
            }
        }
    } else {
        // Relay system - iterate totes
        for (Tote tote : ssm.getAllTotes()) {
            if (tote.getTasks().isEmpty()) continue;

            GridCoordinate totePos = getTotePosition(tote);

            for (PickTask task : tote.getTasks()) {
                totalOperatorTime += task.getQuantity() * operatorTimePerPick;

                StationId stationId = task.getStationId();
                GridCoordinate stationPos = stationManager.getStation(stationId).getPosition().position();
                double distance = manhattanDistance(totePos, stationPos);
                totalTravelTime += (distance / robotSpeed) * navFactor;

                taskCount++;
            }
        }
    }

    double systemTime = totalTravelTime + totalOperatorTime;
    double tpq = totalOperatorTime > 0 ? systemTime / totalOperatorTime : 0.0;

    return new TpqSnapshot(tpq, systemTime, totalOperatorTime, totalTravelTime, taskCount);
}

private GridCoordinate getTotePosition(Tote tote) {
    ToteLocation location = warehouseComponents.storageSpaceManager().getToteLocation(tote.getId());
    return switch (location) {
        case ToteLocation.InRack inRack ->
            warehouseComponents.storageSpaceManager().getRack(inRack.rackId()).getCurrentCoordinate();
        case ToteLocation.AtRelay atRelay ->
            warehouseComponents.relayPositionManager().getRelayPosition(atRelay.relayId()).getPosition();
        case ToteLocation.AtStation atStation ->
            warehouseComponents.stationManager().getStation(atStation.stationId()).getPosition().position();
        case ToteLocation.WithRobot withRobot ->
            // Use FleetManager for O(1) robot lookup with Optional
            warehouseComponents.fleetManager()
                .getRobot(withRobot.robotId())
                .map(Robot::getCurrentPosition)
                .orElse(null);
    };
}

private double manhattanDistance(GridCoordinate a, GridCoordinate b) {
    return Math.abs(a.row() - b.row()) + Math.abs(a.col() - b.col());
}
```

### DesmojSimulationEngine Changes

Instantiate `FleetManager` and `TpqCalculator` alongside `OrderManager`:

```java
// Where OrderManager is created
OrderManager orderManager = new OrderManager(...);

// Create FleetManager (manages robot fleet with fast lookups)
FleetManager fleetManager = new FleetManager(robots);

// Create warehouse components container
WarehouseComponents warehouseComponents = new WarehouseComponents(
    storageSpaceManager,
    stationManager,
    relayPositionManager,  // null for RTP
    fleetManager
);

// Create TpqCalculator with components and config
TpqCalculator tpqCalculator = new TpqCalculator(warehouseComponents, config);

// Pass to model/bin management process
```

### DesmojBinManagementProcess Changes

**New field and constructor parameter:**

```java
private final TpqCalculator tpqCalculator;

public DesmojBinManagementProcess(
    // ... existing parameters ...
    TpqCalculator tpqCalculator
) {
    // ... existing code ...
    this.tpqCalculator = tpqCalculator;
}
```

**Emit event after batch assignment:**

```java
if (!newTasks.isEmpty()) {
    log.debug("Bin management assigned {} new tasks at t={}", newTasks.size(), currentTime);

    assignTasksAndUpdateTracking(newTasks);

    // Calculate system-wide TPQ (overall state after assignment)
    TpqSnapshot snapshot = tpqCalculator.getCurrentSystemTpq();
    
    // Count unique orders in batch (batchSize = order count, not task count)
    int batchSize = (int) newTasks.stream()
        .map(PickTask::getOrderId)
        .distinct()
        .count();
    
    // Build additional data
    Map<String, Object> additionalData = new HashMap<>();
    additionalData.put("task_count", newTasks.size());
    additionalData.put("batch_size", batchSize);
    additionalData.put("system_task_count", snapshot.taskCount());
    
    // Emit event with system TPQ as primary metric
    metricsManager.emit(DomainEvent.orderBatchSelected(
        currentTime,
        batchSize,                  // order count (not task count)
        snapshot.tpq(),             // system TPQ
        snapshot.systemTime(),      // system time
        snapshot.operatorTime(),    // operator time
        additionalData
    ));

    log.debug("System TPQ at t={}: {:.2f} ({} tasks)", currentTime, snapshot.tpq(), snapshot.taskCount());

    // ... existing callback code ...
}
```

## Data Flow

```
DesmojSimulationEngine
  ├── Creates FleetManager(robots)
  ├── Creates WarehouseComponents(ssm, stationManager, relayManager, fleetManager)
  ├── Creates TpqCalculator(warehouseComponents, config)
  └── Creates DesmojBinManagementProcess(... tpqCalculator)

DesmojBinManagementProcess.lifeCycle()
  └── Every 5 seconds:
      ├── orderManager.processOrders() → List<PickTask>
      ├── if (!newTasks.isEmpty()):
      │   ├── assignTasksAndUpdateTracking(newTasks)
      │   ├── tpqCalculator.getCurrentSystemTpq() → TpqSnapshot
      │   ├── Count unique orders for batchSize
      │   └── metricsManager.emit(ORDER_BATCH_SELECTED with system TPQ)
      └── TaskPoolQualityCollector receives event → builds timeline
```

## Files to Modify

| File | Changes |
|------|---------|
| `FleetManager.java` (new) | Create manager class with robotsById map, getRobot(), getAllRobots(), getFleetSize() |
| `WarehouseComponents.java` (new) | Create record with storageSpaceManager, stationManager, relayPositionManager, fleetManager |
| `TpqCalculator.java` | Add constructor, instance fields, `getCurrentSystemTpq()`, `TpqSnapshot` record, helper methods |
| `DesmojSimulationEngine.java` | Create FleetManager, create WarehouseComponents, instantiate TpqCalculator, pass to model |
| `DesmojWarehouseModel.java` | Pass `TpqCalculator` to bin management process |
| `DesmojBinManagementProcess.java` | Add field, constructor param, emit ORDER_BATCH_SELECTED event with system TPQ after batch assignment |

## Testing

1. Run RTP simulation - verify `taskPoolQualityMetrics` is populated
2. Run Relay simulation - verify tote position resolution works (uses FleetManager)
3. Check timeline has data points at ~5s intervals
4. Verify system TPQ values are reasonable (typically 1.5-3.0 range)
5. Confirm batchSize equals order count (not task count)
6. Verify system_task_count in additionalData shows total tasks on storage spaces
7. Check that TPQ changes over time as tasks are added/removed from system

## Future Enhancements

**Batch TPQ (Phase 2):**
- Add `calculateBatchTpq(List<PickTask> tasks)` method to TpqCalculator
- Emit both batch TPQ and system TPQ in ORDER_BATCH_SELECTED event
- Compare batch quality vs system state over time
- Helps identify if algorithm is selecting good batches vs just maintaining steady state
