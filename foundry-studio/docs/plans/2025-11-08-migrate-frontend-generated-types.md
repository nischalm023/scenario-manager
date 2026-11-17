# Frontend Migration to Generated OpenAPI Types

**Date:** 2025-11-08
**Status:** Design Complete - Ready for Implementation
**Author:** Claude + kumar.d

## Overview

Migrate the frontend from manually maintained snake_case API types to auto-generated camelCase types from the OpenAPI specification. This establishes compile-time type safety between the Java backend and React frontend, eliminating type drift and runtime errors.

## Background

### Current State
- Backend (Java): Sends JSON with camelCase fields after 2025-11-08 camelCase migration
- Frontend types (`types/api.ts`): Manual snake_case type definitions (outdated)
- Generated types (`types/generated-api.ts`): OpenAPI-generated camelCase types (unused)

### Problem
Type mismatch between backend responses and frontend expectations:
- Backend sends: `{ gridWidth: 100, mapConfig: {...} }`
- Frontend expects: `{ grid_width: 100, map_config: {...} }`
- TypeScript cannot catch these mismatches at compile time
- Runtime errors and undefined field access

### Solution
Replace all manual types with OpenAPI-generated types throughout the frontend codebase (~52 TypeScript files).

## Design Decisions

### 1. Migration Strategy: Big Bang (Option A)
- **Chosen:** Replace all manual types at once in coordinated effort
- **Rationale:** Clean break, no confusion, TypeScript catches all mismatches
- **Rejected:** Gradual migration (confusion), Wrapper layer (defeats purpose)

### 2. Type Consumption: Direct Import with Flattened Exports (Option A)
- **Chosen:** Create `types/generated.ts` with flattened re-exports for easy imports
- **Pattern:**
  ```typescript
  export type SimulationConfigDTO = components["schemas"]["SimulationConfigDTO"]
  // Usage:
  import type { SimulationConfigDTO } from '../types/generated'
  ```
- **Benefits:** Full IDE autocomplete, inline documentation, clean imports
- **Rejected:** Namespace imports (verbose), Domain re-exports (unnecessary layer)

### 3. Type Transformations: Pure Migration (Option A)
- **Chosen:** Update field names only, no behavioral changes
- **Rationale:** TypeScript will catch all mismatches, backend already validated
- **Rejected:** Runtime validation (unnecessary dependency), Migration + tests (overkill)

### 4. Union Types: Use As-Is (Option A)
- **Chosen:** Use TypeScript's discriminated unions directly from generated types
- **Example:**
  ```typescript
  if (config.robotConfig.configType === "RtpRobotConfig") {
    // TypeScript narrows type automatically
  }
  ```
- **Rejected:** Type guards (premature), Both (unnecessary complexity)

### 5. Domain Types: Hybrid Approach (Option C)
- **Chosen:** Generated types for API data, custom types for UI-specific structures
- **API types:** Use generated (SimulationResults, Robot, Station, etc.)
- **UI types:** Keep custom (ChartDataPoint, HeatmapData, UIState)
- **Rejected:** Replace entirely (loses UI flexibility), Keep separate (duplicate work)

### 6. Import Strategy: Explicit Imports (Option B)
- **Chosen:** Import directly from specific type files
- **Pattern:**
  ```typescript
  import type { SimulationConfigDTO } from '../types/generated'
  import type { ChartDataPoint } from '../types/simulation'
  ```
- **Benefits:** Clear provenance, better tree-shaking
- **Rejected:** Barrel exports (unclear origin), Mixed (inconsistent)

## Architecture

### Type System Structure

**After Migration:**
```
types/
├── generated-api.ts    → Raw OpenAPI types (untouched, auto-generated)
├── generated.ts        → NEW: Flattened exports for easy import
├── simulation.ts       → UI-only types (ChartDataPoint, HeatmapData)
├── scenarios.ts        → UI-only types (if any)
└── index.ts            → Minimal barrel export for UI types only
```

**Deleted:**
```
types/api.ts            → DELETE: Replaced by generated types
```

### Core Principles
1. **API types = Generated from OpenAPI** (single source of truth)
2. **UI types = Hand-written** (only for frontend-specific structures)
3. **No mapping layer** - components use API types directly
4. **Explicit imports** - clear type provenance

## Implementation Plan

### Step 1: Create Flattened Type Exports

**File:** `apps/frontend/src/types/generated.ts`

```typescript
import type { components, operations } from './generated-api'

// API Response wrapper
export type ApiResponse<T> = {
  success?: boolean
  data?: T
  error?: string
  timestamp?: string
}

// Configuration Types
export type SimulationConfigDTO = components["schemas"]["SimulationConfigDTO"]
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

// Domain Objects
export type SimulationResults = components["schemas"]["SimulationResults"]
export type SimulationConfig = components["schemas"]["SimulationConfig"]
export type SimulationMetric = components["schemas"]["SimulationMetric"]
export type Robot = components["schemas"]["Robot"]
export type Rack = components["schemas"]["Rack"]
export type Station = components["schemas"]["Station"]
export type GridCoordinate = components["schemas"]["GridCoordinate"]
export type StationGridPosition = components["schemas"]["StationGridPosition"]
export type Item = components["schemas"]["Item"]
export type PickTask = components["schemas"]["PickTask"]

// Metrics
export type TaskMetric = components["schemas"]["TaskMetric"]
export type StorageSpaceMetric = components["schemas"]["StorageSpaceMetric"]
export type OperatorMetric = components["schemas"]["OperatorMetric"]
export type RobotMetric = components["schemas"]["RobotMetric"]
export type StationMetric = components["schemas"]["StationMetric"]
export type RelayPositionMetric = components["schemas"]["RelayPositionMetric"]
export type TaskPoolQualityMetric = components["schemas"]["TaskPoolQualityMetric"]
export type InventoryMetric = components["schemas"]["InventoryMetric"]

// API Responses
export type ApiResponseSimulationResults = components["schemas"]["ApiResponseSimulationResults"]
export type ApiResponseListScenarioListItem = components["schemas"]["ApiResponseListScenarioListItem"]
export type ApiResponseScenarioCreated = components["schemas"]["ApiResponseScenarioCreated"]
export type ApiResponseScenarioDeleted = components["schemas"]["ApiResponseScenarioDeleted"]
export type ApiResponseMapStringObject = components["schemas"]["ApiResponseMapStringObject"]

// Scenario Types
export type ScenarioListItem = components["schemas"]["ScenarioListItem"]
export type ScenarioConfig = components["schemas"]["ScenarioConfig"]
export type ScenarioCreated = components["schemas"]["ScenarioCreated"]
export type ScenarioDeleted = components["schemas"]["ScenarioDeleted"]

// Map Types
export type MapDataRequestDTO = components["schemas"]["MapDataRequestDTO"]
export type MapDataResponse = components["schemas"]["MapDataResponse"]
export type CompactMapData = components["schemas"]["CompactMapData"]
export type MapConfiguration = components["schemas"]["MapConfiguration"]
export type Position = components["schemas"]["Position"]
export type StationElement = components["schemas"]["StationElement"]
export type RackElement = components["schemas"]["RackElement"]
export type HighwayElement = components["schemas"]["HighwayElement"]

// Comparison Types
export type ComparisonConfigDTO = components["schemas"]["ComparisonConfigDTO"]

// ID Types
export type RackId = components["schemas"]["RackId"]
export type StationId = components["schemas"]["StationId"]
export type StorageSpaceId = components["schemas"]["StorageSpaceId"]
```

### Step 2: Refactor UI-Only Type Files

**File:** `apps/frontend/src/types/simulation.ts`

```typescript
// types/simulation.ts - UI-only types after migration

// Chart and visualization types (not in API)
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

// UI state types (frontend-only)
export interface UIState {
  currentPage: 'simulator' | 'scenarios'
  simulationInProgress: boolean
  lastSimulationResults: any | null  // Will import SimulationResults from generated
  activeTab: string
  sidebarExpanded: boolean
}

export interface MockSimulationResponse {
  status: 'running' | 'completed' | 'error'
  progress: number
  results?: any  // Will import SimulationResults from generated
  error?: string
}
```

**File:** `apps/frontend/src/types/scenarios.ts`

Review and keep only UI-specific types. If `SKUData` and `OrderAnalysisData` come from the API, delete them. If they're frontend-only transformations, keep them.

**File:** `apps/frontend/src/types/index.ts`

```typescript
// types/index.ts - Minimal barrel export for UI types only

// UI-specific types only
export type {
  ChartDataPoint,
  HeatmapData,
  UIState,
  MockSimulationResponse,
} from './simulation'

// Export scenarios types if any remain after cleanup
export type {
  SKUData,
  OrderAnalysisData,
} from './scenarios'

// Note: API types should be imported directly from './generated'
// This promotes explicit imports and better tree-shaking
```

### Step 3: Delete Old Manual Types

```bash
rm apps/frontend/src/types/api.ts
```

### Step 4: Update Service Layer

**File:** `apps/frontend/src/services/api.ts`

```typescript
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

  // Comparison endpoints
  async runComparison(config: ComparisonConfigDTO): Promise<ApiResponseMapStringObject> {
    return this.request('/comparison/run', {
      method: 'POST',
      body: JSON.stringify(config),
    })
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

  async deleteScenario(scenarioId: string): Promise<ApiResponseScenarioDeleted> {
    return this.request(`/scenarios/${scenarioId}`, {
      method: 'DELETE',
    })
  }

  async getScenarioStatistics(): Promise<ApiResponseMapStringObject> {
    return this.request('/scenarios/statistics')
  }

  async getSKUData(scenarioId: string): Promise<ApiResponseMapStringObject> {
    return this.request(`/scenarios/${scenarioId}/sku-data`)
  }

  async getOrderAnalysis(scenarioId: string): Promise<ApiResponseMapStringObject> {
    return this.request(`/scenarios/${scenarioId}/order-analysis`)
  }
}

export const apiService = new ApiService()
export default apiService
```

**Key Changes:**
- Import from `'../types/generated'` instead of `'../types/api'`
- Use generated response types (`ApiResponseSimulationResults`, etc.)
- Method signatures use `SimulationConfigDTO` instead of `SimulationRunRequestConfigAPI`

**File:** `apps/frontend/src/services/warehouseMapService.ts`

Update similarly - replace any manual type imports with generated types.

### Step 5: Update Components - Field Access Patterns

Update all ~52 TypeScript component files with these field name changes:

#### Config Fields
```typescript
// BEFORE (snake_case)
config.map_config          → config.mapConfig
config.scenario_id         → config.scenarioId
config.simulation_duration → config.simulationDuration
config.robot_config        → config.robotConfig
config.operator_config     → config.operatorConfig
config.algorithm_config    → config.algorithmConfig
```

#### Map Config Fields
```typescript
// BEFORE
mapConfig.grid_width           → mapConfig.gridWidth
mapConfig.grid_depth           → mapConfig.gridDepth
mapConfig.cell_size_meters     → mapConfig.cellSizeMeters
mapConfig.use_advanced_layout  → mapConfig.useAdvancedLayout
mapConfig.station_layout       → mapConfig.stationLayout
mapConfig.highway_config       → mapConfig.highwayConfig
mapConfig.rack_block_config    → mapConfig.rackBlockConfig
```

#### Station Fields
```typescript
// BEFORE
station.station_id              → station.stationId.value  // ID wrapped in object
station.position.grid_row       → station.position.position.row  // nested structure
station.position.grid_col       → station.position.position.col
station.total_picks_processed   → station.totalPicksProcessed
station.total_processing_time   → station.totalProcessingTime
station.queue_length            → station.queueLength
station.max_queue_length        → station.maxQueueLength
station.total_tasks_completed   → station.totalTasksCompleted
station.operator_busy_time      → station.operatorBusyTime
station.operator_idle_time      → station.operatorIdleTime
station.racks_processed         → station.racksProcessed
station.orders_completed        → station.ordersCompleted
station.last_activity_time      → station.lastActivityTime
station.average_time_per_pick   → station.averageTimePerPick
```

#### Robot Fields
```typescript
// BEFORE
robot.current_position          → robot.currentPosition
robot.current_position.row      → robot.currentPosition.row
robot.current_position.col      → robot.currentPosition.col
robot.tasks_completed           → robot.tasksCompleted
robot.total_working_time        → robot.totalWorkingTime
robot.total_idle_time           → robot.totalIdleTime
robot.total_distance_traveled   → robot.totalDistanceTraveled
robot.robot_type                → robot.robotType
robot.zone_assignment           → robot.zoneAssignment
robot.carrying_cargo            → robot.carryingCargo
```

#### Rack Fields
```typescript
// BEFORE
rack.storage_position    → rack.storageCoordinate
rack.current_position    → rack.currentCoordinate
rack.assigned_robot      → rack.assignedRobotId
rack.max_capacity        → rack.maxCapacity
rack.max_tote_capacity   → rack.maxToteCapacity
rack.tasks_by_station    → rack.tasksByStation
rack.being_transported   → rack.beingTransported
rack.next_task           → rack.nextTask
```

#### Metrics Fields
```typescript
// BEFORE
metrics.task_metrics                → metrics.taskMetrics
metrics.storage_space_metrics       → metrics.storageSpaceMetrics
metrics.operator_metrics            → metrics.operatorMetrics
metrics.robot_metrics               → metrics.robotMetrics
metrics.station_metrics             → metrics.stationMetrics
metrics.inventory_metrics           → metrics.inventoryMetrics
metrics.relay_position_metrics      → metrics.relayPositionMetrics
metrics.task_pool_quality_metrics   → metrics.taskPoolQualityMetrics
```

#### Task Metrics Fields
```typescript
// BEFORE
taskMetrics.total_tasks_completed         → taskMetrics.totalTasksCompleted
taskMetrics.total_orders_completed        → taskMetrics.totalOrdersCompleted
taskMetrics.task_arrival_rate             → taskMetrics.taskArrivalRate
taskMetrics.task_completion_rate          → taskMetrics.taskCompletionRate
taskMetrics.average_order_completion_time → taskMetrics.averageOrderCompletionTime
```

#### Storage Space Metrics Fields
```typescript
// BEFORE
storageSpaceMetrics.rack_utilizations        → storageSpaceMetrics.rackUtilizations
storageSpaceMetrics.picks_per_face           → storageSpaceMetrics.picksPerFace
storageSpaceMetrics.total_face_presentations → storageSpaceMetrics.totalFacePresentations
storageSpaceMetrics.picks_per_face_data      → storageSpaceMetrics.picksPerFaceData
storageSpaceMetrics.tote_utilizations        → storageSpaceMetrics.toteUtilizations
storageSpaceMetrics.picks_per_tote           → storageSpaceMetrics.picksPerTote
storageSpaceMetrics.total_tote_deliveries    → storageSpaceMetrics.totalToteDeliveries
storageSpaceMetrics.picks_per_tote_data      → storageSpaceMetrics.picksPerToteData
```

#### Operator Metrics Fields
```typescript
// BEFORE
operatorMetrics.operator_utilization   → operatorMetrics.operatorUtilization
operatorMetrics.operator_idle_time     → operatorMetrics.operatorIdleTime
operatorMetrics.total_picks_processed  → operatorMetrics.totalPicksProcessed
operatorMetrics.total_processing_time  → operatorMetrics.totalProcessingTime
```

#### Robot Metrics Fields
```typescript
// BEFORE
robotMetrics.per_robot_stats           → robotMetrics.perRobotStats
robotMetrics.by_type_stats             → robotMetrics.byTypeStats
robotMetrics.overall_robot_utilization → robotMetrics.overallRobotUtilization
```

#### Station Metrics Fields
```typescript
// BEFORE
stationMetrics.per_station_stats            → stationMetrics.perStationStats
stationMetrics.overall_throughput           → stationMetrics.overallThroughput
stationMetrics.average_station_utilization  → stationMetrics.averageStationUtilization
```

#### Inventory Metrics Fields
```typescript
// BEFORE
inventoryMetrics.initial_inventory_state              → inventoryMetrics.initialInventoryState
inventoryMetrics.initial_inventory_state.initial_inventory_rack_data
  → inventoryMetrics.initialInventoryState (structure may differ - check generated types)
inventoryMetrics.inventory_changes_log                → inventoryMetrics.inventoryChangesLog
```

#### Relay Position Metrics Fields
```typescript
// BEFORE
relayPositionMetrics.per_relay_stats          → relayPositionMetrics.perRelayStats
relayPositionMetrics.total_blocking_events    → relayPositionMetrics.totalBlockingEvents
relayPositionMetrics.total_blocking_time      → relayPositionMetrics.totalBlockingTime
relayPositionMetrics.most_congested_relay_id  → relayPositionMetrics.mostCongestedRelayId
```

#### Task Pool Quality Metrics Fields
```typescript
// BEFORE
tpqMetrics.task_pool_quality       → tpqMetrics.taskPoolQuality
tpqMetrics.total_tpq_events        → tpqMetrics.totalTpqEvents
tpqMetrics.total_system_time       → tpqMetrics.totalSystemTime
tpqMetrics.total_operator_time     → tpqMetrics.totalOperatorTime
tpqMetrics.task_pool_quality_data  → tpqMetrics.taskPoolQualityData
```

### Step 6: Component Update Process

For each component file (~52 files):

1. **Update imports**
   ```typescript
   // BEFORE
   import type { SimulationResultAPI, RobotAPI, StationAPI } from '../../../types/api'

   // AFTER
   import type { SimulationResults, Robot, Station } from '../../../types/generated'
   ```

2. **Update type annotations in interfaces/props**
   ```typescript
   // BEFORE
   interface Props {
     results: SimulationResultAPI
     stations: StationAPI[]
   }

   // AFTER
   interface Props {
     results: SimulationResults
     stations: Station[]
   }
   ```

3. **Update field access throughout the component**
   - Use the field mapping patterns above
   - Pay special attention to nested structures
   - Handle ID type changes (now wrapped in objects with `.value`)

4. **Run TypeScript compiler after each batch**
   ```bash
   npm run typecheck
   ```

5. **Fix any remaining TypeScript errors**

### Step 7: Critical Files Requiring Special Attention

These files have complex data access patterns and need careful review:

#### 1. `apps/frontend/src/components/common/maps/WarehouseHeatmap.tsx`

**Lines 20-23:** Inventory metrics access
```typescript
// BEFORE
results.metrics?.inventory_metrics.initial_inventory_state.initial_inventory_rack_data

// AFTER - Check generated type structure
results.metrics?.inventoryMetrics.initialInventoryState
// Note: Structure may have changed - verify in generated-api.ts
```

**Lines 23, 219-221:** Map config access
```typescript
// BEFORE
results.config.map_config.cellSizeMeters
results.config.map_config.gridWidth
results.config.map_config.gridDepth

// AFTER
results.config.mapConfig.cellSizeMeters
results.config.mapConfig.gridWidth
results.config.mapConfig.gridDepth
```

**Lines 80-98:** Station and robot position access
```typescript
// BEFORE
station.station_id?.toString()
station.position?.grid_col
station.position?.grid_row
robot.current_position?.col
robot.current_position?.row

// AFTER
station.stationId?.value?.toString()  // ID wrapped in object
station.position?.position?.col  // Nested structure
station.position?.position?.row
robot.currentPosition?.col
robot.currentPosition?.row
```

#### 2. `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

**Line 53:** Scenario ID access
```typescript
// BEFORE
config.scenario_id

// AFTER
config.scenarioId
```

**Lines 101-102:** Metrics access
```typescript
// BEFORE
metrics.config.simulation_duration
metrics.task_metrics.total_tasks_completed

// AFTER
metrics.config.simulationDuration
metrics.taskMetrics.totalTasksCompleted
```

#### 3. Result Display Components

**`PickResults.tsx`, `PutResults.tsx`, `ResultsOverview.tsx`:**
- Heavy metrics field access throughout
- Update all snake_case to camelCase
- Pay attention to nested metric objects

#### 4. `apps/frontend/src/services/warehouseMapService.ts`

- Check if this file uses any manual types
- Update to use generated types from `types/generated`

## Execution Order

Execute in this specific order to minimize breakage:

1. ✅ **Create new files** (no breakage)
   - Create `types/generated.ts`

2. ✅ **Update type files** (no breakage yet)
   - Refactor `types/simulation.ts` (UI types only)
   - Refactor `types/scenarios.ts` (UI types only)
   - Update `types/index.ts` (minimal exports)

3. ✅ **Update service layer** (services now typed correctly)
   - Update `services/api.ts`
   - Update `services/warehouseMapService.ts`

4. ✅ **Update components bottom-up** (minimize cascading errors)
   - Start with leaf components (pure display)
   - Then container components
   - Finally pages and top-level components
   - Run `npm run typecheck` after each batch

5. ✅ **Clean up** (after everything migrates)
   - Delete `types/api.ts`
   - Verify no imports remain from old file

6. ✅ **Test** (verification)
   - Runtime testing with backend
   - Verify all user flows work

## Testing Strategy

### Compile-Time Validation (Primary)

```bash
# After each component batch update
npm run typecheck

# Should show zero TypeScript errors when complete
```

### Runtime Testing (Verification)

1. **Start services:**
   ```bash
   # Terminal 1: Java backend
   nx serve backend

   # Terminal 2: Frontend
   cd apps/frontend
   npm run dev
   ```

2. **Test critical user flows:**
   - ✅ Run a simulation with test scenario
   - ✅ Verify results display correctly in all tabs (Overview, Put, Picks)
   - ✅ Check warehouse map renders with correct data
   - ✅ Test warehouse heatmap visualization
   - ✅ Verify metrics display (tasks, robots, stations)
   - ✅ Test scenario management (list, create, delete)
   - ✅ Run comparison analysis
   - ✅ Check SKU data display
   - ✅ Check order analysis display

3. **Browser console:**
   - ✅ No TypeScript/runtime errors
   - ✅ No "undefined" field access warnings
   - ✅ API responses parse correctly

### Test Scenarios

**Scenario 1: Basic Simulation**
- Navigate to simulator page
- Select test scenario
- Configure simulation (RTP system, 4 robots, 2 hours)
- Run simulation
- Verify results display in all tabs
- Check all metrics render correctly

**Scenario 2: Warehouse Visualization**
- Run simulation from Scenario 1
- Navigate to warehouse map view
- Switch between heatmap modes (popularity, unique SKUs, quantity)
- Verify rack colors update correctly
- Check station and robot positions display

**Scenario 3: Scenario Management**
- Navigate to scenarios page
- List all scenarios
- Create new scenario
- View scenario details
- Check SKU data loads
- Check order analysis loads
- Delete test scenario

**Scenario 4: Comparison Analysis**
- Run comparison between two configurations
- Verify comparison results display
- Check timeline data renders

## Rollback Plan

### Git Safety

Work on feature branch for easy rollback:

```bash
git checkout -b feature/migrate-to-generated-types
```

### Atomic Commits

Commit after each logical group:

```bash
# Commit 1
git add apps/frontend/src/types/generated.ts
git commit -m "Add flattened OpenAPI type exports"

# Commit 2
git add apps/frontend/src/types/simulation.ts
git add apps/frontend/src/types/scenarios.ts
git add apps/frontend/src/types/index.ts
git commit -m "Refactor type files - UI types only"

# Commit 3
git add apps/frontend/src/services/
git commit -m "Update service layer to use generated types"

# Commit 4-N (component batches)
git add apps/frontend/src/components/common/maps/
git commit -m "Update warehouse map components to use generated types"

git add apps/frontend/src/pages/simulator/components/
git commit -m "Update simulator result components to use generated types"

# ... more batches as needed

# Final commit
git rm apps/frontend/src/types/api.ts
git add apps/frontend/src/types/
git commit -m "Delete old manual API types"
```

### Rollback Process

If critical issues arise:

```bash
# Revert specific commit
git revert <commit-hash>

# Or rollback entire branch
git checkout master
git branch -D feature/migrate-to-generated-types
```

## Success Criteria

Migration is complete when:

- ✅ Zero TypeScript compilation errors (`npm run typecheck` passes)
- ✅ All components import from `types/generated` (not `types/api`)
- ✅ `types/api.ts` deleted and no references remain
- ✅ Frontend connects to backend successfully (port 8080)
- ✅ Simulation runs and displays results correctly in all tabs
- ✅ All field accesses use camelCase (no snake_case)
- ✅ Warehouse map and heatmap render correctly
- ✅ Scenario management works (list, create, view, delete)
- ✅ Comparison analysis works
- ✅ No runtime errors in browser console
- ✅ No "undefined" field access warnings
- ✅ IDE autocomplete works for all generated types

## Benefits After Migration

1. **Type Safety:** Compile-time errors catch backend/frontend mismatches
2. **No Type Drift:** OpenAPI spec is single source of truth
3. **Better DX:** IDE autocomplete with inline documentation
4. **Less Maintenance:** No manual type updates needed
5. **Documentation:** OpenAPI comments appear in IDE tooltips
6. **Confidence:** TypeScript guarantees correct field access

## Risks and Mitigations

### Risk: Breaking Existing Functionality
**Mitigation:**
- Big bang approach ensures TypeScript catches all issues
- Comprehensive testing before merging
- Feature branch for easy rollback

### Risk: Missing Field Name Changes
**Mitigation:**
- TypeScript compiler will catch all mismatches
- No compilation = no missed changes

### Risk: Nested Structure Changes
**Mitigation:**
- Document known structure changes (Station.position, ID wrappers)
- Manual review of complex components
- Runtime testing to verify

### Risk: Development Time
**Mitigation:**
- Clear field mapping patterns for efficiency
- Bottom-up component updates minimize cascading errors
- Atomic commits for incremental progress

## Future Improvements

### Automated Type Generation

Add to CI/CD pipeline:

```bash
# In package.json
"scripts": {
  "generate-types": "curl http://localhost:8080/openapi.json -o /tmp/openapi.json && npx openapi-typescript /tmp/openapi.json -o src/types/generated-api.ts"
}
```

Run after backend changes to keep types in sync.

### Runtime Validation (Optional)

If needed later, add Zod for runtime validation:

```typescript
import { z } from 'zod'

const SimulationResultsSchema = z.object({
  success: z.boolean(),
  data: z.object({
    // ... schema matching generated types
  })
})

// Validate API responses
const validated = SimulationResultsSchema.parse(apiResponse)
```

## References

- **OpenAPI Spec:** `http://localhost:8080/openapi.json`
- **Generated Types:** `apps/frontend/src/types/generated-api.ts`
- **CamelCase Migration:** `docs/plans/2025-11-08-camelcase-migration.md`
- **CLAUDE.md Section:** "Breaking Changes - CamelCase Migration (2025-11-08)"

## Appendix: Component Files to Update

Total: ~52 TypeScript files in `apps/frontend/src/`

**Services:** (2 files)
- `services/api.ts`
- `services/warehouseMapService.ts`

**Components:** (~50 files)
- All files in `components/`
- All files in `pages/`
- Use `find apps/frontend/src -name "*.tsx" -o -name "*.ts"` to get full list

Focus on files that import from `types/api` first (use grep to find them).
