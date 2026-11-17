# Where each Map Creator construct lands, and why

One row per construct of the Map Creator export. This is the table to argue with:
it says what happens to what, and the third verdict column is the one that asks
for a ruling.

The four verdicts:

- **core as is** — a component the protocol already defines carries it, used
  exactly as SPEC §10 fixes it.
- **core, merged** — a fact Map Creator states about a thing the protocol already
  has a component for, carried as one of that component's optional fields, which
  the protocol's own schema (`proto/schema/components/`) and SPEC §10 state. No
  field the core defined changes its meaning. Two components take these: `cell`
  and `presentation_point` (see "Same-named components" below).
- **vocabulary** — a component of the adapter's vocabulary carries it, with its
  meaning and JSON Schema in its file under `vocabulary/`: a shared, undotted,
  ratified one, or one of the three still under Map Creator's own name at rung
  `experimental` — the export's table of contents, and the frame and station
  facts whose merge is not obvious. This is where a fact the core cannot say
  goes, so that the round trip is exact while the gap stays visible.
- **conform the simulator instead** — a *proposal*: the core ought to carry this
  and the simulator ought to read it that way. Nothing of the simulator is touched
  by this adapter; these rows are the ones to rule on.

Beside all four, how the file wrote a record — its place among the file's
records, its key order, a whole number it wrote `40.0`, the JSON kind it spelled
a key in, a field it left out — is no fact about the site, and rides in the
record's entry of `import_formatting` rather than in any of these; such a row
says **formatting**. That component is shared with other importers, keyed by the
lens a record is written back through, and the ingest rewrites only this adapter's
entries in it, as it rewrites only its own `external_ids` keys: another importer's
entry stays as it was.

"Simulator agrees?" reads the other way round from what the column name suggests.
The simulator's own reading of a world document is its `sim-model` library, and it is
narrow on purpose: it reads `machine`, `easyloop` (still `go.easyloop` in the
simulator's own documents), `queue`, `port`, `pick`,
`waiting`, `pose` and `cell` and nothing else. So for most rows the honest answer
is **not yet read** — the simulator neither agrees nor disagrees, because it never
looks. Where it does look, the row says whether what it looks for is what this
crate writes.

## Same-named components: merged, or kept apart

Five of Map Creator's components shared a name with a component the protocol
defines: `cell`, `frame`, `presentation_point`, `sector` and `station`. Ruled on
2026-10-08: converge each where the two are the same thing, and keep one whose
merge is not obvious under Map Creator's name. The test of obvious is the
protocol's own text: a merge that would contradict it is not obvious. A sixth,
`mapcreator.export`, shares no name and keeps its prefix for a reason of its
own, given below the table.

| component | outcome | why |
| --- | --- | --- |
| `cell` | merged | Every fact Map Creator states of a cell describes the cell, and none collides with a field the core defines. The four half-extents (`size_info`) stay beside `size` because `size` is their per-axis sum, a derived form that loses where a footprint off the cell's centre lies; the adjacency and its grades stay beside the stitches for the same reason — SPEC §4 keeps a recorded value beside the nominal one, which `legacy_grid_coordinate` already does. |
| `presentation_point` | merged | Map Creator's facts describe the point, and its `station` is the field the shipped `presentation_cell` lenses already read, which another site's points state too. `attributes.cell` is the cell the producer named, which agrees with the point's pose; whether a point binds a serving cell explicitly stays open (SPEC §12.11). |
| `sector` | nothing left | `mapcreator.sector` held the key's JSON kind and its place in the sector index, both how the files write a sector, which `import_formatting` now carries; a sector is its core `sector` alone. |
| `frame` | kept as `mapcreator.frame` | Its `role` names four kinds of frame — the site's, the floor's, the legacy grid's and the robot vendor's — and SPEC §4 says a vendor frame "is not a kind of frame — just a frame so tagged". A role on `frame` would give the protocol the frame taxonomy it refuses. |
| `station` | kept as `mapcreator.station` | SPEC §10 and the protocol's schema require a station's one `kind`, and 24 stations across three production sites state several modes and no type that settles one. Their Map Creator facts would have to ride on a `station` with no kind, which the schema and the reference reader refuse, or with a kind the site does not have. A ruling that makes `kind` optional would make this merge mechanical. Until then `mapcreator.station.role` repeats `station.role` where a kind is derivable. |

**`mapcreator.export` keeps its prefix, though no component is named `export`**
(ruled 2026-10-18). A prefix is for a collision: Daniel, that day, "most of the
time, we shouldn't even have a prefix/namespace btw. that's only there in case
there are naming collisions" (`server/DESIGN.md` §7, the amendment of that day;
SPEC §2). The core, this vocabulary, the seed and every example hold no
`export`, so the name collides with nothing that exists. It collides with the
word's next claimant instead. The component is one file format's own table of
contents — which of the nineteen files a site shipped, which of them were empty
and in which form, the envelopes around them, and how each file's text was
written — in fields only Map Creator has, and a registry name means one thing
org-wide. A plain `export` would claim a generic word that the next format
shipped as several files needs for a table of contents of its own, with other
fields and another meaning; and the server already says "export" for a scope's
whole document (`server/DESIGN.md` §9). So the name stays under Map Creator's
domain, and the plain word stays free for whatever meaning the org agrees on.

## The spine

| construct | lands | verdict | simulator agrees? |
| --- | --- | --- | --- |
| a cell's millimetre position | `pose` in the floor frame, in steps of the primary frame | core as is | yes — `sim-model` resolves a cell by its pose |
| a cell's footprint | `cell.size`, the two half-extents on each axis summed | core as is | yes |
| a cell's four half-extents | `cell.size_info`, whole | core, merged | not yet read. The core keeps only the sum, which is a *derived* form, so the original stays beside it and the export writes from the original (spec §4). A footprint that is not centred on its cell is a real fact about a surveyed floor and the sum loses it. |
| a cell's traversability | `cell.traversable_by`, the roles a profile permits | core as is | yes |
| a cell's `blocked` flag | `cell.blocked` | core as is | not yet read |
| a cell's grid coordinate | `external_ids.mapcreator_grid_coordinate` | core as is | n/a — data, never a reference target |
| a cell's barcode | `external_ids.barcode` | core as is | n/a |
| a cell's vendor coordinate | `external_ids.vda_world_coordinate`, beside a declared vendor frame that reproduces most of them | core as is | n/a. One that is not two finite numbers in range rides back as written and is no vote in the frame's fit; a site with no usable one declares no vendor frame, and a ledger note says so. |
| a traversable adjacency | one `stitch` entity per pair both ends permit travel across | core as is | yes — the pipeline compiles these into arcs |
| the full adjacency array and its grades | `cell.adjacency` and `.neighbours`, whole | core, merged | not yet read. The stitches are the derived form: a link is bidirectional and ungraded, so it cannot state a one-way lane, an impassable neighbour pair, or an edge a robot may cross empty and not loaded. 8,728 adjacencies across the five sites are one of those three. |
| a cell's rack-parking flag (`store_status`) | `cell.store_status` | **conform the simulator instead** | The square a movable rack may be parked on. The core has racks that are built structure and none that get carried, so `a rack may stand here` has no component. This is the RTP generation's whole storage model and it should be a core component, not a vendor one. |
| a cell's re-initialization flag (`special`) | `cell.special` | **conform the simulator instead** | A square where a robot re-establishes its position, which every charger points at by name. A real place with a real purpose; a marker component (`reinit`, as `parking` and `highway` are markers) would carry it in two lines. |
| a cell's classification (`grid_attribute`, `composite_attribute_list`) | `cell.grid_attribute` and `.composite_attribute_list` | core, merged | not yet read. The orchestrator recomputes its own attribute from eleven other tables and overwrites whatever the export said, so this is a field the export should probably stop writing rather than one the protocol should learn. |
| a cell's zone | a reference to the `zone` entity, on `cell.zone` | core, merged | not yet read |
| a cell's location profile | a reference to the `location_profile` entity, on `cell.location_profile` | core, merged | not yet read. **One fact, one home, and the reference is the home**: the legacy `location_profile_id` is not copied onto the cell, and the export rebuilds it from the profile entity's own key — the same rule the zone follows, and what keeps a display string out of reference position (SPEC §2). A cell naming a profile the dump does not declare keeps the string under `location_profile_unresolved` and no reference; no cell of these five sites does. |
| a cell's sector | membership of a `sector` entity | core as is | not yet read |
| a cell's block | membership of the block entity's `queue.path` | core as is | not yet read |
| a cell's `floor` | `cell.floor` | **conform the simulator instead** | A second statement of the file's own `floor_id`, present on 1,198 of one site's 5,055 cells and absent on the rest. Carried only because its *presence* is per-cell, and every value on every site equals the file's `floor_id`; a cell stating another floor is refused, naming the cell and both floors, since a cell of another floor has no place in this file. The right answer is for Map Creator to stop writing it. |
| a cell's `dockPoint` | rebuilt from the dock point entity that names the cell | core as is | n/a — one fact, one home. The flagged cells and `dock_point.json`'s rows agree exactly on every site. |
| a cell's `barcode_properties`, `hasStorables`, `conveyor_direction_attribute`, `world_coordinate_reference_neighbour` | `cell.*`, each as stated | core, merged | not yet read |
| a cell's `botid` and `block_reasons` | `cell.runtime.*` | core, merged | not yet read. Live state. SPEC §1 separates a world from what is happening in it, so this is carried to be dropped by a policy that decides to rather than lost by a converter that forgot (spec §4). |
| the floor | a declared `frame`, plus `mapcreator.frame` for the role and the floor id | core as is | yes |
| the legacy grid | a declared `frame`, `integer_only`, fitted by least squares, for projection only | core as is | n/a |
| the robot vendor's frame | a declared `frame` with the recovered transform | core as is | n/a |
| `vda5050_reference` | `mapcreator.frame.vda5050_reference_barcode` on the floor frame; the coordinate half is rebuilt from the cell | vocabulary | n/a. SPEC §4 names a sample point offered in place of a transform as the anti-pattern; the transform is declared and the sample is kept so the round trip is exact. |
| a location profile | one entity with `location_profile`, the permission table whole | **conform the simulator instead** | The richest thing in the export: per bot version and per heading, which lift heights are legal loaded and unloaded and how much ground clearance there is. `cell.traversable_by` is a flat list of classes and cannot say that one model of a role is permitted where another is not — 474 of one site's cells are exactly that case. This is the format's own physical-capability split and the core should carry it. |

## The groupings

| construct | lands | verdict | simulator agrees? |
| --- | --- | --- | --- |
| a zone | one entity with `zone` | **conform the simulator instead** | A zone is an e-stop and light-curtain domain: the orchestrator pauses and blocks whole zones on a physical safety event and computes a buffer of cells within 1,700 mm of one for pre-emptive stopping. IDIOMS reserves the name `safety_domain`; until that exists this is an entity a core consumer learns nothing from. |
| a zone's `blocked` and `paused` | `zone.runtime.*` | vocabulary | n/a — live state, carried as above |
| a sector | one `sector` entity per key, with the cells as explicit members | core as is | not yet read |
| the sector key's spelling | `import_formatting.mapcreator_sector.key`, beside the sector's place in the index | formatting | n/a. A numbered sector is an integer in the cell list and a string key in the sector index, and `undefined` is a string in both: how the files spell the key, which `external_ids.mapcreator_sector` holds as text. |
| `sectors.json` / `sector.json` | nothing: rebuilt from the sector entities' members in cell order | core as is | n/a — one fact, one home. Verified on all five sites. |
| an aisle | one `sector` entity, its io-point cells as members | core as is | not yet read. `vtm_zone.json` states no extent, no rack and no direction, so a shapeless named grouping is all there is to import. |
| a queue block | `queue` with the block's cells as its path | core as is | yes — `sim-model` reads `queue.path` |
| a waiting block | `waiting`, with the cells on `block.cells` | vocabulary | partly. `waiting` is what the simulator reads; SPEC §10 makes a waiting area's capacity its cell count and gives it no way to name its cells, which is fine for a tiled region and not for a surveyed floor. |
| a sequenced queue's sync and exit steps | `block.extra_details` | **conform the simulator instead** | The orchestrator validates that both are members of the block. A core queue has a path and a discipline and no way to tell one step from another; `queue.sync` and `queue.exit` as cell addresses would carry it. |
| a conveyor touch-point block | `block` with its cells | vocabulary | not yet read |

## Stations

| construct | lands | verdict | simulator agrees? |
| --- | --- | --- | --- |
| a station | `station` with a kind and a role where the kind is derivable, `pose` at the cell it names, and `mapcreator.station` | core as is | partly — `sim-model` reads `machine` and `easyloop`, not `station`. See the question in the report. |
| a station with several modes | a `station` of kind `induct` or `extract` where its type names induction or extraction, which decides the kind whatever the modes; otherwise no `station` component at all, and the type and modes on `mapcreator.station` | vocabulary | 24 stations across three sites. SPEC §10 gives a station one kind, and choosing one of pick/put would invent a restriction the site does not have. |
| the export dialect (`version`) and the spelling of `pps_id` | `import_formatting.mapcreator_station`: `dialect`, verbatim, `dialect_shape` beside it, and `pps_id` in the JSON kind the record states it | formatting | n/a. How the file wrote the record: `dialect` is the record's own `version` value, carried as written; `dialect_shape` is which of `pps.json`'s two record shapes the import read the record by — `v3` where it states `entities`, `v2` otherwise — which decides the spelling of `location`, of the point list, and of `pps_id` itself, so the export cannot write a record back without it; and `pps_id` is the station's key, which `external_ids.mapcreator_pps_id` holds as text, in the JSON kind its dialect spells it in. |
| which queues serve a station | `mapcreator.station.served_by`, by identity | **conform the simulator instead** | The core's `station` component names nothing. A station that cannot say which queues feed it is a station a scheduler has to guess about, and the guess today is proximity. |
| `blocks` stated twice per station | `mapcreator.station.blocks` and `.served_by`, both | vocabulary | n/a. Not redundant, which the one-way importer's table had wrong: the two lists differ in *order* on three stations of two sites, and the top-level one is absent on two more. |
| a station's associations | `mapcreator.station.associations`, whole, handles and all | **conform the simulator instead** | The one place the export reifies a relationship: a typed connected graph over station-local handles saying which transports between a station's parts are allowed. SPEC §2 wants exactly this shape with entity refs for endpoints. |
| a station's docking bays | `mapcreator.station.docking_stations`, whole | vocabulary | Sub-objects rather than entities, because nothing in the export gives a bay a place on the floor. A ruling that a bay is an entity would move them. |
| a v3 presentation point | `presentation_point` with `for` and Map Creator's fields beside it, and `pose` at its cell | core, merged | yes — the simulator reads presentation points by pose |
| a presentation point's geometry | `presentation_point.attributes` | **conform the simulator instead** | The height a carrier is presented at, the robot class that presents it, the heading it presents at, the direction the operator reaches from, and whether this is where the operator works. The core's `presentation_point` now states these as Map Creator's optional fields; the simulator computes every one of them itself today. |
| a v2 station point | `presentation_point` (empty) plus `station_point` | vocabulary | not yet read |
| a v2 station's queue (`queue_barcodes`) | `mapcreator.station.queue_cells`, in file order | vocabulary | 30 stations. A core queue path is ordered and the order *is* the queue, so importing an unordered list as one would invent it. The file order is carried and is not claimed to be a queue order. |
| a station's interior — bins, seats, put-to-light | one entity each with `pps_bin`, `.pps_seat`, `.pptl`, `.pps_bin_type` | **conform the simulator instead** | A station-local frame and a containment hierarchy, which is exactly what SPEC §6 templates are for. Two of the four files are checked-in HTTP request envelopes rather than models, which the export entity carries so the files write back. |

## The furniture and the storage model

| construct | lands | verdict | simulator agrees? |
| --- | --- | --- | --- |
| a charger | one entity with `charger`, its three cells by identity | **conform the simulator instead** | SPEC §11 puts charging *behaviour* outside v0 world data, and that is right — but the *fixture* is built structure standing on the floor, and the protocol cannot say it is there. |
| a docking point | one entity with `dock_point` | vocabulary | not yet read |
| an obstacle-detection exclusion | one entity with `ods_exclusion`: the cell by identity and the heading | vocabulary | not yet read. IDIOMS reserves the mapping to a heading-aware policy component. |
| a conveyor | one entity with `conveyor`; the steps and the ports whole | **conform the simulator instead** | SPEC §11 keeps conveyor mechanics outside world data and §10 has a `port` component for where things enter and leave the world, but nothing joins a port to a conveyor's topology. Every step's coordinate names a real cell on the `x == 999` escape hatch and its world coordinate equals that cell's, so the steps are a second reading of cells the document already declares. |
| a conveyor mapping row | one entity with `conveyor_mapping` | vocabulary | not yet read. The vendor half is `UNDEFINED` in every dump. |
| a storage position | **one entity each** with `tote_storage` | **conform the simulator instead** | 55,721 across three sites, and the cost is in the README. A core `slot` exists only as something a rack derives, and no export declares a rack — no extent, no bay boundaries, no columns. The natural key the rows already have, (access cell, heading, direction, depth, height), is unique across all 55,721, so the identity is there waiting for a rack to hang it on. |
| a rack and carrier geometry template | one entity with `rack_type`, the shelf tree whole | vocabulary | Nothing in the export instantiates one at a place, so the tree has no parts to be entities of. The units differ inside one dump: this file is centimetres and `tote_storage.height` is millimetres, describing the same racking. |
| a sector preference | one entity with `sector_preference` | vocabulary | SPEC §7 keeps a policy in an overlay document rather than in world data, so this rides here rather than being made a core component. One site prefers sectors for a rack type its own `tote_type.json` does not declare. |
| the five empty-but-named files | `mapcreator.export.files` records their presence and their empty form | vocabulary | `elevator`, `zone_controller`, `fire_emergency`, `opc_connection`, `queue_data`. Their names are the only evidence of intent, and multi-floor especially has to be first-class in the frame model from the start. |
| the file inventory itself | one entity with `mapcreator.export` | vocabulary | Nothing else can carry it. A file the site never shipped and a file it shipped empty are two different facts, and `[]` and `{}` are two more. |
| how each file's text is written: its objects' key order, its spacing, a whole number written `40.0`, a zero written `-0.0` | the usual key order of each place and set of keys, each file's spacing, and what lies outside every record, on `mapcreator.export` (`key_orders`, `spacing`, `key_order`, `written_with_point`, `written_negative_zero`); a record's own key order where it departs, its whole numbers written with a decimal point and its zeros written with a minus sign, in its entry of `import_formatting` | formatting | n/a. JSON gives none of these a meaning and a parse forgets them; the round trip is byte for byte, so each is recorded, on the record it is about where there is one, and apart from what the record states about the site. |

## The lookup keys

§4 of the spec makes identity a lookup, never a hash: the adapter is handed the
site as it stands, indexes the `external_ids` its entities carry, and for each
record works out that record's **lookup key**. Exactly one entity of the site
carrying that key means the record IS that entity and keeps its identity; none
means a new identity, which the host issues; more than one is refused by name.

The consequence is the same for every row and worth stating once: **a record whose
key Map Creator edits resolves to nothing and becomes a new entity**, and the
entity it was is deleted by the same ingest, since no record of the export claims
it any more. So the key has to be the thing about a record that survives the edits
Map Creator users actually make.

| construct | key | what an edit costs | a secondary key that would survive it |
| --- | --- | --- | --- |
| cell | `mapcreator_floor_id` + `mapcreator_grid_coordinate` | Re-gridding a cell makes it a new entity. This is butler's own cell identity — `{floor, X, Y}` is *the* key over there — so it is the one a site's own tooling agrees with. | **The barcode.** A grid coordinate changes when a surveyor renumbers; a barcode changes when the sticker peels. Both happen. Proposed: try the grid coordinate, then the barcode, and refuse if the two disagree about which entity this is — because that disagreement is a real question about the site, not a tie to break. |
| floor frame | `mapcreator_floor` | Nothing: one export is one floor and the id is stated once at the top of `map.json`. | — |
| site, legacy-grid and vendor frames | `mapcreator_frame` (a fixed role string) | Nothing: the roles are this adapter's own and no site states them. | — |
| station | `mapcreator_pps_id` | Renumbering a station makes it a new entity. A `pps_id` is a configured number, not a measured one, so it is about as stable as a name. | **The cell it stands at.** A station moves rarely and is renumbered occasionally; proposed as the secondary key, refusing on disagreement as above. |
| presentation point | `mapcreator_of_pps` + `mapcreator_point_id` | Moving a point to another station makes it a new entity, which is arguably right: a point of station 3 is not a point of station 4. | — |
| v2 station point | `mapcreator_of_pps` + `mapcreator_position_id` | as the presentation point | — |
| zone | `mapcreator_zone_id` | Renaming a zone makes it a new entity, and the names are fragile: one site's largest zone name carries a typo and a trailing space, and it was the join key for 925 cells. | **None available.** A zone has nothing else about it in the export: no extent, no devices, no cells of its own (the cells name it, not the other way round). This is the weakest key in the table and the one to rule on. |
| sector | `mapcreator_sector` | Renumbering a sector makes it a new entity. | **Its member cells**, as a set. A sector IS its members in this format, so an unchanged member set is strong evidence of the same sector. Proposed, not built. |
| aisle | `mapcreator_aisle_id` | Renumbering an aisle makes it a new entity. | **Its io-point cells**, as an ordered list, for the same reason. |
| block | `mapcreator_block_id` | Renumbering a block makes it a new entity. | **Its cells**, in order. |
| location profile | `mapcreator_location_profile_id` | Renaming a profile makes it a new entity, and every cell that named it re-resolves to the new one. The names are short and hand-typed. | **None available**, and none needed: a profile is a bundle of settings with no place in the world, so a renamed one costs one delete, one declare, and a patch of each cell that names it. |
| robot class | `mapcreator_bot_version` (the bot-type string; `butler_bot_version` carries the same string beside it as plain data) | Renaming a bot type makes it a new class. The string is the vendor's product code and does not drift. | — |
| stitch | `mapcreator_stitch` (the two grid coordinates, lower first) | Re-gridding either end makes it a new stitch. A stitch is not a Map Creator record at all — it is the protocol's reading of an adjacency — so the pair of coordinates is the only thing about it the export states. | **The two cell identities.** Once both cells are resolved the pair of UUIDs is the honest key, and it survives re-gridding exactly as well as the cells do. Proposed; the coordinate key is what is built, because it is computable before the cells resolve. |
| charger | `mapcreator_charger_id` | Renumbering a charger makes it a new entity. | **The cell it stands at**, which is a fixture bolted to the floor. |
| dock point | `mapcreator_dock_point_id` | The id IS the barcode of the cell it stands on, so a re-stickered cell makes it a new entity. | **The cell it stands at.** |
| obstacle-detection exclusion | `mapcreator_ods_tuple` | A re-stickered cell makes it a new exclusion, and 24 of one site's 180 already name a barcode with no cell. | **The cell and the heading**, which the component already carries. Proposed as the primary key; the tuple is what is built, because it is what the file states and 24 of them resolve to no cell at all. |
| storage position | `mapcreator_tote_location` (the `TLOC_…` id) | An allocation-counter id. Nothing says it is stable across exports, and this is the row most likely to churn a site's whole storage model into new entities. | **(access cell, heading, direction, depth, height)** — unique across all 55,721 rows of all three sites that have any, and a statement about where the position physically is rather than about when it was allocated. Proposed, and the one this table would most like ruled. |
| rack type | `mapcreator_racktype_id` | Renaming a type makes it a new entity. | — |
| conveyor | `mapcreator_conveyor_id` | Renumbering a conveyor makes it a new entity. | **Its step chain**, as an ordered list of cells. |
| conveyor mapping row | `mapcreator_of_conveyor` + `mapcreator_step_id` (the conveyor id and the step id, two values rather than one string joined from them) | as the conveyor | **The cell it joins.** |
| sector preference | `mapcreator_preferred_for` (the carrier type) | Renaming a carrier type makes it a new entity. | — |
| put-wall bin | `mapcreator_of_pps` + `mapcreator_bin_id` | Renumbering a bin makes it a new entity, and so does moving it to another station, in the files or by re-pointing its `station`: the station is a reference as well as half the key, and the file takes the reference. | **Its place in the wall** (`wall_position`), which is where the bin physically is. |
| bin type | `mapcreator_bin_type_id` | Renaming a type makes it a new entity. | — |
| operator seat | `mapcreator_of_pps` + `mapcreator_seat_name` | Renaming a seat makes it a new entity, and the bins that referenced it by name re-resolve; moving it to another station does too, as for a bin. | — |
| put-to-light module | `mapcreator_peripheral_id` | Renumbering a module makes it a new entity. | **The bin it lights**, one to one. |
| the export header | `mapcreator_export`, always `site` | Nothing: a site holds one Map Creator export, so the key is the same on every site. The crate this adapter was ported from keyed it by the export folder's name, which an adapter is never handed. | **None needed.** |

Two structural rules hold the key table together, and both are asserted rather
than remembered:

- **No key shape's field set is a subset of another's.** An entity is matched
  against every shape, so a bin carrying `mapcreator_pps_id` would answer for its
  own station as well. That is why a part names its station under
  `mapcreator_of_pps` and a conveyor mapping row names its conveyor under
  `mapcreator_of_conveyor`. `identity::tests::no_shape_contains_another` asserts
  the first half. The second is what the second round-trip law asserts on every
  export: a site whose own records collide on a key is one the next ingest
  refuses, and an entity carrying the keys of two kinds is refused by both
  directions. The crate's check of it is what found the conveyor collision: every
  conveyor-mapping row carried the conveyor's own key.
- **A lookup key never becomes a reference target.** SPEC §2 keeps a legacy
  identifier out of reference position, so `external_ids` holds it as data and the
  site references by entity identity. The export writes a reference back as the
  named record's own key, which is what `records.rs`'s `Cell`, `Cells`, `Join`
  and `Blocks` slots do; a reference that names nothing, or is not spelled as the
  export would write it back, is kept as the file wrote it, under the field's
  `_unresolved` name or at its own place in a list whose other entries resolved.
  The bodies below are the exception.

## Kept bodies

Some collections of the export ride whole, as the file states them, inside one
component. Their handles name cells by legacy coordinate and stations' parts by
station-local names, and nothing resolves them to entities, so another tool can
read them as JSON and cannot follow them by identity. Each is listed with the
number of field paths the coverage tables find inside it.

| body | where it rides | field paths inside |
| --- | --- | --- |
| a conveyor's steps | `conveyor.steps` | 4, naming cells by legacy coordinate |
| a conveyor's entry ports | `conveyor.entry_points` | 6, heights in millimetres |
| a conveyor's exit ports | `conveyor.exit_points` | 6, heights in millimetres |
| the conveyors a conveyor connects to | `conveyor.connected_conveyors` | none: empty on every export seen |
| a station's associations | `mapcreator.station.associations` | 6, endpoints named by station-local handles |
| a station's docking bays | `mapcreator.station.docking_stations` | 6 |
| a station's put docking positions | `mapcreator.station.put_docking_positions` | none: empty on every export seen |

A conveyor-mapping row names its step by the step's id, which is half of the
row's lookup key and a handle into the conveyor's kept steps, not a reference.
And a dock point's `station_id` names a docking bay by the bay's own handle
inside a station's kept `docking_stations`, which stays a plain string on
`dock_point.docking_station_id`: a bay is no entity, so there is no
identity to reference.
