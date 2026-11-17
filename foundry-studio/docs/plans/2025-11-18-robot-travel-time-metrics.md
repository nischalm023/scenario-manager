# Robot Travel Time Metrics Design

## Overview

Add average travel time tracking alongside cycle time for all robot types (RTP, VTM, HTM). This provides visibility into whether robots are traveling to nearby or distant storage locations.

## Data Model Changes

### RobotCycleTimelinePoint (extend)

Add 2 fields:
- `avgTravelTime` - 10-minute rolling window average
- `cumulativeAvgTravelTime` - Overall average to this point

### PerRobotSummary (extend)

Add 2 fields:
- `totalTravelTime` - Sum of all travel times for this robot
- `avgTravelTime` - Average travel time per cycle

### RobotCycleLogEntry (internal record, extend)

Add 1 field:
- `travelTime` - Travel time for this cycle (alongside existing `cycleTime`)

### Per-robot and by-type stats

New stat keys:
- `total_travel_time` - Accumulated travel time

## Backend Computation

### RobotCollector.handleRobotCycleCompleted

Extract travel time from event data (same pattern as cycle time):

```java
Object travelTimeObj = event.getData().get("travel_time");
double travelTime = 0.0;
if (travelTimeObj instanceof Number) {
    travelTime = ((Number) travelTimeObj).doubleValue();
    stats.merge("total_travel_time", travelTime, (a, b) -> (Double) a + (Double) b);
}

// Also add to byTypeStats
typeStats.merge("total_travel_time", travelTime, (a, b) -> (Double) a + (Double) b);

// Update cycle log entry to include travel time
cycleLog.add(new RobotCycleLogEntry(
    event.getTimestamp(), robotId, robotType, cycleTime, travelTime));
```

### Timeline computation

Extend `computeCycleTimeTimeline()` and `computeCycleTimeTimelineForType()`:
- Track `cumulativeTravelTime` and `windowTravelTime` alongside cycle time
- Same 10-minute sliding window logic
- Compute `avgTravelTime = windowTravelTime / windowCycleCount`
- Compute `cumulativeAvgTravelTime = cumulativeTravelTime / cumulativeCycles`

### Per-robot summary

In `buildPerRobotSummaries()`:

```java
double totalTravelTime = stats.get("total_travel_time") instanceof Number
    ? ((Number) stats.get("total_travel_time")).doubleValue()
    : 0.0;

double avgTravelTime = cycleCount > 0 ? totalTravelTime / cycleCount : 0.0;
```

## Frontend Display

### MetricCard section

Add average travel time card, change grid to `grid-cols-4`:

```tsx
<MetricCard
  title="Avg Travel Time"
  value={`${avgTravelTime.toFixed(1)}s`}
  status="neutral"
/>
```

### Timeline charts

Add travel time as additional line (orange color #F97316):

```tsx
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
```

Same pattern for VTM and HTM timelines in relay mode.

### Bar charts

Add travel time bar chart, change grid to `grid-cols-3`:
- Per-robot average travel time alongside cycle time and utilization

### Data processing

Extend timeline data transformation:

```tsx
const processTimelineData = (timeline: typeof cycleTimeTimeline) =>
  timeline.map((point) => ({
    time_minutes: point.timeMinutes,
    avg_cycle_time: point.avgCycleTime,
    avg_travel_time: point.avgTravelTime,
    cumulative_avg_cycle_time: point.cumulativeAvgCycleTime,
    cumulative_avg_travel_time: point.cumulativeAvgTravelTime,
    window_cycles: point.windowCycles,
    total_cycles: point.totalCycles,
  }))
```

## Files to Modify

### Backend
1. `apps/backend/.../metrics/RobotMetrics.java` - Extend records
2. `apps/backend/.../metrics/collectors/MetricsCollectors.java` - Update RobotCollector

### Frontend
1. `apps/frontend/src/pages/simulator/components/RobotResults.tsx` - Add displays

### Generated
- Regenerate frontend types after backend changes

## Testing

1. Run simulation with both RTP and relay systems
2. Verify travel time appears in metrics response
3. Verify frontend displays all new components
4. Sanity check: travel time < cycle time

## Rationale

Travel time is bundled with cycle time in the same timeline points because:
1. They come from the same events at identical timestamps
2. Travel time is a component of cycle time (cycle = travel + processing)
3. Matches existing codebase patterns (FaceLogEntry bundles related metrics)
4. Simplifies frontend correlation and charting
