# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Critical Rules

1. **ALWAYS** consult `.claude/skills/java-style-checklist/SKILL.md` before creating or modifying Java files
2. **NEVER** modify `apps/frontend/src/components/ui/` - shadcn/ui components managed externally
3. **Run** `mvn spotless:apply` before committing Java code
4. **All APIs** use camelCase JSON (not snake_case)
5. **Run tests** for affected services before committing (`npx nx test [service-name]`)

---

## Project Overview

GTP (Goods-to-Person) Robotics Queue Simulator - discrete event simulator for warehouse operations built with Java/Desmo-J and Spring Boot. Models robot movements, operator workloads, and system bottlenecks.

**Architecture**: Polyglot monorepo with microservices communicating via gRPC.

```
queue_simulator/
├── apps/
│   ├── fleet-simulator/     # Java: Simulation engine (port 8085)
│   ├── layout-service/      # Java: Layout/map service (port 8082, gRPC 9091)
│   ├── navigation-service/  # Java: A* pathfinding (port 8083, gRPC 9092)
│   ├── put-service/         # Java: Storage algorithms (port 8081, gRPC 9090)
│   └── frontend/            # React TypeScript UI (port 4200 dev, nginx in prod)
├── infrastructure/
│   ├── envoy/               # API Gateway (port 8080)
│   └── nginx/               # Static frontend serving (port 8090 in prod)
├── libs/java/               # Shared libraries (common-domain, grpc-stubs)
├── libs/proto/              # gRPC proto definitions
└── docs/, specs/            # Documentation and specifications
```

---

## Quick Start

```bash
# Start services (in order - put-service must be ready first)
cd apps/put-service && mvn spring-boot:run        # Terminal 1
cd apps/layout-service && mvn spring-boot:run     # Terminal 2
cd apps/navigation-service && mvn spring-boot:run # Terminal 3
cd apps/fleet-simulator && mvn spring-boot:run    # Terminal 4
npx nx serve envoy                                # Terminal 5
cd apps/frontend && npm run dev                   # Terminal 6

# Format code before commit
cd apps/fleet-simulator && mvn spotless:apply
cd apps/put-service && mvn spotless:apply
cd apps/layout-service && mvn spotless:apply
cd apps/navigation-service && mvn spotless:apply

# Run tests (REQUIRED before committing)
npx nx test fleet-simulator      # 24 test files
npx nx test put-service          # 79 tests
npx nx test layout-service       # 107 tests
npx nx test navigation-service   # 40 tests
npx nx test frontend             # 10 test files

# Regenerate frontend types after DTO changes (requires fleet-simulator + layout-service running)
node scripts/generate-merged-api-types.js

# Full quality check (format + compile + test + static analysis)
cd apps/<service> && mvn clean verify

# Coverage report only
cd apps/<service> && mvn test jacoco:report
# View: open apps/<service>/target/site/jacoco/index.html

# SpotBugs report only
cd apps/<service> && mvn compile spotbugs:spotbugs
# View: open apps/<service>/target/spotbugs.html

# Skip analysis during dev (faster builds)
cd apps/<service> && mvn compile -Dspotbugs.skip=true -Djacoco.skip=true
```

---

## Skills Reference

Skills are auto-activated by hooks based on your prompt. Consult directly when needed:

| Skill | When to Use |
|-------|-------------|
| `.claude/skills/java-backend/` | Editing Java files, Desmo-J patterns, metrics integration |
| `.claude/skills/java-style-checklist/` | Java style guide: organization, comments, modern Java 21+ |
| `.claude/skills/commands/` | Running services, builds, tests, Docker commands |
| `.claude/skills/architecture/` | Service boundaries, gRPC communication patterns |
| `.claude/skills/frontend/` | React/TypeScript, type generation, shadcn/ui |
| `.claude/skills/debugging/` | Service issues, common errors, health checks |
| `.claude/skills/deployment/` | Cloud Run deployment, Docker Compose, CI/CD pipelines |
| `.claude/skills/relay-system/` | VTM/HTM robots, relay positions, zone capacity |
| `.claude/skills/feature-guides/` | Set cover, animation playback, algorithm comparison |
| `.claude/skills/grpc-patterns/` | Adding new gRPC clients, proto definitions |
| `.claude/skills/put-benchmark/` | After major put-service changes, validate algorithm performance |

---

## Key Documents

| Document | Purpose |
|----------|---------|
| `.claude/skills/java-style-checklist/` | Java style guide (SKILL.md + references/) |
| `docs/architecture/microservices-decomposition-design.md` | Service boundaries |
| `docs/architecture/polyglot-monorepo-structure-design.md` | Directory structure |
| `specs/relay_system/` | Relay system design and implementation notes |
| `docs/plans/` | Implementation plans (Phase 1-4) |

---

## Development Methodology

**Spec-Driven Development**:
1. Create `specs/feature_name/plan.md` with requirements, architecture, tasks
2. Review and approve before implementation
3. Validate post-implementation against plan

---

## Testing

**IMPORTANT: Always run tests for affected services before committing.**

### Unit Tests

| Service | Command | Tests |
|---------|---------|-------|
| fleet-simulator | `npx nx test fleet-simulator` | 24 files |
| put-service | `npx nx test put-service` | 79 tests |
| layout-service | `npx nx test layout-service` | 107 tests |
| navigation-service | `npx nx test navigation-service` | 40 tests |
| frontend | `npx nx test frontend` | 10 files |

```bash
# Run all service tests
npx nx run-many --target=test --all

# Run specific test class (Java services)
cd apps/[service] && mvn test -Dtest=TestClassName

# Pass null for PutServiceClient in unit tests
PutServiceClient putServiceClient = null;  // WarehouseBuilder handles gracefully
```

### Integration Testing

**Starting the Cluster:**

Option 1 - Manual (recommended for local dev):
```bash
# Start services in separate terminals (in order)
cd apps/put-service && mvn spring-boot:run        # Terminal 1
cd apps/layout-service && mvn spring-boot:run     # Terminal 2
cd apps/navigation-service && mvn spring-boot:run # Terminal 3
cd apps/fleet-simulator && mvn spring-boot:run    # Terminal 4

# Verify health
curl http://localhost:8081/actuator/health  # put-service
curl http://localhost:8082/actuator/health  # layout-service
curl http://localhost:8083/actuator/health  # navigation-service
curl http://localhost:8085/actuator/health  # fleet-simulator
```

Option 2 - Docker Compose:
```bash
docker-compose -f docker-compose.dev.yml up -d
docker-compose -f docker-compose.dev.yml ps
```

**Test Configs** (`apps/fleet-simulator/src/test/resources/`):

| Config | System Type | Purpose |
|--------|-------------|---------|
| `smoke-test-rtp.json` | RTP | Quick cluster verification (~1 min) |
| `smoke-test-relay.json` | Relay | Quick cluster verification (~1 min) |
| `integration-test-config.json` | RTP | Full RTP integration test |
| `relay-test-config.json` | Relay | Full Relay integration test |

**Running Integration Tests:**
```bash
# Test RTP system
curl -X POST http://localhost:8085/api/simulation/run \
  -H "Content-Type: application/json" \
  -d @apps/fleet-simulator/src/test/resources/smoke-test-rtp.json

# Test Relay system
curl -X POST http://localhost:8085/api/simulation/run \
  -H "Content-Type: application/json" \
  -d @apps/fleet-simulator/src/test/resources/smoke-test-relay.json
```

**Stopping the Cluster:**
```bash
# Manual: Ctrl+C in each terminal
# Docker: docker-compose -f docker-compose.dev.yml down
```

**Config Constraints:**
- `binsCount`: min 4
- `rackBlockConfig.rackColumns`: 1-5
- `orderSelectionAlgorithm`: `maximize_ppf_directly`, `maximize_ppf_with_tiering`, `maximize_tpq_with_vut`
- Relay requires: `numVtms`, `numHtms`, `vtmSpeed`, `htmSpeed` at top level

---

## Configuration

- **Java**: 21
- **Line length**: 100 chars (Google Java Format)
- **Frameworks**: Spring Boot 3.2, Desmo-J 2.5.1e, React 19

---

## TypeScript LSP

Use the TypeScript Language Service for frontend code analysis (refactoring, unused code, type checking). Run from `apps/frontend/`:

```js
// Require from repo root node_modules
const ts = require('/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/node_modules/typescript');
```

- Use `tsconfig.app.json` (not `tsconfig.json`) for frontend files
- **findReferences** — semantic reference lookup before renaming/deleting symbols
- **getSemanticDiagnostics** — type errors and unused imports (enable `noUnusedLocals: true`)
- Diagnostic codes: TS6133 (declared but never read), TS6196/6198/6199 (unused variants)

Use LSP when: cross-file refactoring, verifying edits, finding callers, unused code detection.
Skip LSP when: simple single-line fixes, adding new code, unique string is easily greppable.

## Rust: no inline qualified type paths

Import types with `use` instead of writing qualified paths inline
(`SimTime`, not `crate::model::SimTime`). Checker:
`scripts/find-qualified-paths.py --check` (exit 1 on violations, with fix
rules in the message). Enforced at commit time once per clone via:

```bash
make hooks   # sets core.hooksPath -> .githooks (pre-commit gate)
```

Claude Code also gets same-turn feedback via the PostToolUse hook in
`.claude/settings.json`. Exceptions the checker already knows: macro_rules!
bodies (must stay qualified — they resolve at expansion sites), idiomatic
families (`fmt::`/`io::` Result, `mpsc::`, `syn::Result`, egui/wgpu/eframe,
derive position), and lines waived with `// keep-qualified: <reason>`.

## Module doc comments (charter rule)

Every file touched by any change gets a module doc comment (`//!`) stating:
(1) what the module's noun MEANS relative to its parent module, and (2) its
specific purpose — plainly spoken, no vague word-alignment with the filename
("integrates aisle snapshots into capacity-time ratios per bucket/zone", never
"utilization stuff"). If the honest purpose is murky, overstuffed, or plural,
write a `TODO(overstuffed)`/`TODO(module)` in that doc calling for the rethink
instead of a vague blurb. The doc is a falsifiable charter: code that doesn't
match it is visibly misplaced — use it to drive relocation over time.
