# Order Algorithm Comparison Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable users to compare any 2 order selection algorithms side-by-side using a modal-based selection interface, reusing the existing comparison infrastructure.

**Architecture:** Extend ComparisonService to support `comparisonType: "order"` with user-selected algorithms. Add modal-based algorithm selection for both put and order comparisons. Reuse existing comparison state, loading, and results components.

**Tech Stack:** Java 21, Spring Boot, Lombok, shadcn/ui, React, TypeScript, OpenAPI code generation

---

## Task 1: Backend - Update ComparisonConfigDTO

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonConfigDTO.java` (if exists as separate file)
- OR Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java` (if DTO is nested)

**Step 1: Locate ComparisonConfigDTO**

Run: `grep -r "record ComparisonConfigDTO" apps/backend/src/`

Expected: Find file location of DTO definition

**Step 2: Add algorithm selection fields to DTO**

Add two new fields to the existing record:

```java
public record ComparisonConfigDTO(
    String comparisonType,     // "put" or "order"
    String algorithm1,         // NEW: First algorithm to compare
    String algorithm2,         // NEW: Second algorithm to compare

    // Existing fields remain unchanged
    String systemType,
    Double duration,
    String scenario,
    WarehouseMapConfig mapConfig,
    // ... all other existing fields
) {
  // Compact constructor with validation (add to existing validation)
  public ComparisonConfigDTO {
    if (algorithm1 != null && algorithm2 != null && algorithm1.equals(algorithm2)) {
      throw new IllegalArgumentException("algorithm1 and algorithm2 must be different");
    }
  }
}
```

**Step 3: Compile to verify changes**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/
git commit -m "feat: add algorithm1 and algorithm2 fields to ComparisonConfigDTO"
```

---

## Task 2: Backend - Refactor getOperationModes to accept algorithm parameters

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java:88-111`

**Step 1: Update method signature**

Change from:
```java
private List<Map<String, Object>> getOperationModes(String comparisonType) {
```

To:
```java
private List<Map<String, Object>> getOperationModes(
    String comparisonType,
    String algorithm1,
    String algorithm2) {
```

**Step 2: Refactor put algorithm logic to use parameters**

Replace hardcoded put algorithms (lines 90-108) with:

```java
if ("put".equals(comparisonType)) {
  return List.of(
      buildPutModeInfo(algorithm1),
      buildPutModeInfo(algorithm2));
}
```

**Step 3: Add order algorithm support**

After the put logic, add:

```java
if ("order".equals(comparisonType)) {
  return List.of(
      buildOrderModeInfo(algorithm1),
      buildOrderModeInfo(algorithm2));
}

throw new IllegalArgumentException("Unknown comparison type: " + comparisonType);
```

**Step 4: Update runComparison call site**

Find the call to `getOperationModes()` in `runComparison()` method (around line 28) and update:

```java
// Change from:
List<Map<String, Object>> operationModes = getOperationModes(comparisonType);

// To:
List<Map<String, Object>> operationModes =
    getOperationModes(comparisonType, baseConfig.algorithm1, baseConfig.algorithm2);
```

**Step 5: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 6: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "refactor: make getOperationModes accept algorithm parameters"
```

---

## Task 3: Backend - Add metadata helper methods

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java`

**Step 1: Add buildPutModeInfo helper**

Add method at end of ComparisonService class:

```java
private Map<String, Object> buildPutModeInfo(String algorithm) {
  return switch (algorithm) {
    case "greedy" -> Map.of(
        "name", "Greedy Put",
        "key", algorithm,
        "config", Map.of("putAlgorithm", algorithm),
        "emoji", "🎯");
    case "min_weighted_cycle_distance" -> Map.of(
        "name", "Min Weighted Cycle Distance",
        "key", algorithm,
        "config", Map.of("putAlgorithm", algorithm),
        "emoji", "📐");
    default -> throw new IllegalArgumentException("Unknown put algorithm: " + algorithm);
  };
}
```

**Step 2: Add buildOrderModeInfo helper**

Add method after buildPutModeInfo:

```java
private Map<String, Object> buildOrderModeInfo(String algorithm) {
  return switch (algorithm) {
    case "maximize_ppf_directly" -> Map.of(
        "name", "Maximize PPF Directly",
        "key", algorithm,
        "config", Map.of("orderSelectionAlgorithm", algorithm),
        "emoji", "📊");
    case "maximize_ppf_with_tiering" -> Map.of(
        "name", "PPF with Tiering",
        "key", algorithm,
        "config", Map.of("orderSelectionAlgorithm", algorithm),
        "emoji", "🎚️");
    case "maximize_tpq_with_vut" -> Map.of(
        "name", "Maximize TPQ (VUT)",
        "key", algorithm,
        "config", Map.of("orderSelectionAlgorithm", algorithm),
        "emoji", "🎯");
    default -> throw new IllegalArgumentException("Unknown order algorithm: " + algorithm);
  };
}
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "feat: add metadata helpers for put and order algorithms"
```

---

## Task 4: Backend - Update applyModeConfig to handle order algorithms

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java:113-115`

**Step 1: Read current applyModeConfig implementation**

Run: `grep -A 5 "private SimulationConfig applyModeConfig" apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java`

Expected: See current implementation (likely returns baseConfig unchanged)

**Step 2: Replace with algorithm-aware implementation**

Replace the method:

```java
private SimulationConfig applyModeConfig(SimulationConfig baseConfig, Map<String, Object> mode) {
  @SuppressWarnings("unchecked")
  Map<String, String> configOverrides = (Map<String, String>) mode.get("config");

  if (configOverrides == null || configOverrides.isEmpty()) {
    return baseConfig;
  }

  // Handle orderSelectionAlgorithm override
  if (configOverrides.containsKey("orderSelectionAlgorithm")) {
    OrderSelectionAlgorithm algorithm =
        OrderSelectionAlgorithm.fromValue(configOverrides.get("orderSelectionAlgorithm"));

    AlgorithmConfig newAlgorithmConfig = baseConfig.algorithmConfig().toBuilder()
        .orderSelectionAlgorithm(algorithm)
        .build();

    return baseConfig.toBuilder()
        .algorithmConfig(newAlgorithmConfig)
        .build();
  }

  // Handle putAlgorithm override (if needed in future)
  if (configOverrides.containsKey("putAlgorithm")) {
    AlgorithmConfig newAlgorithmConfig = baseConfig.algorithmConfig().toBuilder()
        .putAlgorithm(configOverrides.get("putAlgorithm"))
        .build();

    return baseConfig.toBuilder()
        .algorithmConfig(newAlgorithmConfig)
        .build();
  }

  return baseConfig;
}
```

**Step 3: Add required imports**

Add at top of file if not present:

```java
import com.greyorange.dessimulator.domain.order.OrderSelectionAlgorithm;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.AlgorithmConfig;
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/ComparisonService.java
git commit -m "feat: implement applyModeConfig to override order selection algorithm"
```

---

## Task 5: Backend - Format and verify compilation

**Files:**
- Format: All modified Java files

**Step 1: Apply formatting**

Run: `cd apps/backend && mvn spotless:apply`

Expected: All files formatted successfully

**Step 2: Full compile with tests**

Run: `cd apps/backend && mvn clean compile`

Expected: BUILD SUCCESS

**Step 3: Commit formatting if changed**

```bash
git add apps/backend/
git commit -m "style: format ComparisonService and related files"
```

---

## Task 6: Frontend - Create algorithm metadata constants

**Files:**
- Create: `apps/frontend/src/constants/algorithms.ts`

**Step 1: Create constants file**

```typescript
import type { OrderSelectionAlgorithm } from '../types/generated'

/**
 * Display metadata for put algorithms.
 * Used in algorithm selection modal and comparison results.
 */
export const PUT_ALGORITHMS = [
  {
    value: 'greedy',
    label: 'Greedy Put',
    emoji: '🎯',
    description: 'Fast greedy placement algorithm'
  },
  {
    value: 'min_weighted_cycle_distance',
    label: 'Min Weighted Cycle Distance',
    emoji: '📐',
    description: 'Optimizes robot travel distance'
  }
] as const

/**
 * Display metadata for order selection algorithms.
 * Used in algorithm selection modal and comparison results.
 */
export const ORDER_ALGORITHMS = [
  {
    value: 'maximize_ppf_directly',
    label: 'Maximize PPF Directly',
    emoji: '📊',
    description: 'Greedy algorithm maximizing picks per face'
  },
  {
    value: 'maximize_ppf_with_tiering',
    label: 'PPF with Tiering',
    emoji: '🎚️',
    description: 'PPF optimization with rack reuse preference'
  },
  {
    value: 'maximize_tpq_with_vut',
    label: 'Maximize TPQ (VUT)',
    emoji: '🎯',
    description: 'Tasks per tote with variety, urgency, and tiering'
  }
] as const

/**
 * Default algorithm selections for comparisons.
 */
export const DEFAULT_PUT_COMPARISON = {
  algorithm1: 'greedy',
  algorithm2: 'min_weighted_cycle_distance'
} as const

export const DEFAULT_ORDER_COMPARISON = {
  algorithm1: 'maximize_ppf_directly',
  algorithm2: 'maximize_tpq_with_vut'
} as const

/**
 * Type helper for algorithm metadata.
 */
export type AlgorithmMetadata = {
  value: string
  label: string
  emoji: string
  description: string
}
```

**Step 2: Verify TypeScript compiles**

Run: `npm run typecheck`

Expected: No errors (may have unrelated warnings)

**Step 3: Commit**

```bash
git add apps/frontend/src/constants/algorithms.ts
git commit -m "feat: add algorithm display metadata constants"
```

---

## Task 7: Frontend - Create AlgorithmSelectionModal component

**Files:**
- Create: `apps/frontend/src/pages/simulator/components/AlgorithmSelectionModal.tsx`

**Step 1: Create modal component**

```typescript
import type React from 'react'
import { useState } from 'react'
import { Button } from '../../../components/ui/Button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../../../components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../../../components/ui/select'
import type { AlgorithmMetadata } from '../../../constants/algorithms'

interface AlgorithmSelectionModalProps {
  isOpen: boolean
  comparisonType: 'put' | 'order'
  algorithms: readonly AlgorithmMetadata[]
  defaultAlgorithm1?: string
  defaultAlgorithm2?: string
  onConfirm: (algorithm1: string, algorithm2: string) => void
  onClose: () => void
}

const AlgorithmSelectionModal: React.FC<AlgorithmSelectionModalProps> = ({
  isOpen,
  comparisonType,
  algorithms,
  defaultAlgorithm1,
  defaultAlgorithm2,
  onConfirm,
  onClose,
}) => {
  const [algorithm1, setAlgorithm1] = useState(defaultAlgorithm1 || algorithms[0]?.value || '')
  const [algorithm2, setAlgorithm2] = useState(defaultAlgorithm2 || algorithms[1]?.value || '')

  const canRun = algorithm1 && algorithm2 && algorithm1 !== algorithm2

  const handleConfirm = () => {
    if (canRun) {
      onConfirm(algorithm1, algorithm2)
    }
  }

  const comparisonTypeLabel = comparisonType === 'put' ? 'Put' : 'Order Selection'

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle className="text-xl font-semibold text-gray-900">
            Select {comparisonTypeLabel} Algorithms to Compare
          </DialogTitle>
          <p className="text-sm text-gray-600">
            Choose 2 different algorithms to compare side-by-side
          </p>
        </DialogHeader>

        <div className="space-y-6 py-4">
          {/* Algorithm 1 Selection */}
          <div className="space-y-2">
            <label className="text-sm font-medium text-gray-700">Algorithm 1</label>
            <Select value={algorithm1} onValueChange={setAlgorithm1}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Select first algorithm" />
              </SelectTrigger>
              <SelectContent>
                {algorithms.map((alg) => (
                  <SelectItem key={alg.value} value={alg.value}>
                    <div className="flex items-center gap-2">
                      <span>{alg.emoji}</span>
                      <div className="flex flex-col">
                        <span className="font-medium">{alg.label}</span>
                        <span className="text-xs text-gray-500">{alg.description}</span>
                      </div>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Algorithm 2 Selection */}
          <div className="space-y-2">
            <label className="text-sm font-medium text-gray-700">Algorithm 2</label>
            <Select value={algorithm2} onValueChange={setAlgorithm2}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Select second algorithm" />
              </SelectTrigger>
              <SelectContent>
                {algorithms.map((alg) => (
                  <SelectItem key={alg.value} value={alg.value}>
                    <div className="flex items-center gap-2">
                      <span>{alg.emoji}</span>
                      <div className="flex flex-col">
                        <span className="font-medium">{alg.label}</span>
                        <span className="text-xs text-gray-500">{alg.description}</span>
                      </div>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Validation Message */}
          {!canRun && algorithm1 === algorithm2 && (
            <div className="text-sm text-amber-600 bg-amber-50 border border-amber-200 rounded p-3">
              ⚠️ Please select 2 different algorithms to compare
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 pt-4 border-t border-gray-200">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            className="bg-orange-500 hover:bg-orange-600 text-white"
            onClick={handleConfirm}
            disabled={!canRun}
          >
            Run Comparison
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

export default AlgorithmSelectionModal
```

**Step 2: Verify TypeScript compiles**

Run: `npm run typecheck`

Expected: No errors in new file

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmSelectionModal.tsx
git commit -m "feat: create AlgorithmSelectionModal component"
```

---

## Task 8: Frontend - Update ConfigurationPanel with order comparison button

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx`

**Step 1: Update props interface**

Find the props interface and modify:

```typescript
interface ConfigurationPanelProps {
  config: SimulationConfig
  onConfigChange: (key: keyof SimulationConfig, value: any) => void
  onRunSimulation: () => void
  onCompareAlgorithms: () => void  // RENAME: was onCompareAlgorithms
  onCompareOrderAlgorithms: () => void  // NEW
}
```

**Step 2: Update component destructuring**

In component definition:

```typescript
const ConfigurationPanel: React.FC<ConfigurationPanelProps> = ({
  config,
  onConfigChange,
  onRunSimulation,
  onCompareAlgorithms,  // Rename to onComparePutAlgorithms for clarity
  onCompareOrderAlgorithms,  // NEW
})
```

**Step 3: Update existing button text**

Find the existing "Compare Put Algorithms" button (around line 512) and ensure it says "Compare Put Algorithms"

**Step 4: Add order comparison button**

After the put comparison button, add:

```typescript
<Button
  variant="outline"
  onClick={onCompareOrderAlgorithms}
  className="w-full"
>
  Compare Order Algorithms
</Button>
```

**Step 5: Verify TypeScript compiles**

Run: `npm run typecheck`

Expected: Errors about missing prop in SimulatorPage (will fix next)

**Step 6: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
git commit -m "feat: add Compare Order Algorithms button to ConfigurationPanel"
```

---

## Task 9: Frontend - Regenerate OpenAPI types

**Files:**
- Modify: `apps/frontend/src/types/generated-api.ts` (auto-generated)
- Modify: `apps/frontend/src/types/generated.ts` (manual aliases)

**Step 1: Start backend server**

Run in separate terminal:
```bash
npx nx serve backend
```

Wait for: "Started DessimulatorApplication" message

**Step 2: Regenerate TypeScript types**

Run:
```bash
npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts
```

Expected: File regenerated with new algorithm1 and algorithm2 fields

**Step 3: Verify new fields exist**

Run: `grep -A 10 "ComparisonConfigDTO" apps/frontend/src/types/generated-api.ts`

Expected: See algorithm1 and algorithm2 in the schema

**Step 4: Stop backend server**

Run: `lsof -ti:8080 | xargs kill -9`

**Step 5: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "chore: regenerate OpenAPI types with algorithm selection fields"
```

---

## Task 10: Frontend - Update SimulatorPage state and handlers

**Files:**
- Modify: `apps/frontend/src/pages/SimulatorPage.tsx`

**Step 1: Add imports**

Add to imports section:

```typescript
import AlgorithmSelectionModal from './simulator/components/AlgorithmSelectionModal'
import {
  ORDER_ALGORITHMS,
  PUT_ALGORITHMS,
  DEFAULT_ORDER_COMPARISON,
  DEFAULT_PUT_COMPARISON,
} from '../constants/algorithms'
```

**Step 2: Add modal state**

After existing state declarations (around line 32):

```typescript
// Algorithm selection modal state
const [algorithmModalOpen, setAlgorithmModalOpen] = useState(false)
const [pendingComparisonType, setPendingComparisonType] = useState<'put' | 'order' | null>(null)
```

**Step 3: Update handleRunComparison signature and implementation**

Replace the existing handleRunComparison function:

```typescript
const handleRunComparison = async (
  comparisonType: 'put' | 'order',
  algorithm1: string,
  algorithm2: string
) => {
  try {
    console.log(`🔄 ${comparisonType} algorithm comparison button clicked!`)

    // Close modal
    setAlgorithmModalOpen(false)
    setPendingComparisonType(null)

    // Set loading state
    setComparisonState('loading')
    setComparisonError(null)
    setComparisonStartTime(new Date())

    // Convert frontend config to comparison API format
    const comparisonConfig = {
      systemType: config.systemType,
      duration: config.simulationDuration,
      scenario: config.scenario,
      mapConfig: {
        ...config.mapConfig,
        stationLayout: {
          ...config.mapConfig?.stationLayout,
          totalStations: config.stationCount,
        },
      },
      rtpRobotCount: config.rtpRobotCount,
      rtpRobotSpeed: config.rtpRobotSpeed,
      numVtms: config.numVtms,
      numHtms: config.numHtms,
      vtmSpeed: config.vtmSpeed,
      htmSpeed: config.htmSpeed,
      operatorTime: config.operatorTime,
      picksPerFace: config.picksPerFace,
      binsCount: config.binsCount,
      orderSelectionAlgorithm: config.orderSelectionAlgorithm,
      putAlgorithm: config.putAlgorithm,
      enableSnapshots: config.enableSnapshots,
      snapshotIntervalSeconds: config.snapshotIntervalSeconds,
      tpqTarget: config.tpqTarget,
      comparisonType,
      algorithm1,
      algorithm2,
    }

    console.log('🚀 Starting algorithm comparison with config:', comparisonConfig)

    // Call the comparison API
    const response = await apiService.runComparison(comparisonConfig)

    if (response.success && response.data) {
      setComparisonResults(response.data)
      setComparisonState('completed')
      console.log('✅ Algorithm comparison completed successfully:', response.data)
    } else {
      throw new Error(response.error || 'Algorithm comparison failed')
    }
  } catch (error) {
    console.error('❌ Algorithm comparison failed:', error)
    let errorMessage = 'Unknown error occurred'

    if (error instanceof Error) {
      if (error.message.includes('fetch')) {
        errorMessage =
          'Cannot connect to simulation backend. Please ensure the backend server is running.'
      } else if (error.message.includes('HTTP')) {
        errorMessage = `Backend error: ${error.message}`
      } else {
        errorMessage = error.message
      }
    }

    setComparisonError(errorMessage)
    setComparisonState('error')
  }
}
```

**Step 4: Add button click handlers**

Add these new handler functions:

```typescript
const handleComparePutClick = () => {
  setPendingComparisonType('put')
  setAlgorithmModalOpen(true)
}

const handleCompareOrderClick = () => {
  setPendingComparisonType('order')
  setAlgorithmModalOpen(true)
}

const handleAlgorithmModalConfirm = (algorithm1: string, algorithm2: string) => {
  if (pendingComparisonType) {
    handleRunComparison(pendingComparisonType, algorithm1, algorithm2)
  }
}

const handleAlgorithmModalClose = () => {
  setAlgorithmModalOpen(false)
  setPendingComparisonType(null)
}
```

**Step 5: Update ConfigurationPanel props**

Find the ConfigurationPanel component usage (around line 289) and update:

```typescript
<ConfigurationPanel
  config={config}
  onConfigChange={handleConfigChange}
  onRunSimulation={handleRunSimulation}
  onCompareAlgorithms={handleComparePutClick}
  onCompareOrderAlgorithms={handleCompareOrderClick}
/>
```

**Step 6: Add AlgorithmSelectionModal to JSX**

After the comparison error modal (around line 383), add:

```typescript
{/* Algorithm Selection Modal */}
{algorithmModalOpen && pendingComparisonType && (
  <AlgorithmSelectionModal
    isOpen={algorithmModalOpen}
    comparisonType={pendingComparisonType}
    algorithms={
      pendingComparisonType === 'put' ? PUT_ALGORITHMS : ORDER_ALGORITHMS
    }
    defaultAlgorithm1={
      pendingComparisonType === 'put'
        ? DEFAULT_PUT_COMPARISON.algorithm1
        : DEFAULT_ORDER_COMPARISON.algorithm1
    }
    defaultAlgorithm2={
      pendingComparisonType === 'put'
        ? DEFAULT_PUT_COMPARISON.algorithm2
        : DEFAULT_ORDER_COMPARISON.algorithm2
    }
    onConfirm={handleAlgorithmModalConfirm}
    onClose={handleAlgorithmModalClose}
  />
)}
```

**Step 7: Verify TypeScript compiles**

Run: `npm run typecheck`

Expected: No errors

**Step 8: Commit**

```bash
git add apps/frontend/src/pages/SimulatorPage.tsx
git commit -m "feat: integrate AlgorithmSelectionModal for both put and order comparisons"
```

---

## Task 11: Frontend - Format and build

**Files:**
- Format: All modified frontend files

**Step 1: Run formatter**

Run: `npm run check:fix`

Expected: All files formatted

**Step 2: Build frontend**

Run: `npx nx build frontend`

Expected: BUILD SUCCESS

**Step 3: Commit formatting if changed**

```bash
git add apps/frontend/
git commit -m "style: format frontend files with Biome"
```

---

## Task 12: Manual Testing - Put Algorithm Comparison

**Files:**
- Test: Full stack integration

**Step 1: Start backend**

Run in terminal 1:
```bash
npx nx serve backend
```

Wait for: Server started on port 8080

**Step 2: Start frontend**

Run in terminal 2:
```bash
npm run dev
```

Wait for: Server started on port 4200

**Step 3: Test put algorithm comparison flow**

1. Open browser: http://localhost:4200
2. Click "Compare Put Algorithms" button
3. Expected: Modal opens with 2 dropdowns
4. Verify: Algorithm 1 defaults to "Greedy Put 🎯"
5. Verify: Algorithm 2 defaults to "Min Weighted Cycle Distance 📐"
6. Select same algorithm in both → "Run Comparison" button disabled
7. Select different algorithms → "Run Comparison" button enabled
8. Click "Run Comparison"
9. Expected: Modal closes, loading modal appears
10. Wait for completion (30-60 seconds)
11. Expected: Results modal shows side-by-side comparison
12. Verify: 2 algorithm cards displayed
13. Verify: Metrics table with 5 rows
14. Verify: Best performer highlighted in green
15. Click "Close"

**Step 4: Document any issues**

Create checklist in plan:
- [ ] Modal opens correctly
- [ ] Dropdowns show proper labels with emojis
- [ ] Validation works (same algorithm disabled)
- [ ] API call succeeds
- [ ] Results display correctly

---

## Task 13: Manual Testing - Order Algorithm Comparison

**Files:**
- Test: Full stack integration

**Step 1: Ensure backend and frontend running**

Backend on :8080, frontend on :4200

**Step 2: Test order algorithm comparison flow**

1. Open browser: http://localhost:4200
2. Click "Compare Order Algorithms" button
3. Expected: Modal opens with 2 dropdowns
4. Verify: Algorithm 1 defaults to "Maximize PPF Directly 📊"
5. Verify: Algorithm 2 defaults to "Maximize TPQ (VUT) 🎯"
6. Verify: Dropdown shows all 3 order algorithms:
   - Maximize PPF Directly 📊
   - PPF with Tiering 🎚️
   - Maximize TPQ (VUT) 🎯
7. Select "PPF with Tiering" for Algorithm 1
8. Select "Maximize TPQ (VUT)" for Algorithm 2
9. Click "Run Comparison"
10. Expected: Modal closes, loading modal appears
11. Wait for completion (30-60 seconds)
12. Expected: Results modal shows comparison
13. Verify: 2 algorithm cards with correct names
14. Verify: Metrics table highlights best performer
15. Verify: Timeline chart shows picks per face data
16. Try different algorithm combinations
17. Click "Run New Comparison"
18. Expected: Returns to simulator, modal closed

**Step 3: Document results**

Create checklist:
- [ ] Order algorithms modal works
- [ ] All 3 algorithms selectable
- [ ] API call with order comparison succeeds
- [ ] Results display correctly
- [ ] Can run multiple comparisons
- [ ] Timeline data shows correctly

---

## Task 14: Backend Unit Test - getOperationModes

**Files:**
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/api/simulations/ComparisonServiceTest.java`

**Step 1: Create test class**

```java
package com.greyorange.dessimulator.api.simulations;

import static org.junit.jupiter.api.Assertions.*;

import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.springframework.test.util.ReflectionTestUtils;

class ComparisonServiceTest {

  private ComparisonService comparisonService;

  @BeforeEach
  void setUp() {
    comparisonService = new ComparisonService(null); // ScenarioManager not needed for these tests
  }

  @Test
  void testGetOperationModes_Put_ReturnsTwoModes() {
    @SuppressWarnings("unchecked")
    List<Map<String, Object>> modes = (List<Map<String, Object>>)
        ReflectionTestUtils.invokeMethod(
            comparisonService,
            "getOperationModes",
            "put",
            "greedy",
            "min_weighted_cycle_distance");

    assertNotNull(modes);
    assertEquals(2, modes.size());

    assertEquals("greedy", modes.get(0).get("key"));
    assertEquals("Greedy Put", modes.get(0).get("name"));
    assertEquals("🎯", modes.get(0).get("emoji"));

    assertEquals("min_weighted_cycle_distance", modes.get(1).get("key"));
    assertEquals("Min Weighted Cycle Distance", modes.get(1).get("name"));
    assertEquals("📐", modes.get(1).get("emoji"));
  }

  @Test
  void testGetOperationModes_Order_ReturnsTwoModes() {
    @SuppressWarnings("unchecked")
    List<Map<String, Object>> modes = (List<Map<String, Object>>)
        ReflectionTestUtils.invokeMethod(
            comparisonService,
            "getOperationModes",
            "order",
            "maximize_ppf_directly",
            "maximize_tpq_with_vut");

    assertNotNull(modes);
    assertEquals(2, modes.size());

    assertEquals("maximize_ppf_directly", modes.get(0).get("key"));
    assertEquals("Maximize PPF Directly", modes.get(0).get("name"));
    assertEquals("📊", modes.get(0).get("emoji"));

    assertEquals("maximize_tpq_with_vut", modes.get(1).get("key"));
    assertEquals("Maximize TPQ (VUT)", modes.get(1).get("name"));
    assertEquals("🎯", modes.get(1).get("emoji"));
  }

  @Test
  void testGetOperationModes_InvalidComparisonType_ThrowsException() {
    assertThrows(
        IllegalArgumentException.class,
        () -> ReflectionTestUtils.invokeMethod(
            comparisonService,
            "getOperationModes",
            "invalid",
            "algo1",
            "algo2"));
  }

  @Test
  void testGetOperationModes_InvalidPutAlgorithm_ThrowsException() {
    assertThrows(
        IllegalArgumentException.class,
        () -> ReflectionTestUtils.invokeMethod(
            comparisonService,
            "getOperationModes",
            "put",
            "invalid_algorithm",
            "greedy"));
  }

  @Test
  void testGetOperationModes_InvalidOrderAlgorithm_ThrowsException() {
    assertThrows(
        IllegalArgumentException.class,
        () -> ReflectionTestUtils.invokeMethod(
            comparisonService,
            "getOperationModes",
            "order",
            "invalid_algorithm",
            "maximize_ppf_directly"));
  }
}
```

**Step 2: Run test**

Run: `cd apps/backend && mvn test -Dtest=ComparisonServiceTest`

Expected: All tests pass

**Step 3: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/api/simulations/ComparisonServiceTest.java
git commit -m "test: add unit tests for getOperationModes"
```

---

## Task 15: Backend Unit Test - applyModeConfig

**Files:**
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/api/simulations/ComparisonServiceTest.java`

**Step 1: Add test methods**

Add to existing ComparisonServiceTest class:

```java
@Test
void testApplyModeConfig_OrderAlgorithm_OverridesCorrectly() {
  // Create base config with default order algorithm
  SimulationConfig baseConfig = SimulationConfig.builder()
      .algorithmConfig(AlgorithmConfig.builder()
          .orderSelectionAlgorithm(OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY)
          .build())
      .build();

  // Create mode with different order algorithm
  Map<String, Object> mode = Map.of(
      "config", Map.of("orderSelectionAlgorithm", "maximize_tpq_with_vut"));

  SimulationConfig result = (SimulationConfig) ReflectionTestUtils.invokeMethod(
      comparisonService, "applyModeConfig", baseConfig, mode);

  assertNotNull(result);
  assertEquals(
      OrderSelectionAlgorithm.MAXIMIZE_TPQ_WITH_VUT,
      result.algorithmConfig().orderSelectionAlgorithm());
}

@Test
void testApplyModeConfig_PutAlgorithm_OverridesCorrectly() {
  // Create base config with default put algorithm
  SimulationConfig baseConfig = SimulationConfig.builder()
      .algorithmConfig(AlgorithmConfig.builder()
          .putAlgorithm("greedy")
          .build())
      .build();

  // Create mode with different put algorithm
  Map<String, Object> mode = Map.of(
      "config", Map.of("putAlgorithm", "min_weighted_cycle_distance"));

  SimulationConfig result = (SimulationConfig) ReflectionTestUtils.invokeMethod(
      comparisonService, "applyModeConfig", baseConfig, mode);

  assertNotNull(result);
  assertEquals("min_weighted_cycle_distance", result.algorithmConfig().putAlgorithm());
}

@Test
void testApplyModeConfig_EmptyConfig_ReturnsOriginal() {
  SimulationConfig baseConfig = SimulationConfig.builder().build();
  Map<String, Object> mode = Map.of("config", Map.of());

  SimulationConfig result = (SimulationConfig) ReflectionTestUtils.invokeMethod(
      comparisonService, "applyModeConfig", baseConfig, mode);

  assertEquals(baseConfig, result);
}
```

**Step 2: Add imports**

Add to imports section:

```java
import com.greyorange.dessimulator.domain.order.OrderSelectionAlgorithm;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.AlgorithmConfig;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.SimulationConfig;
```

**Step 3: Run tests**

Run: `cd apps/backend && mvn test -Dtest=ComparisonServiceTest`

Expected: All tests pass

**Step 4: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/api/simulations/ComparisonServiceTest.java
git commit -m "test: add unit tests for applyModeConfig"
```

---

## Task 16: Frontend Component Test - AlgorithmSelectionModal

**Files:**
- Create: `apps/frontend/src/pages/simulator/components/AlgorithmSelectionModal.test.tsx`

**Step 1: Create test file**

```typescript
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import AlgorithmSelectionModal from './AlgorithmSelectionModal'
import { ORDER_ALGORITHMS } from '../../../constants/algorithms'

describe('AlgorithmSelectionModal', () => {
  const mockOnConfirm = vi.fn()
  const mockOnClose = vi.fn()

  const defaultProps = {
    isOpen: true,
    comparisonType: 'order' as const,
    algorithms: ORDER_ALGORITHMS,
    defaultAlgorithm1: 'maximize_ppf_directly',
    defaultAlgorithm2: 'maximize_tpq_with_vut',
    onConfirm: mockOnConfirm,
    onClose: mockOnClose,
  }

  it('renders with correct title for order comparison', () => {
    render(<AlgorithmSelectionModal {...defaultProps} />)
    expect(screen.getByText(/Select Order Selection Algorithms/i)).toBeInTheDocument()
  })

  it('renders with correct title for put comparison', () => {
    render(<AlgorithmSelectionModal {...defaultProps} comparisonType="put" />)
    expect(screen.getByText(/Select Put Algorithms/i)).toBeInTheDocument()
  })

  it('initializes with default algorithms', () => {
    render(<AlgorithmSelectionModal {...defaultProps} />)
    // Verify defaults are set (implementation depends on Select component)
    expect(screen.getByText('Run Comparison')).not.toBeDisabled()
  })

  it('disables Run Comparison button when same algorithm selected', () => {
    render(
      <AlgorithmSelectionModal
        {...defaultProps}
        defaultAlgorithm1="maximize_ppf_directly"
        defaultAlgorithm2="maximize_ppf_directly"
      />
    )
    expect(screen.getByText('Run Comparison')).toBeDisabled()
  })

  it('shows validation message when algorithms are the same', () => {
    render(
      <AlgorithmSelectionModal
        {...defaultProps}
        defaultAlgorithm1="maximize_ppf_directly"
        defaultAlgorithm2="maximize_ppf_directly"
      />
    )
    expect(screen.getByText(/Please select 2 different algorithms/i)).toBeInTheDocument()
  })

  it('calls onConfirm with selected algorithms when Run Comparison clicked', () => {
    render(<AlgorithmSelectionModal {...defaultProps} />)
    fireEvent.click(screen.getByText('Run Comparison'))
    expect(mockOnConfirm).toHaveBeenCalledWith(
      'maximize_ppf_directly',
      'maximize_tpq_with_vut'
    )
  })

  it('calls onClose when Cancel clicked', () => {
    render(<AlgorithmSelectionModal {...defaultProps} />)
    fireEvent.click(screen.getByText('Cancel'))
    expect(mockOnClose).toHaveBeenCalled()
  })
})
```

**Step 2: Run tests**

Run: `npm test -- AlgorithmSelectionModal.test.tsx`

Expected: All tests pass (may need to adjust based on shadcn Select behavior)

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/AlgorithmSelectionModal.test.tsx
git commit -m "test: add component tests for AlgorithmSelectionModal"
```

---

## Task 17: Documentation - Update implementation notes

**Files:**
- Create: `docs/plans/2025-11-17-order-algorithm-comparison-notes.md`

**Step 1: Create notes file**

```markdown
# Order Algorithm Comparison - Implementation Notes

## Overview

Extended the comparison runner to support order selection algorithm comparisons alongside existing put algorithm comparisons.

## Key Design Decisions

### Modal-Based Selection Pattern

**Decision:** Use modal for algorithm selection for both put and order comparisons

**Rationale:**
- Consistent UX pattern for both comparison types
- Allows future extension (compare robot speeds, scenarios, etc.)
- More flexible than hardcoded algorithm pairs
- Reusable modal component reduces code duplication

### Backend Algorithm Parameters

**Decision:** Add `algorithm1` and `algorithm2` to `ComparisonConfigDTO`

**Rationale:**
- Type-safe contract between frontend and backend
- Backend validates algorithm values exist
- Easy to extend to 3+ algorithm comparisons in future
- OpenAPI generates TypeScript types automatically

### Frontend Display Metadata

**Decision:** Maintain algorithm display metadata (labels, emojis) in frontend constants

**Rationale:**
- Display concerns belong in presentation layer
- Backend controls valid algorithm values via enum
- No runtime API calls for static metadata
- Type safety via `satisfies` keyword ensures coverage

### Reuse Existing Comparison Infrastructure

**Decision:** Reuse comparison state, loading modal, and results modal

**Rationale:**
- `AlgorithmComparison` component is generic - displays any `ComparisonResponse`
- Users won't run both comparisons simultaneously
- Reduces code duplication
- Easier to maintain single code path

## Architecture

```
User clicks button → Modal opens → Select 2 algorithms → Run Comparison
  ↓
Frontend sends ComparisonConfigDTO { comparisonType, algorithm1, algorithm2 }
  ↓
ComparisonService.getOperationModes(type, alg1, alg2) → builds mode configs
  ↓
ComparisonService.applyModeConfig() → overrides algorithmConfig via toBuilder()
  ↓
Run 2 simulations with different algorithms → calculate metrics
  ↓
Return ComparisonResponse → Frontend displays results
```

## Files Modified

**Backend:**
- `ComparisonConfigDTO` - Added algorithm1, algorithm2 fields
- `ComparisonService` - Refactored getOperationModes, added metadata helpers, implemented applyModeConfig

**Frontend:**
- `constants/algorithms.ts` - Algorithm display metadata (NEW)
- `AlgorithmSelectionModal.tsx` - Reusable selection modal (NEW)
- `ConfigurationPanel.tsx` - Added order comparison button
- `SimulatorPage.tsx` - Modal state, updated handlers
- `generated-api.ts` - Regenerated with new DTO fields

## Testing

**Backend Unit Tests:**
- `ComparisonServiceTest.testGetOperationModes_Order_ReturnsTwoModes()`
- `ComparisonServiceTest.testApplyModeConfig_OrderAlgorithm_OverridesCorrectly()`

**Frontend Component Tests:**
- `AlgorithmSelectionModal.test.tsx` - Modal rendering and validation

**Manual Testing:**
- Put algorithm comparison flow
- Order algorithm comparison flow
- Validation (same algorithm selection)
- Results display

## Future Extensions

**Easy to add:**
- Compare 3+ algorithms (change validation, add third dropdown)
- Compare robot speeds (new comparison type)
- Compare scenarios (new comparison type)
- Export comparison results (add button to AlgorithmComparison)

**Pattern to follow:**
1. Add new `comparisonType` to backend
2. Add metadata helper (like `buildOrderModeInfo`)
3. Update `applyModeConfig` to handle new config field
4. Add algorithm metadata to frontend constants
5. Reuse existing modal and results components
```

**Step 2: Commit**

```bash
git add docs/plans/2025-11-17-order-algorithm-comparison-notes.md
git commit -m "docs: add implementation notes for order algorithm comparison"
```

---

## Task 18: Final Verification - Full Integration Test

**Files:**
- Test: End-to-end flow

**Step 1: Clean build backend**

Run:
```bash
cd apps/backend
mvn clean compile
mvn test
```

Expected: All tests pass

**Step 2: Clean build frontend**

Run:
```bash
npm run check:fix
npm run typecheck
npx nx build frontend
```

Expected: BUILD SUCCESS, no type errors

**Step 3: Start both servers**

Terminal 1: `npx nx serve backend`
Terminal 2: `npm run dev`

**Step 4: Run complete test scenarios**

**Scenario 1: Put Comparison**
1. Navigate to http://localhost:4200
2. Click "Compare Put Algorithms"
3. Select "Greedy Put" and "Min Weighted Cycle Distance"
4. Run comparison
5. Verify results display correctly
6. Close results

**Scenario 2: Order Comparison**
1. Click "Compare Order Algorithms"
2. Select "Maximize PPF Directly" and "PPF with Tiering"
3. Run comparison
4. Verify results display correctly
5. Run another comparison with different algorithms
6. Verify results update

**Scenario 3: Validation**
1. Click "Compare Order Algorithms"
2. Select same algorithm in both dropdowns
3. Verify button disabled
4. Change to different algorithms
5. Verify button enabled

**Step 5: Check browser console**

Verify:
- No console errors
- API requests succeed
- Comparison results logged correctly

**Step 6: Document completion**

All scenarios pass → Feature complete

---

## Task 19: Final Cleanup and Commit

**Files:**
- All modified files

**Step 1: Format all code**

Backend:
```bash
cd apps/backend && mvn spotless:apply
```

Frontend:
```bash
npm run check:fix
```

**Step 2: Run full test suite**

Backend:
```bash
cd apps/backend && mvn test
```

Frontend:
```bash
npm test
```

**Step 3: Final build verification**

```bash
npx nx build frontend
cd apps/backend && mvn clean package -DskipTests
```

Expected: Both build successfully

**Step 4: Review git status**

Run: `git status`

Verify all changes committed

**Step 5: Final commit if any formatting changes**

```bash
git add .
git commit -m "chore: final formatting and cleanup"
```

---

## Summary

**Implementation complete!**

**Features added:**
- ✅ Order algorithm comparison support in backend
- ✅ Modal-based algorithm selection for put and order
- ✅ Reusable AlgorithmSelectionModal component
- ✅ Algorithm display metadata constants
- ✅ Updated ConfigurationPanel with order comparison button
- ✅ Updated SimulatorPage with modal integration
- ✅ Backend unit tests for getOperationModes and applyModeConfig
- ✅ Frontend component tests for AlgorithmSelectionModal
- ✅ Manual testing documentation

**Files created:**
- `apps/frontend/src/constants/algorithms.ts`
- `apps/frontend/src/pages/simulator/components/AlgorithmSelectionModal.tsx`
- `apps/backend/src/test/java/com/greyorange/dessimulator/api/simulations/ComparisonServiceTest.java`
- `apps/frontend/src/pages/simulator/components/AlgorithmSelectionModal.test.tsx`
- `docs/plans/2025-11-17-order-algorithm-comparison-notes.md`

**Files modified:**
- `ComparisonConfigDTO` (or SimulationsController if nested)
- `ComparisonService.java`
- `ConfigurationPanel.tsx`
- `SimulatorPage.tsx`
- `generated-api.ts` (regenerated)

**Ready for production!**
