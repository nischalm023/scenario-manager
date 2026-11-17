# Algorithm Scenario Schema

## AlgorithmScenario Record

```java
public record AlgorithmScenario(
    String id,                              // Unique identifier
    String name,                            // Human-readable name
    String description,                     // Use case description
    String createdDate,                     // ISO 8601 timestamp
    AlgorithmConfig algorithmConfig         // Algorithm configuration
)
```

---

## Complete JSON Schema

### Default/Balanced Example

```json
{
  "id": "balanced",
  "name": "Balanced Configuration",
  "description": "Default algorithm settings suitable for most scenarios",
  "createdDate": "2024-01-01T00:00:00",
  "algorithmConfig": {
    "orderSelectionAlgorithm": "maximize_ppf_directly",
    "putAlgorithm": "greedy",
    "navFactor": 1.8,
    "maxTotesPerOrder": 10,
    "relayZoneLoadBalancingEnabled": true,
    "relayZoneLoadBalancingBucketSize": 5
  }
}
```

### High Throughput Example

```json
{
  "id": "high_throughput",
  "name": "High Throughput",
  "description": "Optimized for maximum throughput with TPQ-based order selection",
  "createdDate": "2024-01-01T00:00:00",
  "algorithmConfig": {
    "orderSelectionAlgorithm": "maximize_tpq_with_vut",
    "putAlgorithm": "greedy",
    "navFactor": 1.5,
    "relayZoneLoadBalancingEnabled": true,
    "tpqConfig": {
      "targetTPQ": null,
      "returnFactor": 0.8,
      "stationDeviation": 1.0,
      "aStarPenaltyFactor": 1.0
    }
  }
}
```

### Conservative Example

```json
{
  "id": "conservative",
  "name": "Conservative",
  "description": "Higher nav factor for conservative travel time estimation",
  "createdDate": "2024-01-01T00:00:00",
  "algorithmConfig": {
    "orderSelectionAlgorithm": "maximize_ppf_with_tiering",
    "putAlgorithm": "greedy",
    "navFactor": 2.2,
    "relayZoneLoadBalancingEnabled": true,
    "relayZoneLoadBalancingBucketSize": 8
  }
}
```

---

## AlgorithmConfig Schema

```json
{
  "orderSelectionAlgorithm": "maximize_ppf_directly",  // Required
  "putAlgorithm": "greedy",                            // Default: "greedy"
  "navFactor": 1.8,                                    // Default: 1.8
  "maxTotesPerOrder": 10,                              // Default: 10
  "relayZoneLoadBalancingEnabled": true,               // Default: true
  "relayZoneLoadBalancingBucketSize": 5,               // Default: 5
  "tpqConfig": { ... }                                 // Optional, for TPQ algorithm
}
```

---

## Order Selection Algorithms

### maximize_ppf_directly

**Best for:** General use, most scenarios

- Directly maximizes picks per face (PPF)
- Greedy approach, fast execution
- Good balance of throughput and simplicity

```json
{
  "orderSelectionAlgorithm": "maximize_ppf_directly"
}
```

### maximize_ppf_with_tiering

**Best for:** Large warehouses with varied SKU popularity

- PPF optimization with tiering
- Considers SKU popularity tiers
- Better for high-volume operations

```json
{
  "orderSelectionAlgorithm": "maximize_ppf_with_tiering"
}
```

### maximize_tpq_with_vut

**Best for:** Relay systems, high-throughput requirements

- TPQ (Totes Per Question) based optimization
- VUT (Variable Unit Time) adjustment
- Requires `tpqConfig` for tuning

```json
{
  "orderSelectionAlgorithm": "maximize_tpq_with_vut",
  "tpqConfig": {
    "targetTPQ": null,
    "returnFactor": 0.8,
    "stationDeviation": 1.0,
    "aStarPenaltyFactor": 1.0
  }
}
```

---

## TPQAlgorithmConfig Schema

```json
{
  "targetTPQ": null,                        // Auto-calculated if null
  "returnFactor": 0.8,                      // Default: 0.8
  "stationDeviation": 1.0,                  // Default: 1.0
  "navFactor": 1.8,                         // Default: 1.8 (can override parent)
  "aStarPenaltyFactor": 1.0                 // Default: 1.0 (no penalty)
}
```

**Parameters:**
| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| `targetTPQ` | null or 1-50 | null | Target totes per question (auto if null) |
| `returnFactor` | 0.1-1.0 | 0.8 | Factor for return trip estimation |
| `stationDeviation` | 0.1-3.0 | 1.0 | Station assignment deviation tolerance |
| `aStarPenaltyFactor` | 0.5-3.0 | 1.0 | A* pathfinding penalty multiplier |

---

## Put Algorithms

### greedy

**Best for:** Most scenarios, fast execution

- O(n*m) time complexity
- Fast placement decisions
- Default choice

```json
{
  "putAlgorithm": "greedy"
}
```

### min_weighted_cycle_distance

**Best for:** Optimization-heavy scenarios

- Uses OR-Tools LP solver
- Minimizes weighted cycle distance
- Slower but more optimal placement

```json
{
  "putAlgorithm": "min_weighted_cycle_distance"
}
```

---

## Navigation Factor (navFactor)

The `navFactor` multiplies estimated travel times to account for:
- Traffic congestion
- Path deviations
- Real-world delays

| Value | Use Case |
|-------|----------|
| 1.0-1.5 | Aggressive, assumes minimal congestion |
| 1.5-2.0 | Balanced (default 1.8) |
| 2.0-2.5 | Conservative, high-traffic environments |
| 2.5-3.0 | Very conservative, congested systems |

---

## Relay Zone Load Balancing

For relay systems, controls how VTMs are assigned across zones:

```json
{
  "relayZoneLoadBalancingEnabled": true,    // Enable/disable balancing
  "relayZoneLoadBalancingBucketSize": 5     // Bucket size for balancing
}
```

| Parameter | Default | Description |
|-----------|---------|-------------|
| `relayZoneLoadBalancingEnabled` | true | Enable zone load balancing |
| `relayZoneLoadBalancingBucketSize` | 5 | Number of tasks per balancing bucket |

---

## Algorithm Selection Guide

| Scenario | Recommended Algorithm | Notes |
|----------|----------------------|-------|
| Small warehouse (<500 SKUs) | `maximize_ppf_directly` | Simple, fast |
| Large warehouse (>2000 SKUs) | `maximize_ppf_with_tiering` | Better for varied popularity |
| Relay system | `maximize_tpq_with_vut` | Optimized for VTM/HTM handoffs |
| High throughput required | `maximize_tpq_with_vut` | With low `navFactor` (1.5) |
| Conservative/reliable | `maximize_ppf_directly` | With high `navFactor` (2.2) |

---

## File Location

`apps/fleet-simulator/src/main/resources/scenarios/algorithms/{id}/scenario_info.json`

## Related Classes

- `ScenarioModels.AlgorithmScenario` - Record definition
- `AlgorithmScenarioManager` - CRUD operations
- `SimulationConfigs.AlgorithmConfig` - Algorithm configuration
- `SimulationConfigs.TPQAlgorithmConfig` - TPQ-specific config
- `SimulationConfigs.OrderSelectionAlgorithm` - Enum of algorithms
