# A5 Shuttle Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add A5 shuttle robot support as an alternative to VTMs in the relay system, where shuttles are aisle-bound (cannot move between aisles).

**Architecture:** Extend RelayRobotConfig with `vtmType` ("robot"|"shuttle") and `vtmModel` fields. Add validation in WarehouseBuilder (fail-fast) to require numVtms >= aisles for shuttle mode. Aisle-bound behavior is enforced via existing zone assignment - when numVtms >= numAisles, each shuttle is assigned to one column's zones only.

**Tech Stack:** Java 21, Spring Boot 3.2, Desmo-J 2.5.1e

**Reference:** See `.claude/skills/relay-system/` for VTM/HTM architecture, `.claude/skills/java-style-checklist/` before committing.

---

## Task 1: Add vtmType and vtmModel to RelayRobotConfig

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java:173-228`
- Test: `apps/fleet-simulator/src/test/java/com/greyorange/dessimulator/simulator/config/RelayRobotConfigTest.java` (create)

**Step 1: Write the failing test**

Create test file:

```java
package com.greyorange.dessimulator.simulator.config;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.simulator.config.SimulationConfigs.RelayRobotConfig;
import org.junit.jupiter.api.Test;

class RelayRobotConfigTest {

  @Test
  void defaultVtmType_shouldBeRobot() {
    RelayRobotConfig config = RelayRobotConfig.builder().build();

    assertThat(config.vtmType()).isEqualTo("robot");
    assertThat(config.vtmModel()).isNull();
  }

  @Test
  void shuttleType_shouldBeAccepted() {
    RelayRobotConfig config =
        RelayRobotConfig.builder().vtmType("shuttle").vtmModel("quicktron-a5").build();

    assertThat(config.vtmType()).isEqualTo("shuttle");
    assertThat(config.vtmModel()).isEqualTo("quicktron-a5");
  }

  @Test
  void isShuttle_shouldReturnTrueForShuttleType() {
    RelayRobotConfig shuttleConfig = RelayRobotConfig.builder().vtmType("shuttle").build();
    RelayRobotConfig robotConfig = RelayRobotConfig.builder().vtmType("robot").build();
    RelayRobotConfig defaultConfig = RelayRobotConfig.builder().build();

    assertThat(shuttleConfig.isShuttle()).isTrue();
    assertThat(robotConfig.isShuttle()).isFalse();
    assertThat(defaultConfig.isShuttle()).isFalse();
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/fleet-simulator && mvn test -Dtest=RelayRobotConfigTest -q`

Expected: Compilation error - `vtmType()` method not found

**Step 3: Write minimal implementation**

Modify `SimulationConfigs.java` - update the RelayRobotConfig record:

```java
  /**
   * Relay robot-specific configuration.
   *
   * <p>Defines parameters for relay system robots including VTM (Vertical Tote Movers) and HTM
   * (Horizontal Tote Movers) fleet sizes, speeds, and capacities.
   *
   * <p>VTM types:
   * <ul>
   *   <li>"robot" (default) - Zone-bound, can move between aisles within zone
   *   <li>"shuttle" - Aisle-bound (e.g., Quicktron A5), fixed to one aisle
   * </ul>
   */
  @Builder
  public record RelayRobotConfig(
      String configType,
      String vtmType,
      String vtmModel,
      int numVtms,
      double vtmHorizontalSpeed,
      double vtmVerticalSpeed,
      int vtmCapacity,
      int numHtms,
      double htmSpeed)
      implements RobotConfig {
    public RelayRobotConfig {
      if (configType == null) {
        configType = "relay";
      }
      if (vtmType == null) {
        vtmType = "robot";
      }
      // vtmModel can be null (optional)
      if (numVtms == 0) {
        numVtms = 4;
      }
      if (vtmHorizontalSpeed == 0) {
        vtmHorizontalSpeed = 1.5;
      }
      if (vtmVerticalSpeed == 0) {
        vtmVerticalSpeed = 1.5;
      }
      if (vtmCapacity == 0) {
        vtmCapacity = 8;
      }
      if (numHtms == 0) {
        numHtms = 4;
      }
      if (htmSpeed == 0) {
        htmSpeed = 1.5;
      }
    }

    /** Check if this config uses shuttle (aisle-bound) VTMs. */
    public boolean isShuttle() {
      return "shuttle".equals(vtmType);
    }

    @Override
    public void validate() {
      if (numVtms < 1 || numVtms > 100) {
        throw new IllegalArgumentException("numVtms must be between 1 and 100");
      }
      if (numHtms < 1 || numHtms > 100) {
        throw new IllegalArgumentException("numHtms must be between 1 and 100");
      }
      if (vtmHorizontalSpeed < 0.1 || vtmHorizontalSpeed > 5.0) {
        throw new IllegalArgumentException("vtmHorizontalSpeed must be between 0.1 and 5.0");
      }
      if (vtmVerticalSpeed < 0.1 || vtmVerticalSpeed > 5.0) {
        throw new IllegalArgumentException("vtmVerticalSpeed must be between 0.1 and 5.0");
      }
      if (htmSpeed < 0.1 || htmSpeed > 5.0) {
        throw new IllegalArgumentException("htmSpeed must be between 0.1 and 5.0");
      }
      if (vtmCapacity < 4 || vtmCapacity > 32) {
        throw new IllegalArgumentException("vtmCapacity must be between 4 and 32");
      }
      if (vtmType != null && !vtmType.equals("robot") && !vtmType.equals("shuttle")) {
        throw new IllegalArgumentException("vtmType must be 'robot' or 'shuttle'");
      }
    }
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/fleet-simulator && mvn test -Dtest=RelayRobotConfigTest -q`

Expected: All 3 tests PASS

**Step 5: Format and commit**

```bash
cd apps/fleet-simulator && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java
git add src/test/java/com/greyorange/dessimulator/simulator/config/RelayRobotConfigTest.java
git commit -m "feat(relay): add vtmType and vtmModel fields to RelayRobotConfig

Support for A5 shuttle robots (aisle-bound) as alternative to VTMs (zone-bound).
- vtmType: 'robot' (default) or 'shuttle'
- vtmModel: optional, e.g., 'quicktron-a5'
- isShuttle() helper method

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>"
```

---

## Task 2: Add API DTO Fields for vtmType and vtmModel

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java`

**Note:** The `easyloop-simulation` app's `SimulationController.java` is a separate station simulator service and does NOT need changes.

**Step 1: Modify SimulationConfigDTO**

Add `vtmType` and `vtmModel` fields to the record definition (around line 204). Insert after `htmSpeed`:

```java
  record SimulationConfigDTO(
      @Min(1) @Max(24) Integer duration,
      @NotBlank String scenario,
      @NotBlank String systemType,
      @NotNull LayoutConfigDTO mapConfig,
      @Min(1) @Max(100) Integer rtpRobotCount,
      @DecimalMin("0.1") @DecimalMax("5.0") Double rtpRobotSpeed,
      Integer numVtms,
      Integer numHtms,
      Double vtmSpeed,
      Double htmSpeed,
      String vtmType,      // NEW: "robot" (default) or "shuttle"
      String vtmModel,     // NEW: optional, e.g., "quicktron-a5"
      @Min(4) @Max(32) Integer verticalToteCapacityInRack,
      @DecimalMin("5.0") @DecimalMax("30.0") Double operatorTime,
      // ... rest unchanged
```

**Step 2: Add defaults in compact constructor**

In the `SimulationConfigDTO` compact constructor (around line 228), add:

```java
    SimulationConfigDTO {
      // ... existing defaults ...
      if (vtmType == null) vtmType = "robot";  // NEW
      // vtmModel can be null (optional)
      // ... rest unchanged
    }
```

**Step 3: Update toCoreConfig() method**

In the `toCoreConfig()` method, update the RelayRobotConfig builder (around line 261):

```java
        robotConfig =
            RelayRobotConfig.builder()
                .vtmType(vtmType)        // NEW
                .vtmModel(vtmModel)      // NEW
                .numVtms(numVtms)
                .numHtms(numHtms)
                .vtmHorizontalSpeed(vtmSpeed)
                .vtmVerticalSpeed(1.5)
                .vtmCapacity(8)
                .htmSpeed(htmSpeed)
                .build();
```

**Step 4: Update ComparisonConfigDTO similarly**

Add the same fields to `ComparisonConfigDTO` (around line 310):
1. Add `vtmType` and `vtmModel` fields after `htmSpeed`
2. Add default for `vtmType` in compact constructor
3. Pass the fields through in `toCoreConfig()` (line 359-383) - they get forwarded to `SimulationConfigDTO`

**Step 5: Verify build compiles**

Run: `cd apps/fleet-simulator && mvn compile -q`

Expected: BUILD SUCCESS

**Step 6: Format and commit**

```bash
cd apps/fleet-simulator && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java
git commit -m "feat(api): add vtmType and vtmModel to simulation config DTOs

Expose A5 shuttle configuration via REST API.
- vtmType: 'robot' (default) or 'shuttle'
- vtmModel: optional, e.g., 'quicktron-a5'

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>"
```

---

## Task 3: Add Shuttle Validation in WarehouseBuilder (Fail Early)

**Why WarehouseBuilder, not DesmojSimulationEngine?**

Validation should happen BEFORE robots are built (Phase 4) to fail fast. The `WarehouseBuilder.build()` method runs phases in order:
1. Phase 1: Map (layout)
2. Phase 2: System-specific (zones, relay positions) - **zones are now available**
3. Phase 3: Inventory
4. Phase 4: Robots - **validation should happen BEFORE this**

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`
- Test: `apps/fleet-simulator/src/test/java/com/greyorange/dessimulator/simulator/ShuttleValidationTest.java` (create)

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.simulator;

import static org.assertj.core.api.Assertions.assertThatThrownBy;

import com.greyorange.dessimulator.simulator.config.SimulationConfigs.RelayRobotConfig;
import org.junit.jupiter.api.Test;

class ShuttleValidationTest {

  @Test
  void shuttleMode_withFewerVtmsThanAisles_shouldThrow() {
    // 8 aisles but only 4 shuttles - should fail
    int numberOfAisles = 8;
    RelayRobotConfig config =
        RelayRobotConfig.builder().vtmType("shuttle").numVtms(4).numHtms(4).build();

    assertThatThrownBy(() -> validateShuttleConfig(config, numberOfAisles))
        .isInstanceOf(IllegalArgumentException.class)
        .hasMessageContaining("at least one VTM per aisle")
        .hasMessageContaining("4 VTMs")
        .hasMessageContaining("8 aisles");
  }

  @Test
  void shuttleMode_withEnoughVtms_shouldPass() {
    int numberOfAisles = 8;
    RelayRobotConfig config =
        RelayRobotConfig.builder().vtmType("shuttle").numVtms(8).numHtms(4).build();

    // Should not throw
    validateShuttleConfig(config, numberOfAisles);
  }

  @Test
  void robotMode_withFewerVtmsThanAisles_shouldPass() {
    // Robot mode doesn't have the aisle constraint
    int numberOfAisles = 8;
    RelayRobotConfig config =
        RelayRobotConfig.builder().vtmType("robot").numVtms(4).numHtms(4).build();

    // Should not throw
    validateShuttleConfig(config, numberOfAisles);
  }

  /** Extracted validation logic for testing. */
  private void validateShuttleConfig(RelayRobotConfig config, int numberOfAisles) {
    if (config.isShuttle() && config.numVtms() < numberOfAisles) {
      throw new IllegalArgumentException(
          String.format(
              "Shuttle mode requires at least one VTM per aisle. Got %d VTMs but %d aisles.",
              config.numVtms(), numberOfAisles));
    }
  }
}
```

**Step 2: Run test to verify it passes**

Run: `cd apps/fleet-simulator && mvn test -Dtest=ShuttleValidationTest -q`

Expected: All 3 tests PASS (validation logic is in test for now)

**Step 3: Add getNumberOfAisles() to ZoneManager**

Add public method to `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java` (after `groupZonesByColumn()` around line 470):

```java
  /**
   * Get the number of unique aisle columns (for shuttle validation).
   *
   * @return Number of aisles (unique zone columns)
   */
  public int getNumberOfAisles() {
    return groupZonesByColumn().size();
  }
```

**Step 4: Add validation to WarehouseBuilder.build()**

Modify `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`.

Add validation AFTER Phase 2 (zones) and BEFORE Phase 3 (inventory). Find the `build()` method around line 154 and add validation between Phase 2 and Phase 3:

```java
    // Phase 2: System-specific (RELAY only)
    if (config.systemType() == SystemType.RELAY) {
      buildRelayPositionManager();
      buildZoneManager();
      zoneManager.registerRelayPositions(relayPositionManager);
      validatePhase("system-specific", zoneManager, relayPositionManager);

      // NEW: Validate shuttle configuration after zones are generated
      validateShuttleConfig();
    }
```

Add the validation method to WarehouseBuilder (in the Validation Helpers section around line 915):

```java
  /**
   * Validate shuttle configuration requires sufficient VTMs for aisle coverage.
   *
   * <p>Shuttles are aisle-bound (cannot move between aisles), so we need at least
   * one VTM per aisle. This validation runs after zones are generated but before
   * robots are built, to fail fast with a clear error message.
   *
   * @throws IllegalArgumentException if shuttle mode has fewer VTMs than aisles
   */
  private void validateShuttleConfig() {
    RelayRobotConfig relayConfig = config.getRelayRobotConfig();
    if (relayConfig.isShuttle()) {
      int numberOfAisles = zoneManager.getNumberOfAisles();
      if (relayConfig.numVtms() < numberOfAisles) {
        throw new IllegalArgumentException(
            String.format(
                "Shuttle mode requires at least one VTM per aisle. Got %d VTMs but %d aisles. "
                    + "Increase numVtms to at least %d or use vtmType='robot' for zone-bound VTMs.",
                relayConfig.numVtms(), numberOfAisles, numberOfAisles));
      }
      log.info(
          "Shuttle mode validated: {} shuttles for {} aisles (vtmModel={})",
          relayConfig.numVtms(),
          numberOfAisles,
          relayConfig.vtmModel());
    }
  }
```

**Step 5: Run all relay tests**

Run: `cd apps/fleet-simulator && mvn test -Dtest="*Relay*,*Shuttle*" -q`

Expected: All tests PASS

**Step 6: Format and commit**

```bash
cd apps/fleet-simulator && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git add src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java
git add src/test/java/com/greyorange/dessimulator/simulator/ShuttleValidationTest.java
git commit -m "feat(relay): add shuttle mode validation in WarehouseBuilder

Shuttle mode requires numVtms >= numberOfAisles since shuttles cannot
move between aisles. Validation runs after zones are generated but
before robots are built, to fail fast with a clear error message.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>"
```

---

## Task 4: Verify Zone Assignment Handles Shuttle Mode (No Code Changes)

**Why no code changes needed:**

The existing `ZoneManager.balanceVtmAssignments()` already handles shuttle mode correctly:

1. When `numVtms >= numColumns` (which shuttle validation enforces), each VTM is assigned to zones in **only one column** (aisle)
2. `DesmojVtmProcess.findToteWithTasksInAnyZone()` already filters totes to only those in assigned zones
3. Therefore, shuttles automatically only see totes in their assigned aisle - no additional constraint enforcement needed

**Verification steps:**

**Step 1: Review zone assignment logic**

File: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java:148-173`

```java
    if (numVtms >= numColumns) {
      // MORE VTMs than columns: distribute VTMs across columns
      // Each VTM gets all zones in its assigned column (many-to-many)
      int vtmsPerColumn = numVtms / numColumns;
      // ... assigns each VTM to ONE column's zones
    }
```

**Step 2: Review tote selection logic**

File: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java:458-485`

```java
  private Optional<Tote> findToteWithTasksInAnyZone() {
    Set<String> zoneIds = zoneManager.getZonesForVtm(vtm.getId());
    // ... filters totes to only those in VTM's assigned zones
  }
```

**Conclusion:** Shuttle constraint is enforced by zone assignment. No VTM process changes needed.

**Step 3: No commit needed** - this is a verification task only

---

## Task 5: Integration Test with Shuttle Config

**Files:**
- Create: `apps/fleet-simulator/src/test/resources/smoke-test-shuttle.json`

**Step 1: Create shuttle test config**

```json
{
  "duration": 1,
  "scenario": "scenario_a",
  "systemType": "relay",
  "vtmType": "shuttle",
  "vtmModel": "quicktron-a5",
  "numVtms": 8,
  "numHtms": 4,
  "vtmSpeed": 3.0,
  "htmSpeed": 4.5,
  "mapConfig": {
    "gridWidth": 50,
    "gridDepth": 30,
    "cellSizeMeters": 1.0,
    "rackBlockConfig": {
      "rackColumns": 3,
      "rackRows": 10,
      "verticalToteCapacity": 8
    },
    "stationLayout": {
      "layoutType": "SINGLE_BOTTOM",
      "stationsPerEdge": 2
    }
  },
  "operatorTime": 12.0,
  "picksPerFace": 3.0,
  "binsCount": 8,
  "enableSnapshots": false
}
```

**Step 2: Run integration test manually**

Start the cluster:
```bash
# Terminal 1-4: Start services
cd apps/put-service && mvn spring-boot:run
cd apps/layout-service && mvn spring-boot:run
cd apps/navigation-service && mvn spring-boot:run
cd apps/fleet-simulator && mvn spring-boot:run
```

Run the test:
```bash
curl -X POST http://localhost:8085/api/simulation/run \
  -H "Content-Type: application/json" \
  -d @apps/fleet-simulator/src/test/resources/smoke-test-shuttle.json
```

Expected: Simulation completes successfully with shuttle mode active in logs

**Step 3: Commit test config**

```bash
git add apps/fleet-simulator/src/test/resources/smoke-test-shuttle.json
git commit -m "test(relay): add shuttle mode smoke test config

Integration test configuration for A5 shuttle support.

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>"
```

---

## Task 6: Final Verification and Cleanup

**Step 1: Run all tests**

```bash
cd apps/fleet-simulator && mvn test -q
```

Expected: All tests PASS

**Step 2: Verify backward compatibility**

Run existing relay smoke test to ensure robot mode still works:
```bash
curl -X POST http://localhost:8085/api/simulation/run \
  -H "Content-Type: application/json" \
  -d @apps/fleet-simulator/src/test/resources/smoke-test-relay.json
```

Expected: Simulation completes successfully (vtmType defaults to "robot")

**Step 3: Final format check**

```bash
cd apps/fleet-simulator && mvn spotless:check
```

Expected: No formatting issues

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Add vtmType/vtmModel to RelayRobotConfig | SimulationConfigs.java |
| 2 | Add API DTO fields | SimulationsController.java |
| 3 | Add shuttle validation (fail early) | WarehouseBuilder.java, ZoneManager.java |
| 4 | Verify zone assignment (no code changes) | - |
| 5 | Integration test | smoke-test-shuttle.json |
| 6 | Final verification | - |

**Key Architecture Insights:**

1. **Fail-fast validation:** Shuttle validation happens in `WarehouseBuilder` after zones are generated (Phase 2) but BEFORE robots are built (Phase 4). This provides clear error messages before wasting resources.

2. **Aisle-bound via zone assignment:** When shuttle validation ensures `numVtms >= numAisles`, the existing `balanceVtmAssignments()` assigns each shuttle to exactly one column's zones, and `findToteWithTasksInAnyZone()` automatically restricts tote selection to that aisle.

3. **No changes to easyloop-simulation:** The `SimulationController.java` in `apps/greymatter-platform/easyloop-simulation/` is a separate station simulator service and does NOT need changes for this feature.
