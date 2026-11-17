# Storage Position Simplification Design

**Date:** 2025-11-18
**Scope:** Java backend only (Python to be removed)

## Overview

Simplify coordinate model by removing redundant `storageCoordinate` from Rack. Storage positions are computed on-demand by `StoragePositionManager` with reservation tracking.

## Key Decisions

1. **Remove `storage_coordinate` from Rack** - Keep only `currentCoordinate`
2. **StoragePositionManager computes optimal position on demand**
3. **Reservations tracked in StoragePositionManager**
4. **Reserve rack position at station completion** (before robot assignment)
5. **Totes use two-level reservations** - Soft (zone) and Hard (rack slot)
6. **Filter zones by relay availability** - Minimum 3 free relay positions
7. **Fallback to best available zone** if all below threshold

---

## Section 1: Rack Changes

### Remove
- `storageCoordinate` field
- `getStorageCoordinate()` / `setStorageCoordinate()` methods

### Keep
- `currentCoordinate` - physical location of rack (at storage, at station, in transit)

### Update
- `moveToStorage(GridCoordinate position)` - takes position from StoragePositionManager
- Serialization - remove `storage_coordinate` from output

---

## Section 2: StoragePositionManager - Rack Reservations

### New Fields

```java
private final Map<GridCoordinate, RackId> reservedPositions = new ConcurrentHashMap<>();
private final Map<RackId, GridCoordinate> rackReservations = new ConcurrentHashMap<>(); // reverse lookup
```

### New Methods

```java
/**
 * Reserve optimal position for rack returning to storage.
 * Called at station completion (before robot assignment).
 */
public GridCoordinate reservePositionForRack(Rack rack) {
    Set<GridCoordinate> unavailable = new HashSet<>(getOccupiedPositions());
    unavailable.addAll(reservedPositions.keySet());

    GridCoordinate optimal = getOptimalPositionForRack(rack, unavailable);

    reservedPositions.put(optimal, rack.getId());
    rackReservations.put(rack.getId(), optimal);

    return optimal;
}

/**
 * Release reservation when rack arrives at storage position.
 */
public void releaseReservation(RackId rackId) {
    GridCoordinate position = rackReservations.remove(rackId);
    if (position != null) {
        reservedPositions.remove(position);
    }
}

/**
 * Get reserved position for a rack (for robot navigation).
 */
public Optional<GridCoordinate> getReservedPosition(RackId rackId) {
    return Optional.ofNullable(rackReservations.get(rackId));
}
```

---

## Section 3: StoragePositionManager - Tote Reservations

### Model

- **Soft reservation** = Tote assigned to zone (counts against zone capacity)
- **Hard reservation** = Specific rack slot reserved (when VTM starts return)

### New Fields

```java
// Soft reservations
private final Map<ZoneId, Set<ToteId>> softZoneReservations = new ConcurrentHashMap<>();
private final Map<ToteId, ZoneId> toteSoftReservations = new ConcurrentHashMap<>();

// Hard reservations
private final Map<RackId, Set<Integer>> reservedToteSlots = new ConcurrentHashMap<>();
private final Map<ToteId, ToteSlotReservation> toteReservations = new ConcurrentHashMap<>();

private record ToteSlotReservation(RackId rackId, int zIndex) {}

private static final int MIN_FREE_RELAY_POSITIONS = 3;
```

### New Methods

```java
/**
 * Get available relay capacity for a zone, accounting for soft reservations.
 */
private int getEffectiveFreeRelayCount(Zone zone) {
    int physicalFree = zone.getFreeRelayPositionCount();
    int softReserved = softZoneReservations.getOrDefault(zone.getId(), Set.of()).size();
    return physicalFree - softReserved;
}

/**
 * Soft-reserve zone when tote is assigned tasks (VTM will deliver to relay).
 */
public ZoneId reserveSoftZone(Tote tote) {
    List<Zone> eligibleZones = zones.stream()
        .filter(z -> getEffectiveFreeRelayCount(z) >= MIN_FREE_RELAY_POSITIONS)
        .toList();

    if (eligibleZones.isEmpty()) {
        log.warn("All zones below relay threshold, using best available");
        eligibleZones = selectBestAvailableZones();
    }

    ZoneId targetZone = selectOptimalZone(tote, eligibleZones);

    softZoneReservations.computeIfAbsent(targetZone, k -> ConcurrentHashMap.newKeySet())
                        .add(tote.getId());
    toteSoftReservations.put(tote.getId(), targetZone);

    return targetZone;
}

/**
 * Release soft reservation when tote arrives at relay position.
 */
public void releaseSoftReservation(ToteId toteId) {
    ZoneId zone = toteSoftReservations.remove(toteId);
    if (zone != null) {
        Set<ToteId> zoneSet = softZoneReservations.get(zone);
        if (zoneSet != null) {
            zoneSet.remove(toteId);
        }
    }
}

/**
 * Hard-reserve specific rack slot when VTM starts return journey.
 */
public ToteSlotReservation reserveHardSlot(Tote tote, ZoneId targetZone) {
    List<Rack> racksInZone = getRacksInZone(targetZone);
    Rack optimalRack = findOptimalRackWithFreeSlot(tote, racksInZone);
    int zIndex = findFreeSlotInRack(optimalRack);

    reservedToteSlots.computeIfAbsent(optimalRack.getId(), k -> ConcurrentHashMap.newKeySet())
                     .add(zIndex);
    ToteSlotReservation reservation = new ToteSlotReservation(optimalRack.getId(), zIndex);
    toteReservations.put(tote.getId(), reservation);

    return reservation;
}

/**
 * Release hard reservation when tote arrives at rack slot.
 */
public void releaseHardReservation(ToteId toteId) {
    ToteSlotReservation reservation = toteReservations.remove(toteId);
    if (reservation != null) {
        Set<Integer> slots = reservedToteSlots.get(reservation.rackId());
        if (slots != null) {
            slots.remove(reservation.zIndex());
        }
    }
}
```

---

## Section 4: Tote Class Changes

```java
// Add to Tote class
private ZoneId targetZone;  // Soft target - which zone to return to

public void setTargetZone(ZoneId zone) {
    this.targetZone = zone;
}

public Optional<ZoneId> getTargetZone() {
    return Optional.ofNullable(targetZone);
}
```

---

## Section 5: Reservation Lifecycle

### Rack (RTP System)

| Event | Action |
|-------|--------|
| All picks complete at station | `reservePositionForRack(rack)` |
| Robot navigates to storage | `getReservedPosition(rackId)` |
| Rack arrives at storage | `releaseReservation(rackId)` |

### Tote (Relay System)

| Event | Action |
|-------|--------|
| Tote assigned tasks | `reserveSoftZone(tote)` |
| VTM delivers tote to relay | `releaseSoftReservation(toteId)` |
| VTM picks up tote for return to rack | `reserveHardSlot(tote, targetZone)` |
| Tote arrives at rack slot | `releaseHardReservation(toteId)` |

---

## Section 6: Files to Modify

### Core Domain Changes

**`Rack.java`**
- Remove `storageCoordinate` field and accessors
- Update `moveToStorage()` to take position parameter
- Update serialization

**`Tote.java`**
- Add `targetZone: ZoneId` field
- Add getter/setter

**`StoragePositionManager.java`**
- Add rack reservation maps
- Add tote soft/hard reservation maps
- Add all reservation methods
- Add zone filtering with `MIN_FREE_RELAY_POSITIONS = 3`
- Add `getEffectiveFreeRelayCount()` helper

### Simulation Process Changes

**`DesmojRtpRobotProcess.java`**
- At station completion: call `reservePositionForRack()`
- Get target from `getReservedPosition()` for navigation
- At arrival: call `releaseReservation()`

**`DesmojVtmProcess.java`**
- At tote task assignment: call `reserveSoftZone()`
- At relay arrival: call `releaseSoftReservation()`
- At return journey start: call `reserveHardSlot()`
- At rack arrival: call `releaseHardReservation()`

### Other Updates

- Grep for all usages of `rack.getStorageCoordinate()` and update to use `StoragePositionManager.getReservedPosition()` or `rack.getCurrentCoordinate()` as appropriate

---

## Benefits

1. **Single source of truth** - No confusion between "home" vs "current" position
2. **Dynamic re-slotting** - High-demand racks naturally migrate closer to stations
3. **Deadlock prevention** - Zone filtering prevents relay congestion
4. **Clean reservation lifecycle** - Clear soft/hard distinction for totes
