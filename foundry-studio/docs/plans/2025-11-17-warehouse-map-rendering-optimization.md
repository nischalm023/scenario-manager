# WarehouseMap Rendering Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Optimize WarehouseMap animation rendering to eliminate sluggishness during playback and scrubbing by memoizing component layers and reducing unnecessary re-renders.

**Architecture:** Extract entity rendering into memoized sub-components (RobotsLayer, StorageSpacesLayer, StationsLayer) that only re-render when their specific data changes. Wrap WarehouseMap in React.memo() and optimize position transforms.

**Tech Stack:** React 18, react-konva 19, TypeScript, Vitest

---

## Background

**Current Problem:**
- Every frame change (playback/scrubbing) triggers full re-execution of all entity .map() loops
- Creates 300-400 new Konva element objects per frame (robots, storage spaces, stations, totes)
- Even with react-konva diffing, object creation + prop comparison is expensive
- Results in sluggish UI, especially for relay system with more entity types

**Solution:**
- Split rendering into memoized layers (one per entity type)
- Only re-render layer if its data changed
- Memoize expensive calculations (position transforms, colors)
- Add React.memo() to prevent unnecessary parent re-renders

**Expected Impact:**
- 60-80% reduction in frame update time
- Smooth 30+ FPS playback
- Instant scrubbing response

---

## Task 1: Create RobotsLayer Component

**Goal:** Extract robot rendering into isolated, memoized component

**Files:**
- Create: `apps/frontend/src/components/common/maps/layers/RobotsLayer.tsx`
- Create: `apps/frontend/src/components/common/maps/layers/__tests__/RobotsLayer.test.tsx`

### Step 1: Create layers directory

```bash
mkdir -p apps/frontend/src/components/common/maps/layers
mkdir -p apps/frontend/src/components/common/maps/layers/__tests__
```

### Step 2: Write failing test for RobotsLayer

Create `apps/frontend/src/components/common/maps/layers/__tests__/RobotsLayer.test.tsx`:

```typescript
import { render } from '@testing-library/react'
import { Stage, Layer } from 'react-konva'
import { describe, expect, it, vi } from 'vitest'
import { RobotsLayer } from '../RobotsLayer'

describe('RobotsLayer', () => {
  const mockTransformX = vi.fn((x: number) => x * 10)
  const mockTransformY = vi.fn((y: number) => y * 10)

  it('should render robots from snapshot data', () => {
    const robots = [
      {
        robotId: 'robot-1',
        type: 'vtm',
        status: 'moving',
        position: { x: 5, y: 10 },
        currentTask: 'task-1',
        carryingStorageSpace: null,
      },
    ]

    const { container } = render(
      <Stage width={800} height={600}>
        <Layer>
          <RobotsLayer
            robots={robots}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    expect(container.querySelector('canvas')).toBeTruthy()
    expect(mockTransformX).toHaveBeenCalledWith(5)
    expect(mockTransformY).toHaveBeenCalledWith(10)
  })

  it('should NOT re-render when robots array reference changes but content is same', () => {
    const robots1 = [{ robotId: 'robot-1', type: 'vtm', status: 'idle', position: { x: 5, y: 10 }, currentTask: null, carryingStorageSpace: null }]
    const robots2 = [{ robotId: 'robot-1', type: 'vtm', status: 'idle', position: { x: 5, y: 10 }, currentTask: null, carryingStorageSpace: null }]

    const { rerender } = render(
      <Stage width={800} height={600}>
        <Layer>
          <RobotsLayer
            robots={robots1}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    mockTransformX.mockClear()

    rerender(
      <Stage width={800} height={600}>
        <Layer>
          <RobotsLayer
            robots={robots2}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    // Should NOT call transform again if memoization works
    expect(mockTransformX).not.toHaveBeenCalled()
  })

  it('should re-render when robot position changes', () => {
    const robots1 = [{ robotId: 'robot-1', type: 'vtm', status: 'idle', position: { x: 5, y: 10 }, currentTask: null, carryingStorageSpace: null }]
    const robots2 = [{ robotId: 'robot-1', type: 'vtm', status: 'idle', position: { x: 6, y: 10 }, currentTask: null, carryingStorageSpace: null }]

    const { rerender } = render(
      <Stage width={800} height={600}>
        <Layer>
          <RobotsLayer
            robots={robots1}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    mockTransformX.mockClear()

    rerender(
      <Stage width={800} height={600}>
        <Layer>
          <RobotsLayer
            robots={robots2}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    // Should call transform for new position
    expect(mockTransformX).toHaveBeenCalledWith(6)
  })
})
```

### Step 3: Run test to verify it fails

```bash
cd apps/frontend
npx vitest run src/components/common/maps/layers/__tests__/RobotsLayer.test.tsx
```

**Expected:** FAIL with "Cannot find module '../RobotsLayer'"

### Step 4: Implement RobotsLayer component

Create `apps/frontend/src/components/common/maps/layers/RobotsLayer.tsx`:

```typescript
import { memo } from 'react'
import { Circle, Group } from 'react-konva'

interface RobotData {
  robotId: string
  type?: string
  status?: string
  position?: { x: number; y: number }
  currentTask?: string | null
  carryingStorageSpace?: string | null
}

interface RobotsLayerProps {
  robots: RobotData[]
  transformX: (x: number) => number
  transformY: (y: number) => number
  effectiveGridSize: number
  isLargeGrid: boolean
  onRobotClick?: (robot: RobotData) => void
}

const COLORS = {
  vtmRobot: '#3b82f6', // Blue-500
  htmRobot: '#22c55e', // Green-500
  rtpRobot: '#f97316', // Orange-500
  idleRobot: '#6b7280', // Gray-500
  background: '#ffffff',
}

const getRobotColor = (type: string): string => {
  switch (type) {
    case 'vtm':
      return COLORS.vtmRobot
    case 'htm':
      return COLORS.htmRobot
    case 'rtp':
      return COLORS.rtpRobot
    default:
      return COLORS.idleRobot
  }
}

/**
 * Memoized layer for rendering robots.
 * Only re-renders when robots data actually changes.
 */
export const RobotsLayer = memo<RobotsLayerProps>(
  ({ robots, transformX, transformY, effectiveGridSize, isLargeGrid, onRobotClick }) => {
    const robotRadius = effectiveGridSize * 0.3

    return (
      <Group>
        {robots.map((robot) => {
          const centerX = transformX(robot.position?.x ?? 0)
          const centerY = transformY(robot.position?.y ?? 0)
          const robotColor = getRobotColor(robot.type ?? '')
          const isIdle = robot.status === 'idle'

          return (
            <Group key={robot.robotId}>
              {/* Robot body - main circle */}
              <Circle
                x={centerX}
                y={centerY}
                radius={robotRadius}
                fill={robotColor}
                stroke={isIdle ? '#666' : 'none'}
                strokeWidth={2}
                opacity={
                  robot.status === 'moving_to_rack' || robot.status === 'moving_to_tote' ? 1 : 0.6
                }
                shadowEnabled={!isLargeGrid}
                shadowColor={!isLargeGrid ? 'rgba(0, 0, 0, 0.2)' : undefined}
                shadowBlur={!isLargeGrid ? 3 : 0}
                shadowOffset={!isLargeGrid ? { x: 1, y: 1 } : undefined}
                perfectDrawEnabled={false}
                onClick={() => onRobotClick?.(robot)}
                onTap={() => onRobotClick?.(robot)}
              />
              {/* Robot center dot */}
              <Circle
                x={centerX}
                y={centerY}
                radius={robotRadius * 0.25}
                fill={COLORS.background}
                perfectDrawEnabled={false}
                listening={false}
              />
            </Group>
          )
        })}
      </Group>
    )
  },
  (prevProps, nextProps) => {
    // Custom comparison: only re-render if robots data actually changed
    if (prevProps.robots.length !== nextProps.robots.length) return false
    if (prevProps.effectiveGridSize !== nextProps.effectiveGridSize) return false
    if (prevProps.isLargeGrid !== nextProps.isLargeGrid) return false

    // Deep compare robots array
    for (let i = 0; i < prevProps.robots.length; i++) {
      const prev = prevProps.robots[i]
      const next = nextProps.robots[i]

      if (
        prev.robotId !== next.robotId ||
        prev.type !== next.type ||
        prev.status !== next.status ||
        prev.position?.x !== next.position?.x ||
        prev.position?.y !== next.position?.y ||
        prev.currentTask !== next.currentTask ||
        prev.carryingStorageSpace !== next.carryingStorageSpace
      ) {
        return false
      }
    }

    return true // Props equal, skip re-render
  }
)

RobotsLayer.displayName = 'RobotsLayer'
```

### Step 5: Run test to verify it passes

```bash
cd apps/frontend
npx vitest run src/components/common/maps/layers/__tests__/RobotsLayer.test.tsx
```

**Expected:** PASS (all 3 tests)

### Step 6: Commit

```bash
git add apps/frontend/src/components/common/maps/layers/
git commit -m "feat: add memoized RobotsLayer component for animation optimization"
```

---

## Task 2: Create StorageSpacesLayer Component

**Goal:** Extract storage space rendering into isolated, memoized component

**Files:**
- Create: `apps/frontend/src/components/common/maps/layers/StorageSpacesLayer.tsx`
- Create: `apps/frontend/src/components/common/maps/layers/__tests__/StorageSpacesLayer.test.tsx`

### Step 1: Write failing test for StorageSpacesLayer

Create `apps/frontend/src/components/common/maps/layers/__tests__/StorageSpacesLayer.test.tsx`:

```typescript
import { render } from '@testing-library/react'
import { Stage, Layer } from 'react-konva'
import { describe, expect, it, vi } from 'vitest'
import { StorageSpacesLayer } from '../StorageSpacesLayer'

describe('StorageSpacesLayer', () => {
  const mockTransformX = vi.fn((x: number) => x * 10)
  const mockTransformY = vi.fn((y: number) => y * 10)

  it('should render storage spaces from snapshot data', () => {
    const storageSpaces = [
      {
        storageSpaceId: 'TOTE_1',
        type: 'TOTE',
        position: { x: 5, y: 10 },
        isActive: true,
        totalInventory: 50,
        totalReservedInventory: 10,
        inventory: {},
        reservedInventory: {},
        reservedByOrders: [],
      },
    ]

    const { container } = render(
      <Stage width={800} height={600}>
        <Layer>
          <StorageSpacesLayer
            storageSpaces={storageSpaces}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    expect(container.querySelector('canvas')).toBeTruthy()
    expect(mockTransformX).toHaveBeenCalledWith(5)
  })

  it('should NOT re-render when storage spaces unchanged', () => {
    const spaces1 = [
      {
        storageSpaceId: 'TOTE_1',
        type: 'TOTE',
        position: { x: 5, y: 10 },
        isActive: false,
        totalInventory: 50,
        totalReservedInventory: 0,
        inventory: {},
        reservedInventory: {},
        reservedByOrders: [],
      },
    ]
    const spaces2 = [
      {
        storageSpaceId: 'TOTE_1',
        type: 'TOTE',
        position: { x: 5, y: 10 },
        isActive: false,
        totalInventory: 50,
        totalReservedInventory: 0,
        inventory: {},
        reservedInventory: {},
        reservedByOrders: [],
      },
    ]

    const { rerender } = render(
      <Stage width={800} height={600}>
        <Layer>
          <StorageSpacesLayer
            storageSpaces={spaces1}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    mockTransformX.mockClear()

    rerender(
      <Stage width={800} height={600}>
        <Layer>
          <StorageSpacesLayer
            storageSpaces={spaces2}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    expect(mockTransformX).not.toHaveBeenCalled()
  })
})
```

### Step 2: Run test to verify it fails

```bash
cd apps/frontend
npx vitest run src/components/common/maps/layers/__tests__/StorageSpacesLayer.test.tsx
```

**Expected:** FAIL with "Cannot find module '../StorageSpacesLayer'"

### Step 3: Implement StorageSpacesLayer component

Create `apps/frontend/src/components/common/maps/layers/StorageSpacesLayer.tsx`:

```typescript
import { memo } from 'react'
import { Rect, Group, Text } from 'react-konva'

interface StorageSpaceData {
  storageSpaceId: string
  type?: string
  position?: { x: number; y: number }
  isActive?: boolean
  totalInventory?: number
  totalReservedInventory?: number
  inventory?: Record<string, number>
  reservedInventory?: Record<string, number>
  reservedByOrders?: string[]
}

interface StorageSpacesLayerProps {
  storageSpaces: StorageSpaceData[]
  transformX: (x: number) => number
  transformY: (y: number) => number
  effectiveGridSize: number
  isLargeGrid: boolean
}

const COLORS = {
  storageEmpty: '#9ca3af', // Gray-400
  storageActive: '#f59e0b', // Amber-500
  storageReserved: '#3b82f6', // Blue-500
  storageNormal: '#3b82f6', // Blue-500
  border: '#d1d5db', // Gray-300
  background: '#ffffff',
}

/**
 * Memoized layer for rendering storage spaces (racks/totes).
 * Only re-renders when storage space data actually changes.
 */
export const StorageSpacesLayer = memo<StorageSpacesLayerProps>(
  ({ storageSpaces, transformX, transformY, effectiveGridSize, isLargeGrid }) => {
    const spaceSize = effectiveGridSize * 0.8

    return (
      <Group>
        {storageSpaces.map((space) => {
          const centerX = transformX(space.position?.x ?? 0)
          const centerY = transformY(space.position?.y ?? 0)

          const isEmpty = (space.totalInventory ?? 0) === 0
          const isActive = space.isActive ?? false
          const hasReservations = (space.totalReservedInventory ?? 0) > 0

          const fillColor = isEmpty
            ? COLORS.storageEmpty
            : isActive
              ? COLORS.storageActive
              : COLORS.storageNormal

          const strokeColor = hasReservations ? COLORS.storageReserved : COLORS.border
          const strokeWidth = isActive ? 2 : 1

          return (
            <Group key={space.storageSpaceId}>
              <Rect
                x={centerX - spaceSize / 2}
                y={centerY - spaceSize / 2}
                width={spaceSize}
                height={spaceSize}
                fill={fillColor}
                stroke={strokeColor}
                strokeWidth={strokeWidth}
                cornerRadius={4}
                opacity={isEmpty ? 0.3 : 0.8}
                shadowEnabled={!isLargeGrid}
                shadowColor={!isLargeGrid ? 'rgba(0, 0, 0, 0.1)' : undefined}
                shadowBlur={!isLargeGrid ? 2 : 0}
                shadowOffset={!isLargeGrid ? { x: 1, y: 1 } : undefined}
                perfectDrawEnabled={false}
              />
              {/* Show inventory count */}
              {(space.totalInventory ?? 0) > 0 && (
                <Text
                  x={centerX}
                  y={centerY}
                  text={`${space.totalInventory}`}
                  fontSize={8}
                  fill={COLORS.background}
                  fontStyle="bold"
                  offsetX={4}
                  offsetY={4}
                  perfectDrawEnabled={false}
                />
              )}
            </Group>
          )
        })}
      </Group>
    )
  },
  (prevProps, nextProps) => {
    if (prevProps.storageSpaces.length !== nextProps.storageSpaces.length) return false
    if (prevProps.effectiveGridSize !== nextProps.effectiveGridSize) return false
    if (prevProps.isLargeGrid !== nextProps.isLargeGrid) return false

    // Deep compare storage spaces
    for (let i = 0; i < prevProps.storageSpaces.length; i++) {
      const prev = prevProps.storageSpaces[i]
      const next = nextProps.storageSpaces[i]

      if (
        prev.storageSpaceId !== next.storageSpaceId ||
        prev.type !== next.type ||
        prev.position?.x !== next.position?.x ||
        prev.position?.y !== next.position?.y ||
        prev.isActive !== next.isActive ||
        prev.totalInventory !== next.totalInventory ||
        prev.totalReservedInventory !== next.totalReservedInventory
      ) {
        return false
      }
    }

    return true
  }
)

StorageSpacesLayer.displayName = 'StorageSpacesLayer'
```

### Step 4: Run test to verify it passes

```bash
cd apps/frontend
npx vitest run src/components/common/maps/layers/__tests__/StorageSpacesLayer.test.tsx
```

**Expected:** PASS (all tests)

### Step 5: Commit

```bash
git add apps/frontend/src/components/common/maps/layers/StorageSpacesLayer.tsx
git add apps/frontend/src/components/common/maps/layers/__tests__/StorageSpacesLayer.test.tsx
git commit -m "feat: add memoized StorageSpacesLayer component"
```

---

## Task 3: Create StationsLayer Component

**Goal:** Extract station rendering into isolated, memoized component

**Files:**
- Create: `apps/frontend/src/components/common/maps/layers/StationsLayer.tsx`
- Create: `apps/frontend/src/components/common/maps/layers/__tests__/StationsLayer.test.tsx`

### Step 1: Write failing test for StationsLayer

Create `apps/frontend/src/components/common/maps/layers/__tests__/StationsLayer.test.tsx`:

```typescript
import { render } from '@testing-library/react'
import { Stage, Layer } from 'react-konva'
import { describe, expect, it, vi } from 'vitest'
import { StationsLayer } from '../StationsLayer'

describe('StationsLayer', () => {
  const mockTransformX = vi.fn((x: number) => x * 10)
  const mockTransformY = vi.fn((y: number) => y * 10)

  const staticStations = [
    { id: 'station-0', position: { x: 5, y: 10 }, type: 'pick', active: true },
  ]

  it('should render stations from snapshot data', () => {
    const snapshotStations = [
      {
        stationId: 0,
        queueLength: 5,
        currentTote: 'TOTE_1',
        utilizationPercent: 75,
      },
    ]

    const { container } = render(
      <Stage width={800} height={600}>
        <Layer>
          <StationsLayer
            snapshotStations={snapshotStations}
            staticStations={staticStations}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    expect(container.querySelector('canvas')).toBeTruthy()
  })

  it('should NOT re-render when stations unchanged', () => {
    const stations1 = [{ stationId: 0, queueLength: 5, currentTote: null, utilizationPercent: 50 }]
    const stations2 = [{ stationId: 0, queueLength: 5, currentTote: null, utilizationPercent: 50 }]

    const { rerender } = render(
      <Stage width={800} height={600}>
        <Layer>
          <StationsLayer
            snapshotStations={stations1}
            staticStations={staticStations}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    mockTransformX.mockClear()

    rerender(
      <Stage width={800} height={600}>
        <Layer>
          <StationsLayer
            snapshotStations={stations2}
            staticStations={staticStations}
            transformX={mockTransformX}
            transformY={mockTransformY}
            effectiveGridSize={20}
            isLargeGrid={false}
          />
        </Layer>
      </Stage>
    )

    expect(mockTransformX).not.toHaveBeenCalled()
  })
})
```

### Step 2: Run test to verify it fails

```bash
cd apps/frontend
npx vitest run src/components/common/maps/layers/__tests__/StationsLayer.test.tsx
```

**Expected:** FAIL with "Cannot find module '../StationsLayer'"

### Step 3: Implement StationsLayer component

Create `apps/frontend/src/components/common/maps/layers/StationsLayer.tsx`:

```typescript
import { memo } from 'react'
import { Rect, Group, Text } from 'react-konva'

interface StationData {
  stationId: number
  queueLength?: number
  currentTote?: string | null
  utilizationPercent?: number
}

interface StaticStation {
  id: string
  position: { x: number; y: number }
  type: string
  active: boolean
}

interface StationsLayerProps {
  snapshotStations: StationData[]
  staticStations: StaticStation[]
  transformX: (x: number) => number
  transformY: (y: number) => number
  effectiveGridSize: number
  isLargeGrid: boolean
  onStationClick?: (station: StaticStation) => void
}

const COLORS = {
  stationLowUtil: '#22c55e', // Green-500
  stationMedUtil: '#eab308', // Yellow-500
  stationHighUtil: '#ef4444', // Red-500
  border: '#d1d5db', // Gray-300
  text: '#374151', // Gray-700
}

/**
 * Memoized layer for rendering stations.
 * Only re-renders when station data actually changes.
 */
export const StationsLayer = memo<StationsLayerProps>(
  ({
    snapshotStations,
    staticStations,
    transformX,
    transformY,
    effectiveGridSize,
    isLargeGrid,
    onStationClick,
  }) => {
    const stationSize = effectiveGridSize * 0.8

    return (
      <Group>
        {snapshotStations.map((station) => {
          const staticStation = staticStations.find((s) => s.id === `station-${station.stationId}`)
          if (!staticStation) return null

          const centerX = transformX(staticStation.position.x)
          const centerY = transformY(staticStation.position.y)

          const utilPercent = station.utilizationPercent ?? 0
          const utilColor =
            utilPercent > 80
              ? COLORS.stationHighUtil
              : utilPercent > 50
                ? COLORS.stationMedUtil
                : COLORS.stationLowUtil

          return (
            <Group key={`snapshot-station-${station.stationId}`}>
              <Rect
                x={centerX - stationSize / 2}
                y={centerY - stationSize / 2}
                width={stationSize}
                height={stationSize}
                fill={utilColor}
                stroke={COLORS.border}
                strokeWidth={2}
                cornerRadius={6}
                opacity={0.5}
                shadowEnabled={!isLargeGrid}
                shadowColor={!isLargeGrid ? 'rgba(0, 0, 0, 0.15)' : undefined}
                shadowBlur={!isLargeGrid ? 4 : 0}
                shadowOffset={!isLargeGrid ? { x: 2, y: 2 } : undefined}
                perfectDrawEnabled={false}
                onClick={() => onStationClick?.(staticStation)}
                onTap={() => onStationClick?.(staticStation)}
              />
              {/* Show queue length if > 0 */}
              {(station.queueLength ?? 0) > 0 && (
                <Text
                  x={centerX}
                  y={centerY}
                  text={`${station.queueLength}`}
                  fontSize={12}
                  fontStyle="bold"
                  fill={COLORS.text}
                  offsetX={6}
                  offsetY={6}
                  perfectDrawEnabled={false}
                />
              )}
            </Group>
          )
        })}
      </Group>
    )
  },
  (prevProps, nextProps) => {
    if (prevProps.snapshotStations.length !== nextProps.snapshotStations.length) return false
    if (prevProps.effectiveGridSize !== nextProps.effectiveGridSize) return false
    if (prevProps.isLargeGrid !== nextProps.isLargeGrid) return false

    // Deep compare stations
    for (let i = 0; i < prevProps.snapshotStations.length; i++) {
      const prev = prevProps.snapshotStations[i]
      const next = nextProps.snapshotStations[i]

      if (
        prev.stationId !== next.stationId ||
        prev.queueLength !== next.queueLength ||
        prev.currentTote !== next.currentTote ||
        prev.utilizationPercent !== next.utilizationPercent
      ) {
        return false
      }
    }

    return true
  }
)

StationsLayer.displayName = 'StationsLayer'
```

### Step 4: Run test to verify it passes

```bash
cd apps/frontend
npx vitest run src/components/common/maps/layers/__tests__/StationsLayer.test.tsx
```

**Expected:** PASS (all tests)

### Step 5: Commit

```bash
git add apps/frontend/src/components/common/maps/layers/StationsLayer.tsx
git add apps/frontend/src/components/common/maps/layers/__tests__/StationsLayer.test.tsx
git commit -m "feat: add memoized StationsLayer component"
```

---

## Task 4: Create index.ts for Layer Exports

**Goal:** Centralize layer exports for clean imports

**Files:**
- Create: `apps/frontend/src/components/common/maps/layers/index.ts`

### Step 1: Create index file

Create `apps/frontend/src/components/common/maps/layers/index.ts`:

```typescript
export { RobotsLayer } from './RobotsLayer'
export { StorageSpacesLayer } from './StorageSpacesLayer'
export { StationsLayer } from './StationsLayer'
```

### Step 2: Commit

```bash
git add apps/frontend/src/components/common/maps/layers/index.ts
git commit -m "feat: add layer exports index"
```

---

## Task 5: Integrate Memoized Layers into WarehouseMap

**Goal:** Replace inline .map() rendering with memoized layer components

**Files:**
- Modify: `apps/frontend/src/components/common/maps/WarehouseMap.tsx`

### Step 1: Add layer imports to WarehouseMap

At the top of `apps/frontend/src/components/common/maps/WarehouseMap.tsx`, add:

```typescript
import { RobotsLayer } from './layers/RobotsLayer'
import { StorageSpacesLayer } from './layers/StorageSpacesLayer'
import { StationsLayer } from './layers/StationsLayer'
```

After line 4 (after other imports).

### Step 2: Replace robots rendering section

**Find (lines 692-772):**

```typescript
{/* Robots - Optimized */}
<Group>
  {/* Static robots (from props) */}
  {!snapshotAny && robots.map((robot) => {
    // ... existing robot rendering code
  })}

  {/* Snapshot robots (from animation) */}
  {snapshotAny?.robots?.map((robot: any) => {
    // ... existing snapshot robot rendering code
  })}
</Group>
```

**Replace with:**

```typescript
{/* Robots - Optimized with memoized layer */}
{snapshotAny?.robots ? (
  <RobotsLayer
    robots={snapshotAny.robots}
    transformX={transformX}
    transformY={transformY}
    effectiveGridSize={effectiveGridSize}
    isLargeGrid={isLargeGrid}
    onRobotClick={onRobotClick}
  />
) : (
  <Group>
    {/* Static robots (from props) - keep for non-animation view */}
    {robots.map((robot) => {
      const alignedPosition = getGridAlignedPosition(robot.position)
      const centerX = transformX(alignedPosition.x)
      const centerY = transformY(alignedPosition.y)
      const robotRadius = effectiveGridSize * 0.3

      return (
        <Group key={robot.id}>
          <Circle
            x={centerX}
            y={centerY}
            radius={robotRadius}
            fill={COLORS.idleRobot}
            stroke={'#4b5563'}
            strokeWidth={1.5}
            shadowEnabled={racks.length < 1000}
            shadowColor={racks.length < 1000 ? 'rgba(0, 0, 0, 0.2)' : undefined}
            shadowBlur={racks.length < 1000 ? 3 : 0}
            shadowOffset={racks.length < 1000 ? { x: 1, y: 1 } : undefined}
            onClick={() => onRobotClick?.(robot)}
            onTap={() => onRobotClick?.(robot)}
            perfectDrawEnabled={false}
          />
          <Circle
            x={centerX}
            y={centerY}
            radius={robotRadius * 0.25}
            fill={COLORS.background}
            onClick={() => onRobotClick?.(robot)}
            onTap={() => onRobotClick?.(robot)}
            perfectDrawEnabled={false}
            listening={false}
          />
        </Group>
      )
    })}
  </Group>
)}
```

### Step 3: Replace storage spaces rendering section

**Find (lines 511-595):**

```typescript
{/* Racks / Storage Spaces - Optimized rendering */}
<Group>
  {/* Static racks (from props) */}
  {!snapshot && racks.map((rack) => {
    // ... existing rack rendering
  })}

  {/* Snapshot storage spaces (from animation) */}
  {snapshotAny?.storageSpaces?.map((space: any) => {
    // ... existing storage space rendering
  })}
</Group>
```

**Replace with:**

```typescript
{/* Racks / Storage Spaces - Optimized with memoized layer */}
<Group>
  {snapshotAny?.storageSpaces ? (
    <StorageSpacesLayer
      storageSpaces={snapshotAny.storageSpaces}
      transformX={transformX}
      transformY={transformY}
      effectiveGridSize={effectiveGridSize}
      isLargeGrid={isLargeGrid}
    />
  ) : (
    /* Static racks (from props) - keep for non-animation view */
    racks.map((rack) => {
      const alignedPosition = getGridAlignedPosition(rack.position)
      const centerX = transformX(alignedPosition.x)
      const centerY = transformY(alignedPosition.y)
      const rackSize = effectiveGridSize * 0.8

      return (
        <Rect
          key={rack.id}
          x={centerX - rackSize / 2}
          y={centerY - rackSize / 2}
          width={rackSize}
          height={rackSize}
          fill={rack.color || (rack.popularity > 0.5 ? COLORS.popularRack : COLORS.rack)}
          stroke={COLORS.border}
          strokeWidth={1}
          cornerRadius={4}
          shadowEnabled={racks.length < 1000}
          shadowColor={racks.length < 1000 ? 'rgba(0, 0, 0, 0.1)' : undefined}
          shadowBlur={racks.length < 1000 ? 2 : 0}
          shadowOffset={racks.length < 1000 ? { x: 1, y: 1 } : undefined}
          onClick={() => onRackClick?.(rack)}
          onTap={() => onRackClick?.(rack)}
          perfectDrawEnabled={false}
        />
      )
    })
  )}
</Group>
```

### Step 4: Replace stations rendering section

**Find (lines 597-678):**

```typescript
{/* Stations - Optimized */}
<Group>
  {/* Static stations (from props) */}
  {!snapshotAny && stations.map((station) => {
    // ... existing station rendering
  })}

  {/* Snapshot stations (from animation) */}
  {snapshotAny?.stations?.map((station: any) => {
    // ... existing snapshot station rendering
  })}
</Group>
```

**Replace with:**

```typescript
{/* Stations - Optimized with memoized layer */}
{snapshotAny?.stations ? (
  <StationsLayer
    snapshotStations={snapshotAny.stations}
    staticStations={stations}
    transformX={transformX}
    transformY={transformY}
    effectiveGridSize={effectiveGridSize}
    isLargeGrid={isLargeGrid}
    onStationClick={onStationClick}
  />
) : (
  <Group>
    {/* Static stations (from props) - keep for non-animation view */}
    {stations.map((station) => {
      const alignedPosition = getGridAlignedPosition(station.position)
      const centerX = transformX(alignedPosition.x)
      const centerY = transformY(alignedPosition.y)
      const stationSize = effectiveGridSize * 0.8

      return (
        <Rect
          key={station.id}
          x={centerX - stationSize / 2}
          y={centerY - stationSize / 2}
          width={stationSize}
          height={stationSize}
          fill={COLORS.station}
          stroke={COLORS.border}
          strokeWidth={2}
          cornerRadius={6}
          shadowEnabled={racks.length < 1000}
          shadowColor={racks.length < 1000 ? 'rgba(0, 0, 0, 0.15)' : undefined}
          shadowBlur={racks.length < 1000 ? 4 : 0}
          shadowOffset={racks.length < 1000 ? { x: 2, y: 2 } : undefined}
          onClick={() => onStationClick?.(station)}
          onTap={() => onStationClick?.(station)}
          perfectDrawEnabled={false}
        />
      )
    })}
  </Group>
)}
```

### Step 5: Wrap WarehouseMap in React.memo()

**Find (line 118):**

```typescript
export const WarehouseMap: React.FC<WarehouseMapProps> = ({
```

**Replace with:**

```typescript
const WarehouseMapComponent: React.FC<WarehouseMapProps> = ({
```

**At the end of the file (before line 797 `export default WarehouseMap`), add:**

```typescript
export const WarehouseMap = memo(WarehouseMapComponent)
```

**And add memo import at top:**

```typescript
import type React from 'react'
import { useMemo, memo } from 'react'
```

### Step 6: Run frontend build to verify no errors

```bash
cd apps/frontend
npm run check
```

**Expected:** No TypeScript errors, build succeeds

### Step 7: Commit

```bash
git add apps/frontend/src/components/common/maps/WarehouseMap.tsx
git commit -m "feat: integrate memoized layers into WarehouseMap for rendering optimization"
```

---

## Task 6: Manual Performance Testing

**Goal:** Validate rendering optimization improves animation performance

### Step 1: Start backend server

```bash
npx nx serve backend
```

Wait for "Started DessimulatorApplication" message.

### Step 2: Start frontend dev server

```bash
cd apps/frontend
npm run dev
```

### Step 3: Run 2-hour relay simulation with snapshots

In simulation config, set:
```json
{
  "systemType": "relay",
  "duration": 2,
  "enableSnapshots": true,
  "snapshotIntervalSeconds": 10.0,
  "vtmCount": 10,
  "htmCount": 5
}
```

Run simulation, wait for completion.

### Step 4: Open animation and measure performance

1. Open results dialog
2. Click "Animation" tab
3. **Test playback smoothness:**
   - Press Play
   - Observe frame rate (should be smooth, not choppy)
   - Open browser DevTools → Performance tab
   - Record 10 seconds of playback
   - Stop recording, check frame rate (target: 30+ FPS)

4. **Test scrubbing responsiveness:**
   - Drag timeline slider back and forth
   - Should feel instant, no lag

5. **Test random jumping:**
   - Click different positions on timeline rapidly
   - Should update instantly

### Step 5: Compare with baseline (optional)

If you have baseline metrics before optimization:
- Frame update time should be 60-80% faster
- Memory usage should be similar (this task doesn't address memory)

### Step 6: Document results

Create `docs/performance/2025-11-17-rendering-optimization-results.md`:

```markdown
# Rendering Optimization Performance Results

**Date:** 2025-11-17
**Test:** 2-hour relay simulation, 720 snapshots, 10 VTMs, 5 HTMs

## Metrics

**Before optimization:**
- Playback FPS: [BASELINE]
- Scrubbing lag: [BASELINE]
- Frame update time: [BASELINE]

**After optimization:**
- Playback FPS: [MEASURED]
- Scrubbing lag: [MEASURED]
- Frame update time: [MEASURED]

## Observations

[Notes on smoothness, any remaining issues]

## Next Steps

[If still sluggish, note that lazy reconstruction (Approach 1) is next]
```

### Step 7: Commit performance results

```bash
git add docs/performance/2025-11-17-rendering-optimization-results.md
git commit -m "docs: add rendering optimization performance test results"
```

---

## Success Criteria

- [ ] All unit tests pass (RobotsLayer, StorageSpacesLayer, StationsLayer)
- [ ] Frontend build succeeds with no TypeScript errors
- [ ] Animation playback is smooth (30+ FPS)
- [ ] Scrubbing timeline feels instant (no visible lag)
- [ ] Random jumping to frames updates immediately
- [ ] No visual regressions (entities render correctly)

---

## Follow-Up Work (If Still Sluggish)

If rendering optimization alone doesn't fully solve the problem, proceed with:

**Lazy Reconstruction + LRU Cache (Approach 1 from design):**
- Modify `useSnapshotReconstruction` to only reconstruct on-demand
- Implement LRU cache for reconstructed snapshots (50-100 frame window)
- Add forward buffering during playback (pre-reconstruct next 10 frames)
- Expected additional improvement: 70-90% reduction in initial load time, bounded memory

See: `docs/plans/2025-11-17-lazy-snapshot-reconstruction.md` (to be created if needed)

---

## References

- Design discussion: Brainstorming session 2025-11-17
- Original animation design: `docs/plans/2025-11-12-animation-playback-design.md`
- React.memo documentation: https://react.dev/reference/react/memo
- react-konva performance guide: https://konvajs.org/docs/performance/All_Performance_Tips.html
