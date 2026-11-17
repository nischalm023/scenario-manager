# API Paths Consolidation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Consolidate all backend API paths under `/api/*` prefix with OpenAPI as single source of truth for frontend path generation.

**Architecture:** All backend endpoints move to `/api/*` prefix. Frontend generates path constants from OpenAPI spec. Vite proxy simplifies to single `/api` rule.

**Tech Stack:** Java Spring Boot, TypeScript, Vite, OpenAPI

---

## Task 1: Create Backend ApiPaths Constants

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/api/config/ApiPaths.java`

**Step 1: Create the ApiPaths constants class**

```java
package com.greyorange.dessimulator.api.config;

/**
 * Centralized API path constants for all controllers.
 *
 * <p>All API endpoints should be under the /api prefix to clearly separate
 * backend API routes from frontend SPA routes.
 */
public final class ApiPaths {

  private ApiPaths() {}

  public static final String API_PREFIX = "/api";

  public static final String HEALTH = API_PREFIX + "/health";
  public static final String SIMULATION = API_PREFIX + "/simulation";
  public static final String COMPARISON = API_PREFIX + "/comparison";
  public static final String SCENARIOS = API_PREFIX + "/scenarios";
  public static final String WAREHOUSE_MAP = API_PREFIX + "/warehouse-map";
}
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn compile -q`
Expected: BUILD SUCCESS (no output means success)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/config/ApiPaths.java
git commit -m "feat(api): add centralized ApiPaths constants"
```

---

## Task 2: Update HealthController

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/health/HealthController.java`

**Step 1: Update imports and remove root endpoint**

Replace the entire file with:

```java
package com.greyorange.dessimulator.api.health;

import com.greyorange.dessimulator.api.config.ApiPaths;
import io.swagger.v3.oas.annotations.Operation;
import io.swagger.v3.oas.annotations.tags.Tag;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.Map;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
@Tag(name = "Health")
class HealthController {

  @GetMapping(ApiPaths.HEALTH)
  @Operation(
      summary = "Health Check",
      description = "Check if the API server is running and healthy")
  ResponseEntity<Map<String, String>> health() {
    return ResponseEntity.ok(
        Map.of(
            "status",
            "healthy",
            "timestamp",
            LocalDateTime.now().format(DateTimeFormatter.ISO_DATE_TIME),
            "version",
            "1.0.0"));
  }
}
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn compile -q`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/health/HealthController.java
git commit -m "refactor(api): move health endpoint to /api/health"
```

---

## Task 3: Update SimulationsController

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java`

**Step 1: Add ApiPaths import**

Add after line 4 (after common.ApiResponse import):

```java
import com.greyorange.dessimulator.api.config.ApiPaths;
```

**Step 2: Update endpoint mappings**

Replace each endpoint annotation:

Line 53 - Change:
```java
@PostMapping("/simulation/run")
```
To:
```java
@PostMapping(ApiPaths.SIMULATION + "/run")
```

Line 77 - Change:
```java
@GetMapping("/simulation/status")
```
To:
```java
@GetMapping(ApiPaths.SIMULATION + "/status")
```

Line 85 - Change:
```java
@PostMapping("/simulation/analyze")
```
To:
```java
@PostMapping(ApiPaths.SIMULATION + "/analyze")
```

Line 98 - Change:
```java
@PostMapping("/comparison/run")
```
To:
```java
@PostMapping(ApiPaths.COMPARISON + "/run")
```

**Step 3: Verify compilation**

Run: `cd apps/backend && mvn compile -q`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java
git commit -m "refactor(api): move simulation/comparison endpoints to /api/*"
```

---

## Task 4: Update WarehouseMapController

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/warehousemap/WarehouseMapController.java`

**Step 1: Add ApiPaths import**

Add after line 3 (after the static import):

```java
import com.greyorange.dessimulator.api.config.ApiPaths;
```

**Step 2: Update RequestMapping**

Line 22 - Change:
```java
@RequestMapping("/warehouse-map")
```
To:
```java
@RequestMapping(ApiPaths.WAREHOUSE_MAP)
```

**Step 3: Verify compilation**

Run: `cd apps/backend && mvn compile -q`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/warehousemap/WarehouseMapController.java
git commit -m "refactor(api): move warehouse-map endpoints to /api/warehouse-map"
```

---

## Task 5: Simplify SpaRoutingConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/api/config/SpaRoutingConfig.java`

**Step 1: Simplify path exclusion logic**

Replace lines 52-60 (the if statement checking multiple paths):

```java
                // Don't serve index.html for API routes - let controllers handle them
                if (resourcePath.startsWith("api/")
                    || resourcePath.startsWith("simulation/")
                    || resourcePath.startsWith("comparison/")
                    || resourcePath.startsWith("warehouse-map/")
                    || resourcePath.startsWith("health")
                    || resourcePath.startsWith("openapi")) {
                  return null;
                }
```

With:

```java
                // Don't serve index.html for API routes - let controllers handle them
                if (resourcePath.startsWith("api/") || resourcePath.startsWith("openapi")) {
                  return null;
                }
```

**Step 2: Verify compilation**

Run: `cd apps/backend && mvn compile -q`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/api/config/SpaRoutingConfig.java
git commit -m "refactor(api): simplify SpaRoutingConfig to only check /api/*"
```

---

## Task 6: Update Dockerfile Health Check

**Files:**
- Modify: `Dockerfile`

**Step 1: Update health check path**

Line 65-66 - Change:
```dockerfile
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:8080/health || exit 1
```

To:
```dockerfile
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:8080/api/health || exit 1
```

**Step 2: Commit**

```bash
git add Dockerfile
git commit -m "chore: update Dockerfile health check to /api/health"
```

---

## Task 7: Create API Paths Generator Script

**Files:**
- Create: `scripts/generate-api-paths.js`

**Step 1: Create the generator script**

```javascript
#!/usr/bin/env node

/**
 * Generate TypeScript API path constants from OpenAPI spec.
 *
 * Usage: node scripts/generate-api-paths.js [openapi-url]
 * Default URL: http://localhost:8080/openapi.json
 */

const fs = require('fs');
const path = require('path');

const OPENAPI_URL = process.argv[2] || 'http://localhost:8080/openapi.json';
const OUTPUT_FILE = path.join(__dirname, '../apps/frontend/src/constants/api-paths.ts');

async function generateApiPaths() {
  console.log(`Fetching OpenAPI spec from ${OPENAPI_URL}...`);

  const response = await fetch(OPENAPI_URL);
  if (!response.ok) {
    throw new Error(`Failed to fetch OpenAPI spec: ${response.status} ${response.statusText}`);
  }

  const spec = await response.json();
  const paths = Object.keys(spec.paths || {});

  console.log(`Found ${paths.length} API paths`);

  // Convert paths to constant names
  // /api/simulation/run -> SIMULATION_RUN
  // /api/scenarios/{scenarioId} -> SCENARIOS_BY_ID
  const constants = {};
  const dynamicPaths = [];

  for (const apiPath of paths) {
    // Skip paths with parameters for now, handle separately
    if (apiPath.includes('{')) {
      dynamicPaths.push(apiPath);
      continue;
    }

    // Convert path to constant name
    const constName = apiPath
      .replace(/^\/api\//, '')  // Remove /api/ prefix
      .replace(/\//g, '_')       // Replace / with _
      .replace(/-/g, '_')        // Replace - with _
      .toUpperCase();

    if (constName) {
      constants[constName] = apiPath;
    }
  }

  // Generate TypeScript file
  const output = `// Auto-generated from OpenAPI spec - DO NOT EDIT
// Generated at: ${new Date().toISOString()}
// Source: ${OPENAPI_URL}

export const API_PATHS = {
${Object.entries(constants)
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([name, path]) => `  ${name}: '${path}',`)
  .join('\n')}
} as const

export type ApiPath = typeof API_PATHS[keyof typeof API_PATHS]

// Helper functions for dynamic paths with parameters
export const apiPath = {
  scenario: (id: string) => \`\${API_PATHS.SCENARIOS}/\${id}\`,
  scenarioSkuData: (id: string) => \`\${API_PATHS.SCENARIOS}/\${id}/sku-data\`,
  scenarioOrderAnalysis: (id: string) => \`\${API_PATHS.SCENARIOS}/\${id}/order-analysis\`,
}
`;

  // Ensure directory exists
  const dir = path.dirname(OUTPUT_FILE);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }

  fs.writeFileSync(OUTPUT_FILE, output);
  console.log(`Generated ${OUTPUT_FILE}`);
  console.log(`  - ${Object.keys(constants).length} static paths`);
  console.log(`  - ${dynamicPaths.length} dynamic paths (handled by apiPath helpers)`);
}

generateApiPaths().catch(err => {
  console.error('Error:', err.message);
  process.exit(1);
});
```

**Step 2: Make script executable**

Run: `chmod +x scripts/generate-api-paths.js`

**Step 3: Create constants directory**

Run: `mkdir -p apps/frontend/src/constants`

**Step 4: Commit**

```bash
git add scripts/generate-api-paths.js
git commit -m "feat(frontend): add API paths generator script"
```

---

## Task 8: Start Backend and Generate API Paths

**Step 1: Start the backend server**

Run: `npx nx serve backend &`
Wait: 15 seconds for startup

**Step 2: Verify backend is running**

Run: `curl -s http://localhost:8080/api/health | head -c 100`
Expected: `{"status":"healthy","timestamp":"...","version":"1.0.0"}`

**Step 3: Generate API paths**

Run: `node scripts/generate-api-paths.js`
Expected output:
```
Fetching OpenAPI spec from http://localhost:8080/openapi.json...
Found X API paths
Generated apps/frontend/src/constants/api-paths.ts
```

**Step 4: Verify generated file**

Run: `cat apps/frontend/src/constants/api-paths.ts | head -30`
Expected: File with API_PATHS object containing paths like `HEALTH: '/api/health'`

**Step 5: Stop the backend**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 6: Commit**

```bash
git add apps/frontend/src/constants/api-paths.ts
git commit -m "feat(frontend): generate API path constants from OpenAPI"
```

---

## Task 9: Update Frontend API Service

**Files:**
- Modify: `apps/frontend/src/services/api.ts`

**Step 1: Add imports for API paths**

Add after line 15 (after the generated imports):

```typescript
import { API_PATHS, apiPath } from '../constants/api-paths'
```

**Step 2: Update all endpoint paths**

Replace each hardcoded path with the constant:

Line 54 - Change:
```typescript
    return this.request('/health')
```
To:
```typescript
    return this.request(API_PATHS.HEALTH)
```

Line 59 - Change:
```typescript
    return this.request('/simulation/run', {
```
To:
```typescript
    return this.request(API_PATHS.SIMULATION_RUN, {
```

Line 66 - Change:
```typescript
    return this.request('/simulation/status')
```
To:
```typescript
    return this.request(API_PATHS.SIMULATION_STATUS)
```

Line 70 - Change:
```typescript
    return this.request('/simulation/results')
```
To:
```typescript
    return this.request(`${API_PATHS.SIMULATION}/results`)
```

Line 74 - Change:
```typescript
    return this.request('/simulation/cancel', {
```
To:
```typescript
    return this.request(`${API_PATHS.SIMULATION}/cancel`, {
```

Line 81 - Change:
```typescript
    return this.request('/comparison/run', {
```
To:
```typescript
    return this.request(API_PATHS.COMPARISON_RUN, {
```

Line 88 - Change:
```typescript
    return this.request('/comparison/status')
```
To:
```typescript
    return this.request(`${API_PATHS.COMPARISON}/status`)
```

Line 93 - Change:
```typescript
    return this.request('/api/scenarios/')
```
To:
```typescript
    return this.request(`${API_PATHS.SCENARIOS}/`)
```

Line 97 - Change:
```typescript
    return this.request(`/api/scenarios/${scenarioId}`)
```
To:
```typescript
    return this.request(apiPath.scenario(scenarioId))
```

Line 101 - Change:
```typescript
    return this.request('/api/scenarios/create', {
```
To:
```typescript
    return this.request(API_PATHS.SCENARIOS_CREATE, {
```

Line 112 - Change:
```typescript
    return this.request('/api/scenarios/create-preset', {
```
To:
```typescript
    return this.request(`${API_PATHS.SCENARIOS}/create-preset`, {
```

Line 119 - Change:
```typescript
    return this.request(`/api/scenarios/${scenarioId}`, {
```
To:
```typescript
    return this.request(apiPath.scenario(scenarioId), {
```

Line 125 - Change:
```typescript
    return this.request('/api/scenarios/statistics')
```
To:
```typescript
    return this.request(API_PATHS.SCENARIOS_STATISTICS)
```

Line 130 - Change:
```typescript
    return this.request(`/api/scenarios/${scenarioId}/sku-data`)
```
To:
```typescript
    return this.request(apiPath.scenarioSkuData(scenarioId))
```

Line 134 - Change:
```typescript
    return this.request(`/api/scenarios/${scenarioId}/order-analysis`)
```
To:
```typescript
    return this.request(apiPath.scenarioOrderAnalysis(scenarioId))
```

**Step 3: Verify TypeScript compiles**

Run: `npx nx run frontend:typecheck`
Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/services/api.ts
git commit -m "refactor(frontend): use generated API path constants in api.ts"
```

---

## Task 10: Update Warehouse Map Service

**Files:**
- Modify: `apps/frontend/src/services/warehouseMapService.ts`

**Step 1: Add API paths import**

Add after line 17 (after the generated imports):

```typescript
import { API_PATHS } from '../constants/api-paths'
```

**Step 2: Update fetch calls**

Line 155 - Change:
```typescript
      const response = await fetch(`${this.baseUrl}/warehouse-map/generate`, {
```
To:
```typescript
      const response = await fetch(`${this.baseUrl}${API_PATHS.WAREHOUSE_MAP_GENERATE}`, {
```

Line 201 - Change:
```typescript
      const response = await fetch(`${this.baseUrl}/warehouse-map/layouts`)
```
To:
```typescript
      const response = await fetch(`${this.baseUrl}${API_PATHS.WAREHOUSE_MAP_LAYOUTS}`)
```

Line 217 - Change:
```typescript
      const response = await fetch(`${this.baseUrl}/warehouse-map/cell-types`)
```
To:
```typescript
      const response = await fetch(`${this.baseUrl}${API_PATHS.WAREHOUSE_MAP_CELL_TYPES}`)
```

**Step 3: Verify TypeScript compiles**

Run: `npx nx run frontend:typecheck`
Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/services/warehouseMapService.ts
git commit -m "refactor(frontend): use generated API path constants in warehouseMapService"
```

---

## Task 11: Update Simulation API

**Files:**
- Modify: `apps/frontend/src/api/simulationApi.ts`

**Step 1: Add API paths import**

Add after line 1:

```typescript
import { API_PATHS } from '../constants/api-paths'
```

**Step 2: Update fetch call**

Line 14 - Change:
```typescript
  const response = await fetch('/simulation/analyze', {
```
To:
```typescript
  const response = await fetch(API_PATHS.SIMULATION_ANALYZE, {
```

**Step 3: Verify TypeScript compiles**

Run: `npx nx run frontend:typecheck`
Expected: No errors

**Step 4: Commit**

```bash
git add apps/frontend/src/api/simulationApi.ts
git commit -m "refactor(frontend): use generated API path constants in simulationApi"
```

---

## Task 12: Simplify Vite Proxy Configuration

**Files:**
- Modify: `apps/frontend/vite.config.ts`

**Step 1: Replace proxy configuration**

Replace lines 14-45 (the entire proxy object):

```typescript
    proxy: {
      // Proxy API calls to backend server during local development
      // Only proxy requests with JSON accept header (API calls), not HTML requests (page navigation)
      '/api': {
        target: 'http://localhost:8080',
      },
      '/simulation': {
        target: 'http://localhost:8080',
        bypass: (req) => {
          if (req.headers.accept?.includes('text/html')) {
            return req.url
          }
        },
      },
      '/comparison': {
        target: 'http://localhost:8080',
        bypass: (req) => {
          if (req.headers.accept?.includes('text/html')) {
            return req.url
          }
        },
      },
      '/warehouse-map': {
        target: 'http://localhost:8080',
        bypass: (req) => {
          if (req.headers.accept?.includes('text/html')) {
            return req.url
          }
        },
      },
      '/health': 'http://localhost:8080',
    },
```

With:

```typescript
    proxy: {
      // Proxy all API calls to backend server during local development
      // All API endpoints are under /api/* prefix
      '/api': {
        target: 'http://localhost:8080',
      },
    },
```

**Step 2: Verify Vite config is valid**

Run: `npx nx run frontend:build --skip-nx-cache`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add apps/frontend/vite.config.ts
git commit -m "refactor(frontend): simplify Vite proxy to single /api rule"
```

---

## Task 13: Update Package.json Scripts

**Files:**
- Modify: `package.json`

**Step 1: Add API types generation script**

Find the "scripts" section and add:

```json
    "generate:api-types": "npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts && node scripts/generate-api-paths.js",
```

**Step 2: Commit**

```bash
git add package.json
git commit -m "chore: add generate:api-types script for OpenAPI code generation"
```

---

## Task 14: Regenerate Frontend Types

**Step 1: Start the backend server**

Run: `npx nx serve backend &`
Wait: 15 seconds for startup

**Step 2: Regenerate TypeScript types from OpenAPI**

Run: `npx openapi-typescript http://localhost:8080/openapi.json -o apps/frontend/src/types/generated-api.ts`
Expected: File generated successfully

**Step 3: Regenerate API paths**

Run: `node scripts/generate-api-paths.js`
Expected: Generated with updated paths

**Step 4: Stop the backend**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 5: Commit**

```bash
git add apps/frontend/src/types/generated-api.ts apps/frontend/src/constants/api-paths.ts
git commit -m "chore: regenerate frontend types with new API paths"
```

---

## Task 15: Verify Full Stack

**Step 1: Build backend**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 2: Build frontend**

Run: `npx nx build frontend`
Expected: Build succeeds

**Step 3: Run frontend type check**

Run: `npx nx run frontend:typecheck`
Expected: No errors

**Step 4: Start backend and test endpoints**

Run: `npx nx serve backend &`
Wait: 15 seconds

Test health endpoint:
Run: `curl -s http://localhost:8080/api/health`
Expected: `{"status":"healthy",...}`

Test old health endpoint is gone:
Run: `curl -s -o /dev/null -w "%{http_code}" http://localhost:8080/health`
Expected: `404` or redirects to frontend

Test simulation status:
Run: `curl -s http://localhost:8080/api/simulation/status`
Expected: JSON response with status

**Step 5: Stop backend**

Run: `lsof -ti:8080 | xargs kill -9 2>/dev/null || true`

**Step 6: Final commit with verification**

```bash
git status
git log --oneline -10
```

---

## Summary

After completing all tasks:

1. All backend endpoints are under `/api/*`
2. Frontend uses generated path constants from OpenAPI
3. Vite proxy is simplified to single rule
4. Dockerfile health check updated
5. Type safety maintained with generated types

To add new endpoints in the future:
1. Add endpoint in backend controller using `ApiPaths` constants
2. Restart backend
3. Run `npm run generate:api-types`
4. Use new constant in frontend code
