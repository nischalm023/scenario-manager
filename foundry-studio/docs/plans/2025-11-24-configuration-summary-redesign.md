# Configuration Summary Redesign

## Overview

Redesign the Configuration Summary section in ResultsOverview to use a cleaner, more organized layout with vertical sections and two-column key-value pairs within each section.

### Goals
1. **Better organization**: Group related fields logically (Simulation, Infrastructure, Optimization)
2. **Clearer hierarchy**: Vertical sections instead of side-by-side columns
3. **Consistent layout**: All sections use same 2-column key-value pattern
4. **Better readability**: More whitespace, clearer labels, aligned values
5. **Complete information**: Show all relevant configuration parameters

## Current State

The current implementation (`ResultsOverview.tsx:71-136`) uses:
- A 2-column grid at the top level (Simulation Settings | Infrastructure)
- Mixed concerns in each column
- A separate bottom row for Warehouse Size and Operator Time
- Inconsistent grouping that doesn't match the ConfigurationPanel mental model

### Data Structure

Working with `SimulationConfig` from backend (nested structure):
```typescript
interface SimulationConfig {
  systemType: 'rtp' | 'relay'
  simulationDuration: number
  scenarioId: string
  enableSnapshots: boolean
  snapshotIntervalSeconds?: number
  
  mapConfig?: {
    gridWidth: number
    gridDepth: number
    cellSizeMeters: number
    stationLayout?: {
      totalStations: number
      // ...
    }
    // ...
  }
  
  robotConfig?: RtpRobotConfig | RelayRobotConfig
  operatorConfig?: {
    operatorTimePerPick: number
    targetPicksPerFace: number
    numBins: number
  }
  
  algorithmConfig?: {
    orderSelectionAlgorithm: 'maximize_ppf_directly' | 'maximize_ppf_with_tiering' | 'maximize_tpq_with_vut'
    putAlgorithm: string
    navFactor: number
    tpqConfig?: {
      targetTPQ: number
    }
    // ...
  }
}
```

## Design

### Structure

Three vertical sections, each containing a 2-column grid of key-value pairs:

```
┌───────────────────────────────────────────────────────────────┐
│ Simulation                                                    │
│   System ............. RTP    │   Duration .............. 4h  │
│   Scenario ...... scenario_a  │   Snapshots ............ 10s  │
├───────────────────────────────────────────────────────────────┤
│ Infrastructure                                                │
│   Grid ............. 22×26    │   Robots ................. 8  │
│   Stations .............. 4   │   Robot Speed ........ 1.5m/s │
│   Bins .................. 8   │   Cell Size ........... 1.0m  │
├───────────────────────────────────────────────────────────────┤
│ Optimization                                                  │
│   Pick Time ........... 12s   │   Put Algorithm ...... greedy │
│   Target PPF .......... 3.0   │   Order Algo ... maximize_tpq │
│   Target TPQ ......... 0.85   │   Nav Factor ............ 1.8 │
└───────────────────────────────────────────────────────────────┘
```

### Section Contents

**Simulation**
| Field | Source |
|-------|--------|
| System | `config.systemType` (RTP or Relay) |
| Scenario | `config.scenarioId` |
| Duration | `config.simulationDuration` (formatted as hours, e.g., "4h") |
| Snapshots | `config.snapshotIntervalSeconds + "s"` or "Disabled" if `!config.enableSnapshots` |

**Infrastructure**
| Field | Source |
|-------|--------|
| Grid | `config.mapConfig?.gridWidth` × `config.mapConfig?.gridDepth` |
| Stations | `config.mapConfig?.stationLayout?.totalStations` |
| Bins | `config.operatorConfig?.numBins` |
| Robots | `getRobotCount(config)` (handles RTP vs Relay) |
| Robot Speed | `getRobotSpeed(config)` + "m/s" (formatted to 1 decimal) |
| Cell Size | `config.mapConfig?.cellSizeMeters` + "m" (formatted to 1 decimal) |

**Optimization**
| Field | Source |
|-------|--------|
| Pick Time | `config.operatorConfig?.operatorTimePerPick` + "s" |
| Target PPF | `config.operatorConfig?.targetPicksPerFace` (formatted to 1 decimal) |
| Target TPQ | `config.algorithmConfig?.tpqConfig?.targetTPQ` (formatted to 2 decimals, or "N/A" if not set) |
| Put Algorithm | `config.algorithmConfig?.putAlgorithm` (capitalize first letter) |
| Order Algo | `config.algorithmConfig?.orderSelectionAlgorithm` (display friendly name) |
| Nav Factor | `config.algorithmConfig?.navFactor` (formatted to 1 decimal) |

### Relay System Variations

For relay systems, Infrastructure section shows different robot fields:

**RTP System:**
- Robots: `(config.robotConfig as RtpRobotConfig).numRobots`
- Robot Speed: `(config.robotConfig as RtpRobotConfig).robotSpeed` + "m/s"

**Relay System:**
- VTMs: `(config.robotConfig as RelayRobotConfig).numVtms`
- HTMs: `(config.robotConfig as RelayRobotConfig).numHtms`
- VTM Speed: `(config.robotConfig as RelayRobotConfig).vtmHorizontalSpeed` + "m/s"
- HTM Speed: `(config.robotConfig as RelayRobotConfig).htmSpeed` + "m/s"

This adds 2 more rows to Infrastructure for relay (4 robot fields instead of 2).

**Note:** Use the helper functions `getRobotCount(config)` and `getRobotSpeed(config)` from `type-guards.ts` to abstract the system type checking.

### Component Structure

**Option 1: Inline Implementation** (Recommended for first pass)
- Keep the configuration summary inline within `ResultsOverview.tsx`
- Build sections directly using conditional logic for RTP vs Relay
- Simplest approach since it's only used in one place

**Option 2: Reusable Component** (If needed later)
```typescript
// ConfigSummarySection - renders one section with title and 2-col grid
interface ConfigSummaryItem {
  label: string
  value: string | number
}

interface ConfigSummarySectionProps {
  title: string
  items: ConfigSummaryItem[]
}
```

For initial implementation, use **Option 1** to keep code localized and easy to iterate on.

### Visual Design

**Layout approach:**
- Use div-based grid layouts (not tables) for better flexibility
- Each section: vertical stack with header + 2-column grid for items
- 2-column grid within each section for key-value pairs

**Styling:**
- Container: `bg-gray-50 rounded-lg p-6` (matches current)
- Section headers: `text-sm font-medium text-gray-700 mb-3`
- Item grid: `grid grid-cols-2 gap-x-6 gap-y-3`
- Labels: `text-sm text-gray-500`
- Values: `text-sm font-medium text-gray-900` aligned right
- Sections separated by: `pt-4 border-t border-gray-200` (except first)

### Algorithm Display Names

**Order Selection Algorithm** (displayed as "Order Algo"):
- `maximize_ppf_directly` → "Maximize PPF (Direct)"
- `maximize_ppf_with_tiering` → "Maximize PPF (Tiered)"
- `maximize_tpq_with_vut` → "Maintain TPQ"

**Put Algorithm** (displayed as-is, capitalized):
- `greedy` → "Greedy"
- `min_weighted_cycle_distance` → "Min Weighted Cycle"
- Other values: capitalize first letter

## Implementation

### Files to Modify

1. `apps/frontend/src/pages/simulator/components/ResultsOverview.tsx`
   - Replace Configuration Summary section (lines 71-136)
   - Build three vertical sections with 2-column grids
   - Handle RTP vs Relay variations conditionally
   - Format values consistently (decimals, units)

### Helper Functions to Use

From `apps/frontend/src/types/type-guards.ts`:
- `getRobotCount(config: SimulationConfig): number`
- `getRobotSpeed(config: SimulationConfig): number`

### Value Formatting Helpers

Create inline formatting functions:
```typescript
const formatDuration = (hours: number) => `${hours}h`
const formatSpeed = (speed: number) => `${speed.toFixed(1)}m/s`
const formatSize = (size: number) => `${size.toFixed(1)}m`
const formatTPQ = (tpq?: number) => tpq ? tpq.toFixed(2) : 'N/A'
const formatSnapshot = (enabled: boolean, interval?: number) => 
  enabled && interval ? `${interval}s` : 'Disabled'
const formatAlgorithm = (algo: string) => {
  const map = {
    'maximize_ppf_directly': 'Maximize PPF (Direct)',
    'maximize_ppf_with_tiering': 'Maximize PPF (Tiered)',
    'maximize_tpq_with_vut': 'Maintain TPQ',
  }
  return map[algo] || algo
}
const capitalizePutAlgo = (algo: string) => {
  if (algo === 'min_weighted_cycle_distance') return 'Min Weighted Cycle'
  return algo.charAt(0).toUpperCase() + algo.slice(1)
}
```

### Edge Cases to Handle

1. **Missing optional fields**: Use optional chaining (`?.`) and fallbacks
2. **TPQ not configured**: Display "N/A" when `tpqConfig` is undefined
3. **Snapshots disabled**: Show "Disabled" instead of interval
4. **Unknown algorithm**: Fall back to displaying raw value
5. **Relay vs RTP**: Conditionally render different robot fields

### Optional: Extract Component

If reuse is needed elsewhere, extract to:
`apps/frontend/src/components/common/display/ConfigSummary.tsx`

For now, keep inline in ResultsOverview since it's only used there.

## Testing

### Manual Testing Scenarios

1. **RTP System**
   - Run RTP simulation with default config
   - Verify: System shows "RTP", Robots count and Robot Speed display
   - Check all three sections render with proper spacing

2. **Relay System**
   - Switch to Relay system and run simulation
   - Verify: System shows "Relay", VTMs/HTMs/speeds display instead of Robot fields
   - Confirm 4 robot fields shown (not 2)

3. **Snapshots**
   - Run with snapshots enabled (10s interval) → verify "10s" displays
   - Run with snapshots disabled → verify "Disabled" displays

4. **Algorithms**
   - Test each order selection algorithm:
     - `maximize_ppf_directly` → "Maximize PPF (Direct)"
     - `maximize_ppf_with_tiering` → "Maximize PPF (Tiered)"
     - `maximize_tpq_with_vut` → "Maintain TPQ"
   - Test put algorithms display properly capitalized

5. **TPQ Configuration**
   - Run with TPQ enabled → verify decimal formatting (2 places)
   - Run without TPQ config → verify "N/A" displays

6. **Visual Layout**
   - Verify section headers have proper styling
   - Check key-value pairs align properly (label left, value right)
   - Confirm sections separated by subtle borders
   - Test that container matches existing `bg-gray-50` style

7. **Edge Cases**
   - Very long scenario names
   - Decimal values formatting consistency
   - Missing/undefined config values
