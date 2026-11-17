# DES-Integrated Resource Implementation Plan

## Problem Statement

The current `Resource` class uses Java `Semaphore` for blocking, which blocks threads in **real time** rather than **simulation time**. This causes the DES event loop to exhaust all events while robot processes wait on semaphores, resulting in:

- Zero metrics collected
- Simulation time jumping to keep-alive events (t=1000)
- Robots never completing work cycles

### Evidence from Logs

```
01:59:38.758 | DEBUG | Robot 0 attempting to acquire operator resource at t=15.333
01:59:38.758 | DEBUG | Robot 3 attempting to acquire operator resource at t=23.333
01:59:38.758 | DEBUG | About to poll: queue_size=0, peek=null  [Event queue empty!]
01:59:38.758 | DEBUG | Robot 0 ACQUIRED operator resource at t=1000.0  [Jumped to t=1000!]
```

## Solution Overview

Refactor `Resource.java` to integrate with the DES event loop by:

1. Replace `Semaphore` blocking with simple permit counter
2. Maintain explicit FIFO queue of waiting processes
3. Schedule DES events (not real-time operations) when releasing permits
4. Ensure thread-safety with minimal synchronized blocks

## Architecture Design

### Current Implementation (Broken)

```java
// Resource.java - Current
private final Semaphore permits;  // Blocks in REAL TIME

public void acquire() throws InterruptedException {
    permits.acquire();  // ❌ Blocks thread, doesn't schedule DES events
}

public void release() {
    permits.release();  // ❌ Immediate release, not in simulation time
}
```

**Problem Flow**:
1. Robot 0 acquires permit → semaphore count = 0
2. Robot 1 calls `acquire()` → blocks on semaphore (real-time blocking)
3. Robot 0 processing → calls `engine.timeout(60)` → schedules event at t+60
4. Event loop continues → no more events → simulation ends
5. Robot 1 still blocked on semaphore when simulation terminates

### New Implementation (DES-Integrated)

```java
// Resource.java - New
private int availablePermits;  // Simple counter
private final Queue<CountDownLatch> waitQueue;  // FIFO queue
private final SimulationEngine engine;

public void acquire() throws InterruptedException {
    CountDownLatch myLatch = null;

    synchronized (this) {
        if (availablePermits > 0) {
            availablePermits--;
            stats.recordAcquisition();
            return;  // Got permit immediately
        }

        // No permits - add to wait queue
        myLatch = new CountDownLatch(1);
        waitQueue.add(myLatch);
        stats.recordQueueEntry();
    }

    // Block outside synchronized block (virtual thread efficiency)
    myLatch.await();

    synchronized (this) {
        stats.recordQueueExit();
        stats.recordAcquisition();
    }
}

public void release() {
    synchronized (this) {
        if (!waitQueue.isEmpty()) {
            // Wake next waiter via DES event
            CountDownLatch nextWaiter = waitQueue.poll();
            engine.schedule(
                0.0,  // Immediate in simulation time
                () -> nextWaiter.countDown(),
                "Resource release: " + name
            );
        } else {
            // No waiters - return permit to pool
            availablePermits++;
        }
        stats.recordRelease();
    }
}
```

**Correct Flow**:
1. Robot 0 acquires permit → availablePermits = 0
2. Robot 1 calls `acquire()` → creates latch, adds to waitQueue, blocks on latch
3. Robot 0 finishes processing → calls `release()`
4. `release()` schedules DES event: "wake Robot 1 at t=current"
5. Event loop processes release event → `nextWaiter.countDown()` → Robot 1 wakes
6. Robot 1 proceeds with processing

## Implementation Tasks

### Task 1: Refactor Resource.java Core Fields

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**:
- Remove `private final Semaphore permits;`
- Add `private int availablePermits;`
- Add `private final Queue<CountDownLatch> waitQueue = new ConcurrentLinkedQueue<>();`
- Ensure `private final SimulationEngine engine;` exists

**Code Location**: Lines 23-27

**Before**:
```java
@Getter private final String name;
@Getter private final int capacity;
private final Semaphore permits;
private final ResourceStats stats;
private final SimulationEngine engine;
```

**After**:
```java
@Getter private final String name;
@Getter private final int capacity;
private int availablePermits;  // Simple counter instead of Semaphore
private final Queue<CountDownLatch> waitQueue;  // FIFO queue for waiting processes
private final ResourceStats stats;
private final SimulationEngine engine;
```

**Validation**:
- Compilation succeeds
- No remaining references to `permits` semaphore field

---

### Task 2: Update Resource Constructor

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**:
- Initialize `availablePermits = capacity`
- Initialize `waitQueue = new ConcurrentLinkedQueue<>()`
- Remove `permits = new Semaphore(capacity, true)`

**Code Location**: Lines 36-56

**Before**:
```java
public Resource(String name, int capacity, SimulationEngine engine) {
    // validation...
    this.name = name;
    this.capacity = capacity;
    this.engine = engine;

    // Fair semaphore ensures FIFO ordering
    this.permits = new Semaphore(capacity, true);
    this.stats = new ResourceStats(name, capacity);

    log.debug("Created resource '{}' with capacity={}", name, capacity);
}
```

**After**:
```java
public Resource(String name, int capacity, SimulationEngine engine) {
    // validation...
    this.name = name;
    this.capacity = capacity;
    this.engine = engine;

    // Initialize permit counter and wait queue
    this.availablePermits = capacity;
    this.waitQueue = new ConcurrentLinkedQueue<>();
    this.stats = new ResourceStats(name, capacity);

    log.debug("Created resource '{}' with capacity={}", name, capacity);
}
```

**Validation**:
- Constructor compiles
- Resource creation logs appear in test runs

---

### Task 3: Implement DES-Integrated acquire() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Complete rewrite of `acquire()` method

**Code Location**: Lines 66-93

**Implementation**:
```java
/**
 * Acquire one unit of this resource (blocking in simulation time).
 *
 * <p>If the resource is available, acquire immediately. Otherwise, add to FIFO queue
 * and block until a DES event wakes this process when the resource is released.
 *
 * <p>Thread-safe: Uses synchronized blocks for critical sections, blocks outside lock.
 *
 * @throws InterruptedException if interrupted while waiting
 */
public void acquire() throws InterruptedException {
  double startSimTime = engine.now();
  CountDownLatch myLatch = null;

  synchronized (this) {
    if (availablePermits > 0) {
      // Permit available - take it immediately
      availablePermits--;
      stats.recordAcquisition();

      if (log.isTraceEnabled()) {
        log.trace(
            "Resource '{}' acquired immediately at t={:.3f} (available={}/{})",
            name,
            engine.now(),
            availablePermits,
            capacity);
      }
      return;
    }

    // No permits available - need to wait
    myLatch = new CountDownLatch(1);
    waitQueue.add(myLatch);
    stats.recordQueueEntry();

    if (log.isTraceEnabled()) {
      log.trace(
          "Resource '{}' not available at t={:.3f}, joining queue (queue_length={})",
          name,
          engine.now(),
          waitQueue.size());
    }
  }

  // Block outside synchronized block (virtual thread will park efficiently)
  myLatch.await();

  // When we wake up, we have been given the permit by release()
  synchronized (this) {
    stats.recordQueueExit();
    stats.recordAcquisition();

    if (log.isTraceEnabled()) {
      log.trace(
          "Resource '{}' acquired after waiting at t={:.3f} (waited {:.3f} sim-time)",
          name,
          engine.now(),
          engine.now() - startSimTime);
    }
  }
}
```

**Key Points**:
- Short synchronized blocks (just counter/queue manipulation)
- Virtual thread blocks on latch **outside** synchronized block
- FIFO ordering maintained by queue
- Logging shows acquisition flow

**Validation**:
- Method compiles
- Logs show "acquired immediately" for first robot
- Logs show "joining queue" for subsequent robots

---

### Task 4: Implement DES-Integrated release() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Complete rewrite of `release()` method

**Code Location**: Lines 147-159

**Implementation**:
```java
/**
 * Release one unit of this resource.
 *
 * <p>If processes are waiting, schedule a DES event to wake the next waiter.
 * Otherwise, increment available permits. Always call in a finally block.
 *
 * <p>Thread-safe: Uses synchronized block for queue/counter access.
 */
public void release() {
  synchronized (this) {
    if (!waitQueue.isEmpty()) {
      // Someone is waiting - wake them via DES event
      CountDownLatch nextWaiter = waitQueue.poll();

      if (log.isTraceEnabled()) {
        log.trace(
            "Resource '{}' released at t={:.3f}, waking next waiter (queue_remaining={})",
            name,
            engine.now(),
            waitQueue.size());
      }

      // Schedule immediate DES event to wake the waiting process
      // This ensures the wake happens in simulation time order
      engine.schedule(
          0.0, // Immediate in simulation time
          () -> {
            nextWaiter.countDown(); // Wake the waiting thread
          },
          "Resource release: " + name);

    } else {
      // No one waiting - return permit to pool
      availablePermits++;

      if (log.isTraceEnabled()) {
        log.trace(
            "Resource '{}' released at t={:.3f} (available={}/{})",
            name,
            engine.now(),
            availablePermits,
            capacity);
      }
    }

    stats.recordRelease();
  }
}
```

**Key Points**:
- Check waitQueue first (FIFO fairness)
- Schedule DES event at t=0.0 (immediate in sim time)
- Event wakes next waiter by calling `countDown()`
- Only increment counter if no waiters

**Validation**:
- Method compiles
- Logs show "waking next waiter" when queue has processes
- Logs show permit returned when queue empty

---

### Task 5: Update getAvailablePermits() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Use new field instead of semaphore

**Code Location**: Lines 166-168

**Before**:
```java
public int getAvailablePermits() {
  return permits.availablePermits();
}
```

**After**:
```java
public int getAvailablePermits() {
  synchronized (this) {
    return availablePermits;
  }
}
```

**Validation**:
- Method compiles
- Returns correct permit count

---

### Task 6: Update getInUse() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Use new field instead of semaphore

**Code Location**: Lines 175-177

**Before**:
```java
public int getInUse() {
  return capacity - permits.availablePermits();
}
```

**After**:
```java
public int getInUse() {
  synchronized (this) {
    return capacity - availablePermits;
  }
}
```

**Validation**:
- Method compiles
- Returns correct in-use count

---

### Task 7: Update isFull() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Use new field instead of semaphore

**Code Location**: Lines 184-186

**Before**:
```java
public boolean isFull() {
  return permits.availablePermits() == 0;
}
```

**After**:
```java
public boolean isFull() {
  synchronized (this) {
    return availablePermits == 0;
  }
}
```

**Validation**:
- Method compiles
- Returns correct full status

---

### Task 8: Update isFree() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Use new field instead of semaphore

**Code Location**: Lines 193-195

**Before**:
```java
public boolean isFree() {
  return permits.availablePermits() == capacity;
}
```

**After**:
```java
public boolean isFree() {
  synchronized (this) {
    return availablePermits == capacity;
  }
}
```

**Validation**:
- Method compiles
- Returns correct free status

---

### Task 9: Update getQueueLength() Method

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Use waitQueue instead of semaphore queue

**Code Location**: Lines 204-206

**Before**:
```java
public int getQueueLength() {
  return permits.getQueueLength();
}
```

**After**:
```java
public int getQueueLength() {
  return waitQueue.size();
}
```

**Validation**:
- Method compiles
- Returns correct queue length

---

### Task 10: Remove tryAcquire() Methods (Optional Cleanup)

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Remove or update `tryAcquire()` methods if not used

**Code Locations**:
- Lines 100-110 (non-blocking tryAcquire)
- Lines 120-139 (tryAcquire with timeout)

**Decision**: Check if these methods are used anywhere. If not, remove them. If used, update to use new fields:

```java
public boolean tryAcquire() {
  synchronized (this) {
    if (availablePermits > 0) {
      availablePermits--;
      stats.recordAcquisition();

      if (log.isTraceEnabled()) {
        log.trace("Resource '{}' acquired (non-blocking) at t={:.3f}", name, engine.now());
      }
      return true;
    }
    return false;
  }
}
```

**Validation**:
- Search codebase for `tryAcquire()` usage
- Update or remove accordingly

---

### Task 11: Add Required Imports

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Update imports

**Code Location**: Lines 3-7

**Before**:
```java
import com.greyorange.dessimulator.infrastructure.des.core.SimulationEngine;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;
```

**After**:
```java
import com.greyorange.dessimulator.infrastructure.des.core.SimulationEngine;
import java.util.Queue;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;
```

**Validation**:
- File compiles without import errors

---

### Task 12: Update Class JavaDoc

**File**: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

**Changes**: Update documentation to reflect DES integration

**Code Location**: Lines 9-20

**Before**:
```java
/**
 * Thread-safe resource with limited capacity (e.g., operators at stations).
 *
 * <p>This is a critical component for modeling resource contention in the simulation. Uses
 * Semaphore for fair, FIFO resource allocation with proper blocking behavior.
 *
 * <p>Key features: - Fair FIFO ordering (first come, first served) - Blocking and non-blocking
 * acquisition - Comprehensive statistics tracking - Works perfectly with virtual threads (no
 * pinning)
 *
 * <p>Thread-safety: All methods are thread-safe and can be called from any thread.
 */
```

**After**:
```java
/**
 * DES-integrated resource with limited capacity (e.g., operators at stations).
 *
 * <p>This is a critical component for modeling resource contention in discrete event simulation.
 * Unlike traditional threading resources, this integrates with the DES event loop to ensure
 * resource releases happen in simulation time, not real time.
 *
 * <p>Key features:
 * - Fair FIFO ordering (first come, first served) via explicit wait queue
 * - DES event scheduling for resource releases
 * - Blocking acquisition that respects simulation time
 * - Comprehensive statistics tracking
 * - Works efficiently with virtual threads (minimal synchronized blocks)
 *
 * <p>Thread-safety: All public methods are thread-safe. Synchronized blocks protect the
 * permit counter and wait queue. Blocking happens outside synchronized sections for efficiency.
 *
 * <p>Integration with DES: When release() is called, if processes are waiting, a DES event
 * is scheduled to wake the next waiter. This ensures proper simulation time advancement.
 */
```

**Validation**:
- Documentation accurately describes new implementation

---

## Testing Strategy

### Unit Tests

**File**: Create `apps/backend/src/test/java/com/greyorange/dessimulator/infrastructure/des/resource/ResourceDESIntegrationTest.java`

**Test Cases**:

1. **Test Immediate Acquisition**
   - Create resource with capacity=1
   - Acquire should succeed immediately
   - availablePermits should be 0

2. **Test Blocking and Release**
   - Create resource with capacity=1
   - Process 1 acquires (succeeds)
   - Process 2 acquires (blocks)
   - Process 1 releases
   - Verify Process 2 wakes and acquires

3. **Test FIFO Ordering**
   - Create resource with capacity=1
   - Process 1 acquires
   - Processes 2, 3, 4 attempt to acquire (all block)
   - Release and verify they wake in order 2→3→4

4. **Test DES Event Scheduling**
   - Mock SimulationEngine
   - Verify `engine.schedule(0.0, ...)` is called on release with waiters
   - Verify no schedule call when no waiters

5. **Test Statistics**
   - Verify recordAcquisition, recordQueueEntry, recordQueueExit called correctly
   - Verify queue length tracking

**Implementation Example**:
```java
@Test
void testBlockingAndRelease() throws InterruptedException {
    SimulationEngine engine = new SimulationEngine();
    Resource resource = new Resource("test", 1, engine);

    // Process 1 acquires
    resource.acquire();
    assertEquals(0, resource.getAvailablePermits());
    assertEquals(1, resource.getInUse());

    // Process 2 attempts to acquire (will block)
    CountDownLatch process2Started = new CountDownLatch(1);
    CountDownLatch process2Completed = new CountDownLatch(1);

    Thread.startVirtualThread(() -> {
        try {
            process2Started.countDown();
            resource.acquire();  // Blocks here
            process2Completed.countDown();
        } catch (InterruptedException e) {
            throw new RuntimeException(e);
        }
    });

    // Wait for process 2 to start and block
    process2Started.await();
    Thread.sleep(100);  // Give time to block
    assertEquals(1, resource.getQueueLength());

    // Process 1 releases
    resource.release();

    // Verify process 2 wakes up
    boolean completed = process2Completed.await(1, TimeUnit.SECONDS);
    assertTrue(completed, "Process 2 should wake after release");
    assertEquals(0, resource.getQueueLength());
}
```

### Integration Tests

**File**: Use existing simulation test

**Test Scenario**:
- Run full simulation with 4 robots, 1 operator
- Verify all robots complete cycles
- Verify metrics are non-zero
- Verify simulation time advances correctly
- Verify operator resource queue behavior

**Success Criteria**:
```json
{
  "events_executed": "> 100",
  "simulation_time": "≈ 14400.0",
  "robot_cycles_completed": "> 0",
  "total_picks_completed": "> 0",
  "operator_idle_time": "< simulation_duration"
}
```

---

## Rollout Plan

### Phase 1: Implementation
1. Complete Tasks 1-12 (Resource.java refactoring)
2. Ensure compilation succeeds
3. Run unit tests

### Phase 2: Testing
1. Run ResourceDESIntegrationTest
2. Run integration test with curl
3. Examine logs for correct behavior

### Phase 3: Validation
1. Verify metrics are non-zero
2. Verify robots complete work cycles
3. Verify operator utilization is reasonable
4. Check for any deadlocks or race conditions

### Phase 4: Documentation
1. Update race-condition-fix-summary.md with resource fix details
2. Document the DES integration pattern
3. Add comments explaining simulation time vs real time

---

## Success Metrics

### Before Fix
- ✗ Events executed: 4 (only initial robot timeouts)
- ✗ Simulation time: jumps to 1000.0
- ✗ Robot cycles: 0
- ✗ Metrics: all zeros
- ✗ Operator idle time: 1000.0

### After Fix (Expected)
- ✓ Events executed: 100-1000+ (depends on duration)
- ✓ Simulation time: advances smoothly, ends at configured duration
- ✓ Robot cycles: 4+ (one or more per robot)
- ✓ Metrics: all non-zero (picks, processing time, etc.)
- ✓ Operator idle time: < total simulation time

---

## Risks and Mitigation

### Risk 1: Deadlock on Resource Acquisition
**Scenario**: Virtual threads block forever on latch.await()

**Mitigation**:
- Ensure release() always calls schedule()
- Add timeout to latch.await() in testing
- Comprehensive logging to detect stuck threads

### Risk 2: Race Condition on Wait Queue
**Scenario**: Queue manipulation not properly synchronized

**Mitigation**:
- Use ConcurrentLinkedQueue (thread-safe)
- Protect counter with synchronized blocks
- Unit test with multiple concurrent acquirers

### Risk 3: Event Loop Starvation
**Scenario**: Too many release events overwhelm event queue

**Mitigation**:
- Release events are lightweight (just countDown())
- Event queue uses PriorityBlockingQueue (handles load)
- Monitor event queue size in logs

---

## Timeline Estimate

- Task 1-2 (Fields & Constructor): 10 minutes
- Task 3 (acquire() method): 20 minutes
- Task 4 (release() method): 20 minutes
- Task 5-9 (Helper methods): 15 minutes
- Task 10-12 (Cleanup & docs): 10 minutes
- **Total Implementation**: ~75 minutes

- Unit tests: 30 minutes
- Integration testing: 15 minutes
- Documentation: 15 minutes
- **Total Project**: ~2 hours

---

## References

- Original race condition fix: `docs/race-condition-fix-summary.md`
- SimPy Resource documentation: https://simpy.readthedocs.io/en/latest/api_reference/simpy.resources.html
- Java Virtual Threads: https://openjdk.org/jeps/444
- DES Core Engine: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/core/SimulationEngine.java`
