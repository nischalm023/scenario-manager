---
name: commands
description: Use when you need to run any command - build, test, start, stop, format. Complete command reference. For troubleshooting errors, use debugging skill.
---

# Development Commands

---

## Quick Start - Most Common

```bash
# Start all services (in separate terminals)
cd apps/put-service && mvn spring-boot:run        # Port 8081 REST, 9090 gRPC
cd apps/layout-service && mvn spring-boot:run    # Port 8082 REST, 9091 gRPC
cd apps/fleet-simulator && mvn spring-boot:run   # Port 8085 REST
npx nx serve envoy                                # Port 8080 (API Gateway)
cd apps/frontend && npm run dev                   # Port 5173

# Format code before commit
cd apps/fleet-simulator && mvn spotless:apply
cd apps/put-service && mvn spotless:apply
cd apps/layout-service && mvn spotless:apply

# Run tests
npx nx test fleet-simulator
npx nx test put-service
```

> **Note**: For service architecture overview, see `.claude/skills/architecture/`

**Start Order**: put-service → layout-service → fleet-simulator → envoy → frontend

---

## Per-Service Commands

### Fleet Simulator (`apps/fleet-simulator/`)
```bash
mvn spring-boot:run          # Start server (port 8085)
mvn clean compile            # Compile
mvn test                     # Run tests
mvn spotless:apply           # Format code
mvn spotless:check           # Check formatting
```

### Layout Service (`apps/layout-service/`)
```bash
mvn spring-boot:run          # Start server (port 8082 REST, 9091 gRPC)
mvn clean compile            # Compile
mvn test                     # Run tests
mvn spotless:apply           # Format code
```

### Put Service (`apps/put-service/`)
```bash
mvn spring-boot:run          # Start gRPC + REST server (8081, 9090)
mvn clean compile            # Compile
mvn test                     # Run tests
mvn spotless:apply           # Format code
```

### Frontend (`apps/frontend/`)
```bash
npm run dev                  # Development server
npm run check:fix            # Lint and format (Biome)
npm run build                # Production build

# Regenerate TypeScript types from OpenAPI
npx openapi-typescript http://localhost:8085/openapi.json -o apps/frontend/src/types/generated-api.ts

# Add shadcn/ui components
npx shadcn@latest add <component>
```

---

## Envoy (API Gateway)

```bash
npx nx serve envoy       # Start Envoy for local dev
npx nx stop envoy        # Stop Envoy container
npx nx logs envoy        # View Envoy logs
npx nx admin envoy       # Open Envoy admin UI (localhost:9901)
```

---

## Build Order (Dependencies)

```bash
# 1. Build shared libraries first
npx nx build common-domain
npx nx build grpc-stubs

# 2. Build services
npx nx build put-service
npx nx build layout-service
npx nx build fleet-simulator
```

---

## Docker Commands

```bash
# Build all images (uses Jib - no Dockerfiles needed)
./docker-build.sh

# Build individual service
cd apps/put-service && mvn compile jib:dockerBuild
cd apps/fleet-simulator && mvn compile jib:dockerBuild

# Start with Docker Compose
docker-compose -f docker-compose.dev.yml up
```

---

## Testing

```bash
# Run all tests
npx nx run-many --target=test

# Individual service tests
npx nx test fleet-simulator
npx nx test put-service

# Test config location
# apps/fleet-simulator/src/test/resources/integration-test-config.json
```

---

## Health Checks

```bash
curl http://localhost:8081/actuator/health  # put-service
curl http://localhost:8082/actuator/health  # layout-service
curl http://localhost:8085/health           # fleet-simulator
curl http://localhost:8080/health           # envoy (routes to fleet-simulator)
```

---

## Makefile Targets

```bash
make deps      # Install all dependencies (asdf + npm)
make build     # Build all projects
make run       # Start all services locally (interactive, interleaved output)
make run-debug # Start services with logs to /tmp/des-logs/ (for debugging)
make test      # Run all tests
make format    # Format code
make clean     # Clean build artifacts
make stop      # Stop all running services
make help      # Show available commands
```

For debug mode log locations and debugging commands, see `.claude/skills/debugging/`.

---

## NX Commands

```bash
npx nx show projects              # List all projects
npx nx graph                      # Visualize dependency graph
npx nx show project <name>        # Show project details
npx nx run-many --target=build    # Build all affected
npx nx affected --target=test     # Test affected projects
```

---

## Related Skills

- `.claude/skills/debugging/` - When services don't start
- `.claude/skills/architecture/` - Understanding service boundaries
