# Robot Travel Time Metrics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add average travel time tracking alongside cycle time for all robot types (RTP, VTM, HTM) with full parity including timelines and bar charts.

**Architecture:** Extend existing RobotCollector to track travel time from cycle events. VTM events need modification to include travel time data. Frontend displays travel time in metric cards, timeline charts, and bar charts.

**Tech Stack:** Java 21, Spring Boot, Lombok, React, TypeScript, Recharts

---

## CRITICAL BUG FIX: navFactor not applied to emitted travel time

**Bug:** All three processes (RTP, VTM, HTM) apply `navFactor` to the simulation hold time but emit raw travel time in metrics. This causes metrics to report incorrect values.

**Example (HTM):**
```java
// Simulation uses navFactor
hold(new TimeSpan(travelToRelay * navFactor, TimeUnit.SECONDS));

// But metrics emit raw value (BUG!)
double totalTravelTime = travelToRelay + travelToStation + returnTravel;
```

**Fix:** Multiply travel time by `navFactor` before emitting in metrics events.

---

## Task 1: Fix navFactor bug in HTM process

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java:374`

**Step 1: Apply navFactor to emitted travel time**

Find line 374 and update the calculation:

```java
          // Calculate metrics for event - APPLY navFactor!
          double totalTravelTime = (travelToRelay + travelToStation + returnTravel) * navFactor;
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

---

## Task 2: Fix navFactor bug in RTP process

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java`

**Step 1: Fix returnRackToStorage travel time (around line 528)**

Find `double travelTime = pathToStorage.totalTime();` and update:

```java
    // Calculate metrics - APPLY navFactor!
    double travelTime = pathToStorage.totalTime() * navFactor;
```

**Step 2: Fix TransportMetrics travel time**

The `TransportMetrics.travelTime()` values are aggregated in `emitRackCycleCompleted`. We need to apply navFactor there.

Find the aggregation loop (around line 647-648):

```java
    // Aggregate transport metrics (outbound only, exclude return)
    for (TransportMetrics tm : transportMetrics) {
      travelTime += tm.travelTime();
    }
```

Change to:

```java
    // Aggregate transport metrics (outbound only, exclude return) - APPLY navFactor!
    for (TransportMetrics tm : transportMetrics) {
      travelTime += tm.travelTime() * navFactor;
    }
```

**Step 3: Also apply navFactor to return metrics (around line 652-653)**

```java
    // Add return travel time - APPLY navFactor!
    if (returnMetrics != null) {
      travelTime += returnMetrics.travelTime() * navFactor;
    }
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 5: Commit bug fixes**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java \
        apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java
git commit -m "fix: apply navFactor to travel time in metrics events

HTM and RTP processes were emitting raw travel time without navFactor,
causing metrics to report incorrect values that don't match simulation."
```

---

## Task 3: Add travelTime to VtmCycleCompleted event

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/RobotEvents.java:131-164`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/DomainEvent.java:112-121`

**Step 1: Update VtmCycleCompleted record**

In `RobotEvents.java`, add `travelTime` parameter to the record:

```java
record VtmCycleCompleted(
    double timestamp,
    int robotId,
    String toteId,
    double cycleStartTime,
    double cycleTime,
    int rackId,
    int relayPositionId,
    double travelTime)  // NEW
    implements RobotEvent {

  public VtmCycleCompleted {
    if (timestamp < 0) {
      throw new IllegalArgumentException("timestamp must be >= 0, got: " + timestamp);
    }
    if (robotId < 0) {
      throw new IllegalArgumentException("robotId must be >= 0, got: " + robotId);
    }
    if (cycleTime < 0) {
      throw new IllegalArgumentException("cycleTime must be >= 0, got: " + cycleTime);
    }
    if (rackId < 0) {
      throw new IllegalArgumentException("rackId must be >= 0, got: " + rackId);
    }
    if (relayPositionId < 0) {
      throw new IllegalArgumentException("relayPositionId must be >= 0, got: " + relayPositionId);
    }
    if (travelTime < 0) {
      throw new IllegalArgumentException("travelTime must be >= 0, got: " + travelTime);
    }
    // Let NPE happen for null toteId
  }

  @Override
  public MetricEvent eventType() {
    return MetricEvent.VTM_CYCLE_COMPLETED;
  }
}
```

**Step 2: Update DomainEvent factory method**

In `DomainEvent.java`, add `travelTime` parameter:

```java
  static VtmCycleCompleted vtmCycleCompleted(
      double timestamp,
      int robotId,
      String toteId,
      double cycleStartTime,
      double cycleTime,
      int rackId,
      int relayPositionId,
      double travelTime) {  // NEW
    return new VtmCycleCompleted(
        timestamp, robotId, toteId, cycleStartTime, cycleTime, rackId, relayPositionId, travelTime);
  }
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors in DesmojVtmProcess and MetricsManager (missing travelTime argument)

---

## Task 4: Update DesmojVtmProcess to pass travelTime (with navFactor fix)

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java:314-321`

**Step 1: Calculate and pass travelTime with navFactor**

Find the vtmCycleCompleted emission (around line 314) and update to calculate total travel time with navFactor applied:

```java
        // Emit cycle completion
        double cycleEnd = now();
        // CRITICAL: Apply navFactor to match actual simulation time!
        double totalTravelTime = (travelToRack + travelToRelay) * navFactor;
        if (metricsManager != null) {
          metricsManager.emit(
              DomainEvent.vtmCycleCompleted(
                  cycleEnd,
                  vtm.getId(),
                  String.valueOf(tote.getId().value()),
                  cycleStart,
                  cycleEnd - cycleStart,
                  sourceRack.getId().value(),
                  extractRelayNumericId(relay.getId()),
                  totalTravelTime));  // NEW
        }
```

**Note:** The variables `travelToRack` and `travelToRelay` are already computed earlier in the method (lines 207 and 233). The `navFactor` is a field in the class.

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation error in MetricsManager (needs to extract travelTime)

---

## Task 5: Update MetricsManager to extract VTM travelTime

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricsManager.java:316-335`

**Step 1: Extract and include travelTime in event data**

Update the VTM_CYCLE_COMPLETED case in convertRobotEvent:

```java
      } else { // VTM_CYCLE_COMPLETED
        var toteId = (String) invokeMethod(event, "toteId");
        var cycleStartTime = (Double) invokeMethod(event, "cycleStartTime");
        var cycleTime = (Double) invokeMethod(event, "cycleTime");
        var rackId = (Integer) invokeMethod(event, "rackId");
        var relayPositionId = (Integer) invokeMethod(event, "relayPositionId");
        var travelTime = (Double) invokeMethod(event, "travelTime");  // NEW

        return MetricEventData.builder()
            .eventType(eventType)
            .timestamp(timestamp)
            .robotId(robotId)
            .data(
                Map.of(
                    "tote_id", toteId,
                    "cycle_start_time", cycleStartTime,
                    "cycle_time", cycleTime,
                    "rack_id", rackId,
                    "relay_position_id", relayPositionId,
                    "travel_time", travelTime))  // NEW
            .build();
      }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS - all event chain complete

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/RobotEvents.java \
        apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/DomainEvent.java \
        apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java \
        apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricsManager.java
git commit -m "feat: add travelTime to VtmCycleCompleted event"
```

---

## Task 6: Extend RobotCycleTimelinePoint and PerRobotSummary records

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/RobotMetrics.java:38-58`

**Step 1: Add travel time fields to RobotCycleTimelinePoint**

```java
  @Builder
  public record RobotCycleTimelinePoint(
      double time,
      double timeMinutes,
      double avgCycleTime,
      double cumulativeAvgCycleTime,
      double avgTravelTime,              // NEW
      double cumulativeAvgTravelTime,    // NEW
      int windowCycles,
      int totalCycles) {}
```

**Step 2: Add travel time fields to PerRobotSummary**

```java
  @Builder
  public record PerRobotSummary(
      int robotId,
      String robotType,
      int cycleCount,
      double totalCycleTime,
      double avgCycleTime,
      double totalTravelTime,   // NEW
      double avgTravelTime,     // NEW
      double utilization) {}
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors in MetricsCollectors.java (builders need new fields)

---

## Task 7: Update RobotCycleLogEntry to include travelTime

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:139-140`

**Step 1: Add travelTime to internal record**

```java
    /** Internal record for raw cycle data. */
    private record RobotCycleLogEntry(
        double time, int robotId, String robotType, double cycleTime, double travelTime) {}
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors (cycleLog.add needs travelTime)

---

## Task 8: Update handleRobotCycleCompleted to track travelTime

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:147-178`

**Step 1: Extract travelTime and update tracking**

Replace the handleRobotCycleCompleted method body:

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

        // NEW: Extract and track travel time
        Object travelTimeObj = event.getData().get("travel_time");
        double travelTime = 0.0;
        if (travelTimeObj instanceof Number) {
          travelTime = ((Number) travelTimeObj).doubleValue();
          stats.merge("total_travel_time", travelTime, (a, b) -> (Double) a + (Double) b);
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
        // NEW: Track travel time by type
        if (travelTimeObj instanceof Number) {
          typeStats.merge("total_travel_time", travelTime, (a, b) -> (Double) a + (Double) b);
        }

        // Add to cycle log for timeline computation (with travelTime)
        cycleLog.add(new RobotCycleLogEntry(event.getTimestamp(), robotId, robotType, cycleTime, travelTime));
      }
      log.trace("Robot {} cycle completed at t={}", robotId, event.getTimestamp());
    }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors in computeCycleTimeTimeline (builder needs travel time fields)

---

## Task 9: Update computeCycleTimeTimeline to track travel time

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:211-264`

**Step 1: Add travel time tracking to timeline computation**

Replace the computeCycleTimeTimeline method:

```java
    private List<RobotCycleTimelinePoint> computeCycleTimeTimeline() {
      if (cycleLog.isEmpty()) {
        return List.of();
      }

      List<RobotCycleTimelinePoint> timeline = new ArrayList<>();

      // Cumulative tracking
      int cumulativeCycles = 0;
      double cumulativeCycleTime = 0.0;
      double cumulativeTravelTime = 0.0;  // NEW

      // Sliding window tracking
      int windowStartIdx = 0;
      int windowCycleCount = 0;
      double windowCycleTime = 0.0;
      double windowTravelTime = 0.0;  // NEW

      for (int i = 0; i < cycleLog.size(); i++) {
        RobotCycleLogEntry entry = cycleLog.get(i);
        double currentTime = entry.time();

        // Update cumulative metrics
        cumulativeCycles++;
        cumulativeCycleTime += entry.cycleTime();
        cumulativeTravelTime += entry.travelTime();  // NEW
        double cumulativeAvg = cumulativeCycleTime / cumulativeCycles;
        double cumulativeAvgTravel = cumulativeTravelTime / cumulativeCycles;  // NEW

        // Slide window start forward to maintain 10-minute window
        while (windowStartIdx < i
            && cycleLog.get(windowStartIdx).time() < currentTime - TIME_WINDOW_SECONDS) {
          windowCycleCount--;
          windowCycleTime -= cycleLog.get(windowStartIdx).cycleTime();
          windowTravelTime -= cycleLog.get(windowStartIdx).travelTime();  // NEW
          windowStartIdx++;
        }

        // Add current entry to window
        windowCycleCount++;
        windowCycleTime += entry.cycleTime();
        windowTravelTime += entry.travelTime();  // NEW

        double windowAvg = windowCycleCount > 0 ? windowCycleTime / windowCycleCount : 0.0;
        double windowAvgTravel = windowCycleCount > 0 ? windowTravelTime / windowCycleCount : 0.0;  // NEW

        RobotCycleTimelinePoint point =
            RobotCycleTimelinePoint.builder()
                .time(currentTime)
                .timeMinutes(currentTime / 60.0)
                .avgCycleTime(windowAvg)
                .cumulativeAvgCycleTime(cumulativeAvg)
                .avgTravelTime(windowAvgTravel)  // NEW
                .cumulativeAvgTravelTime(cumulativeAvgTravel)  // NEW
                .windowCycles(windowCycleCount)
                .totalCycles(cumulativeCycles)
                .build();

        timeline.add(point);
      }

      return timeline;
    }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors in computeCycleTimeTimelineForType (same issue)

---

## Task 10: Update computeCycleTimeTimelineForType to track travel time

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:272-328`

**Step 1: Add travel time tracking (same pattern as Task 7)**

Replace the computeCycleTimeTimelineForType method:

```java
    private List<RobotCycleTimelinePoint> computeCycleTimeTimelineForType(String robotType) {
      List<RobotCycleLogEntry> filteredLog =
          cycleLog.stream().filter(entry -> entry.robotType().equals(robotType)).toList();

      if (filteredLog.isEmpty()) {
        return List.of();
      }

      List<RobotCycleTimelinePoint> timeline = new ArrayList<>();

      // Cumulative tracking
      int cumulativeCycles = 0;
      double cumulativeCycleTime = 0.0;
      double cumulativeTravelTime = 0.0;  // NEW

      // Sliding window tracking
      int windowStartIdx = 0;
      int windowCycleCount = 0;
      double windowCycleTime = 0.0;
      double windowTravelTime = 0.0;  // NEW

      for (int i = 0; i < filteredLog.size(); i++) {
        RobotCycleLogEntry entry = filteredLog.get(i);
        double currentTime = entry.time();

        // Update cumulative metrics
        cumulativeCycles++;
        cumulativeCycleTime += entry.cycleTime();
        cumulativeTravelTime += entry.travelTime();  // NEW
        double cumulativeAvg = cumulativeCycleTime / cumulativeCycles;
        double cumulativeAvgTravel = cumulativeTravelTime / cumulativeCycles;  // NEW

        // Slide window start forward to maintain 10-minute window
        while (windowStartIdx < i
            && filteredLog.get(windowStartIdx).time() < currentTime - TIME_WINDOW_SECONDS) {
          windowCycleCount--;
          windowCycleTime -= filteredLog.get(windowStartIdx).cycleTime();
          windowTravelTime -= filteredLog.get(windowStartIdx).travelTime();  // NEW
          windowStartIdx++;
        }

        // Add current entry to window
        windowCycleCount++;
        windowCycleTime += entry.cycleTime();
        windowTravelTime += entry.travelTime();  // NEW

        double windowAvg = windowCycleCount > 0 ? windowCycleTime / windowCycleCount : 0.0;
        double windowAvgTravel = windowCycleCount > 0 ? windowTravelTime / windowCycleCount : 0.0;  // NEW

        RobotCycleTimelinePoint point =
            RobotCycleTimelinePoint.builder()
                .time(currentTime)
                .timeMinutes(currentTime / 60.0)
                .avgCycleTime(windowAvg)
                .cumulativeAvgCycleTime(cumulativeAvg)
                .avgTravelTime(windowAvgTravel)  // NEW
                .cumulativeAvgTravelTime(cumulativeAvgTravel)  // NEW
                .windowCycles(windowCycleCount)
                .totalCycles(cumulativeCycles)
                .build();

        timeline.add(point);
      }

      return timeline;
    }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors in buildPerRobotSummaries (builder needs travel time fields)

---

## Task 11: Update buildPerRobotSummaries to include travel time

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:335-376`

**Step 1: Extract and compute travel time metrics**

Replace the buildPerRobotSummaries method:

```java
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

        // NEW: Extract total travel time
        double totalTravelTime =
            stats.get("total_travel_time") instanceof Number
                ? ((Number) stats.get("total_travel_time")).doubleValue()
                : 0.0;

        String robotType = stats.get("type") instanceof String ? (String) stats.get("type") : "RTP";

        double avgCycleTime = cycleCount > 0 ? totalCycleTime / cycleCount : 0.0;
        double avgTravelTime = cycleCount > 0 ? totalTravelTime / cycleCount : 0.0;  // NEW
        double utilization = simulationDuration > 0 ? totalCycleTime / simulationDuration : 0.0;

        PerRobotSummary summary =
            PerRobotSummary.builder()
                .robotId(robotId)
                .robotType(robotType)
                .cycleCount(cycleCount)
                .totalCycleTime(totalCycleTime)
                .avgCycleTime(avgCycleTime)
                .totalTravelTime(totalTravelTime)  // NEW
                .avgTravelTime(avgTravelTime)      // NEW
                .utilization(utilization)
                .build();

        summaries.add(summary);
      }

      // Sort by robot ID for consistent ordering
      summaries.sort((a, b) -> Integer.compare(a.robotId(), b.robotId()));

      return summaries;
    }
```

**Step 2: Compile to verify all backend changes**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 3: Format code**

Run: `cd apps/backend && mvn spotless:apply`

**Step 4: Commit backend changes**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/RobotMetrics.java \
        apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat: track robot travel time in metrics collector"
```

---

## Task 12: Start backend and regenerate frontend types

**Step 1: Start the backend server**

Run: `rm -f /tmp/server.log && lsof -ti:8080 | xargs kill -9 2>/dev/null || true && sleep 2 && npx nx serve backend > /tmp/server.log 2>&1 &`

Wait for startup: `for i in {1..30}; do sleep 1; if curl -s http://localhost:8080/health > /dev/null; then echo "Server ready!"; break; fi; done`

**Step 2: Regenerate frontend types**

Run: `npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts`
Expected: File updated with new avgTravelTime and cumulativeAvgTravelTime fields

**Step 3: Verify types exported**

Check that `generated.ts` exports the updated types (should auto-export from generated-api.ts).

**Step 4: Commit generated types**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "chore: regenerate frontend types with travel time fields"
```

---

## Task 13: Add travel time MetricCard to RobotResults

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:38-48,99-121`

**Step 1: Calculate average travel time from summaries**

Add after avgCycleTime calculation (around line 47):

```tsx
  const avgTravelTime =
    filteredSummaries.length > 0
      ? filteredSummaries.reduce((sum, s) => sum + s.avgTravelTime, 0) / filteredSummaries.length
      : 0
```

**Step 2: Update grid to 4 columns and add MetricCard**

Change grid from `grid-cols-3` to `grid-cols-4` and add the new card:

```tsx
          <div className="grid grid-cols-4 gap-4">
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

            <MetricCard
              title="Avg Travel Time"
              value={`${avgTravelTime.toFixed(1)}s`}
              status="neutral"
            />
          </div>
```

**Step 3: Run typecheck**

Run: `npm run check`
Expected: PASS (or fix any type errors)

---

## Task 14: Update timeline data processing for travel time

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:50-57`

**Step 1: Add travel time fields to processTimelineData**

```tsx
  const processTimelineData = (timeline: typeof cycleTimeTimeline) =>
    timeline.map((point) => ({
      time_minutes: point.timeMinutes,
      avg_cycle_time: point.avgCycleTime,
      avg_travel_time: point.avgTravelTime,  // NEW
      cumulative_avg_cycle_time: point.cumulativeAvgCycleTime,
      cumulative_avg_travel_time: point.cumulativeAvgTravelTime,  // NEW
      window_cycles: point.windowCycles,
      total_cycles: point.totalCycles,
    }))
```

---

## Task 15: Add travel time line to RTP timeline chart

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:207-228`

**Step 1: Add travel time metric to RTP TimelineChart**

```tsx
              <TimelineChart
                data={processedTimelineData as never}
                metrics={[
                  {
                    dataKey: 'avg_cycle_time',
                    name: 'Cycle Time (Window)',
                    color: '#FF6600',
                    strokeWidth: 3,
                  },
                  {
                    dataKey: 'avg_travel_time',
                    name: 'Travel Time (Window)',
                    color: '#F97316',
                    strokeWidth: 3,
                  },
                  {
                    dataKey: 'cumulative_avg_cycle_time',
                    name: 'Cycle Time (Overall)',
                    color: '#6B7280',
                    strokeWidth: 2,
                    strokeDasharray: '5 5',
                  },
                ]}
                yAxisLabel="Time (seconds)"
                height={400}
                formatYAxis={(value) => `${value.toFixed(0)}s`}
                formatTooltipValue={(value, name) => [`${value.toFixed(1)}s`, name]}
              />
```

---

## Task 16: Add travel time line to VTM timeline chart

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:131-153`

**Step 1: Add travel time metric to VTM TimelineChart**

```tsx
              <TimelineChart
                data={processedVtmTimelineData as never}
                metrics={[
                  {
                    dataKey: 'avg_cycle_time',
                    name: 'Cycle Time (Window)',
                    color: '#3B82F6',
                    strokeWidth: 3,
                  },
                  {
                    dataKey: 'avg_travel_time',
                    name: 'Travel Time (Window)',
                    color: '#F97316',
                    strokeWidth: 3,
                  },
                  {
                    dataKey: 'cumulative_avg_cycle_time',
                    name: 'Cycle Time (Overall)',
                    color: '#6B7280',
                    strokeWidth: 2,
                    strokeDasharray: '5 5',
                  },
                ]}
                yAxisLabel="Time (seconds)"
                height={400}
                formatYAxis={(value) => `${value.toFixed(0)}s`}
                formatTooltipValue={(value, name) => [`${value.toFixed(1)}s`, name]}
              />
```

---

## Task 17: Add travel time line to HTM timeline chart

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:168-190`

**Step 1: Add travel time metric to HTM TimelineChart**

```tsx
              <TimelineChart
                data={processedHtmTimelineData as never}
                metrics={[
                  {
                    dataKey: 'avg_cycle_time',
                    name: 'Cycle Time (Window)',
                    color: '#8B5CF6',
                    strokeWidth: 3,
                  },
                  {
                    dataKey: 'avg_travel_time',
                    name: 'Travel Time (Window)',
                    color: '#F97316',
                    strokeWidth: 3,
                  },
                  {
                    dataKey: 'cumulative_avg_cycle_time',
                    name: 'Cycle Time (Overall)',
                    color: '#6B7280',
                    strokeWidth: 2,
                    strokeDasharray: '5 5',
                  },
                ]}
                yAxisLabel="Time (seconds)"
                height={400}
                formatYAxis={(value) => `${value.toFixed(0)}s`}
                formatTooltipValue={(value, name) => [`${value.toFixed(1)}s`, name]}
              />
```

---

## Task 18: Add travel time bar data arrays

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:64-93`

**Step 1: Add travel time bar data for all robot types**

After utilizationBarData (around line 72), add:

```tsx
  const travelTimeBarData = filteredSummaries.map((s) => ({
    name: `${s.robotType} ${s.robotId}`,
    value: s.avgTravelTime,
  }))

  // For relay: separate VTM and HTM travel time data
  const vtmTravelTimeData = vtmSummaries.map((s) => ({
    name: `VTM ${s.robotId}`,
    value: s.avgTravelTime,
  }))

  const htmTravelTimeData = htmSummaries.map((s) => ({
    name: `HTM ${s.robotId}`,
    value: s.avgTravelTime,
  }))
```

---

## Task 19: Add travel time bar chart for RTP

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:297-321`

**Step 1: Update RTP grid to 3 columns and add travel time chart**

```tsx
          <div>
            <SectionHeader title="RTP Robot Performance" />

            <div className="grid grid-cols-3 gap-6">
              <BarChart
                data={cycleTimeBarData}
                yAxisLabel="Cycle Time (seconds)"
                height={300}
                barColor="#FF6600"
                formatYAxis={(value) => `${value.toFixed(0)}s`}
                formatTooltipValue={(value) => `${value.toFixed(1)}s`}
              />

              <BarChart
                data={travelTimeBarData}
                yAxisLabel="Travel Time (seconds)"
                height={300}
                barColor="#F97316"
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
```

---

## Task 20: Add travel time bar chart for VTM

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:243-267`

**Step 1: Update VTM grid to 3 columns and add travel time chart**

```tsx
            {vtmSummaries.length > 0 && (
              <div>
                <SectionHeader title="VTM Robot Performance" />

                <div className="grid grid-cols-3 gap-6">
                  <BarChart
                    data={vtmCycleTimeData}
                    yAxisLabel="Cycle Time (seconds)"
                    height={300}
                    barColor="#3B82F6"
                    formatYAxis={(value) => `${value.toFixed(0)}s`}
                    formatTooltipValue={(value) => `${value.toFixed(1)}s`}
                  />

                  <BarChart
                    data={vtmTravelTimeData}
                    yAxisLabel="Travel Time (seconds)"
                    height={300}
                    barColor="#F97316"
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
```

---

## Task 21: Add travel time bar chart for HTM

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/RobotResults.tsx:270-294`

**Step 1: Update HTM grid to 3 columns and add travel time chart**

```tsx
            {htmSummaries.length > 0 && (
              <div>
                <SectionHeader title="HTM Robot Performance" />

                <div className="grid grid-cols-3 gap-6">
                  <BarChart
                    data={htmCycleTimeData}
                    yAxisLabel="Cycle Time (seconds)"
                    height={300}
                    barColor="#8B5CF6"
                    formatYAxis={(value) => `${value.toFixed(0)}s`}
                    formatTooltipValue={(value) => `${value.toFixed(1)}s`}
                  />

                  <BarChart
                    data={htmTravelTimeData}
                    yAxisLabel="Travel Time (seconds)"
                    height={300}
                    barColor="#F97316"
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
```

**Step 2: Run typecheck and lint**

Run: `npm run check`
Expected: PASS

**Step 3: Commit frontend changes**

```bash
git add apps/frontend/src/pages/simulator/components/RobotResults.tsx
git commit -m "feat: display robot travel time metrics in frontend"
```

---

## Task 22: End-to-end testing

**Step 1: Start backend if not running**

Run: `lsof -ti:8080 || (npx nx serve backend > /tmp/server.log 2>&1 &)`

**Step 2: Start frontend**

Run: `npm run dev`

**Step 3: Test RTP system**

1. Configure an RTP simulation (systemType: "rtp")
2. Run simulation
3. Navigate to Robots tab in results
4. Verify:
   - "Avg Travel Time" MetricCard shows value
   - Timeline chart shows travel time line (orange)
   - Travel Time bar chart appears between Cycle Time and Utilization

**Step 4: Test Relay system**

1. Configure a Relay simulation (systemType: "relay")
2. Run simulation
3. Navigate to Robots tab in results
4. Verify:
   - "Avg Travel Time" MetricCard shows value
   - VTM timeline shows travel time line
   - HTM timeline shows travel time line
   - VTM/HTM sections each have travel time bar chart

**Step 5: Sanity check**

Verify: avgTravelTime < avgCycleTime (travel is component of cycle)

**Step 6: Cleanup**

Stop servers: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

---

## Summary

**Total Tasks:** 22

**Files Modified:**
- `apps/backend/.../orchestrator/DesmojHtmProcess.java` (navFactor bug fix)
- `apps/backend/.../orchestrator/DesmojRtpRobotProcess.java` (navFactor bug fix)
- `apps/backend/.../orchestrator/DesmojVtmProcess.java` (navFactor fix + travelTime)
- `apps/backend/.../metrics/events/RobotEvents.java`
- `apps/backend/.../metrics/events/DomainEvent.java`
- `apps/backend/.../metrics/MetricsManager.java`
- `apps/backend/.../metrics/RobotMetrics.java`
- `apps/backend/.../metrics/collectors/MetricsCollectors.java`
- `apps/frontend/src/types/generated-api.ts`
- `apps/frontend/src/pages/simulator/components/RobotResults.tsx`

**Commits:**
1. `fix: apply navFactor to travel time in metrics events` (HTM + RTP)
2. `feat: add travelTime to VtmCycleCompleted event` (with navFactor)
3. `feat: track robot travel time in metrics collector`
4. `chore: regenerate frontend types with travel time fields`
5. `feat: display robot travel time metrics in frontend`
