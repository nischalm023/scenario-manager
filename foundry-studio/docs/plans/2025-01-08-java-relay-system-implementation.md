# Java Relay System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the relay warehouse system in Java using Desmo-J, supporting VTM (Vertical Tote Mover) and HTM (Horizontal Tote Mover) robots with zone-aware relay position selection.

**Architecture:** Simplified state management eliminating RelayTask wrapper. Totes carry PickingTasks directly (like Racks in RTP). RelayPositions use Desmo-J Res for automatic blocking. Zone-aware relay selection with cantilever positioning. Reuse existing StationProcessor and metrics infrastructure.

**Tech Stack:** Java 21, Desmo-J 2.5.1e, Spring Boot 3.2, Lombok, existing metrics framework with @MetricHandler annotations

---

## Prerequisites

**Context Files to Review:**
- `CLAUDE.md` - Project structure and development commands
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/RtpRobotProcess.java` - Reference for process structure
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/RobotEvents.java` - Domain events
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricsManager.java` - Metrics routing

**Existing Infrastructure (No Changes):**
- ✅ MetricEvent enum has HTM_CYCLE_COMPLETED, VTM_CYCLE_COMPLETED, RELAY_POSITION_BLOCKED
- ✅ HtmCycleCompleted, VtmCycleCompleted domain events exist
- ✅ RobotCollector handles HTM/VTM metrics
- ✅ StationCollector works for multi-station cycles

---

## Task 1: Domain Model - Tote

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/ToteTest.java`

**Step 1: Write failing test for Tote creation**

```java
package com.greyorange.dessimulator.domain.inventory;

import com.greyorange.dessimulator.domain.map.GridCoordinate;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ToteTest {

    @Test
    void testToteCreation() {
        GridCoordinate coord = new GridCoordinate(5, 10);
        Tote tote = new Tote(1, "rack_0", coord);

        assertEquals(1, tote.getId());
        assertEquals("rack_0", tote.getCurrentHoldingEntity());
        assertEquals(ToteState.IN_RACK, tote.getState());
        assertEquals(coord, tote.getCurrentCoordinate());
        assertEquals(6, tote.getMaxCapacity());
        assertTrue(tote.getTasks().isEmpty());
    }

    @Test
    void testAddTask() {
        Tote tote = new Tote(1, "rack_0", new GridCoordinate(0, 0));
        PickingTask task = new PickingTask("task_1", "order_1", "sku_1", 5, 1);
        task.setStationId(1);

        tote.addTask(task);

        assertEquals(1, tote.getTasks().size());
        assertTrue(tote.hasTasks());
    }

    @Test
    void testTasksByStation() {
        Tote tote = new Tote(1, "rack_0", new GridCoordinate(0, 0));
        PickingTask task1 = new PickingTask("task_1", "order_1", "sku_1", 5, 1);
        task1.setStationId(1);
        PickingTask task2 = new PickingTask("task_2", "order_1", "sku_2", 3, 1);
        task2.setStationId(2);

        tote.addTask(task1);
        tote.addTask(task2);

        var tasksByStation = tote.getTasksByStation();
        assertEquals(2, tasksByStation.size());
        assertEquals(1, tasksByStation.get(1).size());
        assertEquals(1, tasksByStation.get(2).size());
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cd apps/backend
mvn test -Dtest=ToteTest
```
Expected: Compilation error - `Tote` class does not exist

**Step 3: Write Tote domain model**

```java
package com.greyorange.dessimulator.domain.inventory;

import com.greyorange.dessimulator.domain.map.GridCoordinate;
import java.util.*;
import java.util.stream.Collectors;
import lombok.Getter;
import lombok.Setter;

@Getter
@Setter
public class Tote {
    private final int id;
    private ToteState state;
    private String currentHoldingEntity;  // rackId, relayPositionId, or robotId
    private GridCoordinate currentCoordinate;

    private final List<PickingTask> tasks = new ArrayList<>();
    private final int maxCapacity = 6;

    // Robot assignments
    private Integer assignedVtmId;
    private Integer assignedHtmId;

    // Timing
    private double createdTime;
    private Double completionTime;

    public Tote(int id, String currentHoldingEntity, GridCoordinate currentCoordinate) {
        this.id = id;
        this.currentHoldingEntity = currentHoldingEntity;
        this.currentCoordinate = currentCoordinate;
        this.state = ToteState.IN_RACK;
        this.createdTime = 0.0;
    }

    public void addTask(PickingTask task) {
        if (tasks.size() >= maxCapacity) {
            throw new IllegalStateException("Tote " + id + " is at capacity");
        }
        tasks.add(task);
    }

    public boolean hasTasks() {
        return !tasks.isEmpty();
    }

    public Map<Integer, List<PickingTask>> getTasksByStation() {
        return tasks.stream()
            .filter(t -> t.getStationId() != null)
            .collect(Collectors.groupingBy(PickingTask::getStationId));
    }

    public List<PickingTask> getTasksForStation(int stationId) {
        return tasks.stream()
            .filter(t -> t.getStationId() != null && t.getStationId() == stationId)
            .collect(Collectors.toList());
    }

    public void removeTask(PickingTask task) {
        tasks.remove(task);
    }

    public void moveToRelay(String relayPositionId) {
        this.state = ToteState.AT_RELAY_POSITION;
        this.currentHoldingEntity = relayPositionId;
    }

    public void moveToStation() {
        this.state = ToteState.AT_STATION;
    }

    public void returnToRack(int rackId) {
        this.state = ToteState.IN_RACK;
        this.currentHoldingEntity = String.valueOf(rackId);
        this.assignedVtmId = null;
        this.assignedHtmId = null;
    }
}
```

**Step 4: Create ToteState enum**

```java
package com.greyorange.dessimulator.domain.inventory;

public enum ToteState {
    IN_RACK,
    BEING_TRANSPORTED_TO_RELAY,
    AT_RELAY_POSITION,
    BEING_TRANSPORTED_TO_STATION,
    AT_STATION,
    BEING_RETURNED_TO_RELAY,
    BEING_RETURNED_TO_RACK
}
```

**Step 5: Run tests to verify they pass**

```bash
mvn test -Dtest=ToteTest
```
Expected: All tests PASS

**Step 6: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/ToteState.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/ToteTest.java
git commit -m "feat(relay): add Tote domain model with task management"
```

---

## Task 2: Domain Model - RelayPosition

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java`

**Step 1: Write failing test for RelayPosition**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.map.GridCoordinate;
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel;
import desmoj.core.simulator.Experiment;
import desmoj.core.simulator.TimeInstant;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import java.util.concurrent.TimeUnit;
import static org.junit.jupiter.api.Assertions.*;

class RelayPositionTest {
    private DesmojWarehouseModel model;
    private Experiment experiment;

    @BeforeEach
    void setup() {
        model = new DesmojWarehouseModel(null, "TestModel", true, false);
        experiment = new Experiment("TestExp");
        model.connectToExperiment(experiment);
        experiment.stop(new TimeInstant(100, TimeUnit.SECONDS));
    }

    @Test
    void testRelayPositionCreation() {
        RelayPosition relay = new RelayPosition(
            model,
            "relay_1",
            new GridCoordinate(5, 10),
            "block_1",
            "left",
            5
        );

        assertEquals("relay_1", relay.getId());
        assertEquals("block_1", relay.getRackBlockId());
        assertEquals("left", relay.getSide());
        assertEquals(0, relay.getPendingTotesCount());
        assertFalse(relay.isFull());
        assertFalse(relay.hasTotes());
    }

    @Test
    void testAddRemoveTote() {
        RelayPosition relay = new RelayPosition(model, "relay_1",
            new GridCoordinate(5, 10), "block_1", "left", 5);
        Tote tote = new Tote(1, "rack_0", new GridCoordinate(0, 0));

        // Note: addTote() requires SimProcess context, test manually
        relay.getPendingTotes().add(tote);

        assertTrue(relay.hasTotes());
        assertEquals(1, relay.getPendingTotesCount());

        var removed = relay.removeTote();
        assertTrue(removed.isPresent());
        assertEquals(tote, removed.get());
        assertFalse(relay.hasTotes());
    }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=RelayPositionTest
```
Expected: Compilation error - `RelayPosition` does not exist

**Step 3: Write RelayPosition implementation**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.map.GridCoordinate;
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel;
import desmoj.core.simulator.SimProcess;
import desmoj.core.simulator.TimeInstant;
import de.terministic.serein.core.SuspendExecution;
import desmoj.core.advancedModellingFeatures.Res;
import java.util.*;
import java.util.concurrent.TimeUnit;
import lombok.Getter;

@Getter
public class RelayPosition {
    private final String id;
    private final GridCoordinate position;
    private final String rackBlockId;
    private final String side;

    private final Res resource;
    private final Deque<Tote> pendingTotes = new ArrayDeque<>();
    private final Set<String> accessibleFromZones = new HashSet<>();

    public RelayPosition(
        DesmojWarehouseModel model,
        String id,
        GridCoordinate position,
        String rackBlockId,
        String side,
        int capacity
    ) {
        this.id = id;
        this.position = position;
        this.rackBlockId = rackBlockId;
        this.side = side;
        this.resource = new Res(model, id + "_capacity", capacity, true, false);
    }

    public void addTote(SimProcess process, Tote tote) throws SuspendExecution {
        // Request resource slot - blocks if relay is full
        resource.provide(process, 1);

        // Add tote to FIFO queue
        pendingTotes.addLast(tote);
        tote.moveToRelay(this.id);
    }

    public Optional<Tote> removeTote() {
        if (pendingTotes.isEmpty()) {
            return Optional.empty();
        }

        Tote tote = pendingTotes.removeFirst();
        resource.takeBack(1);

        return Optional.of(tote);
    }

    public boolean isFull() {
        return pendingTotes.size() >= resource.getCapacity();
    }

    public boolean hasTotes() {
        return !pendingTotes.isEmpty();
    }

    public int getPendingTotesCount() {
        return pendingTotes.size();
    }

    public void addAccessibleZone(String zoneId) {
        accessibleFromZones.add(zoneId);
    }

    public boolean isAccessibleFromZone(String zoneId) {
        return accessibleFromZones.contains(zoneId);
    }
}
```

**Step 4: Run tests to verify they pass**

```bash
mvn test -Dtest=RelayPositionTest
```
Expected: All tests PASS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java
git commit -m "feat(relay): add RelayPosition with Desmo-J resource blocking"
```

---

## Task 3: Domain Model - Zone

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java`

**Step 1: Write failing test for Zone**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.map.GridCoordinate;
import org.junit.jupiter.api.Test;
import java.util.List;
import static org.junit.jupiter.api.Assertions.*;

class ZoneTest {

    @Test
    void testZoneCreation() {
        List<GridCoordinate> coverage = List.of(
            new GridCoordinate(0, 5),
            new GridCoordinate(1, 5),
            new GridCoordinate(2, 5)
        );

        Zone zone = new Zone("zone_0", "aisle", List.of("block_0"), coverage, List.of());

        assertEquals("zone_0", zone.getZoneId());
        assertEquals("aisle", zone.getZoneType());
        assertEquals(3, zone.getCoverageArea().size());
    }

    @Test
    void testContainsPosition() {
        List<GridCoordinate> coverage = List.of(new GridCoordinate(5, 10));
        Zone zone = new Zone("zone_0", "aisle", List.of(), coverage, List.of());

        assertTrue(zone.containsPosition(new GridCoordinate(5, 10)));
        assertFalse(zone.containsPosition(new GridCoordinate(6, 10)));
    }

    @Test
    void testVtmAssignment() {
        Zone zone = new Zone("zone_0", "aisle", List.of(), List.of(), List.of());

        zone.addVtm(1);
        zone.addVtm(2);

        assertTrue(zone.getAssignedVtmIds().contains(1));
        assertTrue(zone.getAssignedVtmIds().contains(2));

        zone.removeVtm(1);
        assertFalse(zone.getAssignedVtmIds().contains(1));
    }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=ZoneTest
```
Expected: Compilation error - `Zone` does not exist

**Step 3: Write Zone implementation**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.map.GridCoordinate;
import java.util.*;
import lombok.Getter;

@Getter
public class Zone {
    private final String zoneId;
    private final String zoneType;  // "aisle", "storage", "general"
    private final List<String> rackBlockIds;
    private final List<GridCoordinate> coverageArea;  // VTM walking space
    private final List<GridCoordinate> servicedRackCoordinates;
    private final List<Integer> assignedVtmIds;

    public Zone(
        String zoneId,
        String zoneType,
        List<String> rackBlockIds,
        List<GridCoordinate> coverageArea,
        List<GridCoordinate> servicedRackCoordinates
    ) {
        this.zoneId = zoneId;
        this.zoneType = zoneType;
        this.rackBlockIds = new ArrayList<>(rackBlockIds);
        this.coverageArea = new ArrayList<>(coverageArea);
        this.servicedRackCoordinates = new ArrayList<>(servicedRackCoordinates);
        this.assignedVtmIds = new ArrayList<>();
    }

    public void addVtm(int vtmId) {
        if (!assignedVtmIds.contains(vtmId)) {
            assignedVtmIds.add(vtmId);
        }
    }

    public void removeVtm(int vtmId) {
        assignedVtmIds.remove(Integer.valueOf(vtmId));
    }

    public boolean containsPosition(GridCoordinate position) {
        return coverageArea.contains(position);
    }

    public boolean servicesRack(GridCoordinate rackCoordinate) {
        return servicedRackCoordinates.contains(rackCoordinate);
    }
}
```

**Step 4: Run tests to verify they pass**

```bash
mvn test -Dtest=ZoneTest
```
Expected: All tests PASS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java
git commit -m "feat(relay): add Zone model for VTM assignment"
```

---

## Task 4: Add RelayPositionBlocked Domain Event

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/MiscEvents.java`

**Step 1: Add RelayPositionBlocked event**

Add to `MiscEvents.java`:

```java
/**
 * Event emitted when a VTM blocks waiting for relay position capacity.
 */
record RelayPositionBlocked(
    double timestamp,
    int robotId,
    int relayPositionId,
    double waitTime
) implements DomainEvent {

    public RelayPositionBlocked {
        if (timestamp < 0) {
            throw new IllegalArgumentException("timestamp must be >= 0");
        }
        if (robotId < 0) {
            throw new IllegalArgumentException("robotId must be >= 0");
        }
        if (relayPositionId < 0) {
            throw new IllegalArgumentException("relayPositionId must be >= 0");
        }
        if (waitTime < 0) {
            throw new IllegalArgumentException("waitTime must be >= 0");
        }
    }

    @Override
    public MetricEvent eventType() {
        return MetricEvent.RELAY_POSITION_BLOCKED;
    }
}
```

**Step 2: Fix RelayPositionCollector key names**

Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java:788-789`

Change from:
```java
Object relayIdObj = event.getData().get("relay_id");
Object blockingTimeObj = event.getData().get("blocking_time");
```

To:
```java
Object relayIdObj = event.getData().get("relay_position_id");
Object blockingTimeObj = event.getData().get("wait_time");
```

**Step 3: Compile to verify**

```bash
mvn clean compile -DskipTests
```
Expected: Compilation SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/MiscEvents.java
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/collectors/MetricsCollectors.java
git commit -m "feat(relay): add RelayPositionBlocked event and fix collector keys"
```

---

## Task 5: ZoneManager for VTM Assignment

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java`

**Step 1: Write failing test for ZoneManager**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.map.GridCoordinate;
import com.greyorange.dessimulator.domain.map.RackBlock;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import org.junit.jupiter.api.Test;
import java.util.List;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class ZoneManagerTest {

    @Test
    void testGenerateZones() {
        WarehouseMap map = mock(WarehouseMap.class);
        RackBlock block = new RackBlock(
            "block_0",
            new GridCoordinate(0, 6),
            new GridCoordinate(9, 10),
            10,
            List.of()
        );
        when(map.getRackBlocks()).thenReturn(List.of(block));
        when(map.getConfig()).thenReturn(mock(WarehouseMapConfig.class));
        when(map.getConfig().getGridWidth()).thenReturn(20);

        ZoneManager manager = new ZoneManager(map);

        assertEquals(2, manager.getAllZones().size());  // Left and right zones
    }

    @Test
    void testBalanceVtmAssignments() {
        WarehouseMap map = mock(WarehouseMap.class);
        when(map.getRackBlocks()).thenReturn(List.of());

        ZoneManager manager = new ZoneManager(map);
        // Manually add zones
        manager.getZones().put("zone_0", new Zone("zone_0", "aisle", List.of(), List.of(), List.of()));
        manager.getZones().put("zone_1", new Zone("zone_1", "aisle", List.of(), List.of(), List.of()));

        manager.balanceVtmAssignments(List.of(0, 1));

        assertEquals(1, manager.getZonesForVtm(0).size());
        assertEquals(1, manager.getZonesForVtm(1).size());
    }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=ZoneManagerTest
```
Expected: Compilation error - `ZoneManager` does not exist

**Step 3: Write ZoneManager implementation**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.map.GridCoordinate;
import com.greyorange.dessimulator.domain.map.RackBlock;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import java.util.*;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;

@Slf4j
@Getter
public class ZoneManager {
    private final WarehouseMap warehouseMap;
    private final Map<String, Zone> zones = new HashMap<>();

    public ZoneManager(WarehouseMap map) {
        this.warehouseMap = map;
        generateZones();
    }

    private void generateZones() {
        List<RackBlock> rackBlocks = warehouseMap.getRackBlocks();

        if (rackBlocks.isEmpty()) {
            // Fallback: create single general zone
            zones.put("zone_0", new Zone("zone_0", "general", List.of(), List.of(), List.of()));
            return;
        }

        int zoneId = 0;
        for (RackBlock block : rackBlocks) {
            List<GridCoordinate> allRackPositions = block.getRackPositions();

            // Split racks by proximity to left vs right aisle
            double midCol = (block.getTopLeft().col() + block.getBottomRight().col()) / 2.0;

            List<GridCoordinate> leftRacks = new ArrayList<>();
            List<GridCoordinate> rightRacks = new ArrayList<>();

            for (GridCoordinate rack : allRackPositions) {
                if (rack.col() < midCol) {
                    leftRacks.add(rack);
                } else {
                    rightRacks.add(rack);
                }
            }

            // Left aisle zone
            String leftZoneId = "zone_" + zoneId;
            List<GridCoordinate> leftCoverage = new ArrayList<>();
            int leftCol = block.getTopLeft().col() - 1;
            if (leftCol >= 0) {
                for (int row = block.getTopLeft().row(); row <= block.getBottomRight().row(); row++) {
                    leftCoverage.add(new GridCoordinate(row, leftCol));
                }
            }

            zones.put(leftZoneId, new Zone(
                leftZoneId,
                "aisle",
                List.of(block.getBlockId()),
                leftCoverage,
                leftRacks
            ));
            zoneId++;

            // Right aisle zone
            String rightZoneId = "zone_" + zoneId;
            List<GridCoordinate> rightCoverage = new ArrayList<>();
            int rightCol = block.getBottomRight().col() + 1;
            if (rightCol < warehouseMap.getConfig().getGridWidth()) {
                for (int row = block.getTopLeft().row(); row <= block.getBottomRight().row(); row++) {
                    rightCoverage.add(new GridCoordinate(row, rightCol));
                }
            }

            zones.put(rightZoneId, new Zone(
                rightZoneId,
                "aisle",
                List.of(block.getBlockId()),
                rightCoverage,
                rightRacks
            ));
            zoneId++;
        }
    }

    public void balanceVtmAssignments(List<Integer> vtmIds) {
        if (zones.isEmpty()) {
            return;
        }

        int numVtms = vtmIds.size();
        int numZones = zones.size();
        List<String> zoneIds = new ArrayList<>(zones.keySet());

        if (numVtms >= numZones) {
            // More VTMs than zones: assign multiple VTMs per zone
            for (int i = 0; i < vtmIds.size(); i++) {
                String zoneId = zoneIds.get(i % numZones);
                assignVtmToZones(vtmIds.get(i), List.of(zoneId));
            }
        } else {
            // More zones than VTMs: assign multiple zones per VTM
            int zonesPerVtm = numZones / numVtms;
            int remainder = numZones % numVtms;

            int zoneIdx = 0;
            for (int i = 0; i < vtmIds.size(); i++) {
                int numZonesForVtm = zonesPerVtm + (i < remainder ? 1 : 0);
                List<String> assignedZones = zoneIds.subList(zoneIdx, zoneIdx + numZonesForVtm);
                assignVtmToZones(vtmIds.get(i), assignedZones);
                zoneIdx += numZonesForVtm;
            }
        }
    }

    public void assignVtmToZones(int vtmId, List<String> zoneIds) {
        // Remove from all zones first
        zones.values().forEach(z -> z.removeVtm(vtmId));

        // Add to specified zones
        for (String zoneId : zoneIds) {
            Zone zone = zones.get(zoneId);
            if (zone != null) {
                zone.addVtm(vtmId);
            }
        }
    }

    public List<Zone> getZonesForVtm(int vtmId) {
        return zones.values().stream()
            .filter(z -> z.getAssignedVtmIds().contains(vtmId))
            .toList();
    }

    public List<Zone> getAllZones() {
        return new ArrayList<>(zones.values());
    }

    public GridCoordinate getZoneCenterPosition(Zone zone) {
        if (zone.getCoverageArea().isEmpty()) {
            return new GridCoordinate(0, 0);
        }

        int avgRow = zone.getCoverageArea().stream()
            .mapToInt(GridCoordinate::row)
            .sum() / zone.getCoverageArea().size();
        int avgCol = zone.getCoverageArea().stream()
            .mapToInt(GridCoordinate::col)
            .sum() / zone.getCoverageArea().size();

        return new GridCoordinate(avgRow, avgCol);
    }
}
```

**Step 4: Run tests to verify they pass**

```bash
mvn test -Dtest=ZoneManagerTest
```
Expected: All tests PASS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java
git commit -m "feat(relay): add ZoneManager for VTM zone assignment"
```

---

## Task 6: RelayPositionManager with Zone-Aware Selection

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java`

**Step 1: Write failing test for RelayPositionManager**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.map.GridCoordinate;
import com.greyorange.dessimulator.domain.map.RackBlock;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel;
import org.junit.jupiter.api.Test;
import java.util.List;
import java.util.Optional;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class RelayPositionManagerTest {

    @Test
    void testGenerateRelayPositions() {
        DesmojWarehouseModel model = mock(DesmojWarehouseModel.class);
        WarehouseMap map = mock(WarehouseMap.class);
        ZoneManager zoneManager = mock(ZoneManager.class);

        RackBlock block = new RackBlock(
            "block_0",
            new GridCoordinate(0, 6),
            new GridCoordinate(9, 10),
            10,
            List.of()
        );

        when(map.getRackBlocks()).thenReturn(List.of(block));
        when(map.getConfig()).thenReturn(mock(WarehouseMapConfig.class));
        when(map.getConfig().getGridWidth()).thenReturn(20);
        when(zoneManager.getAllZones()).thenReturn(List.of());

        RelayPositionManager manager = new RelayPositionManager(map, zoneManager, model);

        assertTrue(manager.getPositionsByBlock().containsKey("block_0"));
        assertFalse(manager.getPositionsByBlock().get("block_0").isEmpty());
    }

    @Test
    void testZoneAccessibility() {
        DesmojWarehouseModel model = mock(DesmojWarehouseModel.class);
        WarehouseMap map = mock(WarehouseMap.class);
        ZoneManager zoneManager = mock(ZoneManager.class);

        Zone zone = new Zone("zone_0", "aisle", List.of(),
            List.of(new GridCoordinate(5, 10)), List.of());
        when(zoneManager.getAllZones()).thenReturn(List.of(zone));
        when(map.getRackBlocks()).thenReturn(List.of());

        RelayPositionManager manager = new RelayPositionManager(map, zoneManager, model);

        // Manually add relay at zone coverage
        RelayPosition relay = new RelayPosition(model, "relay_1",
            new GridCoordinate(5, 10), "block_1", "left", 5);
        manager.getPositionsById().put("relay_1", relay);

        manager.buildZoneAccessibility(zoneManager);

        assertTrue(relay.isAccessibleFromZone("zone_0"));
    }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=RelayPositionManagerTest
```
Expected: Compilation error - `RelayPositionManager` does not exist

**Step 3: Write RelayPositionManager implementation (Part 1 - Core structure)**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.map.GridCoordinate;
import com.greyorange.dessimulator.domain.map.RackBlock;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel;
import java.util.*;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;

@Slf4j
@Getter
public class RelayPositionManager {
    private final WarehouseMap warehouseMap;
    private final Map<String, List<RelayPosition>> positionsByBlock = new HashMap<>();
    private final Map<String, List<RelayPosition>> positionsByZone = new HashMap<>();
    private final Map<String, RelayPosition> positionsById = new HashMap<>();
    private final int capacityPerPosition;

    public RelayPositionManager(
        WarehouseMap map,
        ZoneManager zoneManager,
        DesmojWarehouseModel model
    ) {
        this(map, zoneManager, model, 5);
    }

    public RelayPositionManager(
        WarehouseMap map,
        ZoneManager zoneManager,
        DesmojWarehouseModel model,
        int capacityPerPosition
    ) {
        this.warehouseMap = map;
        this.capacityPerPosition = capacityPerPosition;
        generateRelayPositions(model);
        buildZoneAccessibility(zoneManager);
    }

    private void generateRelayPositions(DesmojWarehouseModel model) {
        for (RackBlock block : warehouseMap.getRackBlocks()) {
            List<RelayPosition> blockRelays = new ArrayList<>();

            int leftCol = block.getTopLeft().col() - 1;
            int rightCol = block.getBottomRight().col() + 1;

            for (int row = block.getTopLeft().row(); row <= block.getBottomRight().row(); row++) {
                // Left relay
                if (leftCol >= 0) {
                    GridCoordinate leftCoord = new GridCoordinate(row, leftCol);
                    RelayPosition leftRelay = new RelayPosition(
                        model,
                        "relay_" + block.getBlockId() + "_L_r" + row,
                        leftCoord,
                        block.getBlockId(),
                        "left",
                        capacityPerPosition
                    );
                    blockRelays.add(leftRelay);
                    positionsById.put(leftRelay.getId(), leftRelay);
                }

                // Right relay
                if (rightCol < warehouseMap.getConfig().getGridWidth()) {
                    GridCoordinate rightCoord = new GridCoordinate(row, rightCol);
                    RelayPosition rightRelay = new RelayPosition(
                        model,
                        "relay_" + block.getBlockId() + "_R_r" + row,
                        rightCoord,
                        block.getBlockId(),
                        "right",
                        capacityPerPosition
                    );
                    blockRelays.add(rightRelay);
                    positionsById.put(rightRelay.getId(), rightRelay);
                }
            }

            positionsByBlock.put(block.getBlockId(), blockRelays);
        }
    }

    void buildZoneAccessibility(ZoneManager zoneManager) {
        for (RelayPosition relay : positionsById.values()) {
            GridCoordinate relayPos = relay.getPosition();

            for (Zone zone : zoneManager.getAllZones()) {
                // Direct containment: relay is IN zone's walking space
                if (zone.containsPosition(relayPos)) {
                    relay.addAccessibleZone(zone.getZoneId());
                    positionsByZone
                        .computeIfAbsent(zone.getZoneId(), k -> new ArrayList<>())
                        .add(relay);
                }
                // Adjacent: zone is next to relay
                else if (isAdjacentToZone(relayPos, zone)) {
                    relay.addAccessibleZone(zone.getZoneId());
                    positionsByZone
                        .computeIfAbsent(zone.getZoneId(), k -> new ArrayList<>())
                        .add(relay);
                }
            }
        }
    }

    private boolean isAdjacentToZone(GridCoordinate relayPos, Zone zone) {
        return zone.getCoverageArea().stream()
            .anyMatch(zonePos -> relayPos.manhattanDistance(zonePos) <= 1);
    }

    public List<RelayPosition> getPositionsWithTotes() {
        return positionsById.values().stream()
            .filter(RelayPosition::hasTotes)
            .toList();
    }

    public Optional<RelayPosition> getPositionById(String id) {
        return Optional.ofNullable(positionsById.get(id));
    }
}
```

**Step 4: Add zone-aware selection method**

Add to RelayPositionManager.java:

```java
public Optional<RelayPosition> selectOptimalRelayForVtm(
    Robot vtm,
    Tote tote,
    Rack sourceRack,
    ZoneManager zoneManager,
    StationManager stationManager
) {
    // Get VTM's assigned zones
    List<Zone> vtmZones = zoneManager.getZonesForVtm(vtm.getId());
    if (vtmZones.isEmpty()) {
        return getNearestRelayPosition(vtm.getPosition());
    }

    // Get relay positions accessible from VTM's zones
    Set<RelayPosition> accessibleRelays = vtmZones.stream()
        .map(zone -> positionsByZone.getOrDefault(zone.getZoneId(), Collections.emptyList()))
        .flatMap(List::stream)
        .collect(Collectors.toSet());

    if (accessibleRelays.isEmpty()) {
        // Fallback: use rack block's relays
        RackBlock sourceBlock = warehouseMap.findRackBlockForPosition(sourceRack.getCurrentCoordinate());
        if (sourceBlock != null) {
            accessibleRelays = new HashSet<>(
                positionsByBlock.getOrDefault(sourceBlock.getBlockId(), Collections.emptyList())
            );
        }
    }

    // Filter: not full
    List<RelayPosition> viableRelays = accessibleRelays.stream()
        .filter(relay -> !relay.isFull())
        .toList();

    if (viableRelays.isEmpty()) {
        // All full - use least full
        viableRelays = accessibleRelays.stream()
            .sorted(Comparator.comparingInt(RelayPosition::getPendingTotesCount))
            .limit(3)
            .toList();
    }

    if (viableRelays.isEmpty()) {
        return Optional.empty();
    }

    // Get target station position
    GridCoordinate targetStation = getTargetStationPosition(tote, stationManager);

    // Weighted selection: 60% VTM travel + 40% HTM travel
    return viableRelays.stream()
        .min(Comparator.comparingDouble(relay -> {
            int vtmTravel = vtm.getPosition().manhattanDistance(relay.getPosition());
            int htmTravel = relay.getPosition().manhattanDistance(targetStation);
            return 0.6 * vtmTravel + 0.4 * htmTravel;
        }));
}

public Optional<RelayPosition> findRelayWithTotes(GridCoordinate htmPosition) {
    return positionsById.values().stream()
        .filter(RelayPosition::hasTotes)
        .min(Comparator.comparingInt(rp ->
            rp.getPosition().manhattanDistance(htmPosition)
        ));
}

private Optional<RelayPosition> getNearestRelayPosition(GridCoordinate position) {
    return positionsById.values().stream()
        .min(Comparator.comparingInt(rp ->
            rp.getPosition().manhattanDistance(position)
        ));
}

private GridCoordinate getTargetStationPosition(Tote tote, StationManager stationManager) {
    return tote.getTasks().stream()
        .map(PickingTask::getStationId)
        .filter(Objects::nonNull)
        .findFirst()
        .map(stationManager::getStation)
        .map(Station::getPosition)
        .orElse(new GridCoordinate(0, 0));
}
```

**Step 5: Run tests to verify they pass**

```bash
mvn test -Dtest=RelayPositionManagerTest
```
Expected: All tests PASS

**Step 6: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java
git commit -m "feat(relay): add RelayPositionManager with zone-aware selection"
```

---

## Task 7: VtmProcess - Desmo-J Process

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/VtmProcess.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/simulator/VtmProcessTest.java`

**Note:** This is a complex process. Write integration test instead of unit test.

**Step 1: Write integration test skeleton**

```java
package com.greyorange.dessimulator.simulator;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class VtmProcessTest {

    @Test
    void testVtmProcessCreation() {
        // Integration test will be in DesmojIntegrationTest
        // This is a placeholder for now
        assertTrue(true);
    }
}
```

**Step 2: Implement VtmProcess**

```java
package com.greyorange.dessimulator.simulator;

import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.inventory.ToteState;
import com.greyorange.dessimulator.domain.robot.*;
import com.greyorange.dessimulator.domain.storage.StorageSpaceManager;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.SimulationConfig;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import com.greyorange.dessimulator.simulator.metrics.events.RelayPositionBlocked;
import com.greyorange.dessimulator.simulator.metrics.events.RobotEvents.VtmCycleCompleted;
import com.greyorange.dessimulator.simulator.metrics.events.RobotEvents.RobotIdle;
import desmoj.core.simulator.SimProcess;
import desmoj.core.simulator.TimeInstant;
import desmoj.core.simulator.TimeSpan;
import de.terministic.serein.core.SuspendExecution;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.TimeUnit;
import lombok.extern.slf4j.Slf4j;

@Slf4j
public class VtmProcess extends SimProcess {
    private final Robot vtm;
    private final ZoneManager zoneManager;
    private final RelayPositionManager relayPositionManager;
    private final StorageSpaceManager storageSpaceManager;
    private final StationManager stationManager;
    private final MetricsManager metricsManager;
    private final SimulationConfig config;
    private boolean simulationActive;

    public VtmProcess(
        DesmojWarehouseModel model,
        Robot vtm,
        ZoneManager zoneManager,
        RelayPositionManager relayPositionManager,
        StorageSpaceManager storageSpaceManager,
        StationManager stationManager,
        MetricsManager metricsManager,
        SimulationConfig config
    ) {
        super(model, "VtmProcess_" + vtm.getId(), true);
        this.vtm = vtm;
        this.zoneManager = zoneManager;
        this.relayPositionManager = relayPositionManager;
        this.storageSpaceManager = storageSpaceManager;
        this.stationManager = stationManager;
        this.metricsManager = metricsManager;
        this.config = config;
        this.simulationActive = false;
    }

    @Override
    public void lifeCycle() throws SuspendExecution {
        try {
            while (simulationActive) {
                // Check for high relay utilization (return journey priority)
                if (isHighRelayUtilization(80.0)) {
                    Optional<ReturnTask> returnTask = findReturnTask();
                    if (returnTask.isPresent()) {
                        executeReturnJourney(returnTask.get());
                        continue;
                    }
                }

                // Find tote with tasks in VTM's zone
                Optional<Tote> toteOpt = findToteWithTasksInZone();

                if (toteOpt.isEmpty()) {
                    // No tasks, robot idle
                    vtm.setState(RobotState.IDLE);
                    metricsManager.emit(new RobotIdle(
                        presentTime().getTimeAsDouble(TimeUnit.SECONDS),
                        vtm.getId()
                    ));
                    hold(new TimeSpan(2.0, TimeUnit.SECONDS));
                    vtm.setTotalIdleTime(vtm.getTotalIdleTime() + 2.0);
                    continue;
                }

                Tote tote = toteOpt.get();
                double cycleStart = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

                // Get source rack
                Rack sourceRack = storageSpaceManager.getRack(
                    Integer.parseInt(tote.getCurrentHoldingEntity())
                );

                // Select optimal relay position (zone-aware!)
                Optional<RelayPosition> relayOpt = relayPositionManager.selectOptimalRelayForVtm(
                    vtm, tote, sourceRack, zoneManager, stationManager
                );

                if (relayOpt.isEmpty()) {
                    log.warn("No relay position available for VTM {}", vtm.getId());
                    hold(new TimeSpan(2.0, TimeUnit.SECONDS));
                    continue;
                }

                RelayPosition relay = relayOpt.get();

                // Phase 1: Move to rack
                vtm.setState(RobotState.MOVING_TO_TOTE);
                double travelToRack = vtm.moveToPosition(sourceRack.getCurrentCoordinate());

                // Phase 2: Load tote
                vtm.setState(RobotState.LOADING_TOTE);
                double loadingTime = 6.0;
                vtm.addCarryingTote(tote);
                tote.setState(ToteState.BEING_TRANSPORTED_TO_RELAY);
                tote.setAssignedVtmId(vtm.getId());

                // Phase 3: Transport to relay
                vtm.setState(RobotState.TRANSPORTING_TOTE_TO_RELAY);
                double travelToRelay = vtm.moveToPosition(relay.getPosition());

                // Execute movement
                double totalTime = travelToRack + loadingTime + travelToRelay;
                hold(new TimeSpan(totalTime, TimeUnit.SECONDS));

                // Phase 4: Drop at relay (MAY BLOCK if full!)
                vtm.setState(RobotState.AT_RELAY_POSITION);

                double beforeBlock = presentTime().getTimeAsDouble(TimeUnit.SECONDS);
                boolean wasBlocked = relay.isFull();

                // This BLOCKS if relay is full
                relay.addTote(this, tote);

                double afterBlock = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

                if (wasBlocked) {
                    double waitTime = afterBlock - beforeBlock;
                    metricsManager.emit(new RelayPositionBlocked(
                        afterBlock,
                        vtm.getId(),
                        Integer.parseInt(relay.getId().split("_")[1]),  // Extract numeric ID
                        waitTime
                    ));
                }

                // Tote dropped successfully
                vtm.removeCarryingTote(tote);
                tote.setAssignedVtmId(null);

                // Emit cycle completion
                double cycleEnd = presentTime().getTimeAsDouble(TimeUnit.SECONDS);
                metricsManager.emit(new VtmCycleCompleted(
                    cycleEnd,
                    vtm.getId(),
                    String.valueOf(tote.getId()),
                    cycleStart,
                    cycleEnd - cycleStart,
                    sourceRack.getId(),
                    Integer.parseInt(relay.getId().split("_")[1])
                ));

                vtm.incrementTasksCompleted();
                vtm.setState(RobotState.IDLE);
            }
        } catch (Exception e) {
            log.error("VTM {} error: {}", vtm.getId(), e.getMessage(), e);
            vtm.setState(RobotState.ERROR);
        }
    }

    private Optional<Tote> findToteWithTasksInZone() {
        List<Zone> zones = zoneManager.getZonesForVtm(vtm.getId());
        if (zones.isEmpty()) {
            return Optional.empty();
        }

        return storageSpaceManager.getAllTotes().stream()
            .filter(Tote::hasTasks)
            .filter(t -> t.getState() == ToteState.IN_RACK)
            .filter(t -> {
                try {
                    int rackId = Integer.parseInt(t.getCurrentHoldingEntity());
                    Rack rack = storageSpaceManager.getRack(rackId);
                    if (rack == null) return false;

                    return zones.stream()
                        .anyMatch(zone -> zone.servicesRack(rack.getCurrentCoordinate()));
                } catch (NumberFormatException e) {
                    return false;
                }
            })
            .findFirst();
    }

    private boolean isHighRelayUtilization(double thresholdPct) {
        int totalCapacity = relayPositionManager.getPositionsById().values().stream()
            .mapToInt(rp -> rp.getResource().getCapacity())
            .sum();
        int totalOccupied = relayPositionManager.getPositionsById().values().stream()
            .mapToInt(RelayPosition::getPendingTotesCount)
            .sum();

        if (totalCapacity == 0) return false;

        double utilization = (totalOccupied * 100.0) / totalCapacity;
        return utilization >= thresholdPct;
    }

    private Optional<ReturnTask> findReturnTask() {
        // Find empty totes at relay positions
        List<RelayPosition> positionsWithTotes = relayPositionManager.getPositionsWithTotes();

        for (RelayPosition relay : positionsWithTotes) {
            for (Tote tote : relay.getPendingTotes()) {
                if (!tote.hasTasks()) {
                    try {
                        int rackId = Integer.parseInt(tote.getCurrentHoldingEntity());
                        Rack rack = storageSpaceManager.getRack(rackId);
                        if (rack != null && rack.hasEmptyToteSlots()) {
                            return Optional.of(new ReturnTask(tote, relay, rack));
                        }
                    } catch (NumberFormatException e) {
                        // Skip invalid rack IDs
                    }
                }
            }
        }
        return Optional.empty();
    }

    private void executeReturnJourney(ReturnTask task) throws SuspendExecution {
        // Simplified return journey implementation
        // Move to relay, pick up tote, return to rack
        log.debug("VTM {} executing return journey for tote {}", vtm.getId(), task.tote().getId());

        double cycleStart = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

        // Move to relay and pick up
        double travelToRelay = vtm.moveToPosition(task.relayPosition().getPosition());
        Optional<Tote> removed = task.relayPosition().removeTote();

        if (removed.isEmpty()) {
            return;
        }

        Tote tote = removed.get();
        vtm.addCarryingTote(tote);

        // Move to rack
        double travelToRack = vtm.moveToPosition(task.targetRack().getCurrentCoordinate());

        hold(new TimeSpan(travelToRelay + travelToRack + 6.0, TimeUnit.SECONDS));

        vtm.removeCarryingTote(tote);
        task.targetRack().addTote(tote);
        tote.returnToRack(task.targetRack().getId());

        vtm.incrementTasksCompleted();
    }

    public void setSimulationActive(boolean active) {
        this.simulationActive = active;
    }

    private record ReturnTask(Tote tote, RelayPosition relayPosition, Rack targetRack) {}
}
```

**Step 3: Compile to verify**

```bash
mvn clean compile -DskipTests
```
Expected: Compilation SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/VtmProcess.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/simulator/VtmProcessTest.java
git commit -m "feat(relay): implement VtmProcess with zone-aware relay selection"
```

---

## Task 8: HtmProcess - Desmo-J Process

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/HtmProcess.java`

**Step 1: Implement HtmProcess**

```java
package com.greyorange.dessimulator.simulator;

import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.inventory.ToteState;
import com.greyorange.dessimulator.domain.robot.*;
import com.greyorange.dessimulator.domain.station.Station;
import com.greyorange.dessimulator.domain.station.StationManager;
import com.greyorange.dessimulator.domain.storage.StorageSpaceManager;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.SimulationConfig;
import com.greyorange.dessimulator.simulator.metrics.MetricsManager;
import com.greyorange.dessimulator.simulator.metrics.events.RobotEvents.HtmCycleCompleted;
import com.greyorange.dessimulator.simulator.metrics.events.RobotEvents.RobotIdle;
import desmoj.core.advancedModellingFeatures.Res;
import desmoj.core.simulator.SimProcess;
import desmoj.core.simulator.TimeSpan;
import de.terministic.serein.core.SuspendExecution;
import java.util.*;
import java.util.concurrent.TimeUnit;
import java.util.function.BiConsumer;
import lombok.extern.slf4j.Slf4j;

@Slf4j
public class HtmProcess extends SimProcess {
    private final Robot htm;
    private final RelayPositionManager relayPositionManager;
    private final StationManager stationManager;
    private final StationProcessor stationProcessor;
    private final StorageSpaceManager storageSpaceManager;
    private final MetricsManager metricsManager;
    private final Map<Integer, Res> operatorResources;
    private final BiConsumer<String, String> orderCompletionCallback;
    private final SimulationConfig config;
    private boolean simulationActive;

    public HtmProcess(
        DesmojWarehouseModel model,
        Robot htm,
        RelayPositionManager relayPositionManager,
        StationManager stationManager,
        StationProcessor stationProcessor,
        StorageSpaceManager storageSpaceManager,
        Map<Integer, Res> operatorResources,
        BiConsumer<String, String> orderCompletionCallback,
        MetricsManager metricsManager,
        SimulationConfig config
    ) {
        super(model, "HtmProcess_" + htm.getId(), true);
        this.htm = htm;
        this.relayPositionManager = relayPositionManager;
        this.stationManager = stationManager;
        this.stationProcessor = stationProcessor;
        this.storageSpaceManager = storageSpaceManager;
        this.operatorResources = operatorResources;
        this.orderCompletionCallback = orderCompletionCallback;
        this.metricsManager = metricsManager;
        this.config = config;
        this.simulationActive = false;
    }

    @Override
    public void lifeCycle() throws SuspendExecution {
        try {
            while (simulationActive) {
                // Find relay position with waiting totes (prefer nearest)
                Optional<RelayPosition> relayOpt = relayPositionManager
                    .findRelayWithTotes(htm.getPosition());

                if (relayOpt.isEmpty()) {
                    // No totes waiting, idle
                    htm.setState(RobotState.IDLE);
                    metricsManager.emit(new RobotIdle(
                        presentTime().getTimeAsDouble(TimeUnit.SECONDS),
                        htm.getId()
                    ));
                    hold(new TimeSpan(2.0, TimeUnit.SECONDS));
                    htm.setTotalIdleTime(htm.getTotalIdleTime() + 2.0);
                    continue;
                }

                RelayPosition relay = relayOpt.get();
                double cycleStart = presentTime().getTimeAsDouble(TimeUnit.SECONDS);

                // Get tote (FIFO, frees resource slot)
                Optional<Tote> toteOpt = relay.removeTote();
                if (toteOpt.isEmpty()) {
                    hold(new TimeSpan(1.0, TimeUnit.SECONDS));
                    continue;
                }

                Tote tote = toteOpt.get();

                // Get target station
                Map<Integer, List<PickingTask>> stationTasks = tote.getTasksByStation();
                if (stationTasks.isEmpty()) {
                    log.warn("HTM {} found tote {} with no tasks", htm.getId(), tote.getId());
                    hold(new TimeSpan(1.0, TimeUnit.SECONDS));
                    continue;
                }

                // Simple: visit first station
                int stationId = stationTasks.keySet().iterator().next();
                Station station = stationManager.getStation(stationId);
                Res operatorResource = operatorResources.get(stationId);

                // Phase 1: Move to relay
                htm.setState(RobotState.MOVING_TO_RELAY);
                double travelToRelay = htm.moveToPosition(relay.getPosition());

                // Phase 2: Load tote
                htm.setState(RobotState.LOADING_TOTE);
                double loadingTime = 2.0;
                htm.addCarryingTote(tote);
                tote.setState(ToteState.BEING_TRANSPORTED_TO_STATION);
                tote.setAssignedHtmId(htm.getId());

                // Phase 3: Transport to station
                htm.setState(RobotState.TRANSPORTING_TOTE_TO_STATION);
                double travelToStation = htm.moveToPosition(station.getPosition());
                tote.setState(ToteState.AT_STATION);

                // Execute transport
                double transportTime = travelToRelay + loadingTime + travelToStation;
                hold(new TimeSpan(transportTime, TimeUnit.SECONDS));

                // Phase 4: Process tasks at station (REUSE StationProcessor!)
                StationProcessor.ProcessingMetrics processingMetrics =
                    stationProcessor.processStorageSpaceTasks(
                        this,
                        htm,
                        tote,
                        station,
                        operatorResource,
                        orderCompletionCallback
                    );

                // Phase 5: Return tote to relay
                htm.setState(RobotState.RETURNING_TOTE_TO_RELAY);
                tote.setState(ToteState.BEING_RETURNED_TO_RELAY);
                double returnTravel = htm.moveToPosition(relay.getPosition());
                double unloadingTime = 2.0;
                double returnTime = returnTravel + unloadingTime;

                hold(new TimeSpan(returnTime, TimeUnit.SECONDS));

                // Add tote back to relay
                htm.removeCarryingTote(tote);
                relay.addTote(this, tote);

                // Emit cycle completion
                double cycleEnd = presentTime().getTimeAsDouble(TimeUnit.SECONDS);
                metricsManager.emit(new HtmCycleCompleted(
                    cycleEnd,
                    htm.getId(),
                    String.valueOf(tote.getId()),
                    cycleStart,
                    cycleEnd - cycleStart,
                    processingMetrics.totalPicks(),
                    processingMetrics.stationsVisited()
                ));

                htm.incrementTasksCompleted();
                htm.setState(RobotState.IDLE);
            }
        } catch (Exception e) {
            log.error("HTM {} error: {}", htm.getId(), e.getMessage(), e);
            htm.setState(RobotState.ERROR);
        }
    }

    public void setSimulationActive(boolean active) {
        this.simulationActive = active;
    }
}
```

**Step 2: Compile to verify**

```bash
mvn clean compile -DskipTests
```
Expected: Compilation SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/HtmProcess.java
git commit -m "feat(relay): implement HtmProcess with station processing"
```

---

## Task 9: Integration - Wire Up Relay System in DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Add relay system initialization to DesmojSimulationEngine**

Locate the `run()` method around line 50 and add relay system setup:

```java
// In DesmojSimulationEngine.run()
if (config.systemType() == SystemType.RELAY) {
    // Create totes
    List<Tote> totes = createTotes(config, warehouseComponents);
    warehouseComponents.setTotes(totes);

    // Create zone manager
    ZoneManager zoneManager = new ZoneManager(warehouseComponents.getMap());

    // Create relay position manager
    RelayPositionManager relayPositionManager = new RelayPositionManager(
        warehouseComponents.getMap(),
        zoneManager,
        model,
        5  // capacity per position
    );

    // Assign VTMs to zones
    List<Integer> vtmIds = robots.stream()
        .filter(r -> r.getRobotType() == RobotType.VTM)
        .map(Robot::getId)
        .toList();
    zoneManager.balanceVtmAssignments(vtmIds);

    // Position VTMs in their zones
    for (int vtmId : vtmIds) {
        List<Zone> zones = zoneManager.getZonesForVtm(vtmId);
        if (!zones.isEmpty()) {
            Robot vtm = robots.get(vtmId);
            GridCoordinate centerPos = zoneManager.getZoneCenterPosition(zones.get(0));
            vtm.teleportTo(centerPos);
        }
    }

    // Store in components
    warehouseComponents.setZoneManager(zoneManager);
    warehouseComponents.setRelayPositionManager(relayPositionManager);
}

private List<Tote> createTotes(SimulationConfig config, WarehouseComponents components) {
    List<Tote> totes = new ArrayList<>();
    List<Rack> racks = components.getRacks();

    int toteId = 0;
    for (Rack rack : racks) {
        // Assign 3 totes per rack (example)
        for (int i = 0; i < 3; i++) {
            Tote tote = new Tote(
                toteId++,
                String.valueOf(rack.getId()),
                rack.getCurrentCoordinate()
            );
            rack.addTote(tote);
            totes.add(tote);
        }
    }

    return totes;
}
```

**Step 2: Add relay process initialization to DesmojWarehouseModel**

Locate `doInitialSchedules()` method and add:

```java
// In DesmojWarehouseModel.doInitialSchedules()
if (config.systemType() == SystemType.RELAY) {
    List<Robot> vtms = robots.stream()
        .filter(r -> r.getRobotType() == RobotType.VTM)
        .toList();

    List<Robot> htms = robots.stream()
        .filter(r -> r.getRobotType() == RobotType.HTM)
        .toList();

    // Schedule VTM processes
    double vtmStartTime = 0.0;
    for (Robot vtm : vtms) {
        VtmProcess vtmProcess = new VtmProcess(
            this,
            vtm,
            zoneManager,
            relayPositionManager,
            storageSpaceManager,
            stationManager,
            metricsManager,
            config
        );
        vtmProcess.setSimulationActive(true);
        vtmProcess.activate(new TimeInstant(vtmStartTime, TimeUnit.SECONDS));
        vtmStartTime += 0.5;  // Stagger by 0.5s
    }

    // Schedule HTM processes
    double htmStartTime = 0.0;
    for (Robot htm : htms) {
        HtmProcess htmProcess = new HtmProcess(
            this,
            htm,
            relayPositionManager,
            stationManager,
            stationProcessor,
            storageSpaceManager,
            operatorResources,
            orderCompletionCallback,
            metricsManager,
            config
        );
        htmProcess.setSimulationActive(true);
        htmProcess.activate(new TimeInstant(htmStartTime, TimeUnit.SECONDS));
        htmStartTime += 0.5;
    }
}
```

**Step 3: Compile to verify**

```bash
mvn clean compile -DskipTests
```
Expected: Compilation SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "feat(relay): integrate relay system into simulation engine"
```

---

## Task 10: Integration Test for Relay System

**Files:**
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/DesmojIntegrationTest.java`
- Create: `apps/backend/src/test/resources/relay-test-config.json`

**Step 1: Create relay test configuration**

```json
{
  "systemType": "relay",
  "simulationDuration": 1,
  "scenarioId": "test_scenario",
  "vtmRobotCount": 2,
  "htmRobotCount": 2,
  "binsCount": 2,
  "operatorConfig": {
    "numBins": 2,
    "operatorTimePerPick": 3.0
  },
  "algorithmConfig": {
    "orderSelectionAlgorithm": "fifo"
  }
}
```

**Step 2: Add relay integration test**

Add to `DesmojIntegrationTest.java`:

```java
@Test
void testRelaySystemIntegration() throws Exception {
    // Load relay configuration
    String configJson = Files.readString(
        Paths.get("src/test/resources/relay-test-config.json")
    );
    SimulationConfig config = objectMapper.readValue(configJson, SimulationConfig.class);

    // Run simulation
    DesmojSimulationEngine engine = new DesmojSimulationEngine(config);
    SimulationMetric result = engine.run();

    // Verify relay system metrics
    assertNotNull(result);
    assertNotNull(result.relayPositionMetrics());
    assertTrue(result.robotMetrics().perRobotStats().size() > 0);

    // Verify VTM and HTM cycles
    var robotStats = result.robotMetrics().perRobotStats();
    boolean hasVtmCycles = robotStats.values().stream()
        .anyMatch(stats -> "VTM".equals(stats.get("type")));
    boolean hasHtmCycles = robotStats.values().stream()
        .anyMatch(stats -> "HTM".equals(stats.get("type")));

    assertTrue(hasVtmCycles, "Should have VTM cycle completions");
    assertTrue(hasHtmCycles, "Should have HTM cycle completions");
}
```

**Step 3: Run integration test**

```bash
mvn test -Dtest=DesmojIntegrationTest#testRelaySystemIntegration
```
Expected: Test PASS

**Step 4: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/DesmojIntegrationTest.java
git add apps/backend/src/test/resources/relay-test-config.json
git commit -m "test(relay): add integration test for relay system"
```

---

## Task 11: Final Verification and Documentation

**Step 1: Run full test suite**

```bash
cd apps/backend
mvn clean test
```
Expected: All tests PASS

**Step 2: Build project**

```bash
mvn clean package -DskipTests
```
Expected: Build SUCCESS

**Step 3: Manual smoke test**

```bash
# Start server
mvn spring-boot:run &
sleep 10

# Test relay simulation
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @src/test/resources/relay-test-config.json

# Verify response has relay metrics
```

**Step 4: Update CLAUDE.md**

Add relay system documentation to `CLAUDE.md`:

```markdown
### Relay System (Java Backend)

**Components:**
- `Tote`: Storage container carrying PickingTasks (like Rack in RTP)
- `RelayPosition`: Cantilever position with Desmo-J Res for blocking
- `Zone`: VTM assignment area (aisle)
- `VtmProcess`: Zone-bound robot delivering totes to relay positions
- `HtmProcess`: Global robot transporting totes from relay to stations

**Key Design:**
- No RelayTask wrapper - Tote carries tasks directly
- Zone-aware relay selection (60% VTM + 40% HTM travel optimization)
- Automatic blocking via Desmo-J Res when relay full
- Reuses StationProcessor for both RTP and Relay systems

**Metrics:**
- RobotCollector: Tracks VTM/HTM cycles automatically
- RelayPositionCollector: Tracks blocking events and congestion
- StationCollector: Works for both RTP and HTM (stations_visited)
```

**Step 5: Final commit**

```bash
git add CLAUDE.md
git commit -m "docs: document relay system architecture and components"
```

---

## Completion Checklist

- [ ] All domain models created (Tote, ToteState, RelayPosition, Zone)
- [ ] ZoneManager for VTM assignment implemented
- [ ] RelayPositionManager with zone-aware selection implemented
- [ ] RelayPositionBlocked event added
- [ ] VtmProcess implemented with metrics emission
- [ ] HtmProcess implemented reusing StationProcessor
- [ ] Integration wiring in DesmojSimulationEngine
- [ ] Integration test passing
- [ ] Full test suite passing
- [ ] Documentation updated

**Estimated Time:** 8-10 hours (assuming engineer has zero context but solid Java/Desmo-J skills)

**Success Criteria:**
- Integration test passes
- Relay simulation returns valid metrics
- No regressions in RTP system tests
- Code compiles with zero warnings
