# DES Architecture Issues and Analysis

**Date**: 2025-11-08
**Status**: Critical - Zero Metrics Issue
**Affected Components**: SimulationEngine, Resource, ProcessScheduler

## Executive Summary

The Java DES implementation has a fundamental race condition between the event loop thread and virtual thread processes. This causes the simulation to terminate prematurely before robot processes can complete their work cycles, resulting in zero metrics being collected.

## Problem Overview

### Symptom
- Simulation completes but reports zero metrics (events_executed=0, robot_cycles_completed=0, total_picks_completed=0)
- Event queue empties prematurely
- Simulation time jumps to keep-alive events at t=1000 despite work remaining

### Root Cause
The event loop thread and virtual thread processes race in a way that allows the event loop to terminate before virtual threads can schedule their next events.

## Architecture Background

### Current DES Design

```
┌─────────────────┐
│  Event Loop     │  (Single thread)
│  Thread         │  - Polls event queue
└────────┬────────┘  - Executes events
         │           - Advances sim time
         │
         v
┌─────────────────┐
│  Event Queue    │  (Priority queue by time)
└────────┬────────┘
         │
         v
┌─────────────────┐
│  Virtual        │  (Multiple threads)
│  Threads        │  - Robot processes
│  (Processes)    │  - Execute concurrently
└─────────────────┘
```

### Key Components

1. **SimulationEngine** (infrastructure/des/core/SimulationEngine.java)
   - Runs event loop on main thread
   - Manages event queue (EventQueue)
   - Advances simulation clock
   - Terminates when queue is empty

2. **ProcessScheduler** (infrastructure/des/process/ProcessScheduler.java)
   - Launches robot processes as virtual threads
   - Schedules keep-alive events at t=1000
   - Virtual threads execute concurrently with event loop

3. **Resource** (infrastructure/des/resource/Resource.java)
   - Models limited resources (e.g., operators at stations)
   - Previously used Java Semaphore (blocking in real-time)
   - Now uses CountDownLatch + DES events (blocking in sim-time)

4. **engine.timeout()** (SimulationEngine.java:249-270)
   - Allows processes to "sleep" in simulation time
   - Schedules event and blocks thread with CountDownLatch
   - Critical for advancing simulation time in process-oriented DES

## Detailed Problem Analysis

### Issue #1: Resource Blocking in Real-Time (FIXED)

**Original Problem:**
```java
// Old implementation
private final Semaphore semaphore;

public void acquire() throws InterruptedException {
    semaphore.acquire();  // Blocks in REAL time, not simulation time!
}
```

**Impact:**
- Robot virtual threads blocked on semaphore in real-time
- Event loop continued and exhausted all events
- Simulation jumped to keep-alive events while robots waited

**Solution Applied:**
Refactored Resource.java to integrate with DES event loop:
```java
private int availablePermits;
private final Queue<CountDownLatch> waitQueue;

public void acquire() throws InterruptedException {
    synchronized (this) {
        if (availablePermits > 0) {
            availablePermits--;
            return;  // Immediate acquisition
        }
        myLatch = new CountDownLatch(1);
        waitQueue.add(myLatch);
    }
    myLatch.await();  // Block outside synchronized
}

public void release() {
    synchronized (this) {
        if (!waitQueue.isEmpty()) {
            CountDownLatch nextWaiter = waitQueue.poll();
            // Schedule DES event to wake waiter
            engine.schedule(0.0, () -> nextWaiter.countDown(), "Resource release");
        } else {
            availablePermits++;
        }
    }
}
```

**Files Modified:**
- `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java`

### Issue #2: Event Loop / Virtual Thread Race Condition (CRITICAL - UNFIXED)

**The Race:**

```
Timeline of Events:

t=0.0   : Event loop starts, processes begin
t=14.0  : Robot 0 transport timeout event fires
          └─> Robot 0 virtual thread wakes
          └─> Robot 0 calls operatorResource.acquire() - gets permit
          └─> Robot 0 about to call engine.timeout(processingTime)...

t=15.3  : Event loop polls next event (Robot 1 transport timeout)
          [Before Robot 0 can call engine.timeout()!]

t=16.7  : Event loop polls next event (Robot 2 transport timeout)

t=16.7+ : Event loop checks queue - sees only keep-alive events at t=1000
          └─> Event loop terminates simulation

t=???   : Robot 0 never gets CPU time to call engine.timeout(60.0)
          └─> Processing event never scheduled
          └─> Robot 0 never completes work cycle
          └─> Zero metrics collected
```

**Detailed Sequence:**

1. Robot virtual threads call `engine.timeout(transportTime)` for transport
2. Event loop processes these timeout events, waking virtual threads
3. Virtual threads wake and execute:
   - Calculate transport (no blocking, just math)
   - Call `operatorResource.acquire()`
   - Robot 0 gets permit immediately
   - **Robot 0 is about to call `engine.timeout(processingTime)`**
4. **BUT** event loop runs faster and polls the queue again
5. Event loop sees queue is empty (except keep-alives)
6. Event loop terminates before Robot 0 schedules its processing event

**Evidence from Logs:**

```
02:19:48.789 | Robot 2 ACQUIRED operator resource 'operator_1' at t=7.333
02:19:48.789 | About to poll: queue_size=6, peek=Event[time=15.333...]
02:19:48.789 | About to poll: queue_size=5, peek=Event[time=16.667...]
02:19:48.789 | About to poll: queue_size=5, peek=Event[time=75.333...]  ← Processing event!
02:19:48.790 | About to poll: queue_size=4, peek=Event[time=1000.000...]
02:19:48.790 | About to poll: queue_size=0, peek=null
02:19:48.790 | Event queue empty after poll, simulation terminating naturally
02:19:48.793 | Rack RackId[value=8] processed 1 tasks at Station...  ← Too late!
```

**Analysis:**
- An event at t=75.333 exists (Robot's processing timeout)
- Event loop skips from t=16.67 to t=1000 keep-alives
- The t=75.333 event appears briefly but simulation terminates anyway
- Robot 2 successfully processed its task but AFTER simulation ended

### Issue #3: Thread.yield() Insufficient

**Attempted Fix:**
```java
// SimulationEngine.java:201
Thread.yield();  // After executing each event
```

**Result:**
- Partially helped - Robot 2 successfully scheduled its processing event
- But event loop still terminated before processing that event
- Yield gives up CPU but doesn't guarantee virtual threads run

**Why It Failed:**
- `Thread.yield()` is a hint, not a guarantee
- Virtual threads may not get scheduled immediately
- Event loop continues too quickly after yielding

## Architectural Design Flaws

### Flaw #1: No Process State Tracking

The event loop has no knowledge of active processes:
```java
// SimulationEngine.runEventLoop() - Line 169
if (nextEvent.isEmpty()) {
    log.debug("Event queue empty, terminating");
    break;  // Terminates without checking if processes are still active!
}
```

**Problem:** Event loop doesn't know that virtual threads are actively executing and about to schedule events.

### Flaw #2: Asynchronous Event Scheduling

Virtual threads schedule events asynchronously from event loop:
```java
// Virtual thread executes:
engine.timeout(60.0);  // Schedules event + blocks

// Event loop simultaneously:
if (queue.isEmpty()) { terminate(); }  // Race!
```

**Problem:** No synchronization between event scheduling and queue polling.

### Flaw #3: Keep-Alive Events as Termination Guard

Keep-alive events prevent premature termination but are too far in future:
```java
engine.schedule(1000.0, () -> {}, "Keep-alive");  // Too late!
```

**Problem:** Simulation terminates when real events end, before reaching keep-alives.

## Potential Solutions

### Option 1: Quiescence Detection (Recommended)

Wait for system to reach a quiet state before terminating:

```java
private void runEventLoop() {
    while (running.get() && !stopped.get()) {
        Optional<Event> nextEvent = eventQueue.poll();

        if (nextEvent.isEmpty()) {
            // Queue empty - check if processes are still active
            if (hasActiveProcesses()) {
                // Yield and wait briefly for events to be scheduled
                Thread.sleep(10);  // Small delay in real-time
                continue;  // Check queue again
            }
            // No active processes - safe to terminate
            break;
        }

        // Process event...
    }
}

private boolean hasActiveProcesses() {
    // Check ProcessScheduler for non-terminated processes
    // that might schedule more events
}
```

**Pros:**
- Allows virtual threads time to schedule events
- Detects true completion (no active work)
- Mimics SimPy's behavior

**Cons:**
- Requires ProcessScheduler integration
- Adds small real-time delays

### Option 2: Synchronous Event Scheduling

Make event scheduling synchronous with event loop:

```java
public void timeout(double delay) throws InterruptedException {
    CountDownLatch latch = new CountDownLatch(1);

    synchronized (eventQueue) {
        // Schedule event while holding queue lock
        schedule(delay, () -> latch.countDown(), "Timeout");
        // Event is guaranteed in queue before we block
    }

    latch.await();
}
```

**Pros:**
- Eliminates race condition
- Events always in queue before polling

**Cons:**
- Requires event queue synchronization
- May impact performance

### Option 3: Two-Phase Event Loop

Separate event processing from termination checking:

```java
private void runEventLoop() {
    while (running.get()) {
        processAvailableEvents();  // Process all current events

        if (shouldTerminate()) {   // Check termination criteria
            break;
        }

        Thread.yield();  // Give processes time to schedule
    }
}

private boolean shouldTerminate() {
    return eventQueue.isEmpty()
        && !hasActiveProcesses()
        && timeExceeded();
}
```

**Pros:**
- Clear separation of concerns
- Multiple termination checks

**Cons:**
- More complex logic

### Option 4: Barrier Synchronization

Use barriers to coordinate event loop and processes:

```java
// After each event batch
CyclicBarrier barrier = new CyclicBarrier(numProcesses + 1);

// Event loop waits for all processes to reach barrier
barrier.await();

// Processes wait at barrier after scheduling next event
barrier.await();
```

**Pros:**
- Explicit synchronization
- Guaranteed ordering

**Cons:**
- Complex coordination
- Potential deadlocks

## Recommended Solution Path

### Phase 1: Quick Fix - Increase Yield Delay
```java
// SimulationEngine.java
Thread.sleep(1);  // 1ms instead of yield()
```
Or add multiple yields in a loop.

### Phase 2: Process State Tracking
Add interface to ProcessScheduler to query active processes:
```java
public boolean hasRunningProcesses() {
    return processes.values().stream()
        .anyMatch(ctx -> ctx.process.getState() == ProcessState.RUNNING);
}
```

### Phase 3: Quiescence-Based Termination
```java
if (nextEvent.isEmpty()) {
    if (processScheduler.hasRunningProcesses()) {
        Thread.sleep(10);  // Wait for processes to schedule events
        continue;
    }
    break;  // Safe to terminate
}
```

### Phase 4: Event Scheduling Guarantee
Ensure `engine.timeout()` completes event scheduling before returning to process:
```java
public void timeout(double delay) throws InterruptedException {
    CountDownLatch scheduledLatch = new CountDownLatch(1);
    CountDownLatch completionLatch = new CountDownLatch(1);

    schedule(delay, () -> {
        scheduledLatch.countDown();  // Signal scheduling complete
        completionLatch.countDown(); // Signal timeout complete
    });

    scheduledLatch.await();  // Wait for event to be in queue
    completionLatch.await(); // Wait for event to fire
}
```

## Testing Strategy

### Unit Tests Needed
1. Test Resource acquire/release with DES events
2. Test event loop doesn't terminate with active processes
3. Test quiescence detection
4. Test multiple processes scheduling events concurrently

### Integration Tests Needed
1. Full simulation with 4 robots
2. Verify all robots complete work cycles
3. Verify metrics collection (non-zero)
4. Verify simulation time advances correctly

### Regression Tests
1. Ensure old Semaphore-based Resource is gone
2. Verify no real-time blocking in DES components
3. Confirm all timeouts use engine.timeout()

## Related Files

### Core DES Infrastructure
- `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/core/SimulationEngine.java` - Event loop
- `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/event/EventQueue.java` - Priority queue
- `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/process/ProcessScheduler.java` - Virtual thread launcher
- `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/des/resource/Resource.java` - Resource management

### Robot Orchestrators
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/RtpRobotOrchestrator.java` - Robot processes
- `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/StationOrchestrator.java` - Station processing

### Documentation
- `docs/des-resource-integration-plan.md` - Resource refactoring plan
- `docs/race-condition-fix-summary.md` - Previous race condition fix

## References

- SimPy documentation: https://simpy.readthedocs.io/
- Java Virtual Threads (JEP 444): https://openjdk.org/jeps/444
- Discrete Event Simulation patterns: https://en.wikipedia.org/wiki/Discrete-event_simulation

## Action Items

- [ ] Implement quiescence detection in SimulationEngine
- [ ] Add ProcessScheduler.hasRunningProcesses() method
- [ ] Update event loop termination logic
- [ ] Add comprehensive integration tests
- [ ] Verify metrics collection works
- [ ] Document DES threading model
- [ ] Consider migrating to established DES framework (Desmo-J, SSJ)

## Conclusion

The current DES implementation has a critical race condition between the event loop and virtual thread processes. The Resource refactoring successfully eliminated real-time blocking, but revealed a deeper architectural issue with event loop termination. The recommended solution is to implement quiescence detection to ensure the event loop waits for all active processes to complete before terminating.
