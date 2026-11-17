# A5 Shuttle Frontend Implementation

## What is an A5 Shuttle?

The **Quicktron A5** (part of the QuickBin Ultra system) is a rack-mounted vertical picker designed for high-density warehouse operations. Unlike traditional mobile VTM robots that travel on the ground, the A5 shuttle:

- **Rides on rails mounted to the rack** - moves vertically and horizontally within a single aisle
- **Is aisle-bound by design** - cannot cross to other aisles (physically constrained to its rail system)
- **Operates alongside ground robots** - while A5 handles vertical picking, M5E ground movers transport totes through narrow aisles

**Key specs:**
- Travel/lift speed: up to 3 m/s
- Pick frequency: ~200 picks/hour
- Minimum aisle width: 860mm (allows multiple robots in same aisle)
- 12-hour battery life

**Why simulate it?** The aisle-bound constraint fundamentally changes fleet sizing - you need at least one shuttle per aisle, unlike zone-bound VTMs that can cover multiple aisles.

## Backend Changes (Already Implemented)

Backend now supports A5 shuttle robots as an alternative to VTMs. Shuttles are aisle-bound (cannot move between aisles), unlike regular VTMs which can move within their zone.

## API Fields

Two new optional fields added to simulation config:

| Field | Type | Default | Values |
|-------|------|---------|--------|
| `vtmType` | string | `"robot"` | `"robot"` or `"shuttle"` |
| `vtmModel` | string | `null` | e.g., `"quicktron-a5"` |

**Example API request:**
```json
{
  "systemType": "relay",
  "vtmType": "shuttle",
  "vtmModel": "quicktron-a5",
  "numVtms": 8,
  "numHtms": 4,
  ...
}
```

## Frontend Tasks

### 1. Add VTM Type Selector

**Location:** Relay system configuration panel (where numVtms/numHtms are set)

**UI:** Radio buttons or dropdown
- "Robot (Zone-bound)" - default
- "Shuttle (Aisle-bound)"

### 2. Add VTM Model Field (Optional)

**Location:** Below VTM type selector, only visible when "Shuttle" selected

**UI:** Text input or dropdown with presets
- Placeholder: "e.g., quicktron-a5"
- Optional field

### 3. Validation Hint

When shuttle mode selected, show helper text:
> "Shuttle mode requires at least one VTM per aisle. Ensure numVtms >= number of rack columns."

### 4. Regenerate Types

After merging backend changes, regenerate frontend types:
```bash
node scripts/generate-merged-api-types.js
```

## Files to Modify

- `apps/frontend/src/components/SimulationConfig.tsx` (or similar config form)
- Regenerate: `apps/frontend/src/types/api.ts`

## Validation

Backend will reject requests where `vtmType: "shuttle"` and `numVtms < numberOfAisles` with a clear error message.
