# Transport Overhead in Relay Systems: Concept Document

**Version:** 2.0
**Date:** 2025-11-25
**Status:** Design Review
**Authors:** Design Session with Claude
**Prerequisites:** [concept.md](./concept.md) - Core TOR framework

---

## Nomenclature (Relay-Specific)

| Term | Definition |
|------|------------|
| **TOR_vtm** | VTM loop overhead: ((1+RF_vtm)×T_vtm) / S |
| **TOR_htm** | HTM loop overhead: ((1+RF_htm)×T_htm) / S |
| **Target TOR_vtm** | V/O - VTMs per operator |
| **Target TOR_htm** | H/O - HTMs per operator |
| **Pool TOR** | Aggregate TOR of current task pool (VTM + HTM components) |
| **PPF** | Picks Per Face - tote reuse efficiency |

See [concept.md](./concept.md) for base terminology (TOR, Target TOR, RF).

---

## Executive Summary

This document extends the TOR framework to Relay warehouse systems. Unlike RTP's single transport loop, Relay operates as a **two-loop decoupled network**:

- **VTM Loop:** Zone-bound, rack ↔ relay
- **HTM Loop:** Global, relay ↔ station

The key insight is that **PPF (tote reuse) must be satisfied first**, then TOR is optimized for both loops. The algorithm uses the same PPF-first threshold approach as RTP:

```
IF active totes in pool < minimum threshold
   → Select orders from active totes (PPF focus)
ELSE
   → Select orders to optimize Pool TOR (TOR focus, both loops)
```

Target TOR is derived separately for each loop: Target TOR_vtm = V/O, Target TOR_htm = H/O.

---

## Table of Contents

1. [Structural Comparison: RTP vs Relay](#1-structural-comparison-rtp-vs-relay)
2. [The Two-Loop Architecture](#2-the-two-loop-architecture)
3. [Transport Overhead in Relay](#3-transport-overhead-in-relay)
   - [3.5 Active Tote Handling (Piggybacking)](#35-active-tote-handling-piggybacking)
   - [3.6 Return Factors for Relay (RF_vtm, RF_htm)](#36-return-factors-for-relay-rf_vtm-rf_htm)
4. [Balance Equations Derivation](#4-balance-equations-derivation)
5. [Pool Health: Two-Dimensional](#5-pool-health-two-dimensional)
6. [Order Selection for Relay](#6-order-selection-for-relay)
7. [Zone Management](#7-zone-management)
8. [Relay Buffer Dynamics](#8-relay-buffer-dynamics)
9. [Configuration Parameters](#9-configuration-parameters)
10. [Corner Cases](#10-corner-cases)
11. [FAQ](#11-faq)

---

## 1. Structural Comparison: RTP vs Relay

### 1.1 RTP: Single Loop

```
┌─────────────────────────────────────────────────────────────┐
│                      RTP SYSTEM                             │
│                                                             │
│    [Rack A]  [Rack B]  [Rack C]  [Rack D]                  │
│        \        |        |        /                         │
│         \       |        |       /                          │
│          ↘      ↓        ↓      ↙                           │
│              [Bot Pool]                                     │
│          ↙      ↓        ↓      ↘                           │
│         /       |        |       \                          │
│        ↓        ↓        ↓        ↓                         │
│   [Station 1] [Station 2] [Station 3] [Station 4]          │
│                                                             │
│   One loop: Rack ↔ Bot ↔ Station                           │
│   One decision: Which rack to which station                │
│   One TOR: (2×Travel + Service) / Service                  │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Characteristics:**
- Direct rack-to-station transport
- Bot sees full journey
- Station sees full overhead
- Single optimization problem

### 1.2 Relay: Two Decoupled Loops

```
┌─────────────────────────────────────────────────────────────┐
│                     RELAY SYSTEM                            │
│                                                             │
│         LOOP 1 (VTM)              │         LOOP 2 (HTM)    │
│         Zone-bound                │         Global          │
│                                   │                         │
│   Zone A    Zone B    Zone C      │                         │
│  ┌──────┐  ┌──────┐  ┌──────┐    │                         │
│  │Racks │  │Racks │  │Racks │    │                         │
│  └──┬───┘  └──┬───┘  └──┬───┘    │                         │
│     │         │         │        │                         │
│     ↕VTM_A    ↕VTM_B    ↕VTM_C   │                         │
│     │         │         │        │                         │
│  ┌──┴───┐  ┌──┴───┐  ┌──┴───┐    │                         │
│  │Relay │  │Relay │  │Relay │ ←──┼── Coupling Point        │
│  │  A   │  │  B   │  │  C   │    │                         │
│  └──┬───┘  └──┬───┘  └──┬───┘    │                         │
│     └─────────┼─────────┘        │                         │
│               │                  │                         │
│               ↓                  │                         │
│          [HTM Pool] ─────────────┘                         │
│               │                                             │
│     ┌─────────┼─────────┐                                  │
│     ↓         ↓         ↓                                  │
│  [Stn 1]  [Stn 2]  [Stn 3]                                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Characteristics:**
- Two-stage transport with relay buffer
- VTM sees only rack ↔ relay (zone-bound)
- HTM sees only relay ↔ station (global)
- Station sees only HTM overhead
- Two partially independent optimization problems

### 1.3 Key Differences Summary

| Aspect | RTP | Relay |
|--------|-----|-------|
| Transport stages | 1 | 2 |
| Robot types | 1 (Bot) | 2 (VTM, HTM) |
| Robot scope | Global | VTM: Zone-bound, HTM: Global |
| Buffering | Station queue only | Relay buffer + station queue |
| TOR components | Single | TOR_vtm + TOR_htm |
| Target TOR | B/O | V/O and H/O |
| Station visibility | Full overhead | HTM overhead only |
| Optimization | Single decision | Two decoupled decisions |

---

## 2. The Two-Loop Architecture

### 2.1 Loop 1: VTM (Vertical Tote Mover)

**Scope:** Zone-bound (each VTM serves one zone)

**Operation:**
```
Forward: Rack → Pick tote → Travel to relay → Deposit tote
Return:  Relay → Pick empty tote → Travel to rack → Return tote
```

**Cycle time:**
```
T_vtm_cycle = 2 × T_vtm + load_unload
            ≈ 2 × distance(rack, relay) / vtm_speed
```

**What VTM knows:**
- Rack locations in its zone
- Relay positions in its zone
- Current zone workload

**What VTM does NOT know:**
- Which station needs the tote
- Service time at station
- HTM availability

### 2.2 Loop 2: HTM (Horizontal Tote Mover)

**Scope:** Global (any HTM can serve any relay/station)

**Operation:**
```
Forward: Relay → Pick tote → Travel to station → Wait for operator → Picking
Return:  Station → Travel to relay → Deposit tote
```

**Cycle time:**
```
T_htm_cycle = 2 × T_htm + S + load_unload
            ≈ 2 × distance(relay, station) / htm_speed + service_time
```

**What HTM knows:**
- All relay positions
- All station positions
- Current tote's pick requirements

**What HTM does NOT know:**
- How "expensive" the tote was to retrieve (VTM cost)
- Zone workload distribution

### 2.3 The Coupling Point: Relay Buffer

The relay layer serves as a **decoupling buffer** between loops:

```
┌─────────────────────────────────────────────────────────────┐
│  RELAY BUFFER DYNAMICS                                      │
│                                                             │
│  Inflow:  VTM deposits totes (rate = VTM capacity)         │
│  Outflow: HTM picks totes (rate = HTM capacity)            │
│  Buffer:  Finite relay positions per zone                   │
│                                                             │
│  States:                                                    │
│  ├── Buffer empty: HTMs starve, wait for VTM               │
│  ├── Buffer partial: Normal operation                       │
│  └── Buffer full: VTMs block, wait for HTM                 │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Decoupling benefits:**
1. VTM can work ahead (pre-stage totes)
2. HTM doesn't wait for rack retrieval
3. Absorbs temporal variability between loops

**Decoupling challenges:**
1. Information hiding between loops
2. Buffer overflow/underflow risks
3. Two separate optimization problems

---

## 3. Transport Overhead in Relay

### 3.1 Two-Component TOR

Unlike RTP's single TOR, Relay has two components:

```
TOR_vtm = ((1 + RF_vtm) × T_vtm) / S     [VTM overhead per unit productive work]
TOR_htm = ((1 + RF_htm) × T_htm) / S     [HTM overhead per unit productive work]
TOR_total = TOR_vtm + TOR_htm + 1
```

Where RF_vtm and RF_htm are Return Factors for each loop (see Section 3.6).

With default RF = 1.0 for both:
```
TOR_vtm = (2 × T_vtm) / S
TOR_htm = (2 × T_htm) / S
```

**Why normalize to service time S?**

Even though VTM doesn't interact with operators, the operator pace sets system throughput. All overhead is measured relative to productive work for consistency.

### 3.2 Per-Task TOR Calculation

For a specific tote/task:

```
Given:
  - Rack at position (r_x, r_y) in Zone Z
  - Zone Z relay at position (z_x, z_y)
  - Assigned to Station at position (s_x, s_y)
  - Service time S (based on picks in tote)
  - RF_vtm, RF_htm = Return Factors for each loop

T_vtm = distance((r_x,r_y), (z_x,z_y)) / vtm_speed
T_htm = distance((z_x,z_y), (s_x,s_y)) / htm_speed

TOR_vtm = (1 + RF_vtm) × T_vtm / S
TOR_htm = (1 + RF_htm) × T_htm / S
TOR_total = TOR_vtm + TOR_htm + 1
```

### 3.3 Aggregation (Work-Weighted)

For a pool of tasks:

```
Pool TOR_vtm = Σ((1+RF_vtm)×T_vtm_i) / Σ S_i
Pool TOR_htm = Σ((1+RF_htm)×T_htm_i) / Σ S_i
Pool TOR_total = Pool TOR_vtm + Pool TOR_htm + 1
```

### 3.4 Critical Insight: VTM Cost is Per-Tote

**RTP:** More picks per rack → overhead amortized → lower TOR

**Relay:** VTM moves a TOTE, not picks. Whether tote has 1 pick or 6 picks:
```
VTM travel time is IDENTICAL
VTM does NOT benefit from batching
```

**Implication:** TOR_vtm is determined by rack location, not by pick count. More picks per tote reduces TOR_htm (amortizes HTM travel + service) but does NOT reduce TOR_vtm.

### 3.5 Active Tote Handling (Piggybacking)

**Scenario:** A tote is already active (at relay, in HTM transit, or at station). A new order wants items from that same tote.

**Question:** What TOR should we assign to the new order?

#### The Two-Cost Principle (Same as RTP)

TOR serves **two different purposes**, requiring different treatments:

| Purpose | Which TOR to Use | Why |
|---------|------------------|-----|
| **Selection scoring** | Marginal (≈ 0) | Reflects true incremental cost |
| **Pool Health tracking** | Amortized (per-pick) | Prevents metric distortion |

#### Relay-Specific States

In a two-loop system, "active" has more states than RTP:

| Tote State | TOR_vtm | TOR_htm | Rationale |
|------------|---------|---------|-----------|
| In rack (storage) | Full | Full | New journey required |
| VTM fetching | 0 | Full | VTM journey paid, HTM not started |
| At relay (waiting) | 0 | Full | VTM done, HTM pending |
| HTM fetching | 0 | 0 | Both journeys in progress |
| At station | 0 | 0 | Both journeys paid |
| HTM returning | 0 | Intercept* | VTM done, HTM may be interceptable |
| VTM returning | Intercept* | Full | May intercept VTM before rack |

*Intercept cost = remaining travel if tote can be redirected

#### Selection Scoring by State

```java
double scoreTor(ToteCandidate candidate) {
    Tote tote = candidate.tote();

    // TOR_vtm component
    double torVtm = switch (tote.getState()) {
        case IN_STORAGE -> calculateFullTorVtm(tote, candidate.zone());
        case VTM_FETCHING, AT_RELAY, HTM_FETCHING, AT_STATION -> 0.0;
        case VTM_RETURNING -> canInterceptVtm(tote)
            ? calculateInterceptTorVtm(tote)
            : calculateFullTorVtm(tote, candidate.zone());
        case HTM_RETURNING -> 0.0;  // VTM already done
    };

    // TOR_htm component
    double torHtm = switch (tote.getState()) {
        case IN_STORAGE, VTM_FETCHING, AT_RELAY ->
            calculateFullTorHtm(tote, candidate.station());
        case HTM_FETCHING, AT_STATION -> 0.0;
        case HTM_RETURNING -> canInterceptHtm(tote)
            ? calculateInterceptTorHtm(tote, candidate.station())
            : calculateFullTorHtm(tote, candidate.station());
        case VTM_RETURNING -> calculateFullTorHtm(tote, candidate.station());
    };

    return torVtm + torHtm;
}
```

#### Pool Health: Track at Tote Level

Like RTP, track costs at the **tote level**, not order level:

```java
public record ActiveToteStats(
    // VTM loop
    double totalVtmTravelCost,    // Sum of tote VTM round-trips
    int totalPicksVtmActive,      // Picks on VTM-active totes

    // HTM loop
    double totalHtmTravelCost,    // Sum of tote HTM round-trips
    int totalPicksHtmActive,      // Picks on HTM-active totes

    // Per-tote breakdown
    Map<String, ToteContribution> perTote
) {
    public double avgTorVtmPerPick() {
        return totalPicksVtmActive > 0
            ? totalVtmTravelCost / totalPicksVtmActive : 0;
    }

    public double avgTorHtmPerPick() {
        return totalPicksHtmActive > 0
            ? totalHtmTravelCost / totalPicksHtmActive : 0;
    }
}
```

#### Why This Matters More in Relay

Piggybacking is even more valuable in Relay because:

1. **Two travel costs to avoid:** Both VTM and HTM travel can be zero for active totes
2. **Tote capacity is limited:** Totes hold fewer picks than racks, so batching is critical
3. **Relay buffer creates opportunity:** Totes at relay are "free" targets for new picks

**Example:**
```
Tote at relay (VTM done, waiting for HTM):
- New pick from this tote: TOR_vtm = 0, TOR_htm = full
- vs new tote from storage: TOR_vtm = full, TOR_htm = full

Saving = TOR_vtm (could be 50% of total overhead)
```

### 3.6 Return Factors for Relay (RF_vtm, RF_htm)

Unlike RTP's single Return Factor, Relay systems have **two independent loops** with potentially different return characteristics.

#### Why Separate Return Factors?

| Loop | Return Behavior | Typical RF |
|------|-----------------|------------|
| VTM | Returns tote to same rack location (consistent) | 0.8 - 1.0 |
| HTM | May use station landing zones, frequent chaining | 0.4 - 0.8 |

**VTM Loop:**
- VTMs typically return totes to the same rack position
- Less opportunity for landing zones (racks have fixed positions)
- RF_vtm ≈ 1.0 is often accurate

**HTM Loop:**
- HTMs can return to relay landing zones (not original relay position)
- More opportunity for chaining (pick up next tote on return)
- RF_htm often < 1.0

#### Configuration Options

**Option 1: Shared RF** (simpler)
```java
double returnFactor = 0.8;  // Applied to both loops
```

**Option 2: Per-Loop RF** (more accurate)
```java
double returnFactorVtm = 1.0;   // VTM returns to same rack
double returnFactorHtm = 0.6;   // HTM uses landing zones
```

#### Impact on Target TOR

Just like RTP, the Return Factors affect achievable distances, not Target TOR values:

```
Target TOR_vtm = V / O  (independent of RF_vtm)
Target TOR_htm = H / O  (independent of RF_htm)

Achievable T_vtm = (Target_TOR_vtm × S) / (1 + RF_vtm)
Achievable T_htm = (Target_TOR_htm × S) / (1 + RF_htm)
```

**Example:** V=8, H=4, O=4, S=60s
```
Target TOR_vtm = 2.0, Target TOR_htm = 1.0

With RF_vtm=1.0, RF_htm=0.6:
  Achievable T_vtm = (2.0 × 60) / 2.0 = 60s
  Achievable T_htm = (1.0 × 60) / 1.6 = 37.5s

With RF_vtm=1.0, RF_htm=1.0 (conservative):
  Achievable T_vtm = (2.0 × 60) / 2.0 = 60s
  Achievable T_htm = (1.0 × 60) / 2.0 = 30s

Lower RF_htm → can use relays farther from stations
```

#### Calibration

Calibrate each loop independently:

```java
// VTM loop calibration
double rfVtm = calibrateReturnFactor(vtmCycleObservations);

// HTM loop calibration
double rfHtm = calibrateReturnFactor(htmCycleObservations);
```

Typically: RF_vtm > RF_htm because VTM has fewer return optimization opportunities.

---

## 4. Balance Equations Derivation

### 4.1 Capacity Analysis

Each stage has capacity measured in totes per unit time:

| Stage | Work per Tote | Capacity | Capacity Formula |
|-------|---------------|----------|------------------|
| VTM | (1+RF_vtm) × T_vtm | V | V / ((1+RF_vtm) × T_vtm) |
| HTM | (1+RF_htm) × T_htm | H | H / ((1+RF_htm) × T_htm) |
| Operator | S | O | O / S |

Note: HTM work excludes service time (operator does that work).

With default RF = 1.0 for both loops, this simplifies to the 2× formulas.

### 4.2 System Throughput Constraints

For system throughput λ (totes per unit time):

```
VTM constraint:      λ × (1+RF_vtm) × T_vtm ≤ V   → λ ≤ V / ((1+RF_vtm) × T_vtm)
HTM constraint:      λ × (1+RF_htm) × T_htm ≤ H   → λ ≤ H / ((1+RF_htm) × T_htm)
Operator constraint: λ × S ≤ O                     → λ ≤ O / S

Maximum throughput: λ_max = min(all three)
```

### 4.3 Utilization Equations

```
ρ_vtm = λ × (1+RF_vtm) × T_vtm / V
ρ_htm = λ × (1+RF_htm) × T_htm / H
ρ_op  = λ × S / O
```

### 4.4 Balance Condition

For balanced system (no single bottleneck): ρ_vtm = ρ_htm = ρ_op

Setting VTM utilization = Operator utilization:
```
λ × (1+RF_vtm) × T_vtm / V = λ × S / O

(1+RF_vtm) × T_vtm / V = S / O

(1+RF_vtm) × T_vtm / S = V / O

TOR_vtm = V / O
```

Setting HTM utilization = Operator utilization:
```
λ × (1+RF_htm) × T_htm / H = λ × S / O

(1+RF_htm) × T_htm / H = S / O

(1+RF_htm) × T_htm / S = H / O

TOR_htm = H / O
```

**Note:** Return Factors cancel out in the Target TOR derivation, just like in RTP.

### 4.5 Target TOR Values

```
Target TOR_vtm = V / O
Target TOR_htm = H / O
```

**Example:** 8 VTMs, 4 HTMs, 4 Operators
```
Target TOR_vtm = 8/4 = 2.0
Target TOR_htm = 4/4 = 1.0
Target TOR_total = 2.0 + 1.0 + 1 = 4.0
```

### 4.6 Why Service Time Appears in VTM Balance

The operator sets the system pace. Even though VTM doesn't interact with operators:

1. Operators consume totes at rate O/S
2. VTMs must deliver totes at this rate
3. Therefore: V/(2×T_vtm) ≥ O/S
4. Rearranging: TOR_vtm = 2×T_vtm/S ≤ V/O

**Analogy:** In an assembly line, Station 1 doesn't know about Station 3's process, but must produce at Station 3's consumption rate.

---

## 5. Pool Health: Two-Dimensional

### 5.1 RTP Pool Health (Review)

```
Single dimension:
├── Mean TOR
├── TOR variance
├── Station coverage
└── Total tasks
```

### 5.2 Relay Pool Health: Two Loops

```
┌─────────────────────────────────────────────────────────────┐
│  RELAY POOL HEALTH                                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  VTM LOOP (per zone):                                       │
│  ├── Zone mean TOR_vtm                                      │
│  ├── Zone TOR_vtm variance                                  │
│  ├── Zone task count                                        │
│  ├── Zone VTM utilization                                   │
│  └── Zone relay utilization                                 │
│                                                             │
│  HTM LOOP (per station):                                    │
│  ├── Station mean TOR_htm                                   │
│  ├── Station TOR_htm variance                               │
│  ├── Station task count                                     │
│  └── Station queued work (seconds)                          │
│                                                             │
│  COUPLING (relay buffer):                                   │
│  ├── Per-zone relay utilization                             │
│  ├── System relay utilization                               │
│  └── Imbalance indicator (VTM rate vs HTM rate)            │
│                                                             │
│  SYSTEM TOTALS:                                             │
│  ├── Mean TOR_vtm (target: V/O)                            │
│  ├── Mean TOR_htm (target: H/O)                            │
│  └── Mean TOR_total                                         │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 5.3 Example Pool Health Report

```
RELAY POOL HEALTH REPORT
========================

VTM LOOP:
  Zone A: TOR_vtm=1.8  tasks=12  VTM_util=72%  relay=67%
  Zone B: TOR_vtm=2.3  tasks=8   VTM_util=88%  relay=100% 🔴
  Zone C: TOR_vtm=1.6  tasks=14  VTM_util=65%  relay=50%
  ─────────────────────────────────────────────────────
  System: TOR_vtm=1.9 (target=2.0) ✓

HTM LOOP:
  Station 1: TOR_htm=0.9  tasks=8   queued=180s
  Station 2: TOR_htm=1.2  tasks=6   queued=140s
  Station 3: TOR_htm=1.0  tasks=7   queued=160s
  ─────────────────────────────────────────────────────
  System: TOR_htm=1.0 (target=1.0) ✓

COUPLING:
  Zone B relay at 100% - VTMs may block ⚠️

SYSTEM:
  TOR_total = 1.9 + 1.0 + 1 = 3.9 (target=4.0) ✓
```

### 5.4 What Each Level Sees

| Level | Sees | Controls |
|-------|------|----------|
| Station | TOR_htm distribution | Task selection by TOR_htm |
| Zone | TOR_vtm distribution | Rack selection by TOR_vtm |
| System | Both + coupling | Zone balance, station balance |

**Critical:** Stations cannot see or influence TOR_vtm. Zones cannot see or influence TOR_htm. Only system-level optimization can balance both.

---

## 6. Order Selection for Relay

### 6.1 The Two-Decision Problem

When selecting an order in Relay:

```
Decision 1: Which rack/tote? → Determines TOR_vtm (zone assignment)
Decision 2: Which station?   → Determines TOR_htm

These decisions are partially decoupled by the relay buffer.
```

### 6.2 Selection Bias States (Extended)

| Pool State | Bias | Action |
|------------|------|--------|
| TOR_vtm > target | PREFER_CLOSER_RACKS | Reduce VTM travel |
| TOR_vtm < target | PREFER_FARTHER_RACKS | Preserve nearby inventory |
| TOR_htm > target | PREFER_CLOSER_STATIONS | Reduce HTM travel |
| TOR_htm < target | PREFER_FARTHER_STATIONS | Balance station load |
| Zone Z overloaded | AVOID_ZONE_Z | Distribute VTM work |
| Zone Z relay full | AVOID_ZONE_Z | Prevent VTM blocking |
| Station S starving | PREFER_STATION_S | Prevent operator idle |

### 6.3 Bottleneck-Adaptive Selection

```java
SelectionBias determineBias(PoolHealth health) {
    double vtmGap = health.meanTorVtm() - targetTorVtm;
    double htmGap = health.meanTorHtm() - targetTorHtm;

    // Which loop is the problem?
    if (Math.abs(vtmGap) > Math.abs(htmGap) + 0.3) {
        // VTM loop is further from target
        if (vtmGap > 0) {
            return PREFER_CLOSER_RACKS;
        } else {
            return PREFER_FARTHER_RACKS;  // Preserve nearby
        }
    } else if (Math.abs(htmGap) > Math.abs(vtmGap) + 0.3) {
        // HTM loop is further from target
        if (htmGap > 0) {
            return PREFER_CLOSER_STATIONS;
        } else {
            return PREFER_FARTHER_STATIONS;
        }
    } else {
        return BALANCED;
    }
}
```

### 6.4 Scoring Function

```java
double scoreCandidate(ToteCandidate candidate, PoolHealth health) {
    double score = 0;

    // === VTM LOOP SCORING ===

    // TOR_vtm targeting
    double vtmDeviation = Math.abs(candidate.torVtm() - targetTorVtm);
    score -= vtmDeviation * VTM_TOR_WEIGHT;

    // Zone balance
    Zone zone = candidate.zone();
    double avgZoneTasks = health.avgTasksPerZone();
    score += (avgZoneTasks - zone.taskCount()) * ZONE_BALANCE_WEIGHT;

    // Zone relay congestion
    if (zone.relayUtilization() > 0.8) {
        score -= RELAY_CONGESTION_PENALTY;
    }
    if (zone.relayUtilization() >= 1.0) {
        score -= RELAY_FULL_PENALTY;  // Hard penalty
    }

    // === HTM LOOP SCORING ===

    // TOR_htm targeting
    double htmDeviation = Math.abs(candidate.torHtm() - targetTorHtm);
    score -= htmDeviation * HTM_TOR_WEIGHT;

    // Station balance
    Station station = candidate.station();
    double avgStationTasks = health.avgTasksPerStation();
    score += (avgStationTasks - station.taskCount()) * STATION_BALANCE_WEIGHT;

    // Station starvation prevention
    if (station.taskCount() < MIN_TASKS_PER_STATION) {
        score += STATION_STARVING_BONUS;
    }

    // === SYSTEM SCORING ===

    // Total TOR budget
    double projectedTorTotal = candidate.torVtm() + candidate.torHtm() + 1;
    if (projectedTorTotal > systemTorBudget) {
        score -= (projectedTorTotal - systemTorBudget) * BUDGET_PENALTY;
    }

    return score;
}
```

### 6.5 Selection Algorithm

```java
void selectOrdersForRelay(List<Order> pending, PoolHealth health) {
    while (needMoreTasks() && !pending.isEmpty()) {
        // Generate candidates: order × zone × station combinations
        List<ToteCandidate> candidates = generateCandidates(pending);

        // Score based on current pool health
        ToteCandidate best = candidates.stream()
            .max(comparing(c -> scoreCandidate(c, health)))
            .orElse(null);

        if (best == null || scoreTooLow(best)) {
            break;
        }

        // Commit selection
        pool.add(best);
        pending.remove(best.order());

        // Reassess (pool state changed)
        health = pool.assessHealth();
    }
}
```

---

## 7. Zone Management

### 7.1 Zone-Level Balance

Each zone operates semi-independently in the VTM loop:

```
Zone Z health:
├── TOR_vtm_z: Mean VTM overhead for zone
├── Task count: Orders assigned to zone
├── VTM utilization: ρ_vtm_z = demand_z × 2×T_vtm_z / V_z
├── Relay utilization: occupied / capacity
└── Relay queue depth: Totes waiting for HTM
```

### 7.2 Zone Imbalance Detection

```java
boolean isZoneImbalanced(ZoneHealth[] zones) {
    double[] utilizations = Arrays.stream(zones)
        .mapToDouble(z -> z.vtmUtilization())
        .toArray();

    double mean = mean(utilizations);
    double cv = stddev(utilizations) / mean;

    return cv > 0.2;  // >20% variation is imbalanced
}

Zone getMostOverloadedZone(ZoneHealth[] zones) {
    return Arrays.stream(zones)
        .max(comparing(z -> z.vtmUtilization()))
        .orElse(null);
}
```

### 7.3 Zone-Aware Selection Bias

```java
double zoneScore(Zone zone, ZoneHealth health) {
    double score = 0;

    // Penalize overloaded zones
    if (health.vtmUtilization() > 0.85) {
        score -= (health.vtmUtilization() - 0.85) * 100;
    }

    // Penalize congested relay
    if (health.relayUtilization() > 0.8) {
        score -= (health.relayUtilization() - 0.8) * 150;
    }

    // Bonus for underutilized zones
    double avgUtil = systemHealth.avgZoneUtilization();
    score += (avgUtil - health.vtmUtilization()) * 30;

    return score;
}
```

---

## 8. Relay Buffer Dynamics

### 8.1 Buffer States

```
┌─────────────────────────────────────────────────────────────┐
│  RELAY BUFFER STATE MACHINE                                 │
│                                                             │
│  EMPTY (0% utilized)                                        │
│  ├── HTMs starve, wait for VTM deliveries                  │
│  ├── Operator may idle                                      │
│  └── Action: Bias toward this zone's racks                 │
│                                                             │
│  HEALTHY (20-80% utilized)                                  │
│  ├── Normal operation                                       │
│  ├── Buffer absorbs variability                            │
│  └── Action: Maintain balance                              │
│                                                             │
│  HIGH (80-95% utilized)                                     │
│  ├── VTMs may start blocking soon                          │
│  ├── HTMs should prioritize this zone                      │
│  └── Action: Reduce new tasks to this zone                 │
│                                                             │
│  FULL (100% utilized)                                       │
│  ├── VTMs block, cannot deposit                            │
│  ├── Zone throughput drops                                  │
│  └── Action: Stop adding tasks to this zone                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 8.2 Buffer-Aware Selection

```java
double relayBufferScore(Zone zone) {
    double utilization = zone.relayUtilization();

    if (utilization < 0.2) {
        // Buffer too empty - zone is underutilized
        return 20;  // Slight bonus to fill it
    } else if (utilization < 0.8) {
        // Healthy range
        return 0;
    } else if (utilization < 1.0) {
        // Getting full - penalize
        return -50 * (utilization - 0.8) / 0.2;
    } else {
        // Full - hard penalty
        return -200;
    }
}
```

### 8.3 Coupling Health Metric

```java
record CouplingHealth(
    double vtmDeliveryRate,   // Totes/time VTMs are delivering
    double htmPickupRate,     // Totes/time HTMs are picking up
    double imbalanceRatio     // vtmRate / htmRate
) {
    boolean isHealthy() {
        return imbalanceRatio > 0.8 && imbalanceRatio < 1.2;
    }

    String diagnosis() {
        if (imbalanceRatio > 1.2) {
            return "VTM > HTM: Relays filling up, risk of VTM blocking";
        } else if (imbalanceRatio < 0.8) {
            return "HTM > VTM: Relays draining, HTMs may starve";
        } else {
            return "Balanced";
        }
    }
}
```

---

## 9. Configuration Parameters

### 9.1 Core Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `targetTorVtm` | V/O (auto) | Target VTM overhead |
| `targetTorHtm` | H/O (auto) | Target HTM overhead |
| `returnFactorVtm` | 1.0 | VTM return as fraction of outbound. Range [0.5, 1.2] |
| `returnFactorHtm` | 0.8 | HTM return as fraction of outbound. Range [0.3, 1.0] |
| `zoneDeviation` | 0.5 | Allowed ±δ for zone TOR_vtm |
| `stationDeviation` | 1.0 | Allowed ±δ for station TOR_htm |
| `minTasksPerZone` | 3 | Minimum before zone starves |
| `minTasksPerStation` | 4 | Minimum before station starves |

### 9.2 Scoring Weights

| Weight | Default | Description |
|--------|---------|-------------|
| `vtmTorWeight` | 25.0 | VTM loop TOR targeting |
| `htmTorWeight` | 25.0 | HTM loop TOR targeting |
| `zoneBalanceWeight` | 15.0 | Zone load distribution |
| `stationBalanceWeight` | 15.0 | Station load distribution |
| `relayCongestionPenalty` | 50.0 | Relay >80% utilized |
| `relayFullPenalty` | 200.0 | Relay at 100% |
| `stationStarvingBonus` | 50.0 | Station below minimum |
| `zoneStarvingBonus` | 40.0 | Zone below minimum |

### 9.3 Buffer Thresholds

| Parameter | Default | Description |
|-----------|---------|-------------|
| `relayHealthyMin` | 0.2 | Below this = underutilized |
| `relayHealthyMax` | 0.8 | Above this = congestion risk |
| `relayCritical` | 0.95 | Above this = near blocking |

### 9.4 Configuration Record

```java
public record RelayTorConfig(
    // Targets (null = auto-calculate)
    Double targetTorVtm,
    Double targetTorHtm,

    // Return Factors
    double returnFactorVtm,    // RF for VTM loop, default 1.0
    double returnFactorHtm,    // RF for HTM loop, default 0.8

    // Deviations
    double zoneDeviation,
    double stationDeviation,

    // Minimums
    int minTasksPerZone,
    int minTasksPerStation,
    int minPoolSize,

    // Weights
    double vtmTorWeight,
    double htmTorWeight,
    double zoneBalanceWeight,
    double stationBalanceWeight,
    double relayCongestionPenalty,
    double relayFullPenalty,
    double stationStarvingBonus,
    double zoneStarvingBonus,

    // Buffer thresholds
    double relayHealthyMin,
    double relayHealthyMax,
    double relayCritical
) {
    public static RelayTorConfig defaults(int numVtms, int numHtms, int numOperators) {
        return new RelayTorConfig(
            (double) numVtms / numOperators,   // targetTorVtm
            (double) numHtms / numOperators,   // targetTorHtm
            1.0,   // returnFactorVtm (VTM returns to same rack)
            0.8,   // returnFactorHtm (HTM has landing zone opportunities)
            0.5,   // zoneDeviation
            1.0,   // stationDeviation
            3,     // minTasksPerZone
            4,     // minTasksPerStation
            Math.max(2 * numVtms, 2 * numHtms),  // minPoolSize
            25.0, 25.0, 15.0, 15.0,  // TOR and balance weights
            50.0, 200.0,              // relay penalties
            50.0, 40.0,               // starving bonuses
            0.2, 0.8, 0.95            // buffer thresholds
        );
    }

    /** Calculate TOR_vtm for given outbound travel and service time */
    public double calculateTorVtm(double tVtmOutbound, double tService) {
        return ((1 + returnFactorVtm) * tVtmOutbound) / tService;
    }

    /** Calculate TOR_htm for given outbound travel and service time */
    public double calculateTorHtm(double tHtmOutbound, double tService) {
        return ((1 + returnFactorHtm) * tHtmOutbound) / tService;
    }

    /** Calculate total TOR */
    public double calculateTorTotal(double tVtmOutbound, double tHtmOutbound, double tService) {
        return calculateTorVtm(tVtmOutbound, tService)
             + calculateTorHtm(tHtmOutbound, tService)
             + 1;
    }
}
```

---

## 10. Corner Cases

### 10.1 Single Zone Overload

**Scenario:** One zone has disproportionate demand (e.g., hot SKUs concentrated).

**Detection:**
```java
zone.vtmUtilization() > 0.9 && otherZones.avgUtilization() < 0.6
```

**Response:**
1. Hard penalty for adding tasks to overloaded zone
2. If order MUST come from that zone, accept with warning
3. Log for slotting optimization (redistribute SKUs)

### 10.2 Relay Deadlock

**Scenario:** All relay positions full, VTMs blocked, HTMs can't keep up.

**Detection:**
```java
zone.relayUtilization() == 1.0 && zone.vtmsBlocked() > 0
```

**Response:**
1. Emergency: Stop adding tasks to this zone
2. Signal HTMs to prioritize this zone's relay
3. If persistent, indicates configuration problem (not enough HTMs)

### 10.3 HTM-Operator Mismatch

**Scenario:** H > O significantly (more HTMs than operators can use).

**Detection:**
```java
htmUtilization < 0.5 && operatorUtilization > 0.9
```

**Response:**
1. Not a selection problem - configuration issue
2. Extra HTMs are essentially idle
3. Effective Target TOR_htm = O/O = 1 (operator-limited)

### 10.4 VTM-HTM Imbalance

**Scenario:** VTM capacity >> HTM capacity (or vice versa).

**Detection:**
```java
couplingHealth.imbalanceRatio() > 1.5  // VTM too fast
couplingHealth.imbalanceRatio() < 0.67 // HTM too fast
```

**Response:**
1. If VTM >> HTM: Relays will fill, VTMs block
   - Selection should prefer zones with lower relay utilization
2. If HTM >> VTM: Relays will empty, HTMs starve
   - Selection should increase throughput (accept higher TOR_vtm)

### 10.5 Cross-Zone Order

**Scenario:** Order requires totes from multiple zones.

**Handling:**
```java
List<Zone> zonesRequired = order.getRequiredZones();

if (zonesRequired.size() > 1) {
    // Score each zone component separately
    double score = zonesRequired.stream()
        .mapToDouble(z -> scoreZoneContribution(order, z))
        .sum();

    // Additional penalty for cross-zone coordination
    score -= CROSS_ZONE_PENALTY * (zonesRequired.size() - 1);
}
```

### 10.6 Late Simulation Zone Depletion

**Scenario:** One zone's inventory nearly exhausted.

**Detection:**
```java
zone.remainingTasks() < zone.totalTasks() * 0.1
```

**Response:**
1. Expected if order selection was balanced
2. Reduce zone weight in scoring (can't contribute much)
3. Focus on remaining zones

---

## 11. FAQ

### Q1: Why doesn't the station see VTM overhead?

**A:** The relay buffer decouples the loops. From the station's perspective:
- Totes appear at relay positions (VTM's job is done)
- HTM fetches from relay to station
- Station only experiences HTM travel + service

VTM overhead is "hidden" behind the relay. This is the decoupling benefit - but also means station-level metrics don't reflect full system cost.

### Q2: Can high VTM efficiency compensate for low HTM efficiency (or vice versa)?

**A:** Partially, but with limits.

```
Total TOR = TOR_vtm + TOR_htm + 1

If TOR_vtm = 1.0 and TOR_htm = 2.0 → Total = 4.0
If TOR_vtm = 2.0 and TOR_htm = 1.0 → Total = 4.0

Same total, but different bottleneck implications.
```

However, each loop has its own Target TOR. Exceeding one while being under the other means one resource type is overworked while another is underutilized.

### Q3: Why is VTM zone-bound but HTM global?

**A:** Physical warehouse design:
- VTMs operate in aisles (vertical movement within rack blocks)
- Aisles are grouped into zones
- VTM cannot easily cross to another zone's aisles

HTMs operate on the floor between relay positions and stations:
- No aisle constraints
- Can reach any relay position and any station
- Global pool is more efficient (better load balancing)

### Q4: How do I know which loop is the bottleneck?

**A:** Compare utilizations:

```
If ρ_vtm > ρ_htm and ρ_vtm > ρ_op → VTM bottleneck
If ρ_htm > ρ_vtm and ρ_htm > ρ_op → HTM bottleneck
If ρ_op > ρ_vtm and ρ_op > ρ_htm  → Operator bottleneck

Or compare TOR to Target:
If TOR_vtm >> Target TOR_vtm → VTM overhead too high
If TOR_htm >> Target TOR_htm → HTM overhead too high
```

### Q5: What if I have more operators than HTMs?

**A:** HTM becomes the constraint for the HTM-Operator stage.

```
Effective capacity = min(H/(2×T_htm), O/S)

If O/S > H/(2×T_htm):
  - Operators will have idle time
  - System is HTM-limited
  - Adding operators won't help
```

### Q6: Should pool health track VTM and HTM separately or combined?

**A:** Both.

- **Separately:** To identify which loop needs attention
- **Combined:** To verify system-level balance

Selection decisions need to consider both loops, but the diagnosis requires seeing them separately.

### Q7: How does picks-per-tote affect Relay vs RTP differently?

**A:**

**RTP:** More picks per rack → TOR decreases (overhead amortized)

**Relay:**
- TOR_vtm: UNCHANGED (VTM moves tote regardless of picks)
- TOR_htm: Decreases (more service time amortizes HTM travel)
- TOR_total: Decreases (but less than RTP)

```
Example: Double the picks per tote

RTP:     TOR = (2×100 + 60) / 60 = 4.3
         TOR = (2×100 + 120) / 120 = 2.7  (37% reduction)

Relay:   TOR_vtm = 2×50 / 60 = 1.67
         TOR_vtm = 2×50 / 120 = 0.83 (50% reduction)

         TOR_htm = 2×30 / 60 = 1.0
         TOR_htm = 2×30 / 120 = 0.5 (50% reduction)

         TOR_total = 1.67 + 1.0 + 1 = 3.67
         TOR_total = 0.83 + 0.5 + 1 = 2.33 (36% reduction)
```

Similar total reduction, but the mechanism is different.

### Q8: What's the ideal relay utilization?

**A:** 40-60% in steady state.

- **< 20%:** Buffer underutilized, HTMs may starve during variability spikes
- **20-80%:** Healthy operating range
- **> 80%:** Risk of VTM blocking, reduced throughput
- **100%:** VTMs blocked, zone throughput degraded

### Q9: How do I handle priority orders in a two-loop system?

**A:** Priority affects both loops:

```java
double priorityBonus(Order order, Zone zone, Station station) {
    double bonus = order.priority() * PRIORITY_WEIGHT;

    // High priority can override zone congestion (within limits)
    if (order.priority() > HIGH_PRIORITY) {
        // Reduce relay congestion penalty
        bonus += zone.relayUtilization() * 0.5 * RELAY_CONGESTION_PENALTY;
    }

    return bonus;
}
```

### Q10: Can I use the same Pool Health infrastructure for RTP and Relay?

**A:** Partially. Common infrastructure:
- Task count per station
- Station TOR distribution
- Total pool size

Relay-specific additions:
- Per-zone metrics (TOR_vtm, utilization)
- Relay buffer state
- Two-loop coupling health

Recommend: Abstract base PoolHealth with RtpPoolHealth and RelayPoolHealth subclasses.

### Q11: How should I handle orders that pick from totes already active in the system?

**A:** Use **marginal TOR for selection**, **amortized TOR for Pool Health** - same principle as RTP, but with two-loop complexity.

The key is recognizing **which loops have already been paid for**:

| Tote State | TOR_vtm for scoring | TOR_htm for scoring |
|------------|---------------------|---------------------|
| In storage | Full | Full |
| VTM fetching / At relay | 0 | Full |
| HTM fetching / At station | 0 | 0 |
| Returning | Intercept or Full | Intercept or Full |

**Why this matters more in Relay:** Totes at relay are especially valuable targets - VTM cost is already paid, so only HTM cost remains. This can cut TOR by 50% for piggybacked picks.

See Section 3.5 "Active Tote Handling (Piggybacking)" for complete state transition table and implementation patterns.

---

## Appendix A: Complete TOR Calculation Example

**Given:**
- 8 VTMs, 4 HTMs, 4 Operators
- VTM speed: 2 m/s, HTM speed: 3 m/s
- Rack at (50, 30), Zone A relay at (10, 30), Station 1 at (0, 10)
- Tote has 5 picks, time per pick = 12s
- RF_vtm = 1.0 (VTM returns to same rack)
- RF_htm = 0.8 (HTM uses relay landing zones)

**Calculations:**

```
Service time:
S = 5 × 12s = 60s

VTM travel (outbound):
d_vtm = distance((50,30), (10,30)) = 40m
T_vtm_outbound = 40m / 2(m/s) = 20s

HTM travel (outbound):
d_htm = distance((10,30), (0,10)) = √(100 + 400) = 22.4m
T_htm_outbound = 22.4m / 3(m/s) = 7.5s

TOR components (with Return Factors):
TOR_vtm = (1 + 1.0) × 20s / 60s = 2.0 × 20 / 60 = 0.67
TOR_htm = (1 + 0.8) × 7.5s / 60s = 1.8 × 7.5 / 60 = 0.225
TOR_total = 0.67 + 0.225 + 1 = 1.895

Compare with RF = 1.0 for both (conservative):
TOR_vtm = 2.0 × 20 / 60 = 0.67 (same)
TOR_htm = 2.0 × 7.5 / 60 = 0.25
TOR_total = 0.67 + 0.25 + 1 = 1.92

Targets:
Target TOR_vtm = 8/4 = 2.0
Target TOR_htm = 4/4 = 1.0

Assessment:
TOR_vtm = 0.67 < 2.0 → VTM loop very efficient (maybe too efficient?)
TOR_htm = 0.225 < 1.0 → HTM loop very efficient
TOR_total = 1.895 < 4.0 → System running efficiently

This task is "easy" - close rack, close station.
Selection should balance with some "harder" tasks.
```

---

## Appendix B: Relay vs RTP Decision Guide

| Factor | Favors RTP | Favors Relay |
|--------|------------|--------------|
| Warehouse size | Small-medium | Large |
| Aisle congestion | Low | High (VTM in aisles) |
| Picks per order | High (amortizes travel) | Any (decoupled) |
| Throughput variability | Low | High (buffer helps) |
| SKU concentration | Distributed | Zoned (hot zones) |
| Operator count | Few | Many |
| System complexity tolerance | Low | High |

---

**End of Document**
