# Rack Slot Reservation Design - Move to HTM Journey

**Date:** 2025-11-22
**Author:** Design validated through collaborative brainstorming
**Status:** Design Complete - Ready for Implementation

---

## 1. Problem Statement

Currently, VTM return journeys perform rack slot reservation on-the-fly during execution. This means VTMs must:
- Search for available rack slots after picking up empty totes from relays
- Handle reservation failures mid-journey
- Manage complex error paths and fallback logic

This design moves rack slot reservation earlier in the lifecycle to simplify VTM logic and provide guaranteed return paths for all totes at relay positions.

---

## 2. Solution Overview

**Key Change:** Move rack slot reservation from VTM return journey → HTM return journey.

When HTM completes station processing and selects a return relay, it will atomically reserve three resources:
1. **Relay position** (existing)
2. **Zone capacity** (existing)
3. **Rack slot in target zone** (NEW)

VTM return journeys then become lookup-based:
- Find empty tote at relay
- Lookup existing rack slot reservation
- Deliver to reserved slot
- Release reservation

---

## 3. Benefits & Trade-offs

### Benefits
- **Simpler VTM logic**: No search/reservation logic, just reservation lookup
- **Earlier failure detection**: HTM discovers rack unavailability before committing to relay delivery
- **Atomic reservations**: All three resources reserved together, preventing partial reservation states
- **Guaranteed return paths**: Every tote at relay has a guaranteed rack destination

### Trade-offs
- **Longer reservation holding time**: Rack slots reserved earlier (during HTM return vs VTM return)
- **HTM may idle more**: If no zones have both relay + rack capacity, HTM blocks

---

## 4. Current System Flow

### HTM Return Journey (Station → Relay)
```
HTM @ station (after processing)
  ↓
selectRelayForReturn() → reserves:
  1. Relay position ✅
  2. Zone capacity ✅
  ↓
HTM travels to relay, drops tote
  ↓
HTM releases relay + zone reservations ✅
```

### VTM Return Journey (Relay → Rack)
```
VTM finds empty tote at relay (congestion >80%)
  ↓
reserveHardSlot() → searches for rack with empty slot
  ↓
Reserves specific rack slot (rackId + zIndex)
  ↓
VTM travels to rack, drops tote
  ↓
Releases rack slot reservation ✅
```

---

## 5. Proposed System Flow

### HTM Return Journey (Station → Relay)
```
HTM @ station (after processing)
  ↓
selectRelayForReturn() → reserves:
  1. Relay position ✅ (existing)
  2. Zone capacity ✅ (existing)
  3. Rack slot in target zone 🆕 (NEW)
  ↓
HTM travels to relay, drops tote
  ↓
HTM releases relay + zone reservations ✅
Rack slot reservation STAYS in toteReservations map 🆕
```

### VTM Return Journey (Relay → Rack)
```
VTM finds empty tote at relay (congestion >80%)
  ↓
getToteReservation(toteId) → lookup existing reservation 🆕 (CHANGED)
  ↓
If not found → throw IllegalStateException (programming bug)
  ↓
VTM travels to reserved rack slot
  ↓
Releases rack slot reservation ✅ (unchanged)
```

---

## 6. Component Changes

### 6.1. RelayBackToStoreSelector

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/backtostorestrategies/RelayBackToStoreSelector.java`

#### New Record Type
```java
// Add after line 34
public record ReturnRelayReservation(RelayPosition relay, ToteSlotReservation rackSlot) {}
```

#### Modified Method Signature
```java
// OLD (line 67)
public synchronized RelayPosition selectRelayForReturn(...)

// NEW
public synchronized Optional<ReturnRelayReservation> selectRelayForReturn(...)
```

#### Modified Logic - selectRelayForReturn()

**After line 82** (after zone selection):
```java
String selectedZoneId = selectByAdditionalCriteria(eligibleZones, tote, originStationId);

// Find relay in zone (existing logic from line 85-86)
Optional<RelayPosition> selectedRelay =
    relayPositionManager.selectOptimalRelayForHtm(htm, tote, selectedZoneId, zoneManager);

if (selectedRelay.isEmpty()) {
    return Optional.empty(); // No relay available
}

// NEW: Reserve rack slot BEFORE zone/relay reservation
Zone zone = zoneManager.getZones().get(selectedZoneId);
List<Rack> racksInZone = getRacksInZone(zone);
Optional<ToteSlotReservation> rackSlot =
    reserveHardSlot(tote, racksInZone);

if (rackSlot.isEmpty()) {
    // Cleanup relay reservation
    relayPositionManager.releaseReservation(selectedRelay.get().getId(), tote.getId());
    return Optional.empty(); // No rack slots available
}

// Continue with existing zone reservation (lines 97-100)
if (!zone.tryReserve()) {
    // Cleanup both reservations
    releaseHardReservation(tote.getId());
    relayPositionManager.releaseReservation(selectedRelay.get().getId(), tote.getId());
    return Optional.empty();
}

zoneLoadTracker.incrementLoad(selectedZoneId);

// Return both reservations
return Optional.of(new ReturnRelayReservation(selectedRelay.get(), rackSlot.get()));
```

#### New Helper Method
```java
// Add after line 236
private List<Rack> getRacksInZone(Zone zone) {
    return storageSpaceManager.findRacksWithCapacity().stream()
        .filter(r -> zone.servicesRack(r.getCurrentCoordinate()))
        .toList();
}
```

#### Modified reserveHardSlot()

**Change signature (line 115):**
```java
// OLD - returns ToteSlotReservation (throws on failure)
public synchronized ToteSlotReservation reserveHardSlot(...)

// NEW - returns Optional<ToteSlotReservation> (empty on failure)
public synchronized Optional<ToteSlotReservation> reserveHardSlot(...)
```

**Change failure handling (lines 134-135):**
```java
// OLD
if (targetRack == null) {
    throw new IllegalStateException("No free rack slots available for tote " + tote.getId());
}

// NEW
if (targetRack == null) {
    return Optional.empty(); // Let caller handle unavailability
}
```

**Change return (line 145):**
```java
// OLD
return reservation;

// NEW
return Optional.of(reservation);
```

---

### 6.2. DesmojHtmProcess

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

#### Modified returnToRelay() method

**Replace lines 223-248:**
```java
private RelayPosition returnToRelay(CycleContext ctx) throws SuspendExecution {
    // Step 1: Select relay for return (now returns Optional)
    Optional<ReturnRelayReservation> reservationOpt =
        relayBackToStoreSelector.selectRelayForReturn(
            htm, ctx.tote(), ctx.targetStationId(), htm.getPosition());

    if (reservationOpt.isEmpty()) {
        // No relay+rack capacity available - should not happen mid-cycle
        // This HTM already picked up the tote, must complete the cycle
        throw new IllegalStateException(
            "No return relay available for HTM " + htm.getId() +
            " carrying tote " + ctx.tote().getId() +
            " - rack or relay capacity exhausted");
    }

    ReturnRelayReservation reservation = reservationOpt.get();
    RelayPosition returnRelay = reservation.relay();

    // Rack slot reservation is now stored in toteReservations map
    // VTM will look it up later via getToteReservation(toteId)

    String selectedZoneId = returnRelay.getOwnerZoneId();
    Zone selectedZone = model.getZoneManager().getZones().get(selectedZoneId);

    // Phase 6: Transport to relay
    htm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
    double returnTravel = htm.moveToPosition(returnRelay.getPosition());
    holdForTravel(returnTravel);

    // Phase 7: Unload tote
    htm.setState(RobotState.UNLOADING_RACK);
    hold(new TimeSpan(UNLOADING_TIME, TimeUnit.SECONDS));

    htm.removeCarryingTote(ctx.tote());
    model
        .getStorageSpaceManager()
        .moveToteFromRobotToRelay(ctx.tote().getId(), returnRelay.getId(), returnRelay, this);
    selectedZone.completeReservation();
    relayPositionManager.releaseReservation(returnRelay.getId(), ctx.tote().getId());

    // NOTE: Rack slot reservation is NOT released here
    // It stays in toteReservations map for VTM to use during return journey

    return returnRelay;
}
```

---

### 6.3. DesmojVtmProcess

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

#### Modified executeReturnJourney() method

**Replace lines 624-629:**
```java
// OLD
List<Rack> availableRacks = List.of(task.targetRack());
RelayBackToStoreSelector.ToteSlotReservation reservation =
    relayBackToStoreSelector.reserveHardSlot(removedTote.get(), availableRacks);
Rack targetRack = storageSpaceManager.getRack(reservation.rackId());

// NEW
// Lookup existing rack slot reservation (instead of creating)
Optional<RelayBackToStoreSelector.ToteSlotReservation> reservationOpt =
    relayBackToStoreSelector.getToteReservation(removedTote.get().getId());

if (reservationOpt.isEmpty()) {
    // This should NEVER happen - indicates reservation leak
    throw new IllegalStateException(
        "VTM " + vtm.getId() + " found tote " + removedTote.get().getId() +
        " at relay " + task.relayPosition().getId() +
        " but no rack slot reservation exists. This indicates a programming bug.");
}

RelayBackToStoreSelector.ToteSlotReservation reservation = reservationOpt.get();
Rack targetRack = storageSpaceManager.getRack(reservation.rackId());
```

**No other changes needed** - lines 632-661 remain unchanged (travel, unload, release).

---

## 7. Data Flow & Storage

### Reservation Storage

**Good news:** Infrastructure already exists!

```java
// RelayBackToStoreSelector.java

// Line 38 - Already exists!
private final Map<ToteId, ToteSlotReservation> toteReservations = new ConcurrentHashMap<>();

// Lines 164-166 - Already exists!
public Optional<ToteSlotReservation> getToteReservation(ToteId toteId) {
    return Optional.ofNullable(toteReservations.get(toteId));
}
```

### Complete Data Flow

#### HTM Return Journey
```
1. DesmojHtmProcess.returnToRelay()
   └─ relayBackToStoreSelector.selectRelayForReturn(htm, tote, stationId, htmPosition)

2. RelayBackToStoreSelector.selectRelayForReturn():
   ├─ Select eligible zones (low load + has relay + rack capacity)
   ├─ For each eligible zone (try in order):
   │  ├─ Get racks in zone with empty slots
   │  ├─ Call reserveHardSlot(tote, racksInZone) 🆕 MOVED HERE
   │  ├─ If success → reserve relay + zone → return ReturnRelayReservation
   │  └─ If failure → try next zone
   └─ If all zones fail → return Optional.empty()

3. DesmojHtmProcess.returnToRelay():
   ├─ Extract relay position from ReturnRelayReservation
   ├─ Travel to relay, drop tote
   ├─ Release relay + zone reservations
   └─ Rack slot reservation stays in toteReservations map ✅
```

#### VTM Return Journey
```
1. DesmojVtmProcess.executeReturnJourney():
   ├─ Pick empty tote from relay
   ├─ Lookup: relayBackToStoreSelector.getToteReservation(toteId) 🆕 CHANGED
   ├─ If empty → throw IllegalStateException (should never happen!)
   ├─ Get target rack from reservation
   ├─ Travel to rack, drop tote at reserved z-index
   └─ Release: releaseHardReservation(toteId) ✅ UNCHANGED
```

---

## 8. Error Handling

### Error Scenario 1: HTM cannot find rack slot during selectRelayForReturn()

**Scenario:** All eligible zones have relay capacity but NO rack slots available

**Handling:**
- `selectRelayForReturn()` returns `Optional.empty()`
- `DesmojHtmProcess.returnToRelay()` checks if empty
- If empty → throws `IllegalStateException` (HTM already carrying tote, deadlock scenario)

**Code Pattern:**
```java
Optional<ReturnRelayReservation> reservationOpt =
    relayBackToStoreSelector.selectRelayForReturn(...);

if (reservationOpt.isEmpty()) {
    throw new IllegalStateException(
        "No return relay available for HTM " + htm.getId() +
        " carrying tote " + tote.getId() +
        " - rack or relay capacity exhausted");
}
```

**Note:** This should rarely happen. HTM only enters `executeCycle()` after successfully picking tote from relay. If capacity exhausts between pickup and return selection, system is overloaded and needs operational intervention (add more racks/relays).

---

### Error Scenario 2: VTM finds tote without rack slot reservation

**Scenario:** VTM picks empty tote from relay, but `getToteReservation()` returns empty

**Handling:**
- **Should never happen** (invariant: every tote at relay has reservation)
- Throw `IllegalStateException` with diagnostic info
- Log: tote ID, relay ID, simulation time
- Mark robot as ERROR state

**Rationale:** This indicates a programming bug (reservation leak), not a valid runtime condition.

**Code:**
```java
if (reservationOpt.isEmpty()) {
    throw new IllegalStateException(
        "VTM " + vtm.getId() + " found tote " + tote.getId() +
        " at relay " + relayId +
        " but no rack slot reservation exists. This indicates a programming bug.");
}
```

---

### Error Scenario 3: Zone filtering fails (no eligible zones)

**Scenario:** All zones either have no relay capacity OR no rack slots

**Handling:**
- Already handled by existing code path
- `selectRelayForReturn()` returns `Optional.empty()`
- HTM throws `IllegalStateException` (mid-cycle deadlock)

---

### Error Scenario 4: Rack slot reservation succeeds but relay/zone reservation fails

**Scenario:** Race condition - rack slot reserved, but zone/relay no longer available

**Handling:**
- **Atomic reservation pattern**: Check zone/relay availability BEFORE calling `reserveHardSlot()`
- If zone/relay unavailable after rack slot reserved → release rack slot immediately
- Continue to next eligible zone

**Code Pattern:**
```java
for (String zoneId : eligibleZones) {
    // 1. Select relay in zone
    Optional<RelayPosition> relay = selectOptimalRelayForHtm(...);
    if (relay.isEmpty()) continue;

    // 2. Reserve rack slot
    Optional<ToteSlotReservation> rackSlot = reserveHardSlot(tote, racksInZone);
    if (rackSlot.isEmpty()) {
        releaseReservation(relay); // cleanup
        continue;
    }

    // 3. Reserve zone (all-or-nothing)
    if (!zone.tryReserve()) {
        releaseHardReservation(toteId); // cleanup rack slot
        releaseReservation(relay); // cleanup relay
        continue;
    }

    // Success - all three reserved atomically!
    return Optional.of(new ReturnRelayReservation(relay, rackSlot));
}
```

---

## 9. System Invariants

After implementation, the following invariants must hold:

1. **Every tote at relay has rack slot reservation:**
   - `toteAt(relayPosition) → getToteReservation(toteId).isPresent()`

2. **No orphaned rack slot reservations:**
   - `getToteReservation(toteId).isPresent() → (toteAt(relay) OR toteOnVtm)`

3. **Atomic cleanup on failure:**
   - If `selectRelayForReturn()` returns empty → no reservations leaked

4. **VTM never creates reservations:**
   - `executeReturnJourney()` only calls `getToteReservation()` and `releaseHardReservation()`

---

## 10. Implementation Notes

### Compilation Order
1. `RelayBackToStoreSelector.java` - Add record, modify methods
2. `DesmojHtmProcess.java` - Update return type handling
3. `DesmojVtmProcess.java` - Replace create with lookup

### Backward Compatibility
- **Breaking change**: `selectRelayForReturn()` signature changes
- **Impact**: Only called by `DesmojHtmProcess.returnToRelay()`
- **Migration**: Update call site to handle `Optional<ReturnRelayReservation>`

### Logging Additions
Add debug logs for:
- Rack slot reservation success/failure in `selectRelayForReturn()`
- Reservation lookup in VTM `executeReturnJourney()`
- Cleanup paths in atomic reservation

### Metrics Considerations
No new metrics required. Existing metrics should show:
- **Decreased VTM cycle time** (no rack search overhead)
- **Possible increased HTM idle time** (if rack capacity tight)

---

## 11. Rollback Plan

If issues arise post-implementation:

1. **Revert commit** containing the three file changes
2. **Verify** `DesmojIntegrationTest` passes
3. **Run simulation** with existing config to confirm behavior matches pre-change

Atomic change (3 files) makes rollback straightforward.

---

## 12. Future Enhancements

### Performance Optimization
If rack slot holding time becomes problematic:
- Consider "soft reservations" that expire after timeout
- Add reservation priority (prefer racks closer to relay)

### Capacity Planning
Add metrics to track:
- How often `selectRelayForReturn()` returns empty (rack capacity pressure)
- Average rack slot holding time (reservation duration)

---

## 13. References

**Related Files:**
- `RelayPositionManager.java` - Relay position reservations
- `Zone.java` - Zone capacity tracking
- `ZoneLoadTracker.java` - Zone load balancing
- `DesmojWarehouseModel.java` - Simulation orchestration

**Design Documents:**
- `specs/relay_system/design.md` - Original relay system design
- `specs/relay_system/IMPLEMENTATION_NOTES.md` - Implementation details
- `docs/designs/reservation-services-design.md` - Reservation architecture

---

## 14. Approval & Sign-off

- [x] Design validated through collaborative brainstorming
- [ ] Code review completed
- [ ] Integration tests passing
- [ ] Production deployment approved

**Ready for implementation.**
