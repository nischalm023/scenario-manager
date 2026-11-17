# Relay System Metrics Hierarchy - Problem Statement

## Context

GreyOrange Relay systems achieve goods-to-person picking through coordinated:
- **VTMs** (Vertical Tote Movers): Fetch totes from racks, deliver to relay positions
- **HTMs** (Horizontal Tote Movers): Move totes from relay positions to pick stations
- **Relay Positions**: Buffer/handoff points between VTM and HTM
- **Pick Stations**: Where operators pick items from totes

**Primary KPI: UPH (Units Per Hour)**

---

## Problem

Create a formal metrics hierarchy ("metric tree") for Relay systems that:

1. **Decomposes UPH** through two relationship types:
   - **FORMULA** (═══): Exact mathematical relationship (A = B × C)
   - **CAUSAL** (───): Directional influence, magnitude varies by site/conditions

2. **Maps each metric to its control layer:**
   - **PIC**: Order selection, tote/SKU selection
   - **ORCHESTRATION**: Robot↔task assignment, relay allocation, HTM↔station pairing
   - **NAVIGATION**: Path planning
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
═══════════════════
Formula: UPH = Picks/Tote × Tote Presentations/Hour
```

### Level 1: Primary Split

| Branch | Description | Controlled By |
|--------|-------------|---------------|
| **Picks/Tote** | Demand side: value extracted per tote presentation | PIC |
| **Tote Presentations/Hour** | Supply side: system throughput | All layers |

---

### Demand Side: Picks/Tote

```
                    Picks/Tote
                        ║
            ════════════╬════════════
            ║                        ║
    Orderlines/Tote           Quantity/Orderline
            │                        │
       ─────┴─────                   │
       │         │               [EXTERNAL]
   SKU Affinity  Tote Selection   Customer
   in Orders     Strategy         order data
       │              │
   [EXTERNAL]     [PIC]
```

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| Picks/Tote | FORMULA | - | Orderlines/Tote × Qty/Orderline |
| Orderlines/Tote | CAUSAL | PIC | Order lines fulfillable from one tote |
| Quantity/Orderline | EXTERNAL | Customer | Average units per line item |
| SKU Affinity | EXTERNAL | Customer | Co-occurrence of SKUs across orders |
| Tote Selection Strategy | CONTROL | PIC | Algorithm selecting which tote to fetch |

**Key Insight:** PIC influences Orderlines/Tote through smart tote selection (picking totes with SKUs needed by multiple open orders).

---

### Supply Side: Tote Presentations/Hour

```
                    Tote Presentations/Hour
                            ║
        ════════════════════╬════════════════════
        ║                   ║                   ║
   VTM System          HTM System          Station System
   Throughput          Throughput          Throughput
        ║                   ║                   ║
   # Effective VTMs    # Effective HTMs    # Stations ×
   ────────────────    ────────────────    Picks/Station/Hr
    VTM Cycle Time      HTM Cycle Time


        ┌─────────────────────────────────────────┐
        │  RELAY BUFFER (Connecting VTM ↔ HTM)    │
        │                                         │
        │  # Relay Positions    Blocking Rate     │
        │        │                   │            │
        │   [PHYSICAL]         [ORCHESTRATION]    │
        └─────────────────────────────────────────┘
```

**Throughput Formula:**
```
Actual Throughput = min(VTM capacity, Relay capacity, HTM capacity, Station capacity)
                    × (1 - blocking_losses)
                    × (1 - idle_losses)
```

---

### VTM Cycle Time Decomposition

```
                         VTM Cycle Time
                              ║
        ══════════════════════╬══════════════════════════
        ║           ║                  ║              ║
    Fetch Time   Travel Time      Handoff Time    Wait Time
        ║           ║                  │              │
   ═════╬═════   ═══╬═══              │         ─────┴─────
   ║         ║   ║     ║              │         │         │
Vertical  Extract  Horiz  Vertical    │     VTM Blocked  HTM Not
Movement  Time    Move   Move         │     at Relay     Ready
   ║         │      ║      ║          │         │         │
[PHYSICAL] [PHYSICAL] ║   [PHYSICAL]  │    [ORCHESTRATION]
                      ║          [PHYSICAL]
                 ═════╬═════
                 ║         ║
            Horizontal   Aisle
            Distance    Changes
                 │         │
            ─────┴───  ────┴────
            │       │  │       │
         Rack    Relay Layout  Nav
         Location  Pos  Geometry Factor
            │       │      │       │
         [PIC]  [ORCH] [PHYSICAL] [NAV]
```

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| VTM Cycle Time | FORMULA | - | Fetch + Travel + Handoff + Wait |
| Fetch Time | FORMULA | Physical | Vertical Move + Extract Time |
| Vertical Movement | FORMULA | Physical | Height / Lift Speed |
| Extract Time | CAUSAL | Physical | Depends on N-deep, telescopic |
| Travel Time | FORMULA | Multiple | Horizontal + Vertical components |
| Horizontal Distance | CAUSAL | PIC + Orch | Rack-to-relay distance |
| Aisle Changes | CAUSAL | Physical + Nav | Layout navigation complexity |
| Handoff Time | PHYSICAL | Physical | Fixed by mechanism |
| Wait Time | CAUSAL | Orchestration | Blocking + HTM readiness |

**Key Insight:** PIC's tote selection directly influences Horizontal Distance. Selecting totes closer to assigned relay positions reduces VTM cycle time.

---

### HTM Cycle Time Decomposition

```
                              HTM Cycle Time
                                    ║
        ════════════════════════════╬════════════════════════════
        ║              ║                    ║                ║
   Pickup Time    Travel Time         Station Time      Return Time
        │              ║                    ║                ║
   [PHYSICAL]     ═════╬═════          ═════╬═════      ════╬════
                  ║         ║          ║         ║      ║       ║
              Relay→Stn  Stn→Stn    Wait for   Pick  Travel  Putaway
              Distance   Distance   Operator   Time    to      Time
                  │         │          │         │   Storage    │
             ─────┴───  ────┴────     ─────┴─────┴─────  │   [PHYSICAL]
             │       │  │       │     │               │  │
          Relay   Station   # of   Queue    Operator  # Picks
          Pos     Assign   Stops   Time     Speed     per Stop
             │       │       │       │         │         │
        [ORCH]   [ORCH]  [ORCH]  [ORCH]   [PHYSICAL]   [PIC]
```

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| HTM Cycle Time | FORMULA | - | Pickup + Travel + Station Time + Return |
| Pickup Time | PHYSICAL | Physical | Relay handoff mechanism |
| Travel Time | FORMULA | Multiple | Distance / Speed |
| Relay→Station Distance | CAUSAL | Orchestration | Layout + station assignment |
| Station→Station Distance | CAUSAL | Orchestration | Multi-stop routing |
| # of Stops | CONTROL | Orchestration | Routing decision |
| Station Time | FORMULA | Multiple | Wait + Pick Time |
| Queue Time | CAUSAL | Orchestration | Station load balancing |
| Operator Speed | PHYSICAL | Physical | Picks per minute |
| Picks per Stop | CAUSAL | PIC | Order consolidation |
| Return Time | FORMULA | Physical | Travel + Putaway |

**Multi-Stop Trade-off:**
- Single stop: Lower cycle time, fewer picks per cycle
- Multi-stop: More picks per cycle, longer cycle time
- Decision by: ORCHESTRATION (influenced by PIC's order consolidation)

---

### Effective Robots

```
                      # Effective Robots
                             ║
              ═══════════════╬═══════════════
              ║              ║              ║
         # Deployed    Availability    Utilization
              │              │              │
         [PHYSICAL]    ─────┴─────    ─────┴─────
                       │         │    │         │
                     Uptime    Error  Task     Blocking
                     Rate      Rate   Avail    Losses
                       │         │      │         │
                  [PHYSICAL] [PHYSICAL] │    ─────┴─────
                                        │    │         │
                                   [PIC +  Traffic   Waiting
                                    ORCH]  Conflicts  on Other
                                             │        System
                                          [NAV]    [ORCH]
```

**Metrics:**

| Metric | Type | Controlled By | Description |
|--------|------|---------------|-------------|
| # Effective | FORMULA | - | Deployed × Availability × Utilization |
| # Deployed | INPUT | Physical/Business | Fleet size |
| Availability | FORMULA | Physical | Uptime × (1 - Error Rate) |
| Utilization | CAUSAL | Multiple | Task availability, blocking |
| Task Availability | CAUSAL | PIC + Orch | Work in queue |
| Traffic Conflicts | CAUSAL | Navigation | Robot congestion |
| Waiting on Other System | CAUSAL | Orchestration | VTM↔HTM sync |

**Idle Robot Diagnosis:**
- VTMs idle, HTMs busy → HTM bottleneck
- HTMs idle, VTMs busy → VTM bottleneck
- Both idle → PIC not releasing tasks or low demand

---

## Layer Responsibility Matrix

| Layer | Metrics Influenced |
|-------|-------------------|
| **PIC** | Orderlines/Tote, VTM Horizontal Distance, Picks per Stop, Task Availability |
| **ORCHESTRATION** | VTM Wait Time, HTM Travel Distance, # of Stops, Queue Time, Blocking Losses, Relay Position Blocking |
| **NAVIGATION** | Aisle Changes, Traffic Conflicts |
| **PHYSICAL** | Robot Speed, Rack Depth, # Relay Positions, # Stations, Operator Speed, Handoff Times, Fleet Size, Uptime, Error Rate |

---

## Use Cases

### 1. Bottleneck Diagnosis

**Scenario:** UPH is 280, target is 400

```
Step 1: Top-level split
  └─ Picks/Tote = 3.2 (target: 3.0) ✓
  └─ Tote Presentations/Hr = 87.5 (target: 133) ✗ LOW

Step 2: Drill into Tote Presentations
  └─ VTM Throughput: 150/hr ✓
  └─ HTM Throughput: 95/hr ✗ BOTTLENECK
  └─ Station Capacity: 160/hr ✓

Step 3: Drill into HTM System
  └─ # Effective HTMs: 4.2 (of 5) ✓
  └─ HTM Cycle Time: 2.6 min ✗ HIGH

Step 4: Drill into HTM Cycle Time
  └─ Travel Time: 1.4 min ✗ HIGH
  └─ Station Time: 0.9 min ✓

Step 5: Identify Layer
  └─ Travel Time → Relay→Station distance → ORCHESTRATION

DIAGNOSIS: Orchestration station assignment causing excessive HTM travel.
```

### 2. Target Setting

**Goal:** Achieve 500 UPH

```
UPH = Picks/Tote × Tote Presentations/Hr
500 = 3.0 × Tote Presentations/Hr
→ Need: 167 Tote Presentations/Hr

Tote Presentations/Hr = # Effective HTMs / HTM Cycle Time
If HTM Cycle Time = 2.0 min = 0.033 hr:
→ Need: 167 × 0.033 = 5.5 effective HTMs
→ With 85% utilization: need 7 deployed HTMs

Check VTM capacity:
167 totes/hr with VTM cycle = 1.5 min = 0.025 hr
→ Need: 4.2 effective VTMs ✓ (have 5)
```

### 3. Sensitivity Analysis

**Question:** "If PIC reduces average tote distance by 20%, what's UPH impact?"

```
Tote Distance ↓20%
    ↓
VTM Horizontal Move Time ↓20%
    ↓
VTM Travel Time ↓~15% (Horiz is ~75% of travel)
    ↓
VTM Cycle Time ↓~10% (Travel is ~65% of cycle)
    ↓
VTM Throughput ↑~11%
    ↓
IF VTM was bottleneck → UPH ↑~11%
IF NOT bottleneck → UPH unchanged
```

### 4. Responsibility Attribution

**Question:** "UPH is 320 vs target 400. Who's responsible?"

| Layer | Actual | Optimal | Gap | UPH Impact |
|-------|--------|---------|-----|------------|
| PIC (Picks/Tote) | 2.8 | 3.2 | -12.5% | -40 UPH (50%) |
| ORCH (HTM cycle) | 2.4 min | 2.0 min | +20% | -25 UPH (31%) |
| NAV (conflicts) | 3% loss | 1% loss | +2% | -8 UPH (10%) |
| PHYSICAL (uptime) | 92% | 95% | -3% | -7 UPH (9%) |

**Attribution:** PIC 50%, Orchestration 31%, Navigation 10%, Physical 9%

---

## Deliverables

A complete metric tree document with:
- [ ] All metrics and their relationships (formula vs causal)
- [ ] Control layer mapping for each metric
- [ ] Measurement methodology for each metric
- [ ] Worked examples for all four use cases

---

## Appendix: Mathematical Notation

### Variables

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

**Speed Variables:**
- `V_vtm_horiz` = VTM horizontal speed
- `V_vtm_vert` = VTM vertical speed
- `V_htm` = HTM travel speed

### Formulas

**Root:**
```
UPH = P_tote × TPH
where:
  P_tote = Picks per tote
  TPH = Tote presentations per hour
```

**Tote Presentations:**
```
TPH = min(TPH_vtm, TPH_relay, TPH_htm, TPH_stn) × (1 - L_block) × (1 - L_idle)
where:
  L_block = Blocking losses
  L_idle = Idle losses
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

**Picks per Tote:**
```
P_tote = OL_tote × Q_ol
where:
  OL_tote = Orderlines per tote
  Q_ol = Quantity per orderline
```

### Causal Relationships (Non-Formula)

These relationships are directional but magnitude varies:

```
↑ Tote selection quality  →  ↑ OL_tote (PIC)
↑ SKU affinity in orders  →  ↑ OL_tote (External)
↑ Rack-relay distance     →  ↑ D_horiz (PIC, Orchestration)
↑ Station imbalance       →  ↑ T_queue (Orchestration)
↑ Traffic density         →  ↑ T_travel (Navigation)
↑ Multi-stop routing      →  ↑ T_htm but ↑ picks/cycle (Orchestration)
```
