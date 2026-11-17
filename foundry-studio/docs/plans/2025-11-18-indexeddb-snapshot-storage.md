# IndexedDB Snapshot Storage Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce animation memory usage from ~466MB to ~20MB by storing raw snapshots in IndexedDB with a bidirectional memory buffer.

**Architecture:** Store all snapshots in IndexedDB on simulation complete, keep 30-frame sliding window buffer in memory (±15 frames around current). Async `getSnapshot()` with 50-100ms latency for cache misses. Clear all stale data on app startup.

**Tech Stack:** React 18, TypeScript, IndexedDB (idb library), Vitest

---

## Background

**Current Problem:**
- Raw snapshots stored in React state: 720 × 648KB = ~466MB
- Memory stays high even after disabling reconstruction cache
- Browser becomes sluggish with large heap

**Solution:**
- Store snapshots in IndexedDB (persisted but cleared on app load)
- Keep 30-frame bidirectional buffer in memory (~20MB)
- Async frame access with loading state

**Expected Impact:**
- Memory: 466MB → ~20MB (95% reduction)
- Latency: 50-100ms for cache miss (acceptable per design)
- UX: Smooth scrubbing within buffer, slight delay on random jumps

---

## Task 1: Create Snapshot Store Service

**Goal:** IndexedDB service for storing and retrieving snapshots

**Files:**
- Create: `apps/frontend/src/utils/snapshotStore.ts`
- Create: `apps/frontend/src/utils/__tests__/snapshotStore.test.ts`

### Step 1: Install idb library

```bash
cd apps/frontend && npm install idb
```

### Step 2: Write failing test for store and retrieve

Create `apps/frontend/src/utils/__tests__/snapshotStore.test.ts`:

```typescript
import { describe, expect, it, beforeEach } from 'vitest'
import { snapshotStore } from '../snapshotStore'

describe('snapshotStore', () => {
  beforeEach(async () => {
    await snapshotStore.clearAll()
  })

  it('should store and retrieve snapshots', async () => {
    const snapshots = [
      { type: 'KEYFRAME', timestamp: 0, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: {} },
      { type: 'DELTA', timestamp: 10, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: {} },
    ]

    const storageKey = await snapshotStore.store(snapshots)
    expect(storageKey).toBeTruthy()

    const retrieved = await snapshotStore.get(storageKey, 0)
    expect(retrieved).toEqual(snapshots[0])

    const retrieved2 = await snapshotStore.get(storageKey, 1)
    expect(retrieved2).toEqual(snapshots[1])
  })

  it('should retrieve range of snapshots', async () => {
    const snapshots = Array.from({ length: 10 }, (_, i) => ({
      type: i % 5 === 0 ? 'KEYFRAME' : 'DELTA',
      timestamp: i * 10,
      robots: [],
      relays: [],
      stations: [],
      totes: [],
      storageSpaces: [],
      metrics: {},
    }))

    const storageKey = await snapshotStore.store(snapshots)
    const range = await snapshotStore.getRange(storageKey, 2, 5)

    expect(range).toHaveLength(4)
    expect(range[0].timestamp).toBe(20)
    expect(range[3].timestamp).toBe(50)
  })

  it('should delete specific simulation data', async () => {
    const snapshots = [
      { type: 'KEYFRAME', timestamp: 0, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: {} },
    ]

    const storageKey = await snapshotStore.store(snapshots)
    await snapshotStore.delete(storageKey)

    const retrieved = await snapshotStore.get(storageKey, 0)
    expect(retrieved).toBeNull()
  })

  it('should clear all snapshot data', async () => {
    const snapshots1 = [{ type: 'KEYFRAME', timestamp: 0, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: {} }]
    const snapshots2 = [{ type: 'KEYFRAME', timestamp: 0, robots: [], relays: [], stations: [], totes: [], storageSpaces: [], metrics: {} }]

    const key1 = await snapshotStore.store(snapshots1)
    const key2 = await snapshotStore.store(snapshots2)

    await snapshotStore.clearAll()

    const retrieved1 = await snapshotStore.get(key1, 0)
    const retrieved2 = await snapshotStore.get(key2, 0)

    expect(retrieved1).toBeNull()
    expect(retrieved2).toBeNull()
  })
})
```

### Step 3: Run test to verify it fails

Run:
```bash
cd apps/frontend && npx vitest run src/utils/__tests__/snapshotStore.test.ts
```

Expected: FAIL with "Cannot find module '../snapshotStore'"

### Step 4: Implement snapshotStore service

Create `apps/frontend/src/utils/snapshotStore.ts`:

```typescript
import { openDB, type IDBPDatabase } from 'idb'

const DB_NAME = 'warehouse-snapshots'
const DB_VERSION = 1
const STORE_NAME = 'snapshots'

interface SnapshotRecord {
  id: string // storageKey_frameIndex
  storageKey: string
  frameIndex: number
  data: any
}

let dbPromise: Promise<IDBPDatabase> | null = null

function getDB(): Promise<IDBPDatabase> {
  if (!dbPromise) {
    dbPromise = openDB(DB_NAME, DB_VERSION, {
      upgrade(db) {
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          const store = db.createObjectStore(STORE_NAME, { keyPath: 'id' })
          store.createIndex('storageKey', 'storageKey', { unique: false })
        }
      },
    })
  }
  return dbPromise
}

function generateStorageKey(): string {
  return `sim_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`
}

export const snapshotStore = {
  /**
   * Store all snapshots for a simulation.
   * Returns a storage key to retrieve them later.
   */
  async store(snapshots: any[]): Promise<string> {
    const db = await getDB()
    const storageKey = generateStorageKey()

    const tx = db.transaction(STORE_NAME, 'readwrite')
    const store = tx.objectStore(STORE_NAME)

    // Store each snapshot with its frame index
    await Promise.all(
      snapshots.map((snapshot, frameIndex) =>
        store.put({
          id: `${storageKey}_${frameIndex}`,
          storageKey,
          frameIndex,
          data: snapshot,
        })
      )
    )

    await tx.done
    return storageKey
  },

  /**
   * Retrieve a single snapshot by frame index.
   */
  async get(storageKey: string, frameIndex: number): Promise<any | null> {
    const db = await getDB()
    const record = await db.get(STORE_NAME, `${storageKey}_${frameIndex}`)
    return record?.data ?? null
  },

  /**
   * Retrieve a range of snapshots (inclusive).
   */
  async getRange(storageKey: string, start: number, end: number): Promise<any[]> {
    const db = await getDB()
    const results: any[] = []

    for (let i = start; i <= end; i++) {
      const record = await db.get(STORE_NAME, `${storageKey}_${i}`)
      if (record) {
        results.push(record.data)
      }
    }

    return results
  },

  /**
   * Delete all snapshots for a specific simulation.
   */
  async delete(storageKey: string): Promise<void> {
    const db = await getDB()
    const tx = db.transaction(STORE_NAME, 'readwrite')
    const store = tx.objectStore(STORE_NAME)
    const index = store.index('storageKey')

    // Get all records with this storage key
    const keys = await index.getAllKeys(storageKey)

    // Delete each record
    await Promise.all(keys.map((key) => store.delete(key)))

    await tx.done
  },

  /**
   * Clear all snapshot data (call on app startup).
   */
  async clearAll(): Promise<void> {
    const db = await getDB()
    const tx = db.transaction(STORE_NAME, 'readwrite')
    await tx.objectStore(STORE_NAME).clear()
    await tx.done
  },
}
```

### Step 5: Run test to verify it passes

Run:
```bash
cd apps/frontend && npx vitest run src/utils/__tests__/snapshotStore.test.ts
```

Expected: PASS (all 4 tests)

### Step 6: Commit

```bash
git add apps/frontend/src/utils/snapshotStore.ts apps/frontend/src/utils/__tests__/snapshotStore.test.ts apps/frontend/package.json apps/frontend/package-lock.json
git commit -m "feat: add IndexedDB snapshot store service

Stores simulation snapshots in IndexedDB for memory efficiency.
Supports store, get, getRange, delete, and clearAll operations."
```

---

## Task 2: Create useIndexedDBSnapshots Hook

**Goal:** Hook with bidirectional buffer management and async snapshot access

**Files:**
- Create: `apps/frontend/src/hooks/useIndexedDBSnapshots.ts`
- Create: `apps/frontend/src/hooks/__tests__/useIndexedDBSnapshots.test.ts`

### Step 1: Write failing test for basic snapshot retrieval

Create `apps/frontend/src/hooks/__tests__/useIndexedDBSnapshots.test.ts`:

```typescript
import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, beforeEach } from 'vitest'
import { useIndexedDBSnapshots } from '../useIndexedDBSnapshots'
import { snapshotStore } from '../../utils/snapshotStore'

describe('useIndexedDBSnapshots', () => {
  let storageKey: string

  const createSnapshots = (count: number) =>
    Array.from({ length: count }, (_, i) => ({
      type: i % 6 === 0 ? 'KEYFRAME' : 'DELTA',
      timestamp: i * 10,
      robots: [{ robotId: 'robot-1', position: { x: i, y: 0 } }],
      relays: [],
      stations: [],
      totes: [],
      storageSpaces: [],
      metrics: {},
    }))

  beforeEach(async () => {
    await snapshotStore.clearAll()
    const snapshots = createSnapshots(100)
    storageKey = await snapshotStore.store(snapshots)
  })

  it('should retrieve snapshot asynchronously', async () => {
    const { result } = renderHook(() => useIndexedDBSnapshots(storageKey, 100))

    // Initial state
    expect(result.current.isLoading).toBe(false)
    expect(result.current.currentSnapshot).toBeNull()

    // Request frame 0
    act(() => {
      result.current.loadFrame(0)
    })

    // Should show loading
    expect(result.current.isLoading).toBe(true)

    // Wait for load
    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    // Should have snapshot
    expect(result.current.currentSnapshot).not.toBeNull()
    expect(result.current.currentSnapshot?.robots[0].position.x).toBe(0)
  })

  it('should load from buffer instantly on second access', async () => {
    const { result } = renderHook(() => useIndexedDBSnapshots(storageKey, 100))

    // Load frame 0
    act(() => {
      result.current.loadFrame(0)
    })

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    // Load frame 1 (should be in buffer after prefetch)
    act(() => {
      result.current.loadFrame(1)
    })

    // Should load quickly from buffer
    await waitFor(() => {
      expect(result.current.currentSnapshot?.robots[0].position.x).toBe(1)
    }, { timeout: 50 })
  })

  it('should report buffer status', async () => {
    const { result } = renderHook(() => useIndexedDBSnapshots(storageKey, 100))

    act(() => {
      result.current.loadFrame(50)
    })

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    const status = result.current.bufferStatus
    expect(status.current).toBe(50)
    expect(status.start).toBeLessThanOrEqual(50)
    expect(status.end).toBeGreaterThanOrEqual(50)
  })
})
```

### Step 2: Run test to verify it fails

Run:
```bash
cd apps/frontend && npx vitest run src/hooks/__tests__/useIndexedDBSnapshots.test.ts
```

Expected: FAIL with "Cannot find module '../useIndexedDBSnapshots'"

### Step 3: Implement useIndexedDBSnapshots hook

Create `apps/frontend/src/hooks/useIndexedDBSnapshots.ts`:

```typescript
import { useCallback, useRef, useState } from 'react'
import { snapshotStore } from '../utils/snapshotStore'

interface RobotState {
  robotId: string
  position: { x: number; y: number }
  [key: string]: any
}

interface ReconstructedSnapshot {
  timestamp: number
  robots: RobotState[]
  relays: any[]
  stations: any[]
  totes: any[]
  storageSpaces: any[]
  metrics: any
}

interface BufferStatus {
  current: number
  start: number
  end: number
}

interface BufferEntry {
  frameIndex: number
  snapshot: ReconstructedSnapshot
}

const BUFFER_SIZE = 30 // ±15 frames around current
const HALF_BUFFER = Math.floor(BUFFER_SIZE / 2)

/**
 * Hook for accessing snapshots stored in IndexedDB with memory buffer.
 *
 * Maintains a sliding window of 30 frames in memory for fast access.
 * Frames outside buffer are loaded asynchronously from IndexedDB.
 */
export function useIndexedDBSnapshots(storageKey: string | null, snapshotCount: number) {
  const [currentSnapshot, setCurrentSnapshot] = useState<ReconstructedSnapshot | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [bufferStatus, setBufferStatus] = useState<BufferStatus>({ current: 0, start: 0, end: 0 })

  const bufferRef = useRef<Map<number, ReconstructedSnapshot>>(new Map())
  const keyframeIndicesRef = useRef<number[]>([])
  const isInitializedRef = useRef(false)

  /**
   * Initialize by loading keyframe indices.
   */
  const initialize = useCallback(async () => {
    if (!storageKey || isInitializedRef.current) return

    // Scan for keyframes (every 6th frame with 10s interval and 60s keyframe interval)
    const indices: number[] = []
    for (let i = 0; i < snapshotCount; i += 6) {
      indices.push(i)
    }
    // Ensure frame 0 is included
    if (!indices.includes(0)) {
      indices.unshift(0)
    }
    keyframeIndicesRef.current = indices
    isInitializedRef.current = true
  }, [storageKey, snapshotCount])

  /**
   * Find nearest keyframe at or before given frame index.
   */
  const findKeyframeIndex = useCallback((frameIndex: number): number => {
    const indices = keyframeIndicesRef.current
    let result = 0
    for (const idx of indices) {
      if (idx <= frameIndex) {
        result = idx
      } else {
        break
      }
    }
    return result
  }, [])

  /**
   * Reconstruct a snapshot from keyframe + deltas.
   */
  const reconstructSnapshot = useCallback(
    async (frameIndex: number): Promise<ReconstructedSnapshot | null> => {
      if (!storageKey) return null

      const keyframeIdx = findKeyframeIndex(frameIndex)

      // Load keyframe and all deltas up to target frame
      const rawSnapshots = await snapshotStore.getRange(storageKey, keyframeIdx, frameIndex)
      if (rawSnapshots.length === 0) return null

      // Start with keyframe
      let state: ReconstructedSnapshot = {
        timestamp: rawSnapshots[0].timestamp ?? 0,
        robots: rawSnapshots[0].robots ?? [],
        relays: rawSnapshots[0].relays ?? [],
        stations: rawSnapshots[0].stations ?? [],
        totes: rawSnapshots[0].totes ?? [],
        storageSpaces: rawSnapshots[0].storageSpaces ?? [],
        metrics: rawSnapshots[0].metrics ?? {},
      }

      // Apply deltas
      for (let i = 1; i < rawSnapshots.length; i++) {
        const delta = rawSnapshots[i]
        state = {
          timestamp: delta.timestamp ?? state.timestamp,
          robots: mergeById(state.robots, delta.robots ?? [], 'robotId'),
          relays: mergeById(state.relays, delta.relays ?? [], 'relayId'),
          stations: mergeById(state.stations, delta.stations ?? [], 'stationId'),
          totes: mergeById(state.totes, delta.totes ?? [], 'toteId'),
          storageSpaces: mergeById(state.storageSpaces, delta.storageSpaces ?? [], 'storageSpaceId'),
          metrics: delta.metrics ?? state.metrics,
        }
      }

      return state
    },
    [storageKey, findKeyframeIndex]
  )

  /**
   * Load buffer around a frame index.
   */
  const loadBuffer = useCallback(
    async (centerFrame: number) => {
      if (!storageKey) return

      const start = Math.max(0, centerFrame - HALF_BUFFER)
      const end = Math.min(snapshotCount - 1, centerFrame + HALF_BUFFER)

      // Load frames not already in buffer
      const framesToLoad: number[] = []
      for (let i = start; i <= end; i++) {
        if (!bufferRef.current.has(i)) {
          framesToLoad.push(i)
        }
      }

      // Reconstruct missing frames
      await Promise.all(
        framesToLoad.map(async (frameIndex) => {
          const snapshot = await reconstructSnapshot(frameIndex)
          if (snapshot) {
            bufferRef.current.set(frameIndex, snapshot)
          }
        })
      )

      // Evict frames outside buffer
      for (const frameIndex of bufferRef.current.keys()) {
        if (frameIndex < start || frameIndex > end) {
          bufferRef.current.delete(frameIndex)
        }
      }

      setBufferStatus({ current: centerFrame, start, end })
    },
    [storageKey, snapshotCount, reconstructSnapshot]
  )

  /**
   * Load a specific frame.
   */
  const loadFrame = useCallback(
    async (frameIndex: number) => {
      if (!storageKey) return

      await initialize()

      // Check buffer first
      const buffered = bufferRef.current.get(frameIndex)
      if (buffered) {
        setCurrentSnapshot(buffered)
        setBufferStatus((prev) => ({ ...prev, current: frameIndex }))
        // Refresh buffer in background
        loadBuffer(frameIndex)
        return
      }

      // Load from IndexedDB
      setIsLoading(true)

      try {
        const snapshot = await reconstructSnapshot(frameIndex)
        if (snapshot) {
          bufferRef.current.set(frameIndex, snapshot)
          setCurrentSnapshot(snapshot)
        }

        // Load surrounding buffer
        await loadBuffer(frameIndex)
      } finally {
        setIsLoading(false)
      }
    },
    [storageKey, initialize, reconstructSnapshot, loadBuffer]
  )

  return {
    currentSnapshot,
    isLoading,
    bufferStatus,
    loadFrame,
  }
}

/**
 * Merge entities by ID with structural sharing.
 */
function mergeById<T extends Record<string, any>>(
  current: T[],
  changes: T[],
  idField: keyof T
): T[] {
  if (!changes || changes.length === 0) {
    return current
  }

  const changesMap = new Map(changes.map((item) => [item[idField], item]))
  const result: T[] = []
  const processedIds = new Set<any>()
  let hasChanges = false

  for (const item of current) {
    const id = item[idField]
    const updated = changesMap.get(id)
    if (updated) {
      result.push(updated)
      hasChanges = true
    } else {
      result.push(item)
    }
    processedIds.add(id)
  }

  for (const change of changes) {
    const id = change[idField]
    if (!processedIds.has(id)) {
      result.push(change)
      hasChanges = true
    }
  }

  return hasChanges ? result : current
}
```

### Step 4: Run test to verify it passes

Run:
```bash
cd apps/frontend && npx vitest run src/hooks/__tests__/useIndexedDBSnapshots.test.ts
```

Expected: PASS (all 3 tests)

### Step 5: Commit

```bash
git add apps/frontend/src/hooks/useIndexedDBSnapshots.ts apps/frontend/src/hooks/__tests__/useIndexedDBSnapshots.test.ts
git commit -m "feat: add useIndexedDBSnapshots hook with buffer management

Manages 30-frame bidirectional buffer around current frame.
Loads from IndexedDB asynchronously with ~50-100ms latency."
```

---

## Task 3: Add App Startup Cleanup

**Goal:** Clear stale IndexedDB data when app loads

**Files:**
- Modify: `apps/frontend/src/main.tsx`

### Step 1: Import snapshotStore

Add import at top of `apps/frontend/src/main.tsx`:

```typescript
import { snapshotStore } from './utils/snapshotStore'
```

### Step 2: Add cleanup before render

In `apps/frontend/src/main.tsx`, wrap the render in an async IIFE that clears snapshots first:

Find:
```typescript
ReactDOM.createRoot(document.getElementById('root')!).render(
```

Replace with:
```typescript
// Clear any stale snapshot data from previous sessions
snapshotStore.clearAll().then(() => {
  ReactDOM.createRoot(document.getElementById('root')!).render(
```

And add closing parenthesis at the end:
```typescript
  )
})
```

### Step 3: Verify app still starts

Run:
```bash
cd apps/frontend && npm run dev
```

Expected: App starts normally, no errors in console

### Step 4: Commit

```bash
git add apps/frontend/src/main.tsx
git commit -m "feat: clear stale IndexedDB snapshots on app startup"
```

---

## Task 4: Integrate IndexedDB Storage into SimulationResults

**Goal:** Store snapshots in IndexedDB and use new hook

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

### Step 1: Update imports

In `apps/frontend/src/pages/simulator/components/SimulationResults.tsx`, replace:

```typescript
import { useLazySnapshotReconstruction } from '../../../hooks/useLazySnapshotReconstruction'
```

With:
```typescript
import { useEffect, useState } from 'react'
import { useIndexedDBSnapshots } from '../../../hooks/useIndexedDBSnapshots'
import { snapshotStore } from '../../../utils/snapshotStore'
```

Also ensure `useState` is imported from 'react' if not already.

### Step 2: Replace hook usage and add storage logic

Find the existing hook usage (around line 34):
```typescript
  // Lazy reconstruction with LRU caching (only reconstructs frames when viewed)
  const { getSnapshot, prefetch, getCacheStats } = useLazySnapshotReconstruction(snapshots || [], {
    cacheSize: 100, // Keep 100 most recent frames in memory
    prefetchCount: 10, // Prefetch next 10 frames during playback
  })
```

Replace with:
```typescript
  // IndexedDB storage for memory efficiency
  const [storageKey, setStorageKey] = useState<string | null>(null)
  const snapshotCount = snapshots?.length ?? 0

  // Store snapshots in IndexedDB on mount
  useEffect(() => {
    if (snapshots && snapshots.length > 0 && !storageKey) {
      snapshotStore.store(snapshots).then((key) => {
        setStorageKey(key)
      })
    }

    // Cleanup on unmount
    return () => {
      if (storageKey) {
        snapshotStore.delete(storageKey)
      }
    }
  }, [snapshots, storageKey])

  // Hook for accessing snapshots from IndexedDB
  const { currentSnapshot, isLoading, loadFrame, bufferStatus } = useIndexedDBSnapshots(
    storageKey,
    snapshotCount
  )

  // Load frame when currentFrame changes
  useEffect(() => {
    if (storageKey) {
      loadFrame(currentFrame)
    }
  }, [currentFrame, storageKey, loadFrame])
```

### Step 3: Update AnimationControls onPrefetch

Find:
```typescript
onPrefetch={(frame, direction) => prefetch(frame, direction)}
```

Replace with:
```typescript
onPrefetch={() => {}} // Prefetching handled by buffer management
```

### Step 4: Update WarehouseMap snapshot prop

Find:
```typescript
snapshot={getSnapshot(currentFrame) as any}
```

Replace with:
```typescript
snapshot={currentSnapshot as any}
```

### Step 5: Update cache stats display (or remove)

Find the cache stats section:
```typescript
{showCacheStats && (
  <div className="mt-4 p-4 bg-gray-50 rounded-lg border border-gray-200">
    <h3 className="text-sm font-semibold mb-2">Cache Statistics</h3>
    <div className="grid grid-cols-2 gap-2 text-xs">
      <div>
        <span className="text-gray-600">Size:</span>{' '}
        <span className="font-mono">
          {getCacheStats().size} / {getCacheStats().capacity}
        </span>
      </div>
      ...
    </div>
  </div>
)}
```

Replace with:
```typescript
{showCacheStats && (
  <div className="mt-4 p-4 bg-gray-50 rounded-lg border border-gray-200">
    <h3 className="text-sm font-semibold mb-2">Buffer Statistics</h3>
    <div className="grid grid-cols-2 gap-2 text-xs">
      <div>
        <span className="text-gray-600">Current:</span>{' '}
        <span className="font-mono">{bufferStatus.current}</span>
      </div>
      <div>
        <span className="text-gray-600">Buffer Range:</span>{' '}
        <span className="font-mono">
          {bufferStatus.start} - {bufferStatus.end}
        </span>
      </div>
      <div>
        <span className="text-gray-600">Loading:</span>{' '}
        <span className="font-mono">{isLoading ? 'Yes' : 'No'}</span>
      </div>
      <div>
        <span className="text-gray-600">Storage:</span>{' '}
        <span className="font-mono">{storageKey ? 'IndexedDB' : 'Pending'}</span>
      </div>
    </div>
  </div>
)}
```

### Step 6: Add loading indicator to WarehouseMap

Find the WarehouseMap wrapper div and add loading overlay:
```typescript
{config && config.mapConfig && (
  <div className="mt-4 border rounded-lg p-4 bg-gray-50 relative">
    {isLoading && (
      <div className="absolute inset-0 bg-white/50 flex items-center justify-center z-10">
        <span className="text-sm text-gray-500">Loading frame...</span>
      </div>
    )}
    <WarehouseMap
```

### Step 7: Run frontend build

Run:
```bash
cd apps/frontend && npm run check
```

Expected: No TypeScript errors

### Step 8: Commit

```bash
git add apps/frontend/src/pages/simulator/components/SimulationResults.tsx
git commit -m "feat: integrate IndexedDB snapshot storage in SimulationResults

Stores snapshots in IndexedDB on mount, loads via buffer.
Reduces memory from ~466MB to ~20MB."
```

---

## Task 5: Manual Memory Testing

**Goal:** Verify memory reduction with heap snapshot

### Step 1: Start backend server

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/api/health
```

Expected: `{"version":"1.0.0","status":"healthy",...}`

### Step 2: Start frontend dev server

```bash
cd apps/frontend && npm run dev
```

### Step 3: Run 2-hour relay simulation with snapshots

In the UI:
- System Type: relay
- Duration: 2 hours
- Enable Snapshots: true
- Snapshot Interval: 10 seconds
- VTM Count: 10, HTM Count: 5

### Step 4: Open Animation tab and scrub

1. Click Animation tab
2. Wait for "Storage: IndexedDB" in buffer stats
3. Scrub timeline back and forth
4. Jump to random frames

### Step 5: Take heap snapshot

1. Chrome DevTools → Memory tab
2. Click "Take heap snapshot"
3. Note total heap size

**Expected Results:**
- Total heap: ~50-80MB (down from ~300MB)
- No large snapshot arrays in React state

### Step 6: Test scrubbing performance

- Frames within buffer: Instant
- Random jumps: 50-100ms delay (brief loading indicator)

### Step 7: Commit results documentation

Create `docs/performance/2025-11-18-indexeddb-storage-results.md` with findings.

```bash
git add docs/performance/2025-11-18-indexeddb-storage-results.md
git commit -m "docs: add IndexedDB storage performance test results"
```

---

## Success Criteria

- [ ] Snapshots stored in IndexedDB on simulation complete
- [ ] Memory reduced from ~466MB to ~50-80MB
- [ ] Frames within buffer load instantly
- [ ] Random jumps load within 50-100ms
- [ ] Loading indicator shows during fetch
- [ ] Data cleared on app startup
- [ ] Data cleared on dialog close
- [ ] All tests pass

---

## Follow-Up Work

If performance needs further improvement:

1. **Web Worker for reconstruction** - Move delta merging off main thread
2. **Compression** - Compress snapshots before IndexedDB storage
3. **Smarter prefetching** - Predict scrubbing direction from mouse velocity

---

## References

- Design brainstorming: Session 2025-11-18
- idb library: https://github.com/jakearchibald/idb
- IndexedDB API: https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API
