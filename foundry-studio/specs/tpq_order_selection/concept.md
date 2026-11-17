# Transport Overhead Based Order Selection: Concept Document

**Version:** 3.0
**Date:** 2025-11-25
**Status:** Design Review
**Authors:** Design Session with Claude

---

## Nomenclature

| Term | Definition |
|------|------------|
| **TOR** | Transport Overhead Ratio: ((1+RF)×Travel + Service) / Service |
| **Target TOR** | B/O - the target TOR derived from resource balance (bots ÷ operators) |
| **Pool TOR** | Aggregate TOR of the current task pool |
| **PPF** | Picks Per Face - rack reuse efficiency metric |
| **RF** | Return Factor - return travel as fraction of outbound (default 1.0) |

**Note:** We use "TOR" as the single unifying term. Target TOR replaces "BOR", Pool TOR replaces "Pool Health".

---

## Executive Summary

This document presents a Transport Overhead Ratio (TOR) based order selection algorithm for Goods-to-Person (GTP) warehouse systems. The algorithm balances two objectives:

1. **PPF (Picks Per Face)** - rack reuse efficiency
2. **TOR (Transport Overhead)** - transport efficiency relative to productive work

The key insight is that **PPF must be satisfied first**, then TOR is optimized. The algorithm uses a threshold-based approach:

```
IF active racks in pool < minimum threshold
   → Select orders from active racks (PPF focus)
ELSE
   → Select orders to optimize Pool TOR (TOR focus)
```

Target TOR (= B/O) serves as a **control setpoint** derived from resource balance equations. The algorithm maintains Pool TOR near target while ensuring adequate rack reuse.

---

## Table of Contents

1. [Problem Statement](#1-problem-statement)
2. [Theoretical Foundation](#2-theoretical-foundation)
3. [System Architecture](#3-system-architecture)
4. [TOR Definition and Target TOR Derivation](#4-tor-definition-and-target-tor-derivation)
   - [4.5 Active Rack Handling (Piggybacking)](#45-active-rack-handling-piggybacking)
5. [Pool Tracking](#5-pool-tracking)
6. [Order Selection Algorithm](#6-order-selection-algorithm)
   - [6.1 Primary: PPF-First Threshold Approach](#61-primary-ppf-first-threshold-approach)
   - [6.2 Alternative: Picks/Travel Scoring](#62-alternative-pickstravel-scoring)
7. [Station-Level Flexibility](#7-station-level-flexibility)
8. [Configuration Parameters](#8-configuration-parameters)
9. [Corner Cases and Edge Conditions](#9-corner-cases-and-edge-conditions)
10. [FAQ](#10-faq)
11. [References](#11-references)

---

## 1. Problem Statement

### 1.1 Current State: PPF Degradation

Existing order selection algorithms optimize for Picks Per Face (PPF):

```
PPF = Total_picks / Number_of_storage_spaces_visited
```

**The Degradation Pattern:**

| Phase | Time | Behavior | Result |
|-------|------|----------|--------|
| Early | 0-30% | Select nearby A-items | High throughput |
| Mid | 30-70% | A-items depleting, mixed selection | Declining throughput |
| Late | 70-100% | Only distant C-items remain | Throughput collapse |

**Root Cause:** PPF is a static metric that ignores:
- Bot travel overhead (measures rack reuse, not efficiency)
- Temporal resource depletion
- Bot-operator capacity balance

### 1.2 Desired State

An order selection algorithm that:
1. Maintains consistent throughput across simulation lifetime
2. Creates opportunity for downstream scheduling optimization
3. Prevents premature depletion of nearby inventory
4. Balances bot and operator utilization dynamically
5. Provides per-station flexibility within system constraints

---

## 2. Theoretical Foundation

### 2.1 Warehouse as Queuing System

The RTP warehouse operates as a queuing system:

```
[Storage] → [Bots: Transport] → [Stations: Service] → [Complete]
              Pipeline delay        Servers (O)
```

**Key Parameters:**
- B = Number of bots (transport capacity)
- O = Number of operator stations (service capacity)
- T = Travel time (variable, depends on rack location)
- S = Service time (picking duration)

### 2.2 The Fundamental Difference from Classical Queuing

In classical queuing theory, arrival rates and service times are **given parameters**:

```
Classical: Given λ (arrival rate), μ (service rate)
           Calculate: utilization, waiting time, throughput
```

In warehouse order selection, travel time is a **decision variable**:

```
Warehouse: We CHOOSE which orders to select
           This DETERMINES travel time distribution
           Which AFFECTS achievable TOR
           Which INFLUENCES what we should select
```

This circular dependency means Target TOR is best understood as a **control setpoint** rather than a predicted outcome.

### 2.3 Travel Time Components

A bot cycle consists of:

```
Bot_cycle = T_outbound + T_queue_wait + T_service + T_return + T_load_unload
```

| Component | Known at Selection? | Controllable? |
|-----------|---------------------|---------------|
| T_outbound (rack → station) | ✅ Yes | ✅ Yes (rack choice) |
| T_queue_wait | ❌ No (emergent) | ⚠️ Indirectly |
| T_service | ✅ Yes | ✅ Yes (order choice) |
| T_return | ⚠️ Partially | ❌ No (policy-dependent) |
| T_load_unload | ✅ Yes (constant) | ❌ No |

### 2.4 The Return Factor (RF)

**Question:** How should we model return travel?

**The Problem with Hardcoded 2×:**

The naive assumption that return travel equals outbound travel breaks down in practice:

| Scenario | Actual Return | 2× Assumption |
|----------|---------------|---------------|
| Landing zones (racks returned closer) | < 1× outbound | Overestimates |
| Dynamic slotting (hot items migrate) | < 1× outbound | Overestimates |
| Bot chaining (picks up new rack on return) | ~0× | Grossly overestimates |
| One-way aisles / congestion | > 1× outbound | Underestimates |

**Solution: Return Factor (RF)**

Instead of hardcoding 2×, use a configurable Return Factor:

```
Travel_overhead = (1 + RF) × T_outbound
```

| RF Value | Total Travel | Use Case |
|----------|--------------|----------|
| 1.0 | 2.0× outbound | Same-location return (default) |
| 0.7 | 1.7× outbound | Mild landing zone policy |
| 0.4 | 1.4× outbound | Aggressive landing zones |
| 0.2 | 1.2× outbound | Close returns + frequent chaining |
| 1.2 | 2.2× outbound | Congested return paths |

**Default:** RF = 1.0 (conservative, equivalent to 2× assumption)

**Calibration:** RF should be calibrated from observed cycle times:
```
Observe:   actual_cycle = T_actual
Predicted: predicted_cycle = (1 + RF) × T_outbound + S
Calibrate: Minimize Σ(actual - predicted)²
```

**Advanced: Hybrid Formula**

For more precision, model return as proportional + fixed components:
```
T_return = α × T_outbound + β

Where:
- α = proportional factor (how much of outbound is repeated)
- β = fixed overhead (minimum travel, load/unload time)
```

| Policy | α | β | Example |
|--------|---|---|---------|
| Same location | 1.0 | 0 | Traditional warehouse |
| Landing zones | 0.3 | 10s | Modern automated facility |
| Heavy chaining | 0.1 | 5s | High-throughput operation |

### 2.5 Average Travel Distance Estimation

Since travel time depends on which orders we select, we use a geometric heuristic:

```
E[T_travel] = α × (warehouse_characteristic_dimension) / robot_speed
```

| Scenario | α value |
|----------|---------|
| Uniform inventory distribution | 0.5 |
| ABC slotting (hot items near stations) | 0.3-0.4 |
| Hot items distributed throughout | 0.5-0.6 |

This heuristic is acceptable because:
1. Bounded error (off by at most 2×, not 10×)
2. Control loop self-corrects if estimate is slightly wrong
3. Simple and explainable for capacity planning

---

## 3. System Architecture

### 3.1 Two-Layer Design

Order selection does not directly execute tasks. It operates as a **pool creation layer** feeding a downstream **task scheduling layer**:

```
┌─────────────────────────────────────────────────────────────┐
│  ORDER SELECTION LAYER                                      │
│  ─────────────────────                                      │
│  Responsibility:                                            │
│  • Create pool of task "opportunities"                      │
│  • Target pool COMPOSITION and SIZE                         │
│  • Enable scheduler flexibility                             │
│  • Prevent resource depletion                               │
│                                                             │
│  Does NOT:                                                  │
│  • Assign specific bots to tasks                            │
│  • Directly achieve TOR                                     │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
                    ┌───────────────┐
                    │   Task Pool   │
                    │               │
                    │  • Size ≥ min │
                    │  • TOR mix    │
                    │  • Station    │
                    │    coverage   │
                    └───────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│  TASK SCHEDULING LAYER (Downstream)                         │
│  ─────────────────────────────────                          │
│  Responsibility:                                            │
│  • Deterministic job shop scheduling                        │
│  • Assign bots to tasks                                     │
│  • ACHIEVE target TOR                                       │
│  • Real-time optimization                                   │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 Why This Separation Matters

Order selection's job is **not** to achieve TOR = 7.

Order selection's job is to **create a pool where the scheduler CAN achieve TOR = 7**.

This means the pool needs:
1. **Sufficient quantity** - scheduler has options
2. **Right distribution** - mix of low/high TOR tasks
3. **Station coverage** - no station starves
4. **Variance** - scheduler can adapt to system state

---

## 4. TOR Definition and Target TOR Derivation

### 4.1 TOR Formula

Transport Overhead Ratio measures overhead relative to productive work:

```
TOR = (Travel_overhead + Service_time) / Service_time
    = 1 + (Travel_overhead / Service_time)
```

Using the Return Factor (RF) from Section 2.4:

```
Travel_overhead = (1 + RF) × T_outbound
TOR = ((1 + RF) × T_outbound + T_service) / T_service
```

With default RF = 1.0:
```
TOR = (2 × T_outbound + T_service) / T_service
```

**Interpretation:**
- TOR = 1.0: Zero travel overhead (theoretical minimum)
- TOR = 3.0: Travel overhead is 2× productive work
- TOR = 7.0: Travel overhead is 6× productive work

**Impact of RF on TOR:**

For fixed T_outbound = 60s, T_service = 60s:

| RF | Travel Overhead | TOR |
|----|-----------------|-----|
| 1.0 | 120s | 3.0 |
| 0.7 | 102s | 2.7 |
| 0.4 | 84s | 2.4 |

### 4.2 Work-Weighted Aggregation

For a pool of tasks, use work-weighted (not count-weighted) aggregation:

```
Pool_TOR = Σ(Travel_i + Service_i) / Σ Service_i     ✅ Correct

NOT: (1/n) × Σ[(Travel_i + Service_i) / Service_i]  ❌ Wrong
```

**Why work-weighted:**
- Heavy jobs (more picks) impact throughput more
- Aligns with Little's Law
- Additive across subsystems

**Example:**

```
Rack 1: 360s travel, 120s service (10 picks) → Individual TOR = 4.0
Rack 2: 360s travel, 600s service (50 picks) → Individual TOR = 1.6

Work-weighted: (360+360+120+600) / (120+600) = 1440/720 = 2.0 ✅
Count-weighted: (4.0 + 1.6) / 2 = 2.8 ❌

Rack 2 dominates throughput (83% of picks), so 2.0 is correct.
```

### 4.3 Target TOR Derivation

**Goal:** Balance bot capacity with operator capacity.

Using Return Factor (RF):
```
Bot capacity:      B / ((1+RF)×T_outbound + T_service)
Operator capacity: O / T_service
```

Setting equal for balance:

```
B / ((1+RF)×T_outbound + T_service) = O / T_service

B × T_service = O × ((1+RF)×T_outbound + T_service)

Solving for TOR = ((1+RF)×T_outbound + T_service) / T_service:

Target TOR = B / O
```

**Result:** Target TOR equals bots per operator (B/O), regardless of RF.

**Key Insight:** RF affects what outbound distances are achievable for a given Target TOR, not the target itself:

```
Target T_outbound / T_service = (Target_TOR - 1) / (1 + RF)
```

**Example:** With Target TOR = 5.0, T_service = 60s:

| RF | Target T_outbound | Interpretation |
|----|-------------------|----------------|
| 1.0 | 120s | Can use racks 120s away |
| 0.7 | 141s | Can use racks 141s away (more flexibility) |
| 0.4 | 171s | Can use racks 171s away (even more flexibility) |

**Lower RF = can tolerate farther racks for same Target TOR.**

**Examples:**

| Bots (B) | Operators (O) | Target TOR (B/O) |
|----------|---------------|------------------|
| 20 | 4 | 5.0 |
| 28 | 4 | 7.0 |
| 12 | 4 | 3.0 |

### 4.4 Feasibility Check

The Target TOR must be achievable given warehouse geometry:

```
TOR_min = 1 + (2 × min_travel / avg_service)  // Only nearest racks
TOR_max = 1 + (2 × max_travel / avg_service)  // Only farthest racks
TOR_uniform = 1 + (2 × avg_travel / avg_service)  // Random selection
```

| Condition | Meaning |
|-----------|---------|
| Target TOR < TOR_min | Not enough bots; will always be bot-constrained |
| Target TOR > TOR_max | Too many bots; operators will bottleneck |
| TOR_min ≤ Target TOR ≤ TOR_max | Achievable; use as control setpoint |

### 4.5 Active Rack Handling (Piggybacking)

**Scenario:** A rack is already in the task pool (bot is fetching it, or it's at/near a station). A new order wants items from that same rack.

**Question:** What TOR should we assign to the new order?

#### The Two-Cost Principle

TOR serves **two different purposes**, and they need different treatments:

| Purpose | Which TOR to Use | Why |
|---------|------------------|-----|
| **Selection scoring** | Marginal (≈ 0) | Reflects true incremental cost, encourages batching |
| **Pool Health tracking** | Amortized (per-pick) | Prevents metric distortion |

#### Selection Scoring: Use Marginal TOR

When scoring a candidate order, if it picks from an already-active rack:

```
TOR_selection = 0  (or near-zero if there's minor repositioning)
```

This naturally implements the "Preserve Nearby" principle:
- Active racks are "free" to pick from
- System gravitates toward batching
- No penalty for adding picks to in-flight racks

#### Pool Health: Track at Rack Level

**Key insight:** Pool Health should track **racks**, not **orders**.

Instead of:
```
avg_TOR = Σ(order_TOR) / num_orders  ← distorted by batching
```

Use:
```
avg_TOR_per_pick = Σ(rack_travel_cost) / Σ(picks_on_active_racks)
```

This means:
- Each rack's travel is counted **once**
- Picks are accumulated **per rack**
- Pool Health reflects true system efficiency

**Example:**
```
Rack A: 60s travel, 5 picks → contributes 60s to numerator, 5 to denominator
Rack B: 40s travel, 2 picks → contributes 40s to numerator, 2 to denominator
Pool avg TOR per pick = (60 + 40) / (5 + 2) = 14.3s
```

#### Implementation Pattern

```java
public record ActiveRackStats(
    double totalTravelCost,    // Sum of all rack round-trip travels
    int totalPicks,            // Sum of all picks across racks
    Map<String, RackContribution> perRack
) {
    public double avgTorPerPick() {
        return totalPicks > 0 ? totalTravelCost / totalPicks : 0;
    }
}

// During order selection
double scoreTor = rack.isActive() ? 0.0 : calculateFullTor(rack, station);
```

#### Rack State Transitions

| Rack State | Selection TOR | Pool Health Contribution |
|------------|---------------|-------------------------|
| In storage | Full travel | Add to numerator & denominator on selection |
| Bot en route | 0 | Already counted |
| At station | 0 | Already counted |
| Returning (interceptable) | Intercept distance | Partial cost |
| Returning (past point-of-no-return) | Full travel | New journey |

#### Corner Cases

**1. Rack just selected (bot not yet dispatched):**
- Treat as active (TOR = 0 for additional picks)
- Bot assignment is commitment, not departure

**2. Rack at station, about to leave:**
- If picks can be added before departure: TOR = 0
- Requires coordination with station queue

**3. Multiple stations want same rack:**
- First station: full TOR
- Second station: TOR = station-to-station travel (not rack home to station)

---

## 5. Pool Tracking

### 5.1 Simple Pool State

Track the pool using just two running sums:

```java
public record PoolState(
    double totalTravelOverhead,  // Σ (1+RF) × T_outbound
    double totalServiceTime,     // Σ S
    int taskCount,
    int activeRackCount,         // Racks currently in pool
    Map<Integer, Integer> tasksPerStation
) {
    /** Current Pool TOR */
    public double poolTor() {
        return totalServiceTime > 0
            ? (totalTravelOverhead + totalServiceTime) / totalServiceTime
            : 1.0;
    }

    /** Projected Pool TOR if we add this candidate */
    public double projectedTor(double candidateTravel, double candidateService) {
        double newTravel = totalTravelOverhead + candidateTravel;
        double newService = totalServiceTime + candidateService;
        return (newTravel + newService) / newService;
    }
}
```

**Why this works:**
- Work-weighted TOR = Σ(travel + service) / Σ(service)
- Adding a candidate: just update numerator and denominator
- O(1) projection for any candidate

### 5.2 Minimum Thresholds

```java
int MIN_ACTIVE_RACKS = 3;           // Minimum racks with orders in pool (for PPF)
int MIN_POOL_SIZE = 2 × numBots;    // Minimum total tasks
int MIN_TASKS_PER_STATION = 4;      // Prevent station starvation
```

### 5.3 TOR Bounds

Pool TOR should stay within bounds of Target TOR:

```
Lower bound: Target TOR × 0.85  (don't over-consume nearby inventory)
Upper bound: Target TOR × 1.15  (don't hurt throughput)
```

**The "Preserve Nearby" Principle:**

When Pool TOR is low (below lower bound):
- Don't add more nearby racks
- This preserves nearby inventory for later
- Prevents late-simulation throughput collapse

---

## 6. Order Selection Algorithm

### 6.1 Primary: PPF-First Threshold Approach

**Core principle:** Ensure PPF (rack reuse) first, then optimize TOR.

```java
void selectOrders(PoolState pool, List<Order> pending) {
    while (pool.taskCount() < MIN_POOL_SIZE && !pending.isEmpty()) {

        // Phase 1: PPF Focus - ensure minimum active rack utilization
        if (pool.activeRackCount() < MIN_ACTIVE_RACKS) {
            selectFromActiveRacks(pool, pending);
        }
        // Phase 2: TOR Focus - optimize transport efficiency
        else {
            selectForTor(pool, pending);
        }
    }
}
```

**Phase 1: PPF Focus (selectFromActiveRacks)**

```java
void selectFromActiveRacks(PoolState pool, List<Order> pending) {
    // Find orders that can use already-active racks
    List<Order> ordersFromActiveRacks = pending.stream()
        .filter(o -> pool.hasActiveRack(o.rackId()))
        .toList();

    if (!ordersFromActiveRacks.isEmpty()) {
        // Prefer orders with most picks from active racks
        Order best = ordersFromActiveRacks.stream()
            .max(comparing(Order::pickCount))
            .get();
        addToPool(pool, best);
        pending.remove(best);
    } else {
        // No orders from active racks - need to add a new rack
        // Select rack with most available picks (maximize PPF potential)
        Order best = pending.stream()
            .max(comparing(o -> o.rack().availablePicks()))
            .get();
        addToPool(pool, best);
        pending.remove(best);
    }
}
```

**Phase 2: TOR Focus (selectForTor)**

```java
void selectForTor(PoolState pool, List<Order> pending, double targetTor) {
    Order best = null;
    double bestScore = Double.NEGATIVE_INFINITY;

    for (Order candidate : pending) {
        double projectedTor = pool.projectedTor(
            candidate.travelOverhead(),
            candidate.serviceTime()
        );

        // Score: how much closer to target does this get us?
        double currentGap = Math.abs(pool.poolTor() - targetTor);
        double projectedGap = Math.abs(projectedTor - targetTor);
        double score = currentGap - projectedGap;  // Positive = improvement

        // Bonus for using active racks (PPF)
        if (pool.hasActiveRack(candidate.rackId())) {
            score += ACTIVE_RACK_BONUS;
        }

        // Bonus for underserved stations
        if (pool.tasksForStation(candidate.stationId()) < MIN_TASKS_PER_STATION) {
            score += STATION_STARVING_BONUS;
        }

        if (score > bestScore) {
            bestScore = score;
            best = candidate;
        }
    }

    if (best != null) {
        addToPool(pool, best);
        pending.remove(best);
    }
}
```

### 6.2 Alternative: Picks/Travel Scoring

An alternative approach that combines PPF and TOR in a single score:

```java
double scoreCandidate(Order candidate, PoolState pool, double targetTor) {
    // Primary: Picks per travel (combines PPF + distance)
    double picksPerTravel = candidate.pickCount() / candidate.travelOverhead();

    // Guardrail: Check TOR bounds
    double projectedTor = pool.projectedTor(
        candidate.travelOverhead(),
        candidate.serviceTime()
    );

    double torPenalty = 0;
    if (projectedTor > targetTor * 1.15) {
        // Too far - would hurt throughput
        torPenalty = (projectedTor - targetTor * 1.15) * TOR_PENALTY_WEIGHT;
    } else if (projectedTor < targetTor * 0.85 && pool.taskCount() > MIN_POOL_SIZE) {
        // Too close - preserve nearby inventory
        torPenalty = (targetTor * 0.85 - projectedTor) * TOR_PENALTY_WEIGHT;
    }

    return picksPerTravel - torPenalty;
}
```

**Comparison:**

| Aspect | PPF-First (Primary) | Picks/Travel (Alternative) |
|--------|---------------------|---------------------------|
| Logic | Two-phase: PPF then TOR | Single combined score |
| Priority | PPF explicitly first | Implicit in formula |
| Simplicity | Easier to explain | More elegant mathematically |
| Production | Proven in practice | Theoretical |

### 6.3 Trigger Conditions

Order selection runs when:

```
shouldReplenish() =
    pool.size() < minPoolSize
    OR anyStationStarving()
    OR timeSinceLastReplenish > maxInterval
    OR schedulerRequestedMore
```

### 6.2 Algorithm Flow

```
┌─────────────────────────────────────────────────────────────┐
│  ORDER SELECTION LOOP                                       │
│                                                             │
│  1. ASSESS                                                  │
│     └─ Capture Pool Health snapshot (size, mean TOR,        │
│        variance, per-station breakdown)                     │
│                                                             │
│  2. DETERMINE NEEDS                                         │
│     └─ Calculate: tasks needed, TOR bias, starving stations │
│                                                             │
│  3. GENERATE CANDIDATES                                     │
│     └─ For each pending order × possible station:           │
│        compute TOR, service time, rack requirements         │
│                                                             │
│  4. SCORE CANDIDATES                                        │
│     └─ Apply scoring function based on current needs        │
│                                                             │
│  5. SELECT BEST                                             │
│     └─ Choose highest-scoring candidate                     │
│                                                             │
│  6. COMMIT                                                  │
│     └─ Add to pool, update tracking                         │
│                                                             │
│  7. REASSESS                                                │
│     └─ Re-evaluate Pool Health after each selection         │
│     └─ Loop until pool healthy or no good candidates        │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 6.3 Scoring Function

```
Score(candidate) =
    + BaseValue                           // Any task has value if needed
    + CompositionScore(candidate, bias)   // TOR bias alignment
    + StationScore(candidate, needs)      // Station balance
    + BudgetScore(candidate, system)      // System constraint
```

**Composition Score (TOR Bias):**

```
switch (bias) {
    PREFER_LOW:
        score += (BOR - candidateTOR) × 20

    PREFER_HIGH:
        score += (candidateTOR - BOR) × 15

    PREFER_VARIETY:
        score += |candidateTOR - poolMeanTOR| × 25

    BALANCED:
        score -= |candidateTOR - BOR| × 10
}
```

**Station Score:**

```
if (station in starvingStations):
    score += 50  // High priority

// Prefer underloaded stations
score += (avgStationLoad - thisStationLoad) × 5
```

**Budget Score:**

```
projectedSystemTOR = calculateIfAdded(candidate)

if (projectedSystemTOR > systemBudget):
    score -= (projectedSystemTOR - systemBudget) × 100  // Hard penalty
```

### 6.4 Reassessment After Each Selection

Critical: Pool state changes after each selection. Bias may shift.

```
Before: poolMeanTOR = 8.5, bias = PREFER_LOW
Select: Order with TOR = 5
After:  poolMeanTOR = 7.8, bias = PREFER_LOW (less urgent)

Before: poolMeanTOR = 7.8, bias = PREFER_LOW
Select: Order with TOR = 4
After:  poolMeanTOR = 7.1, bias = BALANCED (fixed!)

→ Next iteration uses BALANCED scoring, not PREFER_LOW
```

---

## 7. Station-Level Flexibility

### 7.1 Budget Constraint vs Rigid Targeting

**Rigid approach (NOT recommended):**
```
Every station must have TOR = 7
```

**Budget approach (RECOMMENDED):**
```
System constraint: Σ station_TOR ≤ B (total bot capacity)
Station flexibility: Individual TOR can vary within bounds
```

**Example:** 4 stations, 28 bots, BOR = 7

```
Rigid:
  Station 1: TOR = 7
  Station 2: TOR = 7
  Station 3: TOR = 7
  Station 4: TOR = 7

Flexible (budget):
  Station 1: TOR = 5  (critical orders, nearby racks)
  Station 2: TOR = 9  (absorbing overflow)
  Station 3: TOR = 7
  Station 4: TOR = 7
  Total: 28 ✅ (within budget)
```

### 7.2 Setting Station Bounds (±δ)

The allowed deviation δ around BOR.

**Derivation from utilization theory:**

Safe utilization range: ρ ∈ [0.70, 0.85]

If BOR corresponds to ρ = 0.75:
```
ρ = 0.70 → TOR = BOR × 1.07
ρ = 0.85 → TOR = BOR × 0.88
```

For BOR = 7: bounds ≈ [6.2, 7.5]

**Derivation from coefficient of variation:**

Industry guideline: CV across stations ≤ 0.15
```
σ = CV × mean = 0.15 × 7 = 1.05
```

**Derivation from throughput variance:**
```
TOR = 6 → throughput = 117% of baseline
TOR = 8 → throughput = 87.5% of baseline
```

±1 TOR gives ±15% throughput variance.

**Recommendation:**

| δ value | Utilization range | Throughput variance | Use case |
|---------|-------------------|---------------------|----------|
| ±0.5 | Very tight | ±7% | Strict balancing |
| **±1.0** | **Safe zone** | **±15%** | **Default** |
| ±1.5 | Wider | ±20% | High flexibility |
| ±2.0 | Very wide | ±25% | Critical orders |

### 7.3 Implementation

```
Constraints:
  HARD: Σ station_TOR_contribution ≤ B
  SOFT: station_TOR ∈ [BOR - δ, BOR + δ]

Scoring:
  if (violates HARD): score -= 100 × violation_amount
  if (violates SOFT): score -= 50 × violation_amount
  bonus for bringing station closer to BOR
```

---

## 8. Configuration Parameters

### 8.1 Core Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `bor` | B/O (auto) | Target TOR, null = auto-calculate |
| `returnFactor` | 1.0 | Return travel as fraction of outbound (RF). Range [0.2, 1.5] |
| `stationDeviation` | 1.0 | Allowed ±δ around BOR per station |
| `minPoolSize` | 2×B | Minimum tasks in pool |
| `minTasksPerStation` | 4 | Minimum per station before starving |

### 8.2 Scoring Weights

| Weight | Default | Description |
|--------|---------|-------------|
| `torBiasWeight` | 20.0 | Composition alignment |
| `stationStarvingBonus` | 50.0 | Priority for starving stations |
| `stationBalanceWeight` | 5.0 | Load balancing across stations |
| `budgetViolationPenalty` | 100.0 | Hard constraint penalty |

### 8.3 Pool Health Targets

| Parameter | Default | Description |
|-----------|---------|-------------|
| `minTorVariance` | 0.5 | Minimum σ for pool diversity |
| `torBiasThreshold` | 0.5 | Deviation from BOR before bias activates |

### 8.4 Configuration Record

```java
public record TorSelectionConfig(
    // Core
    Double bor,                    // null = auto (B/O)
    double returnFactor,           // RF, default 1.0, range [0.2, 1.5]
    double stationDeviation,       // ±δ, default 1.0
    int minPoolSize,               // default 2×B
    int minTasksPerStation,        // default 4

    // Scoring weights
    double torBiasWeight,          // default 20.0
    double stationStarvingBonus,   // default 50.0
    double stationBalanceWeight,   // default 5.0
    double budgetViolationPenalty, // default 100.0

    // Pool Health targets
    double minTorVariance,         // default 0.5
    double torBiasThreshold        // default 0.5
) {
    public static TorSelectionConfig defaults(int numBots, int numOperators) {
        return new TorSelectionConfig(
            (double) numBots / numOperators,  // bor
            1.0,                               // returnFactor (conservative default)
            1.0,                               // stationDeviation
            2 * numBots,                       // minPoolSize
            4,                                 // minTasksPerStation
            20.0, 50.0, 5.0, 100.0,           // weights
            0.5, 0.5                           // Pool Health targets
        );
    }

    /** Calculate TOR for a given outbound travel and service time */
    public double calculateTor(double tOutbound, double tService) {
        return ((1 + returnFactor) * tOutbound + tService) / tService;
    }
}
```

### 8.5 Return Factor Calibration

The Return Factor should be calibrated from observed data:

**Method 1: Offline Calibration**
```java
// Collect observed cycle times
List<CycleObservation> observations = collectCycleData();

// Find RF that minimizes prediction error
double bestRf = 1.0;
double minError = Double.MAX_VALUE;

for (double rf = 0.2; rf <= 1.5; rf += 0.1) {
    double error = observations.stream()
        .mapToDouble(obs -> {
            double predicted = (1 + rf) * obs.outboundTime() + obs.serviceTime();
            return Math.pow(obs.actualCycle() - predicted, 2);
        })
        .sum();

    if (error < minError) {
        minError = error;
        bestRf = rf;
    }
}
```

**Method 2: Online Adaptive**
```java
// Exponential moving average of observed vs predicted
double observedAvgCycle = emaObserved.get();
double predictedAvgCycle = emaPredicted.get();

// Adjust RF if consistent bias detected
if (observedAvgCycle < predictedAvgCycle * 0.9) {
    // Actual cycles shorter than predicted → reduce RF
    config = config.withReturnFactor(config.returnFactor() * 0.95);
} else if (observedAvgCycle > predictedAvgCycle * 1.1) {
    // Actual cycles longer than predicted → increase RF
    config = config.withReturnFactor(config.returnFactor() * 1.05);
}
```

**Recommended Calibration Frequency:**
- Initial: Calibrate from first 1000 cycles
- Ongoing: Re-evaluate weekly or after operational changes
- Alert: If RF drifts beyond [0.5, 1.2], investigate root cause

---

## 9. Corner Cases and Edge Conditions

### 9.1 Insufficient Nearby Inventory

**Scenario:** All remaining orders require distant racks; cannot achieve BOR.

**Detection:**
```
bestCandidateTOR > BOR + 2 for all candidates
```

**Response:**
1. Relax BOR temporarily
2. Log warning for capacity planning
3. Continue with best available (minimize deviation)

```java
if (allCandidatesAboveTarget) {
    effectiveBor = min(bestAvailableTOR, BOR × 1.3);
    logWarning("BOR target relaxed due to inventory constraints");
}
```

### 9.2 Station Starvation Conflict

**Scenario:** Starving station only has high-TOR candidates; adding them would exceed budget.

**Resolution priority:**
1. Prevent starvation (operator idle time is waste)
2. Accept temporary budget overage
3. Compensate by selecting lower-TOR for other stations

```java
if (station.isStarving() && onlyHighTorAvailable) {
    // Allow budget overage for starving station
    tempBudgetRelaxation = true;
    // Flag other stations to prefer low TOR
    compensationMode = true;
}
```

### 9.3 Pool Oscillation

**Scenario:** Pool alternates between PREFER_HIGH and PREFER_LOW on consecutive selections.

**Cause:** Threshold too sensitive, each selection overcorrects.

**Solution:** Add hysteresis to bias transitions.

```java
// Don't flip bias unless clearly needed
if (currentBias == PREFER_LOW && poolMeanTOR < BOR - 0.3) {
    // Only switch to PREFER_HIGH if clearly below target
    newBias = PREFER_HIGH;
} else if (currentBias == PREFER_HIGH && poolMeanTOR > BOR + 0.3) {
    newBias = PREFER_LOW;
} else {
    newBias = currentBias;  // Maintain current bias
}
```

### 9.4 Late Simulation Inventory Depletion

**Scenario:** Near end of simulation, nearby racks depleted, only distant remain.

**Response:**
1. Expected behavior if PREFER_HIGH was used appropriately
2. If not, this indicates earlier over-consumption of nearby inventory
3. Accept higher TOR in late phase; focus on completion

```java
if (simulationProgress > 0.8 && poolMeanTOR > BOR + 1) {
    // Late phase: prioritize completion over BOR target
    scoringMode = COMPLETION_PRIORITY;
}
```

### 9.5 Demand Surge for Single Station

**Scenario:** Sudden influx of orders all assigned to one station.

**Detection:**
```
stationTaskCount[X] > 2 × avgTaskCount
```

**Response:**
1. Temporarily block additional assignments to overloaded station
2. Redirect to other stations if possible
3. If not possible, accept imbalance but flag for monitoring

### 9.6 Empty Pending Orders

**Scenario:** No pending orders to select from.

**Response:**
1. Pool cannot be replenished
2. Scheduler works with existing pool until exhausted
3. Signal upstream (order generation) if persistent

---

## 10. FAQ

### Q1: Why not just minimize travel distance?

**A:** Minimizing distance depletes nearby inventory early. Late simulation has only distant racks, causing performance collapse. TOR with PREFER_HIGH bias intentionally uses some distant racks early to preserve nearby inventory.

### Q2: Why use a Return Factor instead of hardcoded 2× travel?

**A:** The hardcoded 2× assumption breaks down in practice:
- Landing zones return racks closer → actual < 2×
- Bot chaining picks up new racks on return → actual << 2×
- Congestion on return paths → actual > 2×

The Return Factor (RF) provides flexibility:
- Default RF = 1.0 gives 2× (conservative)
- Calibrate RF from observed cycle times
- Different warehouses can have different RF values

See Section 2.4 and 8.5 for RF details and calibration methods.

### Q3: How does queue waiting factor in?

**A:** Queue waiting is NOT included in TOR calculation because:
- Cannot predict at selection time
- Is emergent from arrival patterns
- Is addressed indirectly by:
  - BOR (controls utilization)
  - Station balance (prevents hotspots)
  - Pool variance (enables scheduler smoothing)

### Q4: Why does the pool need TOR variance?

**A:** A uniform pool gives the scheduler no flexibility:

```
Uniform pool [7, 7, 7, 7]:
  System running hot? → No low-TOR options
  System has slack? → No high-TOR to clear backlog

Varied pool [5, 6, 7, 8, 9]:
  System running hot? → Pick TOR=5, catch up
  System has slack? → Pick TOR=9, clear backlog
```

### Q5: What if BOR is outside achievable range?

**A:**
- BOR < TOR_min: System is bot-constrained; add more bots or accept lower throughput
- BOR > TOR_max: System has excess bots; operators will bottleneck regardless

Selection algorithm will do its best but cannot overcome physical constraints.

### Q6: How does this interact with order priority?

**A:** Priority can be added as a scoring component:

```java
score += order.priority() × priorityWeight;
score -= max(0, currentTime - order.deadline()) × urgencyWeight;
```

TOR ensures throughput; priority overlay ensures business rules.

### Q7: Why reassess after each selection?

**A:** Pool state changes with each addition. A selection that made sense when pool was high-TOR might be wrong after adding a low-TOR task. Reassessment ensures scoring reflects current reality.

### Q8: How do we know the algorithm is working?

**A:** Monitor these Pool Health metrics:

| Metric | Healthy Range | Problem Indicator |
|--------|---------------|-------------------|
| Pool mean TOR | BOR ± 0.5 | Persistent deviation |
| Pool variance | > 0.5 | Too low = no flexibility |
| Station TOR range | BOR ± δ | Station outside bounds |
| Late/early throughput ratio | > 0.8 | Performance degradation |
| Tasks per station | ≥ minTasksPerStation | Starvation |

### Q9: Can this work with Relay systems?

**A:** Yes, with modifications:
- Two-stage TOR (VTM + HTM)
- Zone-aware Pool Health management
- Relay position capacity constraints

See companion document: [relay_concept.md](./relay_concept.md)

Core principles (Pool Health, bias-driven selection) remain the same.

### Q10: What's the relationship to PPF?

**A:** PPF (Picks Per Face) and TOR are complementary:
- PPF measures rack reuse efficiency
- TOR measures transport overhead relative to productive work

High PPF is good but insufficient; a high-PPF batch from distant racks has poor TOR.

### Q11: How should I handle orders that pick from racks already in the task pool?

**A:** Use **marginal TOR for selection**, **amortized TOR for Pool Health**.

When a rack is already active (bot fetching, at station, etc.):
- **Selection scoring:** TOR = 0 (additional picks are "free")
- **Pool Health:** Track at rack level: `avg_TOR = Σ(rack_travel) / Σ(picks_on_active_racks)`

This encourages batching (good for throughput) while maintaining accurate Pool Health metrics.

See Section 4.5 "Active Rack Handling (Piggybacking)" for detailed state transitions and implementation patterns.

---

## 11. References

### Academic Foundations

1. **Little, J.D.C.** (1961). "A proof for the queuing formula: L = λW." *Operations Research*, 9(3), 383-387.
   - Fundamental relationship between queue length, arrival rate, and waiting time

2. **Kingman, J.F.C.** (1961). "The single server queue in heavy traffic." *Mathematical Proceedings of the Cambridge Philosophical Society*, 57(4), 902-904.
   - VUT equation: W_q ≈ (V × U × T) / (2 × (1 - U))

3. **Gross, D., & Harris, C.M.** (2008). *Fundamentals of Queueing Theory* (4th ed.). Wiley.
   - M/M/c queues, utilization bounds, multi-server systems

### Warehouse Operations

4. **De Koster, R., Le-Duc, T., & Roodbergen, K.J.** (2007). "Design and control of warehouse order picking: A literature review." *European Journal of Operational Research*, 182(2), 481-501.
   - Comprehensive review of order picking optimization

5. **Gu, J., Goetschalckx, M., & McGinnis, L.F.** (2007). "Research on warehouse operation: A comprehensive review." *European Journal of Operational Research*, 177(1), 1-21.
   - Warehouse design and operational policies

### Control Theory Perspective

6. **Hopp, W.J., & Spearman, M.L.** (2011). *Factory Physics* (3rd ed.). Waveland Press.
   - Production system dynamics, WIP management, bottleneck analysis

---

## Appendix A: Derivation of BOR

**Given:**
- B bots
- O operators
- RF = Return Factor
- Goal: Bot capacity = Operator capacity

**Bot capacity (racks per unit time):**
```
Bot_capacity = B / avg_bot_cycle
             = B / ((1+RF)×T_outbound + T_service)
```

**Operator capacity (racks per unit time):**
```
Operator_capacity = O / T_service
```

**Balance equation:**
```
B / ((1+RF)×T_outbound + T_service) = O / T_service

B × T_service = O × ((1+RF)×T_outbound + T_service)

B × T_service = (1+RF)×O×T_outbound + O×T_service

(B - O) × T_service = (1+RF)×O×T_outbound

T_outbound / T_service = (B - O) / ((1+RF)×O)
```

**TOR definition:**
```
TOR = ((1+RF)×T_outbound + T_service) / T_service
    = (1+RF)×(T_outbound / T_service) + 1
    = (1+RF)×(B - O)/((1+RF)×O) + 1
    = (B - O)/O + 1
    = (B - O + O)/O
    = B/O
```

**Result:** BOR = B/O (independent of RF) ∎

**Note:** RF cancels out in the BOR derivation. This means the target TOR is always B/O regardless of return policy. What RF affects is the *achievable outbound distances* for that target:

```
Achievable T_outbound = (BOR - 1) × T_service / (1 + RF)
```

Lower RF → can reach farther racks while maintaining same BOR target.

---

## Appendix B: Station Deviation Bounds

**From utilization theory:**

Target utilization ρ = 0.75 (industry standard for stable queues)
Safe range: ρ ∈ [0.70, 0.85]

Utilization scales inversely with TOR (higher TOR = lower operator utilization):
```
ρ_actual = ρ_target × (BOR / TOR_actual)
```

For ρ to stay in [0.70, 0.85]:
```
0.70 ≤ 0.75 × (7 / TOR) ≤ 0.85

Solving:
TOR_min = 7 × 0.75 / 0.85 = 6.18
TOR_max = 7 × 0.75 / 0.70 = 7.50
```

**Bounds ≈ [6, 8] or ±1 around BOR.**

**From coefficient of variation:**

Industry guideline: CV ≤ 0.15 for balanced systems
```
σ = CV × μ = 0.15 × 7 = 1.05 ≈ 1
```

**±1σ corresponds to ±1 TOR bounds.**

---

**End of Document**
