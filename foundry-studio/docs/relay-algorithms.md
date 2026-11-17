# Relay System Algorithms

> **Status**: Work in Progress
> **Last Updated**: 2025-11-27
> **Author**: Kumar Devvrat

---

## 1. Problem Statement

Traditional Goods-to-Person (GTP) systems use a Racks-to-Picker (R2P) model where general-purpose robots carry entire racks to operator stations. While simple, this approach hits fundamental scaling limits: racks are heavy and slow, robots compete for shared aisles, and a single robot type creates bottlenecks at every stage of the fulfillment cycle.

The Relay System addresses these limitations by decomposing the transport problem into two specialized stages. Vertical Tote Movers (VTMs) operate within confined zones, extracting individual totes from racks and delivering them to intermediate relay positions. Horizontal Tote Movers (HTMs) then transport totes from relay positions to operator stations. This separation enables parallel processing, reduces travel distances, and creates natural buffer points that absorb demand variability.

The core insight is that **totes, not racks, are the right unit of transport**. A tote weighs less, moves faster, and carries only the inventory needed for immediate orders. By moving totes instead of racks, the system achieves higher throughput with fewer robots while maintaining the flexibility to scale each stage independently.

### Success Criteria

The system succeeds when operators experience continuous work without waiting. Every algorithm in this document ultimately serves this goal: minimizing operator idle time while maximizing picks per hour per station. Secondary metrics include robot utilization, relay throughput, and zone balance—but these are means to the end, not the end itself.

---

## 2. Non-Goals

- **Real-time order injection**: This system processes batch orders loaded at simulation start. Dynamic order arrival during execution is out of scope.

- **Multi-facility coordination**: The algorithms assume a single warehouse. Cross-facility inventory balancing and order routing are not addressed.

- **Robot hardware optimization**: Load times, speeds, and capacities are treated as fixed parameters. Mechanical improvements are outside this scope.

- **Pick-to-box consolidation**: The high-throughput variant where HTMs visit multiple stations per cycle is documented conceptually but not implemented.

---

## 3. Tenets

These tenets guide algorithm design and resolve conflicts when requirements compete. They represent the system's values—what we optimize for when we can't have everything.

### 3.1 Operators Should Not Wait

This is the north star. When any decision involves a tradeoff between robot efficiency and operator continuity, operator continuity wins. An idle operator is lost throughput that cannot be recovered; an idle robot is a buffer against future variance.

### 3.2 Navigation Guides Design

Path planning is not an afterthought retrofitted onto algorithms that ignore physical space. Order processing, inventory placement, and task scheduling must create conditions where robots can navigate without congestion. Algorithms that optimize locally but create grid-level traffic jams fail the system.

### 3.3 Each Module Creates Opportunity for Downstream

Modules do not operate in isolation. Order Processing must select orders that Task Scheduling can execute efficiently. Task Scheduling must position totes where Relay Management can route them smoothly. Each module's output should include headroom for the next module's constraints—not just meet minimum requirements but create favorable conditions.

### 3.4 Predictable Over Peak

Consistency beats heroics. A system that sustains 950 picks/hour is more valuable than one that spikes to 1100 and drops to 700. Algorithms should dampen variance, not amplify it. Target a bounded range (±10%) rather than maximum theoretical throughput.

### 3.5 Backpressure Over Failure

When load exceeds capacity, the system must slow down, not break down. Robots should idle before resources saturate. Queues should apply pressure upstream before overflowing. Graceful degradation preserves the ability to recover; deadlock does not.

---

## 4. System Model

The Relay System comprises four primary components that interact through well-defined interfaces.

### Components

- **Racks**: Store totes in vertical stacks within rack blocks. Each rack has a fixed number of tote slots (typically 4-6) and occupies a grid position. Racks are stationary; only totes move.

- **Totes**: The transport unit. Each tote holds inventory for one or more SKUs and can be assigned up to 6 pick tasks. A tote's lifecycle moves through states: in-rack, in-transit (with VTM), at-relay, in-transit (with HTM), at-station, and returning.

- **Relay Positions**: Handoff points between VTMs and HTMs. Each relay position (a cantilever structure) holds exactly one tote. Relays are organized by zone and create the buffer that decouples VTM and HTM operations.

- **Stations**: Where operators pick items from totes. Each station has a queue capacity (bins) and one operator resource. Totes arrive, operators pick, and totes depart for return.

### Robot Types

- **VTMs (Vertical Tote Movers)**: Operate within assigned zones. A VTM extracts a tote from a rack, transports it to a relay position within its zone, and returns empty totes from relay to rack. VTMs are zone-bound—they cannot cross zone boundaries.

- **HTMs (Horizontal Tote Movers)**: Operate globally. An HTM picks totes from any relay position, delivers them to stations, processes picks with operators, and returns totes to relay positions. HTMs are not zone-constrained.

### Zones

A zone is a vertical slice of the warehouse corresponding to an aisle between rack blocks. Each zone owns the relay positions on its boundaries and tracks capacity as the sum of occupied and reserved relay slots. VTMs are assigned to zones; the assignment balances workload across the warehouse.

### Constraints

| Constraint | Value | Implication |
|------------|-------|-------------|
| Relay capacity | 1 tote | Simple blocking semantics, no queuing |
| Tote task limit | 6 tasks | Bounds work per tote cycle |
| VTM zone binding | Strict | VTMs cannot help other zones |
| VTM load/unload | 9s each | Fixed transfer overhead |
| HTM load/unload | 15s each | Longer due to station interaction |

---

## 5. System Flow

This section describes how work moves through the system, from order arrival to fulfillment.

### Happy Path

Orders enter the system during initialization and are assigned to stations based on bin availability and TPQ optimization. Once assigned, orders generate pick tasks that are bound to specific totes containing the required SKUs.

A VTM in the tote's zone detects the pending task. It reserves a relay position, claims the tote to prevent duplicate selection, and travels to the rack. After loading (9s), the VTM transports the tote to the reserved relay position and unloads (9s). The tote is now available for HTM pickup, and the storage space is marked active for piggybacking opportunities.

An HTM scanning for work finds the tote at the relay. It claims the tote, travels to the relay, loads (15s), and transports to the target station. At the station, the HTM acquires the operator resource (blocking if the operator is busy), and the operator picks items from the tote. Once complete, the HTM reserves a return relay position and rack slot atomically, transports to the relay, and unloads the tote.

When zone utilization exceeds 80%, VTMs prioritize return journeys. A VTM finds an empty tote at a relay in its zone, retrieves the pre-reserved rack slot information, and returns the tote to storage. This clears relay capacity and prevents congestion from blocking forward progress.

```
┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
│  Orders  │────▶│  Tasks   │────▶│   VTM    │────▶│  Relay   │────▶│   HTM    │
│          │     │ (Totes)  │     │ Forward  │     │ Buffer   │     │ Station  │
└──────────┘     └──────────┘     └──────────┘     └──────────┘     └──────────┘
                                        ▲                                │
                                        │         ┌──────────┐           │
                                        └─────────│   VTM    │◀──────────┘
                                                  │  Return  │
                                                  └──────────┘
```

### Degraded Paths

The system degrades gracefully under stress. When all relay positions fill, VTMs with totes block at the relay—but backpressure ensures most VTMs idle before picking up totes, limiting blocked robots. When no totes have tasks, VTMs enter idle polling (2s intervals) rather than spinning. When HTMs cannot find work, stations idle and throughput drops, but the system remains stable and recovers when totes arrive.

| Condition | Detection | Response |
|-----------|-----------|----------|
| Relay saturation | Zone utilization ≥80% | VTMs prioritize returns, then idle |
| No pending tasks | VTM finds no totes | Idle with 2s polling |
| HTM starvation | No totes at relays | HTM idles, stations wait |
| Rack slot exhaustion | No empty slots in zone | HTM selects different zone for return |

---

## 6. Variations

This document focuses on the **Base Relay** system. A second variant exists conceptually but is not implemented.

### 6.1 Base Relay (This Document)

The standard relay architecture as described throughout this document. VTMs move totes from racks to relay positions; HTMs move totes from relays to stations and back. Each HTM cycle visits exactly one station per tote.

Key characteristics:
- Single station visit per HTM cycle
- Return to relay after each station
- Simple task-to-tote binding

### 6.2 High-Throughput Pick-to-Box (Future)

An enhanced variant designed for higher throughput scenarios where order consolidation happens at stations.

Conceptual differences:
- HTMs may visit multiple stations per cycle
- Orders consolidated directly into shipping boxes at stations
- More complex routing decisions for HTM path planning
- Higher HTM utilization, fewer relay round-trips

**Status**: Not implemented. Documented for future reference.

---

## 7. Critical Bottlenecks

Bottlenecks are where the system chokes under load. Each bottleneck motivated specific algorithms; understanding bottlenecks explains why those algorithms exist.

### 7.1 VTM Throughput Ceiling

VTMs are the producers in the relay system. Their throughput sets the upper bound on totes entering the relay buffer. When VTM capacity is insufficient, relay positions empty faster than they fill, HTMs starve, and stations idle.

**Causes**: Too few VTMs per zone, long travel distances to racks, frequent return journeys consuming forward capacity.

**Detection**: Declining relay occupancy, increasing HTM idle time, station throughput below target.

**Algorithms that address this**: Task Scheduling (VTM lifecycle optimization), Relay Position Management (minimizing VTM travel), Tote & Location Popularity (placing high-demand SKUs closer to relays).

### 7.2 Relay Saturation

Relay positions are the buffer between VTM production and HTM consumption. When relays fill completely, VTMs block, forward progress halts, and the system deadlocks.

**Causes**: HTMs slower than VTMs, station processing delays, imbalanced zone utilization.

**Detection**: Zone utilization approaching 80%, VTMs blocking at relay drop-off, rising blocked-time metrics.

**Algorithms that address this**: Task Scheduling (backpressure control at 80%), Back to Store (prioritizing return relay selection), Zone load balancing in Order Processing.

### 7.3 Zone Load Imbalance

Zones share no resources. An overloaded zone cannot borrow VTMs from an underutilized neighbor. Imbalance means some zones bottleneck while others have spare capacity.

**Causes**: Uneven inventory distribution, hot SKUs clustered in one zone, order patterns favoring specific areas.

**Detection**: Cross-zone utilization variance, per-zone throughput divergence, some VTMs constantly busy while others idle.

**Algorithms that address this**: Tote & Location Popularity (zone-aware placement), Order Processing (zone load bucketing), Back to Store (returning totes to low-load zones).

### 7.4 HTM Starvation

HTMs are the consumers. When they cannot find totes with tasks at relay positions, stations idle and throughput drops directly.

**Causes**: VTM bottlenecks upstream, order patterns with sparse task distribution, excessive return journey priority.

**Detection**: Low relay occupancy of task-bearing totes, increasing HTM idle time, station utilization below target.

**Algorithms that address this**: Order Processing (TPQ optimization batches tasks onto fewer totes), Relay Position Management (positioning totes for efficient HTM access).

### 7.5 Operator Blocking

Operators are the ultimate constraint. If totes arrive faster than operators can pick, queues build and HTMs block waiting for operator resources.

**Causes**: Underestimated pick times, uneven station assignment, variable task complexity.

**Detection**: Station queue at capacity, HTM wait time for operator resource rising, picks per hour declining.

**Algorithms that address this**: Order Processing (station-aware assignment), Station Planner (queue management).

---

## 8. Invariants

Invariants are rules that must always hold. Violating an invariant indicates a bug, not an edge case. These properties are the system's safety guarantees.

### Ownership Invariants

**Single tote claim.** A tote can be claimed by at most one VTM (via `assignedVtmId`) and at most one HTM (via `assignedHtmId`) at any time. Concurrent claims indicate a race condition.

**Single relay reservation.** A relay position can be reserved for at most one tote at any time. The reservation maps are bijective: each relay maps to at most one tote, and each tote maps to at most one relay.

**Rack slot exclusivity.** A hard-reserved rack slot (for return journey) cannot be reserved by another tote until released. The reservation includes both rack ID and z-index.

### Capacity Invariants

**Relay holds one tote.** A relay position's physical capacity is exactly 1. Any state showing multiple totes at one relay is invalid.

**Zone capacity is derived.** Zone available capacity equals physical relay count minus occupied minus reserved. This is computed, not stored—preventing drift between tracked and actual state.

### State Transition Invariants

**Totes follow valid paths.** A tote's location transitions must follow legal paths: InRack → WithVTM → AtRelay → WithHTM → AtStation → WithHTM → AtRelay → WithVTM → InRack. Skipping states (e.g., InRack → AtStation) is invalid.

**Reservations precede placement.** Before a VTM drops a tote at a relay, the relay must be reserved for that tote. Before a VTM returns a tote to a rack slot, that slot must be hard-reserved. Placement without reservation indicates a protocol violation.

### Liveness Invariants

**Backpressure prevents deadlock.** When zone utilization reaches 80%, VTMs must idle or return—not attempt forward journeys. This ensures relay capacity never fully saturates under normal operation.

**Claims are released.** After a journey completes (forward or return), the tote's claim must be cleared. Orphaned claims prevent future selection.

---

## 9. Algorithms

Each algorithm section follows a consistent structure: the bottleneck it addresses, the problem it solves, its approach, key design decisions, and known edge cases.

### 9.1 Tote & Location Popularity

**Bottleneck addressed**: Zone Load Imbalance (7.3), VTM Throughput (7.1)

**Problem**: Where should high-demand inventory live? Random placement creates hotspots; naive distance optimization ignores zone balance. The placement strategy must consider both proximity to stations and even distribution across zones.

**Approach**: The system calculates two complementary popularity scores. Tote popularity measures demand intensity—how frequently the SKUs in a tote are ordered—using a demand-weighted formula that normalizes against maximum order volume. Location popularity measures grid position quality based on average Manhattan distance to the nearest stations.

When zone load balancing is enabled, the system combines these scores with zone utilization feedback. A hybrid formula weights distance (30%) against zone load (70%), preferring underutilized zones even when slightly farther from stations. As SKUs are placed, zone popularity points accumulate; future placements see updated scores reflecting current distribution.

```
Tote Score = Σ(orderedQty[sku] × quantity[sku]) / totalItems / maxOrderedQty

Location Score = 1.0 - (avgDistanceToStations - minDist) / distRange

Hybrid Score = 0.3 × LocationScore + 0.7 × ZoneLoadScore
```

**Key decisions**: The 70/30 weighting favors balance over proximity. This reflects the tenet that consistency beats peak performance—a balanced warehouse sustains throughput better than one with fast zones that overload.

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `distanceWeight` | 0.2–0.5 | Higher values favor proximity to stations. Use 0.3 (default) for balanced warehouses; increase to 0.4–0.5 if zone imbalance is not a concern. |
| `zoneLoadWeight` | 0.5–0.8 | Complement of distance weight. Higher values spread load more evenly at cost of longer average travel. |
| `popularityDecayFactor` | 0.0–1.0 | How quickly historical demand fades. 0.0 = only recent orders; 1.0 = all history weighted equally. Start at 0.7 for stable SKU profiles. |

**Limitations**: Empty storage spaces score 0.0 on tote popularity. Warehouses with single zones ignore zone balancing and fall back to pure distance scoring.

---

### 9.2 Put Strategy

**Bottleneck addressed**: Zone Load Imbalance (7.3), VTM Throughput (7.1)

**Problem**: When returning a tote to storage, which rack slot should receive it? The choice affects future retrieval distance, zone balance, and rack utilization.

**Approach**: TBD — Greedy strategy implementation planned.

**Key decisions**: TBD

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `putAlgorithm` | greedy, min_weighted_cycle_distance | Greedy minimizes immediate travel; min_weighted_cycle_distance optimizes expected future retrieval. Start with greedy for simplicity. |
| `zoneAffinityWeight` | 0.0–1.0 | How strongly to prefer returning totes to their origin zone. 0.0 = pure distance; 1.0 = strict zone affinity. |
| `rackFillThreshold` | 0.7–0.9 | Utilization above which to avoid placing more totes. Prevents over-concentration on popular racks. |

**Limitations**: TBD

---

### 9.3 Order Processing

**Bottleneck addressed**: HTM Starvation (7.4), Operator Blocking (7.5), Zone Load Imbalance (7.3)

**Problem**: Given a pool of pending orders and available station bins, which orders should be selected and how should they be assigned to stations? Poor selection leads to excessive tote movements; poor assignment creates station imbalance.

#### TOR in Relay vs RTP

Transport Overhead Ratio (TOR) measures how much non-productive time (travel) accompanies each unit of productive time (operator picking). The relay system fundamentally changes how TOR is calculated and optimized compared to RTP.

**RTP (Single Loop)**: One robot type carries racks directly to stations. TOR is straightforward:
```
TOR = (2 × T_travel) / S
```
where `T_travel` is rack-to-station distance and `S` is service time. More picks per rack amortizes travel—a rack with 10 picks has lower TOR than one with 2 picks.

**Relay (Two Loops)**: Transport splits into VTM (rack↔relay) and HTM (relay↔station). TOR becomes two-dimensional:
```
TOR_vtm = (2 × T_vtm) / S    [VTM overhead per unit productive work]
TOR_htm = (2 × T_htm) / S    [HTM overhead per unit productive work]
TOR_total = TOR_vtm + TOR_htm + 1
```

**Critical difference**: VTM moves a *tote*, not picks. Whether a tote has 1 pick or 6 picks, VTM travel time is identical. This means TOR_vtm is determined purely by rack-to-relay distance—batching more picks onto a tote reduces TOR_htm (amortizes HTM travel + service) but does NOT reduce TOR_vtm.

**Target TOR derivation**: For a balanced system where no single resource is bottlenecked:
- RTP: Target TOR = B/O (bots per operator)
- Relay: Target TOR_vtm = V/O, Target TOR_htm = H/O (VTMs and HTMs per operator, independently)

**Example**: 8 VTMs, 4 HTMs, 4 operators → Target TOR_vtm = 2.0, Target TOR_htm = 1.0, Target TOR_total = 4.0

**Zone dimension**: Unlike RTP where all racks compete in a single pool, relay adds zone-level balancing. Each zone has its own TOR_vtm distribution, and order selection must spread work across zones to prevent any single zone from becoming a bottleneck.

#### Approach

Order Processing uses TPQ (Task Pool Quality) optimization—a multi-objective scoring function that balances travel efficiency against system constraints.

TPQ measures the ratio of total time (travel + operator) to operator time alone. A TPQ of 1.0 means zero travel overhead (theoretical minimum); higher values indicate more travel per pick. The algorithm targets a pre-calculated TPQ based on warehouse geometry, penalizing orders that push the pool away from this target.

Selection proceeds in two phases. First, orders are grouped into zone load buckets and processed lowest-load-first, spreading work across zones. Within each bucket, the algorithm performs station-first selection: for each station with available bins, it evaluates every unprocessed order and selects the (order, station) pair with the highest score.

Scoring incorporates four factors:
- **TPQ targeting**: Penalize deviation from target TPQ (weight: 100)
- **Rack reuse bonus**: Reward orders that reuse already-active racks (bonus: 10 per rack)
- **Rack overload penalty**: Penalize concentrating >7 picks on one rack (penalty: 20 per excess pick)
- **Station deviation penalty**: Penalize stations drifting from target TPQ (penalty: 20)

**Piggybacking** is central to the approach. Travel time is charged only for the first task on each storage space; subsequent tasks ride free. This encourages batching orders that share totes, dramatically improving TPQ for well-matched orders.

In the relay system, piggybacking is especially valuable because totes pass through multiple states where costs are already paid:

| Tote State | TOR_vtm Cost | TOR_htm Cost | Why |
|------------|--------------|--------------|-----|
| In rack (storage) | Full | Full | New journey required |
| VTM fetching | 0 | Full | VTM journey in progress |
| At relay (waiting) | 0 | Full | VTM done, only HTM remains |
| HTM fetching / At station | 0 | 0 | Both journeys paid |

Totes waiting at relay positions are prime targets—VTM overhead is already paid, so new picks from that tote incur only HTM cost. This can cut TOR by 50% compared to fetching a new tote from storage.

**Key decisions**: The scoring weights were tuned empirically. TPQ weight dominates (100) because travel efficiency compounds across thousands of cycles. Rack reuse and overload terms balance batching benefits against concentration risks.

**Example**: Consider two candidate orders for Station 1, with target TPQ = 2.0:

| Order | Picks | Totes Required | Active Totes Reused | New Totes |
|-------|-------|----------------|---------------------|-----------|
| Order A | 15 | 3 | 2 | 1 |
| Order B | 8 | 1 | 1 | 0 |

Scoring (assuming current pool TPQ = 1.8):
- Order A: Pool TPQ with A = 2.1, deviation = 0.1 → TPQ penalty = -10. Reuse bonus = +20. Score ≈ +10
- Order B: Pool TPQ with B = 1.9, deviation = 0.1 → TPQ penalty = -10. Reuse bonus = +10. Score ≈ 0

Order A selected despite more totes—the reuse bonus and favorable TPQ outweigh the extra tote.

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `targetTPQ` | Auto (V/O + H/O + 1) or manual | Auto-calculated from robot/operator ratios. Override only if warehouse geometry creates systematic bias (e.g., stations clustered on one side). |
| `returnFactor` | 0.5–1.2 | Fraction of outbound travel for return leg. 1.0 = symmetric; <1.0 if robots chain tasks or use landing zones. Calibrate from observed cycles. |
| `navFactor` | 1.2–2.0 | Multiplier for grid navigation overhead vs. straight-line distance. Higher for congested aisles or complex layouts. Start at 1.5. |
| `aStarPenaltyFactor` | 0.8–1.5 | Calibration factor aligning A* estimates to actual travel. Tune empirically by comparing estimated vs. observed travel times. |
| `tpqWeight` | 50–150 | Scoring weight for TPQ deviation. Higher values enforce stricter adherence to target. Default 100. |
| `rackReuseBonus` | 5–20 | Bonus per reused active rack. Higher values favor piggybacking. Default 10. |
| `rackOverloadThreshold` | 5–10 | Picks per rack before penalty applies. Lower values spread work; higher values allow concentration. Default 7. |
| `stationDeviationPenalty` | 10–30 | Penalty for station-level TPQ drift. Higher values enforce station balance. Default 20. |
| `maxTotesPerOrder` | 3–15 | Maximum totes to fulfill one order. Lower values reject complex orders; higher values increase flexibility but may increase travel. |

**Limitations**: Orders requiring more totes than `maxTotesPerOrder` are rejected. Orders with no fulfillable SKUs (inventory depleted) are skipped. Stations at bin capacity receive no new assignments until bins free.

---

### 9.4 Task Scheduling

**Bottleneck addressed**: Relay Saturation (7.2), VTM Throughput (7.1), HTM Starvation (7.4)

**Problem**: How should VTMs and HTMs decide what to do next? The scheduler must balance forward progress (moving totes toward stations) against system health (preventing congestion).

**Approach**: Task Scheduling implements the VTM and HTM lifecycles as continuous loops with priority-based decision making.

**VTM Lifecycle**: Each iteration, a VTM evaluates three priorities in order:

1. **Physical relay congestion**: If any assigned zone has relay utilization ≥80% (counting only occupied positions, not reservations), and empty totes exist at relays, execute a return journey. This clears actual congestion.

2. **Pipeline congestion**: If any assigned zone has utilization ≥80% (counting occupied + reserved), idle. This prevents adding to in-flight work when the pipeline is full.

3. **Forward journey**: If no congestion, find a tote with tasks in an assigned zone, reserve a relay, claim the tote, and execute the forward journey.

The distinction between physical and pipeline congestion is subtle but important. Physical congestion (check 1) means totes are actually blocking relays—return journeys directly help. Pipeline congestion (check 2) means totes are in-flight—adding more would worsen the situation even though relays aren't physically full yet.

**HTM Lifecycle**: HTMs are simpler—they scan for relay positions holding totes with tasks, claim the closest one, and execute a station journey. The complexity lives in return relay selection (see Back to Store).

**Backpressure Control**: The 80% threshold is the critical parameter. Below 80%, VTMs prioritize forward work to maximize throughput. Above 80%, they shift to returns and idling to protect system stability. This threshold was chosen to leave headroom—at 80%, there's still capacity for in-flight totes to land without blocking.

**Key decisions**: Checking physical congestion separately from pipeline congestion prevents oscillation. If VTMs only checked pipeline congestion, they might idle when return work would help. If they only checked physical congestion, they'd keep adding to the pipeline until blocking occurred.

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `congestionThreshold` | 0.7–0.9 | Zone relay utilization triggering backpressure. Lower values are more conservative (idle earlier); higher values maximize throughput at risk of blocking. Default 0.8. |
| `vtmIdleTimeout` | 1–10 seconds | How long VTM waits when pipeline-congested before rechecking. Shorter values increase responsiveness but add overhead. Start at 2–3 seconds. |
| `vtmZoneAssignments` | 1–3 zones per VTM | Number of zones each VTM serves. Single-zone simplifies logic; multi-zone improves flexibility at cost of complexity. Match to warehouse layout. |
| `htmScanRadius` | All or distance-limited | How far HTM searches for relay work. "All" is simplest; distance-limited reduces travel but may miss urgent totes. |

**Limitations**: VTMs with no assigned zones throw an error—this is a configuration bug. Totes already claimed by another robot are skipped. Zones with zero relay capacity are filtered from consideration.

---

### 9.5 Relay Position Management

**Bottleneck addressed**: VTM Throughput (7.1), HTM Starvation (7.4)

**Problem**: When a VTM needs to deliver a tote, which relay position should it use? When an HTM needs to return a tote, which relay position should it target? Poor selection increases travel distances and can strand totes far from where they're needed.

**Approach**: Relay selection uses weighted distance optimization with reservation tracking.

**VTM Relay Selection** (forward journey): The algorithm filters relay positions to those in the VTM's assigned zones that are empty and unreserved. From this set, it selects the position minimizing a weighted sum of VTM travel (from current position to relay) and HTM travel (from relay to target station). The weights are 60% VTM, 40% HTM.

```
score = 0.6 × VTM_distance + 0.4 × HTM_distance
select relay with minimum score
```

The 60/40 split prioritizes immediate efficiency (VTM travel is happening now) while considering downstream impact (HTM travel happens soon). Pure VTM optimization would cluster totes far from stations; pure HTM optimization would ignore VTM travel costs.

**Example**: VTM at position (5, 10), target station at (20, 10). Two candidate relays:

| Relay | Position | VTM Distance | HTM Distance | Score |
|-------|----------|--------------|--------------|-------|
| Relay A | (8, 10) | 3 | 12 | 0.6×3 + 0.4×12 = 6.6 |
| Relay B | (10, 10) | 5 | 10 | 0.6×5 + 0.4×10 = 7.0 |

Relay A selected (score 6.6 < 7.0)—closer VTM travel wins despite slightly longer HTM leg.

**HTM Relay Selection** (return journey): The algorithm selects the closest empty, unreserved relay in the target zone. Simplicity wins here—the complex logic lives in zone selection (see Back to Store), not relay selection within a zone.

**Reservation System**: Reservations use dual-map tracking: one map from relay ID to tote ID, another from tote ID to relay ID. This enables O(1) lookups in both directions and enforces the single-reservation invariant through synchronized access.

**Key decisions**: The 60/40 weighting was empirically tuned. Higher VTM weight (e.g., 80/20) reduces VTM travel but clusters totes, increasing HTM travel variance. Lower VTM weight improves HTM paths but increases total system travel.

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `vtmDistanceWeight` | 0.5–0.8 | Weight for VTM travel in relay selection. Higher values prioritize immediate VTM efficiency. Default 0.6. |
| `htmDistanceWeight` | 0.2–0.5 | Complement of VTM weight. Higher values optimize for downstream HTM travel. Default 0.4. |
| `reservationTimeout` | 30–300 seconds | How long a reservation holds before auto-release. Prevents leaked reservations from blocking relays. Set based on maximum expected cycle time. |
| `relayCapacityPerZone` | 2–10 positions | Number of relay positions per zone. More positions add buffer but require more space. Size based on VTM/HTM ratio and cycle times. |

**Limitations**: If no relay positions are available in a VTM's zones, the system throws an error—this indicates either misconfiguration (too few relays) or a reservation leak. HTMs handle empty zones gracefully by trying the next eligible zone.

---

### 9.6 Back to Store

**Bottleneck addressed**: Relay Saturation (7.2), Zone Load Imbalance (7.3)

**Problem**: When an HTM finishes at a station, where should it return the tote? The choice affects zone balance, relay availability, future retrieval efficiency, and category alignment. A poor choice can cascade into zone overload, category mismatches that degrade picking performance, or suboptimal tote positioning for future operations.

**Approach**: Back to Store uses differentiated algorithms for empty versus non-empty totes, recognizing their distinct operational purposes. Both paths implement atomic multi-resource reservation—securing a relay position and rack slot before committing—but apply different selection criteria.

#### For Non-Empty Totes

Non-empty totes contain inventory that will be picked again. The algorithm optimizes for future retrieval efficiency by matching tote popularity to location popularity and minimizing distance from Pick PPS.

**Selection hierarchy** (applied as successive filters with bucketing):

1. **Category matching**: Filter zones with available storage locations matching the tote's popularity category. Search in preference order:
   - Category A totes: A → B → C locations
   - Category B totes: B → A → C locations
   - Category C totes: C → B → A locations

2. **VTM load balancing**: Track active VTM tasks per zone. Apply bucketing factor to group zones with similar load. Prefer zones with zero active tasks when number of active zones is less than available VTMs.

3. **Availability ratio**: Calculate category-specific availability within each zone:
   ```
   Availability Ratio = Empty Locations / Occupied Locations (for tote's category)
   ```
   Only consider unblocked locations. Use discrete buckets to group zones with similar availability. Higher ratios (more available capacity) are preferred.

4. **Distance from Pick PPS**: Measure zone distance from the Pick PPS that will most likely request this tote. Apply bucketing to group zones by distance bands. Prefer closer zones to minimize future retrieval travel.

#### For Empty Totes

Empty totes are staging inventory for future put operations. The algorithm optimizes for put efficiency and warehouse-wide balance rather than category matching.

**Selection hierarchy**:

1. **VTM load balancing**: Same zone load tracking as non-empty totes. Distribute return tasks evenly across available VTMs.

2. **Empty location percentage**: Calculate warehouse-wide availability:
   ```
   Empty Ratio = Total Empty Locations / Total Locations (all categories)
   ```
   Prefer zones with higher empty ratios—these have capacity to absorb new inventory.

3. **Distance from Put PPS**: Measure zone distance from Put PPS. Prefer zones closer to put operations since empty totes will likely receive new inventory soon.

#### Storage Location Selection Within Zone

After zone selection, the algorithm selects a specific rack slot:

- **Non-empty totes**: Random selection among eligible locations within the chosen category. Randomization prevents hotspots and distributes wear.

- **Empty totes**: Popularity-based sequence (default: A → B → C). Places empty totes in high-popularity locations first, staging them optimally for future picks when they receive inventory.

#### Atomic Reservation

Both paths use atomic multi-resource reservation to prevent partial allocation deadlocks:

1. **Zone selection**: Apply the appropriate hierarchy (non-empty or empty) to identify candidate zones.

2. **Eligibility filtering**: For each candidate zone, verify relay capacity (available > 0) and rack space (at least one rack has unreserved slots matching criteria).

3. **Atomic reservation**: For the first eligible zone, attempt to reserve a relay position. If successful, attempt to reserve a rack slot (specific rack and z-index). If rack reservation fails, release the relay and try the next zone. Only when both reservations succeed does the algorithm commit.

4. **Fallback**: If no preferred zones work, expand criteria (e.g., accept non-matching categories for non-empty totes) and repeat. If all zones fail, the system is at capacity—an error condition.

**Key decisions**:

- Separating empty and non-empty logic reflects their different purposes: non-empty totes optimize for retrieval, empty totes optimize for receiving.
- Category preference order (A→B→C vs B→A→C) keeps high-velocity totes in high-accessibility locations while allowing spillover to adjacent categories.
- Bucketing at each hierarchy level prevents thrashing—small differences in load or distance don't cause constant rebalancing.
- Random selection for non-empty totes within a zone avoids deterministic patterns that could create wear hotspots.

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `categoryMatchStrict` | true/false | If true, reject zones without matching category locations. If false, fall back to adjacent categories. Start with false for flexibility. |
| `availabilityBucketCount` | 3–5 | Number of buckets for availability ratio. More buckets = finer granularity. Default 3 (low/medium/high). |
| `distanceBucketCount` | 3–5 | Number of buckets for PPS distance. More buckets = finer granularity. Default 3 (near/medium/far). |
| `zoneLoadBucketCount` | 2–5 | Number of load buckets for VTM task distribution. Default 3. |
| `emptyToteLocationOrder` | A→B→C configurable | Popularity sequence for empty tote placement. Reverse (C→B→A) if empties should fill low-popularity first. |
| `rackSlotReservationEnabled` | true/false | Whether to reserve specific rack slots. Enabled prevents race conditions. Default true. |
| `fallbackToAnyCategory` | true/false | Whether to expand to non-matching categories if preferred unavailable. Default true. |

**Limitations**: Totes with existing reservations (from a previous interrupted cycle) have their old reservation cleaned up before new selection. Zones with no racks serviced are skipped entirely. Category matching requires location popularity scores to be pre-calculated (see §9.1 Tote & Location Popularity).

---

### 9.7 Path Planning

**Bottleneck addressed**: VTM Throughput (7.1), Navigation efficiency

**Problem**: Given a robot's current position and destination, what path should it take? The path must avoid obstacles, minimize travel time, and account for navigation overhead in the real grid.

**Approach**: Path Planning uses A* search with Manhattan distance heuristic. The algorithm operates on the warehouse grid, treating rack positions as obstacles and aisles as traversable space.

Travel time calculation incorporates multiple factors:

```
travelTime = ((1.0 + returnFactor) × distance / robotSpeed) × navFactor × aStarPenaltyFactor
```

- **returnFactor**: Accounts for round-trip vs. one-way travel (1.0 = symmetric round trip)
- **navFactor**: Navigation overhead from grid constraints (typically 1.2-1.5)
- **aStarPenaltyFactor**: Calibration factor aligning A* estimates to observed travel

**Access Positions**: Robots do not navigate directly to racks or relays. Each storage location defines access positions—grid cells where robots stop to interact. Racks have VTM access positions (aisle-side); relays have separate VTM and HTM access positions (reflecting their physical orientation).

**Key decisions**: Using access positions rather than direct coordinates models real robot behavior. A VTM doesn't drive into a rack—it stops adjacent and extends a mechanism. This detail matters for accurate travel time estimation.

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `robotSpeed` | 1.0–3.0 m/s | Base robot movement speed. VTMs typically slower (1.0–1.5) due to load; HTMs faster (2.0–3.0). Calibrate from hardware specs. |
| `returnFactor` | 0.5–1.2 | Return leg as fraction of outbound. <1.0 if robots chain tasks; >1.0 if returns face congestion. Calibrate from observed cycles. |
| `navFactor` | 1.2–2.0 | Multiplier for navigation overhead. Higher for complex layouts with many turns. Start at 1.5, tune from A* vs. actual comparison. |
| `aStarPenaltyFactor` | 0.8–1.5 | Final calibration factor. <1.0 if A* overestimates; >1.0 if underestimates. Tune empirically. |
| `loadUnloadTime` | 2–10 seconds | Time for tote pickup/putdown operations. Measure from hardware or use vendor specs. |

**Limitations**: Robots already at destination return empty paths with zero travel time. Unreachable destinations (blocked by obstacles) return failure—a configuration error.

---

### 9.8 Bot Parking

**Bottleneck addressed**: VTM Throughput (7.1), Navigation efficiency

**Problem**: When a robot has no work, where should it wait? Poor parking positions increase response time to new tasks and can create congestion for active robots.

**Approach**: TBD — Parking strategy not yet defined.

**Key decisions**: TBD

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `parkingStrategy` | nearest, distributed, zone-affinity | Nearest minimizes travel to park; distributed spreads robots for faster response; zone-affinity keeps VTMs near their zones. |
| `parkingPositions` | Dedicated or shared | Dedicated positions prevent congestion but require space; shared positions are flexible but may cause blocking. |
| `maxIdleBeforeParking` | 5–30 seconds | How long to wait at current position before moving to parking. Shorter values reduce congestion; longer values save movement. |
| `parkingZoneRadius` | 5–20 grid cells | How far from work zones robots should park. Closer improves response time; farther reduces aisle congestion. |

**Limitations**: TBD

---

### 9.9 Station Planner

**Bottleneck addressed**: Operator Blocking (7.5), HTM Starvation (7.4)

**Problem**: How should work be distributed across stations, and how should station queues be managed? Uneven distribution idles some operators while others are overwhelmed.

**Approach**: TBD — Station planning strategy not yet defined.

**Key decisions**: TBD

**Key configurations**:

| Parameter | Range | How to Decide |
|-----------|-------|---------------|
| `binsPerStation` | 2–8 | Queue depth at each station. More bins reduce operator idle but require more totes in flight. Size based on operator speed and HTM cycle time. |
| `operatorTimePerPick` | 3–15 seconds | Time operator spends per pick. Measure from real operations or use industry benchmarks (6–10s typical). |
| `stationBalanceWeight` | 0.3–0.7 | How strongly to balance work across stations vs. optimize individual station efficiency. Higher values enforce balance. |
| `minQueueDepth` | 1–3 totes | Minimum totes queued before station considered "starving". Lower values trigger earlier intervention. |
| `maxQueueWaitTime` | 60–300 seconds | Maximum time a tote should wait in queue. Exceeding triggers priority escalation for that station. |

**Limitations**: TBD

---

## 10. Failure Modes & Mitigations

This section catalogs known failure modes—conditions where the system degrades or fails—and the mitigations in place or needed.

### 10.1 Deadlock: All Relays Full

**Symptom**: VTMs block indefinitely waiting to drop totes; no forward progress.

**Cause**: HTM consumption rate fell below VTM production rate long enough to saturate all relay positions.

**Mitigation**: Backpressure control (80% threshold) prevents this under normal operation. VTMs idle before relays fill completely, and return journey priority clears occupied positions.

**Residual risk**: If backpressure threshold is set too high or return journeys are disabled, deadlock remains possible.

### 10.2 Starvation: Stations Idle

**Symptom**: Operators waiting with no totes; throughput below target.

**Cause**: VTM throughput insufficient, order patterns creating sparse task distribution, or zone imbalance starving specific relays.

**Mitigation**: TPQ optimization batches tasks onto fewer totes, reducing the number of VTM cycles needed. Zone load balancing spreads work across all zones.

**Residual risk**: Fundamental VTM capacity constraints cannot be mitigated algorithmically—requires adding VTMs.

### 10.3 Reservation Leak

**Symptom**: Relay positions or rack slots appear reserved but no tote is in-flight; available capacity incorrectly reported.

**Cause**: Robot process crashed or was interrupted between reservation and release.

**Mitigation**: Reservations are tracked in-memory and cleared on simulation restart. For production systems, TTL-based expiration or heartbeat-based cleanup would be needed.

**Residual risk**: Long-running simulations with process interruptions could accumulate orphaned reservations.

### 10.4 Zone Imbalance Cascade

**Symptom**: One zone at 95% utilization while others are at 40%; system-wide throughput limited by the bottlenecked zone.

**Cause**: Initial inventory placement favored certain zones; order patterns concentrated in specific areas.

**Mitigation**: Zone-aware popularity scoring during placement, zone load bucketing during order selection, and return-to-low-load-zone logic during Back to Store.

**Residual risk**: Extreme order skew (e.g., 80% of orders from one SKU family in one zone) can overwhelm balancing mechanisms.

### 10.5 Claim Race Condition

**Symptom**: Two robots attempt to process the same tote; state corruption or duplicate work.

**Cause**: Claim assignment not atomic or check-then-set race.

**Mitigation**: Desmo-J single-threaded execution model eliminates true races. Claims are set immediately after selection, before any `hold()` calls that would yield to other processes.

**Residual risk**: None in current implementation. Production systems with multi-threading would need atomic claim operations.

---

## 11. Observability

Effective observability answers three questions: Is the system healthy? Where is it struggling? What happened?

### Key Metrics

**Throughput metrics** (are we meeting goals?):
- Picks per hour per station
- Orders completed per hour
- Picks per face (PPF) — batching efficiency

**Utilization metrics** (are resources balanced?):
- Operator utilization (% time actively picking)
- VTM utilization (% time not idle)
- HTM utilization (% time not idle)
- Relay utilization per zone (% occupied + reserved)

**Latency metrics** (where are delays?):
- VTM cycle time (rack to relay)
- HTM cycle time (relay to station to relay)
- Operator wait time between totes
- Robot blocked time at resources

### Debugging Workflow

When throughput drops:
1. Check relay utilization — if high, VTM/HTM imbalance
2. Check VTM idle rate — if high, no tasks or backpressure active
3. Check HTM idle rate — if high, relay starvation
4. Check per-zone metrics — identify imbalanced zones
5. Check blocked time — identify resource contention

---

## 12. Implementation Risks

This section identifies architectural risks that may surface during production implementation. These are not algorithm bugs but structural gaps between the design model and real-world system behavior.

### 12.1 Multi-Station Tote Return Cycles

**Risk**: The current design assumes HTMs return totes to relay positions after each station visit. When a tote has picks destined for multiple stations, this creates repeated relay cycles that multiply travel overhead.

**Scenario**: A tote contains picks for Station 1 (3 picks) and Station 3 (2 picks). Current flow:

```
Relay → Station 1 → pick → Relay → Station 3 → pick → Relay → Rack
       \_________/          \_________/          \________/
         Cycle 1              Cycle 2              Return
```

Each station visit incurs a full HTM round-trip to relay. For a tote visiting N stations, this creates N relay cycles instead of a single multi-stop journey.

**Impact**:
- HTM travel overhead scales with stations-per-tote, not picks-per-tote
- TOR_htm calculation underestimates true overhead for multi-station totes
- System throughput degrades as order complexity increases

**Mitigation options**:
1. **Multi-stop HTM journeys**: Allow HTMs to visit multiple stations before returning to relay. Requires journey planning and station sequencing logic.
2. **Station clustering**: Assign orders to minimize totes spanning multiple stations. Reduces multi-station scenarios at cost of flexibility.
3. **Relay position at station clusters**: Place relay positions to serve station groups, reducing inter-station travel.

**Current status**: Base relay design uses single-station cycles. Multi-stop journeys are a future enhancement (see Variations: High-Throughput Pick-to-Box).

### 12.2 MVTS Late Dispatch

**Risk**: The algorithms assume tasks are dispatched to robots immediately when selected. In production, the Multi-Vehicle Task Scheduler (MVTS) manages actual robot dispatch and may introduce latency between algorithm decisions and physical robot movement.

**Scenario**: Order Processing selects orders and creates tasks at T=0. The algorithm assumes VTMs begin moving immediately. However, MVTS may:
- Queue tasks behind existing robot commitments
- Delay dispatch due to traffic management
- Reorder tasks based on fleet-level optimization

By the time the VTM actually moves at T=15s, the operator is already idle—but the algorithm's pool metrics showed healthy coverage at T=0.

**Impact**:
- Operator starvation despite "healthy" task pool metrics
- Pool health calculations reflect algorithm state, not physical reality
- Backpressure signals arrive too late (based on relay state, not dispatch pipeline)

**Mitigation options**:
1. **Dispatch pipeline modeling**: Include MVTS queue depth and estimated dispatch latency in pool health calculations. Trigger order selection earlier to account for dispatch delay.
2. **MVTS feedback integration**: Receive actual dispatch timestamps from MVTS and recalibrate pool metrics against physical reality.
3. **Safety margins**: Add buffer to minimum pool thresholds (e.g., if minimum is 4 tasks/station, target 6 to absorb dispatch latency).
4. **Dispatch latency monitoring**: Track time from task creation to robot movement. Alert when latency exceeds threshold.

**Current status**: Algorithms assume instantaneous dispatch. MVTS integration requires coordination with fleet management layer.

---

## Appendix A: Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `HIGH_RELAY_UTILIZATION_THRESHOLD` | 80% | Backpressure trigger |
| `IDLE_CHECK_INTERVAL` | 2.0s | Robot polling when idle |
| `VTM_LOADING_TIME` | 9.0s | Rack ↔ VTM transfer |
| `VTM_UNLOADING_TIME` | 9.0s | VTM ↔ Relay transfer |
| `HTM_LOADING_TIME` | 15.0s | Relay ↔ HTM transfer |
| `HTM_UNLOADING_TIME` | 15.0s | HTM ↔ Station/Relay transfer |
| `TPQ_WEIGHT` | 100.0 | Order scoring: TPQ deviation penalty |
| `RACK_REUSE_BONUS` | 10.0 | Order scoring: piggybacking reward |
| `RACK_OVERLOAD_THRESHOLD` | 7 | Picks before overload penalty |
| `RACK_OVERLOAD_PENALTY` | 20.0 | Per-pick penalty over threshold |
| `STATION_DEVIATION_PENALTY` | 20.0 | Station TPQ deviation penalty |
| `ZONE_BALANCE_WEIGHT` | 0.7 | Popularity: zone vs. distance weight |
| `VTM_RELAY_WEIGHT` | 0.6 | Relay selection: VTM travel weight |
| `HTM_RELAY_WEIGHT` | 0.4 | Relay selection: HTM travel weight |
