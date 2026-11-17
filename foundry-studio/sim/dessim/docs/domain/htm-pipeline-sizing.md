# HTM Pipeline Sizing

How to determine the number of HTMs required per station and across the fleet
to keep stations continuously fed.

## Core Formula

Each station requires enough HTMs in-flight to deliver a new tote every P
seconds (the operator dwell time per pick). The steady-state count is:

```
HTMs_per_station = ceil(F_cycle / P)
```

Where:
- **F_cycle** is the HTM round-trip time in seconds (relay -> station -> return
  relay), excluding dwell. Measured per relay-station pair using A* pathfinding
  on the compiled map (`desbench fpi`).
- **P** is the operator dwell time per pick in seconds. This is a scenario
  parameter (`operator.time_per_pick_secs` in SimConfig, overridable via
  `--settings pick_time=X` in desbench). Default: 9.0s.

The logic: if an HTM takes F_cycle seconds to complete a round trip, and the
station needs a new tote every P seconds, the pipeline must contain
F_cycle / P HTMs so that one is always arriving as the current pick completes.

This is a direct application of Little's Law (L = lambda * W) for steady-state
queuing: L = HTMs in-flight, lambda = 1/P (tote consumption rate), W = F_cycle
(time in system). **This is the theoretical minimum for zero-variance
operation.** With real variance in F_cycle, additional margin is needed; see
the Variance section below.

### Worked Example (cencora_phase0)

Measured via `desbench fpi`:
- F_cycle_mean = 54.5s (mean round-trip, per relay to its nearest station)
- P = 9.0s (default dwell time)

```
HTMs_per_station = ceil(54.5 / 9.0) = ceil(6.06) = 7
```

With 10 picking stations: 7 x 10 = **70 HTMs total** at steady state.

### Sensitivity to Inputs

The result depends entirely on both F_cycle and P:

| F_cycle (s) | P (s) | HTMs/station | 10-station total |
|-------------|-------|--------------|-----------------|
| 54.5        | 1.0   | 55           | 550             |
| 54.5        | 9.0   | 7            | 70              |
| 54.5        | 30.0  | 2            | 16              |
| 120.0       | 9.0   | 14           | 112             |
| 30.0        | 9.0   | 4            | 32              |

A warehouse with longer travel times needs proportionally more HTMs per
station. A faster operator (lower P) also increases HTM demand.

## What the FPI Metrics Mean

`desbench fpi` computes several related but distinct metrics:

### FPI (Fetch Pressure Index)

Mean one-way travel time from each relay to its **nearest** station:

```
FPI = (1 / N_relays) * SUM over relays r: min over stations s: A*(r, s)
```

For cencora_phase0: **FPI = 27.4s**. This is the best-case mean: each relay
is paired with its closest station.

### F_cycle_mean (Mean Round-Trip)

Mean round-trip time, where each relay is paired with its nearest station:

```
F_cycle_mean = mean over relays r: min over stations s: (F_est(r,s) + F_return(r,s))
```

For cencora_phase0: **F_cycle_mean = 54.5s +/- 18.4s**. Note that
F_cycle_mean is NOT simply 2 x FPI in general — the return leg (unladen HTM)
may use different routing than the outbound leg (laden HTM), and the nearest
station may differ for outbound vs return because routing is asymmetric.

### Per-Station One-Way Means

`desbench fpi` also reports the mean one-way travel time from ALL relays to
each station. These are useful for understanding station accessibility but
**do not represent the relays that would actually be routed to that station**.
In practice, each relay is routed to its nearest station, so a station's
effective relay set is smaller and closer than the full-warehouse average.

cencora_phase0 per-station one-way means (all relays -> that station):

| Station | Mean one-way (s) | sigma (s) | Min (s) | Max (s) |
|---------|-----------------|-----------|---------|---------|
| 1       | 79.2            | 36.5      | 15.0    | 142.0   |
| 2       | 68.9            | 32.7      | 15.0    | 128.7   |
| 3       | 61.2            | 27.2      | 15.0    | 115.3   |
| 4       | 56.2            | 21.8      | 15.0    | 102.0   |
| 5       | 54.2            | 19.1      | 15.0    | 92.0    |
| 6       | 53.9            | 19.2      | 15.0    | 94.0    |
| 7       | 56.2            | 23.5      | 15.0    | 107.3   |
| 8       | 63.5            | 28.1      | 15.0    | 120.7   |

The 1.5x spread between Station 1 (79.2s) and Station 6 (53.9s) reflects
their positions relative to the warehouse layout. Stations closer to relay
clusters have shorter mean travel times.

**Note:** These per-station means are across ALL relays in the warehouse.
For per-station fleet sizing, the relevant metric is the mean round-trip
from only the relays that would actually be assigned to that station (i.e.,
relays whose nearest station is that one). This narrower set will have
shorter travel times than the all-relay average shown here.

### F_max (Worst-Case One-Way)

Maximum one-way travel time across all relay-station pairs: **142.0s** for
cencora. This is the binding constraint for the assignment depth check
(`pipeline_span <= F_max`).

## Variance and Safety Margin

F_cycle has variance (sigma = 18.4s for cencora, CV = 34%). The mean-based
formula gives the minimum for zero-variance steady state. For production
sizing with a target of < 5% station idle time, use the 95th-percentile
formula:

```
HTMs_per_station_95 = ceil((F_cycle_mean + 1.645 * sigma_F) / P)
```

For cencora:
```
ceil((54.5 + 1.645 * 18.4) / 9.0) = ceil(84.8 / 9.0) = ceil(9.42) = 10
```

Total for 10 stations at p95: 100 HTMs. With ~200 available for picking
(of 400 total), this provides 2x headroom.

**Note on sigma:** The 18.4s value is the standard deviation of per-relay
min round-trips from the static A* computation. It reflects geometric
variance across the warehouse layout. The actual in-simulation variance
will be higher due to dynamic effects (congestion, queueing, HTM-to-relay
travel). For production safety margins, use the in-simulation measured sigma
if available.

**Note on independence:** The p95 formula assumes each in-flight HTM's
round-trip time is independent. In practice, HTMs share aisles and relays,
introducing positive correlation — when one HTM is slow (due to congestion),
others tend to be slow too. This means the independence assumption
**understates** the practical requirement; the formula may undersize in
high-contention scenarios. With cencora's 2.5x headroom at p95 this is not
a concern, but for tightly-sized fleets, use in-simulation measurements to
validate.

## Static A* vs In-Simulation F_cycle

The `desbench fpi` command computes F_cycle using static A* pathfinding on
the compiled map. This is a **lower bound** on the actual in-simulation
F_cycle because it excludes:

- **HTM-to-relay leg**: the travel time from an idle HTM's current position to
  the relay where it picks up the tote. At steady state with abundant idle
  HTMs, this adds roughly 5-15s depending on spatial distribution.
- **Relay contention**: multiple HTMs competing for the same relay cluster.
- **Congestion**: traffic on shared highways and approach lanes.
- **Queueing**: HTMs waiting at the station for their turn.

The actual in-simulation F_cycle will be somewhat higher than the static A*
measurement. The degree of increase depends on fleet utilization and layout
congestion — desbench sweeps at different fleet sizes will quantify this for
a given map.

## HTM Saturation Count

The total HTMs required to keep all stations continuously fed:

```
N_saturation = N_stations * ceil(F_cycle_mean / P)
```

This uses the global F_cycle_mean (per-relay min round-trip to nearest
station). It is a lower-bound approximation that assumes all stations
have similar effective travel times. For layouts with significant
per-station heterogeneity, the actual requirement may be higher.

For cencora_phase0:
- N_saturation (mean): 56 HTMs (lower bound)
- N_saturation (p95): 80 HTMs
- Fleet available for picking: ~200 (of 400 total; putaway not implemented)
- Headroom: 3.6x (mean) / 2.5x (p95)

## How Pipeline Depth Is Controlled

**Target architecture (Phase 1 of the dispatch timing addendum):** The
dispatch system does not use a hard HTM count cap. A hard cap would require
per-station in-flight tracking and complicate re-evaluation on wake-up.
Instead, pipeline depth is an emergent property of the dispatch timing:

1. Each pick has a computed `target_arrival` time (when the tote should arrive
   at the station).
2. Each pick has a computed `dispatch_time = target_arrival - F_est` (when to
   send the HTM so it arrives on time).
3. The dispatcher schedules a wake-up event at `dispatch_time`.
4. When the wake-up fires, the dispatcher re-evaluates and sends the nearest
   idle HTM.

At steady state with P=9s and 10 stations, this produces approximately 1.1
dispatcher wake-up events per second of sim time — a sustained, low-rate
stream rather than a burst.

This naturally produces ~ceil(F_cycle / P) HTMs in-flight per station — the
formula is descriptive, not prescriptive. The time-based assignment depth
check (`pipeline_span <= F_max`) controls how many orders are assigned ahead,
which indirectly bounds how many HTMs can be in-flight.

**Current state:** The staggered wake-up dispatch mechanism described above
is not yet active. The current code dispatches all tasks immediately
(order.rs:896). Implementing the staggered model is Phase 1 of the dispatch
timing addendum.

## Measuring These Metrics

```bash
# Compute FPI, F_cycle, and saturation for a map
desbench fpi <map.json>

# Override dwell time (default 9.0s)
desbench fpi <map.json> --pick-time 6.0
```

The output includes per-station one-way breakdown, overall FPI, F_cycle_mean,
F_max, and HTM saturation count.
