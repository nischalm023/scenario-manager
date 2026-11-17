---
name: put-benchmark
description: Use after major changes to put-service algorithms (GreedyPutStrategy, ABC classification, GridPopularityManager) to validate performance. Run benchmark to measure execution time with 10K storage spaces and 6K SKUs.
---

# PUT Algorithm Benchmark

---

## When to Use

Run the benchmark after:
- Modifying `GreedyPutStrategy` or `MinWeightedCycleDistancePutStrategy`
- Changing `AbstractPutStrategy` base class (ABC classification, capacity limits)
- Updating `GridPopularityManager` scoring logic
- Adding new PUT algorithms
- Changing storage space or SKU handling logic

---

## Quick Start

```bash
# Run benchmark (from repo root or put-service directory)
cd apps/put-service && mvn test -Dtest=PutAlgorithmBenchmarkTest

# Expected output format:
# === PUT ALGORITHM BENCHMARK ===
# Configuration:
#   Storage Spaces: 10,000
#   SKUs: 6,000
#   System Type: RELAY
#   Algorithm: GREEDY
#
# Results:
#   Execution Time: ~3,000 ms
#   SKUs Placed: 6,000 / 6,000
#   Units Placed: ~8,000
#   Assignments: 6,000
# ===============================
```

---

## Benchmark Configuration

| Parameter | Value | Notes |
|-----------|-------|-------|
| Storage Spaces | 10,000 | Tote locations in 100x18 grid |
| SKUs | 6,000 | ABC distribution (20/30/50%) |
| Tote Capacity | 6 units | Per-tote storage limit |
| Stations | 4 | Bottom edge of field |
| System Type | RELAY | Single SKU per tote |
| Random Seed | 42 | Deterministic results |

---

## Performance Expectations

| Metric | Acceptable | Warning | Critical |
|--------|------------|---------|----------|
| Execution Time | < 5s | 5-10s | > 10s |
| SKUs Placed | 100% | > 95% | < 95% |
| Algorithm Complexity | O(n*m) | - | > O(n*m) |

---

## Excluding from Regular Tests

The benchmark uses `@Tag("benchmark")` and can be excluded:

```bash
# Run all tests EXCEPT benchmarks
cd apps/put-service && mvn test -DexcludedGroups=benchmark

# Run ONLY benchmarks
cd apps/put-service && mvn test -Dgroups=benchmark
```

---

## Key Files

| File | Purpose |
|------|---------|
| `apps/put-service/src/test/java/com/greyorange/put/benchmark/PutAlgorithmBenchmarkTest.java` | Benchmark test |
| `apps/put-service/src/main/java/com/greyorange/put/domain/putstrategies/GreedyPutStrategy.java` | Algorithm under test |
| `apps/put-service/src/main/java/com/greyorange/put/domain/putstrategies/AbstractPutStrategy.java` | ABC classification, base logic |
| `apps/put-service/src/main/java/com/greyorange/put/domain/GridPopularityManager.java` | Popularity scoring |

---

## Interpreting Results

### Healthy Output
```
ABC Classification: A=1200 SKUs, B=1800 SKUs, C=3000 SKUs
Per-storage-space limits: Category A ≤ 4 units, Category B ≤ 3 units, Category C ≤ 1 units
Placed 6000 SKUs, 8061 total units across 10000 storage spaces
```

### Warning Signs
- `Placed 0 SKUs` → Check tote capacity vs ABC utilization limits
- Execution time > 10s → Algorithm regression, check sorting/iteration
- SKUs not fully placed → Check storage space capacity or ABC limits

---

## Debugging Benchmark Issues

```bash
# Check exported CSV for placement details
cat /tmp/put_assignment_debug.csv

# Run with verbose logging
cd apps/put-service && mvn test -Dtest=PutAlgorithmBenchmarkTest -X
```

---

## Related Skills

- `.claude/skills/java-backend/` - PUT strategy patterns
- `.claude/skills/java-style-checklist/` - Before committing changes
- `.claude/skills/commands/` - Test commands
