# Put Algorithm Comparison Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix and simplify the algorithm comparison runner to compare Put Algorithms (Greedy vs Min Weighted Cycle Distance) with backend-calculated metrics and a streamlined frontend display.

**Architecture:** Backend extracts metrics from nested `SimulationResults.metrics.*` paths, calculates comparisons (best performer, percentage differences), and returns type-safe DTOs. Frontend displays pre-calculated data without computation logic.

**Tech Stack:** Java 21 (Spring Boot, Records, Enums), TypeScript/React, OpenAPI type generation

---

## Task 1: Create ComparisonMetricType Enum

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonMetricType.java`

**Step 1: Create the enum file**

```java
package com.greyorange.dessimulator.api.simulations;

/**
 * Enumeration of metrics used in algorithm comparison analysis.
 *
 * <p>Each metric represents a key performance indicator extracted from simulation results.
 * OpenAPI will generate TypeScript union types for type-safe frontend access.
 */
public enum ComparisonMetricType {
  /** Total number of orders completed during simulation */
  ORDERS_COMPLETED,

  /** Total number of pick tasks processed by operators */
  TOTAL_PICKS_PROCESSED,

  /** Average picks per hour per station (throughput metric) */
  PICKS_PER_HOUR_PER_STATION,

  /** Operator utilization as percentage (0-100) */
  OPERATOR_UTILIZATION,

  /** Average picks per face presentation (batching efficiency) */
  PICKS_PER_FACE
}
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonMetricType.java
git commit -m "feat: add ComparisonMetricType enum for type-safe metrics"
```

---

## Task 2: Create ComparisonMetric Record

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonMetric.java`

**Step 1: Create the record file**

```java
package com.greyorange.dessimulator.api.simulations;

import java.util.List;

/**
 * Represents a single metric comparison across multiple algorithm modes.
 *
 * <p>Contains raw values for each mode, identifies the best performer, and provides
 * pre-calculated percentage differences for frontend display.
 *
 * <p>Example: Comparing "Picks Per Face" for Greedy (2.3) vs Min Weighted (2.8)
 *
 * <ul>
 *   <li>values: [2.3, 2.8]
 *   <li>bestModeIndex: 1
 *   <li>percentageDifferences: [-17.86, 0.0]
 * </ul>
 */
public record ComparisonMetric(
    /** Type of metric being compared */
    ComparisonMetricType metricName,

    /** Metric values, one per algorithm mode (order matches mode list) */
    List<Double> values,

    /** Display unit (e.g., "picks", "%", "picks/hr") */
    String unit,

    /** True if higher values are better, false if lower is better */
    boolean higherIsBetter,

    /** Index of the best performing mode in the values list */
    int bestModeIndex,

    /**
     * Percentage difference from best performer for each mode.
     *
     * <p>Calculated as: ((value - bestValue) / bestValue) * 100
     *
     * <p>Best performer always has 0.0%, others show negative percentages.
     */
    List<Double> percentageDifferences) {}
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonMetric.java
git commit -m "feat: add ComparisonMetric record for metric comparisons"
```

---

## Task 3: Create ComparisonResponse DTOs

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonResponse.java`

**Step 1: Create the response records**

```java
package com.greyorange.dessimulator.api.simulations;

import com.greyorange.dessimulator.simulator.config.SimulationConfigs.SimulationConfig;
import com.greyorange.dessimulator.simulator.metrics.SimulationMetrics.SimulationResults;
import java.util.List;
import java.util.Map;

/**
 * Response containing algorithm comparison results with pre-calculated analytics.
 *
 * <p>Includes full simulation results per mode, extracted comparison metrics, timeline data, and
 * win counts for frontend display without additional computation.
 */
public record ComparisonResponse(
    /** Overall success status */
    boolean success,

    /** Full simulation results per algorithm mode, keyed by mode key (e.g., "greedy") */
    Map<String, ModeResult> results,

    /** Pre-calculated metric comparisons with best performer analysis */
    List<ComparisonMetric> comparisonMetrics,

    /** Timeline data per mode for charting (e.g., picks per face over time) */
    Map<String, List<?>> timelineData,

    /** Count of metrics won by each mode, keyed by mode key */
    Map<String, Integer> winsPerMode,

    /** Execution time in seconds per mode */
    Map<String, Double> executionTimes,

    /** Total execution time for all modes in seconds */
    double totalExecutionTime,

    /** ISO 8601 timestamp of comparison completion */
    String timestamp) {}

/**
 * Result for a single algorithm mode within a comparison.
 *
 * <p>Contains configuration, full simulation results, mode metadata, and success status.
 */
record ModeResult(
    /** Configuration used for this mode's simulation */
    SimulationConfig config,

    /** Full simulation results (null if failed) */
    SimulationResults result,

    /** Mode metadata (name, key, emoji, config overrides) */
    Map<String, Object> modeInfo,

    /** Whether this mode's simulation succeeded */
    boolean success,

    /** Error message if simulation failed (null if successful) */
    String error) {}
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonResponse.java
git commit -m "feat: add ComparisonResponse DTOs for comparison results"
```

---

## Task 4: Update ComparisonService - Metric Extraction

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java:28-104`

**Step 1: Update runComparison method signature**

Change line 28 from:
```java
Map<String, Object> runComparison(String comparisonType, SimulationConfig baseConfig) {
```

To:
```java
ComparisonResponse runComparison(String comparisonType, SimulationConfig baseConfig) {
```

**Step 2: Add private fields for structured results**

After line 22 (`private final Map<String, Double> executionTimes = new HashMap<>();`), add:

```java
  private final Map<String, ModeResult> modeResults = new HashMap<>();
```

**Step 3: Update result storage logic**

Replace lines 52-59 (the modeResult Map creation) with:

```java
        ModeResult modeResult =
            new ModeResult(modeConfig, result, mode, result.success(), null);
        modeResults.put(modeKey, modeResult);
```

**Step 4: Update error handling result storage**

Replace lines 72-80 (the error modeResult Map creation) with:

```java
        ModeResult modeResult = new ModeResult(baseConfig, null, mode, false, e.getMessage());
        modeResults.put(modeKey, modeResult);
```

**Step 5: Add metric extraction helper method**

Add this method after the `getSummaryStats()` method (after line 160):

```java
  private List<ComparisonMetric> calculateComparisonMetrics() {
    List<ModeResult> successfulResults =
        modeResults.values().stream().filter(ModeResult::success).toList();

    if (successfulResults.isEmpty()) {
      return List.of();
    }

    return List.of(
        extractMetric(
            ComparisonMetricType.ORDERS_COMPLETED,
            successfulResults,
            "orders",
            true,
            r -> (double) r.result().metrics().taskMetrics().totalOrdersCompleted()),
        extractMetric(
            ComparisonMetricType.TOTAL_PICKS_PROCESSED,
            successfulResults,
            "picks",
            true,
            r -> (double) r.result().metrics().operatorMetrics().totalPicksProcessed()),
        extractMetric(
            ComparisonMetricType.PICKS_PER_HOUR_PER_STATION,
            successfulResults,
            "picks/hr",
            true,
            r -> r.result().metrics().stationMetrics().picksPerHourPerStation()),
        extractMetric(
            ComparisonMetricType.OPERATOR_UTILIZATION,
            successfulResults,
            "%",
            true,
            r -> r.result().metrics().operatorMetrics().operatorUtilization() * 100),
        extractMetric(
            ComparisonMetricType.PICKS_PER_FACE,
            successfulResults,
            "picks/face",
            true,
            r -> r.result().metrics().storageSpaceMetrics().picksPerFace()));
  }
```

**Step 6: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 7: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "feat: update ComparisonService to use new DTOs and extract metrics"
```

---

## Task 5: Add Metric Comparison Logic

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java`

**Step 1: Add extractMetric helper method**

Add this method after `calculateComparisonMetrics()`:

```java
  private ComparisonMetric extractMetric(
      ComparisonMetricType metricType,
      List<ModeResult> results,
      String unit,
      boolean higherIsBetter,
      java.util.function.Function<ModeResult, Double> extractor) {

    List<Double> values = results.stream().map(extractor).toList();

    // Find best performer index
    int bestIndex = 0;
    double bestValue = values.get(0);
    for (int i = 1; i < values.size(); i++) {
      double value = values.get(i);
      if (higherIsBetter ? value > bestValue : value < bestValue) {
        bestValue = value;
        bestIndex = i;
      }
    }

    // Calculate percentage differences
    final double finalBestValue = bestValue;
    List<Double> percentageDifferences =
        values.stream()
            .map(
                value ->
                    finalBestValue == 0
                        ? 0.0
                        : ((value - finalBestValue) / finalBestValue) * 100)
            .toList();

    return new ComparisonMetric(
        metricType, values, unit, higherIsBetter, bestIndex, percentageDifferences);
  }
```

**Step 2: Add wins calculation method**

Add this method after `extractMetric()`:

```java
  private Map<String, Integer> calculateWinsPerMode(List<ComparisonMetric> metrics) {
    Map<String, Integer> wins = new HashMap<>();
    List<String> modeKeys = modeResults.keySet().stream().sorted().toList();

    for (String key : modeKeys) {
      wins.put(key, 0);
    }

    for (ComparisonMetric metric : metrics) {
      String winningKey = modeKeys.get(metric.bestModeIndex());
      wins.put(winningKey, wins.get(winningKey) + 1);
    }

    return wins;
  }
```

**Step 3: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "feat: add metric comparison and wins calculation logic"
```

---

## Task 6: Add Timeline Data Extraction

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java`

**Step 1: Add timeline extraction method**

Add this method after `calculateWinsPerMode()`:

```java
  private Map<String, List<?>> extractTimelineData() {
    Map<String, List<?>> timelineData = new HashMap<>();

    for (Map.Entry<String, ModeResult> entry : modeResults.entrySet()) {
      String modeKey = entry.getKey();
      ModeResult result = entry.getValue();

      if (result.success() && result.result() != null) {
        var picksPerFaceData =
            result.result().metrics().storageSpaceMetrics().picksPerFaceData();

        if (picksPerFaceData != null && picksPerFaceData.timeline() != null) {
          timelineData.put(modeKey, picksPerFaceData.timeline());
        } else {
          timelineData.put(modeKey, List.of());
        }
      } else {
        timelineData.put(modeKey, List.of());
      }
    }

    return timelineData;
  }
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "feat: add timeline data extraction for comparison charts"
```

---

## Task 7: Update ComparisonService Return Value

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java:86-103`

**Step 1: Replace return statement**

Replace lines 87-103 (the Map.of return statement) with:

```java
    List<ComparisonMetric> comparisonMetrics = calculateComparisonMetrics();
    Map<String, Integer> winsPerMode = calculateWinsPerMode(comparisonMetrics);
    Map<String, List<?>> timelineData = extractTimelineData();

    return new ComparisonResponse(
        true,
        modeResults,
        comparisonMetrics,
        timelineData,
        winsPerMode,
        executionTimes,
        totalExecutionTime,
        LocalDateTime.now().format(DateTimeFormatter.ISO_DATE_TIME));
```

**Step 2: Clear old HashMaps**

Add these lines at the end of the `runComparison` method, just before the return statement:

```java
    // Clear state for next comparison
    results.clear();
    comparisonMetrics.clear();
    modeResults.clear();
```

**Step 3: Remove unused fields**

Delete lines 20-21 (the old `results` and `comparisonMetrics` fields):

```java
  private final Map<String, Object> results = new HashMap<>();
  private final Map<String, Object> comparisonMetrics = new HashMap<>();
```

**Step 4: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "feat: update ComparisonService to return ComparisonResponse DTO"
```

---

## Task 8: Update ComparisonController

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonController.java`

**Step 1: Update return type**

Change the `runComparison` method return type from:
```java
public ApiResponse<Map<String, Object>> runComparison(
```

To:
```java
public ApiResponse<ComparisonResponse> runComparison(
```

**Step 2: Update method body**

The method body stays the same - it already calls `comparisonService.runComparison()` and wraps in `ApiResponse.success()`.

**Step 3: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Restart server to regenerate OpenAPI spec**

```bash
npx nx serve backend
```

Wait for server to start (check logs for "Started DessimulatorApplication").

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonController.java
git commit -m "feat: update ComparisonController return type to ComparisonResponse"
```

---

## Task 9: Regenerate Frontend Types

**Files:**
- Modify: `apps/frontend/src/types/generated-api.ts` (auto-generated)

**Step 1: Ensure backend is running**

Run: `curl -s http://localhost:8080/health`
Expected: `{"status":"healthy"}`

**Step 2: Regenerate TypeScript types**

Run:
```bash
npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts
```

Expected: `✓ apps/frontend/src/types/generated-api.ts`

**Step 3: Verify new types exist**

Run:
```bash
grep -A 5 "ComparisonMetricType" apps/frontend/src/types/generated-api.ts
grep -A 10 "ComparisonMetric:" apps/frontend/src/types/generated-api.ts
grep -A 15 "ComparisonResponse:" apps/frontend/src/types/generated-api.ts
```

Expected: Should show the new enum and record types.

**Step 4: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "chore: regenerate frontend types with new comparison DTOs"
```

---

## Task 10: Fix Frontend Comparison Type

**Files:**
- Modify: `apps/frontend/src/pages/SimulatorPage.tsx:206`

**Step 1: Change comparison type**

Change line 206 from:
```typescript
comparisonType: 'order' as const, // Default to order comparison
```

To:
```typescript
comparisonType: 'put' as const, // Compare put algorithms (greedy vs min_weighted)
```

**Step 2: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/SimulatorPage.tsx
git commit -m "fix: change comparison type from 'order' to 'put'"
```

---

## Task 11: Update Button Text

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx:510-513`

**Step 1: Update button label**

Change line 512 from:
```typescript
Compare all algorithms
```

To:
```typescript
Compare Put Algorithms
```

**Step 2: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
git commit -m "feat: update comparison button text to 'Compare Put Algorithms'"
```

---

## Task 12: Simplify AlgorithmComparison Component - Part 1 (Interfaces)

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx:10-51`

**Step 1: Import generated types**

Add this import at the top (after existing imports):

```typescript
import type { components } from '../../../types/generated-api'
```

**Step 2: Replace interfaces with generated types**

Delete lines 10-31 (old interfaces) and replace with:

```typescript
type ComparisonMetric = components['schemas']['ComparisonMetric']
type ComparisonMetricType = components['schemas']['ComparisonMetricType']
type ComparisonResponse = components['schemas']['ComparisonResponse']
type ModeResult = components['schemas']['ModeResult']

interface AlgorithmComparisonProps {
  results: ComparisonResponse
  onClose: () => void
  onRunAnother: () => void
}
```

**Step 3: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: Errors expected (we'll fix in next steps)

**Step 4: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
git commit -m "refactor: use generated types for AlgorithmComparison interfaces"
```

---

## Task 13: Simplify AlgorithmComparison Component - Part 2 (Remove Tabs)

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx:44-565`

**Step 1: Remove tabs state and config**

Delete lines 44-51 (activeTab state and tabs array):

```typescript
  const [activeTab, setActiveTab] = useState('overview')

  const tabs = [
    { id: 'overview', label: 'Overview' },
    { id: 'picks', label: 'Picks' },
    { id: 'detailed', label: 'Detailed Analysis' },
    { id: 'recommendations', label: 'Recommendations' },
  ]
```

**Step 2: Remove tabs UI wrapper**

Delete the `<Tabs>` component wrapper (lines 196-533).

Keep only the content currently inside `<TabsContent value="overview">` (lines 213-363).

**Step 3: Update Dialog structure**

The component structure should now be:

```typescript
return (
  <Dialog open={true} onOpenChange={onClose}>
    <DialogContent className="max-w-7xl h-[90vh] flex flex-col bg-white border border-gray-200">
      <DialogHeader>
        <DialogTitle className="text-xl font-semibold text-gray-900">
          Algorithm Comparison Results
        </DialogTitle>
        <p className="text-sm text-gray-600">
          Performance comparison across put algorithms
        </p>
      </DialogHeader>

      <div className="flex-1 flex flex-col overflow-hidden">
        <div className="flex-1 bg-white overflow-y-auto m-0">
          <div className="p-6">
            {/* Overview content will go here */}
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="p-6 border-t border-gray-200 bg-gray-50">
        {/* Footer content */}
      </div>
    </DialogContent>
  </Dialog>
)
```

**Step 4: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: Errors expected (we'll fix data access next)

**Step 5: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
git commit -m "refactor: remove tabs from AlgorithmComparison, keep only overview"
```

---

## Task 14: Update Metric Data Access

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx:54-143`

**Step 1: Update data extraction**

Replace lines 54-95 (old data processing) with:

```typescript
  // Extract algorithm results (now using ModeResult type)
  const algorithmResults = Object.entries(results.results || {})
  const modeNames = algorithmResults.map(([_, result]) => result.modeInfo?.name || 'Unknown')

  // Use backend-calculated comparison metrics directly
  const comparisonMetrics = results.comparisonMetrics || []
```

**Step 2: Remove old metric processing**

Delete lines 97-143 (old comparisonMetrics array construction).

**Step 3: Update helper functions**

Keep `formatValue`, `getBestPerformer`, `getPerformanceIcon`, `getPerformanceColor`, and `calculatePercentageDiff` functions but they'll use backend data.

Actually, delete `getBestPerformer` - we now get `bestModeIndex` from backend.

**Step 4: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: Some errors remaining (we'll fix display logic next)

**Step 5: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
git commit -m "refactor: use backend-calculated comparison metrics"
```

---

## Task 15: Update Comparison Table Display

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx` (comparison table section)

**Step 1: Update algorithm summary cards**

Replace the algorithm summary cards section with:

```typescript
<div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-8">
  {algorithmResults.map(([key, result], index) => {
    // Get wins count from backend
    const wins = results.winsPerMode?.[key] || 0
    const totalMetrics = comparisonMetrics.length

    return (
      <Card
        key={key}
        className={`p-6 border-l-4 ${index === 0 ? 'border-orange-500' : 'border-blue-500'}`}
      >
        <div className="flex items-center justify-between mb-4">
          <h4 className="text-lg">{result.modeInfo?.name}</h4>
          <div className="flex items-center gap-2">
            <span className="text-lg">{result.modeInfo?.emoji}</span>
            <span
              className={`px-2 py-1 rounded text-xs font-medium ${
                wins === totalMetrics
                  ? 'bg-green-100 text-green-800'
                  : wins > 0
                    ? 'bg-yellow-100 text-yellow-800'
                    : 'bg-gray-100 text-gray-800'
              }`}
            >
              {wins === totalMetrics
                ? 'Recommended'
                : wins > 0
                  ? 'Competitive'
                  : 'Baseline'}
            </span>
          </div>
        </div>
        <div className="space-y-3">
          <div className="flex justify-between">
            <span className="text-sm text-gray-600">Wins</span>
            <span className={`text-sm font-medium ${wins > 0 ? 'text-green-600' : 'text-gray-600'}`}>
              {wins} / {totalMetrics} metrics
            </span>
          </div>
          <div className="flex justify-between">
            <span className="text-sm text-gray-600">Execution time</span>
            <span className="text-sm font-medium">
              {(results.executionTimes?.[key] || 0).toFixed(1)}s
            </span>
          </div>
          <div className="flex justify-between">
            <span className="text-sm text-gray-600">Status</span>
            <span className={`text-sm font-medium ${result.success ? 'text-green-600' : 'text-red-600'}`}>
              {result.success ? 'Completed' : 'Failed'}
            </span>
          </div>
        </div>
      </Card>
    )
  })}
</div>
```

**Step 2: Update comparison metrics table**

Replace the comparison table tbody with:

```typescript
<tbody className="divide-y divide-gray-100">
  {comparisonMetrics.map((metric, metricIndex) => {
    const bestIndex = metric.bestModeIndex || 0
    const modeKeys = Object.keys(results.results || {}).sort()

    return (
      <tr key={metricIndex} className="hover:bg-gray-50">
        <td className="py-4 px-4 text-sm text-gray-900 font-medium">
          {metric.metricName?.replace(/_/g, ' ').toLowerCase().replace(/\b\w/g, l => l.toUpperCase())}
        </td>
        {metric.values?.map((value, valueIndex) => (
          <td
            key={valueIndex}
            className={`py-4 px-4 text-sm text-center rounded-md mx-1 ${
              valueIndex === bestIndex ? 'bg-green-50 text-green-700' : ''
            }`}
          >
            <div className="flex items-center justify-center space-x-2">
              <span>
                {formatValue(value, metric.unit || '')} {metric.unit}
              </span>
              {valueIndex === bestIndex && <TrendingUp className="w-4 h-4 text-green-600" />}
            </div>
            {valueIndex !== bestIndex && metric.percentageDifferences && (
              <div className="text-xs text-gray-500 mt-1">
                {metric.percentageDifferences[valueIndex]?.toFixed(1)}%
              </div>
            )}
          </td>
        ))}
        <td className="py-4 px-4 text-sm text-center">
          <span className="px-2 py-1 rounded bg-green-100 text-green-700 font-medium text-xs">
            {modeKeys[bestIndex]}
          </span>
        </td>
      </tr>
    )
  })}
</tbody>
```

**Step 3: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: No errors

**Step 4: Test in browser**

Run: `npm run dev`
Navigate to simulator, click "Compare Put Algorithms"
Expected: Should show comparison table (timeline chart still missing)

**Step 5: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
git commit -m "refactor: update comparison display to use backend-calculated metrics"
```

---

## Task 16: Add Timeline Chart

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx`

**Step 1: Add timeline data processing**

After the comparison table section, add:

```typescript
{/* Timeline Chart Section */}
<div className="mt-8">
  <SectionHeader
    title="Picks per Face Timeline"
    subtexts={[
      'Performance over simulation time',
      'Compares batching efficiency trends',
      'Higher is better'
    ]}
  />

  {(() => {
    // Merge timeline data from both modes
    const timelineData = results.timelineData || {}
    const modeKeys = Object.keys(results.results || {}).sort()

    // Check if we have data
    const hasData = modeKeys.some(key =>
      timelineData[key] && Array.isArray(timelineData[key]) && timelineData[key].length > 0
    )

    if (!hasData) {
      return (
        <div className="bg-white border border-gray-200 rounded-lg p-12 text-center">
          <div className="text-gray-400 text-lg mb-2">📊 No Timeline Data</div>
          <p className="text-gray-500 text-sm">
            Enable snapshots in simulation config to see timeline charts
          </p>
        </div>
      )
    }

    // Get first mode's timeline as base structure
    const firstModeKey = modeKeys[0]
    const baseTimeline = timelineData[firstModeKey] || []

    // Merge data from all modes into single timeline array
    const mergedData = baseTimeline.map((point: any, index: number) => {
      const merged: any = {
        time: point.time,
        timeMinutes: point.timeMinutes || point.time / 60,
      }

      // Add data from each mode
      modeKeys.forEach(key => {
        const modeTimeline = timelineData[key] || []
        const modePoint = modeTimeline[index]
        if (modePoint) {
          merged[`${key}_ppf`] = modePoint.avgPicksPerFace || 0
          merged[`${key}_cumulative_ppf`] = modePoint.cumulativeAvgPicksPerFace || 0
        }
      })

      return merged
    })

    // Build metrics array for TimelineChart
    const chartMetrics = modeKeys.flatMap((key, index) => {
      const colors = ['#FF6600', '#3B82F6', '#10B981', '#F59E0B']
      return [
        {
          dataKey: `${key}_ppf`,
          name: `${results.results?.[key]?.modeInfo?.name} (Window)`,
          color: colors[index % colors.length],
          strokeWidth: 3,
        },
        {
          dataKey: `${key}_cumulative_ppf`,
          name: `${results.results?.[key]?.modeInfo?.name} (Overall)`,
          color: colors[index % colors.length],
          strokeWidth: 2,
          strokeDasharray: '5 5',
        },
      ]
    })

    return (
      <TimelineChart
        data={mergedData}
        metrics={chartMetrics}
        title="Picks per Face Comparison"
        subtitle="Compares batching efficiency between put algorithms over time"
        yAxisLabel="Picks per Face"
        height={450}
        formatYAxis={(value) => value.toFixed(1)}
        formatTooltipValue={(value, name) => [value.toFixed(2), name]}
      />
    )
  })()}
</div>
```

**Step 2: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: No errors

**Step 3: Test in browser**

Run: `npm run dev`
Click "Compare Put Algorithms"
Expected: Should show timeline chart if data exists

**Step 4: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
git commit -m "feat: add timeline chart for picks per face comparison"
```

---

## Task 17: Clean Up Unused Code

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx`

**Step 1: Remove unused imports**

Remove `BarChart3` from lucide-react imports (if not used elsewhere).

**Step 2: Remove old helper functions**

Remove `processTimelineData` function and any other unused data processing functions from the old implementation.

**Step 3: Remove debug console.logs**

Delete lines 63-95 (debug logging).

**Step 4: Update footer statistics**

Update footer to use new data structure:

```typescript
<div className="p-6 border-t border-gray-200 bg-gray-50">
  <div className="flex items-center justify-between">
    <div>
      <h4 className="text-sm font-medium text-gray-900">Comparison completed successfully</h4>
      <p className="text-xs text-gray-500">
        Compared {algorithmResults.length} put algorithms in{' '}
        {(results.totalExecutionTime || 0).toFixed(1)} seconds
      </p>
    </div>
    <div className="flex items-center space-x-3 ml-auto">
      <Button variant="outline" onClick={onClose}>
        Close
      </Button>
      <Button
        className="bg-orange-500 hover:bg-orange-600 text-white"
        onClick={() => {
          onRunAnother()
          onClose()
        }}
      >
        <Play className="w-4 h-4 mr-2" />
        Run New Comparison
      </Button>
    </div>
  </div>
</div>
```

**Step 5: Verify TypeScript compilation**

Run: `npm run typecheck`
Expected: No errors

**Step 6: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
git commit -m "refactor: clean up unused code and debug logs"
```

---

## Task 18: Format Code

**Files:**
- Modify: All modified Java and TypeScript files

**Step 1: Format Java code**

Run:
```bash
cd apps/backend
mvn spotless:apply
```

Expected: All Java files formatted according to Google Java Format

**Step 2: Format TypeScript code**

Run:
```bash
npx biome check --fix apps/frontend/src/pages/simulator/components/AlgorithmComparison.tsx
npx biome check --fix apps/frontend/src/pages/SimulatorPage.tsx
npx biome check --fix apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
```

Expected: All formatting issues fixed

**Step 3: Commit formatting changes**

```bash
git add -A
git commit -m "style: format Java and TypeScript code"
```

---

## Task 19: Integration Testing

**Files:**
- Test: Full comparison flow

**Step 1: Start backend**

Run:
```bash
npx nx serve backend
```

Wait for: `Started DessimulatorApplication` in logs

**Step 2: Start frontend**

Run:
```bash
npm run dev
```

Navigate to: `http://localhost:4200`

**Step 3: Run comparison**

1. Click "Compare Put Algorithms" button
2. Wait for loading modal
3. Verify comparison results modal appears with:
   - 2 algorithm cards (Greedy, Min Weighted)
   - Win counts displayed
   - Comparison table with 5 metrics
   - Green highlighting on best performers
   - Percentage differences shown
   - Timeline chart with 2 lines (if timeline data available)

**Step 4: Verify backend logs**

Check backend logs show:
```
Starting put comparison: Running 2 operation modes...
Running Greedy Put...
Greedy Put completed successfully in X.Xs
Running Min Weighted Cycle Distance...
Min Weighted Cycle Distance completed successfully in X.Xs
Comparison completed in X.Xs
```

**Step 5: Verify no errors**

- Browser console: No errors
- Backend logs: No exceptions
- Network tab: 200 OK response from `/comparison/run`

**Step 6: Document results**

Create test result summary in plan:
```
## Test Results

**Date:** 2025-11-17
**Status:** ⏳ MANUAL TESTING REQUIRED

**Backend:**
- Server status: Not running (confirmed via curl check)
- Backend port 8080: Available
- Server can be started via: `npx nx serve backend`

**Frontend:**
- Server status: Not running
- Frontend port 4200: Can be started via `npm run dev`
- TypeScript compilation: All formatting complete, warnings acceptable

**Testing Instructions for User:**
1. Start backend: `npx nx serve backend` (wait for "Started DessimulatorApplication")
2. Start frontend: `npm run dev` (navigate to http://localhost:4200)
3. Click "Compare Put Algorithms" button
4. Verify:
   - Loading modal appears
   - Comparison results modal shows 2 algorithm cards
   - Metrics table displays 5 comparison metrics
   - Best performer has green highlighting
   - Percentage differences shown for non-winners
   - Timeline chart visible (if snapshot data enabled)
   - Backend logs show successful execution

**Note:** Manual browser testing required by user to complete verification. All code formatting and type-safety updates have been completed.
```

---

## Task 20: Update Documentation

**Files:**
- Modify: `apps/frontend/README.md` or `CLAUDE.md`

**Step 1: Document comparison feature**

Add section to CLAUDE.md:

```markdown
## Algorithm Comparison

**Feature**: Compare put algorithms (Greedy vs Min Weighted Cycle Distance) with backend-calculated metrics.

**Usage**:
1. Configure simulation parameters
2. Click "Compare Put Algorithms"
3. View side-by-side performance comparison

**Metrics Compared**:
- Orders completed
- Total picks processed
- Picks per hour per station
- Operator utilization
- Picks per face

**Backend** (`ComparisonService.java`):
- Runs 2 simulations with different `putAlgorithm` values
- Extracts metrics from nested paths
- Calculates best performer and percentage differences
- Returns type-safe `ComparisonResponse` DTO

**Frontend** (`AlgorithmComparison.tsx`):
- Displays pre-calculated metrics (no computation)
- Shows timeline chart for picks per face
- Highlights best performer with green styling

**Adding Metrics**:
1. Add to `ComparisonMetricType` enum
2. Add extractor in `calculateComparisonMetrics()`
3. Regenerate frontend types
4. Display appears automatically in table
```

**Step 2: Commit documentation**

```bash
git add CLAUDE.md
git commit -m "docs: add algorithm comparison feature documentation"
```

---

## Summary

**Total Tasks:** 20
**Estimated Time:** 2-3 hours
**Tech Stack:** Java 21, Spring Boot, TypeScript, React, OpenAPI

**Key Changes:**
- ✅ Type-safe metric comparisons with enums
- ✅ Backend-calculated analytics (best performer, percentages)
- ✅ Simplified frontend (no computation logic)
- ✅ Timeline chart for visual comparison
- ✅ Fixed comparison type (put algorithms)

**Verification Steps:**
1. All tests pass
2. TypeScript compiles without errors
3. Comparison runs successfully
4. Results display correctly
5. No console errors

**Follow-Up Tasks** (Future):
- Add order selection algorithm comparison
- Add configurable comparison modes (UI selector)
- Add more metrics (cycle time, wait time)
- Export comparison results to CSV
