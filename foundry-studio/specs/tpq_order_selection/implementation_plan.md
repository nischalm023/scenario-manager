# TPQ Order Selection - Implementation Plan

**Date:** 2025-11-17
**Related:** [Design Document](./design.md)

---

## Overview

This document outlines the step-by-step implementation plan for adding TPQ (Task Pool Quality) based order selection to the warehouse simulator. The implementation refactors the existing architecture to support multiple order selection strategies and adds comprehensive TPQ metrics collection.

---

## Current State Analysis

### 1. OrderProcessor Architecture (PROBLEM)

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java`

**Current Issues:**
- ❌ Two algorithms hardcoded directly in OrderProcessor class
- ❌ `selectNextOrders()` uses switch statement to dispatch (lines 75-81)
- ❌ No abstraction/interface for order selection strategies
- ❌ Difficult to add new algorithms without modifying existing code
- ❌ All selection logic tightly coupled to single class

**Current Structure:**
```java
public class OrderProcessor {
  public List<OrderSelection> selectNextOrders(...) {
    List<OrderSelection> selectedOrders = switch (orderSelectionAlgorithm) {
      case MAXIMIZE_PPF_WITH_TIERING -> selectOrdersPicksPerFaceWithTiering(...);
      case MAXIMIZE_PPF_DIRECTLY -> selectOrdersMaximizePpfDirectly(...);
    };
    // ... validation
  }

  private List<OrderSelection> selectOrdersPicksPerFaceWithTiering(...) { }
  private List<OrderSelection> selectOrdersMaximizePpfDirectly(...) { }
}
```

### 2. Configuration Flow (BROKEN)

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java`

**Problem at line 177:**
```java
AlgorithmConfig algoConfig = AlgorithmConfig.builder().build();
```

**Issue:** DTO has `orderSelectionAlgorithm` and `enableTpqOptimization` fields (lines 121, 125) but they're **NOT passed** to AlgorithmConfig! Configuration is completely ignored.

**DTO Definition (lines 106-145):**
```java
record SimulationConfigDTO(
  ...
  @ValidOrderSelectionAlgorithm String orderSelectionAlgorithm,  // ← NOT USED!
  String putAlgorithm,
  ...
  Boolean enableTpqOptimization,  // ← NOT USED!
  ...
) {
  SimulationConfigDTO {
    if (orderSelectionAlgorithm == null) orderSelectionAlgorithm = "maximize_ppf_directly";
    if (enableTpqOptimization == null) enableTpqOptimization = false;
  }

  SimulationConfig toCoreConfig() {
    // ... creates configs
    AlgorithmConfig algoConfig = AlgorithmConfig.builder().build();  // ← WRONG!
    // Should be:
    // AlgorithmConfig algoConfig = AlgorithmConfig.builder()
    //     .orderSelectionAlgorithm(OrderSelectionAlgorithm.fromValue(orderSelectionAlgorithm))
    //     .putAlgorithm(putAlgorithm)
    //     .build();
  }
}
```

**Validator (lines 270-280):**
```java
private static final Set<String> VALID_ALGORITHMS =
    Set.of("maximize_ppf_directly", "maximize_ppf_with_tiering");  // ← Needs TPQ added
```

### 3. Metrics Collection (INCOMPLETE)

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java`

**Lines 862-884: TaskPoolQualityCollector exists but is a stub:**
```java
public static class TaskPoolQualityCollector extends BaseCollector {
  private double totalTaskPoolQuality = 0.0;
  private int tpqEventCount = 0;
  private double totalSystemTime = 0.0;
  private double totalOperatorTime = 0.0;

  // ❌ NO @MetricHandler methods!
  // ❌ No event subscription
  // ❌ No timeline tracking

  public TaskPoolQualityMetric getMetrics() {
    // Returns static data, no timeline
  }
}
```

**Problem:** No events are emitted or captured for TPQ. The collector exists but does nothing.

### 4. Frontend Support (EXISTS BUT UNUSED)

**File:** `apps/frontend/src/pages/simulator/components/PickResults.tsx`

**Lines 136-174: TPQ Timeline Chart Already Implemented!**
```tsx
<SectionHeader
  title="Task Pool Quality Timeline"
  subtexts={['TPQ = 1.0 is optimal', 'Lower values indicate better efficiency']}
/>

{processedTPQData.length > 0 ? (
  <TimelineChart
    data={processedTPQData}
    metrics={[
      { dataKey: 'task_pool_quality', name: 'Window TPQ', ... },
      { dataKey: 'cumulative_task_pool_quality', name: 'Overall TPQ', ... }
    ]}
  />
) : (
  <div>No TPQ Timeline Data</div>  // ← Always shows this because no data!
)}
```

**Line 16:**
```tsx
const tpqTimelineRaw = metrics.taskPoolQualityMetrics.taskPoolQualityData || {}
```

**Expected Data Structure:**
```typescript
taskPoolQualityData: Array<{
  timeMinutes: number,
  task_pool_quality: number,  // Window average
  cumulative_task_pool_quality: number,  // Overall average
  // ... other fields
}>
```

**Problem:** Frontend expects timeline array but backend returns empty object because no events are collected.

### 5. Configuration Schema (INCOMPLETE)

**File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java`

**Lines 269-282: AlgorithmConfig (missing TPQ fields):**
```java
@Builder
public record AlgorithmConfig(
  String putAlgorithm,
  OrderSelectionAlgorithm orderSelectionAlgorithm,  // ← Exists but not wired
  int maxTotesPerOrder
  // ❌ Missing: TPQAlgorithmConfig tpqConfig
) {
  // Defaults...
}
```

**Needed but doesn't exist:**
```java
public record TPQAlgorithmConfig(
  Double targetTPQ,  // null = auto-calculate
  Double stationVarianceTolerance,  // Default: 1.0
  Boolean enableVariabilityPenalty,
  Double variabilityWeight,
  Boolean enableDiversityBonus,
  Double targetDistanceCV,
  Double diversityWeight,
  Boolean enableDepletionPenalty,
  Double depletionWeight
) { }
```

---

## Implementation Phases

### Phase 1: Architecture Refactoring (Strategy Pattern)

**Goal:** Extract order selection algorithms into separate strategy classes

#### 1.1 Create OrderSelectionStrategy Interface

**New File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/domain/order/OrderSelectionStrategy.java`

```java
package com.greyorange.dessimulator.domain.order;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.order.OrderProcessor.OrderSelection;
import java.util.List;
import java.util.Set;

/**
 * Strategy interface for order selection algorithms.
 *
 * Implementations provide different heuristics for selecting which orders
 * to assign to available bins.
 */
public interface OrderSelectionStrategy {

  /**
   * Select next batch of orders to process.
   *
   * @param pendingOrders Available orders to select from
   * @param activeStorageSpaces Storage spaces currently in use (for piggybacking)
   * @param maxOrders Maximum number of orders to select
   * @param currentTime Current simulation time
   * @return Selected orders with tasks and station assignments
   */
  List<OrderSelection> selectOrders(
      List<Order> pendingOrders,
      Set<StorageSpaceId> activeStorageSpaces,
      int maxOrders,
      double currentTime
  );

  /**
   * Get algorithm name for logging/metrics.
   */
  String getAlgorithmName();
}
```

#### 1.2 Extract PPF Strategies

**New File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfDirectlyStrategy.java`

```java
package com.greyorange.dessimulator.domain.order.strategies;

import com.greyorange.dessimulator.domain.order.OrderSelectionStrategy;
// ... imports

/**
 * Greedy PPF maximization strategy.
 *
 * Iteratively selects orders with highest marginal picks per face.
 */
@Slf4j
public class MaximizePpfDirectlyStrategy implements OrderSelectionStrategy {

  private final WarehouseInventory inventory;
  private final List<Station> stations;
  private final Map<StationId, BinManager> stationBinManagers;
  private final int maxTotesPerOrder;
  private final double operatorTimePerPick;

  // Constructor...

  @Override
  public List<OrderSelection> selectOrders(
      List<Order> pendingOrders,
      Set<StorageSpaceId> activeStorageSpaces,
      int maxOrders,
      double currentTime) {

    // Move implementation from OrderProcessor.selectOrdersMaximizePpfDirectly()
    // ... existing algorithm code
  }

  @Override
  public String getAlgorithmName() {
    return "MAXIMIZE_PPF_DIRECTLY";
  }
}
```

**New File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfWithTieringStrategy.java`

```java
package com.greyorange.dessimulator.domain.order.strategies;

// Similar structure, implements selectOrdersPicksPerFaceWithTiering logic
```

#### 1.3 Update OrderProcessor to Use Strategy

**Modified:** `OrderProcessor.java`

```java
public class OrderProcessor {

  private final OrderSelectionStrategy selectionStrategy;
  // Remove: orderSelectionAlgorithm enum field

  public OrderProcessor(
      WarehouseInventory inventory,
      List<Station> stations,
      Map<StationId, BinManager> stationBinManagers,
      OrderSelectionStrategy selectionStrategy,  // ← Strategy injected
      int maxTotesPerOrder) {
    this.inventory = inventory;
    this.stations = stations;
    this.stationBinManagers = stationBinManagers;
    this.selectionStrategy = selectionStrategy;
    this.maxTotesPerOrder = maxTotesPerOrder;
    this.orderTaskCache = null; // Disabled
  }

  public List<OrderSelection> selectNextOrders(
      List<Order> pendingOrders,
      Set<StorageSpaceId> activeStorageSpaces,
      int maxOrders,
      double currentTime) {

    log.debug("selectNextOrders() called with {} pending orders, maxOrders={}, strategy={}",
        pendingOrders.size(), maxOrders, selectionStrategy.getAlgorithmName());

    if (pendingOrders.isEmpty() || maxOrders <= 0) {
      return List.of();
    }

    // Delegate to strategy
    List<OrderSelection> selectedOrders = selectionStrategy.selectOrders(
        pendingOrders,
        activeStorageSpaces,
        maxOrders,
        currentTime
    );

    // Reservation validation phase (Option B) - stays in OrderProcessor
    List<OrderSelection> validatedOrders = new ArrayList<>();
    for (OrderSelection selection : selectedOrders) {
      if (inventory.tryReserveForOrder(selection.order().getOrderId(), selection.tasks())) {
        validatedOrders.add(selection);
      } else {
        log.debug("Failed to reserve inventory for order {} - skipping",
            selection.order().getOrderId());
      }
    }

    // Assign station IDs
    for (OrderSelection selection : validatedOrders) {
      for (PickTask task : selection.tasks()) {
        task.setStationId(selection.stationId());
      }
    }

    return validatedOrders;
  }

  // Keep helper methods that strategies can use:
  // - generateTasksWithSetCover() → make package-private or move to utility
  // - selectOptimalStationForOrder() → make package-private

  // Remove: selectOrdersPicksPerFaceWithTiering() - moved to strategy
  // Remove: selectOrdersMaximizePpfDirectly() - moved to strategy
}
```

#### 1.4 Strategy Instantiation (No Factory Needed)

**Note:** Strategies are instantiated directly where OrderProcessor is created (likely in simulation setup code). No factory class needed for MVP.

**Example instantiation pattern:**

```java
// In simulation setup (e.g., DesmojSimulationEngine or similar)
OrderSelectionStrategy strategy = switch (config.algorithmConfig().orderSelectionAlgorithm()) {
  case MAXIMIZE_PPF_DIRECTLY -> new MaximizePpfDirectlyStrategy(
      inventory, stations, stationBinManagers, maxTotesPerOrder, operatorTimePerPick);

  case MAXIMIZE_PPF_WITH_TIERING -> new MaximizePpfWithTieringStrategy(
      inventory, stations, stationBinManagers, maxTotesPerOrder, operatorTimePerPick);

  case MAXIMIZE_TPQ_WITH_VUT -> new MaximizeTPQStrategy(
      config.algorithmConfig().tpqConfig(),
      inventory,
      stations,
      stationBinManagers,
      maxTotesPerOrder,
      operatorTimePerPick,
      robotSpeed,
      numRobots,
      numOperators);
};

OrderProcessor orderProcessor = new OrderProcessor(
    inventory,
    stations,
    stationBinManagers,
    strategy,  // ← Inject strategy
    maxTotesPerOrder
);
```

---

### Phase 2: TPQ Configuration & Wiring

#### 2.1 Update OrderSelectionAlgorithm Enum

**Modified:** `OrderSelectionAlgorithm.java`

```java
public enum OrderSelectionAlgorithm {
  MAXIMIZE_PPF_DIRECTLY("maximize_ppf_directly"),
  MAXIMIZE_PPF_WITH_TIERING("maximize_ppf_with_tiering"),
  MAXIMIZE_TPQ_WITH_VUT("maximize_tpq_with_vut");  // ← NEW

  // ... rest same
}
```

#### 2.2 Add TPQAlgorithmConfig (SIMPLIFIED)

**Modified:** `SimulationConfigs.java`

```java
/**
 * Configuration for TPQ-based order selection algorithm.
 *
 * <p>Simplified configuration exposes only target TPQ.
 * Scoring weights and other parameters are constants in MaximizeTPQStrategy.
 */
@Builder
public record TPQAlgorithmConfig(
    Double targetTPQ  // null = auto-calculate from robot/operator ratio
) {
  public TPQAlgorithmConfig {
    // targetTPQ defaults to null (auto-calculated)
  }
}
```

**Note:** All other parameters (variability weight, diversity weight, depletion weight, etc.) are **hardcoded as constants** in `MaximizeTPQStrategy.java`. This keeps configuration simple for MVP.

#### 2.3 Update AlgorithmConfig

**Modified:** `SimulationConfigs.java` (AlgorithmConfig)

```java
@Builder
public record AlgorithmConfig(
    String putAlgorithm,
    OrderSelectionAlgorithm orderSelectionAlgorithm,
    int maxTotesPerOrder,
    TPQAlgorithmConfig tpqConfig  // ← NEW
) {
  public AlgorithmConfig {
    if (putAlgorithm == null) {
      putAlgorithm = "greedy";
    }
    if (orderSelectionAlgorithm == null) {
      orderSelectionAlgorithm = OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY;
    }
    if (maxTotesPerOrder == 0) {
      maxTotesPerOrder = 10;
    }
    if (tpqConfig == null) {
      tpqConfig = TPQAlgorithmConfig.builder().build();  // Default config
    }
  }
}
```

#### 2.4 Fix SimulationsController DTO Mapping

**Modified:** `SimulationsController.java`

**Add to SimulationConfigDTO (around line 125):**
```java
record SimulationConfigDTO(
    // ... existing fields
    @ValidOrderSelectionAlgorithm String orderSelectionAlgorithm,
    String putAlgorithm,
    // ... existing fields

    // NEW: TPQ configuration (simplified - only target)
    Double tpqTarget,  // null = auto-calculate

    // ... snapshots fields
) {
  SimulationConfigDTO {
    // ... existing defaults
    if (orderSelectionAlgorithm == null) orderSelectionAlgorithm = "maximize_ppf_directly";
    if (putAlgorithm == null) putAlgorithm = "greedy";

    // tpqTarget: Leave null to auto-calculate from robot/operator ratio
  }

  SimulationConfig toCoreConfig() {
    SystemType sysType = systemType.equalsIgnoreCase("rtp") ? SystemType.RTP : SystemType.RELAY;

    // ... robot config creation (existing)

    // ... operator config creation (existing)

    // FIX: Actually build AlgorithmConfig with DTO values!
    OrderSelectionAlgorithm algo = OrderSelectionAlgorithm.fromValue(orderSelectionAlgorithm);

    // Simplified TPQ config - only target exposed
    TPQAlgorithmConfig tpqConf = TPQAlgorithmConfig.builder()
        .targetTPQ(tpqTarget)  // null = auto-calculate
        .build();

    AlgorithmConfig algoConfig = AlgorithmConfig.builder()
        .orderSelectionAlgorithm(algo)
        .putAlgorithm(putAlgorithm)
        .maxTotesPerOrder(10)  // Could expose in DTO
        .tpqConfig(tpqConf)
        .build();

    return SimulationConfig.builder()
        .simulationDuration(duration * 3600.0)
        .scenarioId(scenario)
        .systemType(sysType)
        .mapConfig(mapConfig)
        .robotConfig(robotConfig)
        .operatorConfig(opConfig)
        .algorithmConfig(algoConfig)  // ← Now properly configured!
        .enableSnapshots(enableSnapshots)
        .snapshotIntervalSeconds(snapshotIntervalSeconds)
        .build();
  }
}
```

**Update Validator (line 273):**
```java
private static final Set<String> VALID_ALGORITHMS =
    Set.of("maximize_ppf_directly", "maximize_ppf_with_tiering", "maximize_tpq_with_vut");
```

---

### Phase 3: TPQ Metrics Collection

#### 3.1 Define Order Selection Event

**Modified:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEvent.java`

```java
public enum MetricEvent {
  // ... existing events

  // Order selection event (all strategies)
  ORDER_BATCH_SELECTED,  // Emitted after each order selection batch (all algorithms)
}
```

**Note:** Only one event type needed. All strategies can emit `ORDER_BATCH_SELECTED` with algorithm-specific data:
- **PPF strategies:** Can optionally include `picks_per_face`, `total_faces`, etc. (not required for MVP)
- **TPQ strategy:** Include `tpq`, `system_time`, `operator_time`, etc.

**Collectors filter by data presence:**
- `TaskPoolQualityCollector` checks for `tpq` field
- If not present → ignores event (PPF algorithm)
- If present → captures TPQ timeline data

**Benefits:**
- Single event type for all order selection algorithms
- Strategy-specific metrics in payload
- Collectors self-filter based on relevant data
- Extensible for future algorithms

**TPQ Timeline Calculation:**
Similar to picks per face, TPQ timeline uses:
- **Window TPQ:** 10-minute rolling window average (smooths out batch variations)
- **Cumulative TPQ:** Overall average from simulation start (shows long-term trend)

This matches the pattern in `PicksPerFaceTimelinePoint`:
```java
// Picks Per Face (existing)
avgPicksPerFace              // 10-min window
cumulativeAvgPicksPerFace    // Overall average

// TPQ (new, same pattern)
taskPoolQuality              // 10-min window
cumulativeTaskPoolQuality    // Overall average
```

#### 3.2 Update TaskPoolQualityCollector

**Modified:** `MetricsCollectors.java` (TaskPoolQualityCollector class)

```java
@Slf4j
public static class TaskPoolQualityCollector extends BaseCollector {

  private static final double TIME_WINDOW_SECONDS = 600.0; // 10-minute rolling window (same as picks per face)

  // Timeline tracking (similar to PicksPerFaceTimeline)
  private final List<TPQLogEntry> tpqLog = new ArrayList<>();
  private double simulationStartTime = 0.0;
  private double simulationEndTime = 0.0;

  @MetricHandler({MetricEvent.SIMULATION_STARTED})
  public void handleSimulationStarted(MetricEventData event) {
    recordEvent(event);
    simulationStartTime = event.getTimestamp();
  }

  @MetricHandler({MetricEvent.SIMULATION_COMPLETED})
  public void handleSimulationCompleted(MetricEventData event) {
    recordEvent(event);
    simulationEndTime = event.getTimestamp();
  }

  @MetricHandler({MetricEvent.ORDER_BATCH_SELECTED})
  public void handleOrderBatchSelected(MetricEventData event) {
    recordEvent(event);

    if (event.getData() == null) return;

    // Extract TPQ-specific data (only present when TPQ algorithm is used)
    Object tpqObj = event.getData().get("tpq");
    Object systemTimeObj = event.getData().get("system_time");
    Object operatorTimeObj = event.getData().get("operator_time");
    Object batchSizeObj = event.getData().get("batch_size");

    // If TPQ data not present, this is from a PPF algorithm - ignore
    if (!(tpqObj instanceof Number) || !(systemTimeObj instanceof Number)
        || !(operatorTimeObj instanceof Number)) {
      return;  // Not a TPQ event
    }

    // Process TPQ data
    double tpq = ((Number) tpqObj).doubleValue();
    double systemTime = ((Number) systemTimeObj).doubleValue();
    double operatorTime = ((Number) operatorTimeObj).doubleValue();
    int batchSize = batchSizeObj instanceof Number
        ? ((Number) batchSizeObj).intValue() : 0;

    // Create log entry (raw batch data)
    TPQLogEntry entry = TPQLogEntry.builder()
        .time(event.getTimestamp())
        .batchTPQ(tpq)  // Individual batch TPQ
        .systemTime(systemTime)
        .operatorTime(operatorTime)
        .batchSize(batchSize)
        .build();

    tpqLog.add(entry);

    log.debug("TPQ batch at t={:.1f}min: TPQ={:.2f}, Batch size={}",
        event.getTimestamp() / 60.0, tpq, batchSize);
  }

  /**
   * Compute TPQ timeline with sliding window average.
   *
   * <p>Similar to picks per face computation - uses 10-minute rolling window for
   * windowed average and maintains cumulative average across all batches.
   */
  private List<TPQTimelinePoint> computeTPQTimeline() {
    if (tpqLog.isEmpty()) {
      return List.of();
    }

    List<TPQTimelinePoint> timeline = new ArrayList<>();

    // Cumulative tracking (incremental)
    double cumulativeSystemTime = 0.0;
    double cumulativeOperatorTime = 0.0;

    // Sliding window tracking
    int windowStartIdx = 0;
    double windowSystemTime = 0.0;
    double windowOperatorTime = 0.0;

    // Compute timeline point for each batch
    for (int i = 0; i < tpqLog.size(); i++) {
      TPQLogEntry entry = tpqLog.get(i);
      double currentTime = entry.time();

      // Update cumulative metrics (O(1))
      cumulativeSystemTime += entry.systemTime();
      cumulativeOperatorTime += entry.operatorTime();
      double cumulativeTPQ = cumulativeOperatorTime > 0
          ? cumulativeSystemTime / cumulativeOperatorTime
          : 0.0;

      // Slide window start forward to maintain 10-minute window (O(1) amortized)
      while (windowStartIdx < i
          && tpqLog.get(windowStartIdx).time() < currentTime - TIME_WINDOW_SECONDS) {
        windowSystemTime -= tpqLog.get(windowStartIdx).systemTime();
        windowOperatorTime -= tpqLog.get(windowStartIdx).operatorTime();
        windowStartIdx++;
      }

      // Add current entry to window
      windowSystemTime += entry.systemTime();
      windowOperatorTime += entry.operatorTime();

      double windowTPQ = windowOperatorTime > 0
          ? windowSystemTime / windowOperatorTime
          : 0.0;

      TPQTimelinePoint point = TPQTimelinePoint.builder()
          .time(currentTime)
          .timeMinutes(currentTime / 60.0)
          .taskPoolQuality(windowTPQ)  // 10-minute window average
          .cumulativeTaskPoolQuality(cumulativeTPQ)  // Overall average
          .batchTPQ(entry.batchTPQ())  // Individual batch TPQ (for debugging)
          .windowSystemTime(windowSystemTime)
          .windowOperatorTime(windowOperatorTime)
          .batchSize(entry.batchSize())
          .build();

      timeline.add(point);
    }

    return timeline;
  }

  public TaskPoolQualityMetric getMetrics() {
    // Compute timeline with windowed + cumulative TPQ
    List<TPQTimelinePoint> timeline = computeTPQTimeline();

    // Calculate overall average TPQ from log
    double totalSystemTime = tpqLog.stream()
        .mapToDouble(TPQLogEntry::systemTime)
        .sum();
    double totalOperatorTime = tpqLog.stream()
        .mapToDouble(TPQLogEntry::operatorTime)
        .sum();
    double averageTPQ = totalOperatorTime > 0
        ? totalSystemTime / totalOperatorTime
        : 0.0;

    // Sample timeline if too large (like picks per face - max 200 points)
    List<TPQTimelinePoint> sampledTimeline = sampleTimeline(timeline, 200);

    log.info("TPQ metrics computed - {} batches, {} timeline points (sampled to {}), avg TPQ={:.2f}",
        tpqLog.size(), timeline.size(), sampledTimeline.size(), averageTPQ);

    return TaskPoolQualityMetric.builder()
        .taskPoolQuality(averageTPQ)  // Overall average
        .totalTpqEvents(tpqLog.size())
        .totalSystemTime(totalSystemTime)
        .totalOperatorTime(totalOperatorTime)
        .taskPoolQualityData(sampledTimeline)  // ← Windowed timeline for frontend!
        .build();
  }

  private <T> List<T> sampleTimeline(List<T> timeline, int maxPoints) {
    if (timeline.size() <= maxPoints) {
      return timeline;
    }

    List<T> sampled = new ArrayList<>();
    double step = (double) timeline.size() / maxPoints;
    for (int i = 0; i < maxPoints; i++) {
      int index = (int) (i * step);
      sampled.add(timeline.get(index));
    }
    return sampled;
  }

  // Helper records
  @Builder
  private record TPQLogEntry(
      double time,
      double batchTPQ,      // Individual batch TPQ
      double systemTime,    // System time for this batch
      double operatorTime,  // Operator time for this batch
      int batchSize
  ) {}
}
```

#### 3.3 Add Timeline Point DTOs

**Modified:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java`

```java
/**
 * Timeline point for Task Pool Quality visualization.
 *
 * <p>Follows same pattern as PicksPerFaceTimelinePoint with windowed and cumulative metrics.
 */
@Builder
public record TPQTimelinePoint(
    double time,
    double timeMinutes,
    double taskPoolQuality,  // 10-minute rolling window TPQ (like avgPicksPerFace)
    double cumulativeTaskPoolQuality,  // Overall average from start (like cumulativeAvgPicksPerFace)
    double batchTPQ,  // Individual batch TPQ (for debugging)
    double windowSystemTime,  // System time in 10-minute window
    double windowOperatorTime,  // Operator time in 10-minute window
    int batchSize  // Number of orders in this batch
) {}

/**
 * Task Pool Quality metrics.
 */
@Builder
public record TaskPoolQualityMetric(
    double taskPoolQuality,  // Overall average
    int totalTpqEvents,
    double totalSystemTime,
    double totalOperatorTime,
    List<TPQTimelinePoint> taskPoolQualityData  // ← Changed from Map to List!
) {}
```

---

### Phase 4: Implement TPQ Strategy

#### 4.1 Create MaximizeTPQStrategy

**New File:** `apps/backend-java/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizeTPQStrategy.java`

```java
package com.greyorange.dessimulator.domain.order.strategies;

import com.greyorange.dessimulator.domain.order.*;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.TPQAlgorithmConfig;
import com.greyorange.dessimulator.simulator.metrics.MetricEvent;
import com.greyorange.dessimulator.simulator.metrics.MetricEventData;
import com.greyorange.dessimulator.simulator.metrics.MetricEventBus;
import lombok.extern.slf4j.Slf4j;

import java.util.*;
import java.util.stream.Collectors;

/**
 * Order selection strategy based on Task Pool Quality (TPQ) optimization.
 *
 * Implements multi-objective score function:
 * 1. TPQ targeting (resource balance)
 * 2. Per-station arrival variability (temporal smoothness)
 * 3. Diversity bonus (spatial mixing to prevent depletion)
 * 4. Depletion penalty (resource scarcity awareness)
 */
@Slf4j
public class MaximizeTPQStrategy implements OrderSelectionStrategy {

  // HARDCODED SCORING WEIGHTS (tunable constants for MVP)
  private static final double TPQ_WEIGHT = 50.0;
  private static final double VARIABILITY_WEIGHT = 20.0;
  private static final double DIVERSITY_WEIGHT = 10.0;
  private static final double DEPLETION_WEIGHT = 50.0;

  // HARDCODED DIVERSITY TARGET
  private static final double TARGET_DISTANCE_CV = 0.4;  // Healthy spatial mix

  // HARDCODED STATION VARIANCE TOLERANCE
  private static final double STATION_VARIANCE_TOLERANCE = 1.0;  // ±1 around target

  private final TPQAlgorithmConfig config;
  private final WarehouseInventory inventory;
  private final List<Station> stations;
  private final Map<StationId, BinManager> stationBinManagers;
  private final int maxTotesPerOrder;
  private final double operatorTimePerPick;
  private final double robotSpeed;
  private final int numRobots;
  private final int numOperators;

  // State tracking for depletion penalty
  private final Map<StorageSpaceId, Integer> storageSpaceUsageCount = new HashMap<>();

  // Calculated target (from config or auto-calculated)
  private final double targetTPQ;

  public MaximizeTPQStrategy(
      TPQAlgorithmConfig config,
      WarehouseInventory inventory,
      List<Station> stations,
      Map<StationId, BinManager> stationBinManagers,
      int maxTotesPerOrder,
      double operatorTimePerPick,
      double robotSpeed,
      int numRobots,
      int numOperators) {

    this.config = config;
    this.inventory = inventory;
    this.stations = stations;
    this.stationBinManagers = stationBinManagers;
    this.maxTotesPerOrder = maxTotesPerOrder;
    this.operatorTimePerPick = operatorTimePerPick;
    this.robotSpeed = robotSpeed;
    this.numRobots = numRobots;
    this.numOperators = numOperators;

    // Calculate target TPQ from throughput balance equation
    this.targetTPQ = config.targetTPQ() != null
        ? config.targetTPQ()
        : calculateTargetTPQ(numRobots, numOperators);

    log.info("TPQ Strategy initialized: target={:.2f}, robots={}, operators={}",
        targetTPQ, numRobots, numOperators);
  }

  /**
   * Calculate target TPQ from system configuration.
   * Formula: (num_bots + num_operators) / (2 × num_operators)
   */
  private double calculateTargetTPQ(int numBots, int numOperators) {
    return (numBots + numOperators) / (2.0 * numOperators);
  }

  @Override
  public List<OrderSelection> selectOrders(
      List<Order> pendingOrders,
      Set<StorageSpaceId> activeStorageSpaces,
      int maxOrders,
      double currentTime) {

    // Similar greedy structure to PPF algorithms but with TPQ scoring

    List<OrderSelection> selected = new ArrayList<>();
    List<Task> currentPool = new ArrayList<>();  // Active tasks

    // TODO: Should get actual active tasks from system, not empty list

    while (selected.size() < maxOrders && !pendingOrders.isEmpty()) {
      Order best = null;
      double bestScore = Double.NEGATIVE_INFINITY;
      List<PickTask> bestTasks = null;

      for (Order candidate : pendingOrders) {
        // Generate tasks using set cover
        List<PickTask> tasks = generateTasksWithSetCover(
            candidate, currentTime, activeStorageSpaces);

        if (tasks.isEmpty()) continue;

        // Create hypothetical pool
        List<Task> hypotheticalPool = new ArrayList<>(currentPool);
        hypotheticalPool.addAll(tasks);

        // Calculate multi-objective score
        double score = calculateOrderScore(
            candidate, tasks, hypotheticalPool, activeStorageSpaces);

        if (score > bestScore) {
          bestScore = score;
          best = candidate;
          bestTasks = tasks;
        }
      }

      if (best != null) {
        StationId stationId = selectOptimalStationForOrder(best, bestTasks);
        selected.add(new OrderSelection(best, bestTasks, stationId));
        currentPool.addAll(bestTasks);
        pendingOrders.remove(best);

        // Update usage tracking
        for (PickTask task : bestTasks) {
          storageSpaceUsageCount.merge(task.getStorageSpaceId(), 1, Integer::sum);
        }
      } else {
        break;
      }
    }

    // Emit TPQ metrics event
    if (!selected.isEmpty()) {
      emitTPQMetrics(selected, currentTime);
    }

    return selected;
  }

  /**
   * Calculate multi-objective score for an order.
   *
   * Uses hardcoded weights for MVP. Can be exposed as config later if needed.
   */
  private double calculateOrderScore(
      Order candidate,
      List<PickTask> tasks,
      List<Task> hypotheticalPool,
      Set<StorageSpaceId> activeStorageSpaces) {

    double score = 0.0;

    // 1. TPQ targeting (primary objective)
    double tpq = calculateTPQ(hypotheticalPool);
    double tpqScore = -Math.abs(tpq - targetTPQ) * TPQ_WEIGHT;
    score += tpqScore;

    // 2. Per-station arrival variability (temporal smoothness)
    Map<StationId, List<Task>> byStation = groupByStation(hypotheticalPool);
    double variabilityScore = -calculateSystemArrivalVariability(byStation) * VARIABILITY_WEIGHT;
    score += variabilityScore;

    // 3. Diversity bonus (spatial mixing to prevent depletion)
    double diversityScore = calculateDiversityBonus(hypotheticalPool) * DIVERSITY_WEIGHT;
    score += diversityScore;

    // 4. Depletion penalty (resource scarcity awareness)
    double depletionPenalty = calculateDepletionPenalty(tasks) * DEPLETION_WEIGHT;
    score += depletionPenalty;

    return score;
  }

  /**
   * Calculate TPQ for a pool of tasks.
   * TPQ = (total_travel_time + total_operator_time) / total_operator_time
   */
  private double calculateTPQ(List<Task> pool) {
    double totalTravel = 0.0;
    double totalOperator = 0.0;

    for (Task task : pool) {
      double travelTime = calculateTravelTime(task);
      double operatorTime = task.getQuantity() * operatorTimePerPick;

      totalTravel += travelTime;
      totalOperator += operatorTime;
    }

    return totalOperator > 0
        ? (totalTravel + totalOperator) / totalOperator
        : Double.POSITIVE_INFINITY;
  }

  // TODO: Implement other scoring components:
  // - calculateSystemArrivalVariability()
  // - calculateDiversityBonus()
  // - calculateDepletionPenalty()
  // - Helper methods for distance calculations, CV computation, etc.

  /**
   * Emit ORDER_BATCH_SELECTED event with TPQ-specific metrics.
   *
   * <p>All strategies emit ORDER_BATCH_SELECTED. TPQ strategy includes TPQ-specific
   * data that TaskPoolQualityCollector will recognize and capture.
   */
  private void emitTPQMetrics(List<OrderSelection> selected, double currentTime) {
    // Extract tasks from selections
    List<Task> tasks = selected.stream()
        .flatMap(sel -> sel.tasks().stream())
        .map(pt -> (Task) pt)
        .collect(Collectors.toList());

    double tpq = calculateTPQ(tasks);

    double totalTravel = tasks.stream()
        .mapToDouble(this::calculateTravelTime)
        .sum();
    double totalOperator = tasks.stream()
        .mapToDouble(t -> t.getQuantity() * operatorTimePerPick)
        .sum();
    double systemTime = totalTravel + totalOperator;

    // Build event data with TPQ-specific fields
    Map<String, Object> eventData = new HashMap<>();
    eventData.put("tpq", tpq);                      // TPQ-specific
    eventData.put("system_time", systemTime);       // TPQ-specific
    eventData.put("operator_time", totalOperator);  // TPQ-specific
    eventData.put("batch_size", selected.size());   // Common

    MetricEventData event = MetricEventData.builder()
        .eventType(MetricEvent.ORDER_BATCH_SELECTED)  // Generic event type
        .timestamp(currentTime)
        .data(eventData)  // TPQ-specific payload
        .build();

    MetricEventBus.getInstance().publish(event);

    log.debug("TPQ batch selected: {} orders, TPQ={:.2f}, target={:.2f}",
        selected.size(), tpq, targetTPQ);
  }

  @Override
  public String getAlgorithmName() {
    return "MAXIMIZE_TPQ_WITH_VUT";
  }

  // TODO: Add helper methods from design document
}
```

---

### Phase 5: Frontend Wiring

#### 5.1 Update Configuration Panel

**File:** `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx` (assumed location)

**Add TPQ algorithm option (SIMPLIFIED):**
```tsx
<Select
  label="Order Selection Algorithm"
  value={config.orderSelectionAlgorithm}
  onChange={(value) => setConfig({...config, orderSelectionAlgorithm: value})}
>
  <option value="maximize_ppf_directly">Maximize PPF Directly</option>
  <option value="maximize_ppf_with_tiering">Maximize PPF with Tiering</option>
  <option value="maximize_tpq_with_vut">Maximize TPQ (Queuing-Based)</option>
</Select>

{config.orderSelectionAlgorithm === 'maximize_tpq_with_vut' && (
  <div className="mt-4">
    <NumberInput
      label="Target TPQ (optional)"
      value={config.tpqTarget}
      onChange={(value) => setConfig({...config, tpqTarget: value})}
      placeholder="Auto-calculated from robot/operator ratio"
      min={1.0}
      max={20.0}
      step={0.5}
      helpText="Leave empty to auto-calculate. Lower = more efficient (less bot overhead)."
    />
  </div>
)}
```

**Note:** All other TPQ parameters (weights, CV targets, etc.) are **hardcoded in MaximizeTPQStrategy.java** and not exposed in UI for MVP.

#### 5.2 Verify PickResults.tsx

**File:** `apps/frontend/src/pages/simulator/components/PickResults.tsx`

**No changes needed!** Already expects timeline array format:
- Line 16: `const tpqTimelineRaw = metrics.taskPoolQualityMetrics.taskPoolQualityData || {}`
- Line 26: `const tpqTimeline = Array.isArray(tpqTimelineRaw) ? tpqTimelineRaw : []`
- Lines 136-174: TPQ timeline chart

Will automatically work once backend emits correct data structure.

---

## Summary of Changes

### Files to Create (New)
1. `OrderSelectionStrategy.java` - Strategy interface
2. `MaximizePpfDirectlyStrategy.java` - PPF direct strategy
3. `MaximizePpfWithTieringStrategy.java` - PPF tiering strategy
4. `MaximizeTPQStrategy.java` - TPQ strategy (main implementation, **weights hardcoded**)

**Note:** No factory class needed - strategies instantiated directly in simulation setup code.

### Files to Modify (Existing)
1. `OrderProcessor.java` - Refactor to use strategy pattern
2. `OrderSelectionAlgorithm.java` - Add MAXIMIZE_TPQ_WITH_VUT
3. `SimulationConfigs.java` - Add TPQAlgorithmConfig record (**only targetTPQ field**)
4. `SimulationsController.java` - Fix DTO→Config mapping, add **tpqTarget** field
5. `MetricEvent.java` - Add ORDER_BATCH_SELECTED event (**single event for all algorithms**)
6. `MetricsCollectors.java` - Implement TaskPoolQualityCollector (**filters by tpq data presence**)
7. `SimulationMetrics.java` - Add TPQTimelinePoint, update TaskPoolQualityMetric
8. `ConfigurationPanel.tsx` (frontend) - Add TPQ option + **single target input**
9. PickResults.tsx - **No changes needed** (already works!)

### Architecture Changes
- **Before:** Algorithms hardcoded in OrderProcessor switch statement
- **After:** Strategy pattern with direct instantiation, extensible architecture
- **Instantiation location:** Where OrderProcessor is created (likely in `DesmojSimulationEngine` or similar simulation setup class)

### Configuration Flow (Fixed)
- **Before:** DTO fields ignored, AlgorithmConfig always default
- **After:** DTO → AlgorithmConfig → Strategy (properly wired)

### Metrics Flow (Fixed)
- **Before:** TaskPoolQualityCollector stub, no events, empty timeline
- **After:**
  - TPQ strategy emits `ORDER_BATCH_SELECTED` with TPQ data
  - TaskPoolQualityCollector filters events (checks for `tpq` field)
  - Timeline generated from captured events
  - Frontend displays (chart already built!)
- **Single event type:** `ORDER_BATCH_SELECTED` used by all strategies, strategy-specific data in payload

### Configuration Philosophy (Simplified for MVP)
- **User-configurable:** Target TPQ only (null = auto-calculate from robots/operators)
- **Hardcoded constants in strategy:**
  - TPQ_WEIGHT = 50.0
  - VARIABILITY_WEIGHT = 20.0
  - DIVERSITY_WEIGHT = 10.0
  - DEPLETION_WEIGHT = 50.0
  - TARGET_DISTANCE_CV = 0.4
  - STATION_VARIANCE_TOLERANCE = 1.0
- **Rationale:** Keep UI simple, avoid overwhelming users with tuning parameters. Can expose more config later if needed based on feedback.

---

## Testing Checkpoints

### Phase 1: Architecture Refactoring
- [ ] Existing PPF algorithms still work (no regression)
- [ ] Strategy factory creates correct instances
- [ ] OrderProcessor delegates to strategy

### Phase 2: Configuration Wiring
- [ ] TPQ algorithm appears in enum
- [ ] DTO accepts TPQ config fields
- [ ] AlgorithmConfig properly built from DTO
- [ ] Validation accepts new algorithm string

### Phase 3: Metrics Collection
- [ ] ORDER_BATCH_SELECTED events emitted
- [ ] TaskPoolQualityCollector captures events (filters by `tpq` field presence)
- [ ] Timeline computed with 10-minute rolling window (like picks per face)
- [ ] Both `taskPoolQuality` (window) and `cumulativeTaskPoolQuality` (overall) present
- [ ] Timeline data structure matches frontend expectations
- [ ] Backend returns `taskPoolQualityData` as array
- [ ] Sampling works (max 200 points like picks per face)

### Phase 4: TPQ Strategy
- [ ] Strategy calculates TPQ correctly
- [ ] Target TPQ auto-calculated from config
- [ ] Multi-objective scoring works
- [ ] Orders selected maintain target TPQ

### Phase 5: Frontend
- [ ] TPQ algorithm selectable in UI
- [ ] TPQ config panel appears when selected
- [ ] Timeline chart displays data
- [ ] "No data" message disappears

---

## Implementation Order Recommendation

1. **Phase 1 First:** Refactor to strategy pattern
   - Establishes clean architecture
   - No functional changes (regression-safe)
   - Makes Phase 4 much easier

2. **Phase 2 Second:** Fix configuration wiring
   - Critical bug fix (DTO→Config broken)
   - Enables proper algorithm selection
   - Unblocks Phase 4 testing

3. **Phase 3 Third:** Implement metrics collection
   - Establishes event infrastructure
   - Needed for Phase 4 validation
   - Frontend already expects this data

4. **Phase 4 Fourth:** Implement TPQ strategy
   - Builds on clean architecture from Phase 1
   - Uses config from Phase 2
   - Emits metrics from Phase 3

5. **Phase 5 Last:** Frontend polish
   - Everything already wired backend-side
   - Just expose configuration options
   - Test end-to-end

---

## Open Questions / TODOs

1. **OrderProcessor Instantiation Location:**
   - Need to find where OrderProcessor is currently created
   - Likely in `DesmojSimulationEngine` or similar setup class
   - Update to instantiate strategy first, then pass to OrderProcessor

2. **Active Task Pool:** How to pass currently active tasks to strategy?
   - Currently strategies start with empty pool
   - Need system state for accurate TPQ calculation

3. **Distance Calculations:** Where to get geometry data?
   - Need StoragePositionManager or similar
   - Calculate travel times for TPQ formula

4. **Relay System TPQ:** Two-stage calculation
   - Need separate strategy or extend MaximizeTPQStrategy?
   - VTM and HTM cycle time calculations

5. **Helper Methods:** Set cover, station selection
   - Extract from OrderProcessor to utility class?
   - Or make package-private for strategies to use?

6. **Testing Data:** Integration test scenarios
   - Need test configs with TPQ algorithm
   - Verify depletion prevention works
   - Compare PPF vs TPQ results

---

**Next Step:** Review this plan, then implement Phase 1 (architecture refactoring) first.
