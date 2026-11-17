# Map & Visualization System

Detailed reference for the warehouse map canvas. The main skill file (SKILL.md) has a summary — this file has implementation details.

## Architecture

The warehouse map uses **react-konva** (Konva.js canvas library), not a geographic map library.

```
WarehouseMap.tsx (885 lines)
├── Konva Stage (canvas element)
│   └── Layer (main rendering layer)
│       ├── Inline static elements: grid lines, distance markers, racks, highways, legend
│       └── Delegated layer components (conditionally rendered):
│           ├── CellOccupancyLayer      — heatmap of robot cell visits (amber gradient)
│           ├── CombinedOverlayLayer    — cell occupancy fill + conflict factor border
│           ├── ConflictFactorLayer     — directional flow conflict (blue→red)
│           ├── RelayPositionsLayer     — relay positions with tote occupancy status
│           ├── RobotsLayer             — animated robot positions (circles)
│           ├── StationsLayer           — animated station states
│           └── StorageSpacesLayer      — animated rack/tote positions with inventory
└── HTML tooltip overlays (absolutely positioned <div>s outside Konva)
```

All layer components live in `components/common/maps/layers/` and are re-exported from `layers/index.ts`.

## Coordinate System

**Backend** uses bottom-left origin (Y=0 at bottom, standard math coordinates).
**Canvas** uses top-left origin (Y=0 at top, standard screen coordinates).

Transform functions in WarehouseMap.tsx:
```
transformX(x):  gridX = x / cellSizeMeters  →  gridStartX + gridX * effectiveGridSize
transformY(y):  gridY = y / cellSizeMeters  →  gridStartY + (gridDepth - gridY) * effectiveGridSize
```

- `cellSizeMeters` comes from `mapConfig` (default 1.0)
- `effectiveGridSize = Math.min(gridSizeX, gridSizeY) * 0.95` — fits grid to canvas with 5% margin
- Canvas has padding=20, markerSpace=25 (if markers shown), legendSpace=50 (if legend shown)

**Important**: When adding new coordinate transforms, always apply the Y-flip (`gridDepth - gridY`). Forgetting this will render everything upside-down.

## Visualization Modes

Defined in `types/visualization.ts` as union type `VisualizationMode`:

| Mode | Gradient | Systems | Rendering |
|------|----------|---------|-----------|
| `inventory` | Blue | RTP, Relay | StorageSpacesLayer — total inventory per rack |
| `picking_tasks` | Orange | RTP, Relay | StorageSpacesLayer — active task counts |
| `relay_occupancy` | Green/Red | Relay only | RelayPositionsLayer — tote present/absent |
| `cell_occupancy` | Amber | RTP, Relay | CellOccupancyLayer — robot visit heatmap |
| `conflict_factor` | Blue→Red | RTP, Relay | ConflictFactorLayer — flow direction conflicts |
| `combined_overlay` | Both | RTP, Relay | CombinedOverlayLayer — occupancy fill + conflict border |

The last three (`cell_occupancy`, `conflict_factor`, `combined_overlay`) require grid metrics to be enabled in the simulation config. `VisualizationModeSelector` disables them when `gridMetricsAvailable === false`.

## Layer Component Details

Each layer component receives coordinate transform functions and snapshot data as props.

| Component | Rendered When | Key Props |
|-----------|--------------|-----------|
| `CellOccupancyLayer` | `visualizationMode === 'cell_occupancy'` + data present | `cellOccupancies`, transform fns, grid dimensions |
| `CombinedOverlayLayer` | `visualizationMode === 'combined_overlay'` + both datasets | `cellOccupancies`, `cellConflictFactors`, transform fns |
| `ConflictFactorLayer` | `visualizationMode === 'conflict_factor'` + data present | `cellConflictFactors`, transform fns |
| `RelayPositionsLayer` | snapshot has `.relays` + relay mode | `relays[]`, transform fns, `onRelayHover` callback |
| `RobotsLayer` | snapshot has `.robots` | `robots[]`, transform fns, color per type |
| `StationsLayer` | snapshot has `.stations` | `stations[]`, transform fns, utilization colors |
| `StorageSpacesLayer` | snapshot has `.storageSpaces` + not relay mode | `storageSpaces[]`, transform fns, `onStorageSpaceHover` |

## Performance Patterns

1. **`isLargeGrid`** flag: `gridWidth * gridDepth > 1000`
   - Reduces distance marker density
   - Smaller font for axis labels
   - Layer-level `listening={false}` to skip hit detection

2. **Static element optimizations** (all inline racks, grid lines, etc.):
   - `perfectDrawEnabled={false}` — skips anti-aliasing buffer (faster)
   - `listening={false}` — skips event hit testing
   - Shadows disabled when `racks.length >= 1000`

3. **Memoization**:
   - `rackColors` memoized on `racks` array (popularity-based orange gradient)
   - Grid lines, distance markers, highway elements, legend all wrapped in `useMemo`
   - Component exported as `memo(WarehouseMapComponent)`

4. **Color constants**: Defined in a `COLORS` object at file top — consistent across all rendering modes.

## Tooltip Pattern

Konva can't use DOM tooltips (it renders to `<canvas>`). Tooltips are implemented as absolutely-positioned HTML `<div>` elements rendered outside the Konva `Stage`, positioned using the mouse coordinates from Konva events.

Two tooltip types:
- **Storage space tooltip**: rack ID, position, tote count, inventory, reserved inventory, active status
- **Relay position tooltip**: relay ID, position, owner zone ID, tote presence/ID

Both triggered via `onStorageSpaceHover` / `onRelayHover` callbacks from layer components.
