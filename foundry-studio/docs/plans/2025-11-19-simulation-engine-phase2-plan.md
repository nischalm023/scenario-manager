# Simulation Engine Phase 2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add section markers, method documentation, and switch expressions to DesmojWarehouseModel and DesmojSimulationEngine for improved maintainability.

**Architecture:** Pure documentation and code style improvements with no functional changes. Section markers divide large files into navigable sections, method Javadoc explains public API and overrides, and switch expressions replace if-else chains for SystemType handling.

**Tech Stack:** Java 21, Desmo-J

---

## Task 1: Add Section Markers to DesmojWarehouseModel

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Add section markers**

Add section markers throughout the file. Insert before each logical section:

After line 52 (after class declaration, before record):
```java

  // ========================================
  // Nested Types
  // ========================================

```

After line 66 (after record, before fields):
```java

  // ========================================
  // Fields
  // ========================================

```

Before the internal accessors section (around line 92):
```java

  // ========================================
  // Internal Accessors
  // ========================================

```

Before the constructor:
```java

  // ========================================
  // Constructor
  // ========================================

```

After the constructor, before `markStorageSpaceActive`:
```java

  // ========================================
  // Public API
  // ========================================

```

Before `description()` override:
```java

  // ========================================
  // Desmo-J Lifecycle
  // ========================================

```

Before `scheduleRtpRobots()`:
```java

  // ========================================
  // Process Scheduling
  // ========================================

```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "style: add section markers to DesmojWarehouseModel"
```

---

## Task 2: Add Method Documentation to DesmojWarehouseModel

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Add Javadoc to override methods**

Find `description()` method and add:
```java
  /** Return model description for Desmo-J reporting. */
  @Override
  public String description() {
```

Find `init()` method and add:
```java
  /** Initialize operator resources and pathfinder. */
  @Override
  public void init() {
```

Find `doInitialSchedules()` method and add:
```java
  /** Schedule all simulation processes (robots, bin management, monitoring). */
  @Override
  public void doInitialSchedules() {
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "docs: add method documentation to DesmojWarehouseModel"
```

---

## Task 3: Convert if-else to Switch in DesmojWarehouseModel

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Convert SystemType if-else to switch**

Find in `doInitialSchedules()` method (around line 247):
```java
    // 1. Schedule robot processes
    if (config.systemType() == SystemType.RTP) {
      scheduleRtpRobots();
    } else if (config.systemType() == SystemType.RELAY) {
      scheduleRelayRobots();
    } else {
      log.warn("Unknown system type: {}", config.systemType());
    }
```

Replace with:
```java
    // 1. Schedule robot processes
    switch (config.systemType()) {
      case RTP -> scheduleRtpRobots();
      case RELAY -> scheduleRelayRobots();
    }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "refactor: use switch expression for SystemType in DesmojWarehouseModel"
```

---

## Task 4: Add Section Markers to DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Add section markers**

After line 47 (after class declaration, before fields):
```java

  // ========================================
  // Fields
  // ========================================

```

Before the constructor (around line 94):
```java

  // ========================================
  // Constructor
  // ========================================

```

After constructor, before `run()`:
```java

  // ========================================
  // Public API
  // ========================================

```

Before `initializeRelaySystem()`:
```java

  // ========================================
  // Simulation Lifecycle
  // ========================================

```

Before `assignTasksAndUpdateTracking()`:
```java

  // ========================================
  // Order Processing
  // ========================================

```

Before `optimizeInitialStorageSpacePositions()`:
```java

  // ========================================
  // Utilities
  // ========================================

```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "style: add section markers to DesmojSimulationEngine"
```

---

## Task 5: Add Method Documentation to DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Add Javadoc to public methods**

Find `run()` method and update comment to Javadoc:
```java
  /** Run the simulation from start to completion. */
  public void run() {
```

Find `now()` method and add:
```java
  /** Get current simulation time in seconds. */
  public double now() {
```

Find `optimizeInitialStorageSpacePositions()` method and add:
```java
  /**
   * Optimize initial storage space positions using configured put algorithm.
   *
   * <p>Assigns racks to optimal grid positions based on popularity and distance.
   */
  public void optimizeInitialStorageSpacePositions() {
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "docs: add method documentation to DesmojSimulationEngine"
```

---

## Task 6: Convert if-else to Switch in DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Convert first SystemType if-else (rackIdValue calculation)**

Find in `assignTasksAndUpdateTracking()` method (around line 330):
```java
      // Get rack ID for metrics event (needed for both RTP and Relay)
      int rackIdValue;
      if (config.systemType() == SystemType.RTP) {
        // For RTP, storage space ID is directly a RackId
        rackIdValue = ((RackId) task.getStorageSpaceId()).value();
      } else {
        // For Relay, storage space ID is a ToteId - need to get rack from tote
        var toteId = (ToteId) task.getStorageSpaceId();
        Tote tote = storageSpaceManager().getTote(toteId);
        if (tote != null && tote.getCurrentHoldingEntity() instanceof RackId) {
          rackIdValue = ((RackId) tote.getCurrentHoldingEntity()).value();
        } else {
          // Tote not found or not in rack - use 0 as placeholder
          rackIdValue = 0;
          log.warn("Tote {} not found or not in rack for task {}", toteId, task.getTaskId());
        }
      }
```

Replace with:
```java
      // Get rack ID for metrics event (needed for both RTP and Relay)
      int rackIdValue = switch (config.systemType()) {
        case RTP -> ((RackId) task.getStorageSpaceId()).value();
        case RELAY -> {
          var toteId = (ToteId) task.getStorageSpaceId();
          Tote tote = storageSpaceManager().getTote(toteId);
          if (tote != null && tote.getCurrentHoldingEntity() instanceof RackId rackId) {
            yield rackId.value();
          }
          log.warn("Tote {} not found or not in rack for task {}", toteId, task.getTaskId());
          yield 0;
        }
      };
```

**Step 2: Convert second SystemType if-else (task assignment)**

Find later in the same method (around line 352):
```java
      // Find the storage space for this task (same as original SimulationEngine)
      if (config.systemType() == SystemType.RTP) {
        // For RTP, storage space ID is a RackId
        var rackId = (RackId) task.getStorageSpaceId();
        Rack rack = storageSpaceManager().getRack(rackId);
        if (rack != null) {
          rack.addTask(task);
        }
      } else {
        // For Relay, storage space ID is a ToteId
        var toteId = (ToteId) task.getStorageSpaceId();
        Tote tote = storageSpaceManager().getTote(toteId);
        if (tote != null) {
          tote.addTask(task);
        }
      }
```

Replace with:
```java
      // Find the storage space for this task and add the task
      switch (config.systemType()) {
        case RTP -> {
          var rackId = (RackId) task.getStorageSpaceId();
          Rack rack = storageSpaceManager().getRack(rackId);
          if (rack != null) {
            rack.addTask(task);
          }
        }
        case RELAY -> {
          var toteId = (ToteId) task.getStorageSpaceId();
          Tote tote = storageSpaceManager().getTote(toteId);
          if (tote != null) {
            tote.addTask(task);
          }
        }
      }
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "refactor: use switch expressions for SystemType in DesmojSimulationEngine"
```

---

## Task 7: Format and Run Tests

**Files:**
- All modified files

**Step 1: Format code**

Run: `cd apps/backend && mvn spotless:apply`
Expected: BUILD SUCCESS

**Step 2: Run all tests**

Run: `cd apps/backend && mvn test`
Expected: BUILD SUCCESS with all tests passing

**Step 3: Commit formatting if changed**

```bash
git add -A
git commit -m "style: format code with spotless"
```

---

## Summary

| Task | Description | Estimated Time |
|------|-------------|----------------|
| 1 | Add section markers to DesmojWarehouseModel | 5 min |
| 2 | Add method documentation to DesmojWarehouseModel | 3 min |
| 3 | Convert if-else to switch in DesmojWarehouseModel | 2 min |
| 4 | Add section markers to DesmojSimulationEngine | 5 min |
| 5 | Add method documentation to DesmojSimulationEngine | 3 min |
| 6 | Convert if-else to switch in DesmojSimulationEngine | 5 min |
| 7 | Format and run tests | 3 min |

**Total: ~26 minutes**
