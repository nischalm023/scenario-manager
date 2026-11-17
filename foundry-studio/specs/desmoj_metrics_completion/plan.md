# Desmo-J RTP Robot Process Metrics Completion Plan

## Overview

Complete the metrics collection implementation in `DesmojRtpRobotProcess.java` to match the reference implementations in both Python (`rtp_robot_orchestrator.py`) and Java custom DES (`RtpRobotOrchestrator.java`). Currently, the Desmo-J implementation performs all the operations but discards critical timing and performance metrics, making it impossible to analyze transport phases, operator efficiency, and cycle time breakdowns.

## Business Requirements

### Problem Statement
The Desmo-J RTP robot process is functionally complete but **lacks comprehensive metrics collection**, resulting in:
- ❌ Inability to analyze transport phase breakdowns (move to rack, loading, move to station)
- ❌ Loss of processing metrics data (total picks, tasks processed, operator time)
- ❌ Missing return transport analysis (travel time, unloading time)
- ❌ Incomplete cycle events that cannot support performance analytics
- ❌ No per-station metrics aggregation for multi-station cycles

### Success Criteria
1. All transport phases emit detailed timing metrics
2. Processing metrics are captured and included in cycle events
3. Return transport metrics are collected and tracked
4. Cycle completion events contain full metric breakdowns
5. Multi-station cycles aggregate metrics per station
6. Metrics parity with Python SimPy and Java custom DES implementations
7. All existing tests pass without modification
8. Integration test validates comprehensive metrics collection

## Current State Analysis

### Reference Implementations

#### Python SimPy (`rtp_robot_orchestrator.py`)
- **Lines 167-208**: `_transport_rack_to_station()` returns `TransportMetrics`
  - Includes: `start_time`, `loading_time`, `travel_time`, `total_time`, `end_time`
  - Includes: `phases` list with `PhaseData` for each phase
- **Lines 210-250**: `_return_rack_to_storage()` returns `ReturnTransportMetrics`
  - Includes: `start_time`, `travel_time`, `unloading_time`, `total_time`, `end_time`, `destination_position`
- **Lines 98-113**: Collects metrics per station in lists
  - `all_transport_metrics = []`
  - `all_processing_metrics = []`
- **Lines 146-157**: Emits comprehensive `RtpCycleMetrics` with all sub-metrics

#### Java Custom DES (`RtpRobotOrchestrator.java`)
- **Lines 214-243**: `transportRackToStation()` returns `TransportMetrics`
- **Lines 245-275**: `returnRackToStorage()` returns `ReturnTransportMetrics`
- **Lines 127-128**: Declares metric collection lists
- **Lines 141-145**: Captures and stores transport metrics
- **Lines 148-156**: Captures and stores processing metrics
- **Lines 177-179**: Captures return transport metrics
- **Lines 182-196**: Builds comprehensive `RtpCycleMetrics` object
- **Lines 357-378**: Emits complete cycle event with all metric data

### Current Desmo-J Implementation Issues

#### File: `DesmojRtpRobotProcess.java`

**Issue 1: No Transport Metrics Capture** (lines 158-159)
```java
// Current code
transportRackToStation(selectedRack, targetStation.getPosition().position());
// ❌ Method is void, no metrics returned
// ❌ No variable to capture transport timing
```

**Issue 2: Processing Metrics Discarded** (lines 176-178)
```java
// Current code
processTasksAtStation(selectedRack, targetStation, tasksForStation);
// ❌ Method is void, doesn't return ProcessingMetrics
// ❌ StationOrchestrator emits events but robot process doesn't capture aggregated data
```

**Issue 3: No Return Transport Metrics** (lines 190-191)
```java
// Current code
returnRackToStorage(selectedRack);
// ❌ Method is void, no metrics returned
```

**Issue 4: No Metric Collection Lists** (entire file)
```java
// ❌ Missing field declarations
// private List<TransportMetrics> allTransportMetrics;
// private List<ProcessingMetrics> allProcessingMetrics;
```

**Issue 5: Incomplete Cycle Event** (lines 379-400)
```java
private void emitCycleEvent(...) {
    Map<String, Object> data = new HashMap<>();
    data.put("robot_id", robotId);
    data.put("cycle_time", cycleEndTime - cycleStartTime);
    // ... basic fields only
    // ❌ NO transport metrics breakdown
    // ❌ NO processing metrics breakdown
    // ❌ NO return transport metrics
}
```

**Issue 6: Method Signatures Don't Return Metrics**
- `transportRackToStation()`: returns `void` instead of `TransportMetrics`
- `processTasksAtStation()`: returns `void` instead of `ProcessingMetrics`
- `returnRackToStorage()`: returns `void` instead of `ReturnTransportMetrics`

## Technical Architecture

### Metrics Data Models

#### TransportMetrics (from RobotMetrics.java)
```java
@Builder
public record TransportMetrics(
    double startTime,
    double loadingTime,    // 6.0 seconds constant
    double travelTime,     // Sum of movement times
    double totalTime,      // loadingTime + travelTime
    double endTime
) {}
```

#### ProcessingMetrics (from SimulationMetrics.java)
```java
@Builder
public record ProcessingMetrics(
    int totalPicks,           // Sum of all quantities
    int tasksProcessed,       // Count of tasks
    double processingTime,    // Total operator time
    List<Map<String, Object>> tasksDetail  // Per-task details
) {}
```

#### ReturnTransportMetrics (from RobotMetrics.java)
```java
@Builder
public record ReturnTransportMetrics(
    double startTime,
    double travelTime,
    double unloadingTime,  // 6.0 seconds constant
    double totalTime,      // travelTime + unloadingTime
    double endTime,
    GridCoordinate destinationPosition
) {}
```

### Component Interaction Flow

```
DesmojRtpRobotProcess (Main Loop)
  │
  ├─► transportRackToStation()
  │     └─► Returns TransportMetrics
  │           └─► Store in list per station
  │
  ├─► processTasksAtStation()
  │     └─► Tracks picks/tasks internally
  │     └─► Returns ProcessingMetrics
  │           └─► Store in list per station
  │
  ├─► returnRackToStorage()
  │     └─► Returns ReturnTransportMetrics
  │
  └─► emitRtpCycle()
        └─► Includes ALL metrics in event data
              ├─► Transport breakdown
              ├─► Processing breakdown
              └─► Return breakdown
```

### Desmo-J Specific Considerations

**Process Structure:**
- All timing tracked via `presentTime().getTimeAsDouble(TimeUnit.SECONDS)`
- Delays via `hold(new TimeSpan(duration, TimeUnit.SECONDS))`
- Metrics must be captured **before** `hold()` calls complete
- Return metrics objects instead of void

**Timing Accuracy:**
- Start time: Capture before first operation
- End time: Capture after all `hold()` calls complete
- Total time: Pre-calculated (not dependent on actual hold() timing)
- Desmo-J advances time only during `hold()`, not during calculations

**Resource Coordination:**
- Operator resources handled in `processTasksAtStation()`
- Transport metrics independent of operator blocking
- Return metrics independent of all resources

## Detailed Implementation Tasks

### Task 1: Add Metric Collection Lists to DesmojRtpRobotProcess

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** After line 48 (after field declarations)

**Changes:**
```java
// Add metric collection fields
private final List<TransportMetrics> allTransportMetrics = new ArrayList<>();
private final List<ProcessingMetrics> allProcessingMetrics = new ArrayList<>();
```

**Verification:**
- [ ] Lists initialized as instance fields
- [ ] Cleared at start of each robot cycle
- [ ] Used to store per-station metrics

**Why:** Multi-station cycles require aggregating metrics from each station visit. These lists enable per-station analysis and comprehensive cycle reporting.

---

### Task 2: Refactor transportRackToStation() to Return TransportMetrics

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** Lines 217-234 (replace entire method)

**Current Signature:**
```java
private void transportRackToStation(Rack rack, GridCoordinate stationPosition)
    throws SuspendExecution
```

**New Signature:**
```java
private TransportMetrics transportRackToStation(Rack rack, GridCoordinate stationPosition)
    throws SuspendExecution
```

**Implementation:**
```java
private TransportMetrics transportRackToStation(Rack rack, GridCoordinate stationPosition)
    throws SuspendExecution {

  double startTime = now();

  // Phase 1: Robot moves to rack
  robot.setState(RobotState.MOVING_TO_RACK);
  double travelTimeToRack = robot.moveToPosition(rack.getCurrentCoordinate());
  hold(new TimeSpan(travelTimeToRack, TimeUnit.SECONDS));

  // Phase 2: Loading time (attach to rack)
  robot.setState(RobotState.LOADING_RACK);
  double loadingTime = LOADING_TIME; // 6.0 seconds
  hold(new TimeSpan(loadingTime, TimeUnit.SECONDS));

  // Phase 3: Transport to station
  robot.setState(RobotState.TRANSPORTING_RACK_TO_STATION);
  double travelTimeToStation = robot.moveToPosition(stationPosition);
  rack.moveToStation(stationPosition);
  hold(new TimeSpan(travelTimeToStation, TimeUnit.SECONDS));

  // Calculate metrics
  double travelTime = travelTimeToRack + travelTimeToStation;
  double totalTime = travelTimeToRack + loadingTime + travelTimeToStation;
  double endTime = now();

  // Build and return metrics
  return TransportMetrics.builder()
      .startTime(startTime)
      .loadingTime(loadingTime)
      .travelTime(travelTime)
      .totalTime(totalTime)
      .endTime(endTime)
      .build();
}
```

**Key Changes:**
1. Return type changed from `void` to `TransportMetrics`
2. Capture `startTime` before any operations
3. Calculate `travelTime` as sum of both movement phases
4. Calculate `totalTime` including loading time
5. Capture `endTime` after all `hold()` calls
6. Build and return `TransportMetrics` object

**Verification:**
- [ ] Return type matches reference implementation
- [ ] Start time captured before first operation
- [ ] End time captured after all holds complete
- [ ] Loading time constant (6.0s) included
- [ ] Travel time includes both movement phases
- [ ] Total time = travelTimeToRack + loadingTime + travelTimeToStation
- [ ] All robot state transitions preserved
- [ ] Rack state updates preserved

**Why:** Transport metrics enable analysis of:
- Loading efficiency (is 6s optimal?)
- Travel time optimization opportunities
- Bottlenecks in movement phases
- Cycle time breakdown (what % is transport vs. processing)

---

### Task 3: Refactor processTasksAtStation() to Return ProcessingMetrics

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** Lines 236-279 (replace entire method)

**Current Signature:**
```java
private void processTasksAtStation(Rack rack, Station station, List<PickTask> tasks)
    throws SuspendExecution
```

**New Signature:**
```java
private ProcessingMetrics processTasksAtStation(Rack rack, Station station, List<PickTask> tasks)
    throws SuspendExecution
```

**Implementation:**
```java
private ProcessingMetrics processTasksAtStation(Rack rack, Station station, List<PickTask> tasks)
    throws SuspendExecution {

  double operatorTimePerPick =
      ((DesmojWarehouseModel) getModel()).getConfig().operatorConfig().operatorTimePerPick();

  int totalPicks = 0;
  int tasksProcessed = 0;
  double totalOperatorTime = 0.0;
  List<Map<String, Object>> taskDetails = new ArrayList<>();

  for (PickTask task : tasks) {
    // Simulate picking
    hold(new TimeSpan(operatorTimePerPick, TimeUnit.SECONDS));

    // Track task details
    Map<String, Object> taskDetail = new HashMap<>();
    taskDetail.put("task_id", task.getTaskId());
    taskDetail.put("sku", task.getItem().sku());
    taskDetail.put("quantity", task.getQuantity());
    taskDetail.put("processing_time", operatorTimePerPick);
    taskDetail.put("station_id", station.getStationId().value());
    taskDetails.add(taskDetail);

    // Update inventory
    inventory.consumeStock(task.getItem().sku(), task.getStorageSpaceId(), task.getQuantity());

    // Emit task completion event (PRESERVE EXISTING)
    Map<String, Object> eventData = new HashMap<>();
    eventData.put("robot_id", robot.getId());
    eventData.put("station_id", station.getStationId().value());

    metricsManager.emitEvent(
        MetricEvent.TASK_COMPLETED,
        now(),
        robot.getId(),
        null,
        task.getTaskId(),
        task.getOrderId(),
        eventData);

    // Notify order completion callback (PRESERVE EXISTING)
    if (orderCompletionCallback != null) {
      orderCompletionCallback.accept(task.getTaskId(), task.getOrderId());
    }

    // Update totals
    totalPicks += task.getQuantity();
    tasksProcessed++;
    totalOperatorTime += operatorTimePerPick;

    log.debug(
        "Robot {} completed task {} at station {} at t={}",
        robot.getId(),
        task.getTaskId(),
        station.getStationId(),
        presentTime());
  }

  // Build and return metrics
  return ProcessingMetrics.builder()
      .totalPicks(totalPicks)
      .tasksProcessed(tasksProcessed)
      .processingTime(totalOperatorTime)
      .tasksDetail(taskDetails)
      .build();
}
```

**Key Changes:**
1. Return type changed from `void` to `ProcessingMetrics`
2. Added tracking variables: `totalPicks`, `tasksProcessed`, `totalOperatorTime`, `taskDetails`
3. Accumulate metrics during task processing loop
4. Build task detail maps with relevant information
5. Build and return `ProcessingMetrics` object
6. **PRESERVE** all existing event emissions
7. **PRESERVE** order completion callback

**Verification:**
- [ ] Return type matches reference implementation
- [ ] Total picks = sum of all task quantities
- [ ] Tasks processed = count of tasks in loop
- [ ] Total operator time = tasksProcessed * operatorTimePerPick
- [ ] Task details list populated with per-task info
- [ ] All existing TASK_COMPLETED events still emitted
- [ ] Order completion callback still invoked
- [ ] Inventory consumption still occurs

**Why:** Processing metrics enable analysis of:
- Operator utilization and efficiency
- Pick rate performance
- Task complexity (picks per task)
- Station workload distribution
- Bottleneck identification (is operator or transport limiting throughput?)

---

### Task 4: Refactor returnRackToStorage() to Return ReturnTransportMetrics

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** Lines 281-299 (replace entire method)

**Current Signature:**
```java
private void returnRackToStorage(Rack rack) throws SuspendExecution
```

**New Signature:**
```java
private ReturnTransportMetrics returnRackToStorage(Rack rack) throws SuspendExecution
```

**Implementation:**
```java
private ReturnTransportMetrics returnRackToStorage(Rack rack) throws SuspendExecution {

  double startTime = now();

  robot.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);

  GridCoordinate storagePosition = rack.getStorageCoordinate();

  // Phase 1: Transport to storage position
  double travelTime = robot.moveToPosition(storagePosition);
  rack.moveToStorage(storagePosition);
  hold(new TimeSpan(travelTime, TimeUnit.SECONDS));

  // Phase 2: Unloading time (detach from rack)
  double unloadingTime = UNLOADING_TIME; // 6.0 seconds
  hold(new TimeSpan(unloadingTime, TimeUnit.SECONDS));

  // Complete the return process (PRESERVE EXISTING)
  robot.setCarryingRack(null);
  robot.setCurrentTask(null);
  robot.setState(RobotState.IDLE);

  // Calculate metrics
  double totalTime = travelTime + unloadingTime;
  double endTime = now();

  // Build and return metrics
  return ReturnTransportMetrics.builder()
      .startTime(startTime)
      .travelTime(travelTime)
      .unloadingTime(unloadingTime)
      .totalTime(totalTime)
      .endTime(endTime)
      .destinationPosition(storagePosition)
      .build();
}
```

**Key Changes:**
1. Return type changed from `void` to `ReturnTransportMetrics`
2. Capture `startTime` before operations
3. Calculate `totalTime` as travel + unloading
4. Capture `endTime` after all holds complete
5. Include `destinationPosition` for analysis
6. Build and return `ReturnTransportMetrics` object
7. **PRESERVE** robot state cleanup logic

**Verification:**
- [ ] Return type matches reference implementation
- [ ] Start time captured before first operation
- [ ] End time captured after all holds complete
- [ ] Unloading time constant (6.0s) included
- [ ] Total time = travelTime + unloadingTime
- [ ] Destination position captured
- [ ] Robot state cleanup preserved (carrying_rack = null, state = IDLE)

**Why:** Return transport metrics enable analysis of:
- Return journey optimization (different from outbound?)
- Unloading efficiency (is 6s optimal?)
- Storage position selection effectiveness
- Complete cycle time accounting

---

### Task 5: Capture Metrics in Main Robot Loop

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** Lines 138-208 (within the station visit loop and after)

**Changes:**

**5.1: Initialize metric collection lists** (after line 139, before station loop)
```java
double cycleStartTime = now();

// Clear and initialize metric collection for this cycle
allTransportMetrics.clear();
allProcessingMetrics.clear();

log.info(
    "Robot {} starting cycle with rack {} at t={}",
    robot.getId(),
    selectedRack.getId(),
    presentTime());
```

**5.2: Capture transport metrics** (replace line 159)
```java
// OLD CODE (line 159):
// transportRackToStation(selectedRack, targetStation.getPosition().position());

// NEW CODE:
TransportMetrics transportMetrics =
    transportRackToStation(selectedRack, targetStation.getPosition().position());
allTransportMetrics.add(transportMetrics);

log.debug(
    "Robot {} transported rack {} to station {} (loading: {}s, travel: {}s, total: {}s)",
    robot.getId(),
    selectedRack.getId(),
    stationId,
    transportMetrics.loadingTime(),
    transportMetrics.travelTime(),
    transportMetrics.totalTime());
```

**5.3: Capture processing metrics** (replace lines 176-178)
```java
// OLD CODE (lines 176-178):
// List<PickTask> tasksForStation = stationTasks.get(stationId);
// processTasksAtStation(selectedRack, targetStation, tasksForStation);

// NEW CODE:
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

**5.4: Capture return transport metrics** (replace line 191)
```java
// OLD CODE (line 191):
// returnRackToStorage(selectedRack);

// NEW CODE:
ReturnTransportMetrics returnMetrics = returnRackToStorage(selectedRack);

log.debug(
    "Robot {} returned rack {} to storage (travel: {}s, unloading: {}s, total: {}s)",
    robot.getId(),
    selectedRack.getId(),
    returnMetrics.travelTime(),
    returnMetrics.unloadingTime(),
    returnMetrics.totalTime());
```

**5.5: Update cycle completion event call** (replace lines 203-208)
```java
// OLD CODE (lines 203-208):
// emitCycleEvent(
//     robot.getId(),
//     selectedRack.getId().value(),
//     cycleStartTime,
//     cycleEndTime,
//     stationsToVisit);

// NEW CODE:
emitRtpCycle(
    robot.getId(),
    selectedRack.getId().value(),
    cycleStartTime,
    cycleEndTime,
    stationsToVisit,
    allTransportMetrics,
    allProcessingMetrics,
    returnMetrics);
```

**Verification:**
- [ ] Metric lists cleared at cycle start
- [ ] Transport metrics captured per station
- [ ] Processing metrics captured per station
- [ ] Return metrics captured once per cycle
- [ ] All metrics passed to cycle event emission
- [ ] Debug logging provides visibility
- [ ] No duplicate holds (metrics methods already hold)

**Why:** This wires together all the metric collection to enable comprehensive cycle analysis. Without capturing these return values, the refactored methods would still lose the metric data.

---

### Task 6: Refactor emitCycleEvent() to emitRtpCycle() with Essential Metrics

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** Lines 379-400 (replace entire method)

**Current Signature:**
```java
private void emitCycleEvent(
    int robotId,
    int rackId,
    double cycleStartTime,
    double cycleEndTime,
    List<StationId> stationsVisited)
```

**New Signature:**
```java
private void emitRtpCycle(
    int robotId,
    int rackId,
    double cycleStartTime,
    double cycleEndTime,
    List<StationId> stationsVisited,
    List<TransportMetrics> transportMetrics,
    List<ProcessingMetrics> processingMetrics,
    ReturnTransportMetrics returnMetrics)
```

**Implementation:**
```java
private void emitRtpCycle(
    int robotId,
    int rackId,
    double cycleStartTime,
    double cycleEndTime,
    List<StationId> stationsVisited,
    List<TransportMetrics> transportMetrics,
    List<ProcessingMetrics> processingMetrics,
    ReturnTransportMetrics returnMetrics) {

  if (metricsManager == null) {
    return;
  }

  Map<String, Object> data = new HashMap<>();

  // Basic cycle info (PRESERVE EXISTING)
  data.put("robot_id", robotId);
  data.put("rack_id", rackId);
  data.put("cycle_start_time", cycleStartTime);
  data.put("cycle_end_time", cycleEndTime);
  data.put("cycle_time", cycleEndTime - cycleStartTime);  // ✅ Used by Robot collector
  data.put("stations_visited", stationsVisited);
  data.put("is_multi_station", stationsVisited.size() > 1);

  // Calculate aggregated metrics across all stations
  int totalPicks = 0;
  int tasksProcessed = 0;
  double processingTime = 0.0;
  double travelTime = 0.0;

  // Aggregate processing metrics
  for (ProcessingMetrics pm : processingMetrics) {
    totalPicks += pm.totalPicks();
    tasksProcessed += pm.tasksProcessed();
    processingTime += pm.processingTime();
  }

  // Aggregate transport metrics (outbound only, exclude return)
  for (TransportMetrics tm : transportMetrics) {
    travelTime += tm.travelTime();
  }

  // Add return travel time
  if (returnMetrics != null) {
    travelTime += returnMetrics.travelTime();
  }

  // Emit ONLY the fields that collectors actually use
  data.put("total_picks", totalPicks);                     // ✅ Used by Storage, Station, Robot
  data.put("tasks_processed", tasksProcessed);             // ✅ Used by Storage (Python)
  data.put("travel_time", travelTime);                     // ✅ Used by TPQ, Robot (Python)
  data.put("processing_time", processingTime);             // ✅ Used by Station
  data.put("station_id", stationsVisited.size() == 1 ? stationsVisited.get(0).value() : -1);  // ✅ Used by Station

  // Emit cycle event
  metricsManager.emitEvent(
      MetricEvent.RACK_CYCLE_COMPLETED,
      now(),
      robotId,
      rackId,
      null,
      null,
      data);
}
```

**Key Changes:**
1. Renamed from `emitCycleEvent` to `emitRtpCycle` (matches reference)
2. Added parameters for all metric collections
3. **PRESERVE** all existing fields (robot_id, cycle_time, etc.)
4. **AGGREGATE** metrics across all stations (not just first station)
5. **ONLY EMIT** fields that collectors actually consume:
   - `cycle_time` - used by Robot collector
   - `total_picks` - used by Storage, Station, Robot collectors
   - `tasks_processed` - used by Storage collector (Python)
   - `travel_time` - used by TPQ, Robot collectors (Python)
   - `processing_time` - used by Station collector
   - `station_id` - used by Station collector (single-station only)
6. **REMOVED** unused fields:
   - ❌ Detailed transport breakdowns (loading time, phases)
   - ❌ Task details lists
   - ❌ Return transport breakdowns
   - ❌ Destination coordinates
   - ❌ Multi-station prefixed fields

**Verification:**
- [ ] Method renamed to match reference implementations
- [ ] All existing data fields preserved
- [ ] Metrics aggregated across all stations
- [ ] Only essential fields emitted (used by collectors)
- [ ] Multi-station cycles properly aggregated
- [ ] Travel time includes outbound + return
- [ ] Processing time summed across stations
- [ ] Null checks prevent crashes if lists empty
- [ ] Event emission still uses RACK_CYCLE_COMPLETED
- [ ] MetricsManager null check preserved

**Why:** This simplified approach:
- Emits **only what collectors actually consume**
- Reduces event payload size significantly
- Matches Python collector expectations
- Properly handles multi-station aggregation
- Eliminates unused data that was cluttering the event
- Makes the event structure cleaner and more maintainable

---

### Task 7: Add Missing Import Statements

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Location:** Top of file (import section, after existing imports)

**Add Imports:**
```java
import com.greyorange.dessimulator.simulator.metrics.RobotMetrics.TransportMetrics;
import com.greyorange.dessimulator.simulator.metrics.RobotMetrics.ReturnTransportMetrics;
import com.greyorange.dessimulator.simulator.metrics.SimulationMetrics.ProcessingMetrics;
```

**Verification:**
- [ ] TransportMetrics import added
- [ ] ReturnTransportMetrics import added
- [ ] ProcessingMetrics import added (may already exist)
- [ ] No duplicate imports
- [ ] Import ordering follows Google Java Format conventions

**Why:** These metric record types must be imported to use them as return types and parameter types.

---

### Task 8: Update Integration Test to Validate Comprehensive Metrics

**File:** `apps/backend-java/src/test/java/com/greyorange/dessimulator/infrastructure/desmoj/DesmojIntegrationTest.java`

**Location:** Within existing test methods (after simulation runs)

**Add Validation:**
```java
@Test
void testRtpSimulationWithComprehensiveMetrics() throws Exception {
    // ... existing test setup ...

    SimulationResults results = simulator.runSimulation();

    // Existing assertions (PRESERVE)
    assertThat(results).isNotNull();
    // ... existing metric checks ...

    // NEW: Validate comprehensive cycle metrics
    Map<String, Object> cycleEvents = results.events().stream()
        .filter(e -> e.eventType() == MetricEvent.RACK_CYCLE_COMPLETED)
        .findFirst()
        .orElseThrow(() -> new AssertionError("No RACK_CYCLE_COMPLETED events found"));

    // Validate transport metrics present
    assertThat(cycleEvents).containsKey("outbound_loading_time");
    assertThat(cycleEvents).containsKey("outbound_travel_time");
    assertThat(cycleEvents).containsKey("outbound_total_time");
    assertThat(cycleEvents.get("outbound_loading_time")).isEqualTo(6.0);

    // Validate processing metrics present
    assertThat(cycleEvents).containsKey("total_picks");
    assertThat(cycleEvents).containsKey("tasks_processed");
    assertThat(cycleEvents).containsKey("operator_time");
    assertThat(cycleEvents.get("tasks_processed")).isInstanceOf(Integer.class);
    assertThat((Integer) cycleEvents.get("tasks_processed")).isGreaterThan(0);

    // Validate return metrics present
    assertThat(cycleEvents).containsKey("return_travel_time");
    assertThat(cycleEvents).containsKey("return_unloading_time");
    assertThat(cycleEvents).containsKey("return_total_time");
    assertThat(cycleEvents.get("return_unloading_time")).isEqualTo(6.0);

    // Validate metric consistency
    double cycleTime = (Double) cycleEvents.get("cycle_time");
    double outboundTotal = (Double) cycleEvents.get("outbound_total_time");
    double operatorTime = (Double) cycleEvents.get("operator_time");
    double returnTotal = (Double) cycleEvents.get("return_total_time");

    // Cycle time should be at least the sum of major components
    // (may be more due to operator resource blocking)
    assertThat(cycleTime).isGreaterThanOrEqualTo(outboundTotal + returnTotal);

    log.info("Comprehensive metrics validation passed:");
    log.info("  Outbound transport: {}s (loading: {}s, travel: {}s)",
        outboundTotal,
        cycleEvents.get("outbound_loading_time"),
        cycleEvents.get("outbound_travel_time"));
    log.info("  Processing: {} tasks, {} picks, {}s operator time",
        cycleEvents.get("tasks_processed"),
        cycleEvents.get("total_picks"),
        operatorTime);
    log.info("  Return transport: {}s (travel: {}s, unloading: {}s)",
        returnTotal,
        cycleEvents.get("return_travel_time"),
        cycleEvents.get("return_unloading_time"));
    log.info("  Total cycle time: {}s", cycleTime);
}
```

**Verification:**
- [ ] Test validates transport metrics presence
- [ ] Test validates processing metrics presence
- [ ] Test validates return metrics presence
- [ ] Test validates loading time = 6.0s
- [ ] Test validates unloading time = 6.0s
- [ ] Test validates tasks_processed > 0
- [ ] Test validates cycle time consistency
- [ ] Test includes descriptive logging
- [ ] Existing tests still pass

**Why:** Integration test ensures:
- Metrics are actually emitted (not just calculated)
- Metric values are realistic and consistent
- No regressions in existing functionality
- Complete end-to-end validation

---

### Task 9: Manual Verification with Background Server Testing

**Process:**

1. **Start background server:**
```bash
cd apps/backend-java
mvn clean compile
mvn spring-boot:run > /tmp/server.log 2>&1 &
echo $!  # Save PID
```

2. **Run simulation:**
```bash
curl -X POST http://localhost:8080/simulations/run \
  -H "Content-Type: application/json" \
  -d @src/test/resources/integration-test-config.json \
  | jq . > /tmp/simulation_response.json
```

3. **Validate response metrics:**
```bash
# Check for comprehensive cycle metrics
jq '.metrics.rack_cycle_metrics[0]' /tmp/simulation_response.json

# Should include:
# - outbound_loading_time: 6.0
# - outbound_travel_time: <number>
# - outbound_total_time: <number>
# - total_picks: <number>
# - tasks_processed: <number>
# - operator_time: <number>
# - return_travel_time: <number>
# - return_unloading_time: 6.0
# - return_total_time: <number>
```

4. **Check server logs:**
```bash
# Validate metric logging
grep "transported rack" /tmp/server.log
grep "processed.*tasks" /tmp/server.log
grep "returned rack" /tmp/server.log
```

5. **Kill server:**
```bash
pkill -f "spring-boot:run"
```

**Verification Checklist:**
- [ ] Server starts without errors
- [ ] Simulation completes successfully
- [ ] Response includes comprehensive metrics
- [ ] Loading times = 6.0s consistently
- [ ] Unloading times = 6.0s consistently
- [ ] Processing metrics show non-zero values
- [ ] Cycle time = transport + processing + return (approximately)
- [ ] Logs show metric capture at each phase
- [ ] No exceptions in server logs
- [ ] No null pointer exceptions

**Why:** Manual testing validates:
- Real-world API behavior (not just unit tests)
- Metric serialization through JSON
- End-to-end timing accuracy
- Integration with full system stack

---

## Testing Strategy

### Unit Tests
**Not Required** - Methods are private helpers, tested via integration

### Integration Tests
**File:** `DesmojIntegrationTest.java`

**Test Cases:**
1. ✅ **Existing tests pass** (no regressions)
2. 🆕 **Cycle metrics comprehensive** (new validation in Task 8)
3. 🆕 **Transport metrics present and valid**
4. 🆕 **Processing metrics present and valid**
5. 🆕 **Return metrics present and valid**
6. 🆕 **Multi-station aggregations correct**

### Manual Testing
**Process:** Background server + curl (Task 9)

**Validation:**
- Response JSON structure
- Metric value ranges
- Timing consistency
- Server log inspection

### Regression Prevention
- All existing tests must pass unchanged
- All existing events must still emit
- All existing callback behavior preserved
- Backward compatibility maintained

## Success Criteria Checklist

### Functional Requirements
- [ ] All transport phases emit detailed timing metrics
- [ ] Processing metrics are captured and included in cycle events
- [ ] Return transport metrics are collected and tracked
- [ ] Cycle completion events contain full metric breakdowns
- [ ] Multi-station cycles aggregate metrics per station

### Code Quality
- [ ] No new checkstyle violations
- [ ] Google Java Format compliance maintained
- [ ] All imports properly organized
- [ ] Logging follows existing patterns
- [ ] Exception handling preserved

### Testing
- [ ] All existing integration tests pass
- [ ] New metric validations added to integration test
- [ ] Manual testing via background server successful
- [ ] No null pointer exceptions
- [ ] No timing inconsistencies

### Documentation
- [ ] Plan document complete (this file)
- [ ] Code comments updated where needed
- [ ] Metric fields self-documenting via naming

### Parity with Reference Implementations
- [ ] Python SimPy: TransportMetrics structure matches
- [ ] Python SimPy: ProcessingMetrics structure matches
- [ ] Python SimPy: ReturnTransportMetrics structure matches
- [ ] Java Custom DES: Method signatures match
- [ ] Java Custom DES: Cycle event data matches
- [ ] Java Custom DES: Metric collection flow matches

## Implementation Order

Execute tasks in this order to minimize errors:

1. **Task 7**: Add imports (no functional changes, prevents compile errors)
2. **Task 1**: Add metric collection lists (fields only, not used yet)
3. **Task 2**: Refactor transportRackToStation() (isolated method)
4. **Task 4**: Refactor returnRackToStorage() (isolated method)
5. **Task 3**: Refactor processTasksAtStation() (depends on understanding flow)
6. **Task 6**: Refactor emitRtpCycle() (new signature, not called yet)
7. **Task 5**: Capture metrics in main loop (wires everything together)
8. **Task 8**: Update integration test (validates implementation)
9. **Task 9**: Manual verification (final validation)

**Rationale:**
- Start with non-breaking changes (imports, fields)
- Refactor methods in isolation before wiring together
- Wire together only after all pieces ready
- Test after implementation complete

## Risk Mitigation

### Risk: Breaking Existing Functionality
**Mitigation:**
- Preserve all existing event emissions
- Preserve all callback invocations
- Preserve all state transitions
- Run existing tests after each task

### Risk: Timing Inconsistencies
**Mitigation:**
- Calculate metrics before and after hold() calls
- Validate cycle time = sum of components
- Add logging for debugging
- Manual verification with background server

### Risk: Null Pointer Exceptions
**Mitigation:**
- Null checks before emitting metrics
- Empty list checks before accessing elements
- Optional.ofNullable() where appropriate
- Integration test validates happy path

### Risk: Memory Leaks from Growing Lists
**Mitigation:**
- Clear metric lists at start of each cycle
- Lists are local to process instance (garbage collected)
- No static collections

### Risk: Performance Degradation
**Mitigation:**
- Metrics are lightweight records
- No complex calculations in hot path
- Same metric emission pattern as existing code
- Integration test measures timing

## Rollback Plan

If implementation fails:

1. **Git revert**: All changes in single commit for easy revert
2. **Feature flag**: Could wrap metric collection in config flag
3. **Minimal fix**: Remove return types, keep void methods
4. **Gradual rollout**: Test with small scenario first

## Post-Implementation Validation

### Metrics Analysis Dashboard
After implementation, analyze collected metrics:
- Cycle time distribution histogram
- Transport vs. processing time ratio
- Loading/unloading time verification (should be 6.0s)
- Multi-station routing efficiency
- Operator utilization percentage

### Performance Comparison
Compare Desmo-J metrics with:
- Python SimPy implementation (same scenario)
- Java custom DES implementation (same scenario)
- Validate timing consistency across implementations

### Documentation Updates
Update CLAUDE.md to reflect:
- Comprehensive metrics now available
- Cycle event structure
- Metric analysis capabilities

## Conclusion

This plan comprehensively addresses all missing metrics in the Desmo-J RTP robot process by:

1. **Refactoring** three core methods to return metric objects instead of void
2. **Capturing** returned metrics in the main robot loop
3. **Aggregating** metrics across multi-station cycles
4. **Emitting** comprehensive cycle events with full breakdowns
5. **Testing** via integration tests and manual verification
6. **Preserving** all existing functionality and behavior

Upon completion, the Desmo-J implementation will have **full metrics parity** with both the Python SimPy implementation and the Java custom DES implementation, enabling comprehensive performance analysis and optimization opportunities.

**Estimated Effort:** 4-6 hours
**Complexity:** Medium (straightforward refactoring, extensive testing)
**Risk Level:** Low (well-defined changes, preserves existing behavior)
