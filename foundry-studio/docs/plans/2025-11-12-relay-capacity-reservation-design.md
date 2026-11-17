# Relay Capacity Reservation System Design

**Date**: 2025-11-12
**Problem**: HTM deadlocks when returning totes because relay positions fill up during processing
**Solution**: Zone-based capacity reservation with two-sided flow control

---

## Problem Analysis

### System Architecture
- **VTMs (Vertical Tote Movers)**: Zone-bound robots that pick totes from racks and deliver to relay positions
- **HTMs (Horizontal Tote Movers)**: Global robots that pick totes from relays, transport to stations, and return processed totes
- **Relay Positions**: Bounded buffers (capacity 5 per relay) for VTM→HTM handoff

### Root Cause: Producer-Consumer Deadlock
This maps to the classic **Bounded Buffer Producer-Consumer Problem** with circular dependency:

1. **Mutual exclusion**: Relay slots are exclusive resources
2. **Hold and wait**: HTM holds processed tote, waits for relay slot
3. **No preemption**: Can't forcibly take relay slot
4. **Circular wait**: VTM waits for HTM to free relay, HTM waits for relay capacity

**Timeline of deadlock:**
```
t=0:   VTM delivers tote 1 to relay A (1/5)
t=10:  HTM picks tote 1 from relay A (0/5)
t=20:  VTM delivers tote 2 to relay A (1/5)
t=30:  VTM delivers tote 3 to relay A (2/5)
t=40:  VTM delivers tote 4 to relay A (3/5)
t=50:  VTM delivers tote 5 to relay A (4/5)
t=60:  VTM delivers tote 6 to relay A (5/5 FULL)
t=70:  HTM finishes processing, tries to return tote 1 → BLOCKS (relay full)
t=∞:   Deadlock: HTM can't return, relay stays full, VTM can't deliver
```

---

## Design Solution: Two-Sided Flow Control

### Mathematical Property
**Guarantee**: `Total totes in system ≤ Total relay capacity` → No deadlock possible

**Mechanism**: Capacity reservation at decision points
- **VTM**: Reserve capacity BEFORE picking from rack
- **HTM**: Reserve capacity at station exit BEFORE returning

---

## Architecture

### 1. Zone Capacity Tracking

Each zone (set of relay positions) tracks available capacity:

```java
class ZoneCapacity {
  private final int physicalCapacity;     // Total slots in this zone
  private int currentOccupancy;           // Totes currently in relays
  private int reserved;                   // In-flight reservations (VTM + HTM)

  public synchronized int available() {
    return physicalCapacity - currentOccupancy - reserved;
  }

  public synchronized boolean tryReserve() {
    if (available() > 0) {
      reserved++;
      return true;
    }
    return false;
  }

  public synchronized void fulfill() {
    reserved--;
    currentOccupancy++;
  }

  public synchronized void remove() {
    currentOccupancy--;
  }
}
```

**Key Properties:**
- `available()` = capacity still available for new reservations
- Atomic `tryReserve()` prevents race conditions on last slot
- Reservation is a **hard guarantee** - once reserved, slot will be available

---

### 2. VTM Admission Control (Inflow)

**Purpose**: Prevent VTMs from overloading relay positions

**Flow:**
```java
// VTM lifecycle
while (true) {
  // 1. Find tote with tasks in VTM's zone
  Optional<Tote> toteOpt = findToteWithTasksInZone();
  if (toteOpt.isEmpty()) {
    idle();
    continue;
  }
  Tote tote = toteOpt.get();

  // 2. Determine target relay zone
  Zone targetZone = selectOptimalRelayZone(tote);

  // 3. Try to reserve capacity BEFORE picking from rack
  if (!targetZone.tryReserve()) {
    // No capacity - VTM idles instead of picking
    idle();
    continue;
  }

  // 4. Reservation successful - proceed with pickup
  pickToteFromRack(tote);
  travelToRelay(targetZone);

  // 5. Deliver to relay (guaranteed to succeed due to reservation)
  relay.addTote(tote);
  targetZone.fulfill();  // Convert reservation to occupancy
}
```

**Critical Point**: VTM checks and reserves **before** committing to pick tote from rack. This prevents the scenario where VTM picks tote but can't deliver.

---

### 3. HTM Return Reservation (Outflow)

**Purpose**: Prevent HTMs from blocking when returning processed totes

**Flow:**
```java
// HTM lifecycle
while (true) {
  // 1. Pick tote from relay
  RelayPosition sourceRelay = findRelayWithTotes();
  Tote tote = sourceRelay.removeTote();
  Zone sourceZone = getZone(sourceRelay);
  sourceZone.remove();  // Decrement occupancy

  // 2. Transport to station and process
  transportToStation(tote);
  processAtStation(tote);

  // 3. At station exit: "back to store" algorithm determines target zones
  List<Zone> candidateZones = backToStoreAlgorithm.rankZones(tote);

  // 4. Try to reserve in candidate zones (best first)
  Zone selectedZone = null;
  for (Zone zone : candidateZones) {
    if (zone.tryReserve()) {
      selectedZone = zone;
      break;  // Reservation successful
    }
  }

  if (selectedZone == null) {
    // All zones full - HTM waits/blocks at station
    // This creates backpressure: stations fill up → HTMs stop picking → relays empty
    waitForCapacity(candidateZones);
    continue;  // Retry after wait
  }

  // 5. Reservation successful - return to selected zone
  travelToRelay(selectedZone);
  relay.addTote(tote);
  selectedZone.fulfill();  // Convert reservation to occupancy
}
```

**Critical Point**: HTM reserves at station exit (after "back to store" decision) but **before** traveling to relay. This prevents TOCTOU race condition.

---

### 4. Integration with "Back to Store" Algorithm

The "back to store" algorithm determines optimal storage location for returned tote. It now must consider **both** storage optimality and relay capacity:

```java
class BackToStoreAlgorithm {
  List<Zone> rankZones(Tote tote) {
    return zones.stream()
      .map(zone -> {
        double storageScore = calculateStorageOptimality(zone, tote);
        double capacityScore = zone.available() > 0 ? 1.0 : 0.0;
        return new ZoneCandidate(zone, storageScore, capacityScore);
      })
      .filter(c -> c.capacityScore > 0)  // Only consider zones with capacity
      .sorted(comparingDouble(c -> -c.storageScore))  // Rank by storage optimality
      .map(ZoneCandidate::zone)
      .toList();
  }
}
```

**Key**: Algorithm returns ranked list of zones with available capacity. HTM tries them in order.

---

## Edge Cases

### 1. All zones full when HTM needs to return
**Scenario**: HTM at station exit, all zones have `available() = 0`

**Behavior**: HTM blocks/waits at station

**System Response**:
- HTMs accumulate at stations (not picking new totes)
- Relays gradually empty as HTMs stop consuming
- VTMs idle (can't reserve capacity for delivery)
- System reaches equilibrium: HTMs waiting creates backpressure

**Prevention**: Size relay capacity ≥ maximum expected concurrent totes

---

### 2. Race condition on last slot
**Scenario**: Zone has `available() = 1`, VTM and HTM both try to reserve

**Solution**: Atomic `synchronized tryReserve()` - only one succeeds

---

### 3. Reservation honored guarantee
**Scenario**: Robot reserved slot, but during travel, other reservations/deliveries occur

**Guarantee**: Once `reserved++`, that capacity is committed. The `available()` calculation accounts for reserved slots, so no overbooking can occur.

---

## Benefits

1. **Deadlock-free**: Mathematically guaranteed by capacity reservation
2. **No wasted capacity**: Single `reserved` counter shares capacity between VTM and HTM
3. **Realistic backpressure**: System self-regulates under load
4. **Flexible return destinations**: HTM can try multiple zones ranked by optimality
5. **Simple implementation**: Single counter per zone, atomic operations

---

## Implementation Plan

### Phase 1: Add ZoneCapacity tracking
- Create `ZoneCapacity` class with reservation system
- Integrate with existing `ZoneManager`

### Phase 2: Update VTM process
- Add capacity check before picking from rack
- Reserve → Pick → Deliver → Fulfill pattern

### Phase 3: Update HTM process
- Remove capacity check before return (was causing TOCTOU race)
- Add capacity reservation at station exit
- Integrate with "back to store" algorithm

### Phase 4: Testing
- Test with 1 VTM + 1 HTM (original deadlock scenario)
- Test with multiple VTMs + HTMs under high load
- Verify no deadlocks, measure throughput

---

## Metrics

Track per-zone:
- `currentOccupancy`: Current totes in relay
- `reserved`: In-flight reservations
- `available()`: Remaining capacity
- `reservationFailures`: Count of failed VTM/HTM reservation attempts

Track globally:
- `totalVTMIdle`: Time VTMs spent idle due to capacity
- `totalHTMWaitingAtStation`: Time HTMs spent waiting for return capacity
- `averageRelayUtilization`: Overall system efficiency
