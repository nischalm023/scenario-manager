# Robot Results Page Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a "Robots" tab to simulation results showing robot performance metrics with timeline and bar charts.

**Architecture:** Enhance RobotCollector to track cycle time log and compute timeline with 10-minute sliding window (like PicksPerFace). Add new DTOs for timeline points and per-robot summaries. Create RobotResults.tsx component with MetricCards, timeline chart, and bar charts.

**Tech Stack:** Java 21 (records, Lombok), Spring Boot, React, TypeScript, Recharts

---

## Task 1: Add RobotCycleTimelinePoint DTO

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/RobotMetrics.java:22-26`

**Step 1: Add timeline point record after RobotMetric**

Add these new records to `RobotMetrics.java` after the existing `RobotMetric` record (around line 27):

```java
  /**
   * Timeline point for robot cycle time tracking.
   *
   * <p>Uses 10-minute rolling window for windowed average, similar to PicksPerFaceTimelinePoint.
   */
  @Builder
  public record RobotCycleTimelinePoint(
      double time,
      double timeMinutes,
      double avgCycleTime,
      double cumulativeAvgCycleTime,
      int windowCycles,
      int totalCycles) {}

  /**
   * Per-robot summary for bar charts.
   *
   * <p>Contains aggregated metrics for individual robot performance visualization.
   */
  @Builder
  public record PerRobotSummary(
      int robotId,
      String robotType,
      int cycleCount,
      double totalCycleTime,
      double avgCycleTime,
      double utilization) {}
```

**Step 2: Update RobotMetric to include new fields**

Replace the existing `RobotMetric` record with:

```java
  /** Robot performance metrics. */
  @Builder
  public record RobotMetric(
      Map<Integer, Map<String, Object>> perRobotStats,
      Map<String, Map<String, Object>> byTypeStats,
      double overallRobotUtilization,
      List<RobotCycleTimelinePoint> cycleTimeTimeline,
      List<PerRobotSummary> perRobotSummaries) {}
```

**Step 3: Add List import if not present**

Ensure this import exists at the top of the file:

```java
import java.util.List;
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/RobotMetrics.java
git commit -m "feat: add RobotCycleTimelinePoint and PerRobotSummary DTOs

New records for robot results page:
- RobotCycleTimelinePoint: timeline with windowed/cumulative averages
- PerRobotSummary: per-robot stats for bar charts
- Updated RobotMetric to include new fields

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: Add Cycle Log Tracking to RobotCollector

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:127-218`

**Step 1: Add cycle log entry record and list to RobotCollector**

Add these fields after the existing field declarations (around line 131):

```java
    private final List<RobotCycleLogEntry> cycleLog = new ArrayList<>();

    /** Internal record for raw cycle data. */
    private record RobotCycleLogEntry(
        double time, int robotId, String robotType, double cycleTime) {}
```

**Step 2: Update handleRobotCycleCompleted to populate cycle log**

Find the `handleRobotCycleCompleted` method and add this after the existing tracking logic (after line 166, before the log.trace):

```java
        // Add to cycle log for timeline computation
        cycleLog.add(new RobotCycleLogEntry(event.getTimestamp(), robotId, robotType, cycleTime));
```

The complete method should now include:

```java
    @MetricHandler({
      MetricEvent.RACK_CYCLE_COMPLETED,
      MetricEvent.HTM_CYCLE_COMPLETED,
      MetricEvent.VTM_CYCLE_COMPLETED
    })
    public void handleRobotCycleCompleted(MetricEventData event) {
      recordEvent(event);
      Integer robotId = event.getRobotId();
      if (robotId != null && event.getData() != null) {
        Map<String, Object> stats = perRobotStats.computeIfAbsent(robotId, k -> new HashMap<>());

        stats.merge("cycle_count", 1, (a, b) -> (Integer) a + (Integer) b);

        Object cycleTimeObj = event.getData().get("cycle_time");
        double cycleTime = 0.0;
        if (cycleTimeObj instanceof Number) {
          cycleTime = ((Number) cycleTimeObj).doubleValue();
          stats.merge("total_cycle_time", cycleTime, (a, b) -> (Double) a + (Double) b);
        }

        String robotType =
            event.getEventType() == MetricEvent.RACK_CYCLE_COMPLETED
                ? "RTP"
                : event.getEventType() == MetricEvent.HTM_CYCLE_COMPLETED ? "HTM" : "VTM";
        stats.put("type", robotType);

        Map<String, Object> typeStats =
            byTypeStats.computeIfAbsent(robotType, k -> new HashMap<>());
        typeStats.merge("cycle_count", 1, (a, b) -> (Integer) a + (Integer) b);
        if (cycleTimeObj instanceof Number) {
          typeStats.merge("total_cycle_time", cycleTime, (a, b) -> (Double) a + (Double) b);
        }

        // Add to cycle log for timeline computation
        cycleLog.add(new RobotCycleLogEntry(event.getTimestamp(), robotId, robotType, cycleTime));
      }
      log.trace("Robot {} cycle completed at t={}", robotId, event.getTimestamp());
    }
```

**Step 3: Add import for RobotMetrics types**

Add this import at the top of the file:

```java
import com.greyorange.dessimulator.simulator.metrics.RobotMetrics.*;
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat: add cycle log tracking to RobotCollector

Track individual cycle completions for timeline computation:
- RobotCycleLogEntry record with time, robotId, type, cycleTime
- Populate cycleLog in handleRobotCycleCompleted

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: Add Timeline Computation to RobotCollector

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:127-218`

**Step 1: Add TIME_WINDOW_SECONDS constant**

Add this constant at the beginning of the RobotCollector class (after line 129):

```java
    private static final double TIME_WINDOW_SECONDS = 600.0; // 10-minute rolling window
```

**Step 2: Add computeCycleTimeTimeline method**

Add this method after the `handleSimulationCompleted` method (around line 191):

```java
    /**
     * Compute cycle time timeline with sliding window average.
     *
     * <p>Uses 10-minute rolling window for windowed average and maintains cumulative average. O(n)
     * complexity with sliding window approach.
     *
     * @return List of timeline points
     */
    private List<RobotCycleTimelinePoint> computeCycleTimeTimeline() {
      if (cycleLog.isEmpty()) {
        return List.of();
      }

      List<RobotCycleTimelinePoint> timeline = new ArrayList<>();

      // Cumulative tracking
      int cumulativeCycles = 0;
      double cumulativeCycleTime = 0.0;

      // Sliding window tracking
      int windowStartIdx = 0;
      int windowCycleCount = 0;
      double windowCycleTime = 0.0;

      for (int i = 0; i < cycleLog.size(); i++) {
        RobotCycleLogEntry entry = cycleLog.get(i);
        double currentTime = entry.time();

        // Update cumulative metrics
        cumulativeCycles++;
        cumulativeCycleTime += entry.cycleTime();
        double cumulativeAvg = cumulativeCycleTime / cumulativeCycles;

        // Slide window start forward to maintain 10-minute window
        while (windowStartIdx < i
            && cycleLog.get(windowStartIdx).time() < currentTime - TIME_WINDOW_SECONDS) {
          windowCycleCount--;
          windowCycleTime -= cycleLog.get(windowStartIdx).cycleTime();
          windowStartIdx++;
        }

        // Add current entry to window
        windowCycleCount++;
        windowCycleTime += entry.cycleTime();

        double windowAvg = windowCycleCount > 0 ? windowCycleTime / windowCycleCount : 0.0;

        RobotCycleTimelinePoint point =
            RobotCycleTimelinePoint.builder()
                .time(currentTime)
                .timeMinutes(currentTime / 60.0)
                .avgCycleTime(windowAvg)
                .cumulativeAvgCycleTime(cumulativeAvg)
                .windowCycles(windowCycleCount)
                .totalCycles(cumulativeCycles)
                .build();

        timeline.add(point);
      }

      return timeline;
    }
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat: add computeCycleTimeTimeline to RobotCollector

O(n) sliding window algorithm for cycle time timeline:
- 10-minute rolling window average
- Cumulative average across all cycles
- Same pattern as PicksPerFace timeline

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: Add Per-Robot Summaries to RobotCollector

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:127-218`

**Step 1: Add buildPerRobotSummaries method**

Add this method after the `computeCycleTimeTimeline` method:

```java
    /**
     * Build per-robot summaries for bar charts.
     *
     * @return List of per-robot summaries sorted by robot ID
     */
    private List<PerRobotSummary> buildPerRobotSummaries() {
      double simulationDuration = simulationEndTime - simulationStartTime;

      List<PerRobotSummary> summaries = new ArrayList<>();

      for (Map.Entry<Integer, Map<String, Object>> entry : perRobotStats.entrySet()) {
        int robotId = entry.getKey();
        Map<String, Object> stats = entry.getValue();

        int cycleCount =
            stats.get("cycle_count") instanceof Number
                ? ((Number) stats.get("cycle_count")).intValue()
                : 0;

        double totalCycleTime =
            stats.get("total_cycle_time") instanceof Number
                ? ((Number) stats.get("total_cycle_time")).doubleValue()
                : 0.0;

        String robotType = stats.get("type") instanceof String ? (String) stats.get("type") : "RTP";

        double avgCycleTime = cycleCount > 0 ? totalCycleTime / cycleCount : 0.0;
        double utilization = simulationDuration > 0 ? totalCycleTime / simulationDuration : 0.0;

        PerRobotSummary summary =
            PerRobotSummary.builder()
                .robotId(robotId)
                .robotType(robotType)
                .cycleCount(cycleCount)
                .totalCycleTime(totalCycleTime)
                .avgCycleTime(avgCycleTime)
                .utilization(utilization)
                .build();

        summaries.add(summary);
      }

      // Sort by robot ID for consistent ordering
      summaries.sort((a, b) -> Integer.compare(a.robotId(), b.robotId()));

      return summaries;
    }
```

**Step 2: Add sampleTimeline helper method**

Add this method after `buildPerRobotSummaries`:

```java
    /**
     * Sample a timeline list to reduce response size while preserving shape for plotting.
     *
     * @param timeline Full timeline list
     * @param maxPoints Maximum points to return
     * @return Sampled timeline (evenly spaced or all if under limit)
     */
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
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat: add buildPerRobotSummaries to RobotCollector

Build per-robot summaries for bar charts:
- Calculate avgCycleTime and utilization per robot
- Sort by robotId for consistent ordering
- Add sampleTimeline helper for response size control

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: Update RobotCollector.getMetrics()

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:193-217`

**Step 1: Replace getMetrics method**

Replace the existing `getMetrics()` method with:

```java
    public RobotMetric getMetrics() {
      double simulationDuration = simulationEndTime - simulationStartTime;
      double totalActiveCycleTime =
          perRobotStats.values().stream()
              .mapToDouble(
                  stats -> {
                    Object totalCycleTimeObj = stats.get("total_cycle_time");
                    return totalCycleTimeObj instanceof Number
                        ? ((Number) totalCycleTimeObj).doubleValue()
                        : 0.0;
                  })
              .sum();

      int totalRobots = perRobotStats.size();
      double overallRobotUtilization =
          (simulationDuration > 0 && totalRobots > 0)
              ? totalActiveCycleTime / (simulationDuration * totalRobots)
              : 0.0;

      // Compute timeline and summaries (lazy computation)
      List<RobotCycleTimelinePoint> timeline = computeCycleTimeTimeline();
      List<PerRobotSummary> summaries = buildPerRobotSummaries();

      // Sample timeline to max 200 points for response size control
      List<RobotCycleTimelinePoint> sampledTimeline = sampleTimeline(timeline, 200);

      log.debug(
          "Robot metrics computed - {} robots, {} cycles, {} timeline points (sampled to {})",
          totalRobots,
          cycleLog.size(),
          timeline.size(),
          sampledTimeline.size());

      return RobotMetric.builder()
          .perRobotStats(Map.copyOf(perRobotStats))
          .byTypeStats(Map.copyOf(byTypeStats))
          .overallRobotUtilization(overallRobotUtilization)
          .cycleTimeTimeline(sampledTimeline)
          .perRobotSummaries(summaries)
          .build();
    }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat: update RobotCollector.getMetrics with timeline and summaries

Enhanced getMetrics to include:
- Computed cycleTimeTimeline (sampled to 200 points)
- Per-robot summaries for bar charts
- Lazy computation pattern for performance

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: Format Java Code

**Files:**
- All modified Java files

**Step 1: Run Spotless formatter**

Run: `cd apps/backend && mvn spotless:apply`
Expected: BUILD SUCCESS with formatting applied

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit formatting changes (if any)**

```bash
git add apps/backend/
git commit -m "style: format Java code with Spotless

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 7: Start Backend and Regenerate Frontend Types

**Files:**
- Modify: `apps/frontend/src/types/generated-api.ts`

**Step 1: Start the backend server**

Run: `npx nx serve backend`
Expected: Server starts on port 8080

**Step 2: Wait for server to be ready**

Run: `curl -s http://localhost:8080/health`
Expected: `{"status":"UP",...}`

**Step 3: Regenerate frontend types**

Run: `npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts`
Expected: File generated with new RobotCycleTimelinePoint and PerRobotSummary types

**Step 4: Verify new types exist**

Check that `generated-api.ts` contains:
- `RobotCycleTimelinePoint`
- `PerRobotSummary`
- Updated `RobotMetric` with `cycleTimeTimeline` and `perRobotSummaries`

**Step 5: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "chore: regenerate frontend types with robot metrics

New types for robot results page:
- RobotCycleTimelinePoint
- PerRobotSummary
- Updated RobotMetric

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 8: Create BarChart Component

**Files:**
- Create: `apps/frontend/src/components/common/charts/BarChart.tsx`

**Step 1: Create the BarChart component**

```tsx
import type React from 'react'
import {
  Bar,
  BarChart as RechartsBarChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

interface BarChartDataPoint {
  name: string
  value: number
  color?: string
  [key: string]: any
}

interface BarChartProps {
  data: BarChartDataPoint[]
  dataKey?: string
  title?: string
  subtitle?: string
  xAxisLabel?: string
  yAxisLabel: string
  height?: number
  className?: string
  barColor?: string
  formatYAxis?: (value: number) => string
  formatTooltipValue?: (value: number) => string
}

const BarChart: React.FC<BarChartProps> = ({
  data,
  dataKey = 'value',
  title,
  subtitle,
  xAxisLabel,
  yAxisLabel,
  height = 300,
  className = '',
  barColor = '#FF6600',
  formatYAxis,
  formatTooltipValue,
}) => {
  const CustomTooltip = ({ active, payload, label }: any) => {
    if (active && payload && payload.length) {
      const value = payload[0].value
      return (
        <div className="bg-white p-3 border border-gray-200 rounded-lg shadow-lg">
          <p className="font-medium text-gray-900">{label}</p>
          <p style={{ color: barColor }} className="text-sm">
            {formatTooltipValue ? formatTooltipValue(value) : value.toFixed(2)}
          </p>
        </div>
      )
    }
    return null
  }

  return (
    <div className={`bg-white border border-gray-200 rounded-lg p-6 ${className}`}>
      {(title || subtitle) && (
        <div className="mb-4">
          {title && <h3 className="text-lg font-medium text-gray-900">{title}</h3>}
          {subtitle && <p className="text-sm text-gray-500 mt-1">{subtitle}</p>}
        </div>
      )}

      <ResponsiveContainer width="100%" height={height}>
        <RechartsBarChart
          data={data}
          margin={{
            top: 20,
            right: 30,
            left: 20,
            bottom: 60,
          }}
        >
          <CartesianGrid strokeDasharray="3 3" stroke="#E5E7EB" />
          <XAxis
            dataKey="name"
            tick={{ fontSize: 12 }}
            stroke="#6B7280"
            label={xAxisLabel ? { value: xAxisLabel, position: 'insideBottom', offset: -10 } : undefined}
            interval={0}
            angle={-45}
            textAnchor="end"
            height={60}
          />
          <YAxis
            tick={{ fontSize: 12 }}
            stroke="#6B7280"
            label={{ value: yAxisLabel, angle: -90, position: 'insideLeft' }}
            tickFormatter={formatYAxis}
          />
          <Tooltip content={<CustomTooltip />} />
          <Bar dataKey={dataKey} fill={barColor} radius={[4, 4, 0, 0]} />
        </RechartsBarChart>
      </ResponsiveContainer>
    </div>
  )
}

export default BarChart
```

**Step 2: Verify file exists**

Run: `ls apps/frontend/src/components/common/charts/BarChart.tsx`
Expected: File exists

**Step 3: Commit**

```bash
git add apps/frontend/src/components/common/charts/BarChart.tsx
git commit -m "feat: add BarChart component for robot metrics

Reusable bar chart using Recharts:
- Customizable colors, labels, formatters
- Responsive container
- Rotated X-axis labels for robot IDs

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 9: Create RobotResults Component

**Files:**
- Create: `apps/frontend/src/pages/simulator/components/RobotResults.tsx`

**Step 1: Create the RobotResults component**

```tsx
import type React from 'react'
import BarChart from '../../../components/common/charts/BarChart'
import TimelineChart from '../../../components/common/charts/TimelineChart'
import MetricCard from '../../../components/common/display/MetricCard'
import SectionHeader from '../../../components/common/display/SectionHeader'
import type { SimulationResults } from '../../../types/generated'

interface RobotResultsProps {
  results: SimulationResults
  formatNumber: (num: number) => string
}

const RobotResults: React.FC<RobotResultsProps> = ({ results, formatNumber }) => {
  const { metrics, config } = results
  const robotMetrics = metrics.robotMetrics

  // Determine system type for conditional display
  const isRelay = config?.systemType === 'relay'

  // Get timeline data
  const cycleTimeTimeline = robotMetrics.cycleTimeTimeline || []

  // Get per-robot summaries
  const perRobotSummaries = robotMetrics.perRobotSummaries || []

  // Filter summaries by system type
  const filteredSummaries = isRelay
    ? perRobotSummaries.filter((s) => s.robotType === 'VTM' || s.robotType === 'HTM')
    : perRobotSummaries.filter((s) => s.robotType === 'RTP')

  // Group by robot type for relay system
  const vtmSummaries = perRobotSummaries.filter((s) => s.robotType === 'VTM')
  const htmSummaries = perRobotSummaries.filter((s) => s.robotType === 'HTM')
  const rtpSummaries = perRobotSummaries.filter((s) => s.robotType === 'RTP')

  // Calculate aggregate metrics
  const totalRobots = filteredSummaries.length
  const avgUtilization =
    filteredSummaries.length > 0
      ? filteredSummaries.reduce((sum, s) => sum + s.utilization, 0) / filteredSummaries.length
      : 0
  const avgCycleTime =
    filteredSummaries.length > 0
      ? filteredSummaries.reduce((sum, s) => sum + s.avgCycleTime, 0) / filteredSummaries.length
      : 0

  // Convert timeline data for chart
  const processedTimelineData = cycleTimeTimeline.map((point) => ({
    time_minutes: point.timeMinutes,
    avg_cycle_time: point.avgCycleTime,
    cumulative_avg_cycle_time: point.cumulativeAvgCycleTime,
    window_cycles: point.windowCycles,
    total_cycles: point.totalCycles,
  }))

  // Convert summaries to bar chart format
  const cycleTimeBarData = filteredSummaries.map((s) => ({
    name: `${s.robotType} ${s.robotId}`,
    value: s.avgCycleTime,
  }))

  const utilizationBarData = filteredSummaries.map((s) => ({
    name: `${s.robotType} ${s.robotId}`,
    value: s.utilization * 100, // Convert to percentage
  }))

  // For relay: separate VTM and HTM bar data
  const vtmCycleTimeData = vtmSummaries.map((s) => ({
    name: `VTM ${s.robotId}`,
    value: s.avgCycleTime,
  }))

  const htmCycleTimeData = htmSummaries.map((s) => ({
    name: `HTM ${s.robotId}`,
    value: s.avgCycleTime,
  }))

  const vtmUtilizationData = vtmSummaries.map((s) => ({
    name: `VTM ${s.robotId}`,
    value: s.utilization * 100,
  }))

  const htmUtilizationData = htmSummaries.map((s) => ({
    name: `HTM ${s.robotId}`,
    value: s.utilization * 100,
  }))

  return (
    <div className="p-6">
      <div className="space-y-8">
        {/* Performance Metrics */}
        <div>
          <SectionHeader title="Robot Performance" />

          <div className="grid grid-cols-3 gap-4">
            <MetricCard
              title="Total Robots"
              value={formatNumber(totalRobots)}
              subtitle={isRelay ? `${vtmSummaries.length} VTM, ${htmSummaries.length} HTM` : 'RTP'}
              status="neutral"
            />

            <MetricCard
              title="Avg Utilization"
              value={`${(avgUtilization * 100).toFixed(1)}%`}
              status="neutral"
            />

            <MetricCard
              title="Avg Cycle Time"
              value={`${avgCycleTime.toFixed(1)}s`}
              status="neutral"
            />
          </div>
        </div>

        {/* Cycle Time Timeline */}
        <div>
          <SectionHeader title="Cycle Time Over Time" />

          {processedTimelineData.length > 0 ? (
            <TimelineChart
              data={processedTimelineData as never}
              metrics={[
                {
                  dataKey: 'avg_cycle_time',
                  name: 'Window Average',
                  color: '#3B82F6',
                  strokeWidth: 3,
                },
                {
                  dataKey: 'cumulative_avg_cycle_time',
                  name: 'Overall Average',
                  color: '#6B7280',
                  strokeWidth: 2,
                  strokeDasharray: '5 5',
                },
              ]}
              yAxisLabel="Cycle Time (seconds)"
              height={400}
              formatYAxis={(value) => `${value.toFixed(0)}s`}
              formatTooltipValue={(value, name) => [`${value.toFixed(1)}s`, name]}
            />
          ) : (
            <div className="bg-white border border-gray-200 rounded-lg p-12 text-center">
              <div className="text-gray-400 text-lg mb-2">No Timeline Data</div>
              <p className="text-gray-500 text-sm">
                No cycle time timeline data available for this simulation
              </p>
            </div>
          )}
        </div>

        {/* Bar Charts - different layout for RTP vs Relay */}
        {isRelay ? (
          <>
            {/* VTM Section */}
            {vtmSummaries.length > 0 && (
              <div>
                <SectionHeader title="VTM Robot Performance" />

                <div className="grid grid-cols-2 gap-6">
                  <BarChart
                    data={vtmCycleTimeData}
                    yAxisLabel="Cycle Time (seconds)"
                    height={300}
                    barColor="#3B82F6"
                    formatYAxis={(value) => `${value.toFixed(0)}s`}
                    formatTooltipValue={(value) => `${value.toFixed(1)}s`}
                  />

                  <BarChart
                    data={vtmUtilizationData}
                    yAxisLabel="Utilization (%)"
                    height={300}
                    barColor="#10B981"
                    formatYAxis={(value) => `${value.toFixed(0)}%`}
                    formatTooltipValue={(value) => `${value.toFixed(1)}%`}
                  />
                </div>
              </div>
            )}

            {/* HTM Section */}
            {htmSummaries.length > 0 && (
              <div>
                <SectionHeader title="HTM Robot Performance" />

                <div className="grid grid-cols-2 gap-6">
                  <BarChart
                    data={htmCycleTimeData}
                    yAxisLabel="Cycle Time (seconds)"
                    height={300}
                    barColor="#8B5CF6"
                    formatYAxis={(value) => `${value.toFixed(0)}s`}
                    formatTooltipValue={(value) => `${value.toFixed(1)}s`}
                  />

                  <BarChart
                    data={htmUtilizationData}
                    yAxisLabel="Utilization (%)"
                    height={300}
                    barColor="#F59E0B"
                    formatYAxis={(value) => `${value.toFixed(0)}%`}
                    formatTooltipValue={(value) => `${value.toFixed(1)}%`}
                  />
                </div>
              </div>
            )}
          </>
        ) : (
          /* RTP Section */
          <div>
            <SectionHeader title="RTP Robot Performance" />

            <div className="grid grid-cols-2 gap-6">
              <BarChart
                data={cycleTimeBarData}
                yAxisLabel="Cycle Time (seconds)"
                height={300}
                barColor="#FF6600"
                formatYAxis={(value) => `${value.toFixed(0)}s`}
                formatTooltipValue={(value) => `${value.toFixed(1)}s`}
              />

              <BarChart
                data={utilizationBarData}
                yAxisLabel="Utilization (%)"
                height={300}
                barColor="#10B981"
                formatYAxis={(value) => `${value.toFixed(0)}%`}
                formatTooltipValue={(value) => `${value.toFixed(1)}%`}
              />
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

export default RobotResults
```

**Step 2: Verify file exists**

Run: `ls apps/frontend/src/pages/simulator/components/RobotResults.tsx`
Expected: File exists

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/RobotResults.tsx
git commit -m "feat: add RobotResults component

Robot results page with:
- MetricCards for total robots, avg utilization, avg cycle time
- Timeline chart for cycle time over time
- Bar charts for per-robot cycle time and utilization
- Separate sections for VTM/HTM in relay system

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 10: Add Robots Tab to SimulationResults

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/SimulationResults.tsx:14,110-116,175-177`

**Step 1: Add import for RobotResults**

Add after the other results imports (around line 16):

```tsx
import RobotResults from './RobotResults'
```

**Step 2: Add Robots tab to tabs array**

Update the `tabs` array (around line 110) to add the Robots tab after Pick:

```tsx
  const tabs = [
    { id: 'overview', label: 'Overview' },
    { id: 'put', label: 'Put' },
    { id: 'picks', label: 'Pick' },
    { id: 'robots', label: 'Robots' },
    ...(snapshotCount > 0 ? [{ id: 'animation', label: 'Animation' }] : []),
    ...(analysisState.hasRequested ? [{ id: 'ai-analysis', label: 'AI Analysis' }] : []),
  ]
```

**Step 3: Add Robots TabsContent**

Add this after the picks TabsContent (around line 177):

```tsx
            <TabsContent value="robots" className="flex-1 bg-white overflow-y-auto m-0 min-h-0">
              <RobotResults results={results} formatNumber={formatNumber} />
            </TabsContent>
```

**Step 4: Run type check**

Run: `npm run check --prefix apps/frontend`
Expected: No errors

**Step 5: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/SimulationResults.tsx
git commit -m "feat: add Robots tab to simulation results

Integrate RobotResults component:
- Add Robots tab after Pick in results modal
- Pass results and formatNumber props

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 11: Test End-to-End

**Files:**
- None (testing only)

**Step 1: Start backend if not running**

Run: `npx nx serve backend`
Expected: Server running on port 8080

**Step 2: Start frontend**

Run: `npm run dev --prefix apps/frontend`
Expected: Frontend running on port 4200

**Step 3: Run RTP simulation**

1. Open http://localhost:4200
2. Select RTP system type
3. Configure 4 robots, 2 hour simulation
4. Click "Run Simulation"
5. When complete, click "Robots" tab

Expected:
- 3 MetricCards showing total robots (4), avg utilization, avg cycle time
- Timeline chart with windowed and cumulative cycle time
- Bar chart showing cycle time per robot (Robot 0, Robot 1, etc.)
- Bar chart showing utilization per robot

**Step 4: Run Relay simulation**

1. Close results
2. Select Relay system type
3. Configure 4 VTM, 4 HTM, 2 hour simulation
4. Click "Run Simulation"
5. When complete, click "Robots" tab

Expected:
- MetricCards showing total robots (8), subtitle "4 VTM, 4 HTM"
- Timeline chart with cycle time data
- VTM section with cycle time and utilization bar charts
- HTM section with cycle time and utilization bar charts (different colors)

**Step 5: Final commit**

```bash
git add -A
git commit -m "feat: complete robot results page implementation

End-to-end tested:
- RTP system with aggregated robot metrics
- Relay system with separate VTM/HTM sections
- Timeline and bar charts rendering correctly

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Summary

**Total tasks:** 11

**Files created:**
- `apps/frontend/src/components/common/charts/BarChart.tsx`
- `apps/frontend/src/pages/simulator/components/RobotResults.tsx`

**Files modified:**
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/RobotMetrics.java`
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java`
- `apps/frontend/src/types/generated-api.ts`
- `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

**Key patterns used:**
- Same 10-minute sliding window algorithm as PicksPerFace
- Lazy computation in getMetrics()
- Timeline sampling to max 200 points
- System type filtering (RTP vs VTM/HTM)
