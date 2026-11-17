# Frontend Code Quality Review

**Date:** 2026-02-08
**Scope:** `apps/frontend/` (~45 components, 107 TS/TSX files, ~15-18K LOC)
**Method:** 5 parallel review agents covering component architecture, state management, TypeScript quality, performance/rendering, and testing/config.

---

## CRITICAL (P0) — 3 Issues

### 1. No TanStack Query (despite being documented in CLAUDE.md and frontend skills)

`@tanstack/react-query` is **not installed**. All data fetching uses raw `fetch` + manual `useState`/`useEffect`.

**Consequences:**
- Duplicated fetch logic (e.g., `getWorkloadScenarios()` called independently from both `WorkloadScenarioTab.tsx` and `ConfigurationPanel.tsx`)
- No shared cache across components — navigating away and back re-fetches everything
- Race conditions — no `AbortController` cleanup in any `useEffect`
- No automatic refetching, optimistic updates, or stale-while-revalidate

**Files affected:**
- `package.json` — no `@tanstack/react-query` dependency
- `src/main.tsx` — no `QueryClientProvider`
- All scenario tabs (`WorkloadScenarioTab.tsx`, `InfrastructureScenarioTab.tsx`, `AlgorithmScenarioTab.tsx`)
- `src/pages/simulator/components/ConfigurationPanel.tsx`
- `src/hooks/useWarehouseMap.ts`

**Fix:** Install `@tanstack/react-query`, add `QueryClientProvider` at root, migrate all data fetching to `useQuery`/`useMutation` hooks.

---

### 2. TypeScript Strict Mode Completely Disabled

**File:** `apps/frontend/tsconfig.app.json` (lines 22-27)

```json
"strict": false,
"noImplicitAny": false,
"strictNullChecks": false,
"noImplicitReturns": false,
"noFallthroughCasesInSwitch": false
```

The base `tsconfig.base.json` correctly sets `strict: true`, but `tsconfig.app.json` overrides it to `false` with a TODO comment.

**Consequences:**
- 40+ `as any` casts across production code
- Fake type guards in `useLazySnapshotReconstruction.ts` (12 `as any` casts)
- 8 `as any` casts in `SimulationResults.tsx`
- `(response.data as any).data` pattern in all tab components
- Zero compile-time null safety

**Key `as any` locations:**
| File | Count | Root cause |
|------|-------|------------|
| `src/hooks/useLazySnapshotReconstruction.ts` | 12 | Generated types missing `type` discriminator and tote sub-properties |
| `src/pages/simulator/components/SimulationResults.tsx` | 8 | `SimulationResults` type missing `snapshotCount`, `binarySnapshots` |
| `src/pages/simulator/components/AlgorithmComparison.tsx` | 4 | `modeInfo` typed as generic object |
| `src/pages/simulator/tabs/InfrastructureTab.tsx` | 1 | API response wrapper type mismatch |
| `src/pages/simulator/tabs/CatalogTab.tsx` | 1 | Same API wrapper issue |
| `src/pages/simulator/tabs/OrdersTab.tsx` | 2 | Same API wrapper issue |
| `src/components/common/maps/WarehouseMap.tsx` | 4 | Snapshot cast to `any` at component entry |

**Fix:** Enable incrementally — start with `strictNullChecks: true`, then `noImplicitAny: true`. Create extension interfaces for generated types that are missing fields.

---

### 3. Low Test Coverage (~33%)

10 test files for ~45 testable components.

**What IS tested (good quality):**
- `BinarySnapshotReader.test.ts` — exemplary, 1718 lines
- `LRUCache.test.ts` — clean, focused
- `useLazySnapshotReconstruction.test.ts` — good `renderHook` usage
- `AlgorithmSelectionModal.test.tsx` — 12 tests, good assertions
- 4 Konva layer tests (StationsLayer, StorageSpacesLayer, RobotsLayer, RelayPositionsLayer)
- `AnimationControls.test.tsx`, `VisualizationModeSelector.test.tsx`

**What is NOT tested (zero coverage):**
- **Pages:** `SimulatorPage.tsx`, `ScenariosPage.tsx`
- **Core components:** `ConfigurationPanel.tsx`, `SimulationResults.tsx`, `SimulationLoading.tsx`
- **Forms:** `ConfigurationForm.tsx`, `ConfigurationTable.tsx`
- **Maps:** `WarehouseMap.tsx`, `WarehouseHeatmap.tsx`, `CellOccupancyLayer.tsx`, `CombinedOverlayLayer.tsx`, `ConflictFactorLayer.tsx`
- **Entire scenario feature:** All 8 files in `src/components/scenarios/`
- **All services:** `api.ts`, `warehouseMapService.ts`, `simulationApi.ts`
- **Hooks:** `useWarehouseMap.ts`, `use-mobile.tsx`
- **Utils:** `colorGradient.ts`

**Fix:** Add Vitest coverage config, target 60%+ coverage. Prioritize services/API layer and page-level integration tests.

---

## WARNING (P1) — 12 Issues

### 4. SimulatorPage God Component

**File:** `src/pages/SimulatorPage.tsx` (506 lines)

- 13 `useState` calls (lines 32-47)
- Stores ~14.9MB `ArrayBuffer` in React state (line 177)
- Mixes 5 concerns: form config, simulation execution, comparison execution, modal management, tab rendering
- Duplicated API config building at lines 115-145 and 250-283
- Duplicated error parsing at lines 186-196 and 299-309
- 12 `console.log`/`console.error` calls left from debugging

**Fix:** Extract `useSimulation` and `useComparison` custom hooks. Store `ArrayBuffer` in `useRef`. Extract `buildBaseApiConfig()` utility. Remove debug logs.

### 5. ConfigurationPanel Oversized

**File:** `src/pages/simulator/components/ConfigurationPanel.tsx` (727 lines)

- 12 boolean `useState` for collapsible sections (lines 39-52)
- Mixes data fetching (lines 55-101) and presentation
- Complex scenario application logic (lines 104-173)
- `eslint-disable-next-line react-hooks/exhaustive-deps` at line 182
- Duplicated fallback scenario arrays (lines 68-72, 77-81)

**Fix:** Extract sub-components (`WorkloadSection`, `InfrastructureSection`). Use `Set<string>` or uncontrolled pattern for collapsibles. Extract `useConfigurationScenarios` hook.

### 6. Copy-Pasted Scenario Tabs

**Files:** `WorkloadScenarioTab.tsx`, `InfrastructureScenarioTab.tsx`, `AlgorithmScenarioTab.tsx`

All three implement identical patterns: same `useState` calls, same `useEffect` for loading, same delete with `window.confirm`, same error/loading/empty JSX.

**Fix:** Create generic `useScenarioList<T>` hook and `ScenarioListLayout<T>` component. Eliminates ~200 lines of duplication.

### 7. No Error Boundaries

No `ErrorBoundary` component anywhere. `App.tsx` and `main.tsx` have no error boundary wrapping. Any rendering error crashes the entire app to a white screen.

**Fix:** Add top-level `ErrorBoundary` wrapping `<App />`, and per-route error boundaries.

### 8. Animation Performance — `setInterval` Instead of `requestAnimationFrame`

**File:** `src/components/AnimationControls.tsx`

- Uses `setInterval` (line 43) which doesn't sync with browser paint cycle
- `useEffect` (lines 34-53) tears down and recreates the interval on every frame because `currentFrame` is in the dependency array
- `onPrefetch={() => {}}` (in `SimulationResults.tsx:237`) creates new ref every render, also triggering effect re-creation

**Fix:** Replace with `requestAnimationFrame` + elapsed-time accumulator. Use `useRef` for current frame to avoid effect teardown every frame.

### 9. `transformX`/`transformY` Not Memoized

**File:** `src/components/common/maps/WarehouseMap.tsx` (lines 225-241)

Plain functions recreated every render, passed to all 4 map layers. Since map layers use custom memo comparators that check these by reference, all layers re-render on every frame during animation.

**Fix:** Wrap both in `useCallback` with deps `[mapConfig, gridStartX, gridStartY, effectiveGridSize]`. Single highest-impact performance fix for animation.

### 10. Zero `useCallback` in Application Code

No application component uses `useCallback`. All event handlers (`handleRunSimulation`, `handleConfigChange`, etc.) are recreated every render and passed as props, causing unnecessary child re-renders.

**Fix:** Wrap handlers passed as props in `useCallback`.

### 11. `as any` Proliferation

See Critical #2 above for full details. 40+ occurrences across production code.

### 12. Duplicate Type Systems

Three parallel type systems for the same concepts:

| System | File | Status |
|--------|------|--------|
| Generated API types | `generated-api.ts` + `generated.ts` | Active, authoritative |
| Legacy snake_case types | `api.ts` (356 lines) | Should be removed |
| Manual UI types | `simulation.ts` | Partially duplicates generated types |

`SimulationConfig` and `SimulationResults` exist in both `generated.ts` and `simulation.ts`. Barrel `index.ts` exports the manual version, not the generated one.

**Fix:** Delete `api.ts`. Update `simulation.ts` to only contain UI-specific types. Re-export shared types from `generated.ts`.

### 13. `warehouseMapService` Cache Leak

**File:** `src/services/warehouseMapService.ts`

Custom `Map` cache with `setTimeout` for eviction (lines 179-181). No `clearTimeout` cleanup — timers leak on HMR or service re-instantiation. JSON.stringify-based cache key is fragile.

**Fix:** Move caching to TanStack Query, or add proper timer cleanup.

### 14. No Vitest Coverage Configuration

**File:** `vite.config.ts`

No `coverage` block in test config. No way to track coverage trends or set thresholds.

**Fix:** Add `coverage: { provider: 'v8', reporter: ['text', 'lcov'], thresholds: { statements: 30 } }`.

### 15. No Code Splitting

No `React.lazy`, no `Suspense`, no `manualChunks` in Vite config. Entire app (including heavy `konva`, `react-konva`, `recharts`) is a single bundle.

**Fix:** Add `manualChunks` for vendor splitting. Use `React.lazy` for tab content in `SimulationResults` (especially `AIAnalysis`, `GridResults`, animation view).

---

## INFO (P2) — 8 Issues

### 16. Inconsistent API Layers

Two separate API patterns:
- `src/services/api.ts` — class-based singleton `ApiService` with generic `request<T>()`
- `src/api/simulationApi.ts` — standalone function using raw `fetch`
- `src/services/warehouseMapService.ts` — another class with raw `fetch`

**Fix:** Consolidate to single approach (TanStack Query will supersede both).

### 17. `fireEvent` Instead of `userEvent`

`@testing-library/user-event` not installed. Tests use `fireEvent` (synchronous) instead of `userEvent` (more realistic simulation of user behavior).

### 18. Tailwind Version Mismatch

- `tailwindcss: ^3.4.3` (core)
- `@tailwindcss/vite: ^4.1.13` (plugin)

These are incompatible major versions.

### 19. Silent Fallback to Hardcoded Data

`warehouseMapService.ts` and `ConfigurationPanel.tsx` silently return hardcoded fallback data when API calls fail. User is never informed they're seeing stale data.

### 20. `JSON.stringify` in `useEffect` Dependencies

**File:** `src/pages/simulator/components/WarehousePreview.tsx:49`

`JSON.stringify(config.mapConfig)` used for deep comparison in deps array. Called every render. `generateMap` missing from deps.

### 21. `React.FC` Used Everywhere

50+ components use `React.FC<Props>`. Modern community consensus recommends avoiding it (implicit children pre-React 18, poor generic support).

### 22. Mixed Export Patterns

Most components use `export default`. `App.tsx` and `AppSidebar.tsx` have both named and default exports.

### 23. O(racks x totes) Aggregation Complexity

**File:** `src/hooks/useLazySnapshotReconstruction.ts:351-355`

For every rack, scans entire totes array. With 500 racks and 2000 totes = 1M comparisons per frame.

**Fix:** Pre-build position-keyed `Map<"x,y", Tote[]>` for O(1) lookup.

---

## Positive Findings

| Area | Details |
|------|---------|
| **Map layer memoization** | `CellOccupancyLayer`, `StorageSpacesLayer` use `memo` + custom comparators + `useMemo` throughout |
| **BinarySnapshotReader tests** | 1718 lines, thorough edge case coverage, excellent structure |
| **LRUCache tests** | Clean, focused, covers all operations |
| **OpenAPI type generation** | Well-structured pipeline via `scripts/generate-merged-api-types.js` |
| **Biome config** | `useImportType: "error"`, `useExhaustiveDependencies: "warn"` |
| **File organization** | Logical hybrid feature-based/type-based structure |
| **`colorGradient` utility** | Well-designed, shared across 6+ components |
| **Test setup** | Clean Konva/canvas mocking, fake-indexeddb |
| **Key prop usage** | Correct throughout — no array index keys in dynamic lists |
| **Props interface consistency** | 29/30 use `interface` (only shadcn exception) |

---

## Recommended Fix Order

| Priority | Action | Impact | Effort |
|----------|--------|--------|--------|
| **P0-1** | Install TanStack Query + `QueryClientProvider` + migrate fetching | Eliminates duplicate fetching, race conditions, manual cache | High |
| **P0-2** | Enable `strictNullChecks: true` then `noImplicitAny: true` | Catches null bugs at compile time | Medium |
| **P0-3** | Add tests for pages, services, scenarios | Raises coverage from ~33% toward 60%+ | High |
| **P1-1** | Memoize `transformX`/`transformY` with `useCallback` | Biggest animation perf win — prevents all 4 layers re-rendering every frame | Low |
| **P1-2** | Replace `setInterval` with `requestAnimationFrame` | Smooth animation, proper frame pacing | Low |
| **P1-3** | Extract `useSimulation`/`useComparison` from SimulatorPage | Breaks up god component | Medium |
| **P1-4** | Create generic `ScenarioTab<T>` | Eliminates ~200 lines copy-paste | Medium |
| **P1-5** | Add top-level `ErrorBoundary` | Prevents white-screen crashes | Low |
| **P1-6** | Delete legacy `api.ts` types, consolidate to generated | Reduces type confusion | Medium |
| **P2-1** | Add Vite `manualChunks` + `React.lazy` | Improves initial load time | Low |
| **P2-2** | Fix Tailwind version mismatch | Prevents potential style issues | Low |
| **P2-3** | Add Vitest coverage config | Enables coverage tracking | Low |
