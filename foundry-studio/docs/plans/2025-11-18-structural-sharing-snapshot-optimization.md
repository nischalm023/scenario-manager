# Structural Sharing Snapshot Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce animation memory usage from ~300MB to ~50MB by implementing structural sharing in snapshot reconstruction, reusing array and object references when entities haven't changed.

**Architecture:** Replace `mergeById` function with `mergeByIdWithSharing` that returns the original array when no changes exist, and reuses object references for unchanged entities. Also remove dead code (`useSnapshotReconstruction.ts`).

**Tech Stack:** React 18, TypeScript, Vitest

---

## Background

**Current Problem:**
- `useLazySnapshotReconstruction` creates new arrays for every `mergeById` call, even when no entities changed
- With 720 snapshots × 600 storage spaces = 432K object allocations
- Heap snapshot shows 61MB for storage spaces alone, 300MB total

**Solution:**
- Return original array reference when `changes` is empty (fast path)
- Only create new array when actual changes exist
- Reuse object references for unchanged entities

**Expected Impact:**
- 60-70% memory reduction (300MB → ~50MB)
- Storage spaces: 600 objects shared instead of 432K duplicated

---

## Task 1: Remove Dead Code (useSnapshotReconstruction)

**Goal:** Clean up unused eager reconstruction hook

**Files:**
- Delete: `apps/frontend/src/hooks/useSnapshotReconstruction.ts`
- Delete: `apps/frontend/src/hooks/__tests__/useSnapshotReconstruction.test.ts`

### Step 1: Verify no production usage

Run:
```bash
grep -r "useSnapshotReconstruction" apps/frontend/src --include="*.ts" --include="*.tsx" | grep -v "__tests__" | grep -v "useLazySnapshotReconstruction"
```

Expected: No output (only test file imports it)

### Step 2: Delete the files

```bash
rm apps/frontend/src/hooks/useSnapshotReconstruction.ts
rm apps/frontend/src/hooks/__tests__/useSnapshotReconstruction.test.ts
```

### Step 3: Verify frontend still builds

Run:
```bash
cd apps/frontend && npm run check
```

Expected: PASS with no errors

### Step 4: Commit

```bash
git add -A
git commit -m "chore: remove unused useSnapshotReconstruction hook

Superseded by useLazySnapshotReconstruction for better performance."
```

---

## Task 2: Write Tests for Structural Sharing Behavior

**Goal:** Add tests that verify array/object reference reuse

**Files:**
- Modify: `apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts`

### Step 1: Add test for array reuse when no changes

Add this test to `apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts`:

```typescript
it('should reuse storage space array reference when no changes in delta', () => {
  const storageSpaces = [
    { storageSpaceId: 'rack-1', position: { x: 0, y: 0 }, isActive: false, totalInventory: 100 },
    { storageSpaceId: 'rack-2', position: { x: 1, y: 0 }, isActive: false, totalInventory: 200 },
  ]

  const snapshots = [
    {
      type: 'KEYFRAME' as const,
      timestamp: 0,
      robots: [{ robotId: 'robot-1', position: { x: 0, y: 0 } }],
      relays: [],
      stations: [],
      totes: [],
      storageSpaces,
      metrics: {},
    },
    {
      type: 'DELTA' as const,
      timestamp: 10,
      robots: [{ robotId: 'robot-1', position: { x: 5, y: 0 } }], // Robot moved
      relays: [],
      stations: [],
      totes: [],
      storageSpaces: [], // No storage space changes
      metrics: {},
    },
  ]

  const { result } = renderHook(() => useLazySnapshotReconstruction(snapshots))

  const frame0 = result.current.getSnapshot(0)
  const frame1 = result.current.getSnapshot(1)

  // Storage spaces array should be the SAME reference (structural sharing)
  expect(frame1?.storageSpaces).toBe(frame0?.storageSpaces)

  // Robot should be different (it changed)
  expect(frame1?.robots).not.toBe(frame0?.robots)
})
```

### Step 2: Add test for object reuse within arrays

Add this test after the previous one:

```typescript
it('should reuse unchanged object references within arrays', () => {
  const robot1 = { robotId: 'robot-1', position: { x: 0, y: 0 } }
  const robot2 = { robotId: 'robot-2', position: { x: 1, y: 1 } }

  const snapshots = [
    {
      type: 'KEYFRAME' as const,
      timestamp: 0,
      robots: [robot1, robot2],
      relays: [],
      stations: [],
      totes: [],
      storageSpaces: [],
      metrics: {},
    },
    {
      type: 'DELTA' as const,
      timestamp: 10,
      robots: [{ robotId: 'robot-1', position: { x: 5, y: 0 } }], // Only robot-1 changed
      relays: [],
      stations: [],
      totes: [],
      storageSpaces: [],
      metrics: {},
    },
  ]

  const { result } = renderHook(() => useLazySnapshotReconstruction(snapshots))

  const frame0 = result.current.getSnapshot(0)
  const frame1 = result.current.getSnapshot(1)

  // robot-2 should be the SAME object reference (unchanged)
  const robot2Frame0 = frame0?.robots.find(r => r.robotId === 'robot-2')
  const robot2Frame1 = frame1?.robots.find(r => r.robotId === 'robot-2')
  expect(robot2Frame1).toBe(robot2Frame0)

  // robot-1 should be a DIFFERENT object (changed)
  const robot1Frame0 = frame0?.robots.find(r => r.robotId === 'robot-1')
  const robot1Frame1 = frame1?.robots.find(r => r.robotId === 'robot-1')
  expect(robot1Frame1).not.toBe(robot1Frame0)
  expect(robot1Frame1?.position.x).toBe(5)
})
```

### Step 3: Run tests to verify they fail

Run:
```bash
cd apps/frontend && npx vitest run src/hooks/__tests__/useLazySnapshotReconstruction.test.ts
```

Expected: FAIL - The new tests should fail because current `mergeById` always creates new arrays

### Step 4: Commit failing tests

```bash
git add apps/frontend/src/hooks/__tests__/useLazySnapshotReconstruction.test.ts
git commit -m "test: add structural sharing tests for snapshot reconstruction

Tests verify array and object reference reuse when entities unchanged.
Currently failing - implementation follows."
```

---

## Task 3: Implement mergeByIdWithSharing Function

**Goal:** Replace `mergeById` with structural sharing version

**Files:**
- Modify: `apps/frontend/src/hooks/useLazySnapshotReconstruction.ts`

### Step 1: Replace mergeById function

In `apps/frontend/src/hooks/useLazySnapshotReconstruction.ts`, replace the `mergeById` function (lines 238-268) with:

```typescript
/**
 * Merge entities by ID with structural sharing.
 *
 * Returns the original array reference when no changes exist (fast path).
 * Reuses object references for unchanged entities.
 * Preserves original array order to prevent visual jitter.
 */
function mergeById<T extends Record<string, any>>(
  current: T[],
  changes: T[],
  idField: keyof T
): T[] {
  // Fast path: no changes = reuse entire array
  if (!changes || changes.length === 0) {
    return current
  }

  const changesMap = new Map(changes.map((item) => [item[idField], item]))
  const result: T[] = []
  const processedIds = new Set<any>()
  let hasChanges = false

  // First pass: update existing items in original order
  for (const item of current) {
    const id = item[idField]
    const updated = changesMap.get(id)
    if (updated) {
      result.push(updated)
      hasChanges = true
    } else {
      result.push(item) // Reuse object reference
    }
    processedIds.add(id)
  }

  // Second pass: add new items not in original array
  for (const change of changes) {
    const id = change[idField]
    if (!processedIds.has(id)) {
      result.push(change)
      hasChanges = true
    }
  }

  // If nothing actually changed, return original array reference
  return hasChanges ? result : current
}
```

### Step 2: Run tests to verify they pass

Run:
```bash
cd apps/frontend && npx vitest run src/hooks/__tests__/useLazySnapshotReconstruction.test.ts
```

Expected: PASS (all tests including new structural sharing tests)

### Step 3: Run full frontend check

Run:
```bash
cd apps/frontend && npm run check
```

Expected: PASS with no TypeScript errors

### Step 4: Commit implementation

```bash
git add apps/frontend/src/hooks/useLazySnapshotReconstruction.ts
git commit -m "feat: implement structural sharing in snapshot reconstruction

- Return original array when no changes (fast path)
- Reuse object references for unchanged entities
- Expected 60-70% memory reduction for animation playback"
```

---

## Task 4: Manual Memory Testing

**Goal:** Verify memory reduction with heap snapshot

### Step 1: Start backend server

```bash
rm -f /tmp/server.log /tmp/server.pid
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/health
```

Expected: `{"status":"UP"}`

### Step 2: Start frontend dev server

```bash
cd apps/frontend && npm run dev
```

### Step 3: Run 2-hour relay simulation with snapshots

In the UI, configure:
- System Type: relay
- Duration: 2 hours
- Enable Snapshots: true
- Snapshot Interval: 10 seconds
- VTM Count: 10
- HTM Count: 5

Run simulation and wait for completion.

### Step 4: Open Animation tab and scrub through timeline

1. Click "Animation" tab
2. Scrub timeline slider back and forth multiple times
3. Visit frames at different positions (beginning, middle, end)

### Step 5: Take heap snapshot and compare

1. Open Chrome DevTools → Memory tab
2. Click "Take heap snapshot"
3. Search for "storageSpaceId" in the snapshot
4. Note the count and retained size

**Expected Results:**
- Before optimization: ~437K storage space objects, ~61MB retained
- After optimization: ~600-1000 storage space objects, ~1-2MB retained

### Step 6: Document results

Create `docs/performance/2025-11-18-structural-sharing-results.md`:

```markdown
# Structural Sharing Optimization Results

**Date:** 2025-11-18
**Test:** 2-hour relay simulation, 720 snapshots, 10 VTMs, 5 HTMs

## Memory Comparison

**Before optimization:**
- Storage space objects: [MEASURED]
- Storage space retained size: [MEASURED]
- Total heap: [MEASURED]

**After optimization:**
- Storage space objects: [MEASURED]
- Storage space retained size: [MEASURED]
- Total heap: [MEASURED]

## Reduction

- Storage space objects: [X]% reduction
- Total heap: [X]% reduction

## Observations

[Notes on animation smoothness, scrubbing responsiveness]
```

### Step 7: Commit results

```bash
git add docs/performance/2025-11-18-structural-sharing-results.md
git commit -m "docs: add structural sharing optimization memory test results"
```

### Step 8: Cleanup

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
```

---

## Success Criteria

- [ ] Dead code removed (useSnapshotReconstruction.ts)
- [ ] All existing tests pass
- [ ] New structural sharing tests pass
- [ ] Frontend builds with no errors
- [ ] Storage space objects reduced by >90% (from 437K to <5K)
- [ ] Total heap reduced by >50% (from 300MB to <150MB)
- [ ] Animation playback remains smooth
- [ ] Scrubbing timeline remains responsive

---

## Follow-Up Work (If Memory Still Too High)

If structural sharing alone doesn't achieve target memory:

1. **Reduce LRU cache size** - From 100 to 50 frames
2. **Aggressive GC hints** - Call `cache.current.clear()` on tab switch
3. **IndexedDB offloading** - Move raw snapshots to disk, reconstruct on-demand

---

## References

- Heap analysis: Brainstorming session 2025-11-18
- Lazy reconstruction implementation: `apps/frontend/src/hooks/useLazySnapshotReconstruction.ts`
- React structural sharing patterns: https://react.dev/learn/updating-objects-in-state
