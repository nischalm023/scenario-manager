# Relay System Enhancement Specifications

This directory contains all design documents, specifications, and planning materials for the **Relay System Enhancement** - a major architectural feature that introduces VTM (Vertical Tote Mover) and HTM (Horizontal Tote Mover) based fulfillment to the warehouse simulator.

## Overview

The relay system represents a fundamental shift from the current "Racks-to-Picker" (R2P) model to a modern automated warehouse fulfillment pattern where:

- **VTMs** operate within aisles, retrieving totes from stationary racks
- **HTMs** transport totes between relay buffers and stations
- **Relay buffers** serve as handoff points between VTM and HTM fleets
- **Zones** organize the warehouse into manageable areas with dedicated VTM assignments

## Documents

### Core Design

- **[design.md](./design.md)** - Comprehensive system design and implementation plan
  - Architecture overview
  - Domain model changes
  - Critical abstractions
  - Task orchestration
  - Algorithm adaptations
  - Testing strategy
  - Migration plan

### Coming Soon

- **implementation_tasks.md** - Detailed task breakdown with time estimates
- **architecture_diagrams.md** - Visual architecture and flow diagrams
- **api_spec.md** - Detailed API specifications for relay endpoints
- **performance_analysis.md** - Performance benchmarks and optimization strategies
- **comparison_study.md** - R2P vs Relay performance comparisons

## Key Concepts

### Robot Types

- **R2P (Racks-to-Picker)**: Legacy system where robots carry entire racks to stations
- **VTM (Vertical Tote Mover)**: Zone-bound robots that retrieve totes from racks in aisles
- **HTM (Horizontal Tote Mover)**: Global robots that transport totes between relay buffers and stations

### New Abstractions

1. **Tote**: Container that holds items and travels through the relay system
2. **RelayBuffer**: Handoff point where VTMs and HTMs exchange totes
3. **AisleZone**: Spatial zone with dedicated VTM fleet
4. **RelayTask**: Multi-stage task representing complete tote journey

### Workflow Stages

```
1. Order Allocation → Multiple Totes (by zone)
2. VTM Retrieval → Pick tote from rack
3. VTM Transport → Deliver to relay buffer
4. Handoff → VTM releases, HTM claims
5. HTM Transport → Deliver to station
6. Station Processing → Operator picks items
7. Return Journey → HTM → Buffer → VTM → Rack
```

## Implementation Timeline

**Estimated Duration:** 12-14 weeks

### Phase 1: Foundation (Weeks 1-3)

Core domain models and data structures

### Phase 2: Spatial Infrastructure (Weeks 4-5)

Relay buffers, zones, and map enhancements

### Phase 3: Orchestration (Weeks 6-8)

Multi-stage task coordination and robot behaviors

### Phase 4: Algorithms (Weeks 9-10)

Order processing and routing optimization

### Phase 5: API & Config (Week 11)

Configuration schemas and API endpoints

### Phase 6: Frontend (Week 12)

Visualization and metrics dashboard

### Phase 7: Validation (Weeks 13-14)

Testing, benchmarking, and documentation

## Quick Links

### Related Specifications

- [Map Abstraction Design](../map_abstraction/design.md)
- [Configuration Refactor Plan](../config_refactor/plan.md)

### Implementation Files

- `apps/backend/src/queue_simulator/domain/robots.py` - Robot types and states
- `apps/backend/src/queue_simulator/domain/containers.py` - Tote abstraction (NEW)
- `apps/backend/src/queue_simulator/domain/relay_tasks.py` - Relay task types (NEW)
- `apps/backend/src/queue_simulator/domain/relay_buffers.py` - Buffer management (NEW)
- `apps/backend/src/queue_simulator/domain/zones.py` - Zone management (NEW)
- `apps/backend/src/queue_simulator/domain/relay_orchestrator.py` - Task orchestration (NEW)

### Testing Files

- `apps/backend/tests/test_relay_orchestrator.py` (NEW)
- `apps/backend/tests/test_relay_buffers.py` (NEW)
- `apps/backend/tests/test_totes.py` (NEW)
- `apps/backend/tests/test_zones.py` (NEW)
- `apps/backend/tests/test_robot_types.py` (NEW)

## Key Decisions

| Decision          | Choice           | Rationale                       |
| ----------------- | ---------------- | ------------------------------- |
| Racks exist?      | Yes, stationary  | VTMs retrieve from fixed racks  |
| Tote capacity     | Fixed (50 items) | Simplicity first, enhance later |
| Buffer placement  | Per-zone         | Balance coverage vs complexity  |
| VTM assignment    | Static per zone  | Predictable performance         |
| HTM pooling       | Global           | Better utilization              |
| Handoff mechanism | FIFO queue       | Natural buffer behavior         |

## Success Criteria

### Functional

- ✅ Complete relay task execution
- ✅ Zone-constrained VTM operation
- ✅ Multi-stage coordination
- ✅ Backward compatibility with R2P

### Performance

- ✅ >80% R2P throughput initially
- ✅ <50% memory overhead
- ✅ <2x simulation time
- ✅ >60% buffer utilization

### Quality

- ✅ >90% test coverage
- ✅ Zero R2P regressions
- ✅ Comprehensive documentation

## Getting Started

### For Developers

1. Read [design.md](./design.md) thoroughly
2. Understand the current R2P system
3. Review the map abstraction system
4. Check out feature branch: `feature/relay-system`
5. Start with Phase 1 tasks

### For Reviewers

1. Review architectural decisions in [design.md](./design.md)
2. Validate performance assumptions
3. Verify compatibility requirements
4. Provide feedback on API designs

### For Product

1. Understand business value and use cases
2. Review success criteria
3. Plan user acceptance testing
4. Coordinate with stakeholders

## Questions or Feedback?

For questions, clarifications, or feedback on the relay system design:

- Open a GitHub issue with tag `relay-system`
- Schedule a design review meeting
- Contact the engineering team

---

**Last Updated:** October 29, 2025  
**Status:** Design Phase  
**Owner:** Engineering Team
