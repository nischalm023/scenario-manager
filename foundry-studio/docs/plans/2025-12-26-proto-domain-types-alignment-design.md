# Proto and Domain Types Alignment Design

**Date**: 2025-12-26
**Status**: Draft

## Problem Statement

The codebase has common types defined in two places:

1. **Proto types** (`libs/proto/src/grpc/common/v1/types.proto`): Wire format for gRPC
2. **Java domain types** (`libs/java/common-domain/`): Rich domain objects with behavior

Current issues:
- **Naming inconsistency**: Proto uses `x, y` while Java uses `row, col`
- **Unclear conversion ownership**: Where should proto↔domain converters live?
- **ID types implicit coupling**: Java ID types (RackId, ToteId) have implicit wire format contracts

## Decisions

### 1. Naming: Align proto with Java domain language

Proto will use `row, col` to match the domain model:

```protobuf
message GridCoordinate {
    int32 row = 1;
    int32 col = 2;
}
```

**Rationale**: The Java domain types are the richer, more established representation. Domain language should flow outward to wire format, not the reverse.

### 2. Separation: Domain types have no proto knowledge

Domain types in `common-domain` remain pure - no `toProto()` or `fromProto()` methods.

**Rationale**: `common-domain` is shared across all services. If a domain type knows about a specific service's proto, it creates coupling. Different services could have different wire formats for the same domain concept.

### 3. Converters: Centralized in common-messaging

Common proto converters (GridCoordinate, SystemType) live in `common-messaging`:

```
libs/java/common-domain/
  └── GridCoordinate.java                    ← Pure, no proto knowledge

libs/java/common-messaging/
  └── proto/
      └── ProtoConverters.java               ← Shared converters for common types

apps/put-service/                            ← imports common-messaging
apps/fleet-simulator/                        ← imports common-messaging
```

Converter pattern:
```java
public final class ProtoConverters {
    private ProtoConverters() {}

    public static GridCoordinate toDomain(
            com.greyorange.grpc.common.v1.GridCoordinate proto) {
        return new GridCoordinate(proto.getRow(), proto.getCol());
    }

    public static com.greyorange.grpc.common.v1.GridCoordinate toProto(
            GridCoordinate domain) {
        return com.greyorange.grpc.common.v1.GridCoordinate.newBuilder()
            .setRow(domain.row())
            .setCol(domain.col())
            .build();
    }
}
```

### 4. ID types: Primitives on wire with documentation

ID types (RackId, ToteId, StorageSpaceId, StationId) remain Java-only. Wire format uses primitives with documentation:

```protobuf
message InitializeInventoryRequest {
    // Maps to SystemType enum in common-domain
    SystemType system_type = 1;

    // Rack positions - each maps to RackId(int) in Java
    repeated int32 rack_ids = 2;

    // Storage positions - each maps to StorageSpaceId(String) in Java
    repeated string storage_space_ids = 3;
}
```

**Rationale**:
- Monorepo with same team - implicit coupling is manageable
- Changing an ID's underlying type is a breaking change regardless of wire format
- Wrapper messages for every ID type adds verbosity without benefit

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      common-domain                          │
│  GridCoordinate, RackId, ToteId, SystemType, etc.          │
│  (Pure Java, no proto dependencies)                         │
└─────────────────────────────────────────────────────────────┘
                              │
                    used by   │
                              ▼
┌─────────────────────┐              ┌─────────────────────┐
│   fleet-simulator   │              │    put-service      │
│                     │   gRPC       │                     │
│  dto/ProtoConverters┼──────────────┼─▶domain/ProtoConvert│
│                     │              │                     │
└─────────────────────┘              └─────────────────────┘
         │                                    │
         │ imports                            │ imports
         ▼                                    ▼
┌─────────────────────────────────────────────────────────────┐
│                       grpc-stubs                            │
│  Generated from libs/proto/ definitions                     │
└─────────────────────────────────────────────────────────────┘
```

## Changes Required

### 1. Update proto naming (libs/proto/)

**File**: `libs/proto/src/grpc/common/v1/types.proto`

```protobuf
// Before
message GridCoordinate {
    int32 x = 1;
    int32 y = 2;
}

// After
message GridCoordinate {
    int32 row = 1;
    int32 col = 2;
}
```

### 2. Regenerate gRPC stubs

```bash
cd libs/java/grpc-stubs
mvn clean compile
```

### 3. Create/update converters in put-service

**File**: `apps/put-service/src/main/java/com/greyorange/put/domain/ProtoConverters.java`

```java
package com.greyorange.put.domain;

import com.greyorange.greymatter.common.coordinates.GridCoordinate;
import com.greyorange.greymatter.common.types.SystemType;

public final class ProtoConverters {
    private ProtoConverters() {}

    public static GridCoordinate toDomain(
            com.greyorange.grpc.common.v1.GridCoordinate proto) {
        return new GridCoordinate(proto.getRow(), proto.getCol());
    }

    public static com.greyorange.grpc.common.v1.GridCoordinate toProto(
            GridCoordinate domain) {
        return com.greyorange.grpc.common.v1.GridCoordinate.newBuilder()
            .setRow(domain.row())
            .setCol(domain.col())
            .build();
    }

    public static SystemType toDomain(
            com.greyorange.grpc.common.v1.SystemType proto) {
        return switch (proto) {
            case SYSTEM_TYPE_RTP -> SystemType.RTP;
            case SYSTEM_TYPE_RELAY -> SystemType.RELAY;
            case SYSTEM_TYPE_UNSPECIFIED, UNRECOGNIZED ->
                throw new IllegalArgumentException("Invalid system type: " + proto);
        };
    }

    public static com.greyorange.grpc.common.v1.SystemType toProto(
            SystemType domain) {
        return switch (domain) {
            case RTP -> com.greyorange.grpc.common.v1.SystemType.SYSTEM_TYPE_RTP;
            case RELAY -> com.greyorange.grpc.common.v1.SystemType.SYSTEM_TYPE_RELAY;
        };
    }
}
```

### 4. Create/update converters in fleet-simulator

**File**: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/dto/ProtoConverters.java`

```java
package com.greyorange.dessimulator.dto;

// Same converter implementation as put-service
// Separate because no shared library dependency between services
```

### 5. Update existing code to use converters

Search for direct proto field access and replace with converter calls.

## What We're NOT Doing

- **MapStruct**: Overkill for 2 types. Revisit if converter count grows significantly.
- **Proto wrapper messages for IDs**: Unnecessary verbosity for monorepo context.
- **Domain types knowing about proto**: Would couple common-domain to specific service APIs.

## Testing

1. Update any tests that use proto GridCoordinate with x/y to use row/col
2. Verify gRPC calls work end-to-end after regenerating stubs
3. Run existing integration tests

## Migration Notes

This is a **breaking change** for the proto wire format. Since this is internal and pre-production:
- No backwards compatibility needed
- Just regenerate stubs and update all usages

If this were a production API, we'd need a versioned migration strategy.
