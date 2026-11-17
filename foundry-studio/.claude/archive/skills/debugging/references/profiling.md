# Profiling & Performance Guide

## Prerequisites

Install async-profiler (macOS):
```bash
brew tap bric3/homebrew-tap && brew install async-profiler
```

Verify installation:
```bash
asprof --version
```

---

## Profiling Modes

### CPU Profiling (Default)
Shows where CPU time is spent. Best for finding computational bottlenecks.

```bash
./scripts/profile-fleet-simulator.sh cpu 60
```

### Wall-Clock Profiling
Includes I/O waits, locks, and blocking operations. Best for finding latency issues.

```bash
./scripts/profile-fleet-simulator.sh wall 60
```

### Allocation Profiling
Shows memory allocation hotspots. Best for GC pressure issues.

```bash
./scripts/profile-fleet-simulator.sh alloc 60
```

### Lock Profiling
Shows lock contention. Best for concurrency bottlenecks.

```bash
./scripts/profile-fleet-simulator.sh lock 60
```

---

## Interactive Profiling

For profiling specific simulation runs:

```bash
# Terminal 1: Start fleet-simulator
cd apps/fleet-simulator && mvn spring-boot:run

# Terminal 2: Start profiling (before triggering simulation)
./scripts/profile-control.sh start cpu

# Terminal 3: Trigger simulation
curl -X POST http://localhost:8085/api/simulation/run \
  -H "Content-Type: application/json" \
  -d @apps/fleet-simulator/src/test/resources/smoke-test-relay.json

# Terminal 2: Stop profiling when simulation completes
./scripts/profile-control.sh stop
```

Output: `profiling-output/flamegraph_cpu_<timestamp>.html`

---

## Interpreting Flamegraphs

1. **Width = Time**: Wider bars indicate more CPU/time spent
2. **Stack grows upward**: Bottom = entry point, top = leaf methods
3. **Look for wide plateaus**: These are hotspots
4. **Color coding**: Same color = same package/class

### Common Patterns to Look For

| Pattern | Indicates | Action |
|---------|-----------|--------|
| Wide `stream()` bars | O(n) filtering in hot path | Replace with indexed lookup |
| `HashMap.get` spikes | Hash collisions or frequent lookups | Consider caching |
| `synchronized` blocks | Lock contention | Reduce lock scope or use concurrent structures |
| gRPC/network calls | I/O latency | Add caching or batching |
| GC activity | Memory pressure | Check allocation profiling |

---

## JVM Tuning for Large Simulations

### Heap Sizing

| Simulation Scale | Recommended Heap |
|------------------|------------------|
| Small (20 robots, 1h) | `-Xms512m -Xmx1g` |
| Medium (60 robots, 1h) | `-Xms1g -Xmx2g` |
| Large (150+ robots, 8h) | `-Xms2g -Xmx4g` |

### G1 Garbage Collector (Recommended)

```bash
MAVEN_OPTS="-Xms2g -Xmx4g -XX:+UseG1GC -XX:MaxGCPauseMillis=200" \
  mvn spring-boot:run
```

### Java Flight Recorder (JFR)

Built-in, low-overhead profiling:

```bash
# Start with JFR recording
MAVEN_OPTS="-XX:StartFlightRecording=duration=300s,filename=simulation.jfr" \
  mvn spring-boot:run

# Analyze with JDK Mission Control
jmc simulation.jfr
```

---

## Known Bottlenecks (Large Relay Simulations)

| Bottleneck | Location | Complexity | Description |
|------------|----------|------------|-------------|
| Tote finding | `DesmojVtmProcess.java:467-494` | O(total_totes) | Stream filtering all totes per VTM cycle |
| Relay selection | `RelayPositionManager.java:123-180` | O(relays) + lock | Synchronized reservation with full scan |
| gRPC pathfinding | `NavigationServiceClient.java:78` | O(1) + network | Blocking RPC per robot movement |
| Zone congestion | `DesmojVtmProcess.java:184-205` | O(zones × relays) | Repeated stream operations |
| TPQ measurement | `DesmojBinManagementProcess.java:96` | O(totes + racks) | Full warehouse scan every 5s |

### Optimization Strategies

| Strategy | Approach |
|----------|----------|
| **Index by zone** | `Map<ZoneId, Set<Tote>>` instead of filtering all totes |
| **Cache paths** | LRU cache for common source→destination pairs |
| **Batch reservations** | Zone-level batching instead of per-relay synchronized |
| **Lazy TPQ** | On-demand calculation instead of periodic |
| **Pre-compute** | Initialize `Map<ZoneId, List<Rack>>` once at startup |

---

## Benchmark Tests

Run the relay simulation benchmark to measure performance:

```bash
cd apps/fleet-simulator && mvn test -Dtest=RelaySimulationBenchmarkTest

# Exclude benchmarks from regular test runs
mvn test -DexcludedGroups=benchmark
```

See `apps/fleet-simulator/src/test/java/.../benchmark/` for benchmark implementations.
