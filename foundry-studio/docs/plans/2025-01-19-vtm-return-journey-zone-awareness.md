# VTM Return Journey Zone-Awareness Fix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix VTM return journey to check zone-specific capacity instead of global relay utilization, and find return tasks only within the VTM's assigned zone.

**Architecture:** Replace global `isHighRelayUtilization()` with zone-aware `findCongestedZone()` that uses ZoneCapacity tracking. Replace global `findReturnTask()` with `findReturnTaskInZone()` that filters by zone ownership and claims totes to prevent duplicate selection. Update lifecycle loop to idle (backpressure) when zone is congested but no return tasks available.

**Tech Stack:** Java 21, Desmo-J simulation framework, Lombok

---

## Task 1: Add Zone-Aware Congestion Detection

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java:467-484`

**Step 1: Add the new `findCongestedZone()` method**

Add this method after the existing `isHighRelayUtilization()` method (around line 484):

```java
/**
 * Find a congested zone among VTM's assigned zones.
 *
 * <p>Utilization formula: (currentOccupancy + reserved) / physicalCapacity
 * This includes in-transit deliveries for better backpressure control.
 *
 * @param thresholdPct Utilization threshold (0-100)
 * @return Zone ID if any assigned zone exceeds threshold, empty otherwise
 */
private Optional<String> findCongestedZone(double thresholdPct) {
  Set<String> zoneIds = zoneManager.getZonesForVtm(vtm.getId());

  for (String zoneId : zoneIds) {
    Zone zone = zoneManager.getZones().get(zoneId);
    if (zone == null) {
      continue;
    }

    ZoneCapacity capacity = zone.getCapacity();
    int physicalCapacity = capacity.getPhysicalCapacity();
    if (physicalCapacity == 0) {
      continue;
    }

    int total = capacity.getCurrentOccupancy() + capacity.getReserved();
    double utilization = (total * 100.0) / physicalCapacity;

    if (utilization >= thresholdPct) {
      log.debug(
          "VTM {}: Zone {} congested at {:.1f}% (occupancy={}, reserved={}, capacity={})",
          vtm.getId(),
          zoneId,
          utilization,
          capacity.getCurrentOccupancy(),
          capacity.getReserved(),
          physicalCapacity);
      return Optional.of(zoneId);
    }
  }

  return Optional.empty();
}
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
cd apps/backend
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: add zone-aware congestion detection for VTM return journey"
```

---

## Task 2: Add Zone-Aware Return Task Finding

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java:491-512`

**Step 1: Add the new `findReturnTaskInZone()` method**

Add this method after the `findCongestedZone()` method:

```java
/**
 * Find empty tote for return journey in specified zone.
 *
 * <p>Selection criteria:
 * - Relay position must be owned by the zone
 * - Tote must have no tasks (empty) and not claimed by another VTM
 * - Closest tote to VTM is selected (minimize travel)
 *
 * <p>Claims tote via setAssignedVtmId() to prevent duplicate selection.
 *
 * @param zoneId Zone to search for return tasks
 * @return Return task if found, empty otherwise
 */
private Optional<ReturnTask> findReturnTaskInZone(String zoneId) {
  GridCoordinate vtmPosition = vtm.getPosition();

  Optional<ReturnTask> bestTask = relayPositionManager.getPositionsWithTotes().stream()
      .filter(relay -> zoneId.equals(relay.getOwnerZoneId()))
      .flatMap(relay -> relay.getPendingTotes().stream()
          .filter(tote -> !tote.hasTasks() && tote.getAssignedVtmId() == null)
          .map(tote -> createReturnTaskIfValid(tote, relay))
          .flatMap(Optional::stream))
      .min(Comparator.comparingInt(task ->
          vtmPosition.manhattanDistance(task.relayPosition().getPosition())));

  bestTask.ifPresent(task -> {
    task.tote().setAssignedVtmId(vtm.getId());
    log.debug("VTM {}: Found return task in zone {} - tote={}, relay={}",
        vtm.getId(), zoneId, task.tote().getId(), task.relayPosition().getId());
  });

  return bestTask;
}

/** Create ReturnTask if tote has valid rack with empty slots. */
private Optional<ReturnTask> createReturnTaskIfValid(Tote tote, RelayPosition relay) {
  if (!(tote.getCurrentHoldingEntity() instanceof RackId rackId)) {
    return Optional.empty();
  }
  Rack rack = storageSpaceManager.getRack(rackId);
  if (rack == null || !rack.hasEmptyToteSlots()) {
    return Optional.empty();
  }
  return Optional.of(new ReturnTask(tote, relay, rack));
}
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
cd apps/backend
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: add zone-aware return task finding with claim mechanism"
```

---

## Task 3: Update VTM Lifecycle Loop

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java:114-162`

**Step 1: Replace the lifecycle loop with zone-aware logic**

Replace the `lifeCycle()` method content (lines 114-162) with:

```java
@Override
public void lifeCycle() throws SuspendExecution {
  log.info("VTM {} starting lifecycle at t={}", vtm.getId(), presentTime());

  try {
    // Main VTM loop
    while (true) {
      // Priority 1: Check for zone congestion (backpressure control)
      Optional<String> congestedZoneOpt = findCongestedZone(HIGH_RELAY_UTILIZATION_THRESHOLD);

      if (congestedZoneOpt.isPresent()) {
        String congestedZoneId = congestedZoneOpt.get();
        Optional<ReturnTask> returnTask = findReturnTaskInZone(congestedZoneId);

        if (returnTask.isPresent()) {
          executeReturnJourney(returnTask.get());
          continue;
        } else {
          // Backpressure: zone congested but no return tasks available
          // Idle to prevent adding more totes to congested zone
          vtm.setState(RobotState.IDLE);

          if (metricsManager != null) {
            metricsManager.emit(DomainEvent.robotIdle(now(), vtm.getId()));
          }

          log.debug(
              "VTM {} idle - zone {} congested at >{}% but no return tasks available",
              vtm.getId(),
              congestedZoneId,
              HIGH_RELAY_UTILIZATION_THRESHOLD);

          hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
          continue;  // Re-check congestion, do NOT proceed to forward journey
        }
      }

      // Priority 2: Find tote and prepare for cycle (forward journey)
      Optional<VtmCycleContext> ctxOpt = findAndPrepareForCycle();

      if (ctxOpt.isEmpty()) {
        // No tasks or preparation failed, robot idle
        vtm.setState(RobotState.IDLE);

        if (metricsManager != null) {
          metricsManager.emit(DomainEvent.robotIdle(now(), vtm.getId()));
        }

        log.debug(
            "VTM {} idle at t={}, checking again in {}s",
            vtm.getId(),
            presentTime(),
            IDLE_CHECK_INTERVAL);

        hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
        continue;
      }

      // Execute the cycle
      executeCycle(ctxOpt.get());
    }

  } catch (desmoj.core.exception.SimFinishedException e) {
    // Expected at simulation end - gracefully terminate
    log.info("VTM {} ending lifecycle at t={}", vtm.getId(), presentTime());
  } catch (Exception e) {
    log.error("VTM {} encountered error: {}", vtm.getId(), e.getMessage(), e);
    vtm.setState(RobotState.ERROR);
  }
}
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
cd apps/backend
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: update VTM lifecycle with zone-aware congestion and backpressure"
```

---

## Task 4: Remove Deprecated Global Methods

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Remove or deprecate the old methods**

Delete the `isHighRelayUtilization()` method (lines 467-484) and the old `findReturnTask()` method (lines 491-512) since they are no longer used.

**Step 2: Compile to verify no broken references**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Format code**

Run: `cd apps/backend && mvn spotless:apply`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
cd apps/backend
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "refactor: remove deprecated global relay utilization methods"
```

---

## Task 5: Update executeReturnJourney to Clear VTM Assignment

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java:523-599`

**Step 1: Clear assignedVtmId after return journey completes**

In the `executeReturnJourney()` method, after returning the tote to rack (around line 563), add:

```java
// Return tote to rack
vtm.removeCarryingTote(tote);
targetRack.addTote(tote, reservation.zIndex());

// Clear VTM assignment (tote is back in rack, available for future cycles)
tote.setAssignedVtmId(null);

// Release hard reservation
storagePositionManager.releaseHardReservation(tote.getId());
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
cd apps/backend
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "fix: clear VTM assignment after return journey completes"
```

---

## Task 6: Run Integration Tests

**Files:**
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/simulator/DesmojIntegrationTest.java`

**Step 1: Run the relay system integration test**

Run: `cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -DfailIfNoTests=false`

Expected: Tests pass with RELAY system type

**Step 2: Verify VTM return journeys are triggered**

Check test logs for:
- `VTM * idle - zone * congested at >80%` (backpressure working)
- `VTM *: Found return task in zone *` (zone-aware return tasks)

**Step 3: Commit test verification**

```bash
cd apps/backend
git commit --allow-empty -m "test: verify VTM zone-aware return journey integration"
```

---

## Task 7: Final Cleanup and Format

**Step 1: Format all code**

Run: `cd apps/backend && mvn spotless:apply`

Expected: BUILD SUCCESS

**Step 2: Run full test suite**

Run: `cd apps/backend && mvn test`

Expected: All tests pass

**Step 3: Final commit**

```bash
cd apps/backend
git add -A
git commit -m "style: format code with spotless"
```

---

## Summary

This plan implements zone-aware return journey logic for VTM robots:

1. **Zone congestion detection** - Uses `ZoneCapacity` (occupancy + reserved) instead of global relay count
2. **Zone-scoped return tasks** - Only searches relay positions owned by VTM's assigned zone
3. **Claim mechanism** - Uses `tote.setAssignedVtmId()` to prevent duplicate selection
4. **Backpressure control** - VTM idles when zone is congested but no return tasks available
5. **Closest tote selection** - Minimizes travel distance for return journeys

**Key behavior change:** VTM will NOT proceed with forward journey when its zone is >80% full. It will either do return journeys or idle until HTMs clear totes.
