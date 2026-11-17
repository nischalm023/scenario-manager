# Common Messaging Library Design

**Date:** 2025-12-26
**Status:** Approved
**Author:** Claude + Kumar

## Problem Statement

Currently, each service that needs to call another via gRPC must manage its own channel boilerplate:
- `ManagedChannelBuilder` setup
- Host/port configuration
- TLS vs plaintext handling
- Channel lifecycle (`@PostConstruct`, `@PreDestroy`)
- Graceful shutdown with `awaitTermination`

This leads to duplicated code across services and inconsistent patterns as the number of microservices grows.

## Solution

Create a shared `common-messaging` library that abstracts gRPC channel management. Services declare their gRPC dependencies in `application.yml` and use a factory to create stubs.

## Design

### Configuration Model

Services declare gRPC dependencies in `application.yml`:

```yaml
grpc:
  clients:
    put-service:
      host: ${PUT_SERVICE_HOST:localhost}
      port: ${PUT_SERVICE_PORT:9090}
      plaintext: true  # false in prod for TLS

    # Future services just add entries:
    pick-service:
      host: ${PICK_SERVICE_HOST:localhost}
      port: ${PICK_SERVICE_PORT:9091}
```

Configuration class:

```java
@ConfigurationProperties(prefix = "grpc")
public record GrpcClientsConfig(
    Map<String, GrpcClientConfig> clients
) {
    public record GrpcClientConfig(
        String host,
        int port,
        boolean plaintext  // default true for dev
    ) {}
}
```

### GrpcChannelFactory

Core component managing channel lifecycle:

```java
@Component
@RequiredArgsConstructor
@Slf4j
public class GrpcChannelFactory {

    private final GrpcClientsConfig config;
    private final Map<String, ManagedChannel> channels = new ConcurrentHashMap<>();

    /**
     * Get or create a blocking stub for a service.
     *
     * Usage: factory.createStub("put-service", PutServiceGrpc::newBlockingStub)
     */
    public <T> T createStub(String serviceName, Function<Channel, T> stubFactory) {
        ManagedChannel channel = channels.computeIfAbsent(serviceName, this::createChannel);
        return stubFactory.apply(channel);
    }

    private ManagedChannel createChannel(String serviceName) {
        var clientConfig = config.clients().get(serviceName);
        if (clientConfig == null) {
            throw new IllegalArgumentException("No gRPC config for: " + serviceName);
        }

        var builder = ManagedChannelBuilder
            .forAddress(clientConfig.host(), clientConfig.port());

        if (clientConfig.plaintext()) {
            builder.usePlaintext();
        }

        log.info("Created gRPC channel: {} -> {}:{}",
            serviceName, clientConfig.host(), clientConfig.port());

        return builder.build();
    }

    @PreDestroy
    void shutdown() {
        channels.forEach((name, channel) -> {
            try {
                channel.shutdown().awaitTermination(5, TimeUnit.SECONDS);
                log.info("Closed gRPC channel: {}", name);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        });
    }
}
```

Key characteristics:
- **Lazy creation** - channels created on first use
- **Cached** - same channel reused for multiple stubs
- **Auto-cleanup** - Spring calls `@PreDestroy` on shutdown

### Caller Usage

`PutServiceClient` in fleet-simulator becomes simpler:

```java
@Service
@RequiredArgsConstructor
@Slf4j
public class PutServiceClient {

    private final GrpcChannelFactory channelFactory;
    private PutServiceBlockingStub stub;

    @PostConstruct
    void init() {
        stub = channelFactory.createStub("put-service", PutServiceGrpc::newBlockingStub);
        log.info("PutServiceClient initialized");
    }

    // All domain conversion logic stays as-is:
    // - initializeInventory() with domain params
    // - buildRequest()
    // - applyResponse()
    // - helper methods
}
```

**What changes in PutServiceClient:**
- Remove: `PutServiceConfig`, `ManagedChannel`, channel setup, `@PreDestroy`
- Add: `GrpcChannelFactory` injection, one-liner stub creation
- Keep: All domain conversion logic stays in place

**Net result:** ~40 lines deleted, ~3 lines added.

### Library Structure

```
libs/java/common-messaging/
├── pom.xml
└── src/main/java/com/greyorange/greymatter/messaging/
    └── grpc/
        ├── GrpcChannelFactory.java      # Channel management
        ├── GrpcClientsConfig.java       # Config record
        └── GrpcAutoConfiguration.java   # Spring auto-config
```

**pom.xml dependencies:**
```xml
<dependencies>
    <!-- gRPC -->
    <dependency>
        <groupId>io.grpc</groupId>
        <artifactId>grpc-netty-shaded</artifactId>
    </dependency>

    <!-- Spring for @Component, @ConfigurationProperties -->
    <dependency>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-autoconfigure</artifactId>
    </dependency>
</dependencies>
```

**Auto-configuration:**
```java
@AutoConfiguration
@EnableConfigurationProperties(GrpcClientsConfig.class)
public class GrpcAutoConfiguration {

    @Bean
    @ConditionalOnMissingBean
    public GrpcChannelFactory grpcChannelFactory(GrpcClientsConfig config) {
        return new GrpcChannelFactory(config);
    }
}
```

Plus `META-INF/spring/org.springframework.boot.autoconfigure.AutoConfiguration.imports` to register it.

## Migration Plan

### Files to Create

| File | Description |
|------|-------------|
| `libs/java/common-messaging/pom.xml` | Maven module |
| `libs/java/common-messaging/src/.../grpc/GrpcChannelFactory.java` | Channel management |
| `libs/java/common-messaging/src/.../grpc/GrpcClientsConfig.java` | Config record |
| `libs/java/common-messaging/src/.../grpc/GrpcAutoConfiguration.java` | Spring auto-config |

### Files to Modify

| File | Change |
|------|--------|
| `apps/fleet-simulator/pom.xml` | Add `common-messaging` dependency |
| `apps/fleet-simulator/.../PutServiceClient.java` | Use factory, remove channel boilerplate |
| `apps/fleet-simulator/.../PutServiceConfig.java` | Delete (config moves to application.yml) |
| `apps/fleet-simulator/application.yml` | Add `grpc.clients.put-service` section |

### Documentation Updates

| File | Change |
|------|--------|
| `CLAUDE.md` | Add `common-messaging` to project structure, update "Communication Patterns" section |
| `docs/architecture/microservices-decomposition-design.md` | Add `common-messaging` library description |
| `docs/architecture/polyglot-monorepo-structure-design.md` | Update `libs/java/` section |

## Future Considerations

- **Service mesh (Envoy)**: When more services are added, consider Envoy for service discovery instead of explicit host/port configuration
- **Retries/Circuit breakers**: Can be added to `GrpcChannelFactory` as cross-cutting concerns
- **Async stubs**: Factory can support `newFutureStub` and `newStub` variants
- **Metrics/Tracing**: Interceptors can be added at the factory level

## Usage Guide (for CLAUDE.md)

### Adding a new gRPC client

1. Add config to `application.yml`:
   ```yaml
   grpc:
     clients:
       my-service:
         host: ${MY_SERVICE_HOST:localhost}
         port: ${MY_SERVICE_PORT:9090}
         plaintext: true
   ```

2. Inject and use:
   ```java
   @Autowired
   private GrpcChannelFactory channelFactory;

   @PostConstruct
   void init() {
       var stub = channelFactory.createStub("my-service", MyServiceGrpc::newBlockingStub);
   }
   ```
