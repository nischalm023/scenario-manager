---
name: java-backend
description: Use when writing or modifying Java code in fleet-simulator or put-service — covers
  Desmo-J simulation patterns, metrics integration, API/DTO conventions, domain modeling, and
  testing rules. For relay system (VTM/HTM/zones), use relay-system skill instead. For code
  organization and style, use java-style-checklist skill.
---

# Java Backend Patterns

## Convention Quick Reference

These rules cause the most violations. Full details in `references/project-conventions.md`.

1. **Controllers implement OpenAPI-generated interfaces** (e.g., `implements SimulationApi`). No `@RequestMapping` or `@GetMapping` — routes live in generated code.
2. **Controllers are package-private** — no `public` modifier. Spring component-scans the package.
3. **All endpoints return `ResponseEntity<ApiResponse<T>>`** using `ApiResponse.success(data)` or `ApiResponse.error(msg)`.
4. **DTOs are records** with compact constructor defaults. No `@Builder`, no separate mapper classes.
5. **ArchUnit enforces zero Spring dependencies** in `domain`/`simulator` packages — importing Spring in domain code compiles but fails tests.

## Desmo-J Process Structure

**All processes extend `SimProcess`:**
- Override `lifeCycle()` with `throws SuspendExecution`
- Use `hold(TimeSpan)` for delays
- Process names: `"ProcessType_EntityId"` (e.g., `"RtpRobot_0"`)

**Process Scheduling** (in `DesmojWarehouseModel.doInitialSchedules()`):
- Create process instances in `doInitialSchedules()`, NOT in `init()`
- Schedule: `process.activate(new TimeInstant(startTime, TimeUnit.SECONDS))`
- Stagger robot starts by 0.5s to avoid contention

**Continuous Processes**:
```java
while (true) {
    hold(new TimeSpan(interval, TimeUnit.SECONDS));
    // ... process logic
}
```

**Resource Blocking** (`Res` from Desmo-J):
- Create in `init()`: `new Res(model, name, capacity, showInReport, showInTrace)`
- Request: `resource.provide(process, amount)` - blocks if unavailable
- Release: Automatic when process completes

**Error Handling**:
- `SimFinishedException` is expected at simulation end - catch and ignore gracefully
- Wrap process lifecycle in try-catch for graceful termination

### Adding a New Process

1. Create class in `simulator/orchestrator/` extending `SimProcess`
2. Accept `MetricsManager` in constructor, store as field
3. Implement `lifeCycle()` with `while(true)` + `hold()` loop, catch `SimFinishedException`
4. Add a `schedule*()` method in `DesmojWarehouseModel`
5. Call from `doInitialSchedules()`: `process.activate(new TimeInstant(startTime, TimeUnit.SECONDS))`

See existing: `DesmojRtpRobotProcess`, `DesmojBinManagementProcess`, `DesmojMetricMonitoringProcess`.

## Experiment Setup

```java
// 1. Create model with all components
var model = new DesmojWarehouseModel(warehouse, config, /* ... */);

// 2. Create experiment and connect
var experiment = new Experiment("simulation-run");
model.connectToExperiment(experiment);

// 3. Configure stop time
experiment.stop(new TimeInstant(durationSeconds, TimeUnit.SECONDS));

// 4. Disable trace for production runs
experiment.tracePeriod(new TimeInstant(0), new TimeInstant(0));

// 5. Run
experiment.start();
experiment.finish();
```

## Metrics Integration

**Event-Driven Architecture**: Metrics collectors use `@MetricHandler` annotation to subscribe to domain events.

**Domain Events**:
- Robot: `RACK_CYCLE_COMPLETED`, `HTM_CYCLE_COMPLETED`, `VTM_CYCLE_COMPLETED`
- Tasks: `TASK_COMPLETED`, `ORDER_COMPLETED`, `ALL_ORDERS_COMPLETED`
- Simulation: `SIMULATION_STARTED`, `SIMULATION_COMPLETED`

**Collector Pattern**:
```java
@MetricHandler({MetricEvent.RACK_CYCLE_COMPLETED})
public void handleRackCycle(MetricEventData event) {
    // Extract from event.getData() Map
    Object picksObj = event.getData().get("total_picks");
    Object stationsObj = event.getData().get("stations_visited"); // List<Integer>

    // Process and accumulate metrics
}
```

**CRITICAL**:
- Events with `stations_visited` contain a `List<Integer>` - iterate to record per-station stats
- Always check `event.getData()` for null before accessing
- Extract data defensively with instanceof checks

### Emitting Events from Processes

Events are created via static factory methods on the sealed `DomainEvent` interface, then routed through `MetricsManager`:

```java
// In any SimProcess that has a MetricsManager reference:
DomainEvent event = DomainEvent.rackCycleCompleted(
    model.presentTime().getTimeAsDouble(TimeUnit.SECONDS),
    robotId, picksCompleted, stationsVisited, travelTime, waitTime);
metricsManager.emit(event);
```

Key files:
- `simulator/metrics/events/DomainEvent.java` — sealed interface with all factory methods
- `simulator/metrics/MetricsManager.emit()` — routes events to `@MetricHandler` collectors

When adding a new event type: add a record + factory method in `DomainEvent.java`, then add `@MetricHandler` method in the appropriate collector.

## Order Completion Tracking

- Share `orderTaskMapping` between `DesmojSimulationEngine` and `DesmojWarehouseModel`
- Use callback pattern: `BiConsumer<String, String> orderCompletionCallback`
- **CRITICAL**: Call `orderManager.completeBinnedOrder(orderId, currentTime)` in callback to free bins

## Derived Metrics

All derived metrics are calculated in the backend, not the frontend. Use actual `simulationEndTime - simulationStartTime`, never `config.simulationDuration`.

**Adding a new metric**:
1. Add field to DTO record in `*Metrics.java`
2. Calculate in collector's `getMetrics()` using actual simulation duration
3. Recompile and regenerate frontend types (`node scripts/generate-merged-api-types.js`)

**NEVER**: Calculate metrics using `config.simulationDuration` (configured time, not actual runtime), or duplicate calculation logic across frontend components.

## Project Conventions

This codebase enforces specific patterns that differ from typical Spring Boot projects. **Before writing any API, DTO, domain, or test code**, consult `references/project-conventions.md` for the binding rules — violating them will cause ArchUnit test failures or inconsistency with the existing code.

> **Note**: For VTM/HTM orchestration patterns (backpressure, zones, relay positions), see `.claude/skills/relay-system/`

## Key Classes

See `references/key-classes.md` for the call chain, build phases, and class interaction map.

## Related Skills

| Skill | When |
|-------|------|
| `.claude/skills/java-style-checklist/` | Before committing Java code — organization, comments, modern Java 21+ |
| `.claude/skills/relay-system/` | VTM/HTM robots, relay positions, zone capacity |
| `.claude/skills/grpc-patterns/` | Adding or modifying gRPC clients |
| `.claude/skills/architecture/` | Service boundaries and communication patterns |
| `.claude/skills/put-benchmark/` | After major put-service algorithm changes |
| `.claude/skills/commands/` | Build, test, format, run commands |
