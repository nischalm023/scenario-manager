# API Paths Consolidation Design

## Overview

Consolidate all backend API paths under `/api/*` prefix to simplify configuration and create clear separation between API and frontend routes. Use OpenAPI as single source of truth for path generation.

## Current State

Mixed API path patterns:
- `/api/scenarios/*` - Already prefixed
- `/simulation/*`, `/comparison/*` - No prefix
- `/warehouse-map/*` - No prefix
- `/health`, `/` - Root level

Multiple proxy rules in Vite config, multiple path checks in SpaRoutingConfig.

## Target State

All API endpoints under `/api/*`:
- `/api/health`
- `/api/simulation/*`
- `/api/comparison/*`
- `/api/scenarios/*`
- `/api/warehouse-map/*`

Single proxy rule, simplified SPA routing.

## Design

### 1. Backend API Path Changes

**Path mapping:**

| Controller | Current | New |
|------------|---------|-----|
| HealthController | `/health`, `/` | `/api/health` only |
| SimulationsController | `/simulation/*`, `/comparison/*` | `/api/simulation/*`, `/api/comparison/*` |
| WarehouseMapController | `/warehouse-map/*` | `/api/warehouse-map/*` |
| ScenariosController | `/api/scenarios/*` | (no change) |

**New `ApiPaths.java` constants:**

```java
package com.greyorange.dessimulator.api.config;

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

**SpaRoutingConfig simplification:**

```java
// Before: Multiple path checks
if (resourcePath.startsWith("api/")
    || resourcePath.startsWith("simulation/")
    || resourcePath.startsWith("comparison/")
    || resourcePath.startsWith("warehouse-map/")
    || resourcePath.startsWith("health")
    || resourcePath.startsWith("openapi")) {
  return null;
}

// After: Single check
if (resourcePath.startsWith("api/") || resourcePath.startsWith("openapi")) {
    return null;
}
```

### 2. Frontend API Path Generation

**File structure:**

```
apps/frontend/
├── src/
│   ├── constants/
│   │   └── api-paths.ts       # Generated from OpenAPI
│   ├── types/
│   │   └── generated-api.ts   # Existing generated types
scripts/
└── generate-api-paths.js      # New generator script
```

**Generated `api-paths.ts`:**

```typescript
// Auto-generated from OpenAPI spec - DO NOT EDIT
export const API_PATHS = {
  HEALTH: '/api/health',
  SIMULATION_RUN: '/api/simulation/run',
  SIMULATION_STATUS: '/api/simulation/status',
  SIMULATION_ANALYZE: '/api/simulation/analyze',
  COMPARISON_RUN: '/api/comparison/run',
  COMPARISON_STATUS: '/api/comparison/status',
  SCENARIOS: '/api/scenarios',
  SCENARIOS_CREATE: '/api/scenarios/create',
  WAREHOUSE_MAP_GENERATE: '/api/warehouse-map/generate',
  WAREHOUSE_MAP_LAYOUTS: '/api/warehouse-map/layouts',
  WAREHOUSE_MAP_CELL_TYPES: '/api/warehouse-map/cell-types',
} as const

export type ApiPath = typeof API_PATHS[keyof typeof API_PATHS]

// Helper functions for dynamic paths
export const apiPath = {
  scenario: (id: string) => `${API_PATHS.SCENARIOS}/${id}`,
  scenarioSkuData: (id: string) => `${API_PATHS.SCENARIOS}/${id}/sku-data`,
  scenarioOrderAnalysis: (id: string) => `${API_PATHS.SCENARIOS}/${id}/order-analysis`,
}
```

**Generation command:**

```bash
npm run generate:api-types
# Runs: openapi-typescript + generate-api-paths.js
```

### 3. Frontend Service Updates

Update all services to use generated constants:

**`services/api.ts`:**

```typescript
import { API_PATHS, apiPath } from '../constants/api-paths'

class ApiService {
  async healthCheck() {
    return this.request(API_PATHS.HEALTH)
  }

  async runSimulation(config: SimulationConfigDTO) {
    return this.request(API_PATHS.SIMULATION_RUN, {
      method: 'POST',
      body: JSON.stringify(config),
    })
  }

  async getScenario(scenarioId: string) {
    return this.request(apiPath.scenario(scenarioId))
  }
}
```

**Files to update:**
- `services/api.ts` - Main API service
- `services/warehouseMapService.ts` - Warehouse map endpoints
- `api/simulationApi.ts` - Analyze endpoint

### 4. Vite Proxy Simplification

**Before:**

```typescript
proxy: {
  '/api': { target: 'http://localhost:8080' },
  '/simulation': { target: '...', bypass: ... },
  '/comparison': { target: '...', bypass: ... },
  '/warehouse-map': { target: '...', bypass: ... },
  '/health': 'http://localhost:8080',
}
```

**After:**

```typescript
proxy: {
  '/api': {
    target: 'http://localhost:8080',
  },
}
```

### 5. Infrastructure Updates

**Dockerfile health check:**

```dockerfile
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:8080/api/health || exit 1
```

**OpenAPI spec:** Keep at `/openapi.json` (standard location, tools expect it here).

## Benefits

1. **Clear separation** - All API under `/api/*`, all frontend routes handled by SPA
2. **Simplified configuration** - Single proxy rule, single SPA routing check
3. **Single source of truth** - OpenAPI generates both types and path constants
4. **Better DX** - Autocomplete for paths, compile-time error checking
5. **Maintainability** - New endpoints automatically work without config changes

## Implementation Order

1. Create `ApiPaths.java` constants in backend
2. Update all controllers to use new paths
3. Simplify `SpaRoutingConfig`
4. Update Dockerfile health check
5. Create `generate-api-paths.js` script
6. Generate `api-paths.ts` from OpenAPI
7. Update frontend services to use constants
8. Simplify Vite proxy config
9. Regenerate frontend types
10. Test all endpoints

## Testing

- Verify all API endpoints respond correctly
- Test SPA routing (direct navigation to `/scenarios`, etc.)
- Test Dockerfile health check
- Verify frontend can call all endpoints
- Run existing integration tests
