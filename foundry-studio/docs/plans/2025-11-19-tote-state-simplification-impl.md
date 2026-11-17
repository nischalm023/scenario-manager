# Tote State Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate redundant state in Tote by centralizing location tracking in StorageSpaceManager, deriving state from location.

**Architecture:** Containers (Rack, RelayPosition, Robot) remain source of truth for physical containment. StorageSpaceManager maintains a ToteLocation index and orchestrates all moves. Tote becomes minimal: id, tasks, claims only.

**Tech Stack:** Java 21, Desmo-J, JUnit 5, Spring Boot 3.2

---

## Task 1: Create ToteLocation Sealed Types

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/ToteLocation.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/ToteLocationTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.common.types.StationId;
import org.junit.jupiter.api.Test;

class ToteLocationTest {

    @Test
    void inRack_createsCorrectLocation() {
        RackId rackId = new RackId(5);
        ToteLocation location = new ToteLocation.InRack(rackId, 2);

        assertThat(location).isInstanceOf(ToteLocation.InRack.class);
        ToteLocation.InRack inRack = (ToteLocation.InRack) location;
        assertThat(inRack.rackId()).isEqualTo(rackId);
        assertThat(inRack.zIndex()).isEqualTo(2);
    }

    @Test
    void atRelay_createsCorrectLocation() {
        ToteLocation location = new ToteLocation.AtRelay("relay_block_0_L_r5");

        assertThat(location).isInstanceOf(ToteLocation.AtRelay.class);
        ToteLocation.AtRelay atRelay = (ToteLocation.AtRelay) location;
        assertThat(atRelay.relayId()).isEqualTo("relay_block_0_L_r5");
    }

    @Test
    void atStation_createsCorrectLocation() {
        ToteLocation location = new ToteLocation.AtStation(new StationId(3));

        assertThat(location).isInstanceOf(ToteLocation.AtStation.class);
        ToteLocation.AtStation atStation = (ToteLocation.AtStation) location;
        assertThat(atStation.stationId()).isEqualTo(new StationId(3));
    }

    @Test
    void withRobot_vtmToRelay_createsCorrectLocation() {
        TransportIntent intent = new TransportIntent.ToRelay("relay_block_0_L_r3");
        ToteLocation location = new ToteLocation.WithRobot(1, RobotType.VTM, intent);

        assertThat(location).isInstanceOf(ToteLocation.WithRobot.class);
        ToteLocation.WithRobot withRobot = (ToteLocation.WithRobot) location;
        assertThat(withRobot.robotId()).isEqualTo(1);
        assertThat(withRobot.robotType()).isEqualTo(RobotType.VTM);
        assertThat(withRobot.intent()).isInstanceOf(TransportIntent.ToRelay.class);
    }

    @Test
    void withRobot_htmToStation_createsCorrectLocation() {
        TransportIntent intent = new TransportIntent.ToStation(new StationId(2));
        ToteLocation location = new ToteLocation.WithRobot(3, RobotType.HTM, intent);

        ToteLocation.WithRobot withRobot = (ToteLocation.WithRobot) location;
        assertThat(withRobot.robotType()).isEqualTo(RobotType.HTM);
        assertThat(withRobot.intent()).isInstanceOf(TransportIntent.ToStation.class);
    }

    @Test
    void withRobot_returningToRack_createsCorrectLocation() {
        TransportIntent intent = new TransportIntent.ReturningToRack();
        ToteLocation location = new ToteLocation.WithRobot(2, RobotType.VTM, intent);

        ToteLocation.WithRobot withRobot = (ToteLocation.WithRobot) location;
        assertThat(withRobot.intent()).isInstanceOf(TransportIntent.ReturningToRack.class);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=ToteLocationTest -q`
Expected: FAIL with compilation error "cannot find symbol: class ToteLocation"

**Step 3: Write minimal implementation**

```java
package com.greyorange.dessimulator.domain.inventory;

import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.common.types.StationId;

/**
 * Represents a tote's current physical location.
 *
 * <p>Key characteristics:
 * <ul>
 *   <li>Sealed type enabling exhaustive pattern matching
 *   <li>Coordinates are derived from container (rack, relay, robot, station)
 *   <li>TransportIntent tracks journey destination for state derivation
 * </ul>
 *
 * <p>Used by StorageSpaceManager to maintain the single source of truth
 * for all tote locations in the warehouse.
 */
public sealed interface ToteLocation {

    /** Tote stored in a rack at a specific slot. */
    record InRack(RackId rackId, int zIndex) implements ToteLocation {}

    /** Tote at a relay position waiting for pickup. */
    record AtRelay(String relayId) implements ToteLocation {}

    /** Tote at a station being processed by operator. */
    record AtStation(StationId stationId) implements ToteLocation {}

    /** Tote being carried by a robot. */
    record WithRobot(int robotId, RobotType robotType, TransportIntent intent)
            implements ToteLocation {}
}

/** Type of robot carrying a tote. */
public enum RobotType {
    VTM,
    HTM
}

/** Intent describing where a robot is taking a tote. */
public sealed interface TransportIntent {
    record ToRelay(String relayId) implements TransportIntent {}

    record ToStation(StationId stationId) implements TransportIntent {}

    record ReturningToRelay(String relayId) implements TransportIntent {}

    record ReturningToRack() implements TransportIntent {}
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=ToteLocationTest -q`
Expected: PASS (6 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/ToteLocation.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/ToteLocationTest.java
git commit -m "feat: add ToteLocation sealed types for location tracking"
```

---

## Task 2: Add Location Index to StorageSpaceManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManager.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManagerLocationTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import java.util.List;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class StorageSpaceManagerLocationTest {

    private StorageSpaceManager manager;
    private Rack rack;
    private Tote tote;

    @BeforeEach
    void setUp() {
        RackId rackId = new RackId(1);
        rack = new Rack(rackId, new GridCoordinate(5, 5), 4);
        tote = new Tote(new ToteId(100));

        manager = new StorageSpaceManager(List.of(rack), List.of(tote));
    }

    @Test
    void registerToteInRack_tracksLocation() {
        manager.registerToteInRack(tote.getId(), rack.getId(), 0);

        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.InRack.class);
        ToteLocation.InRack inRack = (ToteLocation.InRack) location;
        assertThat(inRack.rackId()).isEqualTo(rack.getId());
        assertThat(inRack.zIndex()).isEqualTo(0);
    }

    @Test
    void registerToteAtRelay_tracksLocation() {
        String relayId = "relay_block_0_L_r5";
        manager.registerToteAtRelay(tote.getId(), relayId);

        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.AtRelay.class);
        ToteLocation.AtRelay atRelay = (ToteLocation.AtRelay) location;
        assertThat(atRelay.relayId()).isEqualTo(relayId);
    }

    @Test
    void registerToteWithRobot_tracksLocation() {
        TransportIntent intent = new TransportIntent.ToRelay("relay_block_0_L_r3");
        manager.registerToteWithRobot(tote.getId(), 5, RobotType.VTM, intent);

        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.WithRobot.class);
        ToteLocation.WithRobot withRobot = (ToteLocation.WithRobot) location;
        assertThat(withRobot.robotId()).isEqualTo(5);
        assertThat(withRobot.robotType()).isEqualTo(RobotType.VTM);
    }

    @Test
    void getToteLocation_returnsNullForUnregistered() {
        ToteLocation location = manager.getToteLocation(new ToteId(999));
        assertThat(location).isNull();
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=StorageSpaceManagerLocationTest -q`
Expected: FAIL with "cannot find symbol: method registerToteInRack"

**Step 3: Write minimal implementation**

Add to `StorageSpaceManager.java`:

```java
// Add field after existing fields (around line 26)
// Note: HashMap is safe because Desmo-J simulation is single-threaded
private final Map<ToteId, ToteLocation> toteLocations = new HashMap<>();

// Add methods at end of class (before closing brace)

/**
 * Register tote location in a rack.
 *
 * @param toteId Tote identifier
 * @param rackId Rack containing the tote
 * @param zIndex Slot index within the rack
 */
public void registerToteInRack(ToteId toteId, RackId rackId, int zIndex) {
    ToteLocation newLocation = new ToteLocation.InRack(rackId, zIndex);
    ToteLocation previous = toteLocations.put(toteId, newLocation);
    if (previous != null) {
        log.debug("Tote {} moved from {} to InRack({}, {})",
            toteId, previous.getClass().getSimpleName(), rackId, zIndex);
    }
}

/**
 * Register tote location at a relay position.
 *
 * @param toteId Tote identifier
 * @param relayId Relay position identifier
 */
public void registerToteAtRelay(ToteId toteId, String relayId) {
    ToteLocation newLocation = new ToteLocation.AtRelay(relayId);
    ToteLocation previous = toteLocations.put(toteId, newLocation);
    if (previous != null) {
        log.debug("Tote {} moved from {} to AtRelay({})",
            toteId, previous.getClass().getSimpleName(), relayId);
    }
}

/**
 * Register tote location with a robot.
 *
 * @param toteId Tote identifier
 * @param robotId Robot carrying the tote
 * @param robotType Type of robot (VTM or HTM)
 * @param intent Where the robot is taking the tote
 */
public void registerToteWithRobot(ToteId toteId, int robotId, RobotType robotType, TransportIntent intent) {
    ToteLocation newLocation = new ToteLocation.WithRobot(robotId, robotType, intent);
    ToteLocation previous = toteLocations.put(toteId, newLocation);
    if (previous != null) {
        log.debug("Tote {} moved from {} to WithRobot({}, {})",
            toteId, previous.getClass().getSimpleName(), robotId, robotType);
    }
}

/**
 * Get current location of a tote.
 *
 * @param toteId Tote identifier
 * @return Location or null if not registered
 */
public ToteLocation getToteLocation(ToteId toteId) {
    return toteLocations.get(toteId);
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=StorageSpaceManagerLocationTest -q`
Expected: PASS (4 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManager.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManagerLocationTest.java
git commit -m "feat: add tote location tracking to StorageSpaceManager"
```

---

## Task 3: Add State Derivation to StorageSpaceManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManager.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManagerLocationTest.java`

**Step 1: Write the failing test**

Add to `StorageSpaceManagerLocationTest.java`:

```java
@Test
void deriveToteState_inRack_returnsInRack() {
    manager.registerToteInRack(tote.getId(), rack.getId(), 0);

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.IN_RACK);
}

@Test
void deriveToteState_atRelay_returnsAtRelayPosition() {
    manager.registerToteAtRelay(tote.getId(), "relay_block_0_L_r5");

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.AT_RELAY_POSITION);
}

@Test
void deriveToteState_vtmToRelay_returnsBeingTransportedToRelay() {
    TransportIntent intent = new TransportIntent.ToRelay("relay_block_0_L_r3");
    manager.registerToteWithRobot(tote.getId(), 1, RobotType.VTM, intent);

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.BEING_TRANSPORTED_TO_RELAY);
}

@Test
void deriveToteState_htmToStation_returnsBeingTransportedToStation() {
    TransportIntent intent = new TransportIntent.ToStation(new StationId(2));
    manager.registerToteWithRobot(tote.getId(), 3, RobotType.HTM, intent);

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.BEING_TRANSPORTED_TO_STATION);
}

@Test
void deriveToteState_htmReturningToRelay_returnsBeingReturnedToRelay() {
    TransportIntent intent = new TransportIntent.ReturningToRelay("relay_block_0_L_r5");
    manager.registerToteWithRobot(tote.getId(), 3, RobotType.HTM, intent);

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.BEING_RETURNED_TO_RELAY);
}

@Test
void deriveToteState_vtmReturningToRack_returnsBeingReturnedToRack() {
    TransportIntent intent = new TransportIntent.ReturningToRack();
    manager.registerToteWithRobot(tote.getId(), 2, RobotType.VTM, intent);

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.BEING_RETURNED_TO_RACK);
}

@Test
void deriveToteState_atStation_returnsAtStation() {
    manager.registerToteAtStation(tote.getId(), new StationId(3));

    Tote.ToteState state = manager.deriveToteState(tote.getId());
    assertThat(state).isEqualTo(Tote.ToteState.AT_STATION);
}

@Test
void deriveToteState_unregistered_returnsInRack() {
    Tote.ToteState state = manager.deriveToteState(new ToteId(999));
    assertThat(state).isEqualTo(Tote.ToteState.IN_RACK);
}
```

Also add import: `import com.greyorange.dessimulator.domain.common.types.StationId;`

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=StorageSpaceManagerLocationTest -q`
Expected: FAIL with "cannot find symbol: method deriveToteState"

**Step 3: Write minimal implementation**

Add to `StorageSpaceManager.java`:

```java
/**
 * Register tote location at a station.
 *
 * @param toteId Tote identifier
 * @param stationId Station where tote is being processed
 */
public void registerToteAtStation(ToteId toteId, StationId stationId) {
    ToteLocation newLocation = new ToteLocation.AtStation(stationId);
    ToteLocation previous = toteLocations.put(toteId, newLocation);
    if (previous != null) {
        log.debug("Tote {} moved from {} to AtStation({})",
            toteId, previous.getClass().getSimpleName(), stationId);
    }
}

/**
 * Derive tote state from its current location.
 *
 * @param toteId Tote identifier
 * @return Derived state based on location
 */
public Tote.ToteState deriveToteState(ToteId toteId) {
    ToteLocation loc = toteLocations.get(toteId);
    if (loc == null) {
        return Tote.ToteState.IN_RACK; // Default for untracked
    }
    return switch (loc) {
        case ToteLocation.InRack _ -> Tote.ToteState.IN_RACK;
        case ToteLocation.AtRelay _ -> Tote.ToteState.AT_RELAY_POSITION;
        case ToteLocation.AtStation _ -> Tote.ToteState.AT_STATION;
        case ToteLocation.WithRobot(_, RobotType.VTM, TransportIntent.ToRelay _) ->
                Tote.ToteState.BEING_TRANSPORTED_TO_RELAY;
        case ToteLocation.WithRobot(_, RobotType.HTM, TransportIntent.ToStation _) ->
                Tote.ToteState.BEING_TRANSPORTED_TO_STATION;
        case ToteLocation.WithRobot(_, RobotType.HTM, TransportIntent.ReturningToRelay _) ->
                Tote.ToteState.BEING_RETURNED_TO_RELAY;
        case ToteLocation.WithRobot(_, RobotType.VTM, TransportIntent.ReturningToRack _) ->
                Tote.ToteState.BEING_RETURNED_TO_RACK;
        case ToteLocation.WithRobot _ -> Tote.ToteState.BEING_TRANSPORTED_TO_RELAY; // Fallback
    };
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=StorageSpaceManagerLocationTest -q`
Expected: PASS (12 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManager.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManagerLocationTest.java
git commit -m "feat: add deriveToteState to StorageSpaceManager"
```

---

## Task 4: Add Internal Methods to Rack

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Rack.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/RackInternalMethodsTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class RackInternalMethodsTest {

    private Rack rack;
    private Tote tote;

    @BeforeEach
    void setUp() {
        rack = new Rack(new RackId(1), new GridCoordinate(5, 5), 4);
        tote = new Tote(new ToteId(100));
    }

    @Test
    void addToteInternal_addsToteWithoutStateChange() {
        rack.addToteInternal(tote, 0);

        assertThat(rack.getTotes()).containsKey(0);
        assertThat(rack.getTotes().get(0)).isEqualTo(tote);
    }

    @Test
    void removeToteInternal_removesToteWithoutStateChange() {
        rack.addToteInternal(tote, 0);

        Tote removed = rack.removeToteInternal(0);

        assertThat(removed).isEqualTo(tote);
        assertThat(rack.getTotes()).doesNotContainKey(0);
    }

    @Test
    void addToteInternal_autoAssignsZIndexWhenNull() {
        int zIndex = rack.addToteInternal(tote, null);

        assertThat(zIndex).isEqualTo(0);
        assertThat(rack.getTotes().get(0)).isEqualTo(tote);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=RackInternalMethodsTest -q`
Expected: FAIL with "cannot find symbol: method addToteInternal"

**Step 3: Write minimal implementation**

Add to `Rack.java` (after existing `removeTote` method around line 142):

```java
/**
 * Internal method to add tote without state changes.
 * Used by StorageSpaceManager orchestration.
 *
 * @param tote Tote to add
 * @param zIndex Slot index (null for auto-assign)
 * @return Actual z-index used
 */
int addToteInternal(Tote tote, Integer zIndex) {
    if (!hasEmptyToteSlots()) {
        throw new IllegalStateException("Rack %s is full".formatted(id));
    }

    int actualZIndex = zIndex != null ? zIndex : findEmptyZIndex();

    if (totes.containsKey(actualZIndex)) {
        throw new IllegalArgumentException("Z-index %d already occupied".formatted(actualZIndex));
    }

    totes.put(actualZIndex, tote);
    return actualZIndex;
}

/**
 * Internal method to remove tote without state changes.
 * Used by StorageSpaceManager orchestration.
 *
 * @param zIndex Slot index to remove from
 * @return Removed tote
 */
Tote removeToteInternal(int zIndex) {
    var tote = totes.remove(zIndex);
    if (tote == null) {
        throw new java.util.NoSuchElementException("No tote at z-index %d".formatted(zIndex));
    }
    return tote;
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RackInternalMethodsTest -q`
Expected: PASS (3 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Rack.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/RackInternalMethodsTest.java
git commit -m "feat: add internal methods to Rack for orchestration"
```

---

## Task 5: Add Internal Methods to RelayPosition

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionInternalMethodsTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.robot;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.inventory.Tote;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class RelayPositionInternalMethodsTest {

    private RelayPosition relay;
    private Tote tote;

    @BeforeEach
    void setUp() {
        relay = new RelayPosition(
                "relay_test", new GridCoordinate(10, 10), "block_0", "L", "zone_0");
        tote = new Tote(new ToteId(100));
    }

    @Test
    void setToteInternal_setsToteWithoutStateChange() {
        relay.setToteInternal(tote);

        assertThat(relay.peek()).isPresent();
        assertThat(relay.peek().get()).isEqualTo(tote);
    }

    @Test
    void clearToteInternal_clearsToteWithoutStateChange() {
        relay.setToteInternal(tote);

        relay.clearToteInternal();

        assertThat(relay.peek()).isEmpty();
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionInternalMethodsTest -q`
Expected: FAIL with "cannot find symbol: method setToteInternal"

**Step 3: Write minimal implementation**

Add to `RelayPosition.java` (after `hasToteWithTasks` method around line 140):

```java
/**
 * Internal method to set tote without state changes.
 * Used by StorageSpaceManager orchestration.
 * Does NOT acquire Res resource - caller must handle blocking.
 *
 * @param tote Tote to set
 */
void setToteInternal(Tote tote) {
    this.tote = Optional.of(tote);
}

/**
 * Internal method to clear tote without state changes.
 * Used by StorageSpaceManager orchestration.
 * Does NOT release Res resource - caller must handle.
 *
 * @return The cleared tote if present
 */
Optional<Tote> clearToteInternal() {
    Optional<Tote> removed = this.tote;
    this.tote = Optional.empty();
    return removed;
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionInternalMethodsTest -q`
Expected: PASS (2 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionInternalMethodsTest.java
git commit -m "feat: add internal methods to RelayPosition for orchestration"
```

---

## Task 6: Add Orchestration Move Methods to StorageSpaceManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManager.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManagerMoveTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.robot.RelayPosition;
import java.util.List;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class StorageSpaceManagerMoveTest {

    private StorageSpaceManager manager;
    private Rack rack;
    private Tote tote;
    private RelayPosition relay;

    @BeforeEach
    void setUp() {
        RackId rackId = new RackId(1);
        rack = new Rack(rackId, new GridCoordinate(5, 5), 4);
        tote = new Tote(new ToteId(100));
        relay = new RelayPosition(
                "relay_block_0_L_r5", new GridCoordinate(10, 10), "block_0", "L", "zone_0");

        manager = new StorageSpaceManager(List.of(rack), List.of(tote));
    }

    @Test
    void moveToteFromRackToRobot_updatesLocationAndRack() {
        // Setup: tote in rack
        rack.addToteInternal(tote, 0);
        manager.registerToteInRack(tote.getId(), rack.getId(), 0);

        // Move to robot
        TransportIntent intent = new TransportIntent.ToRelay("relay_block_0_L_r5");
        manager.moveToteFromRackToRobot(tote.getId(), rack.getId(), 0, 1, RobotType.VTM, intent);

        // Verify location updated
        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.WithRobot.class);

        // Verify rack no longer has tote
        assertThat(rack.getTotes()).doesNotContainKey(0);
    }

    @Test
    void moveToteFromRobotToRelay_updatesLocationAndRelay() {
        // Setup: tote with robot
        TransportIntent intent = new TransportIntent.ToRelay("relay_block_0_L_r5");
        manager.registerToteWithRobot(tote.getId(), 1, RobotType.VTM, intent);

        // Move to relay
        manager.moveToteFromRobotToRelay(tote.getId(), relay.getId(), relay);

        // Verify location updated
        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.AtRelay.class);

        // Verify relay has tote
        assertThat(relay.peek()).isPresent();
        assertThat(relay.peek().get()).isEqualTo(tote);
    }

    @Test
    void moveToteFromRelayToRobot_updatesLocationAndRelay() {
        // Setup: tote at relay
        relay.setToteInternal(tote);
        manager.registerToteAtRelay(tote.getId(), relay.getId());

        // Move to robot
        TransportIntent intent = new TransportIntent.ToStation(new StationId(2));
        manager.moveToteFromRelayToRobot(tote.getId(), relay.getId(), relay, 3, RobotType.HTM, intent);

        // Verify location updated
        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.WithRobot.class);

        // Verify relay no longer has tote
        assertThat(relay.peek()).isEmpty();
    }

    @Test
    void moveToteFromRobotToRack_updatesLocationAndRack() {
        // Setup: tote with robot
        TransportIntent intent = new TransportIntent.ReturningToRack();
        manager.registerToteWithRobot(tote.getId(), 2, RobotType.VTM, intent);

        // Move to rack
        manager.moveToteFromRobotToRack(tote.getId(), rack.getId(), 1);

        // Verify location updated
        ToteLocation location = manager.getToteLocation(tote.getId());
        assertThat(location).isInstanceOf(ToteLocation.InRack.class);

        // Verify rack has tote
        assertThat(rack.getTotes()).containsKey(1);
        assertThat(rack.getTotes().get(1)).isEqualTo(tote);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=StorageSpaceManagerMoveTest -q`
Expected: FAIL with "cannot find symbol: method moveToteFromRackToRobot"

**Step 3: Write minimal implementation**

Add to `StorageSpaceManager.java`:

```java
// Add import
import com.greyorange.dessimulator.domain.robot.RelayPosition;

// Add orchestration methods

/**
 * Move tote from rack to robot.
 * Updates location index and removes from rack.
 */
public void moveToteFromRackToRobot(
        ToteId toteId, RackId rackId, int zIndex,
        int robotId, RobotType robotType, TransportIntent intent) {
    Rack rack = getRack(rackId);
    rack.removeToteInternal(zIndex);
    toteLocations.put(toteId, new ToteLocation.WithRobot(robotId, robotType, intent));
}

/**
 * Move tote from robot to relay.
 * Updates location index and adds to relay.
 */
public void moveToteFromRobotToRelay(ToteId toteId, String relayId, RelayPosition relay) {
    Tote tote = getTote(toteId);
    relay.setToteInternal(tote);
    toteLocations.put(toteId, new ToteLocation.AtRelay(relayId));
}

/**
 * Move tote from relay to robot.
 * Updates location index and removes from relay.
 */
public void moveToteFromRelayToRobot(
        ToteId toteId, String relayId, RelayPosition relay,
        int robotId, RobotType robotType, TransportIntent intent) {
    relay.clearToteInternal();
    toteLocations.put(toteId, new ToteLocation.WithRobot(robotId, robotType, intent));
}

/**
 * Move tote from robot to rack.
 * Updates location index and adds to rack.
 */
public void moveToteFromRobotToRack(ToteId toteId, RackId rackId, int zIndex) {
    Rack rack = getRack(rackId);
    Tote tote = getTote(toteId);
    rack.addToteInternal(tote, zIndex);
    toteLocations.put(toteId, new ToteLocation.InRack(rackId, zIndex));
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=StorageSpaceManagerMoveTest -q`
Expected: PASS (4 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManager.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/StorageSpaceManagerMoveTest.java
git commit -m "feat: add orchestration move methods to StorageSpaceManager"
```

---

## Task 7: Simplify Tote Constructor

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/ToteTest.java`

**Step 1: Update test to use simplified constructor**

Replace `ToteTest.java` content:

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.robot.PickTask;
import org.junit.jupiter.api.Test;

class ToteTest {

    @Test
    void testToteCreation() {
        ToteId toteId = new ToteId(1);
        Tote tote = new Tote(toteId);

        assertEquals(toteId, tote.getId());
        assertEquals(6, tote.getMaxCapacity());
        assertTrue(tote.getTasks().isEmpty());
        assertNull(tote.getAssignedVtmId());
        assertNull(tote.getAssignedHtmId());
    }

    @Test
    void testAddTask() {
        ToteId toteId = new ToteId(1);
        Tote tote = new Tote(toteId);

        Item item = Item.create("sku_1", "Test Item", "test", 1.0, 100, 10);
        PickTask task = new PickTask("task_1", "order_1", item, 5, toteId, 0.0);
        task.setStationId(new StationId(1));

        tote.addTask(task);

        assertEquals(1, tote.getTasks().size());
        assertTrue(tote.hasTasks());
    }

    @Test
    void testTasksByStation() {
        ToteId toteId = new ToteId(1);
        Tote tote = new Tote(toteId);

        Item item1 = Item.create("sku_1", "Test Item 1", "test", 1.0, 100, 10);
        Item item2 = Item.create("sku_2", "Test Item 2", "test", 1.0, 100, 10);

        PickTask task1 = new PickTask("task_1", "order_1", item1, 5, toteId, 0.0);
        task1.setStationId(new StationId(1));
        PickTask task2 = new PickTask("task_2", "order_1", item2, 3, toteId, 0.0);
        task2.setStationId(new StationId(2));

        tote.addTask(task1);
        tote.addTask(task2);

        var tasksByStation = tote.getTasksByStation();
        assertEquals(2, tasksByStation.size());
        assertEquals(1, tasksByStation.get(new StationId(1)).size());
        assertEquals(1, tasksByStation.get(new StationId(2)).size());
    }

    @Test
    void testClaims() {
        Tote tote = new Tote(new ToteId(1));

        assertNull(tote.getAssignedVtmId());
        assertNull(tote.getAssignedHtmId());

        tote.setAssignedVtmId(5);
        tote.setAssignedHtmId(3);

        assertEquals(5, tote.getAssignedVtmId());
        assertEquals(3, tote.getAssignedHtmId());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=ToteTest -q`
Expected: FAIL with "constructor Tote(ToteId) not found" or similar

**Step 3: Update Tote class**

First, remove class-level `@Setter` annotation from `Tote.java` (around line 17):

```java
// BEFORE:
@Slf4j
@Getter
@Setter  // REMOVE THIS LINE
public final class Tote implements WarehouseInventory.StorageSpace {

// AFTER:
@Slf4j
@Getter
public final class Tote implements WarehouseInventory.StorageSpace {
```

Then add field-level `@Setter` only on claim fields (around lines 27-28):

```java
    @Setter private Integer assignedVtmId;
    @Setter private Integer assignedHtmId;
```

Then add simplified constructor after existing constructor (around line 41):

```java
/**
 * Simplified constructor for new location-based architecture.
 * Location tracking is handled by StorageSpaceManager.
 *
 * <p>NOTE: Legacy fields (state, currentHoldingEntity, etc.) are initialized
 * for backwards compatibility during migration. They will be removed in Phase 2.
 *
 * @param id Tote identifier
 */
public Tote(ToteId id) {
    this.id = id;
    this.state = ToteState.IN_RACK;
    this.currentHoldingEntity = null;
    this.currentCoordinate = null;
    this.createdTime = 0.0;
    this.completionTime = null;
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=ToteTest -q`
Expected: PASS (4 tests)

**Step 5: Commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/ToteTest.java
git commit -m "feat: add simplified Tote constructor for location-based architecture"
```

---

## Task 8: Run Full Test Suite

**Step 1: Run all tests**

Run: `cd apps/backend && mvn test -q`
Expected: All tests pass

**Step 2: If failures occur**

Fix any failing tests that depend on old Tote constructor or state fields. Common fixes:
- Update `new Tote(toteId, rackId, coord)` to `new Tote(toteId)`
- Update state checks to use StorageSpaceManager where needed

**Step 3: Commit fixes if needed**

```bash
cd apps/backend && mvn spotless:apply
git add -A
git commit -m "fix: update tests for simplified Tote"
```

---

## Task 9: Integration Test with Simulation

**Step 1: Run integration test**

Run: `cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q`
Expected: PASS

**Step 2: If failures occur**

This indicates VTM/HTM processes need updating to use StorageSpaceManager orchestration. This is Phase 2 of the migration (not covered in this plan - do after core infrastructure is stable).

---

## Summary

This plan establishes the core infrastructure for tote location tracking:

1. **ToteLocation sealed types** - Type-safe location representation
2. **StorageSpaceManager location index** - Central tracking of all tote locations
3. **State derivation** - Derive ToteState from location
4. **Internal container methods** - Rack and RelayPosition methods for orchestration
5. **Orchestration move methods** - Single point for all tote movements
6. **Simplified Tote** - Constructor without location params

**Phase 2 (separate plan):**
- Update DesmojVtmProcess to use orchestration
- Update DesmojHtmProcess to use orchestration
- Remove deprecated fields from Tote
- Update WarehouseBuilder tote initialization
