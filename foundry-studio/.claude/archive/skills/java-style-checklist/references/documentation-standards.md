# Documentation Standards

## Class-Level Javadoc

**Rule**: Always required on public classes. Multi-line with `<p>` structure.

### Good

```java
/**
 * Manages relay positions in the warehouse relay system.
 *
 * <p>Key responsibilities:
 * - Generate relay positions from rack blocks
 * - Manage zone accessibility mapping
 * - Select optimal relays for VTM/HTM routing
 *
 * <p>Relay positions are cantilever positions where VTMs deposit totes
 * for HTMs to pick up. Each position has capacity constraints enforced
 * via Desmo-J resources.
 */
@Slf4j
@Getter
public class RelayPositionManager {
```

### Bad

```java
/**
 * Order processor.
 */
// Too brief - doesn't explain responsibilities or algorithms

/**
 * This class is responsible for processing orders and selecting which orders
 * should be assigned to bins based on various optimization algorithms...
 */
// Wall of text - use <p> tags and bullet points
```

## Public Method Javadoc

### Simple Methods -> Single Line

```java
/** Get all stations in the warehouse. */
public List<Station> getAllStations() { ... }

/** Calculate travel time from current position to destination. */
public double calculateTravelTime(GridCoordinate destination) { ... }
```

### Complex Methods -> Multi-line with @param/@return

```java
/**
 * Select next orders to assign to available bins using configured algorithm.
 *
 * <p>Process:
 * 1. Run selection algorithm (PPF maximization or tiering)
 * 2. Validate inventory reservations (all-or-nothing)
 * 3. Assign station IDs to validated tasks
 *
 * @param pendingOrders Orders awaiting assignment
 * @param activeStorageSpaces Storage spaces currently active (for piggybacking)
 * @param maxOrders Maximum orders to select
 * @param currentTime Simulation time in seconds
 * @return List of validated order selections with assigned stations
 */
public List<OrderSelection> selectNextOrders(...) {
```

### Don't Write Boilerplate

```java
// BAD - just repeats parameter names
/**
 * @param pendingOrders the pending orders
 * @param maxOrders the max orders
 * @return the order selections
 */
```

## Private Method Documentation

### Document: Complex algorithms

```java
/**
 * Enhanced picks per face algorithm: heavily penalize new storage spaces,
 * reward active storage space reuse.
 *
 * <p>Tiered scoring system:
 * - Base: Total picks quantity
 * - Bonus: +100 per already-active storage space
 * - Penalty: -1000 per newly-opened storage space
 */
private List<OrderSelection> selectOrdersPicksPerFaceWithTiering(...) {
```

### Skip: Simple helpers

```java
// Self-explanatory - no doc needed
private boolean isAdjacentToZone(GridCoordinate relayPos, Zone zone) { ... }

private double now() {
    return presentTime().getTimeAsDouble(TimeUnit.SECONDS);
}
```

## Inline Comments

### Good Uses

```java
// Phase 1: Move to relay
htm.setState(RobotState.MOVING_TO_RACK);
double travelToRelay = htm.moveToPosition(relay.getPosition());
hold(new TimeSpan(travelToRelay, TimeUnit.SECONDS));

// CRITICAL: Desmo-J blocking call - process will suspend here
operatorResource.provide(1);

// Weighted selection: 60% VTM travel + 40% HTM travel
return viableRelays.stream()
    .min(Comparator.comparingDouble(relay -> {
        int vtmTravel = vtm.getPosition().manhattanDistance(relay.getPosition());
        int htmTravel = relay.getPosition().manhattanDistance(targetStation);
        return 0.6 * vtmTravel + 0.4 * htmTravel;
    }));

// TODO: Re-evaluate caching strategy after performance profiling
this.orderTaskCache = null;
```

### Bad Uses

```java
// Set state to idle           <- repeats code
robot.setState(RobotState.IDLE);

// Loop through all orders     <- obvious from code
for (Order order : orders) {

// TODO: fix this later        <- vague, no context
```

## Records and DTOs

### Document: Non-obvious fields or business meaning

```java
/**
 * Configuration for relay system robots.
 *
 * @param numVtms Number of Vertical Tote Movers (zone-bound)
 * @param numHtms Number of Horizontal Tote Movers (global)
 * @param vtmCapacity Maximum totes a VTM can carry simultaneously
 */
@Builder
public record RelayRobotConfig(int numVtms, int numHtms, ...) {}
```

### Skip: Self-explanatory records

```java
// No doc needed
public record OrderSelection(Order order, List<PickTask> tasks, StationId stationId) {}
public record Position(double x, double y) {}
private record StationDistance(Station station, int distance) {}
```

## Decision Tree

- **Public class?** -> Always add class-level Javadoc
- **Public method?**
  - Simple -> single-line Javadoc
  - Complex -> multi-line with @param/@return
- **Private method?**
  - Simple helper -> no doc
  - Complex/algorithm -> document WHY
- **Inline comment?**
  - Explains WHY -> yes (phase markers, critical patterns, TODOs, formulas)
  - Repeats WHAT -> no

## Anti-Patterns

| Pattern | Example | Fix |
|---------|---------|-----|
| Over-documentation | Javadoc on `getId()` | Let @Getter handle it |
| Under-documentation | No class doc on 200-line class | Add multi-line Javadoc |
| Stale docs | "Use cache" on `cache = null` | Update or remove |
| Boilerplate | `@param name the name` | Add real context or remove |
| Comment clutter | `// Step 1` on `doStepOne()` | Extract to methods instead |
