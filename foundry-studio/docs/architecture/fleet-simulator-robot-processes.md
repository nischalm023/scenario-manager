# Fleet Simulator — Robot Processes

Each robot is a Desmo-J `SimProcess` with its own lifecycle loop. Three robot types serve two warehouse system types.

## Robot Types

| Type | System | Moves | Zone-bound | Operator interaction |
|------|--------|-------|------------|---------------------|
| **RTP** | RTP | Entire racks: storage → station → storage | No | Yes (blocks for operator) |
| **VTM** | RELAY | Single totes: rack → relay position | Yes (assigned zones) | No |
| **HTM** | RELAY | Single totes: relay → station → relay | No | Yes (blocks for operator) |

## RTP Robot Lifecycle

`DesmojRtpRobotProcess` — transports racks from storage to picking stations and back.

```mermaid
flowchart TD
    START(["Start"]) --> FIND["Find rack with tasks\n(state=IN_STORAGE, closest)"]
    FIND --> FOUND{Found?}
    FOUND -->|No| IDLE["hold(1s)\nState: IDLE"]
    IDLE --> FIND

    FOUND -->|Yes| OPT["Optimize station visit order\nReserve rack"]
    OPT --> NAV1["Navigate to rack\n(hold for travel time)"]
    NAV1 --> LOAD["Load rack\nhold(6s)"]

    LOAD --> STATION["Navigate to next station\n(hold for travel time)"]
    STATION --> WAIT["Res.provide(1)\n⏳ blocks if operator busy"]
    WAIT --> PICK["Process picks at station\nhold(operatorTime × quantity)"]
    PICK --> RELEASE["Res.takeBack(1)"]
    RELEASE --> MORE{More\nstations?}
    MORE -->|Yes| STATION
    MORE -->|No| NAV2["Navigate to storage position\n(hold for travel time)"]
    NAV2 --> UNLOAD["Unload rack\nhold(6s)"]
    UNLOAD --> EMIT["Emit RackCycleCompleted"]
    EMIT --> FIND

    style WAIT fill:#ff9,stroke:#cc0
    style PICK fill:#9f9,stroke:#0a0
```

**Constants:** `LOADING_TIME = 6s`, `UNLOADING_TIME = 6s`, `IDLE_CHECK = 1s`

**Multi-station optimization:** Nearest-neighbor heuristic orders station visits to minimize total travel.

## VTM Lifecycle

`DesmojVtmProcess` — vertical tote mover, zone-bound. Priority-based decision making prevents relay congestion.

```mermaid
flowchart TD
    START(["Start"]) --> P1{"Priority 1:\nRelay congestion?"}
    P1 -->|"Empty tote at relay"| RETURN["Return journey:\nRelay → Rack"]

    P1 -->|No| P2{"Priority 2:\nZone congestion?\n(relay util > 80%)"}
    P2 -->|Yes| IDLE["hold(2s)\nState: IDLE"]

    P2 -->|No| P3{"Priority 3:\nTote with tasks?"}
    P3 -->|Yes| FORWARD["Forward journey:\nRack → Relay"]

    P3 -->|No| P4{"Priority 4:\nOpportunistic\nreturn?"}
    P4 -->|"Empty tote exists"| RETURN
    P4 -->|No| IDLE

    IDLE --> START
    RETURN --> START

    FORWARD --> NAV1["Navigate to rack (empty)\n(hold for travel)"]
    NAV1 --> LOAD["Load tote from rack\nhold(9s)"]
    LOAD --> NAV2["Navigate to relay (carrying)\n(hold for travel)"]
    NAV2 --> DROP{"Relay slot\navailable?"}
    DROP -->|Yes| UNLOAD["Drop tote at relay\nhold(9s)"]
    DROP -->|"No (Res full)"| BLOCK["⏳ Block until slot frees"]
    BLOCK --> UNLOAD
    UNLOAD --> EMIT["Emit VtmCycleCompleted"]
    EMIT --> START

    style BLOCK fill:#ff9,stroke:#cc0
    style P1 fill:#fdd,stroke:#c00
    style P2 fill:#fdd,stroke:#c00
```

**Constants:** `LOADING_TIME = 9s`, `UNLOADING_TIME = 9s`, `IDLE_CHECK = 2s`, `HIGH_RELAY_UTIL = 80%`

**Zone-bound:** Each VTM only services racks/relays in its assigned zones. Prevents cross-zone traffic.

**Congestion management:** Priority system ensures return journeys (clearing relay) always happen before forward journeys (adding to relay).

## HTM Lifecycle

`DesmojHtmProcess` — horizontal tote mover, global operation. Delivers totes from relays to stations.

```mermaid
flowchart TD
    START(["Start"]) --> FIND["Find relay with tote + tasks\n(FIFO order, any zone)"]
    FIND --> FOUND{Found?}
    FOUND -->|No| IDLE["hold(2s)\nState: IDLE"]
    IDLE --> FIND

    FOUND -->|Yes| CLAIM["Claim tote\n(set assignedHtmId)"]
    CLAIM --> NAV1["Navigate to relay (empty)\n(hold for travel)"]
    NAV1 --> LOAD["Load tote from relay\nhold(15s)\nFrees relay Res slot"]

    LOAD --> NAV2["Navigate to station (carrying)\n(hold for travel)"]
    NAV2 --> WAIT["Res.provide(1)\n⏳ blocks if operator busy"]
    WAIT --> PICK["Process picks at station\nhold(operatorTime × quantity)"]
    PICK --> RELEASE["Res.takeBack(1)"]

    RELEASE --> SELECT["Select return relay\n(RelayBackToStoreSelector)"]
    SELECT --> NAV3["Navigate to relay (carrying)\n(hold for travel)"]
    NAV3 --> UNLOAD["Unload tote at relay\nhold(15s)"]
    UNLOAD --> EMIT["Emit HtmCycleCompleted"]
    EMIT --> FIND

    style WAIT fill:#ff9,stroke:#cc0
    style PICK fill:#9f9,stroke:#0a0
```

**Constants:** `LOADING_TIME = 15s`, `UNLOADING_TIME = 15s`, `IDLE_CHECK = 2s`

**Currently single-station per cycle.** Multi-station planned for future.

## Navigation Integration

All robots share the same pathfinding interface via `DesmojWarehouseModel.findPath()`:

```mermaid
sequenceDiagram
    participant Robot
    participant Model as DesmojWarehouseModel
    participant Nav as Navigation Service

    Robot->>Model: findPath(position, dest, robotType, isCarrying)
    Model->>Nav: FindPath (gRPC :9092)
    Nav-->>Model: PathResult (waypoints)
    Model-->>Robot: PathResult

    loop Each waypoint
        Robot->>Robot: executeAction(waypoint)
        Robot->>Robot: hold(timeDelta × navFactor)
    end
```

**Obstacle avoidance modes:**
- `isCarrying = false` — empty robot, can use narrow paths through racks
- `isCarrying = true` — loaded robot, needs wider clearance

**navFactor** — configurable multiplier on travel time (default 1.0, increase to model congestion).

## Operator Resource Queueing

Operators are modeled as Desmo-J `Res` resources with capacity=1 per station:

```mermaid
sequenceDiagram
    participant R1 as Robot A
    participant R2 as Robot B
    participant Op as Station 1 (Res, cap=1)

    R1->>Op: provide(1) → granted
    Note over R1,Op: Robot A picks items
    R2->>Op: provide(1) → ⏳ BLOCKED
    R1->>Op: takeBack(1)
    Op-->>R2: unblocked, granted
    Note over R2,Op: Robot B picks items
    R2->>Op: takeBack(1)
```

Only RTP and HTM robots interact with operators. VTMs drop totes at relays (no operator needed).

## Task Processing at Station

`DesmojStationHelper.processStorageSpaceTasks()` — called while operator resource is held:

1. For each task at this station on the rack/tote:
   - Remove task from storage space
   - Consume inventory
   - Emit `TaskCompleted` event
   - Check if order is fully completed
2. **Batched time advancement** — single `hold(totalOperatorTime)` at end
   - More efficient than per-task holds
   - `totalOperatorTime = operatorTimePerPick × totalQuantity`

## Cycle Metrics

### RTP Cycle (`RackCycleCompleted`)

| Metric | Description |
|--------|-------------|
| travelTime | Total navigation time (outbound + return) |
| processingTime | Total operator time at stations |
| totalPicks | Items picked across all stations |
| stationsVisited | List of station IDs |
| isMultiStation | Whether multiple stations visited |
| perStationPicks | Breakdown per station |

### VTM Cycle (`VtmCycleCompleted`)

| Metric | Description |
|--------|-------------|
| totalTravelTime | Navigation time (to rack + to relay) |
| toteId | Tote transported |
| rackId | Source rack |
| relayId | Destination relay |

### HTM Cycle (`HtmCycleCompleted`)

| Metric | Description |
|--------|-------------|
| totalTravelTime | Navigation time (relay → station → relay) |
| processingTime | Total operator time |
| totalPicks | Items picked |
| stationsVisited | List of station IDs |
| perStationPicks | Breakdown per station |

## Comparison Table

| Aspect | RTP | VTM | HTM |
|--------|-----|-----|-----|
| Cargo | Entire rack | Single tote | Single tote |
| Pickup from | Storage position | Rack in zone | Relay position |
| Deliver to | Station(s) | Relay position | Station(s) |
| Zone-bound | No | Yes | No |
| Operator queue | Yes | No | Yes |
| Multi-station | Yes (optimized) | No | No (planned) |
| Loading time | 6s | 9s | 15s |
| Unloading time | 6s | 9s | 15s |
| Idle check | 1s | 2s | 2s |
| Blocking on | Operator `Res` | Relay `Res` capacity | Operator `Res` |

## Key Files

| Class | Path (relative to `fleet-simulator/src/main/java/.../`) |
|-------|------|
| `DesmojRtpRobotProcess` | `simulator/orchestrator/DesmojRtpRobotProcess.java` |
| `DesmojVtmProcess` | `simulator/orchestrator/DesmojVtmProcess.java` |
| `DesmojHtmProcess` | `simulator/orchestrator/DesmojHtmProcess.java` |
| `DesmojStationHelper` | `simulator/orchestrator/helper/DesmojStationHelper.java` |
| `DesmojBinManagementProcess` | `simulator/orchestrator/DesmojBinManagementProcess.java` |
| `DesmojMetricMonitoringProcess` | `simulator/orchestrator/DesmojMetricMonitoringProcess.java` |
| `DesmojSnapshotCaptureProcess` | `simulator/snapshot/DesmojSnapshotCaptureProcess.java` |
| `Robot` | `domain/robot/Robot.java` |
