# Scenario API Endpoints

Base URL: `http://localhost:8085`

---

## Infrastructure Scenarios

### List All Infrastructure Scenarios

```bash
GET /api/scenarios/infrastructure
```

**Response:**
```json
[
  {
    "id": "default_rtp",
    "name": "Default RTP Warehouse",
    "description": "Standard RTP warehouse with 4 robots",
    "systemType": "rtp",
    "createdDate": "2024-01-01T00:00:00"
  }
]
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/infrastructure
```

---

### Get Infrastructure Scenario

```bash
GET /api/scenarios/infrastructure/{id}
```

**Response:** Full `InfrastructureScenario` object

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/infrastructure/default_rtp
```

---

### Create Infrastructure Scenario

```bash
POST /api/scenarios/infrastructure/create
Content-Type: application/json
```

**Request Body (RTP):**
```json
{
  "name": "My RTP Warehouse",
  "description": "Custom RTP configuration",
  "systemType": "rtp",
  "gridWidth": 30,
  "gridDepth": 25,
  "cellSizeMeters": 1.0,
  "layoutType": "SINGLE_BOTTOM",
  "totalStations": 4,
  "highwaysPerEdge": 2,
  "rowsPerBlock": 4,
  "rackColumns": 3,
  "crossAisleWidthGridUnits": 2,
  "numRobots": 6,
  "robotSpeed": 1.5,
  "operatorTimePerPick": 12.0,
  "numBins": 8,
  "targetPicksPerFace": 3.0
}
```

**Request Body (Relay):**
```json
{
  "name": "My Relay Warehouse",
  "description": "Custom relay configuration",
  "systemType": "relay",
  "gridWidth": 40,
  "gridDepth": 30,
  "cellSizeMeters": 1.0,
  "layoutType": "SINGLE_BOTTOM",
  "totalStations": 4,
  "highwaysPerEdge": 2,
  "rowsPerBlock": 4,
  "rackColumns": 3,
  "verticalToteCapacityInRack": 12,
  "vtmType": "ROBOT",
  "numVtms": 6,
  "vtmHorizontalSpeed": 1.5,
  "vtmVerticalSpeed": 1.5,
  "vtmCapacity": 8,
  "numHtms": 4,
  "htmSpeed": 1.5,
  "operatorTimePerPick": 12.0,
  "numBins": 8,
  "targetPicksPerFace": 3.0
}
```

**cURL:**
```bash
curl -X POST http://localhost:8085/api/scenarios/infrastructure/create \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test RTP",
    "systemType": "rtp",
    "gridWidth": 20,
    "gridDepth": 20,
    "numRobots": 4,
    "robotSpeed": 1.5,
    "operatorTimePerPick": 12.0,
    "numBins": 8
  }'
```

---

### Delete Infrastructure Scenario

```bash
DELETE /api/scenarios/infrastructure/{id}
```

**cURL:**
```bash
curl -X DELETE http://localhost:8085/api/scenarios/infrastructure/infra_a
```

---

## Workload Scenarios

### List All Workload Scenarios

```bash
GET /api/scenarios/workloads
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/workloads
```

---

### Get Workload Scenario

```bash
GET /api/scenarios/workloads/{id}
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/workloads/scenario_a
```

---

### Create/Generate Workload Scenario

```bash
POST /api/scenarios/workloads/create
Content-Type: application/json
```

**Request Body:**
```json
{
  "name": "My Workload",
  "description": "500 SKUs, 1000 orders",
  "numSkus": 500,
  "totalInventoryUnits": 20000,
  "numOrders": 1000,
  "totalOrderPicks": 4000,
  "popularSkusPercentage": 20.0,
  "inventoryPopularQuantityShare": 50.0,
  "orderPopularPicksShare": 80.0,
  "categories": ["Electronics", "Sports", "Home", "Clothing", "Books"]
}
```

**cURL:**
```bash
curl -X POST http://localhost:8085/api/scenarios/workloads/create \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test Workload",
    "numSkus": 100,
    "totalInventoryUnits": 5000,
    "numOrders": 200,
    "totalOrderPicks": 800
  }'
```

---

### Delete Workload Scenario

```bash
DELETE /api/scenarios/workloads/{id}
```

**cURL:**
```bash
curl -X DELETE http://localhost:8085/api/scenarios/workloads/scenario_x
```

---

### Get Workload SKU Data

```bash
GET /api/scenarios/workloads/{id}/sku-data
```

**Response:**
```json
{
  "skuData": [
    {"sku": "SKU001", "name": "Product 1", "orderedQuantity": 150, "isPopular": true},
    {"sku": "SKU002", "name": "Product 2", "orderedQuantity": 45, "isPopular": false}
  ]
}
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/workloads/scenario_a/sku-data
```

---

### Get Workload Order Analysis

```bash
GET /api/scenarios/workloads/{id}/order-analysis
```

**Response:**
```json
{
  "totalOrders": 1000,
  "totalPicks": 4000,
  "avgPicksPerOrder": 4.0,
  "maxPicksPerOrder": 12,
  "minPicksPerOrder": 1
}
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/workloads/scenario_a/order-analysis
```

---

### Get Workload Statistics

```bash
GET /api/scenarios/workloads/statistics
```

**Response:** Aggregate statistics across all workload scenarios

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/workloads/statistics
```

---

## Algorithm Scenarios

### List All Algorithm Scenarios

```bash
GET /api/scenarios/algorithms
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/algorithms
```

---

### Get Algorithm Scenario

```bash
GET /api/scenarios/algorithms/{id}
```

**cURL:**
```bash
curl http://localhost:8085/api/scenarios/algorithms/high_throughput
```

---

### Create Algorithm Scenario

```bash
POST /api/scenarios/algorithms/create
Content-Type: application/json
```

**Request Body (Basic):**
```json
{
  "name": "My Algorithm",
  "description": "Custom algorithm configuration",
  "orderSelectionAlgorithm": "maximize_ppf_directly",
  "putAlgorithm": "greedy",
  "navFactor": 1.8
}
```

**Request Body (With TPQ Config):**
```json
{
  "name": "TPQ Optimized",
  "description": "TPQ algorithm with custom tuning",
  "orderSelectionAlgorithm": "maximize_tpq_with_vut",
  "putAlgorithm": "greedy",
  "navFactor": 1.5,
  "relayZoneLoadBalancingEnabled": true,
  "relayZoneLoadBalancingBucketSize": 5,
  "targetTPQ": null,
  "returnFactor": 0.8,
  "stationDeviation": 1.0,
  "aStarPenaltyFactor": 1.0
}
```

**cURL:**
```bash
curl -X POST http://localhost:8085/api/scenarios/algorithms/create \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test Algorithm",
    "orderSelectionAlgorithm": "maximize_ppf_directly",
    "putAlgorithm": "greedy",
    "navFactor": 1.8
  }'
```

---

### Delete Algorithm Scenario

```bash
DELETE /api/scenarios/algorithms/{id}
```

**cURL:**
```bash
curl -X DELETE http://localhost:8085/api/scenarios/algorithms/algo_x
```

---

## Error Responses

### 404 Not Found

```json
{
  "error": "Scenario not found",
  "scenarioId": "invalid_id"
}
```

### 400 Bad Request

```json
{
  "error": "Invalid configuration",
  "details": "systemType must be 'rtp' or 'relay'"
}
```

### 500 Internal Server Error

```json
{
  "error": "Failed to create scenario",
  "details": "..."
}
```

---

## Running Simulations with Scenarios

To run a simulation using scenarios:

```bash
POST /api/simulation/run
Content-Type: application/json
```

**Request with scenario references:**
```json
{
  "duration": 4,
  "scenario": "scenario_a",
  "systemType": "rtp",
  "rtpRobotCount": 4,
  "rtpRobotSpeed": 1.5,
  "mapConfig": { ... },
  "orderSelectionAlgorithm": "maximize_ppf_directly",
  "putAlgorithm": "greedy"
}
```

The `scenario` field references a workload scenario by ID. Infrastructure and algorithm configurations are typically provided inline or merged from scenario files.

---

## Controller Location

`apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/api/scenarios/ScenariosController.java`
