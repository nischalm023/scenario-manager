# Infrastructure Scenario Schema

## InfrastructureScenario Record

```java
public record InfrastructureScenario(
    String id,                              // Required, unique identifier
    String name,                            // Required, human-readable name
    String description,                     // Optional, detailed description
    String createdDate,                     // ISO 8601 timestamp
    String systemType,                      // "rtp" or "relay"
    LayoutConfigDTO mapConfig,              // Required, layout configuration
    RobotConfig robotConfig,                // Required, RtpRobotConfig or RelayRobotConfig
    OperatorConfig operatorConfig           // Required, operator parameters
)
```

---

## Complete JSON Schema

### RTP System Example

```json
{
  "id": "large_rtp_warehouse",
  "name": "Large RTP Warehouse",
  "description": "Production-scale RTP warehouse with 8 robots and 6 stations",
  "createdDate": "2025-01-15T10:30:00",
  "systemType": "rtp",
  "mapConfig": {
    "gridWidth": 40,
    "gridDepth": 30,
    "cellSizeMeters": 1.0,
    "stationLayout": {
      "layoutType": "SINGLE_BOTTOM",
      "totalStations": 6
    },
    "highwayConfig": {
      "highwaysPerEdge": 2
    },
    "rackBlockConfig": {
      "rowsPerBlock": 6,
      "rackColumns": 4,
      "crossAisleWidthGridUnits": 2
    }
  },
  "robotConfig": {
    "configType": "rtp",
    "numRobots": 8,
    "robotSpeed": 1.5
  },
  "operatorConfig": {
    "operatorTimePerPick": 12.0,
    "numBins": 8,
    "targetPicksPerFace": 3.0
  }
}
```

### Relay System Example

```json
{
  "id": "relay_high_capacity",
  "name": "High Capacity Relay Warehouse",
  "description": "Relay warehouse with zone-based VTMs and 4 HTMs",
  "createdDate": "2025-01-15T10:30:00",
  "systemType": "relay",
  "mapConfig": {
    "gridWidth": 50,
    "gridDepth": 30,
    "cellSizeMeters": 1.0,
    "stationLayout": {
      "layoutType": "SINGLE_BOTTOM",
      "totalStations": 4
    },
    "highwayConfig": {
      "highwaysPerEdge": 2
    },
    "rackBlockConfig": {
      "rowsPerBlock": 4,
      "rackColumns": 3,
      "crossAisleWidthGridUnits": 1
    },
    "verticalToteCapacityInRack": 12
  },
  "robotConfig": {
    "configType": "relay",
    "vtmType": "ROBOT",
    "vtmModel": null,
    "numVtms": 6,
    "vtmHorizontalSpeed": 1.5,
    "vtmVerticalSpeed": 1.5,
    "vtmCapacity": 8,
    "numHtms": 4,
    "htmSpeed": 1.5
  },
  "operatorConfig": {
    "operatorTimePerPick": 12.0,
    "numBins": 8,
    "targetPicksPerFace": 3.0
  }
}
```

### Shuttle VTM Example

```json
{
  "id": "shuttle_warehouse",
  "name": "Shuttle-Based Warehouse",
  "description": "Relay warehouse using Quicktron A5 shuttles (aisle-bound)",
  "systemType": "relay",
  "mapConfig": {
    "gridWidth": 60,
    "gridDepth": 40,
    "cellSizeMeters": 1.0,
    "stationLayout": {
      "layoutType": "SINGLE_BOTTOM",
      "totalStations": 4
    },
    "rackBlockConfig": {
      "rackColumns": 5,
      "rowsPerBlock": 8,
      "crossAisleWidthGridUnits": 1
    },
    "verticalToteCapacityInRack": 10
  },
  "robotConfig": {
    "configType": "relay",
    "vtmType": "SHUTTLE",
    "vtmModel": "quicktron-a5",
    "numVtms": 12,
    "vtmHorizontalSpeed": 3.0,
    "vtmVerticalSpeed": 2.0,
    "vtmCapacity": 1,
    "numHtms": 6,
    "htmSpeed": 2.5
  },
  "operatorConfig": {
    "operatorTimePerPick": 10.0,
    "numBins": 12,
    "targetPicksPerFace": 4.0
  }
}
```

---

## LayoutConfigDTO Schema

```json
{
  "gridWidth": 20,                          // Grid units (world = gridWidth * cellSizeMeters)
  "gridDepth": 20,                          // Grid units
  "cellSizeMeters": 1.0,                    // Physical size per cell (default 1.0)
  "stationLayout": { ... },                 // Station placement configuration
  "highwayConfig": { ... },                 // Highway lane configuration
  "rackBlockConfig": { ... },               // Rack organization
  "verticalToteCapacityInRack": 12          // Totes per rack (relay only, 8-20)
}
```

### StationLayoutDTO

```json
{
  "layoutType": "SINGLE_BOTTOM",            // Station edge placement
  "totalStations": 4,                       // Total stations in warehouse
  "stationEdges": null                      // Optional: List of StationEdgeDTO for multi-edge layouts
}
```

**Layout Types:**
- `SINGLE_BOTTOM` - All stations on bottom edge (default)
- `SINGLE_TOP` - All stations on top edge
- `SINGLE_LEFT` - All stations on left edge
- `SINGLE_RIGHT` - All stations on right edge

### HighwayDTO

```json
{
  "highwaysPerEdge": 2                      // Highway lanes per warehouse edge (1-4)
}
```

### RackBlockDTO

```json
{
  "rowsPerBlock": 4,                        // Rows of racks per block (1-10)
  "rackColumns": 2,                         // Columns per block (1-5, critical for set-cover)
  "crossAisleWidthGridUnits": 2             // Cross-aisle width in grid units (1+)
}
```

---

## RobotConfig (Sealed Interface)

### RtpRobotConfig

```json
{
  "configType": "rtp",                      // Required discriminator
  "numRobots": 4,                           // 1-100
  "robotSpeed": 1.5                         // 0.1-5.0 m/s
}
```

### RelayRobotConfig

```json
{
  "configType": "relay",                    // Required discriminator
  "vtmType": "ROBOT",                       // "ROBOT" or "SHUTTLE"
  "vtmModel": null,                         // Optional model name (e.g., "quicktron-a5")
  "numVtms": 4,                             // 1-100
  "vtmHorizontalSpeed": 1.5,                // 0.1-5.0 m/s
  "vtmVerticalSpeed": 1.5,                  // 0.1-5.0 m/s
  "vtmCapacity": 8,                         // 4-32 totes per VTM
  "numHtms": 4,                             // 1-100
  "htmSpeed": 1.5                           // 0.1-5.0 m/s
}
```

**VTM Types:**
- `ROBOT` - Zone-bound, can move between aisles within zone
- `SHUTTLE` - Aisle-bound (e.g., Quicktron A5), fixed to one aisle

---

## OperatorConfig Schema

```json
{
  "operatorTimePerPick": 12.0,              // 0.1-30.0 seconds per pick
  "numBins": 8,                             // 4-32 picking bins at station
  "targetPicksPerFace": 3.0                 // 1.0-10.0 picks per display face
}
```

---

## Validation Constraints

| Field | Constraint | Notes |
|-------|------------|-------|
| `systemType` | `rtp` or `relay` | Normalized to lowercase |
| `gridWidth`, `gridDepth` | > 0 | Default 20 |
| `cellSizeMeters` | > 0 | Default 1.0 |
| `rackColumns` | 1-5 | Critical for set-cover optimization |
| `rowsPerBlock` | 1-10 | Default 4 |
| `highwaysPerEdge` | 1-4 | Default 2 |
| `totalStations` | 1-20 | Default 4 |
| `numRobots` (RTP) | 1-100 | Default 4 |
| `robotSpeed` (RTP) | 0.1-5.0 | Default 1.5 m/s |
| `numVtms` (Relay) | 1-100 | Default 4 |
| `numHtms` (Relay) | 1-100 | Default 4 |
| `vtmSpeed` (Relay) | 0.1-5.0 | Default 1.5 m/s |
| `htmSpeed` (Relay) | 0.1-5.0 | Default 1.5 m/s |
| `vtmCapacity` (Relay) | 4-32 | Default 8 totes |
| `verticalToteCapacityInRack` | 8-20 | Default 12, relay only |
| `operatorTimePerPick` | 0.1-30.0 | Default 12.0 seconds |
| `numBins` | 4-32 | Default 8 |
| `targetPicksPerFace` | 1.0-10.0 | Default 3.0 |

---

## File Location

`apps/fleet-simulator/src/main/resources/scenarios/infrastructure/{id}/scenario_info.json`

## Related Classes

- `ScenarioModels.InfrastructureScenario` - Record definition
- `InfrastructureScenarioManager` - CRUD operations
- `LayoutConfigDTO` - Layout configuration DTO
- `SimulationConfigs.RobotConfig` - Sealed interface for robot configs
- `SimulationConfigs.OperatorConfig` - Operator parameters
