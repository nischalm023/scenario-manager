# TPQ Calculator API Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a shared TPQ calculator utility and REST endpoint to provide accurate TPQ metrics based on warehouse geometry and station layout.

**Architecture:** Domain utility class `TpqCalculator` performs all calculations based on `SimulationConfig`. Controller exposes endpoint that reuses `SimulationConfigDTO` and returns calculated metrics. Frontend GoalsTab calls this API instead of doing rough local calculations.

**Tech Stack:** Java 21, Spring Boot 3.2, React/TypeScript, OpenAPI type generation

---

## Task 1: Create TpqCalculator Domain Utility

**Note:** The `targetTpq` is calculated from warehouse geometry and `targetPicksPerFace`. This calculated value is then:
1. Returned by the API endpoint for Goals tab display
2. Pre-calculated in `DesmojSimulationEngine` and set in `TPQAlgorithmConfig` for `MaximizeTPQStrategy`

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/TpqCalculator.java`

**Step 1: Create TpqMetrics record and TpqCalculator class**

```java
package com.greyorange.dessimulator.domain.order.strategies;

import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.map.StationLayoutType;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.*;

/**
 * Utility class for Task Pool Quality (TPQ) calculations.
 *
 * <p>Provides static methods for calculating TPQ metrics based on warehouse geometry,
 * station layout, and robot/operator configuration. Used by both the API endpoint
 * and MaximizeTPQStrategy for consistent calculations.
 */
public final class TpqCalculator {

  private static final double SAFETY_FACTOR = 1.2;

  private TpqCalculator() {
    throw new UnsupportedOperationException("Utility class");
  }

  /**
   * TPQ metrics result containing all calculated values.
   *
   * <p>targetTpq is the calculated TPQ based on warehouse geometry and targetPicksPerFace.
   * This is the value that MaximizeTPQStrategy should use as its optimization target.
   */
  public record TpqMetrics(
      double warehouseWidthMeters,
      double warehouseDepthMeters,
      double avgTravelDistanceMeters,
      double avgTravelTimeSeconds,
      double avgOperatorTimeSeconds,
      double targetTpq,
      int optimalRobotsPerStation,
      int totalOptimalRobots,
      String fleetStatus,
      int robotDifference) {}

  /**
   * Calculate all TPQ metrics from simulation configuration.
   */
  public static TpqMetrics calculate(SimulationConfig config) {
    // Extract dimensions
    double cellSize = config.mapConfig().cellSizeMeters();
    double warehouseWidth = config.mapConfig().gridWidth() * cellSize;
    double warehouseDepth = config.mapConfig().gridDepth() * cellSize;

    // Get station layout type
    StationLayoutType layoutType = config.mapConfig().stationLayout().layoutType();

    // Calculate travel distance based on station layout
    double avgTravelDistance = calculateAvgTravelDistance(warehouseWidth, warehouseDepth, layoutType);

    // Get robot speed and count based on system type
    double robotSpeed;
    int robotCount;
    if (config.systemType() == SystemType.RTP) {
      RtpRobotConfig rtpConfig = config.getRtpRobotConfig();
      robotSpeed = rtpConfig.robotSpeed();
      robotCount = rtpConfig.numRobots();
    } else {
      RelayRobotConfig relayConfig = config.getRelayRobotConfig();
      robotSpeed = relayConfig.htmSpeed();
      robotCount = relayConfig.numVtms() + relayConfig.numHtms();
    }

    // Calculate travel time (one-way, not round trip for this estimation)
    double avgTravelTime = avgTravelDistance / robotSpeed;

    // Calculate operator time
    double operatorTimePerPick = config.operatorConfig().operatorTimePerPick();
    double picksPerFace = config.operatorConfig().targetPicksPerFace();
    double avgOperatorTime = operatorTimePerPick * picksPerFace;

    // Calculate target TPQ from geometry and picks per face
    double targetTpq = calculateTpq(avgTravelTime, avgOperatorTime);

    // Calculate optimal fleet size based on target TPQ
    int stationCount = config.mapConfig().stationLayout().totalStations();
    int optimalRobotsPerStation = (int) Math.ceil(targetTpq * SAFETY_FACTOR);
    int totalOptimalRobots = optimalRobotsPerStation * stationCount;

    // Determine fleet status
    int robotDifference = robotCount - totalOptimalRobots;
    String fleetStatus = determineFleetStatus(robotCount, totalOptimalRobots);

    return new TpqMetrics(
        warehouseWidth,
        warehouseDepth,
        avgTravelDistance,
        avgTravelTime,
        avgOperatorTime,
        targetTpq,
        optimalRobotsPerStation,
        totalOptimalRobots,
        fleetStatus,
        robotDifference);
  }

  /**
   * Calculate average travel distance based on station layout geometry.
   *
   * <p>Uses Manhattan distance estimation based on where stations are positioned:
   * - Bottom/Top edge: depth/2 (vertical) + width/4 (lateral)
   * - Left/Right edge: width/2 (horizontal) + depth/4 (lateral)
   * - Opposite edges: reduced travel due to multiple access points
   */
  public static double calculateAvgTravelDistance(
      double warehouseWidth, double warehouseDepth, StationLayoutType layoutType) {
    return switch (layoutType) {
      case SINGLE_BOTTOM, SINGLE_TOP -> warehouseDepth / 2 + warehouseWidth / 4;
      case SINGLE_LEFT, SINGLE_RIGHT -> warehouseWidth / 2 + warehouseDepth / 4;
      case OPPOSITE_VERTICAL -> warehouseDepth / 4 + warehouseWidth / 4;
      case OPPOSITE_HORIZONTAL -> warehouseWidth / 4 + warehouseDepth / 4;
      case FOUR_EDGE -> (warehouseWidth + warehouseDepth) / 4;
      case CUSTOM -> warehouseDepth / 2 + warehouseWidth / 4; // Default to bottom edge
    };
  }

  /**
   * Calculate TPQ from travel and operator times.
   *
   * <p>TPQ = (travel_time + operator_time) / operator_time
   */
  public static double calculateTpq(double travelTime, double operatorTime) {
    if (operatorTime <= 0) {
      return 0.0;
    }
    return (travelTime + operatorTime) / operatorTime;
  }

  /**
   * Determine fleet status based on current vs optimal robot count.
   */
  private static String determineFleetStatus(int currentRobots, int optimalRobots) {
    double percentDifference = Math.abs(currentRobots - optimalRobots) * 100.0 / optimalRobots;

    if (percentDifference <= 10) {
      return "optimal";
    } else if (currentRobots > optimalRobots) {
      return "over-staffed";
    } else {
      return "under-staffed";
    }
  }
}
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/TpqCalculator.java
git commit -m "feat: add TpqCalculator domain utility for TPQ metrics

Provides static methods for calculating TPQ based on warehouse geometry
and station layout. Reusable by both API endpoint and MaximizeTPQStrategy.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: Add TPQ Metrics Endpoint to SimulationsController

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java`

**Step 1: Add TpqMetricsResponseDTO record**

Add after the existing `ComparisonConfigDTO` record (around line 283):

```java
  record TpqMetricsResponseDTO(
      double warehouseWidthMeters,
      double warehouseDepthMeters,
      double avgTravelDistanceMeters,
      double avgTravelTimeSeconds,
      double avgOperatorTimeSeconds,
      double targetTpq,
      int optimalRobotsPerStation,
      int totalOptimalRobots,
      String fleetStatus,
      int robotDifference) {}
```

**Step 2: Add import for TpqCalculator**

Add to imports section:

```java
import com.greyorange.dessimulator.domain.order.strategies.TpqCalculator;
```

**Step 3: Add endpoint method**

Add after the `runComparison` method (around line 109):

```java
  @PostMapping(ApiPaths.SIMULATION + "/tpq-metrics")
  @Tag(name = "Simulation")
  @Operation(summary = "Calculate TPQ Metrics from Configuration")
  ResponseEntity<ApiResponse<TpqMetricsResponseDTO>> calculateTpqMetrics(
      @Valid @RequestBody SimulationConfigDTO config) {
    SimulationConfig coreConfig = config.toCoreConfig();
    TpqCalculator.TpqMetrics metrics = TpqCalculator.calculate(coreConfig);

    TpqMetricsResponseDTO response =
        new TpqMetricsResponseDTO(
            metrics.warehouseWidthMeters(),
            metrics.warehouseDepthMeters(),
            metrics.avgTravelDistanceMeters(),
            metrics.avgTravelTimeSeconds(),
            metrics.avgOperatorTimeSeconds(),
            metrics.targetTpq(),
            metrics.optimalRobotsPerStation(),
            metrics.totalOptimalRobots(),
            metrics.fleetStatus(),
            metrics.robotDifference());

    return ResponseEntity.ok(ApiResponse.success(response));
  }
```

**Step 4: Verify compilation**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java
git commit -m "feat: add POST /simulation/tpq-metrics endpoint

Calculates TPQ metrics from SimulationConfigDTO using TpqCalculator.
Returns warehouse dimensions, travel/operator times, TPQ values, and
fleet optimization recommendations.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: Regenerate Frontend Types

**Files:**
- Modify: `apps/frontend/src/types/generated-api.ts` (auto-generated)

**Step 1: Start backend server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true && sleep 2 && cd apps/backend && mvn spring-boot:run > /tmp/server.log 2>&1 &`

Wait for server to start:
Run: `for i in {1..30}; do sleep 1; if curl -s http://localhost:8080/api/health > /dev/null; then echo "Server ready"; break; fi; done`

**Step 2: Regenerate TypeScript types**

Run: `npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts`
Expected: File updated with new `TpqMetricsResponseDTO` type

**Step 3: Stop backend server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 4: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "chore: regenerate frontend types with TpqMetricsResponseDTO

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: Add API Service Method for TPQ Metrics

**Files:**
- Modify: `apps/frontend/src/services/api.ts`

**Step 1: Add getTpqMetrics method to apiService**

Find the `apiService` object and add this method:

```typescript
  async getTpqMetrics(config: SimulationConfig): Promise<ApiResponse<components['schemas']['TpqMetricsResponseDTO']>> {
    // Convert frontend config to API format (same as runSimulation)
    const apiConfig = {
      systemType: config.systemType,
      duration: config.simulationDuration,
      scenario: config.scenario,
      mapConfig: {
        ...config.mapConfig,
        stationLayout: {
          ...config.mapConfig?.stationLayout,
          totalStations: config.stationCount,
        },
      },
      rtpRobotCount: config.rtpRobotCount,
      rtpRobotSpeed: config.rtpRobotSpeed,
      numVtms: config.numVtms,
      numHtms: config.numHtms,
      vtmSpeed: config.vtmSpeed,
      htmSpeed: config.htmSpeed,
      operatorTime: config.operatorTime,
      picksPerFace: config.picksPerFace,
      binsCount: config.binsCount,
      orderSelectionAlgorithm: config.orderSelectionAlgorithm,
      putAlgorithm: config.putAlgorithm,
    }

    const response = await fetch(`${API_BASE_URL}/simulation/tpq-metrics`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(apiConfig),
    })

    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`)
    }

    return response.json()
  },
```

**Step 2: Verify TypeScript compiles**

Run: `npx tsc --noEmit --project apps/frontend/tsconfig.json`
Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/services/api.ts
git commit -m "feat: add getTpqMetrics API service method

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: Update GoalsTab to Use Backend API

**Files:**
- Modify: `apps/frontend/src/pages/simulator/tabs/GoalsTab.tsx`

**Step 1: Replace local calculations with API call**

Replace the entire file content:

```typescript
import type React from 'react'
import { useEffect, useState } from 'react'
import MetricCard from '../../../components/common/display/MetricCard'
import SectionHeader from '../../../components/common/display/SectionHeader'
import ConfigurationTable from '../../../components/common/forms/ConfigurationTable'
import { apiService } from '../../../services/api'
import type { SimulationConfig } from '../../../types'
import type { components } from '../../../types/generated-api'

type TpqMetrics = components['schemas']['TpqMetricsResponseDTO']

interface GoalsTabProps {
  config: SimulationConfig
}

const GoalsTab: React.FC<GoalsTabProps> = ({ config }) => {
  const [metrics, setMetrics] = useState<TpqMetrics | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Fetch TPQ metrics from backend when config changes
  useEffect(() => {
    const fetchMetrics = async () => {
      setLoading(true)
      setError(null)
      try {
        const response = await apiService.getTpqMetrics(config)
        if (response.success && response.data) {
          setMetrics(response.data)
        } else {
          setError(response.error || 'Failed to fetch TPQ metrics')
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch TPQ metrics')
      } finally {
        setLoading(false)
      }
    }

    fetchMetrics()
  }, [config])

  // Show loading state
  if (loading && !metrics) {
    return (
      <div className="flex items-center justify-center h-64">
        <p className="text-gray-500">Calculating TPQ metrics...</p>
      </div>
    )
  }

  // Show error state
  if (error && !metrics) {
    return (
      <div className="flex items-center justify-center h-64">
        <p className="text-red-500">Error: {error}</p>
      </div>
    )
  }

  // Use metrics or defaults
  const warehouseWidth = metrics?.warehouseWidthMeters ?? 0
  const warehouseDepth = metrics?.warehouseDepthMeters ?? 0
  const avgTravelDistance = metrics?.avgTravelDistanceMeters ?? 0
  const avgTravelTime = metrics?.avgTravelTimeSeconds ?? 0
  const avgOperatorTime = metrics?.avgOperatorTimeSeconds ?? 0
  const targetTPQ = metrics?.targetTpq ?? 0
  const optimalRobotsPerStation = metrics?.optimalRobotsPerStation ?? 0
  const totalOptimalRobots = metrics?.totalOptimalRobots ?? 0
  const fleetStatus = metrics?.fleetStatus ?? 'unknown'
  const robotDifference = metrics?.robotDifference ?? 0

  // Ideal TPQ threshold for display purposes (lower is better)
  const idealTPQ = 2.0

  // Get current robot count from config
  const currentTotalRobots =
    config.systemType === 'rtp'
      ? config.rtpRobotCount ?? 0
      : (config.numVtms ?? 0) + (config.numHtms ?? 0)

  const robotSpeed =
    config.systemType === 'rtp' ? config.rtpRobotSpeed ?? 1.0 : config.htmSpeed ?? 1.0

  const operatorTimePerPick = config.operatorTime ?? 12
  const picksPerFace = config.picksPerFace ?? 3
  const stationCount = config.mapConfig?.stationLayout?.totalStations ?? config.stationCount ?? 4

  // Determine status colors
  const getFleetStatusVariant = (): 'success' | 'warning' | 'info' | 'neutral' => {
    switch (fleetStatus) {
      case 'optimal':
        return 'success'
      case 'over-staffed':
        return 'warning'
      case 'under-staffed':
        return 'info'
      default:
        return 'neutral'
    }
  }

  const getFleetStatusLabel = (): string => {
    switch (fleetStatus) {
      case 'optimal':
        return 'Optimal'
      case 'over-staffed':
        return 'Over-staffed'
      case 'under-staffed':
        return 'Under-staffed'
      default:
        return 'Unknown'
    }
  }

  // Configuration table data
  const configurationRows = [
    {
      category: 'Travel Parameters',
      parameter: 'Robot Speed',
      value: robotSpeed.toFixed(1),
      unit: 'm/s',
      notes: 'Base travel speed',
      rowSpan: 4,
    },
    {
      parameter: 'Warehouse Grid',
      value: `${config.mapConfig?.gridWidth ?? 0}×${config.mapConfig?.gridDepth ?? 0}`,
      unit: 'racks',
      notes: 'Layout dimensions',
    },
    {
      parameter: 'Avg Travel Distance',
      value: avgTravelDistance.toFixed(1),
      unit: 'm',
      notes: 'Station to rack average',
    },
    {
      parameter: 'Avg Travel Time',
      value: avgTravelTime.toFixed(0),
      unit: 's',
      notes: 'Based on robot speed',
    },
    {
      category: 'Operator Parameters',
      parameter: 'Operator Time per Pick',
      value: operatorTimePerPick,
      unit: 's',
      notes: 'Per item processing',
      rowSpan: 4,
    },
    {
      parameter: 'Picks per Face',
      value: picksPerFace,
      unit: 'items',
      notes: 'Items per rack face',
    },
    {
      parameter: 'Avg Operator Time',
      value: avgOperatorTime.toFixed(0),
      unit: 's',
      notes: 'Total per cycle',
    },
    {
      parameter: 'Calculated TPQ',
      value: actualTPQ.toFixed(2),
      unit: 'ratio',
      notes: '(Travel + Operator) / Operator',
    },
  ]

  return (
    <div className="space-y-12">
      {/* Task Pool Quality / Robot Fleet Optimization */}
      <div>
        <SectionHeader title="Task Pool Quality / Robot Fleet Optimization" />

        <div className="grid grid-cols-4 gap-4">
          <MetricCard
            title="Target TPQ"
            value={targetTPQ.toFixed(2)}
            subtitle={`Ideal: ≤${idealTPQ.toFixed(1)}`}
            status={targetTPQ <= idealTPQ ? 'success' : 'warning'}
            icon={<div className="w-4 h-4 text-green-600">{targetTPQ <= idealTPQ ? '✓' : '⚠'}</div>}
          />

          <MetricCard
            title="Optimal Robots per Station"
            value={optimalRobotsPerStation}
            subtitle="Based on calculated TPQ"
            status="neutral"
          />

          <MetricCard
            title="Current Total Robots"
            value={currentTotalRobots}
            subtitle={`vs. ${totalOptimalRobots} optimal`}
            status={getFleetStatusVariant()}
            icon={
              <div className="w-4 h-4 text-orange-600">
                {robotDifference > 0 ? '↑' : robotDifference < 0 ? '↓' : '='}
              </div>
            }
          />

          <MetricCard
            title="Fleet Status"
            value={getFleetStatusLabel()}
            subtitle={`${Math.abs(robotDifference)} robot difference`}
            status={getFleetStatusVariant()}
          />
        </div>
      </div>

      {/* Configuration Parameters */}
      <div>
        <SectionHeader title="Configuration Parameters" />

        <ConfigurationTable rows={configurationRows} />
      </div>

      {/* Task Pool Quality Analysis */}
      <div>
        <SectionHeader title="Task Pool Quality Analysis" />

        <div className="bg-blue-50 rounded-lg p-6">
          <div className="text-sm text-gray-700 space-y-4">
            <p>
              Task Pool Quality (TPQ) measures warehouse efficiency using travel time optimization:
            </p>
            <ul className="list-disc list-inside space-y-2 ml-4">
              <li>
                <strong>
                  TPQ = (Travel Time + Operator Time) / Operator Time = ({avgTravelTime.toFixed(0)}s +{' '}
                  {avgOperatorTime.toFixed(0)}s) / {avgOperatorTime.toFixed(0)}s = {targetTPQ.toFixed(2)}
                </strong>
              </li>
              <li>
                Your warehouse dimensions: {config.mapConfig?.gridWidth ?? 0}×
                {config.mapConfig?.gridDepth ?? 0} racks = {warehouseWidth.toFixed(1)}m ×{' '}
                {warehouseDepth.toFixed(1)}m
              </li>
              <li>
                Average travel distance: {avgTravelDistance.toFixed(1)}m at {robotSpeed}
                m/s = {avgTravelTime.toFixed(0)}s travel time
              </li>
              <li>
                Optimal robot count per station = ceil(TPQ × 1.2 safety factor) ={' '}
                {optimalRobotsPerStation} robots
              </li>
              <li>
                Total optimal robots = {optimalRobotsPerStation} robots/station ×{' '}
                {stationCount} stations = {totalOptimalRobots} robots
              </li>
            </ul>
            <div className="mt-6 p-4 bg-orange-50 border border-orange-200 rounded-lg">
              <strong>
                Current Analysis: Your warehouse has a target TPQ of {targetTPQ.toFixed(2)} (ideal: ≤{idealTPQ.toFixed(1)}).
                {targetTPQ <= idealTPQ
                  ? ` Excellent! This is within optimal range.`
                  : ` This exceeds the ideal range - consider increasing picks per face, improving operator efficiency, or optimizing warehouse layout.`}
                You need {totalOptimalRobots} robots total ({currentTotalRobots} current ={' '}
                {fleetStatus}).
              </strong>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

export default GoalsTab
```

**Step 2: Verify TypeScript compiles**

Run: `npx tsc --noEmit --project apps/frontend/tsconfig.json`
Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/tabs/GoalsTab.tsx
git commit -m "feat: update GoalsTab to fetch TPQ metrics from backend API

Replaces local rough calculations with accurate backend calculations
based on station layout geometry. Uses useEffect to fetch metrics
when config changes.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: Move Picks Per Face and Remove TPQ Target Input

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx`

**Step 1: Move picksPerFace slider from Operator section to Algorithm section**

Find the Operator CollapsibleSection (around line 438) and remove the picksPerFace slider:

```typescript
          <CollapsibleSection
            title="Operator"
            isOpen={isOperatorOpen}
            onToggle={() => setIsOperatorOpen(!isOperatorOpen)}
          >
            <ConfigSlider
              label="Operator time per pick"
              value={config.operatorTime}
              unit="s"
              min={5}
              max={20}
              onChange={(value) => onConfigChange('operatorTime', value)}
            />

            {/* REMOVE picksPerFace SLIDER FROM HERE */}
          </CollapsibleSection>
```

Then update the Algorithm CollapsibleSection to add picksPerFace and REMOVE the TPQ target input:

```typescript
          <CollapsibleSection
            title="Algorithm"
            isOpen={isAlgorithmOpen}
            onToggle={() => setIsAlgorithmOpen(!isAlgorithmOpen)}
          >
            <FormSelect
              label="Put algorithm"
              value={config.putAlgorithm}
              onValueChange={(value) => onConfigChange('putAlgorithm', value)}
              options={putAlgorithm}
              placeholder="Select a put algorithm..."
            />

            <FormSelect
              label="Order selection algorithm"
              value={config.orderSelectionAlgorithm}
              onValueChange={(value) => onConfigChange('orderSelectionAlgorithm', value)}
              options={orderSelectionAlgorithm}
              placeholder="Select an algorithm..."
            />

            <ConfigSlider
              label="Target picks per face"
              value={config.picksPerFace}
              min={1}
              max={10}
              onChange={(value) => onConfigChange('picksPerFace', value)}
            />
            <p className="text-xs text-gray-500 mt-2">
              Higher values improve TPQ efficiency by amortizing travel time over more picks.
            </p>
          </CollapsibleSection>
```

**Note:** The TPQ target input (`{config.orderSelectionAlgorithm === 'maximize_tpq_with_vut' && ...}`) is REMOVED because targetTpq is now calculated from targetPicksPerFace.

**Step 2: Verify TypeScript compiles**

Run: `npx tsc --noEmit --project apps/frontend/tsconfig.json`
Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
git commit -m "refactor: move picksPerFace from Operator to Algorithm section

picksPerFace is primarily an algorithm parameter affecting TPQ calculation
and order selection strategy, not a physical operator characteristic.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 7: Remove tpqTarget from SimulationConfigDTO

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java`
- Modify: `apps/frontend/src/pages/SimulatorPage.tsx`

**Step 1: Remove tpqTarget from SimulationConfigDTO**

In SimulationsController.java, remove the `tpqTarget` field from `SimulationConfigDTO` record (around line 131):

```java
  record SimulationConfigDTO(
      @Min(1) @Max(24) Integer duration,
      // ... other fields ...
      Boolean enableTpqOptimization,
      // REMOVE: Double tpqTarget,
      Boolean enableSnapshots,
      @DecimalMin("1.0") @DecimalMax("3600.0") Double snapshotIntervalSeconds) {
```

Also remove from the compact constructor defaults and the `toCoreConfig()` method's TPQAlgorithmConfig creation.

**Step 2: Remove tpqTarget from ComparisonConfigDTO**

Same removal in `ComparisonConfigDTO`.

**Step 3: Remove tpqTarget from frontend config**

In SimulatorPage.tsx, remove `tpqTarget` from the default config state (around line 48) and from `handleRunSimulation` API config mapping.

**Step 4: Verify compilation**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java apps/frontend/src/pages/SimulatorPage.tsx
git commit -m "refactor: remove tpqTarget from config DTOs

targetTpq is now calculated from warehouse geometry and targetPicksPerFace
using TpqCalculator, not set manually by user.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 8: Update DesmojSimulationEngine to Pre-calculate targetTpq

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Add import for TpqCalculator**

```java
import com.greyorange.dessimulator.domain.order.strategies.TpqCalculator;
```

**Step 2: Pre-calculate targetTpq before creating OrderManager**

Find where OrderManager is created (around line 121) and add calculation before it:

```java
    // Pre-calculate targetTpq using TpqCalculator
    TpqCalculator.TpqMetrics tpqMetrics = TpqCalculator.calculate(config);
    TPQAlgorithmConfig tpqConfigWithTarget = TPQAlgorithmConfig.builder()
        .targetTPQ(tpqMetrics.targetTpq())
        .build();

    this.orderManager =
        new OrderManager(
            warehouseComponents.inventory(),
            warehouseComponents.stationManager().getAllStations(),
            config.operatorConfig().numBins(),
            config.algorithmConfig().orderSelectionAlgorithm(),
            config.algorithmConfig().maxTotesPerOrder(),
            tpqConfigWithTarget,  // Use pre-calculated config
            warehouseComponents.gridPopularityManager(),
            storageSpaceManager,
            config.operatorConfig().operatorTimePerPick(),
            robotSpeed,
            numRobots,
            metricsManager);
```

**Step 3: Add logging**

```java
    log.info(
        "Calculated targetTpq={:.2f} from geometry and targetPicksPerFace={}",
        tpqMetrics.targetTpq(),
        config.operatorConfig().targetPicksPerFace());
```

**Step 4: Verify compilation**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "feat: pre-calculate targetTpq in DesmojSimulationEngine

Uses TpqCalculator to compute targetTpq from warehouse geometry and
targetPicksPerFace, then passes to MaximizeTPQStrategy via TPQAlgorithmConfig.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 9: Simplify MaximizeTPQStrategy

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizeTPQStrategy.java`

**Step 1: Remove fallback targetTpq calculation**

In the constructor, replace the conditional targetTpq assignment (around line 69):

```java
    // OLD CODE - REMOVE:
    // this.targetTPQ =
    //     config.targetTPQ() != null
    //         ? config.targetTPQ()
    //         : calculateTargetTPQ(numRobots, numOperators);

    // NEW CODE - just use what's passed (pre-calculated by TpqCalculator):
    if (config.targetTPQ() == null) {
      throw new IllegalArgumentException("targetTPQ must be pre-calculated by TpqCalculator");
    }
    this.targetTPQ = config.targetTPQ();
```

**Step 2: Remove the calculateTargetTPQ method**

Delete the private method `calculateTargetTPQ(int numBots, int numOperators)` since it's no longer used.

**Step 3: Update constructor parameters**

Remove `numRobots` and `numOperators` from constructor since they were only used for the fallback calculation.

**Step 4: Update OrderManager call**

In OrderManager.java, update the `MaximizeTPQStrategy` instantiation to not pass `numRobots` and `stations.size()`:

```java
          case MAXIMIZE_TPQ_WITH_VUT -> new MaximizeTPQStrategy(
              tpqConfig,
              inventory,
              stations,
              stationBinManagers,
              maxTotesPerOrder,
              operatorTimePerPick,
              robotSpeed,
              gridPopularityManager,
              storageSpaceManager);
```

**Step 5: Verify compilation**

Run: `cd apps/backend && mvn compile -DskipTests`
Expected: BUILD SUCCESS

**Step 6: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizeTPQStrategy.java apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderManager.java
git commit -m "refactor: simplify MaximizeTPQStrategy to use pre-calculated targetTpq

Remove fallback calculation - targetTpq must now be provided via
TPQAlgorithmConfig, pre-calculated by TpqCalculator in DesmojSimulationEngine.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 10: Integration Test

**Step 1: Start backend server**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true && sleep 2 && cd apps/backend && mvn spring-boot:run > /tmp/server.log 2>&1 &`

Wait for server:
Run: `for i in {1..30}; do sleep 1; if curl -s http://localhost:8080/api/health > /dev/null; then echo "Server ready"; break; fi; done`

**Step 2: Test TPQ metrics endpoint**

Run:
```bash
curl -X POST http://localhost:8080/api/simulation/tpq-metrics \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "rtp",
    "duration": 4,
    "scenario": "scenario_a",
    "mapConfig": {
      "gridWidth": 22,
      "gridDepth": 26,
      "cellSizeMeters": 1.0,
      "stationLayout": {
        "layoutType": "single_bottom",
        "totalStations": 4
      },
      "highwayConfig": { "highwaysPerEdge": 3 },
      "rackBlockConfig": { "rowsPerBlock": 10, "aisleWidthGridUnits": 1, "crossAisleWidthGridUnits": 1, "rackColumns": 2 }
    },
    "rtpRobotCount": 4,
    "rtpRobotSpeed": 1.5,
    "operatorTime": 12,
    "picksPerFace": 3,
    "binsCount": 8
  }' | jq
```

Expected: JSON response with calculated TPQ metrics including `warehouseWidthMeters`, `warehouseDepthMeters`, `avgTravelDistanceMeters`, `calculatedTpq`, `targetTpq`, `fleetStatus`, etc.

**Step 3: Start frontend and verify GoalsTab**

Run: `npm run dev`

Navigate to Simulator page, click Goals tab. Verify:
- Metrics are fetched from backend (no more Infinity/NaN)
- Values update when config changes
- Fleet status shows correctly

**Step 4: Stop servers**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

---

## Summary

This plan implements:
1. **TpqCalculator** - Domain utility with geometry-based travel distance calculation
2. **API Endpoint** - `POST /simulation/tpq-metrics` using existing `SimulationConfigDTO`
3. **Frontend Integration** - GoalsTab fetches from API, ConfigurationPanel moves picksPerFace to Algorithm section
4. **Remove tpqTarget override** - User sets targetPicksPerFace, system calculates targetTpq
5. **DesmojSimulationEngine update** - Pre-calculates targetTpq and passes to MaximizeTPQStrategy
6. **MaximizeTPQStrategy simplification** - Uses passed targetTpq without fallback formula

**Key insight:** The user controls `targetPicksPerFace` (how many items to batch per rack visit). The system calculates `targetTpq` from warehouse geometry + picks per face. This targetTpq is what MaximizeTPQStrategy optimizes toward.
