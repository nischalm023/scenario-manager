# Zone Capacity Simplification Design

**Date**: 2025-11-18
**Status**: Implemented

## 1. Problem Statement

The current zone capacity tracking system has become unmaintainable due to:

### 1.1 Three Independent Reservation Systems

Must stay synchronized but drift apart:
- `StoragePositionManager.softZoneReservations` - earliest reservation
- `ZoneCapacity.reserved/currentOccupancy` - middle reservation
- `RelayPosition.Res` (Desmo-J) - physical blocking

### 1.2 Multiple Ways to Determine Zone for Relay

- Geometric lookup (iterates zones, checks coverage area)
- Stored on tote (`tote.targetZoneId`)
- Relay's `accessibleFromZones` Set (N:M relationship)

### 1.3 Split Initialization

Between WarehouseBuilder and DesmojSimulationEngine:
- Zone capacity not available during inventory distribution
- Duplicate manager creation (StoragePositionManager created twice)
- Unclear ownership of managers

**Result**: Zone capacity accounting drifts, HTM crashes with "zone is empty" errors when trying to decrement capacity that was never incremented.

---

## 2. Design Principles

1. **Single owner zone per relay** - Each relay belongs to exactly one zone for capacity purposes. No N:M relationship.

2. **Zone derived from relay** - Never use geometric lookups or stored tote state. Always `relay.getOwnerZoneId()`.

3. **Single reservation system** - Only `ZoneCapacity` (reserved + occupancy). Remove soft reservations entirely.

4. **Lazy Res initialization** - RelayPosition created with metadata in WarehouseBuilder, Desmo-J Res initialized in Engine.

5. **BackToStoreSelector returns relay** - Not zone. Zone derived from selected relay.

6. **WarehouseComponents as single source** - All managers created in WarehouseBuilder, Engine only accesses through warehouseComponents.

---

## 3. Class Changes

### 3.1 RelayPosition

```java
public class RelayPosition {
    // Definition fields (set at construction in WarehouseBuilder)
    private final String id;
    private final GridCoordinate position;
    private final String rackBlockId;
    private final String side;
    private final String ownerZoneId;  // NEW: replaces accessibleFromZones
    private final int capacity;

    // Runtime fields (initialized later in Engine)
    private Res resource;  // null until initializeRuntime()
    private final Deque<Tote> pendingTotes = new ArrayDeque<>();

    // Two-phase construction
    public RelayPosition(String id, GridCoordinate position, String rackBlockId,
                        String side, String ownerZoneId, int capacity) { ... }

    public void initializeRuntime(DesmojWarehouseModel model) {
        this.resource = new Res(model, id + "_capacity", capacity, false, false);
    }

    public String getOwnerZoneId() {
        return ownerZoneId;
    }
}

// REMOVE: accessibleFromZones, addAccessibleZone(), isAccessibleFromZone()
```

### 3.2 RelayPositionManager

```java
public class RelayPositionManager {
    private final Map<String, RelayPosition> positionsById;
    private boolean initialized = false;

    // Called in WarehouseBuilder
    public RelayPositionManager(WarehouseMap map, ZoneManager zoneManager) {
        this.positionsById = buildPositions(map, zoneManager);
        // Assigns ownerZoneId to each relay during construction
    }

    // Called in DesmojSimulationEngine
    public void initializeRuntime(DesmojWarehouseModel model) {
        for (RelayPosition relay : positionsById.values()) {
            relay.initializeRuntime(model);
        }
        initialized = true;
    }

    public String getOwnerZoneForRelay(String relayId) {
        return positionsById.get(relayId).getOwnerZoneId();
    }
}
```

### 3.3 BackToStoreSelector

```java
public class BackToStoreSelector {

    // NEW: Returns relay, not zone
    public synchronized RelayPosition selectRelayForReturn(
            Tote tote, StationId originStationId, GridCoordinate htmPosition) {

        // Step 1: Select best zone (load balancing)
        String selectedZoneId = selectBestZone(tote, originStationId);

        // Step 2: Select best relay in that zone
        RelayPosition relay = findBestRelayInZone(selectedZoneId, htmPosition);

        // Step 3: Reserve in relay's owner zone
        String ownerZoneId = relay.getOwnerZoneId();
        Zone zone = zoneManager.getZones().get(ownerZoneId);
        zone.getCapacity().tryReserve();

        // Step 4: Update load tracking
        zoneLoadTracker.incrementLoad(ownerZoneId);

        return relay;
    }
}

// REMOVE: selectAndReserveZone() (returns zone)
// REMOVE: storagePositionManager.reserveSoftZoneForReturn() calls
```

### 3.4 WarehouseComponents

```java
@Builder
public record WarehouseComponents(
    WarehouseMap map,
    ItemCatalog itemCatalog,
    List<Robot> robots,

    // Managers
    GridPopularityManager gridPopularityManager,
    WarehouseInventory inventory,
    StationManager stationManager,
    ZoneManager zoneManager,
    RelayPositionManager relayPositionManager,  // NEW
    StoragePositionManager storagePositionManager,
    StorageSpaceManager storageSpaceManager
) {
    // Convenience accessors
    public List<Rack> racks() {
        return storageSpaceManager.getAllRacks();
    }

    public List<Tote> totes() {
        return storageSpaceManager.getAllTotes();
    }
}

// REMOVE: separate racks and totes fields (delegate to StorageSpaceManager)
```

---

## 4. Simplified Flows

### 4.1 VTM Delivery Flow

```java
// 1. Find tote with tasks
Optional<Tote> toteOpt = findToteWithTasks();
Tote tote = toteOpt.get();

// 2. Get source rack
RackId rackId = (RackId) tote.getCurrentHoldingEntity();
Rack sourceRack = storageSpaceManager.getRack(rackId);

// 3. Select optimal relay (from VTM's assigned zones)
Optional<RelayPosition> relayOpt = relayPositionManager.selectOptimalRelayForVtm(
    vtm, tote, sourceRack, zoneManager, stationManager);
RelayPosition relay = relayOpt.get();

// 4. Get zone from relay and reserve
String targetZoneId = relay.getOwnerZoneId();
Zone targetZone = zoneManager.getZones().get(targetZoneId);

if (!targetZone.getCapacity().tryReserve()) {
    // No capacity - idle and retry
    continue;
}

// 5. Transport to relay
// ... move to rack, load tote, move to relay ...

// 6. Drop at relay and fulfill
relay.addTote(this, tote);  // Res blocks if full
targetZone.getCapacity().fulfill();

// REMOVED: getTargetZoneForTote()
// REMOVED: storagePositionManager.reserveSoftZoneForReturn()
// REMOVED: storagePositionManager.releaseSoftReservation()
// REMOVED: tote.setTargetZoneId()
```

### 4.2 HTM Pickup Flow

```java
// 1. Find relay with totes
Optional<RelayPosition> relayOpt = relayPositionManager.findRelayWithTotes(htm.getPosition());
RelayPosition relay = relayOpt.get();

// 2. Remove tote from relay
Tote tote = relay.removeTote().get();

// 3. Decrement zone capacity (using relay's owner zone)
String sourceZoneId = relay.getOwnerZoneId();
Zone sourceZone = zoneManager.getZones().get(sourceZoneId);
sourceZone.getCapacity().remove();

// 4. Transport to station and process
// ... move to station, process picks ...

// REMOVED: tote.getTargetZoneId()
// REMOVED: getZoneForRelay()
// REMOVED: tote.setTargetZoneId(null)
```

### 4.3 HTM Return Flow

```java
// 1. Select relay for return (BackToStoreSelector does everything)
RelayPosition returnRelay = backToStoreSelector.selectRelayForReturn(
    tote, targetStationId, htm.getPosition());

// Zone already reserved inside selectRelayForReturn()
String returnZoneId = returnRelay.getOwnerZoneId();
Zone returnZone = zoneManager.getZones().get(returnZoneId);

// 2. Transport to relay
// ... move to relay ...

// 3. Add tote and fulfill
returnRelay.addTote(this, tote);
returnZone.getCapacity().fulfill();

// REMOVED: getCandidateZonesForReturn()
// REMOVED: Loop trying to reserve in multiple zones
// REMOVED: selectRelayInZone()
// REMOVED: storagePositionManager.reserveSoftZoneForReturn()
// REMOVED: storagePositionManager.releaseSoftReservation()
// REMOVED: tote.setTargetZoneId()
```

### 4.4 VTM Return-to-Rack Flow

```java
// 1. Find empty tote at relay to return
ReturnTask task = findReturnTask();
RelayPosition relay = task.relayPosition();

// 2. Remove from relay and decrement zone
Tote tote = relay.removeTote().get();
String zoneId = relay.getOwnerZoneId();
Zone zone = zoneManager.getZones().get(zoneId);
zone.getCapacity().remove();

// 3. Return to rack
// ... move to rack, place tote ...

// REMOVED: Geometric lookup for zone
```

---

## 5. Initialization Order

### 5.1 WarehouseBuilder.build()

```java
public WarehouseComponents build() {
    // 1. Build map and layout
    buildMap();
    buildGridPopularityManager();

    // 2. Build RelayPositionManager (definitions only, no Res)
    buildRelayPositionManager();  // NEW: moved here

    // 3. Build ZoneManager with capacity
    buildZoneManager();
    // Now uses relayPositionManager to calculate zone capacity correctly

    // 4. Enable zone load balancing (zone capacity now available!)
    if (config.systemType() == SystemType.RELAY && zoneManager != null) {
        gridPopularityManager.enableZoneLoadBalancing(zoneManager);
    }

    // 5. Build storage
    addRacks();
    addTotes();
    buildStorageSpaceManager();  // NEW: single creation

    // 6. Build inventory (can now use zone-balanced placement)
    buildItemCatalog();
    buildInventory();
    buildStoragePositionManager();  // Simplified, no soft reservations

    // 7. Build station and robots
    buildStationManager();
    buildRobotFleet();

    return WarehouseComponents.builder()
        .map(map)
        .itemCatalog(itemCatalog)
        .robots(robots)
        .gridPopularityManager(gridPopularityManager)
        .inventory(warehouseInventory)
        .stationManager(stationManager)
        .zoneManager(zoneManager)
        .relayPositionManager(relayPositionManager)
        .storagePositionManager(storagePositionManager)
        .storageSpaceManager(storageSpaceManager)
        .build();
}
```

### 5.2 DesmojSimulationEngine

```java
public class DesmojSimulationEngine {
    private final SimulationConfig config;
    private final WarehouseComponents warehouseComponents;  // Single source
    private final MetricsManager metricsManager;
    private final ScenarioManager scenarioManager;

    // Engine-specific (runtime only)
    private final OrderManager orderManager;
    private ZoneLoadTracker zoneLoadTracker;

    // Desmo-J runtime
    private DesmojWarehouseModel model;
    private Experiment experiment;

    // Accessors (no private fields for managers)
    private ZoneManager zoneManager() { return warehouseComponents.zoneManager(); }
    private RelayPositionManager relayPositionManager() { return warehouseComponents.relayPositionManager(); }
    private StoragePositionManager storagePositionManager() { return warehouseComponents.storagePositionManager(); }
    private StorageSpaceManager storageSpaceManager() { return warehouseComponents.storageSpaceManager(); }

    // In setupExperiment()
    private void setupExperiment() {
        // 1. Create model and experiment
        model = new DesmojWarehouseModel(...);
        experiment = new Experiment("Warehouse");
        model.connectToExperiment(experiment);

        // 2. Initialize relay runtime (creates Res resources)
        relayPositionManager().initializeRuntime(model);

        // 3. Balance VTM assignments
        zoneManager().balanceVtmAssignments(vtmIds);

        // 4. Schedule processes
        // ...
    }
}

// REMOVED: private StoragePositionManager storagePositionManager (duplicate)
// REMOVED: private StorageSpaceManager storageSpaceManager (duplicate)
// REMOVED: private ZoneManager zoneManager (duplicate)
// REMOVED: private RelayPositionManager relayPositionManager (now in warehouseComponents)
```

---

## 6. Deletions

### 6.1 StoragePositionManager

Remove soft reservation system entirely:

```java
// DELETE these fields
private final Map<String, Set<ToteId>> softZoneReservations;
private final Map<ToteId, String> toteSoftReservations;

// DELETE these methods
public synchronized String reserveSoftZone(Tote tote)
public synchronized void reserveSoftZoneForReturn(Tote tote, String zoneId)
public synchronized void releaseSoftReservation(ToteId toteId)
public int getSoftReservationCount(String zoneId)
private int getEffectiveFreeRelayCount(Zone zone)
private List<Zone> selectBestAvailableZones()
```

### 6.2 RelayPosition

```java
// DELETE
private final Set<String> accessibleFromZones;
public void addAccessibleZone(String zoneId)
public boolean isAccessibleFromZone(String zoneId)
```

### 6.3 DesmojVtmProcess

```java
// DELETE
private String getTargetZoneForTote(Tote tote)

// REMOVE calls to:
// - storagePositionManager.reserveSoftZoneForReturn()
// - storagePositionManager.releaseSoftReservation()
// - tote.setTargetZoneId()
```

### 6.4 DesmojHtmProcess

```java
// DELETE
private String getZoneForRelay(RelayPosition relay)
private List<String> getCandidateZonesForReturn(...)
private RelayPosition selectRelayInZone(String zoneId, GridCoordinate htmPosition)

// REMOVE calls to:
// - tote.getTargetZoneId()
// - tote.setTargetZoneId()
// - storagePositionManager.reserveSoftZoneForReturn()
// - storagePositionManager.releaseSoftReservation()
```

### 6.5 BackToStoreSelector

```java
// DELETE
public synchronized String selectAndReserveZone(Tote tote, StationId originStationId)

// REPLACE with:
public synchronized RelayPosition selectRelayForReturn(Tote tote, StationId originStationId, GridCoordinate htmPosition)
```

### 6.6 Tote

```java
// DELETE
private String targetZoneId;
public void setTargetZoneId(String zoneId)
public Optional<String> getTargetZoneId()
```

### 6.7 RelayPositionManager

```java
// DELETE
private final Map<String, List<RelayPosition>> positionsByZone;  // N:M mapping
public void buildZoneAccessibility(ZoneManager zoneManager)

// REMOVE zone constraint parameter from selectOptimalRelayForVtm
// (Relay selection already constrained by VTM's assigned zones)
```

---

## 7. Summary

### What We're Building

A simplified zone capacity system with:
- **Single owner zone per relay** - `relay.getOwnerZoneId()`
- **Single reservation system** - `ZoneCapacity` only
- **Two-phase relay initialization** - Metadata in WarehouseBuilder, Res in Engine
- **BackToStoreSelector returns relay** - Zone derived from relay
- **All managers in WarehouseComponents** - Single source of truth

### Key Benefits

1. **No capacity drift** - One counter (ZoneCapacity), one lookup (ownerZoneId)
2. **No geometric lookups** - Pure field access
3. **No soft reservations** - Eliminates redundant tracking
4. **Zone capacity available for inventory** - Correct initialization order
5. **Clear ownership** - WarehouseBuilder creates, Engine uses

### Files Changed

| File | Changes |
|------|---------|
| `RelayPosition.java` | Add ownerZoneId, lazy Res init, remove accessibleFromZones |
| `RelayPositionManager.java` | Build in WarehouseBuilder, initializeRuntime(), getOwnerZoneForRelay() |
| `ZoneManager.java` | Use relay ownership for capacity calculation |
| `BackToStoreSelector.java` | selectRelayForReturn() returns RelayPosition |
| `StoragePositionManager.java` | Remove all soft reservation code |
| `Tote.java` | Remove targetZoneId |
| `DesmojVtmProcess.java` | Simplified flow, remove getTargetZoneForTote() |
| `DesmojHtmProcess.java` | Simplified flow, remove getZoneForRelay(), selectRelayInZone() |
| `WarehouseBuilder.java` | Create RelayPositionManager, StorageSpaceManager |
| `DesmojSimulationEngine.java` | Access managers via warehouseComponents, remove duplicates |
| `WarehouseComponents.java` | Add relayPositionManager, storageSpaceManager, delegate racks/totes |
