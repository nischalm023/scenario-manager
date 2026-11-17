# Relay System Metrics Hierarchy

## System Overview

### What is Grey Matter Relay?

Grey Matter Relay is GreyOrange's goods-to-person fulfillment system designed for high-density tote storage and retrieval. It is architecturally similar to Quicktron's QuickBin system and Amazon's Kiva-derived tote shuttle systems.

**The core innovation:** A two-loop decoupled architecture where vertical tote movers (VTMs) and horizontal tote movers (HTMs) operate independently, connected by a relay buffer. This decoupling allows each subsystem to be optimized and scaled independently.

### Physical Components

| Component | Function | Constraints |
|-----------|----------|-------------|
| **VTM** (Vertical Tote Mover) | Fetches totes from storage racks, delivers to relay positions | Bounded to a Virtual Aisle (VA) |
| **HTM** (Horizontal Tote Mover) | Moves totes from relay positions to pick stations | Operates across the floor |
| **Relay Positions** | Buffer/handoff points between VTM and HTM loops | Finite capacity per aisle |
| **Pick Stations (PPS)** | Where operators pick items from totes | Operator-limited throughput |
| **Virtual Aisle (VA)** | Logical grouping of racks served by dedicated VTMs | Unit of zone control |

### Two-Loop Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         TWO-LOOP ARCHITECTURE                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   STORAGE          VTM LOOP              RELAY           HTM LOOP    STATION│
│                                          BUFFER                             │
│   ┌─────┐     ┌──────────────┐     ┌─────────────┐  ┌─────────────┐ ┌─────┐│
│   │Racks│◄───►│     VTM      │◄───►│   Relay     │◄►│     HTM     │◄┤ PPS ││
│   │     │     │              │     │  Positions  │  │             │ │     ││
│   └─────┘     └──────────────┘     └─────────────┘  └─────────────┘ └─────┘│
│                     │                     │                │                │
│               Bounded to VA         Decoupling        Floor-wide           │
│               (local loop)           Point           (global loop)          │
│                                                                             │
│   Why Two Loops?                                                            │
│   • Independent scaling: Add VTMs for retrieval, HTMs for delivery         │
│   • Fault isolation: One loop can continue if other slows                  │
│   • Buffer absorbs timing variance between loops                           │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Concept: TOR (Transport Overhead Ratio)

**TOR** measures how much transport work is required per unit of productive work.

```
TOR = (Travel Time + Service Time) / Service Time
```

From queuing theory, a warehouse is a **closed queueing network** where bots cycle between storage and stations. The system is balanced when bot capacity matches operator capacity:

```
Bot capacity:     B / (Travel + Service)
Operator capacity: O / Service

Setting equal → Target TOR = B / O (Bots per Operator)
```

**Interpretation:**
- `TOR = 1.0`: Zero travel overhead (theoretical minimum)
- `TOR = 3.0`: Travel is 2× productive work
- `TOR > Target`: Bot-constrained (select closer totes)
- `TOR < Target`: Operator-constrained (can afford farther totes)

In Relay's two-loop architecture, TOR splits into `TOR_vtm` and `TOR_htm`, each with independent targets.

---

### Software Layers

| Layer | Module | Responsibility |
|-------|--------|----------------|
| **PICK** | Order Processing | Which totes to fetch, TOR management, task distribution |
| **MVTS** | Task Scheduling + Relay Mgmt | Bot-to-task assignment, relay buffer management |
| **NAVIGATION** | Path Planning | Actual paths, traffic avoidance |
| **PHYSICAL** | Hardware | Speeds, capacities, uptime |

---

## Problem Statement

**Primary KPI: UPH (Units Per Hour)**

Create a formal metrics hierarchy ("metric tree") for Relay systems that:

1. **Decomposes UPH** through two relationship types:
   - **FORMULA** (═══): Exact mathematical relationship (A = B × C)
   - **CAUSAL** (───): Directional influence, magnitude varies by site/conditions

2. **Maps each metric to its control layer:**
   - **PICK**: Order selection, TOR control, tote selection, task distribution
   - **MVTS**: Robot↔task assignment, relay management, HTM↔station pairing
   - **NAVIGATION**: Path planning, traffic management
   - **PHYSICAL**: Hardware specs, layout constraints

3. **Enables four use cases:**
   - Bottleneck diagnosis
   - Target setting
   - Sensitivity analysis
   - Responsibility attribution

---

## Metrics Tree

### Level 0: Root

```
UPH (Units Per Hour)
═══════════════════════════════════════════════════════════════════════════════
Formula: UPH = Picks/Tote × Tote_Presentations/Hour

                              UPH
                               ║
              ═════════════════╬═════════════════
              ║                                 ║
        Picks/Tote                    Tote Presentations/Hour
        (DEMAND)                           (SUPPLY)
              ║                                 ║
          [PICK +                      Two-Loop Architecture
          EXTERNAL]                    with Relay Buffer
```

**Balance Equations:**
```
VTM Loop Balanced when: TOR_vtm ≈ V/O (VTMs / Operators)
HTM Loop Balanced when: TOR_htm ≈ H/O (HTMs / Operators)
```

**TOR (Transport Overhead Ratio)** is the key control variable that PICK uses to balance supply and demand:
- When `TOR_actual > Target`: System is bot-constrained, select closer totes
- When `TOR_actual < Target`: Operators may starve, can afford farther totes

---

### Level 1: Demand Side (Picks/Tote)

```
                        Picks/Tote
                            ║
                ════════════╬════════════
                ║                       ║
        Orderlines/Tote          Quantity/Orderline
                │                       │
           ─────┴─────                  │
           │         │              [EXTERNAL]
      SKU Affinity  Tote Selection   Customer
      in Orders     Strategy         order data
           │              │
       [EXTERNAL]    ─────┴─────
                     │         │
                 PPF-First  TOR-Aware
                 Strategy   Selection
                     │         │
                   [PICK]   [PICK]
```

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| Picks/Tote | FORMULA | - | Orderlines/Tote × Qty/Orderline |
| Orderlines/Tote | CAUSAL | PICK | Order lines fulfillable from one tote |
| Quantity/Orderline | EXTERNAL | Customer | Average units per line item |
| SKU Affinity | EXTERNAL | Customer | Co-occurrence of SKUs across orders |
| Tote Selection Strategy | CONTROL | PICK | PPF-first vs TOR-aware selection |

**Key Insight:** PICK influences Orderlines/Tote through smart tote selection. When TOR is healthy, PICK can select totes with higher PPF potential even if farther away.

---

### Level 1: Supply Side (Tote Presentations/Hour)

```
                    Tote Presentations/Hour
                            ║
    ════════════════════════╬════════════════════════
    ║                       ║                       ║
VTM Loop               Relay Buffer            HTM Loop
Throughput              (Coupling)             Throughput
    ║                       ║                       ║
N_eff_vtm × 60         Occupancy %           N_eff_htm × 60
─────────────          Dwell Time            ─────────────
 T_vtm_cycle           Flow Balance           T_htm_cycle
    │                       │                       │
  [PICK +                [MVTS]                  [MVTS]
   MVTS]
```

**Throughput Formula:**
```
TPH_actual = min(TPH_vtm, TPH_htm, TPH_station) × (1 - L_block) × (1 - L_idle)

Where:
  L_block ← f(Relay Occupancy > 80%)  — VTMs blocked, can't drop totes
  L_idle  ← f(Relay Occupancy < 20%)  — HTMs/operators starve
```

---

### Level 2: Relay Buffer (Two-Loop Coupling Point)

The relay buffer is the physical decoupling point between VTM and HTM loops. Its state indicates system health.

```
Relay Buffer
═══════════════════════════════════════════════════════════════════════════════
Physical: N_relay positions distributed across aisles

┌─────────────────────────────────────────────────────────────────────────────┐
│                         BUFFER OCCUPANCY GAUGE                              │
├─────────────────────────────────────────────────────────────────────────────┤
│░░░░░░░░░░░░░░░░░░░░│██████████████████████████████│░░░░░░░░░░░░░░░░░░░░░░░░│
│ 0%                20%                            80%                    100%│
│    STARVATION           HEALTHY OPERATING ZONE           BLOCKING          │
│    • HTMs idle          • Both loops independent         • VTMs wait       │
│    • Operators wait     • Timing variance absorbed       • Supply stops    │
│    • idle_losses ↑      • Optimal throughput             • blocking_losses↑│
└─────────────────────────────────────────────────────────────────────────────┘
```

**Relay Buffer Metrics:**

| Metric | Formula | Description | Controlled By |
|--------|---------|-------------|---------------|
| Occupancy % | `Totes_in_buffer / N_relay × 100` | Buffer fill level (like fuel gauge) | MVTS |
| Dwell Time | `Avg seconds tote waits at relay` | Coordination quality indicator | MVTS |
| Flow Balance | `(VTM_arrival - HTM_departure) / HTM_departure` | Rate of change indicator | PICK + MVTS |

**Interpretation:**
- **Flow Balance > 0**: Buffer filling, risk of VTM blocking
- **Flow Balance ≈ 0**: Steady state, loops synchronized
- **Flow Balance < 0**: Buffer draining, risk of HTM starvation

---

### Level 2: VTM Cycle Time (Two Task Types)

VTMs perform two types of tasks with different characteristics:

```
                    VTM Cycle Time (Aggregate)
                              ║
               ═══════════════╬═══════════════
               ║                             ║
    Storable → Relay Cycle          Relay → Storable Cycle
       (Buffer Filling)                (Buffer Draining)
               ║                             ║
    ═══════════╬═══════════       ═══════════╬═══════════
    ║    ║     ║     ║            ║    ║     ║     ║
  Fetch Travel Drop Wait        Pickup Travel Put  Wait
    ║    ║     │     │            │     ║     ║     │
    ║    ║   [PHYS]  │          [PHYS]  ║     ║   [NAV]
    ║    ║         Relay                ║     ║  Traffic
    ║    ║         Full               ──┴─────┴──
 ───┴─── ║        [MVTS]              │         │
 │     │ ║                          Relay    Storable
Vert Extract                       Location  Location
Move   │   ║                          │         │
 │     │ ──╬──                     [MVTS]    [MVTS]
[PHYS] │   │  │                            (back-to-store)
    [PHYS] │  │
        Rack  Relay
        Loc   Pos
         │     │
      [PICK] [MVTS]
```

**VTM Task Type Comparison:**

| Aspect | Storable → Relay | Relay → Storable |
|--------|------------------|------------------|
| Purpose | Fills buffer (production) | Drains buffer (back-to-store) |
| Controlled By | PICK (tote selection) | MVTS (location selection) |
| Priority Trigger | Relay Occupancy < threshold | Relay Occupancy > threshold |
| Travel: Source | Storable rack location | Relay position |
| Travel: Destination | Relay position | Storable rack location |

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| VTM Cycle Time | FORMULA | - | Fetch + Travel + Handoff + Wait |
| Fetch Time | FORMULA | Physical | Vertical Move + Extract Time |
| Travel Time (Outbound) | CAUSAL | PICK + MVTS | Rack location (PICK), Relay position (MVTS) |
| Travel Time (Return) | CAUSAL | MVTS | Back-to-store location selection |
| Handoff Time | PHYSICAL | Physical | Drop/pickup mechanism |
| Wait: Relay Full | CAUSAL | MVTS | When Occupancy > threshold, VTM blocks |
| Wait: Traffic | CAUSAL | NAV | Aisle congestion |

**Key Insight:** When relay occupancy is high, MVTS prioritizes Relay→Storable tasks to drain the buffer and prevent VTM blocking.

---

### Level 2: HTM Cycle Time (with Multi-Stop)

HTMs operate in the Relay ↔ Station loop, potentially visiting multiple stations per trip.

```
                    HTM Cycle Time (Relay ↔ Station)
                              ║
    ══════════════════════════╬══════════════════════════
    ║           ║                    ║              ║
Pickup     Travel Time          Station Time    Return Time
(relay)         ║                    ║              ║
   │      ══════╬══════         ═════╬═════    ════╬════
   │      ║           ║         ║         ║    ║       ║
 [PHYS] Relay→     Stn→Stn    Queue     Pick  Travel Handoff
        Station   (multi)     Wait      Time    │       │
           │         │          │         │     │    [PHYS]
        [MVTS]    [MVTS]     [MVTS]   ───┴───   │
                    │                 │     │   │
              # of Stops           Oper  Picks [MVTS]
                  │                Speed  /Tote (relay
               [MVTS]                │     │   selection)
                                  [PHYS] [PICK]
```

**Multi-Stop Trade-off:**

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        MULTI-STOP ROUTING DECISION                          │
├────────────────────────────────┬────────────────────────────────────────────┤
│         SINGLE STOP            │              MULTI-STOP                    │
├────────────────────────────────┼────────────────────────────────────────────┤
│ • Lower cycle time per trip    │ • Higher cycle time per trip              │
│ • Fewer picks per trip         │ • More picks per trip                     │
│ • Higher HTM trips/hour        │ • Better order consolidation              │
│ • Used when: Low consolidation │ • Used when: Same order across stations   │
├────────────────────────────────┴────────────────────────────────────────────┤
│ Decision by: MVTS (influenced by PICK's order consolidation)                │
│ Optimal choice depends on: station queue depths, tote consolidation         │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| HTM Cycle Time | FORMULA | - | Pickup + Travel + Station Time + Return |
| Pickup Time | PHYSICAL | Physical | Relay handoff mechanism |
| Relay→Station Distance | CAUSAL | MVTS | Station assignment optimization |
| Station→Station Distance | CAUSAL | MVTS | Multi-stop routing |
| # of Stops | CONTROL | MVTS | Single vs multi-stop decision |
| Queue Time | CAUSAL | MVTS | Station load balancing |
| Pick Time | FORMULA | Physical + PICK | # Picks × Operator Speed |
| Return Time | FORMULA | MVTS + Physical | Travel + Handoff |

---

### Level 2: PICK / Order Processing Metrics

PICK (Order Processing) controls TOR as its primary output, which becomes input to MVTS.

```
                    PICK / Order Processing
                              ║
        ══════════════════════╬══════════════════════
        ║                     ║                     ║
   Tote Selection        TOR Management      Task Distribution
        ║                     ║                     ║
   ═════╬═════           ═════╬═════           ════╬════
   ║         ║           ║         ║           ║        ║
Picks/     Rack       System    PPS          VA Load  PPS Load
Tote     Distance      TOR      TOR         Balance  Balance
   │         │           │         │           │        │
[PICK]    [PICK]      [PICK]   [PICK]      [PICK]   [PICK]
                    (output)  (output)
```

**TOR Control Loop:**

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           TOR CONTROL LOOP                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   ┌─────────┐      ┌─────────┐      ┌─────────┐      ┌─────────┐           │
│   │ Measure │      │ Compare │      │ Choose  │      │  Score  │           │
│   │ Current │─────►│   to    │─────►│Strategy │─────►│ Orders  │           │
│   │   TOR   │      │ Target  │      │ PPF/TOR │      │         │           │
│   └─────────┘      └─────────┘      └─────────┘      └─────────┘           │
│                                                                             │
│   If TOR > Target: Select closer racks (reduce travel overhead)             │
│   If TOR < Target: Can afford farther racks (maximize PPF)                  │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Task Distribution Quality:**

| Metric | Formula | Healthy | Impact |
|--------|---------|---------|--------|
| VA Load Variance | `σ²(tasks per VA)` | Low | VTM utilization balance |
| PPS Load Variance | `σ²(tasks per PPS)` | Low | Operator utilization |
| Max VA Load Ratio | `max(tasks) / avg(tasks)` | < 1.5× | Hotspot indicator |
| PPS Starvation Count | `# PPS with 0 tasks when orders available` | 0 | Operator idle time |

---

### Level 2: Effective Robots

```
                      # Effective Robots (VTM or HTM)
                                   ║
                    ═══════════════╬═══════════════
                    ║              ║              ║
               # Deployed    Availability    Utilization
                    │              │              │
               [PHYSICAL]    ─────┴─────    ─────┴─────
                             │         │    │         │
                           Uptime    Error Task      Blocking
                           Rate      Rate  Avail     Losses
                             │         │     │         │
                          [PHYS]   [PHYS]   │    ─────┴─────
                                           │    │         │
                                      [PICK + Traffic   Waiting
                                       MVTS]  Conflicts on Buffer
                                               │         │
                                             [NAV]    [MVTS]
```

**Idle Robot Diagnosis:**

| VTM State | HTM State | Diagnosis |
|-----------|-----------|-----------|
| Idle | Busy | HTM bottleneck or relay buffer draining |
| Busy | Idle | VTM bottleneck or relay buffer filling |
| Both Idle | Both Idle | PICK not releasing tasks or low demand |
| Both Busy | Both Busy | System at capacity |

---

## Layer Responsibility Matrix

```
┌─────────────┬───────────────────────────────────────────────────────────────┐
│   Layer     │                    Metrics Influenced                         │
├─────────────┼───────────────────────────────────────────────────────────────┤
│             │ • Picks/Tote, Orderlines/Tote (demand side)                   │
│   PICK      │ • System TOR, PPS TOR (control setpoints, output)             │
│   (Order    │ • Rack Distance → VTM Travel Time                             │
│  Processing)│ • Task Distribution (VA balance, PPS balance)                 │
│             │ • TOR Deviation (triggers strategy switch)                    │
│             │ • Order Score, Priority                                       │
├─────────────┼───────────────────────────────────────────────────────────────┤
│             │ HTM Scheduling:                                               │
│             │ • HTM Cycle Time, HTM TOR (input from PICK)                   │
│             │ • Station Assignment, Multi-stop Routing                      │
│             │ • Queue Time, Operator Idle Time                              │
│   MVTS      │                                                               │
│   (Task     │ VTM Scheduling:                                               │
│  Scheduling │ • VTM Aisle Selection, Task Selection                         │
│  + Relay)   │ • Storable→Relay vs Relay→Storable Priority                   │
│             │                                                               │
│             │ Relay Management:                                             │
│             │ • Relay Occupancy %, Dwell Time, Flow Balance                 │
│             │ • Relay Position Assignment                                   │
│             │ • Back-to-Store Location Selection                            │
│             │ • Blocking Losses, Starvation Losses                          │
├─────────────┼───────────────────────────────────────────────────────────────┤
│             │ • Path Planning, Actual Travel Times                          │
│ NAVIGATION  │ • Traffic Conflicts, Congestion                               │
│             │ • Nav Factor (actual / theoretical travel ratio)              │
│             │ • Aisle Changes overhead                                      │
├─────────────┼───────────────────────────────────────────────────────────────┤
│             │ • Robot Speeds (VTM horizontal, VTM vertical, HTM)            │
│             │ • Fleet Size (# VTMs, # HTMs)                                 │
│  PHYSICAL   │ • # Stations, # Relay Positions, # Aisles                     │
│             │ • Rack Depth (N-deep), Aisle Geometry                         │
│             │ • Operator Speed (picks/min)                                  │
│             │ • Handoff/Mechanism Times                                     │
│             │ • Uptime, Error Rate, Availability                            │
└─────────────┴───────────────────────────────────────────────────────────────┘
```

**TOR Flow Between Layers:**

```
┌──────────┐         ┌──────────┐         ┌──────────┐
│   PICK   │  TOR    │   MVTS   │  Tasks  │   NAV    │
│  (Order  │────────►│  (Task   │────────►│  (Path   │
│Processing)│ target  │Scheduling)│ assigned │ Planning)│
└──────────┘         └──────────┘         └──────────┘
     │                    │                     │
     │ Controls:          │ Controls:           │ Controls:
     │ • Which totes      │ • Which bot         │ • Actual path
     │ • System TOR       │ • When to dispatch  │ • Travel time
     │ • PPS TOR          │ • HTM TOR           │ • Nav factor
     │ • Task distrib.    │ • Relay buffer      │
```

---

## Use Cases

### 1. Bottleneck Diagnosis

**Scenario:** UPH is 280, target is 400

```
Step 1: Top-level split
  └─ Picks/Tote = 3.2 (target: 3.0) ✓
  └─ Tote Presentations/Hr = 87.5 (target: 133) ✗ LOW

Step 2: Check Relay Buffer
  └─ Occupancy = 15% ✗ STARVATION ZONE
  └─ HTMs starving for totes

Step 3: Drill into VTM Loop (supply side of buffer)
  └─ VTM Throughput: 90/hr ✗ LOW
  └─ # Effective VTMs: 4.2 (of 5) ✓
  └─ VTM Cycle Time: 2.8 min ✗ HIGH

Step 4: Drill into VTM Cycle Time
  └─ Fetch Time: 0.8 min ✓
  └─ Travel Time: 1.6 min ✗ HIGH (Rack→Relay distance)
  └─ Wait Time: 0.4 min ✓

Step 5: Identify Layer
  └─ Travel Time high → Rack Distance → PICK (tote selection)

DIAGNOSIS: PICK selecting distant totes, causing VTM travel overhead.
           Relay buffer draining, HTMs starving.
RECOMMENDATION: Adjust TOR target or tote selection strategy.
```

### 2. Target Setting

**Goal:** Achieve 500 UPH

```
UPH = Picks/Tote × Tote Presentations/Hr
500 = 3.0 × Tote Presentations/Hr
→ Need: 167 Tote Presentations/Hr

Check HTM capacity:
  167 totes/hr with HTM cycle = 2.0 min = 0.033 hr
  → Need: 167 × 0.033 = 5.5 effective HTMs
  → With 85% utilization: need 7 deployed HTMs
  → Target TOR_htm = 7 / 10 operators = 0.7

Check VTM capacity:
  167 totes/hr with VTM cycle = 1.5 min = 0.025 hr
  → Need: 4.2 effective VTMs ✓ (have 5)
  → Target TOR_vtm = 5 / 10 operators = 0.5

Check Relay Buffer:
  → Ensure steady-state occupancy 20-80%
  → VTM arrival ≈ HTM departure at 167 totes/hr
```

### 3. Sensitivity Analysis

**Question:** "If PICK reduces average tote distance by 20%, what's UPH impact?"

```
Tote Distance ↓20%
    ↓
VTM Travel Time ↓20%
    ↓
VTM Cycle Time ↓~13% (Travel is ~65% of cycle)
    ↓
VTM Throughput ↑~15%
    ↓
Relay Buffer fills faster (if HTM unchanged)
    ↓
IF VTM was bottleneck → UPH ↑~15%
IF HTM was bottleneck → Buffer fills, VTMs eventually block
IF Balanced → Need to also improve HTM to realize gains
```

### 4. Responsibility Attribution

**Question:** "UPH is 320 vs target 400. Who's responsible?"

| Layer | Metric | Actual | Optimal | Gap | UPH Impact |
|-------|--------|--------|---------|-----|------------|
| PICK | Picks/Tote | 2.8 | 3.2 | -12.5% | -40 UPH (50%) |
| PICK | TOR Deviation | +15% | ±5% | +10% | Indirect |
| MVTS | HTM cycle | 2.4 min | 2.0 min | +20% | -25 UPH (31%) |
| NAV | Traffic losses | 3% | 1% | +2% | -8 UPH (10%) |
| PHYSICAL | Uptime | 92% | 95% | -3% | -7 UPH (9%) |

**Attribution:** PICK 50%, MVTS 31%, NAV 10%, PHYSICAL 9%

---

## Appendix A: Mathematical Notation

### Variables

**TOR (Transport Overhead Ratio):**
- `TOR_vtm` = VTM transport overhead ratio
- `TOR_htm` = HTM transport overhead ratio
- `TOR_system` = System-level TOR (work-weighted)
- `TOR_pps` = Per-station TOR

**Time Variables (in minutes unless noted):**
- `T_vtm` = VTM cycle time
- `T_htm` = HTM cycle time
- `T_fetch` = VTM fetch time (vertical move + extract)
- `T_travel` = Travel time
- `T_handoff` = Handoff time
- `T_wait` = Wait time
- `T_pickup` = HTM pickup time
- `T_station` = Station time (queue + pick)
- `T_return` = HTM return time
- `T_dwell` = Tote dwell time at relay

**Count Variables:**
- `N_vtm` = Number of deployed VTMs
- `N_htm` = Number of deployed HTMs
- `N_stn` = Number of stations
- `N_relay` = Number of relay positions
- `N_eff_vtm` = Number of effective VTMs
- `N_eff_htm` = Number of effective HTMs

**Rate Variables:**
- `η_avail` = Availability (0-1)
- `η_util` = Utilization (0-1)
- `η_uptime` = Uptime rate (0-1)
- `η_error` = Error rate (0-1)

**Distance Variables:**
- `D_horiz` = Horizontal distance (rack to relay)
- `D_r2s` = Relay to station distance
- `D_s2s` = Station to station distance

### Formulas

**Root:**
```
UPH = P_tote × TPH
where:
  P_tote = Picks per tote
  TPH = Tote presentations per hour
```

**TOR Calculations:**
```
TOR_vtm = ((1 + RF_vtm) × T_vtm_travel) / T_service
TOR_htm = ((1 + RF_htm) × T_htm_travel) / T_service
TOR_total = TOR_vtm + TOR_htm + 1

Target_TOR_vtm = V / O  (VTMs / Operators)
Target_TOR_htm = H / O  (HTMs / Operators)

Pool TOR (work-weighted):
  Pool_TOR = Σ(Travel_i + Service_i) / Σ Service_i
```

**Tote Presentations:**
```
TPH = min(TPH_vtm, TPH_htm, TPH_stn) × (1 - L_block) × (1 - L_idle)
where:
  L_block = Blocking losses (relay full)
  L_idle = Idle losses (relay empty)
```

**Relay Buffer:**
```
Occupancy = Totes_in_buffer / N_relay × 100%
Dwell_Time = Avg(T_drop_to_pickup)
Flow_Balance = (λ_vtm - μ_htm) / μ_htm
where:
  λ_vtm = VTM arrival rate (totes/hr)
  μ_htm = HTM departure rate (totes/hr)
```

**Throughput by System:**
```
TPH_vtm = (N_eff_vtm × 60) / T_vtm
TPH_htm = (N_eff_htm × 60) / T_htm
TPH_stn = N_stn × Rate_stn
```

**Effective Robots:**
```
N_eff = N_deployed × η_avail × η_util
η_avail = η_uptime × (1 - η_error)
```

**VTM Cycle Time:**
```
T_vtm = T_fetch + T_travel_vtm + T_handoff + T_wait

T_fetch = T_vert_move + T_extract
T_vert_move = H / V_vtm_vert
T_travel_vtm = D_horiz / V_vtm_horiz + T_aisle_change
```

**HTM Cycle Time:**
```
T_htm = T_pickup + T_travel_htm + T_station + T_return

T_travel_htm = D_r2s / V_htm + (N_stops - 1) × D_s2s / V_htm
T_station = T_queue + T_pick
T_pick = N_picks_stop / Rate_operator
```

### Causal Relationships (Non-Formula)

These relationships are directional but magnitude varies by site:

```
↑ Tote selection quality   →  ↑ Orderlines/Tote (PICK)
↑ SKU affinity in orders   →  ↑ Orderlines/Tote (External)
↑ Rack-relay distance      →  ↑ VTM Travel Time (PICK)
↑ Station imbalance        →  ↑ Queue Time (MVTS)
↑ Traffic density          →  ↑ Travel Time (NAV)
↑ Multi-stop routing       →  ↑ T_htm but ↑ picks/cycle (MVTS)
↑ Relay occupancy > 80%    →  ↑ Blocking losses (MVTS)
↑ Relay occupancy < 20%    →  ↑ Idle losses (MVTS)
↑ VA load imbalance        →  ↑ VTM congestion (PICK)
```

---

## Appendix B: Technical Metrics (Algorithmic Details)

These metrics are important for system debugging but are algorithmic internals rather than direct UPH drivers.

### Set Cover Algorithm (PICK)

Finds minimum racks/totes to fulfill an order. NP-hard, uses greedy heuristic.

| Metric | Description | Threshold |
|--------|-------------|-----------|
| `set_cover_latency_p99` | Algorithm execution time | < 200ms |
| `optimal_set_found_ratio` | % valid solutions within K=100 limit | > 95% |
| `cache_invalidation_rate` | Recalculation frequency | Low = stable inventory |

### Microclustering (PICK)

PPS → Virtual Aisle affinity management.

| Metric | Description | Purpose |
|--------|-------------|---------|
| `microcluster_TOR_vtm` | Avg TOR within cluster | Retrieval efficiency |
| `microcluster_VTM_util` | VTM utilization in cluster | Congestion indicator |
| `VA_task_count` | Tasks per Virtual Aisle | Load balance |
| `VARC` | Virtual Aisle Retrieval Capacity | Hard limit |
| `VASC` | Virtual Aisle Shedding Capacity | Soft threshold |

### Order Scoring (PICK)

```
Score(k) = Base - (System_Deviation × w_sys) - (PPS_Deviation × w_pps)

Where:
  System_Deviation = |TOR_target - TOR_system_with_order_k|
  PPS_Deviation = |TOR_target - TOR_pps_with_order_k|
```

### HTM Job Shop Scheduling (MVTS)

Lexicographic objectives (priority order):
1. Maximize critical task assignment
2. Maximize PBT-approaching tasks
3. Maximize total assigned tasks
4. Maximize priority-weighted assignment
5. Maximize completed bins
6. Minimize operator idle time
7. Minimize makespan
8. Maximize distance for ties (pre-position bots)

### Relay Hysteresis (MVTS)

Prevents mode oscillation when occupancy hovers near threshold.

| Parameter | Description |
|-----------|-------------|
| `T_low` | Enter "protect relay" mode |
| `T_high` | Exit "protect relay" mode (`T_high > T_low`) |

### Back-to-Store Algorithm (MVTS)

Selects storable location for returning totes.

| Factor | Weight | Description |
|--------|--------|-------------|
| Task Count | Configurable | Fewer tasks = preferred |
| Occupancy Ratio | Configurable | Lower occupancy = preferred |
| IDC Distance | Configurable | Closer = preferred |

---

## Document History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | - | - | Initial problem statement |
| 2.0 | 2025-01 | - | Enhanced with implementation alignment: Two-loop architecture, TOR control, Relay buffer metrics, VTM task types, Layer matrix update |
