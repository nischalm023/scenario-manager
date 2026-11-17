# Frontend OpenAPI Type Migration - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace manual snake_case API types with auto-generated camelCase types from OpenAPI spec for compile-time type safety.

**Architecture:** Big bang migration - create new type exports, update all components bottom-up, delete old types. TypeScript compiler catches all mismatches.

**Tech Stack:** TypeScript, React, OpenAPI TypeScript Generator

**Related Design:** `docs/plans/2025-11-08-migrate-frontend-generated-types.md`

---

## Prerequisites Check

**Before starting, verify:**
- ✅ Backend running on port 8080: `curl http://localhost:8080/health`
- ✅ OpenAPI spec generated: File exists at `apps/frontend/src/types/generated-api.ts`
- ✅ Frontend builds: `cd apps/frontend && npm run typecheck`

---

## Task 1: Create Flattened Type Exports

**Goal:** Create `types/generated.ts` with easy-to-import type aliases from OpenAPI schema.

**Files:**
- Create: `apps/frontend/src/types/generated.ts`

### Step 1: Create generated.ts with flattened exports

**Action:** Create new file with type re-exports

**File:** `apps/frontend/src/types/generated.ts`

```typescript
/**
 * Flattened type exports from OpenAPI generated types
 *
 * This file provides easy-to-import type aliases for the generated OpenAPI types.
 * Import from this file instead of directly from generated-api.ts for cleaner code.
 */

import type { components, operations } from './generated-api'

// ============================================================================
// API Response Wrapper
// ============================================================================

export type ApiResponse<T> = {
  success?: boolean
  data?: T
  error?: string
  timestamp?: string
}

// ============================================================================
// Configuration Types
// ============================================================================

export type SimulationConfigDTO = components["schemas"]["SimulationConfigDTO"]
export type SimulationConfig = components["schemas"]["SimulationConfig"]
export type WarehouseMapConfig = components["schemas"]["WarehouseMapConfig"]
export type StationLayoutConfig = components["schemas"]["StationLayoutConfig"]
export type StationEdgeConfig = components["schemas"]["StationEdgeConfig"]
export type HighwayConfig = components["schemas"]["HighwayConfig"]
export type RackBlockConfig = components["schemas"]["RackBlockConfig"]
export type RobotConfig = components["schemas"]["RobotConfig"]
export type RtpRobotConfig = components["schemas"]["RtpRobotConfig"]
export type RelayRobotConfig = components["schemas"]["RelayRobotConfig"]
export type OperatorConfig = components["schemas"]["OperatorConfig"]
export type AlgorithmConfig = components["schemas"]["AlgorithmConfig"]

// ============================================================================
// Domain Objects
// ============================================================================

export type SimulationResults = components["schemas"]["SimulationResults"]
export type SimulationMetric = components["schemas"]["SimulationMetric"]
export type Robot = components["schemas"]["Robot"]
export type Rack = components["schemas"]["Rack"]
export type Station = components["schemas"]["Station"]
export type GridCoordinate = components["schemas"]["GridCoordinate"]
export type StationGridPosition = components["schemas"]["StationGridPosition"]
export type Item = components["schemas"]["Item"]
export type PickTask = components["schemas"]["PickTask"]

// ============================================================================
// Metrics Types
// ============================================================================

export type TaskMetric = components["schemas"]["TaskMetric"]
export type StorageSpaceMetric = components["schemas"]["StorageSpaceMetric"]
export type OperatorMetric = components["schemas"]["OperatorMetric"]
export type RobotMetric = components["schemas"]["RobotMetric"]
export type StationMetric = components["schemas"]["StationMetric"]
export type RelayPositionMetric = components["schemas"]["RelayPositionMetric"]
export type TaskPoolQualityMetric = components["schemas"]["TaskPoolQualityMetric"]
export type InventoryMetric = components["schemas"]["InventoryMetric"]

// ============================================================================
// API Response Types
// ============================================================================

export type ApiResponseSimulationResults = components["schemas"]["ApiResponseSimulationResults"]
export type ApiResponseListScenarioListItem = components["schemas"]["ApiResponseListScenarioListItem"]
export type ApiResponseScenarioCreated = components["schemas"]["ApiResponseScenarioCreated"]
export type ApiResponseScenarioDeleted = components["schemas"]["ApiResponseScenarioDeleted"]
export type ApiResponseMapStringObject = components["schemas"]["ApiResponseMapStringObject"]

// ============================================================================
// Scenario Types
// ============================================================================

export type ScenarioListItem = components["schemas"]["ScenarioListItem"]
export type ScenarioConfig = components["schemas"]["ScenarioConfig"]
export type ScenarioCreated = components["schemas"]["ScenarioCreated"]
export type ScenarioDeleted = components["schemas"]["ScenarioDeleted"]

// ============================================================================
// Map Types
// ============================================================================

export type MapDataRequestDTO = components["schemas"]["MapDataRequestDTO"]
export type MapDataResponse = components["schemas"]["MapDataResponse"]
export type CompactMapData = components["schemas"]["CompactMapData"]
export type MapConfiguration = components["schemas"]["MapConfiguration"]
export type Position = components["schemas"]["Position"]
export type StationElement = components["schemas"]["StationElement"]
export type RackElement = components["schemas"]["RackElement"]
export type HighwayElement = components["schemas"]["HighwayElement"]

// ============================================================================
// Comparison Types
// ============================================================================

export type ComparisonConfigDTO = components["schemas"]["ComparisonConfigDTO"]

// ============================================================================
// ID Types (wrapped objects from backend)
// ============================================================================

export type RackId = components["schemas"]["RackId"]
export type StationId = components["schemas"]["StationId"]
export type StorageSpaceId = components["schemas"]["StorageSpaceId"]
```

### Step 2: Verify file compiles

**Command:**
```bash
cd apps/frontend
npm run typecheck
```

**Expected:** No TypeScript errors (file just exports types, no usage yet)

### Step 3: Commit

```bash
git add apps/frontend/src/types/generated.ts
git commit -m "feat(types): add flattened OpenAPI type exports

Create easy-to-import type aliases from generated OpenAPI schema.
Provides cleaner import syntax for components.
"
```

---

## Task 2: Refactor UI-Only Type Files

**Goal:** Remove API types from `simulation.ts` and `scenarios.ts`, keep only UI-specific types.

**Files:**
- Modify: `apps/frontend/src/types/simulation.ts`
- Modify: `apps/frontend/src/types/scenarios.ts`
- Modify: `apps/frontend/src/types/index.ts`

### Step 1: Refactor types/simulation.ts

**Action:** Keep only UI-specific types, remove API types

**File:** `apps/frontend/src/types/simulation.ts`

```typescript
/**
 * UI-specific types for simulation visualization
 *
 * These types are NOT part of the API contract - they're frontend-only
 * structures for charts, UI state, and visualization.
 *
 * For API types, import from './generated'
 */

// ============================================================================
// Chart and Visualization Types (Frontend-only)
// ============================================================================

export interface ChartDataPoint {
  x: number
  y: number
  label?: string
}

export interface HeatmapData {
  value: number
  color: string
  label: string
}

// ============================================================================
// UI State Types (Frontend-only)
// ============================================================================

export interface UIState {
  currentPage: 'simulator' | 'scenarios'
  simulationInProgress: boolean
  lastSimulationResults: any | null  // Should use SimulationResults from generated
  activeTab: string
  sidebarExpanded: boolean
}

export interface MockSimulationResponse {
  status: 'running' | 'completed' | 'error'
  progress: number
  results?: any  // Should use SimulationResults from generated
  error?: string
}

// ============================================================================
// Legacy Config Types (Consider removing if unused)
// ============================================================================

// TODO: Check if these are actually used, might be duplicates of generated types
export interface MapConfig {
  gridWidth: number
  gridDepth: number
  cellSizeMeters: number
}

export interface StationEdgeConfig {
  edge: 'bottom' | 'top' | 'left' | 'right'
  stationCount: number
  spacingGridUnits: number
}

export interface StationLayoutConfig {
  layoutType: string
  totalStations: number
  stationEdges: StationEdgeConfig[]
}

export interface HighwayConfig {
  highwaysPerEdge: number
}

export interface RackBlockConfig {
  rowsPerBlock: number
  rackColumns: number
  crossAisleWidthGridUnits: number
}
```

### Step 2: Check types/scenarios.ts for UI-specific types

**Action:** Read and identify what to keep

**Command:**
```bash
cat apps/frontend/src/types/scenarios.ts
```

**Decision:** If `SKUData` and `OrderAnalysisData` are API types, delete the file or remove them. If they're UI transformations, keep them.

**File:** `apps/frontend/src/types/scenarios.ts` (if keeping UI types)

```typescript
/**
 * UI-specific types for scenario management
 *
 * For API types like ScenarioListItem, import from './generated'
 */

// Only keep types that are genuinely UI-specific transformations
// If these come from the API, delete this file entirely

// Example: Keep if this is a UI transformation
export interface SKUData {
  // Only if this is transformed data for UI display
}

export interface OrderAnalysisData {
  // Only if this is transformed data for UI display
}
```

### Step 3: Update types/index.ts - minimal exports

**Action:** Export only UI types, not API types

**File:** `apps/frontend/src/types/index.ts`

```typescript
/**
 * Barrel export for UI-specific types only
 *
 * For API types, import directly from './generated' for clarity
 */

// UI-specific types from simulation
export type {
  ChartDataPoint,
  HeatmapData,
  UIState,
  MockSimulationResponse,
} from './simulation'

// Legacy config types (TODO: evaluate if still needed)
export type {
  MapConfig,
  StationEdgeConfig,
  StationLayoutConfig,
  HighwayConfig,
  RackBlockConfig,
} from './simulation'

// Scenario UI types (if any remain after cleanup)
// export type { SKUData, OrderAnalysisData } from './scenarios'

// Note: API types should be imported directly from './generated'
// Example: import type { SimulationResults, Robot } from '../types/generated'
```

### Step 4: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck
```

**Expected:** May have errors from files still importing from old locations - that's OK, we'll fix in next tasks

### Step 5: Commit

```bash
git add apps/frontend/src/types/simulation.ts apps/frontend/src/types/scenarios.ts apps/frontend/src/types/index.ts
git commit -m "refactor(types): keep only UI-specific types

Remove API types from simulation.ts and scenarios.ts.
Update index.ts to export UI types only.
API types should now be imported from './generated'.
"
```

---

## Task 3: Update Service Layer

**Goal:** Update API service to use generated types instead of manual types.

**Files:**
- Modify: `apps/frontend/src/services/api.ts`

### Step 1: Update api.ts imports and signatures

**Action:** Replace manual type imports with generated types

**File:** `apps/frontend/src/services/api.ts`

**Find and replace:**
- Remove: All imports from `'../types/api'`
- Add: Imports from `'../types/generated'`
- Update: All type annotations to use generated types

**Complete file:**

```typescript
/**
 * Core API Service for Warehouse Simulator
 * Handles all communication with the Java backend
 */

import type {
  ApiResponseSimulationResults,
  ApiResponseListScenarioListItem,
  ApiResponseScenarioCreated,
  ApiResponseScenarioDeleted,
  ApiResponseMapStringObject,
  SimulationConfigDTO,
  ComparisonConfigDTO,
  ScenarioConfig,
} from '../types/generated'

const API_BASE_URL = 'http://localhost:8080'

class ApiService {
  private baseUrl: string

  constructor(baseUrl: string = API_BASE_URL) {
    this.baseUrl = baseUrl
  }

  private async request<T>(endpoint: string, options: RequestInit = {}): Promise<T> {
    const url = `${this.baseUrl}${endpoint}`

    const config: RequestInit = {
      headers: {
        'Content-Type': 'application/json',
        ...options.headers,
      },
      ...options,
    }

    try {
      const response = await fetch(url, config)

      if (!response.ok) {
        const errorText = await response.text()
        throw new Error(`HTTP ${response.status}: ${errorText}`)
      }

      return await response.json()
    } catch (error) {
      console.error(`API request failed for ${endpoint}:`, error)
      throw error
    }
  }

  // Health check
  async healthCheck(): Promise<{ status: string; timestamp: string }> {
    return this.request('/health')
  }

  // Simulation endpoints
  async runSimulation(config: SimulationConfigDTO): Promise<ApiResponseSimulationResults> {
    return this.request('/simulation/run', {
      method: 'POST',
      body: JSON.stringify(config),
    })
  }

  async getSimulationStatus(): Promise<ApiResponseMapStringObject> {
    return this.request('/simulation/status')
  }

  async getSimulationResults(): Promise<ApiResponseSimulationResults> {
    return this.request('/simulation/results')
  }

  async cancelSimulation(): Promise<{ success: boolean; message: string }> {
    return this.request('/simulation/cancel', {
      method: 'POST',
    })
  }

  // Comparison endpoints
  async runComparison(config: ComparisonConfigDTO): Promise<ApiResponseMapStringObject> {
    return this.request('/comparison/run', {
      method: 'POST',
      body: JSON.stringify(config),
    })
  }

  async getComparisonStatus(): Promise<{ running: boolean; timestamp: string }> {
    return this.request('/comparison/status')
  }

  // Scenario endpoints
  async getScenarios(): Promise<ApiResponseListScenarioListItem> {
    return this.request('/scenarios/')
  }

  async getScenario(scenarioId: string): Promise<ApiResponseMapStringObject> {
    return this.request(`/scenarios/${scenarioId}`)
  }

  async createScenario(config: ScenarioConfig): Promise<ApiResponseScenarioCreated> {
    return this.request('/scenarios/create', {
      method: 'POST',
      body: JSON.stringify(config),
    })
  }

  async createScenarioFromPreset(config: {
    scenario_name: string
    preset: string
    custom_scenario_id?: string
  }): Promise<ApiResponseScenarioCreated> {
    return this.request('/scenarios/create-preset', {
      method: 'POST',
      body: JSON.stringify(config),
    })
  }

  async deleteScenario(scenarioId: string): Promise<ApiResponseScenarioDeleted> {
    return this.request(`/scenarios/${scenarioId}`, {
      method: 'DELETE',
    })
  }

  async getScenarioStatistics(): Promise<ApiResponseMapStringObject> {
    return this.request('/scenarios/statistics')
  }

  // SKU endpoints
  async getSKUData(scenarioId: string): Promise<ApiResponseMapStringObject> {
    return this.request(`/scenarios/${scenarioId}/sku-data`)
  }

  async getOrderAnalysis(scenarioId: string): Promise<ApiResponseMapStringObject> {
    return this.request(`/scenarios/${scenarioId}/order-analysis`)
  }
}

// Export singleton instance
export const apiService = new ApiService()
export default apiService
```

### Step 2: Check if warehouseMapService.ts exists and needs updates

**Command:**
```bash
cat apps/frontend/src/services/warehouseMapService.ts 2>/dev/null || echo "File not found"
```

**Action:** If file exists and uses manual types, update similarly to api.ts

### Step 3: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck
```

**Expected:** Errors from components still using old imports - will fix next

### Step 4: Commit

```bash
git add apps/frontend/src/services/
git commit -m "refactor(services): use generated OpenAPI types

Replace manual type imports with generated types.
All API methods now use types from OpenAPI spec.
"
```

---

## Task 4: Update WarehouseHeatmap Component

**Goal:** Update critical component with complex data access patterns.

**Files:**
- Modify: `apps/frontend/src/components/common/maps/WarehouseHeatmap.tsx`

### Step 1: Update imports

**Action:** Replace manual type imports with generated types

**Find:**
```typescript
import type { RobotAPI, SimulationResultAPI, StationAPI } from '../../../types/api'
```

**Replace with:**
```typescript
import type { Robot, SimulationResults, Station } from '../../../types/generated'
```

### Step 2: Update type annotations

**Find:**
```typescript
interface WarehouseHeatmapProps {
  results: SimulationResultAPI
  className?: string
}
```

**Replace with:**
```typescript
interface WarehouseHeatmapProps {
  results: SimulationResults
  className?: string
}
```

### Step 3: Update field access - inventory metrics (line 20)

**Find:**
```typescript
results.metrics?.inventory_metrics.initial_inventory_state.initial_inventory_rack_data
```

**Replace with:**
```typescript
results.metrics?.inventoryMetrics.initialInventoryState
```

**Note:** Check the actual structure in generated-api.ts - the nested structure may have changed

### Step 4: Update field access - map config (lines 23, 219-221)

**Find:**
```typescript
const cellSize = results.config.map_config.cellSizeMeters || 1.0
results.config.map_config.gridWidth
results.config.map_config.gridDepth
```

**Replace with:**
```typescript
const cellSize = results.config.mapConfig?.cellSizeMeters || 1.0
results.config.mapConfig?.gridWidth
results.config.mapConfig?.gridDepth
```

### Step 5: Update field access - station positions (lines 80-88)

**Find:**
```typescript
const processedStations: Station[] = simulationStations.map((station: StationAPI) => ({
  id: station.station_id?.toString() || '0',
  position: {
    x: (station.position?.grid_col || 0) * cellSize,
    y: (station.position?.grid_row || 0) * cellSize,
  },
  type: 'operator',
  active: true,
}))
```

**Replace with:**
```typescript
const processedStations: Station[] = simulationStations.map((station: Station) => ({
  id: station.stationId?.value?.toString() || '0',
  position: {
    x: (station.position?.position?.col || 0) * cellSize,
    y: (station.position?.position?.row || 0) * cellSize,
  },
  type: 'operator',
  active: true,
}))
```

### Step 6: Update field access - robot positions (lines 91-99)

**Find:**
```typescript
const processedRobots: Robot[] = simulationRobots.map((robot: RobotAPI, index: number) => ({
  id: robot.id?.toString() || index.toString(),
  position: {
    x: (robot.current_position?.col || 0) * cellSize,
    y: (robot.current_position?.row || 0) * cellSize,
  },
  state: robot.state !== 'idle' ? 'active' : 'idle',
  speed: robot.speed || 1.0,
}))
```

**Replace with:**
```typescript
const processedRobots: Robot[] = simulationRobots.map((robot: Robot, index: number) => ({
  id: robot.id?.toString() || index.toString(),
  position: {
    x: (robot.currentPosition?.col || 0) * cellSize,
    y: (robot.currentPosition?.row || 0) * cellSize,
  },
  state: robot.state !== 'idle' ? 'active' : 'idle',
  speed: robot.speed || 1.0,
}))
```

### Step 7: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -A 5 "WarehouseHeatmap"
```

**Expected:** No errors in WarehouseHeatmap.tsx

### Step 8: Commit

```bash
git add apps/frontend/src/components/common/maps/WarehouseHeatmap.tsx
git commit -m "refactor(heatmap): use generated types with camelCase

Update WarehouseHeatmap to use generated OpenAPI types.
Key changes:
- inventory_metrics -> inventoryMetrics
- map_config -> mapConfig
- station_id.value, position.position.col/row
- current_position -> currentPosition
"
```

---

## Task 5: Update SimulationResults Component

**Goal:** Update main results component with metrics access.

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

### Step 1: Update imports

**Find:**
```typescript
import type { SimulationResultAPI } from '../../../types/api'
```

**Replace with:**
```typescript
import type { SimulationResults } from '../../../types/generated'
```

### Step 2: Update type annotations

**Find:**
```typescript
interface SimulationResultsProps {
  results: SimulationResultAPI
  onClose: () => void
  onRunAnother: () => void
}
```

**Replace with:**
```typescript
interface SimulationResultsProps {
  results: SimulationResults
  onClose: () => void
  onRunAnother: () => void
}
```

### Step 3: Update field access - scenario ID (line 53)

**Find:**
```typescript
{config.scenario_id} • {results.timestamp} • Analysis and performance metrics
```

**Replace with:**
```typescript
{config.scenarioId} • {results.timestamp} • Analysis and performance metrics
```

### Step 4: Update field access - metrics (lines 101-102)

**Find:**
```typescript
Duration: {formatDuration(metrics.config.simulation_duration)} • Processed:{' '}
{formatNumber(metrics.task_metrics.total_tasks_completed)} picks • Efficiency:{' '}
```

**Replace with:**
```typescript
Duration: {formatDuration(metrics?.config?.simulationDuration || 0)} • Processed:{' '}
{formatNumber(metrics?.taskMetrics?.totalTasksCompleted || 0)} picks • Efficiency:{' '}
```

### Step 5: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -A 5 "SimulationResults"
```

**Expected:** No errors in SimulationResults.tsx

### Step 6: Commit

```bash
git add apps/frontend/src/pages/simulator/components/SimulationResults.tsx
git commit -m "refactor(results): use generated types with camelCase

Update SimulationResults to use generated OpenAPI types.
Key changes:
- scenario_id -> scenarioId
- simulation_duration -> simulationDuration
- task_metrics -> taskMetrics
"
```

---

## Task 6: Update ResultsOverview Component

**Goal:** Update overview component with extensive metrics display.

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ResultsOverview.tsx`

### Step 1: Read current file to identify all field accesses

**Command:**
```bash
cat apps/frontend/src/pages/simulator/components/ResultsOverview.tsx
```

### Step 2: Update imports

**Find:**
```typescript
import type { SimulationResultAPI } from '../../../types/api'
```

**Replace with:**
```typescript
import type { SimulationResults } from '../../../types/generated'
```

### Step 3: Update type annotations

**Find:**
```typescript
interface ResultsOverviewProps {
  results: SimulationResultAPI
  formatNumber: (num: number) => string
  formatDuration: (hours: number) => string
}
```

**Replace with:**
```typescript
interface ResultsOverviewProps {
  results: SimulationResults
  formatNumber: (num: number) => string
  formatDuration: (hours: number) => string
}
```

### Step 4: Update all snake_case field accesses

**Common patterns to replace throughout the file:**

```typescript
// Task metrics
metrics.task_metrics.total_tasks_completed -> metrics?.taskMetrics?.totalTasksCompleted
metrics.task_metrics.total_orders_completed -> metrics?.taskMetrics?.totalOrdersCompleted
metrics.task_metrics.average_order_completion_time -> metrics?.taskMetrics?.averageOrderCompletionTime
metrics.task_metrics.task_completion_rate -> metrics?.taskMetrics?.taskCompletionRate

// Operator metrics
metrics.operator_metrics.operator_utilization -> metrics?.operatorMetrics?.operatorUtilization
metrics.operator_metrics.total_picks_processed -> metrics?.operatorMetrics?.totalPicksProcessed
metrics.operator_metrics.operator_idle_time -> metrics?.operatorMetrics?.operatorIdleTime

// Robot metrics
metrics.robot_metrics.overall_robot_utilization -> metrics?.robotMetrics?.overallRobotUtilization
metrics.robot_metrics.per_robot_stats -> metrics?.robotMetrics?.perRobotStats

// Station metrics
metrics.station_metrics.overall_throughput -> metrics?.stationMetrics?.overallThroughput
metrics.station_metrics.average_station_utilization -> metrics?.stationMetrics?.averageStationUtilization

// Storage metrics
metrics.storage_space_metrics.picks_per_face -> metrics?.storageSpaceMetrics?.picksPerFace
metrics.storage_space_metrics.total_face_presentations -> metrics?.storageSpaceMetrics?.totalFacePresentations
```

**Action:** Use find-and-replace with regex or manual update for each occurrence

### Step 5: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -A 5 "ResultsOverview"
```

**Expected:** No errors in ResultsOverview.tsx

### Step 6: Commit

```bash
git add apps/frontend/src/pages/simulator/components/ResultsOverview.tsx
git commit -m "refactor(overview): use generated types with camelCase

Update ResultsOverview to use generated OpenAPI types.
Convert all metrics field access from snake_case to camelCase.
"
```

---

## Task 7: Update PickResults Component

**Goal:** Update pick metrics display component.

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/PickResults.tsx`

### Step 1: Read file to identify field accesses

**Command:**
```bash
cat apps/frontend/src/pages/simulator/components/PickResults.tsx
```

### Step 2: Update imports

**Find:**
```typescript
import type { SimulationResultAPI } from '../../../types/api'
```

**Replace with:**
```typescript
import type { SimulationResults } from '../../../types/generated'
```

### Step 3: Update type annotations

**Find:**
```typescript
interface PickResultsProps {
  results: SimulationResultAPI
  formatNumber: (num: number) => string
}
```

**Replace with:**
```typescript
interface PickResultsProps {
  results: SimulationResults
  formatNumber: (num: number) => string
}
```

### Step 4: Update field accesses

**Common patterns in PickResults:**

```typescript
// Storage space metrics (RTP)
metrics.storage_space_metrics.picks_per_face -> metrics?.storageSpaceMetrics?.picksPerFace
metrics.storage_space_metrics.total_face_presentations -> metrics?.storageSpaceMetrics?.totalFacePresentations
metrics.storage_space_metrics.picks_per_face_data -> metrics?.storageSpaceMetrics?.picksPerFaceData

// Storage space metrics (Relay)
metrics.storage_space_metrics.picks_per_tote -> metrics?.storageSpaceMetrics?.picksPerTote
metrics.storage_space_metrics.total_tote_deliveries -> metrics?.storageSpaceMetrics?.totalToteDeliveries
metrics.storage_space_metrics.picks_per_tote_data -> metrics?.storageSpaceMetrics?.picksPerToteData

// Task pool quality
metrics.task_pool_quality_metrics.task_pool_quality -> metrics?.taskPoolQualityMetrics?.taskPoolQuality
metrics.task_pool_quality_metrics.total_tpq_events -> metrics?.taskPoolQualityMetrics?.totalTpqEvents
metrics.task_pool_quality_metrics.task_pool_quality_data -> metrics?.taskPoolQualityMetrics?.taskPoolQualityData
```

### Step 5: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -A 5 "PickResults"
```

**Expected:** No errors in PickResults.tsx

### Step 6: Commit

```bash
git add apps/frontend/src/pages/simulator/components/PickResults.tsx
git commit -m "refactor(picks): use generated types with camelCase

Update PickResults to use generated OpenAPI types.
Convert storage space and task pool quality metrics to camelCase.
"
```

---

## Task 8: Update PutResults Component

**Goal:** Update put/storage metrics display component.

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/PutResults.tsx`

### Step 1: Read file to identify field accesses

**Command:**
```bash
cat apps/frontend/src/pages/simulator/components/PutResults.tsx
```

### Step 2: Update imports

**Find:**
```typescript
import type { SimulationResultAPI } from '../../../types/api'
```

**Replace with:**
```typescript
import type { SimulationResults } from '../../../types/generated'
```

### Step 3: Update type annotations and field accesses

**Similar pattern to PickResults - update all snake_case to camelCase**

Common patterns:
```typescript
metrics.storage_space_metrics.rack_utilizations -> metrics?.storageSpaceMetrics?.rackUtilizations
metrics.inventory_metrics.initial_inventory_state -> metrics?.inventoryMetrics?.initialInventoryState
```

### Step 4: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -A 5 "PutResults"
```

**Expected:** No errors in PutResults.tsx

### Step 5: Commit

```bash
git add apps/frontend/src/pages/simulator/components/PutResults.tsx
git commit -m "refactor(put): use generated types with camelCase

Update PutResults to use generated OpenAPI types.
Convert inventory and storage metrics to camelCase.
"
```

---

## Task 9: Update Remaining Components - Batch 1 (Configuration)

**Goal:** Update configuration-related components.

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx`
- Modify: `apps/frontend/src/components/common/forms/ConfigurationForm.tsx`
- Modify: `apps/frontend/src/components/common/forms/ConfigurationTable.tsx`

### Step 1: Check if files use API types

**Command:**
```bash
grep -l "from.*types/api" apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx apps/frontend/src/components/common/forms/Configuration*.tsx 2>/dev/null || echo "No imports found"
```

### Step 2: For each file that imports from types/api

**Action:** Update imports and field accesses similar to previous tasks

**Pattern:**
- Replace imports: `'../types/api'` -> `'../types/generated'`
- Update type names: `SimulationConfigAPI` -> `SimulationConfig` or `SimulationConfigDTO`
- Update field access: snake_case -> camelCase

### Step 3: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -E "(ConfigurationPanel|ConfigurationForm|ConfigurationTable)"
```

**Expected:** No errors in these files

### Step 4: Commit

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx apps/frontend/src/components/common/forms/Configuration*.tsx
git commit -m "refactor(config): use generated types with camelCase

Update configuration components to use generated OpenAPI types.
"
```

---

## Task 10: Update Remaining Components - Batch 2 (Pages)

**Goal:** Update top-level page components.

**Files:**
- Modify: `apps/frontend/src/pages/SimulatorPage.tsx`
- Modify: `apps/frontend/src/pages/ScenariosPage.tsx`

### Step 1: Check for API type usage

**Command:**
```bash
grep -n "from.*types/api" apps/frontend/src/pages/*.tsx 2>/dev/null || echo "No imports found"
```

### Step 2: Update imports and field accesses

**Action:** Similar pattern - update imports and camelCase conversion

### Step 3: Verify compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck 2>&1 | grep -E "(SimulatorPage|ScenariosPage)"
```

**Expected:** No errors in page files

### Step 4: Commit

```bash
git add apps/frontend/src/pages/*.tsx
git commit -m "refactor(pages): use generated types with camelCase

Update page components to use generated OpenAPI types.
"
```

---

## Task 11: Update Remaining Components - Batch 3 (Other Components)

**Goal:** Update any remaining components with API type imports.

**Files:**
- All remaining .tsx files that import from types/api

### Step 1: Find remaining files with old imports

**Command:**
```bash
grep -r "from.*types/api" apps/frontend/src --include="*.tsx" --include="*.ts" -l 2>/dev/null || echo "All updated!"
```

### Step 2: Update each remaining file

**Action:** For each file found, apply the update pattern

### Step 3: Verify no old imports remain

**Command:**
```bash
grep -r "from.*types/api" apps/frontend/src --include="*.tsx" --include="*.ts" 2>/dev/null && echo "ERROR: Old imports still exist!" || echo "SUCCESS: All imports updated"
```

**Expected:** "SUCCESS: All imports updated"

### Step 4: Commit

```bash
git add apps/frontend/src/
git commit -m "refactor(components): update remaining components to use generated types

Update all remaining components to use generated OpenAPI types.
"
```

---

## Task 12: Delete Old Manual Types

**Goal:** Remove obsolete `types/api.ts` file.

**Files:**
- Delete: `apps/frontend/src/types/api.ts`

### Step 1: Verify no remaining imports

**Command:**
```bash
grep -r "from.*types/api" apps/frontend/src --include="*.tsx" --include="*.ts" 2>/dev/null && echo "ERROR: Still have imports!" || echo "OK: Safe to delete"
```

**Expected:** "OK: Safe to delete"

### Step 2: Delete the file

**Command:**
```bash
rm apps/frontend/src/types/api.ts
```

### Step 3: Verify TypeScript compilation

**Command:**
```bash
cd apps/frontend
npm run typecheck
```

**Expected:** Zero TypeScript errors

### Step 4: Commit

```bash
git add apps/frontend/src/types/api.ts
git commit -m "refactor(types): delete obsolete manual API types

Remove manual snake_case API types - replaced by generated OpenAPI types.
All components now use generated types from types/generated.ts.
"
```

---

## Task 13: Full Compilation Check

**Goal:** Verify entire frontend compiles without errors.

### Step 1: Clean build

**Command:**
```bash
cd apps/frontend
rm -rf node_modules/.cache
npm run typecheck
```

**Expected:** No TypeScript errors

**If errors:** Review error messages and fix remaining field name mismatches

### Step 2: Try production build

**Command:**
```bash
cd apps/frontend
npm run build
```

**Expected:** Successful build with no errors

### Step 3: Commit if any fixes were needed

```bash
git add apps/frontend/src/
git commit -m "fix(types): resolve remaining TypeScript compilation errors

Final fixes for type migration compilation issues.
"
```

---

## Task 14: Runtime Testing - Start Services

**Goal:** Verify the migration works at runtime with actual API calls.

### Step 1: Start Java backend

**Command:**
```bash
# In terminal 1
cd apps/backend
mvn spring-boot:run
```

**Expected:** Server starts on port 8080

**Verify:**
```bash
curl http://localhost:8080/health
```

Should return: `{"status":"UP","timestamp":"..."}`

### Step 2: Start frontend dev server

**Command:**
```bash
# In terminal 2
cd apps/frontend
npm run dev
```

**Expected:** Dev server starts on port 4200

### Step 3: Open browser and check console

**Action:**
1. Open http://localhost:4200
2. Open browser DevTools Console (F12)
3. Check for errors

**Expected:** No TypeScript or runtime errors in console

---

## Task 15: Runtime Testing - Simulation Flow

**Goal:** Test complete simulation workflow end-to-end.

### Step 1: Run a simulation

**Actions:**
1. Navigate to Simulator page
2. Select test scenario (e.g., "small-test" or "simple_scenario_v2")
3. Configure:
   - System Type: RTP
   - Robots: 4
   - Duration: 2 hours
4. Click "Run Simulation"

**Expected:**
- No console errors during submission
- Loading indicator appears
- Results display after completion

### Step 2: Verify results display - Overview tab

**Check:**
- ✅ Task metrics display correctly
- ✅ Operator utilization shows number
- ✅ Robot utilization shows number
- ✅ Station throughput shows number
- ✅ No "undefined" values

### Step 3: Verify results display - Put tab

**Check:**
- ✅ Storage space metrics display
- ✅ Rack utilizations show (if RTP)
- ✅ Inventory data displays
- ✅ Charts render without errors

### Step 4: Verify results display - Picks tab

**Check:**
- ✅ Picks per face displays (if RTP)
- ✅ Task pool quality displays
- ✅ Timeline charts render
- ✅ No data access errors

### Step 5: Verify warehouse visualization

**Actions:**
1. Open warehouse heatmap view
2. Switch between heatmap modes (popularity, unique SKUs, quantity)

**Expected:**
- ✅ Map renders with racks
- ✅ Stations display at correct positions
- ✅ Robots display at correct positions
- ✅ Heatmap colors update when switching modes
- ✅ No console errors

### Step 6: Document any issues found

**If issues found:**
- Note the specific field access that failed
- Check generated-api.ts for actual structure
- Fix field access in component
- Commit fix

---

## Task 16: Runtime Testing - Scenario Management

**Goal:** Test scenario management features.

### Step 1: List scenarios

**Action:** Navigate to Scenarios page

**Expected:**
- ✅ Scenarios list loads
- ✅ Each scenario shows: name, description, created date
- ✅ No console errors

### Step 2: View scenario details

**Action:** Click on a scenario to view details

**Expected:**
- ✅ Scenario info displays
- ✅ SKU data tab loads
- ✅ Order analysis tab loads
- ✅ All data displays correctly with camelCase fields

### Step 3: Create new scenario (if applicable)

**Action:** Create a test scenario

**Expected:**
- ✅ Form submits successfully
- ✅ New scenario appears in list
- ✅ No API errors

### Step 4: Delete test scenario

**Action:** Delete the test scenario created above

**Expected:**
- ✅ Deletion succeeds
- ✅ Scenario removed from list
- ✅ No errors

---

## Task 17: Runtime Testing - Comparison Analysis

**Goal:** Test comparison feature if applicable.

### Step 1: Run comparison

**Action:**
1. Navigate to comparison page (if exists)
2. Configure comparison parameters
3. Run comparison

**Expected:**
- ✅ Comparison runs successfully
- ✅ Results display
- ✅ Timeline data renders
- ✅ No console errors

### Step 2: Verify comparison data displays correctly

**Check:**
- ✅ All metrics use camelCase fields
- ✅ Charts render
- ✅ Summary statistics display

---

## Task 18: Final Verification and Cleanup

**Goal:** Ensure migration is complete and clean.

### Step 1: Verify no old type imports remain

**Command:**
```bash
grep -r "from.*types/api" apps/frontend/src --include="*.tsx" --include="*.ts"
```

**Expected:** No results (command returns nothing)

### Step 2: Verify types/api.ts is deleted

**Command:**
```bash
ls apps/frontend/src/types/api.ts 2>/dev/null && echo "ERROR: File still exists!" || echo "OK: File deleted"
```

**Expected:** "OK: File deleted"

### Step 3: Full TypeScript check

**Command:**
```bash
cd apps/frontend
npm run typecheck
```

**Expected:** Zero errors

### Step 4: Verify all success criteria from design doc

**Checklist:**
- ✅ Zero TypeScript compilation errors
- ✅ All components import from `types/generated`
- ✅ `types/api.ts` deleted
- ✅ Frontend connects to backend successfully (port 8080)
- ✅ Simulation runs and displays results in all tabs
- ✅ All field accesses use camelCase
- ✅ Warehouse map and heatmap render correctly
- ✅ Scenario management works
- ✅ No runtime errors in console
- ✅ IDE autocomplete works for generated types

### Step 5: Final commit

```bash
git status
# Review all changes
git add -A
git commit -m "feat(types): complete migration to generated OpenAPI types

Migration complete - all components now use generated types.

Breaking Changes:
- All API types now use camelCase field names (was snake_case)
- Removed manual type definitions from types/api.ts
- All components updated to use generated OpenAPI types

Benefits:
- Compile-time type safety between frontend and backend
- No type drift - OpenAPI spec is single source of truth
- Better IDE support with inline API documentation
- Automatic field validation from backend schema

Testing:
- All TypeScript compilation passes
- Runtime testing with Java backend verified
- Simulation, scenarios, and comparison features tested
- No console errors or undefined field access

Related: #<issue-number>
"
```

---

## Task 19: Update CLAUDE.md Documentation

**Goal:** Document the completed migration in project docs.

**Files:**
- Modify: `CLAUDE.md`

### Step 1: Add migration completion note

**Action:** Add section to CLAUDE.md under "Breaking Changes - CamelCase Migration"

**Add after the existing camelCase migration section:**

```markdown
### Frontend Type Migration (2025-11-08) - COMPLETE

**Status:** ✅ Complete

The frontend has been fully migrated to use auto-generated OpenAPI types:

- **Before:** Manual type definitions in `types/api.ts` with snake_case fields
- **After:** Generated types from `types/generated-api.ts` with camelCase fields

**Type Import Pattern:**
```typescript
// Correct - use generated types
import type { SimulationResults, Robot, Station } from '../types/generated'

// DO NOT use - file deleted
import type { SimulationResultAPI } from '../types/api'  // ❌
```

**Field Access Pattern:**
```typescript
// Correct - camelCase
results.config.mapConfig.gridWidth
metrics.taskMetrics.totalTasksCompleted
station.stationId.value

// DO NOT use - old snake_case
results.config.map_config.grid_width  // ❌
metrics.task_metrics.total_tasks_completed  // ❌
station.station_id  // ❌
```

**Generating Updated Types:**

When backend API changes, regenerate frontend types:

```bash
# Start backend on port 8080
cd apps/backend
mvn spring-boot:run

# In another terminal, regenerate types
cd apps/frontend
curl http://localhost:8080/openapi.json -o /tmp/openapi.json
npx openapi-typescript /tmp/openapi.json -o src/types/generated-api.ts
```

**Key Files:**
- `apps/frontend/src/types/generated-api.ts` - Raw OpenAPI types (auto-generated)
- `apps/frontend/src/types/generated.ts` - Flattened exports for easy import
- `apps/frontend/src/types/simulation.ts` - UI-only types (charts, state)

**Related Docs:**
- Design: `docs/plans/2025-11-08-migrate-frontend-generated-types.md`
- Implementation: `docs/plans/2025-11-08-migrate-frontend-generated-types-impl.md`
```

### Step 2: Commit documentation

```bash
git add CLAUDE.md
git commit -m "docs: document completed frontend type migration

Add notes about OpenAPI type generation and usage patterns.
"
```

---

## Task 20: Create Pull Request / Merge

**Goal:** Merge the migration into main branch.

### Step 1: Push branch

**Command:**
```bash
git push origin feature/migrate-to-generated-types
```

### Step 2: Create pull request

**Title:** "feat: Migrate frontend to generated OpenAPI types"

**Description:**
```markdown
## Overview

Migrates frontend from manual snake_case API types to auto-generated camelCase types from OpenAPI specification.

## Changes

- Created `types/generated.ts` with flattened OpenAPI type exports
- Updated all components (~50+ files) to use generated types
- Converted all field access from snake_case to camelCase
- Deleted obsolete `types/api.ts`
- Updated service layer to use generated types

## Breaking Changes

All API types now use camelCase field names:
- `config.map_config` → `config.mapConfig`
- `metrics.task_metrics` → `metrics.taskMetrics`
- `station.station_id` → `station.stationId.value`

## Benefits

- ✅ Compile-time type safety between frontend and backend
- ✅ No type drift - OpenAPI spec is single source of truth
- ✅ Better IDE support with inline API documentation
- ✅ Automatic field validation from backend schema

## Testing

- All TypeScript compilation passes (zero errors)
- Runtime tested with Java backend on port 8080
- Verified: simulation, scenarios, comparison features
- No console errors or undefined field access

## Related

- Design: `docs/plans/2025-11-08-migrate-frontend-generated-types.md`
- Implementation: `docs/plans/2025-11-08-migrate-frontend-generated-types-impl.md`
```

### Step 3: Merge when approved

**After review and approval:**
```bash
git checkout master
git merge feature/migrate-to-generated-types
git push origin master
```

---

## Completion Checklist

**Before considering this migration complete, verify:**

- ✅ All TypeScript compilation passes (`npm run typecheck`)
- ✅ Production build succeeds (`npm run build`)
- ✅ All components import from `types/generated` (not `types/api`)
- ✅ `types/api.ts` deleted and no references remain
- ✅ Backend running on port 8080 successfully connects
- ✅ Simulation runs end-to-end successfully
- ✅ All result tabs display data correctly (Overview, Put, Picks)
- ✅ Warehouse heatmap renders with all modes working
- ✅ Scenario management works (list, view, create, delete)
- ✅ Comparison analysis works (if applicable)
- ✅ No console errors during runtime testing
- ✅ No "undefined" field access warnings
- ✅ IDE autocomplete works for all generated types
- ✅ CLAUDE.md updated with migration notes
- ✅ All code committed and pushed
- ✅ Pull request created and merged

---

## Troubleshooting

### Issue: TypeScript errors about missing fields

**Symptom:** `Property 'xyz' does not exist on type 'Foo'`

**Solution:**
1. Check the actual structure in `types/generated-api.ts`
2. The OpenAPI type may have a different structure than expected
3. Update field access to match actual generated type
4. Pay attention to nested structures (e.g., `station.position.position.row`)

### Issue: Runtime "undefined" errors

**Symptom:** Console shows `Cannot read property 'x' of undefined`

**Solution:**
1. Check if field name is correct (camelCase)
2. Add optional chaining: `obj?.field?.nested`
3. Verify backend is sending the expected data structure
4. Check OpenAPI spec matches actual API response

### Issue: ID fields not accessible

**Symptom:** Can't access `station.stationId` - it's an object

**Solution:** IDs are wrapped in objects with `value` property
- `station.stationId.value`
- `rack.id.value`
- This matches the backend's type-safe ID pattern

### Issue: Position fields nested unexpectedly

**Symptom:** `station.position.grid_row` doesn't exist

**Solution:** Structure changed to:
- `station.position.position.row`
- `station.position.position.col`
- Check `StationGridPosition` type in generated-api.ts

### Issue: Backend returns different structure than OpenAPI spec

**Symptom:** Runtime data doesn't match TypeScript types

**Solution:**
1. Regenerate OpenAPI types from latest backend
2. Verify backend API version matches frontend expectations
3. Check if backend has unreleased changes

---

## Post-Migration Maintenance

### Keeping Types in Sync

**When backend API changes:**

1. Start updated backend
2. Regenerate frontend types:
   ```bash
   cd apps/frontend
   curl http://localhost:8080/openapi.json -o /tmp/openapi.json
   npx openapi-typescript /tmp/openapi.json -o src/types/generated-api.ts
   ```
3. Run typecheck to find breaking changes:
   ```bash
   npm run typecheck
   ```
4. Fix any TypeScript errors in components
5. Test runtime behavior
6. Commit updated types

### Adding to CI/CD

Consider adding type generation to pipeline:

```yaml
# .github/workflows/type-sync.yml
name: Sync OpenAPI Types
on:
  workflow_dispatch:
  schedule:
    - cron: '0 0 * * *'  # Daily

jobs:
  sync-types:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Start backend
        run: |
          cd apps/backend
          mvn spring-boot:run &
          sleep 30
      - name: Generate types
        run: |
          cd apps/frontend
          curl http://localhost:8080/openapi.json -o /tmp/openapi.json
          npx openapi-typescript /tmp/openapi.json -o src/types/generated-api.ts
      - name: Create PR if changes
        uses: peter-evans/create-pull-request@v5
        with:
          title: "chore: update OpenAPI types"
```
