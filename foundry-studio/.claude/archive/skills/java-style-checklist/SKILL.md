---
name: java-style-checklist
description: Use when writing, reviewing, or organizing Java files.
  Code organization, commenting standards, and modern Java 21+ patterns.
---

# Java Style Guide

## 1. File Organization

### Section Ordering (canonical)
Constants > Fields > Context Records > Constructor > [Functional Sections] > Helper Methods

### Section Header Format
```java
// ========================================
// Section Name
// ========================================
```
40 `=` chars. Blank line before and after each block.

### When to Use Section Headers
| Lines | Methods | Headers |
|-------|---------|---------|
| >200  | 5+      | Required |
| 100-200 | any   | Optional |
| <100  | any     | Skip |

### Key Principle: Group by FUNCTION, not ACCESS MODIFIER
Public orchestration + private helpers live together in their domain section.
Do NOT separate all public methods from all private methods.

### Templates by File Type
| Type | Sections |
|------|----------|
| Process (SimProcess) | Constants > Fields > Records > Constructor > Lifecycle > Domain Sections > Helpers |
| Service (@Service) | Fields > Constructor > Public API > Private Helpers |
| Domain Entity | Fields > Constructor > Core Ops > Queries > Utilities |
| Controller | Fields > Constructor > Nested DTOs > Endpoints |

See [references/organization-exemplar.md](references/organization-exemplar.md) for annotated example.

## 2. Commenting Patterns

### Class Javadoc (always required)
Multi-line with `<p>` and `<ul>` for responsibilities. Every public class, no exceptions.

### Method Javadoc (tiered)
- **Simple/obvious** -> single-line: `/** Returns the total count. */`
- **Complex/algorithm** -> multi-line with `@param`/`@return` and `<p>` explanation
- **Private simple** -> no doc
- **Private complex** -> document WHY and algorithm

### Inline Comment Prefixes
| Prefix | Usage |
|--------|-------|
| `// Phase N:` | Sequential multi-step operations |
| `// SAFETY NOTE:` | Threading/concurrency assumptions |
| `// CRITICAL:` | Blocking calls, resource locks |
| `// TODO:` | Known improvements (with context) |
| `// NEW:` | Recently added features |

### Don't Comment
- Obvious operations (`robot.setState(RobotState.IDLE)`)
- Lombok-generated methods
- Standard patterns (simple getters, basic loops)

See [references/documentation-standards.md](references/documentation-standards.md) for full examples.

## 3. Modern Java 21+

### Records vs Classes
| Use Records For | Use Classes For |
|-----------------|-----------------|
| DTOs, API responses | Entities with mutable state |
| Configuration objects | Complex behavior/lifecycle |
| Value objects | Spring-managed beans |
| Result types, events | Inheritance hierarchies |

### Pattern Matching
```java
// Modern instanceof
if (obj instanceof String s) {
    return s.length();
}

// Switch expressions (exhaustive with sealed types)
return switch (space) {
    case Rack r -> r.getMaxCapacity();
    case Tote t -> t.getMaxCapacity();
};
```

### Optional
- **Return types only** - never fields, never collections
- Use `orElseThrow()` not `get()`
- Prefer `map()`/`flatMap()` over `ifPresent()`
- Return empty collections, not `Optional<List<T>>`

### Sealed Types
Use for closed hierarchies to enable exhaustive pattern matching:
```java
public sealed interface StorageSpace permits Rack, Tote {
    StorageSpaceId getId();
}
```

### Immutability
- All fields `final` by default
- `List.of()`, `Set.of()`, `Map.of()` for immutable collections
- `List.copyOf()` in constructors for defensive copies
- New instances for value changes (`record.withX(newValue)`)

See [references/modern-java-patterns.md](references/modern-java-patterns.md) for full guide.

## 4. Lombok Rules

### Use These
- `@Slf4j` - logging
- `@Getter` - class-level for immutable data
- `@RequiredArgsConstructor` - for final field injection
- `@Builder` - for complex object construction (4+ fields)
- `@Setter` - **field-level only** for specific mutable fields

### Avoid These
- `@Data` - too permissive, use `@Getter` + explicit `@Setter`
- `@AllArgsConstructor` - brittle to field changes
- Class-level `@Setter` - makes everything mutable

## 5. Exception Handling

**Checked vs unchecked:** Default to unchecked. Use checked only when caller can reasonably recover.

**Core rules:**
- Throw exceptions appropriate to the abstraction (`NoSuchElementException`, not `FileNotFoundException` for non-file ops)
- Always preserve cause chain: `new XException(msg, cause)`
- Catch specific types in business logic; broad `catch (Exception e)` only at system boundaries (gRPC handlers, `@ControllerAdvice`)
- Log-or-rethrow, never swallow, never both in business logic

See [references/exception-handling.md](references/exception-handling.md) for patterns, gRPC status mapping, and anti-patterns table.

## 6. Logging

`@Slf4j` always. SLF4J `{}` placeholders always. Never string concatenation. Never log secrets/PII.

| Level | Use For | Example |
|-------|---------|---------|
| ERROR | Unexpected failures, boundary catches | Simulation engine crash, gRPC INTERNAL |
| WARN  | Recoverable issues, bad client input | Missing session, invalid request params |
| INFO  | Business milestones, gRPC summaries | Sim start/complete, order batch loaded |
| DEBUG | Flow tracing, internal decisions | Robot cycles, pathfinding, rack filtering |

See [references/logging-conventions.md](references/logging-conventions.md) for message formatting, anti-patterns, and performance.

## 7. Before Commit Checklist

### Organization
- [ ] Section headers for files >200 lines
- [ ] Functional grouping (not public/private separation)
- [ ] Class-level Javadoc on all public classes

### Code
- [ ] Records for immutable data, classes for entities
- [ ] `@RequiredArgsConstructor` with final fields (no `@Autowired`)
- [ ] No `@Data` - use `@Getter` + field-level `@Setter`
- [ ] Optional only for return types
- [ ] Pattern matching over manual instanceof
- [ ] Immutable collections (`List.of`, `Set.of`)
- [ ] Defensive copies when exposing mutable collections
- [ ] Sealed types for closed hierarchies

### Exceptions
- [ ] Exceptions are specific to the abstraction (no `FileNotFoundException` for non-file ops)
- [ ] Cause chain preserved (`new XException(msg, cause)`)
- [ ] Catch blocks: log-or-rethrow, never swallow
- [ ] No `catch(Exception e)` in business logic without justifying comment

### Logging
- [ ] Log levels match intent (ERROR=unexpected, INFO=milestones, DEBUG=tracing)
- [ ] No secrets/PII in log messages; SLF4J `{}` not string concat

### Formatting
- [ ] Line length under 100 chars
- [ ] Ran `mvn spotless:apply`

See [references/review-checklist.md](references/review-checklist.md) for the full review checklist.

## Related Skills

- `.claude/skills/java-backend/` - Desmo-J patterns, metrics integration
- `.claude/skills/relay-system/` - VTM/HTM robots, relay positions, zone capacity
