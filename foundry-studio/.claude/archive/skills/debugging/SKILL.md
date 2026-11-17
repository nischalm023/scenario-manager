---
name: debugging
description: Use when something is broken or behaving unexpectedly. Error patterns, common issues, and fixes. For running commands, use commands skill.
---

# Debugging Guide

---

## Debug Mode with Log Files

Use `make run-debug` to start services with separate log files:

```bash
make run-debug   # Starts all services, logs to /tmp/des-logs/
make stop        # Stop all services when done
```

**Log locations:**
- `/tmp/des-logs/put-service.log`
- `/tmp/des-logs/layout-service.log`
- `/tmp/des-logs/fleet-simulator.log`
- `/tmp/des-logs/envoy.log`

**Debugging commands:**
```bash
# Watch all logs live
tail -f /tmp/des-logs/*.log

# Search for errors across services
grep -i error /tmp/des-logs/*.log
grep -i exception /tmp/des-logs/*.log

# Check specific service
tail -100 /tmp/des-logs/fleet-simulator.log

# Find stack traces
grep -A 20 "Exception" /tmp/des-logs/fleet-simulator.log
```

**Note**: Logs are reset on each `make run-debug` invocation.

> **Note**: For service startup order and health check commands, see `.claude/skills/commands/`

---

## Common Issues

| Problem | Cause | Solution |
|---------|-------|----------|
| **gRPC Connection Refused** | put-service not running | Start put-service before fleet-simulator |
| **Bytecode/Class Errors** | Stale compiled classes | `mvn clean compile -DskipTests` |
| **Processes Not Running** | Wrong initialization | Check `doInitialSchedules()` not `init()` |
| **Test Compilation Fails** | Missing null handling | Pass `null` for `PutServiceClient` in tests |
| **Scenarios Not Loading** | Missing env var | Check `SCENARIOS_PATH` in Docker |
| **Metrics Showing Zero** | Event data extraction | Check event.getData() for null |
| **Frontend 502 errors** | Envoy not running | Start Envoy: `npx nx serve envoy` |
| **Type errors after DTO change** | Stale types | Regenerate: `npx openapi-typescript ...` |

---

## Clean Build

When you see weird errors, always try a clean build first:

```bash
# Java services
cd apps/fleet-simulator && mvn clean compile -DskipTests
cd apps/put-service && mvn clean compile -DskipTests
cd apps/layout-service && mvn clean compile -DskipTests

# Frontend
cd apps/frontend && rm -rf node_modules/.cache && npm run build
```

---

## gRPC Debugging

**Connection issues:**
```bash
# Check if gRPC port is open
lsof -i :9090  # put-service gRPC
lsof -i :9091  # layout-service gRPC
```

**Proto regeneration:**
```bash
npx nx build grpc-stubs
mvn clean compile -DskipTests  # In each Java service
```

---

## Simulation Debugging

For Desmo-J lifecycle rules (`doInitialSchedules`, `SimFinishedException`, process scheduling), see `.claude/skills/java-backend/`.

**Metrics not recording:**
- Verify `@MetricHandler` annotation on collector methods
- Check event type matches: `MetricEvent.RACK_CYCLE_COMPLETED`
- Extract data defensively: `event.getData().get("key")`

---

## Docker Debugging

```bash
# View logs
docker-compose -f docker-compose.dev.yml logs -f

# Specific service
docker-compose -f docker-compose.dev.yml logs -f fleet-simulator

# Rebuild images
./docker-build.sh
docker-compose -f docker-compose.dev.yml up --build
```

---

## Profiling & Performance

For large simulations, use async-profiler to identify bottlenecks:

```bash
./scripts/profile-fleet-simulator.sh cpu 60   # CPU profiling (60s)
./scripts/profile-fleet-simulator.sh wall 60  # Wall-clock (includes I/O waits)
./scripts/profile-control.sh start cpu        # Interactive start
./scripts/profile-control.sh stop             # Stop and save flamegraph
```

Output: HTML flamegraph in `profiling-output/`

For detailed profiling guidance, JVM tuning, and known bottlenecks, see `references/profiling.md`.

---

## Related Skills

- `.claude/skills/commands/` - Start commands and health checks
- `.claude/skills/java-backend/` - Desmo-J patterns
- `.claude/skills/architecture/` - Service dependencies
