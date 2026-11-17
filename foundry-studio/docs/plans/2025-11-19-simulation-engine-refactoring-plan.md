# Simulation Engine Refactoring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor DesmojWarehouseModel and DesmojSimulationEngine to reduce constructor complexity, add thread safety, and improve exception handling.

**Architecture:** Extract redundant parameters from constructor by having model access them from warehouseComponents internally. Group order-related dependencies into OrderProcessingRuntime record. Use ConcurrentHashMap for shared state. Add proper cleanup in finally block.

**Tech Stack:** Java 21, Desmo-J, Spring Boot, Lombok

---

## Task 1: Add OrderProcessingRuntime Record

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Add the record as nested class**

Add after the class declaration (around line 52), before the fields:

```java
  /**
   * Runtime dependencies for order processing.
   *
   * @param orderManager Manages order lifecycle and bin assignment
   * @param orderCompletionCallback Called when all tasks for an order complete
   * @param orderTaskMapping Tracks task IDs for each order (shared with engine)
   * @param zoneLoadTracker Tracks zone load for relay systems (may be null)
   */
  record OrderProcessingRuntime(
      OrderManager orderManager,
      BiConsumer<String, String> orderCompletionCallback,
      Map<String, List<String>> orderTaskMapping,
      ZoneLoadTracker zoneLoadTracker) {}
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "feat: add OrderProcessingRuntime record to DesmojWarehouseModel"
```

---

## Task 2: Add Internal Accessor Methods

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Add accessor methods**

Add after the fields section (around line 83), before the constructor:

```java
  // ========================================
  // Internal Accessors (from warehouseComponents)
  // ========================================

  private StoragePositionManager storagePositionManager() {
    return warehouseComponents.storagePositionManager();
  }

  private StorageSpaceManager storageSpaceManager() {
    return warehouseComponents.storageSpaceManager();
  }

  private ZoneManager zoneManager() {
    return warehouseComponents.zoneManager();
  }

  private RelayPositionManager relayPositionManager() {
    return warehouseComponents.relayPositionManager();
  }

  private GridPopularityManager gridPopularityManager() {
    return warehouseComponents.gridPopularityManager();
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "feat: add internal accessor methods for warehouseComponents"
```

---

## Task 3: Update DesmojWarehouseModel Constructor

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Update field declarations**

Remove these fields (they become local accessors):
- `private final StoragePositionManager storagePositionManager;` (line 65)
- `private final StorageSpaceManager storageSpaceManager;` (line 66)
- `private final ZoneManager zoneManager;` (line 71)
- `private final GridPopularityManager gridPopularityManager;` (line 73)

Change `@Getter private final OrderManager orderManager;` to just store locally (we'll access via record).

Add new field for the runtime:
```java
  private final OrderProcessingRuntime orderProcessingRuntime;
```

**Step 2: Update constructor signature**

Replace the constructor (lines 85-117) with:

```java
  public DesmojWarehouseModel(
      Model owner,
      String modelName,
      boolean showInReport,
      boolean showInTrace,
      SimulationConfig config,
      WarehouseComponents warehouseComponents,
      MetricsManager metricsManager,
      OrderProcessingRuntime orderProcessingRuntime) {

    super(owner, modelName, showInReport, showInTrace);

    this.config = config;
    this.warehouseComponents = warehouseComponents;
    this.metricsManager = metricsManager;
    this.orderProcessingRuntime = orderProcessingRuntime;
  }
```

**Step 3: Update getOrderManager() to use runtime**

Add a getter that delegates to the runtime:
```java
  public OrderManager getOrderManager() {
    return orderProcessingRuntime.orderManager();
  }
```

**Step 4: Update internal usages**

Replace direct field references:
- `orderManager` → `orderProcessingRuntime.orderManager()`
- `orderCompletionCallback` → `orderProcessingRuntime.orderCompletionCallback()`
- `orderTaskMapping` → `orderProcessingRuntime.orderTaskMapping()`
- `zoneLoadTracker` → `orderProcessingRuntime.zoneLoadTracker()`
- `storagePositionManager` → `storagePositionManager()` (method call)
- `storageSpaceManager` → `storageSpaceManager()` (method call)
- `zoneManager` → `zoneManager()` (method call)
- `gridPopularityManager` → `gridPopularityManager()` (method call)

Note: Keep `this.zoneManager` field for relay position manager setter - remove the `@Getter` and use accessor method instead.

**Step 5: Compile (will fail - engine not updated yet)**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD FAILURE (DesmojSimulationEngine still uses old constructor)

**Step 6: Commit work in progress**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "refactor: simplify DesmojWarehouseModel constructor to 8 params

WIP: DesmojSimulationEngine needs update to match"
```

---

## Task 4: Update DesmojSimulationEngine Collections

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Update imports**

Add at top of imports:
```java
import java.util.concurrent.ConcurrentHashMap;
```

**Step 2: Update collection declarations**

Replace lines 83-84:

```java
// Before
private final Map<String, List<String>> orderTaskMapping = new HashMap<>();
private final Set<String> completedTasks = new HashSet<>();

// After
private final Map<String, List<String>> orderTaskMapping = new ConcurrentHashMap<>();
private final Set<String> completedTasks = ConcurrentHashMap.newKeySet();
```

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "refactor: use ConcurrentHashMap for thread-safe collections"
```

---

## Task 5: Add Cleanup Method and Update run()

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Add cleanupExperiment method**

Add after `runSimulation()` method (around line 462):

```java
  /** Clean up experiment resources. */
  private void cleanupExperiment() {
    if (experiment != null) {
      try {
        experiment.finish();
      } catch (Exception ignored) {
        // Already finished or failed - ignore
      }
    }
  }
```

**Step 2: Update run() method**

Replace the run() method (lines 172-191):

```java
  /** Run the simulation (main entry point). */
  public void run() {
    log.info("Starting Desmo-J simulation engine...");

    try {
      // Initialize relay system components if needed
      if (config.systemType() == SystemType.RELAY) {
        initializeRelaySystem();
      }

      loadAndProcessOrders();
      setupExperiment();
      runSimulation();

      log.info("Simulation completed successfully");

    } catch (Exception e) {
      log.error("Simulation failed: {}", e.getMessage(), e);
      throw new RuntimeException("Desmo-J simulation failed", e);
    } finally {
      cleanupExperiment();
    }
  }
```

**Step 3: Update runSimulation() to not call finish()**

In `runSimulation()` method, remove the `experiment.finish()` call since cleanup handles it now.

Replace lines 439-462:

```java
  /** Execute the Desmo-J simulation. */
  private void runSimulation() {
    log.info("Running Desmo-J simulation for {}s...", config.simulationDuration());

    long startTime = System.currentTimeMillis();

    // Emit simulation start event
    metricsManager.emit(DomainEvent.simulationStarted(0.0)); // t=0

    // Start simulation (blocking call)
    experiment.start();

    // Note: experiment.finish() is called in cleanupExperiment()

    long endTime = System.currentTimeMillis();
    double realTime = (endTime - startTime) / 1000.0;
    double simulationTime = model.presentTime().getTimeAsDouble(TimeUnit.SECONDS);

    log.info("Simulation completed at t={} (real time: {}s)", simulationTime, realTime);

    // Emit simulation completion event
    metricsManager.emit(DomainEvent.simulationCompleted(simulationTime, simulationTime));
  }
```

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "refactor: add experiment cleanup in finally block"
```

---

## Task 6: Fail Fast on Empty Orders

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Update loadAndProcessOrders()**

In `loadAndProcessOrders()` method, replace the empty check (around line 295-298):

```java
// Before
if (batchOrders.isEmpty()) {
    log.warn("No orders loaded from scenario {}", config.scenarioId());
    return;
}

// After
if (batchOrders.isEmpty()) {
    throw new IllegalStateException(
        "No orders loaded from scenario: " + config.scenarioId());
}
```

**Step 2: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "fix: fail fast when no orders loaded from scenario"
```

---

## Task 7: Update setupExperiment() to Use New Constructor

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Import the record**

Add import:
```java
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel.OrderProcessingRuntime;
```

**Step 2: Update model construction**

In `setupExperiment()` method, replace the model construction (lines 380-397):

```java
    // Create order processing runtime
    var orderRuntime = new OrderProcessingRuntime(
        orderManager,
        this::checkOrderCompletion,
        orderTaskMapping,
        zoneLoadTracker);

    // Create the warehouse model with simplified constructor
    model =
        new DesmojWarehouseModel(
            null, // no owner
            "WarehouseSimulation",
            false, // show in report
            false, // disable trace for production
            config,
            warehouseComponents,
            metricsManager,
            orderRuntime);
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "refactor: update setupExperiment to use new model constructor"
```

---

## Task 8: Format and Run Tests

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
| 1 | Add OrderProcessingRuntime record | 3 min |
| 2 | Add internal accessor methods | 3 min |
| 3 | Update DesmojWarehouseModel constructor | 10 min |
| 4 | Update collections to ConcurrentHashMap | 2 min |
| 5 | Add cleanup method and update run() | 5 min |
| 6 | Fail fast on empty orders | 2 min |
| 7 | Update setupExperiment() | 5 min |
| 8 | Format and run tests | 5 min |

**Total: ~35 minutes**
