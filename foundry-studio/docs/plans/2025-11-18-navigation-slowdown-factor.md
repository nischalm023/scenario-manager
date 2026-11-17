# Navigation Slowdown Factor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a global navigation slowdown factor (navFactor) that multiplies all robot travel times to model real-world congestion, with default 1.8 (range 1.0-3.0).

**Architecture:** Add `navFactor` field to `AlgorithmConfig` in backend, apply multiplier in process files (RTP/VTM/HTM) when calling `hold()` for travel time, include in TPQ calculations. Frontend gets a slider in Algorithm section.

**Tech Stack:** Java 21 (Spring Boot), React/TypeScript, Desmo-J simulation

---

## Task 1: Add navFactor to AlgorithmConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java:281-301`

**Step 1: Add navFactor field to AlgorithmConfig record**

In `SimulationConfigs.java`, modify the `AlgorithmConfig` record to add the new field:

```java
@Builder(toBuilder = true)
public record AlgorithmConfig(
    String putAlgorithm,
    OrderSelectionAlgorithm orderSelectionAlgorithm,
    int maxTotesPerOrder,
    TPQAlgorithmConfig tpqConfig,
    Double navFactor) {
  public AlgorithmConfig {
    if (putAlgorithm == null) {
      putAlgorithm = "greedy";
    }
    if (orderSelectionAlgorithm == null) {
      orderSelectionAlgorithm = OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY;
    }
    if (maxTotesPerOrder == 0) {
      maxTotesPerOrder = 10;
    }
    if (tpqConfig == null) {
      tpqConfig = TPQAlgorithmConfig.builder().build();
    }
    if (navFactor == null) {
      navFactor = 1.8;
    }
  }
}
```

**Step 2: Compile to verify syntax**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java
git commit -m "$(cat <<'EOF'
feat: add navFactor to AlgorithmConfig

Navigation slowdown factor (default 1.8) models real-world congestion.
Range 1.0-3.0, multiplies all robot travel times.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add navFactor to SimulationConfigDTO

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java:145-242`

**Step 1: Add navFactor field to SimulationConfigDTO**

Add the field with validation to the record (around line 165, after `snapshotIntervalSeconds`):

```java
record SimulationConfigDTO(
    @Min(1) @Max(24) Integer duration,
    @NotBlank String scenario,
    @NotBlank String systemType,
    @NotNull WarehouseMapConfig mapConfig,
    @Min(1) @Max(100) Integer rtpRobotCount,
    @DecimalMin("0.1") @DecimalMax("5.0") Double rtpRobotSpeed,
    Integer numVtms,
    Integer numHtms,
    Double vtmSpeed,
    Double htmSpeed,
    @Min(4) @Max(32) Integer verticalToteCapacityInRack,
    @DecimalMin("5.0") @DecimalMax("30.0") Double operatorTime,
    @DecimalMin("1.0") @DecimalMax("10.0") Double picksPerFace,
    @Min(4) @Max(32) Integer binsCount,
    @ValidOrderSelectionAlgorithm String orderSelectionAlgorithm,
    String putAlgorithm,
    String jobShopAlgorithm,
    Boolean enableJobShopScheduling,
    Boolean enableTpqOptimization,
    Boolean enableSnapshots,
    @DecimalMin("1.0") @DecimalMax("3600.0") Double snapshotIntervalSeconds,
    @DecimalMin("1.0") @DecimalMax("3.0") Double navFactor) {
```

**Step 2: Add default in compact constructor**

In the compact constructor (around line 183), add:

```java
if (navFactor == null) navFactor = 1.8;
```

**Step 3: Pass navFactor to AlgorithmConfig in toCoreConfig()**

In the `toCoreConfig()` method, update the AlgorithmConfig builder (around line 222-228):

```java
AlgorithmConfig algoConfig =
    AlgorithmConfig.builder()
        .orderSelectionAlgorithm(algo)
        .putAlgorithm(putAlgorithm)
        .maxTotesPerOrder(10)
        .tpqConfig(tpqConf)
        .navFactor(navFactor)
        .build();
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java
git commit -m "$(cat <<'EOF'
feat: add navFactor to SimulationConfigDTO

Exposes navigation slowdown factor in API with validation (1.0-3.0).
Default 1.8 for 80% congestion overhead.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add navFactor to ComparisonConfigDTO

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java:244-313`

**Step 1: Add navFactor field to ComparisonConfigDTO**

Add the field after `enableTpqOptimization` (around line 266):

```java
record ComparisonConfigDTO(
    @NotBlank String comparisonType,
    String algorithm1,
    String algorithm2,
    @Min(1) @Max(24) Integer duration,
    @NotBlank String scenario,
    @NotBlank String systemType,
    @NotNull WarehouseMapConfig mapConfig,
    @Min(1) @Max(100) Integer rtpRobotCount,
    @DecimalMin("0.1") @DecimalMax("5.0") Double rtpRobotSpeed,
    Integer numVtms,
    Integer numHtms,
    Double vtmSpeed,
    Double htmSpeed,
    @Min(4) @Max(32) Integer verticalToteCapacityInRack,
    @DecimalMin("5.0") @DecimalMax("30.0") Double operatorTime,
    @DecimalMin("1.0") @DecimalMax("10.0") Double picksPerFace,
    @Min(4) @Max(32) Integer binsCount,
    @ValidOrderSelectionAlgorithm String orderSelectionAlgorithm,
    String putAlgorithm,
    String jobShopAlgorithm,
    Boolean enableJobShopScheduling,
    Boolean enableTpqOptimization,
    @DecimalMin("1.0") @DecimalMax("3.0") Double navFactor) {
```

**Step 2: Add default in compact constructor**

Add after the `enableTpqOptimization` default (around line 285):

```java
if (navFactor == null) navFactor = 1.8;
```

**Step 3: Update toCoreConfig() delegation**

Update the `toCoreConfig()` method to pass `navFactor` to `SimulationConfigDTO` (around line 288-311):

```java
SimulationConfig toCoreConfig() {
  return new SimulationConfigDTO(
          duration,
          scenario,
          systemType,
          mapConfig,
          rtpRobotCount,
          rtpRobotSpeed,
          numVtms,
          numHtms,
          vtmSpeed,
          htmSpeed,
          verticalToteCapacityInRack,
          operatorTime,
          picksPerFace,
          binsCount,
          orderSelectionAlgorithm,
          putAlgorithm,
          jobShopAlgorithm,
          enableJobShopScheduling,
          enableTpqOptimization,
          false,
          10.0,
          navFactor)
      .toCoreConfig();
}
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java
git commit -m "$(cat <<'EOF'
feat: add navFactor to ComparisonConfigDTO

Comparison runs now use navFactor from base config.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Apply navFactor in TpqCalculator

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/TpqCalculator.java:52-108`

**Step 1: Apply navFactor to travel time calculation**

In the `calculate()` method, after calculating base travel time (around line 78), multiply by navFactor:

```java
// Calculate travel time (one-way, not round trip for this estimation)
double navFactor = config.algorithmConfig().navFactor();
double avgTravelTime = (avgTravelDistance / robotSpeed) * navFactor;
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/TpqCalculator.java
git commit -m "$(cat <<'EOF'
feat: apply navFactor in TPQ calculations

Travel time for TPQ metrics now includes navigation slowdown factor.
Affects target TPQ and optimal robot fleet calculations.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Apply navFactor in DesmojRtpRobotProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java`

**Step 1: Add navFactor field to the process**

Add a field to store navFactor, populated from config in constructor. First, find where the process stores config and add:

```java
private final double navFactor;
```

In the constructor, extract from config:

```java
this.navFactor = config.algorithmConfig().navFactor();
```

**Step 2: Apply navFactor to waypoint timeDelta**

The RTP process uses A* pathfinding with waypoints. Each `waypoint.timeDelta()` needs the navFactor applied. In the `hold()` calls for waypoints, multiply by navFactor:

For `pickUpRack()` method (around line 337):
```java
for (PathWaypoint waypoint : pathToRack.waypoints()) {
  robot.executeAction(waypoint.action());
  hold(new TimeSpan(waypoint.timeDelta() * navFactor, TimeUnit.SECONDS));
}
```

For `transportToStation()` method (around line 384):
```java
for (PathWaypoint waypoint : pathToStation.waypoints()) {
  robot.executeAction(waypoint.action());
  hold(new TimeSpan(waypoint.timeDelta() * navFactor, TimeUnit.SECONDS));
}
```

For `returnRackToStorage()` method (around line 436):
```java
for (PathWaypoint waypoint : pathToStorage.waypoints()) {
  robot.executeAction(waypoint.action());
  hold(new TimeSpan(waypoint.timeDelta() * navFactor, TimeUnit.SECONDS));
}
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java
git commit -m "$(cat <<'EOF'
feat: apply navFactor to RTP robot travel times

All A* pathfinding waypoint times are multiplied by navFactor
to model real-world congestion in RTP system.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Apply navFactor in DesmojVtmProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Add navFactor field and constructor parameter**

Add field:
```java
private final double navFactor;
```

Update constructor to accept and store navFactor. You'll need to check the current constructor signature and add the parameter, then assign it.

**Step 2: Apply navFactor to travel times**

VTM uses `moveToPosition()` which returns travel time. Apply navFactor to the hold calls.

For move to rack (around line 192):
```java
double travelToRack = vtm.moveToPosition(sourceRack.getCurrentCoordinate());
hold(new TimeSpan(travelToRack * navFactor, TimeUnit.SECONDS));
```

For move to relay (around line 217-218):
```java
double travelToRelay = vtm.moveToPosition(relay.getPosition());
hold(new TimeSpan(travelToRelay * navFactor, TimeUnit.SECONDS));
```

**Also in `executeReturnJourney()` method** (around line 449-465):

For move to relay to pick up tote:
```java
double travelToRelay = vtm.moveToPosition(task.relayPosition().getPosition());
hold(new TimeSpan(travelToRelay * navFactor, TimeUnit.SECONDS));
```

For move to rack to return tote:
```java
double travelToRack = vtm.moveToPosition(task.targetRack().getCurrentCoordinate());
double unloadingTime = 6.0;
hold(new TimeSpan((travelToRack * navFactor) + unloadingTime, TimeUnit.SECONDS));
```

**Step 3: Update caller to pass navFactor**

Find where `DesmojVtmProcess` is instantiated (likely in `DesmojWarehouseModel` or `DesmojSimulationEngine`) and pass the navFactor from config.

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "$(cat <<'EOF'
feat: apply navFactor to VTM robot travel times

VTM travel to rack and relay positions now includes navFactor
for congestion modeling.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Apply navFactor in DesmojHtmProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

**Step 1: Add navFactor field and constructor parameter**

Add field:
```java
private final double navFactor;
```

Update constructor similar to VTM process.

**Step 2: Apply navFactor to travel times**

For move to relay (around line 179):
```java
double travelToRelay = htm.moveToPosition(relay.getPosition());
hold(new TimeSpan(travelToRelay * navFactor, TimeUnit.SECONDS));
```

For move to station (around line 205-206):
```java
double travelToStation = htm.moveToPosition(station.getPosition().position());
hold(new TimeSpan(travelToStation * navFactor, TimeUnit.SECONDS));
```

For return journey (around line 320-322), the `returnTime` calculation should include navFactor for travel portion:
```java
double returnTravel = htm.moveToPosition(returnRelay.getPosition());
double unloadingTime = UNLOADING_TIME;
double returnTime = (returnTravel * navFactor) + unloadingTime;

hold(new TimeSpan(returnTime, TimeUnit.SECONDS));
```

**Step 3: Update caller to pass navFactor**

Update the instantiation site to pass navFactor from config.

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java
git commit -m "$(cat <<'EOF'
feat: apply navFactor to HTM robot travel times

HTM travel to relay and station positions now includes navFactor
for congestion modeling.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Add navFactor to frontend types

**Files:**
- Modify: `apps/frontend/src/types/simulation.ts:91-114`

**Step 1: Add navFactor to SimulationConfig interface**

Add after `putAlgorithm`:

```typescript
export interface SimulationConfig {
  systemType: 'rtp' | 'relay'
  rtpRobotCount: number
  rtpRobotSpeed: number
  numVtms: number
  numHtms: number
  vtmSpeed: number
  htmSpeed: number
  simulationDuration: number
  operatorTime: number
  aisleSpacing: number
  stationCount: number
  binsCount: number
  picksPerFace: number
  orderSelectionAlgorithm: string
  putAlgorithm: string
  navFactor: number
  scenario: string
  xPathwayFrequency: number
  yPathwayFrequency: number
  mapConfig: MapConfig
  enableSnapshots: boolean
  snapshotIntervalSeconds: number
  tpqTarget?: number
}
```

**Step 2: Commit**

```bash
git add apps/frontend/src/types/simulation.ts
git commit -m "$(cat <<'EOF'
feat: add navFactor to frontend SimulationConfig type

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Add navFactor to default config in SimulatorPage

**Files:**
- Modify: `apps/frontend/src/pages/SimulatorPage.tsx:48-70`

**Step 1: Add navFactor to initial state**

Add `navFactor: 1.8` to the config state initialization, after `putAlgorithm`:

```typescript
const [config, setConfig] = useState<SimulationConfig>({
  systemType: 'rtp',
  rtpRobotCount: 8,
  rtpRobotSpeed: 1.5,
  simulationDuration: 4,
  numVtms: 4,
  numHtms: 4,
  vtmSpeed: 1.5,
  htmSpeed: 1.5,
  operatorTime: 12,
  aisleSpacing: 20,
  stationCount: 4,
  binsCount: 8,
  picksPerFace: 3,
  orderSelectionAlgorithm: 'maximize_ppf_directly',
  putAlgorithm: 'greedy',
  navFactor: 1.8,
  scenario: 'scenario_a',
  // ... rest of config
})
```

**Step 2: Commit**

```bash
git add apps/frontend/src/pages/SimulatorPage.tsx
git commit -m "$(cat <<'EOF'
feat: add navFactor default (1.8) to SimulatorPage config

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Add navFactor slider to ConfigurationPanel

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx:453-489`

**Step 1: Add slider in Algorithm section**

Add the navFactor slider after the picksPerFace conditional block (around line 488), before the closing `</CollapsibleSection>`:

```tsx
<ConfigSlider
  label="Navigation Factor"
  value={config.navFactor}
  min={1.0}
  max={3.0}
  step={0.1}
  onChange={(value) => onConfigChange('navFactor', value)}
/>
<p className="text-xs text-gray-500 mt-2">
  Multiplier on travel time to model congestion. 1.0 = ideal, 1.8 = 80% overhead (default).
</p>
```

**Step 2: Run typecheck**

Run: `npm run check`

Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
git commit -m "$(cat <<'EOF'
feat: add navigation factor slider to configuration panel

Slider in Algorithm section, range 1.0-3.0, step 0.1, default 1.8.
Includes helper text explaining the congestion modeling.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Regenerate frontend types and verify

**Files:**
- Regenerate: `apps/frontend/src/types/generated-api.ts`

**Step 1: Start backend server**

```bash
rm -f /tmp/server.log /tmp/server.pid
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
cd apps/backend && npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/health
```

Expected: Health check returns OK

**Step 2: Regenerate types**

```bash
npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts
```

**Step 3: Stop server**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
```

**Step 4: Run full typecheck**

```bash
npm run check
```

Expected: No errors

**Step 5: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts
git commit -m "$(cat <<'EOF'
chore: regenerate frontend types with navFactor

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Manual testing

**Step 1: Start backend**

```bash
cd apps/backend && npx nx serve backend
```

**Step 2: Start frontend**

```bash
npm run dev
```

**Step 3: Test in UI**

1. Open http://localhost:4200
2. Expand Algorithm section in Configuration panel
3. Verify "Navigation Factor" slider appears with default 1.8
4. Adjust slider and run simulation
5. Verify simulation completes successfully
6. Test algorithm comparison - navFactor should be used for both runs

**Step 4: Verify TPQ metrics include navFactor**

In Goals tab, check that TPQ calculations reflect the navFactor (travel times should be 1.8x longer with default).

---

## Summary

**Files modified (10 total):**

Backend (7):
1. `SimulationConfigs.java` - AlgorithmConfig field
2. `SimulationsController.java` - Both DTOs
3. `TpqCalculator.java` - Travel time calculation
4. `DesmojRtpRobotProcess.java` - Waypoint times
5. `DesmojVtmProcess.java` - Travel times
6. `DesmojHtmProcess.java` - Travel times

Frontend (3):
7. `simulation.ts` - SimulationConfig type
8. `SimulatorPage.tsx` - Default config
9. `ConfigurationPanel.tsx` - Slider UI
10. `generated-api.ts` - Regenerated

**Key validation points:**
- All robot processes (RTP, VTM, HTM) apply navFactor to travel times
- TPQ calculator uses navFactor for optimal fleet sizing
- Comparison runs inherit navFactor from base config
- Frontend slider has correct range (1.0-3.0) and default (1.8)
