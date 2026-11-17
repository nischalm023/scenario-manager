# Phase 2: Put Service Extraction - Implementation Summary

**Date:** December 23, 2024  
**Status:** ✅ Complete  
**Next Phase:** Phase 3 - gRPC Integration

---

## Overview

This phase extracted the PUT inventory initialization logic from the monolithic `fleet-simulator` into a standalone `put-service` microservice. The implementation followed a "move code, don't rewrite" philosophy to minimize bugs.

---

## What Was Done

### Phase 2A: Common Domain Extraction

Extracted 6 pure value types from `fleet-simulator` to a shared `common-domain` library.

#### Files Created in `libs/java/common-domain/`

| File | Type | Description |
|------|------|-------------|
| `src/.../common/types/StorageSpaceId.java` | Sealed interface | Base type for storage space identifiers |
| `src/.../common/types/RackId.java` | Record | Rack identifier (implements StorageSpaceId) |
| `src/.../common/types/ToteId.java` | Record | Tote identifier (implements StorageSpaceId) |
| `src/.../common/types/StationId.java` | Record | Station identifier |
| `src/.../common/types/SystemType.java` | Enum | RTP vs RELAY system type (with Jackson annotations) |
| `src/.../common/coordinates/GridCoordinate.java` | Record | 2D grid position with geometry methods |

#### Package Changes

```
OLD: com.greyorange.dessimulator.domain.common.types.*
NEW: com.greyorange.common.types.*

OLD: com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate
NEW: com.greyorange.common.coordinates.GridCoordinate
```

#### Files Modified in `fleet-simulator`

- Updated ~50+ Java files with new import statements
- Updated `pom.xml` to depend on `common-domain`
- Added GridCoordinate import to coordinate classes that stayed in fleet-simulator:
  - `StationGridPosition.java`
  - `RackBlockGridDefinition.java`
  - `HighwayGridSegment.java`
  - `GridToWorldConverter.java`

#### Files Deleted from `fleet-simulator`

- `domain/common/types/StorageSpaceId.java`
- `domain/common/types/RackId.java`
- `domain/common/types/ToteId.java`
- `domain/common/types/StationId.java`
- `domain/common/types/SystemType.java`
- `domain/common/coordinates/GridCoordinate.java`

---

### Phase 2B: Put Service Implementation

Created a stateless PUT service with REST API that computes inventory assignments.

#### New Files in `apps/put-service/`

##### DTOs (`src/.../put/dto/`)

| File | Purpose |
|------|---------|
| `SkuInfo.java` | SKU data (sku, maxInventoryQuantity, pastOrderedQuantity, isPopular) |
| `StorageSpaceInfo.java` | Storage space data (id, maxCapacity, position, isRack) |
| `StationInfo.java` | Station data (id, position) |
| `InventoryAssignment.java` | Output tuple (storageSpaceId, sku, quantity) |
| `InitInventoryRequest.java` | API request containing all input data |
| `InitInventoryResponse.java` | API response with assignments and stats |

##### Domain Logic (`src/.../put/domain/`)

| File | Source | Key Changes |
|------|--------|-------------|
| `PutAlgorithm.java` | Copied from fleet-simulator | Package declaration only |
| `PopularityCalculator.java` | Extracted from GridPopularityManager | Static/stateless; takes positions, returns scores |
| `PutProcessor.java` | Adapted from fleet-simulator | Takes DTOs, returns assignments |

##### Strategies (`src/.../put/domain/strategies/`)

| File | Source | Key Changes |
|------|--------|-------------|
| `PutStrategy.java` | New interface | Returns `List<InventoryAssignment>` |
| `AbstractPutStrategy.java` | Adapted | Works with DTOs; tracks assignments internally |
| `GreedyPutStrategy.java` | Adapted | Core algorithm preserved; uses PopularityResult |
| `MinWeightedCycleDistancePutStrategy.java` | Adapted | Core OR-Tools logic preserved |

##### API Layer (`src/.../put/api/` and `src/.../put/config/`)

| File | Purpose |
|------|---------|
| `PutController.java` | REST endpoint at `/api/v1/put/initialize-inventory` |
| `PutServiceConfig.java` | Spring bean configuration for PutProcessor |

#### POM Dependencies Added

```xml
<dependency>
    <groupId>com.greyorange.warehouse</groupId>
    <artifactId>common-domain</artifactId>
</dependency>
<dependency>
    <groupId>com.google.ortools</groupId>
    <artifactId>ortools-java</artifactId>
</dependency>
```

---

## Architecture After Phase 2

```
┌─────────────────────────────────────────────────────────────────┐
│                        libs/java/                               │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  common-domain                                           │   │
│  │  - GridCoordinate, RackId, ToteId, StationId, SystemType│   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
┌──────────────────────────┐    ┌──────────────────────────┐
│    fleet-simulator       │    │      put-service         │
│                          │    │                          │
│  - WarehouseInventory    │    │  - PopularityCalculator  │
│  - GridPopularityManager │    │  - PutProcessor          │
│  - BackToStore logic     │    │  - GreedyPutStrategy     │
│  - Simulation engine     │    │  - MinWeightedCycle...   │
│                          │    │  - REST API              │
│  (130 tests ✓)           │    │  (1 test ✓)              │
└──────────────────────────┘    └──────────────────────────┘
```

---

## Data Flow: Initialize Inventory

```
fleet-simulator                          put-service
     │                                        │
     │  POST /api/v1/put/initialize-inventory │
     │  ──────────────────────────────────────►
     │  {                                     │
     │    systemType: "rtp",                  │
     │    algorithm: "greedy",                │
     │    skus: [...],                        │
     │    storageSpaces: [...],               │
     │    stations: [...]                     │
     │  }                                     │
     │                                        │
     │                           ┌────────────┴────────────┐
     │                           │ PopularityCalculator    │
     │                           │ PutStrategy.compute()   │
     │                           └────────────┬────────────┘
     │                                        │
     │  ◄──────────────────────────────────────
     │  {                                     │
     │    success: true,                      │
     │    assignments: [                      │
     │      {storageSpaceId, sku, qty},       │
     │      ...                               │
     │    ],                                  │
     │    totalSkusPlaced: 500,               │
     │    totalUnitsPlaced: 15000             │
     │  }                                     │
     │                                        │
     ▼                                        │
Apply assignments to                          │
WarehouseInventory                            │
```

---

## Key Design Decisions

### 1. GridPopularityManager Split

**Problem:** GridPopularityManager has both static calculation logic AND mutable runtime state (zone load tracking).

**Solution:**
- **PopularityCalculator** (in put-service): Stateless, computes scores from positions
- **GridPopularityManager** (stays in fleet-simulator): Keeps runtime zone load tracking

### 2. Return Assignments Instead of Mutating

**Before (fleet-simulator):**
```java
public void initializeInventory() {
    inventory.addItemToStorageSpace(storageSpaceId, sku, qty);
}
```

**After (put-service):**
```java
public List<InventoryAssignment> computeAssignments() {
    assignments.add(new InventoryAssignment(storageSpaceId, sku, qty));
    return assignments;
}
```

### 3. DTOs for Service Boundary

Created simple record DTOs (`SkuInfo`, `StorageSpaceInfo`, etc.) to decouple put-service from fleet-simulator's domain objects.

---

## Verification Results

| Component | Tests | Status |
|-----------|-------|--------|
| common-domain | Build only | ✅ Compiles |
| fleet-simulator | 130 tests | ✅ All pass |
| put-service | 1 test | ✅ Passes |

---

## How to Test Manually

```bash
# 1. Build common-domain
cd libs/java/common-domain && mvn clean install

# 2. Build put-service
cd apps/put-service && mvn clean package

# 3. Run put-service
mvn spring-boot:run

# 4. Test endpoint
curl -X POST http://localhost:8080/api/v1/put/initialize-inventory \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "rtp",
    "algorithm": "greedy",
    "skus": [
      {"sku": "SKU001", "maxInventoryQuantity": 100, "pastOrderedQuantity": 50, "isPopular": true}
    ],
    "storageSpaces": [
      {"id": 1, "maxCapacity": 200, "position": {"row": 0, "col": 5}, "isRack": true}
    ],
    "stations": [
      {"id": 1, "position": {"row": 0, "col": 0}}
    ],
    "storageSpaceToZone": {}
  }'
```

---

## Phase 3: gRPC Integration (Next Steps)

### Goal
Replace REST API with gRPC for better performance and type safety.

### Tasks

1. **Create Proto Definitions** (`libs/proto/put/`)
   ```protobuf
   service PutService {
     rpc InitializeInventory(InitInventoryRequest) returns (InitInventoryResponse);
   }
   ```

2. **Add gRPC Server to put-service**
   - Add `grpc-spring-boot-starter` dependency
   - Implement `PutServiceGrpc.PutServiceImplBase`
   - Wire to existing `PutProcessor`

3. **Add gRPC Client to fleet-simulator**
   - Add gRPC client dependency
   - Create `PutServiceClient` wrapper
   - Add feature flag to toggle local vs remote PUT

4. **Feature Flag Implementation**
   ```java
   if (featureFlags.useRemotePutService()) {
       assignments = putServiceClient.initializeInventory(request);
   } else {
       // existing local logic
   }
   ```

### Recommended Proto Structure

```protobuf
// libs/proto/put/v1/put_service.proto

syntax = "proto3";
package greyorange.put.v1;

message SkuInfo {
  string sku = 1;
  int32 max_inventory_quantity = 2;
  int32 past_ordered_quantity = 3;
  bool is_popular = 4;
}

message StorageSpaceInfo {
  int32 id = 1;
  int32 max_capacity = 2;
  GridCoordinate position = 3;
  bool is_rack = 4;
}

message GridCoordinate {
  int32 row = 1;
  int32 col = 2;
}

message StationInfo {
  int32 id = 1;
  GridCoordinate position = 2;
}

message InventoryAssignment {
  int32 storage_space_id = 1;
  string sku = 2;
  int32 quantity = 3;
}

message InitInventoryRequest {
  string system_type = 1;  // "rtp" or "relay"
  string algorithm = 2;     // "greedy" or "min_weighted_cycle_distance"
  repeated SkuInfo skus = 3;
  repeated StorageSpaceInfo storage_spaces = 4;
  repeated StationInfo stations = 5;
  map<int32, string> storage_space_to_zone = 6;
}

message InitInventoryResponse {
  bool success = 1;
  repeated InventoryAssignment assignments = 2;
  int32 total_skus_placed = 3;
  int32 total_units_placed = 4;
  string algorithm_used = 5;
  int64 execution_time_ms = 6;
}

service PutService {
  rpc InitializeInventory(InitInventoryRequest) returns (InitInventoryResponse);
}
```

---

## Files Changed Summary

### Created (27 files)
- 6 files in `libs/java/common-domain/`
- 16 files in `apps/put-service/`
- Updated POMs (2 files)

### Modified (~55 files)
- Import updates in `apps/fleet-simulator/`
- GridCoordinate imports in coordinate classes

### Deleted (6 files)
- Old type files from `apps/fleet-simulator/domain/common/`

---

## Rollback Instructions

If needed, rollback can be done via git:

```bash
# Revert all Phase 2 changes
git checkout HEAD~1 -- apps/fleet-simulator/
git checkout HEAD~1 -- libs/java/common-domain/
git checkout HEAD~1 -- apps/put-service/
```

Or selectively:

```bash
# Just revert fleet-simulator to use local types again
git checkout HEAD~1 -- apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/
git checkout HEAD~1 -- apps/fleet-simulator/pom.xml
```

---

## Contact

For questions about this implementation, refer to:
- Plan document: `docs/plans/put_service_extraction_plan_e9c7aa38.plan.md`
- Architecture doc: `docs/architecture/microservices-decomposition-design.md`

