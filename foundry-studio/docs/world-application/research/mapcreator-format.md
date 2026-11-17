# Map Creator export format — forensic digest

> Purpose of this document: describe, with receipts, the JSON export that Map Creator
> produces today and that the rest of GreyOrange consumes — so that whoever designs the
> world protocol never has to open the raw dumps. Every claim below names a file and
> carries either an excerpt or the command that produced it.

**Evidence base.** Four full site dumps and one partial:

| Shorthand | Path | Notes |
|---|---|---|
| `$MC` | `/private/tmp/claude-501/-Users-daniel-vt-greyorange-greyorange-foundry/8f4a86f9-2f62-4359-9f95-cef9f1e71603/scratchpad/mapcreator` | holds `site-a/`, `site-b/`, `site-c/`, `site-d/` (session scratch; copy elsewhere before it is reaped) |
| `site-e` | `/Users/daniel/vt/greyorange/site-e` | partial dump with station sub-structure files the four site dumps lack |

`site-d` is **not** the Walmart Dallas site being designed for; it is here as a
semantics sample of a HAI-vendor relay site.

Cell counts: site-a 5055, site-b 3177, site-c 1867, site-d 1424, `site-e` 1882
(`jq '.[0].map_values|length' map.json`).

---

## 1. Executive summary

The export is **19 sibling JSON files with no manifest, no schema version, and no shared
identity discipline.** `map.json` is the spine: a single-element array whose one object
holds `floor_id`, a `vda5050_reference`, an optional `location_profiles` table, and
`map_values` — a flat list of **cells**. Everything else in the dump attaches to cells,
and attaches by four different kinds of reference: grid coordinate string `"[x,y]"`,
raw grid pair `[x,y]`, **barcode string**, and **display-name string**.

Five findings dominate.

**One.** There are **three whole-site coordinate representations plus three local ones**,
and they are all stored per cell, per file, redundantly. The integer **grid** coordinate
is the identity the rest of the format joins on; `world_coordinate` is millimeters in a
site frame whose x axis is mirrored relative to the grid and whose y axis points opposite
to the VDA5050 frame; `vda_world_coordinate` is the VDA5050 frame, related to the world
frame by `vda_x = world_x + Kx`, `vda_y = Ky − world_y` for site constants Kx, Ky
(§4). Nothing in the file declares Kx or Ky — they must be recovered by regression over
the cells. On one of four sites the relation does not even hold globally.

**Two.** The grid has **escape hatches that are magic numbers**: `x == 999` means "this
point has no real grid position, y is a synthetic sequence number", and the diagonal
`[500,500] … [520,520]` family marks bot re-initialization points. 818 of site-a's 5055
cells use the 999 hatch (§6.3).

**Three.** `site-a/map.json` is **two different cell schemas merged into one array**
— 3857 cells carrying `world_coordinate_reference_neighbour` + `location_profile_id` and
`"null"` (the four-character string) for `botid`, and 1198 cells carrying `floor`,
`special`, `block_reasons`, `barcode_properties`, `composite_attribute_list` and real
JSON `null` for `botid`. The two halves disagree about the world-to-VDA transform (§6.1).

**Four.** Identity is by **mutable display string** almost everywhere it matters. Zones are
keyed by human labels; `site-d`'s largest zone — 925 of 1424 cells — is
`"Zone 102 Exite Gate "`, typo and trailing space included, declared that way in
`zone.json` and referenced that way by every cell (§6.4). Station-to-zone association is an
*implicit string template*: cells in zone `PPS_123` belong to `pps_id: 123` by naming
convention only, and site-a has 20 `PPS_1xx` zones with no matching station (§6.5).

**Five.** The format has **no concept of a rack, an aisle wall, or a station as a thing
with extent**. A storage position is a bare tuple `(io cell, direction, ndeep, height_mm)`
with an opaque sequential id `TLOC_0000001`; nothing declares the rack it belongs to. An
aisle exists only as a list of io-point coordinates in `vtm_zone.json`. A station exists
as a numeric id plus a handful of coordinate lists.

The format is, in effect, **a grid with attributes stapled to it** — exactly the founding
assumption the new format sets out to reverse.

---

## 2. File-by-file inventory

Presence and element counts (`empty` = the file is literally `[]` or `{}`):

```
file                        site-a    site-b  site-c  site-d  site-e
map.json                              1       1         1            1    1
zone.json                            42      15        29            5    1
sectors.json / sector.json            1       1         1            1    1
sectorMxUPreferences.json         empty       2         3        empty    -
vtm_zone.json                         1       1     empty            1    -
block.json                           28      41         -           28    -
pps.json                             16      17        20            7   10
tote_storage.json                 15966   21737     empty        18018    -
tote_type.json                        1       1         2            1    -
charger.json                         46      24        14            7    -
conveyor.json                     empty       3     empty            3    -
conveyor_mapping.json             empty      32     empty           76    -
dock_point.json                   empty   empty        53        empty    -
ods_excluded.json                     1       1         1            1    -
elevator.json                     empty   empty     empty        empty    -
fire_emergency.json               empty   empty     empty        empty    -
queue_data.json                   empty   empty     empty        empty    -
opc_connection.json               empty   empty     empty        empty    -
zone_controller.json              empty   empty     empty        empty    -
pps_bin/pps_bintype/pps_seat/pptl      -       -         -            -  4/4/1/20
```

| File | What it describes | Top-level shape | Keying | References out |
|---|---|---|---|---|
| `map.json` | the whole floor: cells + bot-capability profiles + VDA anchor | `[{floor_id, vda5050_reference, location_profiles?, map_values[]}]` | cell → `coordinate` `"[x,y]"`, also unique `barcode` | `zone` (name string), `sector` (id or `"undefined"`), `block` (int), `location_profile_id` (name string) |
| `zone.json` | zone existence + runtime pause/block flags | `[{zonerec:{zone_id, blocked, paused}}]` | `zone_id` display string | none |
| `sectors.json` | sector → member cells | `[{"<sector id>": ["[x,y]", …]}]`, plus a `"undefined"` bucket | sector id string | cell grid coords |
| `sectorMxUPreferences.json` | which sectors a carrier type prefers, in order | `{"<rack type or tote class>": [sector ids]}` | rack type / tote class name | sector ids, `tote_type.racktype_id` |
| `vtm_zone.json` | aisles: an ordered list of io-point cells | `{aisle_info:[{aisle_id, display_name, io_point_coordinates[], additional_information:{type}}]}` | `aisle_id` string; `display_name` `"V1"` | cell grid coords |
| `block.json` | named cell groups used as queues / waiting areas / conveyor points | `[{entity_type, entity_details:{block_id, coordinates[[x,y]…]}}]` | `block_id` **string** | cell grid coords (raw pairs, not strings) |
| `pps.json` | pick/put stations | `[{…}]`, **two incompatible versions** (§8) | `pps_id` (int in v3, string in v2) | cells by grid pair or barcode; `block_id` strings; block/presentation-point refs as `"block-49"` strings |
| `tote_storage.json` | every storage position in the building | `[{location, io_point:{coordinate, bot_orientation}, storage_direction, ndeep, height, relay_point?, storage_sector?}]` | `location` = `TLOC_%07d`, sequential **with gaps** | cell grid coords |
| `tote_type.json` | rack/carrier geometry template | `[{racktype_id, rack_width/depth, floor_structure[…]}]` | `racktype_id` | none; referenced *by* `sectorMxUPreferences`, `pps.docking_stations.rollcage_type_id` |
| `charger.json` | chargers + their approach and re-init points | `[{charger_id, charger_location, entry_point_*, reinit_point_*, …}]` | `charger_id` int | cells **by barcode**, directions 0–3 |
| `conveyor.json` | conveyor topology as a step chain | `{version:"2.0.0", conveyor_data[], hai_port_types[]}` | `conveyor_id` int, `location_id` string per step | cells by grid coord + world mm; io points by grid coord |
| `conveyor_mapping.json` | three-frame join table | `[{go_coordinates, HAI_coordinates, conveyor_coordinates:{conveyor_id, step_id}, location_type, pps_id?}]` | none — it *is* the join | cells, conveyor steps, `pps_id` |
| `dock_point.json` | fixed docking points for carriers at stations | `[{id, dock_barcode, dock_coordinate, direction, station_id, station_type, mtu_id, …}]` | `id` == `dock_barcode` | cells, `station_id` ↔ `pps.docking_stations.dock_station_id` (53/53 overlap) |
| `ods_excluded.json` | (cell, direction) pairs where obstacle detection is suppressed | `{ods_excluded_list:[{ods_tuple:"<barcode>--<dir>", excluded:true}]}` | composite string key | cells by barcode; every resolvable entry is charger-related |
| `elevator.json`, `fire_emergency.json`, `queue_data.json`, `opc_connection.json`, `zone_controller.json` | multi-floor transfer, emergency behaviour, runtime queue state, OPC-UA plant link, zone-controller devices — **`[]` in all four sites**; names are the only evidence of intent | `[]` | — | — |
| `site-e/pps_bin.json`, `pps_bintype.json` | put-wall bins and bin geometry — **stored as a captured HTTP request** | `{url, type:"POST", header, data[]}` | `bin_id` string | `pps_id`, `*_seat_name` |
| `site-e/pps_seat.json` | operator seats at a station | `{pps_seats:[{pps_id, seat_name, seat_type}]}` | `seat_name` | `pps_id` |
| `site-e/pptl.json` | put-to-light devices bound to bins | `[{pptls:[{pps_id, barcode, pps_bin_id, peripheral_id, peripheral_type}]}]` | `peripheral_id` | `pps_bin_id`, `pps_id` |

---

## 3. The cell model decoded

A cell in `map_values`. Fields are **omitted, not nulled**, when absent — so the array is
already sparse-component-shaped, without saying so.

```jsonc
// $MC/site-a/map.json .map_values[…] — the "dialect A" shape
{ "adjacency": [null, null, [49,1], [9,1]],
  "blocked": false,
  "botid": "null",                                   // the STRING "null"
  "neighbours": [[0,0,0],[0,0,0],[1,1,1],[1,1,1]],
  "sector": "undefined",                             // the STRING "undefined"
  "size_info": [750, 475, 405, 410],
  "store_status": 0,
  "zone": "Gate_9",
  "world_coordinate_reference_neighbour": "0,0",     // "x,y" — a THIRD coord encoding
  "vda_world_coordinate": "[231205,124853]",
  "location_profile_id": "HA",
  "barcode": "201.212",
  "world_coordinate": "[84145,-16267]",
  "coordinate": "[7,1]" }
```

| Field | Meaning | Confidence |
|---|---|---|
| `coordinate` | `"[x,y]"` integer grid index. The de-facto primary key: unique in all five dumps (`jq -r '.[0].map_values[].coordinate' map.json \| sort \| uniq -d` → empty). Not dense: site-a has 208 distinct x and 980 distinct y over 5055 cells. | high |
| `barcode` | the floor sticker the robot reads. Also unique per site, and the join key used by `charger.json` and `ods_excluded.json`. Three unrelated encodings across sites (§4.4). | high |
| `world_coordinate` | `"[x,y]"` millimeters, site frame, integer, signed (site-a x ∈ [−57000, 90040]) | high |
| `vda_world_coordinate` | `"[x,y]"` millimeters, VDA5050 frame, integer, all-positive by construction (min is exactly 100000 on every site) | high |
| `world_coordinate_reference_neighbour` | `"x,y"` — the grid coordinate of the cell this cell's `world_coordinate` was measured **from**; a parent pointer in a spanning tree. `"0,0"` marks a root. Present on 3857/5055 site-a, 3177/3177 site-b, 7/1867 site-c. | high |
| `adjacency` | 4 slots; each is `[x,y]` of the connected neighbour cell, or `null`. Slots are **directions**, index order `0 = −y, 1 = −x, 2 = +y, 3 = +x` in grid space (§4.5). Edges may skip cells: site-b slot1 deltas are −3, −2, −1 in x. | high |
| `neighbours` | 4 slots of `[a,b,c]`. Only four values ever occur and they form a monotone staircase: `(1,1,1)` ×34900, `(1,0,0)` ×10540, `(0,0,0)` ×6100, `(1,1,0)` ×2080 across all five files. `(0,0,0)` ⇔ `adjacency[i] == null` exactly, so bit 1 = "an edge exists". Bits 2–3 grade it: `(1,0,0)` concentrates on edges into under-static-rack cells (site-b: neighbour profile `SSouth` 1011, `SNorth` 905) and on long edges (site-b modal length 530 mm vs 740 mm for `(1,1,1)`). Best reading is a graded traversal permission — *reachable / permitted unloaded / permitted loaded*. The same edge carries different triples from each end (site-b: 1454 edges `(1,1,1)` one way, `(1,0,0)` the other), so it states something per cell per direction, not per edge. | high on bit 1; **low on bits 2–3 — do not port blind** |
| `size_info` | 4 slots, mm: the cell's own half-extent toward each direction. `size_info[i] + neighbour.size_info[(i+2)%4] == ` centre-to-centre distance — 7240/7240 in `site-e`, 16296/17500 in site-a (the misses are exactly the skip-edges). | high |
| `blocked` | runtime blocked flag. `true` on 20 site-c cells, `false` everywhere else. | high |
| `block_reasons` | why it is blocked: `["manual"]` on 42 site-c cells, of which only 20 also have `blocked: true` — the two disagree. | high |
| `store_status` | 0 or 1. `1` never coincides with a `tote_storage` io cell on any relay site (site-a: `(0,True)` 1175, `(1,False)` 306, `(1,True)` **0**). On RTP sites it marks the 779/1882 cells where a rack may stand. Reads as "this cell is a rack-parking square", i.e. RTP-era storage. | medium |
| `hasStorables` | present only as `0`, on 390 site-a / 2 site-b / 162 site-d cells. Vestigial. | low |
| `zone` | display-name string into `zone.json`. | high |
| `sector` | integer sector id **or the string `"undefined"`** — 4015/5055 site-a cells, 3177/3177 site-b cells. | high |
| `block` | integer id into `block.json`'s **string** `block_id`. 124/5055 site-a cells; all 124 resolve, and all 124 `block.json` coordinates round-trip. | high |
| `location_profile_id` | name string into `map.json.location_profiles[]`. Absent entirely on the two RTP sites. | high |
| `floor` | per-cell floor number — present on only 1198/5055 site-a cells and 1860/1867 site-c cells, always `1`, while `floor_id` is already stated once at the top of the file. | high |
| `special` | `true` on exactly the `[500,500]…[520,520]` diagonal cells (site-a 46, site-b 24, site-c 14) — the bot re-initialization points referenced by `charger.reinit_point_location`. | high |
| `grid_attribute` | one enum string: `relay_store`, `conveyor_track`, `conveyor_entry`, `conveyor_exit`, `conveyor_pps_point`, `conveyor_blackhole`, `horizontal_aisle`. Only on relay sites. | high |
| `composite_attribute_list` | a *second*, list-shaped attribute channel, site-a only: `[]` ×1156, `["storable"]` ×16, `["vertical_aisle"]` ×16. | high |
| `conveyor_direction_attribute` | direction 0–3 of conveyor travel, on 4 site-b / 8 site-d cells. | high |
| `dockPoint` | `true` on 53 site-c cells — exactly the 53 rows of `dock_point.json` (symmetric difference 0). | high |
| `barcode_properties` | always `{}` wherever present. | high |
| `botid` | which robot occupies the cell right now — **runtime state in a map file**. | high |

`location_profiles` (relay sites only) is the one genuinely well-shaped table in the
format: a named profile → per-bot-type → per-orientation record of what the robot may do
there.

```jsonc
// $MC/site-d/map.json .[0].location_profiles[]
{ "location_profile_id": "S+HCA(HTM_crossaisle_under_static_racks)",
  "bot_orientation_details": {
    "HTM_HAI_K50_L_E1": { "0": { "lift_height_ranges": { "loaded": [[440,600]], "unloaded": [[440,500]] },
                                 "ground_height_clearance": 960 }, … },
    "HTM_HAI_K50_X_E1": {},          // empty object = this bot type is forbidden here
    "VTM_HAI_A42_TDV_E1": {} },
  "edge_height_clearance": {} }      // always {} in all four dumps
```

`ground_height_clearance` is either an integer mm or the **string** `"all"`.
`lift_height_ranges.unloaded` is variously `[]`, `null`, or a list of `[lo,hi]` pairs —
three encodings of "no constraint / unconstrained / constrained". The profile id itself
is prose: `"VA(N-S)+HCA(intersection_of_VTM_aisle_and_HTM_crossaisle)"`.

---

## 4. Coordinate systems decoded

### 4.1 The six representations present

| # | Name | Units | Type | Where |
|---|---|---|---|---|
| 1 | **grid** `coordinate` | cell index | int pair, encoded `"[x,y]"`, `[x,y]`, or `"x,y"` | `map.json`, `sectors.json`, `vtm_zone.json`, `block.json`, `tote_storage.json`, `conveyor.json`, `dock_point.json`, `pps.json` v3 |
| 2 | **world** `world_coordinate` | mm | signed int, `"[x,y]"` | `map.json`, `conveyor.json` steps |
| 3 | **VDA5050** `vda_world_coordinate` | mm | positive int, `"[x,y]"` | `map.json` cells + `vda5050_reference` |
| 4 | **barcode** | — | string; on `site-d` it *is* the VDA coordinate (§4.4) | `map.json`, `charger.json`, `ods_excluded.json`, `dock_point.json`, `pps.json` v2 |
| 5 | **vendor / HAI** `HAI_coordinates` | unknown — `"UNDEFINED"` in all dumps | string | `conveyor_mapping.json` only |
| 6 | **conveyor-local** `{conveyor_id, step_id}` | — | pair of strings | `conveyor_mapping.json`, `conveyor.json` |

`site-e` adds a **seventh, station-local** frame: `pps_bin.coordinate` `[60.0, 40.0]` in
centimetres relative to the put wall, alongside `position [row, col]` — and the metric is
purely derived from the discrete one (`coordinate = [col × breadth, row × length]`, with
`breadth 60 / length 40` from `pps_bintype.json`; verified on `[1,1]→[60,40]`,
`[1,2]→[120,40]`, `[2,1]→[60,80]`).

### 4.2 world ↔ VDA: worked examples

The relation is a translation in x and a **reflection** in y:

```
vda_x = world_x + Kx        vda_y = Ky − world_y
```

| Site | Kx | Ky | Holds for |
|---|---|---|---|
| site-b | 130760 | 157880 | 3177/3177 |
| site-c | 192800 | 127390 | 1867/1867 |
| `site-e` | 190750 | 145540 | 1882/1882 |
| site-d | 126960 | 143540 | 1329/1424 — 95 cells are off by 4–5 mm |
| site-a | — | — | **125 distinct (Kx,Ky) pairs** |

```bash
# recover Kx (and prove it constant) for a site
jq -r '.[0].map_values[] | "\(.world_coordinate) \(.vda_world_coordinate)"' site-b/map.json \
 | tr -d '[]' | awk -F'[ ,]' '{print $3-$1, $4+$2}' | sort | uniq -c
#   3177 130760 157880
```

Worked, cell by cell:

- `site-e`, barcode `002.005`, `coordinate "[5,2]"`, `world "[-6600,0]"`,
  `vda "[184150,145540]"` → `−6600 + 190750 = 184150` ✓, `145540 − 0 = 145540` ✓.
  Four columns from `002.001` at `world [0,0]`, at 1650 mm pitch: `4 × 1650 = 6600` ✓,
  matching `size_info [690,825,690,825]` (825 + 825 = 1650).
- site-c, barcode `015.112`, `coordinate "[112,15]"`, `world "[-73080,15790]"`,
  `vda "[119720,111600]"` → `−73080 + 192800 = 119720` ✓, `127390 − 15790 = 111600` ✓.
- site-d, barcode `1123374354`, `coordinate "[29,1]"`, `world "[-3590,0]"`,
  `vda "[123370,143540]"` → `−3590 + 126960 = 123370` ✓.

**The constants appear nowhere in the file.** `vda5050_reference` looks like it should
carry them, and on site-b and `site-e` it accidentally does — but only because the reference
cell happens to sit at `world [0,0]`. On site-c the reference barcode `015.112` sits at
`world [-73080,15790]`, so `vda5050_reference.vda5050_coordinate` `[119720,111600]` is
neither Kx/Ky nor an origin. It is one sample point, and recovering the transform from one
sample point requires already knowing the reflection.

### 4.3 grid ↔ world

The grid is **not** an affine image of the world: pitch varies cell to cell (site-b has 582
distinct `size_info` tuples; site-a 842). Position is reconstructed by walking the
adjacency graph and accumulating `size_info` half-extents, which is what
`world_coordinate_reference_neighbour` records — the parent edge each cell's mm position
was accumulated along. That accumulation is where site-a's 125 different (Kx,Ky) pairs come
from: neighbouring clusters drift 5–45 mm apart along different accumulation paths.

Axis relations, established from the adjacency slots (§4.5): **grid +y = world +y**, but
**grid +x = world −x**. The grid canvas is mirrored east–west relative to the world frame.

### 4.4 Barcodes: three unrelated encodings

```bash
jq -r '.[0].map_values[].barcode' map.json | sed 's/[0-9]/#/g' | sort | uniq -c
```

- **Dotted `RRR.CCC`** (site-a, site-c, `site-e`) — nominally `row.col` = `(grid y, grid x)`.
  Holds 1882/1882 on `site-e`, 1801/1867 on site-c (counterexamples: `014.052` at
  `[82,24]`, `026.056` at `[74,9]`), and only **381/5055 on site-a** — where the barcode
  has drifted entirely free of the grid index.
- **Plain integer** (site-b): `3155, 3154, 3153…` decreasing along +x. No geometry.
- **Ten-digit positional** (site-d): `barcode = "1" + (vda_x/10) + last4(vda_y/10)`.
  Verified for **1424/1424** cells. `1123374354` ⇒ vda `[123370, 143540]`. The barcode is a
  redundant encoding of the VDA position, at 1 cm resolution, with the y high digit dropped.

### 4.5 Direction enum — one shared 0–3 vocabulary

`adjacency` / `neighbours` / `size_info` slot index, `charger_direction`,
`entry_point_direction`, `storage_direction`, `bot_orientation`, `pick_direction`,
`conveyor_direction_attribute`, `dock_point.direction`, and the suffix of an `ods_tuple`
are all the same enum.

Geometric anchor (`site-e`, whose grid is clean):

```
slot 0 → grid (0,−1), world (0,−1)      slot 2 → grid (0,+1), world (0,+1)
slot 1 → grid (−1,0), world (+1,0)      slot 3 → grid (+1,0), world (−1,0)
```

Cross-checked against `charger.json`, which states a direction and an entry cell
independently: site-a `charger_direction: 3` with entry at barcode col +3 (24 chargers),
`0` with entry at row −1 (6 chargers), `1` with entry at col −2. The direction integer and
the adjacency slot index agree.

Compass reading: site-b's profile `VNS+HN` ("VTM north-south + HTM north") permits HTM
orientation `0` only, and `VRotate+HS` permits HTM `2` only, so **0 = north, 2 = south**;
`V_east_west` permits `1` and `3`, so those are the east–west pair. Combined with the slot
geometry, grid +y is south and grid +x is one of east/west — **which one is not decidable
from the dumps alone**, since no file names an absolute bearing. Flag it as a question for
whoever owns Map Creator; the mirroring in §4.3 makes the naive reading wrong.

`storage_direction` is always perpendicular to `io_point.bot_orientation`
(`(dir − orient) mod 4 ∈ {1,3}` for 15966/15966 site-a rows), and points into a direction
with no traversable neighbour most of the time — 62.6% on site-a, 83.4% on site-b, 81.6% on
site-d — i.e. into the rack.

---

## 5. Entity/relationship model the format is groping toward

```
Floor(floor_id)                        ── only ever 1; elevator.json empty everywhere
 └─ Frame (grid | world_mm | vda_mm | vendor)  ── implicit, never declared
 └─ Cell  [PK grid coordinate; alt key barcode]
      ├─ pose in 3 frames (all stored, none authoritative); footprint size_info[4]
      ├─ edges adjacency[4] + neighbours[4] (permission grade)
      ├─ → Zone (display name) │ Sector (id or "undefined") │ Block (int→string id)
      │  │ LocationProfile (name)
      ├─ runtime: botid, blocked, block_reasons, store_status
      └─ classification: grid_attribute | composite_attribute_list | special | dockPoint
 Zone(zone_id: display string) — blocked, paused. No geometry; membership lives on cells.
 Sector(id) — no attributes at all; sectors.json is the inverted cell index.
 Block(block_id: string, entity_type ∈ {queue, waiting, sequenced_queue, conveyor-point})
      └─ member cells (raw [x,y] pairs)
 Aisle(aisle_id, display_name "V1", type "VTM") [vtm_zone.json]
      └─ ordered io-point cells. No extent, no rack, no direction.
 StoragePosition(TLOC_%07d) [tote_storage.json]
      ├─ io cell + bot_orientation + storage_direction + ndeep + height_mm
      ├─ relay_point → cell (relay sites); storage_sector → sector (site-b only)
      └─ ✗ no rack entity, no bay, no level entity, no link to tote_type
 CarrierType(racktype_id "MSU_1"/"MTU_1"/"Type_1") [tote_type.json]
      └─ floor_structure[] (shelf levels A..F) → rack_face_structure[] (face 0/1)
           → slot_structure[] (slot_tags, height, depth, x, y, length, barcodes[])
 Station / PPS(pps_id)
      ├─ v3: location(grid pair), type, system[], allowed_modes[], orchestrator,
      │      entities.presentation_points[], entities.blocks[], associations[]
      └─ v2: location(barcode), pps_points[], queue_barcodes[], pick_position,
             pick_direction, put_docking_positions[], docking_stations[], eligible_system[]
 PresentationPoint(point_id, station-local) → barcode, "world_coordinates" (actually GRID),
      pick_direction, presentation_height, bot_type, bot_orientation?, is_workbench,
      light_curtain, presented_entity_allowed_types[], presented_for, presenting_agent_type
 DockPoint(id == dock_barcode) → station_id, mtu_id, rack_id, direction, station_type
 DockStation(dock_station_id) ← pps.docking_stations; 53/53 ids match dock_point.station_id
 Charger(charger_id) → charger_location(barcode), entry_point, reinit_point, charger_type
 Conveyor(conveyor_id) → ordered conveyor_steps[] (location_id, grid coord, world mm)
      ├─ entry_points[]/exit_points[]: io cell + height + direction + bot_orientation
      └─ conveyor_mapping.json: {go, HAI, conveyor} triple-frame join, optional pps_id
 ODSExclusion("<barcode>--<direction>") → cell + direction; every one is charger-related
 Seat / Bin / BinType / PPTL [site-e only] — station-internal put-wall structure
```

Relationships the format **cannot** express, evidenced by their absence:
a rack as an object with extent; a bay/level/slot address for a storage position; an aisle's
geometry; a station's footprint; more than one floor; any relation between
`tote_type.floor_structure` slots and `tote_storage` heights; a robot fleet.

---

## 6. Pathology catalogue

### 6.1 Two cell schemas merged in one array (site-a)

```bash
jq '[.[0].map_values[] | has("world_coordinate_reference_neighbour")] | group_by(.) | map({(.[0]|tostring):length}) | add' \
   $MC/site-a/map.json
# {"false":1198,"true":3857}
```

The 3857 carry `location_profile_id` and `botid: "null"`; the 1198 carry
`floor`, `special`, `block_reasons`, `barcode_properties`, `composite_attribute_list` and
`botid: null`. The 1198 share a single, self-consistent transform (Kx 157000, Ky 122500)
that **differs from every one of the other 124**. 799 of the 1198 use the `x == 999`
escape hatch. Two different producers wrote into one file and nothing noticed.

### 6.2 Millimeter drift inside one frame

```jsonc
// $MC/site-d/map.json — Kx = 126965 here, 126960 for 1329 other cells
{"barcode":"1108330866","coordinate":"[75,91]","world_coordinate":"[-18635,34880]","vda_world_coordinate":"[108330,108660]"}
// and Ky = 143545 instead of 143540
{"barcode":"1113740454","coordinate":"[500,500]","world_coordinate":"[-13220,39005]","vda_world_coordinate":"[113740,104540]"}
```

Same building, same frame pair, two answers 5 mm apart. On site-a the spread is 45 mm
(Ky ∈ {108586 … 108631} within the 3857-cell half alone).

### 6.3 Magic values

| Value | Where | Meaning |
|---|---|---|
| `"null"` (string) | `botid` — 3857 site-a, 3177 site-b, 1424 site-d cells | no robot. JSON `null` means the same thing on the other 1198 site-a cells. |
| `"undefined"` (string) | cell `sector` (4015/5055 site-a); a top-level **key** in `sectors.json`; `dock_point.rack_id`, `rack_status`, `reserved_by`; `charger.error_reason`; `conveyor_mapping.HAI_coordinates` = `"UNDEFINED"` | absent |
| `x == 999` | 818 site-a / 97 site-d / 34 site-b / 10 site-c cells; every `conveyor_steps` coordinate | "no grid position — y is a synthetic sequence" |
| coordinates `≥ [500,500]`, almost all on the diagonal `[n,n]` | 46 site-a (`[500,500] … [554,554]` plus `[501,500] … [505,500]`), 24 site-b, 14 site-c, 7 site-d, 8 `site-e` | bot re-initialization points, referenced by `charger.reinit_point_location`; these are **exactly** the `special: true` cells on every site |
| `"all"` (string) | `ground_height_clearance`, alternating with integer mm | unbounded |
| barcode `999.999`, `997.077` | site-c cell, site-a ODS entries | off-grid markers |

### 6.4 Display strings as join keys

```bash
jq -r '.[].zonerec.zone_id' $MC/site-d/zone.json | sed 's/$/<EOL>/'
# defzone<EOL>
# Zone 1 put PPS<EOL>
# Zone 101 Exit Gate<EOL>
# Zone 102 Exite Gate <EOL>      <-- typo AND trailing space, in the join key
# Zone 5 Easyloop PPS <EOL>      <-- trailing space
```

Both whitespace-bearing ids are live, and one of them is the site's largest zone.
`jq -r '.[0].map_values[].zone' site-d/map.json | sort | uniq -c` gives
`Zone 102 Exite Gate ` **925 cells**, `Zone 101 Exit Gate` 433, `Zone 5 Easyloop PPS ` 36,
`Zone 1 put PPS` 20, `defzone` 10. Renaming a zone in the editor rewrites 925 cells; two
zones that differ only in a trailing space are two zones; and a diff tool shows nothing.

Same disease in `sectorMxUPreferences.json`: site-c prefers sectors for `"MSU_4"`, a
`racktype_id` that does not exist in its `tote_type.json` (which declares only `MSU_1` and
`MTU_1`).

### 6.5 Implicit joins by string template

Cells in zone `PPS_123` belong to the station with `pps_id: 123` — by naming convention,
enforced nowhere.

```
site-a: 32 zones named PPS_*, 16 stations.
  20 PPS_* zones have no station:  PPS_101 … PPS_120
  4 stations have no zone:         137, 138, 139, 140
site-b: 11 PPS_* zones, 17 stations; 6 stations (101, 12–16) have no zone.
```

Twenty decommissioned stations left their zone names behind on live cells.

### 6.6 Dangling references

```bash
python3 - <<'P'
import json;R="$MC/site-a"
bc={c["barcode"] for c in json.load(open(R+"/map.json"))[0]["map_values"]}
o=[t["ods_tuple"] for t in json.load(open(R+"/ods_excluded.json"))["ods_excluded_list"]
   if t["ods_tuple"].rsplit("--",1)[0] not in bc]
print(len(o), o[:6])
P
# 24 ['999.075--0', '003.075--0', '997.077--0', '999.075--2', '003.075--2', '997.077--2']
```

24 of site-a's 180 obstacle-detection exclusions name cells that do not exist.

### 6.7 Denormalization: the same fact stored two to four times

`sectors.json` is the inverse of the cell `sector` field, `"undefined"` bucket and all.
`block.json`'s coordinate lists mirror the cell `block` field (they happen to agree —
0/124 site-a, 0/126 site-b, 0/74 site-d mismatches — but nothing enforces it). Every cell
stores its position three times, two of them computable from the third. `pps.json` states
`blocks` twice per station (top level and `entities.blocks`). site-c cells carry
`dockPoint: true` duplicating `dock_point.json` row-for-row. `dock_point.id` is
byte-identical to `dock_point.dock_barcode` in 53/53 rows. `site-e/pps_bin.coordinate` is
`position` times the bin dimensions. `floor: 1` is repeated on 1198 site-a cells although
`floor_id: 1` is stated once at the top of the file.

### 6.8 Runtime state inside the map

`botid`, `blocked`, `charge: 13.9534`, `attached_butler_id: "13958"`, `power_status`,
`status: "connected"`, `mode: "auto"`, `blocked_by_butler_ids`, `zonerec.paused`,
`dock_point.status: "mtu_docked"`, `pps.status: "disconnected"`,
`pps_url: "http://localhost:8181/pps/123/api/"`. The map cannot be diffed across time
because it changes when the building is merely *running*.

### 6.9 Fossilized API calls checked in as data

```jsonc
// site-e/pps_bintype.json — the whole file
{"url":"/api/ppsbintypes","type":"POST","header":{"accept":"application/json","content-type":"application/json"},
 "data":[{"tag":[],"type_id":"CUSTOM","dimension":{"height":60.0,"length":40.0,"breadth":60.0}}]}
```

Two of `site-e`'s eight files are HTTP request envelopes rather than models.

### 6.10 Type instability, and one name that misleads

`pps_id` is `int` in v3 and `string` in v2. `pps.version` is `"3"` (string) in v3 and `2`
(int) in v2. `block_id` is a string in `block.json` and an int in `map.json`. Directions are
ints in most files and the string tail of `"<barcode>--1"` in `ods_excluded.json`.
`lift_height_ranges.unloaded` is `[]`, `null`, or a list. Coordinates are `"[x,y]"`,
`[x,y]`, and `"x,y"` in three different files. And
`pps.entities.presentation_points[].attributes.world_coordinates` holds `[29,5]` — a
**grid** coordinate under a field named "world", which will convert millimeters into cell
indices for anyone porting by field name.

---

## 7. Cross-site schema drift

`map.json` has **no version field on any site** (`jq '.[0]|keys'` → at most
`["floor_id","location_profiles","map_values","vda5050_reference"]`). What versioning exists
is per-file and inconsistent: `pps.version` `"3"` vs `2`; `conveyor.version` `"2.0.0"`.

Cell-field presence — the **key** being present, not its value being true (counts over that
site's cells):

```
field                                 site-a       site-b   site-c  site-d        site-e
adjacency/barcode/blocked/botid/          all         all        all          all          all
 coordinate/neighbours/sector/
 size_info/store_status/
 vda_world_coordinate/world_coordinate/zone
world_coordinate_reference_neighbour   3857/5055  3177/3177     7/1867    1424/1424  1882/1882
location_profile_id                    3821/5055  3140/3177          —    1296/1424          —
grid_attribute                          368/5055   350/3177          —     148/1424          —
hasStorables                            390/5055     2/3177          —     162/1424          —
block                                   124/5055   126/3177          —      74/1424          —
special                                1229/5055    24/3177  1860/1867       7/1424     8/1882
floor / block_reasons / barcode_properties 1198/5055        —  1860/1867          —          —
composite_attribute_list               1198/5055          —          —            —          —
conveyor_direction_attribute                   —    4/3177          —       8/1424          —
dockPoint                                      —          —    53/1867            —          —
```

Two site *generations* are visible, and neither is marked as such. **RTP** (`site-c`,
`site-e`): no `location_profiles`, no `tote_storage`, no `vtm_zone`, `pps.version 2` with
`queue_barcodes`/`docking_stations`, `eligible_system: ["rtp"]`, `store_status` used for
rack squares, `dock_point.json` populated. **Relay** (`site-a`, `site-b`,
`site-d`): `location_profiles`, `tote_storage`, `vtm_zone`, `block.json`,
`pps.version "3"` with `presentation_points`/`associations`, `system: ["relay"]`,
`orchestrator: "butler"`.

Inside the relay generation there are two **vendor** dialects, differing in bot-type and
charger-type vocabulary and sharing no explicit vendor field:

```
site-a  HTM_QT_M5F, VTM_QT_C56_S_2D      chargers: quicktron_htm_charger / quicktron_vtm_charger / side_dock
site-b         HTM_QT_M5F, VTM_QT_C56_GT_2D     chargers: quicktron_*
site-d  HTM_HAI_K50_L_E1, HTM_HAI_K50_X_E1, VTM_HAI_A42_TDV_E1   chargers: hai_htm_charger / hai_vtm_charger
```

`charger.json` itself drifts: site-a and site-c carry five extra runtime fields
(`charge`, `attached_butler_id`, `power_status`, `error_reason`, `blocked_by_butler_ids`)
that site-b and site-d omit — same schema name, different fields, same release.

**What this implies about governance:** there is none. Fields appear because one site
needed them, spread by copy, and never get retired. A consumer cannot tell whether a
missing field means "not applicable here", "older exporter", or "different product line";
it must sniff. Any new format must make the generation and the vendor **declared facts**,
not something inferred from which keys happen to be present.

---

## 8. Station (PPS) model

**v3** (relay sites) is the more evolved shape and is nearly component-structured already:

```jsonc
// $MC/site-d/pps.json[0], abridged to one of three presentation points
{ "version": "3", "pps_id": 1, "orchestrator": "butler",
  "location": [29,5],                                       // grid pair
  "type": "manual", "system": ["relay"], "allowed_modes": ["put","audit"],
  "entities": {
    "presentation_points": [
      { "point_id": "1", "light_curtain": true, "presented_for": "inventory",
        "presented_entity_allowed_types": ["tote"], "presenting_agent_type": "bot",
        "attributes": { "world_coordinates": [29,5],        // GRID, despite the name
                        "display_attributes": {"display_id":"1","direction":1},
                        "barcode": "1123374090", "pick_direction": 1,
                        "presentation_height": 940, "bot_type": "HTM_HAI_K50_L_E1",
                        "bot_orientation": 3, "is_workbench": true } } ],
    "blocks": ["3","5","4","6","7"] },
  "associations": [
    { "id": 1, "type": "allowed_transport",
      "entities": ["block-3","presentation_point-1","block-4"],
      "connections": [ {"from":"block-3","to":"presentation_point-1"},
                       {"from":"presentation_point-1","to":"block-4"} ] } ],
  "status": "disconnected", "pps_url": "http://localhost:8181/pps/1/api/",
  "blocks": ["3","4","5","6","7"] }                          // stated twice
```

`associations` is the one place the format reifies a relationship — a typed, connected
graph over locally-scoped handles (`"block-3"`, `"presentation_point-1"`). Those handles
are **station-local strings that stringly encode a type prefix plus an id**, and
`"block-3"` refers into the *global* `block.json` namespace while `"presentation_point-1"`
refers into the *station-local* one. Same syntax, two scopes.

Station types seen: `manual`, `easyloop`, `manual_tote_induction`, `auto_tote_induction`,
`tote_extraction`, `order_carrier_induction`, `order_carrier_extraction`, `extraction`.
Modes: `put`, `pick`, `audit`. `bot_orientation` is present on site-d presentation
points and absent on site-a/site-b ones.

All 50 presentation-point barcodes across the three v3 sites resolve to real cells, and all
70 station `location`s across all five dumps resolve — this part of the format is
internally sound.

**v2** (RTP sites) is a different model of the same thing: `location` is a barcode,
`pps_points[]` gives `{position_id, coordinate, type:"rtp"}`, and the queue is
`queue_barcodes[]` — a bare list of 6–8 cell barcodes with no ordering semantics, no entry
or exit designation, and no relation to `block.json` (RTP sites ship no `block.json`).
`docking_stations[]` carries `{dock_station_id, rollcage_type_id, group_id, tags,
docking_type, dock_station_type}` and joins to `dock_point.json` by `station_id` (53/53).

`site-e` reveals the station **interior**, which the four site dumps drop entirely: seats
(`front_9` / `back_9`, `seat_type ∈ {front, back}`), a put wall of bins
(`bin_id`, `group_id`, `position [row,col]`, `direction: "left"`, `allowed_tags`,
`front_seat_name`/`back_seat_name`), bin types with dimensions, and put-to-light devices
bound one-to-one to bins. So a station is really: seats × bin wall × light peripherals ×
presentation points × queue blocks × docking positions — and the production export format
can represent perhaps half of that.

---

## 9. Storage model

A storage position is a flat tuple. There is no rack.

```jsonc
// $MC/site-b/tote_storage.json[0]
{ "location": "TLOC_0000001",
  "io_point": { "coordinate": "[56,13]", "bot_orientation": 1 },
  "storage_direction": 0, "ndeep": 0, "height": 390,
  "relay_point": "[58,10]", "storage_sector": 1 }
```

- `location` — opaque `TLOC_%07d`, **sequential with gaps**: site-b has 21737 rows with a
  maximum of `TLOC_0069739`, site-a 15966 rows up to `TLOC_0016066`. An allocation counter
  encodes nothing, is not reproducible from geometry, and is not stable under a rebuild.
- `(io_point.coordinate, bot_orientation, storage_direction, ndeep, height)` is unique
  across every row of every site (0 duplicates in 55721 rows) — the natural key exists and
  is not used.
- `ndeep` — depth index into a multi-deep rack; `0` and `1` only. `relay_point` — the cell
  where a VTM hands a tote to an HTM for this position. `storage_sector` — site-b only;
  elsewhere the sector must be inferred from the io cell.
- `height` — mm above floor. Site-specific ladders: site-a 12 levels at 400 mm pitch
  (390, 840, 1285, 1685, … 5185); site-d 27 levels at ~406 mm up to 11744 mm. Owen has
  24 distinct heights including both `3685` and `3686` — a 1 mm difference the format has
  no way to reject.
- Levels per io cell: site-a {10: 807 cells, 2: 367, 12: 1}; site-d {26: 160, 1: 72,
  11: 32}. The regularity is total and entirely implicit — a 26-level rack is 26 rows
  repeating the same io cell and direction.

`tote_type.json` is the only place a **template** appears in the whole format:
`racktype_id` → `floor_structure[]` (shelf levels lettered A–F, each with `base_height`)
→ `same_base_structure[]` → `rack_face_structure[]` (face 0 / face 1) → `slot_structure[]`
(`slot_tags`, `height`, `depth`, `x`, `y`, `length`, `slot_center`, `barcodes[]`). Units are
**centimetres** here (`rack_width: 97.9`) while `tote_storage.height` is **millimetres** —
in the same dump, describing the same racking. Nothing joins `tote_type` to
`tote_storage`: the geometry template and the position list never meet.

---

## 10. Glossary

Confidence: **H** grounded in the data or cross-checked; **M** strongly implied; **L** guess.

| Term | Best-guess meaning | Conf |
|---|---|---|
| **cell / location / point** | one addressable square of floor a robot can occupy; the format's atom | H |
| **barcode** | the physical sticker on that square; also the export's alternate key | H |
| **zone** | a named region used for pausing and blocking traffic wholesale; membership is per-cell, geometry undeclared | H |
| **defzone** | the default zone every unassigned cell falls into | H |
| **sector** | a partition of storage used to steer where carrier types go (`sectorMxUPreferences` maps carrier type → ordered sector list); no attributes of its own | M |
| **block** | a named group of cells forming a queue, a waiting area, or a conveyor touch point at a station | H |
| **queue / waiting / sequenced_queue** | the three `block.entity_type`s: ordered approach lane, holding area, and an ordered lane whose sequence is enforced | M |
| **PPS** | Pick/Put Station — an operator workstation; the unit of `pps.json` | H |
| **presentation point** | the exact square where a robot presents a carrier to a human, with the height and bot orientation it presents at | H |
| **is_workbench** | this presentation point is where the operator actually works (as opposed to a staging point) | M |
| **light curtain** | optical safety barrier at the presentation point | H |
| **location profile** | a named bundle of "which bot types, in which orientations, with what lift height and ground clearance, may occupy cells of this kind" | H |
| **VA / HA / VCA / HCA** | Vertical Aisle / Horizontal Aisle / Vertical Cross-Aisle / Horizontal Cross-Aisle — profile name components; "vertical"/"horizontal" refer to the drawing, not gravity | M |
| **S / S_left / SNorth** | cells under **static** (non-mobile) racks, where an HTM may pass beneath at limited lift height | H |
| **store_status** | 0/1 flag marking a cell as a rack-parking square; RTP-era, unused by relay storage | M |
| **storable / relay_store** | cell classification meaning a carrier may be stored at or via this cell | M |
| **ODS** | the obstacle-detection sensing an exclusion list turns off for specific (cell, direction) pairs; every resolvable entry sits at a charger, where the fixture would otherwise read as an obstacle | M on the acronym, H on the behaviour |
| **MxU** | the family name covering MSU and MTU (see below) | M |
| **MSU** | Mobile/Movable Storage Unit — a rack a robot carries | M |
| **MTU** | Mobile/Movable Transport Unit — a rollcage/trolley a robot carries; `dock_point.mtu_id` names individual ones | M |
| **rollcage_type_id** | the MTU type a docking station accepts; joins to `tote_type.racktype_id` | H |
| **VTM / HTM** | Vertical / Horizontal Transfer Module — the two robot classes in a relay system: VTMs work the aisles, HTMs the cross-aisles and stations | H |
| **relay / relay_point** | the handover model where a VTM and an HTM exchange a tote at a designated cell | H |
| **RTP** | the older rack-to-person system (`eligible_system: ["rtp"]`, `pps_points[].type: "rtp"`), where one robot carries a whole rack to the station | H |
| **butler** | the GreyOrange orchestration system; `orchestrator: "butler"`, `attached_butler_id`, `blocked_by_butler_ids` — "butler" also stands in for "robot" in those field names | H |
| **bot / botid** | a robot; `botid` is which robot is standing on the cell right now | H |
| **TLOC** | Tote LOCation — a storage position id | H |
| **io_point** | the cell a robot stands on to reach a storage position or a conveyor port | H |
| **ndeep** | how many carriers deep into the rack this position sits | H |
| **PPTL** | Put-To-Light — the lamp/display peripheral on a put-wall bin, one per bin | H |
| **pps_bin / seat** | a cubby in the station's put wall, and the operator position (front/back) that serves it | H |
| **easyloop** | a station type — a continuous loop conveyor feeding an operator | M |
| **side_dock** | a charger type where the robot docks alongside rather than nose-in | M |
| **reinit point** | a designated square where a robot re-establishes its position; encoded off-grid on the `[500,500]…` diagonal and marked `special: true` | H |
| **dock point** | a fixed floor position where an MTU/rollcage is parked at a station | H |
| **VDA5050** | the AGV interoperability standard whose map frame `vda_world_coordinate` targets | H |
| **HAI / QT** | vendor tags inside bot-type names: Hai Robotics and Quicktron | H |
| **conveyor blackhole** | the conveyor step where items leave the modelled world | M |
| **hai_port_types** | vendor-specific conveyor port classification; empty in every dump | L |
| **opc_connection** | OPC-UA link to plant equipment; empty in every dump | L |
| **zone_controller** | hardware controlling a zone's gates/interlocks; empty in every dump | L |
| **fire_emergency** | evacuation behaviour per region; empty in every dump | L |
| **queue_data** | runtime queue occupancy; empty in every dump | L |
| **grid_attribute** vs **composite_attribute_list** | two competing per-cell classification channels — one scalar enum, one list; site-a carries both | H |

---

## 11. Top 12 implications for the new format

1. **Declare the frames, never the projections.** The single most expensive thing here is
   that Kx and Ky exist only as a statistical property of 5000 cells. State one primary mm
   frame plus explicit transforms for the legacy grid frame, the VDA5050 frame, and each
   vendor frame — including the **y reflection**, which is the exact detail a hand port
   will get wrong. `vda5050_reference` is the anti-pattern: a single sample point offered
   in place of a transform.

2. **Store position once.** Three stored copies produced 125 mutually inconsistent
   transforms on one site and 5 mm disagreements on another. A *baked* document that
   carries projections must mark them derived so a validator recomputes and rejects drift.

3. **Give storage positions structured identity and put the rack back.** `TLOC_0069739` for
   the 21737th row is an allocation counter that survives nothing. A rack should be a
   declared entity whose positions derive from it
   (`UUIDv5(rack, "slot:{face}:{level}:{deep}")`) and stay individually declarable. The
   natural key `(io cell, orientation, direction, ndeep, height)` is already unique across
   all 55721 rows — the data is begging for it.

4. **Extent is not a list of cells.** An aisle is 39 coordinates in `vtm_zone.json`; a zone
   and a station have no geometry at all. Declare regions with origin and size in mm and let
   cells materialize, so "which cells are in aisle V1" is a derivation, not a stale list.

5. **Ban display strings from reference position.** `"Zone 102 Exite Gate "` is the join key
   for 925 cells. IDs are UUIDs; `name` is never unique and never referenced. Kill the
   implicit template join (`PPS_123` ↔ `pps_id: 123`) with it — site-a has 20 zones pointing
   at stations that no longer exist.

6. **One direction/orientation type, defined once, with named bearings.** The 0–3 enum is
   genuinely shared across nine files — the format's best idea. Reuse it as a named type
   whose compass meaning is written down: the dumps do not let a reader decide which of
   `1`/`3` is east, and the grid/world mirroring makes guessing wrong silent.

7. **Separate the world from what is happening in it.** `botid`, `charge: 13.9534`,
   `status: "disconnected"`, `pps_url: "http://localhost:8181/…"`, `paused` — the map cannot
   be diffed across two days because a robot moved. World, scenario, and live state are
   three things, not two.

8. **Make generation and vendor declared, not sniffed.** Today a consumer tells RTP from
   relay by whether `location_profiles` exists, and Quicktron from HAI by string prefixes
   inside bot-type names. Both belong in explicit components, so a consumer can reject what
   it does not support instead of half-parsing it.

9. **Adopt `location_profiles` — the one thing worth keeping nearly as-is.** A named profile
   mapping bot type × orientation → lift-height ranges and ground clearance is exactly the
   "physical capability, separate from policy" split we want. Fix only the encoding: the
   string `"all"` alternating with integer mm, and `[]`/`null`/list as three spellings of
   "unconstrained", must collapse to one representation each.

10. **Reify relationships that carry data — and scope their references.** `associations`
    with `type: "allowed_transport"` and `connections: [{from, to}]` is the right instinct,
    executed with stringly-typed handles (`"block-3"`) that silently mix a global namespace
    with a station-local one. Keep the shape, make the endpoints entity refs.

11. **The cell's edge grade needs a design, not a port.** `neighbours[i] = [a,b,c]` takes
    exactly four monotone values, nothing explains bits 2 and 3, and the same edge is graded
    differently from each end 1454 times in site-b alone. Express it as a named policy
    component, and get the intended meaning from the Map Creator owners, not from the bits.

12. **Plan for what the empty files promise.** `elevator`, `fire_emergency`,
    `zone_controller`, `opc_connection`, `queue_data` are empty on all four sites and
    `floor_id` is `1` everywhere — but they name real organizational needs: multi-floor
    transfer, emergency behaviour, zone interlocks, plant-equipment links, queue state.
    Multi-floor especially must be first-class in the frame model from day one; retrofitting
    a floor axis onto a 2-D grid atom is how this format got `floor` on 1198 cells.
