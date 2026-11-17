# Binary Snapshot Pipeline

Detailed reference for the simulation playback system. The main skill file (SKILL.md) has the data flow summary — this file has implementation details.

## Data Flow

```
SimulatorPage
  └→ fetchBinarySnapshots(simulationId)     [services/api.ts — raw fetch()]
      └→ ArrayBuffer
          └→ BinarySnapshotReader.readAll(buffer)  [utils/BinarySnapshotReader.ts]
              └→ Snapshot[]
                  └→ useLazySnapshotReconstruction(snapshots)  [hooks/]
                      └→ { getSnapshot(frame), prefetch(frame, dir) }
                          └→ AnimationControls                 [components/]
                              └→ WarehouseMap (renders current frame)
```

## Binary Format (WHSV)

Magic number: `0x57485356` ('WHSV' in ASCII, little-endian uint32).
Format version: `0x0001`.

### Header (26 bytes)

| Offset | Type | Field |
|--------|------|-------|
| 0 | uint32 | magic (`0x57485356`) |
| 4 | uint16 | version |
| 6 | uint8 | snapshotType (1=KEYFRAME, other=DELTA) |
| 7 | uint8 | flags (reserved) |
| 8 | float64 | timestamp (simulation seconds) |
| 16 | uint8 | rtpRobotCount |
| 17 | uint8 | vtmRobotCount |
| 18 | uint8 | htmRobotCount |
| 19 | uint8 | stationCount |
| 20 | uint16 | rackCount |
| 22 | uint16 | toteCount |
| 24 | uint16 | relayCount |

### Sections

After the header, the buffer contains sections for each entity type. Each section has a 4-byte header:
- `uint16` count
- `uint8` bytesPerEntity
- 1 reserved byte

Then `count * bytesPerEntity` bytes of entity data.

Entity byte sizes (V1): Robot=8, Station=16, Rack=9, Tote=12, Relay=7, Metrics=16.

### Entity Packing

Robot packed byte (1 byte): bits 0-1 type (0=RTP, 1=VTM, 2=HTM), bits 2-6 status (0-31), bit 7 hasTask.

Tote packed byte: bits 0-1 locationType (0=IN_RACK, 1=AT_RELAY, 2=AT_STATION, 3=WITH_ROBOT), bits 2-7 taskCount.

Rack packed byte: bit 0 isActive, bits 1-7 taskCount.

### Keyframe vs Delta

- **KEYFRAME** (snapshotType=1): Full state — all entities present
- **DELTA** (snapshotType!=1): Partial — only changed entities. Reconstruction requires merging onto previous keyframe.

Multiple snapshots are concatenated back-to-back in the response buffer.

## BinarySnapshotReader

**Location**: `utils/BinarySnapshotReader.ts` (679 lines)

```typescript
// Parse all snapshots from a concatenated binary buffer
const snapshots: Snapshot[] = BinarySnapshotReader.readAll(buffer)
```

Exported types: `Position`, `RobotType`, `RobotStatus` (enum with 18 values), `Robot`, `Station`, `StorageSpace`, `Rack`, `Tote`, `Relay`, `SnapshotMetrics`, `SnapshotType`, `Snapshot`.

## useLazySnapshotReconstruction

**Location**: `hooks/useLazySnapshotReconstruction.ts` (388 lines)

```typescript
const { getSnapshot, getSnapshotWithAggregation, prefetch, getCacheStats } =
  useLazySnapshotReconstruction(rawSnapshots, { cacheSize: 100, prefetchCount: 10 })
```

### Reconstruction Algorithm

1. On load: scans `rawSnapshots` to build `keyframeIndices[]` array
2. `getSnapshot(frameIndex)`: checks LRU cache → on miss:
   - Binary search `keyframeIndices` for nearest preceding keyframe
   - Start with keyframe state, apply each DELTA from `[keyframe+1 .. frameIndex]` via `mergeById`
   - Store result in LRU cache
3. `mergeById<T>`: updates existing items by ID, appends new items. Returns original array reference when no changes (structural sharing for React performance).

### Aggregation

`getSnapshotWithAggregation(frameIndex)`:
- Calls `getSnapshot(frameIndex)` then `aggregateStorageSpaces()`
- Separates racks and totes from `storageSpaces[]`
- Matches totes to racks by exact position (`x === rack.x && y === rack.y`)
- Aggregates `totalInventory`, `totalReservedInventory`, `toteCount` onto racks
- Filters totes out of returned array

### Cache

- `LRUCache<number, ReconstructedSnapshot>` (default capacity: 100)
- Cleared when `rawSnapshots` reference changes
- Stats: `{ size, capacity, hits, misses, hitRate }`

### Prefetch

- `prefetch(currentFrame, 'forward')`: pre-computes next N frames (default 10)
- `prefetch(currentFrame, 'backward')`: pre-computes previous N frames
- Skips frames already in cache

## AnimationControls

**Location**: `components/AnimationControls.tsx` (133 lines)

Props: `snapshotCount`, `currentTimestamp`, `currentFrame`, `onFrameChange`, `onPrefetch?`

- **Playback speeds**: 0.5x, 1x, 2x, 5x (via `setInterval` with `intervalMs = 1000 / speed`)
- **Scrubbing**: slider pauses playback, triggers `onFrameChange` + `onPrefetch(frame, 'forward')`
- **Reset**: sets frame to 0, pauses
- **Auto-stop**: pauses at last frame; pressing play from last frame resets to 0
