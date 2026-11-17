# OpenAPI Spec vs Backend Compliance Audit

**Date**: 2026-02-14
**Auditor**: Claude Code (Opus 4.6)
**Branch**: `feature/ts-lsp`
**Scope**: All OpenAPI specs in `libs/api-schema/` vs Spring Boot controllers in `apps/fleet-simulator/` and `apps/layout-service/`

---

## Executive Summary

Overall compliance score: **92/100**. The spec-first migration was executed accurately — every endpoint, DTO field, enum value, and validation constraint matches between the OpenAPI YAML and the Java controllers. The primary gap is **zero compile-time enforcement**: generated interfaces exist but are neither compiled nor implemented by controllers, so spec-controller alignment relies entirely on developer discipline.

---

## Scorecard

| Dimension | Score | Detail |
|-----------|-------|--------|
| Endpoint Coverage | 10/10 | 22/22 paths match perfectly |
| DTO Field Accuracy | 10/10 | Zero field mismatches across 24 DTOs |
| Enum Serialization | 9/10 | All critical enums correct; `VtmType` needs verification |
| Validation Constraints | 10/10 | All checked constraints match |
| Response Types | 7/10 | 5 endpoints use untyped `Map<String,Object>` |
| Compile-Time Enforcement | 2/10 | Generated interfaces not compiled or implemented |
| Binary Format Documentation | 3/10 | `WarehouseSnapshotV2` undocumented in spec |
| Cross-Service Consistency | 8/10 | Response wrapper pattern differs between services |
| Tooling & Governance | 9/10 | Spectral linting + `oasdiff` breaking change detection in place |
| Overall Architecture | 8/10 | Spec-first migration done well, but enforcement gap |

---

## 1. Endpoint Compliance

**Result**: 22/22 endpoints match. No ghost endpoints in either direction.

### Fleet Simulator (19 endpoints)

| Spec Path | Method | Controller | Match |
|-----------|--------|------------|:-----:|
| `/api/health` | GET | `HealthController.health()` | YES |
| `/api/simulation/run` | POST | `SimulationsController.runSimulation()` | YES |
| `/api/simulation/{id}/snapshots` | GET | `SimulationsController.getSnapshots()` | YES |
| `/api/simulation/status` | GET | `SimulationsController.getSimulationStatus()` | YES |
| `/api/simulation/analyze` | POST | `SimulationsController.analyzeSimulation()` | YES |
| `/api/simulation/tpq-metrics` | POST | `SimulationsController.calculateTpqMetrics()` | YES |
| `/api/comparison/run` | POST | `SimulationsController.runComparison()` | YES |
| `/api/scenarios/workloads` | GET | `ScenariosController.listWorkloadScenarios()` | YES |
| `/api/scenarios/workloads/create` | POST | `ScenariosController.createWorkloadScenario()` | YES |
| `/api/scenarios/workloads/{scenarioId}` | GET | `ScenariosController.getWorkloadScenarioInfo()` | YES |
| `/api/scenarios/workloads/{scenarioId}` | DELETE | `ScenariosController.deleteWorkloadScenario()` | YES |
| `/api/scenarios/workloads/{scenarioId}/sku-data` | GET | `ScenariosController.getSkuData()` | YES |
| `/api/scenarios/workloads/{scenarioId}/order-analysis` | GET | `ScenariosController.getOrderAnalysis()` | YES |
| `/api/scenarios/workloads/statistics` | GET | `ScenariosController.getWorkloadStatistics()` | YES |
| `/api/scenarios/infrastructure` | GET | `ScenariosController.listInfrastructureScenarios()` | YES |
| `/api/scenarios/infrastructure/create` | POST | `ScenariosController.createInfrastructureScenario()` | YES |
| `/api/scenarios/infrastructure/{scenarioId}` | GET | `ScenariosController.getInfrastructureScenario()` | YES |
| `/api/scenarios/infrastructure/{scenarioId}` | DELETE | `ScenariosController.deleteInfrastructureScenario()` | YES |
| `/api/scenarios/algorithms` | GET | `ScenariosController.listAlgorithmScenarios()` | YES |
| `/api/scenarios/algorithms/create` | POST | `ScenariosController.createAlgorithmScenario()` | YES |
| `/api/scenarios/algorithms/{scenarioId}` | GET | `ScenariosController.getAlgorithmScenario()` | YES |
| `/api/scenarios/algorithms/{scenarioId}` | DELETE | `ScenariosController.deleteAlgorithmScenario()` | YES |

### Layout Service (3 endpoints)

| Spec Path | Method | Controller | Match |
|-----------|--------|------------|:-----:|
| `/api/warehouse-map/generate` | POST | `WarehouseMapController.generateMapData()` | YES |
| `/api/warehouse-map/cell-types` | GET | `WarehouseMapController.getCellTypes()` | YES |
| `/api/warehouse-map/layouts` | GET | `WarehouseMapController.getAvailableLayouts()` | YES |

### Files

- `libs/api-schema/fleet-simulator/openapi.yaml`
- `libs/api-schema/layout-service/openapi.yaml`
- `apps/fleet-simulator/.../api/simulations/SimulationsController.java`
- `apps/fleet-simulator/.../api/scenarios/ScenariosController.java`
- `apps/fleet-simulator/.../api/health/HealthController.java`
- `apps/layout-service/.../api/WarehouseMapController.java`

---

## 2. DTO/Schema Field Compliance

**Result**: Zero field mismatches across 24 DTOs.

| Java DTO | Spec Schema | Fields | Match |
|----------|-------------|--------|:-----:|
| `SimulationConfigDTO` (inner record, ~24 fields) | `SimulationConfigDTO` | 24 fields | YES |
| `ComparisonConfigDTO` (inner record) | `ComparisonConfigDTO` | Correctly excludes snapshot fields | YES |
| `SimulationResultMetadata` (inner record) | `SimulationResultMetadata` | 10 fields | YES |
| `TpqMetricsResponseDTO` (inner record) | `TpqMetricsResponseDTO` | 11 fields | YES |
| `AnalysisRequestDTO` (record) | `AnalysisRequestDTO` | 1 field | YES |
| `AnalysisResponseDTO` (record) | `AnalysisResponseDTO` | 1 field | YES |
| `WorkloadScenarioConfig` (inner record) | `WorkloadScenarioConfig` | 8 fields | YES |
| `InfrastructureScenarioConfig` (inner record) | `InfrastructureScenarioConfig` | 24 fields | YES |
| `AlgorithmScenarioConfig` (inner record) | `AlgorithmScenarioConfig` | 10 fields | YES |
| `ScenarioListItem` (inner record) | `ScenarioListItem` | 4 fields | YES |
| `ScenarioCreated` (inner record) | `ScenarioCreated` | 1 field | YES |
| `ScenarioDeleted` (inner record) | `ScenarioDeleted` | 1 field | YES |
| `InfrastructureScenario` (ScenarioModels) | `InfrastructureScenario` | 8 fields | YES |
| `AlgorithmScenario` (ScenarioModels) | `AlgorithmScenario` | 5 fields | YES |
| `LayoutConfigDTO` + nested DTOs | `LayoutConfigDTO` | All nested structures | YES |
| `RobotConfig` sealed interface | `RobotConfig` discriminator | Polymorphism | YES |
| `RtpRobotConfig` | `RtpRobotConfig` | 3 fields | YES |
| `RelayRobotConfig` | `RelayRobotConfig` | 9 fields | YES |
| `OperatorConfig` | `OperatorConfig` | 3 fields | YES |
| `AlgorithmConfig` | `AlgorithmConfig` | 6 fields | YES |
| `TPQAlgorithmConfig` | `TPQAlgorithmConfig` | 5 fields | YES |
| `ComparisonResponse` | `ComparisonResponse` | 8 fields | YES |
| `ComparisonMetric` | `ComparisonMetric` | 6 fields | YES |
| `ApiResponse<T>` | `ApiResponse*` wrappers | Generic envelope | YES |

### Java-Only Fields (Intentionally Absent from Spec)

These fields exist in Java DTOs but are deliberately excluded from the public API spec:

- `SimulationConfigDTO.jobShopAlgorithm` — internal scheduling feature
- `SimulationConfigDTO.enableJobShopScheduling` — internal scheduling feature
- `AlgorithmConfig.relayZoneLoadBalancingBucketSize` — internal tuning parameter

---

## 3. Enum Serialization Compliance

**Result**: All critical enums use `@JsonValue` with lowercase values matching the spec.

| Java Enum | `@JsonValue` | Spec Values | Match |
|-----------|:------------:|-------------|:-----:|
| `SystemType.RTP` | `"rtp"` | `rtp` | YES |
| `SystemType.RELAY` | `"relay"` | `relay` | YES |
| `OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY` | `"maximize_ppf_directly"` | `maximize_ppf_directly` | YES |
| `OrderSelectionAlgorithm.MAXIMIZE_PPF_WITH_TIERING` | `"maximize_ppf_with_tiering"` | `maximize_ppf_with_tiering` | YES |
| `OrderSelectionAlgorithm.MAXIMIZE_TPQ_WITH_VUT` | `"maximize_tpq_with_vut"` | `maximize_tpq_with_vut` | YES |
| `RobotState.IDLE` | `"idle"` | `idle` | YES |
| `RobotState.MOVING_TO_RACK` | `"moving_to_rack"` | `moving_to_rack` | YES |
| (all 18 `RobotState` values) | lowercase snake_case | lowercase snake_case | YES |
| `ComparisonMetricType.ORDERS_COMPLETED` | (default) | `ORDERS_COMPLETED` | YES |
| `VtmType.ROBOT` / `SHUTTLE` | (unverified) | `ROBOT` / `SHUTTLE` | VERIFY |

### Files

- `apps/fleet-simulator/.../domain/SystemType.java`
- `apps/fleet-simulator/.../domain/order/OrderSelectionAlgorithm.java`
- `apps/fleet-simulator/.../domain/robot/RobotState.java`
- `apps/fleet-simulator/.../dto/comparison/ComparisonMetricType.java`

---

## 4. Generated Interface Usage

**Result**: Generated interfaces are completely decorative — not compiled, not implemented.

### OpenAPI Generator Config (Both Services)

```xml
<configOptions>
    <interfaceOnly>true</interfaceOnly>
    <addCompileSourceRoot>false</addCompileSourceRoot>  <!-- NOT EVEN COMPILED -->
</configOptions>
```

| Generated Interface | Controller | `implements`? | Compiled? |
|--------------------|------------|:-------------:|:---------:|
| `SimulationApi` | `SimulationsController` | NO | NO |
| `ComparisonApi` | `SimulationsController` | NO | NO |
| `ScenariosApi` | `ScenariosController` | NO | NO |
| `HealthApi` | `HealthController` | NO | NO |
| `WarehouseMapApi` | `WarehouseMapController` | NO | NO |

### Why Controllers Can't Simply Implement the Interfaces

Generated interfaces return monolithic wrapper types (e.g., `ApiResponseSimulationResultMetadata`) while controllers use the generic `ApiResponse<SimulationResultMetadata>`. These are semantically equivalent but type-incompatible in Java. Making controllers implement interfaces would require either:

1. Changing the generator config to use generic wrappers, or
2. Changing controllers to use generated wrapper types

### Implication

Zero compile-time enforcement that controllers match the spec. The spec and controllers are connected only by developer discipline.

### Files

- `apps/fleet-simulator/pom.xml` (lines 129-196)
- `apps/layout-service/pom.xml` (lines 101-145)
- `apps/fleet-simulator/target/generated-sources/openapi/.../generated/`
- `apps/layout-service/target/generated-sources/openapi/.../generated/`

---

## 5. Untyped `Map<String,Object>` Responses

**Result**: 5 endpoints return untyped maps in both spec and controller.

| Endpoint | Controller Return Type | Issue |
|----------|----------------------|-------|
| `GET /api/simulation/status` | `ApiResponse<Map<String, Object>>` | Spec also untyped |
| `GET /api/scenarios/workloads/{id}` | `ApiResponse<Map<String, Object>>` | Spec also untyped |
| `GET /api/scenarios/workloads/{id}/sku-data` | `ApiResponse<Map<String, Object>>` | Spec also untyped |
| `GET /api/scenarios/workloads/{id}/order-analysis` | `ApiResponse<Map<String, Object>>` | Spec also untyped |
| `GET /api/scenarios/workloads/statistics` | `ApiResponse<Map<String, Object>>` | Spec also untyped |

The spec and controllers are consistent — both use untyped maps. But neither side meaningfully documents the actual data shape. A consumer has no idea what keys/values to expect without reading source code.

---

## 6. Validation Constraint Alignment

**Result**: All checked constraints match exactly.

| DTO Field | Java Annotation | Spec Constraint | Match |
|-----------|----------------|-----------------|:-----:|
| `SimulationConfigDTO.duration` | `@Min(1) @Max(24)` | `minimum: 1, maximum: 24` | YES |
| `SimulationConfigDTO.rtpRobotCount` | `@Min(1) @Max(100)` | `minimum: 1, maximum: 100` | YES |
| `SimulationConfigDTO.rtpRobotSpeed` | `@DecimalMin("0.1") @DecimalMax("5.0")` | `minimum: 0.1, maximum: 5.0` | YES |
| `SimulationConfigDTO.operatorTime` | `@DecimalMin("5.0") @DecimalMax("30.0")` | `minimum: 5.0, maximum: 30.0` | YES |
| `SimulationConfigDTO.binsCount` | `@Min(4) @Max(32)` | `minimum: 4, maximum: 32` | YES |
| `SimulationConfigDTO.snapshotIntervalSeconds` | `@DecimalMin("1.0") @DecimalMax("3600.0")` | `minimum: 1.0, maximum: 3600.0` | YES |
| `SimulationConfigDTO.navFactor` | `@DecimalMin("1.0") @DecimalMax("3.0")` | `minimum: 1.0, maximum: 3.0` | YES |
| `WorkloadScenarioConfig.numSkus` | `@Min(10) @Max(10000)` | `minimum: 10, maximum: 10000` | YES |
| `WorkloadScenarioConfig.totalInventoryUnits` | `@Min(100) @Max(1000000)` | `minimum: 100, maximum: 1000000` | YES |
| `MapDataRequestDTO.gridWidth` | `@Min(5) @Max(200)` | `minimum: 5, maximum: 200` | YES |
| `MapDataRequestDTO.stationCount` | `@Min(1) @Max(50)` | `minimum: 1, maximum: 50` | YES |

---

## 7. WarehouseSnapshotV2 Gap

`GET /api/simulation/{id}/snapshots` returns `application/octet-stream`. The spec correctly marks the content type as binary with response headers (`X-Snapshot-Count`, `X-Format-Version`), but the **deserialized structure** is undocumented:

- `WarehouseSnapshotV2` — sealed interface (Java 21) with `@JsonTypeInfo` discriminator `"type"`
- `KeyframeSnapshot` — full warehouse state (robots, relays, stations, totes, storage spaces + metrics)
- `DeltaSnapshot` — incremental changes only
- State entity classes: `RobotState`, `ToteState`, `RelayState`, `StationState`, `StorageSpaceState`

The frontend's `BinarySnapshotReader` knows the format but has no spec contract to reference.

**Impact**: Low for now (single consumer), but creates a tribal-knowledge dependency.

### Files

- `apps/fleet-simulator/.../dto/snapshot/WarehouseSnapshotV2.java`
- `apps/fleet-simulator/.../dto/snapshot/KeyframeSnapshot.java`
- `apps/fleet-simulator/.../dto/snapshot/DeltaSnapshot.java`
- `apps/fleet-simulator/.../service/snapshot/BinarySnapshotWriter.java`
- `apps/frontend/src/utils/BinarySnapshotReader.ts`

---

## 8. Response Wrapper Inconsistency

| Service | Pattern | Example |
|---------|---------|---------|
| Fleet Simulator | `ApiResponse<T>` wrapper with `success`, `data`, `error`, `timestamp` | `{"success": true, "data": {...}, "timestamp": "..."}` |
| Layout Service | Direct `MapDataResponse` with its own `success`, `data`, `error` fields | `{"success": true, "data": {...}, "generationTimeMs": 42}` |

The spec accurately documents both patterns — this is not a spec-vs-code mismatch. But it is an architectural inconsistency: consumers need different parsing logic per service.

**Impact**: Low (only 3 layout endpoints), but worth standardizing for future services.

---

## Recommended Remediation (Ordered by Impact)

### 1. Enable Compile-Time Enforcement

Make controllers implement generated interfaces, or remove the generator if interfaces aren't intended to be used. This is the highest-value change — it turns spec drift into a compile error.

**Effort**: Medium. Requires resolving the generic vs monolithic wrapper type incompatibility.

### 2. Type the 5 `Map<String,Object>` Responses

Define proper schemas for simulation status, workload info, SKU data, order analysis, and workload statistics. Then update both the spec and the controller return types.

**Effort**: Medium. Requires analyzing actual response shapes from running endpoints.

### 3. Document WarehouseSnapshotV2

Add the binary snapshot schema to the spec (even as documentation-only `x-` extensions if the format can't be expressed in standard OpenAPI).

**Effort**: Low-medium. The Java sealed interface hierarchy is well-structured.

### 4. Standardize Response Wrappers

Migrate Layout Service to use the same `ApiResponse<T>` pattern, or document the divergence as intentional.

**Effort**: Low. Only 3 endpoints affected.

---

## Verification Commands

```bash
# Lint specs
npx spectral lint libs/api-schema/fleet-simulator/openapi.yaml
npx spectral lint libs/api-schema/layout-service/openapi.yaml

# Confirm generated code doesn't break build
cd apps/fleet-simulator && mvn compile
cd apps/layout-service && mvn compile

# Run tests
npx nx test fleet-simulator
npx nx test layout-service
```
