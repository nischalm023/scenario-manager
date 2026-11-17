# Lazy Snapshot Reconstruction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement lazy reconstruction with LRU caching to eliminate initial UI freeze and reduce memory usage for 2-4 hour simulations with 720-1440 snapshots.

**Architecture:** Replace eager `useMemo()` reconstruction with on-demand lazy reconstruction backed by LRU cache (50-100 frames). Add forward buffering (prefetch next 10 frames) during playback for smooth animation. Reconstruct frames only when requested, with ~5-10ms reconstruction time per frame being acceptable for random jumps.

**Tech Stack:** React hooks, TypeScript, LRU cache (custom implementation)

---

## Background

**Current Problem (after rendering optimization):**
- Rendering layers work great (60-80% faster frame updates) ✅
- BUT: Still reconstructing ALL snapshots upfront via `useMemo()`
- For 2-hour simulation: 720 snapshots × ~5ms = 3.6 second UI freeze on dialog open
- For 4-hour simulation: 1440 snapshots × ~5ms = 7.2 second UI freeze
- Memory: 1000+ full snapshot objects in memory (50-100MB)

**Solution:**
- **Lazy reconstruction**: Only reconstruct frames when viewed
- **LRU cache**: Keep 50-100 recently accessed frames
- **Forward buffering**: Pre-reconstruct next 10 frames during playback
- **On-demand**: Instant for random jumps (~5-10ms per frame)

**Expected Impact:**
- Initial load: Instant (no upfront reconstruction)
- Memory: ~5-10MB (bounded cache vs unbounded array)
- Playback: Smooth (forward buffer keeps frames ready)
- Scrubbing: Responsive (5-10ms reconstruction is imperceptible)

---

## Task 1: Create LRU Cache Utility

**Goal:** Build generic LRU (Least Recently Used) cache for storing reconstructed snapshots

**Files:**
- Create: `apps/frontend/src/utils/LRUCache.ts`
- Create: `apps/frontend/src/utils/__tests__/LRUCache.test.ts`

### Step 1: Write failing test for LRU cache

Create `apps/frontend/src/utils/__tests__/LRUCache.test.ts`:

```typescript
import { describe, expect, it } from 'vitest'
import { LRUCache } from '../LRUCache'

describe('LRUCache', () => {
  it('should store and retrieve values', () => {
    const cache = new LRUCache<number, string>(3)
    cache.set(1, 'one')
    cache.set(2, 'two')

    expect(cache.get(1)).toBe('one')
    expect(cache.get(2)).toBe('two')
  })

  it('should return undefined for missing keys', () => {
    const cache = new LRUCache<number, string>(3)
    expect(cache.get(999)).toBeUndefined()
  })

  it('should evict least recently used item when capacity exceeded', () => {
    const cache = new LRUCache<number, string>(2)
    cache.set(1, 'one')
    cache.set(2, 'two')
    cache.set(3, 'three') // Should evict key 1 (least recently used)

    expect(cache.get(1)).toBeUndefined() // Evicted
    expect(cache.get(2)).toBe('two')
    expect(cache.get(3)).toBe('three')
  })

  it('should update LRU order on get', () => {
    const cache = new LRUCache<number, string>(2)
    cache.set(1, 'one')
    cache.set(2, 'two')
    cache.get(1) // Access key 1, making it most recently used
    cache.set(3, 'three') // Should evict key 2 (now least recently used)

    expect(cache.get(1)).toBe('one') // Still present
    expect(cache.get(2)).toBeUndefined() // Evicted
    expect(cache.get(3)).toBe('three')
  })

  it('should update existing keys without eviction', () => {
    const cache = new LRUCache<number, string>(2)
    cache.set(1, 'one')
    cache.set(2, 'two')
    cache.set(1, 'ONE') // Update key 1

    expect(cache.get(1)).toBe('ONE')
    expect(cache.get(2)).toBe('two')
    expect(cache.size()).toBe(2)
  })

  it('should report correct size', () => {
    const cache = new LRUCache<number, string>(5)
    expect(cache.size()).toBe(0)

    cache.set(1, 'one')
    cache.set(2, 'two')
    expect(cache.size()).toBe(2)

    cache.set(3, 'three')
    expect(cache.size()).toBe(3)
  })

  it('should clear all entries', () => {
    const cache = new LRUCache<number, string>(3)
    cache.set(1, 'one')
    cache.set(2, 'two')
    cache.clear()

    expect(cache.size()).toBe(0)
    expect(cache.get(1)).toBeUndefined()
  })
})
```

### Step 2: Run test to verify it fails

```bash
npx vitest run apps/frontend/src/utils/__tests__/LRUCache.test.ts
```

**Expected:** FAIL with "Cannot find module '../LRUCache'"

### Step 3: Implement LRU cache

Create `apps/frontend/src/utils/LRUCache.ts`:

```typescript
/**
 * Generic LRU (Least Recently Used) cache implementation.
 *
 * @remarks
 * Uses Map for O(1) lookups and doubly-linked list structure for O(1) evictions.
 * When capacity is exceeded, the least recently used item is evicted.
 *
 * @template K - Key type
 * @template V - Value type
 */
export class LRUCache<K, V> {
  private capacity: number
  private cache: Map<K, V>
  private order: K[] // Track access order (most recent at end)

  constructor(capacity: number) {
    if (capacity <= 0) {
      throw new Error('LRU cache capacity must be > 0')
    }
    this.capacity = capacity
    this.cache = new Map()
    this.order = []
  }

  /**
   * Get value for key, updating LRU order.
   * Returns undefined if key not found.
   */
  get(key: K): V | undefined {
    if (!this.cache.has(key)) {
      return undefined
    }

    // Move to end (most recently used)
    this.updateOrder(key)
    return this.cache.get(key)
  }

  /**
   * Set value for key.
   * Evicts least recently used item if capacity exceeded.
   */
  set(key: K, value: V): void {
    // If key exists, update value and move to end
    if (this.cache.has(key)) {
      this.cache.set(key, value)
      this.updateOrder(key)
      return
    }

    // New key: check capacity
    if (this.cache.size >= this.capacity) {
      // Evict least recently used (first in order array)
      const lruKey = this.order.shift()
      if (lruKey !== undefined) {
        this.cache.delete(lruKey)
      }
    }

    // Add new key
    this.cache.set(key, value)
    this.order.push(key)
  }

  /**
   * Check if key exists in cache.
   */
  has(key: K): boolean {
    return this.cache.has(key)
  }

  /**
   * Get current cache size.
   */
  size(): number {
    return this.cache.size
  }

  /**
   * Clear all entries.
   */
  clear(): void {
    this.cache.clear()
    this.order = []
  }

  /**
   * Move key to end of order array (mark as most recently used).
   */
  private updateOrder(key: K): void {
    const index = this.order.indexOf(key)
    if (index > -1) {
      this.order.splice(index, 1)
    }
    this.order.push(key)
  }
}
```

### Step 4: Run test to verify it passes

```bash
npx vitest run apps/frontend/src/utils/__tests__/LRUCache.test.ts
```

**Expected:** PASS (7/7 tests)

### Step 5: Commit

```bash
git add apps/frontend/src/utils/LRUCache.ts
git add apps/frontend/src/utils/__tests__/LRUCache.test.ts
git commit -m "feat: add LRU cache utility for lazy snapshot reconstruction"
```

---

## Task 2: Create Lazy Reconstruction Hook

**Goal:** Build custom hook that reconstructs snapshots on-demand with LRU caching

**Files:**
- Create: `apps/frontend/src/hooks/useLazySnapshotReconstruction.ts`
- Create: `apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts`

### Step 1: Write failing test for lazy reconstruction hook

Create `apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts`:

```typescript
import { describe, expect, it, vi } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useLazySnapshotReconstruction } from '../useLazySnapshotReconstruction'

describe('useLazySnapshotReconstruction', () => {
  // Mock snapshot data
  const createKeyframe = (timestamp: number) => ({
    type: 'KEYFRAME' as const,
    timestamp,
    robots: [{ robotId: 'robot-1', position: { x: 0, y: 0 } }],
    relays: [],
    stations: [],
    totes: [],
    storageSpaces: [],
    metrics: {},
  })

  const createDelta = (timestamp: number, x: number) => ({
    type: 'DELTA' as const,
    timestamp,
    robots: [{ robotId: 'robot-1', position: { x, y: 0 } }],
    relays: [],
    stations: [],
    totes: [],
    storageSpaces: [],
    metrics: {},
  })

  it('should return null for empty snapshots', () => {
    const { result } = renderHook(() => useLazySnapshotReconstruction([]))

    const snapshot = result.current.getSnapshot(0)
    expect(snapshot).toBeNull()
  })

  it('should reconstruct frame on first access', () => {
    const snapshots = [createKeyframe(0), createDelta(10, 5)]
    const { result } = renderHook(() => useLazySnapshotReconstruction(snapshots))

    const frame0 = result.current.getSnapshot(0)
    expect(frame0).not.toBeNull()
    expect(frame0?.robots[0].position.x).toBe(0)

    const frame1 = result.current.getSnapshot(1)
    expect(frame1).not.toBeNull()
    expect(frame1?.robots[0].position.x).toBe(5)
  })

  it('should cache reconstructed frames', () => {
    const snapshots = [createKeyframe(0), createDelta(10, 5)]
    const { result } = renderHook(() => useLazySnapshotReconstruction(snapshots))

    // First access
    const frame1_first = result.current.getSnapshot(1)

    // Second access (should return cached)
    const frame1_second = result.current.getSnapshot(1)

    expect(frame1_first).toBe(frame1_second) // Same object reference (cached)
  })

  it('should prefetch forward frames during playback', () => {
    const snapshots = Array.from({ length: 20 }, (_, i) =>
      i % 5 === 0 ? createKeyframe(i * 10) : createDelta(i * 10, i)
    )
    const { result } = renderHook(() => useLazySnapshotReconstruction(snapshots, {
      cacheSize: 15,
      prefetchCount: 5,
    }))

    // Request frame 5, should prefetch frames 6-10
    act(() => {
      result.current.getSnapshot(5)
      result.current.prefetch(5, 'forward')
    })

    // Check cache stats (frames 5-10 should be cached)
    const stats = result.current.getCacheStats()
    expect(stats.size).toBeGreaterThanOrEqual(6) // Frame 5 + 5 prefetched
  })

  it('should return cache statistics', () => {
    const snapshots = [createKeyframe(0), createDelta(10, 5), createDelta(20, 10)]
    const { result } = renderHook(() => useLazySnapshotReconstruction(snapshots, {
      cacheSize: 10,
    }))

    result.current.getSnapshot(0)
    result.current.getSnapshot(1)

    const stats = result.current.getCacheStats()
    expect(stats.size).toBe(2)
    expect(stats.capacity).toBe(10)
    expect(stats.hitRate).toBeGreaterThan(0)
  })

  it('should clear cache when snapshots change', () => {
    const snapshots1 = [createKeyframe(0)]
    const snapshots2 = [createKeyframe(0), createDelta(10, 5)]

    const { result, rerender } = renderHook(
      ({ snapshots }) => useLazySnapshotReconstruction(snapshots),
      { initialProps: { snapshots: snapshots1 } }
    )

    result.current.getSnapshot(0)
    expect(result.current.getCacheStats().size).toBe(1)

    // Change snapshots (new simulation loaded)
    rerender({ snapshots: snapshots2 })

    expect(result.current.getCacheStats().size).toBe(0) // Cache cleared
  })
})
```

### Step 2: Run test to verify it fails

```bash
npx vitest run apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts
```

**Expected:** FAIL with "Cannot find module '../useLazySnapshotReconstruction'"

### Step 3: Implement lazy reconstruction hook

Create `apps/frontend/src/hooks/useLazySnapshotReconstruction.ts`:

```typescript
import { useCallback, useEffect, useMemo, useRef } from 'react'
import type { components } from '@/types/generated-api'
import { LRUCache } from '@/utils/LRUCache'

type WarehouseSnapshotV2 = components['schemas']['WarehouseSnapshotV2']
type RobotState = components['schemas']['RobotState']
type ToteState = components['schemas']['ToteState']
type StorageSpaceState = components['schemas']['StorageSpaceState']
type RelayState = components['schemas']['RelayState']
type StationState = components['schemas']['StationState']
type SnapshotMetrics = components['schemas']['SnapshotMetrics']

interface ReconstructedSnapshot {
  timestamp: number
  robots: RobotState[]
  relays: RelayState[]
  stations: StationState[]
  totes: ToteState[]
  storageSpaces: StorageSpaceState[]
  metrics: SnapshotMetrics
}

interface LazyReconstructionOptions {
  cacheSize?: number // Default: 100
  prefetchCount?: number // Default: 10
}

interface CacheStats {
  size: number
  capacity: number
  hits: number
  misses: number
  hitRate: number
}

/**
 * Lazy reconstruction hook with LRU caching and prefetching.
 *
 * @remarks
 * Only reconstructs frames when requested (on-demand).
 * Caches recently accessed frames in LRU cache.
 * Prefetches next N frames during playback for smooth animation.
 *
 * @param rawSnapshots - Compressed snapshots (keyframe + delta)
 * @param options - Cache configuration
 */
export function useLazySnapshotReconstruction(
  rawSnapshots: WarehouseSnapshotV2[],
  options: LazyReconstructionOptions = {}
) {
  const { cacheSize = 100, prefetchCount = 10 } = options

  // LRU cache for reconstructed snapshots
  const cache = useRef(new LRUCache<number, ReconstructedSnapshot>(cacheSize))

  // Cache statistics
  const stats = useRef({ hits: 0, misses: 0 })

  // Clear cache when rawSnapshots change (new simulation loaded)
  useEffect(() => {
    cache.current.clear()
    stats.current = { hits: 0, misses: 0 }
  }, [rawSnapshots])

  // Build index of keyframes for fast lookup
  const keyframeIndices = useMemo(() => {
    const indices: number[] = []
    rawSnapshots.forEach((snapshot, index) => {
      const snapshotAny = snapshot as any
      if (snapshotAny.type === 'KEYFRAME') {
        indices.push(index)
      }
    })
    return indices
  }, [rawSnapshots])

  /**
   * Find the nearest preceding keyframe index for a given frame.
   */
  const findKeyframeIndex = useCallback(
    (frameIndex: number): number => {
      // Binary search for efficiency
      let left = 0
      let right = keyframeIndices.length - 1
      let result = -1

      while (left <= right) {
        const mid = Math.floor((left + right) / 2)
        if (keyframeIndices[mid] <= frameIndex) {
          result = keyframeIndices[mid]
          left = mid + 1
        } else {
          right = mid - 1
        }
      }

      return result
    },
    [keyframeIndices]
  )

  /**
   * Reconstruct a single frame by merging keyframe + deltas.
   */
  const reconstructFrame = useCallback(
    (frameIndex: number): ReconstructedSnapshot | null => {
      if (frameIndex < 0 || frameIndex >= rawSnapshots.length) {
        return null
      }

      // Find nearest keyframe
      const keyframeIdx = findKeyframeIndex(frameIndex)
      if (keyframeIdx === -1) {
        console.error(`No keyframe found before frame ${frameIndex}`)
        return null
      }

      // Start with keyframe
      const keyframe = rawSnapshots[keyframeIdx] as any
      let currentState: ReconstructedSnapshot = {
        timestamp: keyframe.timestamp ?? 0,
        robots: keyframe.robots ?? [],
        relays: keyframe.relays ?? [],
        stations: keyframe.stations ?? [],
        totes: keyframe.totes ?? [],
        storageSpaces: keyframe.storageSpaces ?? [],
        metrics: keyframe.metrics ?? {},
      }

      // Apply deltas up to frameIndex
      for (let i = keyframeIdx + 1; i <= frameIndex; i++) {
        const snapshot = rawSnapshots[i] as any
        if (snapshot.type === 'DELTA') {
          currentState = {
            timestamp: snapshot.timestamp ?? currentState.timestamp,
            robots: mergeById(currentState.robots, snapshot.robots ?? [], 'robotId'),
            relays: mergeById(currentState.relays, snapshot.relays ?? [], 'relayId'),
            stations: mergeById(currentState.stations, snapshot.stations ?? [], 'stationId'),
            totes: mergeById(currentState.totes, snapshot.totes ?? [], 'toteId'),
            storageSpaces: mergeById(
              currentState.storageSpaces,
              snapshot.storageSpaces ?? [],
              'storageSpaceId'
            ),
            metrics: snapshot.metrics ?? currentState.metrics,
          }
        }
      }

      return currentState
    },
    [rawSnapshots, findKeyframeIndex]
  )

  /**
   * Get snapshot for a specific frame (reconstructs if not cached).
   */
  const getSnapshot = useCallback(
    (frameIndex: number): ReconstructedSnapshot | null => {
      if (rawSnapshots.length === 0) {
        return null
      }

      // Check cache first
      const cached = cache.current.get(frameIndex)
      if (cached) {
        stats.current.hits++
        return cached
      }

      // Cache miss: reconstruct
      stats.current.misses++
      const reconstructed = reconstructFrame(frameIndex)
      if (reconstructed) {
        cache.current.set(frameIndex, reconstructed)
      }
      return reconstructed
    },
    [rawSnapshots, reconstructFrame]
  )

  /**
   * Prefetch frames in a direction (for smooth playback).
   */
  const prefetch = useCallback(
    (currentFrame: number, direction: 'forward' | 'backward' = 'forward') => {
      const start = direction === 'forward' ? currentFrame + 1 : currentFrame - prefetchCount
      const end = direction === 'forward' ? currentFrame + prefetchCount : currentFrame - 1

      for (let i = start; i <= end && i >= 0 && i < rawSnapshots.length; i++) {
        // Only prefetch if not already cached
        if (!cache.current.has(i)) {
          const reconstructed = reconstructFrame(i)
          if (reconstructed) {
            cache.current.set(i, reconstructed)
          }
        }
      }
    },
    [rawSnapshots, reconstructFrame, prefetchCount]
  )

  /**
   * Get cache statistics.
   */
  const getCacheStats = useCallback((): CacheStats => {
    const total = stats.current.hits + stats.current.misses
    return {
      size: cache.current.size(),
      capacity: cacheSize,
      hits: stats.current.hits,
      misses: stats.current.misses,
      hitRate: total > 0 ? stats.current.hits / total : 0,
    }
  }, [cacheSize])

  return {
    getSnapshot,
    prefetch,
    getCacheStats,
  }
}

/**
 * Merge entities by ID - updates existing or adds new.
 */
function mergeById<T extends Record<string, any>>(
  current: T[],
  changes: T[],
  idField: keyof T
): T[] {
  const merged = new Map(current.map((item) => [item[idField], item]))

  for (const change of changes) {
    merged.set(change[idField], change)
  }

  return Array.from(merged.values())
}
```

### Step 4: Run test to verify it passes

```bash
npx vitest run apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts
```

**Expected:** PASS (6/6 tests)

### Step 5: Commit

```bash
git add apps/frontend/src/hooks/useLazySnapshotReconstruction.ts
git add apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts
git commit -m "feat: add lazy snapshot reconstruction hook with LRU caching"
```

---

## Task 3: Update SimulationResults to Use Lazy Reconstruction

**Goal:** Replace eager `useSnapshotReconstruction` with lazy hook

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

### Step 1: Import lazy reconstruction hook

In `SimulationResults.tsx`, replace the import on line 10:

**Find:**
```typescript
import { useSnapshotReconstruction } from '../../../hooks/useSnapshotReconstruction'
```

**Replace with:**
```typescript
import { useLazySnapshotReconstruction } from '../../../hooks/useLazySnapshotReconstruction'
```

### Step 2: Replace reconstruction call

**Find (line 33):**
```typescript
// Reconstruct full snapshots from keyframe + delta compression
const reconstructedSnapshots = useSnapshotReconstruction(snapshots || [])
```

**Replace with:**
```typescript
// Lazy reconstruction with LRU caching (only reconstructs frames when viewed)
const { getSnapshot, prefetch, getCacheStats } = useLazySnapshotReconstruction(snapshots || [], {
  cacheSize: 100, // Keep 100 most recent frames in memory
  prefetchCount: 10, // Prefetch next 10 frames during playback
})
```

### Step 3: Update WarehouseMap integration

**Find (around line 194):**
```typescript
snapshot={reconstructedSnapshots[currentFrame] as any}
```

**Replace with:**
```typescript
snapshot={getSnapshot(currentFrame) as any}
```

### Step 4: Update AnimationControls integration

**Find (around line 176):**
```typescript
<AnimationControls
  snapshots={reconstructedSnapshots as any[]}
  currentFrame={currentFrame}
  onFrameChange={setCurrentFrame}
/>
```

**Replace with:**
```typescript
<AnimationControls
  snapshots={snapshots || []}
  currentFrame={currentFrame}
  onFrameChange={setCurrentFrame}
  onPrefetch={(frame, direction) => prefetch(frame, direction)}
/>
```

### Step 5: Verify TypeScript compiles

```bash
cd apps/frontend
npx tsc --noEmit
```

**Expected:** No TypeScript errors

### Step 6: Commit

```bash
git add apps/frontend/src/pages/simulator/components/SimulationResults.tsx
git commit -m "feat: integrate lazy snapshot reconstruction into SimulationResults"
```

---

## Task 4: Update AnimationControls for Prefetching

**Goal:** Add prefetching during playback for smooth animation

**Files:**
- Modify: `apps/frontend/src/components/AnimationControls.tsx`

### Step 1: Add onPrefetch prop to interface

**Find (line 16-20):**
```typescript
interface AnimationControlsProps {
  snapshots: WarehouseSnapshot[]
  currentFrame: number
  onFrameChange: (frame: number) => void
}
```

**Replace with:**
```typescript
interface AnimationControlsProps {
  snapshots: WarehouseSnapshot[]
  currentFrame: number
  onFrameChange: (frame: number) => void
  onPrefetch?: (frame: number, direction: 'forward' | 'backward') => void
}
```

### Step 2: Extract onPrefetch from props

**Find (line 22-26):**
```typescript
export function AnimationControls({
  snapshots,
  currentFrame,
  onFrameChange,
}: AnimationControlsProps) {
```

**Replace with:**
```typescript
export function AnimationControls({
  snapshots,
  currentFrame,
  onFrameChange,
  onPrefetch,
}: AnimationControlsProps) {
```

### Step 3: Add prefetching to playback loop

**Find (line 34-48, inside the useEffect):**
```typescript
// Playback loop
useEffect(() => {
  if (!isPlaying || currentFrame >= totalFrames - 1) {
    if (currentFrame >= totalFrames - 1) {
      setIsPlaying(false)
    }
    return
  }

  const intervalMs = 1000 / playbackSpeed
  const timer = setInterval(() => {
    onFrameChange(currentFrame + 1)
  }, intervalMs)

  return () => clearInterval(timer)
}, [isPlaying, currentFrame, playbackSpeed, totalFrames, onFrameChange])
```

**Replace with:**
```typescript
// Playback loop
useEffect(() => {
  if (!isPlaying || currentFrame >= totalFrames - 1) {
    if (currentFrame >= totalFrames - 1) {
      setIsPlaying(false)
    }
    return
  }

  const intervalMs = 1000 / playbackSpeed
  const timer = setInterval(() => {
    onFrameChange(currentFrame + 1)

    // Prefetch next frames for smooth playback
    if (onPrefetch) {
      onPrefetch(currentFrame + 1, 'forward')
    }
  }, intervalMs)

  return () => clearInterval(timer)
}, [isPlaying, currentFrame, playbackSpeed, totalFrames, onFrameChange, onPrefetch])
```

### Step 4: Add prefetching to scrubbing

**Find (line 64-67, handleSliderChange function):**
```typescript
const handleSliderChange = (values: number[]) => {
  setIsPlaying(false)
  onFrameChange(values[0])
}
```

**Replace with:**
```typescript
const handleSliderChange = (values: number[]) => {
  setIsPlaying(false)
  onFrameChange(values[0])

  // Prefetch surrounding frames for smooth scrubbing
  if (onPrefetch) {
    onPrefetch(values[0], 'forward')
  }
}
```

### Step 5: Verify TypeScript compiles

```bash
cd apps/frontend
npx tsc --noEmit
```

**Expected:** No TypeScript errors

### Step 6: Commit

```bash
git add apps/frontend/src/components/AnimationControls.tsx
git commit -m "feat: add prefetching support to AnimationControls for smooth playback"
```

---

## Task 5: Add Cache Statistics Display (Optional)

**Goal:** Show cache hit rate and memory usage for debugging

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

### Step 1: Add state for showing cache stats

**Add after line 30:**
```typescript
const [showCacheStats, setShowCacheStats] = useState(false)
```

### Step 2: Add cache stats display to Animation tab

**Find the AnimationControls section (around line 175-179), add after AnimationControls:**

```typescript
{/* Cache Statistics (development/debugging) */}
{showCacheStats && (
  <div className="mt-4 p-4 bg-gray-50 rounded-lg border border-gray-200">
    <h3 className="text-sm font-semibold mb-2">Cache Statistics</h3>
    <div className="grid grid-cols-2 gap-2 text-xs">
      <div>
        <span className="text-gray-600">Size:</span>{' '}
        <span className="font-mono">{getCacheStats().size} / {getCacheStats().capacity}</span>
      </div>
      <div>
        <span className="text-gray-600">Hit Rate:</span>{' '}
        <span className="font-mono">{(getCacheStats().hitRate * 100).toFixed(1)}%</span>
      </div>
      <div>
        <span className="text-gray-600">Hits:</span>{' '}
        <span className="font-mono">{getCacheStats().hits}</span>
      </div>
      <div>
        <span className="text-gray-600">Misses:</span>{' '}
        <span className="font-mono">{getCacheStats().misses}</span>
      </div>
    </div>
  </div>
)}
```

### Step 3: Add toggle button to show/hide stats

**Add button near AnimationControls:**

```typescript
<div className="flex justify-end mb-2">
  <button
    onClick={() => setShowCacheStats(!showCacheStats)}
    className="text-xs text-gray-600 hover:text-gray-900 underline"
    type="button"
  >
    {showCacheStats ? 'Hide' : 'Show'} Cache Stats
  </button>
</div>
```

### Step 4: Verify build succeeds

```bash
cd apps/frontend
npm run build
```

**Expected:** Build succeeds (no errors)

### Step 5: Commit

```bash
git add apps/frontend/src/pages/simulator/components/SimulationResults.tsx
git commit -m "feat: add cache statistics display for debugging lazy reconstruction"
```

---

## Task 6: Performance Testing and Validation

**Goal:** Validate lazy reconstruction eliminates freeze and reduces memory

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

### Step 3: Run 2-hour relay simulation

**Simulation config:**
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

Expected: ~720 snapshots generated

### Step 4: Measure initial load performance

1. Open browser DevTools → Performance tab
2. Click "Run Simulation"
3. Wait for completion
4. **Start recording** in Performance tab
5. Click to open results dialog with animation
6. **Stop recording** after 2 seconds

**Metrics to check:**
- Time to first paint of dialog (should be <100ms, was 3-7 seconds before)
- No long tasks blocking main thread
- Memory snapshot: ~5-10MB for cache (was 50-100MB before)

### Step 5: Measure playback smoothness

1. Click "Animation" tab
2. Press Play
3. Open Performance tab, record 10 seconds of playback
4. Stop recording

**Metrics to check:**
- Playback FPS: 30+ (smooth)
- No frame drops or stuttering
- Cache hit rate: >80% (check cache stats display)

### Step 6: Measure scrubbing responsiveness

1. Rapidly drag timeline slider back and forth
2. Observe visual lag (should feel instant, <50ms)
3. Check cache stats: hit rate should be 60-70% (many random jumps)

### Step 7: Document results

Create `docs/performance/2025-11-17-lazy-reconstruction-results.md`:

```markdown
# Lazy Reconstruction Performance Results

**Date:** 2025-11-17
**Test:** 2-hour relay simulation, 720 snapshots, 10 VTMs, 5 HTMs

## Baseline (Before Lazy Reconstruction)

- Initial load: 3.6 second UI freeze
- Memory: 50-100MB (720 full snapshots)
- Playback: Smooth (30+ FPS) after rendering optimization

## After Lazy Reconstruction

**Initial Load:**
- Time to dialog open: [MEASURED] ms (target: <100ms)
- Blocking time: [MEASURED] ms (target: 0ms)
- Improvement: [CALCULATED]%

**Memory Usage:**
- Cache size: [MEASURED] MB
- Total frontend memory: [MEASURED] MB
- Improvement: [CALCULATED]%

**Playback:**
- FPS: [MEASURED] (target: 30+)
- Cache hit rate: [MEASURED]% (target: >80%)

**Scrubbing:**
- Perceived lag: [None/Minimal/Noticeable]
- Cache hit rate: [MEASURED]% (target: >60%)

## Observations

[Notes on user experience, any remaining issues]

## Success Criteria

- ✅/❌ Initial load <100ms (no freeze)
- ✅/❌ Memory <10MB (bounded cache)
- ✅/❌ Playback smooth (30+ FPS, >80% hit rate)
- ✅/❌ Scrubbing responsive (<50ms lag)
```

### Step 8: Commit results

```bash
git add docs/performance/2025-11-17-lazy-reconstruction-results.md
git commit -m "docs: add lazy reconstruction performance test results"
```

---

## Success Criteria

- [ ] All unit tests pass (LRUCache, useLazySnapshotReconstruction)
- [ ] Frontend build succeeds with no TypeScript errors
- [ ] Initial dialog load: <100ms (no freeze)
- [ ] Memory usage: <10MB (bounded cache vs unbounded array)
- [ ] Playback: Smooth 30+ FPS with >80% cache hit rate
- [ ] Scrubbing: Responsive (<50ms lag) with >60% hit rate
- [ ] No visual regressions (animation looks identical to before)

---

## Rollback Plan

If lazy reconstruction introduces issues:

1. **Revert SimulationResults changes:**
   ```bash
   git revert <commit-sha-from-task-3>
   ```

2. **Keep LRU cache and lazy hook** (useful for future features)

3. **Keep rendering optimization** (Tasks 1-5 from previous plan)

4. **Alternative approach:** Implement background Web Worker reconstruction

---

## Follow-Up Work (If Needed)

If lazy reconstruction still has issues:

**Approach 2: Web Worker Reconstruction**
- Move reconstruction to background thread
- Stream results back to main thread
- No UI blocking, but more complex messaging

**Approach 3: Partial Eager + Lazy Hybrid**
- Eagerly reconstruct first 50 frames (instant playback start)
- Lazily reconstruct rest on-demand
- Balances initial load vs total memory

---

## References

- Original animation design: `docs/plans/2025-11-12-animation-playback-design.md`
- Rendering optimization: `docs/plans/2025-11-17-warehouse-map-rendering-optimization.md`
- Brainstorming session: 2025-11-17 (Approach 1: Lazy Reconstruction + LRU Cache)
- LRU cache algorithm: https://en.wikipedia.org/wiki/Cache_replacement_policies#Least_recently_used_(LRU)
