# Orchestration Model

How VTMs, HTMs, and the Dispatcher coordinate tote movement through the relay system. This document specifies the scheduling, scoring, timing, and flow control algorithms that govern robot behavior.

For physical workflow and robot capabilities, see `picking-workflow.md` and `robot-types.md`.

---

## System Architecture

The relay system is a **decoupled producer-consumer pipeline**:

```
OrderManager          Dispatcher                 VTMs              HTMs            OperatorManager
(demand signal)       (state + scoring +         (rack ↔ relay)    (relay ↔ stn)   (station queue)
                       assignment)
```

The **Dispatcher** is the central authority. It owns all tote state, relay reservations, tote claims, and robot-to-work assignment logic. VTMs and HTMs are stateless executors — they receive push assignments from the Dispatcher and report state transitions back. All state mutation is sequential (single-actor processing model), so claims and reservations are inherently atomic.

**Communication model:** Push-based assignment with deferred matching. When a robot registers idle, the Dispatcher batches all idle registrations within the same simulation tick (+1ns deferred scheduling), then runs a single matching pass that atomically scores candidates, claims totes, reserves relays, and pushes assignments. This eliminates the thundering-herd problem of broadcast-wake-and-race designs [Wurman et al., 2008].

**Message efficiency:** ~5 Dispatcher interactions per VTM cycle, ~10 per HTM cycle. At 10 VTMs + 30 HTMs, total Dispatcher load is ~2.7 messages/second — well within single-actor capacity. The matching pass uses rayon parallel scoring: candidates (totes) are scored in parallel, with each parallel task finding the best robot for that tote via manhattan prescreen → A* on top-5. Parallelism is across candidate totes, not across robots.

---

## The Relay as Bidirectional Bounded Buffer

The relay is a **bidirectional shared buffer**, not a simple one-directional queue. Both VTMs and HTMs deposit and withdraw totes:

```
VTM forward deposit ──►  [Relay: K slots]  ◄── HTM return deposit
HTM forward pickup  ◄──  (shared pool)     ──► VTM return pickup
```

- **Forward flow:** VTMs deliver totes from rack → relay. HTMs pick up totes from relay → station.
- **Return flow:** HTMs deposit processed totes at relay. VTMs pick up processed totes from relay → rack.
- **Buffer capacity:** K relay slots per zone (typically 5-15, recommended K ≥ 8).
- **Dual-mandate VTMs:** VTMs both fill the buffer (forward) and drain it (return). This creates a built-in negative feedback loop — as relays fill, VTMs find more return work and fewer available forward slots, organically throttling production.

### Deadlock condition

Deadlock occurs when all K slots contain forward totes (awaiting HTM pickup) and HTMs hold processed totes needing free slots for deposit. Neither side can proceed — circular wait. This is prevented by the deadlock headroom override (§ Flow Control).

---

## VTM Decision Model: Greedy Utility-Based Scoring

VTMs use a unified scoring function to select work. When a VTM becomes idle, the Dispatcher scores ALL available work items — forwards AND returns, across ALL zones — and assigns the highest-scoring item. There is no static zone assignment; VTMs self-balance by proximity scoring.

This approach is grounded in the multi-robot task allocation (MRTA) literature. The problem class is ST-SR-IA (single-task, single-robot, instantaneous assignment) per the Gerkey-Matarić taxonomy [Gerkey & Matarić, 2004]. Greedy utility-based scoring is the standard industry approach for this class, used by Amazon Robotics [Wurman et al., 2008; Enright & Wurman, 2011] and similar systems. The Hungarian algorithm [Kuhn, 1955] yields theoretically optimal assignments but is O(n³) and awkward for event-driven simulation. Auction/consensus approaches [Bertsekas, 1988] are overkill for a centralized system with 10 VTMs.

### The scoring function

```
score(vtm, work_item) = proximity × type_weight(pressure)
```

**Term 1 — Proximity** (prefer nearby work):
```
proximity = 1.0 - (distance_to_work / MAX_TRAVEL).min(1.0)
```
Where `MAX_TRAVEL` ≈ 150,000mm (calibrated to production-scale warehouse diagonals, ~75×75 grid at 1000mm cells; significantly larger than scenario_a's 20×20 test grid). Sensitivity is low — anything within 2× works. A tote 10 cells away scores ~0.93; 100 cells away scores ~0.33. Cross-aisle travel costs ~16 cells of overhead per aisle boundary, creating natural "soft zones" without explicit assignment.

**Term 2 — Type weight** (dynamic forward/return balance based on relay pressure):
```
Forward: 1.0 - p²
Return:  0.3 + 2.0 × p²
```
Where `p = (occupied + β × reserved) / capacity` for the work item's zone, with β = 0.7 to discount reserved-but-empty slots (see § Ghost Reservation Pressure).

Crossover at ~48% relay pressure — below that, forward dominates; above, returns dominate. This matches the queueing equilibrium where VTMs naturally split 50/50 forward/return at steady state.

| Pressure | Forward | Return | Ratio |
|----------|---------|--------|-------|
| 0% | 1.00 | 0.30 | Forward 3.3× |
| 30% | 0.91 | 0.48 | Forward 1.9× |
| 48% | 0.77 | 0.76 | Crossover |
| 70% | 0.51 | 1.28 | Return 2.5× |
| 90% | 0.19 | 1.92 | Return 10× |

### Validation of the quadratic curve

The p² curve shape was validated via Markov chain birth-death analysis [Kleinrock, 1975] against four alternative curve families. For K=10 relay slots:

| Curve family | E[occupancy] | P(deadlock) | Throughput | Verdict |
|--------------|:--:|:--:|:--:|---------|
| Linear (1-p) | 3.8 | 1.8% | 4.47/min | Too conservative mid-range |
| **Quadratic (1-p²)** | **4.7** | **0.77%** | **4.52/min** | **Pareto-optimal** |
| Cubic (1-p³) | 5.8 | 3.2% | 4.54/min | 4× deadlock risk for +0.4% |
| Sigmoid | 5.0 | 0.3% | 4.50/min | Oscillation risk at crossover |
| Step (legacy) | 7.5 | 12% | 4.55/min | Catastrophic deadlock probability |

The quadratic is Pareto-optimal: no candidate achieves strictly better deadlock safety AND throughput simultaneously. It reaches 99.3% of theoretical maximum throughput (4.52 vs 4.55 totes/min) with only 0.77% deadlock probability. The restoring force `F(p) = 3p² - 0.7` is convex, meaning gentle control at low pressure (no over-correction) and aggressive control near deadlock — the correct shape for asymmetric risk [Bertsekas, 2012].

Pontryagin's minimum principle shows the theoretically optimal policy is bang-bang (100% forward or 100% return), but this is unstable with N=10 independent agents. The smooth quadratic is the correct stochastic relaxation — its transition width (~0.3 in p units) matches the noise bandwidth of √N ≈ 3.2 independent VTMs.

### Why not anti-clustering (spread bonus)?

Scenario analysis across 5 workloads showed that an explicit spread_bonus term is actively harmful — it penalizes VTMs for converging on hotspots, reducing throughput by 22-39% in realistic scenarios. Three natural mechanisms provide sufficient anti-clustering:

1. **Cross-aisle travel cost** (~16 cells/boundary) creates strong proximity bias via the scoring function
2. **Serialized Dispatcher** processes claims sequentially — second VTM targeting the same tote simply gets the next-nearest
3. **Position-sensitive matching** — `find_closest_vtm` returns nearest unclaimed tote per robot position, so co-located VTMs naturally select different work

### Deadlock safety override

```
if free_relay_slots <= deadlock_headroom: only return work considered
```

Where `free_relay_slots = capacity - occupied_count - reserved_count` uses **raw counts with no β discount**. The reservation discount (β=0.7) applies ONLY to the soft scoring pressure `p` in the type_weight function, NEVER to this hard safety gate. Mixing these up would allow forward work when only 1 physical slot remains, creating deadlock risk.

Hard constraint, non-negotiable. Default `deadlock_headroom = 2`. For small relays (K < 8), use `max(2, ceil(K × 0.15))` to avoid sawtooth throughput oscillation.

When the override triggers but no return work exists (e.g., cold start — see § Transient Behavior), VTMs fall through to idle. This is safe: HTMs continue picking up forward totes, freeing relay slots within one HTM travel leg (~25-40s).

### Starvation prevention

With zone-free scoring, distant zones may accumulate unclaimed totes under asymmetric workloads. An age-based bonus prevents indefinite starvation:

```
age_bonus = min(starvation_max_bonus, unclaimed_seconds / starvation_decay_secs)
```

Default `starvation_max_bonus = 0.5`, `starvation_decay_secs = 60.0`. Totes unclaimed for >30s get progressively increasing score bonuses, ensuring eventual service without the rigidity of static zone assignment. The cap is configurable because extreme geometric asymmetry (farthest zone proximity ~0.2 vs nearby ~0.93) may require a higher cap to overcome proximity bias.

The `unclaimed_seconds` timer resets when a tote is claimed. If a tote is claimed, processed, and re-unclaimed after rebatch, the timer restarts from zero — the starvation prevention targets totes that no VTM naturally gravitates toward, not recently-serviced totes.

### Tunable parameters

| Parameter | Default | Sensitivity | Rationale |
|-----------|---------|-------------|-----------|
| `max_travel_dist` | 150,000mm | Low | ~warehouse diagonal; anything within 2× works |
| `return_base` | 0.3 | Low | Floor return weight at p=0; enables opportunistic nearby returns |
| `return_urgency` | 2.0 | Medium | 2:1 asymmetry provides built-in hysteresis for pressure spikes |
| `deadlock_headroom` | 2 | Low | 2-3 both safe for K ≥ 8 |
| `reservation_discount` (β) | 0.7 | Medium | Discount for reserved-but-empty slots in pressure calc |
| `starvation_max_bonus` | 0.5 | Low | Cap on age bonus; increase for large warehouses with extreme proximity ratios |

### Work discovery

VTMs discover work through a **push model**:

1. **Registration:** When idle, VTM sends `register_idle_vtm { robot_id, position }` to Dispatcher.
2. **Deferred matching:** Dispatcher batches idle registrations within the same tick (+1ns), then runs `run_deferred_matching` once for all pending robots.
3. **Push assignment:** Dispatcher scores all candidates, atomically claims the best tote, reserves the relay, and pushes `VtmPushAssignment` to the matched VTM.
4. **Fallback poll:** 2s idle interval as safety net if push delivery fails.

A dirty-flag mechanism ensures matching runs at most once per tick, even when multiple events (idle registration, tote arrival, rebatch) fire simultaneously. The Dispatcher maintains separate dirty flags for HTM matching and VTM matching. Under legacy mode (scoring disabled), VTM forward and return have separate flags with return processed first (implicit priority). Under scoring mode, a single unified VTM dirty flag triggers the combined scoring pass.

### Relay selection (forward cycle)

When assigning forward work, the Dispatcher selects the relay that minimizes combined VTM + downstream HTM travel:

```
score = 0.6 × distance(vtm_pos, relay) + 0.4 × distance(relay, target_station)
```

The selected relay is **atomically reserved** during matching — no separate reservation step. The reservation is held for the full forward cycle (~66s). The relay's owning zone (not the tote's source zone) must receive the `reserved_count` increment. With zone-free scoring, the selected relay may be in a different zone than the tote's rack — correct zone accounting is critical for accurate pressure calculations.

---

## HTM Decision Model

HTMs operate globally (not zone-bound) and handle one tote per cycle.

### Work selection

The Dispatcher matches idle HTMs to totes at relays using proximity-based scoring:

1. Scan all totes with `has_tasks() && claim.is_none() && location == AtRelay(r)`.
2. For each candidate tote, find the closest idle HTM using manhattan prescreen → A* on top-5.
3. Atomically claim the tote and push `HtmPushAssignment`.

### Per-tote dispatch

HTMs dispatch per-tote, not per-order. A 3-tote order results in 3 independent HTM cycles, almost always by 3 different HTMs. There is no batching and no concept of order-level grouping at the HTM layer.

### Rebatch short-circuit

After processing a tote at a station, the HTM checks whether rebatch assigned new tasks to the same tote (from newly-dispatched orders that share this tote). If new tasks exist, the HTM re-processes immediately without a relay round-trip.

### Return relay selection

When returning a tote, the HTM selects a relay in the least-loaded zone (bucketed load balancing). Zone load for return relay selection uses the same β-discounted pressure formula as VTM scoring: `p = (occupied + β × reserved) / capacity`. This ensures consistent pressure views across VTM and HTM decision paths. The relay slot and a destination rack slot are reserved atomically. This distributes returned totes across zones to prevent hotspots.

### HTM cycle timing

```
assignment ──► nav_to_relay ──► load (2s) ──► nav_to_station ──► wait_for_operator
               A* × nav_factor                A* × nav_factor     (FIFO queue)

──► processing (9s/pick × num_picks) ──► nav_to_return_relay ──► unload (2s)
                                         A* × nav_factor
```

Current config defaults: HTM load/unload 2s (was 15s), operator pick time 9s
(was 12s). Typical HTM full cycle at current timing: ~35-50s depending on
travel distances. The structural analysis below uses legacy 170s timing and
will be recalibrated once two-phase dispatch is validated at scale.

---

## VTM-HTM Coordination

### Scheduling model: Eager VTM Dispatch, JIT HTM Arrival

The relay system uses **decoupled scheduling** via a work queue (UOW
accumulator). VTMs and HTMs are scheduled independently, with the relay
acting as a bounded buffer that absorbs timing mismatch.

**VTM dispatch is eager.** VTMs work through the queue in ASAP order —
retrieve tote from rack, deposit at relay immediately, chain to the next
Forward with zero idle gaps. VtmReturn items are deferred until the VTM
is free at or after the tote's relay deposit time — the VTM does Forwards
in the gap rather than waiting. This matches the Java reference
implementation (`DesmojVtmProcess.java`).

**HTM arrival is JIT.** HTMs are backward-scheduled to arrive at stations
at the target time using `queue_factor` scoring. The evaluate funnel picks
the HTM that arrives closest to the station's next free slot, with
asymmetric cost (3x late penalty).

```
EAGER VTM:  VTM: ===fwd=== ===fwd=== ===fwd=== | returns when available
                      totes staged at relays...
            HTM:     (matched JIT)  ====travel====|pickup → station
```

The VTM chains Forwards back-to-back. HTMs are matched to staged totes
independently. The relay absorbs timing mismatch.

### Work queue (UOW accumulator)

Station demand generates work items that flow through a linear chain:

```
VtmForward → HtmDeliver → VtmReturn → (done)
```

Each item has an `after_ms` field — the earliest simtime the work becomes
actionable. Items are drained when a bot is available at or after that
time. Undrained items stay in the queue for the next pass.

Commits push successor items: VtmForward → HtmDeliver, HtmDeliver (Store
leg) → VtmReturn. A `HashSet<ToteId>` of in-flight totes prevents
duplicate VtmForward generation across passes.

### Relay reservations (Holds model)

Both deposit directions use a two-phase **SlotHold** lifecycle:

- **Pick relay**: VTM deposits → SlotHold opens as
  `ReservationEnd::PendingPickup { consumer_task }`. HTM picks up →
  SlotHold finalized to `ReservationEnd::Exact(htm_pickup)`.
- **Store relay**: HTM deposits → SlotHold opens. VTM picks up →
  SlotHold finalized.

In addition, the HTM maintains a **CellHold** on the relay HTM access
cell between tasks. This is a `ReservationBound::Unfinalized` oracle
reservation that narrows to `Finite(departure_ms)` when the next task
commits. See `specs/putaway/planner-state-holds-spec.md` for full model.

SlotHold (tote presence) and CellHold (bot presence) are on different
physical cells and track different resources. VTM uses a separate access
cell and is not affected by CellHold.

### FullPreplan solver loop (Phase 6A)

The FullPreplan solver computes all tasks at T=0 using a causal
readiness index for candidate discovery and fair dispatch pools:

```
for each outer pass:
    generate_demand()               // stations → VtmForward/HtmDeliver nodes
    watchdog (every 100 passes):
        assess_dispatchability()    // full-graph consistency check
    inner loop:
        query readiness index       // horizon-eligible candidates
        partition VTM + HTM pools   // rotating start per pass
        scan-forward per pool:
            try_dispatch_task()     // Committed or Declined
        endgame (5 consecutive all-decline):
            force-commit or PlannerStall
hard-stop: pending > 0 → PlannerStall (not silent exit)
```

Passes are cheap. Tasks can decline and revisit. One-task-per-pass
prevents resource coupling. See `specs/putaway/task-graph-spec.md` and
`specs/putaway/planner-state-holds-spec.md`.

### Future: generalized UOW decoupling

The work queue pattern generalizes beyond relays. Any physical handoff
point (station queue cells, charging stations, staging areas) where one
bot deposits and another picks up can use the same UOW chain with
`[deposit, MAX)` → `[deposit, pickup)` reservation semantics. HTM legs
(RelayApproach, StationApproach) could also become UOW items for
finer-grained mid-task re-routing. Work items could carry precomputed
candidate lists to amortize CO-WHCA* pathfinding across passes.

### ETA-based pre-dispatch (future, rolling/JIT modes)

For rolling-horizon and JIT simulation modes (not FullPreplan), the
Dispatcher can pre-dispatch HTMs toward relays based on VTM ETA to
overlap HTM travel with VTM delivery:

```
PRE-DISPATCH: VTM: ===travel=== load ===travel=== unload |
                                        HTM: ====travel====|load
                                        ↑ dispatched early   ↑ arrives ~same time
```

**ETA computation:** A* path length is deterministic in ghost mode (no runtime congestion variance). The VTM's relay ETA = `now + rack_travel + 9s + relay_travel + 9s`. The HTM dispatch time = `vtm_eta - htm_travel_time - 5s_buffer`.

**Soft assignments:** Pre-dispatch assignments are soft (reassignable) until the HTM physically loads the tote. If a closer HTM becomes available, the original assignment is cancelled via a guard check at `arrive_at_relay`.

**Return-side pre-dispatch:** The same pattern applies in reverse — when an HTM starts returning a tote, the Dispatcher can pre-position a VTM near the return relay.

### Expected latency improvement (pre-dispatch, future)

| Metric | Without pre-dispatch | With pre-dispatch |
|--------|:--:|:--:|
| Tote idle at relay | 10-35s | 0-5s |
| End-to-end forward latency | 73-98s | 58-73s |
| Total tote round-trip | ~266s | ~211s |
| Dead time (% of cycle) | 17% | 4% |

Pre-dispatch improves **latency** (25s/tote), not steady-state **throughput** — the system is VTM-bottlenecked at 4.55 totes/min regardless. The throughput benefit comes during burst recovery: faster HTM pickup frees relay slots sooner, reducing the window where VTMs are throttled by backpressure.

---

## Flow Control

### Ghost reservation pressure

Early relay reservation holds a slot for ~66s before the tote physically arrives. With 5 VTMs doing forward work simultaneously on a K=10 relay, 50% of capacity is reserved-but-empty. Counting these at full weight inflates pressure past the crossover point, systematically biasing toward returns even when the relay is physically nearly empty.

**Fix:** Discount reserved slots in the pressure calculation:

```
p = (occupied + 0.7 × reserved) / capacity
```

The discount factor β=0.7 reflects the high (~95%) probability that a reservation converts to a physical tote, while preventing phantom reservations from triggering premature throttling. This change must be applied consistently at all pressure calculation sites: VTM forward backpressure check, backpressure query handler, HTM return relay selection, and any future pressure consumers. It recovers an estimated 10-15% throughput loss from false return bias.

### Relay reservation (hard gate)

The Dispatcher atomically reserves a relay slot before the VTM begins traveling to the rack. The reservation is held for the full forward cycle (~66s). If no slot is available, the candidate is skipped.

Early reservation (before rack travel) serves two purposes:
1. **Guarantees the VTM has a destination** — no "tote in limbo" scenario
2. **Provides advance warning** — reserved slots count (at β discount) in pressure, enabling earlier flow control response

### VTM dual-mandate feedback loop

Because VTMs both produce (forward) and consume (return), the system has a built-in negative feedback loop:

1. Relays fill up → scoring function shifts toward returns → VTMs drain the relay
2. Relays empty → scoring function shifts toward forwards → VTMs fill the relay
3. At equilibrium: 50/50 forward/return split (mathematical consequence of equal cycle times)

This is a self-regulating proportional controller. The quadratic pressure curve provides the damping: gentle near equilibrium (avoids oscillation), aggressive near deadlock (prevents overflow). The 2:1 asymmetry between `return_urgency` (2.0) and forward decay (1.0) provides built-in hysteresis, making return response faster than forward recovery — the correct bias for asymmetric deadlock risk.

**System throughput:** `10 VTMs × 0.5 / 66s = 4.55 totes/min`. HTMs have ~2.3× excess capacity (30 HTMs / ~170s cycle = ~10.6 totes/min capacity vs 4.55 demand), making VTMs the structural bottleneck.

### Zone contention damping

When multiple VTMs converge on the same hot zone, a small contention penalty prevents over-concentration:

```
adjusted_forward = forward_weight / (1.0 + 0.15 × vtms_in_flight_to_zone)
```

This does not affect return scoring (returns should converge on full relays) and preserves proximity-driven convergence during genuine hotspots while damping the initial clustering overshoot by ~15% per competing VTM.

---

## Transient Behavior

### Cold start and the "return desert"

At system start (or after an idle period), all relays are empty and all robots are idle. When orders arrive, all VTMs start forward cycles simultaneously. The first returns don't appear until an HTM completes a full cycle — approximately 170s later (66s VTM forward + 15s HTM load + 25s HTM travel + 24s processing + 25s return travel + 15s unload).

During this ~170s window, there is **no return work for VTMs**. The scoring function correctly favors forwards (p ≈ 0), but as relays fill, the deadlock headroom override may trigger while no returns exist to process. VTMs idle until HTMs begin draining.

**Mitigation:** For K ≥ 10 this is non-dangerous — 10 VTMs produce ~15-20 totes across ~10 zones, keeping per-zone pressure at 0.15-0.2. For K < 8, a cold-start grace period relaxes the backpressure threshold to 95% until the first return tote is deposited, after which normal thresholds apply.

### Asymmetric burst handling

When work concentrates in a few zones, the per-zone pressure calculation correctly isolates hot zones from cold ones. VTMs in hot zones see high forward penalties and shift toward returns (or cold-zone forwards). VTMs in cold zones see low pressure and continue forward work unimpeded. This is a key advantage over the legacy global-veto backpressure.

---

## Structural Analysis

### Bottleneck identification

Mean Value Analysis [Reiser & Lavenberg, 1980] of the system modeled as a Gordon-Newell closed queueing network [Gordon & Newell, 1967] identifies VTMs as the binding constraint by 3.3×:

| Resource | Servers | Demand per tote | Demand/server |
|----------|:--:|:--:|:--:|
| **VTM fleet** | **10** | **132s** | **13.2s** |
| HTM fleet | 30 | ~170s | ~5.7s |
| Station operators | 6 | 24s | 4.0s |

The throughput ceiling is `10 / 132 = 4.55 totes/min`. This is a structural constraint — no scheduling algorithm can exceed it without adding VTMs or reducing VTM cycle time. The bottleneck shifts to stations at ~33 VTMs.

### Sensitivity ranking

| Rank | Lever | Throughput gain | Notes |
|------|-------|:-:|-------|
| 1 | Add 1 VTM | +10% per VTM | Linear scaling to ~33 VTMs |
| 2 | Reduce nav_factor (1.8 → 1.5) | +11% | Shrinks VTM travel legs by 17% |
| 3 | Reduce VTM load/unload (9s → 6s) | +10% | Saves 12s per 132s round-trip |
| 4 | Proximity-based scoring (this design) | +5-10% | Shorter average travel distance |
| 5 | Pre-dispatch HTM | +2-5% | Indirect: lower relay dwell |
| 6 | More relay slots | <1% | Already unconstrained |
| 7 | Faster operator / more HTMs | ~0% | 2-3× over-provisioned |

### Little's Law audit

At λ = 4.55 totes/min, Little's Law `L = λW` [Little, 1961] is satisfied at every queue:

- VTMs: L = 10 totes in service (100% utilization) ✓
- Relay forward dwell: L ≈ 3 totes → W ≈ 40s (the serial handoff cost) ✓
- HTMs: L = λW = 0.0758 × 170 ≈ 13 totes in service (43% utilization, confirms ~2.3× excess) ✓
- Stations: L ≈ 1.8 totes (30% utilization, confirms over-provisioning) ✓

### End-to-end tote lifecycle

```
                                    TYPICAL   % OF TOTAL
VTM travel to rack                    20s       7.5%     ─┐
VTM load from rack                     9s       3.4%      │ VTM forward: 58s
VTM travel rack → relay              20s       7.5%      │
VTM unload at relay                    9s       3.4%     ─┘
Tote idle at relay (handoff gap)      10s       3.8%     ← eliminable (pre-dispatch)
HTM travel to relay                   25s       9.4%     ← overlappable (pre-dispatch)
HTM load at relay                     15s       5.6%
HTM travel relay → station           25s       9.4%
Operator queue wait                    6s       2.3%     ← eliminable (station LB)
Operator picking                      24s       9.0%     ← THE VALUE-ADD
HTM travel station → return relay    25s       9.4%
HTM unload at return relay            15s       5.6%
Return relay wait                      5s       1.9%     ← eliminable (return pre-dispatch)
VTM travel to return relay            20s       7.5%     ─┐
VTM load from relay                    9s       3.4%      │ VTM return: 58s
VTM travel relay → rack              20s       7.5%      │
VTM unload to rack                     9s       3.4%     ─┘
Dispatch overhead (matching, events)  ~16s       6.0%     ← amortized across both VTM cycles
────────────────────────────────────────────────────────
TOTAL                                ~282s      100%
  Productive (picks + load/unload)    90s       32%
  Travel (geometry-bound)            155s       55%
  Dead time (eliminable)              21s        7%  (with pre-dispatch)
  Dispatch overhead                   16s        6%
```

---

## Navigation and Travel Time

### A* pathfinding

All robot travel uses A* on an orientation-aware warehouse graph. Each state is `(CellId, Orientation)`. Costs:

- **Cell traversal:** `edge_distance_mm / speed_mm_per_sec` (typically 1000mm / 1500mm/s = 0.667s per cell)
- **90-degree turn:** 1.0s

### nav_factor

A post-multiplier applied to all A* travel times:

```
actual_travel_time = pathfinder_time × nav_factor
```

Default: **1.8**. This approximates real-world congestion in ghost mode (robots don't physically block each other). Effective speed: `1.5 / 1.8 = 0.83 m/s`. Applied uniformly to all robot types via a shared Pathfinder instance.

---

## Order Pipeline Integration

### Task assignment

OrderManager generates `PickTask`s via greedy set cover [Chvátal, 1979] and assigns them to totes. The Dispatcher learns about tasks through:

- **Bootstrap:** `set_tote_task_info` + `assign_tasks_to_tote` at sim start
- **Rebatch:** `handle_rebatch` during sim, triggered by `OrderManager::complete_tasks_batch`

### Set cover and spatial awareness

Set cover selects the minimum set of totes covering all items in an order. To prevent spatial concentration of tasks (which defeats VTM self-balancing), set cover should incorporate a zone-pressure signal — a small bonus for totes in low-pressure zones, equivalent to covering ~2-3 additional SKUs. Without this, the scoring function optimizes *which VTM* fetches a tote, but cannot change *which totes* have tasks. Upstream spatial blindness limits downstream orchestration effectiveness.

### Active space piggybacking

Set cover gives a bonus to totes already at relays or in transit (piggybacking). This encourages batching multiple orders' picks onto totes that are already moving, reducing VTM rack-to-relay trips. The bonus should be capped at +3 (equivalent to covering 3 extra SKUs) to prevent pathological concentration where the same few totes accumulate all tasks.

### Dispatch and rebatch loop

**Two-phase dispatch** (see `specs/simulator/station-order-assignment.md` v4):

- **Phase 1 (Assignment):** OrderManager assigns orders to stations using
  proximity scoring (sum of min_manhattan). Reserves inventory. Computes
  dispatch schedule per pick (LFF ordering, target_arrival, dispatch_time).
  Does NOT commit an HTM.
- **Phase 2 (Dispatch):** At each scheduled dispatch_time, find nearest idle
  HTM and send it. If no HTM available, mark overdue — T4 trigger retries.

**Rebatch loop** (when picks complete at a station):
1. HTM calls `process_tasks` → Dispatcher clears tote's tasks
2. HTM calls `complete_tasks_batch` → OrderManager consumes stock, completes orders
3. Completed orders reduce station's `assigned_count` → triggers Phase 1 assignment
4. New orders assigned → dispatch schedule computed → tasks generated
5. New tasks → `rebatch_out` → Dispatcher's `handle_rebatch`
6. `handle_rebatch` updates tote state, triggers deferred matching

This loop is the primary throughput driver: each completed pick potentially
unlocks new orders, which generate new tasks, which keep robots busy.

---

## Configuration

```rust
pub struct VtmScoringConfig {
    pub enabled: bool,                    // default: false (A/B comparison)
    pub max_travel_dist: f64,             // default: 150_000.0 (mm)
    pub return_base: f64,                 // default: 0.3
    pub return_urgency: f64,              // default: 2.0
    pub deadlock_headroom: u32,           // default: 2
    pub reservation_discount: f64,        // default: 0.7 (β) — soft scoring only, NOT deadlock gate
    pub starvation_decay_secs: f64,       // default: 60.0
    pub starvation_max_bonus: f64,        // default: 0.5
    pub zone_contention_penalty: f64,     // default: 0.15
}

pub struct PreDispatchConfig {
    pub enabled: bool,                    // default: false
    pub arrival_buffer_secs: f64,         // default: 5.0
    pub max_late_arrival_secs: f64,       // default: 30.0
    pub allow_reassignment: bool,         // default: true
    pub return_pre_dispatch: bool,        // default: false (Phase 2)
    pub station_proximity_weight: f64,    // default: 0.2
}
```

---

## References

- Bertsekas, D.P. (1988). "The Auction Algorithm: A Distributed Relaxation Method for the Assignment Problem." *Annals of Operations Research*, 14(1), 105-123.
- Bertsekas, D.P. (2012). *Dynamic Programming and Optimal Control*, Vol. II, 4th ed. Athena Scientific.
- Chvátal, V. (1979). "A Greedy Heuristic for the Set-Covering Problem." *Mathematics of Operations Research*, 4(3), 233-235.
- Enright, J.J. & Wurman, P.R. (2011). "Optimization and Coordinated Autonomy in Mobile Fulfillment Systems." *AAAI Workshop on Automated Action Planning for Autonomous Mobile Robots*.
- Gerkey, B.P. & Matarić, M.J. (2004). "A Formal Analysis and Taxonomy of Task Allocation in Multi-Robot Systems." *International Journal of Robotics Research*, 23(9), 939-954.
- Gordon, W.J. & Newell, G.F. (1967). "Closed Queuing Systems with Exponential Servers." *Operations Research*, 15(2), 254-265.
- Kleinrock, L. (1975). *Queueing Systems, Volume 1: Theory*. Wiley-Interscience.
- Kuhn, H.W. (1955). "The Hungarian Method for the Assignment Problem." *Naval Research Logistics Quarterly*, 2(1-2), 83-97.
- Little, J.D.C. (1961). "A Proof for the Queuing Formula: L = λW." *Operations Research*, 9(3), 383-387.
- Reiser, M. & Lavenberg, S.S. (1980). "Mean-Value Analysis of Closed Multichain Queuing Networks." *Journal of the ACM*, 27(2), 313-322.
- Wurman, P.R., D'Andrea, R. & Mountz, M. (2008). "Coordinating Hundreds of Cooperative, Autonomous Vehicles in Warehouses." *AI Magazine*, 29(1), 9-20.
