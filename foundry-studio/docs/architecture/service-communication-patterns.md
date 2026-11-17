# Service Communication Patterns

**Status:** Living Document
**Last Updated:** 2025-01-09
**Owner:** Platform Team

---

## Purpose

This document defines when to use synchronous (gRPC) vs asynchronous (Kafka) communication between services. It provides decision frameworks and guidelines to ensure architectural consistency.

---

## Key Tenets

1. **Critical path uses gRPC.** If you need a result to continue, use request/response.

2. **Events use messaging.** If you're broadcasting "something happened" to multiple consumers, use Kafka.

3. **Simplicity over flexibility.** Don't add Kafka until you have a concrete need for decoupling, durability, or fan-out.

4. **Caller decides coupling.** The service making the call owns the decision about sync vs async based on its needs.

5. **Failures should be explicit.** Prefer failing fast (gRPC timeout) over silent failures in queues when the operation is critical.

---

## Decision Framework

### The Two Patterns

| | gRPC | Kafka |
|--|------|-------|
| Pattern | Request/Response | Publish/Subscribe |
| Coupling | Tight | Loose |
| Latency | 1-10ms | 10-100ms+ |
| Durability | None | Configurable retention |
| Consumer availability | Must be online | Can catch up later |

### When to Use What

| Question | If Yes → | If No → |
|----------|----------|---------|
| Do I need the result to continue? | gRPC | Consider Kafka |
| Is this a command ("do this")? | gRPC | - |
| Is this an event ("this happened")? | - | Kafka |
| Single consumer? | gRPC (simpler) | Kafka |
| Multiple independent consumers? | Kafka | - |
| Consumer can be offline? | Kafka | gRPC |
| Latency < 10ms required? | gRPC | Either |
| Need guaranteed delivery? | Kafka | - |

### Decision Tree

```
Need immediate result to continue?
├─ YES → gRPC
└─ NO → Can consumer be offline?
        ├─ YES → Kafka
        └─ NO → gRPC (async)
```

---

## Current Architecture

### Communication Flow

```
Frontend → Envoy:8080 ──┬── /api/warehouse-map/* → LayoutService:8082
                        └── /api/*              → FleetSimulator:8085

FleetSimulator ──gRPC──→ LayoutService    (getLayout, getNavigationData)
FleetSimulator ──gRPC──→ PutService       (initializeInventory)
FleetSimulator ──gRPC──→ NavigationService (createSession, findPath, deleteSession)
```

### Why gRPC for Current Services

| Service Call | Why gRPC |
|--------------|----------|
| `LayoutService.getLayout()` | Simulation cannot start without layout |
| `PutService.initializeInventory()` | Inventory must be placed before robots move |
| `NavigationService.findPath()` | Robot blocks until path is known |

All current inter-service calls are on the **critical path**. The simulation cannot proceed without their results.

### Internal Event System

Within `fleet-simulator`, we use an in-process event system for metrics:

```
Robot Process → metricsManager.emit(event) → Multiple Collectors
```

This is choreography, but **in-process** (no Kafka). Collectors react independently to events like `TASK_COMPLETED`, `ORDER_COMPLETED`, etc.

---

## Guidelines

### Use gRPC When

- You need the result before proceeding
- You're sending a command ("calculate path", "initialize inventory")
- There's a single consumer
- Failure should stop the workflow
- Latency matters (< 10ms)

### Use Kafka When

- Broadcasting events to multiple independent consumers
- Consumer can process later (analytics, audit)
- You need guaranteed delivery / durability
- Decoupling services that evolve independently
- Building audit trails or replay capability

### Async gRPC Pattern

For high-frequency calls where you need the result but can't block threads:

```java
// NavigationServiceClient uses async stub with blocking wrapper
CompletableFuture<FindPathResponse> future = new CompletableFuture<>();
asyncStub.findPath(request, new StreamObserver<>() {
    @Override
    public void onNext(FindPathResponse response) { result = response; }
    @Override
    public void onCompleted() { future.complete(result); }
});
return future.get();  // Block for result without blocking gRPC thread
```

Use this when: High frequency (>10 calls/sec) AND you need the result AND blocking causes thread starvation.

---

## Future Considerations

### When to Introduce Kafka

Add Kafka when you have one of these concrete needs:

| Need | Example |
|------|---------|
| Multiple consumers for same event | Order events → Analytics + Billing + Audit |
| Consumer offline tolerance | Dashboard disconnects, catches up on reconnect |
| Audit/compliance | Immutable log of all simulation events |
| Cross-service analytics | Aggregate metrics across simulation runs |
| Multi-warehouse coordination | Inventory changes broadcast to other warehouses |

### Hybrid Architecture (Future State)

```
┌─────────────────────────────────────────────────────────────┐
│  CRITICAL PATH (gRPC)                                       │
│                                                             │
│  FleetSimulator ──→ LayoutService                           │
│                 ──→ PutService                              │
│                 ──→ NavigationService                       │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│  SIDE EFFECTS (Kafka) - Future                              │
│                                                             │
│  FleetSimulator ──→ Kafka ──→ AnalyticsService              │
│                          ──→ AuditService                   │
│                          ──→ DashboardService               │
└─────────────────────────────────────────────────────────────┘
```

---

## Anti-Patterns

| Anti-Pattern | Problem | Correct Approach |
|--------------|---------|------------------|
| Kafka for request/response | High latency, complex error handling | Use gRPC |
| gRPC for audit logs | No durability, missed events | Use Kafka |
| Sync call to non-critical service | Couples availability | Make async or use Kafka |
| Kafka for single consumer | Unnecessary complexity | Use gRPC |
| Fire-and-forget for critical ops | Silent failures | Use gRPC with retry |

---

## FAQ

**Q: We have gRPC already. Why would we ever add Kafka?**

A: gRPC is point-to-point. When you need fan-out (one event, many consumers), durability (consumer offline), or replay (audit), Kafka is the right tool. Don't add it until you have one of these needs.

**Q: Can't we just use async gRPC instead of Kafka?**

A: Async gRPC still requires the consumer to be online. It's about non-blocking I/O, not message durability. If the consumer is down, the call fails. Kafka queues messages until the consumer recovers.

**Q: What about REST instead of gRPC?**

A: For internal service-to-service calls, gRPC is preferred: type-safe contracts, better performance, streaming support. REST is fine for external APIs or simple CRUD.

**Q: How do we handle failures in gRPC calls?**

A: Retry with exponential backoff for transient failures. Circuit breaker for sustained failures. Fail the operation explicitly if the service is critical path.

**Q: Should NavigationService calls use Kafka since they're high frequency?**

A: No. The robot cannot move without the path. High frequency doesn't mean async—it means you need efficient async gRPC (which we use). The result is still required immediately.

**Q: When would we use the Saga pattern?**

A: For distributed transactions that span multiple services and need rollback capability. Example: reserving inventory across multiple services where partial failure requires compensation. Not needed for our current read-heavy simulation flow.

---

## References

- [microservices-decomposition-design.md](./microservices-decomposition-design.md) - Service boundaries
- [polyglot-monorepo-structure-design.md](./polyglot-monorepo-structure-design.md) - Repository structure
- `.claude/skills/grpc-patterns/` - gRPC implementation guide
- `.claude/skills/architecture/` - Architecture overview
