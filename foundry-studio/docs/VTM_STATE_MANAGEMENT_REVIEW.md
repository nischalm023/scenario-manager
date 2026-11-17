# VTM State Management Review - Summary

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Review Date**: 2025-11-20

**Status**: 🔴 Critical Issues Found

---

## Executive Summary

The VTM process has **6 critical state management issues** that can cause:
- Zone capacity drift (zones appear full when they're not)
- Totes stuck in claimed state (blocking other VTMs)
- Zone load tracking inaccuracies (incorrect congestion detection)
- State leaks on exceptions (reservations/assignments never released)

**Estimated Fix Effort**: 2-3 days

---

## State Change Summary by Entity

### 1. VTM Bot (Robot)

| Aspect | Status | Details |
|--------|--------|---------|
| **State Transitions** | ✅ Complete | 9 locations: IDLE, MOVING_TO_RACK, LOADING_RACK, TRANSPORTING_RACK_TO_STORAGE, ERROR |
| **Cargo Management** | ⚠️ Incomplete | addCarryingTote (lines 385, 701), removeCarryingTote (lines 452, 714) |
| **Missing** | ❌ Error cleanup | Cargo never cleared on exception (line 193) |

**Issue**: When exception occurs while carrying totes, cargo remains populated forever.

---

### 2. Tote

| Aspect | Status | Details |
|--------|--------|---------|
| **assignedVtmId** | ⚠️ Incomplete | Set: lines 322, 623 / Clear: lines 453, 718 |
| **Active/Inactive** | ⚠️ Incomplete | markActive: line 456 / markInactive: line 728 |
| **Missing** | ❌ Error cleanup | Never cleared on exceptions (lines 193, 697) |

**Issues**:
1. Exception between lines 322-453: Tote remains claimed, blocks other VTMs
2. Exception in return journey (line 697): Tote stuck with assignedVtmId set
3. No inactive marking if exception during return journey

---

### 3. Zone Capacity

| Aspect | Status | Details |
|--------|--------|---------|
| **Reserved Count** | ⚠️ Incomplete | tryReserve: line 302 / completeReservation: line 436 |
| **Occupied Count** | ❌ **BROKEN** | Incremented: line 436 / Decremented: **NEVER** |
| **Missing** | ❌ Critical | No occupied decrement when tote removed from relay (line 688) |

**Critical Issue**: Zone.occupied increments on every delivery but NEVER decrements. Over time, zones appear full even when empty.

**Impact**:
- Zones reach max capacity permanently
- VTMs idle even with available relay positions
- System throughput degrades over time

---

### 4. Zone Load Tracker

| Aspect | Status | Details |
|--------|--------|---------|
| **Forward Journey** | ❌ **ASYMMETRIC** | Increment TARGET zone (line 358) / Decrement SOURCE zone (line 428) |
| **Return Journey** | ✅ Symmetric | Increment relay zone (line 680) / Decrement relay zone (line 724) |
| **Missing** | ❌ Error cleanup | Never decremented on exceptions |

**Critical Issue**: Forward journey increments/decrements DIFFERENT zones.

```java
// Line 358: Increment relay's zone
zoneLoadTracker.incrementLoad(ctx.targetZone().getZoneId());

// Line 428: Decrement rack's zone (may be different!)
String sourceZoneId = zoneManager.getZoneForRack(ctx.sourceRack().getCurrentCoordinate()).getZoneId();
zoneLoadTracker.decrementLoad(sourceZoneId);
```

**Impact**:
- Zone loads drift over time (some zones permanently high, others negative)
- Incorrect congestion detection (VTMs idle in wrong zones)
- Backpressure control fails

---

### 5. Rack State

| Aspect | Status | Details |
|--------|--------|---------|
| **Tote Removal** | ✅ Complete | moveToteFromRackToRobot: line 376 |
| **Tote Addition** | ✅ Complete | moveToteFromRobotToRack: line 715 |
| **Missing** | None | Handled by StorageSpaceManager |

**Status**: ✅ No issues found

---

### 6. Relay Position State

| Aspect | Status | Details |
|--------|--------|---------|
| **Tote Addition** | ✅ Complete | moveToteFromRobotToRelay: line 420 |
| **Tote Removal** | ✅ Complete | moveToteFromRelayToRobot: line 688 |
| **Blocking Detection** | ❌ Incorrect | Checks relay BEFORE drop (line 417), not during/after |

**Minor Issue**: wasBlocked detection checks if relay occupied before operation, not if this operation actually blocked.

```java
boolean wasBlocked = ctx.relay().getTote().isPresent();  // ❌ Wrong timing
storageSpaceManager.moveToteFromRobotToRelay(...);      // Actual blocking happens here
```

---

## Critical Issues (Priority Order)

### P0 - Must Fix

| # | Issue | Lines | Impact |
|---|-------|-------|--------|
| 1 | **Zone.occupied never decremented** | 688 | Zone capacity permanently reduced, system degrades |
| 2 | **Zone load tracking asymmetry** | 358, 428 | Incorrect congestion detection, VTMs idle incorrectly |
| 3 | **No exception cleanup** | 193 | State leaks, totes stuck, reservations never released |
| 4 | **Return journey exception leaks** | 697 | Tote stuck, zone load incorrect, assignment never cleared |

### P1 - Should Fix

| # | Issue | Lines | Impact |
|---|-------|-------|--------|
| 5 | **wasBlocked detection incorrect** | 417 | Metrics inaccurate for relay blocking events |
| 6 | **No return journey state transitions** | 673-737 | Monitoring/debugging difficult, VTM appears idle during work |

---

## Quick Fix Checklist

### Fix #1: Add Zone.occupied Decrement (Line 688)
```java
// After removing tote from relay in return journey
Optional<Tote> removedTote = storageSpaceManager.moveToteFromRelayToRobot(...);

// ADD THIS:
Zone relayZone = zoneManager.getZone(task.relayPosition().getOwnerZoneId());
relayZone.decrementOccupied();  // May need to add this method to Zone class
```

### Fix #2: Fix Zone Load Tracking Asymmetry (Lines 358, 428)
```java
// Option A: Track same zone for increment/decrement
// Line 358: Use source zone (where rack is)
String workZoneId = zoneManager.getZoneForRack(ctx.sourceRack().getCurrentCoordinate()).getZoneId();
zoneLoadTracker.incrementLoad(workZoneId);

// Line 428: Use same zone
zoneLoadTracker.decrementLoad(workZoneId);  // Same zone as increment

// Option B: Add workZoneId to VtmCycleContext for consistency
```

### Fix #3: Add Exception Cleanup (Line 193)
```java
} catch (Exception e) {
  log.error("VTM {} encountered error: {}", vtm.getId(), e.getMessage(), e);

  // Cleanup: Clear cargo and release claimed totes
  for (Tote tote : vtm.getCarryingTotes()) {
    tote.setAssignedVtmId(null);
    // Mark inactive if tote was in transit to relay
  }
  vtm.clearCarryingTotes();

  // TODO: Release zone reservations if possible
  // TODO: Decrement zone load tracker

  vtm.setState(RobotState.ERROR);
}
```

### Fix #4: Add Return Journey Exception Cleanup (Line 697)
```java
if (removedTote.isEmpty() || removedTote.get() != task.tote()) {
  // Cleanup before throwing
  task.tote().setAssignedVtmId(null);
  zoneLoadTracker.decrementLoad(task.relayPosition().getOwnerZoneId());

  throw new IllegalStateException(...);
}
```

### Fix #5: Fix wasBlocked Detection (Line 417)
```java
double beforeBlock = now();

storageSpaceManager.moveToteFromRobotToRelay(...);

double afterBlock = now();
double blockTime = afterBlock - beforeBlock;
boolean wasBlocked = blockTime > 0.1;  // Actual blocking time
```

### Fix #6: Add Return Journey State Transitions (Lines 673-737)
```java
// Add state transitions throughout executeReturnJourney()
vtm.setState(RobotState.MOVING_TO_RACK);     // Before moving to relay
vtm.setState(RobotState.LOADING_RACK);        // During pickup
vtm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);  // Moving to rack
vtm.setState(RobotState.IDLE);                // After completion
```

---

## State Symmetry Verification

| Operation Pair | Forward Journey | Return Journey | Status |
|----------------|-----------------|----------------|--------|
| **Tote Assignment** | Set (322) → Clear (453) | Set (623) → Clear (718) | ⚠️ Missing error cleanup |
| **VTM Cargo** | Add (385) → Remove (452) | Add (701) → Remove (714) | ⚠️ Missing error cleanup |
| **Tote Active/Inactive** | Active (456) | Inactive (728) | ⚠️ Missing error cleanup |
| **Zone Reserved** | Reserve (302) → Complete (436) | N/A | ⚠️ Missing error cleanup |
| **Zone Occupied** | Increment (436) | **MISSING** | ❌ Never decremented |
| **Zone Load** | Inc (358) → Dec (428) | Inc (680) → Dec (724) | ❌ Different zones in forward |

---

## Testing Recommendations

After fixes, test these scenarios:

1. **Normal Operation**: Verify all state changes paired correctly
2. **Exception During Forward Journey**: Verify cleanup at each phase
3. **Exception During Return Journey**: Verify cleanup at each phase
4. **Zone Capacity**: Run long simulation, verify occupied count accurate
5. **Zone Load Tracking**: Run long simulation, verify no drift in zone loads
6. **Multi-Zone Operation**: Verify zone load tracking when rack zone ≠ relay zone

---

## Related Files

- `Zone.java`: May need to add `decrementOccupied()` method
- `ZoneLoadTracker.java`: Verify increment/decrement logic
- `StorageSpaceManager.java`: Tote movement operations
- `DesmojHtmProcess.java`: Check if similar issues exist

---

## Next Steps

1. ✅ Review complete - issues documented
2. ⬜ Create implementation plan for fixes
3. ⬜ Implement fixes (estimated 2-3 days)
4. ⬜ Add unit tests for error paths
5. ⬜ Run integration tests with long simulations
6. ⬜ Verify zone capacity/load tracking accuracy

---

**Document Version**: 1.0
**Last Updated**: 2025-11-20
**Reviewed By**: Claude Code (code-reviewer agent)
