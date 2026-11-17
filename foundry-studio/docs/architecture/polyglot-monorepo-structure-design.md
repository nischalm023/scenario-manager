# Polyglot Monorepo Structure Design

**Status**: Proposed  
**Created**: 2024-12-23  
**Author**: Architecture Team  
**Relates to**: [Microservices Decomposition Design](./microservices-decomposition-design.md)

---

## Executive Summary

This document defines the directory structure and build orchestration strategy for evolving the queue simulator from a monolithic Java backend into a polyglot microservices architecture. The design prioritizes:

1. **Language-agnostic project organization** — Services are first-class citizens regardless of implementation language
2. **NX as the universal orchestrator** — Unified task running, caching, and dependency graphing across all languages
3. **Scoped language ecosystems** — Each language maintains its own idioms (Maven for Java, pip/poetry for Python, rebar for Erlang) within designated library directories
4. **Proto-first contracts** — gRPC/Protobuf definitions are language-agnostic and generate stubs for each ecosystem

---

## Problem Statement

### Current State

The repository currently has:

```
queue_simulator/
├── apps/
│   ├── backend/           # Monolithic Java (Spring Boot + Desmo-J)
│   │   └── pom.xml
│   └── frontend/          # TypeScript/React
├── libs/
│   └── api-schema/        # OpenAPI spec
└── nx.json
```

### Future Requirements

As we decompose the monolith and expand the platform, we anticipate:

- **Multiple Java services**: Fleet Simulator, Put Service, Pick Service, Inventory Service, etc.
- **Python services**: ML-based demand prediction, optimization algorithms
- **Erlang/Elixir services**: High-availability messaging, real-time coordination
- **Shared contracts**: gRPC proto definitions consumed by all languages

### The Core Tension

If we place a parent `pom.xml` at the workspace root (standard Maven multi-module approach), we create a **Java-centric workspace** where other languages feel like second-class citizens. When a Python engineer joins the team, seeing `pom.xml` at root signals "this is a Java project."

We need a structure that:
- Welcomes all languages equally
- Maintains language-specific tooling idioms
- Uses NX as the unifying orchestrator
- Scales to 20+ services across multiple languages

---

## Options Considered

### Option A: Root-Level Parent POM

```
queue_simulator/
├── pom.xml                    # ← Java build tool at workspace root
├── nx.json
├── apps/
│   ├── frontend/              # TypeScript
│   ├── fleet-simulator/       # Java (inherits root pom)
│   ├── put-service/           # Java (inherits root pom)
│   └── ml-predictor/          # Python ← feels bolted-on
└── libs/
    └── common-domain/         # Java
```

| Pros | Cons |
|------|------|
| Simple Maven inheritance | Java-centric workspace identity |
| Familiar to Java developers | `pom.xml` at root signals "Java project" |
| Centralized version management | Python/Erlang engineers feel excluded |
| Easy Maven reactor builds | Harder to onboard polyglot teams |

**Verdict**: Acceptable for Java-dominated teams. Poor choice for truly polyglot organizations.

---

### Option B: Language-Scoped Libraries (Recommended)

```
queue_simulator/
├── nx.json                    # Language-agnostic orchestrator
├── package.json               # NX workspace root
│
├── apps/                      # Deployable applications (NX standard)
│   ├── frontend/              # TypeScript/React
│   ├── fleet-simulator/       # Java/Spring Boot
│   ├── put-service/           # Java/Spring Boot
│   ├── ml-predictor/          # Python (future)
│   └── messaging-bus/         # Erlang (future)
│
└── libs/                      # Shared libraries (NX standard)
    ├── java/                  # Java ecosystem
    │   ├── pom.xml            # Parent POM scoped to Java only
    │   ├── common-domain/
    │   └── grpc-stubs/
    ├── python/                # Python ecosystem (future)
    │   └── py-common/
    ├── erlang/                # Erlang ecosystem (future)
    │   └── common/
    └── proto/                 # Language-agnostic contracts
        └── *.proto
```

| Pros | Cons |
|------|------|
| Each language has its own "home" | Slightly deeper nesting for libs |
| Parent POM doesn't pollute root | Cross-language deps need explicit handling |
| Python/Erlang teams feel equal | Services reference libs with longer paths |
| Clear separation of concerns | More directories to navigate initially |
| Proto files are first-class shared | — |

**Verdict**: Best balance for growing polyglot teams. Follows Google's monorepo philosophy.

---

### Option C: Fully Flat (No Parent POM)

```
queue_simulator/
├── nx.json
├── apps/
│   ├── fleet-simulator/       # Standalone pom.xml, no parent
│   ├── put-service/           # Standalone pom.xml, no parent
│   └── ml-predictor/          # Standalone pyproject.toml
└── libs/
    ├── common-domain/         # Java, standalone pom
    └── proto/                 # Just .proto files
```

Each Java service has a **complete, independent** `pom.xml` with no parent inheritance.

| Pros | Cons |
|------|------|
| Maximum independence | Duplication of Maven boilerplate |
| Easy to extract service to own repo | Version drift across Java services |
| NX is the only orchestrator | No central dependency management |
| Language-agnostic structure | Harder to enforce consistency |

**Verdict**: Good for eventual service extraction. More maintenance overhead.

---

### Option D: Bazel/Pants (Unified Polyglot Build)

```
queue_simulator/
├── WORKSPACE                  # Bazel workspace root
├── BUILD.bazel
├── apps/
│   ├── fleet-simulator/
│   │   └── BUILD.bazel        # java_binary rule
│   └── ml-predictor/
│       └── BUILD.bazel        # py_binary rule
```

| Pros | Cons |
|------|------|
| Single build graph across all languages | Steep learning curve (6-12 months) |
| Hermetic builds, incredible caching | External tool integration harder |
| True polyglot first-class support | Bazel community for Erlang is small |
| Google/Stripe/Uber scale proven | Overkill for < 50 services |

**Verdict**: Right answer at massive scale. Significant investment for current team size.

---

## Decision: Option B (Language-Scoped Libraries)

We choose **Option B** because:

1. **Polyglot-ready from day one** — Structure welcomes Python/Erlang as equals to Java
2. **NX leverage** — Already using NX; it excels as a language-agnostic orchestrator
3. **Java idioms preserved** — Parent POM lives in `libs/java/`, Java devs get familiar patterns
4. **Proto-first contracts** — gRPC definitions are language-agnostic, generate into each ecosystem
5. **Migration path exists** — If we ever adopt Bazel, this structure maps cleanly
6. **Clear mental model** — Apps are deployable, libs are shared; language is implementation detail

---

## Target Directory Structure

```
queue_simulator/
│
├── nx.json                          # NX workspace configuration
├── package.json                     # NX workspace root, npm scripts
├── tsconfig.base.json               # TypeScript base config
│
├── apps/                            # ═══ DEPLOYABLE APPLICATIONS ═══
│   │
│   ├── frontend/                    # React/TypeScript web UI
│   │   ├── project.json             # NX project config
│   │   ├── package.json
│   │   ├── vite.config.ts
│   │   └── src/
│   │
│   ├── fleet-simulator/             # Java: Desmo-J simulation engine
│   │   ├── project.json             # NX project config
│   │   ├── pom.xml                  # Inherits from libs/java/pom.xml
│   │   └── src/
│   │       └── main/java/com/greyorange/simulator/
│   │
│   ├── put-service/                 # Java: Storage position selection
│   │   ├── project.json
│   │   ├── pom.xml
│   │   └── src/
│   │       └── main/java/com/greyorange/put/
│   │
│   ├── pick-service/                # Java: Order fulfillment (future)
│   │   ├── project.json
│   │   ├── pom.xml
│   │   └── src/
│   │
│   ├── inventory-service/           # Java: Stock management (future)
│   │   ├── project.json
│   │   ├── pom.xml
│   │   └── src/
│   │
│   ├── ml-predictor/                # Python: Demand prediction (future)
│   │   ├── project.json
│   │   ├── pyproject.toml
│   │   └── src/
│   │
│   └── messaging-bus/               # Erlang: Real-time messaging (future)
│       ├── project.json
│       ├── rebar.config
│       └── src/
│
├── libs/                            # ═══ SHARED LIBRARIES ═══
│   │
│   ├── java/                        # Java ecosystem libraries
│   │   ├── pom.xml                  # Parent POM for all Java
│   │   │
│   │   ├── common-domain/           # Shared value types
│   │   │   ├── project.json
│   │   │   ├── pom.xml
│   │   │   └── src/main/java/com/greyorange/common/
│   │   │       ├── coordinates/     # GridCoordinate, WorldCoordinate
│   │   │       └── types/           # RackId, ToteId, SystemType
│   │   │
│   │   └── grpc-stubs/              # Generated Java gRPC stubs
│   │       ├── project.json
│   │       ├── pom.xml
│   │       └── src/main/java/       # Generated from libs/proto
│   │
│   ├── python/                      # Python ecosystem libraries (future)
│   │   ├── pyproject.toml           # Workspace-level Python config
│   │   │
│   │   ├── py-common/               # Shared Python utilities
│   │   │   ├── project.json
│   │   │   └── src/
│   │   │
│   │   └── grpc-stubs/              # Generated Python gRPC stubs
│   │       ├── project.json
│   │       └── src/
│   │
│   ├── erlang/                      # Erlang ecosystem libraries (future)
│   │   ├── rebar.config
│   │   └── common/
│   │       ├── project.json
│   │       └── src/
│   │
│   ├── typescript/                  # TypeScript shared libraries
│   │   └── ui-components/           # Shared React components
│   │       ├── project.json
│   │       └── src/
│   │
│   ├── proto/                       # ═══ LANGUAGE-AGNOSTIC CONTRACTS ═══
│   │   ├── project.json             # NX targets for stub generation
│   │   ├── buf.yaml                 # Buf schema registry config
│   │   ├── buf.gen.yaml             # Generation config
│   │   └── src/
│   │       ├── common/v1/
│   │       │   └── types.proto      # GridCoordinate, RackId, etc.
│   │       ├── put/v1/
│   │       │   └── put.proto        # PutService definition
│   │       ├── pick/v1/
│   │       │   └── pick.proto       # PickService definition
│   │       ├── inventory/v1/
│   │       │   └── inventory.proto  # InventoryService definition
│   │       └── relay/v1/
│   │           └── relay.proto      # RelayService definition
│   │
│   └── api-schema/                  # OpenAPI specs (existing)
│       └── openapi.yaml
│
├── tools/                           # Build tooling and generators
│   ├── java-codegen/                # Java code generators
│   └── proto-gen/                   # Proto generation scripts
│
├── docs/                            # Documentation
│   └── architecture/
│       ├── microservices-decomposition-design.md
│       └── polyglot-monorepo-structure-design.md  # This document
│
└── deployment/                      # Deployment configs
    ├── docker-compose.yml
    └── k8s/
```

---

## Key Design Principles

### 1. Apps Are Deployable, Libs Are Shared

| Directory | Purpose | Examples |
|-----------|---------|----------|
| `apps/` | Independently deployable services | `fleet-simulator`, `put-service`, `frontend` |
| `libs/` | Shared code consumed by apps | `common-domain`, `grpc-stubs`, `proto` |

A service in `apps/` produces a deployable artifact (JAR, container image, bundle). A library in `libs/` is consumed as a dependency.

### 2. Language Is an Implementation Detail

Services live at `apps/{service-name}`, **not** `apps/java/{service-name}`. The fact that `put-service` is written in Java is visible in its `pom.xml`, but not in its directory location.

```
✅ Correct: apps/put-service/
❌ Wrong:   apps/java/put-service/
```

This means when we rewrite a service in a different language, the directory doesn't change—only the build files inside.

### 3. Language Ecosystems Are Scoped in Libs

Libraries are grouped by language because they share build tooling:

```
libs/
├── java/           # pom.xml parent, Maven conventions
├── python/         # pyproject.toml, pip/poetry
├── erlang/         # rebar.config
└── proto/          # Language-agnostic, generates into others
```

The parent `pom.xml` lives at `libs/java/pom.xml`, not at workspace root. This keeps Java's build tool hierarchy contained.

### 4. Proto Files Are the Contract

Proto definitions in `libs/proto/` are the source of truth for service interfaces. They:
- Define service contracts before implementation
- Generate stubs into each language's grpc-stubs library
- Are versioned (`v1/`, `v2/`) for evolution

### 5. NX Orchestrates Everything

NX is language-agnostic. It understands:
- **Projects** — Defined by `project.json`
- **Targets** — build, test, serve, lint, generate
- **Dependencies** — Implicit or explicit between projects
- **Caching** — Input/output hashing for incremental builds

Each service and library, regardless of language, has a `project.json`.

---

## Java Parent POM Configuration

### `libs/java/pom.xml` (Parent POM)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 
         http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <groupId>com.greyorange.warehouse</groupId>
    <artifactId>java-parent</artifactId>
    <version>1.0.0-SNAPSHOT</version>
    <packaging>pom</packaging>

    <name>Warehouse Java Parent</name>
    <description>Parent POM for all Java modules in the warehouse platform</description>

    <!-- Spring Boot parent for dependency management -->
    <parent>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-starter-parent</artifactId>
        <version>3.2.0</version>
        <relativePath/>
    </parent>

    <!-- Only Java libraries are modules here -->
    <modules>
        <module>common-domain</module>
        <module>grpc-stubs</module>
    </modules>

    <properties>
        <java.version>21</java.version>
        <maven.compiler.source>21</maven.compiler.source>
        <maven.compiler.target>21</maven.compiler.target>
        
        <!-- Dependency versions -->
        <grpc.version>1.59.0</grpc.version>
        <protobuf.version>3.25.1</protobuf.version>
        <lombok.version>1.18.30</lombok.version>
        <caffeine.version>3.1.8</caffeine.version>
    </properties>

    <dependencyManagement>
        <dependencies>
            <!-- Internal modules -->
            <dependency>
                <groupId>com.greyorange.warehouse</groupId>
                <artifactId>common-domain</artifactId>
                <version>${project.version}</version>
            </dependency>
            <dependency>
                <groupId>com.greyorange.warehouse</groupId>
                <artifactId>grpc-stubs</artifactId>
                <version>${project.version}</version>
            </dependency>

            <!-- gRPC BOM -->
            <dependency>
                <groupId>io.grpc</groupId>
                <artifactId>grpc-bom</artifactId>
                <version>${grpc.version}</version>
                <type>pom</type>
                <scope>import</scope>
            </dependency>

            <!-- Caffeine cache -->
            <dependency>
                <groupId>com.github.ben-manes.caffeine</groupId>
                <artifactId>caffeine</artifactId>
                <version>${caffeine.version}</version>
            </dependency>
        </dependencies>
    </dependencyManagement>

    <build>
        <pluginManagement>
            <plugins>
                <plugin>
                    <groupId>org.springframework.boot</groupId>
                    <artifactId>spring-boot-maven-plugin</artifactId>
                    <configuration>
                        <excludes>
                            <exclude>
                                <groupId>org.projectlombok</groupId>
                                <artifactId>lombok</artifactId>
                            </exclude>
                        </excludes>
                    </configuration>
                </plugin>
            </plugins>
        </pluginManagement>
    </build>
</project>
```

### `apps/put-service/pom.xml` (Service POM)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 
         http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <parent>
        <groupId>com.greyorange.warehouse</groupId>
        <artifactId>java-parent</artifactId>
        <version>1.0.0-SNAPSHOT</version>
        <relativePath>../../libs/java/pom.xml</relativePath>
    </parent>

    <artifactId>put-service</artifactId>
    <packaging>jar</packaging>

    <name>Put Service</name>
    <description>Storage position selection and popularity management</description>

    <dependencies>
        <!-- Internal dependencies -->
        <dependency>
            <groupId>com.greyorange.warehouse</groupId>
            <artifactId>common-domain</artifactId>
        </dependency>
        <dependency>
            <groupId>com.greyorange.warehouse</groupId>
            <artifactId>grpc-stubs</artifactId>
        </dependency>

        <!-- Spring Boot -->
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-web</artifactId>
        </dependency>
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-actuator</artifactId>
        </dependency>

        <!-- gRPC -->
        <dependency>
            <groupId>io.grpc</groupId>
            <artifactId>grpc-netty-shaded</artifactId>
        </dependency>
        <dependency>
            <groupId>io.grpc</groupId>
            <artifactId>grpc-protobuf</artifactId>
        </dependency>
        <dependency>
            <groupId>io.grpc</groupId>
            <artifactId>grpc-stub</artifactId>
        </dependency>

        <!-- Utilities -->
        <dependency>
            <groupId>org.projectlombok</groupId>
            <artifactId>lombok</artifactId>
            <scope>provided</scope>
        </dependency>

        <!-- Testing -->
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-test</artifactId>
            <scope>test</scope>
        </dependency>
    </dependencies>

    <build>
        <plugins>
            <plugin>
                <groupId>org.springframework.boot</groupId>
                <artifactId>spring-boot-maven-plugin</artifactId>
            </plugin>
        </plugins>
    </build>
</project>
```

---

## NX Project Configuration Examples

### `apps/put-service/project.json`

```json
{
  "name": "put-service",
  "$schema": "../../node_modules/nx/schemas/project-schema.json",
  "projectType": "application",
  "sourceRoot": "apps/put-service/src",
  "targets": {
    "serve": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn spring-boot:run -f apps/put-service/pom.xml",
        "cwd": "{workspaceRoot}"
      },
      "dependsOn": ["build"]
    },
    "dev": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn spring-boot:run -f apps/put-service/pom.xml -Dspring-boot.run.jvmArguments='-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=5006'",
        "cwd": "{workspaceRoot}"
      },
      "dependsOn": ["build"]
    },
    "build": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn clean package -DskipTests -f apps/put-service/pom.xml",
        "cwd": "{workspaceRoot}"
      },
      "outputs": ["{projectRoot}/target"],
      "dependsOn": ["^build"],
      "cache": true
    },
    "test": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn test -f apps/put-service/pom.xml",
        "cwd": "{workspaceRoot}"
      },
      "dependsOn": ["build"],
      "cache": true
    },
    "lint": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn spotless:check -f apps/put-service/pom.xml",
        "cwd": "{workspaceRoot}"
      },
      "cache": true
    }
  },
  "tags": ["type:app", "scope:put", "language:java"],
  "implicitDependencies": ["common-domain", "grpc-stubs", "proto"]
}
```

### `libs/java/common-domain/project.json`

```json
{
  "name": "common-domain",
  "$schema": "../../../node_modules/nx/schemas/project-schema.json",
  "projectType": "library",
  "sourceRoot": "libs/java/common-domain/src",
  "targets": {
    "build": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn clean install -DskipTests -f libs/java/common-domain/pom.xml",
        "cwd": "{workspaceRoot}"
      },
      "outputs": ["{projectRoot}/target"],
      "cache": true
    },
    "test": {
      "executor": "nx:run-commands",
      "options": {
        "command": "mvn test -f libs/java/common-domain/pom.xml",
        "cwd": "{workspaceRoot}"
      },
      "cache": true
    }
  },
  "tags": ["type:lib", "scope:shared", "language:java"]
}
```

### `libs/proto/project.json`

```json
{
  "name": "proto",
  "$schema": "../../node_modules/nx/schemas/project-schema.json",
  "projectType": "library",
  "sourceRoot": "libs/proto/src",
  "targets": {
    "generate": {
      "executor": "nx:run-commands",
      "options": {
        "commands": [
          "buf generate --template buf.gen.java.yaml",
          "buf generate --template buf.gen.python.yaml"
        ],
        "cwd": "{projectRoot}",
        "parallel": false
      },
      "outputs": [
        "{workspaceRoot}/libs/java/grpc-stubs/src/main/java",
        "{workspaceRoot}/libs/python/grpc-stubs/src"
      ]
    },
    "lint": {
      "executor": "nx:run-commands",
      "options": {
        "command": "buf lint",
        "cwd": "{projectRoot}"
      },
      "cache": true
    },
    "breaking": {
      "executor": "nx:run-commands",
      "options": {
        "command": "buf breaking --against '.git#branch=main'",
        "cwd": "{projectRoot}"
      }
    }
  },
  "tags": ["type:lib", "scope:shared", "language:proto"]
}
```

---

## NX Commands Reference

### Building

```bash
# Build a specific project
nx build put-service

# Build all projects
nx run-many -t build

# Build only affected by changes (CI optimization)
nx affected -t build

# Build all Java services
nx run-many -t build --projects=tag:language:java
```

### Running Services

```bash
# Run single service
nx serve put-service

# Run multiple services
nx run-many -t serve -p frontend,fleet-simulator,put-service

# Run with debug
nx dev put-service
```

### Testing

```bash
# Test a project
nx test put-service

# Test all affected
nx affected -t test

# Test all Java
nx run-many -t test --projects=tag:language:java
```

### Proto Generation

```bash
# Generate all language stubs
nx generate proto

# Lint proto files
nx lint proto

# Check for breaking changes
nx breaking proto
```

### Visualization

```bash
# View dependency graph (opens browser)
nx graph

# Export graph to JSON
nx graph --file=output.json
```

---

## Migration Path

### Phase 1: Restructure (Week 1)

1. Create `libs/java/` directory with parent `pom.xml`
2. Create `libs/java/common-domain/` and extract shared types
3. Create `libs/proto/` with initial proto definitions
4. Update existing `apps/backend/` to reference new parent

### Phase 2: Extract Put Service (Weeks 2-3)

1. Create `apps/put-service/` skeleton
2. Move `domain/put/*` from backend to put-service
3. Implement gRPC server in put-service
4. Implement gRPC client in fleet-simulator
5. Feature flag to toggle between in-process and gRPC

### Phase 3: Rename Backend (Week 3)

1. Rename `apps/backend/` → `apps/fleet-simulator/`
2. Update all NX references
3. Update CI/CD pipelines
4. Update documentation

### Phase 4: Subsequent Extractions (Weeks 4+)

Following the same pattern:
- Pick Service
- Inventory Service
- Station Service
- Relay Service

---

## Cross-Language Dependencies

When a Java service needs Python-generated artifacts (or vice versa), handle via:

### Option 1: Artifact Registry

Publish language-specific artifacts to a registry:
- Java JARs → Maven (Artifactory, Nexus)
- Python wheels → PyPI (private)
- Container images → Container Registry

### Option 2: Generated Code in Repo

For gRPC stubs, generate into the repo:
```
libs/proto/           # Source .proto files
libs/java/grpc-stubs/ # Generated Java stubs (committed or gitignored)
libs/python/grpc-stubs/ # Generated Python stubs
```

### Option 3: Runtime Only

Services communicate only via gRPC at runtime. No compile-time dependency between languages.

**Recommendation**: Option 2 for development speed, Option 1 for production maturity.

---

## FAQ

### Why not just use Bazel?

Bazel provides superior polyglot support but requires:
- 6-12 month learning curve
- Significant build file migration
- Custom rules for some tools (Desmo-J, OR-Tools)

We may adopt Bazel at 50+ services. For now, NX + native tools gives 80% benefit at 20% cost.

### Why are apps flat but libs nested by language?

**Apps** are independently deployable—their language is an implementation detail. When you deploy `put-service`, you don't care that it's Java.

**Libs** share language-specific build tooling. A parent `pom.xml` only makes sense for Java libraries. Python libraries share `pyproject.toml` patterns.

### How do I add a new Java service?

1. Create `apps/{service-name}/`
2. Copy `pom.xml` template with parent reference
3. Create `project.json` with NX targets
4. Add dependencies in `pom.xml`
5. Run `nx build {service-name}`

### How do I add a new language (e.g., Go)?

1. Create `libs/go/` with `go.mod` workspace file
2. Create `libs/go/grpc-stubs/` for generated code
3. Add `buf.gen.go.yaml` for proto generation
4. Create service in `apps/{service-name}/` with `go.mod`
5. Add `project.json` with Go-specific targets

---

## References

- [Microservices Decomposition Design](./microservices-decomposition-design.md)
- [NX Documentation](https://nx.dev/getting-started/intro)
- [Buf Documentation](https://buf.build/docs/)
- [Maven Multi-Module Projects](https://maven.apache.org/guides/mini/guide-multiple-modules.html)
- [Google's Monorepo Approach](https://research.google/pubs/pub45424/)

---

## Appendix: File Listing After Migration

```
queue_simulator/
├── nx.json
├── package.json
├── package-lock.json
├── tsconfig.base.json
├── biome.json
├── docker-compose.yml
├── Dockerfile
│
├── apps/
│   ├── frontend/
│   │   ├── project.json
│   │   ├── package.json
│   │   ├── vite.config.ts
│   │   ├── index.html
│   │   └── src/
│   │
│   ├── fleet-simulator/              # Renamed from 'backend'
│   │   ├── project.json
│   │   ├── pom.xml
│   │   └── src/
│   │       ├── main/
│   │       │   ├── java/com/greyorange/simulator/
│   │       │   │   ├── DesmojSimulationEngine.java
│   │       │   │   ├── DesmojWarehouseModel.java
│   │       │   │   ├── orchestrator/
│   │       │   │   └── ...
│   │       │   └── resources/
│   │       └── test/
│   │
│   └── put-service/                   # Extracted service
│       ├── project.json
│       ├── pom.xml
│       └── src/
│           ├── main/
│           │   ├── java/com/greyorange/put/
│           │   │   ├── PutServiceApplication.java
│           │   │   ├── domain/
│           │   │   │   ├── PutProcessor.java
│           │   │   │   ├── GridPopularityManager.java
│           │   │   │   └── strategies/
│           │   │   ├── api/
│           │   │   │   └── PutGrpcService.java
│           │   │   └── config/
│           │   └── resources/
│           │       └── application.yml
│           └── test/
│
├── libs/
│   ├── java/
│   │   ├── pom.xml                    # Java parent POM
│   │   │
│   │   ├── common-domain/
│   │   │   ├── project.json
│   │   │   ├── pom.xml
│   │   │   └── src/main/java/com/greyorange/common/
│   │   │       ├── coordinates/
│   │   │       │   ├── GridCoordinate.java
│   │   │       │   └── WorldCoordinate.java
│   │   │       └── types/
│   │   │           ├── RackId.java
│   │   │           ├── ToteId.java
│   │   │           └── SystemType.java
│   │   │
│   │   └── grpc-stubs/
│   │       ├── project.json
│   │       ├── pom.xml
│   │       └── src/main/java/         # Generated from libs/proto
│   │
│   ├── proto/
│   │   ├── project.json
│   │   ├── buf.yaml
│   │   ├── buf.gen.yaml
│   │   └── src/
│   │       ├── common/v1/types.proto
│   │       ├── put/v1/put.proto
│   │       ├── pick/v1/pick.proto
│   │       └── inventory/v1/inventory.proto
│   │
│   ├── typescript/
│   │   └── ui-components/
│   │       ├── project.json
│   │       └── src/
│   │
│   └── api-schema/
│       └── openapi.yaml
│
├── tools/
│   └── scripts/
│
├── docs/
│   ├── architecture/
│   │   ├── microservices-decomposition-design.md
│   │   └── polyglot-monorepo-structure-design.md
│   └── ...
│
└── deployment/
    ├── docker-compose.yml
    └── k8s/
```

