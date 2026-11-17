# TPQ Calculator A* Sampling Calibration Design

## Problem

The TPQ Calculator uses Manhattan distance to estimate travel time, which underestimates actual travel because it ignores:
- Turn costs (2s per 90° turn)
- Obstacle avoidance (loaded robots can't pass through rack positions)

Observed: ~80s actual travel vs ~40s Manhattan estimate (roughly 2x difference).

## Solution

Sample 20 random rack positions, run A* pathfinding to nearest station (loaded mode), and calculate an average `aStarPathfindingPenaltyFactor`:

```
aStarPathfindingPenaltyFactor = average(A* time / Manhattan time)
```

This factor is applied **only in TpqCalculator** for estimation purposes. No changes to RTP robot process - it already uses real A* pathfinding during simulation.

**Scope:** RTP system only. Relay system keeps current Manhattan approach.

**Factor Relationship:**
- `navFactor` (1.2-1.5) = congestion/navigation overhead factor
- `aStarPathfindingPenaltyFactor` (~2.0) = A* vs Manhattan distance penalty (turns + obstacles)
- Both are **multiplied** for RTP: `time = (distance / speed) * navFactor * pathfindingPenaltyFactor`
- They measure different aspects: congestion vs. pathfinding complexity

## Design Clarifications

### Random Sampling Strategy
- Use `Random` with fixed seed based on `config.hashCode()` for deterministic results
- Shuffle rack positions and take first 20 samples
- Each sample uses random starting orientation (NORTH, SOUTH, EAST, or WEST)
- Starting position = rack position (robot starts at rack)

### Station Selection
- Find nearest station by Manhattan distance
- If multiple stations are equidistant, pick the first one (deterministic)

### Error Handling & Logging
- **Calibration failure**: Use fallback factor (1.5), log at ERROR level
- **Low success rate** (<50%): Use fallback factor, log at WARN level with success rate
- **Individual path failure**: Skip sample, log at WARN level, continue with others
- **Successful calibration**: Log at INFO level with sample count and factor value

### Performance Expectations
- Calibration time: 1-5 seconds for typical warehouse (20 A* calls)
- Only runs once per `TpqCalculator` instance (lazy + cached)
- Reusing the same instance across calls = no recalibration
- No impact on subsequent calls after calibration

### API Response Enhancement
- `aStarPathfindingPenaltyFactor` added to `TpqMetrics` record and `TpqMetricsResponseDTO`
- **RTP system**: Returns calibrated factor (typically 1.5-3.0)
- **Relay system**: Returns 1.0 (no A* penalty applied)
- **Purpose**: Visibility for users and debugging - shows why travel estimates may be higher than simple distance/speed calculations

## API Changes

**New instance-based API (replaces static method):**
```java
// Factory method creates estimation-only calculator
TpqCalculator calculator = TpqCalculator.forConfig(config);

// First call triggers calibration (lazy), result is cached
TpqMetrics metrics = calculator.calculatePreSimulationMetrics();

// Subsequent calls reuse cached calibration factor
TpqMetrics metrics2 = calculator.calculatePreSimulationMetrics();  // No recalibration
```

**Runtime measurement API (unchanged behavior, renamed method):**
```java
// Instance-based runtime TPQ measurement (different constructor, different purpose)
TpqCalculator runtimeCalculator = new TpqCalculator(warehouseComponents, config);
TpqSnapshot snapshot = runtimeCalculator.measureRuntimeTpq();  // Measures actual simulation state
```

**Method Changes:**
- **REMOVE**: `static TpqMetrics calculate(SimulationConfig config)` - no longer needed
- **RENAME**: `getCurrentSystemTpq()` → `measureRuntimeTpq()` (clearer naming)
- **ADD**: `static TpqCalculator forConfig(SimulationConfig config)` - factory method
- **ADD**: `TpqMetrics calculatePreSimulationMetrics()` - instance method with calibration

## Calibration Algorithm

Runs once on first `calculatePreSimulationMetrics()` call for RTP:

```
1. Build WarehouseMap from config (needed for A* obstacle checking)
2. Create PathFinder instance
3. Get all rack positions from map
4. Randomly select 20 rack positions (using Random with fixed seed for determinism)
5. For each sampled rack:
   a. Rack position becomes the starting position
   b. Random starting orientation (NORTH, SOUTH, EAST, or WEST)
   c. Find nearest station by Manhattan distance (if tie, pick first)
   d. Run A* pathfinding (loaded mode, isCarryingRack=true)
   e. If path found:
      - Calculate ratio = A* totalTime / (manhattanDistance / robotSpeed)
      - Add to ratios list
   f. If no path found:
      - Skip this sample, log warning
6. If less than 50% samples succeeded:
   - Log warning with sample success rate
   - Use fallback factor (1.5)
7. Calculate aStarPathfindingPenaltyFactor = average(ratios)
8. Log INFO with calibration result (sample count, factor value)
9. Cache the factor for subsequent calls
```

**Parameters:**
- Sample count: 20
- Robot speed: from config (`rtpRobotConfig.robotSpeed()`)
- Turn time: `Robot.BASE_TURN_TIME_90` (2.0s)
- Starting orientation: Random (NORTH, SOUTH, EAST, or WEST) for each sample
- Starting position: Each sampled rack position
- Loaded mode: true (carrying rack)
- Random seed: Fixed seed based on config hashCode for deterministic results

**Error Handling:**
- Catch WarehouseMap construction exceptions → use fallback factor (1.5)
- Catch PathFinder exceptions → skip sample, continue with others
- Log all errors at WARN level

## Call Site Updates

### 1. DesmojSimulationEngine.java (line 144)

**Before:**
```java
TpqCalculator.TpqMetrics tpqMetrics = TpqCalculator.calculate(config);
```

**After:**
```java
TpqCalculator calculator = TpqCalculator.forConfig(config);
TpqCalculator.TpqMetrics tpqMetrics = calculator.calculatePreSimulationMetrics();
```

**Purpose:** Pre-calculate targetTpq for MaximizeTPQStrategy. With calibration, this will now use realistic A* estimates for RTP.

### 2. SimulationsController.java (lines 174-187)

**Before:**
```java
TpqCalculator.TpqMetrics metrics = TpqCalculator.calculate(coreConfig);

TpqMetricsResponseDTO response =
    new TpqMetricsResponseDTO(
        metrics.warehouseWidthMeters(),
        metrics.warehouseDepthMeters(),
        metrics.avgTravelDistanceMeters(),
        metrics.avgTravelTimeSeconds(),
        metrics.avgOperatorTimeSeconds(),
        metrics.targetTpq(),
        metrics.optimalRobotsPerStation(),
        metrics.totalOptimalRobots(),
        metrics.fleetStatus(),
        metrics.robotDifference());
```

**After:**
```java
TpqCalculator calculator = TpqCalculator.forConfig(coreConfig);
TpqCalculator.TpqMetrics metrics = calculator.calculatePreSimulationMetrics();

TpqMetricsResponseDTO response =
    new TpqMetricsResponseDTO(
        metrics.warehouseWidthMeters(),
        metrics.warehouseDepthMeters(),
        metrics.avgTravelDistanceMeters(),
        metrics.avgTravelTimeSeconds(),
        metrics.avgOperatorTimeSeconds(),
        metrics.targetTpq(),
        metrics.optimalRobotsPerStation(),
        metrics.totalOptimalRobots(),
        metrics.fleetStatus(),
        metrics.robotDifference(),
        metrics.aStarPathfindingPenaltyFactor());  // NEW: Include penalty factor
```

**Purpose:** REST API endpoint `/api/simulations/tpq-metrics`. Will now return calibrated estimates for RTP with visible penalty factor.

**Also update TpqMetricsResponseDTO record:**
```java
record TpqMetricsResponseDTO(
    double warehouseWidthMeters,
    double warehouseDepthMeters,
    double avgTravelDistanceMeters,
    double avgTravelTimeSeconds,
    double avgOperatorTimeSeconds,
    double targetTpq,
    int optimalRobotsPerStation,
    int totalOptimalRobots,
    String fleetStatus,
    int robotDifference,
    double aStarPathfindingPenaltyFactor) {}  // NEW field
```

### 3. DesmojBinManagementProcess.java (line 96)

**Before:**
```java
TpqCalculator.TpqSnapshot snapshot = tpqCalculator.getCurrentSystemTpq();
```

**After:**
```java
TpqCalculator.TpqSnapshot snapshot = tpqCalculator.measureRuntimeTpq();
```

**Purpose:** Runtime TPQ measurement during simulation. Only method name changes, behavior unchanged.

### 4. Frontend: RobotsTab.tsx

**Add display of A* pathfinding penalty factor in the configuration table.**

After the "Nav Factor" row (around line 180-185), add:

```tsx
{
  parameter: 'A* Pathfinding Factor',
  value: (metrics?.aStarPathfindingPenaltyFactor ?? 1.0).toFixed(2),
  unit: 'x',
  notes: config.systemType === 'rtp' ? 'Calibrated A* penalty' : 'Not applicable (Relay)',
}
```

**Purpose:** Show users the calibrated A* pathfinding penalty factor that's being applied to RTP travel time estimates. For Relay systems, this will show 1.0.

**Example Display:**
```
Travel Parameters
├─ Robot Speed: 1.5 m/s - Base travel speed
├─ Warehouse Grid: 30×20 racks - Layout dimensions
├─ Nav Factor: 1.2 x - Path efficiency multiplier
├─ A* Pathfinding Factor: 2.05 x - Calibrated A* penalty  ← NEW
├─ Avg Travel Distance: 25.5 m - Station to rack average
└─ Avg Travel Time: 42 s - Based on robot speed
```

This helps users understand:
- Why travel time estimates are higher than simple distance/speed calculations
- The actual pathfinding complexity of their warehouse layout
- Difference between RTP (with turns/obstacles) vs Relay systems

## Implementation Summary

**Files to Create:**
- None (all modifications to existing files)

**Files to Modify:**
1. `TpqCalculator.java` - Add `aStarPathfindingPenaltyFactor` field to TpqMetrics record
2. `Robot.java` - Add `BASE_TURN_TIME_90` constant
3. `TpqCalculator.java` - Add calibration logic, factory method, rename methods
4. `DesmojSimulationEngine.java` - Update to use new API
5. `SimulationsController.java` - Update to use new API, add field to TpqMetricsResponseDTO
6. `DesmojBinManagementProcess.java` - Update method name
7. `RobotsTab.tsx` (frontend) - Display A* pathfinding factor in configuration table

**Breaking Changes:**
- `TpqCalculator.calculate(config)` static method removed
- `TpqCalculator.getCurrentSystemTpq()` renamed to `measureRuntimeTpq()`

## Files to Modify

### 1. TpqCalculator.TpqMetrics Record

**Add new field for visibility:**
- `double aStarPathfindingPenaltyFactor` - The calibrated penalty factor (RTP: ~2.0, Relay: 1.0)

**Before:**
```java
public record TpqMetrics(
    double warehouseWidthMeters,
    double warehouseDepthMeters,
    double avgTravelDistanceMeters,
    double avgTravelTimeSeconds,
    double avgOperatorTimeSeconds,
    double targetTpq,
    int optimalRobotsPerStation,
    int totalOptimalRobots,
    String fleetStatus,
    int robotDifference) {}
```

**After:**
```java
public record TpqMetrics(
    double warehouseWidthMeters,
    double warehouseDepthMeters,
    double avgTravelDistanceMeters,
    double avgTravelTimeSeconds,
    double avgOperatorTimeSeconds,
    double targetTpq,
    int optimalRobotsPerStation,
    int totalOptimalRobots,
    String fleetStatus,
    int robotDifference,
    double aStarPathfindingPenaltyFactor) {}  // NEW: Exposes calibrated factor
```

**Purpose:** Expose the calibrated A* penalty factor for debugging and visibility in the frontend robots page.

### 2. Robot.java

Add public static constant for calibration use. The existing instance field `baseTurnTime90` remains unchanged for runtime flexibility.

```java
public class Robot {
  // New public constant for calibration and default values
  public static final double BASE_TURN_TIME_90 = 2.0;
  
  // Existing instance field (unchanged - for runtime flexibility)
  private final double baseTurnTime90;
  
  // Existing constructors already default to 2.0, can now reference constant
  public Robot(int id, double speed, RobotType robotType) {
    this(id, speed, robotType, Orientation.EAST, BASE_TURN_TIME_90, null);
  }
  
  public Robot(int id, double speed, RobotType robotType, List<String> zoneAssignment) {
    this(id, speed, robotType, Orientation.EAST, BASE_TURN_TIME_90, zoneAssignment);
  }
  
  public Robot(int id, double speed) {
    this(id, speed, RobotType.RTP, Orientation.EAST, BASE_TURN_TIME_90, null);
  }
}
```

**Why Both Constant and Field?**
- **Static constant**: Used by TpqCalculator for calibration (standard reference value)
- **Instance field**: Allows runtime flexibility if different robots need different turn times

### 2. TpqCalculator.java

**Changes:**
- **REMOVE** static `calculate(SimulationConfig config)` method
- **ADD** new private constructor for estimation-only instances
- **ADD** static factory method `forConfig(SimulationConfig config)`
- **ADD** instance method `calculatePreSimulationMetrics()`
- **RENAME** `getCurrentSystemTpq()` to `measureRuntimeTpq()`
- **ADD** calibration logic with caching

```java
public final class TpqCalculator {

  // Existing fields for runtime measurement
  private final WarehouseComponents warehouseComponents;  // null for estimation-only instances
  private final SimulationConfig config;
  private final double robotSpeed;
  private final double navFactor;
  private final double operatorTimePerPick;

  // New fields for calibration (estimation-only instances)
  private Double cachedAStarPathfindingPenaltyFactor;  // null = not yet calibrated
  private static final int SAMPLE_COUNT = 20;
  private static final double FALLBACK_PATHFINDING_PENALTY_FACTOR = 1.5;
  private static final double MIN_SUCCESS_RATE = 0.5;

  // Existing constructor for runtime measurement (unchanged)
  public TpqCalculator(WarehouseComponents warehouseComponents, SimulationConfig config) {
    this.warehouseComponents = warehouseComponents;
    this.config = config;
    // ... existing initialization code ...
  }

  // New private constructor for estimation-only (no WarehouseComponents)
  private TpqCalculator(SimulationConfig config) {
    this.warehouseComponents = null;  // Estimation-only, no runtime measurement
    this.config = config;
    this.cachedAStarPathfindingPenaltyFactor = null;  // Lazy init
    
    // Initialize required fields for estimation
    if (config.systemType() == SystemType.RTP) {
      this.robotSpeed = config.getRtpRobotConfig().robotSpeed();
    } else {
      this.robotSpeed = config.getRelayRobotConfig().htmSpeed();
    }
    this.navFactor = config.algorithmConfig().navFactor();
    this.operatorTimePerPick = config.operatorConfig().operatorTimePerPick();
  }

  // New factory method
  public static TpqCalculator forConfig(SimulationConfig config) {
    return new TpqCalculator(config);
  }

  // New instance method for pre-simulation estimation
  public TpqMetrics calculatePreSimulationMetrics() {
    if (config.systemType() == SystemType.RTP && cachedAStarPathfindingPenaltyFactor == null) {
      cachedAStarPathfindingPenaltyFactor = runCalibration();
    }
    double pathfindingFactor = cachedAStarPathfindingPenaltyFactor != null
        ? cachedAStarPathfindingPenaltyFactor 
        : 1.0;
    return calculateWithFactor(pathfindingFactor);
  }

  // Renamed from getCurrentSystemTpq() for clarity
  public TpqSnapshot measureRuntimeTpq() {
    if (warehouseComponents == null) {
      throw new IllegalStateException("Cannot measure runtime TPQ on estimation-only calculator");
    }
    // ... existing runtime measurement code ...
  }

  // Calibration uses Robot.BASE_TURN_TIME_90
  private double runCalibration() {
    try {
      WarehouseMap map = new WarehouseMap(config.systemType(), config.mapConfig());
      PathFinder pathFinder = new PathFinder();
      
      List<GridCoordinate> rackPositions = map.getRackPositions();
      List<GridCoordinate> stationPositions = map.getStationPositions()
          .stream()
          .map(StationGridPosition::position)
          .toList();
      
      if (rackPositions.isEmpty() || stationPositions.isEmpty()) {
        log.warn("Cannot calibrate: no rack or station positions available");
        return FALLBACK_PATHFINDING_PENALTY_FACTOR;
      }
      
      // Random sampling with fixed seed for determinism
      Random random = new Random(config.hashCode());
      Orientation[] orientations = Orientation.values();
      List<Double> ratios = new ArrayList<>();
      int sampleSize = Math.min(SAMPLE_COUNT, rackPositions.size());
      
      // Shuffle and take first N
      List<GridCoordinate> shuffledRacks = new ArrayList<>(rackPositions);
      Collections.shuffle(shuffledRacks, random);
      List<GridCoordinate> sampledRacks = shuffledRacks.subList(0, sampleSize);
      
      for (GridCoordinate rackPos : sampledRacks) {
        // Random starting orientation
        Orientation startOrientation = orientations[random.nextInt(orientations.length)];
        
        // Find nearest station
        GridCoordinate nearestStation = findNearestStation(rackPos, stationPositions);
        int manhattanDist = rackPos.manhattanDistance(nearestStation);
        
        // Run A* pathfinding
        PathResult pathResult = pathFinder.findPath(
            rackPos,
            startOrientation,
            nearestStation,
            map,
            true,  // isCarryingRack
            robotSpeed,
            Robot.BASE_TURN_TIME_90
        );
        
        if (pathResult.waypoints().isEmpty()) {
          log.warn("No path found from rack {} to station {}", rackPos, nearestStation);
          continue;
        }
        
        double manhattanTime = manhattanDist / robotSpeed;
        double aStarTime = pathResult.totalTime();
        double ratio = aStarTime / manhattanTime;
        ratios.add(ratio);
      }
      
      // Check success rate
      double successRate = (double) ratios.size() / sampleSize;
      if (successRate < MIN_SUCCESS_RATE) {
        log.warn(
            "Calibration had low success rate: {}/{} samples ({}%)",
            ratios.size(), sampleSize, String.format("%.1f", successRate * 100));
        return FALLBACK_PATHFINDING_PENALTY_FACTOR;
      }
      
      // Calculate average
      double avgFactor = ratios.stream().mapToDouble(Double::doubleValue).average().orElse(FALLBACK_PATHFINDING_PENALTY_FACTOR);
      
      log.info(
          "A* pathfinding calibration complete: {}/{} samples, factor={:.2f}",
          ratios.size(), sampleSize, avgFactor);
      
      return avgFactor;
      
    } catch (Exception e) {
      log.error("Calibration failed with exception, using fallback factor", e);
      return FALLBACK_PATHFINDING_PENALTY_FACTOR;
    }
  }
  
  private GridCoordinate findNearestStation(GridCoordinate from, List<GridCoordinate> stations) {
    return stations.stream()
        .min(Comparator.comparingInt(from::manhattanDistance))
        .orElseThrow(() -> new IllegalStateException("No stations available"));
  }
  
  // Helper method to calculate metrics with given pathfinding factor
  private TpqMetrics calculateWithFactor(double pathfindingFactor) {
    // Extract dimensions and calculate as before...
    // Use pathfindingFactor in travel time calculation
    
    // Example (similar to existing static calculate method):
    double avgTravelTime = (avgTravelDistance / robotSpeed) * navFactor * pathfindingFactor;
    
    // ... calculate other fields ...
    
    return new TpqMetrics(
        warehouseWidth,
        warehouseDepth,
        avgTravelDistance,
        avgTravelTime,
        avgOperatorTime,
        targetTpq,
        optimalRobotsPerStation,
        totalOptimalRobots,
        fleetStatus,
        robotDifference,
        pathfindingFactor);  // NEW: Include the factor in result
  }
}
```

### Where to Apply the Penalty Factor

**1. In calculatePreSimulationMetrics() method (via calculateWithFactor helper):**
```java
// Apply both navFactor (congestion) and pathfindingPenaltyFactor (A* vs Manhattan)
double pathfindingPenaltyFactor = (config.systemType() == SystemType.RTP)
    ? cachedAStarPathfindingPenaltyFactor
    : 1.0;
double avgTravelTime = (avgTravelDistance / robotSpeed) * navFactor * pathfindingPenaltyFactor;
```

**2. In measureRuntimeTpq() method (renamed from getCurrentSystemTpq):**
```java
// Apply both factors for RTP
double pathfindingPenaltyFactor = (config.systemType() == SystemType.RTP)
    ? cachedAStarPathfindingPenaltyFactor
    : 1.0;

// In the loop over tasks
totalTravelTime += (distance / robotSpeed) * navFactor * pathfindingPenaltyFactor;
```

**Why Both Factors?**
- `navFactor`: Generic congestion/navigation overhead (1.2-1.5)
- `pathfindingPenaltyFactor`: A* path complexity vs Manhattan distance (~2.0)
- They capture different aspects and should be multiplied

## No Changes Required

- **DesmojRtpRobotProcess** - Already uses real A* pathfinding
- **Relay system** - Keeps Manhattan approach (VTM/HTM paths are simpler)
- **DesmojVtmProcess / DesmojHtmProcess** - No changes

## Expected Outcome

- TPQ estimates will be more realistic for RTP system
- Pre-simulation TPQ calculations (robots page) will match actual simulation results more closely
- Target TPQ settings will be based on realistic travel time estimates
- Calibration adds 1-5 seconds to initial calculation (acceptable for pre-simulation)

## Testing Strategy

**Unit Tests:**
1. Test calibration with various warehouse sizes (small, medium, large)
2. Test fallback behavior when <50% samples succeed
3. Test caching behavior (calibration runs only once)
4. Test deterministic behavior (same config → same factor)
5. Test edge cases (empty racks, no stations, exception handling)

**Integration Tests:**
1. Compare calibrated TPQ estimates with actual simulation results
2. Verify penalty factor is in reasonable range (1.5-3.0)
3. Test both static and instance-based APIs produce same results

**Performance Tests:**
1. Measure calibration time for typical warehouse (should be <5 seconds)
2. Verify no performance impact on subsequent calls (caching works)
