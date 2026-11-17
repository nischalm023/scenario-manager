# Desmo-J Station Helper Refactoring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract station task processing logic from `DesmojRtpRobotProcess` into a reusable `DesmojStationHelper` class to eliminate code duplication and enable future relay system implementation.

**Architecture:** Create a stateless static helper class that receives a Desmo-J `SimProcess` context and can call `hold()` to advance simulation time. The helper handles polymorphic storage spaces (Rack/Tote) and batches time advancement for efficiency. Robot processes delegate task processing to this helper while maintaining control of operator resource acquisition/release.

**Tech Stack:** Java 21, Desmo-J 2.5.1e, Lombok, Spring Boot 3.2

---

## Context for Engineers

### What is this codebase?

This is a warehouse robotics discrete event simulator with dual backend implementations (Python/SimPy and Java/Desmo-J). The Java backend uses Desmo-J framework for process-oriented simulation.

### Current Problem

`DesmojRtpRobotProcess` has ~66 lines of task processing logic (lines 296-362) that will need to be duplicated when we add relay system processes (`DesmojVtmProcess`, `DesmojHtmProcess`). We already have `StationOrchestrator` that does this for the custom DES implementation, but it's incompatible with Desmo-J (uses different time advancement API).

### What We're Building

A new `DesmojStationHelper` class that:
- Encapsulates station task processing logic for Desmo-J
- Works with both Rack (RTP system) and Tote (Relay system)
- Receives calling process context to call `hold()` for time advancement
- Batches time advancement (single hold at end, not per-task)
- Matches `StationOrchestrator` business logic pattern

### Key Desmo-J Concepts

**SimProcess**: Base class for processes in Desmo-J, has `lifeCycle()` method and can call `hold()` to advance simulation time.

**hold()**: Blocks the process for a duration (advances simulation time). Critical pattern: `process.hold(new TimeSpan(duration, TimeUnit.SECONDS))` throws `SuspendExecution`.

**SuspendExecution**: Special exception for Desmo-J fiber support - must be declared on any method that calls `hold()`.

**presentTime()**: Returns current simulation time as `TimeInstant`, call `.getTimeAsDouble(TimeUnit.SECONDS)` to get numeric value.

### Testing Strategy

No new tests needed - existing `DesmojIntegrationTest` validates end-to-end behavior. We're refactoring (moving code), not changing behavior. Integration test should pass without modification after refactoring.

---

## Task 1: Create DesmojStationHelper Class Structure

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java`

**Step 1: Create package directory**

```bash
mkdir -p apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper
```

**Step 2: Create class file with imports and skeleton**

Create file: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java`

```java
package com.greyorange.dessimulator.infrastructure.desmoj.helper;

import co.paralleluniverse.fibers.SuspendExecution;
import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.robot.PickTask;
import com.greyorange.dessimulator.domain.robot.Robot;
import com.greyorange.dessimulator.domain.station.Station;
import com.greyorange.dessimulator.infrastructure.desmoj.model.DesmojWarehouseModel;
import com.greyorange.dessimulator.simulator.metrics.MetricEvent;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import com.greyorange.dessimulator.simulator.metrics.SimulationMetrics.ProcessingMetrics;
import desmoj.core.simulator.SimProcess;
import desmoj.core.simulator.TimeSpan;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.TimeUnit;
import java.util.function.BiConsumer;
import lombok.extern.slf4j.Slf4j;

/**
 * Helper class for processing storage space tasks at stations in Desmo-J simulations.
 *
 * <p>This stateless helper encapsulates task processing logic for both RTP (Rack) and Relay (Tote)
 * systems. It receives a calling SimProcess to advance simulation time via hold() calls.
 *
 * <p>Key responsibilities:
 * - Process tasks for a specific station from a storage space (Rack or Tote)
 * - Advance simulation time in batches (single hold at end)
 * - Consume inventory and emit metrics events
 * - Track order completion via callback
 * - Return aggregated processing metrics
 */
@Slf4j
public class DesmojStationHelper {

  /** Private constructor - this is a utility class with only static methods. */
  private DesmojStationHelper() {
    throw new UnsupportedOperationException("Utility class - do not instantiate");
  }

  // Methods will be added in next tasks
}
```

**Step 3: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Build success

**Step 4: Commit skeleton**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java
git commit -m "feat: add DesmojStationHelper skeleton class"
```

---

## Task 2: Add Polymorphic Storage Space Helper Methods

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java`

**Step 1: Add getStorageSpaceId helper method**

Add this method to `DesmojStationHelper` class (after the private constructor):

```java
/**
 * Extract storage space ID from polymorphic storage space object.
 *
 * @param storageSpace Rack or Tote
 * @return StorageSpaceId
 * @throws IllegalArgumentException if storageSpace is neither Rack nor Tote
 */
private static StorageSpaceId getStorageSpaceId(Object storageSpace) {
  if (storageSpace instanceof Rack) {
    return ((Rack) storageSpace).getId();
  } else if (storageSpace instanceof Tote) {
    return ((Tote) storageSpace).getId();
  }
  throw new IllegalArgumentException(
      "Storage space must be Rack or Tote, got: " + storageSpace.getClass().getName());
}
```

**Step 2: Add hasTasksForStation helper method**

```java
/**
 * Check if storage space has tasks for a specific station.
 *
 * @param storageSpace Rack or Tote
 * @param stationId Station to check
 * @return true if storage space has tasks for this station
 */
private static boolean hasTasksForStation(Object storageSpace, StationId stationId) {
  if (storageSpace instanceof Rack) {
    return ((Rack) storageSpace).hasTasksForStation(stationId);
  } else if (storageSpace instanceof Tote) {
    return ((Tote) storageSpace).hasTasksForStation(stationId);
  }
  return false;
}
```

**Step 3: Add getNextTaskForStation helper method**

```java
/**
 * Get and remove next task for a specific station from storage space.
 *
 * <p>IMPORTANT: This removes the task from the storage space.
 *
 * @param storageSpace Rack or Tote
 * @param stationId Station to get task for
 * @return Optional containing the task, or empty if no tasks for this station
 */
private static Optional<PickTask> getNextTaskForStation(
    Object storageSpace, StationId stationId) {
  if (storageSpace instanceof Rack) {
    return ((Rack) storageSpace).getNextTaskForStation(stationId);
  } else if (storageSpace instanceof Tote) {
    return ((Tote) storageSpace).getNextTaskForStation(stationId);
  }
  return Optional.empty();
}
```

**Step 4: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Build success

**Step 5: Commit helper methods**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java
git commit -m "feat: add polymorphic storage space helper methods to DesmojStationHelper"
```

---

## Task 3: Add Metrics Emission Helper Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java`

**Step 1: Add emitTaskCompletionEvent helper method**

Add this method to `DesmojStationHelper` class:

```java
/**
 * Emit task completion event to metrics manager.
 *
 * <p>CRITICAL: taskId and orderId must be passed as direct parameters to emitEvent(), NOT in the
 * data map. This is required for metrics collectors to work correctly.
 *
 * @param metricsManager Metrics manager (can be null)
 * @param robot Robot processing the task
 * @param task Task being completed
 * @param storageSpaceId Storage space ID (rack or tote)
 * @param callingProcess Calling process (for getting current time)
 */
private static void emitTaskCompletionEvent(
    MetricsManager metricsManager,
    Robot robot,
    PickTask task,
    StorageSpaceId storageSpaceId,
    SimProcess callingProcess) {

  if (metricsManager == null) {
    return;
  }

  Map<String, Object> eventData = new HashMap<>();
  eventData.put("robot_id", robot.getId());
  eventData.put("station_id", task.getStationId().value());

  // CRITICAL: Pass taskId and orderId as direct parameters, NOT in data map
  metricsManager.emitEvent(
      MetricEvent.TASK_COMPLETED,
      callingProcess.presentTime().getTimeAsDouble(TimeUnit.SECONDS),
      robot.getId(), // robotId parameter
      null, // rackId (not needed for task completion)
      task.getTaskId(), // taskId parameter - CRITICAL
      task.getOrderId(), // orderId parameter - CRITICAL
      eventData);
}
```

**Step 2: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Build success

**Step 3: Commit metrics helper**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java
git commit -m "feat: add task completion event emission helper to DesmojStationHelper"
```

---

## Task 4: Implement Main processStorageSpaceTasks Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java`

**Step 1: Add main public method**

Add this method to `DesmojStationHelper` class (after private constructor, before helper methods):

```java
/**
 * Process storage space tasks at a station with batched time advancement.
 *
 * <p>This method:
 * 1. Extracts configuration and dependencies from the model
 * 2. Processes all tasks for the specified station from the storage space
 * 3. Consumes inventory for each task
 * 4. Emits metrics events for task completion
 * 5. Tracks order completion via callback
 * 6. Advances simulation time in a SINGLE batch hold at the end (efficient)
 * 7. Returns aggregated processing metrics
 *
 * <p>The storage space (Rack or Tote) owns its tasks. This method queries the storage space
 * directly and removes tasks as they're processed.
 *
 * <p>IMPORTANT: Robot process must acquire operator resource BEFORE calling this method and
 * release it AFTER this method returns. This method does NOT handle operator resource management.
 *
 * @param callingProcess The SimProcess invoking this helper (for hold() calls)
 * @param model The warehouse model (provides config, inventory, metrics)
 * @param robot Robot delivering the storage space
 * @param storageSpace Rack or Tote with tasks to process
 * @param station Station where processing occurs
 * @param completionTracker Callback for order completion checking (taskId, orderId) - can be null
 * @return ProcessingMetrics with aggregated results
 * @throws SuspendExecution Required for Desmo-J fiber support
 */
public static ProcessingMetrics processStorageSpaceTasks(
    SimProcess callingProcess,
    DesmojWarehouseModel model,
    Robot robot,
    Object storageSpace,
    Station station,
    BiConsumer<String, String> completionTracker)
    throws SuspendExecution {

  // Extract dependencies from model
  WarehouseInventory inventory = model.getInventory();
  MetricsManager metricsManager = model.getMetricsManager();
  double operatorTimePerPick = model.getConfig().operatorConfig().operatorTimePerPick();

  // Initialize tracking variables
  int totalPicks = 0;
  int tasksProcessed = 0;
  double totalOperatorTime = 0.0;
  List<Map<String, Object>> taskDetails = new ArrayList<>();

  StationId stationId = station.getStationId();
  StorageSpaceId storageSpaceId = getStorageSpaceId(storageSpace);

  log.debug(
      "Processing tasks at station {} for storage space {} (robot {})",
      stationId,
      storageSpaceId,
      robot.getId());

  // Process all tasks for THIS station (not other stations on the storage space)
  while (hasTasksForStation(storageSpace, stationId)) {
    // Get next task for this station (removes it from storage space)
    PickTask task =
        getNextTaskForStation(storageSpace, stationId)
            .orElseThrow(
                () ->
                    new IllegalStateException(
                        "hasTasksForStation returned true but getNextTaskForStation returned"
                            + " empty"));

    robot.setCurrentTask(task);
    int picks = task.getQuantity();

    // Track task details for metrics
    Map<String, Object> taskDetail = new HashMap<>();
    taskDetail.put("task_id", task.getTaskId());
    taskDetail.put("sku", task.getItem().sku());
    taskDetail.put("quantity", picks);
    taskDetail.put("processing_time", operatorTimePerPick);
    taskDetail.put("station_id", stationId.value());
    taskDetails.add(taskDetail);

    // Consume inventory
    inventory.consumeStock(task.getItem().sku(), storageSpaceId, task.getQuantity());

    // Emit task completion event
    emitTaskCompletionEvent(metricsManager, robot, task, storageSpaceId, callingProcess);

    // Track order completion
    if (completionTracker != null) {
      completionTracker.accept(task.getTaskId(), task.getOrderId());
    }

    // Update totals
    totalPicks += picks;
    tasksProcessed++;
    totalOperatorTime += operatorTimePerPick;

    log.debug(
        "Robot {} completed task {} at station {} (picks: {}, total operator time so far: {}s)",
        robot.getId(),
        task.getTaskId(),
        stationId,
        picks,
        totalOperatorTime);
  }

  // BATCHED TIME ADVANCEMENT: Single hold at end for total operator time
  // This is much more efficient than calling hold() once per task
  if (totalOperatorTime > 0) {
    log.debug(
        "Robot {} advancing simulation time by {}s for operator work at station {}",
        robot.getId(),
        totalOperatorTime,
        stationId);
    callingProcess.hold(new TimeSpan(totalOperatorTime, TimeUnit.SECONDS));
  }

  log.debug(
      "Completed processing at station {}: {} tasks, {} picks, {}s operator time",
      stationId,
      tasksProcessed,
      totalPicks,
      totalOperatorTime);

  // Return aggregated metrics
  return ProcessingMetrics.builder()
      .totalPicks(totalPicks)
      .tasksProcessed(tasksProcessed)
      .processingTime(totalOperatorTime)
      .tasksDetail(taskDetails)
      .build();
}
```

**Step 2: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Build success

**Step 3: Commit main method**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/helper/DesmojStationHelper.java
git commit -m "feat: implement processStorageSpaceTasks main method in DesmojStationHelper"
```

---

## Task 5: Refactor DesmojRtpRobotProcess to Use Helper

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Step 1: Add import for DesmojStationHelper**

Add this import at the top of the file (after existing imports, in alphabetical order):

```java
import com.greyorange.dessimulator.infrastructure.desmoj.helper.DesmojStationHelper;
```

**Step 2: Update call site to use helper (lines 198-209)**

Find this code block (around lines 198-209):

```java
// 4c. Process tasks for this station
List<PickTask> tasksForStation = stationTasks.get(stationId);
ProcessingMetrics processingMetrics =
    processTasksAtStation(selectedRack, targetStation, tasksForStation);
allProcessingMetrics.add(processingMetrics);

log.debug(
    "Robot {} processed {} tasks with {} picks at station {} (operator time: {}s)",
    robot.getId(),
    processingMetrics.tasksProcessed(),
    processingMetrics.totalPicks(),
    stationId,
    processingMetrics.processingTime());
```

Replace it with:

```java
// 4c. Process tasks for this station using helper
ProcessingMetrics processingMetrics =
    DesmojStationHelper.processStorageSpaceTasks(
        this, // callingProcess
        (DesmojWarehouseModel) getModel(), // model
        robot, // robot
        selectedRack, // storageSpace
        targetStation, // station
        orderCompletionCallback); // completionTracker
allProcessingMetrics.add(processingMetrics);

log.debug(
    "Robot {} processed {} tasks with {} picks at station {} (operator time: {}s)",
    robot.getId(),
    processingMetrics.tasksProcessed(),
    processingMetrics.totalPicks(),
    stationId,
    processingMetrics.processingTime());
```

**Step 3: Remove now-unused local variable (line 128)**

Find this line (around line 128):

```java
// 3. Get stations to visit
Map<StationId, List<PickTask>> stationTasks = selectedRack.getTasksByStation();
if (stationTasks.isEmpty()) {
```

Replace it with:

```java
// 3. Get stations to visit
Map<StationId, List<PickTask>> stationTasks = selectedRack.getTasksByStation();
if (stationTasks.isEmpty()) {
```

Actually, keep `stationTasks` - it's still used to check if rack has tasks and get station list. Only the per-station task list extraction is removed.

**Step 4: Delete the old processTasksAtStation method (lines 295-362)**

Find and DELETE this entire method:

```java
/** Process tasks at station. */
private ProcessingMetrics processTasksAtStation(Rack rack, Station station, List<PickTask> tasks)
    throws SuspendExecution {
  // ... entire method body ...
}
```

Delete from line 295 to line 362 (68 lines total).

**Step 5: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: Build success

**Step 6: Commit refactoring**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java
git commit -m "refactor: use DesmojStationHelper in DesmojRtpRobotProcess"
```

---

## Task 6: Run Integration Tests to Validate

**Files:**
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/infrastructure/desmoj/DesmojIntegrationTest.java`

**Step 1: Run integration tests**

```bash
cd apps/backend
mvn test -Dtest=DesmojIntegrationTest
```

Expected: All tests pass

**Step 2: Verify no behavior changes**

The tests should pass without modification because we only refactored (moved code), didn't change behavior.

If tests fail, check:
- Metrics events still emitted correctly (taskId/orderId as direct parameters)
- Timing unchanged (single hold at end should produce same total time)
- Task removal still happening (getNextTaskForStation removes tasks)
- Inventory consumption still occurring

**Step 3: Run full test suite**

```bash
cd apps/backend
mvn test
```

Expected: All tests pass

---

## Task 7: Code Formatting and Final Verification

**Files:**
- All modified Java files

**Step 1: Run Google Java Format**

```bash
cd apps/backend
mvn spotless:apply
```

Expected: Formatting applied successfully

**Step 2: Verify code style**

```bash
cd apps/backend
mvn spotless:check
```

Expected: No violations

**Step 3: Final compilation check**

```bash
cd apps/backend
mvn clean compile
```

Expected: Build success

**Step 4: Commit formatting changes (if any)**

```bash
git add -u
git commit -m "style: apply Google Java Format to refactored files"
```

---

## Task 8: Update CLAUDE.md Documentation

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Add DesmojStationHelper to Java Backend Architecture section**

Find the "Java Backend Architecture" section (around line 363) and update the **Infrastructure** subsection to mention the new helper:

Find this section:

```markdown
**Infrastructure** (`infrastructure/`):
- `des/` - Custom DES implementation (original, deprecated)
  - `core/` - SimulationEngine, SimulationClock
  - `event/` - Event, EventQueue, EventPriority
  - `process/` - SimulationProcess, ProcessScheduler
  - `resource/` - Resource (SimPy-style blocking), ResourceStats
- `desmoj/` - **Desmo-J production implementation** (active)
  - `DesmojSimulationEngine.java` - Experiment coordinator
  - `model/DesmojWarehouseModel.java` - Model & process scheduling
  - `process/` - DesmojRtpRobotProcess, DesmojBinManagementProcess, DesmojMetricMonitoringProcess
  - Uses Desmo-J 2.5.1e (libs/desmoj-2.5.1e-bin.jar)
```

Update to:

```markdown
**Infrastructure** (`infrastructure/`):
- `des/` - Custom DES implementation (original, deprecated)
  - `core/` - SimulationEngine, SimulationClock
  - `event/` - Event, EventQueue, EventPriority
  - `process/` - SimulationProcess, ProcessScheduler
  - `resource/` - Resource (SimPy-style blocking), ResourceStats
- `desmoj/` - **Desmo-J production implementation** (active)
  - `DesmojSimulationEngine.java` - Experiment coordinator
  - `model/DesmojWarehouseModel.java` - Model & process scheduling
  - `process/` - DesmojRtpRobotProcess, DesmojBinManagementProcess, DesmojMetricMonitoringProcess
  - `helper/` - DesmojStationHelper (stateless task processing helper)
  - Uses Desmo-J 2.5.1e (libs/desmoj-2.5.1e-bin.jar)
```

**Step 2: Commit documentation update**

```bash
git add CLAUDE.md
git commit -m "docs: add DesmojStationHelper to architecture documentation"
```

---

## Task 9: Manual Verification with Background Server

**Files:**
- Test: Manual API testing

**Step 1: Start background server**

```bash
cd apps/backend
mvn clean compile
mvn spring-boot:run > /tmp/server.log 2>&1 &
echo $!
```

Save the PID for later.

**Step 2: Run simulation via API**

```bash
curl -X POST http://localhost:8080/simulations/run \
  -H "Content-Type: application/json" \
  -d @src/test/resources/integration-test-config.json \
  | jq . > /tmp/simulation_response.json
```

**Step 3: Verify response metrics**

```bash
# Check for cycle metrics
jq '.results' /tmp/simulation_response.json | head -50

# Verify non-zero metrics
jq '.results.robotMetrics.totalCycles' /tmp/simulation_response.json
jq '.results.robotMetrics.averageCycleTime' /tmp/simulation_response.json
```

Expected: Non-zero values, similar to pre-refactoring results

**Step 4: Check server logs for errors**

```bash
grep -i "error\|exception" /tmp/server.log
```

Expected: No errors related to DesmojStationHelper or task processing

**Step 5: Check task completion events**

```bash
grep "TASK_COMPLETED" /tmp/server.log | head -20
```

Expected: Task completion events being emitted

**Step 6: Kill background server**

```bash
pkill -f "spring-boot:run"
```

**Step 7: Verify manual testing complete**

No commit needed - this was verification only.

---

## Completion Checklist

After completing all tasks, verify:

- [ ] `DesmojStationHelper` class created with all methods
- [ ] Polymorphic storage space handling (Rack/Tote)
- [ ] Batched time advancement (single hold at end)
- [ ] Metrics emission with correct parameter order
- [ ] `DesmojRtpRobotProcess` refactored to use helper
- [ ] Old `processTasksAtStation` method deleted (~68 lines removed)
- [ ] Integration tests pass without modification
- [ ] Code formatting applied (Google Java Format)
- [ ] Documentation updated in CLAUDE.md
- [ ] Manual API testing successful
- [ ] No errors in server logs
- [ ] All commits made with clear messages

---

## Success Criteria

1. **Code reduction**: ~68 lines removed from `DesmojRtpRobotProcess`
2. **Reusability**: Helper can be used by future relay processes
3. **No behavior change**: Integration tests pass without modification
4. **Same metrics**: API responses show same metric values as before
5. **Performance**: Single hold per station (not per task)
6. **Code quality**: Passes Google Java Format checks

---

## Future Work

After this refactoring is complete, the `DesmojStationHelper` can be reused for:

1. **DesmojVtmProcess** (Vertical Tote Mover) - Relay system
2. **DesmojHtmProcess** (Horizontal Tote Mover) - Relay system
3. Any future robot process implementations

The pattern is:
```java
// Robot acquires operator resource
operatorResource.provide(1);

// Delegate task processing to helper
ProcessingMetrics metrics = DesmojStationHelper.processStorageSpaceTasks(
    this, model, robot, storageSpace, station, callback);

// Robot releases operator resource
operatorResource.takeBack(1);
```

This separation of concerns (robot = resource management, helper = task processing) enables clean code reuse across different robot types.
