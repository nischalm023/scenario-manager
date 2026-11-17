# Grid Utilization Metrics: A Concept Document

**Date:** 2025-12-26
**Status:** Concept
**Purpose:** Define a rigorous framework for measuring grid utilization as a resource in warehouse robotics systems

---

## 1. Problem Statement

### The Grid as a Constrained Resource

In a warehouse with autonomous mobile robots (AMRs), the floor grid is a shared resource with finite capacity. Unlike traditional resources (CPU, memory), the grid has unique properties:

- **Spatial exclusivity**: Only one robot can occupy a cell at a time
- **Temporal dynamics**: Capacity is consumed over time, then released
- **Topological constraints**: Not all cells connect to all others; paths must navigate around obstacles
- **Multi-agent contention**: Robots compete for the same cells simultaneously

### The Core Questions

1. **Capacity**: What is the maximum throughput this grid can sustain before performance degrades?
2. **Bottlenecks**: Which cells or regions limit overall system throughput?
3. **Utilization**: Are we using the available grid capacity efficiently?

### Analogy: Highway vs Parking Lot

A highway has high throughput because traffic flows in aligned directions. A parking lot has low throughput despite the same area because movements conflict.

The same grid can behave like a highway (efficient) or a parking lot (congested) depending on traffic patterns. Our metrics must capture this distinction.

---

## 2. Theoretical Foundations

### 2.1 Traffic Flow Theory - The Fundamental Diagram

The foundational insight from traffic engineering: density, flow, and speed are related.

```
Flow (q) = Density (k) × Speed (v)
```

| Variable | Definition | Unit | Warehouse Example |
|----------|------------|------|-------------------|
| Density (k) | Robots per unit area | robots/m² | 5 robots in a 100m² zone = 0.05 |
| Speed (v) | Average robot velocity | m/s | Robots moving at 1.2 m/s average |
| Flow (q) | Robots crossing a boundary per time | robots/s | 8 robots pass checkpoint per second |

**The Critical Insight: Flow is Not Monotonic**

As you add more robots (density increases):
- Initially, flow increases (more robots = more throughput)
- At critical density, flow peaks
- Beyond critical density, flow *decreases* (congestion collapse)

```
Flow
  ↑
  │      ╭──────╮
  │     ╱        ╲
  │    ╱          ╲
  │   ╱            ╲
  │  ╱              ╲
  └─┴────────────────→ Density
     Free    Critical   Jammed
     flow    density
```

**Worked Example:**

A zone is 20m × 5m = 100m². During simulation:
- Average robots in zone: 4
- Average robot speed in zone: 0.8 m/s

```
Density (k) = 4 / 100 = 0.04 robots/m²
Flow (q) = 0.04 × 0.8 = 0.032 robots/m²/s
         = 3.2 robots passing per 100m² per second
```

If you add more robots and speed drops to 0.3 m/s:
```
Density (k) = 8 / 100 = 0.08 robots/m²
Flow (q) = 0.08 × 0.3 = 0.024 robots/m²/s  ← Lower!
```

Double the robots, but 25% less throughput. That's congestion.

---

### 2.2 Queueing Theory - Little's Law

The most useful result in queueing theory, deceptively simple:

```
L = λ × W
```

| Variable | Definition | Warehouse Meaning |
|----------|------------|-------------------|
| L | Average number in system | Robots in a zone |
| λ (lambda) | Arrival rate | Robots entering zone per second |
| W | Average time in system | Time a robot spends in zone |

**Why This Matters**

Little's Law connects observable quantities. If you can measure any two, you get the third for free.

**Worked Example:**

Observing a zone over 10 minutes:
- 120 robots entered the zone (λ = 120/600s = 0.2 robots/s)
- Average robots in zone at any moment: 3 (L = 3)

```
W = L / λ = 3 / 0.2 = 15 seconds
```

Each robot spends 15 seconds in this zone on average.

**Detecting Degradation:**

If the zone is 10m long and robots move at 1 m/s, *expected* traversal is 10s.

Measured W = 15s means robots spend 5 extra seconds—waiting, rerouting, or slowing down.

```
Delay Ratio = Actual W / Expected W = 15 / 10 = 1.5
```

A delay ratio of 1.5 means 50% overhead due to congestion.

---

### 2.2.1 Zone NAV Factor - Localizing the System NAV Factor

At the system level, NAV Factor measures:

```
NAV Factor = Actual Travel Time / Ideal A* Time (system-wide)
```

The zone-level version decomposes *where* that overhead comes from.

**Zone NAV Factor:**

```
Zone NAV Factor = Actual Time in Zone / Expected Time in Zone
```

Where:
- Actual Time = measured (via Little's Law or direct observation)
- Expected Time = zone distance / free-flow speed

**Worked Example:**

System-wide NAV factor = 1.4

Breaking down by zone:

| Zone | Expected (s) | Actual (s) | Zone NAV Factor | Contribution |
|------|--------------|------------|-----------------|--------------|
| Zone A | 8 | 8.5 | 1.06 | Low friction |
| Zone B | 12 | 21 | 1.75 | **Bottleneck** |
| Zone C | 10 | 11 | 1.10 | Low friction |
| Zone D | 6 | 8 | 1.33 | Moderate |

Zone B has NAV factor 1.75—robots take 75% longer than they should. That's your hotspot.

**The sum should reconcile:**

```
Total Expected = 8 + 12 + 10 + 6 = 36s
Total Actual = 8.5 + 21 + 11 + 8 = 48.5s
System NAV Factor = 48.5 / 36 = 1.35 ≈ 1.4 ✓
```

**Measuring Zone NAV Factor - Two Approaches:**

| Approach | How | Pros | Cons |
|----------|-----|------|------|
| **A\* Based** | Compute ideal path through zone, compare to actual | Precise expected time | Requires path segmentation per zone |
| **Little's Law Based** | Measure L and λ, derive W, compare to free-flow | Only needs zone-level counters | Needs accurate free-flow baseline |

---

### 2.3 Pedestrian Dynamics - Multi-Directional Conflict

Traffic flow theory assumes unidirectional flow. Warehouse robots move in all directions—more like pedestrians in a plaza than cars on a highway.

**Key Concept: Bidirectional Flow Degradation**

When two streams of pedestrians (or robots) move in opposite directions, capacity drops significantly.

Research finding (from Fruin, Helbing et al.):

```
Effective Capacity (bidirectional) ≈ 0.6 × Capacity (unidirectional)
```

Opposing flows create friction—robots must yield, wait, or reroute.

**Worked Example:**

A corridor can handle 10 robots/minute flowing East (unidirectional).

With bidirectional flow (5 East, 5 West):
```
Effective capacity ≈ 0.6 × 10 = 6 robots/minute
Attempting 10 → congestion
```

**The Conflict Factor**

For a zone with multi-directional traffic:

```
Conflict Factor = 1 - (|Net Flow| / Gross Flow)
```

Where:
- Gross Flow = total movements in all directions
- Net Flow = vector sum (dominant direction)

| Scenario | Gross | Net | Conflict Factor |
|----------|-------|-----|-----------------|
| All robots go East | 10 | 10 | 1 - 10/10 = 0 (no conflict) |
| 8 East, 2 West | 10 | 6 | 1 - 6/10 = 0.4 |
| 5 East, 5 West | 10 | 0 | 1 - 0/10 = 1.0 (max conflict) |

**Interpretation:**
- Conflict Factor 0 = corridor behavior (efficient)
- Conflict Factor 1 = intersection behavior (high friction)

---

### 2.3.1 Crossing Conflicts - Turning Movements

Opposing flow (East vs West) is one conflict type. Perpendicular crossing (North-South vs East-West) is another.

**Crossing Intensity:**

```
Crossing Intensity = (N↔S flow) × (E↔W flow) / (Total flow)²
```

This captures how much perpendicular traffic intersects.

**Worked Example:**

Zone with 20 robot movements observed:
- 8 going North or South (N↔S)
- 12 going East or West (E↔W)

```
Crossing Intensity = (8 × 12) / 20² = 96 / 400 = 0.24
```

**Comparison across zones:**

| Zone | N↔S | E↔W | Total | Crossing Intensity |
|------|-----|-----|-------|-------------------|
| Corridor | 2 | 18 | 20 | (2×18)/400 = 0.09 |
| Intersection | 10 | 10 | 20 | (10×10)/400 = 0.25 |
| T-junction | 6 | 14 | 20 | (6×14)/400 = 0.21 |

Higher crossing intensity = more turn conflicts = more congestion potential.

**Combined Metric: Zone Friction Score**

```
Zone Friction = α × Conflict Factor + β × Crossing Intensity
```

Where α and β are weights (start with 0.5 each, tune empirically).

---

### 2.4 Multi-Agent Pathfinding (MAPF) - Windowed Reservations

Production systems often use Collaborative A* (CA*) with windowed reservations—robots only reserve cells for a limited time horizon, not the full path.

**Key MAPF Metrics from Literature:**

| Metric | Definition | What it reveals |
|--------|------------|-----------------|
| **Makespan** | Time for ALL robots to complete tasks | Worst-case performance |
| **Flowtime** | Sum of individual completion times | Average performance |
| **Conflict Count** | Number of path conflicts detected | System stress |
| **Replanning Rate** | Path recalculations per robot per second | Reactive overhead |

**Worked Example - Flowtime vs Makespan:**

3 robots complete tasks at t=10s, t=12s, t=25s

```
Makespan = 25s (slowest robot)
Flowtime = 10 + 12 + 25 = 47s (total robot-seconds)
Average Flowtime = 47 / 3 = 15.7s
```

Makespan tells you when everything is done.
Flowtime tells you how much total waiting/inefficiency exists.

---

### 2.4.1 The Windowed Reservation Problem

With full-path reservation: Robot sees all future conflicts, plans globally optimal path.

With windowed reservation (window size W): Robot only reserves W steps ahead.

```
Full path:     ████████████████████ (all reserved)
Windowed:      █████░░░░░░░░░░░░░░░ (only first W steps)
                    ↑
               Uncertainty begins here
```

**Consequences:**

| Aspect | Full Path | Windowed |
|--------|-----------|----------|
| Conflict visibility | Complete | Partial |
| Planning optimality | Global | Local |
| Deadlock risk | Low | Higher |
| Computation cost | High | Lower |
| Reaction to changes | Slow (replanning) | Fast (adaptive) |

**Worked Example - Window Size Effect:**

Grid with 10 robots, average path length 30 cells.

| Window Size | Conflicts Detected Late | Replanning Events | Avg Path Stretch |
|-------------|------------------------|-------------------|------------------|
| W = 5 | High | 4.2 per robot | 1.35× |
| W = 15 | Medium | 2.1 per robot | 1.18× |
| W = 30 (full) | None | 0.8 per robot | 1.08× |

Smaller window = more surprises = more replanning = longer actual paths.

---

### 2.4.2 Metric: Horizon Blindness Cost

```
Horizon Blindness Cost = (Actual Flowtime - Ideal Flowtime) / Ideal Flowtime
```

This measures how much performance you lose due to limited visibility.

**Worked Example:**

Ideal flowtime (full visibility): 450 robot-seconds
Actual flowtime (W=10): 580 robot-seconds

```
Horizon Blindness Cost = (580 - 450) / 450 = 0.29 = 29%
```

You're paying a 29% penalty for using windowed reservations.

---

## 3. Metrics Framework

### 3.1 Tier 1: System Health Metrics (Always Visible)

These are your dashboard metrics—glanceable system state.

| Metric | Formula | Target | Alert When |
|--------|---------|--------|------------|
| **System NAV Factor** | Actual travel time / A* ideal time | < 1.3 | > 1.5 |
| **Throughput** | Tasks completed / hour | Baseline dependent | < 80% of baseline |
| **Fleet Utilization** | Time robots moving / Total time | 70-85% | < 60% or > 90% |
| **System Density** | Active robots / Navigable area | Layout dependent | Approaching critical |

**Worked Example - Fleet Utilization:**

10 robots over 1 hour:
- Total robot-time: 10 × 3600s = 36,000s
- Time spent moving: 28,800s
- Time idle/waiting: 7,200s

```
Fleet Utilization = 28,800 / 36,000 = 80% ✓ Healthy
```

If utilization drops to 55%:
```
Robots spending 45% of time waiting → congestion or starvation
```

**The Utilization Paradox:**

Too low (< 60%) = robots starved for work or blocked
Too high (> 90%) = no slack, system fragile to perturbations

Sweet spot: 70-85%

---

### 3.2 Tier 2: Hotspot Identification (Where is the problem?)

When Tier 1 signals trouble, Tier 2 tells you where to look.

| Metric | Formula | Visualization | Reveals |
|--------|---------|---------------|---------|
| **Zone NAV Factor** | Actual zone time / Expected zone time | Bar chart by zone | Which zones add overhead |
| **Cell Occupancy** | Total seconds occupied / Observation period | Heatmap | High-traffic cells |
| **Zone Friction** | α×Conflict Factor + β×Crossing Intensity | Heatmap by zone | Multi-directional conflict areas |
| **Dwell Rate** | Traversals with delay / Total traversals | Heatmap | Where robots get stuck |

**Worked Example - Cell Occupancy Heatmap:**

Observation period: 600 seconds
Grid: 20×20 cells

| Cell | Seconds Occupied | Occupancy Rate |
|------|------------------|----------------|
| (5,5) | 180 | 30% |
| (10,10) | 420 | 70% ← Hotspot |
| (15,8) | 90 | 15% |
| (3,12) | 12 | 2% |

Cell (10,10) is occupied 70% of the time—either a critical path or a bottleneck.

**Worked Example - Dwell Rate:**

Cell (10,10) analysis:
- 50 robots traversed this cell
- Expected traversal: 1 second
- 35 robots took > 1.5 seconds (delayed)

```
Dwell Rate = 35 / 50 = 70%
```

High occupancy + high dwell rate = congestion bottleneck
High occupancy + low dwell rate = efficient thoroughfare

---

### 3.3 Tier 3: Diagnostic Drill-Down (Why is it a problem?)

When Tier 2 identifies a hotspot, Tier 3 explains the root cause.

| Diagnostic | Data Required | Reveals |
|------------|---------------|---------|
| **Turning Movement Matrix** | Entry/exit direction per traversal | Which movement pairs conflict |
| **Time-Series Occupancy** | Cell state at regular intervals | When congestion occurs (patterns) |
| **Robot Trace Replay** | Timestamped positions for robots through cell | Specific conflict sequences |
| **Wait Event Log** | Robot ID, cell, wait start, wait end | Who waited for whom |

**Worked Example - Turning Movement Matrix:**

Cell (10,10) flagged as hotspot. Analyzing 50 traversals:

```
        Exit →
Entry ↓   N    E    S    W   Total
   N      2   12    3    1    18
   E      1    0    8    2    11
   S      4    3    1    6    14
   W      3    2    2    0     7
                              ────
                               50
```

**Reading the matrix:**
- N→E: 12 robots (24%) — dominant flow, right turn from North
- E→S: 8 robots (16%) — right turn from East
- These two cross each other inside the cell

```
N→E path:  ↓ then →
E→S path:  ← then ↓
               ╳ Conflict point
```

**Diagnosis:** This cell is an uncontrolled intersection with two high-volume turning flows that cross.

**Worked Example - Time-Series Pattern:**

Sampling cell (10,10) occupancy every 10 seconds for 10 minutes:

```
Time(s)   0  10  20  30  40  50  60  70  80  90 ...
Occupied  0   1   1   0   0   1   1   1   1   0 ...

Pattern detected: Occupancy spikes every ~50-60 seconds
```

Correlate with task scheduling—likely batch release pattern causing periodic congestion waves.

---

## 4. Capacity Model

### 4.1 The Fundamental Diagram for Grids

Every system has a capacity curve:

```
Throughput
(tasks/hr)
    ↑
    │         ╭───────╮ ← Maximum capacity
    │        ╱         ╲
    │       ╱           ╲
    │      ╱             ╲
    │     ╱               ╲
    │    ╱                 ╲
    └───┴───────────────────→ Robot count
        Understaffed │ Optimal │ Congested
                     │  zone   │
```

**Key Points:**
- **Understaffed region**: Adding robots increases throughput linearly
- **Optimal zone**: Throughput plateaus, small gains with more robots
- **Congested region**: Adding robots *decreases* throughput

### 4.2 Finding Critical Density

Critical density (k*) is where throughput peaks. Beyond this, adding robots hurts.

**Empirical Method:**

Run simulations with increasing robot counts, measure throughput:

| Robots | Throughput (tasks/hr) | NAV Factor | Status |
|--------|----------------------|------------|--------|
| 5 | 120 | 1.08 | Understaffed |
| 10 | 230 | 1.12 | Growing |
| 15 | 320 | 1.18 | Growing |
| 20 | 380 | 1.28 | Near optimal |
| 25 | 395 | 1.45 | **Peak** |
| 30 | 370 | 1.72 | Congested |
| 35 | 310 | 2.10 | Degraded |

**Reading the data:**
- Maximum throughput: 395 tasks/hr at 25 robots
- Critical density: 25 robots for this grid
- Warning sign: NAV factor jumped from 1.28 to 1.45 at peak

---

### 4.3 Theoretical Capacity Estimation

Before running simulations, estimate capacity from grid structure.

**Max-Flow Min-Cut Principle:**

System capacity is limited by the narrowest bottleneck—like water through a pipe.

```
System Capacity ≤ Bottleneck Capacity
```

**Identifying the Bottleneck:**

For any origin-destination pair, find the "cut" with minimum capacity:

```
        ┌─────────────────────────┐
        │    STORAGE ZONE         │
        │    ════════════         │
        │         ║               │
        │    ─────╫───── ← Cut A: 1 cell wide (bottleneck)
        │         ║               │
        │    ════════════         │
        │    STATION ZONE         │
        └─────────────────────────┘
```

**Worked Example - Aisle Bottleneck:**

Layout has 3 aisles connecting storage to stations.
Each aisle: 1 cell wide, robot traversal: 2 seconds

```
Aisle capacity = 1 robot / 2 seconds = 0.5 robots/second per aisle
Total cut capacity = 3 × 0.5 = 1.5 robots/second = 90 robots/minute
```

If each task requires 2 aisle traversals (there + back):
```
Task capacity = 90 / 2 = 45 tasks/minute = 2,700 tasks/hour (theoretical max)
```

**Reality Factor:**

Theoretical max assumes perfect coordination. Apply discount:
```
Practical Capacity ≈ 0.6 to 0.7 × Theoretical Capacity
Practical Capacity ≈ 0.65 × 2,700 = 1,755 tasks/hour
```

---

### 4.4 Capacity Utilization Ratio

Compare actual throughput to theoretical capacity:

```
Capacity Utilization = Actual Throughput / Theoretical Capacity
```

| Utilization | Interpretation |
|-------------|----------------|
| < 50% | Underutilized—add work or reduce fleet |
| 50-75% | Healthy operating range |
| 75-90% | High utilization—monitor closely |
| > 90% | At risk—small disruptions cause cascading delays |

---

## 5. Bottleneck Detection

Capacity tells you the limit. Bottleneck detection tells you *what* creates that limit.

### 5.1 Bottleneck Definition

A bottleneck is a resource that:
1. Operates at or near 100% utilization
2. Has a queue forming upstream
3. Limits overall system throughput

**The key insight:** Improving a non-bottleneck doesn't improve the system. Only the bottleneck matters.

---

### 5.2 Detection Algorithm: Utilization + Queue Method

**Step 1: Measure utilization per zone**

```
Zone Utilization = Time Zone Occupied / Observation Time
```

**Step 2: Measure queue buildup**

```
Upstream Density = Robots waiting to enter zone / Upstream area
```

**Step 3: Identify bottleneck candidates**

```
IF Zone Utilization > 85% AND Upstream Density increasing
THEN Zone is bottleneck candidate
```

**Worked Example:**

| Zone | Utilization | Upstream Density Trend | Bottleneck? |
|------|-------------|------------------------|-------------|
| A | 45% | Stable | No |
| B | 92% | Increasing ↑ | **Yes** |
| C | 78% | Stable | No |
| D | 88% | Decreasing ↓ | No (draining) |

Zone B: High utilization + growing queue = bottleneck
Zone D: High utilization but queue draining = was bottleneck, now clearing

---

### 5.3 Detection Algorithm: Sensitivity Analysis

Bottlenecks respond to capacity changes. Non-bottlenecks don't.

**Method:**

1. Increase capacity of zone X (e.g., widen aisle, add path)
2. Measure system throughput change
3. If throughput increases → X was bottleneck
4. If throughput unchanged → X was not bottleneck

**Worked Example:**

Baseline throughput: 380 tasks/hour

| Intervention | New Throughput | Change | Conclusion |
|--------------|----------------|--------|------------|
| Widen Zone A | 385 | +1.3% | Not bottleneck |
| Widen Zone B | 440 | +15.8% | **Bottleneck** |
| Widen Zone C | 382 | +0.5% | Not bottleneck |

Zone B improvement unlocked 60 more tasks/hour. That's your constraint.

---

### 5.4 Bottleneck Hierarchy

Fixing one bottleneck reveals the next one.

```
Initial:     [Zone B] limits system at 380/hr
                ↓ Fix Zone B
Next:        [Zone D] limits system at 440/hr
                ↓ Fix Zone D
Next:        [Zone A] limits system at 510/hr
                ↓ ...
Eventually:  Theoretical max approached
```

**Track the chain:**

| Iteration | Bottleneck | Throughput | Gain |
|-----------|------------|------------|------|
| 0 | Zone B | 380 | — |
| 1 | Zone D | 440 | +60 |
| 2 | Zone A | 510 | +70 |
| 3 | Zone C | 545 | +35 |
| 4 | (distributed) | 560 | +15 |

Diminishing returns indicate you're approaching theoretical capacity.

---

## 6. Windowed Reservation Considerations

Production systems using Collaborative A* with windowed reservations exhibit different congestion patterns.

### 6.1 Window Size Trade-offs

| Window Size | Conflict Detection | Compute Cost | Congestion Pattern |
|-------------|-------------------|--------------|-------------------|
| Small (5-10 steps) | Late | Low | Local clusters, frequent replanning |
| Medium (15-20 steps) | Moderate | Moderate | Balanced |
| Large (30+ steps) | Early | High | Smoother but rigid paths |

### 6.2 Emergent Congestion in Windowed Systems

With limited visibility, robots make locally optimal decisions that are globally suboptimal.

**The Herding Problem:**

```
Time T:     Robot A plans path through Zone X (looks clear)
Time T+1:   Robot B plans path through Zone X (looks clear)
Time T+2:   Robot C plans path through Zone X (looks clear)
Time T+5:   All three arrive at Zone X simultaneously → congestion
```

None saw the others because they planned in sequence with limited windows.

**Metric: Herding Index**

```
Herding Index = Peak Zone Density / Average Zone Density
```

**Worked Example:**

Zone X over 10 minutes:
- Average robots in zone: 2.1
- Peak robots in zone: 7

```
Herding Index = 7 / 2.1 = 3.3
```

A herding index > 2.5 suggests robots are clumping—windowed planning may be causing coordination failures.

---

### 6.3 Window-Aware Metrics

Standard metrics need adjustment for windowed systems:

| Standard Metric | Windowed Adjustment |
|-----------------|---------------------|
| NAV Factor | Include replanning overhead in "actual time" |
| Conflict Count | Count conflicts detected AND conflicts discovered late |
| Zone Capacity | Discount by (1 - Late Conflict Rate) |

**Late Conflict Rate:**

```
Late Conflict Rate = Conflicts discovered after window / Total conflicts
```

**Worked Example:**

100 total path conflicts detected:
- 70 detected within planning window (handled smoothly)
- 30 detected after window (required emergency replanning)

```
Late Conflict Rate = 30 / 100 = 30%
Zone Effective Capacity = Theoretical × (1 - 0.30) = 70% of theoretical
```

High late conflict rate = window size may be too small for current density.

---

## 7. Summary

### The Complete Framework

```
┌─────────────────────────────────────────────────────────────┐
│                    SYSTEM HEALTH (Tier 1)                   │
│  • System NAV Factor    • Fleet Utilization                │
│  • Throughput           • System Density                    │
│                                                             │
│  "Are we healthy?" → Yes: monitor / No: drill down ↓       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    HOTSPOTS (Tier 2)                        │
│  • Zone NAV Factor      • Cell Occupancy Heatmap           │
│  • Zone Friction        • Dwell Rate Heatmap               │
│                                                             │
│  "Where is the problem?" → Zone B flagged → drill down ↓   │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    DIAGNOSTICS (Tier 3)                     │
│  • Turning Movement Matrix    • Time-Series Occupancy      │
│  • Robot Trace Replay         • Wait Event Log             │
│                                                             │
│  "Why is Zone B a problem?" → Crossing flows at turn       │
└─────────────────────────────────────────────────────────────┘
```

### Key Formulas Reference

| Metric | Formula |
|--------|---------|
| NAV Factor | Actual Time / A* Ideal Time |
| Zone NAV Factor | Actual Zone Time / Expected Zone Time |
| Flow | Density × Speed |
| Little's Law | L = λ × W |
| Conflict Factor | 1 - (\|Net Flow\| / Gross Flow) |
| Crossing Intensity | (N↔S flow × E↔W flow) / Total² |
| Capacity Utilization | Actual Throughput / Theoretical Capacity |
| Herding Index | Peak Density / Average Density |
| Late Conflict Rate | Late Conflicts / Total Conflicts |

### Capacity Planning Workflow

1. Measure current throughput and NAV factor
2. Run simulations with increasing robot counts
3. Find critical density (throughput peak)
4. Identify bottleneck via sensitivity analysis
5. Address bottleneck, repeat

---

## References

- **Traffic Flow Theory**: Lighthill-Whitham-Richards (LWR) model, Highway Capacity Manual
- **Queueing Theory**: Little's Law (1961), Kendall notation
- **Pedestrian Dynamics**: Helbing et al. (Social Force Model), Fruin (Level of Service)
- **MAPF**: Stern et al. (2019) "Multi-Agent Pathfinding: Definitions, Variants, and Benchmarks"
- **Collaborative A***: Silver (2005) "Cooperative Pathfinding"
