# How Butler represents and consumes world/map data

Reverse-engineering notes for the unified world/scenario format. Source of truth for every
claim below is `/Users/daniel/vt/greyorange/butler_server` (Erlang/OTP 27 umbrella) unless
stated otherwise. Cross-referenced against two Map Creator exports:

- `/private/tmp/claude-501/-Users-daniel-vt-greyorange-greyorange-foundry/8f4a86f9-2f62-4359-9f95-cef9f1e71603/scratchpad/mapcreator/walmart-pot/`
  (recent, relay/HTM/VTM site — referred to below as **walmart-pot**)
- `/Users/daniel/vt/greyorange/warehouse_map/` (older RTP site — referred to below as **legacy-rtp**)

Paths in this document are repo-relative to `/Users/daniel/vt/greyorange/butler_server`
except where an absolute path is given.

---

## 1. Executive summary

**Butler's world model is a set of Mnesia tables keyed by a 2-D integer grid coordinate.**
The single spine record is `#gridinfo{}` (`apps/mhs/include/gmc.hrl:443`), one row per
navigable cell, primary key `coordinate = {X, Y}`. Everything else — stations, storage,
chargers, elevators, conveyors, dock points, aisles, blocks — is a separate table that
*references* cells by that coordinate tuple (or by the cell's barcode string, which resolves
back to a coordinate by table lookup).

**There is no entity id system.** Identity is positional: a location *is* its `{X, Y}`.
`pps_id` is an integer, `rack_id` a binary, `charger_id` a number, `dock_point_id` is
literally a barcode (`src/gm_common.hrl:493`). There is no uniform id type, no id namespace,
and no stable identity that survives a map re-export — if Map Creator renumbers the grid, all
references break.

**Two coordinate frames are stored; a third is computed.** `gridinfo.coordinate` is the
integer grid index. `gridinfo.world_coordinate` is a millimetre pair. The VDA5050 frame is
*derived at read time* from a single per-floor reference point by pure translation with a
Y-axis flip (`apps/mhs/src/layout/models/gridinfo.erl:557-575`). Butler never reads the
per-cell `vda_world_coordinate` that Map Creator writes into `map.json` — that field appears
nowhere in the Erlang source. Barcodes likewise are opaque to Butler: `barcode_to_coordinate/1`
is a Mnesia select, not arithmetic (`gridinfo.erl:705-729`).

**The grid frame is load-bearing, and mm is bolted on.** Three receipts: (a) when a cell has
no explicit `adjacency`, neighbour resolution falls back to "+1/-1 formula"
(`apps/mhs/src/layout/butler_map_functions.erl:879-889`); (b) the post-upload sanity check
*rejects* a map whose cells are not rectilinearly aligned — the north neighbour must share the
same world X, the east neighbour the same world Y
(`apps/mhs/src/validation/grid_sanity_check_functions.erl:51-90`); (c) the VDA5050 `nodeId`
sent to vendor robots is the literal string `"{X,Y}"` of the GO grid coordinate
(`apps/mhs/src/fleet/butler/interface/proto/vda5050.erl:1396`).

**Declared vs derived is already a real distinction in the code, just unnamed.**
`gridinfo.attribute` (the cell's role: `highway`, `storable`, `pps`, `charger`, `conveyor_track`, …)
is *recomputed* by unioning eleven other sources and defaulting the remainder to `highway`
(`gridinfo.erl:966-998`), overwriting whatever `grid_attribute` the JSON supplied. Physical
aisles are computed from the map; virtual aisles are stored rows seeded from the computed ones
(`apps/mhs/src/layout/aisle_manager_utils.erl:155-213`). `mapdet` keeps `sizeinfo` (declared)
next to `actual_sizeinfo` (measured by robots during calibration) — same shape, different
provenance, no marker distinguishing them at read time.

**Storage is split.** The live world lives in Mnesia (in-memory, disc_copies for some tables).
Versioned map *snapshots* live in PostgreSQL table `gmc_maps(map_id, version, parent_version,
create_time, vda5050_json, gmc_json, status)` (`src/migrations/af7a9c46b_migration.erl:50-59`).
Map upload is a destructive whole-map replacement wrapped in a Mnesia backup/restore, and the
success log line literally ends `"Need butler_server restart"`
(`apps/mhs/src/interfaces/in/gridinfo_api_handler.erl:1022`).

**No riak.** Despite `bucket()`/`metadata()` types surviving in `src/gm_common.hrl:347-350`,
the DB layer is Mnesia + PostgreSQL only.

---

## 2. Ingestion and storage architecture

### 2.1 Which files go where

Map Creator produces a directory of JSONs; an operator (or CT setup code) POSTs them in a fixed
order. The order matters — `map.json` needs zones to exist, `tote_storage.json` needs cells to
exist. The authoritative mapping is the CT harness
`apps/mhs/test/ct/mhs_ct_common.erl:1493-1512` (`get_url/2`) together with the ordered walkthrough
in `setup_walmart_pot_data_with_conveyor/0` (`mhs_ct_common.erl:1171-1212`):

| File | Endpoint | Handler | Lands in |
|---|---|---|---|
| `zone.json` | `POST /api/zonerec/` | `src/web/butler_web_api_handler.erl:68` | mnesia `zonerec` |
| `map.json` | `POST /api/map/` | `gridinfo_api_handler:api_new_map/2` (`:436`) | mnesia `gridinfo`, `mapdet`, `pathinfo`, `storage_info`; then pg `gmc_maps` |
| `block.json` | `POST /api/mhs/v1/add_blocks/` | `butler_mhs_api_handler:api_add_blocks/2` (`:6892`) → `gmc_pps_api_handler:add_blocks/1` (`:672`) | mnesia `block` |
| `pps.json` (v3) | `POST /api/mhs/v3/pps/` | `gmc_pps_api_handler:get_pps_points_v3/3` (`:399`) | mnesia `ppsinfo` |
| `tote_storage.json` | `POST /api/mhs/tote_storage/` | `butler_mhs_api_handler:api_tote_storage/2` (`:2865`) → `tote_storage_functions:create_ttp_map/1` (`:27`) | mnesia `tote_storage` |
| `tote_type.json` | `POST /api/racktypes/` | `butler_mhs_api_handler` trails `:83` | mnesia `racktypeinfo` |
| `conveyor.json` | `POST /api/mhs/v2/conveyor_info/` | `butler_mhs_api_handler:api_conveyor_info/2` (`:2905`) | mnesia `conveyor_info`, `conveyor_location`, `conveyor_map_points`, `hai_port_info` |
| `charger.json` | `POST /api/chargers/` | `src/web/butler_web_api_handler.erl:2865` | mnesia `chargerinfo` |
| `elevator.json` | `POST /api/elevator` (dispatched by `path_info`, `:274`) | `butler_web_api_handler:api_add_elevators/2` (`:3865`) | mnesia `elevatorinfo` |
| `vtm_zone.json` | `POST /api/mhs/v1/aisle/` | `gmc_aisle_http_controller:from_aisle_info_map/1` (`:970`) | mnesia `aisle` (type `virtual`) |
| `dock_point.json` | `POST /api/mhs/dock_points` | `butler_mhs_api_handler:api_add_dock_points/2` (`:1108, :3107`) | mnesia `dock_point` |
| `ods_excluded.json` | (model exists: `apps/mhs/src/layout/models/ods_excluded.erl`) | | mnesia `ods_excluded` |
| `sectors.json` | **not ingested** — GET-only export | `butler_mhs_api_handler:api_get_sector_json/2` (`:2525`) | derived from `gridinfo.sector` |
| `conveyor_mapping.json` | no consumer found in butler_server | | — |
| `sectorMxUPreferences.json` | no consumer found in butler_server (empty in sample) | | — |

Legacy-rtp adds `pps_bin.json` (`POST /api/add_pps_bins`), `pps_bintype.json`
(`POST /api/ppsbintypes`), `pps_seat.json`, `pptl.json` — these carry the URL *inside the file*
(`{"url": "/api/add_pps_bins", "type": "POST", ...}`). They target the pick/put station-furniture
tables (`ppsbin` in `apps/pick/include/pick.hrl:187`, `ppsbin_type` in `src/gm_common.hrl:161`,
`pps_seat` — record shape only survives in migrations, e.g. `src/migrations/af17766f4_migration.erl:9`).

`sectors.json` deserves emphasis: **it is a redundant projection.** Sector membership is carried
per-cell in `map.json`'s `sector` field, written into `gridinfo.sector` at ingest
(`gridinfo_api_handler.erl:474-478, 548`), and re-emitted grouped-by-sector by the GET endpoint.
Shipping it as a separate file is a round-trip of data Butler already has.

### 2.2 The map upload transaction

`gridinfo_api_handler:api_new_map/2` (`:436`) → `handle_on_valid_map_schema/3` (`:864`), twelve
logged steps: (1) `validate_system_state()` refuses upload unless emergency level is none;
(2) `validate_map_json/2` picks the schema — presence of a `map_values` key means `multi_floor`,
otherwise `single_floor` (`json_validation_rules.erl:821-827`), and **walmart-pot's `map.json` is
the multi_floor schema with a single floor**; (3) currently-blocked cells are remembered via
`gridinfo:search_blocked_coordinates()` so the new map re-blocks them; (4)
`db_functions:take_mnesia_backup()` dumps all of Mnesia to a file; (5)
`gmc_map:run_add_map_pre_hooks()` → `ranger_comm_interface:block_communication()` cuts robots off
for the duration (`gmc_map.erl:110-114`); (6) `butler_map_functions:clear_layout_data()`
**wipes the existing world**, then `create_normal_map/2` or `create_multilevel_map/2` re-creates
every cell; (7) `gmc_map:add_current_map()` snapshots to PostgreSQL with a bumped version;
(8) `validate_storables()` plus
`grid_sanity_check_functions:validate_alignment_for_all_coordinates()`; (9-12) cache flush,
`storage_info:update_all_io_points()`, `butler_rack_functions:add_old_racks_to_new_map()`, Kafka
`entity_sector_updated`, heuristic reset, and the closing
`"Map posted successfully. Need butler_server restart"`.

Any exception or validation failure triggers `clear_layout_data()` +
`db_functions:restore_mnesia(BackUpFile)` (`:969-987`). So the unit of change is
**the entire map**, and the rollback unit is **the entire Mnesia database**.

### 2.3 Storage layers

- **Mnesia** — the live world. Table definitions are enumerated in `apps/mhs/src/gmc_models.erl`
  (1134 lines, ~70 tables), applied by `erl_migrate` migrations under `apps/mhs/src/migrations/`
  and `src/migrations/` via `apps/mhs/src/gmc_db_setup.erl`. `gridinfo` is `{type, set}` with
  secondary indexes `{index, [#gridinfo.barcode, #gridinfo.zone]}`
  (`src/migrations/a5669798e_migration.erl:112`; expressed as `{index, [3, 9]}` in
  `gmc_models.erl:456`) — so barcode lookup is indexed, everything else is a table scan.
- **PostgreSQL** — versioned snapshots and config. `gmc_maps` DDL at
  `src/migrations/af7a9c46b_migration.erl:50-59`. Also `gmc_configuration` (holds floor metadata
  incl. the VDA reference point), `pps_profiles`, `pps_bin_tags` (`priv/sql/*.sql`).
- **ETS / persistent_term** — derived caches: `vda5050_map_cache`, `vda5050_partial_map_cache`,
  `map_version_cache` (`gmc_map.erl:403-410`), zone buffer areas in `persistent_term`
  (`apps/mhs/src/layout/zone_data_utils.erl:23`), storage-io-point mapping ETS in
  `aisle_manager_utils`.
- **No riak, no other store.**

### 2.4 Versioning and change

`#gmc_map{}` (`gmc_map.erl:69-78`): `map_id`, `version`, `parent_version`, `vda5050_fmt`,
`gmc_fmt`, `status`, `created_at`, `activated_at`. Versions are `Major.Minor.Revision` strings
with bounds `?MAJOR_VERSION_MAX 100 / ?MINOR_VERSION_MAX 99 / ?REVISION_MAX 99` (`:91-96`).
`add_current_map/0` (`:164`) makes the new version's parent the previously-active version, so
history is a chain, not a tree with merges. `map_id` defaults to the installation id
(`?MAP_ID` at `:97`).

Partial updates exist only for one narrow case: `vda5050:create_partial_map/2` handles
`{<<"barcode_replacement">>, List}` (`vda5050.erl:1407`), backed by the `barcode_replacements`
table (`gmc.hrl:833`) which maps `old_barcode → new_barcode`. That is the mechanism for
reprinting a damaged floor label without a full map re-upload — and it is the clearest evidence
that **barcode is treated as a physical artifact identifier, separable from the location**.

Two validation routines worth stealing wholesale:

- **Rectilinear alignment** (`grid_sanity_check_functions.erl:51-90`): for every cell, the north
  neighbour must have the same `world_coordinate` X and the east neighbour the same Y, skipped
  across "stitching points" where one side is a TTP-attribute cell and the other is not
  (`stitching_point_check/2` at `:40`). A failing map is rejected with
  `"Failed: Coordinates are misaligned"` (`gridinfo_api_handler.erl:942`).
- **Storable reachability** (`storage_info:validate_new_storable/2`, `:163-183`): a storage cell
  must have an io point, and the cell's size in the io point's direction must be `>= 750` mm.
  Cell-size-derived feasibility, not a global constant.

---

## 3. World model reference

### 3.1 `#gridinfo{}` — the cell (`apps/mhs/include/gmc.hrl:443-460`, types at `:954-971`)

```erlang
-record(gridinfo, {
    coordinate = {0, 0},                        % {integer(), integer()} — PRIMARY KEY
    barcode = null,                             % string() — physical floor label; indexed
    botid = null,                               % butler_id() — live occupancy, NOT map data
    neighbours = [[1,1,1],[1,1,1],[1,1,1],[1,1,1]],
    adjacency = undefined,                      % [coordinate()|null] per direction, or undefined
    special = false,                            % boolean() | lift
    block_reasons = [],                         % [{coordinate_block_reason(), datetime()}] — runtime
    zone = null,                                % zone() :: binary(); indexed
    proximity = undefined,                      % vestigial: only ever read/written as undefined
    absolute_position = undefined,              % vestigial: same
    floor = undefined,                          % integer()
    world_coordinate,                           % {number(), number()} in mm
    sector = undefined,                         % integer() | undefined
    attribute,                                  % grid_attribute() — DERIVED, see §3.2
    barcode_properties = #{},                   % per-bot-version permission map
    composite_attribute_list = []               % [grid_attribute()]
}).
```

Identity: `coordinate`. Data: everything else. `botid` and `block_reasons` are *live state*
stored in the same record as map data — a structural mixing the new format should not copy.
`proximity` and `absolute_position` are dead: the only references are the `'_'`/`undefined`
defaults in the match-spec builders (`gridinfo.erl:649-650, 671-672`).

**`neighbours`** is a list of four triples ordered N, E, S, W (direction 0,1,2,3 — see
`convert_direction_to_numeric/1`, `gridinfo_api_handler.erl:756-764`). Each triple is
`[NeighbourExists, BotMayMove, BotWithRackMayMove]`. The decode is explicit in
`vda5050_map:get_allowed_movements/1` (`:163-165`): `[1,0,0] → []`, `[1,1,0] → [bot]`,
`[1,1,1] → [bot, rack]`; `[0,0,0]` means no neighbour
(`butler_map_functions.erl:872-874`). **This is an edge property stored redundantly on both
endpoints, with no consistency check.**

**`adjacency`** names the actual neighbour cell per direction. When it is `undefined`, Butler
falls back to `get_existing_neighbour_in_direction/2` — the comment says
*"can use +1/-1 formula to get neighbour as adjacency not defined"*
(`butler_map_functions.erl:879-883`). walmart-pot's grid indices are sparse (29, 37, 44, 48, …),
so that site *requires* `adjacency`; older dense-grid sites did not have it.

**`barcode_properties`** is the per-cell, per-bot-version permission table. Its documented shape
is in the worked-example comment at `gmc.hrl:462-501` and the type at `:502-509`:
`bot_orientation_details :: #{bot_version() => #{direction() => #{lift_height_ranges => #{loaded|unloaded => [{Min,Max}]|[all], ground_height_clearance => integer()|all}}}}`,
plus `edge_height_clearance :: #{direction() => number()}`, `location_profile_id`, and
`bot_orientation_details_source :: barcode_properties | location_profile`. Absence is
permissive: empty map = all bots, all orientations, all heights. **This is the richest piece of
the current format and the one most obviously crying out for composition rather than nesting.**

**`location_profiles`** (walmart-pot `map.json` top level) are named reusable
`barcode_properties` bundles — an inline prototype/instancing mechanism. Merge rules at
`gridinfo_api_handler.erl:767-862`: the profile wins for `bot_orientation_details`, the cell's own
`barcode_properties` are the fallback, and the winner is recorded in
`bot_orientation_details_source`. walmart-pot defines 21 profiles covering 1296 of its 1424 cells, with names that carry the whole
semantics in a string, e.g. `"VA(VTM_Aisle)"`, `"S+HCA(HTM_crossaisle_under_static_racks)"`,
`"HTM(_Under_Tunnel)"`.

### 3.2 `grid_attribute()` — cell role (`gmc.hrl:1143-1174`)

```
pps | pps_entry_queue | pps_exit_queue | charger | charger_reinit | ramp
| vertical_aisle | horizontal_aisle | aisle_cross_point | highway | dock_point | storable
| elevator | elevator_entry | elevator_exit | elevator_entry_path | elevator_exit_path
| ttp_location | conveyor_entry | conveyor_exit | conveyor_pps_point | conveyor_wormhole
| conveyor_track | conveyor_blackhole | ttp_storable_io_point | conveyor_entry_io_point
| conveyor_exit_io_point | port_exit_io_point | port_entry_io_point
| relay_storable | relay_storable_io_point
```

`gridinfo:get_coordinate_to_grid_attribute_map/0` (`:966-998`) builds this by merging eleven
sources — computed aisles, `chargerinfo`, `elevatorinfo`, `dock_point`, `storage_info`,
`ppsinfo`, `tote_storage`, `conveyor_info`, `hai_port_info`, relay storables, relay storable io
points — then assigning `highway` to every cell still `undefined`.
`check_and_set_grid_attribute_data/0` (`:1127-1167`) writes the result back into `gridinfo` and,
if anything changed, calls `gmc_map:update_map_jsons()`. So **`attribute` is a materialized view
persisted into the source table**, and the JSON's `grid_attribute` is a partial seed that gets
overwritten. Note walmart-pot's `map.json` emits `grid_attribute: "relay_store"`, which is *not*
a member of the type above (the type has `relay_storable`) and has no reader in butler_server —
it survives only until the recompute overwrites it.

### 3.3 Stations — `#ppsinfo{}` (`gmc.hrl:738-767`)

Key `pps_id :: integer()`. Fields: `location :: barcode()`, `status :: pps_mode()`,
`pick_position :: barcode()`, `pick_direction :: direction()`, `paused`, `availability`,
`turn_position :: coordinate()`, `type :: pps_type()`, `entry_queue_barcodes`,
`exit_queue_barcodes`, `special_queue_size`,
`inout_limit_at_pps :: #{pick|put|audit|search|unmanaged => integer()}`,
`eligible_system = [rtp] :: [eligible_system()]`, `pps_points :: [pps_point()]`, `is_active`,
`station_id`, `blocks :: [string()]`, `associations`, `queue_enabled`, `safe_to_drop`,
`allowed_modes :: [binary()]`.

`pps_point()` (`src/gm_common.hrl:357-363`):
`#{type => ttp|rtp, position_id => integer(), coordinate => {X,Y}, sub_type => workbench|conveyor|floor|cantilever, pick_height => integer()}`.

The v3 ingest (`gmc_pps_api_handler:get_pps_points_v3/3`, `:399-470`) collapses a much richer
JSON into that map. Two derivations worth naming:

- `presented_entity_allowed_types == ["tote"] → type = ttp`, `["rack"] → rtp`, anything else
  silently falls through to `ttp` (`:414-423`).
- `presenting_agent_type == "bot"` + `is_workbench` → `sub_type = workbench`; `"bot"` alone →
  `floor`; `"conveyor"` → `conveyor`; `"cantilever"` → `cantilever`; anything else → `conveyor`
  (`:424-437`).

**Naming trap:** the presentation point's `attributes.world_coordinates` in `pps.json` is a
**grid coordinate**, not millimetres — walmart-pot pps 1 has `world_coordinates: [29,5]` and
`butler` reads it straight into `coordinate` (`:443`). Butler's `world_coordinate` (singular, on
`gridinfo`) is mm. The two spellings mean different things in the same export.

### 3.4 Storage

Two parallel systems, never unified:

**RTP / rack storage — `#storage_info{}`** (`gmc.hrl:801-806`):
`coordinate` (key), `status :: undefined | available | {rack_id(), reserved} | {rack_id(), stored}`,
`storage_direction :: direction()`, `io_point :: [coordinate()]`. Populated from `map.json`'s
per-cell `store_status: 1` (`gridinfo_api_handler:add_storage_info/1`, `:1025-1041`). In
walmart-pot every cell has `store_status: 0` — that site stores totes, not racks.

**TTP / tote storage — `tote_storage`** (`gmc_models.erl:1018-1038`; no `-record` in a shared
header, model is `apps/mhs/src/layout/models/tote_storage.erl`). Attributes:
`location, io_point, storage_direction, height, ndeep, status, block_status, block_reasons,
storage_type, relay_storable_access_point, relay_storable_io_point, storage_relay_level,
storage_sector`. Key is `location` — a *string* id, `"TLOC_0000001"` in walmart-pot, **the only
opaque non-positional entity id in the whole world model**. Ingest at
`tote_storage_functions:create_ttp_map/1` (`:27-69`): `io_point` becomes
`{Coordinate, BotOrientation}`; presence of `relay_point` flips `storage_type` from `non_relay`
to `relay` and *derives* the relay access direction and io point by reading the relay cell's
`neighbours` structure and picking the single open side (`:71-97`).

**Rack types — `#racktypeinfo{}`** (`gmc.hrl:549-564`): `racktype_id :: string()` (key),
`pps_affinity_ids = all | [pps_id()]`, `intial_mass` [sic], `dimensions :: {L,W,H}` (inner
storage volume), `sway_tolerance`, `interlocking`, `sector_preference_list :: [integer()]`,
`inter_leg_dimensions`, `outer_dimensions` (including material), `height`. `tote_type.json`
carries far more than this — a full `floor_structure → same_base_structure →
rack_face_structure → slot_structure` tree with per-slot `barcodes`, `slot_tags`, `slot_center`,
`x`, `y`, `length`, `height`, `depth`, plus `density_profile` and `weight_limit_of_the_floor`.
That slot geometry lands in the pick/put inventory model, not in `racktypeinfo`.

**Rack instances — `#rackinfo{}`** (`gmc.hrl:659-685`): `id :: rack_id() :: binary()` (key),
`direction`, `position :: coordinate()`, `last_store_position`, `is_stored`,
`lifted_butler_id`, `racktype`, `upcoming_pps_list`, `type`, `is_blocked_map`, `inventory`
(a proplist of `inventory_present`, `is_put_blocked`, `is_pick_blocked`, `number_of_items`,
`center_of_mass`, `weight`), `sector_preference_list`. This is runtime state, not map data.

### 3.5 Chargers, elevators, conveyors, dock points

**`#chargerinfo{}`** (`gmc.hrl:776-792`): `charger_id` (key),
`charger_location :: {barcode(), direction()}`, `entry_point`, `reinit_point` (same shape),
`status :: connected|disconnected`, `attached_butler_id`, `mode :: charger_mode()`, `charge`,
`charger_type`, temperatures, error reasons, `power_status`. `charger.json` supplies exactly
the first six as flat `charger_location`/`charger_direction` pairs
(`butler_web_api_handler.erl:2865-2875`). `charger_type()` (`gmc.hrl:1078-1089`) is a closed
vendor-shaped enum: `rectangular_plate_charger | circular_plate_charger | side_dock | normal |
bottom_dock | quicktron_charger | quicktron_htm_charger | quicktron_vtm_charger |
hai_htm_charger | hai_vtm_charger | hai_charger`.

**`#elevatorinfo{}`** (`gmc.hrl:709-736`): `elevator_id` (key), `position :: barcode()`,
`current_floor :: {integer(), gate_closed|gate_opened}`, `attached_butler_id`,
`butler_event_queue_list`, `status`, `mode`, `type :: z_type | c_type`,
`entry_barcodes :: [{FloorId, Barcode, BoomBarrierId}]`, `exit_barcodes` (same),
`elevator_coordinates :: [{coordinate(), direction()}]`, `reinit_barcodes` (marked
*"Not used now, can remove"*). Upload is delete-all-then-insert inside one Mnesia transaction
with restore-on-validation-failure (`butler_web_api_handler.erl:3865-3895`).

**Conveyors** are three tables (`gmc_models.erl:272-345`): `conveyor_info` (topology and status:
`pps_locations`, `entry_locations`, `exit_locations`, `outgoing_conveyors`,
`incoming_conveyors`, `adjacency`, `blocked_steps`, `conveyor_type`), `conveyor_location`
(per-step: `location`, `world_coordinate`, `conveyor_id`, `type`, `status`, `totes`), and
`conveyor_map_points` (`location_id`, `conveyor_id`, `location_coordinate`, `io_point`,
`io_point_type`, `hai_port_id`, `hai_port_type`, `fork_height`, `fork_orientation`,
`bot_orientation`). Conveyor steps in `conveyor.json` carry both `coordinate` and
`world_coordinate` and reuse the grid frame with a sentinel X of `999` for off-grid track cells
(visible throughout walmart-pot `conveyor.json` and `sectors.json`).

**`#dock_point{}`** (`gmc.hrl:855-863`): `id :: dock_point_id() :: barcode()` (key),
`direction`, `display_name`, `rack`, `station_id :: binary()`, `station_type`, `status`.

### 3.6 `#mapdet{}` — cell geometry (`gmc.hrl:511-515`)

`coordinate` (key), `sizeinfo :: list()`, `actual_sizeinfo :: list()`. Both are four numbers in
N, E, S, W order — the distance in mm from the cell centre to the boundary in each direction
(the reader in `vda5050_map:generate_node/2` destructures `[NSize, ESize, SSize, WSize]` and
re-emits `[ESize, NSize, WSize, SSize]` for the wire, `:73-79`). At ingest both fields get the
same `size_info` value from `map.json` (`butler_map_functions.erl:302`); `actual_sizeinfo` is
later replaced by robot-measured values via `mapdet:update_actual_sizeinfo/2` (`:118-122`) during
`map_calibration`. **Declared geometry and measured geometry, same shape, same table, no
provenance marker.**

`#grid_meta{}` (`gmc.hrl:517-520`) is a separate per-cell side table holding only
`speed_zone :: normal | fast | slow | periphery`.

---

## 4. Coordinate systems and barcode mechanics

### 4.1 The three frames

| Frame | Where | Units | Owner |
|---|---|---|---|
| GMC grid | `gridinfo.coordinate` | integer index, sparse | Butler (primary key) |
| GMC world | `gridinfo.world_coordinate` | mm, site-relative, may be negative | Butler |
| VDA5050 | computed on read | mm, vendor-anchored, Y inverted | derived from a single reference point |

Conversion (`apps/mhs/src/layout/models/gridinfo.erl:557-575`):

```erlang
gmc_wc_to_vda5050_coord({X, Y}, #{gmc_world_coord := [RefX, RefY], vda5050_coord := [VX, VY]}) ->
    DeltaY = Y - RefY,
    DeltaX = X - RefX,
    {VX + DeltaX, VY - DeltaY}.
```

Pure translation with a Y-axis sign flip. The reference is one point per floor, supplied as
`vda5050_reference: {"barcode": ..., "vda5050_coordinate": "[X,Y]"}` at the floor level of
`map.json`, stored as floor metadata in PostgreSQL `gmc_configuration` and cached in
`application:get_env(mhs, floor_wise_vda5050_ref)`
(`gridinfo_api_handler.erl:661-671`, `gridinfo.erl:577-620`). If no reference is supplied, the
code picks the cell whose `world_coordinate` is exactly `{0,0}` and declares the frames identical
(`gridinfo.erl:602-615`) — a lookup that crashes with a match error if no such cell exists.

I verified the formula against walmart-pot: taking barcode `1123374354` (grid `[29,1]`,
world `[-3590,0]`, vda `[123370,143540]`) as the reference, the formula reproduces
`vda_world_coordinate` for **400/400** sampled cells exactly. So the per-cell
`vda_world_coordinate` field is fully redundant *and* unread by Butler — it exists for other
consumers.

**Who owns the conversion:** Butler does, for VDA5050 v1. There is a second, *vendor-specific*
path: `gridinfo:get_vda_cstep_coordinate_from_hai_barcode/1` (`:1386-1409`) hard-codes the HAI
barcode encoding — a 10-digit numeric barcode is split as
`(FloorCode1, FloorCode2, X4digits, Y4digits)` and expanded to
`{FC1*100000 + X*10, FC2*100000 + Y*10}`, with the comment worked example
`3212348765 → (312340, 287650)` and `FloorId` hard-coded to 1. **A vendor's barcode-numbering
convention is compiled into Butler's location model.**

### 4.2 Barcodes

`barcode() :: string() | null | undefined | binary()` (`src/gm_common.hrl:436`).

Butler treats a barcode as an opaque string. `gridinfo:barcode_to_coordinate/1` (`:705-729`) is a
`dirty_select` on the indexed `barcode` column, with an elevator special case that disambiguates
by the elevator's current floor. `coordinate_to_barcode/1` (`:746-763`) is a dirty read. Both
log a `?WARNING` with a stack trace when handed something already of the target type — an
anti-pattern guard that tells you how often the two are confused in this codebase.

Barcodes *are* generated from coordinates, but by Map Creator, not Butler. Confirmed by
reproducing walmart-pot's scheme exactly for **1424/1424** cells:

```
barcode = "1" ++ zero_pad_5(vda_x / 10) ++ last_4_of(zero_pad_5(vda_y / 10))
```

e.g. vda `[123370, 143540]` → `"1" ++ "12337" ++ "4354"` = `"1123374354"`. All 1424 are unique.
Legacy-rtp uses a completely different scheme: `"002.054"` = zero-padded `Y.X` in grid indices,
matching its `pps_points` coordinate `"[54,2]"`. **There is no single barcode grammar; there is
a per-site convention that Butler does not parse** (except the HAI fallback above).

Is barcode the runtime identity? Practically yes for anything a human or a robot names:
`ppsinfo.location`, `ppsinfo.pick_position`, `chargerinfo.charger_location`,
`elevatorinfo.position` and entry/exit barcodes, `dock_point.id`, `chargetaskrec` fields, and all
`goto_barcode` task details are barcodes, not coordinates. Internally the pathfinder and the
tables work in coordinates. So the system has **two identities per location with a lossy lookup
between them**, plus a `barcode_replacements` table to remap one without touching the other.

### 4.3 Directions and orientation

`direction() :: 0 | 1 | 2 | 3 | null | undefined` (`gmc.hrl:1070`), with
0 = north, 1 = east, 2 = south, 3 = west (`gridinfo_api_handler:convert_direction_to_numeric/1`,
`:756-764`). VDA5050 theta mapping: `0→90, 1→0, 2→-90, 3→180`, with `200` as the
"unknown" sentinel (`vda5050.erl:1442-1446`). Rack faces are 0-3 with a
`?DEFAULT_RACK_FACE_TO_DIRECTION_OFFSET_MAP` of `#{0=>2, 1=>0, 2=>3, 3=>1}` (`gmc.hrl:125-130`).

### 4.4 Hardcoded grid assumptions

- Neighbour by `+1/-1` when `adjacency` is absent (`butler_map_functions.erl:879-883`).
- Rectilinear alignment enforced at upload (`grid_sanity_check_functions.erl:51-90`).
- VDA5050 `nodeId = "{X,Y}"` — the grid index is the identifier on the vendor wire
  (`vda5050.erl:1396`); edge ids are `"{X1,Y1}-{X2,Y2}"` (`:1400`).
- `avg_barcode_distance` defaults to 1220 mm (`mhs_setup.erl:331`) but is *recomputed from the
  map* by `config_data_sanity_check_functions:get_barcode_distances_from_map/0` (`:219-259`), which
  takes the **modal** inter-barcode distance as the "average" with the honest comment
  *"Not applicable for rectangular grid ... TODO: logic to be revised for rectangular grid"* (`:239-240`).
- `750` mm appears as a minimum face clearance for storables (`storage_info.erl:171-179`) and as a
  `pillar` size (`butler_map_functions.erl:3305`). It is not a global cell pitch constant.
- `zone_buffer_distance` defaults to 1700 mm (`zone_data_utils.erl:9`).
- Single-floor maps hard-code `floor = 1` (`butler_map_functions.erl:294`).

---

## 5. Zone, sector, block, vtm_zone (aisle) semantics

These four are all "sets of cells", and they gate four unrelated things.

**Zone — safety.** `zone() :: binary()` (`gmc.hrl:1033`). `#zonerec{}` (`gmc.hrl:877-886`):
`zone_id` (key), `blocked :: boolean()` (the "zone clear" flag),
`pause_reasons :: [zone_pause_reason()]`, `paused_butlers :: [butler_id()]`, `zone_details`.
Every cell carries exactly one `zone`, defaulting to `<<"defzone">>` when `map.json` omits it
(`gridinfo_api_handler.erl:519-523`). Pause reasons are physical-safety events
(`gmc.hrl:1050-1054`): `button_press`, `light_curtain_breach`, `access_request`,
`loto_zone_press`, `station_breach`, `controller_disconnected`, `manual`. A zone block becomes a
per-cell `block_reason` of `{zone, ZoneId}` (`gmc.hrl:1448`), surfaced to robots as
`<<"zone-blocked">>` (`vda5050_map.erl:112`). `zone_data_utils` additionally computes a *buffer
area* of cells within 1700 mm of a zone for pre-emptive stopping (`:8-36`). **Zone is an
e-stop / light-curtain domain, nothing else.** Zones must exist before the map is uploaded —
`zone.json` is posted first.

**Sector — inventory placement policy.** `gridinfo.sector :: integer() | undefined`, plus
`tote_storage.storage_sector`. It gates put/replenishment: `apps/put/src/sector_tier.erl` filters
candidate racks by a priority-ordered list of sector sets, and its header states
*"This is a HARD CONSTRAINT - no fallback to non-sector racks. If all sector sets are exhausted
and quantity is not fulfilled, it results in Warehouse Full"* (`:8-11`). The preference list
lives on `racktypeinfo.sector_preference_list` (`gmc.hrl:557`), `rackinfo.sector_preference_list`
(`:684`), and per-tote via `tote_info_db` (`:180`). Sector changes emit Kafka
`entity_sector_updated` (`gmc_sector_http_controller.erl:151-156`). **Sector is a storage-policy
partition; it has nothing to do with traffic or safety.**

**Block — a named ordered set of cells used as a queue.** `#block{}`
(`apps/mhs/src/layout/models/block.erl:57-61`): `block_id :: string()` (key),
`entity_type :: binary()`, `entity_details :: #{binary() => term()}` (an untyped blob holding
`coordinates` and optional `extra_details`). Valid types (`block_validation_functions.erl:25-31`):
`waiting`, `queue`, `sequenced_queue`, `conveyor-point`, plus `waiting_block` retained "for
backward compatibility" with the in-source comment *"Inconsistent naming of entity_type"*.
`sequenced_queue` blocks accept `extra_details.sync_point` and `extra_details.exit_point`, each
of which must be one of the block's own coordinates (`:98-110`). Blocks are addressable as task
destinations: `parse_destination(<<"sequenced_queue_block">>, #{block_id, sequence_index})`
(`transport_request_event_handler.erl:2913`). **Block is ordering and staging, and its payload is
schemaless.**

**vtm_zone / aisle — a storage lane.** `#aisle{}` (`apps/mhs/src/layout/models/aisle.erl:54-60`):
`id :: integer()` (key), `type :: virtual | physical`, `coordinates :: [{X,Y}]`,
`adjacency :: [aisle_id()]`, `display_name :: binary()`. Ingest from `vtm_zone.json`'s
`aisle_info` (`gmc_aisle_http_controller:from_aisle_info_map/1`, `:970-977`). Two notes:
`additional_information` — which is where walmart-pot puts `{"type": "VTM"}` — is parsed and
**discarded** (`_ = parse_additional_information(...)`, `:976`); and the aisle `type` comes from
a top-level `type` key defaulting to the config `aisle_type_enabled` (`:1238-1245`), so the file's
declared VTM-ness never reaches the record. Physical aisles are *computed* from the map by
`aisle_manager`; virtual aisles are Mnesia rows, auto-seeded by copying the computed physical
ones when the table is empty (`aisle_manager_utils:155-213`). Aisles drive relay/tote storage
selection: `get_free_relay_locations_in_aisle`, `compute_aisle_popularity_score_map`,
`aisle_to_pps` distances (`aisle_manager.erl:132-200`, `gmc_aisle_http_controller.erl:37-56`).

**Cell-role aisles are a fifth, separate thing.** `vertical_aisle`, `horizontal_aisle`,
`aisle_cross_point` are `grid_attribute()` values derived by
`butler_map_functions:calculate_complete_aisle_map/0` (`:1893-1896`) from the neighbour
structure — unrelated to the `aisle` table.

**ODS** — `#ods_excluded{}` (`gmc.hrl:522-525`): key `ods_tuple :: {coordinate(), direction()}`,
value `excluded :: boolean()`. Read by exactly two call sites:
`butler_manager.erl:12347` and `butler_map_functions.erl:2124`, both via
`ods_excluded:check_excluded_coordinate/2`, gating whether a bot's obstacle-detection sensor is
suppressed while traversing that cell in that heading. Migration `src/migrations/a7dfa36e9`
shows the key was once `{barcode(), direction()}` and was rewritten to coordinates. In
walmart-pot `ods_excluded.json` the tuples are still serialized barcode-first
(`"1113740350--1"`). **ODS = obstacle-detection-sensor exclusion, keyed by (cell, heading).**

**PPTL** = put-to-light. `pptl_id() :: integer()` and `pptl` as a `peripheral` type
(`src/gm_common.hrl:502, 513, 520, 577`). `pptl.json` binds `(pps_id, pps_bin_id, peripheral_id)`
to a physical light module with its own `barcode` (`"B10_1"`). `pps_bin` is a physical tote/bin
slot at a station (`apps/pick/include/pick.hrl:187`) with a `position :: [row, col]`,
a `coordinate :: [mm, mm]` **in station-local millimetres**, a `direction :: left|right`, a
`type_id` into `ppsbin_type`, `allowed_tags`, and front/back `seat_name`s. `pps_seat` names an
operator-facing side of the station (`front`/`back`). So a station has its own internal
coordinate frame and its own furniture hierarchy: `pps → seat → bin → pptl`.

---

## 6. Bot and vendor model

**Bot types are strings, enumerated in macros in a header.** `apps/mhs/include/gmc.hrl:18-275`
holds `?VDA_PROTO_BOTS`, `?TTP_BOTS`, `?RTP_BOTS`, `?HTM_BOTS`, `?HTM_ORDER_BOTS`, `?VTM_BOTS`,
`?FORK_ENABLED_BOTS`, `?CONVEYOR_UNDERPASS_BOTS`, `?HAI_BOT_VERSIONS`, `?HTM_BOT_VERSIONS`,
`?VTM_BOT_VERSIONS`, `?PICK_SOLUTION_SUPPORTED_BOT_VERSIONS` — each an
`application:get_env/3` with a hardcoded default list. The vendors visible in those lists:

- **HAI / Hikvision** — `hai_RTTP_A42D`, `hai_RTTP_A42N`, `hai_RTTP_A42G_E4`, `hai_RTTP_A42X_E4`,
  `hai_RTTP_A42TD_L_E1`, `hai_RTTP_A42TL_E1`, `hai_RTTP_A42D_L_E4`, `hai_RTTP_A42T_L_E1`,
  `hai_relay_K50`, `HTM_HAI_K50_L_E1`, `HTM_HAI_K50_X_E1`, `VTM_HAI_A42_TDV_E1`
- **Quicktron** — `Quicktron_M100(CE)`, `Quicktron_M60(CE)`, `HTM_QT_M5F`,
  `VTM_QT_C56_ST_2D`, `VTM_QT_C56_S_2D`, `VTM_QT_C56_GT_2D`, `VTM_QT_C56_G_2D`
- **GreyOrange native** — `"1.5"`, `"2.0"`, `"2.1"`, `"XL"`, `"XL1.1"`, `"RangerGTP"`, `"M3.0_CE"`

`?MANUFACTURER` defaults to `"hai"` and `?MANUFACTURER_MAP` maps protocol `"v1" → "quicktron"`
(`vda5050_v2.erl:54-57`).

**Bot categories** — `butler_category() :: rtp | ttp | vtm | htm | undefined`
(`apps/mhs/src/fleet/butler/models/ranger_profile.erl:35`). RTP = rack-to-person (lift a whole
rack), TTP = tote-to-person, HTM = horizontal tote mover, VTM = vertical tote mover.

**Bot capabilities live in `#ranger_profile{}`** (`ranger_profile.erl:37-44`), keyed by
`version :: butler_version() :: string()`: `category`, `dimensions :: tuple()`,
`velocity_profile :: #{primary|secondary => ...}`, `sway_tolerance_details`, `idc_calculated`.
Live per-robot state is `#butlerinfo{}` (`gmc.hrl:387-420`), and fork/tray capability lives in
separate tables `butler_fork_info` (`max_height`, `max_depth_retrievable`,
`current_fork_direction`, `current_fork_height`) and `butler_tray_info`
(`gmc_models.erl:65-121`).

**Capability is expressed twice, in opposite directions.** From the *bot* side,
`?DEFAULT_EXCLUDED_GRID_ATTRIBUTES` (`gmc.hrl:178-208`) maps each bot version to a list of
`grid_attribute()` values it may not enter — TTP bots are barred from `storable`, `pps`,
`charger`, `elevator`, all conveyor attributes and both relay attributes; VTM bots add
`port_entry_io_point` and `port_exit_io_point`; `butler@vCleanZone` is barred from nothing.
From the *cell* side, `gridinfo.barcode_properties.bot_orientation_details` names permitted bot
versions with per-direction lift-height ranges and ground clearances. **Two mechanisms, no
stated precedence, and the bot-version keys in the first are atoms while the second uses
strings/binaries.**

**Protocols** — `ranger_comm_proto() :: ranger_tcp | vda5050 | vda5050_v2` (`gmc.hrl:1074`).
`ranger_tcp` for legacy RTP butlers; `vda5050` (MQTT) for RTP Quicktron; `vda5050_v2` for
TTP/HTM/VTM (`apps/mhs/CLAUDE.md:24-28`, `gmc_mqtt_agv_communicator.erl:44` —
`?VDA_V2_COMPATIBLE_VERSIONS` defaults to `[ttp, vtm, htm]`). The channel is recorded per-robot at
connect time in `bot_details_map_table` (`:109, :144`).

---

## 7. Glossary with receipts

Confidence: **H** = definition read directly in code/type; **M** = inferred from consistent usage;
**L** = inferred from naming plus one or two sites.

| Term | Meaning | Receipt | Conf |
|---|---|---|---|
| **Butler** | A mobile robot. Also the product name. Id `butler_id() :: integer()`. Live row `#butlerinfo{}`. | `gmc.hrl:387, 1420` | H |
| **Ranger** | Synonym for butler in newer code; `ranger_manager` orchestrates HTM/VTM, `ranger_profile` holds per-version capability. | `apps/mhs/CLAUDE.md:22`, `ranger_profile.erl` | H |
| **RTP** | Rack-to-person: bot lifts and carries a whole rack/MSU. Bot category. | `ranger_profile.erl:35`, `?RTP_BOTS` `gmc.hrl:55` | H |
| **TTP** | Tote-to-person: bot carries individual totes. | `?TTP_BOTS` `gmc.hrl:42`, `?TTP_GRID_ATTRIBUTES` `:8` | H |
| **HTM / VTM** | Horizontal / vertical tote mover — the two robot roles in the relay system. | `?HTM_BOTS` `gmc.hrl:69`, `?VTM_BOTS` `:92` | H |
| **MSU** | Mobile Storage Unit = the movable rack. Butler's table for it is `rackinfo`; MSU appears in task names (`msuinductiontaskrec`, `msu_reconfig`) and station types `msu_induction`/`msu_extraction`. | `gmc.hrl:249, 323, 687`; expansion spelled out in `apps/pick/test/ct/flows/pick_shuttle_base_SUITE.erl:10230` | H |
| **Rack** | Same physical thing as MSU, the name used in the data model. `rack_id() :: binary()`. Faces 0-3. | `gmc.hrl:659`, `src/gm_common.hrl:552`, `?VALID_RACK_FACES` `gmc.hrl:289` | H |
| **PPS** | Pick/Put Station — the human workstation. `pps_id() :: integer()`. | `gmc.hrl:738`, `src/gm_common.hrl:554`, `apps/mhs/CLAUDE.md:47` | H |
| **PPP** | A station variant; `ppsinfo.type` can be `ppp_manual`, and there are `ppptaskrec` dock/undock tasks. Exact expansion not stated in-repo. | `gmc.hrl:300, 1181`, `mhs_setup.erl:1555` | L |
| **Presentation point** | A specific spot at a station where an entity is presented. v3 name for `pps_point`. | `gmc_pps_api_handler.erl:401`, `src/gm_common.hrl:357` | H |
| **Seat** | Operator-facing side of a station (`front`/`back`); bins are assigned front/back seat names. | `/Users/daniel/vt/greyorange/warehouse_map/pps_seat.json`, `pick.hrl:187` (`front_seat_name`) | M |
| **Bin (pps_bin)** | A physical container slot at a station, with station-local mm coordinates and a grid `position`. | `pick.hrl:187`, `warehouse_map/pps_bin.json` | H |
| **PPTL** | Put-to-light: the light module bound to a station bin. | `src/gm_common.hrl:502, 513`, `warehouse_map/pptl.json` | H |
| **Slot** | A shelf compartment inside a rack type, with its own barcode(s) and tags. `slotref() :: string()`. | `src/gm_common.hrl:388`, `tote_type.json` `slot_structure` | H |
| **Relay** | The HTM↔VTM handoff scheme: a tote is staged at a relay point rather than carried end-to-end. Grid attributes `relay_storable`, `relay_storable_io_point`; `tote_storage.storage_type = relay`. | `gmc.hrl:1173-1174`, `tote_storage_functions.erl:47-57` | H |
| **Aisle** | A storage lane, `physical` (computed) or `virtual` (declared). Not the same as the `vertical_aisle` cell attribute. | `aisle.erl:54-67`, `aisle_manager_utils.erl:155` | H |
| **Zone** | Safety/e-stop domain. One per cell. | `gmc.hrl:877, 1033, 1050` | H |
| **Sector** | Storage-policy partition used as a hard constraint in put calculation. | `apps/put/src/sector_tier.erl:8-11`, `gmc.hrl:557` | H |
| **Block** | Named ordered cell set used as a waiting area or (sequenced) queue. | `block.erl:57`, `block_validation_functions.erl:25-31` | H |
| **Storable** | A cell where a rack may be parked (`store_status: 1` → `storage_info`). | `gridinfo_api_handler.erl:1025-1041`, `gmc.hrl:1006-1007` | H |
| **IO point** | The traversable cell adjacent to a storage location from which a bot accesses it. | `storage_info.io_point` `gmc.hrl:805`, `tote_storage_functions.erl:33-46` | H |
| **Barcode** | Physical floor label string; the human/robot-facing name of a location. Replaceable via `barcode_replacements`. | `src/gm_common.hrl:436`, `gmc.hrl:833` | H |
| **Size info** | Four mm distances from a cell centre to its N/E/S/W boundaries. `sizeinfo` declared, `actual_sizeinfo` measured. | `gmc.hrl:511`, `vda5050_map.erl:73-79`, `mapdet.erl:118` | H |
| **Location profile** | Named reusable bundle of `barcode_properties`, referenced per cell by `location_profile_id`. | `gridinfo_api_handler.erl:673-720` | H |
| **ODS** | Obstacle-detection-sensor exclusion for a (cell, heading) pair. | `gmc.hrl:522`, `butler_manager.erl:12347` | M |
| **Cstep** | A conveyor/coordinate step barcode in the HAI protocol path. | `gridinfo.erl:1385`, `vda5050_v2.erl:915` | L |
| **Wave** | Not present in butler_server's world model. No `wave` table, record, or type found. | — | H |
| **GMC** | The internal name for this map/fleet subsystem (GreyMatter Core?). Prefixes `gmc_map`, `gmc_maps`, `gmc_models`, `gmc_coord`, `gmc_world_coord`; the `mhs` app's CT alias is `gmc`. Expansion not spelled out in-repo. | `gmc_map.erl`, `README.md` CT commands | M |
| **MHS** | Material Handling System — the core fleet/layout app. | `apps/mhs/CLAUDE.md:1` | H |
| **MAOP** | A monitoring/operations platform Butler publishes map-deployed events to; has its own `maop_maps` repo module and can be disabled by `enable_maop_flow`. | `gmc_map.erl:229`, `maop_event_dispatcher.erl:400` | M |

---

## 8. Top 12 implications for the new format

Each item names what Butler needs, then whether it is domain truth or Butler accident.

**1. Locations need real ids, and barcodes must become an attribute, not a key.**
Butler's location has two identities — `{X,Y}` for tables and pathfinding, a barcode string for
tasks, stations, chargers and operators — plus a `barcode_replacements` remap table to change one
without the other, plus `?WARNING`-guarded coordinate↔barcode conversion functions that fire when
callers confuse them (`gridinfo.erl:700, 756`). *Essential:* a location has a durable identity and
carries zero or more physical labels. *Accident:* the identity being a coordinate tuple, and the
label doubling as a key.

**2. Coordinate frames must be declared objects with an explicit transform, not a magic reference
point.** The whole VDA5050 frame hangs off one `{barcode, vda5050_coordinate}` pair per floor and a
translate-plus-Y-flip (`gridinfo.erl:557-563`), with a fallback that searches for a cell at world
`{0,0}` and crashes if absent (`:602-615`). *Essential:* multiple frames coexist and vendors anchor
differently. *Accident:* one reference point, an implicit axis flip, and a crashing fallback.
Model frames as first-class entities with an explicit transform (translation + axis signs at
minimum), and let entities carry positions in a named frame.

**3. Cell pitch is not uniform and must not be assumed.** walmart-pot's grid indices are sparse
(29, 37, 44, 48, …) and `size_info` varies per cell and per direction (`[750,750,420,585]`).
Butler's `+1/-1` neighbour fallback and its modal-distance "average barcode distance" both assume
otherwise. *Essential:* per-cell, per-direction extents. *Accident:* the integer index pretending
to be a metric coordinate.

**4. Adjacency is an edge with properties; store it once.** Butler stores each edge twice — as a
`neighbours` triple on each endpoint plus an `adjacency` target — with no cross-check, and the
triple `[exists, bot_ok, bot_with_rack_ok]` encodes an unnamed capability lattice
(`vda5050_map.erl:163-165`). *Essential:* directed traversability that differs by what the robot is
carrying. *Accident:* the redundant double storage and the positional triple.

**5. Declared vs derived must be marked in the data, because Butler already has both and cannot
tell them apart.** `gridinfo.attribute` is a materialized union of eleven tables written back into
the source table (`gridinfo.erl:966-998, 1127-1167`); `mapdet.sizeinfo` (declared) and
`actual_sizeinfo` (robot-measured) are the same shape in the same row (`mapdet.erl:118`); virtual
aisles are auto-seeded from computed physical aisles and then diverge silently
(`aisle_manager_utils.erl:155-213`). *Essential:* both kinds exist and both must round-trip.
*Accident:* the conflation. Provenance should be a property the format carries.

**6. Per-robot-type permissions belong to one composable component, expressed once.** Today the
same question ("may this bot stand here, in this heading, at this lift height?") is answered by
`?DEFAULT_EXCLUDED_GRID_ATTRIBUTES` keyed by bot-version *atoms* (`gmc.hrl:178-208`) and by
`barcode_properties.bot_orientation_details` keyed by bot-version *strings* (`gmc.hrl:502`), with
no stated precedence and an "empty means permissive, absent-key means permissive, present-but-empty
means forbidden" convention documented only in a comment (`gmc.hrl:462-501`). *Essential:* the
capability question. *Accident:* everything about how it is currently spelled.

**7. Named reusable property bundles (prototypes) must be first-class.** `location_profiles` is a
real, load-bearing instancing mechanism in the newest exports — 21 profiles covering 1296 of 1424
walmart-pot cells — but it exists only for `barcode_properties`, its merge rules are 95 lines of
ad-hoc case analysis (`gridinfo_api_handler.erl:767-862`), and the profile name is the only place
the semantics are written down (`"S+HCA(HTM_crossaisle_under_static_racks)"`). *Essential:*
prototypes/stamping. *Accident:* the one-component scope and the semantics-in-a-string.

**8. Cell "attribute" is a single enum doing the work of a component set — split it.**
`grid_attribute()` has 31 members mixing role (`highway`, `storable`), ownership
(`pps`, `charger`, `elevator`), subsystem (`conveyor_track`, `relay_storable`) and relation
(`ttp_storable_io_point`, `port_entry_io_point`) (`gmc.hrl:1143-1174`). `composite_attribute_list`
exists as a field but is only ever set to `[Attribute]` (`gridinfo.erl:1141`) — the codebase
already reached for multiplicity and stopped. *Essential:* cells play several roles at once.
*Accident:* the flat enum and the vestigial list.

**9. Cell sets need a typed relation, not four ad-hoc mechanisms.** Zone (safety), sector
(inventory policy), block (queue/staging), aisle (storage lane) are four different table shapes for
"a named set of cells", and a fifth (`vertical_aisle`/`horizontal_aisle`/`aisle_cross_point`) hides
inside the attribute enum. Membership is stored on the cell for zone and sector, on the set for
block and aisle — opposite directions. *Essential:* all four gates are real and distinct.
*Accident:* four representations and two membership directions.

**10. Station internals need their own coordinate frame and a real containment hierarchy.**
`pps_bin.coordinate` is millimetres in station-local space, `pps_bin.position` is `[row, col]`,
`pps_seat` names a side, `pptl` binds a light to a bin — and the newer `pps.json` v3 uses the key
`world_coordinates` for a *grid* coordinate (`gmc_pps_api_handler.erl:443`). Meanwhile
`presented_entity_allowed_types` and `presenting_agent_type` are collapsed into two 4-value enums
with silent fall-through defaults (`:414-437`). *Essential:* a station is a composite with nested
frames and typed attachment points. *Accident:* the naming collision and the lossy enum collapse.

**11. Change must be incremental, addressable and non-destructive — Butler's is none of those.**
Upload wipes the world, rebuilds it, snapshots to PostgreSQL, restores a whole-Mnesia backup on any
failure, and closes with `"Need butler_server restart"`
(`gridinfo_api_handler.erl:864-1023`). The only partial-update path in existence is
barcode replacement (`vda5050.erl:1407`). Versions are a `Major.Minor.Revision` string chain with a
single `parent_version` (`gmc_map.erl:69-96`). *Essential:* versioned, activatable map snapshots
with a parent chain and a deploy event. *Accident:* whole-map replacement as the only granularity.

**12. Keep Butler's validators; they encode real invariants.** Rectilinear alignment across
neighbours with an explicit exemption at TTP/non-TTP stitching points
(`grid_sanity_check_functions.erl:40-90`); storable-must-have-an-io-point-with-≥750mm-clearance
(`storage_info.erl:163-183`); unique block coordinates across blocks
(`block_validation_functions.erl:148`); sequenced-queue sync/exit points must be members of their own
block (`:98-110`); sector preference lists must reference existing sectors and contain no duplicates
(`json_validation_rules.erl:6352-6371`). These are domain truths, and a format that can express
them declaratively is strictly better than one that leaves them to a validator.

### Things Butler needs that the export does not currently give it, and vice versa

- **Emitted but never read by butler_server:** per-cell `vda_world_coordinate` and
  `world_coordinate_reference_neighbour` (zero source references outside test fixtures);
  `sectors.json` (GET-only projection of `gridinfo.sector`); `aisle_info[].additional_information`
  (parsed then discarded, `gmc_aisle_http_controller.erl:976`); `grid_attribute: "relay_store"`
  (not a member of `grid_attribute()`, overwritten by the recompute);
  `conveyor_mapping.json` and `sectorMxUPreferences.json` (no reader found).
- **Read but not in the export:** nothing missing was found for walmart-pot; note however that
  `map.json` has no `blocked`-reason detail, no per-cell `speed_zone` on input (only on the GET
  side, `gridinfo_api_handler.erl:1070-1079`), and no floor-to-floor topology — floors are only
  connected through `elevatorinfo.entry_barcodes`/`exit_barcodes`.
- **Not in butler_server at all:** Map Creator itself, and whatever produces these JSONs, is a
  different repository. The `pps_bin`/`pps_seat`/`pptl` legacy files target handlers in
  `apps/pick` / `apps/gmr_common`; the `pps_seat` record shape survives only inside migration
  modules (`src/migrations/af17766f4_migration.erl:9`), so its current live definition could not be
  confirmed from a shared header.
