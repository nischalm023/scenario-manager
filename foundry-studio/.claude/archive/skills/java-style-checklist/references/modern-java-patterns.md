# Modern Java 21+ Patterns

## Records vs Classes

### Use Records

```java
// DTO / API response
public record OrderSelection(Order order, List<PickTask> tasks, StationId stationId) {}

// Value object with validation
public record GridCoordinate(int row, int col) {
    public GridCoordinate {
        if (row < 0 || col < 0) {
            throw new IllegalArgumentException("Coordinates must be non-negative");
        }
    }

    public int manhattanDistance(GridCoordinate other) {
        return Math.abs(this.row - other.row) + Math.abs(this.col - other.col);
    }
}

// Configuration with @Builder
@Builder
public record RelayRobotConfig(
    int numVtms, int numHtms,
    double vtmHorizontalSpeed, double vtmVerticalSpeed,
    int vtmCapacity, double htmSpeed) implements RobotConfig {}
```

### Use Classes

```java
// Entity with mutable state and lifecycle
@Getter
@Builder
public class Order {
    private final String orderId;
    private final List<OrderItem> items;
    private final double createdTime;
    @Builder.Default
    private final OrderPriority priority = OrderPriority.MEDIUM;
    private Double deadline; // Mutable - calculated on demand
}

// Domain entity with state transitions
@Slf4j
@Getter
public final class Rack implements WarehouseInventory.StorageSpace {
    private final RackId id;
    private final int maxCapacity;
    @Setter private GridCoordinate currentCoordinate;
    @Setter private RackState state;
}
```

### Common Mistakes

```java
// BAD: Class with no behavior - should be record
@Getter @AllArgsConstructor
public class Position { private final double x; private final double y; }
// GOOD: public record Position(double x, double y) {}

// BAD: Record with mutable intent
public record Station(StationId id, int queueLength) {}
// queueLength changes -> should be class
```

## Lombok Best Practices

### Recommended Pattern

```java
// Service: @RequiredArgsConstructor + final fields
@Slf4j
@RequiredArgsConstructor
public class OrderProcessor {
    private final WarehouseInventory inventory;
    private final List<Station> stations;
}

// Entity: @Getter class-level + @Setter field-level
@Slf4j
@Getter
public class Robot {
    private final int id;
    private final double speed;
    @Setter private RobotState state;       // Only mutable fields
    @Setter private GridCoordinate currentPosition;
}
```

### Avoid

| Annotation | Problem | Use Instead |
|------------|---------|-------------|
| `@Data` | Makes everything mutable, generates equals/hashCode/toString | `@Getter` + field-level `@Setter` |
| `@AllArgsConstructor` | Brittle to field reordering | `@RequiredArgsConstructor` with final fields |
| Class-level `@Setter` | No encapsulation | Field-level `@Setter` on mutable fields only |
| `@Builder` on 2-3 fields | Overkill | Direct constructor call |

## Sealed Types

Use for closed type hierarchies. Enables exhaustive pattern matching.

```java
// Declaration
public sealed interface StorageSpace permits Rack, Tote {
    StorageSpaceId getId();
}

public final class Rack implements StorageSpace { ... }
public final class Tote implements StorageSpace { ... }

// Exhaustive switch - compiler enforces all cases
public int getCapacity(StorageSpace space) {
    return switch (space) {
        case Rack r -> r.getMaxCapacity();
        case Tote t -> t.getMaxCapacity();
        // No default needed - compiler knows all cases
    };
}

// With guards
return switch (space) {
    case Rack r when r.hasTasks() -> true;
    case Tote t when !t.getTasks().isEmpty() -> true;
    case Rack _, Tote _ -> false;
};
```

## Optional

### Rules

- **Return types only** - never fields, never collections
- Use `orElseThrow()` not `get()`
- Return empty collections, not `Optional<List<T>>`

### Good

```java
// Return Optional for possibly absent value
public Optional<RelayPosition> findRelayWithTotes(GridCoordinate position) {
    return positions.stream()
        .filter(RelayPosition::hasTotes)
        .min(Comparator.comparingInt(rp ->
            rp.getPosition().manhattanDistance(position)));
}

// Return empty collection, not Optional<Collection>
public List<RelayPosition> getPositionsForZone(String zoneId) {
    return positionsByZone.getOrDefault(zoneId, Collections.emptyList());
}
```

### Bad

```java
private Optional<Rack> carryingRack;         // Never as field
Optional<List<Order>> getPendingOrders();     // Never wrap collections
station.get();                                // Use orElseThrow()
```

## Immutability

### Core Rules

1. All fields `final` by default
2. Records for inherently immutable data
3. `List.copyOf()` / `Map.copyOf()` in constructors
4. Defensive copies when exposing collections
5. New instances for value changes

```java
// Immutable collection in constructor
public class Zone {
    private final List<GridCoordinate> coverageArea;

    public Zone(List<GridCoordinate> coverageArea) {
        this.coverageArea = List.copyOf(coverageArea);
    }
}

// Defensive copy when exposing
public List<Station> getAllStations() {
    return new ArrayList<>(stations);
}

// Acceptable mutability: entities with lifecycle
@Getter
public class Robot {
    private final int id;           // Immutable identity
    private final double speed;     // Immutable characteristic
    @Setter private RobotState state;            // Mutable state
    @Setter private GridCoordinate currentPosition; // Mutable position
}
```

## Pattern Matching (Java 21)

```java
// instanceof with variable binding
if (space instanceof Rack rack) {
    return rack.getCurrentCoordinate().manhattanDistance(target);
}

// Switch expressions
return switch (space) {
    case Rack r -> "rack";
    case Tote t -> "tote";
};

// With guards
return switch (space) {
    case Rack r when r.hasTasks() -> processRack(r);
    case Tote t when !t.isEmpty() -> processTote(t);
    default -> skip();
};
```

**Never** use old-style casting:
```java
// BAD
if (space instanceof Rack) { Rack rack = (Rack) space; }
// GOOD
if (space instanceof Rack rack) { ... }
```

## Enums with Behavior

```java
@Getter
@RequiredArgsConstructor
public enum RobotType {
    RTP("rtp"), VTM("vtm"), HTM("htm");

    private final String value;

    @JsonValue
    public String getValue() { return value; }

    @JsonCreator
    public static RobotType fromValue(String value) {
        for (RobotType type : values()) {
            if (type.value.equals(value)) return type;
        }
        throw new IllegalArgumentException("Unknown RobotType: " + value);
    }
}

public enum OrderPriority {
    LOW(1), MEDIUM(5), HIGH(10), URGENT(20);

    private final int weight;
    OrderPriority(int weight) { this.weight = weight; }
    public int getWeight() { return weight; }
    public boolean isHigherThan(OrderPriority other) {
        return this.weight > other.weight;
    }
}
```
