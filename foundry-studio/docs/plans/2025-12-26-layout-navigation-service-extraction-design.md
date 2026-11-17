# Layout and Navigation Service Extraction Design

**Date**: 2025-12-26
**Status**: Draft
**Authors**: Design session with Claude

## Overview

Extract Layout (map generation) and Navigation (pathfinding) into separate microservices from fleet-simulator. This enables independent deployment and team ownership as part of a larger organizational effort.

## Motivation

- **Independent deployment**: Multiple teams will own different services
- **Team autonomy**: Layout, Navigation, and Fleet teams can work independently
- **Scalability**: Services can be scaled based on their specific needs
- **Consistency**: Ensures all consumers use the same layout/navigation logic

## Architecture

### Service Topology

```
┌─────────────────┐     1. GetLayout()       ┌─────────────────┐
│ Fleet-Simulator │ ──────────────────────▶  │  Layout Service │
│  (Orchestrator) │ ◀──────────────────────  │   (port 9091)   │
└────────┬────────┘     LayoutResponse       └─────────────────┘
         │
         │ 2. CreateSession(navigationData)
         │    returns sessionId
         │
         │ 3. FindPath(sessionId, start, goal, ...)
         │    (called many times during simulation)
         ▼
┌─────────────────┐
│   Navigation    │
│    Service      │
│   (port 9092)   │
└─────────────────┘
```

### Services Summary

| Service | Purpose | Tech | Ports |
|---------|---------|------|-------|
| **fleet-simulator** | Robot orchestration, simulation engine | Java 21, Spring Boot | 8080 (REST) |
| **layout-service** | Map generation from config | Java 21, Spring Boot | 8082 (REST), 9091 (gRPC) |
| **navigation-service** | A* pathfinding with session cache | Java 21, Spring Boot | 8083 (REST), 9092 (gRPC) |
| **put-service** | Storage position selection | Java 21, Spring Boot | 8081 (REST), 9090 (gRPC) |

### Design Pattern: Orchestration

Fleet-simulator acts as the **orchestrator** - it controls the workflow and tells services what to do. This is preferred over choreography (services reacting to events) for:

- Clear control flow, easier to reason about
- Simpler debugging (one place to trace)
- Explicit dependencies

## Layout Service

### Responsibility

Generate warehouse map topology from configuration. Stateless computation - same config always produces same map.

### API

```protobuf
service LayoutService {
  // Full layout for simulation setup
  rpc GetLayout(LayoutRequest) returns (LayoutResponse);

  // Minimal data for pathfinding (coordinate sets only)
  rpc GetNavigationData(LayoutRequest) returns (NavigationDataResponse);
}

message LayoutRequest {
  int32 grid_width = 1;
  int32 grid_depth = 2;
  double cell_size_meters = 3;
  SystemType system_type = 4;
  StationLayoutConfig station_layout = 5;
  HighwayConfig highway_config = 6;
  RackBlockConfig rack_block_config = 7;
}

message LayoutResponse {
  GridBounds bounds = 1;
  repeated StationPosition stations = 2;
  repeated HighwaySegment highways = 3;
  repeated RackBlockDefinition rack_blocks = 4;
  repeated RelayPositionData relay_positions = 5;  // Only for RELAY system
  map<string, RackPositionData> rack_position_data = 6;
}

message NavigationDataResponse {
  GridBounds bounds = 1;
  repeated GridCoordinate blocked_positions = 2;
  repeated GridCoordinate rack_positions = 3;
  repeated GridCoordinate highway_positions = 4;
  repeated GridCoordinate relay_positions = 5;
}
```

### Consumers

| Consumer | Endpoint | Purpose |
|----------|----------|---------|
| Fleet-simulator | `GetLayout` | Full map for simulation setup |
| Navigation Service | `GetNavigationData` | Traversability data (via fleet-simulator) |
| Frontend | `GetLayout` | Visualization rendering |
| Put Service | `GetLayout` | Grid dimensions for SKU placement |

### Code Mapping

| Current Location | Moves To |
|-----------------|----------|
| `domain/map/WarehouseMap.java` | `layout-service/.../domain/WarehouseMap.java` |
| `domain/map/WarehouseMapConfig.java` | `layout-service/.../domain/WarehouseMapConfig.java` |
| `domain/map/StationEdge.java` | `layout-service/.../domain/StationEdge.java` |
| `domain/map/StationEdgeConfig.java` | `layout-service/.../domain/StationEdgeConfig.java` |
| `domain/map/StationLayoutConfig.java` | `layout-service/.../domain/StationLayoutConfig.java` |
| `domain/map/StationLayoutType.java` | `layout-service/.../domain/StationLayoutType.java` |
| `domain/map/HighwayConfig.java` | `layout-service/.../domain/HighwayConfig.java` |
| `domain/map/RackBlockConfig.java` | `layout-service/.../domain/RackBlockConfig.java` |

## Navigation Service

### Responsibility

Compute optimal paths using A* algorithm. Holds session cache for map data during simulation.

### Session Lifecycle

```
1. CreateSession(navigationData, ttl) → sessionId
   - Stores map data in memory cache
   - Returns unique session ID

2. FindPath(sessionId, start, goal, ...) → PathResult
   - Uses cached map data for pathfinding
   - Called many times during simulation

3. DeleteSession(sessionId)
   - Explicit cleanup when simulation ends
   - TTL as safety net for crashes (default: 1 hour)
```

### API

```protobuf
service NavigationService {
  rpc CreateSession(CreateSessionRequest) returns (CreateSessionResponse);
  rpc FindPath(FindPathRequest) returns (FindPathResponse);
  rpc DeleteSession(DeleteSessionRequest) returns (DeleteSessionResponse);
}

message CreateSessionRequest {
  GridBounds bounds = 1;
  repeated GridCoordinate blocked_positions = 2;
  repeated GridCoordinate rack_positions = 3;
  repeated GridCoordinate highway_positions = 4;
  repeated GridCoordinate relay_positions = 5;
  int32 ttl_seconds = 6;  // Optional, default 3600
}

message CreateSessionResponse {
  string session_id = 1;
}

message FindPathRequest {
  string session_id = 1;
  GridCoordinate start = 2;
  GridCoordinate goal = 3;
  Orientation start_orientation = 4;
  RobotType robot_type = 5;
  bool is_carrying_cargo = 6;
  double robot_speed = 7;
  double base_turn_time_90 = 8;
}

message FindPathResponse {
  repeated PathWaypoint waypoints = 1;
  double total_time = 2;
  double total_distance = 3;
  int32 total_turns = 4;
}

message PathWaypoint {
  GridCoordinate position = 1;
  Orientation orientation = 2;
  NavigationAction action = 3;
  double time_delta = 4;
  double cumulative_time = 5;
}

enum NavigationAction {
  NAVIGATION_ACTION_UNSPECIFIED = 0;
  MOVE_FORWARD = 1;
  MOVE_BACKWARD = 2;
  TURN_90_CW = 3;
  TURN_90_CCW = 4;
  TURN_180 = 5;
}

message DeleteSessionRequest {
  string session_id = 1;
}

message DeleteSessionResponse {
  bool success = 1;
}
```

### Code Mapping

| Current Location | Moves To |
|-----------------|----------|
| `domain/navigation/PathFinder.java` | `navigation-service/.../domain/PathFinder.java` |
| `domain/navigation/PathResult.java` | Proto (NavigationService response) |
| `domain/navigation/PathWaypoint.java` | Proto (NavigationService response) |
| `domain/navigation/PathState.java` | `navigation-service/.../domain/PathState.java` |
| `domain/navigation/StateKey.java` | `navigation-service/.../domain/StateKey.java` |
| `domain/navigation/NavigationAction.java` | Proto (enum) |
| `domain/navigation/Orientation.java` | `libs/java/common-domain` |

## Shared Types

### common-domain (libs/java/common-domain/)

Fundamental domain concepts used across services:

```java
// Already exists
GridCoordinate
SystemType

// Add
Orientation      // NORTH, EAST, SOUTH, WEST with delta methods
RobotType        // VTM, HTM, RTP
```

### Proto Definitions

API request/response types generated per-service:

```
libs/proto/src/grpc/
├── common/v1/types.proto      # GridCoordinate, SystemType (existing)
├── layout/v1/layout.proto     # LayoutService API (NEW)
└── navigation/v1/navigation.proto  # NavigationService API (NEW)
```

## Monorepo Structure

```
apps/
├── fleet-simulator/        # Existing - becomes orchestrator
├── put-service/            # Existing
├── layout-service/         # NEW
│   └── src/main/java/com/greyorange/layout/
│       ├── api/            # gRPC controllers
│       ├── domain/         # WarehouseMap, configs (moved from fleet-sim)
│       └── LayoutServiceApplication.java
├── navigation-service/     # NEW
│   └── src/main/java/com/greyorange/navigation/
│       ├── api/            # gRPC controllers
│       ├── domain/         # PathFinder (moved from fleet-sim)
│       ├── session/        # Session management
│       └── NavigationServiceApplication.java
└── frontend/

libs/
├── java/
│   ├── common-domain/      # Add Orientation, RobotType
│   ├── common-messaging/   # GrpcChannelFactory (existing)
│   └── grpc-stubs/         # Generated stubs
└── proto/
    └── src/grpc/
        ├── common/v1/
        ├── put/v1/
        ├── layout/v1/      # NEW
        └── navigation/v1/  # NEW
```

## Error Handling

### Strategy: Fail-Fast

For a simulation system (batch processing), clear failure is better than partial/incorrect results.

```java
// Fleet-simulator client code
try {
    PathResult path = navigationClient.findPath(sessionId, start, goal, ...);
    // use path
} catch (GrpcException e) {
    throw new SimulationFailedException(
        "Navigation service unavailable: " + e.getMessage(), e);
}
```

No fallback to embedded code - simpler codebase, clear failure modes.

## Migration Strategy

### Critical Principle: Move First, Then Adapt

**DO NOT** move and modify code in the same commit. This makes review impossible and risks introducing bugs.

### Phase 1a: Create Empty Service Scaffolding

```bash
# Create Spring Boot projects
apps/layout-service/
apps/navigation-service/
```

**Commit**: `"scaffold layout-service and navigation-service"`

### Phase 1b: Move Files As-Is (NO LOGIC CHANGES)

```bash
# Move map code
git mv apps/fleet-simulator/.../domain/map/* apps/layout-service/.../domain/

# Move navigation code
git mv apps/fleet-simulator/.../domain/navigation/* apps/navigation-service/.../domain/
```

**Commit**: `"move map and navigation code to new services"`

This commit should be reviewable with `git diff --stat` showing ONLY file moves.

### Phase 1c: Fix Imports (NO LOGIC CHANGES)

- Update package declarations
- Fix import statements
- Add missing dependencies to pom.xml
- **NO LOGIC CHANGES**

**Commit**: `"fix imports after move"`

### Phase 2: Add gRPC Layer ON TOP of Existing Code

```java
// layout-service: Add controller that CALLS existing WarehouseMap
@GrpcService
public class LayoutGrpcController {
    public LayoutResponse getLayout(LayoutRequest request) {
        // Convert request to existing config objects
        WarehouseMapConfig config = convertToConfig(request);

        // Call EXISTING code (unchanged)
        WarehouseMap map = new WarehouseMap(systemType, config);

        // Convert to proto response
        return convertToProto(map);
    }
}
```

**Commit**: `"add gRPC layer to layout-service"`
**Commit**: `"add gRPC layer to navigation-service"`

### Phase 3: Fleet-Simulator Becomes Client

```java
// Before: Direct instantiation
WarehouseMap map = new WarehouseMap(systemType, config);

// After: Service call
LayoutResponse response = layoutClient.getLayout(request);
```

**Commit**: `"fleet-simulator calls layout and navigation services"`

### Phase 4: Remove Duplicate Code

Remove old `domain/map/` and `domain/navigation/` from fleet-simulator.

**Commit**: `"remove migrated code from fleet-simulator"`

### Phase 5: Other Consumers Migrate

- Frontend calls Layout Service for visualization
- Put Service calls Layout Service for grid info

**Commits**: Per-consumer migration

## Testing Strategy

### Pre-Migration: Capture Expected Outputs

```java
// Record current behavior for known inputs
@Test
void captureBaselineMapGeneration() {
    WarehouseMap map = new WarehouseMap(config);

    // Serialize to JSON for comparison
    saveBaseline("map_50x50_relay.json", map);
}

@Test
void captureBaselinePathfinding() {
    PathResult result = pathFinder.findPath(start, goal, map, ...);

    saveBaseline("path_0_0_to_25_25.json", result);
}
```

### Post-Migration: Compare Against Baseline

```java
@Test
void verifyLayoutServiceMatchesBaseline() {
    LayoutResponse response = layoutClient.getLayout(request);

    assertMatchesBaseline("map_50x50_relay.json", response);
}

@Test
void verifyNavigationServiceMatchesBaseline() {
    FindPathResponse response = navigationClient.findPath(request);

    assertMatchesBaseline("path_0_0_to_25_25.json", response);
}
```

### Integration Tests

```java
@Test
void fullSimulationProducesSameResults() {
    // Run simulation with new services
    SimulationResult newResult = runSimulation(config);

    // Compare key metrics against baseline
    assertEquals(baseline.totalPicks, newResult.totalPicks);
    assertEquals(baseline.ordersCompleted, newResult.ordersCompleted);
}
```

## Docker Compose

```yaml
# docker-compose.dev.yml (updated)
services:
  layout-service:
    image: layout-service:latest
    ports:
      - "8082:8082"
      - "9091:9091"

  navigation-service:
    image: navigation-service:latest
    ports:
      - "8083:8083"
      - "9092:9092"

  fleet-simulator:
    image: fleet-simulator:latest
    ports:
      - "8080:8080"
    environment:
      - LAYOUT_SERVICE_HOST=layout-service
      - LAYOUT_SERVICE_PORT=9091
      - NAVIGATION_SERVICE_HOST=navigation-service
      - NAVIGATION_SERVICE_PORT=9092
    depends_on:
      - layout-service
      - navigation-service
      - put-service

  put-service:
    image: put-service:latest
    ports:
      - "8081:8081"
      - "9090:9090"
```

## Configuration

### fleet-simulator application.yml

```yaml
grpc:
  clients:
    layout-service:
      host: ${LAYOUT_SERVICE_HOST:localhost}
      port: ${LAYOUT_SERVICE_PORT:9091}
      plaintext: true
    navigation-service:
      host: ${NAVIGATION_SERVICE_HOST:localhost}
      port: ${NAVIGATION_SERVICE_PORT:9092}
      plaintext: true
    put-service:
      host: ${PUT_SERVICE_HOST:localhost}
      port: ${PUT_SERVICE_PORT:9090}
      plaintext: true
```

### navigation-service application.yml

```yaml
server:
  port: 8083

grpc:
  server:
    port: 9092

navigation:
  session:
    default-ttl-seconds: 3600
    cleanup-interval-seconds: 300
```

## Implementation Order

1. **Add types to common-domain** (Orientation, RobotType)
2. **Create proto definitions** (layout.proto, navigation.proto)
3. **Scaffold layout-service** (empty Spring Boot)
4. **Move map code** (as-is, fix imports)
5. **Add gRPC layer to layout-service**
6. **Scaffold navigation-service** (empty Spring Boot)
7. **Move navigation code** (as-is, fix imports)
8. **Add gRPC layer to navigation-service**
9. **Update fleet-simulator** (become client)
10. **Remove old code from fleet-simulator**
11. **Update Docker Compose**
12. **Update other consumers** (frontend, put-service)

## Open Questions

1. **Session cleanup strategy**: Use simple TTL or add explicit heartbeat?
2. **Caching**: Should Layout Service cache generated maps by config hash?
3. **Metrics**: What observability do we need for navigation latency?

## References

- [Microservices Decomposition Design](../architecture/microservices-decomposition-design.md)
- [Polyglot Monorepo Structure](../architecture/polyglot-monorepo-structure-design.md)
- [Phase 4 gRPC Mode Enablement](./2024-12-26-phase4-grpc-mode-enablement.md)
- [Common Messaging Design](./2025-12-26-common-messaging-design.md)
