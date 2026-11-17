---
name: scenario-management
description: Use when creating, managing, or generating scenarios for simulation. Covers infrastructure (layout+fleet), workloads (SKU+orders), and algorithms (order selection, put algorithm).
---

# Scenario Management

## Scenario Types

| Type | Purpose | Storage Path | ID Prefix |
|------|---------|--------------|-----------|
| **Infrastructure** | Layout + robots + operators | `scenarios/infrastructure/{id}/scenario_info.json` | `infra_` |
| **Workload** | SKU catalog + orders | `scenarios/workloads/{id}/` (CSV + JSON) | `scenario_` |
| **Algorithm** | Order selection + put algo + tuning | `scenarios/algorithms/{id}/scenario_info.json` | `algo_` |

## System Types

| System | Description | Robot Config Fields |
|--------|-------------|---------------------|
| **RTP** | Racks-to-Picker - entire racks transported | `numRobots`, `robotSpeed` |
| **Relay** | VTMs + HTMs with zone-based handoffs | `numVtms`, `numHtms`, `vtmSpeed`, `htmSpeed`, `vtmType` |

---

## Quick Reference: Infrastructure Scenarios

**Minimal RTP:**
```json
{
  "id": "my_rtp", "name": "My RTP Warehouse", "systemType": "rtp",
  "mapConfig": { "gridWidth": 20, "gridDepth": 20, "cellSizeMeters": 1.0 },
  "robotConfig": { "configType": "rtp", "numRobots": 4, "robotSpeed": 1.5 },
  "operatorConfig": { "operatorTimePerPick": 12.0, "numBins": 8, "targetPicksPerFace": 3.0 }
}
```

**Minimal Relay:**
```json
{
  "id": "my_relay", "name": "My Relay Warehouse", "systemType": "relay",
  "mapConfig": { "gridWidth": 20, "gridDepth": 20, "verticalToteCapacityInRack": 12 },
  "robotConfig": {
    "configType": "relay", "vtmType": "ROBOT",
    "numVtms": 4, "vtmHorizontalSpeed": 1.5, "vtmVerticalSpeed": 1.5, "vtmCapacity": 8,
    "numHtms": 4, "htmSpeed": 1.5
  },
  "operatorConfig": { "operatorTimePerPick": 12.0, "numBins": 8, "targetPicksPerFace": 3.0 }
}
```

**Key Map Config Fields:**

| Field | Default | Constraints |
|-------|---------|-------------|
| `stationLayout.layoutType` | `SINGLE_BOTTOM` | `SINGLE_BOTTOM`, `SINGLE_TOP`, `SINGLE_LEFT`, `SINGLE_RIGHT` |
| `stationLayout.totalStations` | 4 | 1-20 |
| `rackBlockConfig.rowsPerBlock` | 4 | 1-10 |
| `rackBlockConfig.rackColumns` | 8 | 1-5 (critical for set-cover) |
| `highwayConfig.highwaysPerEdge` | 2 | 1-4 |
| `verticalToteCapacityInRack` | 12 | 8-20 (relay only) |

---

## Quick Reference: Workload Scenarios

**Files in workload directory:**
- `scenario_info.json` - Metadata
- `item_catalog.csv` - SKUs with inventory quantities
- `orders.csv` - Orders with picks

**Generation Parameters:**

| Parameter | Description | Range |
|-----------|-------------|-------|
| `numSkus` | Total SKU count | 10-10000 |
| `totalInventoryUnits` | Total inventory | 100-1000000 |
| `totalOrderPicks` | Total picks | 10-100000 |
| `numOrders` | Order count | 10-10000 |
| `popularSkusPercentage` | % of SKUs marked popular | 1-99 |
| `inventoryPopularQuantityShare` | % inventory in popular SKUs | 1-99 |
| `orderPopularPicksShare` | % picks from popular SKUs | 1-99 |

---

## Quick Reference: Algorithm Scenarios

**Order Selection Algorithms:**

| Algorithm | Use Case |
|-----------|----------|
| `maximize_ppf_directly` | Default, maximizes picks per face |
| `maximize_ppf_with_tiering` | PPF with tiered optimization |
| `maximize_tpq_with_vut` | TPQ-based, best for relay systems |

**Put Algorithms:**

| Algorithm | Description |
|-----------|-------------|
| `greedy` | Fast O(n*m) placement (default) |
| `min_weighted_cycle_distance` | LP optimization via OR-Tools |

**Minimal Algorithm Config:**
```json
{
  "id": "my_algo", "name": "My Algorithm",
  "algorithmConfig": {
    "orderSelectionAlgorithm": "maximize_ppf_directly",
    "putAlgorithm": "greedy",
    "navFactor": 1.8
  }
}
```

---

## API Endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| GET | `/api/scenarios/infrastructure` | List all |
| GET | `/api/scenarios/infrastructure/{id}` | Get one |
| POST | `/api/scenarios/infrastructure/create` | Create |
| DELETE | `/api/scenarios/infrastructure/{id}` | Delete |
| GET | `/api/scenarios/workloads` | List all |
| GET | `/api/scenarios/workloads/{id}` | Get one |
| POST | `/api/scenarios/workloads/create` | Generate |
| DELETE | `/api/scenarios/workloads/{id}` | Delete |
| GET | `/api/scenarios/algorithms` | List all |
| POST | `/api/scenarios/algorithms/create` | Create |
| DELETE | `/api/scenarios/algorithms/{id}` | Delete |

---

## Key Classes

| Class | Location | Purpose |
|-------|----------|---------|
| `ScenarioModels` | `scenarios/` | Records: `InfrastructureScenario`, `AlgorithmScenario`, `ScenarioInfo` |
| `BaseScenarioManager<T>` | `scenarios/` | Abstract CRUD (list, get, delete) |
| `InfrastructureScenarioManager` | `scenarios/` | Infrastructure CRUD |
| `AlgorithmScenarioManager` | `scenarios/` | Algorithm CRUD |
| `ScenarioManager` | `scenarios/` | Workload CRUD + generation |
| `ScenarioGenerator` | `scenarios/` | Pareto-distributed workload generation |
| `ScenariosController` | `api/scenarios/` | REST endpoints |
| `SimulationConfigs` | `simulator/config/` | `RobotConfig`, `OperatorConfig`, `AlgorithmConfig` |
| `LayoutConfigDTO` | `api/dto/` | Map configuration |

**Base path:** `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/`

---

## Pre-built Scenarios

| Path | Type | Description |
|------|------|-------------|
| `scenarios/infrastructure/default_rtp/` | Infrastructure | 4 robots, 4 stations |
| `scenarios/infrastructure/default_relay/` | Infrastructure | 4 VTMs, 4 HTMs |
| `scenarios/workloads/scenario_a/` | Workload | 500 SKUs, 1000 orders |
| `scenarios/workloads/scenario_b/` | Workload | 4100 SKUs, high-volume |
| `scenarios/algorithms/high_throughput/` | Algorithm | TPQ optimization |

**Base path:** `apps/fleet-simulator/src/main/resources/`

---

## Testing

**Integration test configs:** `apps/fleet-simulator/src/test/resources/`
- `smoke-test-rtp.json` - Quick RTP verification
- `smoke-test-relay.json` - Quick Relay verification
- `smoke-test-shuttle.json` - Shuttle VTM variant

**Unit tests with null client:**
```java
PutServiceClient putServiceClient = null; // WarehouseBuilder handles gracefully
```

---

## References

- `references/infrastructure-schema.md` - Full JSON schema with examples
- `references/workload-schema.md` - Workload schema + CSV formats
- `references/algorithm-schema.md` - Algorithm schema + tuning guidance
- `references/api-endpoints.md` - Complete API with cURL examples
- `references/code-patterns.md` - Extension patterns

## Related Skills

- `.claude/skills/relay-system/` - VTM/HTM orchestration patterns
- `.claude/skills/java-backend/` - Desmo-J simulation patterns
- `.claude/skills/architecture/` - Service boundaries
