# Relay System Implementation Notes

## Status: Phase 3 Mostly Complete ✅

### Completed Components (11/13 tasks) ✅

#### Phase 1: Core Domain Models & Relay Positions ✅

- ✅ `Tote` and `TotePool` implementation with tests
- ✅ `RelayPosition` and `RelayPositionManager` with SimPy resource management
- ✅ Conditional relay position generation in `WarehouseMap`
- ✅ `RelayTask` and `TaskStage` enum

#### Phase 2: Task Planner & Configuration ✅

- ✅ `TaskPlanner` module for intelligent relay position assignment
- ✅ Discriminated union for robot configs (`RtpRobotConfig` vs `RelayRobotConfig`)
- ✅ `model_validator` to ensure config consistency

#### Phase 3: Relay Orchestration & Robot Behaviors ✅

- ✅ `RelayTaskOrchestrator` with VTM/HTM leg methods
- ✅ Robot enhancements (relay states, zone_assignment, position property)
- ✅ `Zone` and `ZoneManager` for VTM aisle assignment
- ✅ `WarehouseBuilder` dual-mode support (RTP vs Relay)
- ✅ API schema updates (SystemType, relay robot fields, relay response schemas)
- ✅ `OrderProcessor` extended with `_generate_relay_tasks_for_order()`

---

## Remaining Tasks (2/13)

### 1. SimulationEngine Dual-Mode Support ⏳

**File**: `apps/backend/src/queue_simulator/simulator/simulation_engine.py`

**What Needs to Be Done**:

The SimulationEngine needs to support two distinct operational modes:

#### A. RTP Mode (Existing)

- Robot behavior: Move rack to station, return rack to storage
- Single robot process per robot
- Uses `PickingTask` objects

#### B. Relay Mode (New)

- **VTM Behavior**:
  - Retrieve tote from rack
  - Transport to assigned relay position
  - May block if relay position is full
  - Stay within assigned zone (aisle)
- **HTM Behavior**:
  - Pick tote from relay position
  - Transport to station
  - Wait for operator picking
  - Can move freely across zones

#### Implementation Strategy:

```python
class SimulationEngine:
    def __init__(self, config, warehouse_components):
        self.config = config
        self.system_type = config.system_type

        # Load components based on system type
        if self.system_type == SystemType.RELAY:
            self.tote_pool = warehouse_components["tote_pool"]
            self.relay_position_manager = warehouse_components["relay_position_manager"]
            self.task_planner = warehouse_components["task_planner"]
            self.zone_manager = warehouse_components["zone_manager"]
            self.relay_orchestrator = RelayTaskOrchestrator(...)

    def run(self):
        """Main simulation loop with dual-mode support."""
        # Generate orders (same for both modes)
        orders = self.order_generator.generate_all_batch_orders()
        self.order_processor.add_orders(orders)

        if self.system_type == SystemType.RTP:
            self._run_rtp_mode()
        else:
            self._run_relay_mode()

    def _run_rtp_mode(self):
        """Existing RTP simulation logic."""
        # Spawn robot processes for RTP
        for robot in self.robots:
            self.env.process(self._robot_process_rtp(robot))

        # Run simulation
        self.env.run(until=self.config.simulation_duration)

    def _run_relay_mode(self):
        """New relay simulation logic."""
        # Separate VTMs and HTMs
        vtms = [r for r in self.robots if r.robot_type == RobotType.VTM]
        htms = [r for r in self.robots if r.robot_type == RobotType.HTM]

        # Spawn VTM processes
        for vtm in vtms:
            self.env.process(self._vtm_process(vtm))

        # Spawn HTM processes
        for htm in htms:
            self.env.process(self._htm_process(htm))

        # Spawn order processing
        self.env.process(self._relay_order_processor())

        # Run simulation
        self.env.run(until=self.config.simulation_duration)

    def _vtm_process(self, vtm: Robot):
        """
        VTM robot behavior generator.

        1. Wait for relay task assignment
        2. Execute VTM leg (rack -> relay)
        3. Go idle and wait for next task
        """
        while True:
            # Wait for task assignment
            relay_task = yield self.env.process(self._wait_for_vtm_task(vtm))

            if relay_task is None:
                yield self.env.timeout(1.0)  # Check again in 1 second
                continue

            # Get components
            tote = self.tote_pool.get_tote_by_id(relay_task.tote_id)
            rack = self._get_rack_by_id(relay_task.source_rack_id)

            # Execute VTM leg
            yield from self.relay_orchestrator.execute_vtm_leg(
                vtm=vtm,
                tote=tote,
                relay_task=relay_task,
                rack=rack,
                env=self.env
            )

            # Update metrics
            vtm.tasks_completed += 1

    def _htm_process(self, htm: Robot):
        """
        HTM robot behavior generator.

        1. Wait for tote at relay position
        2. Execute HTM leg (relay -> station)
        3. Go idle and wait for next tote
        """
        while True:
            # Find relay position with waiting totes
            relay_position = self._find_relay_position_with_totes()

            if relay_position is None or not relay_position.has_totes():
                yield self.env.timeout(1.0)  # Check again in 1 second
                continue

            # Get the relay task for this tote
            tote_id = relay_position.pending_totes[0].id
            relay_task = self._get_relay_task_by_tote_id(tote_id)

            if relay_task is None:
                yield self.env.timeout(1.0)
                continue

            # Get components
            tote = relay_position.pending_totes[0]
            station = self.station_manager.get_station(relay_task.target_station_id)

            # Execute HTM leg
            yield from self.relay_orchestrator.execute_htm_leg(
                htm=htm,
                tote=tote,
                relay_task=relay_task,
                station=station,
                env=self.env
            )

            # Update metrics
            htm.tasks_completed += 1

    def _relay_order_processor(self):
        """
        Process orders and generate relay tasks.

        Runs periodically to convert pending orders into relay tasks.
        """
        while True:
            yield self.env.timeout(10.0)  # Process every 10 seconds

            if not self.order_processor.pending_orders:
                continue

            # Generate relay tasks for pending orders
            for order in self.order_processor.pending_orders[:]:
                relay_tasks = self.order_processor._generate_relay_tasks_for_order(
                    order=order,
                    current_time=self.env.now
                )

                # Add to relay task queue
                self.relay_tasks.extend(relay_tasks)

                # Remove from pending
                self.order_processor.pending_orders.remove(order)

    def _wait_for_vtm_task(self, vtm: Robot):
        """
        Find and assign a relay task to a VTM.

        Priority: Tasks in VTM's assigned zone
        """
        # Find pending relay tasks in VTM's zone
        zone = self.zone_manager.get_zone_for_vtm(vtm.id)

        for task in self.relay_tasks:
            if task.stage != TaskStage.PENDING:
                continue

            # Check if task's rack is in VTM's zone
            rack = self._get_rack_by_id(task.source_rack_id)
            if zone and zone.contains_position(rack.storage_coordinate):
                task.assigned_vtm_id = vtm.id
                return task

        return None
```

#### Key Integration Points:

1. **Order Processing**: Use `_generate_relay_tasks_for_order()` instead of `_generate_tasks_for_order()`
2. **Task Tracking**: Maintain a list of `RelayTask` objects instead of `PickingTask`
3. **Robot Assignment**: Use `ZoneManager` for VTM zone-based assignment
4. **Metrics**: Track relay-specific metrics (handoff times, relay utilization)

---

### 2. Integration Test for Relay System ⏳

**File**: `apps/backend/tests/integration/test_relay_system_integration.py` (new)

**What Needs to Be Tested**:

#### Test Structure:

```python
import pytest
import simpy

from queue_simulator.simulator.models import (
    SimulationConfig,
    SystemType,
    RelayRobotConfig,
    OperatorConfig,
    AlgorithmConfig,
)
from queue_simulator.domain.map import WarehouseMapConfig
from queue_simulator.simulator.warehouse_builder import WarehouseBuilder
from queue_simulator.domain.robot.robots import RobotType
from queue_simulator.domain.robot.tasks import RelayTaskStage


class TestRelaySystemIntegration:
    """
    End-to-end integration tests for relay system.

    Tests the complete flow from order to completion using VTMs and HTMs.
    """

    @pytest.fixture
    def relay_config(self):
        """Create a relay system configuration."""
        return SimulationConfig(
            simulation_duration=3600.0,  # 1 hour
            scenario_id="scenario_a",
            system_type=SystemType.RELAY,
            map_config=WarehouseMapConfig(
                grid_width=20,
                grid_depth=20,
                # ... map config
            ),
            robot_config=RelayRobotConfig(
                num_vtms=4,
                num_htms=2,
                vtm_horizontal_speed=1.5,
                htm_speed=1.5,
            ),
            operator_config=OperatorConfig(
                operator_time_per_pick=12.0,
                target_picks_per_face=3.0,
                num_bins=8,
            ),
            algorithm_config=AlgorithmConfig(),
        )

    @pytest.fixture
    def relay_warehouse(self, relay_config):
        """Build a relay warehouse."""
        env = simpy.Environment()
        builder = WarehouseBuilder(relay_config, scenario_manager, env=env)
        components = builder.build()
        return env, components

    def test_relay_components_created(self, relay_warehouse):
        """Test that all relay components are created."""
        env, components = relay_warehouse

        assert components["tote_pool"] is not None
        assert components["relay_position_manager"] is not None
        assert components["task_planner"] is not None
        assert components["zone_manager"] is not None

        # Check robot types
        vtms = [r for r in components["robots"] if r.robot_type == RobotType.VTM]
        htms = [r for r in components["robots"] if r.robot_type == RobotType.HTM]

        assert len(vtms) == 4
        assert len(htms) == 2

        # Check zone assignments
        for vtm in vtms:
            assert vtm.zone_assignment is not None

    def test_relay_positions_generated(self, relay_warehouse):
        """Test that relay positions are generated correctly."""
        env, components = relay_warehouse

        relay_mgr = components["relay_position_manager"]
        stats = relay_mgr.get_stats()

        assert stats["total_positions"] > 0

        # Check relay positions have SimPy resources
        for position in relay_mgr.get_all_positions():
            assert position.resource is not None
            assert position.capacity > 0

    def test_zone_manager_assigns_vtms(self, relay_warehouse):
        """Test that VTMs are assigned to zones."""
        env, components = relay_warehouse

        zone_mgr = components["zone_manager"]
        stats = zone_mgr.get_stats()

        assert stats["total_zones"] > 0
        assert stats["total_vtms_assigned"] == 4

        # Check each VTM has a zone
        vtms = [r for r in components["robots"] if r.robot_type == RobotType.VTM]
        for vtm in vtms:
            zone = zone_mgr.get_zone_for_vtm(vtm.id)
            assert zone is not None

    def test_order_to_relay_task_conversion(self, relay_warehouse, sample_order):
        """Test converting orders to relay tasks."""
        env, components = relay_warehouse

        order_processor = create_order_processor_with_relay(components)
        order_processor.add_order(sample_order)

        # Generate relay tasks
        relay_tasks = order_processor._generate_relay_tasks_for_order(
            order=sample_order,
            current_time=0.0
        )

        assert len(relay_tasks) > 0

        for task in relay_tasks:
            assert task.order_id == sample_order.order_id
            assert task.tote_id is not None
            assert task.relay_position_id is not None
            assert task.stage == RelayTaskStage.PENDING

    def test_vtm_picks_tote_and_delivers_to_relay(self, relay_warehouse):
        """Test VTM leg of relay task."""
        env, components = relay_warehouse

        # Create a relay task
        relay_task = create_sample_relay_task(components)
        tote = components["tote_pool"].get_tote()
        vtm = get_vtm_from_components(components)
        rack = get_sample_rack(components)

        # Execute VTM leg
        orchestrator = create_relay_orchestrator(components)

        # Run the VTM process
        env.process(orchestrator.execute_vtm_leg(
            vtm=vtm,
            tote=tote,
            relay_task=relay_task,
            rack=rack,
            env=env
        ))

        # Run simulation for 100 seconds
        env.run(until=100.0)

        # Verify task progressed
        assert relay_task.stage == RelayTaskStage.AT_RELAY
        assert relay_task.assigned_vtm_id == vtm.id

        # Verify tote is at relay position
        relay_pos = components["relay_position_manager"].get_position_by_id(
            relay_task.relay_position_id
        )
        assert relay_pos.has_totes()

    def test_htm_picks_from_relay_and_delivers_to_station(self, relay_warehouse):
        """Test HTM leg of relay task."""
        env, components = relay_warehouse

        # Setup: VTM has already delivered tote to relay
        relay_task, tote = setup_tote_at_relay(env, components)

        htm = get_htm_from_components(components)
        station = components["station_manager"].get_station(
            relay_task.target_station_id
        )

        # Execute HTM leg
        orchestrator = create_relay_orchestrator(components)

        env.process(orchestrator.execute_htm_leg(
            htm=htm,
            tote=tote,
            relay_task=relay_task,
            station=station,
            env=env
        ))

        # Run simulation
        env.run(until=200.0)

        # Verify task completed
        assert relay_task.stage == RelayTaskStage.COMPLETED
        assert relay_task.assigned_htm_id == htm.id
        assert relay_task.completion_time > 0

        # Verify tote returned to pool
        pool_stats = components["tote_pool"].get_pool_stats()
        assert pool_stats["available"] > 0

    def test_full_relay_workflow(self, relay_warehouse, sample_order):
        """Test complete relay workflow from order to completion."""
        env, components = relay_warehouse

        # This would test the full SimulationEngine integration
        # with order -> relay tasks -> VTM leg -> HTM leg -> completion

        # TODO: Implement once SimulationEngine relay support is added
        pass
```

#### Test Coverage Goals:

- ✅ Component initialization
- ✅ Relay position generation
- ✅ Zone management and VTM assignment
- ✅ Order to relay task conversion
- ✅ VTM leg execution
- ✅ HTM leg execution
- ⏳ Full end-to-end workflow (requires SimulationEngine)

---

## Architecture Summary

### Data Flow in Relay System

```
Order → OrderProcessor._generate_relay_tasks_for_order()
  ↓
RelayTask (PENDING) + Tote (from TotePool)
  ↓
VTM Assignment (via ZoneManager)
  ↓
RelayTaskOrchestrator.execute_vtm_leg()
  ↓
  - VTM moves to rack
  - Picks tote
  - Transports to relay position (may block if full)
  ↓
RelayTask (AT_RELAY) + Tote at RelayPosition
  ↓
HTM Detection (scans relay positions for totes)
  ↓
RelayTaskOrchestrator.execute_htm_leg()
  ↓
  - HTM moves to relay position
  - Picks tote (frees SimPy resource)
  - Transports to station
  - Operator performs picks
  ↓
RelayTask (COMPLETED) + Tote returned to pool
```

### Key Design Patterns

1. **Resource Pool Pattern**: `TotePool` manages reusable tote objects
2. **SimPy Resource Management**: `RelayPosition` uses SimPy resources for blocking behavior
3. **Zone-Based Assignment**: `ZoneManager` ensures VTMs operate in their assigned aisles
4. **Discriminated Union**: Pydantic's discriminator for type-safe robot configs
5. **Task Orchestration**: Multi-stage relay tasks coordinated by `RelayTaskOrchestrator`

---

## Performance Considerations

### Optimizations Implemented

- ✅ TotePool prevents frequent object creation/destruction
- ✅ Rack position caching in OrderProcessor
- ✅ Zone-based VTM assignment reduces search space
- ✅ SimPy resources for efficient blocking behavior

### Future Optimizations

- [ ] Intelligent tote packing (multiple items per tote)
- [ ] HTM batching (pick multiple totes in one trip)
- [ ] Dynamic relay position capacity adjustment
- [ ] VTM task pre-assignment and batching

---

## Migration Guide

To switch a simulation from RTP to Relay:

1. **Update Configuration**:

```python
config = SimulationConfig(
    system_type=SystemType.RELAY,  # Changed from RTP
    robot_config=RelayRobotConfig(  # Changed from RtpRobotConfig
        num_vtms=4,
        num_htms=2,
        vtm_horizontal_speed=1.5,
        htm_speed=1.5,
    ),
    # ... rest of config
)
```

2. **Create SimPy Environment**:

```python
env = simpy.Environment()
builder = WarehouseBuilder(config, scenario_manager, env=env)
```

3. **Use Relay Components**:

```python
components = builder.build()
# components now includes:
# - tote_pool
# - relay_position_manager
# - task_planner
# - zone_manager
```

4. **Update SimulationEngine** (when implemented):

```python
engine = SimulationEngine(config, components)
engine.run()  # Automatically uses relay mode
```

---

## Next Steps

### Immediate (Required for MVP)

1. **Implement `SimulationEngine._run_relay_mode()`** - Core relay simulation loop
2. **Add VTM and HTM robot processes** - Generator functions for robot behavior
3. **Implement relay task queue management** - Track and assign relay tasks
4. **Complete integration tests** - Full end-to-end relay system test

### Short-term Enhancements

1. **Metrics Collection** - Relay-specific metrics (handoff time, relay utilization)
2. **Visualization** - Frontend updates to show VTMs, HTMs, and relay positions
3. **API Endpoints** - Add relay system status endpoints
4. **Performance Testing** - Benchmark relay vs RTP throughput

### Long-term Features

1. **Return Journey** - Implement tote return from station to rack
2. **Multi-Tote Optimization** - Pack multiple orders into totes
3. **Dynamic Zone Rebalancing** - Reassign VTMs based on workload
4. **Hybrid Mode** - Support both RTP and relay in same warehouse

---

## Contact & Support

For questions or issues with relay system implementation:

- Check design document: `specs/relay_system/design.md`
- Review test files: `apps/backend/tests/test_relay_positions.py`, `test_containers.py`
- See this implementation note: `specs/relay_system/IMPLEMENTATION_NOTES.md`
