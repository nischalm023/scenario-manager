# Phase 2: Common Domain Extraction

**Status**: Ready for Implementation  
**Created**: 2024-12-23  
**Depends on**: Phase 1 (Polyglot Monorepo Structure) - COMPLETED  
**Author**: Architecture Team

---

## Executive Summary

Extract shared domain types from `apps/fleet-simulator/` to `libs/java/common-domain/`. This creates the first true shared library in the polyglot monorepo, enabling both `fleet-simulator` and `put-service` to share common value types.

---

## Prerequisites (Phase 1 Completed)

The following structure is already in place:

```
queue_simulator/
├── apps/
│   ├── fleet-simulator/          # Renamed from backend, builds successfully
│   ├── put-service/              # Skeleton Spring Boot app on port 8081
│   └── frontend/
├── libs/
│   ├── java/
│   │   ├── pom.xml               # Parent POM (Java 21, Spring Boot 3.2.0)
│   │   └── common-domain/        # Empty skeleton - TO BE POPULATED
│   ├── proto/                    # Placeholder for gRPC (Phase 3)
│   └── api-schema/
└── nx.json
```

**Verification**: All builds pass, 130 tests pass in fleet-simulator.

---

## Scope

### Types to Extract

From `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/`:

#### Coordinates Package
| File | Description | Used By |
|------|-------------|---------|
| `GridCoordinate.java` | (x, y) grid position | Navigation, Map, Robots |
| `WorldCoordinate.java` | Real-world coordinates | Visualization, Pathfinding |
| `GridToWorldConverter.java` | Coordinate conversion | Map, Visualization |
| `HighwayGridSegment.java` | Highway segment definition | Map, Navigation |
| `RackBlockGridDefinition.java` | Rack block bounds | Map, Storage |
| `StationGridPosition.java` | Station position | Stations, Navigation |

#### Types Package
| File | Description | Used By |
|------|-------------|---------|
| `RackId.java` | Rack identifier (value object) | Inventory, Storage |
| `ToteId.java` | Tote identifier (value object) | Inventory, Robots |
| `StorageSpaceId.java` | Storage space identifier | Inventory, Put logic |
| `StationId.java` | Station identifier | Stations, Robots |
| `SystemType.java` | VTM/HTM/RTP enum | Robot processes |

### Target Structure

```
libs/java/common-domain/
├── pom.xml
├── project.json
└── src/
    ├── main/java/com/greyorange/common/
    │   ├── coordinates/
    │   │   ├── GridCoordinate.java
    │   │   ├── WorldCoordinate.java
    │   │   ├── GridToWorldConverter.java
    │   │   ├── HighwayGridSegment.java
    │   │   ├── RackBlockGridDefinition.java
    │   │   └── StationGridPosition.java
    │   └── types/
    │       ├── RackId.java
    │       ├── ToteId.java
    │       ├── StorageSpaceId.java
    │       ├── StationId.java
    │       └── SystemType.java
    └── test/java/com/greyorange/common/
        └── (any existing tests for these types)
```

---

## Implementation Steps

### Step 1: Copy Files to common-domain

Copy the source files maintaining package structure:

```bash
# From workspace root
cp -r apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates \
      libs/java/common-domain/src/main/java/com/greyorange/common/

cp -r apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/types \
      libs/java/common-domain/src/main/java/com/greyorange/common/
```

### Step 2: Update Package Declarations

In each copied file, change package from:
```java
package com.greyorange.dessimulator.domain.common.coordinates;
```
To:
```java
package com.greyorange.common.coordinates;
```

And for types:
```java
package com.greyorange.dessimulator.domain.common.types;
```
To:
```java
package com.greyorange.common.types;
```

### Step 3: Add Maven Dependency in fleet-simulator

Update `apps/fleet-simulator/pom.xml`:

```xml
<dependencies>
    <!-- Internal dependencies -->
    <dependency>
        <groupId>com.greyorange.warehouse</groupId>
        <artifactId>common-domain</artifactId>
    </dependency>
    
    <!-- ... existing dependencies ... -->
</dependencies>
```

Note: Version is managed by parent POM's `dependencyManagement`.

### Step 4: Update All Imports in fleet-simulator

Find and replace all imports in `apps/fleet-simulator/src/`:

| Old Import | New Import |
|------------|------------|
| `com.greyorange.dessimulator.domain.common.coordinates.*` | `com.greyorange.common.coordinates.*` |
| `com.greyorange.dessimulator.domain.common.types.*` | `com.greyorange.common.types.*` |

**Files to update** (grep for imports):
```bash
grep -r "dessimulator.domain.common" apps/fleet-simulator/src --include="*.java" -l
```

Expected to find references in:
- `domain/map/*.java`
- `domain/inventory/*.java`
- `domain/robot/*.java`
- `domain/station/*.java`
- `domain/navigation/*.java`
- `domain/put/*.java`
- `domain/order/*.java`
- `simulator/**/*.java`
- `api/**/*.java`
- `dto/**/*.java`

### Step 5: Delete Original Files from fleet-simulator

After successful build:
```bash
rm -rf apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/
```

### Step 6: Add Implicit Dependencies to NX

Update `apps/fleet-simulator/project.json`:

```json
{
  "name": "fleet-simulator",
  ...
  "implicitDependencies": ["common-domain"],
  "tags": ["type:app", "scope:simulator", "language:java"]
}
```

Update `apps/put-service/project.json` (for future use):

```json
{
  "name": "put-service",
  ...
  "implicitDependencies": ["common-domain"],
  "tags": ["type:app", "scope:put", "language:java"]
}
```

### Step 7: Add Dependency to put-service pom.xml

Update `apps/put-service/pom.xml`:

```xml
<dependencies>
    <!-- Internal dependencies -->
    <dependency>
        <groupId>com.greyorange.warehouse</groupId>
        <artifactId>common-domain</artifactId>
    </dependency>
    
    <!-- ... existing dependencies ... -->
</dependencies>
```

---

## Verification Checklist

### Build Verification

```bash
# Build common-domain first (installs to local Maven repo)
nx build common-domain

# Build fleet-simulator (should find common-domain dependency)
nx build fleet-simulator

# Build put-service
nx build put-service

# Run all tests
nx test fleet-simulator   # Should pass all 130 tests
nx test put-service       # Should pass context load test
nx test common-domain     # Should pass (if any tests exist)
```

### Dependency Graph Verification

```bash
nx graph --file=project-graph.json
```

Expected dependencies:
```json
{
  "dependencies": {
    "common-domain": [],
    "fleet-simulator": [
      { "source": "fleet-simulator", "target": "common-domain", "type": "implicit" }
    ],
    "put-service": [
      { "source": "put-service", "target": "common-domain", "type": "implicit" }
    ],
    "frontend": [],
    "proto": []
  }
}
```

### Import Verification

```bash
# Should return NO results (all old imports replaced)
grep -r "dessimulator.domain.common" apps/fleet-simulator/src --include="*.java"

# Should return results in common-domain only
grep -r "greyorange.common" libs/java/common-domain/src --include="*.java"
```

---

## Rollback Plan

If issues arise:

1. Revert file deletions (git checkout)
2. Remove dependency from fleet-simulator/pom.xml
3. Remove implicitDependencies from project.json files
4. Clean Maven cache: `mvn dependency:purge-local-repository`

---

## Common Issues and Solutions

### Issue 1: Maven Can't Find common-domain

**Symptom**: `Could not find artifact com.greyorange.warehouse:common-domain`

**Solution**: Build and install common-domain first:
```bash
cd libs/java/common-domain && mvn clean install
```

### Issue 2: Circular Dependencies

**Symptom**: Classes in common-domain import from fleet-simulator

**Solution**: These types should NOT have dependencies on fleet-simulator. If they do:
- Extract only pure value objects
- Leave types with external dependencies in fleet-simulator

### Issue 3: Test Failures Due to Missing Imports

**Symptom**: Tests fail with `cannot find symbol`

**Solution**: Update test file imports the same way as main sources.

---

## Files Changed Summary

| Action | Path |
|--------|------|
| CREATE | `libs/java/common-domain/src/main/java/com/greyorange/common/coordinates/*.java` (6 files) |
| CREATE | `libs/java/common-domain/src/main/java/com/greyorange/common/types/*.java` (5 files) |
| DELETE | `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/` |
| UPDATE | `apps/fleet-simulator/pom.xml` (add dependency) |
| UPDATE | `apps/fleet-simulator/project.json` (add implicitDependencies) |
| UPDATE | `apps/put-service/pom.xml` (add dependency) |
| UPDATE | `apps/put-service/project.json` (add implicitDependencies) |
| UPDATE | ~50-100 Java files in fleet-simulator (import changes) |

---

## Success Criteria

1. ✅ `nx build common-domain` succeeds
2. ✅ `nx build fleet-simulator` succeeds
3. ✅ `nx build put-service` succeeds
4. ✅ `nx test fleet-simulator` passes all 130 tests
5. ✅ `nx graph` shows fleet-simulator → common-domain dependency
6. ✅ No references to old package `dessimulator.domain.common` remain
7. ✅ `apps/fleet-simulator/src/.../domain/common/` directory is deleted

---

## Next Phases

| Phase | Goal |
|-------|------|
| **Phase 3** | Add gRPC proto definitions in `libs/proto/` |
| **Phase 4** | Generate Java stubs in `libs/java/grpc-stubs/` |
| **Phase 5** | Extract Put domain logic to `apps/put-service/` |
| **Phase 6** | Wire fleet-simulator to call put-service via gRPC |

---

## References

- [Polyglot Monorepo Structure Design](../architecture/polyglot-monorepo-structure-design.md)
- [Microservices Decomposition Design](../architecture/microservices-decomposition-design.md)
- Parent POM: `libs/java/pom.xml`
- Current common types: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/`

