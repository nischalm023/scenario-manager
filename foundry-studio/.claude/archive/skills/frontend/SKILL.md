---
name: frontend
description: Use when working with React/TypeScript files in apps/frontend/. Covers openapi-fetch API client, type generation, project structure, map visualization, binary snapshots, Biome formatting, and Google OAuth authentication.
---

# Frontend Development

## Critical Rules

1. **NEVER modify `apps/frontend/src/components/ui/`** — shadcn/ui components managed externally
2. **All APIs use camelCase JSON** — not snake_case
3. **Use Biome** for formatting, NOT Prettier
4. **Path alias**: `@/*` maps to `src/*` in imports

---

## Project Structure

```
apps/frontend/src/
├── pages/                    # Route-level components
│   ├── SimulatorPage.tsx     # Main simulator page (513 lines, primary state holder)
│   ├── ScenariosPage.tsx     # Scenario tabs container
│   ├── LoginPage.tsx         # Google OAuth login (full-viewport, no sidebar)
│   └── simulator/
│       ├── components/       # ConfigurationPanel (730 lines), SimulationResults, etc.
│       └── tabs/             # Configuration tab panels (Catalog, Infrastructure, Orders, Robots)
├── components/
│   ├── ui/                   # shadcn/ui — DO NOT MODIFY
│   ├── auth/                 # ProtectedRoute (route guard, redirects to /login)
│   ├── layout/               # AppSidebar, AppHeader (includes user avatar + logout)
│   ├── scenarios/            # Scenario CRUD (Workload, Infrastructure, Algorithm tabs)
│   ├── common/               # Shared: charts/, display/, feedback/, forms/, maps/
│   │   └── maps/
│   │       ├── WarehouseMap.tsx      # 885-line Konva canvas (see references/visualization.md)
│   │       ├── WarehouseHeatmap.tsx
│   │       └── layers/              # 7 memoized layer components
│   ├── AnimationControls.tsx        # Playback controls for snapshot animation
│   └── VisualizationModeSelector.tsx # Map mode switcher (6 modes)
├── contexts/
│   └── AuthContext.tsx        # AuthProvider + useAuth() hook (Google OAuth, localStorage session)
├── services/
│   ├── client.ts             # openapi-fetch typed client
│   ├── api.ts                # All API call functions (+ raw fetch for binary)
│   └── warehouseMapService.ts
├── hooks/
│   ├── useWarehouseMap.ts               # Map data with 5-min cache
│   ├── useLazySnapshotReconstruction.ts # Frame reconstruction with LRU cache
│   └── use-mobile.tsx                   # Responsive breakpoint hook
├── types/
│   ├── generated-api.ts      # Auto-generated from OpenAPI — NEVER edit
│   ├── generated.ts          # Flattened re-exports — add new type aliases here
│   ├── index.ts              # Barrel for UI-only types (scenarios, simulation)
│   ├── scenarios.ts          # Scenario domain types
│   ├── simulation.ts         # Simulation domain types
│   ├── visualization.ts      # VisualizationMode union type
│   └── type-guards.ts        # Config accessor helpers (getRobotSpeed, getRobotCount, etc.)
├── utils/                    # BinarySnapshotReader, LRUCache, helpers
├── constants/                # App constants (algorithms.ts, auth.ts)
├── lib/                      # Library setup (cn utility)
└── assets/                   # Static assets
```

### Routing

React Router v6 with Google OAuth gate (`App.tsx`):
- `/login` — login page (no sidebar, full viewport)
- `/simulator` — main simulator page (protected)
- `/scenarios/workload` | `/scenarios/infrastructure` | `/scenarios/algorithm` (protected)
- `/` redirects to `/simulator`

All routes except `/login` are wrapped in `ProtectedRoute`, which checks `useAuth().isAuthenticated` and redirects to `/login` preserving the return URL.

---

## API Client (openapi-fetch)

Two files form the API layer:

### `services/client.ts` — typed client
```typescript
import createClient from 'openapi-fetch'
import type { paths } from '../types/generated-api'

const client = createClient<paths>({ baseUrl: '' })
export default client
```

`baseUrl: ''` works because the Vite dev server proxies `/api/*` → `http://localhost:8080` (Envoy). In production, nginx does the same.

### `services/api.ts` — all API functions

Every API call follows this pattern:
```typescript
export async function runSimulation(config: SimulationConfigDTO) {
  const { data, error } = await client.POST('/api/simulation/run', { body: config })
  if (error) throw new Error('Simulation request failed')
  return data!
}
```

**Binary fetch bypass**: `fetchBinarySnapshots()` uses raw `fetch()` with `Accept: application/octet-stream` because openapi-fetch doesn't support binary responses. Returns `ArrayBuffer`. See `references/snapshot-pipeline.md`.

**Adding a new API call**: Add the function to `api.ts`. The path string is autocompleted from the OpenAPI spec.

---

## Proxy & Environment

- **Dev**: Vite proxy in `vite.config.ts` — `/api/*` → `http://localhost:8080` (Envoy gateway)
- **Prod**: nginx reverse proxy does the same routing
- `baseUrl: ''` works in the openapi-fetch client because of the proxy
- **Env vars**: `.env` file with `VITE_GOOGLE_CLIENT_ID` (Google OAuth client ID). Typed in `src/env.d.ts`. Baked into bundle at build time by Vite — must be set as a GitHub Actions secret for CI/CD (`secrets.VITE_GOOGLE_CLIENT_ID`)

---

## Type System

### Generation

After modifying backend DTOs, regenerate frontend types:
```bash
# Requires fleet-simulator (8085) AND layout-service (8082) running
node scripts/generate-merged-api-types.js
```

This merges OpenAPI specs from both services into `types/generated-api.ts`.

### Type imports

```typescript
// CORRECT — use flattened re-exports from generated.ts
import type { SimulationConfigDTO, TpqMetricsResponseDTO } from '../types/generated'

// WRONG — don't import components directly
import type { components } from '../types/generated-api'
```

To add a new type alias, edit `types/generated.ts`:
```typescript
export type NewTypeName = components['schemas']['NewSchemaName']
```

### Other type files

- `types/index.ts` — barrel for UI-only types (`SimulationConfig`, `MapConfig`, `ScenarioInfo`, etc.). API types should NOT be imported from here.
- `types/type-guards.ts` — config accessor helpers: `getRobotSpeed()`, `getRobotCount()`, `getStationCount()`, `getOperatorTime()`, `getPicksPerFace()`. These extract values from `SimulationConfig` based on system type (RTP vs Relay).
- `types/visualization.ts` — `VisualizationMode` union type (6 modes)

---

## Map & Visualization (Konva)

The warehouse map is a **Konva** canvas (react-konva), NOT a geographic map library. See `references/visualization.md` for full details.

**Key concepts:**
- `WarehouseMap.tsx` (885 lines) — Konva `Stage` → `Layer` with inline static elements + 7 delegated layer components
- **Coordinate flip**: backend Y=0 at bottom → canvas Y=0 at top. Formula: `canvasRowIndex = gridDepth - gridY`
- **6 visualization modes**: `inventory`, `picking_tasks`, `relay_occupancy`, `cell_occupancy`, `conflict_factor`, `combined_overlay`
- **Performance**: `isLargeGrid` flag (>1000 cells), `listening={false}` + `perfectDrawEnabled={false}` on static elements, shadows disabled when racks >= 1000, `memo()` on component export
- **Tooltips**: HTML `<div>` overlays positioned absolutely (Konva can't use DOM tooltips)

**Layer components** (`components/common/maps/layers/`):
`CellOccupancyLayer`, `CombinedOverlayLayer`, `ConflictFactorLayer`, `RelayPositionsLayer`, `RobotsLayer`, `StationsLayer`, `StorageSpacesLayer`

---

## Binary Snapshot Pipeline

Simulation playback uses a custom binary format. See `references/snapshot-pipeline.md` for full details.

**Data flow**: `fetchBinarySnapshots()` → `ArrayBuffer` → `BinarySnapshotReader.readAll()` → `Snapshot[]` → `useLazySnapshotReconstruction` (LRU cache + keyframe/delta reconstruction) → `AnimationControls` (playback at 0.5x–5x) → `WarehouseMap`

---

## Data Fetching

- **No data fetching library** — manual `useEffect` + `useState` for loading/error/data
- Pattern: `const [data, setData] = useState(null); const [loading, setLoading] = useState(false); const [error, setError] = useState(null);`
- Binary snapshots bypass openapi-fetch (raw `fetch()`)
- `useWarehouseMap` hook has its own 5-minute in-memory cache (`Map<string, CacheEntry>` with TTL)

---

## Error Handling

- **No ErrorBoundary** — unhandled errors crash the page
- **ErrorModal** (`components/common/feedback/ErrorModal.tsx`) — Dialog with "Try Again" and "Close" buttons; used for simulation errors
- Pattern: `try/catch` around API calls, store error message in `useState`, display conditionally
- No toast/notification system

---

## shadcn/ui Components

```bash
cd apps/frontend
npx shadcn@latest add <component>    # e.g., button, dialog, select
```

**Never modify files in `components/ui/`.** If you need customization:
1. Create a wrapper component in `components/common/`
2. Import and extend the shadcn component

---

## TypeScript

- Config: `tsconfig.app.json` (not `tsconfig.json`)
- Functional components with hooks
- No state management library — local `useState` throughout, prop drilling from `SimulatorPage`

---

## Formatting & Linting

```bash
npm run check:fix              # Lint + format with Biome (from repo root)
npx biome check --write .     # Direct Biome invocation
```

---

## Build & Tooling

- **Vite** with React plugin, dev on `:4200`, preview on `:4300`
- **Tailwind CSS v3** via PostCSS (`tailwind.config.js` + `postcss.config.js`)
- **recharts** for chart components (BarChart, TimelineChart, SKUHistogram)
- **react-konva** / **konva** for warehouse map canvas
- Konva test alias in `vite.config.ts`: forces browser build in Vitest
- `test-setup.ts`: mocks react-konva components as `<div>` elements, mocks `canvas`, `ResizeObserver`, pointer capture

---

## Testing

```bash
npx nx test frontend               # Vitest + React Testing Library
```

Test files primarily live in `__tests__/` directories alongside source. One exception: `AlgorithmSelectionModal.test.tsx` is co-located directly with its component.

Test setup (`test-setup.ts`) mocks: react-konva components, canvas context, ResizeObserver, pointer capture methods, fake-indexeddb.

---

## Related Skills

- `.claude/skills/commands/` — All build/test/run commands
- `.claude/skills/architecture/` — API routing through Envoy
