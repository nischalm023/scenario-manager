# Relay Capacity Diagnostic Logging Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add diagnostic logging to understand relay position bottlenecks by enhancing the existing metric monitoring process with relay status, zone capacity, and problem relay reports.

**Architecture:** Enhance `DesmojMetricMonitoringProcess` to log relay and zone state every 5 seconds. Access `RelayPositionManager` and `ZoneManager` through the model. Use condensed format with summary lines and per-zone breakdown, only showing individual relays when they are problematic (full or high empty ratio).

**Tech Stack:** Java 21, Desmo-J, Lombok (@Slf4j)

---

## Task 1: Add Manager Dependencies to DesmojMetricMonitoringProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java:23-40`

**Step 1: Add imports and fields**

Add imports at top of file (after existing imports around line 7):

```java
import com.greyorange.dessimulator.domain.robot.RelayPosition;
import com.greyorange.dessimulator.domain.robot.RelayPositionManager;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneCapacity;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel;
```

Add fields after existing fields (around line 25):

```java
  private final RelayPositionManager relayPositionManager; // nullable for RTP system
  private final ZoneManager zoneManager; // nullable for RTP system
```

**Step 2: Update constructor**

Replace constructor (lines 33-40) with:

```java
  public DesmojMetricMonitoringProcess(
      Model owner, boolean showInTrace, OrderManager orderManager, MetricsManager metricsManager) {

    super(owner, "MetricMonitoring", showInTrace);

    this.orderManager = orderManager;
    this.metricsManager = metricsManager;

    // Get relay/zone managers from model (null for RTP system)
    DesmojWarehouseModel warehouseModel = (DesmojWarehouseModel) owner;
    this.relayPositionManager = warehouseModel.getRelayPositionManager();
    this.zoneManager = warehouseModel.getZoneManager();
  }
```

**Step 3: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java
git commit -m "feat: add relay/zone manager dependencies to DesmojMetricMonitoringProcess"
```

---

## Task 2: Add Relay Status Summary Logging

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java`

**Step 1: Add helper method for relay status**

Add after the `lifeCycle()` method (around line 87):

```java
  /**
   * Log relay status summary showing total utilization and tote breakdown.
   *
   * @param currentTime Current simulation time in seconds
   */
  private void logRelayStatus(double currentTime) {
    if (relayPositionManager == null) {
      return; // RTP system, no relays
    }

    var relays = relayPositionManager.getPositionsById().values();

    int totalCapacity = 0;
    int totalOccupied = 0;
    int totesWithTasks = 0;
    int emptyTotes = 0;
    int fullRelays = 0;

    for (RelayPosition relay : relays) {
      totalCapacity += relay.getCapacity();
      int occupied = relay.getPendingTotesCount();
      totalOccupied += occupied;

      if (occupied >= relay.getCapacity()) {
        fullRelays++;
      }

      // Count totes with tasks vs empty
      for (var tote : relay.getPendingTotes()) {
        if (tote.hasTasks()) {
          totesWithTasks++;
        } else {
          emptyTotes++;
        }
      }
    }

    double utilizationPct = totalCapacity > 0 ? (totalOccupied * 100.0 / totalCapacity) : 0;

    log.info(
        "[RELAY-STATUS] t={}s | Total: {}/{} ({:.1f}%) | With tasks: {} | Empty: {} | Full: {}",
        String.format("%.0f", currentTime),
        totalOccupied,
        totalCapacity,
        utilizationPct,
        totesWithTasks,
        emptyTotes,
        fullRelays);
  }
```

**Step 2: Call from lifeCycle**

In `lifeCycle()` method, add call after `hold()` (around line 51):

```java
        // Wait for CHECK_INTERVAL seconds (SimPy: yield env.timeout(5))
        hold(new TimeSpan(CHECK_INTERVAL, TimeUnit.SECONDS));

        double currentTime = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

        // Log relay and zone status for diagnostics
        logRelayStatus(currentTime);
```

**Step 3: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java
git commit -m "feat: add relay status summary logging to monitoring process"
```

---

## Task 3: Add Zone Capacity Logging

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java`

**Step 1: Add helper method for zone capacity**

Add after `logRelayStatus()` method:

```java
  /**
   * Log zone capacity breakdown showing reserved, fulfilled, and available per zone.
   *
   * @param currentTime Current simulation time in seconds
   */
  private void logZoneCapacity(double currentTime) {
    if (zoneManager == null || relayPositionManager == null) {
      return; // RTP system, no zones
    }

    var zones = zoneManager.getAllZones();
    if (zones.isEmpty()) {
      return;
    }

    StringBuilder sb = new StringBuilder();
    sb.append(String.format("[ZONE-CAPACITY] t=%.0fs%n", currentTime));

    for (Zone zone : zones) {
      ZoneCapacity capacity = zone.getCapacity();
      int physical = capacity.getPhysicalCapacity();
      int occupancy = capacity.getCurrentOccupancy();
      int reserved = capacity.getReserved();
      int available = capacity.available();

      double utilizationPct = physical > 0 ? (occupancy * 100.0 / physical) : 0;

      // Count totes with tasks vs empty in this zone's relays
      int zoneTotesWithTasks = 0;
      int zoneEmptyTotes = 0;

      for (RelayPosition relay : relayPositionManager.getPositionsById().values()) {
        if (zone.getZoneId().equals(relay.getOwnerZoneId())) {
          for (var tote : relay.getPendingTotes()) {
            if (tote.hasTasks()) {
              zoneTotesWithTasks++;
            } else {
              zoneEmptyTotes++;
            }
          }
        }
      }

      sb.append(String.format(
          "  %s: %d/%d (%.0f%%) | reserved=%d | tasks=%d, empty=%d%n",
          zone.getZoneId(),
          occupancy,
          physical,
          utilizationPct,
          reserved,
          zoneTotesWithTasks,
          zoneEmptyTotes));
    }

    log.info(sb.toString().trim());
  }
```

**Step 2: Call from lifeCycle**

In `lifeCycle()` method, add call after `logRelayStatus()`:

```java
        // Log relay and zone status for diagnostics
        logRelayStatus(currentTime);
        logZoneCapacity(currentTime);
```

**Step 3: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java
git commit -m "feat: add zone capacity logging to monitoring process"
```

---

## Task 4: Add Problem Relays Logging

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java`

**Step 1: Add helper method for problem relays**

Add after `logZoneCapacity()` method:

```java
  /**
   * Log relays that are problematic (full or high empty ratio).
   *
   * <p>Problem criteria:
   * <ul>
   *   <li>Full (100% capacity)</li>
   *   <li>High empty ratio (>50% of totes have no tasks)</li>
   * </ul>
   *
   * @param currentTime Current simulation time in seconds
   */
  private void logProblemRelays(double currentTime) {
    if (relayPositionManager == null) {
      return; // RTP system, no relays
    }

    var relays = relayPositionManager.getPositionsById().values();

    List<String> problemLines = new ArrayList<>();

    for (RelayPosition relay : relays) {
      int occupied = relay.getPendingTotesCount();
      int capacity = relay.getCapacity();

      if (occupied == 0) {
        continue; // Empty relay is not a problem
      }

      // Count totes with tasks vs empty
      int withTasks = 0;
      int empty = 0;
      for (var tote : relay.getPendingTotes()) {
        if (tote.hasTasks()) {
          withTasks++;
        } else {
          empty++;
        }
      }

      boolean isFull = occupied >= capacity;
      boolean highEmptyRatio = occupied > 0 && (empty * 100.0 / occupied) > 50;

      if (isFull || highEmptyRatio) {
        String status = isFull ? "FULL" : "HIGH-EMPTY";
        problemLines.add(String.format(
            "  %s: %d/%d %s (tasks=%d, empty=%d)",
            relay.getId(),
            occupied,
            capacity,
            status,
            withTasks,
            empty));
      }
    }

    if (!problemLines.isEmpty()) {
      StringBuilder sb = new StringBuilder();
      sb.append(String.format("[PROBLEM-RELAYS] t=%.0fs%n", currentTime));
      for (String line : problemLines) {
        sb.append(line).append("\n");
      }
      log.warn(sb.toString().trim());
    }
  }
```

**Step 2: Add ArrayList import**

Add import at top of file:

```java
import java.util.ArrayList;
import java.util.List;
```

**Step 3: Call from lifeCycle**

In `lifeCycle()` method, add call after `logZoneCapacity()`:

```java
        // Log relay and zone status for diagnostics
        logRelayStatus(currentTime);
        logZoneCapacity(currentTime);
        logProblemRelays(currentTime);
```

**Step 4: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java
git commit -m "feat: add problem relays logging to monitoring process"
```

---

## Task 5: Add Order Status Logging (Enhance Existing)

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java`

**Step 1: Add order status log line**

In `lifeCycle()` method, after the relay/zone logging and before the completion check, add:

```java
        // Log relay and zone status for diagnostics
        logRelayStatus(currentTime);
        logZoneCapacity(currentTime);
        logProblemRelays(currentTime);

        // Update order statistics (Python line 362)
        Map<String, Integer> orderStatus = orderManager.getOrderProcessingStatus();

        // Log order status summary
        log.info(
            "[ORDER-STATUS] t={}s | pending={} | binned={} | completed={}",
            String.format("%.0f", currentTime),
            orderStatus.getOrDefault("pending", 0),
            orderStatus.getOrDefault("binned", 0),
            orderStatus.getOrDefault("completed", 0));
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java
git commit -m "feat: add order status logging to monitoring process"
```

---

## Task 6: Format Code and Final Verification

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java`

**Step 1: Format code with Spotless**

Run: `cd apps/backend && mvn spotless:apply`

Expected: BUILD SUCCESS

**Step 2: Compile full project**

Run: `cd apps/backend && mvn clean compile`

Expected: BUILD SUCCESS

**Step 3: Run tests to ensure no regressions**

Run: `cd apps/backend && mvn test`

Expected: BUILD SUCCESS (all tests pass)

**Step 4: Commit formatted code**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java
git commit -m "style: format relay capacity diagnostic logging code"
```

---

## Task 7: Final Verification

**Files:**
- None (verification only)

**Step 1: Run all tests**

Run: `cd apps/backend && mvn test`

Expected: BUILD SUCCESS (all tests pass)

**Step 2: Final commit**

```bash
git add -A
git commit -m "feat: complete relay capacity diagnostic logging

- Added relay status summary with utilization and tote breakdown
- Added zone capacity logging with reserved/fulfilled/available
- Added problem relay detection (full or >50% empty)
- Added order status summary"
```

---

## Summary

This implementation adds diagnostic logging to identify relay capacity bottlenecks:

1. **[RELAY-STATUS]** - Total utilization, totes with tasks vs empty, full relay count
2. **[ZONE-CAPACITY]** - Per-zone: capacity, reserved, available, task/empty breakdown
3. **[PROBLEM-RELAYS]** - Only relays that are full or have >50% empty totes (logged as WARN)
4. **[ORDER-STATUS]** - Pending, binned, completed orders

All logging integrates into the existing 5-second monitoring interval without creating new processes.
