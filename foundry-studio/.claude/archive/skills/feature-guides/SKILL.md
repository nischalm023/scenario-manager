---
name: feature-guides
description: Use when working with Set Cover, Animation Playback, or Algorithm Comparison features. Implementation guides for specific simulator features.
---

# Feature Guides

---

## Set Cover Order Fulfillment

### Overview
Greedy set cover algorithm minimizes storage spaces per order with inventory reservations.

### Key Changes
- `OrderProcessor.selectNextOrders()` requires `activeStorageSpaces` parameter
- Set cover prefers active spaces (in transit, at relay, at station) with +1000 bonus
- Reservations validated after selection (Option B pattern)
- Task cache disabled during initial rollout

### Benefits
- 20-40% PPF improvement from multi-order storage space sharing
- Relay system: Totes at relay positions count as active
- Supports split quantities across multiple storage spaces

### Configuration
```java
maxTotesPerOrder: 10  // Maximum storage spaces per order
```

### How It Works
1. `WarehouseInventory.findBestStorageSpaceForItems()` - Greedy selection based on SKU coverage
2. Active space bonus: +1000 score for spaces already in transit/at relay/at station
3. `OrderProcessor.generateTasksWithSetCover()` - Iteratively selects storage spaces until order fulfilled
4. Reservation validation: `WarehouseInventory.tryReserveForOrder()` - All-or-nothing atomic reservation
5. Multi-order sharing: Different orders can share same storage space

### Active Space Tracking
- **RTP System**: Mark rack active when picked up, inactive when returned
- **Relay System**: Mark tote active when delivered to relay, inactive when returned to rack
- `DesmojWarehouseModel.getActiveStorageSpaces()` - Thread-safe concurrent set

### Key Classes
| Class | Purpose |
|-------|---------|
| `WarehouseInventory` | Reservation tracking, set cover search, consumption |
| `OrderProcessor` | Set cover task generation, reservation validation |
| `DesmojWarehouseModel` | Active storage space tracking |
| `DesmojVtmProcess` | Mark spaces active/inactive (relay) |
| `DesmojRtpRobotProcess` | Mark spaces active/inactive (R2P) |

---

## Animation Playback

### Overview
Post-simulation animation playback for debugging and performance analysis.

### Configuration
```json
{
  "enableSnapshots": true,
  "snapshotIntervalSeconds": 10.0
}
```

### Implementation
- `DesmojSnapshotCaptureProcess` runs every N seconds during simulation
- Captures: robots, relays, stations, totes, storage spaces, metrics
- Returns snapshots array in `SimulationResult`

### Frontend Components
| Component | Purpose |
|-----------|---------|
| `AnimationControls` | Play/pause/scrub timeline with speed control |
| `WarehouseMap` | Renders snapshot state with visual overlays |

### Visual Colors
| Entity | Color |
|--------|-------|
| VTM Robot | Blue |
| HTM Robot | Green |
| RTP Robot | Orange |
| Active Storage | Orange |
| Reserved Storage | Blue |
| Empty Storage | Gray |

### Usage
1. Enable snapshots in simulation config
2. Run simulation
3. View animation in results page after completion

---

## Algorithm Comparison

### Overview
Compare put algorithms (Greedy vs Min Weighted Cycle Distance) with backend-calculated metrics.

### Usage
1. Configure simulation parameters in UI
2. Click "Compare Put Algorithms" button
3. View comparison results modal

### Metrics Compared
| Metric | Description |
|--------|-------------|
| Orders Completed | Total orders fulfilled during simulation |
| Total Picks Processed | Total pick tasks completed by operators |
| Picks Per Hour Per Station | Throughput metric (picks/hr/station) |
| Operator Utilization | Percentage of time actively picking (0-100%) |
| Picks Per Face | Batching efficiency (picks per storage space presentation) |

### Architecture

**Fleet Simulator** (`ComparisonService.java`):
- Runs 2 simulations sequentially with different `putAlgorithm` values
- Extracts metrics from nested `SimulationResults.metrics.*` paths
- Calculates best performer using higherIsBetter logic
- Computes percentage differences: `((value - bestValue) / bestValue) * 100`
- Returns type-safe `ComparisonResponse` DTO

**Frontend** (`AlgorithmComparison.tsx`):
- Displays pre-calculated metrics (no computation logic)
- Shows algorithm summary cards with win counts
- Renders comparison table with green highlighting on best performers
- Timeline chart shows picks per face trends

### Data Flow
```
User clicks button → POST /comparison/run → ComparisonService
  → Run simulation with putAlgorithm: "greedy"
  → Run simulation with putAlgorithm: "min_weighted_cycle_distance"
  → Extract metrics from both results
  → Calculate best performer + percentage diffs
  → Return ComparisonResponse → Frontend displays
```

### Adding New Metrics
1. Add enum value to `ComparisonMetricType.java`
2. Add extraction logic in `calculateComparisonMetrics()`:
   ```java
   extractMetric(
       ComparisonMetricType.NEW_METRIC,
       successfulResults,
       "unit",
       true, // higherIsBetter
       r -> r.result().metrics().someMetrics().newMetric())
   ```
3. Regenerate frontend types
4. Metric automatically appears in comparison table

### Key Classes
| Class | Purpose |
|-------|---------|
| `ComparisonMetricType` | Enum for type-safe metric identification |
| `ComparisonMetric` | Record with values, unit, bestModeIndex, percentageDifferences |
| `ComparisonResponse` | Top-level DTO with results, metrics, timeline, wins |
| `ModeResult` | Individual algorithm run result with config and metrics |

---

## Related Skills

- `.claude/skills/java-backend/` - Backend patterns
- `.claude/skills/frontend/` - Frontend implementation
