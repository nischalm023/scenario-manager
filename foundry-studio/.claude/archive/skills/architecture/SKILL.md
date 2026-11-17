---
name: architecture
description: Use when understanding service boundaries and communication patterns. Overview only. For gRPC implementation details, use grpc-patterns skill.
---

# Microservices Architecture

---

## Service Boundaries

| Service | Purpose | Tech | Ports |
|---------|---------|------|-------|
| **envoy** | API Gateway, routes frontend requests | Envoy Proxy | 8080 (HTTP), 9901 (admin) |
| **fleet-simulator** | Discrete event simulation (Desmo-J), robot orchestration | Java 21, Spring Boot | 8085 (REST) |
| **layout-service** | Warehouse layout, map visualization | Java 21, Spring Boot | 8082 (REST), 9091 (gRPC) |
| **navigation-service** | A* pathfinding, session-based path computation | Java 21, Spring Boot | 8083 (REST), 9092 (gRPC) |
| **put-service** | Storage position selection, put algorithms | Java 21, Spring Boot | 8081 (REST), 9090 (gRPC) |
| **frontend** | Web UI | React, TypeScript | 5173 (dev) |

---

## Communication Patterns

**Frontend → Envoy → Services:**
```
Frontend:4200 → Envoy:8080 ─┬─ /api/warehouse-map/* → Layout-Service:8082
                            └─ /api/*              → Fleet-Simulator:8085
```

**gRPC** (fleet-simulator → put-service):
- `InitializeInventory()` - Called during warehouse setup
- Proto definitions: `libs/proto/src/grpc/put/v1/put.proto`
- Generated stubs: `libs/java/grpc-stubs/`

**gRPC** (fleet-simulator → navigation-service):
- `CreateSession()` - Initialize session with map data (blocked/rack/highway/relay positions)
- `FindPath()` - A* pathfinding from start to goal (async stub to avoid Desmo-J blocking)
- `DeleteSession()` - Clean up session resources
- Proto definitions: `libs/proto/src/grpc/navigation/v1/navigation.proto`

---

## gRPC Channel Management

**common-messaging library** (`libs/java/common-messaging/`):
- `GrpcChannelFactory` - Centralized channel lifecycle (lazy creation, caching, auto-cleanup)
- `GrpcClientsConfig` - Configuration from `application.yml` (`grpc.clients.*`)
- `GrpcAutoConfiguration` - Spring Boot auto-configuration

**Spring Dependency Injection flow**:
```java
GrpcChannelFactory @Component (manages all gRPC channels)
  ↓ injected into
PutServiceClient @Service, NavigationServiceClient @Service
  ↓ injected into
SimulationService / ComparisonService
  ↓ passed to
Simulator → WarehouseBuilder → calls putServiceClient.initializeInventory()
         → DesmojSimulationEngine → calls navigationServiceClient.createSession()/findPath()
```

---

> **Note**: For step-by-step gRPC client implementation, see `.claude/skills/grpc-patterns/`

---

## Proto Types

**Common types** (`libs/proto/src/grpc/common/v1/types.proto`):
- `GridCoordinate` - (x=col, y=row) for proto, converts to common-domain (row, col)
- `SystemType` - RTP or RELAY enum

**Navigation types** (`libs/proto/src/grpc/navigation/v1/navigation.proto`):
- `Orientation` - NORTH/SOUTH/EAST/WEST robot facing direction
- `RobotType` - RTP/VTM/HTM for speed calculations
- `NavigationAction` - MOVE_FORWARD/BACKWARD, TURN_90_CW/CCW, TURN_180
- `PathWaypoint` - position, orientation, action, time delta

---

## Shared Libraries

**`libs/java/common-domain/`** - Language-agnostic value objects:
- `GridCoordinate` - (row, col) grid position
- `RackId`, `ToteId`, `StorageSpaceId` - Type-safe identifiers
- `SystemType` - RTP or RELAY enum

**`libs/java/common-messaging/`** - gRPC infrastructure:
- `GrpcChannelFactory` - Channel management
- `GrpcClientsConfig` - Configuration record
- `GrpcAutoConfiguration` - Spring Boot auto-config

---

## Future Services

See `docs/architecture/microservices-decomposition-design.md`:
- **Pick Service** - Order selection, task generation
- **Inventory Service** - Stock management with TCC pattern
- **Relay Service** - Relay position state
- **Station Service** - Station availability

---

## Key Documents

- `docs/architecture/microservices-decomposition-design.md` - Service boundaries
- `docs/architecture/polyglot-monorepo-structure-design.md` - Directory structure
- `docs/designs/reservation-services-design.md` - Reservation architecture

---

## Related Skills

- `.claude/skills/grpc-patterns/` - Detailed gRPC how-to
- `.claude/skills/commands/` - Service start commands
- `.claude/skills/debugging/` - Service troubleshooting
