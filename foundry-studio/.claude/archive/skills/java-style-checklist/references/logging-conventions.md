# Logging Conventions Reference

## Log Level Semantics

| Level | When to Use | Codebase Examples |
|-------|-------------|-------------------|
| `ERROR` | Unexpected failures, unrecoverable errors, system boundary catches | `log.error("Robot {} encountered error: {}", robot.getId(), e.getMessage(), e)` — DesmojRtpRobotProcess |
| `WARN` | Recoverable issues, invalid client input, missing optional resources | `log.warn("Invalid layout request: {}", e.getMessage())` — LayoutGrpcService |
| `INFO` | Business milestones, gRPC summaries, major state changes, lifecycle events | `log.info("Simulation completed at t={} (real time: {}s)", simulationTime, realTime)` — DesmojSimulationEngine |
| `DEBUG` | Flow tracing, internal decisions, filtering logic, pathfinding detail | `log.debug("Robot {}: Starting cycle - rack={}, stations={}", ...)` — DesmojRtpRobotProcess |
| `TRACE` | Ultra-fine detail (event emission, relay reservation) — rarely used | Reserved for highest-volume code paths only |

### gRPC Service Level Pattern

The established pattern for gRPC services uses consistent level assignment:

```java
// INFO: Request received (summarize inputs, not full payload)
log.info("gRPC initializeInventory: {} SKUs, {} storage spaces, algorithm={}",
    request.getSkusCount(), request.getStorageSpacesCount(), request.getAlgorithm());

// INFO: Request completed (summarize results with timing)
log.info("gRPC initializeInventory completed: {} SKUs placed, {} units in {}ms",
    response.getTotalSkusPlaced(), response.getTotalUnitsPlaced(), elapsed);

// WARN: Client error (invalid input, missing resource)
log.warn("Invalid layout request: {}", e.getMessage());
log.warn("Session not found: {}", sessionId);

// ERROR: Server error (unexpected failure)
log.error("Error in gRPC getLayout", e);
```

## Logger Declaration

**Always use `@Slf4j` (Lombok).** Never manual `LoggerFactory.getLogger()`.
The Lombok-generated field name is `log` (not `logger` or `LOGGER`).

```java
// GOOD
@Slf4j
public class MyService { ... }

// BAD - manual declaration, inconsistent naming
private static final Logger logger = LoggerFactory.getLogger(MyService.class);
```

## Message Formatting

### Always use SLF4J `{}` placeholders

```java
// GOOD - parameterized (deferred evaluation)
log.info("Robot {} starting lifecycle at t={}", robot.getId(), presentTime());

log.debug("gRPC findPath: sessionId={}, start=({},{}), goal=({},{}), robotType={}",
    sessionId,
    request.getStart().getRow(), request.getStart().getCol(),
    request.getGoal().getRow(), request.getGoal().getCol(),
    request.getRobotType());

// BAD - string concatenation (evaluates even when level is disabled)
log.debug("cl --" + cl.toString());
log.debug("Field Map after Removing null value" + fieldMap);
```

### Number formatting

`String.format()` is acceptable inside `{}` placeholders for number formatting:

```java
log.info("Calculated targetTpq={} from geometry",
    String.format("%.2f", tpqMetrics.targetTpq()));
```

### Include business identifiers

Every log message should include enough context to correlate with the operation:

```java
// GOOD - includes robotId, rackId, state
log.debug("Robot {}: Rack {} has tasks but state={} - FILTERED OUT",
    robot.getId(), rack.getId(), rack.getState());

// BAD - no identifiers
log.info("Processing request");
log.error("Something failed");
```

### Structured key=value format

Prefer `key=value` pairs within messages for parseable output:

```java
log.info("gRPC createSession completed: sessionId={}", sessionId);
log.info("Experiment configured: duration={}s, robots={}", duration, robots.size());
```

## What to Log (and What Not To)

| Log This | Don't Log This |
|----------|---------------|
| Business events (sim start/complete, order loaded) | Method entry/exit for every method |
| State transitions (robot idle→moving, rack reserved) | Obvious operations (setting a field) |
| gRPC request/response summaries with timing | Full request/response payloads |
| Unexpected conditions and why they matter | Expected null checks that succeed |
| Error context with identifiers | Passwords, API keys, tokens, PII |
| Filtered/skipped items with reason | Every loop iteration in normal flow |
| Configuration values at startup | Redundant info already in stack trace |

**Lifecycle events:** Log start and end of long-running processes with timing:
`log.info("Running Desmo-J simulation for {}s...", duration)` →
`log.info("Simulation completed at t={} (real time: {}s)", simTime, realTime)`

**Suspicious states:** Use WARN for conditions that aren't errors but warrant investigation:
`log.warn("Robot {}: Rack {} has tasks but no station assignments", robotId, rackId)`

## Exception Logging

**Always pass the throwable as the last argument** — SLF4J captures the full stack trace:

```java
log.error("Simulation failed: {}", e.getMessage(), e);
//                                                  ^ throwable last
```

For all other exception logging rules (log-or-rethrow, level selection in catch blocks,
anti-patterns), see [exception-handling.md](exception-handling.md) § "Logging in Catch Blocks".

## Performance

SLF4J `{}` placeholders defer string formatting — if the level is disabled, no work happens.
Guard clauses are needed **only** when computing the log argument itself is expensive:

```java
// No guard needed - getId() and presentTime() are cheap
log.debug("Robot {} starting cycle at t={}", robot.getId(), presentTime());

// Guard needed - expensive computation just for logging
if (log.isDebugEnabled()) {
    String routeSummary = computeDetailedRouteSummary(path);  // expensive
    log.debug("Route details: {}", routeSummary);
}
```

The SLF4J 2.0 fluent API (`log.atDebug().addKeyValue(...)`) exists but is **not required**.
Standard `{}` placeholders are sufficient for all current use cases.

## Structured Logging (Future Reference)

JSON logging via Logstash encoder (`logstash-logback-encoder`) enables structured output for
centralized log systems (ELK, Datadog, Splunk). The platform currently uses plain-text console
output via Logback. When structured logging becomes relevant:
- MDC values (already used for request tracing) become queryable JSON fields
- Key=value patterns in messages map naturally to JSON properties
- No code changes needed if `{}` placeholders are consistently used

Not prescribed for current use — document here for future reference.

## Anti-Patterns

| Anti-Pattern | Problem | Fix |
|-------------|---------|-----|
| `log.debug("value: " + obj)` | String concatenation evaluates even when DEBUG is off | `log.debug("value: {}", obj)` |
| `log.info("Processing request")` | No identifiers — impossible to correlate | Include IDs: `log.info("Processing order {}", orderId)` |
| Logging every loop iteration at INFO | Floods production logs, hides real signals | Use DEBUG, or log summary after loop |
| `log.error(e.getMessage())` without throwable | Loses stack trace | `log.error("msg: {}", e.getMessage(), e)` |
| Logging secrets/tokens/PII | Security violation | Never log credentials or personal data |
| `private static final Logger logger = ...` | Inconsistent with Lombok convention | Use `@Slf4j` — field is always `log` |
