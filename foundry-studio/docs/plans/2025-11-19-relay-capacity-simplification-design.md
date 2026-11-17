# Relay Position Capacity Simplification Design

## Overview

Simplify the relay position model from a queue with variable capacity to a single tote slot. Each relay position can hold exactly 0 or 1 tote.

## Problem

The current implementation treats relay positions as queues with configurable capacity (e.g., 4 totes per position). This adds unnecessary complexity:
- `ZoneCapacity` class tracks `physicalCapacity`, `currentOccupancy`, `reserved`
- Manual counter management risks drift from actual state
- Complex methods like `fulfill()`, `remove()` for occupancy tracking

## Solution

### Core Simplification

**RelayPosition**: `Queue<Tote>` → `Optional<Tote>`

- Each relay position holds 0 or 1 tote
- Zone capacity = count of empty relay positions
- Occupancy derived from relay state (single source of truth)

### Design Decisions

1. **Optional<Tote> instead of queue** - simpler model, no FIFO complexity
2. **Keep Desmo-J Res(capacity=1)** - safeguard blocking if VTM tries to add to occupied relay
3. **Zone holds relay positions directly** - no filtering by zone ID, direct access
4. **Keep reserved counter** - for VTM/HTM in-transit backpressure control
5. **Delete ZoneCapacity class** - inline `reserved` counter into Zone

---

## Implementation Details

### RelayPosition.java

**Before:**
```java
private final Queue<Tote> pendingTotes;
private final int capacity;
private final Res resource;
```

**After:**
```java
private Optional<Tote> tote = Optional.empty();
private final Res resource;  // capacity = 1
```

**Methods:**
| Old Method | New Method |
|------------|------------|
| `addTote(process, tote)` | Keep - sets Optional, blocks via Res if occupied |
| `removeTote()` | Keep - returns and clears Optional, releases Res |
| `peekFirstTote()` | `peek()` - returns the Optional |
| `isFull()` | `tote.isPresent()` (inline or keep method) |
| `hasAvailableCapacity()` | `isEmpty()` → `tote.isEmpty()` |
| `hasTotes()` | Remove - use `tote.isPresent()` |
| `getPendingTotesCount()` | Remove - always 0 or 1 |
| `firstToteHasTasks()` | Remove - use `tote.map(Tote::hasTasks).orElse(false)` |
| `getCapacity()` | Remove |

---

### Zone.java

**Before:**
```java
private ZoneCapacity capacity;
```

**After:**
```java
private final List<RelayPosition> relayPositions = new ArrayList<>();
private int reserved;  // VTMs in-transit

public void addRelayPosition(RelayPosition relay) {
  relayPositions.add(relay);
}

public int countOccupied() {
  return (int) relayPositions.stream()
      .filter(r -> r.getTote().isPresent())
      .count();
}

public int getPhysicalCapacity() {
  return relayPositions.size();
}

public int available() {
  return getPhysicalCapacity() - countOccupied() - reserved;
}

public boolean tryReserve() {
  if (available() > 0) {
    reserved++;
    return true;
  }
  return false;
}

public void completeReservation() {
  reserved--;
}

public int getReserved() {
  return reserved;
}

public List<RelayPosition> getRelayPositions() {
  return Collections.unmodifiableList(relayPositions);
}
```

**Removed:**
- `ZoneCapacity capacity` field
- `getCapacity()` method
- `updateCapacity()` method

---

### ZoneCapacity.java

**DELETE entire file** - functionality inlined into Zone.

---

### RelayPositionManager.java

**Remove:**
- `capacityPerPosition` field
- Constructor parameter for capacity

**Update relay creation:**
```java
// Before:
new RelayPosition(model, id, position, zoneId, capacityPerPosition);

// After:
new RelayPosition(model, id, position, zoneId);  // capacity always 1
```

**Register relays with zones** (in `initializeRuntime()` or similar):
```java
for (RelayPosition relay : positionsById.values()) {
  String zoneId = relay.getOwnerZoneId();
  Zone zone = zoneManager.getZones().get(zoneId);
  if (zone != null) {
    zone.addRelayPosition(relay);
  }
}
```

**Simplify selection logic:**
```java
// Before:
.filter(relay -> !relay.isFull())
.sorted(Comparator.comparingInt(RelayPosition::getPendingTotesCount))

// After:
.filter(RelayPosition::isEmpty)
// Remove getPendingTotesCount - all full relays have count=1
```

**Update method references:**
```java
// Before:
.filter(RelayPosition::hasTotes)
.filter(RelayPosition::firstToteHasTasks)
.filter(RelayPosition::hasAvailableCapacity)

// After:
.filter(relay -> relay.getTote().isPresent())
.filter(relay -> relay.getTote().map(Tote::hasTasks).orElse(false))
.filter(RelayPosition::isEmpty)
```

---

### ZoneManager.java

**Remove:**
```java
public void initializeCapacityTracking(
    RelayPositionManager relayPositionManager,
    int capacityPerPosition) {
  // DELETE entire method
}
```

---

### DesmojVtmProcess.java

| Line | Before | After |
|------|--------|-------|
| 265 | `targetZone.getCapacity().tryReserve()` | `targetZone.tryReserve()` |
| 271 | `targetZone.getCapacity().available()` | `targetZone.available()` |
| 371 | `ctx.relay().isFull()` | `ctx.relay().getTote().isPresent()` |
| 390 | `ctx.targetZone().getCapacity().fulfill()` | `ctx.targetZone().completeReservation()` |
| 525 | `ZoneCapacity capacity = zone.getCapacity()` | Use `zone` directly |
| 526-532 | `capacity.getCurrentOccupancy()`, etc. | `zone.countOccupied()`, `zone.getReserved()`, `zone.getPhysicalCapacity()` |
| 581, 638 | `relay.peekFirstTote()` | `relay.peek()` |
| 790 | `zone.getCapacity().remove()` | Remove line (not needed) |

---

### DesmojHtmProcess.java

| Line | Before | After |
|------|--------|-------|
| 226 | `selectedZone.getCapacity().fulfill()` | `selectedZone.completeReservation()` |
| 295 | `relay.peekFirstTote()` | `relay.peek()` |
| 335 | `sourceZone.getCapacity().remove()` | Remove line (not needed) |

---

### BackToStoreSelector.java

| Line | Before | After |
|------|--------|-------|
| 55 | `zone.getCapacity().available() > 0` | `zone.available() > 0` |
| 89 | `zone.getCapacity().available()` | `zone.available()` |
| 130 | `zone.getCapacity().tryReserve()` | `zone.tryReserve()` |
| 157 | `RelayPosition::hasAvailableCapacity` | `RelayPosition::isEmpty` |

---

### Other Files

| File | Changes |
|------|---------|
| `DesmojSimulationEngine.java` | Remove `initializeCapacityTracking()` call |
| `WarehouseBuilder.java` | Remove `capacityPerPosition` parameter passing |
| `DesmojSnapshotCaptureProcess.java` | Update relay snapshot (no capacity field) |
| `DesmojMetricMonitoringProcess.java` | Simplify/remove capacity diagnostics |

---

## Flow Summary

### VTM Forward Journey
1. Check zone has capacity: `zone.tryReserve()` (checks `available() > 0`)
2. Select empty relay in zone
3. Travel to rack, pick up tote
4. Travel to relay, add tote
5. Complete reservation: `zone.completeReservation()` (reserved--)

### HTM Pickup
1. Find relay with tote that has tasks
2. Remove tote from relay: `relay.removeTote()`
3. (No zone capacity call - occupancy auto-updates)

### HTM Return
1. Select zone/relay: `BackToStoreSelector.selectRelayForReturn()` → `zone.tryReserve()`
2. Travel to relay
3. Add tote to relay: `relay.addTote()`
4. Complete reservation: `zone.completeReservation()`

### VTM Return Journey
1. Find empty tote at relay in assigned zone
2. Remove from relay: `relay.removeTote()`
3. Travel to rack, return tote
4. (No zone capacity call - occupancy auto-updates)

---

## Testing

### Update Tests
- `RelayPositionTest.java` - update for `Optional<Tote>` instead of queue
- `RelayPositionManagerTest.java` - remove capacity-related tests

### Delete Tests
- `ZoneCapacityTest.java` - class no longer exists

### New Test Cases
- Zone with relay positions: verify `countOccupied()`, `available()`, `tryReserve()`
- Relay position: verify `peek()`, `isEmpty()`, Res blocking behavior

---

## Benefits

1. **Simpler mental model** - 0 or 1 tote per relay, not a queue
2. **Single source of truth** - relay state IS occupancy, no manual counters
3. **Fewer abstractions** - no ZoneCapacity class
4. **Better encapsulation** - Zone owns its relay positions directly
5. **No counter drift risk** - derived state can't get out of sync
6. **Cleaner API** - `zone.tryReserve()` instead of `zone.getCapacity().tryReserve()`
