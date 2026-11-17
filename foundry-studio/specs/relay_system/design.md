# Relay System Enhancement: Design & Implementation Plan

**Date:** October 29, 2025  
**Type:** Major Architectural Enhancement  
**Complexity:** High  
**Est. Timeline:** 8-12 weeks

## Executive Summary

This document outlines the design and implementation plan for introducing a **relay-based fulfillment system** to the warehouse simulator. This is a fundamental architectural shift from the current "racks-to-picker" (R2P) model to a "goods-to-person" relay system using Vertical Tote Movers (VTMs) and Horizontal Tote Movers (HTMs).

### Current System (R2P)

```
[Rack in Storage] → [Robot] → [Station] → [Robot] → [Rack to Storage]
```

### New System (Relay)

```
[Rack in Aisle] → [VTM] → [Relay Buffer] → [HTM] → [Station] → [HTM] → [Relay Buffer] → [VTM] → [Rack in Aisle]
```

---

## Table of Contents

1. [Problem Statement & Motivation](#1-problem-statement--motivation)
2. [System Architecture Overview](#2-system-architecture-overview)
3. [Core Domain Model Changes](#3-core-domain-model-changes)
4. [Critical Abstractions](#4-critical-abstractions)
5. [Task Orchestration Strategy](#5-task-orchestration-strategy)
6. [Algorithm Adaptations](#6-algorithm-adaptations)
7. [Map & Spatial Changes](#7-map--spatial-changes)
8. [Simulation Engine Changes](#8-simulation-engine-changes)
9. [API & Schema Changes](#9-api--schema-changes)
10. [Frontend Visualization](#10-frontend-visualization)
11. [Testing Strategy](#11-testing-strategy)
12. [Migration & Rollout Plan](#12-migration--rollout-plan)
13. [Performance Considerations](#13-performance-considerations)
14. [Open Questions & Decisions](#14-open-questions--decisions)

---

## 1. Problem Statement & Motivation

### Current Limitations

- Entire rack must travel to station for a single SKU pick
- High travel time for racks (large, heavy objects)
- No zone-based picking optimization
- Inefficient for multi-zone orders

### Relay System Benefits

- **Faster picks**: Only totes travel, not entire racks
- **Zone optimization**: VTMs specialized for aisle operations
- **Parallel processing**: Multiple VTMs can work simultaneously in different zones
- **Better throughput**: HTMs focused on horizontal transport while VTMs handle vertical
- **Realistic modeling**: Matches real-world AutoStore/Ocado-style systems

---

## 2. System Architecture Overview

### High-Level Component Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                      Simulation Engine                          │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │         Relay Task Orchestrator (NEW)                    │   │
│  │  - Multi-stage workflow coordination                     │   │
│  │  - Handoff management at relay buffers                   │   │
│  │  - Resource allocation (VTM/HTM)                         │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐     │
│  │   VTM       │  │   HTM       │  │   Tote Container    │     │
│  │   Fleet     │  │   Fleet     │  │   Pool              │     │
│  │   (NEW)     │  │   (NEW)     │  │   (NEW)             │     │
│  └─────────────┘  └─────────────┘  └─────────────────────┘     │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │         Robot Base Class (ENHANCED)                      │   │
│  │  - robot_type: RobotType enum                            │   │
│  │  - zone_assignment: Zone | None                          │   │
│  │  - capacity_constraints: ToteCapacity                    │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Warehouse Map                              │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │         Relay Location Manager (NEW)                     │   │
│  │  - Aisle-to-relay mappings                               │   │
│  │  - Buffer capacity management                            │   │
│  │  - Spatial indexing for nearest relay                    │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐                    │
│  │  Relay Buffers   │  │  Aisle Zones     │                    │
│  │  (GridCoord)     │  │  (NEW)           │                    │
│  └──────────────────┘  └──────────────────┘                    │
└─────────────────────────────────────────────────────────────────┘
```

### Operational Flow

```
┌─────────────────────────────────────────────────────────────────┐
│ Phase 1: Order Allocation                                       │
│  - Order → Multiple SKUs → Multiple Aisles                      │
│  - Create composite relay tasks                                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 2: VTM Retrieval (Per Aisle)                              │
│  - VTM picks tote from rack in aisle                            │
│  - VTM transports tote to nearest relay buffer                  │
│  - Tote placed in buffer queue                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 3: Handoff at Relay Buffer                                │
│  - VTM releases tote to buffer                                  │
│  - HTM claims tote from buffer (FIFO/Priority)                  │
│  - Resource state transitions                                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 4: HTM Station Transport                                  │
│  - HTM picks tote from relay buffer                             │
│  - HTM transports to assigned station                           │
│  - Operator picks items from tote                               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 5: Return Journey                                         │
│  - HTM returns empty tote to relay buffer                       │
│  - VTM retrieves tote from buffer                               │
│  - VTM returns tote to original rack in aisle                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Core Domain Model Changes

### 3.1 Robot Type Enumeration (NEW)

**File:** `apps/backend/src/queue_simulator/domain/robots.py`

```python
class RobotType(Enum):
    """Types of robots in the warehouse system"""
    R2P = "r2p"              # Racks-to-Picker (legacy)
    VTM = "vtm"              # Vertical Tote Mover
    HTM = "htm"              # Horizontal Tote Mover
    HYBRID = "hybrid"        # Can do both (for transition)


class RobotState(Enum):
    """Enhanced robot states for relay system"""
    # Existing states
    IDLE = "idle"
    ERROR = "error"

    # R2P-specific states (legacy)
    MOVING_TO_RACK = "moving_to_rack"
    LOADING_RACK = "loading_rack"
    TRANSPORTING_TO_STATION = "transporting_to_station"
    AT_STATION = "at_station"
    TRANSPORTING_TO_STORAGE = "transporting_to_storage"
    UNLOADING_RACK = "unloading_rack"

    # Relay-specific states (NEW)
    MOVING_TO_TOTE = "moving_to_tote"                    # VTM/HTM going to tote
    LOADING_TOTE = "loading_tote"                        # Picking up tote
    TRANSPORTING_TO_RELAY = "transporting_to_relay"      # VTM → relay buffer
    AT_RELAY_BUFFER = "at_relay_buffer"                  # Waiting at handoff point
    TRANSPORTING_FROM_RELAY = "transporting_from_relay"  # HTM → station
    TRANSPORTING_BACK_TO_RELAY = "transporting_back_to_relay"  # HTM → relay buffer
    UNLOADING_TOTE = "unloading_tote"                    # Releasing tote
    WAITING_FOR_HANDOFF = "waiting_for_handoff"          # At buffer, waiting


class Robot:
    """Enhanced Robot class to support multiple robot types"""

    def __init__(
        self,
        robot_id: int,
        speed: float = 1.0,
        robot_type: RobotType = RobotType.R2P,  # NEW
        zone_assignment: Optional[Zone] = None,  # NEW
        tote_capacity: int = 1  # NEW: How many totes can carry
    ):
        self.id = robot_id
        self.speed = speed
        self.robot_type = robot_type  # NEW
        self.zone_assignment = zone_assignment  # NEW: For VTMs in specific aisles
        self.tote_capacity = tote_capacity  # NEW

        # State management
        self.state = RobotState.IDLE
        self.current_position: GridCoordinate = GridCoordinate(row=0, col=0)
        self.target_position: GridCoordinate = GridCoordinate(row=0, col=0)

        # Payload management
        self.carrying_rack: Optional[Rack] = None  # For R2P mode
        self.carrying_totes: list[Tote] = []  # NEW: For relay mode

        # Task and metrics
        self.current_task: Optional[PickingTask | RelayTask] = None  # ENHANCED
        self.tasks_completed = 0
        self.total_working_time = 0.0
        self.total_idle_time = 0.0
        self.total_distance_traveled = 0.0

        # Relay-specific metrics (NEW)
        self.total_handoffs = 0
        self.total_wait_time_at_relay = 0.0

    def can_handle_task(self, task: Task) -> bool:
        """Check if robot can handle the given task type"""
        if isinstance(task, RackTask):
            return self.robot_type in [RobotType.R2P, RobotType.HYBRID]
        elif isinstance(task, RelayTask):
            if task.stage == TaskStage.VTM_RETRIEVAL:
                return self.robot_type == RobotType.VTM
            elif task.stage in [TaskStage.HTM_TO_STATION, TaskStage.HTM_RETURN]:
                return self.robot_type == RobotType.HTM
        return False

    def has_capacity(self) -> bool:
        """Check if robot has capacity to carry more totes"""
        return len(self.carrying_totes) < self.tote_capacity
```

---

### 3.2 Tote Abstraction (NEW)

**File:** `apps/backend/src/queue_simulator/domain/containers.py` (NEW)

```python
from dataclasses import dataclass, field
from enum import Enum
from typing import Optional
from queue_simulator.domain.map.coordinates import GridCoordinate
from queue_simulator.domain import PickTask


class ToteState(Enum):
    """States a tote can be in during its journey"""
    IN_RACK = "in_rack"  # Stored in rack
    WITH_VTM = "with_vtm"  # Being carried by VTM
    IN_RELAY_BUFFER = "in_relay_buffer"  # At handoff point
    WITH_HTM = "with_htm"  # Being carried by HTM
    AT_STATION = "at_station"  # Being picked from
    RETURNING = "returning"  # On return journey
    EMPTY = "empty"  # No tasks, returning to rack


@dataclass
class Tote:
    """
    Represents a tote container that moves through the relay system.

    Totes are the unit of transport in the relay system, carrying items
    from racks to stations via VTMs and HTMs.
    """

    id: str
    state: ToteState = ToteState.IN_RACK

    # Location tracking
    source_rack_id: Optional[int] = None  # Original rack
    current_position: Optional[GridCoordinate] = None
    target_relay_id: Optional[str] = None  # Assigned relay buffer
    target_station_id: Optional[int] = None  # Destination station

    # Task management
    tasks: list[PickTask] = field(default_factory=list)

    # Robot assignments
    assigned_vtm_id: Optional[int] = None
    assigned_htm_id: Optional[int] = None

    # Journey metadata
    created_time: float = 0.0
    vtm_pickup_time: Optional[float] = None
    relay_arrival_time: Optional[float] = None
    htm_pickup_time: Optional[float] = None
    station_arrival_time: Optional[float] = None
    completion_time: Optional[float] = None

    # Capacity
    max_items: int = 50  # Physical capacity

    def add_task(self, task: PickTask) -> bool:
        """Add a picking task to this tote if capacity allows"""
        total_items = sum(t.quantity for t in self.tasks)
        if total_items + task.quantity <= self.max_items:
            self.tasks.append(task)
            return True
        return False

    def has_tasks(self) -> bool:
        """Check if tote has pending tasks"""
        return len(self.tasks) > 0

    def get_next_task(self) -> Optional[PickTask]:
        """Get and remove next task"""
        if self.tasks:
            return self.tasks.pop(0)
        return None

    def get_total_items(self) -> int:
        """Get total number of items in tote"""
        return sum(t.quantity for t in self.tasks)

    def to_dict(self) -> dict:
        """Serialize to dictionary"""
        return {
            "id": self.id,
            "state": self.state.value,
            "source_rack_id": self.source_rack_id,
            "current_coordinate": self.current_position,
            "target_relay_id": self.target_relay_id,
            "target_station_id": self.target_station_id,
            "num_tasks": len(self.tasks),
            "total_items": self.get_total_items(),
            "assigned_vtm_id": self.assigned_vtm_id,
            "assigned_htm_id": self.assigned_htm_id,
        }
```

---

### 3.3 Relay Task Types (NEW)

**File:** `apps/backend/src/queue_simulator/domain/relay_tasks.py` (NEW)

```python
from dataclasses import dataclass
from enum import Enum
from typing import Optional
from queue_simulator.domain.robot.robots import PickingTask


class TaskStage(Enum):
    """Stages in a relay task lifecycle"""
    PENDING = "pending"  # Not yet started
    VTM_RETRIEVAL = "vtm_retrieval"  # VTM picking from rack
    VTM_TO_RELAY = "vtm_to_relay"  # VTM transporting to relay
    IN_RELAY_BUFFER = "in_relay_buffer"  # Waiting for HTM
    HTM_TO_STATION = "htm_to_station"  # HTM transporting to station
    AT_STATION = "at_station"  # Being picked
    HTM_RETURN = "htm_return"  # HTM returning to relay
    VTM_RETURN = "vtm_return"  # VTM returning to rack
    COMPLETED = "completed"


@dataclass
class RelayTask:
    """
    Represents a complete relay task involving multiple stages and robots.

    A relay task is a composite workflow:
    1. VTM retrieves tote from rack
    2. VTM delivers to relay buffer
    3. HTM picks from relay buffer
    4. HTM delivers to station
    5. Station processing
    6. HTM returns to relay buffer
    7. VTM returns tote to rack
    """

    task_id: str
    order_id: str
    stage: TaskStage = TaskStage.PENDING

    # Tote being moved
    tote_id: str

    # Source and destination
    source_rack_id: int
    relay_buffer_id: str
    target_station_id: int

    # Picking tasks contained
    picking_tasks: list[PickingTask]

    # Robot assignments
    assigned_vtm_id: Optional[int] = None
    assigned_htm_id: Optional[int] = None

    # Timing
    created_time: float = 0.0
    vtm_start_time: Optional[float] = None
    relay_handoff_time: Optional[float] = None
    htm_start_time: Optional[float] = None
    station_start_time: Optional[float] = None
    completion_time: Optional[float] = None

    # Metrics
    total_vtm_distance: float = 0.0
    total_htm_distance: float = 0.0
    handoff_wait_time: float = 0.0

    def get_cycle_time(self) -> Optional[float]:
        """Calculate total cycle time if completed"""
        if self.completion_time and self.created_time:
            return self.completion_time - self.created_time
        return None

    def advance_stage(self, new_stage: TaskStage, current_time: float):
        """Move task to next stage"""
        self.stage = new_stage

        # Update timing based on stage
        if new_stage == TaskStage.VTM_RETRIEVAL:
            self.vtm_start_time = current_time
        elif new_stage == TaskStage.IN_RELAY_BUFFER:
            self.relay_handoff_time = current_time
        elif new_stage == TaskStage.HTM_TO_STATION:
            self.htm_start_time = current_time
        elif new_stage == TaskStage.AT_STATION:
            self.station_start_time = current_time
        elif new_stage == TaskStage.COMPLETED:
            self.completion_time = current_time
```

---

## 4. Critical Abstractions

### 4.1 Relay Buffer Manager (NEW)

**File:** `apps/backend/src/queue_simulator/domain/relay_buffers.py` (NEW)

```python
from dataclasses import dataclass, field
from collections import deque
from typing import Optional
import simpy
from queue_simulator.domain.map.coordinates import GridCoordinate
from queue_simulator.domain.inventory.totes import Tote


@dataclass
class RelayBuffer:
    """
    Represents a physical relay buffer location where VTMs and HTMs exchange totes.

    Acts as a handoff point with queue management and capacity constraints.
    """

    id: str
    position: GridCoordinate
    capacity: int = 10  # How many totes can wait

    # Queues for totes at different stages
    pending_totes: deque[Tote] = field(default_factory=deque)  # Waiting for HTM pickup

    # SimPy resources for synchronization
    vtm_slots: Optional[simpy.Resource] = None  # VTM can drop off
    htm_slots: Optional[simpy.Resource] = None  # HTM can pick up

    # Metrics
    total_handoffs: int = 0
    total_wait_time: float = 0.0
    current_utilization: float = 0.0

    # Zone association
    serving_aisles: list[str] = field(default_factory=list)  # Which aisles feed this buffer

    def is_full(self) -> bool:
        """Check if buffer is at capacity"""
        return len(self.pending_totes) >= self.capacity

    def has_totes(self) -> bool:
        """Check if buffer has totes waiting for pickup"""
        return len(self.pending_totes) > 0

    def add_tote(self, tote: Tote, arrival_time: float):
        """VTM drops off a tote"""
        if self.is_full():
            raise ValueError(f"Buffer {self.id} is full")

        tote.relay_arrival_time = arrival_time
        self.pending_totes.append(tote)
        self.current_utilization = len(self.pending_totes) / self.capacity

    def remove_tote(self, pickup_time: float) -> Optional[Tote]:
        """HTM picks up a tote (FIFO)"""
        if not self.has_totes():
            return None

        tote = self.pending_totes.popleft()
        tote.htm_pickup_time = pickup_time

        # Track wait time
        if tote.relay_arrival_time:
            wait_time = pickup_time - tote.relay_arrival_time
            self.total_wait_time += wait_time

        self.total_handoffs += 1
        self.current_utilization = len(self.pending_totes) / self.capacity
        return tote

    def get_average_wait_time(self) -> float:
        """Calculate average wait time at this buffer"""
        if self.total_handoffs == 0:
            return 0.0
        return self.total_wait_time / self.total_handoffs


class RelayBufferManager:
    """
    Manages all relay buffers in the warehouse.

    Responsibilities:
    - Buffer placement optimization
    - Routing decisions (which buffer for which aisle)
    - Load balancing across buffers
    - Capacity management
    """

    def __init__(self, map_config: WarehouseMapConfig, env: simpy.Environment):
        self.buffers: dict[str, RelayBuffer] = {}
        self.env = env
        self._generate_relay_buffers(map_config)

    def _generate_relay_buffers(self, map_config: WarehouseMapConfig):
        """
        Generate relay buffer locations based on warehouse layout.

        Strategy:
        - Place buffers at strategic locations between aisles and stations
        - Typically at ends of aisles or along highways
        - Optimize for minimal VTM + HTM travel distance
        """
        # Implementation will depend on map layout
        # For now, simple strategy: one buffer per aisle group
        pass

    def get_nearest_buffer(self, position: GridCoordinate) -> RelayBuffer:
        """Find the nearest relay buffer to a given position"""
        min_distance = float('inf')
        nearest = None

        for buffer in self.buffers.values():
            distance = position.manhattan_distance(buffer.position)
            if distance < min_distance:
                min_distance = distance
                nearest = buffer

        return nearest

    def get_optimal_buffer_for_aisle(self, aisle_id: str) -> RelayBuffer:
        """Get the designated buffer for a specific aisle"""
        # Check which buffer serves this aisle
        for buffer in self.buffers.values():
            if aisle_id in buffer.serving_aisles:
                return buffer

        # Fallback to nearest
        # (Need aisle position lookup)
        return list(self.buffers.values())[0]

    def get_buffer_with_capacity(self) -> Optional[RelayBuffer]:
        """Find a buffer with available capacity"""
        for buffer in self.buffers.values():
            if not buffer.is_full():
                return buffer
        return None
```

---

### 4.2 Zone Management (NEW)

**File:** `apps/backend/src/queue_simulator/domain/zones.py` (NEW)

```python
from dataclasses import dataclass
from typing import Optional
from queue_simulator.domain.map.coordinates import GridCoordinate, RackBlockGridDefinition


@dataclass
class AisleZone:
    """
    Represents a zone in the warehouse (typically an aisle or group of aisles).

    VTMs are assigned to specific zones and operate exclusively within them.
    """

    zone_id: str
    zone_type: str = "aisle"  # aisle, cross_aisle, buffer_area

    # Spatial bounds
    rack_blocks: list[RackBlockGridDefinition]
    boundary_coordinates: list[GridCoordinate]

    # Associated relay buffers
    primary_relay_buffer_id: str
    secondary_relay_buffer_ids: list[str]  # For load balancing

    # VTM fleet assignment
    assigned_vtm_ids: list[int]

    # Racks in this zone
    rack_ids: list[int]

    def contains_rack(self, rack_id: int) -> bool:
        """Check if a rack belongs to this zone"""
        return rack_id in self.rack_ids

    def contains_position(self, position: GridCoordinate) -> bool:
        """Check if a position is within this zone"""
        return position in self.boundary_coordinates


class ZoneManager:
    """
    Manages warehouse zones and VTM assignments.

    Responsibilities:
    - Define zone boundaries
    - Assign VTMs to zones
    - Route tasks to appropriate zones
    - Load balancing across zones
    """

    def __init__(self, warehouse_map: WarehouseMap):
        self.zones: dict[str, AisleZone] = {}
        self.rack_to_zone: dict[int, str] = {}
        self._generate_zones(warehouse_map)

    def _generate_zones(self, warehouse_map: WarehouseMap):
        """Generate zones based on rack blocks and aisles"""
        # Each rack block becomes a zone
        for block in warehouse_map._rack_blocks:
            zone = AisleZone(
                zone_id=block.block_id,
                rack_blocks=[block],
                boundary_coordinates=block.aisle_positions,
                primary_relay_buffer_id=f"relay_{block.block_id}",
                secondary_relay_buffer_ids=[],
                assigned_vtm_ids=[],
                rack_ids=[]
            )
            self.zones[zone.zone_id] = zone

    def get_zone_for_rack(self, rack_id: int) -> Optional[AisleZone]:
        """Get the zone containing a specific rack"""
        zone_id = self.rack_to_zone.get(rack_id)
        return self.zones.get(zone_id) if zone_id else None

    def assign_vtm_to_zone(self, vtm_id: int, zone_id: str):
        """Assign a VTM to operate in a specific zone"""
        if zone_id in self.zones:
            self.zones[zone_id].assigned_vtm_ids.append(vtm_id)

    def get_available_vtm_in_zone(self, zone_id: str, robots: list[Robot]) -> Optional[Robot]:
        """Find an idle VTM in the specified zone"""
        zone = self.zones.get(zone_id)
        if not zone:
            return None

        for vtm_id in zone.assigned_vtm_ids:
            robot = next((r for r in robots if r.id == vtm_id), None)
            if robot and robot.state == RobotState.IDLE:
                return robot

        return None
```

---

## 5. Task Orchestration Strategy

### 5.1 Relay Task Orchestrator (NEW)

**File:** `apps/backend/src/queue_simulator/domain/relay_orchestrator.py` (NEW)

This is the most critical component - it coordinates the multi-stage relay workflow.

```python
class RelayTaskOrchestrator:
    """
    Orchestrates the complete relay workflow with multi-stage coordination.

    Manages:
    1. VTM retrieval from rack
    2. VTM transport to relay buffer
    3. Handoff at buffer
    4. HTM transport to station
    5. Station processing
    6. Return journey

    This is a multi-step process requiring careful state management and
    synchronization between different robot types.
    """

    def __init__(
            self,
            operator_time_per_pick: float,
            metrics_manager: MetricsManager,
            relay_buffer_manager: RelayBufferManager,
            zone_manager: ZoneManager,
            tote_pool: TotePool,
    ):
        self.operator_time_per_pick = operator_time_per_pick
        self.metrics_manager = metrics_manager
        self.relay_buffer_manager = relay_buffer_manager
        self.zone_manager = zone_manager
        self.tote_pool = tote_pool

    def execute_relay_task(
            self,
            relay_task: RelayTask,
            env: simpy.Environment,
            warehouse_state: WarehouseState,
    ) -> Generator:
        """
        Execute complete relay task lifecycle.

        This is the main orchestration method that coordinates all stages.
        """

        # Stage 1: VTM Retrieval
        vtm = warehouse_state.get_available_vtm_in_zone(relay_task.source_rack_id)
        if not vtm:
            raise ValueError("No VTM available for task")

        tote = self.tote_pool.get_tote(relay_task.tote_id)
        rack = warehouse_state.get_rack(relay_task.source_rack_id)

        # VTM picks tote from rack
        vtm_retrieval_data = yield from self._vtm_retrieve_tote(
            vtm, tote, rack, env
        )
        relay_task.advance_stage(TaskStage.VTM_TO_RELAY, env.now)

        # Stage 2: VTM Transport to Relay Buffer
        relay_buffer = self.relay_buffer_manager.get_buffer(relay_task.relay_buffer_id)
        vtm_transport_data = yield from self._vtm_transport_to_relay(
            vtm, tote, relay_buffer, env
        )
        relay_task.advance_stage(TaskStage.IN_RELAY_BUFFER, env.now)

        # Stage 3: Handoff at Buffer
        # VTM releases tote to buffer
        relay_buffer.add_tote(tote, env.now)
        tote.state = ToteState.IN_RELAY_BUFFER
        vtm.carrying_totes.remove(tote)
        vtm.state = RobotState.IDLE

        # Wait for HTM to be available
        htm = None
        while not htm:
            # Check for available HTM
            htm = warehouse_state.get_available_htm()
            if not htm:
                yield env.timeout(1.0)  # Wait and retry

        # HTM picks up tote from buffer
        tote = relay_buffer.remove_tote(env.now)
        htm.carrying_totes.append(tote)
        tote.state = ToteState.WITH_HTM
        tote.assigned_htm_id = htm.id
        relay_task.advance_stage(TaskStage.HTM_TO_STATION, env.now)

        # Stage 4: HTM Transport to Station
        station = warehouse_state.get_station(relay_task.target_station_id)
        htm_transport_data = yield from self._htm_transport_to_station(
            htm, tote, station, env
        )
        relay_task.advance_stage(TaskStage.AT_STATION_WITH_RACK, env.now)

        # Stage 5: Station Processing
        processing_data = yield from self._process_tote_at_station(
            htm, tote, station, env, warehouse_state
        )

        # Stage 6: HTM Return to Relay
        return_data = yield from self._htm_return_to_relay(
            htm, tote, relay_buffer, env
        )
        relay_task.advance_stage(TaskStage.HTM_RETURN, env.now)

        # Stage 7: VTM Return to Rack
        # (Similar process in reverse)

        relay_task.advance_stage(TaskStage.COMPLETED, env.now)

        return {
            "vtm_retrieval": vtm_retrieval_data,
            "vtm_transport": vtm_transport_data,
            "htm_transport": htm_transport_data,
            "processing": processing_data,
            "return": return_data,
        }

    def _vtm_retrieve_tote(
            self,
            vtm: Robot,
            tote: Tote,
            rack: Rack,
            env: simpy.Environment,
    ) -> Generator:
        """VTM retrieves tote from rack in aisle"""
        # Move to rack
        vtm.state = RobotState.MOVING_TO_TOTE
        travel_time = vtm.move_to_position(rack.current_coordinate)
        yield env.timeout(travel_time)

        # Pick tote (loading time)
        vtm.state = RobotState.LOADING_TOTE
        loading_time = 3.0  # seconds
        yield env.timeout(loading_time)

        # Attach tote to VTM
        vtm.carrying_totes.append(tote)
        tote.state = ToteState.WITH_VTM
        tote.assigned_vtm_id = vtm.id

        return {
            "travel_time": travel_time,
            "loading_time": loading_time,
        }

    # ... (similar methods for each stage)
```

---

## 6. Algorithm Adaptations

### 6.1 Pick Algorithm Changes

**Current:** Entire rack goes to station for all SKUs on that rack

**New (Relay):**

- Orders are decomposed into tote-sized picking tasks
- Multiple totes may be needed per order if items span multiple aisles
- Tote consolidation strategy needed

**File:** `apps/backend/src/queue_simulator/domain/orders.py` (ENHANCED)

```python
class OrderProcessor:
    # ... existing code ...

    def generate_relay_tasks_for_order(
        self,
        order: Order,
        current_time: float
    ) -> list[RelayTask]:
        """
        Generate relay tasks for an order in relay mode.

        Strategy:
        1. Group SKUs by aisle/zone
        2. Create one tote per aisle group
        3. Generate RelayTask for each tote
        4. Optimize tote routing
        """
        relay_tasks = []

        # Group order items by aisle/zone
        items_by_zone = self._group_items_by_zone(order.items)

        for zone_id, zone_items in items_by_zone.items():
            # Create tote for this zone
            tote_id = f"tote_{order.order_id}_{zone_id}"
            tote = Tote(
                id=tote_id,
                source_rack_id=zone_items[0].rack_id,  # Primary rack
                created_time=current_time,
            )

            # Add picking tasks to tote
            picking_tasks = []
            for item in zone_items:
                task = PickingTask(
                    task_id=f"{order.order_id}-{item.item.sku}",
                    order_id=order.order_id,
                    item=item.item,
                    quantity=item.quantity,
                    rack_id=item.rack_id,
                    created_time=current_time,
                )
                tote.add_task(task)
                picking_tasks.append(task)

            # Determine relay buffer and station
            zone = self.zone_manager.get_zone(zone_id)
            relay_buffer_id = zone.primary_relay_buffer_id
            target_station = self._select_optimal_station_for_zone(zone_id)

            # Create relay task
            relay_task = RelayTask(
                task_id=f"relay_{tote_id}",
                order_id=order.order_id,
                tote_id=tote_id,
                source_rack_id=tote.source_rack_id,
                relay_buffer_id=relay_buffer_id,
                target_station_id=target_station.station_id,
                picking_tasks=picking_tasks,
                created_time=current_time,
            )

            relay_tasks.append(relay_task)

        return relay_tasks

    def _group_items_by_zone(self, order_items: list) -> dict:
        """Group order items by warehouse zone"""
        zone_groups = {}

        for item in order_items:
            rack_id = self.inventory.find_item_location(item.item.sku)
            zone = self.zone_manager.get_zone_for_rack(rack_id)

            if zone.zone_id not in zone_groups:
                zone_groups[zone.zone_id] = []

            zone_groups[zone.zone_id].append(item)

        return zone_groups
```

### 6.2 Put Algorithm Changes

**Consideration:** Do racks still exist in the relay system?

**Answer:** YES - racks still store items, but they don't move. VTMs retrieve totes from racks.

**Impact:** Minimal changes to put algorithm. Racks remain in fixed positions in aisles.

---

## 7. Map & Spatial Changes

### 7.1 Relay Location Generation

**File:** `apps/backend/src/queue_simulator/domain/map/map.py` (ENHANCED)

```python
class WarehouseMap:
    # ... existing code ...

    def _generate_relay_buffer_locations(self) -> list[GridCoordinate]:
        """
        Generate optimal relay buffer locations based on warehouse layout.

        Strategy:
        - Place buffers at intersections of aisles and highways
        - Ensure coverage of all zones
        - Minimize max distance from any aisle to buffer
        """
        relay_locations = []

        for rack_block in self._rack_blocks:
            # Place buffer at end of each aisle block
            # (Assuming vertical orientation)
            buffer_row = rack_block.bottom_right.row + 2  # 2 cells away
            buffer_col = (rack_block.top_left.col + rack_block.bottom_right.col) // 2

            buffer_pos = GridCoordinate(row=buffer_row, col=buffer_col)

            if self.converter.validate_grid_bounds(buffer_pos):
                relay_locations.append(buffer_pos)

        return relay_locations

    def get_relay_positions(self) -> list[GridCoordinate]:
        """Get all relay buffer positions"""
        if not hasattr(self, '_relay_positions'):
            self._relay_positions = self._generate_relay_buffer_locations()
        return self._relay_positions
```

### 7.2 Zone-Aware Pathfinding (FUTURE)

Currently robots "fly" (Manhattan distance). For relay system, may want:

- VTMs constrained to aisle lanes
- HTMs use highways
- Pathfinding respects zones

**Decision:** Phase 2 enhancement. Start with Manhattan distance.

---

## 8. Simulation Engine Changes

### 8.1 Dual-Mode Support

**File:** `apps/backend/src/queue_simulator/simulator/simulation_engine.py` (ENHANCED)

```python
class SimulationEngine:
    def __init__(self, config: dict, warehouse_components: dict, metrics_manager: MetricsManager):
        # ... existing code ...

        # Determine operation mode
        self.operation_mode = config.get("operation_mode", "r2p")  # "r2p" or "relay"

        # Initialize appropriate orchestrator
        if self.operation_mode == "relay":
            self.relay_orchestrator = self._get_relay_orchestrator()
            self.relay_buffer_manager = warehouse_components["relay_buffer_manager"]
            self.zone_manager = warehouse_components["zone_manager"]
            self.tote_pool = warehouse_components["tote_pool"]
        else:
            self.task_orchestrator = self._get_task_orchestrator()  # R2P mode

    def _robot_behavior_process(self, robot: Robot):
        """
        Main robot behavior process - handles both R2P and relay modes.
        """

        if self.operation_mode == "relay":
            # Relay mode: Different behavior for VTM vs HTM
            if robot.robot_type == RobotType.VTM:
                yield from self._vtm_behavior_process(robot)
            elif robot.robot_type == RobotType.HTM:
                yield from self._htm_behavior_process(robot)
        else:
            # R2P mode: Original behavior
            yield from self._r2p_robot_behavior(robot)

    def _vtm_behavior_process(self, robot: Robot):
        """VTM-specific behavior: Operate within assigned zone"""
        while self.simulation_active:
            # Find relay tasks in my zone that need VTM
            zone = robot.zone_assignment
            available_tasks = [
                task for task in self.relay_tasks
                if task.stage == TaskStage.PENDING
                and self.zone_manager.get_zone_for_rack(task.source_rack_id) == zone
            ]

            if not available_tasks:
                robot.state = RobotState.IDLE
                yield self.env.timeout(1.0)
                continue

            # Select closest task
            task = self._select_closest_relay_task(robot, available_tasks)

            # Execute VTM portion of relay task
            # (Retrieval + transport to relay buffer)
            yield from self._execute_vtm_leg(robot, task)

    def _htm_behavior_process(self, robot: Robot):
        """HTM-specific behavior: Transport between relay buffers and stations"""
        while self.simulation_active:
            # Check relay buffers for totes waiting for pickup
            relay_buffer = self.relay_buffer_manager.get_buffer_with_totes()

            if not relay_buffer:
                robot.state = RobotState.IDLE
                yield self.env.timeout(1.0)
                continue

            # Pick up tote from buffer
            tote = relay_buffer.remove_tote(self.env.now)

            # Find associated relay task
            relay_task = self._get_relay_task_for_tote(tote.id)

            # Execute HTM portion
            yield from self._execute_htm_leg(robot, tote, relay_task)
```

---

## 9. API & Schema Changes

### 9.1 Configuration Schema Updates

**File:** `apps/backend/src/queue_simulator/simulator/models.py` (ENHANCED)

```python
class OperationMode(str, Enum):
    """Warehouse operation modes"""
    R2P = "r2p"              # Racks-to-Picker (legacy)
    RELAY = "relay"          # Relay system with VTM/HTM
    HYBRID = "hybrid"        # Both modes (for comparison)


class RelayConfig(BaseModel):
    """Configuration for relay system operation"""

    vtm_count: int = Field(default=10, ge=1, le=100, description="Number of VTMs")
    htm_count: int = Field(default=5, ge=1, le=50, description="Number of HTMs")

    vtm_speed: float = Field(default=2.0, ge=0.5, le=5.0, description="VTM speed (m/s)")
    htm_speed: float = Field(default=3.0, ge=0.5, le=5.0, description="HTM speed (m/s)")

    tote_capacity: int = Field(default=50, ge=10, le=100, description="Items per tote")
    relay_buffer_capacity: int = Field(default=10, ge=5, le=50, description="Totes per buffer")

    tote_loading_time: float = Field(default=3.0, ge=1.0, le=10.0, description="Tote pick/place time (s)")

    # Zone configuration
    vtms_per_zone: int = Field(default=2, ge=1, le=10, description="VTMs assigned per zone")


class RobotConfig(BaseModel):
    """Enhanced robot configuration"""

    num_robots: int = Field(default=5, ge=1, le=100)
    robot_speed: float = Field(default=1.5, ge=0.1, le=5.0)

    # NEW: Relay-specific configuration
    relay_config: Optional[RelayConfig] = None


class SimulationConfig(BaseModel):
    """Enhanced simulation configuration"""

    # ... existing fields ...

    # NEW: Operation mode
    operation_mode: OperationMode = Field(
        default=OperationMode.R2P,
        description="Warehouse operation mode"
    )

    # Configuration for each mode
    robot_config: RobotConfig
    relay_config: Optional[RelayConfig] = None
```

### 9.2 New API Endpoints

**File:** `apps/backend/src/queue_simulator/api/routers/relay.py` (NEW)

```python
from fastapi import APIRouter, HTTPException
from ..schemas.relay import RelayMetricsResponse, RelayBufferStatus

router = APIRouter(prefix="/relay", tags=["relay"])


@router.get("/buffers", response_model=list[RelayBufferStatus])
async def get_relay_buffer_status(simulation_id: str):
    """Get current status of all relay buffers"""
    # Return buffer utilization, queue lengths, etc.
    pass


@router.get("/metrics/handoffs", response_model=RelayMetricsResponse)
async def get_handoff_metrics(simulation_id: str):
    """Get metrics about VTM-HTM handoffs"""
    # Return handoff times, wait times, throughput
    pass


@router.get("/zones", response_model=list[ZoneStatus])
async def get_zone_status(simulation_id: str):
    """Get status of all warehouse zones"""
    # Return VTM assignments, task queues per zone
    pass
```

---

## 10. Frontend Visualization

### 10.1 New Visual Elements

**Additions to WarehouseMap component:**

1. **Relay Buffers**: Distinct icons at buffer locations
2. **Totes**: Small containers visible when being transported
3. **Robot Type Indicators**: Different colors for VTM vs HTM
4. **Zone Boundaries**: Visual indication of aisle zones
5. **Handoff Animations**: Smooth transitions at relay buffers

**File:** `apps/frontend/src/components/common/maps/WarehouseMap.tsx` (ENHANCED)

```typescript
interface RelayBuffer {
  id: string;
  position: Position;
  capacity: number;
  currentTotes: number;
  servingZones: string[];
}

interface Tote {
  id: string;
  position: Position;
  state: "in_rack" | "with_vtm" | "in_buffer" | "with_htm" | "at_station";
  carriedBy?: number; // robot ID
}

interface EnhancedRobot extends Robot {
  robotType: "r2p" | "vtm" | "htm";
  zoneAssignment?: string;
  carryingTotes?: string[]; // tote IDs
}

// Rendering logic
const renderRelayBuffers = () => {
  return relayBuffers.map((buffer) => (
    <Group key={buffer.id}>
      {/* Buffer representation */}
      <Circle
        x={transformX(buffer.position.x)}
        y={transformY(buffer.position.y)}
        radius={effectiveGridSize * 0.8}
        fill={COLORS.relayBuffer}
        stroke={COLORS.border}
        strokeWidth={2}
      />

      {/* Utilization indicator */}
      <Text
        x={transformX(buffer.position.x)}
        y={transformY(buffer.position.y) + effectiveGridSize}
        text={`${buffer.currentTotes}/${buffer.capacity}`}
        fontSize={10}
        fill={COLORS.text}
      />
    </Group>
  ));
};

const renderTotes = () => {
  return totes.map((tote) => (
    <Rect
      key={tote.id}
      x={transformX(tote.position.x) - effectiveGridSize * 0.3}
      y={transformY(tote.position.y) - effectiveGridSize * 0.3}
      width={effectiveGridSize * 0.6}
      height={effectiveGridSize * 0.6}
      fill={getToteColor(tote.state)}
      cornerRadius={2}
    />
  ));
};
```

### 10.2 Relay-Specific Metrics Dashboard

**New tabs/sections:**

- VTM Performance (per zone)
- HTM Performance
- Buffer Utilization
- Handoff Metrics (wait times, throughput)
- Zone Load Balance

---

## 11. Testing Strategy

### 11.1 Unit Tests

**New Test Files:**

1. `test_relay_orchestrator.py` - Multi-stage workflow
2. `test_relay_buffers.py` - Buffer operations, capacity
3. `test_totes.py` - Tote lifecycle, capacity
4. `test_zones.py` - Zone management, VTM assignment
5. `test_robot_types.py` - Type-specific behaviors

### 11.2 Integration Tests

**Scenarios:**

1. Single relay task end-to-end
2. Multiple concurrent relay tasks
3. Buffer overflow handling
4. VTM-HTM coordination
5. Mixed R2P and relay mode (hybrid)

### 11.3 Performance Benchmarks

**Metrics to track:**

1. Orders per hour (relay vs R2P)
2. Average cycle time per order
3. Robot utilization (VTM vs HTM)
4. Buffer wait times
5. Memory footprint (additional data structures)

---

## 12. Migration & Rollout Plan

### Phase 1: Foundation (Weeks 1-3)

- [ ] Implement `RobotType` enum and enhance `Robot` class
- [ ] Create `Tote` and `TotePool` classes
- [ ] Create `RelayTask` and `TaskStage` enums
- [ ] Unit tests for new domain models

### Phase 2: Spatial Infrastructure (Weeks 4-5)

- [ ] Implement `RelayBuffer` and `RelayBufferManager`
- [ ] Implement `AisleZone` and `ZoneManager`
- [ ] Enhance `WarehouseMap` to generate relay locations
- [ ] Integration tests for spatial components

### Phase 3: Orchestration (Weeks 6-8)

- [ ] Implement `RelayTaskOrchestrator`
- [ ] Integrate with `SimulationEngine` (dual-mode support)
- [ ] Implement VTM and HTM behavior processes
- [ ] End-to-end relay task tests

### Phase 4: Algorithms (Weeks 9-10)

- [ ] Enhance `OrderProcessor` for relay task generation
- [ ] Implement zone-based order decomposition
- [ ] Implement tote routing optimization
- [ ] Algorithm validation tests

### Phase 5: API & Config (Week 11)

- [ ] Add `RelayConfig` to configuration schemas
- [ ] Create new relay-specific API endpoints
- [ ] Update existing endpoints for dual-mode
- [ ] API integration tests

### Phase 6: Frontend (Week 12)

- [ ] Enhance `WarehouseMap` component
- [ ] Add relay buffer and tote visualization
- [ ] Create relay metrics dashboard
- [ ] Frontend integration tests

### Phase 7: Validation & Optimization (Weeks 13-14)

- [ ] Performance benchmarking
- [ ] Comparison studies (R2P vs Relay)
- [ ] Documentation
- [ ] User acceptance testing

---

## 13. Performance Considerations

### 13.1 Computational Complexity

**Additional State:**

- `N_totes` active totes (proportional to orders)
- `N_buffers` relay buffers (~10-50)
- `N_relay_tasks` active relay tasks (2-5x regular tasks due to multi-stage)

**Memory Impact:**

- Tote objects: ~500 bytes each
- RelayTask objects: ~1KB each (more metadata)
- Buffer queues: Minimal (pointers)

**Estimated:** +20-30% memory for relay mode vs R2P mode

### 13.2 Event Processing

**R2P Mode:**

- 1 robot process per robot
- ~5-10 events per rack cycle

**Relay Mode:**

- 2 robot processes per relay task (VTM + HTM)
- ~15-20 events per order (multi-stage)
- Buffer synchronization events

**Estimated:** 2-3x more SimPy events in relay mode

### 13.3 Optimization Strategies

1. **Lazy Tote Creation**: Create totes on-demand, reuse pool
2. **Buffer Indexing**: Spatial index for nearest buffer lookups
3. **Zone Caching**: Cache zone-rack mappings
4. **Event Batching**: Batch multiple tote handoffs at buffer
5. **Parallel Simulation**: Consider parallel VTM/HTM processes

---

## 14. Open Questions & Decisions

### Decision Matrix

| Question                 | Options                          | Recommendation      | Rationale                                             |
| ------------------------ | -------------------------------- | ------------------- | ----------------------------------------------------- |
| Do racks still exist?    | Yes / No                         | **Yes**             | Racks are stationary storage, VTMs retrieve from them |
| Tote capacity strategy   | Fixed / Dynamic                  | **Fixed initially** | Simpler to implement, can enhance later               |
| Buffer placement         | Per-aisle / Per-zone / Optimized | **Per-zone**        | Balance between coverage and simplicity               |
| VTM assignment           | Static / Dynamic                 | **Static per zone** | Simpler coordination, predictable performance         |
| HTM pooling              | Global / Per-area                | **Global pool**     | Better utilization, HTMs are not zone-constrained     |
| Handoff mechanism        | Queue / Resource                 | **SimPy queue**     | Natural fit for FIFO buffer behavior                  |
| Return journey           | Same VTM / Any VTM               | **Same VTM**        | Simplifies tracking, VTM knows its zone               |
| Multiple totes per robot | Yes / No                         | **Start with 1**    | Simplify initial implementation                       |

### Outstanding Questions

1. **Q:** Should we support partial tote picks (one tote serves multiple orders)?

   - **A:** Phase 2 feature. Start with one order per tote.

2. **Q:** How to handle buffer overflow (all buffers full)?

   - **A:** VTMs wait at aisle with tote until buffer has space.

3. **Q:** Should HTMs have priority routing (express lanes)?

   - **A:** Nice-to-have. Use uniform speed initially.

4. **Q:** Metrics: What KPIs are most important for relay system?

   - **A:**
     - Handoff wait time
     - Buffer utilization
     - VTM vs HTM utilization ratio
     - Orders per hour
     - Avg tote cycle time

5. **Q:** Can we run R2P and Relay in same simulation (hybrid mode)?
   - **A:** Interesting for comparison. Phase 2 feature.

---

## 15. Success Criteria

### Functional Requirements

- ✅ Relay tasks execute end-to-end without errors
- ✅ VTMs operate only within assigned zones
- ✅ HTMs can pick from any relay buffer
- ✅ Buffer capacity respected (no overflow)
- ✅ Totes return to original racks after completion
- ✅ All existing R2P functionality still works

### Performance Requirements

- ✅ Relay mode achieves >80% of R2P throughput initially
- ✅ Memory overhead <50% vs R2P mode
- ✅ Simulation speed within 2x of R2P mode
- ✅ Buffer utilization >60% (not underutilized)
- ✅ VTM idle time <20%
- ✅ HTM idle time <30%

### Quality Requirements

- ✅ >90% test coverage for new code
- ✅ Zero regressions in existing R2P tests
- ✅ API response times <100ms
- ✅ Frontend renders relay system at 30fps
- ✅ Comprehensive documentation

---

## 16. References & Research

### Real-World Systems

- **AutoStore**: Grid-based system with vertical lifts
- **Ocado**: Swarm robotics with relay points
- **Amazon Robotics**: Hybrid mobile robots with sort/buffer stations

### Academic Papers

- "Multi-agent coordination in robotic warehousing" (cite specific papers)
- "Buffer sizing in automated warehouses"
- "Zone-based task allocation"

### Internal Documents

- `specs/map_abstraction/design.md` - Current map system
- `apps/backend/README.md` - Backend architecture
- `apps/backend/API_README.md` - API documentation

---

## Conclusion

This relay system enhancement represents a fundamental architectural shift that will:

1. Enable modeling of modern automated warehousing systems
2. Provide comparative analysis between R2P and relay fulfillment
3. Introduce realistic multi-robot coordination patterns
4. Support future extensions (multi-level storage, autonomous lifts, etc.)

The phased approach ensures backward compatibility while building a robust foundation for advanced warehouse simulation capabilities.

**Next Steps:**

1. Review and approve this design document
2. Create detailed task breakdown in project management tool
3. Set up feature branch: `feature/relay-system`
4. Begin Phase 1 implementation
5. Weekly progress reviews

---

**Document Version:** 1.0  
**Last Updated:** October 29, 2025  
**Author:** AI Assistant (Staff Engineer Mode)  
**Reviewers:** TBD
