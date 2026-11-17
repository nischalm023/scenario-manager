---
name: relay-system
description: Use for ANY VTM, HTM, tote, zone, or relay position work. The authoritative source for relay system architecture and orchestration patterns.
---

# Relay System Architecture

---

## System Types

| System | Description |
|--------|-------------|
| **R2P (Racks-to-Picker)** | Robots carry entire racks to stations |
| **Relay System** | Specialized robots for vertical (VTM) and horizontal (HTM) transport |

---

## Robot Types

| Robot | Behavior | Scope |
|-------|----------|-------|
| **R2P Robot** | General-purpose, carries entire racks | Global |
| **VTM (Vertical Tote Mover)** | Retrieves totes from racks in aisles | Zone-bound |
| **HTM (Horizontal Tote Mover)** | Transports totes between relay positions and stations | Global |

---

## Key Concepts

**Tote**: Container (6 tasks capacity) carrying PickingTasks directly (no RelayTask wrapper)

**RelayPosition**: Handoff point holding ONE tote (not a queue). Uses Desmo-J Res for automatic blocking when occupied (cantilever positions)

**Zone**: Spatial division with VTM assignments (left/right aisles per rack block). Tracks capacity directly:
```
availableCapacity = physicalCapacity - occupied - reserved
```

**ZoneManager**: Balances VTM assignments across zones

**RelayPositionManager**: Zone-aware relay selection (60% VTM travel + 40% HTM travel optimization)

---

## Workflow

```
Order → VTM picks tote from rack → VTM to relay position (blocks if full via Desmo-J Res)
→ HTM picks from relay (FIFO) → HTM to station → Operator picks (reuses StationProcessor)
→ HTM returns tote to relay → VTM returns to rack
```

---

## Process Architecture

**DesmojVtmProcess**: Zone-bound tote retrieval, relay delivery, return journey handling
- Extends `SimProcess` with `lifeCycle()`
- Assigned to specific zone
- Handles forward (rack→relay) and return (relay→rack) journeys

**DesmojHtmProcess**: Relay-to-station transport, reuses StationProcessor for picking
- Extends `SimProcess` with `lifeCycle()`
- Global scope (can visit any relay/station)
- Manages tote lifecycle at stations

---

## VTM/HTM Orchestration (Critical)

### Producer/Consumer Model
- **VTM = Producer**: Adds totes to relay positions (zone-bound)
- **HTM = Consumer**: Removes totes from relay positions (global)
- **Backpressure required** to prevent VTMs from overwhelming relays

### Zone Capacity Management
```java
// Zone tracks:
physicalCapacity  // Number of relay positions in zone
occupied          // Totes currently at relay
reserved          // In-transit deliveries

// Utilization calculation
utilization = (occupied + reserved) / physicalCapacity

// VTMs must check zone-specific capacity
zone.getAvailableCapacity()  // NOT global relay count
```

### Backpressure Control (VTM Lifecycle)
```java
// Priority 1: Check zone congestion
if (zoneUtilization >= 80%) {
    if (returnTaskExists) {
        executeReturnJourney();  // Help clear congestion
    } else {
        idle();  // Wait for HTMs to clear totes
        continue;  // Do NOT proceed to forward journey!
    }
}
// Priority 2: Forward journey (only when zone not congested)
```

**Why this matters:**
- Without backpressure, VTMs fill relays faster than HTMs can process
- Results in deadlock: all relays full, VTMs blocked, no progress
- VTMs must idle when zone >80% to let HTMs catch up

### Claim Mechanism (Prevent Duplicate Selection)
```java
// When selecting tote for return
tote.setAssignedVtmId(vtmId);

// Filter in selection queries
assignedVtmId == null

// Clear after journey completes
tote.setAssignedVtmId(null);
```
Safe due to Desmo-J single-threaded execution model.

### Return Journey Logic
- Triggered when zone capacity high (>80%) and empty totes exist at relay
- VTM searches ONLY relay positions in its assigned zone (`relay.getOwnerZoneId()`)
- Selects closest tote to minimize travel distance
- Returns tote to rack, freeing relay capacity for new deliveries

---

## Metrics Integration

**VTM Metrics**:
- `VtmCycleCompleted`: Tote ID, source rack, relay position, cycle time

**HTM Metrics**:
- `HtmCycleCompleted`: Tote ID, stations visited, total picks, cycle time

**Congestion Metrics**:
- `RelayPositionBlocked`: Robot ID, relay ID, wait time

Collectors handle via `@MetricHandler` annotations.

---

## Benefits of Relay System

1. **Faster fulfillment** - Only totes travel, not entire racks
2. **Zone optimization** - Parallel processing within zones
3. **Realistic congestion modeling** - Desmo-J resource blocking
4. **Scalability** - Add VTMs/HTMs independently

---

## Key Classes

| Class | Location | Purpose |
|-------|----------|---------|
| `DesmojVtmProcess` | `simulator/` | VTM robot process |
| `DesmojHtmProcess` | `simulator/` | HTM robot process |
| `Tote` | `domain/` | Tote entity |
| `TotePool` | `domain/` | Tote lifecycle management |
| `RelayPosition` | `domain/` | Relay handoff point |
| `Zone` | `domain/` | Spatial division |
| `ZoneManager` | `domain/` | Zone assignment balancing |
| `RelayPositionManager` | `domain/` | Relay selection logic |

---

## Documentation

- `specs/relay_system/design.md` - Original design
- `specs/relay_system/IMPLEMENTATION_NOTES.md` - Implementation details
- `docs/designs/reservation-services-design.md` - Reservation architecture

---

## Related Skills

- `.claude/skills/java-backend/` - Desmo-J patterns
- `.claude/skills/debugging/` - Troubleshooting
