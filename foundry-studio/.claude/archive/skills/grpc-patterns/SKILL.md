---
name: grpc-patterns
description: Use when adding, modifying, or configuring gRPC clients or proto definitions.
  Step-by-step implementation guide for proto → stub → client → config workflow.
  For gRPC debugging, use debugging skill.
---

# gRPC Patterns

## gRPC Channel Management

The `common-messaging` library (`libs/java/common-messaging/`) provides centralized gRPC channel management.

### Key Components

| Component | Purpose |
|-----------|---------|
| `GrpcChannelFactory` | Lazy channel creation, caching, graceful shutdown |
| `GrpcClientsConfig` | Configuration from `application.yml` |
| `GrpcAutoConfiguration` | Spring Boot auto-configuration |

### How It Works

```java
GrpcChannelFactory @Component (manages all gRPC channels)
  ↓ injected into
ServiceClient @Service (creates stub via factory)
  ↓ injected into
SimulationService / ComparisonService
  ↓ passed to
Simulator → WarehouseBuilder → calls client methods
```

### Two Stub Creation Patterns

**Pattern A — `createStub()` (preferred for single stub type):**
```java
stub = channelFactory.createStub("service-name", ServiceGrpc::newBlockingStub);
```

**Pattern B — `getChannel()` (needed for multiple stub types from one channel):**
```java
var channel = channelFactory.getChannel("service-name");
blockingStub = ServiceGrpc.newBlockingStub(channel);
asyncStub = ServiceGrpc.newStub(channel);
```

Use Pattern B when you need both blocking and async stubs (see [Async Stub Pattern](#async-stub-pattern)).

## Adding a New gRPC Client

### Step 1: Define Proto

Create proto file at `libs/proto/src/grpc/<service>/v1/<service>.proto`:

```protobuf
syntax = "proto3";

package grpc.<service>.v1;

option java_package = "com.greyorange.grpc.<service>.v1";
option java_multiple_files = true;

service MyService {
  rpc MyMethod (MyRequest) returns (MyResponse);
}

message MyRequest {
  string id = 1;
}

message MyResponse {
  string result = 1;
}
```

### Step 2: Regenerate Stubs

```bash
npx nx build grpc-stubs
```

Note: Both `generate` and `build` targets have `cache: false` — regeneration always runs fresh.

### Step 3: Add Configuration

In the client service's `application.yml`:

```yaml
grpc:
  clients:
    my-service:
      host: ${MY_SERVICE_HOST:localhost}
      port: ${MY_SERVICE_PORT:9090}
      plaintext: true  # false in prod for TLS
```

### Step 4: Create Client Service

**Standard blocking client (most cases):**

```java
@Service
@RequiredArgsConstructor
@Slf4j
public class MyServiceClient {
    private final GrpcChannelFactory channelFactory;
    private MyServiceBlockingStub stub;

    @PostConstruct
    void init() {
        stub = channelFactory.createStub("my-service", MyServiceGrpc::newBlockingStub);
        log.info("MyServiceClient initialized");
    }

    public MyResponse callMethod(String id) {
        MyRequest request = MyRequest.newBuilder()
            .setId(id)
            .build();
        return stub.myMethod(request);
    }
}
```

**Dual-stub client (when async calls are needed):**

```java
@Service
@RequiredArgsConstructor
@Slf4j
public class MyServiceClient {
    private static final String SERVICE_NAME = "my-service";

    private final GrpcChannelFactory channelFactory;
    private MyServiceBlockingStub blockingStub;
    private MyServiceStub asyncStub;

    @PostConstruct
    void init() {
        var channel = channelFactory.getChannel(SERVICE_NAME);
        blockingStub = MyServiceGrpc.newBlockingStub(channel);
        asyncStub = MyServiceGrpc.newStub(channel);
        log.info("MyServiceClient initialized");
    }
}
```

### Step 5: Inject and Use

```java
@Service
@RequiredArgsConstructor
public class BusinessService {
    private final MyServiceClient myServiceClient;

    public void doSomething(String id) {
        MyResponse response = myServiceClient.callMethod(id);
        // Use response
    }
}
```

## Async Stub Pattern

**CRITICAL: Blocking gRPC stubs hang inside Desmo-J `lifeCycle()` methods.** This was empirically verified — the simulation thread deadlocks. Any gRPC call made during simulation execution must use the async stub pattern.

### When to Use Which Stub

| Context | Stub Type | Reason |
|---------|-----------|--------|
| Setup/teardown (outside simulation) | Blocking | Simpler, runs on Spring-managed threads |
| Inside `lifeCycle()` / during simulation | Async | Blocking stub hangs Desmo-J simulation thread |

### Async Pattern (from `NavigationServiceClient.findPath()`)

```java
public FindPathResponse findPath(FindPathRequest request) {
    CompletableFuture<FindPathResponse> future = new CompletableFuture<>();

    asyncStub.findPath(
        request,
        new StreamObserver<>() {
            private FindPathResponse result;

            @Override
            public void onNext(FindPathResponse response) {
                result = response;
            }

            @Override
            public void onError(Throwable t) {
                future.completeExceptionally(t);
            }

            @Override
            public void onCompleted() {
                future.complete(result);
            }
        });

    return await(future);
}

private <T> T await(CompletableFuture<T> future) {
    try {
        return future.get();
    } catch (InterruptedException e) {
        Thread.currentThread().interrupt();
        throw new RuntimeException("Thread interrupted during gRPC call", e);
    } catch (ExecutionException e) {
        Throwable cause = e.getCause();
        if (cause instanceof RuntimeException) {
            throw (RuntimeException) cause;
        }
        throw new RuntimeException("gRPC call failed", cause);
    }
}
```

Key points:
- `StreamObserver` captures the single response into a `CompletableFuture`
- `future.get()` blocks the calling thread but does NOT hang Desmo-J (unlike blocking stub)
- The `await()` helper unwraps `ExecutionException` and preserves interrupt status

## Existing gRPC Clients

### fleet-simulator Clients

| Client | Stub Type | Methods | Config Key |
|--------|-----------|---------|------------|
| `PutServiceClient` | Blocking | `initializeInventory()` | `put-service` (port 9090) |
| `NavigationServiceClient` | Blocking + Async | `createSession()`, `findPath()`, `deleteSession()` | `navigation-service` (port 9092) |
| `LayoutServiceClient` | Blocking | `getLayout()`, `getNavigationData()` | `layout-service` (port 9091) |

All three live in `apps/fleet-simulator/src/.../adapter/`.

**fleet-simulator `application.yml` config:**
```yaml
grpc:
  clients:
    put-service:
      host: ${PUT_SERVICE_HOST:localhost}
      port: ${PUT_SERVICE_PORT:9090}
      plaintext: true
    layout-service:
      host: ${LAYOUT_SERVICE_HOST:localhost}
      port: ${LAYOUT_SERVICE_PORT:9091}
      plaintext: true
    navigation-service:
      host: ${NAVIGATION_SERVICE_HOST:localhost}
      port: ${NAVIGATION_SERVICE_PORT:9092}
      plaintext: true
```

**NavigationServiceClient** is the only dual-stub client. It uses:
- Blocking stub for `createSession()` and `deleteSession()` (called outside simulation)
- Async stub for `findPath()` (called inside Desmo-J `lifeCycle()`)

### layout-service Client

| Client | Stub Type | Methods | Config Key |
|--------|-----------|---------|------------|
| `PutServiceClient` | Blocking | `calculatePopularityScores()` | `put-service` (port 9090) |

Located at `apps/layout-service/src/.../client/PutServiceClient.java`.

Notable pattern — safe error handling at gRPC boundary:
```java
try {
    response = blockingStub.calculatePopularityScores(request);
} catch (StatusRuntimeException e) {
    log.error("gRPC call to put-service failed: status={}, message={}",
        e.getStatus().getCode(), e.getMessage());
    return Map.of();  // Safe default
}
```

## Proto Types

**Common types** (`libs/proto/src/grpc/common/v1/types.proto`):

| Proto Type | Java Type | Notes |
|------------|-----------|-------|
| `GridCoordinate` | `GridCoordinate` | Proto: (x=col, y=row), Domain: (row, col) |
| `SystemType` | `SystemType` | RTP or RELAY enum |

**Conversion**: Use `ProtoConverters` utility class for domain ↔ proto conversions.

## Proto Locations

| Path | Purpose |
|------|---------|
| `libs/proto/src/grpc/put/v1/` | Put service protos (`initializeInventory`, `calculatePopularityScores`) |
| `libs/proto/src/grpc/layout/v1/` | Layout service protos (`getLayout`, `getNavigationData`) |
| `libs/proto/src/grpc/navigation/v1/` | Navigation service protos (`createSession`, `findPath`, `deleteSession`) |
| `libs/proto/src/grpc/common/v1/` | Shared types (`GridCoordinate`, `SystemType`) |
| `libs/java/grpc-stubs/` | Generated Java stubs (output of `npx nx build grpc-stubs`) |

## Testing with gRPC

### In Unit Tests

Pass `null` for gRPC clients — `WarehouseBuilder` handles gracefully:

```java
// In test setup
PutServiceClient putServiceClient = null;

// WarehouseBuilder checks for null
if (putServiceClient != null) {
    putServiceClient.initializeInventory(config);
}
```

### Integration Tests

Start the gRPC server before tests:

```bash
cd apps/put-service && mvn spring-boot:run &
sleep 5
cd apps/fleet-simulator && mvn test
```

> **Note**: For gRPC debugging and troubleshooting, see `.claude/skills/debugging/`

## Related Skills

- `.claude/skills/architecture/` — Service boundaries
- `.claude/skills/debugging/` — Troubleshooting
- `.claude/skills/commands/` — Build commands
- `.claude/skills/java-backend/` — Desmo-J process patterns (references async stub gotcha)
