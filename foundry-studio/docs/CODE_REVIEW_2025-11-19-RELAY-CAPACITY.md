# Code Review: Relay Position Capacity Simplification Plan

**Reviewer**: Senior Code Reviewer
**Review Date**: 2025-11-19
**Branch**: fix/zone-capacity
**Design Document**: docs/plans/2025-11-19-relay-capacity-simplification-design.md
**Implementation Plan**: docs/plans/2025-11-19-relay-capacity-simplification-impl.md
**Current Status**: Plan not yet executed

---

## Executive Summary

The implementation plan provides clear, sequenced tasks to simplify the relay position capacity model from a queue-based approach with variable capacity to a single-tote slot model using `Optional<Tote>`. The plan is well-structured with 12 tasks and includes detailed code examples. However, there are critical deviations from the design specification and several missing elements that require attention before execution.

**Overall Assessment**: READY WITH CONDITIONS
- Plan completeness: 85% (missing critical design decision implementations)
- Code example quality: Good
- Task ordering: Logical with proper dependencies
- Risk level: MEDIUM (requires clarification on design vs. implementation mismatch)

---

## Issue Severity Summary

| Severity | Count | Status |
|----------|-------|--------|
| CRITICAL | 3 | Must resolve before implementation |
| MAJOR | 4 | Should resolve before implementation |
| MINOR | 5 | Should resolve, can be addressed during implementation |

---

## Critical Issues

### CRITICAL-1: RelayPosition Constructor Signature Mismatch

**Issue**: Design specifies a simplified constructor without capacity parameter, but implementation plan shows multiple constructors with capacity parameter.

**Design Specification** (lines 18-19 of design.md):
```java
// After:
new RelayPosition(model, id, position, zoneId);  // capacity always 1
```

**Implementation Plan** (Task 1, lines 93-106):
```java
public RelayPosition(
    DesmojWarehouseModel model,
    String id,
    GridCoordinate position,
    String rackBlockId,
    String side,
    String ownerZoneId) {
```

**Current Implementation** (RelayPosition.java, lines 33-49):
```java
// Constructor STILL has capacity parameter
public RelayPosition(
    DesmojWarehouseModel model,
    String id,
    GridCoordinate position,
    String rackBlockId,
    String side,
    int capacity) {  // <-- DESIGN says remove this
```

**Root Cause**: Design specifies `capacity always 1` (single tote per relay), implying hard-coded Res(capacity=1). Implementation plan adds constructors with Optional<Tote> but doesn't remove old constructors with capacity parameter.

**Impact**: CRITICAL - Leaves codebase in ambiguous state with both old and new constructors existing, risking caller confusion and incomplete migration.

**Recommendation**:
1. **Clarify design intent**: Should RelayPosition support variable capacity for backward compatibility, or is this a breaking change?
2. If breaking change: Remove ALL constructors with capacity parameter and hard-code Res capacity=1
3. If backward compatible: Keep constructor overloads but deprecate capacity-based ones

**Action Required**: Ask product/architecture to confirm design intent before implementing.

---

### CRITICAL-2: Zone Constructor Incompleteness

**Issue**: Implementation plan (Task 2) shows new Zone constructor WITHOUT capacity parameter, but doesn't address migration from old constructor calls.

**Design Specification** (Zone.java, lines 75-79 of design.md):
```java
public Zone(
    String zoneId,
    String zoneType,
    List<String> rackBlockIds,
    List<GridCoordinate> coverageArea,
    List<GridCoordinate> servicedRackCoordinates) {
  // No capacity parameter
}
```

**Current Implementation** (Zone.java, line 29):
```java
public Zone(
    String zoneId,
    String zoneType,
    List<String> rackBlockIds,
    List<GridCoordinate> coverageArea,
    List<GridCoordinate> servicedRackCoordinates,
    int relayCapacity) {  // <-- Still has capacity parameter
```

**Impact**: CRITICAL - WarehouseBuilder and other callers still pass `relayCapacity` parameter. Task 10 mentions updating WarehouseBuilder but provides no details on how to extract capacity information when capacity is now derived from relay positions.

**Recommendation**:
1. Update WarehouseBuilder to NOT pass capacity to Zone constructor
2. Capacity will be implicitly derived from number of relay positions added via `addRelayPosition()`
3. Document that zones must have relays registered BEFORE capacity queries

**Action Required**: Explicitly address this in Task 10 with code example for WarehouseBuilder changes.

---

### CRITICAL-3: Zone.getCapacity() Return Type Ambiguity

**Issue**: Design shows inlining ZoneCapacity methods directly into Zone, but doesn't specify whether to keep `getCapacity()` method for backward compatibility.

**Design Intent** (line 292 of design.md):
```
5. **No counter drift risk** - derived state can't get out of sync
6. **Cleaner API** - `zone.tryReserve()` instead of `zone.getCapacity().tryReserve()`
```

Implies: Direct methods on Zone (`zone.tryReserve()`), not `zone.getCapacity().tryReserve()`.

**Current Usage** (DesmojVtmProcess.java, lines 265, 271, 390, 525):
```java
targetZone.getCapacity().tryReserve()
targetZone.getCapacity().available()
ZoneCapacity capacity = zone.getCapacity()
```

**Issue**: Implementation plan doesn't show deprecation or removal of `getCapacity()` method. If it remains, callers won't migrate to new API.

**Impact**: CRITICAL - Caller code won't be forced to migrate, leading to technical debt and mixed usage patterns (some code using `zone.tryReserve()`, other code using `zone.getCapacity().tryReserve()`).

**Recommendation**:
1. Do NOT keep `getCapacity()` method - design intent is to remove the abstraction layer
2. All callers must use direct methods (`zone.tryReserve()`, `zone.available()`, etc.)
3. Failing to do this leaves a vestigial method that confuses future developers

**Action Required**: Explicitly state in design clarification that `getCapacity()` should be REMOVED entirely, not kept for backward compatibility.

---

## Major Issues

### MAJOR-1: Missing Zone.registerRelayPositions() in Task 4

**Issue**: Task 2 (Zone update) and Task 5 (ZoneManager update) don't coordinate on when/how relays are registered with zones.

**Design Specification** (lines 145-154 of design.md):
```java
// Register relays with zones (in `initializeRuntime()` or similar):
for (RelayPosition relay : positionsById.values()) {
  String zoneId = relay.getOwnerZoneId();
  Zone zone = zoneManager.getZones().get(zoneId);
  if (zone != null) {
    zone.addRelayPosition(relay);
  }
}
```

**Implementation Plan**:
- Task 2: Adds `addRelayPosition()` method
- Task 5: Adds `registerRelayPositions()` method
- Task 9: Calls `zoneManager().registerRelayPositions(relayPositionManager())`

**Missing**: No explicit test for the relay registration flow. ZoneTest.java tests individual methods but doesn't test the integration of:
1. Creating RelayPositions
2. Creating Zones
3. Registering relays with zones
4. Verifying capacity calculations

**Impact**: MAJOR - The registration flow is critical for correct zone capacity calculation. Without integration tests, we risk breaking the capacity model during implementation.

**Recommendation**:
1. Add integration test in ZoneTest: `testRelayRegistrationUpdatesCapacity()`
2. Test: Create 3 relays, register with zone, verify `getPhysicalCapacity()` == 3
3. Test: Add 2 totes, verify `countOccupied()` == 2
4. Test: Verify `available()` calculation

**Action Required**: Add integration test case to Task 2 or create separate integration test file.

---

### MAJOR-2: RelayPosition.initializeRuntime() Ownership Unclear

**Issue**: Design shows two-phase construction for RelayPosition, but unclear when `initializeRuntime()` is called.

**Implementation Plan** (Task 1, lines 113-137):
```java
public RelayPosition(String id, GridCoordinate position, ...) {
  // metadata only, no Res resource
  this.resource = null;
}

public void initializeRuntime(DesmojWarehouseModel model) {
  // Create Res resource
}
```

**Design Doesn't Clarify**:
- Who calls `initializeRuntime()`?
- When exactly (in DesmojSimulationEngine.setupExperiment)?
- What prevents double initialization?
- Does RelayPositionManager handle this, or does it bubble up to engine?

**Current Implementation** (RelayPosition.java):
- Has check: `if (this.resource != null)` throws exception
- Good defensive programming, but test coverage unclear

**Impact**: MAJOR - Two-phase construction is complex. If `initializeRuntime()` is forgotten on some relays, capacity model silently fails (NullPointerException when trying to request Res).

**Recommendation**:
1. Add explicit Task in plan: "Ensure all RelayPositions are initialized" with verification code
2. Add test: `testUninitializedRelayThrowsException()`
3. Consider factory pattern in RelayPositionManager to enforce initialization

**Action Required**: Add validation task or extend Task 1 with verification code.

---

### MAJOR-3: Zone.countOccupied() Performance Not Addressed

**Issue**: Implementation uses stream().filter().count() for every capacity query, which is O(n) where n = number of relay positions per zone.

**Implementation Plan** (Task 2, lines 396-399):
```java
public int countOccupied() {
  return (int) relayPositions.stream()
      .filter(r -> r.getTote().isPresent())
      .count();
}
```

**Context**:
- Zones can have 4-8 relay positions per zone
- This method called frequently in simulation loops (every VTM/HTM cycle)
- Zone may have 5-10 zones, so aggregate impact: 5-80 stream operations per cycle

**Impact**: MAJOR - Performance regression possible. Not critical for small zones, but could be issue with larger warehouses. Also, relayPositions is ArrayList, not optimized for frequent filtering.

**Recommendation**:
1. Keep current O(n) implementation for correctness
2. Add performance monitoring: log warning if zone.countOccupied() called >1000 times/second
3. Consider caching if profile shows bottleneck (use listener pattern when totes added/removed)
4. For now: Accept O(n) complexity, add TODO comment for future optimization

**Action Required**: Add TODO comment and performance monitoring option.

---

### MAJOR-4: Missing Test Coverage for RelayPosition.isEmpty()

**Issue**: Implementation plan (Task 1) shows test for new Optional-based API but doesn't test critical blocking behavior.

**Current Behavior** (RelayPosition.java):
```java
public void addTote(SimProcess process, Tote tote) throws SuspendExecution {
  resource.provide(1);  // BLOCKS if relay full
  this.tote = Optional.of(tote);
}
```

**Design Requirement**: Res(capacity=1) should block if process tries to add tote to occupied relay.

**Implementation Plan Test** (Task 1, lines 24-45):
```java
@Test
void testOptionalToteStorage() {
  // Tests isEmpty(), getTote(), peek()
  // DOES NOT test blocking behavior
}

@Test
void testIsEmptyAndGetTote() {
  // Also simple getter tests
}
```

**Missing**: No test for:
1. Adding tote to occupied relay blocks (Desmo-J Res behavior)
2. Removing tote releases Res slot
3. Concurrent add/remove doesn't corrupt state

**Impact**: MAJOR - Blocking behavior is core to the design (prevents VTM from overwhelming relay). Without test, we can't verify Desmo-J Res integration works correctly.

**Recommendation**:
1. Add Desmo-J test with mock SimProcess and time advancement
2. Test: Add tote, verify Res count decreased
3. Test: Try to add second tote, verify Res.provide() blocks
4. Alternatively: Accept that Desmo-J integration tested in DesmojIntegrationTest, skip unit-level blocking test

**Action Required**: Either add unit test for blocking behavior OR document why it's tested elsewhere.

---

## Minor Issues

### MINOR-1: Incomplete Method Signature in Implementation Plan (Task 4)

**Issue**: Task 4 (RelayPositionManager) says "Update method references" but doesn't show actual code changes.

**Plan Statement** (Task 4, Step 1, lines 156-177 of impl.md):
```
Update method references:
// Before:
.filter(RelayPosition::hasTotes)
.filter(RelayPosition::firstToteHasTasks)

// After:
.filter(relay -> relay.getTote().isPresent())
.filter(relay -> relay.getTote().map(Tote::hasTasks).orElse(false))
```

**Issue**:
- No actual diff shown for RelayPositionManager.java file
- Step 1 says "Remove capacityPerPosition and update relay creation" but doesn't list all methods needing updates
- Makes it unclear which methods need changes

**Impact**: MINOR - Reviewer must read source code to understand full scope. Incomplete plan makes verification difficult.

**Recommendation**: Add explicit before/after for all methods in RelayPositionManager that use old API.

**Action Required**: Extend Task 4 with method-by-method list or full diff.

---

### MINOR-2: Zone.available() Doesn't Account for Pending Totes

**Issue**: Zone.available() formula may not account for totes in-flight to relay.

**Implementation Plan** (Task 2, lines 412-414):
```java
public int available() {
  return getPhysicalCapacity() - countOccupied() - reserved;
}
```

**Design Context** (design.md, line 29):
```
4. **Keep reserved counter** - for VTM/HTM in-transit backpressure control
```

**Question**: Does `reserved` account for both:
1. VTMs with tote in-flight to relay (forward delivery)
2. HTMs with empty tote in-flight from relay (return journey)

Current formula: `available = capacity - occupied - reserved`

**Scenario**:
- Capacity = 4 relays
- Occupied = 2 relays (have totes)
- Reserved = 1 (VTM in-flight)
- Available = 4 - 2 - 1 = 1 slot

**Concern**: If HTM is also returning tote, does that count as additional reservation? Design says "Keep reserved counter" suggests single counter for all in-transit operations, but DesmojHtmProcess doesn't call `zone.tryReserve()` on return (per design.md line 257).

**Impact**: MINOR - Potential capacity undercount if HTM return journeys not properly reserved. Risk level depends on whether design intent is to reserve for returns or not.

**Recommendation**:
1. Verify in DesmojHtmProcess return logic that `zone.tryReserve()` is called before returning tote
2. If design says HTM returns also need reservation, document this explicitly
3. Add test case: verify available capacity accounts for both forward and return reservations

**Action Required**: Clarify HTM return journey reservation requirement in design.

---

### MINOR-3: Test File Location Inconsistency (Task 2)

**Issue**: Implementation plan creates ZoneTest.java but doesn't specify where.

**Implementation Plan** (Task 2, line 225):
```
Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java`
```

**Expectation**: Should be in `/src/test/java/com/greyorange/dessimulator/domain/robot/`

**Current State**: ZoneTest.java doesn't exist yet (new test file)

**Impact**: MINOR - Doesn't affect implementation, just test organization. Clearly stated in plan.

**Recommendation**: Confirmed as correctly specified. No action needed.

---

### MINOR-4: Missing Javadoc for New Zone Methods

**Issue**: Implementation plan provides code but no Javadoc comments.

**Implementation Plan** (Task 2, lines 382-437):
```java
public void addRelayPosition(RelayPosition relay) {
  // Comment: "Add a relay position to this zone."
  relayPositions.add(relay);
}
```

**Java Style Guide Requirement** (docs/JAVA_STYLE_GUIDE.md, section 2):
- Public methods must have Javadoc
- Multi-line Javadoc for complex methods

**Implementation Plan Javadoc** (Task 2, lines 378-391):
```java
/**
 * Add a relay position to this zone.
 */
public void addRelayPosition(RelayPosition relay) {
```

**Assessment**: Javadoc is present but minimal. For consistency with rest of codebase, should be more detailed.

**Recommendation**:
1. For simple methods like `addRelayPosition()`: Single-line Javadoc is acceptable
2. For complex methods like `available()`, `tryReserve()`: Add more detail on semantics and timing
3. Update Task 2 code example to match style guide requirements

**Example improved Javadoc**:
```java
/**
 * Attempt to reserve a relay position for delivery/return.
 *
 * <p>Atomically checks if zone has available capacity. If available, increments reserved counter
 * and returns true. Used by VTMs before starting forward journey to prevent deadlock.
 *
 * @return true if reservation successful, false if zone at capacity
 */
public boolean tryReserve() {
```

**Impact**: MINOR - Style issue, not functionality issue.

**Action Required**: Update Javadoc in Task 2 code example to match style guide.

---

### MINOR-5: No Backward Compatibility Notes

**Issue**: Implementation plan doesn't document breaking changes for API users.

**Breaking Changes**:
1. RelayPosition no longer accepts capacity parameter
2. Zone no longer accepts relayCapacity parameter
3. `zone.getCapacity()` method removed (replaced with direct methods)
4. `relay.peekFirstTote()` renamed to `relay.peek()`
5. `relay.isFull()` logic changes (now based on Optional presence)

**Current Impact**: Code currently still on old implementation, so any callers outside backend (frontend, tests) may break.

**Design Doc** (line 292): Mentions "Cleaner API" as benefit, but doesn't document migration path for external callers.

**Recommendation**:
1. Document all breaking changes in Task 12 (Final Verification)
2. Add section: "Breaking Changes and Migration Guide"
3. Update frontend type generation after new Java code compiled
4. Test that OpenAPI schema change is handled correctly

**Impact**: MINOR - Important for integration testing but not blocking.

**Action Required**: Add breaking changes section to Task 12.

---

## Completeness Analysis

### Coverage of Design Document

| Design Section | Impl Plan Coverage | Status |
|---|---|---|
| RelayPosition changes | 80% | Partial - missing constructor clarification |
| Zone changes | 75% | Partial - missing capacity derivation details |
| ZoneCapacity deletion | 90% | Good - clear Task 3 |
| RelayPositionManager | 70% | Partial - method updates not fully shown |
| ZoneManager | 85% | Good - explicit Task 5 |
| DesmojVtmProcess | 85% | Good - line-by-line mapping |
| DesmojHtmProcess | 85% | Good - line-by-line mapping |
| BackToStoreSelector | 85% | Good - specific replacements listed |
| DesmojSimulationEngine | 85% | Good - clear removal/addition |
| Other files | 70% | Vague - WarehouseBuilder changes not detailed |
| Testing | 60% | Partial - focuses on unit tests, missing integration tests |

**Overall Coverage**: 80% - Most critical files covered, but several with incomplete details

---

## Code Quality Assessment

### Strengths

1. **Clear Task Sequencing**: Tasks ordered logically with proper dependencies (Task 1 before Task 4, etc.)
2. **Test-First Approach**: Each major task starts with failing test (TDD)
3. **Commit Messages**: Descriptive commit messages showing clear intent
4. **Code Examples**: Good code examples in implementation plan (mostly correct)
5. **Type Safety**: Optional<Tote> is correct use of Java 21 for null safety

### Weaknesses

1. **Incomplete Constructor Handling**: Multiple constructor variants not reconciled
2. **Missing Integration Tests**: No test showing end-to-end zone capacity flow
3. **Performance Not Addressed**: Stream operations in countOccupied() not discussed
4. **Desmo-J Integration**: Res blocking behavior not tested at unit level
5. **Migration Documentation**: Breaking changes not clearly called out

---

## Task-by-Task Assessment

| Task | Completeness | Correctness | Ordering | Notes |
|------|---|---|---|---|
| 1 | 85% | 75% | Good | Constructor ambiguity needs clarification |
| 2 | 75% | 80% | Good | Missing integration test for relay registration |
| 3 | 95% | 90% | Good | Clear deletion task |
| 4 | 60% | 70% | Good | Method changes not fully enumerated |
| 5 | 85% | 85% | Good | Good new registerRelayPositions() method |
| 6 | 85% | 85% | Good | Line mappings accurate |
| 7 | 85% | 85% | Good | Clear replacements |
| 8 | 85% | 85% | Good | Specific method updates listed |
| 9 | 85% | 85% | Good | Good removal/addition of calls |
| 10 | 50% | 60% | Good | Vague - needs WarehouseBuilder details |
| 11 | 70% | 75% | Good | Test updates mentioned but not detailed |
| 12 | 80% | 85% | Good | Good verification steps, but no breaking changes doc |

---

## Risk Assessment

### High Risk Items

1. **Constructor Ambiguity** (CRITICAL-1): Risk of incomplete migration if both old and new constructors exist
2. **Zone Capacity Derivation** (CRITICAL-2): Risk that WarehouseBuilder doesn't properly pass capacity information
3. **getCapacity() Removal** (CRITICAL-3): Risk that callers still use old API, causing technical debt

### Medium Risk Items

1. **Relay Registration Flow** (MAJOR-1): Risk of missing relays if registration step forgotten
2. **Blocking Behavior** (MAJOR-4): Risk of Res blocking behavior not working as expected

### Low Risk Items

1. **Performance** (MAJOR-3): Can be optimized later if needed
2. **Documentation** (MINOR-4 & 5): Style/documentation issues don't affect functionality

---

## Recommendations Summary

### MUST DO Before Implementation

1. **Resolve CRITICAL-1**: Clarify whether capacity parameter should be removed from RelayPosition constructors or deprecated
2. **Resolve CRITICAL-2**: Provide explicit WarehouseBuilder code changes in Task 10
3. **Resolve CRITICAL-3**: Confirm getCapacity() method should be REMOVED entirely

### SHOULD DO Before Implementation

1. **Add integration test** (MAJOR-1): Test relay registration flow end-to-end
2. **Add blocking test** (MAJOR-4): Test Res blocking behavior or document why it's tested elsewhere
3. **Clarify HTM returns** (MINOR-2): Document whether HTM return journeys call tryReserve()
4. **Improve Javadoc** (MINOR-4): Update code examples to match style guide

### CAN DO During Implementation

1. **Performance monitoring** (MAJOR-3): Add TODO for future optimization of countOccupied()
2. **Method enumeration** (MINOR-1): Fully enumerate all method changes in Task 4
3. **Breaking changes doc** (MINOR-5): Document migration path for external callers

---

## Suggested Plan Improvements

### 1. Add Pre-Implementation Verification Task

**New Task 0: Verify Design Decisions**
```markdown
## Task 0: Verify Design Decisions

**Prerequisite**: Must complete before starting implementation

1. Confirm: RelayPosition capacity parameter should be REMOVED (breaking change)
2. Confirm: Zone no longer accepts relayCapacity (capacity derived from relays)
3. Confirm: zone.getCapacity() method should be REMOVED entirely
4. Confirm: HTM return journey calls zone.tryReserve() before adding tote

Verification: Document decisions in DECISIONS.md file
```

### 2. Expand Task 2 with Integration Test

```markdown
## Task 2b: Integration Test for Relay Registration

Add to ZoneTest.java:

@Test
void testRelayRegistrationUpdatesCapacity() {
  Zone zone = new Zone(...);

  // Initially no relays
  assertEquals(0, zone.getPhysicalCapacity());
  assertEquals(0, zone.available());

  // Add 3 relays
  for (int i = 0; i < 3; i++) {
    RelayPosition relay = new RelayPosition(...);
    zone.addRelayPosition(relay);
  }

  // Capacity should be 3
  assertEquals(3, zone.getPhysicalCapacity());
  assertEquals(3, zone.available());
}
```

### 3. Add Explicit WarehouseBuilder Changes to Task 10

```markdown
## Task 10: Update Remaining Files

**Step 1: WarehouseBuilder**

Remove capacity parameter from zone creation:

// Before:
zone = new Zone(zoneId, type, blocks, coverage, racks, relayCapacity);

// After:
zone = new Zone(zoneId, type, blocks, coverage, racks);
// Capacity now derived from relay positions registered in Task 9

// Verify all zone creation follows this pattern
```

### 4. Document Breaking Changes

```markdown
## Task 13: Document Breaking Changes

Create BREAKING_CHANGES.md:

1. RelayPosition no longer accepts capacity parameter
   - Old: new RelayPosition(model, id, pos, rack, side, capacity)
   - New: new RelayPosition(model, id, pos, rack, side, ownerZoneId)
   - Migration: All calls must remove capacity parameter

2. Zone no longer accepts relayCapacity
   - Old: new Zone(..., relayCapacity)
   - New: new Zone(...) // capacity derived from added relays
   - Migration: Remove parameter, ensure relays registered after zone creation

3. API Method Changes
   - zone.getCapacity() REMOVED -> use zone.tryReserve(), zone.available() directly
   - relay.peekFirstTote() -> relay.peek()
   - relay.isFull() logic changes (now Optional-based)

4. Frontend Impact
   - Regenerate types: npx openapi-typescript http://localhost:8080/openapi.json
   - Verify no relayCapacity references in frontend code
```

---

## Overall Assessment

The implementation plan is **READY WITH CONDITIONS**. It provides a solid structure for the simplification effort with clear tasks and mostly correct code examples. However, critical design decisions remain ambiguous:

1. Should RelayPosition.capacity be completely removed or deprecated?
2. How does WarehouseBuilder derive zone capacity without relayCapacity parameter?
3. Should zone.getCapacity() method remain for backward compatibility?

**Recommendation**:
1. Address the 3 CRITICAL issues with explicit design decisions
2. Add the suggested improvements (Tasks 0, 2b, 13)
3. Proceed with implementation following the enhanced plan

**Quality Score**: 7/10 (Good structure, but missing critical details)

---

## Files Referenced in Review

| File | Path | Status |
|------|------|--------|
| Design Document | docs/plans/2025-11-19-relay-capacity-simplification-design.md | Reviewed |
| Implementation Plan | docs/plans/2025-11-19-relay-capacity-simplification-impl.md | Reviewed |
| RelayPosition.java | apps/backend/src/main/java/.../robot/RelayPosition.java | Reviewed |
| Zone.java | apps/backend/src/main/java/.../robot/Zone.java | Reviewed |
| ZoneCapacity.java | apps/backend/src/main/java/.../robot/ZoneCapacity.java | Reviewed |
| DesmojVtmProcess.java | apps/backend/src/main/java/.../orchestrator/DesmojVtmProcess.java | Sampled |
| DesmojHtmProcess.java | apps/backend/src/main/java/.../orchestrator/DesmojHtmProcess.java | Sampled |
| BackToStoreSelector.java | apps/backend/src/main/java/.../put/BackToStoreSelector.java | Sampled |
| Java Style Guide | docs/JAVA_STYLE_GUIDE.md | Referenced |

---

## Sign-Off

This implementation plan provides a reasonable roadmap for the relay capacity simplification effort. With resolution of the 3 critical issues and incorporation of the suggested improvements, it is ready for execution.

**Reviewers**: Senior Code Reviewer
**Status**: APPROVED WITH CONDITIONS
**Next Step**: Address critical issues, then proceed with execution using superpowers:executing-plans
