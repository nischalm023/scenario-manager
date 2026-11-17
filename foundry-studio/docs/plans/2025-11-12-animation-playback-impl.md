# Animation Playback Implementation Plan

**Status**: ✅ COMPLETED
**Completed**: 2025-11-12

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add post-simulation animation playback to visualize warehouse state snapshots on frontend with timeline controls.

**Architecture:** Capture warehouse snapshots every 10s during Java simulation via Desmo-J process, store in list, return with SimulationResult. Frontend renders snapshots on existing WarehouseMap with play/pause/scrub controls.

**Tech Stack:** Java 21 (Spring Boot, Desmo-J), TypeScript/React, shadcn/ui, Canvas/SVG

---

## Task 1: Create Snapshot Data Model Records

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/WarehouseSnapshot.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/RobotState.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/RelayState.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/StationState.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/ToteState.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/StorageSpaceState.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/SnapshotMetrics.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/Position.java`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/StorageSpaceType.java`

**Step 1: Create Position record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/Position.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

public record Position(
    double x,
    double y
) {}
```

**Step 2: Create StorageSpaceType enum**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/StorageSpaceType.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

public enum StorageSpaceType {
    RACK,
    TOTE
}
```

**Step 3: Create RobotState record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/RobotState.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

import com.greyorange.dessimulator.domain.robot.RobotStatus;
import com.greyorange.dessimulator.domain.robot.RobotType;

public record RobotState(
    String robotId,
    RobotType type,
    RobotStatus status,
    Position position,
    String currentTask,
    String carryingStorageSpace
) {}
```

**Step 4: Create RelayState record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/RelayState.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

import java.util.List;

public record RelayState(
    String relayId,
    int currentCapacity,
    int maxCapacity,
    List<String> toteIds
) {}
```

**Step 5: Create StationState record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/StationState.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

public record StationState(
    int stationId,
    int queueLength,
    String currentTote,
    double utilizationPercent
) {}
```

**Step 6: Create ToteState record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/ToteState.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

public record ToteState(
    String toteId,
    String location,
    int taskCount
) {}
```

**Step 7: Create StorageSpaceState record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/StorageSpaceState.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

import java.util.List;
import java.util.Map;

public record StorageSpaceState(
    String storageSpaceId,
    StorageSpaceType type,
    Position position,
    boolean isActive,
    Map<String, Integer> inventory,
    int totalInventory,
    Map<String, Integer> reservedInventory,
    int totalReservedInventory,
    List<String> reservedByOrders
) {}
```

**Step 8: Create SnapshotMetrics record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/SnapshotMetrics.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

public record SnapshotMetrics(
    int completedPicks,
    int completedOrders,
    double currentPicksPerHour
) {}
```

**Step 9: Create WarehouseSnapshot record**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/WarehouseSnapshot.java`

```java
package com.greyorange.dessimulator.dto.snapshot;

import java.util.List;

public record WarehouseSnapshot(
    double timestamp,
    List<RobotState> robots,
    List<RelayState> relays,
    List<StationState> stations,
    List<ToteState> totes,
    List<StorageSpaceState> storageSpaces,
    SnapshotMetrics metrics
) {}
```

**Step 10: Compile to verify syntax**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 11: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/
git commit -m "feat: add snapshot data model records

Add immutable records for warehouse state snapshots:
- WarehouseSnapshot: top-level container
- RobotState, RelayState, StationState, ToteState
- StorageSpaceState with inventory tracking
- SnapshotMetrics for aggregate metrics"
```

---

## Task 2: Add Snapshot Configuration to SimulationConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/SimulationConfig.java`

**Step 1: Add snapshot configuration fields**

Add to `SimulationConfig` record:

```java
// Add these fields to the existing SimulationConfig record
Optional<Boolean> enableSnapshots,
Optional<Double> snapshotIntervalSeconds
```

The complete record should include these new fields alongside existing ones.

**Step 2: Update integration test config**

File: `apps/backend/src/test/resources/integration-test-config.json`

Add to the JSON:

```json
"enableSnapshots": true,
"snapshotIntervalSeconds": 10.0
```

**Step 3: Compile to verify**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/SimulationConfig.java apps/backend/src/test/resources/integration-test-config.json
git commit -m "feat: add snapshot config to SimulationConfig

Add enableSnapshots and snapshotIntervalSeconds fields"
```

---

## Task 3: Update SimulationResult to Include Snapshots

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/SimulationResult.java`

**Step 1: Add snapshots field**

Add import and field to `SimulationResult` record:

```java
import com.greyorange.dessimulator.dto.snapshot.WarehouseSnapshot;
import java.util.List;

// Add this field to the record
List<WarehouseSnapshot> snapshots
```

**Step 2: Compile to verify**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/SimulationResult.java
git commit -m "feat: add snapshots field to SimulationResult"
```

---

## Task 4: Add Snapshot Tracking to DesmojWarehouseModel

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/model/DesmojWarehouseModel.java`

**Step 1: Add snapshot list field and getter**

Add to `DesmojWarehouseModel` class:

```java
import com.greyorange.dessimulator.dto.snapshot.WarehouseSnapshot;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

// Add as class fields
private List<WarehouseSnapshot> snapshotList;

// Add getter method
public List<WarehouseSnapshot> getSnapshotList() {
    return snapshotList != null ? snapshotList : List.of();
}

// Add setter method
public void setSnapshotList(List<WarehouseSnapshot> snapshotList) {
    this.snapshotList = snapshotList;
}
```

**Step 2: Compile to verify**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/model/DesmojWarehouseModel.java
git commit -m "feat: add snapshot list tracking to DesmojWarehouseModel"
```

---

## Task 5: Create Snapshot Capture Process

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`

**Step 1: Create snapshot capture process skeleton**

File: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`

```java
package com.greyorange.dessimulator.simulator.snapshot;

import co.paralleluniverse.fibers.SuspendExecution;
import com.greyorange.dessimulator.dto.snapshot.*;
import com.greyorange.dessimulator.simulator.model.DesmojWarehouseModel;
import desmoj.core.simulator.SimProcess;
import desmoj.core.simulator.TimeInstant;
import desmoj.core.simulator.TimeSpan;

import java.util.List;
import java.util.concurrent.TimeUnit;

import lombok.extern.slf4j.Slf4j;

@Slf4j
public class DesmojSnapshotCaptureProcess extends SimProcess {
    private final DesmojWarehouseModel model;
    private final double intervalSeconds;
    private final List<WarehouseSnapshot> snapshots;

    public DesmojSnapshotCaptureProcess(
            DesmojWarehouseModel model,
            double intervalSeconds,
            List<WarehouseSnapshot> snapshots
    ) {
        super(model, "SnapshotCapture", true);
        this.model = model;
        this.intervalSeconds = intervalSeconds;
        this.snapshots = snapshots;
    }

    @Override
    public void lifeCycle() throws SuspendExecution {
        while (true) {
            try {
                hold(new TimeSpan(intervalSeconds, TimeUnit.SECONDS));
                var snapshot = captureSnapshot();
                snapshots.add(snapshot);
                log.debug("Captured snapshot at t={}", snapshot.timestamp());
            } catch (Exception e) {
                log.debug("Snapshot capture ended: {}", e.getMessage());
                break;
            }
        }
    }

    private WarehouseSnapshot captureSnapshot() {
        double timestamp = model.currentTime().getTimeAsDouble(TimeUnit.SECONDS);

        return new WarehouseSnapshot(
                timestamp,
                captureRobotStates(),
                captureRelayStates(),
                captureStationStates(),
                captureToteStates(),
                captureStorageSpaceStates(),
                captureMetrics()
        );
    }

    private List<RobotState> captureRobotStates() {
        // TODO: Implement in next step
        return List.of();
    }

    private List<RelayState> captureRelayStates() {
        // TODO: Implement in next step
        return List.of();
    }

    private List<StationState> captureStationStates() {
        // TODO: Implement in next step
        return List.of();
    }

    private List<ToteState> captureToteStates() {
        // TODO: Implement in next step
        return List.of();
    }

    private List<StorageSpaceState> captureStorageSpaceStates() {
        // TODO: Implement in next step
        return List.of();
    }

    private SnapshotMetrics captureMetrics() {
        // TODO: Implement in next step
        return new SnapshotMetrics(0, 0, 0.0);
    }
}
```

**Step 2: Compile to verify**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: add snapshot capture process skeleton

Create Desmo-J process that runs every N seconds
Capture methods stubbed for next implementation"
```

---

## Task 6: Implement Robot State Capture

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`

**Step 1: Check robot data structure in model**

Read: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/model/DesmojWarehouseModel.java`

Look for robot tracking methods (likely `getRobots()` or similar).

**Step 2: Implement captureRobotStates**

Replace the `captureRobotStates()` method:

```java
private List<RobotState> captureRobotStates() {
    // For RTP robots
    if (model.getRtpRobots() != null) {
        return model.getRtpRobots().stream()
            .map(robot -> new RobotState(
                robot.getId(),
                robot.getType(),
                robot.getStatus(),
                new Position(robot.getX(), robot.getY()),
                robot.getCurrentTaskId(),
                robot.getCarryingRackId()
            ))
            .toList();
    }

    // For relay system robots (VTM + HTM)
    var allRobots = new java.util.ArrayList<RobotState>();

    if (model.getVtmRobots() != null) {
        model.getVtmRobots().forEach(robot ->
            allRobots.add(new RobotState(
                robot.getId(),
                RobotType.VTM,
                robot.getStatus(),
                new Position(robot.getX(), robot.getY()),
                robot.getCurrentTaskId(),
                robot.getCarryingToteId()
            ))
        );
    }

    if (model.getHtmRobots() != null) {
        model.getHtmRobots().forEach(robot ->
            allRobots.add(new RobotState(
                robot.getId(),
                RobotType.HTM,
                robot.getStatus(),
                new Position(robot.getX(), robot.getY()),
                robot.getCurrentTaskId(),
                robot.getCarryingToteId()
            ))
        );
    }

    return allRobots;
}
```

**Note:** Adjust method calls based on actual robot API (getX/getY vs getPosition, getCurrentTaskId vs getTask, etc.). Check robot class definitions first.

**Step 3: Compile to verify**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS (or compilation errors revealing actual robot API)

**Step 4: Fix compilation errors based on actual robot API**

If compilation fails, read the robot classes to find correct method names and adjust.

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: implement robot state capture for snapshots"
```

---

## Task 7: Implement Relay, Station, Tote State Capture

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`

**Step 1: Implement captureRelayStates**

Replace the method:

```java
private List<RelayState> captureRelayStates() {
    if (model.getRelayPositionManager() == null) {
        return List.of();
    }

    return model.getRelayPositionManager().getRelayPositions().stream()
        .map(relay -> new RelayState(
            relay.getId(),
            relay.getCurrentOccupancy(),
            relay.getCapacity(),
            relay.getToteIds()
        ))
        .toList();
}
```

**Step 2: Implement captureStationStates**

Replace the method:

```java
private List<StationState> captureStationStates() {
    if (model.getStationManager() == null) {
        return List.of();
    }

    return model.getStationManager().getStations().stream()
        .map(station -> new StationState(
            station.getStationId(),
            station.getQueueLength(),
            station.getCurrentToteId(),
            station.getUtilizationPercent()
        ))
        .toList();
}
```

**Step 3: Implement captureToteStates**

Replace the method:

```java
private List<ToteState> captureToteStates() {
    if (model.getTotePool() == null) {
        return List.of();
    }

    return model.getTotePool().getAllTotes().stream()
        .map(tote -> new ToteState(
            tote.getId(),
            tote.getCurrentLocation(),
            tote.getRemainingTaskCount()
        ))
        .toList();
}
```

**Note:** Adjust method names based on actual API from RelayPosition, StationManager, TotePool classes.

**Step 4: Compile and fix errors**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS or errors revealing actual API

Fix compilation errors by checking actual class methods.

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: implement relay, station, tote state capture"
```

---

## Task 8: Implement Storage Space State Capture

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`

**Step 1: Implement captureStorageSpaceStates**

Replace the method:

```java
private List<StorageSpaceState> captureStorageSpaceStates() {
    if (model.getWarehouseInventory() == null) {
        return List.of();
    }

    var inventory = model.getWarehouseInventory();
    var activeSpaces = model.getActiveStorageSpaces();

    return inventory.getAllStorageSpaces().stream()
        .map(space -> {
            var inventoryMap = inventory.getInventory(space.getId());
            var reservedMap = inventory.getReservedInventory(space.getId());
            var totalInventory = inventoryMap.values().stream()
                .mapToInt(Integer::intValue)
                .sum();
            var totalReserved = reservedMap.values().stream()
                .mapToInt(Integer::intValue)
                .sum();

            return new StorageSpaceState(
                space.getId(),
                space.getType(),
                new Position(space.getX(), space.getY()),
                activeSpaces.contains(space.getId()),
                inventoryMap,
                totalInventory,
                reservedMap,
                totalReserved,
                inventory.getReservedByOrders(space.getId())
            );
        })
        .toList();
}
```

**Note:** Adjust based on actual WarehouseInventory API. May need to query storage positions from StoragePositionManager instead.

**Step 2: Compile and fix errors**

Run: `mvn clean compile -DskipTests`

Expected: Compilation errors revealing actual API

Fix by checking WarehouseInventory and StoragePositionManager classes.

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: implement storage space state capture"
```

---

## Task 9: Implement Metrics Capture

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`

**Step 1: Implement captureMetrics**

Replace the method:

```java
private SnapshotMetrics captureMetrics() {
    var taskCollector = model.getTaskMetricsCollector();
    if (taskCollector == null) {
        return new SnapshotMetrics(0, 0, 0.0);
    }

    var currentTime = model.currentTime().getTimeAsDouble(TimeUnit.SECONDS);
    var metrics = taskCollector.getCurrentMetrics();

    // Calculate current picks per hour based on completion rate
    var picksPerHour = currentTime > 0
        ? (metrics.totalPicksCompleted() / currentTime) * 3600.0
        : 0.0;

    return new SnapshotMetrics(
        metrics.totalPicksCompleted(),
        metrics.totalOrdersCompleted(),
        picksPerHour
    );
}
```

**Note:** Adjust based on actual metrics collector API.

**Step 2: Compile and fix errors**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS or errors revealing API

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: implement metrics capture for snapshots"
```

---

## Task 10: Integrate Snapshot Process into Simulation Engine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojSimulationEngine.java`

**Step 1: Schedule snapshot process in doInitialSchedules**

In `DesmojWarehouseModel.doInitialSchedules()` method, add:

```java
// Add snapshot capture if enabled
if (config.enableSnapshots().orElse(false)) {
    var snapshotInterval = config.snapshotIntervalSeconds().orElse(10.0);
    var snapshots = new java.util.concurrent.CopyOnWriteArrayList<WarehouseSnapshot>();
    this.setSnapshotList(snapshots);

    var captureProcess = new DesmojSnapshotCaptureProcess(
        this,
        snapshotInterval,
        snapshots
    );
    captureProcess.activate(new TimeInstant(0, TimeUnit.SECONDS));

    log.info("Snapshot capture enabled: interval={}s", snapshotInterval);
}
```

**Step 2: Return snapshots in SimulationResult**

In `DesmojSimulationEngine.run()`, update the return statement:

```java
var snapshots = config.enableSnapshots().orElse(false)
    ? model.getSnapshotList()
    : List.<WarehouseSnapshot>of();

return new SimulationResult(
    config,
    actualDuration,
    taskMetrics,
    robotMetrics,
    stationMetrics,
    storageSpaceMetrics,
    snapshots,  // Add this
    metadata
);
```

**Step 3: Compile to verify**

Run: `mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/model/DesmojWarehouseModel.java apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojSimulationEngine.java
git commit -m "feat: integrate snapshot capture into simulation engine

Schedule snapshot process if enabled in config
Return snapshots with simulation results"
```

---

## Task 11: Test Backend Snapshot Capture

**Files:**
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/DesmojIntegrationTest.java`

**Step 1: Write test for snapshot capture**

Add test method to `DesmojIntegrationTest`:

```java
@Test
void shouldCaptureSnapshotsWhenEnabled() throws IOException {
    // Load config with snapshots enabled
    var config = loadTestConfig("integration-test-config.json");

    // Run simulation
    var result = simulationEngine.run(config);

    // Verify snapshots captured
    assertThat(result.snapshots()).isNotEmpty();
    assertThat(result.snapshots().get(0).timestamp()).isEqualTo(0.0);

    // Verify snapshot interval
    if (result.snapshots().size() > 1) {
        var interval = result.snapshots().get(1).timestamp() - result.snapshots().get(0).timestamp();
        assertThat(interval).isCloseTo(10.0, within(0.1));
    }

    // Verify snapshot contains data
    var firstSnapshot = result.snapshots().get(0);
    assertThat(firstSnapshot.robots()).isNotEmpty();

    log.info("Captured {} snapshots over {}s simulation",
        result.snapshots().size(),
        result.simulationDuration());
}
```

**Step 2: Run test to verify it fails (if snapshots not working)**

Run: `mvn test -Dtest=DesmojIntegrationTest#shouldCaptureSnapshotsWhenEnabled`

Expected: FAIL or PASS depending on implementation correctness

**Step 3: Fix any issues revealed by test**

Debug and fix issues in snapshot capture logic.

**Step 4: Run test to verify it passes**

Run: `mvn test -Dtest=DesmojIntegrationTest#shouldCaptureSnapshotsWhenEnabled`

Expected: PASS

**Step 5: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/DesmojIntegrationTest.java
git commit -m "test: add integration test for snapshot capture"
```

---

## Task 12: Manual Backend Test

**Step 1: Start server in background**

```bash
# Clean up any existing temp files
rm -f /tmp/server.log /tmp/server.pid

# Kill existing server
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2

# Start server
nx serve backend > /tmp/server.log 2>&1 &
sleep 10

# Verify server started
curl -s http://localhost:8080/health
```

Expected: `{"status":"UP"}` or similar

**Step 2: Run simulation with snapshots enabled**

```bash
cat apps/backend/src/test/resources/integration-test-config.json | \
  curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @- > /tmp/sim_result.json

# Check result
cat /tmp/sim_result.json | grep -o '"snapshots":\[' | head -1
```

Expected: `"snapshots":[` found in output

**Step 3: Verify snapshot data**

```bash
# Count snapshots
cat /tmp/sim_result.json | grep -o '"timestamp":' | wc -l

# Check first snapshot
cat /tmp/sim_result.json | grep -A 20 '"snapshots":\['
```

Expected: Multiple snapshots with timestamp, robots, relays, stations, etc.

**Step 4: Cleanup**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
rm -f /tmp/server.log /tmp/server.pid /tmp/sim_result.json
```

---

## Task 13: Regenerate Frontend Types from OpenAPI

**Step 1: Ensure backend server is running**

```bash
# Check if server running
curl -s http://localhost:8080/health || (nx serve backend > /tmp/server.log 2>&1 & sleep 10)
```

**Step 2: Regenerate TypeScript types**

```bash
npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts
```

Expected: File updated with new snapshot types

**Step 3: Verify types generated**

```bash
grep -A 5 "WarehouseSnapshot" apps/frontend/src/types/generated-api.ts
```

Expected: TypeScript interface for WarehouseSnapshot

**Step 4: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "chore: regenerate frontend types with snapshot DTOs"
```

---

## Task 14: Create Animation Controls Component

**Files:**
- Create: `apps/frontend/src/components/AnimationControls.tsx`

**Step 1: Create AnimationControls component**

File: `apps/frontend/src/components/AnimationControls.tsx`

```typescript
import { useState, useEffect } from 'react';
import { Button } from '@/components/ui/button';
import { Slider } from '@/components/ui/slider';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Play, Pause, SkipBack } from 'lucide-react';
import type { components } from '@/types/generated-api';

type WarehouseSnapshot = components['schemas']['WarehouseSnapshot'];

interface AnimationControlsProps {
  snapshots: WarehouseSnapshot[];
  currentFrame: number;
  onFrameChange: (frame: number) => void;
}

export function AnimationControls({ snapshots, currentFrame, onFrameChange }: AnimationControlsProps) {
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackSpeed, setPlaybackSpeed] = useState(1.0);

  const totalFrames = snapshots.length;
  const currentTimestamp = snapshots[currentFrame]?.timestamp ?? 0;

  // Playback loop
  useEffect(() => {
    if (!isPlaying || currentFrame >= totalFrames - 1) {
      if (currentFrame >= totalFrames - 1) {
        setIsPlaying(false);
      }
      return;
    }

    const intervalMs = 1000 / playbackSpeed;
    const timer = setInterval(() => {
      onFrameChange(currentFrame + 1);
    }, intervalMs);

    return () => clearInterval(timer);
  }, [isPlaying, currentFrame, playbackSpeed, totalFrames, onFrameChange]);

  const handlePlayPause = () => {
    if (currentFrame >= totalFrames - 1) {
      onFrameChange(0);
      setIsPlaying(true);
    } else {
      setIsPlaying(!isPlaying);
    }
  };

  const handleReset = () => {
    setIsPlaying(false);
    onFrameChange(0);
  };

  const handleSliderChange = (values: number[]) => {
    setIsPlaying(false);
    onFrameChange(values[0]);
  };

  if (totalFrames === 0) {
    return (
      <div className="flex items-center justify-center p-4 bg-muted rounded-lg">
        <p className="text-sm text-muted-foreground">No snapshots available</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 bg-muted rounded-lg">
      {/* Timeline Slider */}
      <div className="flex items-center gap-4">
        <span className="text-sm font-mono w-32">
          {currentFrame} / {totalFrames - 1}
        </span>
        <Slider
          value={[currentFrame]}
          onValueChange={handleSliderChange}
          max={totalFrames - 1}
          step={1}
          className="flex-1"
        />
        <span className="text-sm font-mono w-24 text-right">
          {currentTimestamp.toFixed(1)}s
        </span>
      </div>

      {/* Controls */}
      <div className="flex items-center justify-center gap-2">
        <Button
          variant="outline"
          size="icon"
          onClick={handleReset}
          disabled={currentFrame === 0}
        >
          <SkipBack className="h-4 w-4" />
        </Button>

        <Button
          variant="default"
          size="icon"
          onClick={handlePlayPause}
        >
          {isPlaying ? (
            <Pause className="h-4 w-4" />
          ) : (
            <Play className="h-4 w-4" />
          )}
        </Button>

        <Select
          value={playbackSpeed.toString()}
          onValueChange={(value) => setPlaybackSpeed(parseFloat(value))}
        >
          <SelectTrigger className="w-24">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="0.5">0.5x</SelectItem>
            <SelectItem value="1">1x</SelectItem>
            <SelectItem value="2">2x</SelectItem>
            <SelectItem value="5">5x</SelectItem>
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}
```

**Step 2: Verify component compiles**

Run: `npm run check`

Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/components/AnimationControls.tsx
git commit -m "feat: add animation controls component

Add play/pause, scrubbing, speed control
Support timeline navigation with slider"
```

---

## Task 15: Update WarehouseMap to Accept Snapshot Prop

**Files:**
- Modify: `apps/frontend/src/components/WarehouseMap.tsx` (or wherever WarehouseMap is defined)

**Step 1: Locate WarehouseMap component**

```bash
find apps/frontend/src -name "*WarehouseMap*" -o -name "*warehouse*map*"
```

**Step 2: Add snapshot prop to WarehouseMap**

Add to component props:

```typescript
import type { components } from '@/types/generated-api';

type WarehouseSnapshot = components['schemas']['WarehouseSnapshot'];

interface WarehouseMapProps {
  config: components['schemas']['SimulationConfig'];
  snapshot?: WarehouseSnapshot;  // NEW: optional snapshot for animation
}

export function WarehouseMap({ config, snapshot }: WarehouseMapProps) {
  // Existing implementation...

  // If snapshot provided, use it for rendering
  // Otherwise use static config-based rendering
}
```

**Step 3: Verify compiles**

Run: `npm run check`

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/components/WarehouseMap.tsx
git commit -m "feat: add snapshot prop to WarehouseMap

Support optional snapshot for animation rendering"
```

---

## Task 16: Implement Robot Rendering on WarehouseMap

**Files:**
- Modify: `apps/frontend/src/components/WarehouseMap.tsx`

**Step 1: Add robot rendering logic**

Add to WarehouseMap component:

```typescript
import { useMemo } from 'react';

// Inside component
const robotElements = useMemo(() => {
  if (!snapshot?.robots) return null;

  return snapshot.robots.map((robot) => (
    <circle
      key={robot.robotId}
      cx={robot.position.x}
      cy={robot.position.y}
      r={5}
      fill={getRobotColor(robot.type)}
      stroke={robot.status === 'IDLE' ? '#666' : 'none'}
      strokeWidth={2}
      opacity={robot.status === 'MOVING' ? 1 : 0.6}
    >
      <title>
        {robot.robotId} - {robot.type} - {robot.status}
        {robot.carryingStorageSpace && ` carrying ${robot.carryingStorageSpace}`}
      </title>
    </circle>
  ));
}, [snapshot]);

function getRobotColor(type: string): string {
  switch (type) {
    case 'VTM': return '#3b82f6'; // blue
    case 'HTM': return '#22c55e'; // green
    case 'RTP': return '#f97316'; // orange
    default: return '#6b7280'; // gray
  }
}

// In SVG rendering
return (
  <svg>
    {/* Existing map elements */}
    {robotElements}
  </svg>
);
```

**Step 2: Verify compiles**

Run: `npm run check`

Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/components/WarehouseMap.tsx
git commit -m "feat: implement robot rendering on WarehouseMap

Render robots as colored circles with tooltips
Color by type: VTM=blue, HTM=green, RTP=orange"
```

---

## Task 17: Implement Relay and Station Rendering

**Files:**
- Modify: `apps/frontend/src/components/WarehouseMap.tsx`

**Step 1: Add relay rendering**

```typescript
const relayElements = useMemo(() => {
  if (!snapshot?.relays) return null;

  return snapshot.relays.map((relay) => {
    const fillPercent = relay.maxCapacity > 0
      ? (relay.currentCapacity / relay.maxCapacity) * 100
      : 0;

    return (
      <g key={relay.relayId}>
        <rect
          x={/* relay position x */}
          y={/* relay position y */}
          width={20}
          height={20}
          fill={fillPercent >= 100 ? '#ef4444' : '#22c55e'}
          opacity={0.7}
        />
        <text x={/* x + 10 */} y={/* y + 15 */} fontSize="10" textAnchor="middle">
          {relay.currentCapacity}/{relay.maxCapacity}
        </text>
        <title>
          {relay.relayId}: {relay.currentCapacity}/{relay.maxCapacity} capacity
          {relay.toteIds.length > 0 && `\nTotes: ${relay.toteIds.join(', ')}`}
        </title>
      </g>
    );
  });
}, [snapshot]);
```

**Step 2: Add station rendering**

```typescript
const stationElements = useMemo(() => {
  if (!snapshot?.stations) return null;

  return snapshot.stations.map((station) => {
    const utilColor = station.utilizationPercent > 80 ? '#ef4444' :
                     station.utilizationPercent > 50 ? '#eab308' : '#22c55e';

    return (
      <g key={station.stationId}>
        <rect
          x={/* station position x */}
          y={/* station position y */}
          width={30}
          height={30}
          fill={utilColor}
          opacity={0.5}
        />
        {station.queueLength > 0 && (
          <text x={/* x + 15 */} y={/* y + 20 */} fontSize="12" textAnchor="middle" fontWeight="bold">
            {station.queueLength}
          </text>
        )}
        <title>
          Station {station.stationId}
          Queue: {station.queueLength}
          Utilization: {station.utilizationPercent.toFixed(1)}%
          {station.currentTote && `\nProcessing: ${station.currentTote}`}
        </title>
      </g>
    );
  });
}, [snapshot]);
```

**Note:** Adjust positioning based on actual warehouse layout/config.

**Step 3: Add to SVG rendering**

```typescript
return (
  <svg>
    {relayElements}
    {stationElements}
    {robotElements}
  </svg>
);
```

**Step 4: Verify compiles**

Run: `npm run check`

Expected: No errors

**Step 5: Commit**

```bash
git add apps/frontend/src/components/WarehouseMap.tsx
git commit -m "feat: add relay and station rendering

Relays: show capacity and highlight when full
Stations: color by utilization, show queue length"
```

---

## Task 18: Implement Storage Space Rendering

**Files:**
- Modify: `apps/frontend/src/components/WarehouseMap.tsx`

**Step 1: Add storage space rendering**

```typescript
const storageSpaceElements = useMemo(() => {
  if (!snapshot?.storageSpaces) return null;

  return snapshot.storageSpaces.map((space) => {
    const isEmpty = space.totalInventory === 0;
    const isActive = space.isActive;
    const hasReservations = space.totalReservedInventory > 0;

    const fillColor = isEmpty ? '#9ca3af' :
                     isActive ? '#f59e0b' :
                     '#3b82f6';

    const strokeColor = hasReservations ? '#3b82f6' : 'none';

    return (
      <g key={space.storageSpaceId}>
        <rect
          x={space.position.x}
          y={space.position.y}
          width={10}
          height={10}
          fill={fillColor}
          stroke={strokeColor}
          strokeWidth={isActive ? 2 : 1}
          opacity={isEmpty ? 0.3 : 0.8}
        />
        {space.totalInventory > 0 && (
          <text
            x={space.position.x + 5}
            y={space.position.y + 8}
            fontSize="6"
            textAnchor="middle"
          >
            {space.totalInventory}
          </text>
        )}
        <title>
          {space.storageSpaceId} ({space.type})
          Total: {space.totalInventory} items
          Reserved: {space.totalReservedInventory} items
          Active: {isActive ? 'Yes' : 'No'}
          {space.reservedByOrders.length > 0 &&
            `\nOrders: ${space.reservedByOrders.join(', ')}`}
        </title>
      </g>
    );
  });
}, [snapshot]);
```

**Step 2: Add to SVG rendering**

```typescript
return (
  <svg>
    {storageSpaceElements}
    {relayElements}
    {stationElements}
    {robotElements}
  </svg>
);
```

**Step 3: Verify compiles**

Run: `npm run check`

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/components/WarehouseMap.tsx
git commit -m "feat: add storage space rendering

Color by state: active=orange, reserved=blue, empty=gray
Show inventory count on each space"
```

---

## Task 19: Integrate Animation Controls with Simulation Results Page

**Files:**
- Modify: `apps/frontend/src/pages/SimulationResults.tsx` (or wherever results are displayed)

**Step 1: Locate simulation results component**

```bash
find apps/frontend/src -name "*Results*" -o -name "*results*" | grep -i simulation
```

**Step 2: Add animation state and controls**

```typescript
import { useState } from 'react';
import { AnimationControls } from '@/components/AnimationControls';
import type { components } from '@/types/generated-api';

type SimulationResult = components['schemas']['SimulationResult'];

export function SimulationResults({ result }: { result: SimulationResult }) {
  const [currentFrame, setCurrentFrame] = useState(0);

  const currentSnapshot = result.snapshots?.[currentFrame];

  return (
    <div>
      {/* Existing results display */}

      {/* Animation section */}
      {result.snapshots && result.snapshots.length > 0 && (
        <div className="mt-8">
          <h2 className="text-2xl font-bold mb-4">Animation Playback</h2>

          <AnimationControls
            snapshots={result.snapshots}
            currentFrame={currentFrame}
            onFrameChange={setCurrentFrame}
          />

          <div className="mt-4">
            <WarehouseMap
              config={result.config}
              snapshot={currentSnapshot}
            />
          </div>
        </div>
      )}
    </div>
  );
}
```

**Step 3: Verify compiles**

Run: `npm run check`

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/pages/SimulationResults.tsx
git commit -m "feat: integrate animation controls with results page

Add animation section with controls and map
Conditionally render if snapshots available"
```

---

## Task 20: Manual Frontend Test

**Step 1: Start frontend dev server**

```bash
npm run dev
```

Expected: Server starts on port 4200

**Step 2: Run simulation with snapshots**

Use frontend UI or curl to run simulation with `enableSnapshots: true`.

**Step 3: Navigate to results page**

View simulation results and verify:
- Animation controls appear
- Timeline slider works
- Play/pause button works
- Speed selector works
- Map updates as frame changes
- Robots, relays, stations, storage spaces render correctly

**Step 4: Test scrubbing**

Drag timeline slider and verify map updates immediately.

**Step 5: Test playback speeds**

Try 0.5x, 1x, 2x, 5x speeds and verify animation speed changes.

---

## Task 21: Add Component Tests for AnimationControls

**Files:**
- Create: `apps/frontend/src/components/__tests__/AnimationControls.test.tsx`

**Step 1: Write test file**

File: `apps/frontend/src/components/__tests__/AnimationControls.test.tsx`

```typescript
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { AnimationControls } from '../AnimationControls';

const mockSnapshots = [
  { timestamp: 0, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: { completedPicks: 0, completedOrders: 0, currentPicksPerHour: 0 } },
  { timestamp: 10, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: { completedPicks: 5, completedOrders: 1, currentPicksPerHour: 1800 } },
  { timestamp: 20, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: { completedPicks: 12, completedOrders: 3, currentPicksPerHour: 2160 } },
];

describe('AnimationControls', () => {
  it('should render play button initially', () => {
    const onFrameChange = vi.fn();
    render(<AnimationControls snapshots={mockSnapshots} currentFrame={0} onFrameChange={onFrameChange} />);

    const playButton = screen.getByRole('button', { name: /play/i });
    expect(playButton).toBeInTheDocument();
  });

  it('should show frame counter', () => {
    const onFrameChange = vi.fn();
    render(<AnimationControls snapshots={mockSnapshots} currentFrame={1} onFrameChange={onFrameChange} />);

    expect(screen.getByText(/1 \/ 2/)).toBeInTheDocument();
  });

  it('should call onFrameChange when slider moved', () => {
    const onFrameChange = vi.fn();
    render(<AnimationControls snapshots={mockSnapshots} currentFrame={0} onFrameChange={onFrameChange} />);

    const slider = screen.getByRole('slider');
    fireEvent.change(slider, { target: { value: '2' } });

    expect(onFrameChange).toHaveBeenCalledWith(2);
  });

  it('should reset to frame 0 when reset clicked', () => {
    const onFrameChange = vi.fn();
    render(<AnimationControls snapshots={mockSnapshots} currentFrame={2} onFrameChange={onFrameChange} />);

    const resetButton = screen.getByRole('button', { name: /skip back/i });
    fireEvent.click(resetButton);

    expect(onFrameChange).toHaveBeenCalledWith(0);
  });

  it('should show message when no snapshots', () => {
    const onFrameChange = vi.fn();
    render(<AnimationControls snapshots={[]} currentFrame={0} onFrameChange={onFrameChange} />);

    expect(screen.getByText(/no snapshots available/i)).toBeInTheDocument();
  });
});
```

**Step 2: Run tests**

Run: `npm run test AnimationControls.test.tsx`

Expected: All tests pass

**Step 3: Commit**

```bash
git add apps/frontend/src/components/__tests__/AnimationControls.test.tsx
git commit -m "test: add AnimationControls component tests"
```

---

## Task 22: Update Documentation

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Add animation playback section to CLAUDE.md**

Add to CLAUDE.md under appropriate section:

```markdown
## Animation Playback

**Feature**: Post-simulation animation playback for debugging and performance analysis.

**Configuration**:
- `enableSnapshots`: Enable snapshot capture (default: false)
- `snapshotIntervalSeconds`: Capture interval (default: 10.0 seconds)

**Backend**:
- `DesmojSnapshotCaptureProcess` runs every N seconds during simulation
- Captures: robots, relays, stations, totes, storage spaces, metrics
- Returns snapshots array in `SimulationResult`

**Frontend**:
- `AnimationControls`: Play/pause/scrub timeline with speed control
- `WarehouseMap`: Renders snapshot state with visual overlays
- Robot colors: VTM=blue, HTM=green, RTP=orange
- Storage space colors: Active=orange, Reserved=blue, Empty=gray

**Usage**:
```json
{
  "enableSnapshots": true,
  "snapshotIntervalSeconds": 10.0
}
```

View animation in results page after simulation completes.
```

**Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add animation playback feature to CLAUDE.md"
```

---

## Task 23: Final Integration Test

**Step 1: Clean and rebuild backend**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

**Step 2: Start backend server**

```bash
nx serve backend > /tmp/server.log 2>&1 &
sleep 10
curl http://localhost:8080/health
```

Expected: Server healthy

**Step 3: Run full simulation via API**

```bash
cat apps/backend/src/test/resources/integration-test-config.json | \
  curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @- > /tmp/final_test.json

# Verify snapshots present
cat /tmp/final_test.json | grep -o '"snapshots":\['
cat /tmp/final_test.json | grep -o '"timestamp":' | wc -l
```

Expected: Snapshots array present with multiple snapshots

**Step 4: Start frontend and test manually**

```bash
npm run dev
```

Navigate to frontend, run simulation, verify animation works end-to-end.

**Step 5: Cleanup**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
rm -f /tmp/server.log /tmp/final_test.json
```

---

## Task 24: Final Commit and Documentation

**Step 1: Review all changes**

```bash
git status
git diff --cached
```

**Step 2: Create final summary commit (if needed)**

If there are any uncommitted changes:

```bash
git add .
git commit -m "feat: animation playback feature complete

Complete implementation of post-simulation animation:
- Backend: Snapshot capture process with 10s intervals
- Data models: Comprehensive state tracking (robots, relays, stations, totes, storage spaces)
- API: Snapshots returned with SimulationResult
- Frontend: AnimationControls with play/pause/scrub
- Rendering: Visual overlays on WarehouseMap
- Tests: Backend integration and frontend component tests
- Docs: Updated CLAUDE.md with usage instructions"
```

**Step 3: Update implementation plan status**

Add to top of this file:

```markdown
**Status**: ✅ COMPLETED
**Completed**: [Current Date]
```

---

## Summary

This plan implements animation playback in 24 bite-sized tasks:

**Backend (Tasks 1-11)**:
- Data model records (Task 1)
- Configuration (Task 2)
- API changes (Task 3)
- Snapshot capture process (Tasks 4-9)
- Integration (Task 10)
- Testing (Task 11-12)

**Frontend (Tasks 13-19)**:
- Type generation (Task 13)
- Animation controls (Task 14)
- WarehouseMap updates (Tasks 15-18)
- Integration (Task 19)

**Testing & Docs (Tasks 20-24)**:
- Manual testing (Task 20)
- Component tests (Task 21)
- Documentation (Task 22)
- Final integration (Task 23)
- Completion (Task 24)

Each task is 2-10 minutes of focused work with clear verification steps.
