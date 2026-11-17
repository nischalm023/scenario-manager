# Service Client Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire fleet-simulator to use layout-service and navigation-service instead of local WarehouseMap and PathFinder.

**Architecture:** Create adapter classes that wrap gRPC responses to maintain compatibility with existing consumers. Replace local PathFinder with NavigationServiceClient calls. Session lifecycle managed in DesmojSimulationEngine.

**Tech Stack:** gRPC, Spring Boot, Java 21

---

## Overview

### Current State
- Fleet-simulator creates WarehouseMap locally in `WarehouseBuilder`
- Fleet-simulator creates PathFinder locally in `DesmojWarehouseModel.init()`
- Robot.planPath() calls PathFinder.findPath() with WarehouseMap

### Target State
- Fleet-simulator calls LayoutServiceClient.getLayout() to get map data
- Fleet-simulator creates navigation session at simulation start
- Robot.planPath() calls NavigationServiceClient.findPath() with session ID
- Navigation session deleted at simulation end

### Integration Strategy
1. Create `LayoutAdapter` to wrap `LayoutResponse` with WarehouseMap-like interface
2. Replace PathFinder with NavigationServiceClient in DesmojWarehouseModel
3. Manage navigation session lifecycle in DesmojSimulationEngine
4. Remove duplicate domain code from fleet-simulator

---

## Phase 1: Layout Adapter

### Task 1.1: Create LayoutAdapter class

**Files:**
- Create: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/adapter/LayoutAdapter.java`

**Step 1: Create the adapter**

```java
package com.greyorange.dessimulator.adapter;

import com.greyorange.dessimulator.domain.common.coordinates.StationGridPosition;
import com.greyorange.dessimulator.domain.map.RackBlockGridDefinition;
import com.greyorange.dessimulator.domain.map.RackPositionMapData;
import com.greyorange.dessimulator.domain.map.StationEdge;
import com.greyorange.grpc.layout.v1.LayoutResponse;
import com.greyorange.grpc.layout.v1.RackBlockDefinition;
import com.greyorange.grpc.layout.v1.RackPositionData;
import com.greyorange.grpc.layout.v1.RelayPositionData;
import com.greyorange.grpc.layout.v1.StationPosition;
import com.greyorange.greymatter.common.coordinates.GridCoordinate;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;
import lombok.Getter;

/**
 * Adapter that wraps LayoutResponse to provide WarehouseMap-like interface.
 *
 * <p>Converts proto types to domain types for compatibility with existing consumers.
 * Does not perform layout computation - that happens in layout-service.
 */
public class LayoutAdapter {

  @Getter private final int gridWidth;
  @Getter private final int gridDepth;
  @Getter private final double cellSizeMeters;

  private final List<StationGridPosition> stationPositions;
  private final List<GridCoordinate> rackPositions;
  private final List<RackBlockGridDefinition> rackBlocks;
  private final Map<GridCoordinate, RackPositionMapData> rackPositionData;
  private final Set<GridCoordinate> occupiedPositions;
  private final Set<GridCoordinate> highwayPositions;
  private final List<RelayPositionData> relayPositionDataList;

  public LayoutAdapter(LayoutResponse response) {
    this.gridWidth = response.getBounds().getWidth();
    this.gridDepth = response.getBounds().getDepth();
    this.cellSizeMeters = response.getBounds().getCellSizeMeters();

    // Convert stations
    this.stationPositions = response.getStationsList().stream()
        .map(this::convertStation)
        .toList();

    // Convert rack blocks and collect rack positions
    this.rackBlocks = response.getRackBlocksList().stream()
        .map(this::convertRackBlock)
        .toList();

    this.rackPositions = rackBlocks.stream()
        .flatMap(block -> block.rackPositions().stream())
        .toList();

    // Convert rack position data
    this.rackPositionData = new HashMap<>();
    for (Map.Entry<String, RackPositionData> entry : response.getRackPositionDataMap().entrySet()) {
      GridCoordinate pos = convertCoordinate(entry.getValue().getPosition());
      rackPositionData.put(pos, convertRackPositionData(entry.getValue()));
    }

    // Build occupied positions set
    this.occupiedPositions = new HashSet<>();
    for (RackBlockGridDefinition block : rackBlocks) {
      occupiedPositions.addAll(block.rackPositions());
      occupiedPositions.addAll(block.aislePositions());
    }

    // Build highway positions set
    this.highwayPositions = new HashSet<>();
    for (var highway : response.getHighwaysList()) {
      // Add all positions in the highway segment
      GridCoordinate start = convertCoordinate(highway.getStart());
      GridCoordinate end = convertCoordinate(highway.getEnd());
      addLinePositions(highwayPositions, start, end);
    }

    // Store relay positions for relay system
    this.relayPositionDataList = response.getRelayPositionsList();
  }

  public List<StationGridPosition> getStationPositions() {
    return stationPositions;
  }

  public List<GridCoordinate> getRackPositions() {
    return rackPositions;
  }

  public List<RackBlockGridDefinition> getRackBlocks() {
    return rackBlocks;
  }

  public Map<GridCoordinate, RackPositionMapData> getRackPositionData() {
    return rackPositionData;
  }

  public Set<GridCoordinate> getOccupiedPositions() {
    return occupiedPositions;
  }

  public boolean isPositionAvailable(GridCoordinate pos) {
    return !occupiedPositions.contains(pos);
  }

  public boolean isRackPosition(GridCoordinate pos) {
    return rackPositionData.containsKey(pos);
  }

  public boolean isHighway(GridCoordinate pos) {
    return highwayPositions.contains(pos);
  }

  public List<RelayPositionData> getRelayPositionDataList() {
    return relayPositionDataList;
  }

  private StationGridPosition convertStation(StationPosition proto) {
    return new StationGridPosition(
        convertCoordinate(proto.getPosition()),
        convertStationEdge(proto.getEdge()),
        convertCoordinate(proto.getAccessibilityDirection()));
  }

  private GridCoordinate convertCoordinate(
      com.greyorange.grpc.common.v1.GridCoordinate proto) {
    return new GridCoordinate(proto.getRow(), proto.getCol());
  }

  private StationEdge convertStationEdge(
      com.greyorange.grpc.layout.v1.StationEdge proto) {
    return switch (proto) {
      case STATION_EDGE_BOTTOM -> StationEdge.BOTTOM;
      case STATION_EDGE_TOP -> StationEdge.TOP;
      case STATION_EDGE_LEFT -> StationEdge.LEFT;
      case STATION_EDGE_RIGHT -> StationEdge.RIGHT;
      default -> StationEdge.BOTTOM;
    };
  }

  private RackBlockGridDefinition convertRackBlock(RackBlockDefinition proto) {
    List<GridCoordinate> rackPos = proto.getRackPositionsList().stream()
        .map(this::convertCoordinate)
        .toList();
    List<GridCoordinate> aislePos = proto.getAislePositionsList().stream()
        .map(this::convertCoordinate)
        .toList();
    return new RackBlockGridDefinition(
        proto.getBlockId(),
        convertCoordinate(proto.getTopLeft()),
        convertCoordinate(proto.getBottomRight()),
        proto.getRowsPerBlock(),
        rackPos,
        aislePos);
  }

  private RackPositionMapData convertRackPositionData(RackPositionData proto) {
    return new RackPositionMapData(
        convertCoordinate(proto.getPosition()),
        convertCoordinate(proto.getVtmAccessPosition()),
        proto.getRackBlockId(),
        proto.getSide());
  }

  private void addLinePositions(Set<GridCoordinate> positions,
      GridCoordinate start, GridCoordinate end) {
    int minRow = Math.min(start.row(), end.row());
    int maxRow = Math.max(start.row(), end.row());
    int minCol = Math.min(start.col(), end.col());
    int maxCol = Math.max(start.col(), end.col());

    for (int row = minRow; row <= maxRow; row++) {
      for (int col = minCol; col <= maxCol; col++) {
        positions.add(new GridCoordinate(row, col));
      }
    }
  }
}
```

**Step 2: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/adapter/
git commit -m "feat(fleet-simulator): add LayoutAdapter to wrap layout-service responses"
```

---

### Task 1.2: Create RequestConverter utility

**Files:**
- Create: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/adapter/LayoutRequestConverter.java`

**Step 1: Create converter for building LayoutRequest from SimulationConfig**

```java
package com.greyorange.dessimulator.adapter;

import com.greyorange.dessimulator.api.simulation.SimulationConfigDTO;
import com.greyorange.dessimulator.domain.map.WarehouseMapConfig;
import com.greyorange.grpc.common.v1.SystemType;
import com.greyorange.grpc.layout.v1.HighwayConfig;
import com.greyorange.grpc.layout.v1.LayoutRequest;
import com.greyorange.grpc.layout.v1.RackBlockConfig;
import com.greyorange.grpc.layout.v1.StationEdge;
import com.greyorange.grpc.layout.v1.StationEdgeConfig;
import com.greyorange.grpc.layout.v1.StationLayoutConfig;

/**
 * Converts simulation configuration to layout service request.
 */
public final class LayoutRequestConverter {

  private LayoutRequestConverter() {}

  /**
   * Build LayoutRequest from WarehouseMapConfig.
   */
  public static LayoutRequest toLayoutRequest(
      com.greyorange.greymatter.common.types.SystemType systemType,
      WarehouseMapConfig mapConfig) {

    return LayoutRequest.newBuilder()
        .setGridWidth(mapConfig.getGridWidth())
        .setGridDepth(mapConfig.getGridDepth())
        .setCellSizeMeters(mapConfig.getCellSizeMeters())
        .setSystemType(convertSystemType(systemType))
        .setStationLayout(convertStationLayout(mapConfig))
        .setHighwayConfig(convertHighwayConfig(mapConfig))
        .setRackBlockConfig(convertRackBlockConfig(mapConfig))
        .build();
  }

  private static SystemType convertSystemType(
      com.greyorange.greymatter.common.types.SystemType type) {
    return switch (type) {
      case RTP -> SystemType.SYSTEM_TYPE_RTP;
      case RELAY -> SystemType.SYSTEM_TYPE_RELAY;
    };
  }

  private static StationLayoutConfig convertStationLayout(WarehouseMapConfig config) {
    var stationLayout = config.getStationLayout();
    var builder = StationLayoutConfig.newBuilder();

    for (var edgeConfig : stationLayout.getStationEdges()) {
      builder.addStationEdges(StationEdgeConfig.newBuilder()
          .setEdge(convertStationEdge(edgeConfig.edge()))
          .setStationCount(edgeConfig.stationCount())
          .setSpacingGridUnits(edgeConfig.spacingGridUnits())
          .build());
    }

    return builder.build();
  }

  private static StationEdge convertStationEdge(
      com.greyorange.dessimulator.domain.map.StationEdge edge) {
    return switch (edge) {
      case BOTTOM -> StationEdge.STATION_EDGE_BOTTOM;
      case TOP -> StationEdge.STATION_EDGE_TOP;
      case LEFT -> StationEdge.STATION_EDGE_LEFT;
      case RIGHT -> StationEdge.STATION_EDGE_RIGHT;
    };
  }

  private static HighwayConfig convertHighwayConfig(WarehouseMapConfig config) {
    return HighwayConfig.newBuilder()
        .setHighwaysPerEdge(config.getHighwayConfig().highwaysPerEdge())
        .build();
  }

  private static RackBlockConfig convertRackBlockConfig(WarehouseMapConfig config) {
    var rackBlock = config.getRackBlockConfig();
    return RackBlockConfig.newBuilder()
        .setRowsPerBlock(rackBlock.rowsPerBlock())
        .setRackColumns(rackBlock.rackColumns())
        .setCrossAisleWidthGridUnits(rackBlock.crossAisleWidthGridUnits())
        .build();
  }
}
```

**Step 2: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

**Step 3: Commit**

```bash
git add apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/adapter/LayoutRequestConverter.java
git commit -m "feat(fleet-simulator): add LayoutRequestConverter for building layout requests"
```

---

## Phase 2: Navigation Integration

### Task 2.1: Create NavigationAdapter for path result conversion

**Files:**
- Create: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/adapter/NavigationAdapter.java`

**Step 1: Create adapter for converting proto to domain PathResult**

```java
package com.greyorange.dessimulator.adapter;

import com.greyorange.dessimulator.domain.navigation.NavigationAction;
import com.greyorange.dessimulator.domain.navigation.PathResult;
import com.greyorange.dessimulator.domain.navigation.PathWaypoint;
import com.greyorange.greymatter.common.coordinates.GridCoordinate;
import com.greyorange.greymatter.common.navigation.Orientation;
import com.greyorange.greymatter.common.robot.RobotType;
import com.greyorange.grpc.navigation.v1.CreateSessionRequest;
import com.greyorange.grpc.navigation.v1.FindPathRequest;
import com.greyorange.grpc.navigation.v1.FindPathResponse;
import com.greyorange.grpc.navigation.v1.GridBounds;
import java.util.List;
import java.util.Set;

/**
 * Adapter for converting between navigation proto types and domain types.
 */
public final class NavigationAdapter {

  private NavigationAdapter() {}

  /**
   * Convert FindPathResponse to domain PathResult.
   */
  public static PathResult toPathResult(FindPathResponse response) {
    if (!response.getPathFound() || response.getWaypointsList().isEmpty()) {
      return PathResult.empty();
    }

    List<PathWaypoint> waypoints = response.getWaypointsList().stream()
        .map(NavigationAdapter::convertWaypoint)
        .toList();

    return new PathResult(
        waypoints,
        response.getTotalTime(),
        response.getTotalDistance(),
        response.getTotalTurns());
  }

  /**
   * Build CreateSessionRequest from navigation data.
   */
  public static CreateSessionRequest toCreateSessionRequest(
      int gridWidth,
      int gridDepth,
      Set<GridCoordinate> blockedPositions,
      Set<GridCoordinate> rackPositions,
      Set<GridCoordinate> highwayPositions,
      Set<GridCoordinate> relayPositions) {

    var builder = CreateSessionRequest.newBuilder()
        .setBounds(GridBounds.newBuilder()
            .setWidth(gridWidth)
            .setDepth(gridDepth)
            .build());

    for (GridCoordinate pos : blockedPositions) {
      builder.addBlockedPositions(toProtoCoordinate(pos));
    }
    for (GridCoordinate pos : rackPositions) {
      builder.addRackPositions(toProtoCoordinate(pos));
    }
    for (GridCoordinate pos : highwayPositions) {
      builder.addHighwayPositions(toProtoCoordinate(pos));
    }
    for (GridCoordinate pos : relayPositions) {
      builder.addRelayPositions(toProtoCoordinate(pos));
    }

    return builder.build();
  }

  /**
   * Build FindPathRequest.
   */
  public static FindPathRequest toFindPathRequest(
      String sessionId,
      GridCoordinate start,
      GridCoordinate goal,
      Orientation startOrientation,
      RobotType robotType,
      boolean isCarryingCargo,
      double robotSpeed,
      double baseTurnTime90) {

    return FindPathRequest.newBuilder()
        .setSessionId(sessionId)
        .setStart(toProtoCoordinate(start))
        .setGoal(toProtoCoordinate(goal))
        .setStartOrientation(toProtoOrientation(startOrientation))
        .setRobotType(toProtoRobotType(robotType))
        .setIsCarryingCargo(isCarryingCargo)
        .setRobotSpeed(robotSpeed)
        .setBaseTurnTime90(baseTurnTime90)
        .build();
  }

  private static PathWaypoint convertWaypoint(
      com.greyorange.grpc.navigation.v1.PathWaypoint proto) {
    return new PathWaypoint(
        new GridCoordinate(proto.getPosition().getRow(), proto.getPosition().getCol()),
        convertOrientation(proto.getOrientation()),
        convertAction(proto.getAction()),
        proto.getTimeDelta(),
        proto.getCumulativeTime());
  }

  private static Orientation convertOrientation(
      com.greyorange.grpc.navigation.v1.Orientation proto) {
    return switch (proto) {
      case ORIENTATION_NORTH -> Orientation.NORTH;
      case ORIENTATION_SOUTH -> Orientation.SOUTH;
      case ORIENTATION_EAST -> Orientation.EAST;
      case ORIENTATION_WEST -> Orientation.WEST;
      default -> Orientation.NORTH;
    };
  }

  private static NavigationAction convertAction(
      com.greyorange.grpc.navigation.v1.NavigationAction proto) {
    return switch (proto) {
      case NAVIGATION_ACTION_MOVE_FORWARD -> NavigationAction.MOVE_FORWARD;
      case NAVIGATION_ACTION_MOVE_BACKWARD -> NavigationAction.MOVE_BACKWARD;
      case NAVIGATION_ACTION_TURN_90_CW -> NavigationAction.TURN_90_CW;
      case NAVIGATION_ACTION_TURN_90_CCW -> NavigationAction.TURN_90_CCW;
      case NAVIGATION_ACTION_TURN_180 -> NavigationAction.TURN_180;
      default -> NavigationAction.MOVE_FORWARD;
    };
  }

  private static com.greyorange.grpc.common.v1.GridCoordinate toProtoCoordinate(
      GridCoordinate pos) {
    return com.greyorange.grpc.common.v1.GridCoordinate.newBuilder()
        .setRow(pos.row())
        .setCol(pos.col())
        .build();
  }

  private static com.greyorange.grpc.navigation.v1.Orientation toProtoOrientation(
      Orientation orientation) {
    return switch (orientation) {
      case NORTH -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_NORTH;
      case SOUTH -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_SOUTH;
      case EAST -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_EAST;
      case WEST -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_WEST;
    };
  }

  private static com.greyorange.grpc.navigation.v1.RobotType toProtoRobotType(
      RobotType robotType) {
    return switch (robotType) {
      case RTP -> com.greyorange.grpc.navigation.v1.RobotType.ROBOT_TYPE_RTP;
      case VTM -> com.greyorange.grpc.navigation.v1.RobotType.ROBOT_TYPE_VTM;
      case HTM -> com.greyorange.grpc.navigation.v1.RobotType.ROBOT_TYPE_HTM;
    };
  }
}
```

**Step 2: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

**Step 3: Commit**

```bash
git add apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/adapter/NavigationAdapter.java
git commit -m "feat(fleet-simulator): add NavigationAdapter for path result conversion"
```

---

### Task 2.2: Update WarehouseBuilder to use LayoutServiceClient

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Add LayoutServiceClient dependency and LayoutAdapter field**

Add imports and field:
```java
import com.greyorange.dessimulator.adapter.LayoutAdapter;
import com.greyorange.dessimulator.adapter.LayoutRequestConverter;
import com.greyorange.dessimulator.adapter.LayoutServiceClient;
```

Add field alongside existing fields (around line 60):
```java
private final LayoutServiceClient layoutServiceClient;
private LayoutAdapter layoutAdapter;
```

Update constructor to accept LayoutServiceClient:
```java
public WarehouseBuilder(
    SimulationConfig config,
    PutServiceClient putServiceClient,
    LayoutServiceClient layoutServiceClient) {
  this.config = config;
  this.putServiceClient = putServiceClient;
  this.layoutServiceClient = layoutServiceClient;
}
```

**Step 2: Update buildMap() to use layout-service**

Replace the buildMap() method:
```java
/** Build warehouse map using layout-service. */
private void buildMap() {
  var request = LayoutRequestConverter.toLayoutRequest(
      config.systemType(), config.mapConfig());

  var response = layoutServiceClient.getLayout(request);
  this.layoutAdapter = new LayoutAdapter(response);

  // Create local WarehouseMap for components that still need it
  // TODO: Remove once all consumers use LayoutAdapter
  map = new WarehouseMap(config.systemType(), config.mapConfig());

  log.info(
      "Layout received from layout-service: {}x{}",
      layoutAdapter.getGridWidth(),
      layoutAdapter.getGridDepth());
}
```

**Step 3: Add getter for LayoutAdapter**

```java
public LayoutAdapter getLayoutAdapter() {
  return layoutAdapter;
}
```

**Step 4: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

**Step 5: Update callers of WarehouseBuilder constructor**

Search for `new WarehouseBuilder` and update to pass LayoutServiceClient.

**Step 6: Commit**

```bash
git add -A
git commit -m "feat(fleet-simulator): update WarehouseBuilder to use layout-service"
```

---

### Task 2.3: Update DesmojWarehouseModel to use NavigationServiceClient

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Replace PathFinder with NavigationServiceClient**

Add imports:
```java
import com.greyorange.dessimulator.adapter.NavigationAdapter;
import com.greyorange.dessimulator.adapter.NavigationServiceClient;
```

Replace PathFinder field with:
```java
@Getter private NavigationServiceClient navigationClient;
@Getter private String navigationSessionId;
```

Update constructor to accept NavigationServiceClient:
```java
public DesmojWarehouseModel(
    Model owner,
    String name,
    boolean showInReport,
    boolean showInTrace,
    SimulationConfig config,
    WarehouseComponents warehouseComponents,
    OrderProcessingRuntime orderProcessingRuntime,
    MetricEventPublisher metricPublisher,
    NavigationServiceClient navigationClient) {
  super(owner, name, showInReport, showInTrace);
  // ... existing initialization
  this.navigationClient = navigationClient;
}
```

**Step 2: Update init() to create navigation session**

Replace PathFinder creation in init():
```java
@Override
public void init() {
  log.info("Initializing Desmo-J warehouse model...");

  // Create navigation session
  if (navigationClient != null) {
    var layoutAdapter = warehouseComponents.layoutAdapter();
    var request = NavigationAdapter.toCreateSessionRequest(
        layoutAdapter.getGridWidth(),
        layoutAdapter.getGridDepth(),
        layoutAdapter.getOccupiedPositions(),
        new HashSet<>(layoutAdapter.getRackPositions()),
        layoutAdapter.getHighwayPositions(),
        Set.of()); // relay positions if needed

    this.navigationSessionId = navigationClient.createSession(request);
    log.info("Created navigation session: {}", navigationSessionId);
  }

  // ... rest of init() unchanged
}
```

**Step 3: Add method for pathfinding via service**

```java
/**
 * Find path using navigation service.
 */
public PathResult findPath(
    GridCoordinate start,
    Orientation startOrientation,
    GridCoordinate goal,
    RobotType robotType,
    boolean isCarryingCargo,
    double robotSpeed,
    double baseTurnTime90) {

  var request = NavigationAdapter.toFindPathRequest(
      navigationSessionId,
      start,
      goal,
      startOrientation,
      robotType,
      isCarryingCargo,
      robotSpeed,
      baseTurnTime90);

  var response = navigationClient.findPath(request);
  return NavigationAdapter.toPathResult(response);
}
```

**Step 4: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

**Step 5: Commit**

```bash
git add -A
git commit -m "feat(fleet-simulator): update DesmojWarehouseModel to use navigation-service"
```

---

### Task 2.4: Update Robot.planPath() to use model's navigation

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/robot/Robot.java`

**Step 1: Update planPath signature and implementation**

Replace existing planPath method:
```java
/**
 * Plan a path from current position/orientation to destination.
 *
 * @param destination Goal position
 * @param model DesmojWarehouseModel for navigation service access
 * @param isCarryingCargo Whether robot is carrying cargo
 * @return PathResult with waypoints, or empty if unreachable
 */
public PathResult planPath(
    GridCoordinate destination,
    DesmojWarehouseModel model,
    boolean isCarryingCargo) {
  return model.findPath(
      this.currentPosition,
      this.currentOrientation,
      destination,
      this.robotType,
      isCarryingCargo,
      this.speed,
      this.baseTurnTime90);
}
```

**Step 2: Update all callers of planPath()**

Find and update in:
- `DesmojRtpRobotProcess.java`
- `DesmojVtmProcess.java`
- `DesmojHtmProcess.java`

Old pattern:
```java
robot.planPath(destination, pathFinder, model.getWarehouseComponents().map(), isCarrying);
```

New pattern:
```java
robot.planPath(destination, model, isCarrying);
```

**Step 3: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

**Step 4: Commit**

```bash
git add -A
git commit -m "refactor(fleet-simulator): update Robot.planPath to use navigation service"
```

---

## Phase 3: Cleanup and Wire Everything

### Task 3.1: Update WarehouseComponents to include LayoutAdapter

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/WarehouseComponents.java`

**Step 1: Add layoutAdapter field to record**

Add to WarehouseComponents record:
```java
LayoutAdapter layoutAdapter,
```

**Step 2: Update WarehouseBuilder to pass layoutAdapter**

In WarehouseBuilder.build(), add layoutAdapter to the components builder.

**Step 3: Commit**

```bash
git add -A
git commit -m "feat(fleet-simulator): add LayoutAdapter to WarehouseComponents"
```

---

### Task 3.2: Update DesmojSimulationEngine for session lifecycle

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Add NavigationServiceClient to engine**

Add field and constructor parameter:
```java
private final NavigationServiceClient navigationClient;
```

**Step 2: Pass to DesmojWarehouseModel**

Update model creation to pass navigationClient.

**Step 3: Add cleanup in run() finally block**

```java
try {
  // ... existing simulation code
} finally {
  // Cleanup navigation session
  if (navigationClient != null && model != null && model.getNavigationSessionId() != null) {
    navigationClient.deleteSession(model.getNavigationSessionId());
  }
}
```

**Step 4: Commit**

```bash
git add -A
git commit -m "feat(fleet-simulator): manage navigation session lifecycle in simulation engine"
```

---

### Task 3.3: Update SimulationService to inject clients

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/api/simulation/SimulationService.java`

**Step 1: Inject LayoutServiceClient and NavigationServiceClient**

Add constructor dependencies:
```java
private final LayoutServiceClient layoutServiceClient;
private final NavigationServiceClient navigationClient;
```

**Step 2: Pass to WarehouseBuilder and DesmojSimulationEngine**

Update construction calls.

**Step 3: Commit**

```bash
git add -A
git commit -m "feat(fleet-simulator): inject layout and navigation clients in SimulationService"
```

---

## Phase 4: Remove Duplicate Code

### Task 4.1: Remove domain/map from fleet-simulator

**Files:**
- Delete: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/*.java`

**Step 1: Verify no remaining usages**

```bash
grep -r "import com.greyorange.dessimulator.domain.map" apps/fleet-simulator/src/main/java/ | grep -v "StationEdge"
```

Should return minimal results (only types still needed).

**Step 2: Delete files that are fully migrated**

Keep only types that are still used internally and not available from layout-service responses.

**Step 3: Commit**

```bash
git add -A
git commit -m "refactor(fleet-simulator): remove migrated map domain code"
```

---

### Task 4.2: Remove domain/navigation/PathFinder from fleet-simulator

**Files:**
- Delete: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathFinder.java`

**Step 1: Verify PathFinder is not used**

```bash
grep -r "PathFinder" apps/fleet-simulator/src/main/java/
```

Should return no results after Task 2.4.

**Step 2: Delete PathFinder.java**

```bash
rm apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathFinder.java
```

**Step 3: Commit**

```bash
git add -A
git commit -m "refactor(fleet-simulator): remove PathFinder (now in navigation-service)"
```

---

## Verification Checklist

After completing all phases, verify:

1. [ ] `cd apps/fleet-simulator && mvn clean compile` - Compiles
2. [ ] `cd apps/fleet-simulator && mvn test` - Tests pass
3. [ ] Start all services and run a simulation end-to-end
4. [ ] Verify same simulation results as before migration

---

## Rollback Plan

If issues arise:

1. Revert commits in reverse order
2. Services can fall back to local code by keeping WarehouseMap/PathFinder temporarily
3. Add feature flag to switch between local and service modes if needed
