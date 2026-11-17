# Phase 3: Put Service gRPC Integration

**Status**: COMPLETED  
**Completed**: 2024-12-26  
**Depends on**: Phase 2 (Common Domain Extraction) - COMPLETED  
**Author**: Architecture Team

---

## Executive Summary

Implemented full gRPC infrastructure and migrated PUT logic from `fleet-simulator` to `put-service`. The put-service now operates as an independent microservice with proto-based DTOs, callable via gRPC or in-process. Fleet-simulator consumes put-service as a library dependency for in-process mode.

---

## Completed Work

### 1. gRPC Infrastructure Setup

#### 1.1 Proto Definitions Created

**Common types** (`libs/proto/src/grpc/common/v1/types.proto`):
```protobuf
message GridCoordinate {
    int32 x = 1;  // column
    int32 y = 2;  // row
}

enum SystemType {
    SYSTEM_TYPE_UNSPECIFIED = 0;
    SYSTEM_TYPE_RTP = 1;
    SYSTEM_TYPE_RELAY = 2;
}
```

**Put service** (`libs/proto/src/grpc/put/v1/put.proto`):
```protobuf
service PutService {
    rpc InitializeInventory(InitializeInventoryRequest) returns (InitializeInventoryResponse);
}

message SkuInfo { ... }
message StorageSpaceInfo { ... }
message StationInfo { ... }
message InventoryAssignment { ... }
message InitializeInventoryRequest { ... }
message InitializeInventoryResponse { ... }
```

#### 1.2 gRPC Stubs Module

Created `libs/java/grpc-stubs/`:
- Generates Java classes from proto definitions
- Uses `protobuf-maven-plugin` with gRPC plugin
- Produces both message classes and gRPC service stubs

#### 1.3 Parent POM Updates

Added to `libs/java/pom.xml`:
```xml
<properties>
    <grpc.version>1.60.0</grpc.version>
    <protobuf.version>3.25.1</protobuf.version>
    <grpc-spring-boot.version>2.15.0.RELEASE</grpc-spring-boot.version>
</properties>

<dependencyManagement>
    <dependencies>
        <dependency>
            <groupId>io.grpc</groupId>
            <artifactId>grpc-bom</artifactId>
            <version>${grpc.version}</version>
            <type>pom</type>
            <scope>import</scope>
        </dependency>
        <!-- grpc-server/client starters, protobuf-java, grpc-stubs -->
    </dependencies>
</dependencyManagement>
```

---

### 2. Put Service Refactoring

#### 2.1 Proto-Only DTO Strategy

Adopted "proto-only" approach following Google's best practices:
- **No separate Java DTOs** - proto-generated classes serve as DTOs
- **Single source of truth** - `.proto` files define all service contracts
- **Automatic serialization** - protobuf handles JSON and binary formats

#### 2.2 Migrated Classes

| Class | Purpose |
|-------|---------|
| `PutProcessor` | Orchestrates inventory initialization using proto request/response |
| `PutAlgorithm` | Enum for algorithm selection (greedy, min_weighted_cycle_distance) |
| `GridPopularityManager` | Proto-based popularity scoring for stateless operations |
| `AbstractPutStrategy` | Base class using proto DTOs (SkuInfo, StorageSpaceInfo) |
| `GreedyPutStrategy` | Fast O(n*m) placement algorithm |
| `MinWeightedCycleDistancePutStrategy` | LP optimization using OR-Tools |
| `PutGrpcService` | gRPC server implementation |
| `PutController` | REST endpoint with JSON/proto conversion |

#### 2.3 Key Refactoring Patterns

**Proto to common-domain conversion:**
```java
// Proto GridCoordinate uses (x, y) where x=col, y=row
// Common-domain GridCoordinate uses (row, col)
GridCoordinate commonCoord = new GridCoordinate(
    protoCoord.getY(),  // row
    protoCoord.getX()   // col
);
```

**Proto SystemType handling:**
```java
private SystemType toCommonSystemType(com.greyorange.grpc.common.v1.SystemType protoType) {
    return switch (protoType) {
        case SYSTEM_TYPE_RTP -> SystemType.RTP;
        case SYSTEM_TYPE_RELAY -> SystemType.RELAY;
        default -> SystemType.RTP;
    };
}
```

---

### 3. Fleet-Simulator Integration

#### 3.1 Dependency Addition

Added to `apps/fleet-simulator/pom.xml`:
```xml
<dependency>
    <groupId>com.greyorange.greymatter</groupId>
    <artifactId>put-service</artifactId>
    <version>${project.version}</version>
</dependency>
```

#### 3.2 New Adapter Layer

**PutServiceAdapter** - Converts fleet-simulator domain objects to proto:
```java
public void initializeInventory(
    SystemType systemType,
    String algorithmName,
    ItemCatalog itemCatalog,
    List<? extends StorageSpace> storageSpaces,
    StorageSpaceManager storageSpaceManager,
    WarehouseMap map,
    ZoneManager zoneManager,
    WarehouseInventory inventory,
    GridPopularityManager gridPopularityManager);
```

**PutServiceClient** - gRPC client for remote mode:
```java
public void initializeInventory(
    SystemType systemType,
    String algorithm,
    ItemCatalog itemCatalog,
    List<? extends StorageSpace> storageSpaces,
    StorageSpaceManager storageSpaceManager,
    WarehouseMap map,
    ZoneManager zoneManager,
    WarehouseInventory inventory);
```

#### 3.3 Local GridPopularityManager

Fleet-simulator retains its own `GridPopularityManager` for runtime operations:
- Back-to-store strategies
- Order selection (MaximizeTPQ)
- Metrics collection

This version works with `WarehouseMap` and common-domain `GridCoordinate`.

#### 3.4 WarehouseBuilder Update

```java
// Old: Direct PutProcessor usage
PutProcessor putProcessor = new PutProcessor(...);
putProcessor.initializeInventory();

// New: Adapter pattern
PutServiceAdapter putServiceAdapter = new PutServiceAdapter();
putServiceAdapter.initializeInventory(
    config.systemType(),
    config.algorithmConfig().putAlgorithm().toLowerCase(),
    itemCatalog,
    storageSpaces,
    storageSpaceManager,
    map,
    zoneManager,
    warehouseInventory,
    gridPopularityManager);
```

---

### 4. Build Configuration

#### 4.1 Put Service Spring Boot Plugin

Configured to produce both library and executable jars:
```xml
<plugin>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-maven-plugin</artifactId>
    <configuration>
        <classifier>exec</classifier>
    </configuration>
</plugin>
```

This produces:
- `put-service-1.0.0-SNAPSHOT.jar` - Library jar (usable as dependency)
- `put-service-1.0.0-SNAPSHOT-exec.jar` - Executable Spring Boot jar

---

## Files Changed Summary

### Created Files

| Path | Description |
|------|-------------|
| `libs/proto/src/grpc/common/v1/types.proto` | Common proto types |
| `libs/proto/src/grpc/put/v1/put.proto` | Put service proto definitions |
| `libs/java/grpc-stubs/pom.xml` | gRPC stubs module |
| `libs/java/grpc-stubs/project.json` | NX configuration |
| `apps/put-service/src/main/java/.../api/PutGrpcService.java` | gRPC server |
| `apps/fleet-simulator/src/.../put/PutServiceAdapter.java` | In-process adapter |
| `apps/fleet-simulator/src/.../client/PutServiceClient.java` | gRPC client |

### Modified Files

| Path | Change |
|------|--------|
| `libs/java/pom.xml` | Added gRPC versions and dependency management |
| `apps/put-service/pom.xml` | Added gRPC dependencies, classifier config |
| `apps/fleet-simulator/pom.xml` | Added put-service dependency |
| `apps/put-service/src/.../PutProcessor.java` | Refactored to use proto types |
| `apps/put-service/src/.../GridPopularityManager.java` | Proto-based version |
| `apps/put-service/src/.../putstrategies/*.java` | Proto-based strategies |
| `apps/fleet-simulator/src/.../WarehouseBuilder.java` | Uses PutServiceAdapter |
| `apps/fleet-simulator/src/.../GridPopularityManager.java` | Local WarehouseMap version |

### Deleted Files

| Path | Reason |
|------|--------|
| `apps/fleet-simulator/src/.../put/PutProcessor.java` | Moved to put-service |
| `apps/fleet-simulator/src/.../put/PutAlgorithm.java` | Moved to put-service |
| `apps/fleet-simulator/src/.../put/putstrategies/*.java` | Moved to put-service |
| `apps/fleet-simulator/src/test/.../PutAlgorithmTest.java` | Moved to put-service |
| `apps/fleet-simulator/src/test/.../PutStrategyTest.java` | Moved to put-service |

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                         fleet-simulator                              │
│  ┌─────────────────┐  ┌──────────────────┐  ┌───────────────────┐   │
│  │ WarehouseBuilder│  │PutServiceAdapter │  │GridPopularityMgr  │   │
│  │                 │──│                  │  │(local, runtime)   │   │
│  └─────────────────┘  └────────┬─────────┘  └───────────────────┘   │
│                                │ converts domain → proto             │
│                                ▼                                     │
│  ┌─────────────────────────────────────────────────────────────────┐│
│  │                     put-service (in-process)                    ││
│  │  ┌─────────────┐  ┌────────────────┐  ┌─────────────────────┐  ││
│  │  │PutProcessor │──│ PutStrategies  │──│ GridPopularityMgr   │  ││
│  │  │(proto I/O)  │  │ (proto DTOs)   │  │ (proto-based)       │  ││
│  │  └─────────────┘  └────────────────┘  └─────────────────────┘  ││
│  └─────────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────────┘

                    OR (future gRPC mode)

┌─────────────────────────────────────────────────────────────────────┐
│                         fleet-simulator                              │
│  ┌─────────────────┐  ┌──────────────────┐                          │
│  │ WarehouseBuilder│  │PutServiceClient  │                          │
│  │                 │──│ (gRPC)           │                          │
│  └─────────────────┘  └────────┬─────────┘                          │
└────────────────────────────────│────────────────────────────────────┘
                                 │ gRPC call
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     put-service (standalone)                         │
│  ┌─────────────────┐  ┌────────────────┐  ┌─────────────────────┐   │
│  │ PutGrpcService  │──│ PutProcessor   │──│ PutStrategies       │   │
│  │ (gRPC server)   │  │                │  │                     │   │
│  └─────────────────┘  └────────────────┘  └─────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Verification

### Build Commands

```bash
# Build in order (or NX handles dependencies)
npx nx build grpc-stubs
npx nx build put-service
npx nx build fleet-simulator

# Or build all
npx nx run-many --target=build --projects=grpc-stubs,put-service,fleet-simulator
```

### Test Commands

```bash
npx nx test fleet-simulator
npx nx test put-service
```

---

## Known Limitations

1. **GridPopularityManager Duplication**: Fleet-simulator has its own copy for runtime operations. Consider extracting to shared library in future phase.

2. **In-Process Mode Only**: gRPC client exists but fleet-simulator defaults to in-process mode. Feature flag configuration needed for gRPC mode.

3. **Test Coverage**: Some tests moved from fleet-simulator to put-service; additional integration tests recommended.

---

## Configuration Options

### In-Process Mode (Default)

No additional configuration needed. Fleet-simulator uses `PutServiceAdapter` which directly calls `PutProcessor`.

### gRPC Mode (Future)

Add to `application.yml`:
```yaml
put-service:
  mode: grpc  # or in-process
  grpc:
    host: localhost
    port: 9090
```

---

## Next Phases

| Phase | Goal |
|-------|------|
| **Phase 4** | Add feature flag for gRPC vs in-process mode |
| **Phase 5** | Extract OrderSelection logic to order-service |
| **Phase 6** | Add Kubernetes deployment configurations |
| **Phase 7** | Implement service mesh (Istio) for production |

---

## References

- [Microservices Decomposition Design](../architecture/microservices-decomposition-design.md)
- [Polyglot Monorepo Structure Design](../architecture/polyglot-monorepo-structure-design.md)
- [Phase 2: Common Domain Extraction](./2024-12-23-phase2-common-domain-extraction.md)
- Proto definitions: `libs/proto/src/grpc/`
- gRPC stubs: `libs/java/grpc-stubs/`

