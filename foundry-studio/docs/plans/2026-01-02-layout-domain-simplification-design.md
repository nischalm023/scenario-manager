# Layout Domain Simplification Design

**Date:** 2026-01-02
**Status:** In Progress
**Branch:** refactor/layout-and-navigation

## Problem

The layout-service was scaffolded by copying domain classes from fleet-simulator. Now that layout-service is the source of truth and fleet-simulator consumes layouts via gRPC, the duplicated **output domain classes** should be removed (6 files). Input config types (7 files) are kept since they're used to build `LayoutRequest` proto.

**Current state:** Migration already started - `LayoutAdapter` exists and wraps `LayoutResponse`, but `WarehouseMap` is still created locally because some components haven't migrated yet.

## Decision

- **layout-service** owns the domain model for layout generation
- **fleet-simulator** uses proto types via `LayoutAdapter` (no domain classes)
- Migrate remaining consumers from `WarehouseMap` → `LayoutAdapter` + proto types
- Delete duplicated domain files from fleet-simulator
- **Full switch to proto types** - consumers use proto messages directly, no intermediate domain types

## Key Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| LayoutOperations location | `fleet-simulator/adapter/` | Avoids coupling common-domain to grpc-stubs |
| Domain types (StationGridPosition, etc.) | Delete - use proto directly | Full proto switch, no intermediate types |
| TpqCalculator A* calibration | Refactor to use layout-service | Consistent with architecture |
| WarehouseMapConfig in API | Keep as-is (YAGNI) | It's the API contract, works fine |
| Test mode (no layout-service) | Mock LayoutAdapter | Clean test isolation |

## Architecture

```
layout-service (owns domain) ──gRPC──> fleet-simulator (LayoutAdapter wraps proto)
```

## Current Usage Analysis

| WarehouseMap Method | Consumers | LayoutAdapter Status |
|---------------------|-----------|---------------------|
| `getStationPositions()` | StationManager, PutServiceClient, GridPopularityManager, TpqCalculator | Has `getProtoStations()` |
| `getRackBlocks()` | ZoneManager | Has `getProtoRackBlocks()` |
| `getRackPositions()` | WarehouseBuilder, GridPopularityManager, BackToStoreSelector | ✅ Already has |
| `getRackPositionData()` | WarehouseBuilder | Has `getProtoRackPositionData()` |
| `isPositionAvailable()` | PathFinder | ✅ Already has |
| `isRackPosition()` | PathFinder | ✅ Already has |
| `isHighway()` | PathFinder | ✅ Already has |

## Changes Required

### Phase 1: Create LayoutOperations Utility

Create `apps/fleet-simulator/.../adapter/LayoutOperations.java`:

```java
public final class LayoutOperations {
    // Convert proto → domain GridCoordinate
    public static GridCoordinate toDomain(com.greyorange.grpc.common.v1.GridCoordinate proto);

    // Computed: station access position (from proto StationPosition)
    public static GridCoordinate getAccessPosition(StationPosition station);

    // Computed: all coordinates in highway segment
    public static List<GridCoordinate> getAllCoordinates(HighwaySegment segment);

    // Computed: rack positions in block (excluding aisles)
    public static List<GridCoordinate> getRackPositions(RackBlockDefinition block);

    // Check if position is in block bounds
    public static boolean containsPosition(RackBlockDefinition block, GridCoordinate pos);

    // World coordinate conversion (replaces GridToWorldConverter)
    public static double toWorldX(int col, double cellSizeMeters);
    public static double toWorldY(int row, double cellSizeMeters);
}
```

**Note:** This utility works with proto types directly. Consumers call `LayoutOperations.method(protoObject)` instead of `domainObject.method()`.

### Phase 2: Migrate Consumers (one at a time)

| File | Change |
|------|--------|
| `StationManager.java` | Use `layoutAdapter.getProtoStations()` + `LayoutOperations` |
| `GridPopularityManager.java` | Use `LayoutAdapter` instead of `WarehouseMap` |
| `ZoneManager.java` | Use `layoutAdapter.getProtoRackBlocks()` |
| `PutServiceClient.java` | Use `layoutAdapter.getProtoStations()` |
| `TpqCalculator.java` | Use `LayoutAdapter` |
| `WarehouseBuilder.java` | Remove local `WarehouseMap` creation, use `LayoutAdapter` only |
| `BackToStoreSelector` classes | Use `LayoutAdapter` |
| `RelayPositionManager.java` | Use `layoutAdapter.getProtoRelayPositions()` |

### Phase 3: Delete Duplicated Files

**Key distinction:**
- **Input config types** (WarehouseMapConfig, etc.) → KEEP - used to build `LayoutRequest` proto
- **Output domain types** (WarehouseMap, StationGridPosition, etc.) → DELETE - replaced by proto types

```
apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/
├── HighwayConfig.java          ← KEEP (input config for LayoutRequest)
├── RackBlockConfig.java        ← KEEP (input config for LayoutRequest)
├── StationEdge.java            ← KEEP (input config for LayoutRequest)
├── StationEdgeConfig.java      ← KEEP (input config for LayoutRequest)
├── StationLayoutConfig.java    ← KEEP (input config for LayoutRequest)
├── StationLayoutType.java      ← KEEP (input config for LayoutRequest)
├── WarehouseMap.java           ← DELETE (output domain - replaced by LayoutAdapter)
└── WarehouseMapConfig.java     ← KEEP (API contract)

apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates/
├── GridToWorldConverter.java   ← DELETE (move to LayoutOperations)
├── HighwayGridSegment.java     ← DELETE (use proto HighwaySegment)
├── RackBlockGridDefinition.java← DELETE (use proto RackBlockDefinition)
├── StationGridPosition.java    ← DELETE (use proto StationPosition)
└── WorldCoordinate.java        ← DELETE (inline in LayoutOperations)
```

**Summary:** Delete 6 files (1 from domain/map, 5 from coordinates), keep 7 config files.

### Phase 4: Update LayoutProtoConverter

After migration, `LayoutProtoConverter` only needs:
- `toLayoutRequest(SimulationConfig)` - build proto request from API config
- `toDomain(GridCoordinate)` - single coordinate conversion (move to LayoutOperations)

Delete domain→proto conversions (StationEdge, HighwayConfig, etc.) - no longer needed.

## layout-service - KEEP (source of truth)

All domain classes remain unchanged:
- `domain/map/*` - Config records and WarehouseMap generator
- `domain/common/coordinates/*` - Coordinate types used by generator

## Implementation Order

1. **Create `LayoutOperations`** utility in `fleet-simulator/adapter/`
2. **Migrate `StationManager`** - simplest consumer, validates approach
3. **Migrate `GridPopularityManager`** - needs racks + stations
4. **Migrate `ZoneManager`** - uses RackBlockDefinition proto
5. **Migrate `PutServiceClient`** - can use proto directly
6. **Migrate `RelayPositionManager`** - proto to domain mapping
7. **Migrate `TpqCalculator`** - refactor to use layout-service
8. **Migrate `BackToStoreSelector` classes** - depends on LayoutAdapter
9. **Update `WarehouseBuilder.buildMap()`** - remove local WarehouseMap creation
10. **Delete domain files** - compile and test after each deletion
11. **Cleanup `LayoutProtoConverter`** - remove unused conversions

## Testing Strategy

**Mock LayoutAdapter for tests:**
- Create test fixtures with pre-built `LayoutResponse` proto
- Tests don't require layout-service running
- Example: `TestLayoutAdapter.withDefaultLayout()` returns mock

```java
// Test helper
public class TestLayoutAdapter {
    public static LayoutAdapter withDefaultLayout() {
        return new LayoutAdapter(buildTestLayoutResponse());
    }

    private static LayoutResponse buildTestLayoutResponse() {
        // Build minimal valid layout for tests
    }
}
```

## Verification

After each consumer migration:
```bash
cd apps/fleet-simulator
mvn clean compile
mvn test
```

## Rationale

- **Zero type duplication** - Proto types from grpc-stubs are the shared contract
- **Single source of truth** - Layout generation logic lives only in layout-service
- **Incremental migration** - Each consumer migrated independently, tested in isolation
- **Already started** - LayoutAdapter exists, just needs consumers migrated
