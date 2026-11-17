# Configuration Summary Redesign - Implementation Plan

**Date:** November 24, 2025  
**Related Design:** [2025-11-24-configuration-summary-redesign.md](./2025-11-24-configuration-summary-redesign.md)  
**Status:** Ready for Implementation

---

## Summary

Replace the Configuration Summary section in `ResultsOverview.tsx` with a cleaner three-section vertical layout that groups related configuration parameters logically.

---

## Implementation Steps

### Phase 1: Preparation (5 min)

**Step 1.1: Review Current Code**
- [x] Read `apps/frontend/src/pages/simulator/components/ResultsOverview.tsx`
- [x] Identify exact lines to replace (71-136)
- [x] Verify helper functions available in `type-guards.ts`
- [x] Understand current props and data flow

**Step 1.2: Verify Type Definitions**
- [x] Confirm `SimulationConfig` structure in `generated-api.ts`
- [x] Verify nested config objects: `mapConfig`, `robotConfig`, `operatorConfig`, `algorithmConfig`
- [x] Check `RtpRobotConfig` and `RelayRobotConfig` interfaces

---

### Phase 2: Create Formatting Helpers (10 min)

**Step 2.1: Add Value Formatters**

Add these inline helper functions within the component (after existing helper code, before the return statement):

```typescript
// Value formatting helpers for configuration display
const formatDuration = (hours: number) => `${hours}h`

const formatSpeed = (speed: number) => `${speed.toFixed(1)}m/s`

const formatSize = (size: number) => `${size.toFixed(1)}m`

const formatTPQ = (tpq?: number) => tpq ? tpq.toFixed(2) : 'N/A'

const formatSnapshot = (enabled: boolean, interval?: number) => 
  enabled && interval ? `${interval}s` : 'Disabled'

const formatOrderAlgorithm = (algo?: string): string => {
  if (!algo) return 'N/A'
  const map: Record<string, string> = {
    'maximize_ppf_directly': 'Maximize PPF (Direct)',
    'maximize_ppf_with_tiering': 'Maximize PPF (Tiered)',
    'maximize_tpq_with_vut': 'Maintain TPQ',
  }
  return map[algo] || algo
}

const formatPutAlgorithm = (algo?: string): string => {
  if (!algo) return 'N/A'
  if (algo === 'min_weighted_cycle_distance') return 'Min Weighted Cycle'
  return algo.charAt(0).toUpperCase() + algo.slice(1)
}
```

**Step 2.2: Extract System Type**

Add variable after existing variable declarations:

```typescript
const isRelay = config?.systemType === 'relay'
```

---

### Phase 3: Build Section Components (30 min)

**Step 3.1: Create Reusable ConfigRow Component**

Add inline component before the main return statement:

```typescript
// Helper component for config key-value rows
const ConfigRow: React.FC<{ label: string; value: string | number }> = ({ label, value }) => (
  <div className="flex justify-between text-sm">
    <span className="text-gray-500">{label}</span>
    <span className="font-medium text-gray-900">{value}</span>
  </div>
)
```

**Step 3.2: Build Simulation Section**

Create JSX for the first section:

```typescript
{/* Simulation Section */}
<div>
  <h4 className="text-sm font-medium text-gray-700 mb-3">Simulation</h4>
  <div className="grid grid-cols-2 gap-x-6 gap-y-3">
    <ConfigRow 
      label="System" 
      value={config?.systemType?.toUpperCase() || 'N/A'} 
    />
    <ConfigRow 
      label="Duration" 
      value={formatDuration(config?.simulationDuration || 0)} 
    />
    <ConfigRow 
      label="Scenario" 
      value={config?.scenarioId || 'N/A'} 
    />
    <ConfigRow 
      label="Snapshots" 
      value={formatSnapshot(
        config?.enableSnapshots || false, 
        config?.snapshotIntervalSeconds
      )} 
    />
  </div>
</div>
```

**Step 3.3: Build Infrastructure Section (RTP)**

Create conditional JSX for RTP system:

```typescript
{/* Infrastructure Section */}
<div className="pt-4 border-t border-gray-200">
  <h4 className="text-sm font-medium text-gray-700 mb-3">Infrastructure</h4>
  <div className="grid grid-cols-2 gap-x-6 gap-y-3">
    <ConfigRow 
      label="Grid" 
      value={`${config?.mapConfig?.gridWidth || 0}×${config?.mapConfig?.gridDepth || 0}`} 
    />
    <ConfigRow 
      label="Robots" 
      value={config ? getRobotCount(config) : 0} 
    />
    <ConfigRow 
      label="Stations" 
      value={config?.mapConfig?.stationLayout?.totalStations || 0} 
    />
    <ConfigRow 
      label="Robot Speed" 
      value={config ? formatSpeed(getRobotSpeed(config)) : 'N/A'} 
    />
    <ConfigRow 
      label="Bins" 
      value={config?.operatorConfig?.numBins || 0} 
    />
    <ConfigRow 
      label="Cell Size" 
      value={formatSize(config?.mapConfig?.cellSizeMeters || 1.0)} 
    />
  </div>
</div>
```

**Step 3.4: Build Infrastructure Section (Relay Variation)**

Replace the Robot/Robot Speed rows with conditional rendering:

```typescript
{!isRelay ? (
  <>
    <ConfigRow 
      label="Robots" 
      value={config ? getRobotCount(config) : 0} 
    />
    <ConfigRow 
      label="Robot Speed" 
      value={config ? formatSpeed(getRobotSpeed(config)) : 'N/A'} 
    />
  </>
) : (
  <>
    <ConfigRow 
      label="VTMs" 
      value={(config?.robotConfig as RelayRobotConfig)?.numVtms || 0} 
    />
    <ConfigRow 
      label="HTMs" 
      value={(config?.robotConfig as RelayRobotConfig)?.numHtms || 0} 
    />
    <ConfigRow 
      label="VTM Speed" 
      value={formatSpeed((config?.robotConfig as RelayRobotConfig)?.vtmHorizontalSpeed || 0)} 
    />
    <ConfigRow 
      label="HTM Speed" 
      value={formatSpeed((config?.robotConfig as RelayRobotConfig)?.htmSpeed || 0)} 
    />
  </>
)}
```

**Step 3.5: Build Optimization Section**

Create JSX for the third section:

```typescript
{/* Optimization Section */}
<div className="pt-4 border-t border-gray-200">
  <h4 className="text-sm font-medium text-gray-700 mb-3">Optimization</h4>
  <div className="grid grid-cols-2 gap-x-6 gap-y-3">
    <ConfigRow 
      label="Pick Time" 
      value={`${config?.operatorConfig?.operatorTimePerPick || 0}s`} 
    />
    <ConfigRow 
      label="Put Algorithm" 
      value={formatPutAlgorithm(config?.algorithmConfig?.putAlgorithm)} 
    />
    <ConfigRow 
      label="Target PPF" 
      value={(config?.operatorConfig?.targetPicksPerFace || 0).toFixed(1)} 
    />
    <ConfigRow 
      label="Order Algo" 
      value={formatOrderAlgorithm(config?.algorithmConfig?.orderSelectionAlgorithm)} 
    />
    <ConfigRow 
      label="Target TPQ" 
      value={formatTPQ(config?.algorithmConfig?.tpqConfig?.targetTPQ)} 
    />
    <ConfigRow 
      label="Nav Factor" 
      value={(config?.algorithmConfig?.navFactor || 0).toFixed(1)} 
    />
  </div>
</div>
```

---

### Phase 4: Integration (10 min)

**Step 4.1: Replace Old Configuration Summary**

In `ResultsOverview.tsx`:
1. Locate lines 71-136 (current Configuration Summary section)
2. Keep the section header: `<SectionHeader title="Configuration Summary" />`
3. Replace the entire `<div className="bg-gray-50 rounded-lg p-6">` contents with the new three-section layout
4. Ensure proper nesting within the existing structure

**Step 4.2: Add Required Imports**

Add to imports at top of file:

```typescript
import type { RelayRobotConfig } from '../../../types/generated'
```

**Step 4.3: Verify Existing Imports**

Confirm these imports are already present:
- `getRobotCount, getRobotSpeed` from `type-guards`
- `SimulationResults` type
- React and component imports

---

### Phase 5: Testing (15 min)

**Step 5.1: Build and Run**
```bash
# From frontend directory
npm run dev
```

**Step 5.2: Manual Testing Checklist**

- [ ] **RTP System Test**
  - [ ] Run simulation with RTP system
  - [ ] Verify "System" shows "RTP"
  - [ ] Verify "Robots" and "Robot Speed" display (2 fields)
  - [ ] Check all three sections render properly

- [ ] **Relay System Test**
  - [ ] Switch to Relay in config
  - [ ] Run simulation
  - [ ] Verify "System" shows "RELAY"
  - [ ] Verify VTMs, HTMs, VTM Speed, HTM Speed display (4 fields)

- [ ] **Snapshots Test**
  - [ ] Enable snapshots (10s) → should show "10s"
  - [ ] Disable snapshots → should show "Disabled"

- [ ] **Algorithms Test**
  - [ ] Test each order algorithm displays friendly name
  - [ ] Test put algorithm displays properly capitalized

- [ ] **TPQ Test**
  - [ ] Run with TPQ enabled → verify 2 decimal places
  - [ ] Verify "N/A" doesn't show if TPQ is configured

- [ ] **Visual Test**
  - [ ] Section headers styled correctly
  - [ ] Values aligned to the right
  - [ ] Proper spacing between sections
  - [ ] Container has gray background
  - [ ] Borders between sections are subtle

- [ ] **Edge Cases**
  - [ ] Long scenario names don't break layout
  - [ ] All decimals formatted consistently
  - [ ] No "undefined" or "null" text visible

**Step 5.3: Browser Testing**
- [ ] Chrome
- [ ] Safari (if on Mac)
- [ ] Firefox

---

## Rollback Plan

If issues occur:
1. Use git to revert the changes to `ResultsOverview.tsx`
2. The old implementation is in lines 71-136 of the original file
3. No database or backend changes required

---

## Success Criteria

✅ All three sections (Simulation, Infrastructure, Optimization) display correctly  
✅ RTP and Relay system types show appropriate robot fields  
✅ All values are properly formatted with correct units and decimal places  
✅ Snapshots show "Disabled" when off, interval when on  
✅ Algorithm names display in friendly format  
✅ TPQ shows "N/A" when not configured  
✅ Visual styling matches design spec  
✅ No console errors or warnings  
✅ Layout is clean and readable  

---

## Time Estimate

- Preparation: 5 min
- Formatting helpers: 10 min
- Section components: 30 min
- Integration: 10 min
- Testing: 15 min

**Total: ~70 minutes**

---

## Notes

- This is a pure frontend refactor, no backend changes
- No API changes required
- Existing helper functions are reused
- Layout is responsive by using grid with 2 columns
- Future enhancement: Could extract to reusable component if needed elsewhere

