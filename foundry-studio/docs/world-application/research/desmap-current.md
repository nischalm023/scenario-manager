# desmap as it stands: format, materialization, consumers, and where it fights the mm-first vision

Research for the unified world/scenario format ("the proto"). Everything below is receipted against
the foundry checkout at `master` (commit `bd1e7fc3`) plus the `walmart-dallas-fixture` branch
(`5cf20db0`), read-only.

---

## 1. Executive summary

**What desmap actually is.** Two formats, not one. The *authoring* format is
`desmap::design::WarehouseDesign` (`libs/desmap/src/design/types.rs:14-47`) — a v3 JSON document of
rectangular stamps over an integer grid. The *compiled* format is `desmap::WarehouseMap`
(`libs/desmap/src/map/mod.rs:23-46`) — a fully materialized cell/edge graph plus derived station,
relay, zone, and block lists. `WarehouseDesign::compile()` (`libs/desmap/src/design/compile.rs:26-134`)
goes one way; `design::extract()` (`libs/desmap/src/design/extract.rs:14-59`) goes back, but lossily
(it drops `racks`, `clip_boxes`, and `parking_zones` outright — lines 51-55).

**The grid is the substrate, not a view.** A design declares `grid { width, depth, cell_size_mm }`
and every physical thing is placed by `[col, row]`: highways, zones, stations, presentation cells,
queue paths, clip boxes, parking. The compiler allocates one cell per grid position and derives its
identity arithmetically — `CellId(row * width + col)` appears in at least six places
(`compile.rs:246-248`, `compile.rs:408-410`, `compile.rs:668`, `graph_builder.rs:92`,
`graph_builder.rs:133`, `queue_edges.rs:77-79`). Millimetres exist, but only as a *derivative*: a
cell's `world_pos` is computed at compile time from the axis geometry (`graph_builder.rs:99`), and no
authored field except rack-slot geometry is in mm.

**Two escapes from the grid already exist, and both are bolted on.**
`GridDefinition::row_centers_mm` (`design/types.rs:70-71`) lets rows sit at surveyed positions instead
of a uniform pitch — added because a real cencora rack's relay pitch alternates 645/645/850 mm and
"no uniform pitch lands on that sequence at any value" (`design/grid_geometry.rs:1-12`). Columns have
no equivalent ("add one the same way when a map needs it", `design/types.rs:69`). And rack prototypes
(`RackDef`/`RackSlotDef`/`RackPlacement`, `design/types.rs:271-328`) declare slot geometry in true mm
in the rack's own bottom-left frame — the only genuinely mm-native declaration in the format. The
compiler then *snaps every slot back onto a grid cell* by nearest-row-center interpolation
(`compile.rs:576-597`), and dessim and desvis both re-interpolate the mm frame onto cell centers
(`libs/dessim/src/scenario_gen/topology.rs:224-230`; `apps/desvis/src/rack_geometry.rs:8-19`). The mm
data survives a round trip through the grid and comes out stretched — desvis's own module doc admits
cencora renders "12,840 mm of bays across a 14,450 mm cell-center span, ~18% wide."

**Identity is positional almost everywhere.** `CellId` is documented as "opaque… NOT a coordinate"
(`libs/desgraph/src/types.rs:8-10`) and is in fact `row * width + col`. `RelayId` is a compile-order
counter (`compile.rs:243-251`). `RackId` and `StorageSpaceId` are list indices minted in dessim, not
in the map (`libs/dessim/src/scenario_gen/inventory.rs:38,59`;
`libs/dessim/src/scenario_gen/pipeline.rs:298`). Racks, slots, diving boards, highways, clip boxes,
and parking zones have no ids at all. Only `ZoneId`, `StationId`, and `BlockId` come from
author-written numbers, and `deseditor` had to invent a *parallel* stable-id side table because those
numbers are user-editable data rather than identity (`libs/deseditor/src/ids.rs:1-6`).

**The declared/derived split is one-way.** Racks are declared and cells are derived; you cannot
declare a cell. `JunctionCell` (`design/types.rs:419-429`) is the sole exception — an explicitly
authored node with `world_pos` in mm, explicit adjacency, and an id the author must keep above
`width * depth` to avoid colliding with the derived id space (`graph_builder.rs:113-115`). It is the
proof that the format *wants* directly-declarable nodes and got one escape hatch instead of a rule.

**Composition is already unified, and that part works.** `dessim::SimSpec`
(`libs/dessim/src/sim_spec.rs:14-22`) flattens `WarehouseDesign` at the JSON root and adds
`scenario` (demand source) and `settings` (sim/robot/algorithm parameters). One file describes a run.
`deseditor` preserves unknown top-level keys verbatim as `passthrough` (`libs/deseditor/src/io.rs:1-8`)
— which is how the desvis editor stores its reference-drawing calibration under `editor_meta.underlay`
(`apps/desvis/src/app/editor/underlay.rs:1-9`). That passthrough slot is an unnamed request for
vendor/source frames as first-class objects.

**Biggest gaps against the Walmart Dallas needs.** No sector semantics (the `Sector` type exists and
is never populated — `design/graph_builder.rs` writes `sector: None` at lines 97 and 121). No robot fleet beyond
two integers (`FleetConfig { htms, rtps }`, `map/types.rs:11-19`). No carrier types (`ToteType` is a
render-only dessim enum, `libs/dessim/src/model/location.rs:35-41`). No station internals beyond one
presentation cell and a linear queue path. No structural double-deep — Dallas encodes it as two
slots at the *same* `(x, y)` and the VTM model charges a global 80/20 blended fork time
(`libs/dessim/src/robots/vtm/motion.rs:24-34`). No cross-aisle through a rack run: VTM reachability is
BFS restricted to `Aisle` cells (`design/reachability.rs:38-58`) and per-edge `vtm_zone_gate` pins
HTMs to one zone's relays (`graph_builder.rs:451-486`).

---

## 2. desmap format reference

### 2.1 The authoring document — `WarehouseDesign` (`libs/desmap/src/design/types.rs:14-47`)

Serde field names are the JSON keys; `deseditor` pins the list at `libs/deseditor/src/io.rs:20-32`.

| Field | Type | Units | Notes |
|---|---|---|---|
| `version` | `u32` | — | must equal 3, else compile errors (`compile.rs:27-29`) |
| `grid` | `GridDefinition` | mixed | see below |
| `highways` | `Vec<HighwayDefinition>` | cells | optional |
| `zones` | `Vec<ZoneDef>` | cells | required (may be empty array) |
| `racks` | `Vec<RackDef>` | **mm** | prototype library; unreferenced entries allowed |
| `stations` | `Vec<StationDefinition>` | cells | required |
| `edge_exceptions` | `Vec<EdgeException>` | cells + mm | per-directed-edge overrides |
| `junction_cells` | `Vec<JunctionCell>` | **mm** | off-grid nodes with explicit adjacency |
| `clip_boxes` | `Vec<ClipBox>` | cells | blank out regions |
| `parking_zones` | `Vec<ParkingZone>` | cells | |
| `fleet` | `Option<FleetConfig>` | counts | |

**`GridDefinition`** (`design/types.rs:53-72`): `width: u32`, `depth: u32`, `cell_size_mm: f64`,
`row_centers_mm: Option<Vec<f64>>`. `cell_size_mm` is authoritative for columns always, and for rows
only when `row_centers_mm` is absent. Row centers must be `depth` entries, finite, non-negative, and
*strictly* increasing — equal centers are rejected because a zero-length edge would read as a free
move (`design/grid_geometry.rs:43-69`). Positions rather than extents, deliberately: extents "correspond
to no dimension on a drawing" under non-uniform spacing (`grid_geometry.rs:16-22`).

**`HighwayDefinition`** (`design/types.rs:97-108`): `origin: [u32;2]` (col,row) + `width`/`height` in
cells + `direction: LaneDirection`. `LaneDirection` (lines 82-92) is one of `Bidirectional`,
`Eastbound`, `Westbound`, `Northbound`, `Southbound`, `NorthSouthOnly`, `EastWestOnly`. A square block
(width == height, including 1×1) silently loses its direction — `build_lane_dir_maps` only branches on
strict inequality (`graph_builder.rs:142-149`), and the editor raises
`EditWarning::HighwayDirectionIgnored` for it (`libs/deseditor/src/commands.rs:116-119`).

**`ClipBox`** (`design/types.rs:114-122`) and **`ParkingZone`** (`125-133`): identical shape — cell
origin + width + height. Clips stamp `CellKind::Blank`; a clip overlapping any zone is a hard compile
error (`compile.rs:366-381`).

**`ZoneDef`** (`design/types.rs:136-141`): tagged enum, `{"type": "relay" | "rtp", ...}`.

**`RelayZone`** (`design/types.rs:152-201`), `deny_unknown_fields`:
`id: u32`, `origin: [col,row]`, `length: u32` (rack rows), `sides: Sidedness` (`Dual`/`LeftOnly`/
`RightOnly`), `vertical_tote_capacity: u32` (default 8), `zone_height_mm: u64` (default 9000),
`zone_width_mm: Option<u64>`, `real_shelf_count: u16` (default 10), `rack_placements: Vec<RackPlacement>`,
`vtm_model: VtmModel`, `vtm_count: Option<u32>` (default 2 at `compile.rs:644`). The zone footprint is
a hard-coded three-column sandwich — rack / aisle / rack for `Dual`, two columns otherwise
(`compile.rs:191-195`). `vertical_tote_capacity` carries an admitted lie: the doc says it "may be
inflated as a hack for inventory capacity" independent of `real_shelf_count`, which is the physical
truth (`design/types.rs:176-179`).

**`RackDef`** (`design/types.rs:271-287`): `name: String` (the only string-keyed identity in the
format), `length_mm: f64`, `slots: Vec<RackSlotDef>`. Frame convention is bottom-left: x = 0 at the
left end of the steel envelope, y = 0 at the floor (lines 265-270). Slot order is load-bearing — it
fixes the sim's rack enumeration (line 263).

**`RackSlotDef`** (`design/types.rs:291-309`): `x: f64` mm along the run, `y: f64` mm above floor
(the *fork-interface* height, not a slot midpoint — see the reasoning at `design/types.rs:208-212`),
`diving_board: bool`, `diving_board_slot: Option<u32>` (required on boards, forbidden otherwise;
`compile.rs:467-479`). Every slot x must lie inside `[0, length_mm]` (`compile.rs:485-490`).

**`RackPlacement`** (`design/types.rs:314-328`): `rack: String` (a `RackDef` name), `side: BlockSide`,
`x_offset_mm: f64`. Tiling steps by `length_mm`; steel envelopes on one side must be disjoint
(touching allowed — tiled racks share legs) (`compile.rs:553-562`).

**`RtpZone`** (`design/types.rs:335-344`): `id`, `origin`, `rows`, `cols`. Cells only, no vertical model.

**`StationDefinition`** (`design/types.rs:376-398`): `id: u32`, `position: [col,row]`,
`presentation_cell: [col,row]` (serde alias `access_position`), `edge: Option<StationEdge>`,
`queue_path: Vec<[col,row]>` (ordered, entry first; must be a cardinal-adjacent chain ending adjacent
to the presentation cell — `design/queue_edges.rs:19-43`), `exit_direction: CardinalDirection`
(default East), `bins: Option<usize>`, `station_type: StationType` (`Pick` or `Putaway` only,
`libs/desgraph/src/types.rs:163-168`).

**`EdgeException`** (`design/types.rs:405-416`): `from`/`to` as `[col,row]`, `traversability:
EdgeTraversability`, `distance_mm: Option<u64>`. This is the only per-edge authoring surface, and it
is keyed by grid coordinates on both ends.

**`JunctionCell`** (`design/types.rs:419-429`) / **`JunctionEdge`** (`432-441`): `id: u32` (must be
`>= width*depth`), `world_pos: [f64;2]` **mm**, `kind: CellKind`, `zone: u32`, and explicit
`edges: [{ target, traversability, distance_mm, bidirectional }]`.

**`FleetConfig`** (`libs/desmap/src/map/types.rs:11-19`): exactly `{ htms: usize, rtps: usize }`. No
VTM field — the VTM fleet is summed from per-zone `vtm_count` (`map/mod.rs:100-106`).

### 2.2 The compiled document — `WarehouseMap` (`libs/desmap/src/map/mod.rs:23-46`)

`version`, `system_type` (`Rtp` | `Relay`, inferred from whether any zone is relay —
`compile.rs:155-156`), `grid_bounds: Option<GridBounds>`, `graph: WarehouseGraph`, `stations`,
`relays`, `zones`, `sectors`, `blocks`, `fleet`.

- **`Cell`** (`libs/desgraph/src/graph/types.rs:10-28`): `id`, `kind`, `zone`, `sector: Option`,
  `grid_pos: Option<GridPos>`, `world_pos: Option<WorldPos>` (mm), `barcode: Option<String>`,
  `rack_orientation: Option`, `station_id: Option`. Both position kinds are optional and both are
  carried — the format already ships a dual-frame cell, it just never lets you *author* the mm one.
- **`CellKind`** (`libs/desgraph/src/types.rs:88-118`), 12 variants: `Storage`, `Aisle`, `Open`,
  `Highway`, `StationArea`, `Relay`, `Junction`, `Blank`, `Presentation`, `QueueCell`, `Parking`,
  `Charging`. This single enum carries navigability, ownership, and function all at once.
- **`Edge`** (`graph/types.rs:35-50`): `traversability: EdgeTraversability { vtm_traversal: bool,
  htm_traversal: HtmTraversal, rtp_traversal: bool }`, `distance_mm: u64`, `dest_only: Option<CellId>`,
  `vtm_zone_gate: Option<ZoneId>`. `HtmTraversal` is `None | UnladenOnly | Both`
  (`desgraph/types.rs:125-133`).
- **`Station`** (`libs/desmap/src/station.rs:13-36`): same fields as the definition, with cells as
  `CellId`.
- **`RelayPosition`** (`station.rs:44-57`): `id`, `position: CellId`, `htm_access: CellId`,
  `vtm_access: CellId`, `block: BlockId`, `side: BlockSide`. Note `htm_access == position` always
  (`compile.rs:246-247`).
- **`Zone`** (`libs/desmap/src/zone.rs:13-41`): `id`, `name: Option<String>`, `cells: Vec<CellId>`,
  `relays: Vec<RelayId>`, `vtm_count`, `zone_height_mm`, `zone_width_mm`, `real_shelf_count`,
  `slots: Option<Vec<SlotDef>>` — the materialized rack geometry.
- **`SlotDef`** (`design/types.rs:213-238`): `cell: [col,row]` (derived, never authored — line 219),
  `x: f64` mm, `y: f64` mm, `diving_board`, `diving_board_slot`.
- **`Sector`** (`zone.rs:55-59`) and **`RackBlock`** (`zone.rs:62-79`): `Sector` is `{id, cells}` and
  is never produced by any code path. `RackBlock` is `{id, top_left: GridPos, bottom_right: GridPos,
  rack_cells, aisle_cells}` — pure grid rectangle.
- **`FloorId`** exists as a newtype (`desgraph/types.rs:54-55`) and is used nowhere. Single-floor is
  hard-wired.

---

## 3. Materialization and identity today

### 3.1 What the compiler derives

`WarehouseDesign::compile()` (`compile.rs:26-134`) runs a fixed pipeline:

1. **Axis geometry** — columns always uniform, rows from `row_centers_mm` or uniform
   (`compile.rs:38-42`, `grid_geometry.rs:32-34`).
2. **Rack materialization** — `materialize_rack_geometry` (`compile.rs:454-619`) expands each
   placement's prototype slots into absolute `SlotDef`s, deriving each slot's anchor cell: column from
   the placement side, row by *nearest row center* after linearly anchoring the slot's x onto the
   zone's row-center extent (`compile.rs:576-587`). All relay zones must use placements or none —
   mixing is a hard error, because explicit slots switch the whole map to one-rack-per-slot
   (`compile.rs:502-509`).
3. **Stamping** — highways, then zones, then stations, then parking, then clips
   (`compile.rs:51-57`). Order is semantic: later stamps overwrite earlier ones. Dallas relies on
   this (its parking strips overwrite highway cells).
4. **Relay position generation** — for each rack cell of a relay zone that touches an
   `Open`/`Highway`/`Parking`/`Charging` neighbour, emit one `RelayPosition`
   (`compile.rs:222-254`, `touches_open` at 257-270). **This is per rack *cell*, not per declared
   diving board.** Declared boards do not become relay entities.
5. **Rack orientation** — `NorthSouth` if the block is at least as tall as wide, else `EastWest`
   (`compile.rs:393`). Pure bounding-box inference.
6. **Graph build** — one cell per grid position with `world_pos` from axis centers
   (`graph_builder.rs:90-105`); cardinal edges with per-direction distance from axis pitch
   (`graph_builder.rs:228, 255`); HTM overrides from cell-kind pairs, rack orientation, and relay
   membership (`graph_builder.rs:273-334`).
7. **Queue restriction** — every edge touching a queue or presentation cell is dropped except the
   entry, the forward chain, and the exit; survivors get `dest_only` set to the presentation cell
   (`design/queue_edges.rs:65-158`).
8. **Reachability narrowing** — four BFS passes (VTM, HTM unladen, HTM laden, RTP) downgrade or prune
   edges (`design/reachability.rs:14-106`). The VTM pass walks only `Aisle` cells (lines 38-58) —
   that is the structural reason a VTM cannot leave its aisle.
9. **Auto-parking fallback** — if no `parking_zones` and `htms > 0`, stamp `Open` cells column-major
   from the left until the HTM count is covered, and print a warning to stderr
   (`compile.rs:327-357`). Compile fails if parking cells < HTMs (`compile.rs:69-78`).

### 3.2 How derived things are identified

| Thing | Identity today | Stability |
|---|---|---|
| grid cell | `CellId(row * width + col)` (`compile.rs:246`, `graph_builder.rs:92`) | changes if grid width changes |
| junction cell | author-chosen `u32 >= width*depth` (`graph_builder.rs:113-115`) | manual, collision-prone |
| relay position | `RelayId(n)`, n incremented in zone-then-row order (`compile.rs:243-251`) | changes if any earlier zone changes |
| zone | author-written `u32`; compiled `name` is `format!("zone_{id}")` (`compile.rs:643`) | stable, but also a semantic slot |
| rack block | `BlockId` = the zone's `id` (`compile.rs:214-218`) | aliased with zone id |
| station | author-written `u32` | stable |
| rack prototype | `RackDef::name` string, unique-checked (`compile.rs:461`) | stable |
| **rack placement** | none — positional in `rack_placements` | index |
| **rack slot / storage location** | none in the map; dessim mints `RackId(i)` by list index (`scenario_gen/inventory.rs:38`) and `StorageSpaceId(rack_index * vtc + z)` (`scenario_gen/pipeline.rs:298`) | index of an index |
| **diving board** | none — identified by `(anchor cell, diving_board_slot)` (`compile.rs:599-614`) | positional |
| **highway / clip box / parking zone** | none | positional |
| sector | `SectorId` type exists; never populated | — |

Where this bites: (a) metrics are keyed by `CellId` and `RelayId`, so any grid edit renumbers every
downstream series — the observer builds `cell_positions` and `cell_kinds` as dense arrays indexed by
`CellId.0` (`libs/dessim/src/solver.rs:186-199`); (b) the editor cannot reference an element across an
undo without its own side table (`libs/deseditor/src/ids.rs:1-6`); (c) `StorageSpaceId` is derived from
a rack's *position in a sorted cell list*, so inserting one rack cell shifts every inventory identifier
after it; (d) round-tripping through `extract()` renumbers relays because relay ids are regenerated
from block order.

---

## 4. Consumption map — where the grid is load-bearing

### desgraph
`WarehouseGraph` keeps a `grid_lookup: FxHashMap<GridPos, CellId>` and exposes `cell_at(GridPos)`
(`libs/desgraph/src/graph/mod.rs:36, 100-102`). Serialization is a flat `{cells, edges}` with the
indices rebuilt on load (`graph/mod.rs:55-91`). This crate is otherwise frame-agnostic.

### dessim — choke points
- **`FHSippPathfinder`** (`libs/dessim/src/pathfinding/fh_sipp/pathfinder.rs:32-64`) holds
  `cell_positions: Vec<Option<GridPos>>` indexed by `CellId.0`, and the comment states the dense
  `row * width + col` assumption outright (lines 37-41). The A* heuristic is **Manhattan on grid
  ordinals** times a per-cell duration (lines 220-230). On a non-uniform map, admissibility is patched
  by charging each axis its *shortest* edge in mm (`min_axis_edge_distances`, line 143) — an explicit
  workaround for `row_centers_mm`.
- **`select_nearest_*`** (`libs/dessim/src/pathfinding/nearest.rs:92-95, 155-158`) hard-errors with
  `MissingCellPosition` when a candidate cell has no `grid_pos`. Junction cells are therefore
  unreachable as nearest-selection candidates.
- **Reservation table** (`libs/dessim/src/model/reservation/table.rs:1-40`) reserves *cells* over time
  windows. Occupancy granularity is the cell.
- **Parking** (`libs/dessim/src/model/parking.rs:51, 91, 133`) ranks candidates by Manhattan distance
  on `GridPos`.
- **Relay/VTM kinematics** (`libs/dessim/src/model/relay/config.rs:165-201`): `h_dist_m` takes two
  world positions and passes them through `physical_axis_m`, which finds the zone's longer axis span
  and re-normalizes onto `zone_width_m`. Since those world positions were *already* produced by
  interpolating mm slot x onto cell centers (`scenario_gen/topology.rs:224-230`), the true mm geometry
  is normalized twice with a grid round-trip in between.
- **`build_rack_layout`** (`libs/dessim/src/scenario_gen/topology.rs:169-250`) is where slots become
  racks: it groups cells by `(zone, grid_x)`, resolves each slot's authored anchor row back to a
  `CellId`, and panics if the anchor is not a rack cell of that column (line 234). Diving-board slot 0
  becomes `relay_exchange_pos`; higher boards are carried but unused.
- **`build_rack_to_vtm_access`** (`topology.rs:67-94`) panics if any `Relay` cell lacks an adjacent
  `Aisle` cell — the sandwich shape is a runtime invariant, not just a compile convention.
- **Inventory position tables** are `(u32, u32)` grid pairs
  (`libs/dessim/src/model/inventory/types.rs:297-301`).
- **Observer** is the one clean layer: it takes `WorldPos` and `CellKind` arrays and never sees the
  grid (`libs/dessim/src/solver.rs:186-199`).

### deseditor
`EditorDoc` wraps a `WarehouseDesign` plus non-design top-level keys re-emitted verbatim
(`libs/deseditor/src/doc.rs:41-56`, `io.rs:1-8`). All geometry goes through `ModelPoint` / `ModelRect`
(`libs/deseditor/src/geom.rs:11-46`), which are serde-tagged `{"cell":[c,r]}` **specifically so an
`{"mm":[x,y]}` variant can be added later** (`geom.rs:1-7`, `lib.rs:8-14`). The seam is cut and
waiting. The command vocabulary (`libs/deseditor/src/commands.rs:19-62`) can add highways, relay
zones, RTP zones, stations, clip boxes, parking zones; upsert/remove edge exceptions; move, resize,
delete; patch relay-zone and station parameters; set zone/station user ids; resize the grid; set
fleet. **It cannot author racks, rack placements, junction cells, or row centers** — resizing depth
silently drops `row_centers_mm` and warns `RowCentersDropped` because "re-entering it is not an
editor operation" (`commands.rs:126-131`).

### desvis
Renders in a world frame where **1 world unit = 1 grid cell** (`apps/desvis/src/app/central_panel.rs:116`
passes `1.0` as "cell_size in meters"). `build_grid_data` indexes every cell by `y * width + x` and
skips any cell without a `grid_pos` (`apps/desvis/src/app/grid_data.rs:51-70`) — junction cells are
invisible. `rack_geometry` states the whole tension in its own module doc
(`apps/desvis/src/rack_geometry.rs:8-19`): slot mm are mapped by `t = (x - x_min)/(x_max - x_min)`
onto the rack column's cell-center span, matching dessim's interpolation, and this "deliberately
stretches the steel… true-mm rendering would divorce the racks from sim positions." Diving-board tabs,
relay tote reach, and bay proportions are all fractions of a cell (lines 36-51). The editor's
reference-image underlay is the one true-mm surface, and it lives in passthrough JSON, not in the
format (`apps/desvis/src/app/editor/underlay.rs:1-9`).

---

## 5. Dallas fixture anatomy and the format-bending catalog

File: `libs/desmap/tests/fixtures/walmart_dallas_v1.json` on branch `walmart-dallas-fixture`
(7,107 lines). Test: `libs/desmap/tests/walmart_dallas.rs`. Two commits: `e910def5` (survey fixture)
and `5cf20db0` (VTM tunnel racks, #391 layer 1).

### What it declares

- **Grid** 107 × 93, `cell_size_mm: 860.86`, with a 93-entry `row_centers_mm`. Pitch is 850.0 for
  rows 0-7 (the highway/station band), 792.7 once as a transition, then 735.4/735.5 alternating for
  rows 9-84, then 850.0 again at the top band.
- **Highways** two bands: rows 1-7 and rows 85-91, full width, `Bidirectional`.
- **Zones** 20 relay zones, all `origin` row 8, `length` 77, `zone_height_mm` 9754,
  `real_shelf_count` 24, `vertical_tote_capacity` 8, `vtm_count` unset (so 2 each → 40 VTMs).
  Zone 1 is `LeftOnly`, zones 7 and 20 are `RightOnly`, the other 17 are `Dual` → **37 rack sides**.
  Origins cluster into four blocks (7/2/5/6 aisles) with 2-cell or 4-cell open gaps between them.
- **Racks** four prototypes, all `length_mm: 2980.5`: `dallas_bay` (230 slots), `dallas_bay_relay`
  (230 storage + 4 diving boards at y = 394.0), `dallas_bay_tunnel` (120 slots), and
  `dallas_bay_tunnel_relay` (120 + 4 boards).
- **Placements** 19 bays per rack side, tiled at exact multiples of 2980.5 mm; bays 12 and 13 are the
  tunnel variants. 5 of 19 bays carry no relay.
- **Stations** 10: six `Putaway` on row 0, four `Pick` on row 92, each with a 2-cell queue path and an
  `exit_direction`.
- **Parking** 8 rectangles at the column extremes of both highway bands (327 cells for 144 HTMs).
- **Clip box** one, `origin [31, 8]`, 9 × 77 — the building notch between rack blocks 1 and 2.
- **Fleet** `{ htms: 144 }`. **Scenario** BulkSource pointing at the *cencora* CSVs. **Settings**
  10-hour run, `CollisionDes` fidelity, `htm_kinematic` motion, VTM `teleport` isolation.

### Format-bending catalog

1. **Double-deep is a duplicate slot.** Every storage `(x, y)` in every Dallas prototype appears
   exactly twice — 230 slots over 115 distinct `(x, y)` pairs; the tunnel prototypes are 120 over 60.
   There is no depth axis, no front/back distinction, and nothing downstream can tell the two apart.
   The only place depth exists is a constant: `SHALLOW_PROPORTION = 0.8` blending 4.0 s and 5.0 s fork
   times into `WEIGHTED_PICK_PLACE_SECS` (`libs/dessim/src/robots/vtm/motion.rs:26-34`).
2. **Row pitch is reverse-engineered from the rack run, not surveyed.** 19 bays × 2980.5 mm =
   56,629.5 mm; 77 rows at 735.45 mm ≈ 56,629.6 mm. The row grid was chosen so the bay run fits the
   row count, which is the tail wagging the dog: the *relay* pitch is 745.125 mm (4 boards per
   2980.5 mm bay), and the row pitch is not that number.
3. **Bay pitch is a half-millimetre on purpose.** From `e910def5`: "Bay pitch is a half-millimetre
   value deliberately: exact in binary, so an authored `x_offset` lands exactly on the previous bay's
   offset + `length_mm`, which the steel-overlap check requires." Authors are hand-managing float
   equality because placements have no ids and tiling has no `repeat` construct.
4. **Relay count is wrong by construction and the test pins the wrong number.**
   `walmart_dallas.rs:26` asserts `map.relays.len() == 2849` with the comment "uniform fallback: 37
   rack sides × 77 rows (survey: 2,072)". The 56 declared diving boards per side (14 relay bays × 4)
   produce no relay entities; the compiler emits one per rack cell instead (`compile.rs:222-254`). The
   module doc calls this "KNOWN DEGRADATION on this tier" (`walmart_dallas.rs:7-12`).
5. **Every rack cell is a bucket of ~54 racks.** 4,150 storage slots per rack side spread over 77
   rows. The grid cell has stopped being a storage location and become an accounting bin, but it is
   still what pathfinding, reservations, and metrics key on.
6. **The VTM tunnel is a hole in the *inventory*, not a hole in the *floor*.** Bays 12-13 use
   prototypes whose lowest 11 storage levels are simply absent (y starts at 5100.0 instead of 800.0).
   The cross-aisle passage through that opening is not modeled at all — `5cf20db0` says so explicitly
   and defers it to "#391 layers 2-3 (desmap crossing edges, dessim unpinning VTMs from their zone)".
   The format cannot express a rack you drive through.
7. **A survey inconsistency is frozen into the data.** Bay 13 keeps its 4 diving boards "even though
   they sit inside the opening — a flagged inconsistency (#391), deliberately preserved" (`5cf20db0`).
   There is nowhere in the format to *record* that a declared element is disputed.
8. **Parking had to be segmented to avoid a deadlock.** From `e910def5`: "Parking is segmented at the
   column extremes: a full-width parking strip walls off its row east-west (Parking↔Parking edges
   never traverse) and deadlocks the solver." The eight parking rectangles are shaped around an
   emergent traversability rule (`graph_builder.rs:289-297`), not around the site.
9. **The clip box is the only way to say "not our building".** One 9 × 77 rectangle stands in for the
   notch. cencora needed 84 of them (`libs/desmap/tests/fixtures/cencora_v3.world.json`). There is no polygon,
   no named excluded region, no reason attached.
10. **Zone width is inferred and can only be corrected by an override.** Dallas leaves `zone_width_mm`
    unset, so it derives as `span(8, 84) ≈ 56,630 mm` (`compile.rs:651`) — close to but not equal to
    the slot x span of 56,033.4 mm. cencora sets it explicitly (12,200) because the derived value
    "cannot know the real margin past the outermost relay center" (`compile.rs:646-650`).
11. **Cross-aisles are gaps in zone origins, not declared objects.** The 2-cell and 4-cell column gaps
    between blocks are `Open` because nothing stamped them.
12. **The Dallas map runs on cencora demand data.** `scenario.bulk_csv` and `items_csv` point at
    `libs/desmap/tests/fixtures/cencora_data/`. There is no notion of a workload belonging to a site.

### What Kristina could not express at all

Sectors; robot types or capabilities; carrier types; which racks hold inventory versus finished
orders; station internals (waiting blocks, dual presentation points, ingress/egress cells); box
induction or extraction stations; order-buffer racks with dedicated VTMs; double-deep depth; a
drive-through rack opening; a named excluded region; a survey provenance or dispute note; a
per-column pitch (columns are uniform-only by construction, `compile.rs:38`).

---

## 6. Other in-repo world representations, and the term list

| Where | What it encodes | Naming |
|---|---|---|
| `libs/proto/src/grpc/common/v1/types.proto:9-13` | `GridCoordinate { row, col }` — the shared coordinate for all legacy services | **row/col, not x/y** — opposite of desmap's `[col, row]` arrays |
| `libs/proto/src/grpc/layout/v1/layout.proto:20-24, 60-113` | `GridBounds { width, depth, cell_size_meters }`; `LayoutRequest` (grid + station layout + highway + rack-block config); `StationPosition`, `HighwaySegment`, `RackBlockDefinition`, `RelayPositionData`, `RackPositionData` | **metres**, not mm; `accessibility_direction`; `rack_positions`/`aisle_positions` as coordinate lists; string ids (`station_id`, `block_id`) |
| `libs/proto/src/grpc/navigation/v1/navigation.proto` | session-scoped pathfinding: `GridBounds`, `PathWaypoint`, `Orientation`, `RobotType`, `NavigationAction` | `NavigationAction` is a verb vocabulary desmap has no equivalent of |
| `libs/proto/src/grpc/put/v1/put.proto:17-46` | `SkuInfo`, `StorageSpaceInfo`, `StationInfo`, `InventoryAssignment` | **`StorageSpace`** is the legacy name for what desmap calls a rack slot |
| `libs/api-schema/layout-service/openapi.yaml:81-269` | `MapDataRequestDTO`, `CompactMapData`, `MapConfiguration`, `Position`, `StationElement`, `RackElement`, `HighwayElement` | "element" as the generic placed-thing noun |
| `libs/api-schema/fleet-simulator/openapi.yaml:928-1085` | `WorkloadScenarioConfig` (SKUs + orders), `InfrastructureScenarioConfig` (map + robots + operator), `AlgorithmScenarioConfig` | **three separate scenario documents**, composed by reference |
| `libs/api-schema/fleet-simulator/openapi.yaml:1182-1591` | `LayoutConfigDTO`, `StationLayoutDTO`, `StationEdgeDTO`, `HighwayDTO`, `RackBlockDTO`, `RobotConfig`/`RtpRobotConfig`/`RelayRobotConfig`, `OperatorConfig`, `GridCoordinate`, `Robot`, `Rack`, `Station`, `Item`, `PickTask`, `RackId`, `StationId`, `StorageSpaceId` | the fullest legacy lexicon; note `Item` where Rust says `Sku` |
| `apps/fleet-simulator/src/test/resources/relay-test-config.json` | one run: `systemType`, `simulationDuration`, `scenarioId`, `mapConfig`, `robotConfig` (`numVtms`, `vtmHorizontalSpeed`, `vtmVerticalSpeed`, `vtmCapacity`, `numHtms`, `htmSpeed`), `operatorConfig` (`numBins`, `operatorTimePerPick`, `targetPicksPerFace`), `algorithmConfig` | **`numVtms` is authored here**; desmap derives it from zones |
| `apps/crucible/src/protocol.rs:12-23, 47-88` | WebSocket: `Start { spec }` takes a whole SimSpec; `MapCompiledData { grid_width, grid_depth, cell_size_mm, cells, edges }`, `CellData { rack_levels, zone_id, … }` | the wire format is row-major cells — grid-shaped by construction |
| `crucible-ui/` + `apps/crucible/src/grid.rs:11-20` | browser renderer; requires `grid_bounds` or errors | same grid assumption as desvis |
| `apps/greymatter-platform` | **empty submodule**, `.gitmodules` pins branch `easy-loop-simulation`; referenced in `docs/plans/2026-01-15-a5-shuttle-support.md:189` as "a separate station simulator service" | the EasyLoop station model lives outside this repo |
| `specs/map_abstraction/requirements.md:44-51` | BR-4 "Clear Coordinate System Architecture": "All map operations use explicit GridPosition types / World coordinates only used for rendering and distance calculations" | **this is the repealed premise** — the format was designed grid-first on purpose |
| `specs/relay_system/design.md`, `DECISION_POINTS.md`, `METRICS_HIERARCHY.md` | VTM/HTM/zone/relay-position vocabulary and the zone-capacity model | source of `verticalToteCapacity`, `targetPicksPerFace` |

**Old → new lexicon candidates.** `GridCoordinate(row,col)` / `[col,row]` / `GridPos{x,y}` → one
frame-tagged point. `StorageSpace` / `RackSlot` / `SlotDef` / `RackId` → one storage-location entity.
`RelayPosition` / diving board / buffer position / cantilever shelf → one exchange-point entity.
`presentation cell` / `access_position` / `accessibility_direction` → station-internal geometry.
`RackBlock` / `zone` / `aisle` / `sector` → partition entities with declared roles.
`Item` / `Sku` → one. `cell_size_meters` vs `cell_size_mm` → one unit (mm).

---

## 7. Scenario composition today

**One file, three sections.** `dessim::SimSpec` (`libs/dessim/src/sim_spec.rs:14-22`):

```rust
pub struct SimSpec {
    #[serde(flatten)] pub warehouse: WarehouseDesign,
    #[serde(default)] pub scenario: ScenarioConfig,
    #[serde(default)] pub settings: SimConfig,
}
```

- **Map** — the design's own keys at the JSON root.
- **`scenario`** — `ScenarioConfig` (`libs/dessim/src/scenario.rs:36-43`), an *untagged* enum of
  `Generated` (counts and popularity shares), `Inline` (SKUs and orders embedded), `Csv`
  (items + orders paths), `BulkSource` (items + a demand-pool CSV with pallet-size bounds, orders
  built during the run). Untagged means variant selection is by field presence, and the doc has to
  explain the tie-break ordering (lines 28-35).
- **`settings`** — `SimConfig` (`libs/dessim/src/config/mod.rs:194-312`): fidelity, duration, RNG
  seed, thread count, `num_htms`/`num_rtps`, per-robot-type timing (`vtm`, `htm`, `rtp`), subsystem
  blocks (`relay`, `order`, `pick`, `operator`, `vtm_scoring`, `putaway`), `nav_factor`,
  `queue_factor`, metrics slice width, watchdog and tripwire flags, `motion_model`.

**Fleet is declared twice and resolved by precedence.** `apply_fleet` copies the design's
`fleet { htms, rtps }` over `settings.num_htms`/`num_rtps`, then CLI `--settings` wins
(`sim_spec.rs:31-36`). VTM count has no config path at all: it is `sum(zone.vtms_deployed())`
(`libs/desmap/src/map/mod.rs:100-106`), spawned by the same predicate
(`libs/dessim/src/scenario_gen/vtm_placement.rs:14-24`), with only a debug escape hatch
(`--settings vtm.per_zone=N`, `sim_spec.rs:123-129`).

**Robot physics live in `settings`, not in the map.** HTM chassis kinematics (`htm.loaded_speed`,
`empty_accel`, `loaded_rotate`, …) and VTM cycle times (`vtm.pick_secs`, `drop_secs`,
`alignment_secs`, `isolation`) are all sim settings. The map contributes only `VtmModel::A5`
(`libs/desmap/src/vtm_model.rs:6-12`, a one-variant enum) and per-zone `vtm_count`.

**Everything not recognized is passthrough.** `deseditor` splits the top-level object by
`DESIGN_KEYS` and re-emits the rest byte-for-byte (`libs/deseditor/src/io.rs:20-32`). `scenario`,
`settings`, and `editor_meta` all ride that mechanism.

**The Java lineage was the opposite shape** — three named, separately-stored scenario documents
(`WorkloadScenarioConfig`, `InfrastructureScenarioConfig`, `AlgorithmScenarioConfig`,
`libs/api-schema/fleet-simulator/openapi.yaml:928-1085`) composed by id at run time. The Rust side
collapsed them into one file and lost the ability to say "this map, that workload, those settings" as
a reusable triple. Dallas demonstrates the cost: it points at cencora's CSVs because there is no way
to reference a workload independently of a map.

---

## 8. Top 12 implications for the new format

1. **Give every placed thing an id, including the ones desmap treats as anonymous geometry.** Rack
   placements, individual slots, diving boards, highways, clip boxes, parking areas, and cross-aisles
   all currently exist only as list positions. `StorageSpaceId(rack_index * vtc + z)`
   (`libs/dessim/src/scenario_gen/pipeline.rs:298`) is the reductio: an inventory identifier derived
   from a sort order derived from a cell id derived from a grid coordinate. Ids must be minted at
   declaration and survive edits.

2. **Make derived ids deterministic *functions of declared ids*, never of position.** Today
   `CellId = row * width + col` and `RelayId` is a compile-order counter, so widening the grid or
   inserting a zone renumbers everything and invalidates every recorded metric series. Derive
   `cell(highway_id, tile_index)` and `exchange_point(rack_placement_id, board_index)` instead.

3. **Put the grid in the same box as vendor frames: one derived, secondary coordinate system.**
   `Cell` already carries both `grid_pos` and `world_pos` (`libs/desgraph/src/graph/types.rs:16-19`).
   The new format should let `world_pos` be authored and `grid_pos` be a computed projection with an
   explicit transform, so the pathfinder's Manhattan-on-ordinals heuristic
   (`fh_sipp/pathfinder.rs:220-230`) becomes an *optimization over a declared lattice* rather than the
   definition of space.

4. **Kill the double normalization.** mm slot positions are interpolated onto cell centers in dessim
   (`scenario_gen/topology.rs:224-230`) and desvis (`rack_geometry.rs:8-19`), then re-normalized onto
   `zone_width_m` for kinematics (`model/relay/config.rs:183-201`). Three frames, ~18% distortion,
   and desvis documents the distortion as a feature because the alternative is racks that do not line
   up with robots. If positions are mm end to end, all three collapse.

5. **Anything derivable must be directly declarable — apply it to cells first.** `JunctionCell`
   (`design/types.rs:419-429`) already proves the shape works: an mm-positioned node with explicit
   adjacency. But it needs an author-managed id above `width*depth` (`graph_builder.rs:113-115`), and
   nothing else in the pipeline handles it (nearest-selection errors on it,
   `pathfinding/nearest.rs:92-95`; desvis skips it, `app/grid_data.rs:56`). Generalize the junction
   case into the rule, and make the tiling of a highway or a rack run just a shorthand that emits the
   same declarable entities.

6. **Exchange points are entities, not a side effect of cell adjacency.** Dallas declares 2,072
   diving boards and gets 2,849 relay positions, because `generate_relay_positions` emits one per rack
   cell that touches open floor (`compile.rs:222-254`) and ignores the declared boards entirely. The
   test pins the wrong number and says so (`walmart_dallas.rs:7-12, 26`). A declared exchange point
   needs its own id, its own mm position, its own height, and its own access relationships to the
   robot classes that use it — including several stacked at one location, which the format can
   already *say* (`diving_board_slot`) and the sim ignores (only board 0 is used,
   `scenario_gen/topology.rs:239-243`).

7. **Storage locations need a depth axis and a shape, not a duplicated `(x, y)`.** Dallas expresses
   double-deep as two slots at identical coordinates; the only depth semantics anywhere is
   `SHALLOW_PROPORTION = 0.8` blending two fork times (`robots/vtm/motion.rs:24-34`). Model the
   location's position, its depth index, and its carrier compatibility, and let the motion model read
   the real number.

8. **Separate the four things `CellKind` currently conflates.** One 12-variant enum
   (`libs/desgraph/src/types.rs:88-118`) carries navigability (`Blank`), function (`Storage`,
   `Charging`), ownership (`StationArea`, `QueueCell`, `Presentation`), and robot-class restriction
   (`Aisle`, `Highway`). This is why traversability needs a second, entirely separate derivation pass
   with cell-kind-pair special cases (`graph_builder.rs:273-334`) plus a reachability BFS
   (`design/reachability.rs:14-106`) plus per-edge `dest_only`/`vtm_zone_gate` patches. Components
   over kinds: a cell composes a *surface*, an *owner*, a *function*, and a set of *access rules*.

9. **Make regions first-class and reasoned.** `ClipBox` is an anonymous "make this Blank" rectangle —
   Dallas needs 1, cencora needs 84. Sectors are a declared type that nothing populates
   (`libs/desmap/src/design/graph_builder.rs:97, 121`). The Walmart need is the opposite: sectors as
    behavior selectors. One
   region entity with an id, a shape (not necessarily a rectangle), a role, and a reason covers
   excluded areas, embargoed areas, sectors, and cross-aisles.

10. **Stations need internal structure, and the format has exactly one presentation cell.**
    `StationDefinition` (`design/types.rs:376-398`) offers `position`, one `presentation_cell`, a
    linear `queue_path`, one `exit_direction`, a bin count, and a two-value type. EasyLoop wants
    waiting blocks, queue blocks, *dual* presentation points, and distinct ingress/egress cells; box
    induction and extraction stations want new types entirely. Model a station as a composed thing
    with named ports, each port an entity with a position and an access rule — the queue-restriction
    machinery in `design/queue_edges.rs:65-158` is a hard-coded special case of exactly that.

11. **Fleet, carriers, and behavior belong in the scenario layer with real declarations.** Today the
    fleet is `{htms, rtps}` (`map/types.rs:11-19`) with VTMs derived from zones, one robot model
    exists (`VtmModel::A5`, `vtm_model.rs:6-12`), and carrier type is a render-only dessim enum
    (`model/location.rs:35-41`). Dallas needs robot types with capabilities, carriers (tote/tray/box)
    with an inventory-versus-order designation, and racks dedicated to specific VTMs. Declare robot
    classes and carrier classes as entities; let fleet entries reference them; let zones and racks
    reference both.

12. **Keep SimSpec's single-document convenience, but make the parts separately addressable and
    versioned.** `SimSpec` (`sim_spec.rs:14-22`) got the composition right — one file, map + demand +
    settings — and the passthrough mechanism (`libs/deseditor/src/io.rs:1-8`) proves that extension by
    unknown keys already happens in practice (`editor_meta.underlay` carries the site drawing's mm
    calibration). What it lost is the Java-era ability to name a workload independently of a map
    (`libs/api-schema/fleet-simulator/openapi.yaml:928-1085`), which is why Dallas runs on cencora's
    CSVs. Make each part a referenceable document with its own id and version, and let a scenario be
    the tuple. Note also the untagged `ScenarioConfig` enum (`scenario.rs:36-43`) discriminating by
    field presence — tag it.
