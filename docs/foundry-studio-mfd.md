# DES run payloads, one per Agent Studio screen

A run is not one config event. The runner exports the site, generates the
workload, compiles the flow and writes `spec.json`; dessim reads that file.
A field appears below only if its screen stores it. Anything a screen does not
hold is marked **omitted**, and the engine uses its own default.

Values in the JSON examples are the screen defaults unless noted. Angle-bracket
values are filled in by the runner.

---

## 1. Maps: creates the site

```json
{
  "customer": "<customer>",
  "slug": "<slug>",
  "name": "<name>",
  "template": "i-shape",
  "knobs": {
    "aisles": 4,
    "bays": 4,
    "vtms_per_aisle": 2,
    "station_pitch": 4250,
    "htms": 24,
    "orientation": 0,
    "pick_stations": 4,
    "putaway_stations": 2
  }
}
```

- `template`: `i-shape | l-shape | rectangle | square | u-shape`.
- `knobs` are integers only. An omitted knob stays at the preset default, so
  send only the ones the user changed.

| Group | Knob (default) | Templates |
|---|---|---|
| Shared | `aisles` (4), `bays` (4), `vtms_per_aisle` (1 or 2, default 2), `station_pitch` mm (4250), `htms` (24) | all |
| Outline | `width`, `depth`, `orientation` (0 flow +x, 1 flow +y) | I |
| Outline | I's knobs plus `cut_width` (20400), `cut_depth` (13600), `orientation` (0 +x+y, 1 −x+y, 2 +x−y, 3 −x−y) | L |
| Outline | `width`, `depth` | Rectangle |
| Outline | `side` | Square |
| Outline | I's knobs plus `court_width` (13600), `court_depth` (13600), `orientation` (0 open +y, 1 open −y) | U |
| Stations | `pick_stations` (4), `putaway_stations` (2) | I, L, U |
| Stations | `pick_minus_y` (4), `pick_plus_y` (0), `pick_minus_x` (0), `pick_plus_x` (0), `putaway_minus_y` (0), `putaway_plus_y` (2), `putaway_minus_x` (0), `putaway_plus_x` (0) | Rectangle, Square |

`width`, `depth` and `side` are derived unless set.

**What the run receives:** the exported world document
`world/site.world.json` at the plan's changesets, not the knobs again. The VTM
count comes from the map (`vtms_per_aisle`), not from settings.

---

## 2. Inventory & orders: read-only on this page

The values live on the scenario. The payload is the scenario workload:

```json
{
  "workload": {
    "orders_per_hour": 300,
    "lines_per_order": 3,
    "sku_count": 400,
    "inventory_fill_pct": 80,
    "units_per_stored_tote": 1,
    "release": "all_at_start",
    "seed": 0,
    "order_trays": 0
  }
}
```

| Field | Default | Range / values |
|---|---|---|
| `orders_per_hour` | 300 | 1–10000 |
| `lines_per_order` | 3 | 1–50 |
| `sku_count` | 400 | 1–100000 |
| `inventory_fill_pct` | 80 | 0–100; share of the map's inventory tote slots stocked at t=0 |
| `units_per_stored_tote` | 1 | 1–10000 |
| `release` | `all_at_start` | `all_at_start \| even \| random \| waves` |
| `wave_every_sim_ms` | omitted | required only when `release` is `waves` |
| `seed` | 0 | integer, 0 to 2^53−1; the **generator** seed |
| `order_trays` | 0 | FillInventory setup; not shown on the profile card |

The generator writes `data/items.csv` and `data/orders.csv`, and the run points
at those files. **The run does not carry the generator numbers** except
`units_per_stored_tote` (as `scenario.units_per_tote`).

---

## 3. Agents: nothing to send

- Fleet size is already in the map knobs: `htms` and `vtms_per_aisle`.
- Who carries each leg is the process flow.
- Speed, acceleration, lift, pick, drop and turn: **omitted**. The engine
  default stands.
- **Kinematics: omitted.**

---

## 4. Process flow: required

A site with no configured flow does not run. The compiler turns the flow graph
into two outputs.

**`composition`**: the task list, compiled from the flow graph.

**Settings merged into `spec.json`.** Send only the ones the flow actually
states:

```json
{
  "operator": {
    "time_per_pick_secs": "<from flow>",
    "pallet_swap_secs": "<from flow>"
  },
  "putaway": {
    "enabled": "<from flow>",
    "random_slots": "<from flow>",
    "ignore_hour_gate": "<from flow>",
    "htm_wip_factor": "<from flow>",
    "forbid_rack_after_secs": "<from flow>"
  },
  "order": {
    "direct_extraction": "<from flow>"
  },
  "pick": {
    "embargo_secs": "<from flow>"
  }
}
```

Any key the flow does not state is **omitted**.

---

## 5. Stations: the same flow draft

Stations edit the same flow draft as Process flow, one card per station group.
The editable fields are stored on the flow node:

```json
{
  "pick_sim_ms": 0,
  "tote_swap_sim_ms": 0,
  "presentations_per_hour": 0,
  "admission_factor": 0,
  "sequencing": "<value>"
}
```

(Zeros and `<value>` are placeholders for shape only, not defaults.)

- `pick_sim_ms` is shown in seconds on the card.
- The card also shows **member count** and **operation** (`pick`, `put` or
  `decant`). These are displayed, not stored as editable fields.
- Queue limit, breaks and shifts are placeholders: **omitted**. Do not put
  them in the payload.

---

## 6. Scenarios: the run request

```json
{
  "workload": { "...": "the inventory block from screen 2" },
  "run_controls": {
    "duration_sim_ms": 14400000,
    "seed": 0,
    "floor_traffic": "reserved",
    "report_window": {
      "from_sim_ms": 0,
      "to_sim_ms": null,
      "before_name": "warm-up"
    }
  },
  "sweep": null,
  "repeats": null
}
```

| Field | Default | Notes |
|---|---|---|
| `duration_sim_ms` | 4 h (14 400 000) | |
| `seed` | 0 | the **simulation** seed, separate from the generator seed |
| `floor_traffic` | `reserved` | `reserved \| ghost` |
| `report_window.from_sim_ms` | 0 | |
| `report_window.to_sim_ms` | `null` | `null` = end of run |
| `report_window.before_name` | `"warm-up"` | |

- **Sweep** is optional: one field at a time, plus **repeats**. A sweep does
  not change the site. The exact JSON shape of `sweep` and `repeats` is not
  stated in the source, so it is left as `null` here rather than invented.
- Pressing **Run** seals this version and submits it.

---

## 7. Sim runs: not an input

Results, playback and comparison only. **No payload.**

---

## Assembled `spec.json` (what dessim consumes)

Worked example with all defaults, a 4 h run and `reserved` floor traffic. The
`composition` and any flow settings come from the compiled process flow and
stations.

```json
{
  "world": "world/site.world.json",
  "scenario": {
    "items_csv": "data/items.csv",
    "orders_csv": "data/orders.csv",
    "units_per_tote": 1
  },
  "composition": "<compiled from the process flow>",
  "settings": {
    "sim_duration": { "secs": 14400, "nanos": 0 },
    "rng_seed": 0,
    "fidelity": "CollisionDes",
    "vtm": { "isolation": "off" },
    "slice_width_ms": 10000,
    "order": { "release_all_at_start": false },
    "tripwires": false
  }
}
```

Flow settings from Process flow and Stations are merged into `settings`, and
only the ones the flow states. Examples: `operator`, `putaway`, `pick`, and
`order.direct_extraction` merged into the `order` object above.

### How each field is filled

| `spec.json` field | Source |
|---|---|
| `world` | The exported `world/site.world.json` (screen 1) |
| `scenario.items_csv`, `scenario.orders_csv` | Files the generator writes (screen 2) |
| `scenario.units_per_tote` | `units_per_stored_tote` (screen 2) |
| `composition` | Compiled from the process flow (screen 4) |
| `settings.sim_duration` | `duration_sim_ms` split into `secs` = `duration_sim_ms / 1000` and `nanos` = the remainder (screen 6) |
| `settings.rng_seed` | `run_controls.seed` (screen 6) |
| `settings.fidelity` | `reserved` → `CollisionDes`; `ghost` → `Ghost` (screen 6) |
| flow settings | Process flow and Stations (screens 4 and 5) |

### Fixed by the runner, not by any screen

| Setting | Value |
|---|---|
| `vtm.isolation` | `"off"` |
| `slice_width_ms` | `10000` |
| `order.release_all_at_start` | `false` |
| `tripwires` | `false` |

### Omitted on purpose

- `wall_timeout_secs` and `no_watchdog`.
- Kinematics, queues, breaks and shifts: those screens show them as not held,
  and the engine uses its own defaults.
- Any Kafka event: none is part of this payload.

**Order arrival shape** is the generator's `release` field, already baked into
`orders.csv`. It is **not** `order.release_all_at_start`, which the runner
fixes at `false`.
