# Exception Handling Reference

## Checked vs Unchecked Decision Tree

```
Can the caller reasonably recover?
├── YES → Checked exception (force caller to handle)
│         Example: FileNotFoundException when loading user-provided config
└── NO  → Unchecked exception (RuntimeException subclass)
          Example: IllegalStateException when invariant is broken
```

**Default to unchecked.** Most exceptions in this codebase represent programming errors or
unrecoverable failures. Checked exceptions add boilerplate without value when callers just
rethrow anyway.

## Exception Message Format

### Include

- **What** operation failed: `"Failed to parse layout file"`
- **Context values**: `"Robot %s has no assigned zone (state=%s)".formatted(robotId, state)`
- **Expected vs actual** when applicable: `"Expected 4+ bins, got %d".formatted(count)`

### Exclude

- Stack traces (the runtime captures these)
- Sensitive data (passwords, tokens, PII)
- Generic messages with no context (`"Something went wrong"`, `"Error occurred"`)

```java
// BAD - no context
throw new IllegalArgumentException("Invalid value");

// GOOD - actionable context
throw new IllegalArgumentException(
    "Rack column count must be 1-5, got %d".formatted(columns));
```

## Throwing Rules

### Throw appropriate to the abstraction (Effective Java Item 73)

Higher layers should translate lower-level exceptions into exceptions that make sense at their
abstraction level:

```java
// BAD - FileNotFoundException for a non-file operation
public Scenario getScenario(String id) throws FileNotFoundException {
    return scenarios.get(id)
        .orElseThrow(() -> new FileNotFoundException("Scenario not found: " + id));
}

// GOOD - exception matches the abstraction
public Scenario getScenario(String id) {
    return scenarios.get(id)
        .orElseThrow(() -> new NoSuchElementException("Scenario not found: " + id));
}
```

### Always preserve the cause chain

Every re-thrown or wrapped exception **must** include the original as the cause parameter.
Losing the cause chain destroys debugging information (SonarQube S1166).

```java
// BAD - cause chain lost
catch (IOException e) {
    throw new RuntimeException("Failed to load layout: " + e.getMessage());
}

// GOOD - cause chain preserved
catch (IOException e) {
    throw new RuntimeException("Failed to load layout: " + e.getMessage(), e);
}
```

## Catching Rules

### Catch specific types

Catch the most specific exception type you can handle. Generic `catch (Exception e)` is only
acceptable at **system boundaries** (gRPC handlers, `@ControllerAdvice`, top-level schedulers).

```java
// BAD - overly broad in business logic
try {
    parseConfig(path);
} catch (Exception e) {
    log.error("Parse failed", e);
}

// GOOD - catch what you expect
try {
    parseConfig(path);
} catch (JsonProcessingException e) {
    log.error("Invalid JSON in config {}: {}", path, e.getMessage(), e);
}
```

### Never swallow exceptions

Every catch block must either **log** or **rethrow** (or both at system boundaries). An empty
catch block or one that only logs the message without the throwable hides failures.

```java
// BAD - swallowed
catch (IOException e) { }

// BAD - message only, no stack trace
catch (IOException e) {
    log.error("IO error: " + e.getMessage());
}

// GOOD - log with throwable
catch (IOException e) {
    log.error("Failed to write snapshot to {}", outputPath, e);
}
```

### Log-or-rethrow, not both (in business logic)

In non-boundary code, choose one. Logging and rethrowing produces duplicate log entries as the
exception propagates up the call stack.

```java
// BAD - logs then rethrows, causing duplicate log entries
catch (IOException e) {
    log.error("Write failed", e);
    throw new RuntimeException("Write failed", e);  // will be logged again upstream
}

// GOOD in business logic - just rethrow, let boundary handle logging
catch (IOException e) {
    throw new RuntimeException("Snapshot write failed for sim " + simId, e);
}

// GOOD at system boundary - log and convert to error response
catch (Exception e) {
    log.error("Unhandled error in simulation {}", simId, e);
    return ResponseEntity.status(500).body(ApiResponse.error(e.getMessage()));
}
```

## Logging in Catch Blocks

| Log Level | When |
|-----------|------|
| `ERROR`   | Unexpected failures, unrecoverable errors, system boundary catches |
| `WARN`    | Expected/recoverable situations (invalid client input, missing optional resource) |
| `DEBUG`   | Retries, fallback paths, diagnostic detail |

**Always pass the throwable as the last argument** — SLF4J captures the full stack trace:

```java
log.error("Simulation {} failed at tick {}", simId, currentTick, exception);
//                                                                ^^^^^^^^^ last arg = throwable
```

## @ControllerAdvice Pattern (REST)

Centralize REST error handling. Controllers should **throw**, not catch-and-wrap:

```java
// Controller - just throw, no try/catch
@GetMapping("/{id}")
public ScenarioDto getScenario(@PathVariable String id) {
    return scenarioService.findById(id)
        .orElseThrow(() -> new NoSuchElementException("Scenario not found: " + id));
}

// GlobalExceptionHandler - maps exceptions to HTTP responses
@RestControllerAdvice
public class GlobalExceptionHandler {

    @ExceptionHandler(NoSuchElementException.class)
    public ResponseEntity<ApiResponse> handleNotFound(NoSuchElementException e) {
        return ResponseEntity.status(404).body(ApiResponse.error(e.getMessage()));
    }

    @ExceptionHandler(IllegalArgumentException.class)
    public ResponseEntity<ApiResponse> handleBadRequest(IllegalArgumentException e) {
        return ResponseEntity.status(400).body(ApiResponse.error(e.getMessage()));
    }
}
```

## gRPC Error Mapping

At gRPC service boundaries, catch broadly, log fully, and map to status codes:

```java
// Established codebase pattern (see PutGrpcService, NavigationGrpcService)
try {
    return service.process(request);
} catch (IllegalArgumentException e) {
    log.warn("Invalid request: {}", e.getMessage(), e);
    throw Status.INVALID_ARGUMENT.withDescription(e.getMessage()).withCause(e)
        .asRuntimeException();
} catch (Exception e) {
    log.error("Unexpected error processing request", e);
    throw Status.INTERNAL.withDescription(e.getMessage()).withCause(e)
        .asRuntimeException();
}
```

**Status code mapping:**

| Exception Type | gRPC Status |
|----------------|-------------|
| `IllegalArgumentException` | `INVALID_ARGUMENT` |
| `NoSuchElementException` | `NOT_FOUND` |
| `IllegalStateException` | `FAILED_PRECONDITION` |
| `UnsupportedOperationException` | `UNIMPLEMENTED` |
| All other `Exception` | `INTERNAL` |

## Anti-Patterns

| Anti-Pattern | Problem | Fix |
|-------------|---------|-----|
| `catch (Exception e) { throw new Exception("Failed: " + e.getMessage()); }` | Loses cause chain and stack trace | Add cause: `new RuntimeException(msg, e)` |
| `throws Exception` on method signature | Caller can't distinguish failure modes | Declare specific types or use unchecked |
| `FileNotFoundException` for non-file operations | Semantically wrong, misleads readers | Use `NoSuchElementException` or domain-specific type |
| Empty catch block `catch (Exception e) { }` | Silently hides failures | Log or rethrow |
| `log.error(e.getMessage())` without throwable | Loses stack trace | `log.error("msg", e)` — pass throwable last |
| Log AND rethrow in business logic | Duplicate log entries | Choose one: rethrow in business logic, log at boundary |
| `catch (Exception e)` in business logic without comment | Unclear if intentional or lazy | Catch specific types, or add `// Boundary: must catch all` comment |
