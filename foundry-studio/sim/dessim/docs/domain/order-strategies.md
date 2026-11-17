# Order Selection Strategies

How the system decides which pending orders to process next. This is the highest-leverage decision in the warehouse -- choosing orders that share storage containers dramatically reduces robot travel.

---

## The Core Problem

A warehouse has hundreds of pending orders and limited robots. Each order needs items from one or more storage containers (racks or totes). Fetching a storage container requires a robot round-trip. The key insight: if two orders need items from the **same** container, you can fulfill both with a single trip.

The order selection strategy decides:
1. **Which orders** to activate from the pending pool
2. **In what sequence**, to maximize container sharing
3. **At which station**, to balance operator workload

---

## Common Infrastructure

All three strategies share:

### Set Cover

Before scoring an order, the system determines the minimum set of storage containers needed to fulfill it. This is a greedy set cover:

1. For each unfulfilled item, find which containers have it in stock
2. Pick the container covering the most distinct SKUs
3. Containers already in motion (at relay, at station, being carried) get a **+1000 bonus** -- adding tasks to a container that is already traveling is essentially free
4. Repeat until all items are covered or the container limit is reached (default: 10)

The set cover result determines the order's **picks per face (PPF)**: total picks divided by number of containers required. Higher PPF = more efficient.

### Zone Load Bucketing (Relay System)

For relay systems, orders are pre-grouped by zone congestion before selection:

1. For each order's tasks, determine which zone each container lives in
2. Use the bottleneck zone (highest load) as the order's congestion score
3. Group orders into buckets of size 5 by congestion level
4. Process buckets from lowest to highest congestion

This prevents the strategy from overloading already-congested zones. It is a pre-filter applied before the main selection algorithm.

### Fulfillment Plan Caching

Computing set cover is expensive. The system caches fulfillment plans (which containers serve which order) and invalidates only when relevant inventory changes -- specifically, when stock for a particular SKU is consumed or reserved, only orders containing that SKU have their cached plans invalidated.

---

## Strategy 1: PPF-Directly (Maximize Picks Per Face)

**Best for:** RTP systems where minimizing rack fetches is the dominant cost.

**Algorithm:** Greedy marginal PPF maximization.

**How it works:**

1. Compute fulfillment plans for all pending orders
2. Group by zone load bucket (relay only)
3. Within each bucket, iteratively select the order with the highest **marginal PPF**:
   - Track the set of containers already selected by previously chosen orders
   - For each candidate order, count how many NEW containers it would require (excluding those already selected)
   - If the order uses ONLY already-selected containers: it has **infinite marginal PPF** (zero additional trips -- pure piggybacking)
   - Otherwise: `marginal_PPF = total_picks / new_containers`
4. Ties broken by total picks (prefer larger orders)
5. Accumulate the selected container set across rounds

**Key insight:** The first order selected establishes a "container pool." Subsequent orders are scored by how well they overlap with this pool. An order requiring no new containers is a pure win -- its items piggyback on already-committed trips.

**Example:**
- Order A needs totes {1, 2, 3} -- selected first (PPF = picks/3)
- Order B needs totes {2, 3, 4} -- only tote 4 is new, marginal PPF = picks/1
- Order C needs totes {2, 3} -- all containers already selected, infinite marginal PPF (selected immediately)

---

## Strategy 2: PPF-With-Tiering (Maximize PPF with Tiered Optimization)

**Best for:** RTP systems with high rack reuse potential.

**Algorithm:** Similar to PPF-Directly but with tiered scoring that more aggressively penalizes opening new racks. Strongly prefers reusing containers that are already active (being transported, at station, or at relay).

The tiering means: first fill up already-active containers to capacity, then consider opening new ones. This creates a stronger "container reuse" signal than the basic PPF strategy.

---

## Strategy 3: TPQ (Maximize Task Pool Quality)

**Best for:** Relay systems where balancing throughput across stations matters.

**Algorithm:** Multi-objective scoring that optimizes for a target Task Pool Quality value.

### What is TPQ?

TPQ (Task Pool Quality) measures the ratio of total cycle time to productive operator time:

```
TPQ = (total_travel_time + total_operator_time) / total_operator_time
```

- **TPQ = 1.0** means operators are never idle (physically impossible -- robots need travel time)
- **TPQ = 2.0** means half the time is travel, half is productive picking
- **TPQ = 5.0** means 80% of time is travel overhead

Lower TPQ is better. The system computes an ideal **target TPQ** from warehouse geometry and uses it as the optimization target.

### Target TPQ Calculation

Before simulation, the target TPQ is estimated from layout geometry:

```
avg_travel_time = (avg_distance_to_station / robot_speed) * navFactor * pathfinding_penalty
avg_operator_time = operator_time_per_pick * target_picks_per_face
target_TPQ = (avg_travel_time + avg_operator_time) / avg_operator_time
```

This gives the theoretical best TPQ given the warehouse layout. The strategy tries to keep the actual task pool close to this target.

### Scoring Function

For each candidate (order, station) pair, a multi-objective score is computed:

1. **TPQ targeting** (weight 100): Penalizes deviation from target TPQ
   ```
   score -= |pool_TPQ_with_this_order - target_TPQ| * 100
   ```

2. **Rack reuse bonus** (weight 10): Rewards sharing containers with already-active orders
   ```
   score += reused_containers * 10
   ```

3. **Rack overload penalty** (weight 20): Penalizes putting more than 7 picks on one container
   ```
   score -= max(0, picks_on_container - 7) * 20
   ```

4. **Station deviation penalty** (weight 20): Penalizes sending too much or too little work to one station relative to the target TPQ
   ```
   if station_TPQ_deviation > tolerance: score -= (deviation - tolerance) * 20
   ```

### Selection Process

1. Prepare candidates and group by zone load bucket
2. For each bucket (lowest load first):
   - Enumerate all (order, station) pairs
   - Score each pair
   - Select the highest-scoring pair
   - Update the task pool state, station load, and per-container pick counts
   - Repeat until the batch is full or no valid pairs remain

### Travel Time for Scoring

```
travel_time = ((1.0 + return_factor) * manhattan_distance / robot_speed) * navFactor * a_star_penalty
```

The `return_factor` (default: 0.8) accounts for the return trip being slightly cheaper (robot is empty, shorter path possible).

Only the **first task** on a new container incurs travel cost. Subsequent tasks on the same container piggyback for free (same principle as PPF).

---

## Strategy Comparison

| Aspect | PPF-Directly | PPF-With-Tiering | TPQ |
|--------|-------------|-------------------|-----|
| Primary metric | Picks per face | Picks per face (tiered) | Task pool quality |
| Container sharing | Strong | Very strong | Strong |
| Station balancing | None | None | Explicit (scoring) |
| Relay awareness | Via zone bucketing | Via zone bucketing | Via zone bucketing + TPQ |
| Best system type | RTP | RTP | Relay |
| Computational cost | Low | Low | Moderate (per-station scoring) |

---

## Put Algorithms (Inventory Placement)

Before simulation begins, items must be placed into storage containers. The **put algorithm** determines where each item goes:

**Greedy:** Fast placement (O(n*m)). Places items based on proximity to stations and popularity. Popular items go closer to stations.

**Min Weighted Cycle Distance:** Linear programming optimization using OR-Tools. Minimizes expected cycle distance for item retrieval. Much slower but produces better item placement that reduces average travel time during picking.

**Popularity scoring:** Items with higher historical order volume ("popular" items) are placed in storage containers closer to stations, reducing average travel distance. The distribution follows a Pareto curve -- a small percentage of SKUs account for a large percentage of picks.
