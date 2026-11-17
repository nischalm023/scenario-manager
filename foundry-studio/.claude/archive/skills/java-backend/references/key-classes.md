# Key Classes

## Request → Simulation Call Chain

```
HTTP POST /api/simulation/run
  → SimulationsController (implements SimulationApi)
    → DesmojSimulationEngine.run()
      → WarehouseBuilder.build()          // assembles all domain objects
      → new DesmojWarehouseModel(...)      // Desmo-J Model
      → Experiment.start()                 // triggers doInitialSchedules()
        → Robot processes (RTP/VTM/HTM)    // cooperative scheduling
        → DesmojBinManagementProcess       // order dispatching
        → DesmojMetricMonitoringProcess    // checks allOrdersCompleted
        → DesmojSnapshotCaptureProcess     // optional binary snapshots
      → Experiment.finish()
      → MetricsManager.getResults()        // collects from all collectors
```

## WarehouseBuilder: 4 Build Phases

| Phase | What it builds | Key classes |
|-------|---------------|-------------|
| 1. Map | Grid, racks, stations from layout-service | `LayoutServiceAdapter`, `GridPopularityManager` |
| 2. System-specific | Zones, relay positions (Relay only) | `ZoneManager`, `RelayPositionManager` |
| 3. Inventory | SKU placement, tote pools | `ItemCatalog`, `WarehouseInventory`, `StorageSpaceManager` |
| 4. Robots | Robot fleet, station manager | `StationManager`, robot config records |

## "If You Need X, Start at Y"

| Need | Start at |
|------|----------|
| Add/modify an API endpoint | OpenAPI spec → generated interface → controller in `api/` |
| Change simulation config | `SimulationConfigs` (nested records) → `SimulationConfigDTO` |
| Add a robot behavior | `DesmojRtpRobotProcess` (RTP) or `DesmojVtmProcess`/`DesmojHtmProcess` (Relay) |
| Add a metric event | `DomainEvent.java` (factory method) → collector `@MetricHandler` |
| Change inventory placement | `PutStrategy` subclasses in put-service |
| Modify rack/station selection | `StorageSpaceManager` (racks), `StationManager` (stations) |
| Change pathfinding behavior | `NavigationServiceClient` (async gRPC) → navigation-service |
| Add a warehouse component | `WarehouseBuilder` (appropriate build phase) |
| Change order dispatching | `DesmojBinManagementProcess` → `OrderProcessor` |
| Modify operator modeling | `DesmojStationHelper` + operator `Res` in `DesmojWarehouseModel.init()` |

## Orchestrator Processes (fleet-simulator)

`apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/`

| Process | Role |
|---------|------|
| `DesmojRtpRobotProcess` | RTP robot: rack → stations → return cycle |
| `DesmojVtmProcess` | VTM: rack → relay position (zone-bound, congestion-aware) |
| `DesmojHtmProcess` | HTM: relay position → station → return |
| `DesmojBinManagementProcess` | Dispatches orders to racks as bins |
| `DesmojMetricMonitoringProcess` | Fires `allOrdersCompleted` when done |
| `DesmojSnapshotCaptureProcess` | Periodic binary state snapshots |
| `DesmojStationHelper` | Not a process — shared station/operator logic |

## Domain Layer (fleet-simulator)

`apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/`

| Class | Role |
|-------|------|
| `Robot` | State machine (Idle, Moving, Loading, VTM, HTM states) |
| `StorageSpaceManager` | Rack location tracking, move orchestration |
| `OrderProcessor` | Order fulfillment, bin management |
| `StationManager` | Workstation modeling with queuing |
| `BackToStoreCoordinator` | Multi-resource reservation for returns |
| `RelayPosition`, `Zone` | Relay system spatial components |
| `WarehouseInventory` | Inventory tracking |
| `Tote`, `TotePool` | Tote lifecycle (relay system) |

## Domain Layer (put-service)

`apps/put-service/src/main/java/com/greyorange/put/domain/`

| Class | Role |
|-------|------|
| `PutProcessor` | Orchestrates inventory initialization |
| `GreedyPutStrategy` | Fast O(n*m) placement |
| `MinWeightedCycleDistancePutStrategy` | LP optimization (OR-Tools) |
| `GridPopularityManager` | Popularity scoring (proto-based) |
