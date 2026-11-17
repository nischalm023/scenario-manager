# CamelCase Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Migrate entire codebase from snake_case JSON serialization to camelCase to enable OpenAPI type generation and compile-time type safety between Java backend and TypeScript frontend.

**Architecture:** Remove custom Jackson configuration that forces snake_case output, remove explicit @JsonProperty annotations, update all frontend types and components to use camelCase field names, delete transformation layer.

**Tech Stack:** Java 21, Spring Boot 3.2, Jackson, TypeScript, React, OpenAPI

---

## Prerequisites

- Feature branch: `feature/camelcase-migration`
- Java backend at `apps/backend`
- Frontend at `apps/frontend`
- Backend server NOT running (will start for testing)

---

## Phase 1: Backend Changes

### Task 1: Delete Custom Jackson Configuration

**Files:**
- Delete: `apps/backend/src/main/java/com/greyorange/dessimulator/api/config/JacksonConfig.java`

**Step 1: Verify file exists**

```bash
ls -la apps/backend/src/main/java/com/greyorange/dessimulator/api/config/JacksonConfig.java
```

Expected: File exists (81 lines)

**Step 2: Delete the file**

```bash
rm apps/backend/src/main/java/com/greyorange/dessimulator/api/config/JacksonConfig.java
```

**Step 3: Verify deletion**

```bash
ls apps/backend/src/main/java/com/greyorange/dessimulator/api/config/JacksonConfig.java
```

Expected: File not found error

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/config/JacksonConfig.java
git commit -m "refactor: remove custom Jackson snake_case configuration

- Delete JacksonConfig.java with OutputOnlySnakeCaseStrategy
- Default Jackson behavior uses camelCase (industry standard)
- Enables OpenAPI type generation without case conversion

Breaking change: All JSON responses now use camelCase instead of snake_case"
```

---

### Task 2: Update WarehouseMapConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMapConfig.java:49-59`

**Step 1: Read current implementation**

```bash
cat apps/backend/src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMapConfig.java | grep -A 15 "@JsonCreator"
```

**Step 2: Remove @JsonCreator and @JsonProperty annotations**

Replace lines 49-59:

```java
// BEFORE
@JsonCreator
public WarehouseMapConfig(
    @JsonProperty("gridWidth") int gridWidth,
    @JsonProperty("gridDepth") int gridDepth,
    @JsonProperty("cellSizeMeters") double cellSizeMeters,
    @JsonProperty("useAdvancedLayout") boolean useAdvancedLayout,
    @JsonProperty("systemType") SystemType systemType,
    @JsonProperty("stationLayout") StationLayoutConfig stationLayout,
    @JsonProperty("highwayConfig") HighwayConfig highwayConfig,
    @JsonProperty("rackBlockConfig") RackBlockConfig rackBlockConfig,
    @JsonProperty("verticalToteCapacityInRack") Integer verticalToteCapacityInRack) {

// AFTER
public WarehouseMapConfig(
    int gridWidth,
    int gridDepth,
    double cellSizeMeters,
    boolean useAdvancedLayout,
    SystemType systemType,
    StationLayoutConfig stationLayout,
    HighwayConfig highwayConfig,
    RackBlockConfig rackBlockConfig,
    Integer verticalToteCapacityInRack) {
```

**Step 3: Remove unused Jackson imports**

Remove these import lines:

```java
import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonProperty;
```

**Step 4: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMapConfig.java
git commit -m "refactor: remove @JsonProperty from WarehouseMapConfig

- Remove @JsonCreator and 9 @JsonProperty annotations
- Jackson auto-maps camelCase JSON to constructor parameters
- Cleaner code, relies on Jackson conventions"
```

---

### Task 3: Update StationLayoutConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/map/StationLayoutConfig.java:31-34`

**Step 1: Remove @JsonCreator and @JsonProperty annotations**

Replace lines 31-34:

```java
// BEFORE
@JsonCreator
public static StationLayoutConfig fromJson(
    @JsonProperty("layoutType") StationLayoutType layoutType,
    @JsonProperty("stationEdges") List<StationEdgeConfig> stationEdges) {

// AFTER
public static StationLayoutConfig fromJson(
    StationLayoutType layoutType,
    List<StationEdgeConfig> stationEdges) {
```

**Step 2: Remove unused Jackson imports**

Remove these import lines if present:

```java
import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonProperty;
```

**Step 3: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/map/StationLayoutConfig.java
git commit -m "refactor: remove @JsonProperty from StationLayoutConfig

- Remove @JsonCreator and 2 @JsonProperty annotations
- Simplify JSON deserialization"
```

---

### Task 4: Update ScenarioModels

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/scenarios/ScenarioModels.java:18-76`

**Step 1: Update ScenarioInfo record**

Replace lines 18-35:

```java
// BEFORE
public record ScenarioInfo(
    String name,
    String description,
    @JsonProperty("created_date") String createdDate,
    @JsonProperty("item_catalog") ItemCatalogInfo itemCatalog,
    @JsonProperty("order_file") OrderFileInfo orderFile) {

// AFTER
public record ScenarioInfo(
    String name,
    String description,
    String createdDate,
    ItemCatalogInfo itemCatalog,
    OrderFileInfo orderFile) {
```

**Step 2: Update ItemCatalogInfo record**

Replace lines 38-56:

```java
// BEFORE
public record ItemCatalogInfo(
    String filename,
    @JsonProperty("total_skus") int totalSkus,
    @JsonProperty("popular_skus") int popularSkus,
    List<String> categories,
    @JsonProperty("popular_skus_percentage") double popularSkusPercentage,
    @JsonProperty("inventory_popular_quantity_share") double inventoryPopularQuantityShare) {

// AFTER
public record ItemCatalogInfo(
    String filename,
    int totalSkus,
    int popularSkus,
    List<String> categories,
    double popularSkusPercentage,
    double inventoryPopularQuantityShare) {
```

**Step 3: Update OrderFileInfo record**

Replace lines 60-75:

```java
// BEFORE
public record OrderFileInfo(
    String filename,
    @JsonProperty("total_orders") int totalOrders,
    @JsonProperty("total_picks") int totalPicks,
    @JsonProperty("order_popular_picks_share") double orderPopularPicksShare) {

// AFTER
public record OrderFileInfo(
    String filename,
    int totalOrders,
    int totalPicks,
    double orderPopularPicksShare) {
```

**Step 4: Remove unused Jackson import**

Remove this import line:

```java
import com.fasterxml.jackson.annotation.JsonProperty;
```

**Step 5: Verify compilation**

```bash
cd apps/backend
mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

**Step 6: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/scenarios/ScenarioModels.java
git commit -m "refactor: remove @JsonProperty from ScenarioModels

- Remove 9 @JsonProperty annotations from records
- Fields now serialize as camelCase (createdDate, itemCatalog, totalSkus, etc.)
- Breaking change: Scenarios API response format changes"
```

---

### Task 5: Verify Backend Compilation and Tests

**Files:**
- Test: All backend tests

**Step 1: Clean build**

```bash
cd apps/backend
mvn clean compile
```

Expected: BUILD SUCCESS

**Step 2: Run unit tests**

```bash
mvn test
```

Expected: All tests PASS (or identify failures to fix)

**Step 3: Start backend server in background**

```bash
mvn spring-boot:run > /tmp/backend-migration-test.log 2>&1 &
echo $! > /tmp/backend-pid.txt
```

**Step 4: Wait for startup and verify OpenAPI spec**

```bash
sleep 15
curl -s http://localhost:8080/openapi.json | jq '.components.schemas.SimulationConfigDTO.properties | keys' | head -20
```

Expected: Should see camelCase property names like `["binsCount", "duration", "enableJobShopScheduling", "gridDepth", "gridWidth", ...]`

**Step 5: Verify specific schema changes**

```bash
curl -s http://localhost:8080/openapi.json | jq '.components.schemas.WarehouseMapConfig.properties | keys'
```

Expected: `["cellSizeMeters", "gridDepth", "gridWidth", "highwayConfig", "rackBlockConfig", "stationLayout", "systemType", "useAdvancedLayout", "verticalToteCapacityInRack"]`

**Step 6: Stop backend server**

```bash
kill $(cat /tmp/backend-pid.txt)
rm /tmp/backend-pid.txt
```

**Step 7: Commit verification results**

```bash
git add -A
git commit -m "test: verify backend compiles and generates camelCase OpenAPI spec

- All unit tests pass
- OpenAPI spec now uses camelCase property names
- Backend ready for frontend migration" --allow-empty
```

---

## Phase 2: Frontend Changes

### Task 6: Update API Type Definitions

**Files:**
- Modify: `apps/frontend/src/types/api.ts:31-70`

**Step 1: Update StationEdgeConfigAPI**

Replace lines 31-35:

```typescript
// BEFORE
export interface StationEdgeConfigAPI {
  edge: 'bottom' | 'top' | 'left' | 'right'
  station_count: number
  spacing_grid_units: number
}

// AFTER
export interface StationEdgeConfigAPI {
  edge: 'bottom' | 'top' | 'left' | 'right'
  stationCount: number
  spacingGridUnits: number
}
```

**Step 2: Update StationLayoutConfigAPI**

Replace lines 37-49:

```typescript
// BEFORE
export interface StationLayoutConfigAPI {
  layout_type:
    | 'single_bottom'
    | 'single_top'
    | 'single_left'
    | 'single_right'
    | 'opposite_vertical'
    | 'opposite_horizontal'
    | 'four_edge'
    | 'custom'
  total_stations: number
  station_edges: StationEdgeConfigAPI[]
}

// AFTER
export interface StationLayoutConfigAPI {
  layoutType:
    | 'single_bottom'
    | 'single_top'
    | 'single_left'
    | 'single_right'
    | 'opposite_vertical'
    | 'opposite_horizontal'
    | 'four_edge'
    | 'custom'
  totalStations: number
  stationEdges: StationEdgeConfigAPI[]
}
```

**Step 3: Update HighwayConfigAPI**

Replace lines 51-53:

```typescript
// BEFORE
export interface HighwayConfigAPI {
  highways_per_edge: number
}

// AFTER
export interface HighwayConfigAPI {
  highwaysPerEdge: number
}
```

**Step 4: Update RackBlockConfigAPI**

Replace lines 55-59:

```typescript
// BEFORE
export interface RackBlockConfigAPI {
  rows_per_block: number
  rack_columns: number
  cross_aisle_width_grid_units: number
}

// AFTER
export interface RackBlockConfigAPI {
  rowsPerBlock: number
  rackColumns: number
  crossAisleWidthGridUnits: number
}
```

**Step 5: Update WarehouseMapConfigAPI**

Replace lines 61-70:

```typescript
// BEFORE
export interface WarehouseMapConfigAPI {
  grid_width: number
  grid_depth: number
  cell_size_meters: number
  use_advanced_layout: boolean
  station_layout: StationLayoutConfigAPI
  highway_config: HighwayConfigAPI
  rack_block_config: RackBlockConfigAPI
  vertical_tote_capacity_in_rack: number | null
}

// AFTER
export interface WarehouseMapConfigAPI {
  gridWidth: number
  gridDepth: number
  cellSizeMeters: number
  useAdvancedLayout: boolean
  stationLayout: StationLayoutConfigAPI
  highwayConfig: HighwayConfigAPI
  rackBlockConfig: RackBlockConfigAPI
  verticalToteCapacityInRack: number | null
}
```

**Step 6: Verify TypeScript compilation**

```bash
cd apps/frontend
npm run typecheck
```

Expected: Compilation errors (will fix in subsequent tasks)

**Step 7: Commit**

```bash
git add apps/frontend/src/types/api.ts
git commit -m "refactor: migrate API types to camelCase

- Update StationEdgeConfigAPI: station_count → stationCount
- Update WarehouseMapConfigAPI: grid_width → gridWidth, etc.
- Update nested config types to camelCase
- Matches backend OpenAPI schema changes"
```

---

### Task 7: Update Simulation Type Definitions

**Files:**
- Modify: `apps/frontend/src/types/simulation.ts:1-68`

**Step 1: Update StationEdgeConfig**

Replace lines 2-6:

```typescript
// BEFORE
export interface StationEdgeConfig {
  edge: 'bottom' | 'top' | 'left' | 'right'
  station_count: number
  spacing_grid_units: number
}

// AFTER
export interface StationEdgeConfig {
  edge: 'bottom' | 'top' | 'left' | 'right'
  stationCount: number
  spacingGridUnits: number
}
```

**Step 2: Update StationLayoutConfig**

Replace lines 8-19:

```typescript
// BEFORE
export interface StationLayoutConfig {
  layout_type:
    | 'single_bottom'
    | 'single_top'
    | 'single_left'
    | 'single_right'
    | 'opposite_vertical'
    | 'opposite_horizontal'
    | 'four_edge'
    | 'custom'
  station_edges?: StationEdgeConfig[]
}

// AFTER
export interface StationLayoutConfig {
  layoutType:
    | 'single_bottom'
    | 'single_top'
    | 'single_left'
    | 'single_right'
    | 'opposite_vertical'
    | 'opposite_horizontal'
    | 'four_edge'
    | 'custom'
  stationEdges?: StationEdgeConfig[]
}
```

**Step 3: Update HighwayConfig**

Replace lines 21-23:

```typescript
// BEFORE
export interface HighwayConfig {
  highways_per_edge: number
}

// AFTER
export interface HighwayConfig {
  highwaysPerEdge: number
}
```

**Step 4: Update RackBlockConfig**

Replace lines 25-29:

```typescript
// BEFORE
export interface RackBlockConfig {
  rows_per_block: number
  aisle_width_grid_units: number
  cross_aisle_width_grid_units: number
}

// AFTER
export interface RackBlockConfig {
  rowsPerBlock: number
  aisleWidthGridUnits: number
  crossAisleWidthGridUnits: number
}
```

**Step 5: Update MapConfig**

Replace lines 31-40:

```typescript
// BEFORE
export interface MapConfig {
  grid_width: number
  grid_depth: number
  cell_size_meters: number
  use_advanced_layout: boolean
  station_layout: StationLayoutConfig
  highway_config: HighwayConfig
  rack_block_config: RackBlockConfig
  vertical_tote_capacity_in_rack: number | null
}

// AFTER
export interface MapConfig {
  gridWidth: number
  gridDepth: number
  cellSizeMeters: number
  useAdvancedLayout: boolean
  stationLayout: StationLayoutConfig
  highwayConfig: HighwayConfig
  rackBlockConfig: RackBlockConfig
  verticalToteCapacityInRack: number | null
}
```

**Step 6: Commit**

```bash
git add apps/frontend/src/types/simulation.ts
git commit -m "refactor: migrate simulation types to camelCase

- Update all config interfaces to match API types
- Consistent camelCase naming throughout frontend
- Preparation for removing transformation layer"
```

---

### Task 8: Delete Transformation Layer

**Files:**
- Delete: `apps/frontend/src/services/transformers.ts`

**Step 1: Verify file exists**

```bash
wc -l apps/frontend/src/services/transformers.ts
```

Expected: 75 lines

**Step 2: Check for imports of this file**

```bash
grep -r "transformers" apps/frontend/src --include="*.ts" --include="*.tsx" -l
```

Expected: List of files importing transformers (will update next)

**Step 3: Delete the file**

```bash
rm apps/frontend/src/services/transformers.ts
```

**Step 4: Commit**

```bash
git add apps/frontend/src/services/transformers.ts
git commit -m "refactor: delete transformation layer

- Remove transformers.ts (76 lines eliminated)
- No longer needed - backend and frontend both use camelCase
- Eliminates manual case conversion logic"
```

---

### Task 9: Update API Service

**Files:**
- Modify: `apps/frontend/src/services/api.ts`

**Step 1: Remove transformer imports**

Find and remove these import lines:

```typescript
import { convertConfigToAPI, convertConfigToComparisonAPI } from './transformers'
```

**Step 2: Update runSimulation method**

Find the `runSimulation` method and remove transformation:

```typescript
// BEFORE
async runSimulation(config: SimulationRunRequestConfigAPI): Promise<SimulationResponse> {
  const apiConfig = convertConfigToAPI(config)
  const response = await fetch(...)

// AFTER
async runSimulation(config: SimulationRunRequestConfigAPI): Promise<SimulationResponse> {
  // No transformation needed - direct pass-through
  const response = await fetch(`${this.baseUrl}/simulation/run`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config)
  })
```

**Step 3: Update runComparison method**

Similar changes - remove transformation:

```typescript
// BEFORE
const apiConfig = convertConfigToComparisonAPI(config)

// AFTER
// Direct use of config
```

**Step 4: Verify TypeScript compilation**

```bash
cd apps/frontend
npm run typecheck
```

Expected: Still errors (will fix in component updates)

**Step 5: Commit**

```bash
git add apps/frontend/src/services/api.ts
git commit -m "refactor: remove transformation calls from API service

- Remove convertConfigToAPI and convertConfigToComparisonAPI
- Direct JSON serialization of camelCase config
- Simpler, more maintainable code"
```

---

### Task 10: Update WarehouseMapService

**Files:**
- Modify: `apps/frontend/src/services/warehouseMapService.ts:104-151`

**Step 1: Update convertToMapRequest function**

Replace lines 95-111:

```typescript
// BEFORE
export function convertToMapRequest(config: FrontendConfig): MapDataRequest {
  const mapConfig = config.mapConfig

  return {
    gridWidth: config.mapWidth || 20,
    gridDepth: config.mapLength || 25,
    cellSizeMeters: 1.0,
    stationCount: config.stationCount || 4,
    stationLayout: mapConfig?.station_layout?.layout_type || 'single_bottom',
    highwaysPerEdge: mapConfig?.highway_config?.highways_per_edge || 3,
    rowsPerBlock: mapConfig?.rack_block_config?.rows_per_block || 10,
    rackColumns: 2,
    crossAisleWidth: 1,
    useAdvancedLayout: mapConfig?.use_advanced_layout || false,
  }
}

// AFTER
export function convertToMapRequest(config: FrontendConfig): MapDataRequest {
  const mapConfig = config.mapConfig

  return {
    gridWidth: config.mapWidth || 20,
    gridDepth: config.mapLength || 25,
    cellSizeMeters: 1.0,
    stationCount: config.stationCount || 4,
    stationLayout: mapConfig?.stationLayout?.layoutType || 'single_bottom',
    highwaysPerEdge: mapConfig?.highwayConfig?.highwaysPerEdge || 3,
    rowsPerBlock: mapConfig?.rackBlockConfig?.rowsPerBlock || 10,
    rackColumns: 2,
    crossAisleWidth: 1,
    useAdvancedLayout: mapConfig?.useAdvancedLayout || false,
  }
}
```

**Step 2: Update convertToFrontendFormat function**

Replace lines 146-151:

```typescript
// BEFORE
const frontendMapConfig = {
  grid_width: config.gridWidth,
  grid_depth: config.gridDepth,
  cell_size_meters: config.cellSizeMeters,
}

// AFTER
const frontendMapConfig = {
  gridWidth: config.gridWidth,
  gridDepth: config.gridDepth,
  cellSizeMeters: config.cellSizeMeters,
}
```

**Step 3: Commit**

```bash
git add apps/frontend/src/services/warehouseMapService.ts
git commit -m "refactor: update warehouseMapService for camelCase

- Fix field access in convertToMapRequest
- Update frontendMapConfig to use camelCase
- Matches updated type definitions"
```

---

### Task 11: Update ConfigurationPanel Component

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx:106-151`

**Step 1: Update handleMapConfigChange - default MapConfig**

Replace lines 107-125:

```typescript
// BEFORE
const currentMapConfig = config.mapConfig || {
  grid_width: config.mapWidth || 20,
  grid_depth: config.mapLength || 25,
  cell_size_meters: 1.0,
  use_advanced_layout: true,
  station_layout: {
    layout_type: 'single_bottom',
    total_stations: config.stationCount || 4,
    spacing_grid_units: 5,
  },
  highway_config: {
    highways_per_edge: 3,
  },
  rack_block_config: {
    rows_per_block: 10,
    rack_columns: 2,
    cross_aisle_width_grid_units: 1,
  },
}

// AFTER
const currentMapConfig = config.mapConfig || {
  gridWidth: config.mapWidth || 20,
  gridDepth: config.mapLength || 25,
  cellSizeMeters: 1.0,
  useAdvancedLayout: true,
  stationLayout: {
    layoutType: 'single_bottom',
    totalStations: config.stationCount || 4,
    spacingGridUnits: 5,
  },
  highwayConfig: {
    highwaysPerEdge: 3,
  },
  rackBlockConfig: {
    rowsPerBlock: 10,
    rackColumns: 2,
    crossAisleWidthGridUnits: 1,
  },
}
```

**Step 2: Update legacy value sync logic**

Replace lines 148-157:

```typescript
// BEFORE
if (section === 'direct') {
  if (key === 'grid_width') {
    onConfigChange('mapWidth', value)
  } else if (key === 'grid_depth') {
    onConfigChange('mapLength', value)
  }
}

// AFTER
if (section === 'direct') {
  if (key === 'gridWidth') {
    onConfigChange('mapWidth', value)
  } else if (key === 'gridDepth') {
    onConfigChange('mapLength', value)
  }
}
```

**Step 3: Find and update all JSX field references**

Search for all occurrences of snake_case field access and update to camelCase:

```bash
grep -n "grid_width\|grid_depth\|cell_size\|station_layout\|layout_type" apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
```

Update each occurrence to camelCase equivalent.

**Step 4: Verify TypeScript compilation**

```bash
cd apps/frontend
npm run typecheck 2>&1 | grep ConfigurationPanel
```

Expected: No errors for this file

**Step 5: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
git commit -m "refactor: update ConfigurationPanel for camelCase

- Update MapConfig initialization with camelCase fields
- Fix legacy sync logic for gridWidth/gridDepth
- Update all field access to match type definitions"
```

---

### Task 12: Update ResultsOverview Component

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ResultsOverview.tsx`

**Step 1: Find snake_case field access**

```bash
grep -n "grid_width\|grid_depth\|station_layout\|rack_block" apps/frontend/src/pages/simulator/components/ResultsOverview.tsx
```

**Step 2: Update all field accesses to camelCase**

Replace any occurrences like:

```typescript
// BEFORE
config.mapConfig.grid_width
config.mapConfig.station_layout.layout_type

// AFTER
config.mapConfig.gridWidth
config.mapConfig.stationLayout.layoutType
```

**Step 3: Verify TypeScript compilation**

```bash
cd apps/frontend
npm run typecheck 2>&1 | grep ResultsOverview
```

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ResultsOverview.tsx
git commit -m "refactor: update ResultsOverview for camelCase

- Fix mapConfig field access
- Match updated type definitions"
```

---

### Task 13: Update WarehouseMap Component

**Files:**
- Modify: `apps/frontend/src/components/common/maps/WarehouseMap.tsx`

**Step 1: Find snake_case references**

```bash
grep -n "grid_width\|grid_depth\|cell_size" apps/frontend/src/components/common/maps/WarehouseMap.tsx
```

**Step 2: Update all references to camelCase**

```typescript
// Example updates
mapConfig.grid_width → mapConfig.gridWidth
mapConfig.grid_depth → mapConfig.gridDepth
mapConfig.cell_size_meters → mapConfig.cellSizeMeters
```

**Step 3: Verify compilation**

```bash
cd apps/frontend
npm run typecheck 2>&1 | grep WarehouseMap
```

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/components/common/maps/WarehouseMap.tsx
git commit -m "refactor: update WarehouseMap for camelCase

- Fix mapConfig property access
- Ensure map rendering uses correct field names"
```

---

### Task 14: Update WarehouseHeatmap Component

**Files:**
- Modify: `apps/frontend/src/components/common/maps/WarehouseHeatmap.tsx`

**Step 1: Find and update snake_case references**

```bash
grep -n "grid_width\|grid_depth" apps/frontend/src/components/common/maps/WarehouseHeatmap.tsx
```

**Step 2: Update to camelCase**

Similar pattern to WarehouseMap component.

**Step 3: Verify compilation**

```bash
cd apps/frontend
npm run typecheck 2>&1 | grep WarehouseHeatmap
```

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/components/common/maps/WarehouseHeatmap.tsx
git commit -m "refactor: update WarehouseHeatmap for camelCase

- Update field access to camelCase
- Consistent with type definitions"
```

---

### Task 15: Update SimulatorPage

**Files:**
- Modify: `apps/frontend/src/pages/SimulatorPage.tsx`

**Step 1: Find mapConfig initialization and usage**

```bash
grep -n "mapConfig\|grid_width\|station_layout" apps/frontend/src/pages/SimulatorPage.tsx
```

**Step 2: Update config initialization**

Update any default config or mapConfig initialization to use camelCase fields.

**Step 3: Verify compilation**

```bash
cd apps/frontend
npm run typecheck 2>&1 | grep SimulatorPage
```

Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/pages/SimulatorPage.tsx
git commit -m "refactor: update SimulatorPage for camelCase

- Fix config initialization
- Update mapConfig usage"
```

---

### Task 16: Final Frontend Verification

**Files:**
- All frontend TypeScript files

**Step 1: Full TypeScript compilation check**

```bash
cd apps/frontend
npm run typecheck
```

Expected: No errors

**Step 2: Run linting**

```bash
npm run lint
```

Expected: No critical errors (warnings acceptable)

**Step 3: Attempt build**

```bash
npm run build
```

Expected: Build SUCCESS

**Step 4: Commit**

```bash
git add -A
git commit -m "test: verify frontend compiles with camelCase types

- All TypeScript checks pass
- Build completes successfully
- Ready for integration testing" --allow-empty
```

---

## Phase 3: Integration Testing

### Task 17: End-to-End API Test

**Files:**
- Test: Full API integration

**Step 1: Start backend server**

```bash
cd apps/backend
mvn spring-boot:run > /tmp/e2e-backend.log 2>&1 &
echo $! > /tmp/e2e-backend-pid.txt
```

**Step 2: Wait for startup**

```bash
sleep 15
curl http://localhost:8080/health
```

Expected: `{"status":"healthy",...}`

**Step 3: Test simulation endpoint with camelCase request**

```bash
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "rtp",
    "duration": 1,
    "scenario": "scenario_a",
    "mapConfig": {
      "gridWidth": 20,
      "gridDepth": 25,
      "cellSizeMeters": 1.0,
      "useAdvancedLayout": true,
      "stationLayout": {
        "layoutType": "single_bottom",
        "totalStations": 4
      },
      "highwayConfig": {
        "highwaysPerEdge": 3
      },
      "rackBlockConfig": {
        "rowsPerBlock": 10,
        "rackColumns": 2,
        "crossAisleWidthGridUnits": 1
      }
    },
    "rtpRobotCount": 4,
    "rtpRobotSpeed": 1.5,
    "operatorTime": 12.0,
    "picksPerFace": 3.0,
    "binsCount": 8,
    "orderSelectionAlgorithm": "maximize_ppf_directly",
    "putAlgorithm": "greedy"
  }' | jq . > /tmp/simulation-response.json
```

**Step 4: Verify response uses camelCase**

```bash
jq '.data.config.mapConfig | keys' /tmp/simulation-response.json
```

Expected: `["cellSizeMeters", "gridDepth", "gridWidth", "highwayConfig", ...]` (all camelCase)

**Step 5: Test scenarios endpoint**

```bash
curl http://localhost:8080/scenarios/ | jq '.[0]' > /tmp/scenario-response.json
cat /tmp/scenario-response.json
```

Expected: Fields like `createdDate`, `itemCatalog`, `orderFile` (camelCase)

**Step 6: Stop backend**

```bash
kill $(cat /tmp/e2e-backend-pid.txt)
rm /tmp/e2e-backend-pid.txt
```

**Step 7: Commit test results**

```bash
git add /tmp/simulation-response.json /tmp/scenario-response.json
git commit -m "test: verify API returns camelCase responses

- Simulation endpoint works with camelCase request/response
- Scenarios endpoint returns camelCase fields
- Integration test PASS" --allow-empty
```

---

### Task 18: Frontend Integration Test

**Files:**
- Test: Full frontend application

**Step 1: Start backend (if not running)**

```bash
cd apps/backend
mvn spring-boot:run > /tmp/frontend-test-backend.log 2>&1 &
echo $! > /tmp/frontend-test-pid.txt
sleep 15
```

**Step 2: Start frontend dev server**

```bash
cd apps/frontend
npm run dev > /tmp/frontend-dev.log 2>&1 &
echo $! > /tmp/frontend-dev-pid.txt
sleep 10
```

**Step 3: Verify frontend loads**

```bash
curl http://localhost:4200 -I
```

Expected: HTTP 200 OK

**Step 4: Manual testing checklist**

Open browser to http://localhost:4200

- [ ] Configuration panel loads
- [ ] Map configuration section displays
- [ ] Can change gridWidth slider
- [ ] Can select station layout
- [ ] Run simulation button works
- [ ] Simulation executes successfully
- [ ] Results display correctly
- [ ] Warehouse map renders
- [ ] Metrics charts show data
- [ ] No console errors

**Step 5: Stop servers**

```bash
kill $(cat /tmp/frontend-dev-pid.txt)
kill $(cat /tmp/frontend-test-pid.txt)
rm /tmp/frontend-dev-pid.txt /tmp/frontend-test-pid.txt
```

**Step 6: Document test results**

```bash
git add -A
git commit -m "test: manual frontend integration test PASS

- Configuration panel works with camelCase
- Simulation runs successfully
- Results display correctly
- No runtime errors
- Migration complete and verified" --allow-empty
```

---

## Phase 4: Finalization

### Task 19: Update Documentation

**Files:**
- Modify: `apps/backend/README.md` or `CLAUDE.md`

**Step 1: Document breaking changes**

Add section to relevant documentation:

```markdown
## Breaking Changes - CamelCase Migration (2025-11-08)

**All JSON APIs now use camelCase instead of snake_case**

### Changed Fields:
- `grid_width` → `gridWidth`
- `grid_depth` → `gridDepth`
- `cell_size_meters` → `cellSizeMeters`
- `use_advanced_layout` → `useAdvancedLayout`
- `station_layout` → `stationLayout`
- `layout_type` → `layoutType`
- `station_edges` → `stationEdges`
- `station_count` → `stationCount`
- `spacing_grid_units` → `spacingGridUnits`
- `highway_config` → `highwayConfig`
- `highways_per_edge` → `highwaysPerEdge`
- `rack_block_config` → `rackBlockConfig`
- `rows_per_block` → `rowsPerBlock`
- `rack_columns` → `rackColumns`
- `cross_aisle_width_grid_units` → `crossAisleWidthGridUnits`
- `vertical_tote_capacity_in_rack` → `verticalToteCapacityInRack`
- `created_date` → `createdDate`
- `item_catalog` → `itemCatalog`
- `order_file` → `orderFile`
- `total_skus` → `totalSkus`
- `popular_skus` → `popularSkus`
- `total_orders` → `totalOrders`
- `total_picks` → `totalPicks`

### Migration:
- Update all API clients to use camelCase field names
- No backward compatibility with snake_case
- OpenAPI spec available at `/openapi.json`
```

**Step 2: Commit documentation**

```bash
git add CLAUDE.md README.md
git commit -m "docs: document camelCase migration breaking changes

- List all renamed fields
- Provide migration guidance
- Reference OpenAPI spec for type definitions"
```

---

### Task 20: Final Review and Merge

**Files:**
- All changes in feature branch

**Step 1: Review all commits**

```bash
git log --oneline master..HEAD
```

Expected: ~20 commits with clear messages

**Step 2: Create summary of changes**

```bash
echo "# CamelCase Migration Summary

## Backend Changes (4 files)
- Deleted: JacksonConfig.java (80 lines)
- Updated: WarehouseMapConfig.java (removed 9 @JsonProperty)
- Updated: StationLayoutConfig.java (removed 2 @JsonProperty)
- Updated: ScenarioModels.java (removed 9 @JsonProperty)

## Frontend Changes (9 files)
- Deleted: transformers.ts (76 lines)
- Updated: types/api.ts (~50 field renames)
- Updated: types/simulation.ts (~30 field renames)
- Updated: ConfigurationPanel.tsx (~50 lines)
- Updated: warehouseMapService.ts (~30 lines)
- Updated: api.ts (removed transformation calls)
- Updated: ResultsOverview.tsx (~20 lines)
- Updated: WarehouseMap.tsx (~15 lines)
- Updated: WarehouseHeatmap.tsx (~10 lines)
- Updated: SimulatorPage.tsx (~20 lines)

## Total Impact
- Files deleted: 2
- Files modified: 11
- Lines removed: ~156
- Lines changed: ~300
- Breaking changes: All API endpoints

## Benefits
✅ Eliminated 176 lines of transformation code
✅ Enabled OpenAPI code generation
✅ Compile-time type safety between backend and frontend
✅ Consistent camelCase naming (JSON standard)
✅ Simpler codebase, faster development" > /tmp/migration-summary.md

cat /tmp/migration-summary.md
```

**Step 3: Run final verification**

```bash
# Backend
cd apps/backend
mvn clean test

# Frontend
cd apps/frontend
npm run typecheck
npm run build
```

Expected: All PASS

**Step 4: Push feature branch**

```bash
git push -u origin feature/camelcase-migration
```

**Step 5: Create pull request**

```bash
gh pr create \
  --title "Migration: snake_case → camelCase for OpenAPI type generation" \
  --body-file /tmp/migration-summary.md \
  --base master \
  --head feature/camelcase-migration
```

**Step 6: Document completion**

```bash
git add -A
git commit -m "chore: complete camelCase migration

- All backend and frontend changes complete
- Tests passing
- Documentation updated
- Ready for code review and merge

See PR for detailed summary of changes" --allow-empty
```

---

## Success Criteria

**Backend:**
- ✅ JacksonConfig.java deleted
- ✅ All @JsonProperty annotations removed
- ✅ Compilation succeeds
- ✅ Unit tests pass
- ✅ OpenAPI spec uses camelCase

**Frontend:**
- ✅ transformers.ts deleted
- ✅ All type definitions use camelCase
- ✅ No transformation logic in services
- ✅ TypeScript compilation succeeds
- ✅ Build succeeds
- ✅ All components updated

**Integration:**
- ✅ Backend accepts camelCase requests
- ✅ Backend returns camelCase responses
- ✅ Frontend sends camelCase requests
- ✅ Frontend receives and parses camelCase responses
- ✅ Simulation runs end-to-end
- ✅ All UI features work

**Quality:**
- ✅ All tests passing
- ✅ No console errors
- ✅ Documentation updated
- ✅ Git history clean with clear commits

---

## Rollback Plan

If issues discovered after merge:

**Step 1: Revert merge commit**

```bash
git revert -m 1 <merge-commit-sha>
```

**Step 2: Deploy reverted version**

**Step 3: Investigate issues in feature branch**

**Step 4: Fix and re-merge when ready**

---

## Estimated Time

- Backend changes: 30-45 minutes
- Frontend changes: 60-90 minutes
- Integration testing: 30-45 minutes
- Documentation: 15-30 minutes
- **Total: 2.5-4 hours**

---

## Notes

- This is a breaking change - coordinate deployment
- No backward compatibility with snake_case
- OpenAPI spec is now the source of truth for types
- Future: Set up automated OpenAPI type generation
