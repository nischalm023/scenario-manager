# Animation Playback Feature Design

**Date:** 2025-11-12
**Status:** Approved
**Primary Use Cases:** Debugging/validation, Performance analysis

## Overview

Add animation playback capability to visualize discrete event simulation results on the frontend. Capture warehouse state at 10-second intervals during simulation and provide timeline-based playback controls for post-simulation analysis.

## Requirements

- **Post-simulation playback only** - No real-time streaming required
- **Moderate detail level** - Robot positions/states, relay occupancy, station queues, tote/rack tracking, task assignments, basic metrics
- **Reuse existing WarehouseMap** - Overlay animation data on current visualization
- **Performance optimized** - Snapshot capture should not bottleneck simulation
- **Debugging focus** - Accuracy over polish, help identify bottlenecks and validate logic

## Data Model

### WarehouseSnapshot

Core snapshot structure capturing complete warehouse state at a point in time:

```java
public record WarehouseSnapshot(
    double timestamp,                    // Simulation time in seconds
    List<RobotState> robots,
    List<RelayState> relays,
    List<StationState> stations,
    List<ToteState> totes,
    List<StorageSpaceState> storageSpaces,
    SnapshotMetrics metrics
) {}
```

### RobotState

```java
public record RobotState(
    String robotId,
    RobotType type,                      // RTP, VTM, HTM
    RobotStatus status,                  // IDLE, MOVING, LOADING, etc.
    Position position,                   // x, y coordinates
    String currentTask,                  // Optional task description
    String carryingStorageSpace          // Rack ID or Tote ID
) {}
```

### RelayState

```java
public record RelayState(
    String relayId,
    int currentCapacity,
    int maxCapacity,
    List<String> toteIds                 // Totes currently at relay
) {}
```

### StationState

```java
public record StationState(
    int stationId,
    int queueLength,
    String currentTote,                  // Tote being processed
    double utilizationPercent            // Rolling utilization
) {}
```

### ToteState

```java
public record ToteState(
    String toteId,
    String location,                     // "RACK", "VTM_0", "RELAY_1", "HTM_2", "STATION_3"
    int taskCount                        // Tasks remaining in tote
) {}
```

### StorageSpaceState

Comprehensive tracking of racks and totes including inventory and reservations:

```java
public record StorageSpaceState(
    String storageSpaceId,               // "RACK_5_2" or "TOTE_42"
    StorageSpaceType type,               // RACK or TOTE
    Position position,                   // Current position (moves for RTP racks)
    boolean isActive,                    // In transit, at relay, or reserved
    Map<String, Integer> inventory,      // SKU -> total quantity in space
    int totalInventory,                  // Sum of inventory values (read-optimized)
    Map<String, Integer> reservedInventory,  // SKU -> reserved quantity
    int totalReservedInventory,          // Sum of reserved values (read-optimized)
    List<String> reservedByOrders        // Order IDs that reserved this space
) {}

public enum StorageSpaceType {
    RACK,    // RTP system or relay system rack
    TOTE     // Relay system tote
}
```

### SnapshotMetrics

```java
public record SnapshotMetrics(
    int completedPicks,
    int completedOrders,
    double currentPicksPerHour
) {}
```

## Backend Implementation

### Snapshot Capture Process

New Desmo-J process that periodically captures warehouse state:

```java
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
            hold(new TimeSpan(intervalSeconds, TimeUnit.SECONDS));

            try {
                var snapshot = captureSnapshot();
                snapshots.add(snapshot);
            } catch (SimFinishedException e) {
                // Simulation ended, exit gracefully
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
}
```

### Data Extraction Methods

- `captureRobotStates()` - Iterate through `model.getRobots()`, extract position, status, current task
- `captureRelayStates()` - Query `RelayPositionManager` for each relay's occupancy
- `captureStationStates()` - Query `StationManager` for queue lengths and utilization
- `captureToteStates()` - Query `TotePool` for tote locations and task counts
- `captureStorageSpaceStates()` - Query `WarehouseInventory` for inventory/reservations, `model.getActiveStorageSpaces()` for active status
- `captureMetrics()` - Read from metrics collectors (picks, orders, rates)

### Process Scheduling

In `DesmojWarehouseModel.doInitialSchedules()`:

```java
var snapshots = new CopyOnWriteArrayList<WarehouseSnapshot>();
this.snapshotProcess = new DesmojSnapshotCaptureProcess(this, 10.0, snapshots);
snapshotProcess.activate(new TimeInstant(0, TimeUnit.SECONDS));
```

### Performance Optimizations

1. **10-second intervals** - Reduces snapshot count by 50% vs 5 seconds
2. **Shallow copies** - Capture current values, not deep clones of complex objects
3. **Thread-safe collection** - `CopyOnWriteArrayList` prevents blocking simulation
4. **Lazy computation** - Only compute what's needed (e.g., skip empty relays)
5. **Configuration flag** - Disable capture for production/benchmarking

**Configuration:**
```java
// In DesmojSimulationEngine
var captureInterval = config.getSnapshotIntervalSeconds().orElse(10.0);
var enableSnapshots = config.isEnableSnapshots(); // NEW config flag

if (enableSnapshots) {
    var snapshots = new CopyOnWriteArrayList<WarehouseSnapshot>();
    model.setSnapshotList(snapshots);
    var captureProcess = new DesmojSnapshotCaptureProcess(model, captureInterval, snapshots);
    captureProcess.activate(new TimeInstant(0, TimeUnit.SECONDS));
}
```

**Efficient capture example:**
```java
private List<RobotState> captureRobotStates() {
    return model.getRobots().stream()
        .map(robot -> new RobotState(
            robot.getId(),
            robot.getType(),
            robot.getCurrentStatus(),     // Direct field access, no computation
            robot.getCurrentPosition(),   // Reference to Position record
            robot.getCurrentTaskId(),
            robot.getCarryingStorageSpace()
        ))
        .toList();
}
```

**Expected overhead:** For a 1-hour simulation at 10s intervals: 360 snapshots, ~1-2MB JSON. Negligible performance impact.

## API Changes

### SimulationResult DTO Update

```java
public record SimulationResult(
    SimulationConfig config,
    double simulationDuration,
    TaskMetric taskMetrics,
    RobotMetric robotMetrics,
    StationMetric stationMetrics,
    StorageSpaceMetric storageSpaceMetrics,
    List<WarehouseSnapshot> snapshots,      // NEW
    Map<String, Object> metadata
) {}
```

### Engine Integration

In `DesmojSimulationEngine.run()`:

```java
// After simulation completes
var snapshots = config.isEnableSnapshots()
    ? model.getSnapshotList()
    : List.<WarehouseSnapshot>of();

return new SimulationResult(
    config,
    actualDuration,
    taskMetrics,
    robotMetrics,
    stationMetrics,
    storageSpaceMetrics,
    snapshots,           // Pass captured snapshots
    metadata
);
```

### API Response Example

```json
{
  "config": {...},
  "simulationDuration": 3600.0,
  "taskMetrics": {...},
  "snapshots": [
    {
      "timestamp": 0.0,
      "robots": [...],
      "relays": [...],
      "stations": [...],
      "totes": [...],
      "storageSpaces": [...],
      "metrics": {...}
    },
    {
      "timestamp": 10.0,
      ...
    }
  ]
}
```

Response is already camelCase via Jackson. Snapshots array is empty if `enableSnapshots: false` in config.

## Frontend Implementation

### Playback Controls Component

New component: `AnimationControls.tsx`

**State management:**
```typescript
const [currentFrame, setCurrentFrame] = useState(0);
const [isPlaying, setIsPlaying] = useState(false);
const [playbackSpeed, setPlaybackSpeed] = useState(1.0); // 0.5x, 1x, 2x, 5x

const snapshots = simulationResult.snapshots;
const totalFrames = snapshots.length;
```

**Playback loop:**
```typescript
useEffect(() => {
  if (!isPlaying || currentFrame >= totalFrames - 1) return;

  const intervalMs = 1000 / playbackSpeed; // Adjust for speed
  const timer = setInterval(() => {
    setCurrentFrame(prev => {
      if (prev >= totalFrames - 1) {
        setIsPlaying(false);
        return prev;
      }
      return prev + 1;
    });
  }, intervalMs);

  return () => clearInterval(timer);
}, [isPlaying, currentFrame, playbackSpeed, totalFrames]);
```

**UI Components (using shadcn/ui):**
- **Play/Pause button** - Toggle playback
- **Timeline slider** - Scrub to any frame (shadcn Slider component)
- **Speed selector** - Dropdown with 0.5x, 1x, 2x, 5x options
- **Frame counter** - "Frame 42 / 360 (420s simulation time)"
- **Reset button** - Jump to frame 0

**Layout:** Horizontal bar below WarehouseMap with controls centered.

### WarehouseMap Integration

Pass current snapshot to the WarehouseMap component:

```typescript
<WarehouseMap
  config={simulationResult.config}
  snapshot={snapshots[currentFrame]}  // NEW prop
/>
```

### Visual Overlays

**1. Robots** - Render as colored dots/icons on the map
- Color by type: VTM (blue), HTM (green), RTP (orange)
- Status indicator: Idle (hollow), Moving (solid), Loading (pulsing)
- Show carrying indicator if `carryingStorageSpace` is set
- Position from `robot.position.x, robot.position.y`

**2. Storage Spaces (Racks/Totes)**
- Render at grid positions
- Color-code by status:
  - Active (yellow/orange border)
  - Reserved (blue border)
  - Depleted (gray, if `totalInventory === 0`)
- Show inventory count badge: `totalInventory (totalReservedInventory reserved)`

**3. Relay Positions**
- Render at fixed positions on map
- Fill indicator: `currentCapacity / maxCapacity`
- Highlight blocked relays (capacity full)
- Show tote IDs in tooltip

**4. Stations**
- Render at fixed positions
- Queue length indicator (vertical bars or number badge)
- Highlight active station if `currentTote` present
- Utilization color gradient (green → yellow → red)

### Performance Optimization

- Use Canvas API for robots (hundreds of moving objects)
- Use SVG for static elements (relays, stations)
- Memoize WarehouseMap to prevent re-renders when only frame changes

## Testing Strategy

### Backend Tests

**1. Unit test: `SnapshotCaptureProcessTest.java`**
- Mock warehouse model with known state
- Run capture process for 30 simulated seconds
- Verify 3 snapshots captured (at 0s, 10s, 20s)
- Assert snapshot data matches model state

**2. Integration test: `DesmojIntegrationTest` extension**
- Run full simulation with `enableSnapshots: true`
- Verify snapshots list is non-empty
- Verify first snapshot timestamp = 0
- Verify last snapshot timestamp ≈ simulation duration
- Spot-check: robot count matches config, storage space count matches inventory

### Frontend Tests

**1. Component test: `AnimationControls.test.tsx`**
- Test play/pause toggle
- Test scrubbing to specific frame
- Test speed changes
- Test auto-stop at end of timeline

**2. Integration test: `WarehouseMap.test.tsx`**
- Render with snapshot data
- Verify robots rendered at correct positions
- Verify relay occupancy displayed
- Verify station queues shown

### Manual Testing

1. Run relay system simulation (1 hour, 10 VTMs, 5 HTMs)
2. Open frontend, load results
3. Verify snapshots loaded (check console)
4. Play animation, observe:
   - Robots moving between positions
   - Relays filling/emptying
   - Station queues growing/shrinking
   - Storage spaces depleting
5. Test scrubbing to bottleneck moments
6. Test different playback speeds

## Benefits

- **Debugging** - Visually verify simulation logic, spot incorrect behavior
- **Bottleneck identification** - Scrub to moments when queues grow or relays block
- **Set cover validation** - Watch which storage spaces get reserved by orders
- **Hotspot analysis** - See which racks/relays are accessed most frequently
- **Stakeholder communication** - Demonstrate warehouse behavior visually

## Future Enhancements (Out of Scope)

- Event-driven snapshots (capture on specific events, not just time intervals)
- Live streaming during long simulations
- Export animation to video file
- Heatmaps for robot density and storage space access frequency
- Delta encoding for smaller data size
