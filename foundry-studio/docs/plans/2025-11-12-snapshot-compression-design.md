# Animation Snapshot Compression Design

**Date:** 2025-11-12
**Status:** Approved
**Author:** Design collaboration with user

## Executive Summary

Reduce animation snapshot file size from 1.24 GB to ~35-50 MB (96% reduction) using a two-layer compression strategy:

1. **Application Layer:** Keyframe + Delta compression (1,240 MB → 138 MB)
2. **Transport Layer:** Server GZIP compression (138 MB → 35-50 MB)

**Key Requirements:**
- Maintain 10s snapshot interval for smooth debugging/analysis
- Support engineers debugging issues (precision matters)
- Support operations teams analyzing bottlenecks (detail matters)
- No backwards compatibility needed (clean implementation)

## Problem Statement

Current animation snapshot implementation captures full warehouse state every 10 seconds:
- **File size:** 1.24 GB for 4-hour simulation (1,440 snapshots)
- **Bloat breakdown:**
  - storageSpaces: 1,069 MB (77%) - 3,640 items per snapshot
  - totes: 271 MB (20%) - 3,360 items per snapshot
  - relays: 35 MB (2.5%) - 280 items per snapshot
  - robots/stations: <1 MB (negligible)

**Change frequency analysis:**
- Robots (8 items): ~100% change rate (all moving constantly)
- Totes (3,360 items): ~10% change rate (336 items/snapshot)
- StorageSpaces (3,640 items): ~5-10% change rate (182-364 items/snapshot)

**Opportunity:** 90-95% of data is unchanged between snapshots → delta compression ideal.

## High-Level Architecture

### Three-Layer Compression Strategy

```
Backend Simulation
  ↓
DesmojSnapshotCaptureProcess
  ├─ Captures state every 10s
  ├─ Emits KEYFRAME every 60s (full state)
  └─ Emits DELTA every 10s (only changed entities)
  ↓
Snapshot Array [keyframe, Δ, Δ, Δ, Δ, Δ, keyframe, Δ, ...]
  ↓
Server GZIP (Content-Encoding: gzip)
  ├─ Compresses repetitive JSON patterns
  └─ 65-75% additional reduction
  ↓
Browser Auto-Decompression
  ↓
Frontend State Reconstruction
  ├─ Load nearest keyframe
  └─ Apply deltas forward
  ↓
Render Animation
```

### Key Design Decisions

1. **Keyframe interval: 60s** - Balances seek performance vs compression (every 6 snapshots)
2. **Delta based on entity ID** - Frontend merges by ID, allows partial updates
3. **Server GZIP compression** - Standard HTTP compression (no custom base64 encoding)
4. **Snapshot interval: 10s** - Maintained for smooth debugging experience

### Expected Size Reduction

| Stage | Size | Reduction |
|-------|------|-----------|
| Baseline (full snapshots) | 1,240 MB | - |
| After delta compression | 138 MB | 89% |
| After server GZIP | 35-50 MB | 96% |

## Backend Implementation

### 1. Snapshot DTO Structure

Sealed interface for type-safe keyframe/delta snapshots:

```java
// Sealed interface for type safety
public sealed interface WarehouseSnapshotV2 permits KeyframeSnapshot, DeltaSnapshot {
    double timestamp();
    SnapshotType type();
}

public enum SnapshotType {
    KEYFRAME,  // Full state dump
    DELTA      // Only changed entities
}

// Full state snapshot
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

// Delta snapshot - only changed entities
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

### 2. DesmojSnapshotCaptureProcess Changes

Track previous state and emit keyframes/deltas:

```java
@Slf4j
public class DesmojSnapshotCaptureProcess extends SimProcess {
    private final DesmojWarehouseModel model;
    private final double intervalSeconds;
    private final double keyframeIntervalSeconds; // New: 60s
    private final List<WarehouseSnapshotV2> snapshots;

    private double lastKeyframeTime = -1.0;

    // Previous state for delta computation
    private Map<String, RobotState> previousRobots = new HashMap<>();
    private Map<String, ToteState> previousTotes = new HashMap<>();
    private Map<String, StorageSpaceState> previousStorageSpaces = new HashMap<>();
    private Map<String, RelayState> previousRelays = new HashMap<>();
    private Map<Integer, StationState> previousStations = new HashMap<>();

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

        log.debug("Captured KEYFRAME at t={}", timestamp);
    }

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

        log.debug("Captured DELTA at t={} (robots={}, totes={}, storageSpaces={})",
                 timestamp, changedRobots.size(), changedTotes.size(),
                 changedStorageSpaces.size());
    }

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
}
```

### 3. Delta Computation Logic

Use value equality (records have automatic `.equals()`):

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

// Similar for relays and stations
```

### 4. Configuration Changes

Add keyframe interval to `SimulationConfig`:

```java
public record SimulationConfig(
    // ... existing fields ...

    boolean enableSnapshots,
    double snapshotIntervalSeconds,

    // New: keyframe interval
    @Builder.Default
    double keyframeIntervalSeconds = 60.0  // Keyframe every 60s
) {}
```

### 5. Server GZIP Compression

Enable automatic compression in `application.yml`:

```yaml
server:
  compression:
    enabled: true
    mime-types:
      - application/json
      - text/html
      - text/plain
    min-response-size: 1024  # Compress responses > 1KB
```

Spring Boot automatically:
- Detects client supports GZIP (via `Accept-Encoding: gzip` header)
- Compresses response body
- Adds `Content-Encoding: gzip` header
- Browser automatically decompresses

### 6. API Response (Unchanged)

No custom compression needed - server handles it automatically:

```java
public record SimulationResult(
    boolean success,
    SimulationData data,
    String timestamp,
    @Nullable String error
) {}

public record SimulationData(
    SimulationConfig config,
    List<Station> stations,
    List<Rack> racks,
    List<Robot> robots,
    ComprehensiveMetrics metrics,
    List<WarehouseSnapshotV2> snapshots  // Server compresses automatically
) {}
```

### 7. Controller Changes

Add logging for snapshot statistics:

```java
@PostMapping("/run")
public ResponseEntity<SimulationResult> runSimulation(@RequestBody SimulationConfig config) {
    var result = engine.run(config);

    // Log snapshot stats for monitoring
    if (result.data().snapshots() != null) {
        long keyframes = result.data().snapshots().stream()
            .filter(s -> s.type() == SnapshotType.KEYFRAME)
            .count();
        long deltas = result.data().snapshots().size() - keyframes;

        log.info("Returning {} snapshots ({} keyframes, {} deltas)",
                 result.data().snapshots().size(), keyframes, deltas);
    }

    // Spring Boot automatically compresses this response
    return ResponseEntity.ok(result);
}
```

## Frontend Implementation

### 1. State Reconstruction Hook

Preprocess snapshots on load to reconstruct full state:

```typescript
// hooks/useSnapshotReconstruction.ts
import { useMemo } from 'react'
import type { components } from '@/types/generated-api'

type WarehouseSnapshotV2 = components['schemas']['WarehouseSnapshotV2']
type RobotState = components['schemas']['RobotState']
type ToteState = components['schemas']['ToteState']
type StorageSpaceState = components['schemas']['StorageSpaceState']
// ... other state types

interface ReconstructedSnapshot {
  timestamp: number
  robots: RobotState[]
  relays: RelayState[]
  stations: StationState[]
  totes: ToteState[]
  storageSpaces: StorageSpaceState[]
  metrics: SnapshotMetrics
}

export function useSnapshotReconstruction(
  rawSnapshots: WarehouseSnapshotV2[]
): ReconstructedSnapshot[] {
  return useMemo(() => {
    if (!rawSnapshots || rawSnapshots.length === 0) return []

    const reconstructed: ReconstructedSnapshot[] = []
    let currentState: ReconstructedSnapshot | null = null

    for (const snapshot of rawSnapshots) {
      if (snapshot.type === 'KEYFRAME') {
        // Keyframe: use as-is
        currentState = {
          timestamp: snapshot.timestamp,
          robots: snapshot.robots,
          relays: snapshot.relays,
          stations: snapshot.stations,
          totes: snapshot.totes,
          storageSpaces: snapshot.storageSpaces,
          metrics: snapshot.metrics,
        }
        reconstructed.push(currentState)
      } else if (snapshot.type === 'DELTA') {
        // Delta: merge changes into current state
        if (!currentState) {
          console.error('Delta snapshot without preceding keyframe!')
          continue
        }

        currentState = {
          timestamp: snapshot.timestamp,
          robots: mergeById(currentState.robots, snapshot.robots, 'robotId'),
          relays: mergeById(currentState.relays, snapshot.relays, 'relayId'),
          stations: mergeById(currentState.stations, snapshot.stations, 'stationId'),
          totes: mergeById(currentState.totes, snapshot.totes, 'toteId'),
          storageSpaces: mergeById(
            currentState.storageSpaces,
            snapshot.storageSpaces,
            'storageSpaceId'
          ),
          metrics: snapshot.metrics, // Always use latest
        }
        reconstructed.push(currentState)
      }
    }

    return reconstructed
  }, [rawSnapshots])
}

// Generic merge helper - replace entities by ID
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

### 2. Results Page Integration

Use reconstructed snapshots for animation:

```typescript
// pages/SimulationResultsPage.tsx
import { useSnapshotReconstruction } from '@/hooks/useSnapshotReconstruction'

export function SimulationResultsPage() {
  const { data: result } = useSimulationResult() // API call

  // Reconstruct full state from keyframes + deltas
  const reconstructedSnapshots = useSnapshotReconstruction(
    result?.data?.snapshots ?? []
  )

  const [currentFrame, setCurrentFrame] = useState(0)

  // Current frame's full state
  const currentSnapshot = reconstructedSnapshots[currentFrame]

  return (
    <div>
      {/* Animation Controls */}
      {reconstructedSnapshots.length > 0 && (
        <AnimationControls
          snapshots={reconstructedSnapshots}
          currentFrame={currentFrame}
          onFrameChange={setCurrentFrame}
        />
      )}

      {/* Warehouse Map - renders current frame */}
      {currentSnapshot && (
        <WarehouseMap
          robots={currentSnapshot.robots}
          totes={currentSnapshot.totes}
          storageSpaces={currentSnapshot.storageSpaces}
          relays={currentSnapshot.relays}
          stations={currentSnapshot.stations}
        />
      )}

      {/* Metrics display */}
      {currentSnapshot && (
        <MetricsPanel metrics={currentSnapshot.metrics} />
      )}
    </div>
  )
}
```

### 3. No Changes Needed

These components work as-is:
- **AnimationControls**: Already accepts `snapshots` array
- **WarehouseMap**: Already renders snapshot state
- **Type generation**: Just regenerate from updated OpenAPI spec

## Testing Strategy

### Backend Unit Tests

**Delta Computation Tests:**

```java
@Test
void testRobotDeltaDetectsChanges() {
    var previous = new RobotState("0", "VTM", "IDLE",
                                  new Position(10, 20), null, null);
    var current = new RobotState("0", "VTM", "MOVING",
                                 new Position(10, 21), "task_1", null);

    // Different position and status = should be in delta
    assertNotEquals(previous, current);
}

@Test
void testToteDeltaOnlyIncludesChanged() {
    var allTotes = List.of(
        new ToteState("1", "AT_RACK", 5),
        new ToteState("2", "AT_RELAY", 3),
        new ToteState("3", "AT_RACK", 5)  // Unchanged
    );

    var previousMap = Map.of(
        "1", new ToteState("1", "AT_RACK", 6),      // Changed taskCount
        "2", new ToteState("2", "IN_TRANSIT", 3),   // Changed location
        "3", new ToteState("3", "AT_RACK", 5)       // Unchanged
    );

    var changed = computeChangedTotes(allTotes, previousMap);

    assertEquals(2, changed.size());  // Only tote 1 and 2
}

@Test
void testKeyframeIntervalScheduling() {
    // Verify keyframes at t=0, t=60, t=120...
    assertTrue(isKeyframeTime(0.0, -1.0, 60.0));
    assertFalse(isKeyframeTime(10.0, 0.0, 60.0));
    assertTrue(isKeyframeTime(60.0, 0.0, 60.0));
}
```

### Backend Integration Tests

**Size Verification:**

```java
@SpringBootTest
class SnapshotCompressionIntegrationTest {

    @Test
    void testDeltaCompressionReducesSize() throws Exception {
        var config = SimulationConfig.builder()
            .systemType(SystemType.RELAY)
            .duration(4.0)  // 4 hours
            .enableSnapshots(true)
            .snapshotIntervalSeconds(10.0)
            .keyframeIntervalSeconds(60.0)
            .build();

        var result = engine.run(config);
        var snapshots = result.data().snapshots();

        // Verify snapshot counts
        long keyframes = snapshots.stream()
            .filter(s -> s.type() == SnapshotType.KEYFRAME)
            .count();
        long deltas = snapshots.size() - keyframes;

        assertTrue(keyframes > 0, "Should have keyframes");
        assertTrue(deltas > keyframes * 5, "Should have 5+ deltas per keyframe");

        // Serialize and measure sizes
        String fullJson = objectMapper.writeValueAsString(snapshots);
        double sizeMB = fullJson.length() / (1024.0 * 1024.0);

        // Should be under 200 MB uncompressed (vs 1,240 MB baseline)
        assertTrue(sizeMB < 200.0,
            String.format("Uncompressed size %.2f MB exceeds 200 MB", sizeMB));

        log.info("✓ Delta compression: {} snapshots, {:.2f} MB uncompressed",
                 snapshots.size(), sizeMB);
    }

    @Test
    void testServerGzipCompression() throws Exception {
        var config = createTestConfig();
        var result = engine.run(config);

        String json = objectMapper.writeValueAsString(result.data().snapshots());
        byte[] compressed = gzipCompress(json);

        double compressionRatio = (1.0 - (compressed.length / (double) json.length())) * 100;

        // GZIP should achieve 60%+ compression on delta JSON
        assertTrue(compressionRatio > 60.0,
            String.format("Compression ratio %.1f%% below 60%% threshold", compressionRatio));

        log.info("✓ GZIP compression: {:.1f}% reduction ({} MB → {} MB)",
                 compressionRatio,
                 json.length() / (1024.0 * 1024.0),
                 compressed.length / (1024.0 * 1024.0));
    }
}
```

### Frontend Unit Tests

**State Reconstruction:**

```typescript
describe('useSnapshotReconstruction', () => {
  it('should merge delta changes correctly', () => {
    const snapshots = [
      {
        type: 'KEYFRAME',
        timestamp: 0,
        robots: [
          { robotId: '0', status: 'IDLE', position: { x: 10, y: 20 } },
          { robotId: '1', status: 'IDLE', position: { x: 15, y: 25 } },
        ],
      },
      {
        type: 'DELTA',
        timestamp: 10,
        robots: [
          { robotId: '0', status: 'MOVING', position: { x: 10, y: 21 } }, // Changed
        ],
      },
    ]

    const { result } = renderHook(() => useSnapshotReconstruction(snapshots))

    // Frame 1 should have both robots, with robot 0 updated
    expect(result.current[1].robots).toHaveLength(2)
    expect(result.current[1].robots[0].status).toBe('MOVING')
    expect(result.current[1].robots[1].status).toBe('IDLE') // Unchanged
  })
})
```

## Rollout Plan

### Phase 1: Backend Implementation (Week 1)
- [ ] Create `WarehouseSnapshotV2` sealed interface and records
- [ ] Update `DesmojSnapshotCaptureProcess` with delta logic
- [ ] Add `keyframeIntervalSeconds` to `SimulationConfig`
- [ ] Write unit tests for delta computation
- [ ] Enable server GZIP in `application.yml`
- [ ] Update OpenAPI spec generation

### Phase 2: Integration Testing (Week 1-2)
- [ ] Run integration test with production config (4-hour simulation)
- [ ] Verify snapshot counts (should have ~240 keyframes, ~1,200 deltas)
- [ ] Measure uncompressed JSON size (target: <200 MB, expect ~138 MB)
- [ ] Measure GZIP compressed size (target: <100 MB, expect ~35-50 MB)
- [ ] Validate delta correctness (compare reconstructed with full snapshots)

### Phase 3: Frontend Implementation (Week 2)
- [ ] Regenerate TypeScript types from OpenAPI spec
- [ ] Implement `useSnapshotReconstruction` hook
- [ ] Update results page to use reconstructed snapshots
- [ ] Write tests for merge logic
- [ ] Manual testing with real simulation data
- [ ] Verify animation plays smoothly

### Phase 4: Production Deployment (Week 3)
- [ ] Deploy backend with delta compression enabled
- [ ] Run first production simulation
- [ ] Verify file sizes in browser Network tab
- [ ] Monitor for errors or rendering issues
- [ ] Gather user feedback on animation smoothness

## Success Metrics

### Before (Baseline)
- Snapshot size: 1,240 MB uncompressed
- Network transfer: ~800-900 MB (with basic server GZIP)
- Snapshot interval: 10s (1,440 snapshots for 4-hour simulation)

### After (Target)
- Uncompressed JSON: ~138 MB (89% reduction from delta compression)
- Network transfer: ~35-50 MB (96% total reduction)
- Snapshot interval: 10s (maintained)
- Keyframes: ~240 (every 60s)
- Deltas: ~1,200 (every 10s between keyframes)

### Validation Criteria
- ✅ Uncompressed JSON < 200 MB
- ✅ Network transfer < 100 MB
- ✅ GZIP compression ratio > 60%
- ✅ Animation plays smoothly (no visual glitches)
- ✅ Seek to any timestamp works correctly

## Future Optimizations (Not in Scope)

If file size is still problematic after this implementation:

1. **Adaptive keyframe interval** - More keyframes during high activity, fewer during quiet periods
2. **Sparse field encoding** - Omit unchanged fields within entities (more complex merging)
3. **Binary format (Protobuf)** - Would require 50-70% more reduction but adds complexity
4. **Snapshot streaming** - Return snapshots as they're generated (for very long simulations)

## References

- Current implementation: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`
- Animation controls: `apps/frontend/src/components/AnimationControls.tsx`
- Bloat analysis: `/tmp/analyze_json.py`, `/tmp/detailed_bloat_analysis.py`
