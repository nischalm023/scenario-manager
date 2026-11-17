# Snapshot Compression Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use @superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce animation snapshot file size from 1.24 GB to ~35-50 MB using keyframe + delta compression with automatic server GZIP.

**Architecture:** Implement sealed interface for KEYFRAME/DELTA snapshots, modify DesmojSnapshotCaptureProcess to track state changes and emit deltas, add frontend state reconstruction hook.

**Tech Stack:** Java 21 (sealed interfaces, records), Spring Boot 3.2, TypeScript/React, Jackson for JSON serialization

---

## Task 1: Create SnapshotType Enum

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/SnapshotType.java`

**Step 1: Write the enum**

```java
package com.greyorange.dessimulator.dto.snapshot;

/**
 * Type discriminator for warehouse snapshots.
 *
 * <p>KEYFRAME contains full warehouse state, DELTA contains only changed entities since last snapshot.
 */
public enum SnapshotType {
    /** Full state snapshot - all entities included */
    KEYFRAME,

    /** Delta snapshot - only changed entities included */
    DELTA
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/SnapshotType.java
git commit -m "feat: add SnapshotType enum for keyframe/delta discrimination"
```

---

## Task 2: Create WarehouseSnapshotV2 Sealed Interface

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/WarehouseSnapshotV2.java`

**Step 1: Write the sealed interface**

```java
package com.greyorange.dessimulator.dto.snapshot;

import com.fasterxml.jackson.annotation.JsonSubTypes;
import com.fasterxml.jackson.annotation.JsonTypeInfo;

/**
 * Sealed interface for warehouse snapshots supporting keyframe/delta compression.
 *
 * <p>Restricts implementations to KeyframeSnapshot and DeltaSnapshot only.
 * Jackson annotations enable polymorphic serialization with "type" discriminator field.
 */
@JsonTypeInfo(
    use = JsonTypeInfo.Id.NAME,
    include = JsonTypeInfo.As.PROPERTY,
    property = "type"
)
@JsonSubTypes({
    @JsonSubTypes.Type(value = KeyframeSnapshot.class, name = "KEYFRAME"),
    @JsonSubTypes.Type(value = DeltaSnapshot.class, name = "DELTA")
})
public sealed interface WarehouseSnapshotV2 permits KeyframeSnapshot, DeltaSnapshot {
    double timestamp();
    SnapshotType type();
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/WarehouseSnapshotV2.java
git commit -m "feat: add WarehouseSnapshotV2 sealed interface"
```

---

## Task 3: Create KeyframeSnapshot Record

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/KeyframeSnapshot.java`

**Step 1: Write the record**

```java
package com.greyorange.dessimulator.dto.snapshot;

import java.util.List;

/**
 * Full warehouse state snapshot containing all entities.
 *
 * <p>Captured at regular intervals (every 60s) to enable efficient seeking and delta reconstruction.
 */
public record KeyframeSnapshot(
    double timestamp,
    SnapshotType type, // Always KEYFRAME
    List<RobotState> robots,
    List<RelayState> relays,
    List<StationState> stations,
    List<ToteState> totes,
    List<StorageSpaceState> storageSpaces,
    SnapshotMetrics metrics
) implements WarehouseSnapshotV2 {}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/KeyframeSnapshot.java
git commit -m "feat: add KeyframeSnapshot record for full state snapshots"
```

---

## Task 4: Create DeltaSnapshot Record

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/DeltaSnapshot.java`

**Step 1: Write the record**

```java
package com.greyorange.dessimulator.dto.snapshot;

import java.util.List;

/**
 * Delta warehouse state snapshot containing only changed entities.
 *
 * <p>Contains only entities that changed since previous snapshot. Frontend reconstructs full
 * state by merging deltas into nearest preceding keyframe.
 */
public record DeltaSnapshot(
    double timestamp,
    SnapshotType type, // Always DELTA
    List<RobotState> robots,        // Changed robots (likely all 8, 100% change rate)
    List<RelayState> relays,        // Changed relays
    List<StationState> stations,    // Changed stations
    List<ToteState> totes,          // Changed totes (~10% = 336 items)
    List<StorageSpaceState> storageSpaces, // Changed spaces (~5-10% = 182-364 items)
    SnapshotMetrics metrics
) implements WarehouseSnapshotV2 {}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/dto/snapshot/DeltaSnapshot.java
git commit -m "feat: add DeltaSnapshot record for incremental state changes"
```

---

## Task 5: Add keyframeIntervalSeconds to SimulationConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java:48-81`

**Step 1: Add field to SimulationConfig record**

Find the `SimulationConfig` record (line 49-81) and add `Double keyframeIntervalSeconds` parameter:

```java
@Builder
public record SimulationConfig(
    double simulationDuration,
    String scenarioId,
    SystemType systemType,
    WarehouseMapConfig mapConfig,
    RobotConfig robotConfig,
    OperatorConfig operatorConfig,
    AlgorithmConfig algorithmConfig,
    Boolean enableSnapshots,
    Double snapshotIntervalSeconds,
    Double keyframeIntervalSeconds) {  // NEW FIELD
  public SimulationConfig {
    if (simulationDuration == 0) {
      simulationDuration = 14400.0;
    }
    if (scenarioId == null) {
      scenarioId = "scenario_a";
    }
    if (systemType == null) {
      systemType = SystemType.RTP;
    }
    if (operatorConfig == null) {
      operatorConfig = OperatorConfig.builder().build();
    }
    if (algorithmConfig == null) {
      algorithmConfig = AlgorithmConfig.builder().build();
    }
    if (enableSnapshots == null) {
      enableSnapshots = false;
    }
    if (snapshotIntervalSeconds == null) {
      snapshotIntervalSeconds = 10.0;
    }
    if (keyframeIntervalSeconds == null) {  // NEW DEFAULT
      keyframeIntervalSeconds = 60.0;
    }
  }
  // ... rest of the class
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java
git commit -m "feat: add keyframeIntervalSeconds config (default 60s)"
```

---

## Task 6: Update SimulationResults to use WarehouseSnapshotV2

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java:85-95`

**Step 1: Change snapshot type in SimulationResults record**

Find the `SimulationResults` record (around line 86-95) and change `List<WarehouseSnapshot>` to `List<WarehouseSnapshotV2>`:

```java
@Builder
public record SimulationResults(
    boolean success,
    String timestamp,
    String error,
    SimulationConfig config,
    List<com.greyorange.dessimulator.domain.station.Station> stations,
    List<Rack> racks,
    List<Robot> robots,
    SimulationMetric metrics,
    List<WarehouseSnapshotV2> snapshots) {}  // CHANGED: WarehouseSnapshot → WarehouseSnapshotV2
```

**Step 2: Add import at top of file**

Add this import after line 11:

```java
import com.greyorange.dessimulator.dto.snapshot.WarehouseSnapshotV2;
```

**Step 3: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS (may show warnings about callers needing updates - that's expected)

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java
git commit -m "feat: update SimulationResults to use WarehouseSnapshotV2"
```

---

## Task 7: Update DesmojSnapshotCaptureProcess - Add Fields

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java:14-29`

**Step 1: Update constructor and add delta tracking fields**

Replace the constructor and field declarations (lines 14-29) with:

```java
@Slf4j
public class DesmojSnapshotCaptureProcess extends SimProcess {
    private final DesmojWarehouseModel model;
    private final double intervalSeconds;
    private final double keyframeIntervalSeconds;  // NEW
    private final List<WarehouseSnapshotV2> snapshots;  // CHANGED TYPE

    private double lastKeyframeTime = -1.0;  // NEW

    // Previous state for delta computation - NEW
    private Map<String, RobotState> previousRobots = new HashMap<>();
    private Map<String, ToteState> previousTotes = new HashMap<>();
    private Map<String, StorageSpaceState> previousStorageSpaces = new HashMap<>();
    private Map<String, RelayState> previousRelays = new HashMap<>();
    private Map<Integer, StationState> previousStations = new HashMap<>();

    public DesmojSnapshotCaptureProcess(
        DesmojWarehouseModel model,
        double intervalSeconds,
        double keyframeIntervalSeconds,  // NEW PARAMETER
        List<WarehouseSnapshotV2> snapshots  // CHANGED TYPE
    ) {
        super(model, "SnapshotCapture", true);
        this.model = model;
        this.intervalSeconds = intervalSeconds;
        this.keyframeIntervalSeconds = keyframeIntervalSeconds;  // NEW
        this.snapshots = snapshots;
    }
```

**Step 2: Add imports at top of file**

Add these imports after the existing imports (around line 10):

```java
import com.greyorange.dessimulator.dto.snapshot.*;
import java.util.HashMap;
import java.util.Map;
import java.util.stream.Collectors;
```

**Step 3: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: FAILURE (callers need updates - expected, will fix in next task)

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "refactor: add delta tracking fields to DesmojSnapshotCaptureProcess"
```

---

## Task 8: Update DesmojSnapshotCaptureProcess - Rewrite lifeCycle()

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java:31-54`

**Step 1: Replace lifeCycle() method**

Replace the existing `lifeCycle()` method (lines 31-54) with keyframe/delta logic:

```java
@Override
public void lifeCycle() throws SuspendExecution {
    // First snapshot is always a keyframe
    captureKeyframe();

    while (true) {
        try {
            hold(new TimeSpan(intervalSeconds, TimeUnit.SECONDS));

            double currentTime = presentTime().getTimeAsDouble(TimeUnit.SECONDS);
            boolean shouldKeyframe = (currentTime - lastKeyframeTime) >= keyframeIntervalSeconds;

            if (shouldKeyframe) {
                captureKeyframe();
            } else {
                captureDelta();
            }
        } catch (Exception e) {
            log.debug("Snapshot capture ended: {}", e.getMessage());
            break;
        }
    }
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: FAILURE (`captureKeyframe()` and `captureDelta()` don't exist yet - will add next)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "refactor: rewrite lifeCycle to emit keyframes and deltas"
```

---

## Task 9: Add captureKeyframe() Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java:56-68`

**Step 1: Replace captureSnapshot() with captureKeyframe()**

Replace the existing `captureSnapshot()` method (lines 56-68) with:

```java
private void captureKeyframe() {
    double timestamp = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

    var robots = captureRobotStates();
    var relays = captureRelayStates();
    var stations = captureStationStates();
    var totes = captureToteStates();
    var storageSpaces = captureStorageSpaceStates();
    var metrics = captureMetrics();

    var keyframe = new KeyframeSnapshot(
        timestamp, SnapshotType.KEYFRAME,
        robots, relays, stations, totes, storageSpaces, metrics
    );

    snapshots.add(keyframe);
    lastKeyframeTime = timestamp;

    // Update previous state for next delta
    updatePreviousState(robots, relays, stations, totes, storageSpaces);

    log.debug("Captured KEYFRAME at t={} ({} robots, {} totes, {} storageSpaces)",
             timestamp, robots.size(), totes.size(), storageSpaces.size());
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: FAILURE (`updatePreviousState()` doesn't exist yet)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: implement captureKeyframe method"
```

---

## Task 10: Add captureDelta() Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java` (add new method after `captureKeyframe()`)

**Step 1: Add captureDelta() method after captureKeyframe()**

Insert this method right after the `captureKeyframe()` method:

```java
private void captureDelta() {
    double timestamp = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

    var currentRobots = captureRobotStates();
    var currentRelays = captureRelayStates();
    var currentStations = captureStationStates();
    var currentTotes = captureToteStates();
    var currentStorageSpaces = captureStorageSpaceStates();
    var metrics = captureMetrics();

    // Compute deltas (only changed entities)
    var changedRobots = computeChangedRobots(currentRobots);
    var changedRelays = computeChangedRelays(currentRelays);
    var changedStations = computeChangedStations(currentStations);
    var changedTotes = computeChangedTotes(currentTotes);
    var changedStorageSpaces = computeChangedStorageSpaces(currentStorageSpaces);

    var delta = new DeltaSnapshot(
        timestamp, SnapshotType.DELTA,
        changedRobots, changedRelays, changedStations,
        changedTotes, changedStorageSpaces, metrics
    );

    snapshots.add(delta);

    // Update previous state for next delta
    updatePreviousState(currentRobots, currentRelays, currentStations,
                      currentTotes, currentStorageSpaces);

    log.debug("Captured DELTA at t={} ({} robots, {} totes, {} storageSpaces changed)",
             timestamp, changedRobots.size(), changedTotes.size(),
             changedStorageSpaces.size());
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: FAILURE (compute methods don't exist yet)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: implement captureDelta method"
```

---

## Task 11: Add updatePreviousState() Helper

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java` (add new method after `captureDelta()`)

**Step 1: Add updatePreviousState() method**

Insert this method after `captureDelta()`:

```java
private void updatePreviousState(
        List<RobotState> robots,
        List<RelayState> relays,
        List<StationState> stations,
        List<ToteState> totes,
        List<StorageSpaceState> storageSpaces) {

    previousRobots = robots.stream()
        .collect(Collectors.toMap(RobotState::robotId, r -> r));
    previousRelays = relays.stream()
        .collect(Collectors.toMap(RelayState::relayId, r -> r));
    previousStations = stations.stream()
        .collect(Collectors.toMap(StationState::stationId, s -> s));
    previousTotes = totes.stream()
        .collect(Collectors.toMap(ToteState::toteId, t -> t));
    previousStorageSpaces = storageSpaces.stream()
        .collect(Collectors.toMap(StorageSpaceState::storageSpaceId, s -> s));
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: FAILURE (compute methods still missing)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: add updatePreviousState helper method"
```

---

## Task 12: Add Delta Computation Methods

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java` (add new methods at end before closing brace)

**Step 1: Add all five compute methods**

Insert these methods at the end of the class (before the final closing brace around line 304):

```java
private List<RobotState> computeChangedRobots(List<RobotState> current) {
    return current.stream()
        .filter(robot -> {
            var prev = previousRobots.get(robot.robotId());
            // Include if new or changed (records have automatic .equals())
            return prev == null || !prev.equals(robot);
        })
        .toList();
}

private List<RelayState> computeChangedRelays(List<RelayState> current) {
    return current.stream()
        .filter(relay -> {
            var prev = previousRelays.get(relay.relayId());
            return prev == null || !prev.equals(relay);
        })
        .toList();
}

private List<StationState> computeChangedStations(List<StationState> current) {
    return current.stream()
        .filter(station -> {
            var prev = previousStations.get(station.stationId());
            return prev == null || !prev.equals(station);
        })
        .toList();
}

private List<ToteState> computeChangedTotes(List<ToteState> current) {
    return current.stream()
        .filter(tote -> {
            var prev = previousTotes.get(tote.toteId());
            return prev == null || !prev.equals(tote);
        })
        .toList();
}

private List<StorageSpaceState> computeChangedStorageSpaces(List<StorageSpaceState> current) {
    return current.stream()
        .filter(space -> {
            var prev = previousStorageSpaces.get(space.storageSpaceId());
            return prev == null || !prev.equals(space);
        })
        .toList();
}
```

**Step 2: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS (but callers still need updates)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java
git commit -m "feat: add delta computation methods using record equality"
```

---

## Task 13: Update DesmojSimulationEngine to Pass keyframeIntervalSeconds

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java` (find where DesmojSnapshotCaptureProcess is instantiated)

**Step 1: Find the snapshot process instantiation**

Run: `cd apps/backend && grep -n "new DesmojSnapshotCaptureProcess" src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 2: Update the constructor call**

Find the line that creates `DesmojSnapshotCaptureProcess` (likely around line 80-100) and add the `keyframeIntervalSeconds` parameter:

```java
// Before:
var snapshotProcess = new DesmojSnapshotCaptureProcess(
    model,
    config.snapshotIntervalSeconds(),
    snapshots
);

// After:
var snapshotProcess = new DesmojSnapshotCaptureProcess(
    model,
    config.snapshotIntervalSeconds(),
    config.keyframeIntervalSeconds(),  // NEW PARAMETER
    snapshots
);
```

**Step 3: Update snapshot list declaration**

Find where the snapshots list is declared and change type:

```java
// Before:
List<WarehouseSnapshot> snapshots = new ArrayList<>();

// After:
List<WarehouseSnapshotV2> snapshots = new ArrayList<>();
```

**Step 4: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "fix: pass keyframeIntervalSeconds to snapshot process"
```

---

## Task 14: Update application.yml for Better GZIP Compression

**Files:**
- Modify: `apps/backend/src/main/resources/application.yml:1-6`

**Step 1: Update GZIP compression config**

Replace lines 1-6 with improved configuration:

```yaml
server:
  port: 8080
  compression:
    enabled: true
    mime-types:
      - application/json
      - text/html
      - text/plain
    min-response-size: 1024  # Compress responses > 1KB
```

**Step 2: Verify config is valid**

Run: `cd apps/backend && mvn spring-boot:run -DskipTests &`
Wait 5 seconds, then: `curl -I http://localhost:8080/health`
Expected: See `Content-Encoding: gzip` header (if response > 1KB)

**Step 3: Stop server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 4: Commit**

```bash
git add apps/backend/src/main/resources/application.yml
git commit -m "config: optimize GZIP compression settings"
```

---

## Task 15: Add Snapshot Stats Logging to SimulationsController

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java` (find the `runSimulation` method)

**Step 1: Add logging after simulation runs**

Find the method that calls the simulation engine (likely `runSimulation()`) and add logging before returning:

```java
// After: var result = engine.run(config);
// Add this logging block:

if (result.snapshots() != null && !result.snapshots().isEmpty()) {
    long keyframes = result.snapshots().stream()
        .filter(s -> s.type() == SnapshotType.KEYFRAME)
        .count();
    long deltas = result.snapshots().size() - keyframes;

    log.info("Returning {} snapshots ({} keyframes, {} deltas)",
             result.snapshots().size(), keyframes, deltas);
}
```

**Step 2: Add import for SnapshotType**

Add at top of file:

```java
import com.greyorange.dessimulator.dto.snapshot.SnapshotType;
```

**Step 3: Verify it compiles**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java
git commit -m "feat: add snapshot statistics logging"
```

---

## Task 16: Backend Integration Test - Run Real Simulation

**Files:**
- No new files, just testing

**Step 1: Clean and compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: SUCCESS

**Step 2: Start server**

Run: `cd apps/backend && mvn spring-boot:run > /tmp/server.log 2>&1 &`
Wait 10 seconds for startup

**Step 3: Run test simulation (1 hour, snapshots enabled)**

Create test config:

```bash
cat > /tmp/test-snapshot-compression.json <<'EOF'
{
  "simulationDuration": 3600,
  "scenarioId": "scenario_a",
  "systemType": "RELAY",
  "enableSnapshots": true,
  "snapshotIntervalSeconds": 10.0,
  "keyframeIntervalSeconds": 60.0
}
EOF
```

Run simulation:

```bash
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @/tmp/test-snapshot-compression.json \
  > /tmp/snapshot-test-result.json
```

**Step 4: Analyze results**

Check snapshot counts:

```bash
python3 <<'EOF'
import json
with open('/tmp/snapshot-test-result.json') as f:
    result = json.load(f)

snapshots = result.get('data', {}).get('snapshots', [])
print(f"Total snapshots: {len(snapshots)}")

keyframes = [s for s in snapshots if s.get('type') == 'KEYFRAME']
deltas = [s for s in snapshots if s.get('type') == 'DELTA']

print(f"Keyframes: {len(keyframes)}")
print(f"Deltas: {len(deltas)}")
print(f"Expected keyframes for 1hr at 60s interval: ~60")
print(f"Expected deltas for 1hr at 10s interval: ~300")

# Check file size
import os
size_mb = os.path.getsize('/tmp/snapshot-test-result.json') / (1024 * 1024)
print(f"\nFile size: {size_mb:.2f} MB")
print(f"Target: < 50 MB uncompressed for 1-hour simulation")
EOF
```

Expected output:
- Total snapshots: ~360
- Keyframes: ~60
- Deltas: ~300
- File size: < 50 MB

**Step 5: Stop server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 6: Commit if successful**

```bash
git add -A
git commit -m "test: verify snapshot compression works with 1-hour simulation"
```

---

## Task 17: Regenerate Frontend TypeScript Types

**Files:**
- Modify: `apps/frontend/src/types/generated-api.ts` (regenerated)

**Step 1: Start backend server**

Run: `cd apps/backend && mvn spring-boot:run > /tmp/server.log 2>&1 &`
Wait 10 seconds

**Step 2: Regenerate types from OpenAPI spec**

Run: `npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts`

**Step 3: Verify new types exist**

Run: `grep -A5 "WarehouseSnapshotV2\|KeyframeSnapshot\|DeltaSnapshot" apps/frontend/src/types/generated-api.ts`

Expected: See the new snapshot types

**Step 4: Stop server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 5: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "types: regenerate from OpenAPI spec with WarehouseSnapshotV2"
```

---

## Task 18: Create useSnapshotReconstruction Hook

**Files:**
- Create: `apps/frontend/src/hooks/useSnapshotReconstruction.ts`

**Step 1: Create the hook**

```typescript
import { useMemo } from 'react'
import type { components } from '@/types/generated-api'

type WarehouseSnapshotV2 = components['schemas']['WarehouseSnapshotV2']
type KeyframeSnapshot = components['schemas']['KeyframeSnapshot']
type DeltaSnapshot = components['schemas']['DeltaSnapshot']
type RobotState = components['schemas']['RobotState']
type ToteState = components['schemas']['ToteState']
type StorageSpaceState = components['schemas']['StorageSpaceState']
type RelayState = components['schemas']['RelayState']
type StationState = components['schemas']['StationState']
type SnapshotMetrics = components['schemas']['SnapshotMetrics']

interface ReconstructedSnapshot {
  timestamp: number
  robots: RobotState[]
  relays: RelayState[]
  stations: StationState[]
  totes: ToteState[]
  storageSpaces: StorageSpaceState[]
  metrics: SnapshotMetrics
}

/**
 * Reconstructs full warehouse state from keyframe + delta snapshots.
 *
 * Runs once on mount via useMemo. Each delta is merged into the nearest
 * preceding keyframe by entity ID.
 */
export function useSnapshotReconstruction(
  rawSnapshots: WarehouseSnapshotV2[]
): ReconstructedSnapshot[] {
  return useMemo(() => {
    if (!rawSnapshots || rawSnapshots.length === 0) return []

    const reconstructed: ReconstructedSnapshot[] = []
    let currentState: ReconstructedSnapshot | null = null

    for (const snapshot of rawSnapshots) {
      if (snapshot.type === 'KEYFRAME') {
        const keyframe = snapshot as KeyframeSnapshot

        // Keyframe: use as-is
        currentState = {
          timestamp: keyframe.timestamp,
          robots: keyframe.robots,
          relays: keyframe.relays,
          stations: keyframe.stations,
          totes: keyframe.totes,
          storageSpaces: keyframe.storageSpaces,
          metrics: keyframe.metrics,
        }
        reconstructed.push(currentState)
      } else if (snapshot.type === 'DELTA') {
        const delta = snapshot as DeltaSnapshot

        // Delta: merge changes into current state
        if (!currentState) {
          console.error('Delta snapshot without preceding keyframe at t=' + delta.timestamp)
          continue
        }

        currentState = {
          timestamp: delta.timestamp,
          robots: mergeById(currentState.robots, delta.robots, 'robotId'),
          relays: mergeById(currentState.relays, delta.relays, 'relayId'),
          stations: mergeById(currentState.stations, delta.stations, 'stationId'),
          totes: mergeById(currentState.totes, delta.totes, 'toteId'),
          storageSpaces: mergeById(
            currentState.storageSpaces,
            delta.storageSpaces,
            'storageSpaceId'
          ),
          metrics: delta.metrics, // Always use latest metrics
        }
        reconstructed.push(currentState)
      }
    }

    return reconstructed
  }, [rawSnapshots])
}

/**
 * Merge entities by ID - updates existing or adds new.
 */
function mergeById<T extends Record<string, any>>(
  current: T[],
  changes: T[],
  idField: keyof T
): T[] {
  const merged = new Map(current.map(item => [item[idField], item]))

  // Apply changes (updates existing or adds new)
  for (const change of changes) {
    merged.set(change[idField], change)
  }

  return Array.from(merged.values())
}
```

**Step 2: Verify it type-checks**

Run: `cd apps/frontend && npm run typecheck`
Expected: No errors in useSnapshotReconstruction.ts

**Step 3: Commit**

```bash
git add apps/frontend/src/hooks/useSnapshotReconstruction.ts
git commit -m "feat: add useSnapshotReconstruction hook for delta merging"
```

---

## Task 19: Write Tests for useSnapshotReconstruction

**Files:**
- Create: `apps/frontend/src/hooks/__tests__/useSnapshotReconstruction.test.ts`

**Step 1: Write the test file**

```typescript
import { renderHook } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { useSnapshotReconstruction } from '../useSnapshotReconstruction'

describe('useSnapshotReconstruction', () => {
  it('should handle empty snapshots', () => {
    const { result } = renderHook(() => useSnapshotReconstruction([]))
    expect(result.current).toEqual([])
  })

  it('should handle keyframe-only snapshots', () => {
    const snapshots = [
      {
        type: 'KEYFRAME',
        timestamp: 0,
        robots: [{ robotId: '0', status: 'IDLE', position: { x: 10, y: 20 } }],
        relays: [],
        stations: [],
        totes: [],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 0, totalOrdersCompleted: 0, picksPerHour: 0 },
      },
    ]

    const { result } = renderHook(() => useSnapshotReconstruction(snapshots as any))

    expect(result.current).toHaveLength(1)
    expect(result.current[0].robots[0].status).toBe('IDLE')
  })

  it('should merge delta changes correctly', () => {
    const snapshots = [
      {
        type: 'KEYFRAME',
        timestamp: 0,
        robots: [
          { robotId: '0', status: 'IDLE', position: { x: 10, y: 20 } },
          { robotId: '1', status: 'IDLE', position: { x: 15, y: 25 } },
        ],
        relays: [],
        stations: [],
        totes: [],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 0, totalOrdersCompleted: 0, picksPerHour: 0 },
      },
      {
        type: 'DELTA',
        timestamp: 10,
        robots: [
          { robotId: '0', status: 'MOVING', position: { x: 10, y: 21 } }, // Changed
        ],
        relays: [],
        stations: [],
        totes: [],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 5, totalOrdersCompleted: 1, picksPerHour: 1800 },
      },
    ]

    const { result } = renderHook(() => useSnapshotReconstruction(snapshots as any))

    // Frame 0: original keyframe
    expect(result.current[0].robots).toHaveLength(2)
    expect(result.current[0].robots[0].status).toBe('IDLE')

    // Frame 1: delta merged - robot 0 updated, robot 1 unchanged
    expect(result.current[1].robots).toHaveLength(2)
    expect(result.current[1].robots[0].status).toBe('MOVING')
    expect(result.current[1].robots[0].position.y).toBe(21)
    expect(result.current[1].robots[1].status).toBe('IDLE') // Unchanged
    expect(result.current[1].metrics.totalTasksCompleted).toBe(5)
  })

  it('should handle multiple deltas between keyframes', () => {
    const snapshots = [
      {
        type: 'KEYFRAME',
        timestamp: 0,
        robots: [],
        relays: [],
        stations: [],
        totes: [{ toteId: '1', location: 'AT_RACK', taskCount: 5 }],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 0, totalOrdersCompleted: 0, picksPerHour: 0 },
      },
      {
        type: 'DELTA',
        timestamp: 10,
        robots: [],
        relays: [],
        stations: [],
        totes: [{ toteId: '1', location: 'IN_TRANSIT', taskCount: 5 }],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 0, totalOrdersCompleted: 0, picksPerHour: 0 },
      },
      {
        type: 'DELTA',
        timestamp: 20,
        robots: [],
        relays: [],
        stations: [],
        totes: [{ toteId: '1', location: 'AT_RELAY', taskCount: 5 }],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 0, totalOrdersCompleted: 0, picksPerHour: 0 },
      },
      {
        type: 'DELTA',
        timestamp: 30,
        robots: [],
        relays: [],
        stations: [],
        totes: [{ toteId: '1', location: 'AT_STATION', taskCount: 3 }],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 2, totalOrdersCompleted: 0, picksPerHour: 240 },
      },
    ]

    const { result } = renderHook(() => useSnapshotReconstruction(snapshots as any))

    expect(result.current[0].totes[0].location).toBe('AT_RACK')
    expect(result.current[1].totes[0].location).toBe('IN_TRANSIT')
    expect(result.current[2].totes[0].location).toBe('AT_RELAY')
    expect(result.current[3].totes[0].location).toBe('AT_STATION')
    expect(result.current[3].totes[0].taskCount).toBe(3)
  })

  it('should handle delta without preceding keyframe gracefully', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})

    const snapshots = [
      {
        type: 'DELTA',
        timestamp: 10,
        robots: [{ robotId: '0', status: 'MOVING' }],
        relays: [],
        stations: [],
        totes: [],
        storageSpaces: [],
        metrics: { totalTasksCompleted: 0, totalOrdersCompleted: 0, picksPerHour: 0 },
      },
    ]

    const { result } = renderHook(() => useSnapshotReconstruction(snapshots as any))

    expect(result.current).toHaveLength(0) // Delta skipped
    expect(consoleError).toHaveBeenCalledWith(expect.stringContaining('Delta snapshot without'))

    consoleError.mockRestore()
  })
})
```

**Step 2: Run tests**

Run: `cd apps/frontend && npm run test useSnapshotReconstruction`
Expected: All tests pass

**Step 3: Commit**

```bash
git add apps/frontend/src/hooks/__tests__/useSnapshotReconstruction.test.ts
git commit -m "test: add comprehensive tests for useSnapshotReconstruction"
```

---

## Task 20: Update Results Page to Use Reconstructed Snapshots

**Files:**
- Modify: `apps/frontend/src/pages/SimulationResultsPage.tsx` (or wherever AnimationControls is used)

**Step 1: Find where snapshots are passed to AnimationControls**

Run: `cd apps/frontend && grep -rn "AnimationControls" src/`

**Step 2: Import and use the reconstruction hook**

Add import at top:

```typescript
import { useSnapshotReconstruction } from '@/hooks/useSnapshotReconstruction'
```

Find where `result?.data?.snapshots` is used and wrap with reconstruction:

```typescript
// Before:
const snapshots = result?.data?.snapshots ?? []

// After:
const reconstructedSnapshots = useSnapshotReconstruction(
  result?.data?.snapshots ?? []
)
```

Update AnimationControls and WarehouseMap to use `reconstructedSnapshots`:

```typescript
{reconstructedSnapshots.length > 0 && (
  <AnimationControls
    snapshots={reconstructedSnapshots}
    currentFrame={currentFrame}
    onFrameChange={setCurrentFrame}
  />
)}

{currentSnapshot && (
  <WarehouseMap
    robots={currentSnapshot.robots}
    totes={currentSnapshot.totes}
    storageSpaces={currentSnapshot.storageSpaces}
    relays={currentSnapshot.relays}
    stations={currentSnapshot.stations}
  />
)}
```

**Step 3: Verify it type-checks**

Run: `cd apps/frontend && npm run typecheck`
Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/pages/SimulationResultsPage.tsx
git commit -m "feat: integrate useSnapshotReconstruction in results page"
```

---

## Task 21: End-to-End Manual Test

**Files:**
- No code changes, just verification

**Step 1: Clean build backend**

Run: `cd apps/backend && mvn clean compile -DskipTests`

**Step 2: Start backend server**

Run: `cd apps/backend && mvn spring-boot:run > /tmp/server.log 2>&1 &`
Wait 10 seconds

**Step 3: Start frontend dev server**

Run: `cd apps/frontend && npm run dev &`
Wait 10 seconds

**Step 4: Open browser and run simulation**

Open: `http://localhost:4200`

Configure simulation:
- System: RELAY
- Duration: 1 hour (3600s)
- Enable snapshots: YES
- Snapshot interval: 10s
- Keyframe interval: 60s

Click "Run Simulation"

**Step 5: Verify animation works**

After simulation completes:
- Check animation controls appear
- Click play - animation should play smoothly
- Scrub timeline - should jump to any frame
- Check browser Network tab - response should show `Content-Encoding: gzip`
- Check response size - should be < 50 MB transferred

**Step 6: Check browser console**

Expected: No errors
If you see "Delta snapshot without preceding keyframe" - something is wrong

**Step 7: Stop servers**

Run:
```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
lsof -ti:4200 | xargs kill -9 2>/dev/null || true
```

**Step 8: Document results**

If successful, commit:

```bash
git commit --allow-empty -m "test: verify end-to-end snapshot compression works

- Ran 1-hour relay simulation with snapshots enabled
- Verified animation plays smoothly
- Confirmed GZIP compression active
- Response size < 50 MB transferred
- No console errors"
```

---

## Task 22: Production Validation (4-Hour Simulation)

**Files:**
- No code changes, final verification

**Step 1: Create production test config**

```bash
cat > /tmp/production-compression-test.json <<'EOF'
{
  "simulationDuration": 14400,
  "scenarioId": "scenario_a",
  "systemType": "RELAY",
  "enableSnapshots": true,
  "snapshotIntervalSeconds": 10.0,
  "keyframeIntervalSeconds": 60.0,
  "robotConfig": {
    "configType": "relay",
    "numVtms": 6,
    "numHtms": 4
  }
}
EOF
```

**Step 2: Start server and run simulation**

Run:
```bash
cd apps/backend && mvn spring-boot:run > /tmp/prod-test.log 2>&1 &
sleep 15

curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @/tmp/production-compression-test.json \
  > /tmp/prod-test-result.json
```

This will take ~5-10 minutes depending on machine.

**Step 3: Analyze results**

```bash
python3 <<'EOF'
import json
import os

with open('/tmp/prod-test-result.json') as f:
    result = json.load(f)

snapshots = result.get('data', {}).get('snapshots', [])
print(f"Total snapshots: {len(snapshots)}")

keyframes = [s for s in snapshots if s.get('type') == 'KEYFRAME']
deltas = [s for s in snapshots if s.get('type') == 'DELTA']

print(f"Keyframes: {len(keyframes)}")
print(f"Deltas: {len(deltas)}")
print(f"Expected keyframes: ~240 (14400s / 60s)")
print(f"Expected deltas: ~1200 (14400s / 10s - keyframes)")

size_mb = os.path.getsize('/tmp/prod-test-result.json') / (1024 * 1024)
print(f"\nUncompressed JSON: {size_mb:.2f} MB")
print(f"Target: < 200 MB")
print(f"Expected: ~138 MB")

if size_mb < 200:
    print("\n✓ SUCCESS: File size within target")
else:
    print("\n✗ FAILURE: File size exceeds target")

if len(keyframes) > 220 and len(keyframes) < 260:
    print("✓ SUCCESS: Keyframe count looks correct")
else:
    print("✗ WARNING: Keyframe count unexpected")
EOF
```

**Step 4: Stop server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 5: Commit validation results**

```bash
git commit --allow-empty -m "test: production validation complete

4-hour simulation results:
- Uncompressed JSON: ~138 MB (89% reduction from 1,240 MB baseline)
- Keyframes: ~240
- Deltas: ~1,200
- With GZIP: expect ~35-50 MB transfer size

Target metrics achieved."
```

---

## Success Criteria

After completing all tasks, verify:

- ✅ Backend compiles without errors: `mvn clean compile -DskipTests`
- ✅ Frontend compiles without errors: `npm run typecheck`
- ✅ Tests pass: `npm run test useSnapshotReconstruction`
- ✅ 1-hour simulation produces ~60 keyframes, ~300 deltas
- ✅ 4-hour simulation produces ~240 keyframes, ~1,200 deltas
- ✅ Uncompressed JSON < 200 MB for 4-hour simulation
- ✅ GZIP compression enabled (check `Content-Encoding` header)
- ✅ Animation plays smoothly in browser
- ✅ No console errors during playback

## Reference Files

- Design doc: `docs/plans/2025-11-12-snapshot-compression-design.md`
- Current snapshot capture: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`
- Simulation config: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java`
- Animation controls: `apps/frontend/src/components/AnimationControls.tsx`
