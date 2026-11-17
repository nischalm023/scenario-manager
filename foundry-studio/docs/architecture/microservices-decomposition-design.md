# Microservices Decomposition Design

**Status**: Early Design / Introspection
**Created**: 2025-11-18
**Last Updated**: 2025-11-18

## Executive Summary

This document captures the architectural analysis and design for decomposing the queue simulator monolith into microservices. The primary insight is that the **Fleet Simulator (Desmo-J) becomes a simulation layer** on top of stateful base services.

### Key Architectural Decision

```
Fleet Simulator = Simulation Layer (timing, scheduling, process orchestration)
Base Services = State Management (inventory, relay, stations, pick/put logic)
```

---

## Target Service Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Work Layer                          │
├─────────────────┬─────────────────┬─────────────────────────┤
│  Pick Service   │   Put Service   │   Station Service       │
│                 │                 │                         │
│ - Order select  │ - Storage pos   │ - Station registry      │
│ - Task generate │   selection     │ - Availability tracking │
│ - Pick workflow │ - Back-to-store │ - Queue depth           │
│ - PPF optimize  │ - Replenishment │ - Operator assignment   │
│                 │ - Popularity    │                         │
└────────┬────────┴────────┬────────┴────────────┬────────────┘
         │                 │                     │
         ▼                 ▼                     ▼
┌─────────────────────────────────────────────────────────────┐
│                    Shared Services                          │
├─────────────────┬─────────────────┬─────────────────────────┤
│    Inventory    │     Relay       │       Layout            │
│    Service      │    Service      │      Service            │
│                 │                 │                         │
│ - Stock levels  │ - Physical pos  │ - Warehouse geometry    │
│ - Reservations  │ - Tote queues   │ - Pathfinding           │
│ - TCC pattern   │ - Capacity      │ - Zones                 │
└─────────────────┴─────────────────┴─────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                 Fleet Simulator (Desmo-J)                   │
│                                                             │
│  - Robot processes (VTM, HTM, RTP)                          │
│  - Time simulation (hold, scheduling)                       │
│  - Local blocking mirrors                                   │
│  - Metrics emission                                         │
└─────────────────────────────────────────────────────────────┘
```

---

## Service Definitions

### 1. Station Service

**Purpose**: Single source of truth for station state. Both Pick and Put query/reserve stations.

**Owns**:
- Station registry (positions, types, bin counts)
- Availability state (IDLE, ACTIVE, RESERVED, OFFLINE)
- Queue depth tracking
- Operator assignments

**Station Types**:
- `PICK` - Outbound picking operations
- `PUT` - Inbound put-away/replenishment
- `HYBRID` - Can do both (future)

**Key Operations**:
- `FindAvailableStation(type, zone)` - Query available station
- `ReserveStation(stationId, operationId)` - Reserve for operation
- `ReleaseStation(stationId)` - Release after operation
- `UpdateStationState(stationId, update)` - State updates from simulator

**Events Published**:
- `StationBecameAvailableEvent`
- `StationQueueChangedEvent`

---

### 2. Pick Service

**Purpose**: Order fulfillment - selecting orders, generating tasks, orchestrating picks.

**Owns**:
- Order lifecycle (PENDING -> RESERVED -> BINNED -> COMPLETED)
- Pick task generation
- Bin assignments
- Order selection strategies (PPF, TPQ, etc.)

**Key Operations**:
- `SubmitOrders(batch)` - Submit new orders
- `GetNextPickTasks(zoneId, robotId, activeSpaces)` - Get tasks for robot
- `CompletePickTask(taskId, orderId)` - Mark task complete
- `AssignOrderToBin(orderId, stationId)` - Bin assignment

**Coordination**:
- Calls **Inventory Service** for TCC reservation
- Calls **Station Service** for station reservation
- Subscribes to task completion for order lifecycle

**Events Published**:
- `TaskCompletedEvent` (consumed by Put Service for popularity)
- `OrderCompletedEvent`

---

### 3. Put Service

**Purpose**: Storage optimization - selecting where to put items, back-to-store logic, replenishment.

**Owns**:
- Storage position selection algorithms
- Popularity tracking and heatmaps
- Replenishment task management
- Put strategies (Greedy, MinWeightedCycleDistance, PopularityBased)

**Key Operations**:
- `SelectStoragePosition(storageSpaceId, zone, skusOnBoard)` - Back-to-store position
- `GetNextPutTasks(zoneId, robotId)` - Get replenishment tasks
- `CompletePutTask(taskId)` - Mark put complete
- `UpdatePopularityScores()` - Recalculate heatmap

**Subscribes to**:
- `TaskCompletedEvent` from Pick Service (updates SKU popularity)

**Events Published**:
- `StoragePositionSelectedEvent`
- `ReplenishmentCompletedEvent`
- `PopularityScoresUpdatedEvent`

---

### 4. Inventory Service

**Purpose**: Stock management with distributed transaction support.

**Owns**:
- Stock levels per storage space
- Reservations (soft and hard)
- Item locations index

**Key Pattern**: **TCC (Try-Confirm-Cancel)**

```
Work Service                    Inventory Service
     │                                │
     │ TryReserve(orderId, items)     │
     ├───────────────────────────────►│
     │                                │ Create reservation with TTL=30s
     │ ReservationToken + expiry      │
     │◄───────────────────────────────┤
     │                                │
     │ ... process order ...          │
     │                                │
     │ Confirm(reservationToken)      │
     ├───────────────────────────────►│
     │                                │ Make permanent
```

**Key Operations**:
- `TryReserve(orderId, items, ttlSeconds)` - Tentative reservation
- `Confirm(reservationId)` - Make permanent
- `Cancel(reservationId)` - Release reservation
- `ConsumeStock(orderId, sku, quantity)` - Consume during pick
- `FindBestStorageSpaces(remainingItems, activeSpaces)` - Set cover algorithm

**TTL Semantics**:
- Default 30 seconds for tentative reservations
- Auto-cancel on expiry
- Publish `ReservationExpiredEvent` for cleanup

**Events Published**:
- `InventoryReservedEvent`
- `InventoryConsumedEvent`
- `ReservationExpiredEvent`

---

### 5. Relay Service

**Purpose**: Relay position state management (physical positions, capacity, tote queues).

**Key Insight**: Relay positions can live **outside the simulator** as a stateful service. The simulator handles blocking behavior locally.

**Owns**:
- Physical relay positions (coordinates)
- Capacity per position
- Tote queues (FIFO)
- Occupancy state

**Key Operations**:
- `GetRelayPosition(relayId)` - Query state
- `FindAvailableRelay(zoneId, preferEmpty/preferOccupied)` - Find relay
- `DeliverTote(relayId, toteId, robotId)` - VTM delivers
- `PickupTote(relayId, robotId)` - HTM picks up (FIFO)

**Blocking Handling**:
- Service returns `RELAY_FULL` error
- Simulator implements local blocking with event-driven wake-up
- Subscribe to `TotePickedUpEvent` to know when capacity available

**Events Published**:
- `ToteDeliveredEvent`
- `TotePickedUpEvent`
- `RelayCapacityChangedEvent`

---

### 6. Layout Service

**Purpose**: Warehouse geometry, pathfinding, zone management.

**Owns**:
- Warehouse map (grid, highways, rack blocks)
- Coordinate conversion
- PathFinder (A* algorithm)
- Zone definitions

**Key Operations**:
- `GetWarehouseLayout()` - Full layout
- `FindPath(from, to, carrying)` - A* pathfinding
- `GetRackPositionsInZone(zoneId)` - Available positions
- `GetNearestStation(position)` - Closest station

**Performance Note**: PathFinder called 100x/sec per robot. Consider:
- Embedding PathFinder library in Fleet Simulator
- Local caching with event-driven invalidation

---

### 7. Fleet Simulator (Desmo-J Layer)

**Purpose**: Time simulation, process scheduling, robot orchestration.

**What it does**:
- Runs Desmo-J experiment with robot processes
- Handles `hold()` for time advancement
- Implements local blocking mirrors (driven by service events)
- Emits metrics events

**What it does NOT do**:
- Own state (queries services)
- Make business decisions (calls services)

**Process Types**:
- `DesmojVtmProcess` - Vertical Tote Mover
- `DesmojHtmProcess` - Horizontal Tote Mover
- `DesmojRtpRobotProcess` - Racks-to-Picker robot

**Interaction Pattern**:
```java
// VTM Process pseudo-code
while (true) {
    // Query Pick Service for tasks
    PickTaskBatch tasks = pickService.getNextPickTasks(zoneId, robotId);

    // Execute pick cycle
    for (task : tasks) {
        // Query Layout for path
        path = layoutService.findPath(currentPos, rackPos);
        hold(path.travelTime);

        // Deliver to relay (may block)
        relayService.deliverTote(relayId, toteId);

        // Query Put Service for return position
        storagePos = putService.selectStoragePosition(toteId, zone, skus);

        // Return to rack
        hold(returnTravelTime);

        // Complete task
        pickService.completePickTask(taskId);
    }
}
```

---

## Critical Coupling Points (Current Monolith)

### 1. DesmojWarehouseModel (396 lines)

**Problem**: Takes 14 constructor parameters, initializes everything.

**Solution**: Services self-initialize, simulator queries them.

### 2. WarehouseInventory (300+ lines)

**Problem**: Shared mutable state, ACID semantics needed.

**Solution**: TCC pattern with TTL-based reservations.

### 3. activeStorageSpaces Set

**Problem**: Fleet mutates, Work reads (cross-service dependency).

**Solution**: Event-driven updates:
- Fleet publishes `ToteDeliveredToRelay`, `ToteReturnedToRack`
- Pick Service subscribes and maintains local set

### 4. orderTaskMapping

**Problem**: Shared mutable map between processes and OrderManager.

**Solution**: Pick Service owns this entirely, exposes via gRPC.

### 5. MetricsManager (reflection routing)

**Problem**: Cross-cutting, implicit coupling.

**Solution**: Replace with Kafka event bus, each service subscribes locally.

---

## Communication Patterns

### gRPC for Synchronous Operations

- Task fetching: Pick/Put Service
- State queries: Station, Relay, Layout
- Mutations: Inventory TCC, Relay deliver/pickup

### Kafka for Events

- Task completion (Pick → Put for popularity)
- State changes (Station available, Relay capacity)
- Metrics (all services → Metrics aggregator)

### Local Caching Strategy

Services that need high-frequency reads (Relay status, Station availability):

```java
public class CachedRelayClient {
    private final Map<String, RelayPositionDTO> cache;
    private final KafkaConsumer<String, RelayEvent> events;

    // Cache populated from events
    @KafkaListener(topics = "relay-events")
    public void onRelayEvent(RelayEvent event) {
        updateCache(event);
    }

    // Reads from cache
    public RelayPositionDTO getRelay(String id) {
        return cache.get(id);
    }

    // Writes go through gRPC (source of truth)
    public DeliverResponse deliverTote(DeliverRequest req) {
        return grpcClient.deliverTote(req);
    }
}
```

---

## Inventory ACID Pattern: TCC Deep-Dive

### Why TCC for This System

- **Short-lived reservations**: 10s bin management → pick cycle
- **High throughput**: 100+ robots, 1000+ tasks/hour
- **All-or-nothing**: Order needs all items or none

### TTL Configuration

- **Default**: 30 seconds
- **Too short** (5s): Legitimate operations timeout
- **Too long** (5min): Inventory stuck on failures

### Failure Handling

```java
// In Pick Service
TryReserveResponse response = inventoryService.tryReserve(...);

if (!response.getSuccess()) {
    if ("Insufficient inventory".equals(response.getFailureReason())) {
        // Genuinely no stock - mark order BACKORDER
        order.setStatus(BACKORDER);
    } else if (response.getFailureReason().contains("timeout")) {
        // Network issue - retry with backoff
        retryQueue.add(order, Duration.ofSeconds(5));
    }
}
```

### Consumption Path

When robot picks item:
```java
// Fleet Simulator → Inventory Service
inventoryService.consumeStock(ConsumeRequest.newBuilder()
    .setOrderId(task.getOrderId())
    .setStorageSpaceId(task.getStorageSpaceId())
    .setSku(task.getSku())
    .setQuantity(task.getQuantity())
    .build());
```

---

## Concurrent Pick + Put (Future State)

### Station Types

```java
enum StationType {
    PICK,     // Outbound only
    PUT,      // Inbound only
    HYBRID    // Both operations
}
```

### Combined Tote Workflow

A single tote can carry both pick and put tasks:

```java
// Tote carries mixed workload
public class ToteWorkload {
    Tote tote;
    List<PickTaskDTO> pickTasks;
    List<PutTaskDTO> putTasks;
    List<StationVisit> optimizedRoute;  // Minimize travel
}
```

### HTM with Mixed Operations

```java
// HTM visits pick stations then put stations
for (PickTaskDTO task : workload.pickTasks) {
    navigateToStation(task.getStationId());
    processPickAtStation(task);
}

for (PutTaskDTO task : workload.putTasks) {
    navigateToStation(task.getStationId());
    processPutAtStation(task);
}
```

---

## Event Flow Diagram

```
┌─────────────┐     TaskCompletedEvent     ┌─────────────┐
│ Pick Service│ ─────────────────────────► │ Put Service │
│             │                            │             │
│             │ OrderCompletedEvent        │             │
│             │ ─────────────────────────► │ (popularity │
│             │                            │  tracking)  │
└──────┬──────┘                            └──────┬──────┘
       │                                          │
       │ ReserveStation                           │ ReserveStation
       ▼                                          ▼
┌───────────────────────────────────────────────────────────┐
│                    Station Service                        │
│                                                           │
│  StationAvailableEvent ─────────────────────────────────► │
└───────────────────────────────────────────────────────────┘
       │                                          │
       │ TryReserve/Confirm                       │ SelectPosition
       ▼                                          ▼
┌─────────────┐                            ┌─────────────┐
│  Inventory  │                            │   Layout    │
│  Service    │                            │  Service    │
└─────────────┘                            └─────────────┘
```

---

## Migration Path

### Phase 1: Define Interfaces (2-3 weeks)

1. Create gRPC .proto definitions for all services
2. Create Java interfaces matching gRPC contracts
3. Implement in-process adapters wrapping current code
4. Update monolith to use interfaces

**Result**: Code prepared for split, but still monolithic.

### Phase 2: Extract Layout Service (1-2 weeks)

1. Move WarehouseMap, PathFinder, coordinate conversion
2. Deploy as separate Spring Boot service
3. Fleet Simulator calls via gRPC

**Risk**: Low (stateless algorithms)

### Phase 3: Extract Station Service (1-2 weeks)

1. Move Station, StationManager
2. Add reservation logic
3. Both Pick and Put call via gRPC

**Risk**: Low (simple state)

### Phase 4: Extract Relay Service (2 weeks)

1. Move RelayPosition, RelayPositionManager
2. Add event publishing
3. Implement blocking bridge in simulator

**Risk**: Medium (event-driven blocking)

### Phase 5: Extract Inventory Service with TCC (3-4 weeks)

1. Add TCC semantics to WarehouseInventory
2. Deploy as separate service
3. Pick Service calls via gRPC
4. Add TTL cleanup scheduler

**Risk**: High (ACID semantics)

### Phase 6: Split Pick and Put Services (3-4 weeks)

1. Separate OrderManager → Pick Service
2. Separate StoragePositionManager → Put Service
3. Add popularity event subscription
4. Configure Kafka topics

**Risk**: High (complex coordination)

### Phase 7: Event Bus Migration (2-3 weeks)

1. Replace callbacks with Kafka events
2. Replace MetricsManager reflection with subscriptions
3. Add event schemas (Protobuf)

**Risk**: High (wide impact)

---

## gRPC Proto Definitions (Reference)

### inventory.proto

```protobuf
service InventoryService {
    rpc TryReserve(TryReserveRequest) returns (TryReserveResponse);
    rpc Confirm(ConfirmRequest) returns (ConfirmResponse);
    rpc Cancel(CancelRequest) returns (CancelResponse);
    rpc ConsumeStock(ConsumeRequest) returns (ConsumeResponse);
    rpc GetAvailableStock(StockQuery) returns (StockResponse);
    rpc FindBestStorageSpaces(SetCoverRequest) returns (SetCoverResponse);
}
```

### relay.proto

```protobuf
service RelayService {
    rpc GetRelayPosition(RelayId) returns (RelayPositionDTO);
    rpc GetRelaysByZone(ZoneId) returns (RelayPositionList);
    rpc FindAvailableRelay(FindRelayRequest) returns (RelayPositionDTO);
    rpc DeliverTote(DeliverRequest) returns (DeliverResponse);
    rpc PickupTote(PickupRequest) returns (PickupResponse);
    rpc WatchRelayStatus(ZoneId) returns (stream RelayStatusUpdate);
}
```

### station.proto

```protobuf
service StationService {
    rpc ListStations(StationFilter) returns (StationList);
    rpc GetStation(StationId) returns (StationDTO);
    rpc FindAvailableStation(FindStationRequest) returns (StationDTO);
    rpc ReserveStation(ReserveStationRequest) returns (ReserveResponse);
    rpc ReleaseStation(ReleaseStationRequest) returns (ReleaseResponse);
    rpc UpdateStationState(StationStateUpdate) returns (UpdateResponse);
}
```

### pick.proto

```protobuf
service PickService {
    rpc SubmitOrders(OrderBatch) returns (SubmissionResult);
    rpc GetOrderStatus(OrderId) returns (OrderStatus);
    rpc GetNextPickTasks(GetTasksRequest) returns (PickTaskBatch);
    rpc CompletePickTask(TaskCompletionRequest) returns (TaskCompletionResponse);
    rpc AssignOrderToBin(BinAssignmentRequest) returns (BinAssignmentResponse);
}
```

### put.proto

```protobuf
service PutService {
    rpc SelectStoragePosition(SelectPositionRequest) returns (StoragePositionDTO);
    rpc GetNextPutTasks(GetPutTasksRequest) returns (PutTaskBatch);
    rpc CompletePutTask(PutTaskCompletionRequest) returns (PutTaskCompletionResponse);
    rpc UpdatePopularityScores(PopularityUpdateRequest) returns (UpdateResponse);
}
```

---

## Open Questions

1. **PathFinder Latency**: Embed in Fleet Simulator or accept gRPC overhead?
2. **Event Schema Evolution**: How to version Protobuf events?
3. **Saga Timeout**: What happens if Pick Service crashes after Inventory confirms?
4. **Hybrid Stations**: How to prioritize pick vs put at same station?
5. **Multi-Simulator**: Can we run A/B test simulators against same services?

---

## References

- Current inventory implementation: `domain/inventory/WarehouseInventory.java`
- Current order processing: `domain/order/OrderProcessor.java`
- Current relay management: `domain/robot/RelayPositionManager.java`
- Zone load balancing design: `docs/designs/2025-11-18-zone-load-balancing-design.md`
- Storage position simplification: `docs/designs/storage-position-simplification-design.md`

---

## Context for Future Sessions

When resuming this work, key files to review:

1. **Simulation orchestration**: `simulator/DesmojWarehouseModel.java` (396 lines)
2. **Inventory state**: `domain/inventory/WarehouseInventory.java` (270 lines)
3. **Order processing**: `domain/order/OrderProcessor.java` (100 lines)
4. **Relay management**: `domain/robot/RelayPositionManager.java`
5. **Zone management**: `domain/robot/ZoneManager.java`
6. **Robot processes**: `simulator/orchestrator/Desmoj*Process.java`

Key architectural decisions already made:
- TCC pattern for inventory (not 2PC or Saga)
- Relay Service as external stateful service
- Simulator handles blocking locally with event-driven wake-up
- Pick/Put/Station as separate services
- gRPC for sync, Kafka for events
