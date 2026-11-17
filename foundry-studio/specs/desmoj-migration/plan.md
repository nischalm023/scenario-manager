# Desmo-J Migration Implementation Plan

**Date**: 2025-11-08
**Status**: Ready for Implementation
**Goal**: Replace custom DES implementation with Desmo-J to eliminate race conditions

---

## Executive Summary

This plan details the step-by-step migration from the custom DES implementation (`infrastructure/des/`) to Desmo-J (`infrastructure/desmoj/`). The PoC has validated that Desmo-J successfully handles the race condition scenario (12/12 cycles completed vs 0 in custom DES).

**Migration Strategy**: Incremental replacement with parallel packages
- Keep `infrastructure/des/` unchanged (untouched)
- Build complete `infrastructure/desmoj/` implementation
- Direct replacement in `SimulationEngine` (no config flag)
- Start with RTP system only (relay system deferred to Phase 2)

**Success Criteria**:
1. Simulation completes without race conditions
2. Metrics are collected and non-zero
3. API compatibility maintained (SimulationResults format unchanged)
4. End-to-end integration test passes

---

## Table of Contents

1. [Business Requirements](#business-requirements)
2. [Technical Architecture](#technical-architecture)
3. [API Contracts & Data Models](#api-contracts--data-models)
4. [Component Interactions](#component-interactions)
5. [Implementation Tasks](#implementation-tasks)
6. [Testing Strategy](#testing-strategy)
7. [Success Criteria](#success-criteria)

---

## Business Requirements

### Problem Statement

The custom DES implementation in `infrastructure/des/` has a critical race condition:
- Event loop thread polls queue faster than virtual threads can schedule events
- Simulation terminates prematurely (t~16s) with zero metrics
- Documented in `docs/des-architecture-issues.md` as Issue #2

### Business Goals

1. **Eliminate Race Conditions**: Ensure all robot cycles complete
2. **Maintain API Compatibility**: No changes to frontend API contracts
3. **Preserve Functionality**: Keep all existing simulation features working
4. **Incremental Migration**: Low-risk approach with parallel implementations

### User Stories

**As a simulation engineer**, I want:
- The simulation to complete all robot work cycles without premature termination
- Accurate metrics reflecting actual simulation results (not zeros)
- The same API response format so frontend integration is unaffected

**As a developer**, I want:
- Clear separation between old (des/) and new (desmoj/) implementations
- Ability to reference custom DES code during migration
- Comprehensive tests validating the migration

---

## Technical Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────────┐
│                      Simulator                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │Warehouse     │  │Metrics       │  │Simulation    │      │
│  │Builder       │  │Manager       │  │Engine        │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│                                             │                │
│                                             ▼                │
│                   ┌────────────────────────────────────┐    │
│                   │   DesmojSimulationEngine           │    │
│                   │   (NEW - replaces custom DES)      │    │
│                   └────────────────────────────────────┘    │
│                                   │                          │
│              ┌────────────────────┼────────────────────┐    │
│              ▼                    ▼                     ▼    │
│    ┌──────────────────┐  ┌──────────────────┐  ┌──────────┐│
│    │DesmojWarehouse   │  │DesmojRtpRobot    │  │Desmo-J   ││
│    │Model             │  │Process           │  │Experiment││
│    │(extends Model)   │  │(extends SimProcess│  │          ││
│    └──────────────────┘  └──────────────────┘  └──────────┘│
└─────────────────────────────────────────────────────────────┘

Infrastructure Packages:
  infrastructure/des/      ← Custom DES (UNTOUCHED)
  infrastructure/desmoj/   ← New Desmo-J impl
```

### Package Structure

**Before Migration**:
```
apps/backend-java/src/main/java/com/greyorange/dessimulator/
├── infrastructure/
│   ├── des/              # Custom DES (has race conditions)
│   │   ├── core/
│   │   ├── event/
│   │   ├── process/
│   │   └── resource/
│   └── des1/             # PoC code (to be renamed)
│       ├── DesmojPocRunner.java
│       ├── metrics/
│       ├── model/
│       └── process/
└── simulator/
    ├── SimulationEngine.java  # Uses custom DES
    ├── Simulator.java
    └── ...
```

**After Migration**:
```
apps/backend-java/src/main/java/com/greyorange/dessimulator/
├── infrastructure/
│   ├── des/              # Custom DES (PRESERVED, untouched)
│   │   └── ...
│   └── desmoj/           # Desmo-J implementation (NEW)
│       ├── DesmojSimulationEngine.java
│       ├── model/
│       │   └── DesmojWarehouseModel.java
│       ├── process/
│       │   ├── DesmojRtpRobotProcess.java
│       │   └── DesmojStationProcess.java
│       └── metrics/
│           └── DesmojMetricsCollector.java
└── simulator/
    ├── SimulationEngine.java  # Now uses DesmojSimulationEngine
    ├── Simulator.java
    └── ...
```

### Component Responsibilities

#### 1. DesmojSimulationEngine
- **Purpose**: Coordinates entire simulation using Desmo-J
- **Replaces**: `infrastructure/des/core/SimulationEngine`
- **Key Methods**:
  - `run(SimulationConfig, WarehouseComponents, MetricsManager)`
  - `setupExperiment()`: Configure Desmo-J experiment
  - `initializeModel()`: Create DesmojWarehouseModel
  - `runSimulation()`: Execute experiment.start() → finish()
  - `collectResults()`: Extract metrics from model

#### 2. DesmojWarehouseModel
- **Purpose**: Desmo-J model containing all simulation entities
- **Extends**: `desmoj.core.simulator.Model`
- **Key Methods**:
  - `init()`: Create resources (operator Res, station Res)
  - `doInitialSchedules()`: Activate robot processes
  - `description()`: Return model description

#### 3. DesmojRtpRobotProcess
- **Purpose**: Robot lifecycle for RTP system
- **Extends**: `desmoj.core.simulator.SimProcess`
- **Key Methods**:
  - `lifeCycle()`: Main robot loop (find rack → transport → process → return)
  - Uses `hold()` for delays
  - Uses `Res.provide()` / `takeBack()` for operator blocking

#### 4. DesmojStationProcess (Optional)
- **Purpose**: Station-side operations (if needed)
- **Alternative**: Integrate into robot process directly

---

## API Contracts & Data Models

### Public API (Unchanged)

**Endpoint**: `POST /api/simulations/run`

**Request** (SimulationRequest - unchanged):
```json
{
  "simulationDuration": 14400.0,
  "scenarioId": "scenario_a",
  "systemType": "RTP",
  "mapConfig": {...},
  "robotConfig": {
    "configType": "rtp",
    "numRobots": 4,
    "robotSpeed": 1.5
  },
  "operatorConfig": {...},
  "algorithmConfig": {...}
}
```

**Response** (SimulationResults - unchanged):
```json
{
  "success": true,
  "timestamp": "2025-11-08T10:30:00",
  "error": null,
  "config": {...},
  "stations": [...],
  "racks": [...],
  "robots": [...],
  "metrics": {
    "taskMetrics": {...},
    "robotMetrics": {...},
    "operatorMetrics": {...},
    ...
  }
}
```

### Internal Data Models

#### DesmojSimulationEngine Constructor

```java
public DesmojSimulationEngine(
    SimulationConfig config,
    WarehouseComponents warehouseComponents,
    MetricsManager metricsManager
)
```

**Same signature as current SimulationEngine** (line 76-79 of SimulationEngine.java)

#### DesmojWarehouseModel

```java
public class DesmojWarehouseModel extends Model {
    // Configuration
    private final SimulationConfig config;
    private final WarehouseComponents warehouseComponents;
    private final MetricsManager metricsManager;

    // Desmo-J Resources
    private Map<StationId, Res> operatorResources;

    // Domain components (passed from WarehouseBuilder)
    private final WarehouseMap map;
    private final List<Robot> robots;
    private final List<Rack> racks;
    private final StationManager stationManager;
    private final WarehouseInventory inventory;
    // ... (same as current SimulationEngine fields)
}
```

#### DesmojRtpRobotProcess

```java
public class DesmojRtpRobotProcess extends SimProcess {
    private final Robot robot;  // Domain model (unchanged)
    private final DesmojWarehouseModel model;
    private final MetricsManager metricsManager;

    @Override
    public void lifeCycle() {
        // Robot loop: find rack → transport → process → return
    }
}
```

---

## Component Interactions

### Sequence Diagram: Simulation Execution

```
Simulator                    WarehouseBuilder           DesmojSimulationEngine        Desmo-J Experiment
    │                              │                            │                          │
    │──buildWarehouse()──────────> │                            │                          │
    │                              │                            │                          │
    │<─WarehouseComponents─────────│                            │                          │
    │                              │                            │                          │
    │──initializeMetrics()────────────────────────────────────> │                          │
    │                                                            │                          │
    │──new DesmojSimulationEngine(config, components, metrics)─>│                          │
    │                                                            │                          │
    │──run()──────────────────────────────────────────────────> │                          │
    │                                                            │                          │
    │                                                            │──new Experiment()──────> │
    │                                                            │                          │
    │                                                            │──new Model()──────────>  │
    │                                                            │                          │
    │                                                            │──model.connectToExperiment()
    │                                                            │                          │
    │                                                            │──experiment.start()────> │
    │                                                            │                          │
    │                                                            │        [Robots execute]  │
    │                                                            │        [Metrics collected]
    │                                                            │                          │
    │                                                            │──experiment.finish()───> │
    │                                                            │                          │
    │                                                            │──collectMetrics()──────> │
    │                                                            │                          │
    │<────────────────SimulationResults─────────────────────────│                          │
```

### Robot Process Flow (Desmo-J)

```
DesmojRtpRobotProcess.lifeCycle():
  │
  ├─while (simulation running):
  │   │
  │   ├─1. Find available racks with tasks
  │   │    └─ If none: hold(IDLE_CHECK_INTERVAL)
  │   │
  │   ├─2. Select optimal rack (closest)
  │   │
  │   ├─3. Get stations to visit
  │   │
  │   ├─4. For each station:
  │   │    ├─ Transport rack to station
  │   │    │   ├─ moveToRack()
  │   │    │   ├─ hold(LOADING_TIME)
  │   │    │   └─ moveToStation()
  │   │    │
  │   │    ├─ Acquire operator resource
  │   │    │   └─ operatorResource.provide(1)  ← BLOCKS if busy
  │   │    │
  │   │    ├─ Process tasks
  │   │    │   ├─ For each task: hold(operatorTimePerPick)
  │   │    │   └─ Emit TASK_COMPLETED events
  │   │    │
  │   │    └─ Release operator
  │   │        └─ operatorResource.takeBack(1)
  │   │
  │   ├─5. Return rack to storage
  │   │    ├─ moveToStorage()
  │   │    └─ hold(UNLOADING_TIME)
  │   │
  │   └─6. Emit RACK_CYCLE_COMPLETED event
  │
  └─Process completes
```

### Metrics Collection Flow

```
DesmojRobotProcess ──────> MetricsManager.emitEvent()
                              │
                              ├─> TaskMetricsCollector
                              ├─> RobotMetricsCollector
                              ├─> OperatorMetricsCollector
                              └─> ...

MetricsManager.getComprehensiveMetrics() ──> SimulationMetrics
                                               │
                                               └─> Returned to Simulator
```

**Key Difference from Custom DES**:
- Desmo-J manages simulation time internally
- `engine.now()` replaced with `presentTime()` (from SimProcess)
- No manual event scheduling needed

---

## Implementation Tasks

### Phase 1: Package Refactoring

#### Task 1.1: Rename des1/ → desmoj/

**Files to Move**:
- `infrastructure/des1/` → `infrastructure/desmoj/`

**Commands**:
```bash
cd apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure
git mv des1 desmoj
```

**Files to Update (imports)**:
```java
// OLD
import com.greyorange.dessimulator.infrastructure.des1.*;

// NEW
import com.greyorange.dessimulator.infrastructure.desmoj.*;
```

**Affected Files**:
- `infrastructure/desmoj/DesmojPocRunner.java`
- `infrastructure/desmoj/model/DesmojWarehouseModel.java`
- `infrastructure/desmoj/process/DesmojRobotProcess.java`
- `infrastructure/desmoj/metrics/DesmojMetrics.java`
- `src/test/java/.../DesmojPocTest.java`

**Verification**:
```bash
mvn clean compile
# Should compile without errors
```

---

### Phase 2: Core Infrastructure

#### Task 2.1: Create DesmojSimulationEngine

**File**: `infrastructure/desmoj/DesmojSimulationEngine.java`

**Purpose**: Replace `infrastructure/des/core/SimulationEngine` with Desmo-J-based implementation

**Full Implementation**:

```java
package com.greyorange.dessimulator.infrastructure.desmoj;

import com.greyorange.dessimulator.simulator.SimulationConfig;
import com.greyorange.dessimulator.simulator.models.WarehouseComponents;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import com.greyorange.dessimulator.infrastructure.desmoj.model.DesmojWarehouseModel;
import desmoj.core.simulator.Experiment;
import desmoj.core.simulator.TimeInstant;
import java.util.concurrent.TimeUnit;
import lombok.extern.slf4j.Slf4j;

/**
 * Desmo-J based simulation engine.
 *
 * Replaces custom DES implementation to eliminate race conditions.
 * Uses Desmo-J's proven process-oriented simulation framework.
 */
@Slf4j
public class DesmojSimulationEngine {
  private final SimulationConfig config;
  private final WarehouseComponents warehouseComponents;
  private final MetricsManager metricsManager;

  private DesmojWarehouseModel model;
  private Experiment experiment;

  /**
   * Constructor matching original SimulationEngine signature.
   *
   * @param config Simulation configuration
   * @param warehouseComponents Pre-built warehouse (from WarehouseBuilder)
   * @param metricsManager Metrics collection system
   */
  public DesmojSimulationEngine(
      SimulationConfig config,
      WarehouseComponents warehouseComponents,
      MetricsManager metricsManager) {

    this.config = config;
    this.warehouseComponents = warehouseComponents;
    this.metricsManager = metricsManager;

    log.info("DesmojSimulationEngine initialized for {} system with {} robots",
        config.systemType(), warehouseComponents.robots().size());
  }

  /**
   * Run the simulation (main entry point).
   *
   * This method signature matches the original SimulationEngine.run()
   */
  public void run() {
    log.info("Starting Desmo-J simulation engine...");

    try {
      // Phase 1: Load orders and assign tasks
      loadAndProcessOrders();

      // Phase 2: Setup Desmo-J experiment and model
      setupExperiment();

      // Phase 3: Run simulation
      runSimulation();

      // Phase 4: Generate final report
      generateReport();

      log.info("Simulation completed successfully");

    } catch (Exception e) {
      log.error("Simulation failed: {}", e.getMessage(), e);
      throw new RuntimeException("Desmo-J simulation failed", e);
    }
  }

  /**
   * Load orders from scenario and assign tasks to racks.
   */
  private void loadAndProcessOrders() {
    log.info("Loading batch orders from scenario: {}", config.scenarioId());

    // TODO: Implement order loading (copy from original SimulationEngine lines 183-218)
    // This is the same logic as custom DES - no changes needed

    log.info("Orders loaded and tasks assigned");
  }

  /**
   * Setup Desmo-J experiment and warehouse model.
   */
  private void setupExperiment() {
    log.info("Setting up Desmo-J experiment...");

    // Create warehouse model
    model = new DesmojWarehouseModel(
        null,  // no owner
        "WarehouseSimulation",
        true,  // show in report
        false, // disable trace for production
        config,
        warehouseComponents,
        metricsManager
    );

    // Create experiment
    experiment = new Experiment("WarehouseSimulationExperiment");

    // Connect model to experiment
    model.connectToExperiment(experiment);

    // Configure experiment
    experiment.setShowProgressBar(false);

    // Disable trace/debug for production (can enable for debugging)
    experiment.tracePeriod(
        new TimeInstant(0, TimeUnit.SECONDS),
        new TimeInstant(0, TimeUnit.SECONDS)
    );

    // Set simulation stop time
    double duration = config.simulationDuration();
    experiment.stop(new TimeInstant(duration, TimeUnit.SECONDS));

    log.info("Experiment configured: duration={}s, robots={}",
        duration, warehouseComponents.robots().size());
  }

  /**
   * Execute the Desmo-J simulation.
   */
  private void runSimulation() {
    log.info("Running Desmo-J simulation for {}s...", config.simulationDuration());

    long startTime = System.currentTimeMillis();

    // Emit simulation start event
    metricsManager.emitEvent(
        com.greyorange.dessimulator.simulator.metrics.MetricEvent.SIMULATION_STARTED,
        0.0  // t=0
    );

    // Start simulation (blocking call)
    experiment.start();

    // Wait for completion
    experiment.finish();

    long endTime = System.currentTimeMillis();
    double realTime = (endTime - startTime) / 1000.0;

    log.info("Simulation completed at t={} (real time: {}s)",
        model.presentTime(), realTime);

    // Emit simulation completion event
    metricsManager.emitEvent(
        com.greyorange.dessimulator.simulator.metrics.MetricEvent.SIMULATION_COMPLETED,
        model.presentTime().getTimeAsDouble(TimeUnit.SECONDS)
    );
  }

  /**
   * Generate Desmo-J report (optional).
   */
  private void generateReport() {
    log.info("Generating Desmo-J report...");
    experiment.report();
    log.info("Report generated");
  }

  /**
   * Get current simulation time.
   *
   * @return Current simulation time in seconds
   */
  public double now() {
    if (model != null) {
      return model.presentTime().getTimeAsDouble(TimeUnit.SECONDS);
    }
    return 0.0;
  }
}
```

**Key Design Decisions**:
1. **Same constructor signature** as original `SimulationEngine` (line 76-79)
2. **Same `run()` method** - drop-in replacement
3. **Delegates to Desmo-J** for event scheduling and time management
4. **Reuses MetricsManager** - no changes to metrics collection

**Verification**:
```bash
mvn clean compile
# Should compile without errors
```

---

#### Task 2.2: Enhance DesmojWarehouseModel

**File**: `infrastructure/desmoj/model/DesmojWarehouseModel.java`

**Changes from PoC**:
1. Accept full `WarehouseComponents` instead of hardcoded values
2. Create operator resources for ALL stations (not just 1)
3. Initialize domain components (OrderManager, StoragePositionManager, etc.)
4. Schedule ALL robots (not just 4)

**Full Implementation**:

```java
package com.greyorange.dessimulator.infrastructure.desmoj.model;

import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.order.OrderManager;
import com.greyorange.dessimulator.domain.put.StoragePositionManager;
import com.greyorange.dessimulator.domain.robot.Robot;
import com.greyorange.dessimulator.domain.station.Station;
import com.greyorange.dessimulator.domain.station.StationManager;
import com.greyorange.dessimulator.domain.storage.StorageSpaceManager;
import com.greyorange.dessimulator.infrastructure.desmoj.process.DesmojRtpRobotProcess;
import com.greyorange.dessimulator.simulator.SimulationConfig;
import com.greyorange.dessimulator.simulator.models.WarehouseComponents;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import desmoj.core.advancedModellingFeatures.Res;
import desmoj.core.simulator.Model;
import desmoj.core.simulator.TimeInstant;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;

/**
 * Desmo-J warehouse model for production simulation.
 *
 * This model:
 * 1. Creates operator resources for each station
 * 2. Initializes domain managers (OrderManager, StoragePositionManager, etc.)
 * 3. Schedules robot processes based on system type (RTP/Relay)
 */
@Slf4j
public class DesmojWarehouseModel extends Model {
  // Configuration
  private final SimulationConfig config;
  private final WarehouseComponents warehouseComponents;
  private final MetricsManager metricsManager;

  // Desmo-J Resources (operator capacity at stations)
  @Getter private final Map<StationId, Res> operatorResources = new HashMap<>();

  // Domain Managers (initialized here, passed to processes)
  @Getter private OrderManager orderManager;
  @Getter private StoragePositionManager storagePositionManager;
  @Getter private StorageSpaceManager storageSpaceManager;

  public DesmojWarehouseModel(
      Model owner,
      String modelName,
      boolean showInReport,
      boolean showInTrace,
      SimulationConfig config,
      WarehouseComponents warehouseComponents,
      MetricsManager metricsManager) {

    super(owner, modelName, showInReport, showInTrace);

    this.config = config;
    this.warehouseComponents = warehouseComponents;
    this.metricsManager = metricsManager;
  }

  @Override
  public String description() {
    return String.format(
        "Warehouse simulation: %s system, %d robots, %d stations, %d racks",
        config.systemType(),
        warehouseComponents.robots().size(),
        warehouseComponents.stationManager().getAllStations().size(),
        warehouseComponents.racks().size()
    );
  }

  @Override
  public void init() {
    log.info("Initializing Desmo-J warehouse model...");

    // 1. Create operator resources (one per station, capacity=1)
    StationManager stationManager = warehouseComponents.stationManager();
    for (Station station : stationManager.getAllStations()) {
      Res operatorResource = new Res(
          this,
          "Operator_Station_" + station.getStationId().value(),
          1,  // capacity = 1 operator per station
          true,  // show in report
          false  // disable trace for production
      );
      operatorResources.put(station.getStationId(), operatorResource);
    }
    log.info("Created {} operator resources", operatorResources.size());

    // 2. Initialize domain managers
    WarehouseInventory inventory = warehouseComponents.inventory();

    this.orderManager = new OrderManager(
        inventory,
        stationManager.getAllStations(),
        config.operatorConfig().numBins(),
        config.algorithmConfig().orderSelectionAlgorithm()
    );

    this.storagePositionManager = new StoragePositionManager(
        warehouseComponents.map(),
        warehouseComponents.gridPopularityManager(),
        inventory
    );

    this.storageSpaceManager = new StorageSpaceManager(
        warehouseComponents.racks(),
        config.systemType() == SystemType.RELAY ? warehouseComponents.totes() : null
    );

    log.info("Domain managers initialized");
  }

  @Override
  public void doInitialSchedules() {
    log.info("Scheduling robot processes...");

    if (config.systemType() == SystemType.RTP) {
      scheduleRtpRobots();
    } else {
      // TODO: Relay system (Phase 2)
      log.warn("Relay system not yet implemented");
    }
  }

  /**
   * Schedule RTP robot processes.
   */
  private void scheduleRtpRobots() {
    List<Robot> robots = warehouseComponents.robots();

    for (int i = 0; i < robots.size(); i++) {
      Robot robot = robots.get(i);

      // Create robot process
      DesmojRtpRobotProcess robotProcess = new DesmojRtpRobotProcess(
          this,
          robot,
          true,  // show in trace (can disable for production)
          operatorResources,
          storageSpaceManager,
          warehouseComponents.inventory(),
          warehouseComponents.stationManager(),
          metricsManager
      );

      // Schedule with staggered start (0.5s apart to avoid initial contention)
      double startTime = i * 0.5;
      robotProcess.activate(new TimeInstant(startTime, TimeUnit.SECONDS));

      log.debug("Scheduled robot {} to start at t={}s", robot.getId(), startTime);
    }

    log.info("Scheduled {} RTP robot processes", robots.size());
  }

  /**
   * Get warehouse components.
   */
  public WarehouseComponents getWarehouseComponents() {
    return warehouseComponents;
  }

  /**
   * Get simulation configuration.
   */
  public SimulationConfig getConfig() {
    return config;
  }
}
```

**Key Changes from PoC**:
1. Accepts `WarehouseComponents` (racks, robots, stations from `WarehouseBuilder`)
2. Creates operator resources for **all stations** (not just 1)
3. Initializes domain managers (OrderManager, StoragePositionManager, etc.)
4. Schedules **all robots** (from config, not hardcoded 4)

**Verification**:
```bash
mvn clean compile
```

---

#### Task 2.3: Port RtpRobotOrchestrator → DesmojRtpRobotProcess

**File**: `infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

**Purpose**: Convert `RtpRobotOrchestrator` (custom DES) to Desmo-J `SimProcess`

**Mapping**:

| Custom DES | Desmo-J |
|------------|---------|
| `execute(SimulationEngine engine)` | `lifeCycle()` |
| `engine.timeout(duration)` | `hold(new TimeSpan(duration))` |
| `resource.acquire()` / `release()` | `res.provide(1)` / `takeBack(1)` |
| `engine.now()` | `presentTime().getTimeAsDouble(TimeUnit.SECONDS)` |
| `engine.isRunning()` | Loop runs until process completes |

**Full Implementation**:

```java
package com.greyorange.dessimulator.infrastructure.desmoj.process;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.RackState;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.robot.PickTask;
import com.greyorange.dessimulator.domain.robot.Robot;
import com.greyorange.dessimulator.domain.robot.RobotState;
import com.greyorange.dessimulator.domain.station.Station;
import com.greyorange.dessimulator.domain.station.StationManager;
import com.greyorange.dessimulator.domain.storage.StorageSpaceManager;
import com.greyorange.dessimulator.infrastructure.desmoj.model.DesmojWarehouseModel;
import com.greyorange.dessimulator.simulator.metrics.MetricEvent;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import desmoj.core.advancedModellingFeatures.Res;
import desmoj.core.simulator.Model;
import desmoj.core.simulator.SimProcess;
import desmoj.core.simulator.TimeSpan;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;
import lombok.extern.slf4j.Slf4j;

/**
 * Desmo-J RTP robot process.
 *
 * Implements robot lifecycle for Racks-to-Picker system:
 * 1. Find racks with tasks in storage
 * 2. Select optimal rack (closest)
 * 3. Transport rack to station(s)
 * 4. Wait for operator resource (blocking)
 * 5. Process tasks at station
 * 6. Release operator resource
 * 7. Return rack to storage
 * 8. Repeat
 */
@Slf4j
public class DesmojRtpRobotProcess extends SimProcess {
  // Domain components
  private final Robot robot;
  private final Map<StationId, Res> operatorResources;
  private final StorageSpaceManager storageSpaceManager;
  private final WarehouseInventory inventory;
  private final StationManager stationManager;
  private final MetricsManager metricsManager;

  // Timing constants (from RtpRobotOrchestrator)
  private static final double IDLE_CHECK_INTERVAL = 1.0;  // seconds
  private static final double LOADING_TIME = 6.0;
  private static final double UNLOADING_TIME = 6.0;

  public DesmojRtpRobotProcess(
      Model owner,
      Robot robot,
      boolean showInTrace,
      Map<StationId, Res> operatorResources,
      StorageSpaceManager storageSpaceManager,
      WarehouseInventory inventory,
      StationManager stationManager,
      MetricsManager metricsManager) {

    super(owner, "RtpRobot_" + robot.getId(), showInTrace);

    this.robot = robot;
    this.operatorResources = operatorResources;
    this.storageSpaceManager = storageSpaceManager;
    this.inventory = inventory;
    this.stationManager = stationManager;
    this.metricsManager = metricsManager;
  }

  @Override
  public void lifeCycle() {
    log.info("Robot {} starting lifecycle at t={}", robot.getId(), presentTime());

    try {
      // Main robot loop
      while (true) {
        // 1. Find available racks with tasks
        List<Rack> availableRacks = storageSpaceManager.getAllRacks().stream()
            .filter(rack -> rack.hasTasks() && rack.getState() == RackState.IN_STORAGE)
            .collect(Collectors.toList());

        log.debug("Robot {} checking for work at t={}: {} available racks",
            robot.getId(), presentTime(), availableRacks.size());

        if (availableRacks.isEmpty()) {
          // No work available - go idle
          robot.setState(RobotState.IDLE);
          emitEvent(MetricEvent.ROBOT_IDLE, "robot_id", robot.getId());

          log.debug("Robot {} idle at t={}, checking again in {}s",
              robot.getId(), presentTime(), IDLE_CHECK_INTERVAL);

          hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
          robot.addIdleTime(IDLE_CHECK_INTERVAL);
          continue;
        }

        // 2. Select optimal rack (closest)
        Rack selectedRack = selectOptimalRack(robot, availableRacks);
        if (selectedRack == null) {
          log.warn("Robot {}: No rack selected", robot.getId());
          hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
          continue;
        }

        // 3. Get stations to visit
        Map<StationId, List<PickTask>> stationTasks = selectedRack.getTasksByStation();
        if (stationTasks.isEmpty()) {
          log.warn("Robot {}: Rack {} has tasks but no station assignments",
              robot.getId(), selectedRack.getId());
          hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
          continue;
        }

        // Optimize station visit order
        List<StationId> stationsToVisit = optimizeStationRoute(
            robot.getPosition(),
            new ArrayList<>(stationTasks.keySet())
        );

        // Reserve rack
        selectedRack.setBeingTransported(robot.getId());
        robot.setCarryingRack(selectedRack);

        double cycleStartTime = now();

        log.info("Robot {} starting cycle with rack {} at t={}",
            robot.getId(), selectedRack.getId(), presentTime());

        // 4. Visit each station
        for (StationId stationId : stationsToVisit) {
          Station targetStation = stationManager.getStation(stationId);
          if (targetStation == null) {
            log.error("Station {} not found, skipping", stationId);
            continue;
          }

          Res operatorResource = operatorResources.get(stationId);

          // 4a. Transport rack to station
          transportRackToStation(selectedRack, targetStation.getPosition().position());

          // 4b. Acquire operator resource (BLOCKS if busy)
          log.info("Robot {} requesting operator at station {} at t={}",
              robot.getId(), stationId, presentTime());

          operatorResource.provide(1);  // CRITICAL: Desmo-J blocking call

          log.info("Robot {} acquired operator at station {} at t={}",
              robot.getId(), stationId, presentTime());

          // 4c. Process tasks for this station
          List<PickTask> tasksForStation = stationTasks.get(stationId);
          processTasksAtStation(selectedRack, targetStation, tasksForStation);

          // 4d. Release operator resource
          log.info("Robot {} releasing operator at station {} at t={}",
              robot.getId(), stationId, presentTime());

          operatorResource.takeBack(1);
        }

        // 5. Return rack to storage
        returnRackToStorage(selectedRack);

        double cycleEndTime = now();

        log.info("Robot {} completed cycle with rack {} at t={} (duration: {}s)",
            robot.getId(), selectedRack.getId(), presentTime(),
            cycleEndTime - cycleStartTime);

        // Emit cycle completion event
        emitCycleEvent(robot.getId(), selectedRack.getId().value(),
            cycleStartTime, cycleEndTime, stationsToVisit);
      }

    } catch (Exception e) {
      log.error("Robot {} encountered error: {}", robot.getId(), e.getMessage(), e);
      robot.setState(RobotState.ERROR);
    }
  }

  /**
   * Transport rack to station.
   */
  private void transportRackToStation(Rack rack, GridCoordinate stationPosition) {
    // Phase 1: Move to rack
    robot.setState(RobotState.MOVING_TO_RACK);
    double travelTimeToRack = robot.moveToPosition(rack.getCurrentCoordinate());
    hold(new TimeSpan(travelTimeToRack, TimeUnit.SECONDS));

    // Phase 2: Load rack
    robot.setState(RobotState.LOADING_RACK);
    hold(new TimeSpan(LOADING_TIME, TimeUnit.SECONDS));

    // Phase 3: Transport to station
    robot.setState(RobotState.TRANSPORTING_RACK_TO_STATION);
    double travelTimeToStation = robot.moveToPosition(stationPosition);
    rack.moveToStation(stationPosition);
    hold(new TimeSpan(travelTimeToStation, TimeUnit.SECONDS));
  }

  /**
   * Process tasks at station.
   */
  private void processTasksAtStation(
      Rack rack,
      Station station,
      List<PickTask> tasks) {

    double operatorTimePerPick = ((DesmojWarehouseModel) getModel())
        .getConfig()
        .operatorConfig()
        .operatorTimePerPick();

    for (PickTask task : tasks) {
      // Simulate picking
      hold(new TimeSpan(operatorTimePerPick, TimeUnit.SECONDS));

      // Remove task from rack
      rack.removeTask(task);

      // Update inventory
      inventory.decrementQuantity(task.getStorageSpaceId(), task.getSku(), task.getQuantity());

      // Emit task completion event
      Map<String, Object> eventData = new HashMap<>();
      eventData.put("task_id", task.getTaskId());
      eventData.put("order_id", task.getOrderId());
      eventData.put("robot_id", robot.getId());
      eventData.put("station_id", station.getStationId().value());

      metricsManager.emitEvent(
          MetricEvent.TASK_COMPLETED,
          now(),
          null,
          null,
          eventData
      );

      log.debug("Robot {} completed task {} at station {} at t={}",
          robot.getId(), task.getTaskId(), station.getStationId(), presentTime());
    }
  }

  /**
   * Return rack to storage.
   */
  private void returnRackToStorage(Rack rack) {
    robot.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);

    GridCoordinate storagePosition = rack.getStorageCoordinate();

    // Transport to storage
    double travelTime = robot.moveToPosition(storagePosition);
    rack.moveToStorage(storagePosition);
    hold(new TimeSpan(travelTime, TimeUnit.SECONDS));

    // Unload rack
    hold(new TimeSpan(UNLOADING_TIME, TimeUnit.SECONDS));

    // Complete return
    robot.setCarryingRack(null);
    robot.setCurrentTask(null);
    robot.setState(RobotState.IDLE);
  }

  /**
   * Select optimal rack (closest to robot).
   */
  private Rack selectOptimalRack(Robot robot, List<Rack> availableRacks) {
    if (availableRacks.isEmpty()) {
      return null;
    }

    Rack bestRack = null;
    double bestDistance = Double.MAX_VALUE;

    for (Rack rack : availableRacks) {
      double travelTime = robot.calculateTravelTime(rack.getCurrentCoordinate());
      if (travelTime < bestDistance) {
        bestDistance = travelTime;
        bestRack = rack;
      }
    }

    return bestRack;
  }

  /**
   * Optimize station visit order (nearest-neighbor heuristic).
   */
  private List<StationId> optimizeStationRoute(
      GridCoordinate startPosition,
      List<StationId> stationIds) {

    if (stationIds.size() <= 1) {
      return stationIds;
    }

    List<StationId> unvisited = new ArrayList<>(stationIds);
    List<StationId> route = new ArrayList<>();
    GridCoordinate currentPos = startPosition;

    while (!unvisited.isEmpty()) {
      StationId nearestStationId = null;
      double nearestDistance = Double.MAX_VALUE;

      for (StationId sid : unvisited) {
        Station station = stationManager.getStation(sid);
        if (station == null) continue;

        double distance = station.getPosition().position().manhattanDistance(currentPos);
        if (distance < nearestDistance) {
          nearestDistance = distance;
          nearestStationId = sid;
        }
      }

      if (nearestStationId == null) break;

      route.add(nearestStationId);
      unvisited.remove(nearestStationId);

      Station nearestStation = stationManager.getStation(nearestStationId);
      if (nearestStation != null) {
        currentPos = nearestStation.getPosition().position();
      }
    }

    return route;
  }

  /**
   * Get current simulation time in seconds.
   */
  private double now() {
    return presentTime().getTimeAsDouble(TimeUnit.SECONDS);
  }

  /**
   * Emit simple event.
   */
  private void emitEvent(MetricEvent eventType, Object... keyValuePairs) {
    if (metricsManager != null) {
      Map<String, Object> data = new HashMap<>();
      for (int i = 0; i < keyValuePairs.length; i += 2) {
        data.put((String) keyValuePairs[i], keyValuePairs[i + 1]);
      }
      metricsManager.emitEvent(eventType, now(), null, null, data);
    }
  }

  /**
   * Emit cycle completion event.
   */
  private void emitCycleEvent(
      int robotId,
      int rackId,
      double cycleStartTime,
      double cycleEndTime,
      List<StationId> stationsVisited) {

    if (metricsManager != null) {
      Map<String, Object> data = new HashMap<>();
      data.put("robot_id", robotId);
      data.put("rack_id", rackId);
      data.put("cycle_start_time", cycleStartTime);
      data.put("cycle_end_time", cycleEndTime);
      data.put("cycle_time", cycleEndTime - cycleStartTime);
      data.put("stations_visited", stationsVisited);
      data.put("is_multi_station", stationsVisited.size() > 1);

      metricsManager.emitEvent(MetricEvent.RACK_CYCLE_COMPLETED, now(), null, null, data);
    }
  }
}
```

**Key Porting Changes**:

| Aspect | Custom DES | Desmo-J |
|--------|------------|---------|
| Main method | `execute(SimulationEngine engine)` | `lifeCycle()` |
| Loop condition | `while (engine.isRunning())` | `while (true)` (runs until simulation stops) |
| Delay | `engine.timeout(duration)` | `hold(new TimeSpan(duration, TimeUnit.SECONDS))` |
| Current time | `engine.now()` | `presentTime().getTimeAsDouble(TimeUnit.SECONDS)` |
| Resource acquire | `resource.acquire(engine)` | `res.provide(1)` (blocks automatically) |
| Resource release | `resource.release()` | `res.takeBack(1)` |
| Interruption | `InterruptedException` handling | No manual handling (Desmo-J manages) |

**Verification**:
```bash
mvn clean compile
```

---

### Phase 3: Integration with Simulator

#### Task 3.1: Update SimulationEngine to use DesmojSimulationEngine

**File**: `simulator/SimulationEngine.java`

**Changes**: Replace custom DES engine with Desmo-J

**Before** (lines 84-86):
```java
// Initialize DES engine and process scheduler
this.desEngine = new com.greyorange.dessimulator.infrastructure.des.core.SimulationEngine();
this.processScheduler = new ProcessScheduler(desEngine);
```

**After**:
```java
// Initialize Desmo-J engine
this.desmojEngine = new DesmojSimulationEngine(config, warehouseComponents, metricsManager);
```

**Full Updated run() Method**:

```java
/** Run the simulation. */
public void run() {
  log.info("Starting Desmo-J simulation engine...");

  // Delegate to DesmojSimulationEngine
  desmojEngine.run();

  log.info("Simulation completed");
}
```

**SIMPLIFICATION**: Most of `SimulationEngine` logic moves to `DesmojSimulationEngine`

**Alternatively** (Complete Replacement):

Replace entire `SimulationEngine.java` with a thin wrapper:

```java
package com.greyorange.dessimulator.simulator;

import com.greyorange.dessimulator.infrastructure.desmoj.DesmojSimulationEngine;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import com.greyorange.dessimulator.simulator.models.WarehouseComponents;
import lombok.extern.slf4j.Slf4j;

/**
 * Simulation Engine (Desmo-J wrapper).
 *
 * This is now a thin wrapper around DesmojSimulationEngine.
 */
@Slf4j
public class SimulationEngine {
  private final DesmojSimulationEngine desmojEngine;

  public SimulationEngine(
      SimulationConfig config,
      WarehouseComponents warehouseComponents,
      MetricsManager metricsManager) {

    log.info("Initializing SimulationEngine (Desmo-J backend)");

    this.desmojEngine = new DesmojSimulationEngine(
        config,
        warehouseComponents,
        metricsManager
    );
  }

  /** Run the simulation. */
  public void run() {
    desmojEngine.run();
  }

  /** Get current simulation time. */
  public double now() {
    return desmojEngine.now();
  }
}
```

**Decision**: Choose between:
- **Option A**: Minimal changes (replace des engine only)
- **Option B**: Complete replacement (thin wrapper) ← **RECOMMENDED**

**Rationale for Option B**:
- Cleaner separation of concerns
- All DES logic in `infrastructure/desmoj/`
- `simulator/` layer stays DES-agnostic
- Easier to maintain

**Verification**:
```bash
mvn clean compile
```

---

#### Task 3.2: Update Simulator.java (no changes needed)

**File**: `simulator/Simulator.java`

**Analysis**: Lines 122-136 create `SimulationEngine` and call `run()`:

```java
private void runSimulationEngine() {
  log.info("Starting simulation engine...");

  if (metricsManager == null) {
    throw new RuntimeException("Metrics manager is not initialized");
  }

  if (warehouseComponents == null) {
    throw new RuntimeException("Warehouse components are not initialized");
  }

  simulationEngine = new SimulationEngine(config, warehouseComponents, metricsManager);
  simulationEngine.run();

  log.info("Simulation engine completed");
}
```

**No changes needed** - `SimulationEngine` constructor signature unchanged!

**Verification**: Check that `Simulator.runSimulation()` still works.

---

### Phase 4: Testing

#### Task 4.1: Create Integration Test

**File**: `src/test/java/com/greyorange/dessimulator/simulator/DesmojIntegrationTest.java`

**Purpose**: End-to-end test with 4 robots, simple warehouse, validates:
1. Simulation completes without exceptions
2. Metrics are non-zero (robots actually worked)

**Full Implementation**:

```java
package com.greyorange.dessimulator.simulator;

import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.map.WarehouseMapConfig;
import com.greyorange.dessimulator.simulator.models.SimulationMetrics;
import com.greyorange.dessimulator.simulator.models.SimulationResults;
import org.junit.jupiter.api.Test;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Integration test for Desmo-J migration.
 *
 * Tests end-to-end simulation with:
 * - 4 RTP robots
 * - Simple warehouse layout
 * - Real order processing
 *
 * Success criteria:
 * - Simulation completes without exceptions
 * - Metrics are non-zero (robots did work)
 */
class DesmojIntegrationTest {
  private static final Logger log = LoggerFactory.getLogger(DesmojIntegrationTest.class);

  @Test
  void testFourRobotsRtpSystem_CompletesSuccessfully() {
    log.info("=".repeat(70));
    log.info("Starting Desmo-J Integration Test - 4 Robots RTP System");
    log.info("=".repeat(70));

    // Create simulation configuration
    SimulationConfig config = createTestConfig();

    // Run simulation
    log.info("Creating simulator...");
    Simulator simulator = new Simulator(config);

    log.info("Running simulation...");
    long startTime = System.currentTimeMillis();

    SimulationResults results = simulator.runSimulation();

    long endTime = System.currentTimeMillis();
    double executionTime = (endTime - startTime) / 1000.0;

    log.info("=".repeat(70));
    log.info("Simulation completed in {:.2f}s real time", executionTime);
    log.info("=".repeat(70));

    // Validate results
    assertNotNull(results, "Results should not be null");
    assertTrue(results.success(), "Simulation should succeed");
    assertNull(results.error(), "No error should occur");

    // Validate metrics are non-zero
    SimulationMetrics metrics = results.metrics();
    assertNotNull(metrics, "Metrics should not be null");

    // Check robot metrics
    assertNotNull(metrics.robotMetrics(), "Robot metrics should not be null");

    // At least one robot should have completed cycles
    assertTrue(
        metrics.robotMetrics().totalCycles() > 0,
        "Expected at least one robot cycle to complete. Got: "
            + metrics.robotMetrics().totalCycles()
    );

    // At least some tasks should be completed
    if (metrics.taskMetrics() != null) {
      assertTrue(
          metrics.taskMetrics().tasksCompleted() > 0,
          "Expected at least one task to complete. Got: "
              + metrics.taskMetrics().tasksCompleted()
      );
    }

    log.info("=".repeat(70));
    log.info("✓ SUCCESS: Simulation completed with non-zero metrics!");
    log.info("  Total cycles: {}", metrics.robotMetrics().totalCycles());
    if (metrics.taskMetrics() != null) {
      log.info("  Tasks completed: {}", metrics.taskMetrics().tasksCompleted());
    }
    log.info("=".repeat(70));
  }

  /**
   * Create test configuration with 4 robots.
   */
  private SimulationConfig createTestConfig() {
    // Small warehouse for faster test
    WarehouseMapConfig mapConfig = WarehouseMapConfig.builder()
        .gridWidth(20)
        .gridDepth(20)
        .cellSizeMeters(0.4)
        .numStations(2)
        .rackBlocksPerSide(2)
        .aislesPerRackBlock(2)
        .racksPerAisle(5)
        .build();

    // 4 robots (same as PoC)
    RtpRobotConfig robotConfig = RtpRobotConfig.builder()
        .configType("rtp")
        .numRobots(4)
        .robotSpeed(1.5)
        .build();

    // Standard operator config
    OperatorConfig operatorConfig = OperatorConfig.builder()
        .operatorTimePerPick(12.0)
        .targetPicksPerFace(3.0)
        .numBins(8)
        .build();

    // Standard algorithm config
    AlgorithmConfig algorithmConfig = AlgorithmConfig.builder()
        .putAlgorithm("greedy")
        .build();

    return SimulationConfig.builder()
        .simulationDuration(1000.0)  // 1000s (enough for multiple cycles)
        .scenarioId("scenario_a")
        .systemType(SystemType.RTP)
        .mapConfig(mapConfig)
        .robotConfig(robotConfig)
        .operatorConfig(operatorConfig)
        .algorithmConfig(algorithmConfig)
        .build();
  }
}
```

**Run Test**:
```bash
mvn test -Dtest=DesmojIntegrationTest
```

**Expected Output**:
```
[INFO] ======================================================================
[INFO] Starting Desmo-J Integration Test - 4 Robots RTP System
[INFO] ======================================================================
[INFO] Creating simulator...
[INFO] Running simulation...
... (simulation logs) ...
[INFO] ======================================================================
[INFO] Simulation completed in 2.34s real time
[INFO] ======================================================================
[INFO] ======================================================================
[INFO] ✓ SUCCESS: Simulation completed with non-zero metrics!
[INFO]   Total cycles: 12
[INFO]   Tasks completed: 36
[INFO] ======================================================================
[INFO] Tests run: 1, Failures: 0, Errors: 0, Skipped: 0
[INFO] BUILD SUCCESS
```

**Success Criteria**:
- ✅ Test passes (no exceptions)
- ✅ `totalCycles > 0` (robots completed work)
- ✅ `tasksCompleted > 0` (picks were made)
- ✅ No race conditions (simulation runs to completion)

---

#### Task 4.2: Update Existing Tests (if needed)

**Action**: Run all existing tests to ensure no regressions:

```bash
mvn test
```

**Expected**: All tests pass (Desmo-J is drop-in replacement)

**If tests fail**:
1. Check if tests depend on custom DES internals
2. Update tests to use Desmo-J equivalents
3. Most tests should be unaffected (they test domain logic, not DES)

---

### Phase 5: Cleanup and Documentation

#### Task 5.1: Update Documentation

**File**: `docs/desmoj-migration-complete.md`

**Content**:
```markdown
# Desmo-J Migration Complete

**Date**: 2025-11-08
**Status**: Production Ready

## What Changed

- **Before**: Custom DES (`infrastructure/des/`) with race conditions
- **After**: Desmo-J (`infrastructure/desmoj/`) - stable, proven framework

## Migration Summary

1. Created `infrastructure/desmoj/` package (parallel to `des/`)
2. Implemented `DesmojSimulationEngine` (replaces `des/core/SimulationEngine`)
3. Created `DesmojWarehouseModel` (Desmo-J Model)
4. Ported `RtpRobotOrchestrator` → `DesmojRtpRobotProcess` (Desmo-J SimProcess)
5. Updated `simulator/SimulationEngine.java` to use Desmo-J
6. All tests pass, metrics non-zero, no race conditions

## Results

- ✅ Simulation completes without premature termination
- ✅ All robot cycles complete (12/12 in test scenario)
- ✅ Metrics accurate and non-zero
- ✅ API compatibility maintained
- ✅ No changes to frontend

## Custom DES Status

- `infrastructure/des/` preserved (untouched)
- Can be removed in future cleanup (not urgent)
- Kept for reference during stabilization period

## Next Steps

1. Monitor production simulations for stability
2. Run performance benchmarks (compare Desmo-J vs custom DES speed)
3. Phase 2: Implement relay system with Desmo-J
4. Future cleanup: Remove `infrastructure/des/` (optional)
```

---

#### Task 5.2: Update CLAUDE.md

**File**: `CLAUDE.md`

**Add Section**:

```markdown
## Discrete Event Simulation (DES)

**Current Implementation**: Desmo-J (v2.5.1e)

The project uses **Desmo-J**, a mature Java-based discrete event simulation framework, to model warehouse operations.

**Package**: `infrastructure/desmoj/`

**Key Components**:
- `DesmojSimulationEngine`: Main simulation coordinator
- `DesmojWarehouseModel`: Desmo-J model with resources (operator capacity)
- `DesmojRtpRobotProcess`: Robot lifecycle process (extends SimProcess)

**Why Desmo-J**:
- Eliminates race conditions from custom DES
- Process-oriented mental model (similar to Python's SimPy)
- Built-in resource management (automatic blocking/queuing)
- Proven stability in academic and industrial use

**Previous Implementation**: Custom DES (`infrastructure/des/`) - **deprecated**
- Had critical race condition (Issue #2 in `docs/des-architecture-issues.md`)
- Replaced by Desmo-J in November 2025
- Kept in codebase for reference (can be removed in future cleanup)
```

---

## Testing Strategy

### Unit Tests

**Scope**: Individual components in isolation

**Test Files**:

1. **DesmojWarehouseModelTest.java**
   - Test `init()` creates correct number of operator resources
   - Test `doInitialSchedules()` schedules all robots
   - Test description() returns correct string

2. **DesmojRtpRobotProcessTest.java**
   - Mock `Model`, `Res`, `Robot`, etc.
   - Test `selectOptimalRack()` chooses closest rack
   - Test `optimizeStationRoute()` nearest-neighbor heuristic
   - Test resource acquisition/release

**Commands**:
```bash
mvn test -Dtest=DesmojWarehouseModelTest
mvn test -Dtest=DesmojRtpRobotProcessTest
```

---

### Integration Tests

**Scope**: End-to-end simulation flow

**Test Files**:

1. **DesmojIntegrationTest.java** (Task 4.1)
   - 4 robots, simple warehouse, real orders
   - Validates: simulation completes, metrics non-zero

2. **DesmojRaceConditionTest.java**
   - Reproduce exact PoC scenario (4 robots, 1 operator)
   - Validates: all 12 cycles complete (vs 0 in custom DES)
   - Regression test for race condition fix

**Commands**:
```bash
mvn test -Dtest=DesmojIntegrationTest
mvn test -Dtest=DesmojRaceConditionTest
```

---

### Regression Tests

**Scope**: Ensure Desmo-J doesn't break existing functionality

**Test Suites**:
- All existing `simulator/*Test.java` tests
- Domain model tests (`domain/*Test.java`)
- API tests (`api/*Test.java`)

**Commands**:
```bash
mvn test  # Run all tests
```

**Expected**: All tests pass (Desmo-J is transparent to domain logic)

---

### Performance Tests (Future)

**Scope**: Compare Desmo-J vs custom DES performance

**Metrics**:
- Real-time execution duration
- Memory usage
- Throughput (orders/second)

**Approach**:
1. Run same scenario with both engines
2. Measure wall-clock time
3. Profile memory allocation
4. Compare results

**Not blocking migration** - correctness > speed

---

## Success Criteria

### Functional Criteria

- ✅ **Simulation Completes**: No premature termination
- ✅ **No Race Conditions**: All robot cycles complete (not zero)
- ✅ **Metrics Accurate**: Non-zero values reflecting actual work
- ✅ **API Compatibility**: `SimulationResults` format unchanged
- ✅ **Domain Models Unchanged**: Robot, Rack, Station, etc. untouched

### Technical Criteria

- ✅ **Code Compiles**: `mvn clean compile` succeeds
- ✅ **Tests Pass**: `mvn test` all green
- ✅ **Integration Test Passes**: `DesmojIntegrationTest` succeeds
- ✅ **No Custom DES Dependencies**: `infrastructure/desmoj/` standalone

### Acceptance Criteria

- ✅ **User Story 1**: Simulation completes all robot work cycles without premature termination
- ✅ **User Story 2**: Metrics accurately reflect simulation results (not zeros)
- ✅ **User Story 3**: Frontend integration unaffected (API responses identical)

### Validation

**Run End-to-End Test**:
```bash
# Compile
mvn clean compile

# Run integration test
mvn test -Dtest=DesmojIntegrationTest

# Expected: BUILD SUCCESS, totalCycles > 0
```

**API Test** (optional):
```bash
# Start server
nx serve backend-java

# Send request
curl -X POST http://localhost:8080/api/simulations/run \
  -H "Content-Type: application/json" \
  -d '{
    "simulationDuration": 1000.0,
    "scenarioId": "scenario_a",
    "systemType": "RTP",
    "mapConfig": {...},
    "robotConfig": {"configType": "rtp", "numRobots": 4, "robotSpeed": 1.5},
    "operatorConfig": {...},
    "algorithmConfig": {...}
  }'

# Expected: 200 OK, success=true, metrics.robotMetrics.totalCycles > 0
```

---

## Rollback Plan

**If migration fails**:

1. **Revert SimulationEngine.java**:
   ```bash
   git checkout HEAD -- apps/backend-java/src/main/java/com/greyorange/dessimulator/simulator/SimulationEngine.java
   ```

2. **Remove desmoj/ package** (optional):
   ```bash
   git rm -rf apps/backend-java/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/
   ```

3. **Rebuild**:
   ```bash
   mvn clean compile
   ```

**Custom DES preserved** - no data loss, instant rollback possible

---

## Dependencies

### Maven Dependency (Already Added)

```xml
<dependency>
    <groupId>desmoj</groupId>
    <artifactId>desmoj</artifactId>
    <version>2.5.1e</version>
    <scope>system</scope>
    <systemPath>${project.basedir}/libs/desmoj-2.5.1e-bin.jar</systemPath>
</dependency>
```

**Location**: `apps/backend-java/libs/desmoj-2.5.1e-bin.jar` (7.5 MB)

**No external dependencies** - Desmo-J is self-contained

---

## File Checklist

### Files to Create

- ✅ `infrastructure/desmoj/DesmojSimulationEngine.java` (Task 2.1)
- ✅ `infrastructure/desmoj/model/DesmojWarehouseModel.java` (Task 2.2)
- ✅ `infrastructure/desmoj/process/DesmojRtpRobotProcess.java` (Task 2.3)
- ✅ `src/test/java/.../DesmojIntegrationTest.java` (Task 4.1)
- ✅ `docs/desmoj-migration-complete.md` (Task 5.1)

### Files to Modify

- ✅ `simulator/SimulationEngine.java` (Task 3.1) - Wrapper for DesmojSimulationEngine
- ✅ `CLAUDE.md` (Task 5.2) - Document Desmo-J usage

### Files to Rename

- ✅ `infrastructure/des1/` → `infrastructure/desmoj/` (Task 1.1)

### Files to Preserve (No Changes)

- ✅ `infrastructure/des/` - Custom DES (untouched)
- ✅ `simulator/Simulator.java` - No changes needed
- ✅ `domain/**` - All domain models unchanged
- ✅ `api/**` - All API contracts unchanged

---

## Timeline Estimate

**Total: ~6-8 hours** (for engineer with zero Desmo-J experience)

| Phase | Tasks | Estimated Time |
|-------|-------|----------------|
| 1. Package Refactoring | Rename des1/ → desmoj/ | 30 min |
| 2. Core Infrastructure | Create 3 main classes | 3-4 hours |
| 3. Integration | Update SimulationEngine | 1 hour |
| 4. Testing | Write integration test | 1-2 hours |
| 5. Documentation | Update docs | 30 min |
| **Verification** | Run tests, validate | 30 min |

**Fast-track**: If following this plan exactly, implementation could take 4-5 hours

---

## Risk Assessment

### Low Risk

- ✅ **Parallel packages**: Custom DES untouched, easy rollback
- ✅ **PoC validated**: Desmo-J proven to work (12/12 cycles)
- ✅ **API unchanged**: Frontend integration unaffected
- ✅ **Domain models unchanged**: No changes to business logic

### Medium Risk

- ⚠️ **Performance**: Desmo-J may be slower than custom DES (unknown)
  - **Mitigation**: Benchmark after migration, optimize if needed
  - **Correctness > Speed**: Working simulation more valuable than fast broken one

- ⚠️ **Integration complexity**: Order loading, task assignment, metrics collection
  - **Mitigation**: Copy logic from original SimulationEngine (lines 183-254)
  - **Well-defined interfaces**: MetricsManager, WarehouseComponents unchanged

### High Risk (Unlikely)

- ❌ **Desmo-J bugs**: Framework-level issues
  - **Mitigation**: Desmo-J used since 2004, stable in production
  - **PoC validated**: No critical bugs found

---

## Appendix A: Desmo-J API Quick Reference

### Process-Oriented Pattern

```java
public class MyProcess extends SimProcess {
  @Override
  public void lifeCycle() {
    // Process logic here
  }
}
```

### Time Delays

```java
hold(new TimeSpan(duration, TimeUnit.SECONDS));
```

### Resource Blocking

```java
// Acquire (blocks if unavailable)
resource.provide(1);

// Use resource
hold(new TimeSpan(workTime, TimeUnit.SECONDS));

// Release
resource.takeBack(1);
```

### Current Time

```java
double now = presentTime().getTimeAsDouble(TimeUnit.SECONDS);
```

### Scheduling

```java
process.activate(new TimeInstant(startTime, TimeUnit.SECONDS));
```

---

## Appendix B: Comparison - Custom DES vs Desmo-J

| Aspect | Custom DES | Desmo-J |
|--------|------------|---------|
| **Architecture** | Event loop + Virtual threads | Process-oriented framework |
| **Event Scheduling** | Manual `EventQueue.add()` | Automatic (framework-managed) |
| **Time Management** | `SimulationClock` | `TimeInstant`, `TimeSpan` |
| **Resource Blocking** | Manual `Resource.acquire()` + `CountDownLatch` | `Res.provide()` (automatic blocking) |
| **Process Lifecycle** | `SimulationProcess.execute()` | `SimProcess.lifeCycle()` |
| **Delays** | `engine.timeout()` | `hold(new TimeSpan())` |
| **Race Conditions** | Yes (Issue #2) | No (framework guarantees) |
| **Maturity** | Custom (new) | 20+ years in production |
| **Documentation** | Internal docs | Extensive tutorials + API docs |
| **Testing** | Custom test harness | Built-in reporting, traces |

---

## Appendix C: References

- **Desmo-J Homepage**: https://desmoj.sourceforge.net/
- **Tutorial**: https://desmoj.sourceforge.net/tutorial/overview/0.html
- **API Docs**: https://desmoj.sourceforge.net/doc/index.html
- **SimProcess API**: https://desmoj.sourceforge.net/doc/desmoj/core/simulator/SimProcess.html
- **Res (Resource) API**: https://desmoj.sourceforge.net/tutorial/advanced/res0.html
- **PoC Progress Report**: `docs/desmoj-poc-progress.md`
- **DES Architecture Issues**: `docs/des-architecture-issues.md`

---

**End of Plan**
