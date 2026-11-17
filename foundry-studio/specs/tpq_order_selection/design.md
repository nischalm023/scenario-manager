# TPQ-Based Order Selection Algorithm

**Status:** Design Phase
**Date:** 2025-11-17
**Author:** Design Session with Claude
**System:** RTP and Relay warehouse systems

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Academic Framing](#academic-framing)
3. [Metrics and Formulations](#metrics-and-formulations)
4. [System-Specific Considerations](#system-specific-considerations)
5. [Alternatives Considered](#alternatives-considered)
6. [Proposed Solution](#proposed-solution)
7. [Implementation Approach](#implementation-approach)
8. [FAQ](#faq)
9. [References](#references)

---

## Problem Statement

### Current State: PPF (Picks Per Face) Degradation

**Existing algorithms** (MAXIMIZE_PPF_DIRECTLY, MAXIMIZE_PPF_WITH_TIERING) optimize for Picks Per Face:

```
PPF = Total_picks / Number_of_storage_spaces
```

**The Problem:**

PPF-based algorithms exhibit **temporal degradation** in performance:

1. **Early Phase (0-30% simulation time):**
   - Select orders from nearby storage spaces (A-items, front of warehouse)
   - High PPF, low travel time
   - Excellent throughput

2. **Mid Phase (30-70% simulation time):**
   - A-items depleting, forced to mix nearby and distant storage
   - PPF remains reasonable but travel times increasing
   - Throughput begins to decline

3. **Late Phase (70-100% simulation time):**
   - A-items exhausted, only C-items remain (back of warehouse)
   - High travel times dominate
   - Throughput collapses despite maintaining PPF

**Root Cause:** PPF is a **static metric** that doesn't account for:
- Bot travel overhead (only rack reuse)
- Temporal resource depletion
- Queuing dynamics (bot vs operator balance)

**Impact:**
- Unpredictable performance degradation
- Poor long-term throughput despite good early metrics
- Operator idle time in late phases (bots become bottleneck)

### Desired State: Consistent, Predictable Throughput

**Goal:** Order selection algorithm that:
1. **Maintains consistent throughput** throughout simulation
2. **Balances bot and operator utilization** dynamically
3. **Prevents resource depletion** (doesn't exhaust nearby storage early)
4. **Adapts to system configuration** (bot count, operator count, warehouse geometry)
5. **Provides predictable performance** (no surprise degradation)

---

## Academic Framing

### Warehouse as Queuing System

#### RTP System: Single-Stage M/M/c Queue

**Model:**
```
[Storage Spaces] → [Bots (transport)] → [Operator Stations (service)] → [Complete]
                     Pipeline delay        Servers (c)
```

**Components:**
- **Servers (c):** Operator stations (typically 4)
- **Service process:** Operators picking items from racks
- **Arrival process:** Bots delivering racks to stations
- **Transport delay:** Bot travel time (affects arrival rate)

**Utilization:**
```
ρ = λ / (c × μ)

Where:
  λ = Effective arrival rate (bots/second delivering racks)
  c = Number of operator stations
  μ = Service rate (1 / picking_time)

Stability: ρ < 1
Optimal: 0.7 ≤ ρ ≤ 0.85
```

**Cycle time components:**
```
Bot_cycle = 2 × Travel_time + Load_unload + Service_time
Service_time = num_picks × time_per_pick
```

#### Relay System: Two-Stage Tandem Queue (Jackson Network)

**Model:**
```
[Storage] → [VTM] → [Relay Buffer] → [HTM + Operator] → [Complete]
            Stage 1                    Stage 2
```

**Stage 1 (VTM):**
- **Service:** Rack → Relay transport
- **Service time:** **Fixed per tote** (independent of pick count)
- **Output:** Totes queued at relay positions

**Stage 2 (HTM + Operator):**
- **Service:** Relay → Station transport + Operator picking + Return
- **Service time:** Variable (depends on picks)
- **Output:** Completed orders

**Key Difference:** VTM stage has **constant cost** regardless of tote utilization!

**Jackson Network Stability:**
```
Stage 1: ρ₁ = λ / (V × μ_vtm) < 1
Stage 2: ρ₂ = λ / (H × μ_op) < 1

Where:
  V = Number of VTMs
  H = Number of HTMs
  λ = System arrival rate
```

### Theoretical Foundation: Little's Law & Kingman VUT

**Little's Law:**
```
L = λ × W

Where:
  L = Average number in system
  λ = Arrival rate
  W = Average time in system
```

**Kingman VUT Equation** (approximate waiting time):
```
W_q ≈ (V × U × T) / (2 × (1 - U))

Where:
  V = Variability factor = CV_a² + CV_s²
  U = Utilization (ρ)
  T = Average service time

  CV_a = Coefficient of variation of inter-arrival times
  CV_s = Coefficient of variation of service times
```

**Implications:**
- Waiting time grows **super-linearly** with utilization (1/(1-U) term)
- Variability **multiplies** the impact of utilization
- High variance systems need lower utilization to maintain performance

---

## Metrics and Formulations

### Task Pool Quality (TPQ)

**Definition:** Ratio of total system time to productive work time

```
TPQ = (Overhead_time + Productive_time) / Productive_time
    = 1 + (Overhead_time / Productive_time)
```

#### RTP System TPQ

**For a batch of racks/orders:**

```
TPQ_rtp = (Σ Travel_time_i + Σ Operator_time_i) / Σ Operator_time_i

Where:
  Travel_time_i = 2 × distance_i / robot_speed  (round trip)
  Operator_time_i = num_picks_i × time_per_pick
```

**Interpretation:**
- **TPQ = 1:** Instant transport (theoretical minimum)
- **TPQ = 7:** Bot overhead is 6× the picking work
- **Higher TPQ:** More overhead, lower efficiency

**Target Calculation (from throughput balance):**

For balanced system where bot capacity = operator capacity:
```
num_bots / (2×Travel + Service) = num_operators / Service

Solving:
  Travel / Service = (num_bots - num_operators) / (2 × num_operators)

  TPQ_target = 1 + Travel/Service
             = 1 + (num_bots - num_operators) / (2 × num_operators)
             = (num_bots + num_operators) / (2 × num_operators)

Example: 20 bots, 4 operators
  TPQ_target = 24 / 8 = 3.0
```

**Academic Justification:**

TPQ is **work-weighted ratio** (Option A: aggregate then divide):

```
TPQ = Σ(Travel_i + Service_i) / Σ Service_i

NOT: (1/n) × Σ[(Travel_i + Service_i) / Service_i]  [count-weighted, wrong!]
```

**Why work-weighted is correct:**
1. Heavy jobs (more picks) should influence metric more
2. Aligns with throughput analysis: `Throughput = 1/TPQ`
3. Compatible with Little's Law
4. Additive across subsystems

#### Relay System TPQ (Two-Stage)

**For a batch of totes:**

**Stage 1 (VTM overhead):**
```
TPQ_stage1 = Σ VTM_time_i / Σ Operator_time_i

Where:
  VTM_time_i = 2 × distance_rack_to_relay / vtm_speed  (per tote, FIXED)
```

**Stage 2 (HTM overhead):**
```
TPQ_stage2 = Σ HTM_time_i / Σ Operator_time_i

Where:
  HTM_time_i = 2 × distance_relay_to_station / htm_speed  (per tote, FIXED)
```

**Why separate metrics?**

1. **VTM cost doesn't scale with picks** (tote is atomic unit)
2. **Stages have different bottleneck potential**
3. **Requires different optimization strategies**

**Target Calculation:**
```
TPQ_stage1_target = num_vtms / num_operators
TPQ_stage2_target = num_htms / num_operators

Example: 8 VTMs, 4 HTMs, 4 Operators
  TPQ_stage1_target = 8/4 = 2.0
  TPQ_stage2_target = 4/4 = 1.0
```

### Variability Factor (V) from Kingman VUT

**Definition:** Captures system unpredictability

```
V = CV_a² + CV_s²

Where:
  CV = Coefficient of Variation = σ / μ
```

**Calculation from active task pool:**

```java
// Inter-arrival times (bot cycle times)
for each task:
  cycle_time = 2 × travel_time + service_time + load_unload

CV_a = stddev(cycle_times) / mean(cycle_times)

// Service times (operator picking)
service_times = [num_picks × time_per_pick for each task]

CV_s = stddev(service_times) / mean(service_times)

V = CV_a² + CV_s²
```

**Critical Insight: What variability to penalize?**

**WRONG (naive approach):**
```
Minimize: CV across all tasks in system
→ Penalizes mixing near/far orders
→ Encourages homogeneous batches
→ Leads to resource depletion!
```

**CORRECT (per-station arrival smoothness):**
```
Minimize: CV of inter-arrival times AT EACH STATION

NOT the CV of task characteristics across system
```

**Why this matters:**

| Scenario | System CV | Station Arrival CV | Quality |
|----------|-----------|-------------------|---------|
| All near racks | Low (0.1) | ? | May deplete A-items |
| All far racks | Low (0.1) | ? | High overhead |
| Mixed near/far | High (0.7) | Low if spread evenly | **Desired!** |

**The problem is temporal clustering at stations, not spatial diversity!**

### Station-Level Variance Bounds

**From load-balancing literature:**

Optimal **Coefficient of Variation across stations** ≤ 0.15

**For target TPQ = 7, S = 4 stations:**
```
σ(TPQ_stations) ≤ 0.15 × 7 = 1.05

Using 1σ confidence (68%):
  Station TPQ bounds = [7 - 1.05, 7 + 1.05] ≈ [6, 8]
```

**Alternative derivation (from utilization range):**

Optimal utilization: 0.7 ≤ ρ ≤ 0.85

```
ρ_min = 0.70 → TPQ_max ≈ 7 × (0.75/0.70) = 7.5
ρ_max = 0.85 → TPQ_min ≈ 7 × (0.75/0.85) = 6.2

Station bounds ≈ [6.2, 7.5]
```

**Recommendation:** Allow **±1 variance** around target (bounds [6, 8] for target=7)

---

## System-Specific Considerations

### RTP: Single Transport Stage

**Characteristics:**
- One transport stage (rack → station)
- Cost amortizes across all picks in rack
- More picks per rack → Better efficiency (lower TPQ)

**Optimization focus:**
- Maximize picks per rack (set cover)
- Balance travel distance against PPF
- Single TPQ target for system

**Active storage spaces:**
- Rack is "active" when picked up by bot
- Remains active until returned to storage
- Used for piggybacking (prefer active racks in selection)

### Relay: Two Decoupled Transport Stages

**Characteristics:**
- Two transport stages (VTM, HTM) with different properties
- VTM cost: **Fixed per tote** (doesn't benefit from more picks)
- HTM cost: Fixed per tote
- Relay buffer decouples stages

**Optimization focus:**
- Balance both stages independently
- VTM bottleneck: Select nearby racks
- HTM bottleneck: Select nearby stations, maximize picks/tote
- Track two separate TPQ targets

**Active storage spaces:**
- Tote is "active" when delivered to relay position
- Remains active until returned to rack
- VTM and HTM cycles are independent

**Bottleneck identification:**

```
If TPQ_stage1 >> TPQ_stage2:
  VTM is bottleneck
  → Select nearby racks (reduce VTM time)
  → Don't worry about picks/tote (doesn't help VTM)

If TPQ_stage2 >> TPQ_stage1:
  HTM+Operator is bottleneck
  → Maximize picks per tote (better utilization)
  → Select nearby stations

If TPQ_stage1 ≈ TPQ_stage2:
  Balanced (ideal!)
```

---

## Alternatives Considered

### Alternative 1: Direct Utilization Optimization

**Approach:** Optimize ρ = λ/(c×μ) directly

**Formula:**
```
ρ = (num_bots / avg_bot_cycle) / (num_stations / avg_service_time)
```

**Pros:**
- Standard queuing theory metric
- Well-understood stability bounds (ρ < 1)
- Direct performance indicator

**Cons:**
- ❌ Requires estimating arrival rate (circular dependency)
- ❌ Depends on robot count during selection
- ❌ Complex calculation
- ❌ Equivalent to TPQ for selection purposes

**Relationship to TPQ:**
```
TPQ = (2×Travel + Service) / Service

For target ρ:
  ρ = (B / (2×Travel + Service)) / (O / Service)

Solving for Travel/Service gives TPQ-like ratio, but with bot/operator dependencies.
```

**Verdict:** TPQ is simpler and mathematically equivalent for order selection.

### Alternative 2: Cycle Time Minimization

**Approach:** Minimize average cycle time using Pollaczek-Khinchin formula

**Formula:**
```
Cycle_time = Service_time + W_q

Where W_q = (C(c,ρ) / (c×μ)) × (ρ / (1-ρ))
C(c,ρ) = Erlang-C formula
```

**Pros:**
- Comprehensive (includes waiting time)
- Theoretically rigorous

**Cons:**
- ❌ Extremely complex to calculate
- ❌ Requires distribution assumptions (Poisson arrivals, exponential service)
- ❌ Erlang-C computation expensive
- ❌ Overkill for selection heuristic

**Verdict:** Too complex for real-time order selection.

### Alternative 3: Throughput Maximization

**Approach:** Maximize system throughput directly

**Formula:**
```
Throughput = min(Bot_capacity, Operator_capacity)

Bot_capacity = num_bots / avg_cycle_time
Operator_capacity = num_operators / avg_service_time
```

**Analysis:**

For balance:
```
num_bots / (2×Travel + Service) = num_operators / Service

Solving:
  Travel/Service = (num_bots - num_operators) / (2×num_operators)

  TPQ = 1 + Travel/Service [same as TPQ formula!]
```

**Verdict:** Leads back to TPQ formulation. Use TPQ as proxy.

### Alternative 4: Distance Variance Hybrid (PPF + Distance Balance)

**Approach:** Combine PPF with distance variance penalty

**Formula:**
```
Score = PPF_score - λ × variance(distances)
```

**Pros:**
- Simpler than TPQ+VUT
- Builds on existing PPF logic
- Prevents distance clustering

**Cons:**
- ❌ Doesn't capture bot/operator balance
- ❌ No queuing theory foundation
- ❌ Variance target unclear (what's optimal?)

**Verdict:** Less principled than TPQ approach.

### Comparison Summary

| Approach | Complexity | Theoretical Foundation | Practical | Verdict |
|----------|------------|----------------------|-----------|---------|
| **TPQ** | Low | ✅ Throughput balance | ✅ Simple calculation | **Recommended** |
| **Utilization (ρ)** | Medium | ✅ Queuing theory | ⚠️ Requires arrival rate | Equivalent to TPQ |
| **Cycle Time** | High | ✅ Comprehensive | ❌ Too complex | Overkill |
| **Throughput** | Low | ✅ Business metric | ✅ Direct | Leads to TPQ |
| **Distance Variance** | Low | ⚠️ Ad-hoc | ✅ Simple | Less principled |
| **TPQ + VUT** | Medium | ✅ Queuing theory | ✅ Handles variance | **Best balance** |

---

## Proposed Solution

### Multi-Objective Score Function

**Primary:** TPQ targeting (resource balance)
**Secondary:** VUT variability (arrival smoothness)
**Tertiary:** Diversity bonus (prevent depletion)
**Quaternary:** Depletion penalty (resource awareness)

```
Score = w₁×TPQ_score + w₂×Variability_score + w₃×Diversity_score + w₄×Depletion_penalty
```

### Component 1: TPQ Targeting

**RTP System:**
```java
double calculateTPQ(List<Task> pool) {
  double totalTravel = pool.stream()
    .mapToDouble(t -> 2 × distance(t.rackId, t.stationId) / robotSpeed)
    .sum();

  double totalService = pool.stream()
    .mapToDouble(t -> t.numPicks × timePerPick)
    .sum();

  return totalService == 0 ? INFINITY : (totalTravel + totalService) / totalService;
}

double tpqScore = -Math.abs(actualTPQ - targetTPQ) × 50;  // Weight: 50
```

**Relay System:**
```java
record RelayTPQ(double stage1, double stage2) {}

RelayTPQ calculateRelayTPQ(List<Tote> pool) {
  double totalVtm = pool.stream()
    .mapToDouble(t -> 2 × distance(t.rackId, t.relayId) / vtmSpeed)
    .sum();

  double totalHtm = pool.stream()
    .mapToDouble(t -> 2 × distance(t.relayId, t.stationId) / htmSpeed)
    .sum();

  double totalService = pool.stream()
    .mapToDouble(t -> t.numPicks × timePerPick)
    .sum();

  return new RelayTPQ(totalVtm / totalService, totalHtm / totalService);
}

double tpqScore = -Math.abs(relayTPQ.stage1 - target1) × 50
                  -Math.abs(relayTPQ.stage2 - target2) × 50;
```

**Target Calculation:**
```java
double calculateTargetTPQ(int numBots, int numOperators) {
  // From throughput balance equation
  return (numBots + numOperators) / (2.0 × numOperators);
}

// Example: 20 bots, 4 operators → TPQ_target = 3.0
// Example: 8 bots, 4 operators → TPQ_target = 1.5
```

### Component 2: Per-Station Arrival Variability

**CRITICAL: Measure temporal smoothness, not spatial diversity!**

```java
double calculateStationArrivalCV(List<Task> tasksForStation) {
  // Sort by expected arrival time at station
  List<Double> arrivalTimes = tasksForStation.stream()
    .map(t -> t.assignmentTime + calculateTravelTime(t))
    .sorted()
    .collect(toList());

  // Calculate inter-arrival gaps
  List<Double> gaps = new ArrayList<>();
  for (int i = 1; i < arrivalTimes.size(); i++) {
    gaps.add(arrivalTimes.get(i) - arrivalTimes.get(i-1));
  }

  // CV of gaps (what matters for queueing!)
  double mean = gaps.stream().mapToDouble(g -> g).average().orElse(1.0);
  double variance = gaps.stream()
    .mapToDouble(g -> Math.pow(g - mean, 2))
    .average()
    .orElse(0.0);

  return Math.sqrt(variance) / mean;
}

double calculateSystemArrivalVariability(Map<StationId, List<Task>> byStation) {
  return byStation.values().stream()
    .mapToDouble(this::calculateStationArrivalCV)
    .average()
    .orElse(0.0);
}

double variabilityScore = -calculateSystemArrivalVariability(pool) × 20;  // Weight: 20
```

**Why this formulation:**
- ✅ Allows mixing near/far orders (spatial diversity)
- ✅ Penalizes temporal clustering (arrivals at 0s, 5s, 300s, 305s)
- ✅ Rewards smooth arrival streams (arrivals at 0s, 60s, 120s, 180s)

### Component 3: Diversity Bonus

**Encourage mixing of near/far orders explicitly:**

```java
double calculateDiversityBonus(List<Task> pool) {
  List<Double> distances = pool.stream()
    .map(t -> calculateDistance(t.rackId, t.stationId))
    .collect(toList());

  double mean = distances.stream().mapToDouble(d -> d).average().orElse(0);
  double variance = distances.stream()
    .mapToDouble(d -> Math.pow(d - mean, 2))
    .average()
    .orElse(0);
  double stdDev = Math.sqrt(variance);
  double cv = mean == 0 ? 0 : stdDev / mean;

  // Target: CV around 0.3-0.5 (healthy spatial mix)
  double targetCV = 0.4;
  return -Math.abs(cv - targetCV) × 10;  // Weight: 10
}
```

**Interpretation:**
- CV = 0: All orders same distance (homogeneous, bad for depletion)
- CV = 0.4: Good mix of near/far (sustainable)
- CV = 1.0: Extreme variance (may cause instability)

### Component 4: Depletion Penalty

**Track resource scarcity over time:**

```java
double calculateDepletionPenalty(
    Order candidate,
    Map<StorageSpaceId, Integer> usageCount,
    Map<StorageSpaceId, Integer> remainingDemand) {

  double penalty = 0;

  for (StorageSpaceId spaceId : candidate.getRequiredStorageSpaces()) {
    int timesUsed = usageCount.getOrDefault(spaceId, 0);
    int demand = remainingDemand.getOrDefault(spaceId, 1);

    // Scarcity ratio: overuse relative to remaining demand
    double scarcity = (double) timesUsed / demand;

    // Escalating penalty for overuse
    if (scarcity > 1.0) {
      penalty += (scarcity - 1.0) * 50;  // Linear escalation
    }
  }

  return -penalty;
}
```

**Why this helps:**
- Prevents exhausting high-demand storage spaces early
- Preserves near racks for later use
- Balances selection across warehouse zones

### Combined Score Function

```java
public double calculateOrderScore(
    Order candidate,
    List<Task> currentPool,
    SystemState state) {

  // Hypothetical pool with candidate added
  List<Task> hypotheticalPool = new ArrayList<>(currentPool);
  hypotheticalPool.addAll(generateTasks(candidate));

  // 1. TPQ targeting (primary objective)
  double tpq = calculateTPQ(hypotheticalPool);
  double tpqScore = -Math.abs(tpq - targetTPQ) * 50;

  // 2. Per-station arrival smoothness (not system-wide variance!)
  Map<StationId, List<Task>> byStation = groupByStation(hypotheticalPool);
  double variabilityScore = -calculateSystemArrivalVariability(byStation) * 20;

  // 3. Diversity bonus (encourage spatial mixing)
  double diversityScore = calculateDiversityBonus(hypotheticalPool);

  // 4. Depletion penalty (resource awareness)
  double depletionPenalty = calculateDepletionPenalty(
    candidate, state.usageCount, state.remainingDemand);

  // Combined score
  return tpqScore + variabilityScore + diversityScore + depletionPenalty;
}
```

### Greedy Selection Algorithm

```java
public List<OrderSelection> selectOrders(
    List<Order> pendingOrders,
    Set<StorageSpaceId> activeStorageSpaces,
    int maxOrders,
    double currentTime) {

  List<OrderSelection> selected = new ArrayList<>();
  List<Task> currentPool = getCurrentActiveTasks(activeStorageSpaces);

  while (selected.size() < maxOrders && !pendingOrders.isEmpty()) {
    Order best = null;
    double bestScore = Double.NEGATIVE_INFINITY;

    for (Order candidate : pendingOrders) {
      double score = calculateOrderScore(candidate, currentPool, state);

      if (score > bestScore) {
        bestScore = score;
        best = candidate;
      }
    }

    if (best != null) {
      List<Task> tasks = generateTasks(best);
      selected.add(new OrderSelection(best, tasks));
      currentPool.addAll(tasks);
      pendingOrders.remove(best);

      // Update state
      updateUsageCount(best);
    } else {
      break;
    }
  }

  // Validate reservations (Option B pattern from set cover)
  return validateAndReserve(selected);
}
```

---

## Implementation Approach

### Phase 1: Core TPQ Selection (MVP)

**Goal:** Implement basic TPQ-based selection without VUT enhancements

**Components:**
1. TPQ calculation for RTP and Relay systems
2. Dynamic target calculation based on robot/operator config
3. Greedy selection targeting TPQ
4. Per-station variance bounds (±1 around target)

**Success Criteria:**
- Maintains consistent TPQ throughout simulation
- No severe degradation in late phases
- System TPQ within ±0.5 of target

### Phase 2: VUT Variability Enhancement

**Goal:** Add per-station arrival smoothness metric

**Components:**
1. CV calculation for inter-arrival times at each station
2. Variability score integration
3. Weight tuning (TPQ vs variability)

**Success Criteria:**
- Smooth arrival rates at stations (CV < 0.3)
- Still maintains TPQ targets from Phase 1
- Reduced operator idle time

### Phase 3: Depletion Prevention

**Goal:** Add diversity and depletion awareness

**Components:**
1. Diversity bonus (spatial mixing)
2. Depletion penalty (usage tracking)
3. Multi-objective score function

**Success Criteria:**
- No early exhaustion of A-items
- Consistent mixing of near/far orders
- Final phase performance comparable to early phase

### Phase 4: Adaptive Tuning

**Goal:** Dynamic parameter adjustment based on system state

**Components:**
1. Bottleneck detection (bot vs operator utilization)
2. Adaptive weight adjustment
3. Online TPQ target updates

**Success Criteria:**
- Responds to changing system conditions
- Maintains performance under varying configurations
- Self-tuning (minimal manual parameter adjustment)

### Configuration Parameters

**New config fields:**

```java
public record TPQAlgorithmConfig(
  // TPQ targeting
  Double targetTPQ,  // null = auto-calculate from robot/operator ratio
  Double stationVarianceTolerance,  // Default: 1.0 (±1 around target)

  // VUT variability
  Boolean enableVariabilityPenalty,  // Default: true
  Double variabilityWeight,  // Default: 20.0

  // Diversity
  Boolean enableDiversityBonus,  // Default: true
  Double targetDistanceCV,  // Default: 0.4
  Double diversityWeight,  // Default: 10.0

  // Depletion
  Boolean enableDepletionPenalty,  // Default: true
  Double depletionWeight  // Default: 50.0
) {
  // Defaults...
}
```

**Integration with existing config:**

```java
public record AlgorithmConfig(
  String putAlgorithm,
  OrderSelectionAlgorithm orderSelectionAlgorithm,  // Add: MAXIMIZE_TPQ_WITH_VUT
  int maxTotesPerOrder,
  TPQAlgorithmConfig tpqConfig  // New nested config
) {}
```

### Testing Strategy

**Unit Tests:**
- TPQ calculation correctness (RTP and Relay)
- CV calculation for various distributions
- Score function edge cases

**Integration Tests:**
- Small scenario (4 robots, 1 station, 100 orders)
- Validate TPQ stays within target ±0.5
- Compare to PPF algorithms

**Performance Tests:**
- Large scenario (20 robots, 4 stations, 10000 orders)
- Measure consistency across simulation phases
- Track early vs late throughput ratio

**Validation Metrics:**
- **Throughput consistency:** StdDev(throughput per hour) / Mean(throughput)
- **Utilization balance:** |ρ_bots - ρ_operators|
- **Depletion score:** Percentage of A-items remaining at 50% simulation time

---

## FAQ

### Q1: Why not just use PPF directly?

**A:** PPF doesn't account for bot travel overhead. Two scenarios with same PPF can have drastically different throughput:

```
Scenario A: 5 picks/rack, 50s travel → Throughput = 5 / (100 + 60) = 0.031 picks/s
Scenario B: 5 picks/rack, 300s travel → Throughput = 5 / (600 + 60) = 0.0076 picks/s

Same PPF (5), but 4× difference in throughput!
```

TPQ captures this difference.

### Q2: Why work-weighted TPQ instead of count-weighted average?

**A:** Heavy jobs impact system performance more than light jobs.

```
Rack 1: 360s travel, 10 picks → Individual TPQ = 4.0
Rack 2: 360s travel, 50 picks → Individual TPQ = 1.7

Work-weighted: (360+360+120+600)/(120+600) = 2.0 ✓
Count-weighted: (4.0 + 1.7)/2 = 2.85 ✗
```

Work-weighted correctly reflects that Rack 2 dominates system throughput (83% of work).

### Q3: How does TPQ relate to utilization?

**A:** TPQ is inverse throughput efficiency.

```
System_throughput = Total_picks / Total_time
                  = Σ operator_time / (Σ travel_time + Σ operator_time)
                  = 1 / TPQ

Higher TPQ → Lower throughput
```

Utilization (ρ) can be derived from TPQ given robot/operator counts.

### Q4: Why separate TPQ metrics for Relay stages?

**A:** VTM and HTM have different bottleneck potential and require different optimization strategies.

```
If TPQ_stage1 = 5, TPQ_stage2 = 1:
  VTM is bottleneck → Optimize: Select nearby racks

If TPQ_stage1 = 1, TPQ_stage2 = 5:
  HTM+Operator bottleneck → Optimize: Maximize picks/tote
```

Single aggregate TPQ would obscure which stage needs optimization.

### Q5: Won't the algorithm still deplete A-items early?

**A:** Not with diversity bonus and depletion penalty.

- **Diversity bonus:** Explicitly rewards mixing near/far orders (target CV ≈ 0.4)
- **Depletion penalty:** Tracks usage and penalizes overusing scarce racks
- **VUT variability:** Doesn't penalize spatial diversity (only temporal clustering)

Combined, these prevent greedy exhaustion of nearby storage.

### Q6: How to tune the weights (50, 20, 10)?

**A:** Weights reflect relative importance and scale:

- **TPQ (50):** Primary objective, largest scale (deviations typically 1-5)
- **Variability (20):** Secondary, CV values typically 0.1-0.5
- **Diversity (10):** Tertiary, CV deviations typically 0.1-0.3
- **Depletion (50):** Escalating, scarcity ratio can be >2.0

Start with these defaults, tune based on simulation results:
- If depletion still occurs → Increase depletion weight
- If too much variance → Increase variability weight
- If not reaching TPQ target → Increase TPQ weight

### Q7: What if TPQ target can't be achieved?

**A:** Adapt dynamically:

```java
// Calculate achievable TPQ from remaining inventory
double meanRemainingDistance = calculateMean(remainingDistances);
double achievableTPQ = calculateTPQ(meanRemainingDistance, avgPicksPerOrder);

// Gradually shift target toward achievable
targetTPQ = 0.9 × targetTPQ + 0.1 × achievableTPQ;
```

This prevents unrealistic targets late in simulation.

### Q8: How does this handle relay buffer capacity?

**A:** Relay buffer capacity is modeled via Desmo-J `Res` (resource blocking):

- When relay position is full, VTM process blocks automatically
- This is reflected in actual VTM cycle times
- TPQ calculation uses observed/estimated cycle times including blocking

For selection, we can enhance with relay occupancy awareness:

```java
// Penalize selections that would saturate relay positions
if (relayOccupancy > 0.8) {
  score -= (relayOccupancy - 0.8) * 100;  // Congestion penalty
}
```

### Q9: Does this work with different warehouse geometries?

**A:** Yes, TPQ automatically adapts via distance calculations:

- **Narrow warehouse:** Lower travel times → Lower TPQ target appropriate
- **Wide warehouse:** Higher travel times → Higher TPQ target needed
- **Target calculation** uses actual distances and robot/operator counts

The algorithm is geometry-agnostic.

### Q10: How does this compare to machine learning approaches?

**A:** ML could learn optimal policies, but:

**Advantages of TPQ:**
- ✅ Interpretable (can explain why orders selected)
- ✅ No training data needed
- ✅ Guaranteed properties (maintains balance)
- ✅ Computationally cheap (O(n²) greedy vs RL training)

**When ML might help:**
- Learning optimal weights automatically
- Handling complex non-linear interactions
- Adapting to unforeseen conditions

TPQ provides strong baseline; ML could enhance.

### Q11: What about order priority/deadlines?

**A:** Can integrate with priority weights:

```java
double score = baseScore + priorityBonus;

double priorityBonus = order.priority * 100  // High priority orders
                     - max(0, currentTime - order.deadline) * 50;  // Deadline urgency
```

TPQ ensures throughput; priority overlay ensures fairness.

### Q12: How to measure if TPQ algorithm is working?

**Key metrics to track:**

1. **TPQ consistency:** StdDev(TPQ per batch) / Mean(TPQ) < 0.2
2. **Throughput stability:** Ratio(late_throughput / early_throughput) > 0.8
3. **Utilization balance:** Both ρ_bots and ρ_operators in [0.7, 0.85]
4. **Depletion score:** A-items remaining at 50% simulation > 30%
5. **Waiting time:** Operator idle time < 15% of simulation

Compare against PPF baselines across these dimensions.

---

## References

### Academic Papers

1. **Kingman, J.F.C.** (1961). "The single server queue in heavy traffic." *Mathematical Proceedings of the Cambridge Philosophical Society*, 57(4), 902-904.
   - VUT equation foundation

2. **Gross, D., & Harris, C.M.** (2008). *Fundamentals of Queueing Theory* (4th ed.). Wiley.
   - M/M/c queues, utilization bounds

3. **Jackson, J.R.** (1957). "Networks of waiting lines." *Operations Research*, 5(4), 518-521.
   - Jackson networks (tandem queues for relay system)

4. **Pinedo, M.L.** (2016). *Scheduling: Theory, Algorithms, and Systems* (5th ed.). Springer.
   - Multi-stage flow shop scheduling

5. **Little, J.D.C.** (1961). "A proof for the queuing formula: L = λW." *Operations Research*, 9(3), 383-387.
   - Little's Law

### Queuing Theory Resources

- **Erlang-C formula:** Calculator for waiting times in M/M/c queues
- **Coefficient of Variation:** Measure of relative variability (σ/μ)
- **Jackson Network Stability:** Requires all stage utilizations < 1

### Related Work in Warehouse Optimization

- **Picks Per Face (PPF):** Classic metric for rack reuse efficiency
- **Set Cover Order Fulfillment:** Minimize storage spaces per order (already implemented)
- **Zone-based Selection:** Balance VTM assignments across zones (relay-specific)

---

## Appendix A: Mathematical Derivations

### A.1: TPQ Target from Throughput Balance

**Given:**
- B bots
- O operators
- Target: Bot capacity = Operator capacity

**Bot capacity:**
```
Bot_capacity = B / (2×T_travel + T_service + T_load)
```

**Operator capacity:**
```
Operator_capacity = O / T_service
```

**For balance:**
```
B / (2×T_travel + T_service + T_load) = O / T_service

B × T_service = O × (2×T_travel + T_service + T_load)

B × T_service = 2×O×T_travel + O×T_service + O×T_load

(B - O) × T_service = 2×O×T_travel + O×T_load

T_travel = [(B - O) × T_service - O×T_load] / (2×O)
```

**Assuming T_load is small (≈ 10s):**
```
T_travel ≈ [(B - O) / (2×O)] × T_service

TPQ = (2×T_travel + T_service) / T_service
    = 1 + 2×T_travel / T_service
    = 1 + (B - O) / O
    = B / O
```

**Correction (accounting for load time):**
```
TPQ_target ≈ (B + O) / (2×O)
```

**This is the balanced throughput TPQ target.**

### A.2: Station Variance Bounds from CV

**Given:**
- Target TPQ = T
- Target system-wide CV across stations = 0.15

**Standard deviation:**
```
σ(TPQ_stations) = CV × mean(TPQ_stations)
                = 0.15 × T
```

**For T = 7:**
```
σ = 0.15 × 7 = 1.05
```

**Using normal distribution approximation:**
```
68% of stations within [T - 1σ, T + 1σ] = [5.95, 8.05] ≈ [6, 8]
95% of stations within [T - 2σ, T + 2σ] = [4.9, 9.1]
```

**Recommendation:** Target ±1σ bounds for practical systems.

---

## Appendix B: Simulation Pseudocode

### RTP System Selection

```python
def select_orders_rtp(pending_orders, active_tasks, max_orders, config):
    """Select orders using TPQ+VUT for RTP system"""

    # Calculate target TPQ from config
    target_tpq = (config.num_bots + config.num_operators) / (2 * config.num_operators)

    selected = []
    current_pool = list(active_tasks)

    for _ in range(max_orders):
        best_order = None
        best_score = -infinity

        for candidate in pending_orders:
            # Generate tasks for candidate
            tasks = generate_tasks_with_set_cover(candidate, current_pool)
            hypothetical_pool = current_pool + tasks

            # Calculate TPQ
            tpq = calculate_tpq(hypothetical_pool)
            tpq_score = -abs(tpq - target_tpq) * 50

            # Calculate per-station arrival CV
            by_station = group_by_station(hypothetical_pool)
            arrival_cv = mean([calculate_arrival_cv(tasks) for tasks in by_station])
            variability_score = -arrival_cv * 20

            # Calculate diversity
            distances = [calculate_distance(t) for t in hypothetical_pool]
            distance_cv = stdev(distances) / mean(distances)
            diversity_score = -abs(distance_cv - 0.4) * 10

            # Calculate depletion penalty
            depletion_penalty = calculate_depletion(candidate, state)

            # Combined score
            score = tpq_score + variability_score + diversity_score + depletion_penalty

            if score > best_score:
                best_score = score
                best_order = candidate

        if best_order:
            tasks = generate_tasks(best_order)
            selected.append(OrderSelection(best_order, tasks))
            current_pool.extend(tasks)
            pending_orders.remove(best_order)
        else:
            break

    # Validate reservations
    return validate_and_reserve(selected)
```

### Relay System Selection

```python
def select_totes_relay(pending_totes, active_totes, max_totes, config):
    """Select totes using two-stage TPQ for relay system"""

    # Calculate targets for both stages
    target_stage1 = config.num_vtms / config.num_operators
    target_stage2 = config.num_htms / config.num_operators

    selected = []
    current_pool = list(active_totes)

    for _ in range(max_totes):
        best_tote = None
        best_score = -infinity

        for candidate in pending_totes:
            hypothetical_pool = current_pool + [candidate]

            # Calculate two-stage TPQ
            relay_tpq = calculate_relay_tpq(hypothetical_pool)

            tpq_score = (-abs(relay_tpq.stage1 - target_stage1) * 50
                        -abs(relay_tpq.stage2 - target_stage2) * 50)

            # Rest similar to RTP...
            score = tpq_score + variability_score + diversity_score + depletion_penalty

            if score > best_score:
                best_score = score
                best_tote = candidate

        if best_tote:
            selected.append(best_tote)
            current_pool.append(best_tote)
            pending_totes.remove(best_tote)
        else:
            break

    return selected
```

---

**End of Design Document**
