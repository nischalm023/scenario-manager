# Porting dessim onto world-protocol documents

> Purpose of this document: state where the simulator's map loading is being
> cut, what the new loader builds versus what it hands to code that already
> exists, and every place the simulator's behaviour changes because the new
> model says something different from the old one. It is written for whoever
> reviews the port and for whoever picks up the next milestone; the rulings it
> records are the ones the port is built on, not proposals.
>
> It is current as of **SPEC v0.6**. v0.6 changed four things this port had to
> be rebuilt around — a rack's vertical truth became one enumerated `levels`
> list, a relay binds a cell of the floor instead of an access minting one, the
> access slimmed to a single kind that derives nothing, and every length became
> an integer count of a frame's steps. Where an earlier round of this document
> recorded a workaround for something v0.6 now states outright, the workaround
> is retracted here rather than left standing beside its replacement.
>
> Ruling this whole document sits under: **there is no backward compatibility.**
> `WarehouseDesign` loading is replaced, not dual-tracked. The old fixture files
> stay in the tree because `tools/desmap2proto` reads them; dessim stops reading
> them directly.

## 1. Where the seam is

The old path, from a file to a running simulation:

```
SimSpec JSON ──serde──► WarehouseDesign ──compile()──► WarehouseMap ──► scenario_gen ──► ScenarioData ──► planner ──► solver
                        (desmap authoring)             (desmap compiled)
```

`WarehouseDesign::compile()` is the only thing being removed from the path.
Everything to its right stays:

```
world document ──worldproto::load──► World ──dessim::worldmap──► WarehouseMap ──► scenario_gen ──► ScenarioData ──► planner ──► solver
                (expanded entities,           (this port)         (unchanged from here on)
                 cells, edges, frames)
```

**`WarehouseMap` plus a `Racking` is the seam.** The map carries cells, arcs,
relays, zones and stations; the racking carries how the storage is laid out.
Scenario generation used to read the second back out of the first — a rack's
physical row from cell grouping, its aisle side from column alignment, its shelf
heights from a zone default. A world document states all of it, so
`scenario_gen::topology::racking_from_map` is now the *desmap* producer of that
value and `worldmap` is the protocol's; `solver::resolve_scenario_with_racking`
is the entry that takes one.

`WarehouseMap` stays as dessim's internal map representation and is now built
from an expanded world instead of from a compiled design. That is the adapter
the no-backward-compatibility ruling allows: one loader, not two, with dessim's
dense-index types (`CellId`, `CellKind`, `ZoneId`, `RelayId`, `StationId`) kept
as *indices the adapter mints*, and the mapping back to protocol identities kept
beside them.

Why `WarehouseMap` and not something further in: twenty files in dessim take a
`&WarehouseMap`, and every one of them wants cells, edges, kinds, relays, zones
and stations — the same nouns a world document states. Cutting deeper would mean
rewriting the pathfinder's dense cell arrays, the reservation table, the
parking registry and the VTM cycle model in one change, none of which the first
milestone needs.

### What the adapter builds, and what it hands over

| Thing | Where it comes from now |
|---|---|
| cells (centres, footprints) | `World.cells` — the protocol's own geometry, already resolved into the primary frame |
| arcs between cells | `World.edges` — flow arcs and stitch links |
| `CellId` | **minted by the adapter**, dense from 0, in lattice row-major order; the UUID is kept beside it |
| `GridPos` | **minted by the adapter** from a lattice derived from the cells' own centres (§4) |
| `CellKind` | read off the components a cell carries (§3.2) |
| rack cells | **minted by the adapter** as storage/reporting anchors (§3.3); VTM coordinates create no cells (§3.4) |
| `RelayPosition` | one per bay per bound access cell; its three cells resolved from that binding (§3.5) |
| `Zone` | one per region an access is cut along (§3.6) |
| `Station` | one per `station` entity, with the cells it presents at and the lane into each, when one ends there (§3.7) |
| `FleetConfig`, VTM counts | cohorts and their hard assignments (§3.8) |
| storage layout (`Racking`) | one dessim rack per protocol `slot`, with its own shelf height, its own place along the run, and its aisle side (§3.6) |
| millimetres | **converted once**, at this seam, from the primary frame's `unit_mm` (§3.9) |
| scenario, settings | **not carried by a world document** — supplied by the caller (§6) |

## 2. What the protocol says that desmap could not, and the port now uses

- Slots are entities. A bay's storage levels × `slots_wide` × `slots_deep` slot
  entities are exactly dessim's storage spaces on that bay, so the vertical
  capacity is read off the steel instead of off a zone field whose own
  documentation admits it "may be inflated as a hack for inventory capacity".
- Relays are entities, and each one is bound to a cell. A relay is one handoff
  shelf on one bay, and the pipeline itself says which cell of the floor an HTM
  occupies to work it — so the relay count is the shelf count, and the standing
  place is a fact the document carries rather than a guess the loader makes.
- Sidedness is geometry. An access names the face a climbing class works from,
  so a rack's `BlockSide` is read from where the rack sits relative to the
  region it is served from, not from grid-column alignment.
- Fleet references structure. A cohort names a robot class and a sector, and the
  sector names the run, so "how many VTMs work this rack row" is a reference
  chain rather than an integer inside a map record.

## 3. The mapping, entity by entity

### 3.1 Identity and ordering

**Dense indices are assigned from geometry, never from identity.** This is the
rule the port learned the hard way at cencora: two documents describing the same
site — the hand conversion and the converter's own output, with different UUIDs
throughout — produced identical maps and *different runs*, because a zone's bays
were ordered by the run stamp's UUID and a bay's slots by the slot's. `RackId` is
an index into that order, inventory is scattered over those indices by a seeded
shuffle, and so the same site stocked itself differently. Bays are now ordered by
where they stand, slots by the `(level, col, depth)` indices the protocol gives
them, and exchange points by where they sit along the face; the two documents run
event for event alike.

Protocol identity is a UUID (declared) or a derivation address (derived). dessim
wants dense small integers. The adapter mints them and keeps both directions:

- `CellId` — assigned in **lattice row-major order** (ascending row, then
  ascending column, ties broken by UUID), so the ordering matches what desmap's
  `row * width + col` produced and the `RackId(i)` enumeration that
  `collect_storage_cells` derives from `CellId` order lands on the same bays.
- `ZoneId` — 1-based, in ascending order of the aisle region's minimum corner,
  ties by UUID. Zone 0 is "not in any zone", as in desmap.
- `RelayId` — 0-based, in zone order, then bay order along the run, then
  position along the bay's face. Mirrors desmap's zone-then-row counter.
- `StationId` — 1-based, in ascending order of station pose, ties by UUID.

`external_ids` (`desmap_zone_id`, `desmap_station_id`, `desmap_rack_side`) are
**not** used as identity anywhere in the loader. They are data, joinable by a
reader comparing a run against an old fixture, and the loader keeps them in its
index for exactly that.

### 3.2 Cells

Every entity carrying a `cell` component becomes a dessim cell. Bays also receive
rack anchors without floor footprints. Kinds:

| Protocol fact | `CellKind` |
|---|---|
| cell of a cell_grid | `Open` |
| …whose region also carries `flow` | `Highway` |
| …carrying `presentation_point` | `Presentation`, `station_id` set |
| …named by the lane into a station's cell | `QueueCell`, `station_id` set |
| …under a station's `pose` | `StationArea` |
| a bay's rack cell (minted, §3.3) | `Relay` when the bay has bound shelves, else `Storage` |

Edge traversability comes from the two endpoints' `cell.traversable_by`:
`htm_traversal = Both` when both admit `htm` and `None` otherwise;
`vtm_traversal` / `rtp_traversal` likewise. Distance is the distance between the
two centres. Neither gate is set here. `dest_only` is cut in afterwards, by the
pass that restricts a relay's access arcs and by the one that makes each lane a
corridor (§5.2); `vtm_zone_gate`, a zone's HTM gate, is policy the world
document does not state at all (§5.3).

### 3.3 A bay's steel is minted; the cell an HTM stands on is read

A rack is not floor, so the protocol gives it no cell. dessim needs one: a
`RelayPosition.position` is a `CellId`, `Racking` hangs every storage position on
one, and the storage-cell enumeration downstream walks `CellId` order. So the
adapter mints one edgeless cell per bay, at the bay's own centre.

The cell an HTM occupies to work a shelf is **not** minted and never was
computed here. Each relay carries an `access_cell` component naming a cell of the
cell_grid the HTM travels, filled in by the pipeline's own binding search (SPEC
§5) or by a document's override. The loader reads it.

That is the one place this round changed most. Through v0.5 an access derived
cells of its own and stitched them into the floor, and this port folded each of
them onto the floor cell it bound to — a cell at the same spot as another cell
was not something dessim's pathfinder could carry, because the zero-length edge
between them breaks the A\* heuristic's admissibility. v0.6 makes the fold
unnecessary by making it the format's own semantics: the relay reaches for a cell
that already exists. What the fold used to record about its own cost
(`relay_stand_offset_mm` in the loader's index) is retracted with it — nothing
is given up, so there is nothing to report.

What survives is the **grouping**, and it earns its own paragraph in §3.5.

### 3.4 VTM coordinates do not create cells

Daniel's September 7 correction: VTM “docking positions” are not physical
features. The former synthetic `Aisle` cells had no travel edges. Their only
uses were initial rail coordinates and cell references for events and playback.

The loader now keeps those coordinates in `VtmPosition::At`, with an existing
rack cell as the reporting anchor. Initial placement retains the former spatial
ordering; stopped playback uses the reported position, not the anchor's centre.
VTM cycle travel still uses rail and shelf geometry, independently of cell
traversal. No aisle cells are minted. Existing grid maps retain their actual
`Aisle` cells and their numeric cell references.

### 3.5 Relays: a dessim relay is a place to stand, not a shelf

A protocol `relay` is one shelf: one level of one bay, at one offset along the
face, bound to one cell. A dessim `RelayPosition` is one **exchange point** — one
place somebody stands — and dessim exchanges at one height per zone
(`RelayZone::relay_height_m`), carrying the rest of the stack for a two-lane
model it does not run yet.

So the loader gathers a bay's relays **by the cell they bound**, and each group
becomes one `RelayPosition` with the group's heights riding as
`Racking::board_floors_m`. cencora is the case the grouping exists for: two relay
levels over the same three along-face positions derive
2 × 3 × 6 bays × 38 runs = **1,368** relay entities, both levels at a position
find the same nearest cell without being told to, and the result is
228 bays × 3 = **684** exchange points — the number desmap compiles.

The grouping is per *bay*, not per cell across the world. Relay pitch and cell
pitch are incommensurate in real sites: cencora's 18 exchange points per run
stand on 15 cells, because the outermost position of one bay and the innermost of
the next find the same cell, 3 times per run over 38 runs. Those two exchange
points are still on two different bays' steel, so they stay two relays sharing a
standing cell — 684 relays, 570 cells, fan-in at most 2. Both facts are pinned in
`worldproto_cencora.rs`.

| Field | Source |
|---|---|
| `position` | the bay's rack cell (§3.3) — also the storage cell the bay's slots hang on |
| `htm_access` | the cell the group's shelves bound to |
| `vtm_access` | rail coordinates with the bay as reporting anchor (§3.4) |
| `block` | `BlockId` = the zone's id, as in desmap |
| `side` | `Left` when the bay sits below the aisle on the access's normal axis, `Right` above — the same convention desmap's `Dual` layout encoded positionally |

`position != htm_access` now. In desmap they were always the same cell, and that
is the behavioural change §5.1 records.

### 3.6 Zones, and one dessim rack per slot

One dessim `Zone` per distinct `along` region named by an access. This reproduces
desmap's sandwich exactly: a `LeftOnly` zone is one run and one aisle, a `Dual`
zone is two runs sharing one aisle, and both are one zone.

| Field | Source |
|---|---|
| `cells` | the zone's rack anchors |
| `relays` | the exchange points of the runs this aisle serves |
| `vtm_count` | the count of every cohort hard-assigned, directly or through a sector, to a run of this zone — once per zone, not once per run (§5.5) |
| `zone_height_mm` | the last entry of the bay's `levels` list: the topmost shelf (§5.4) |
| `zone_width_mm` | the span of the zone's slot and exchange positions along their longer axis |
| `real_shelf_count` | **storage** levels per bay — the relay levels in the same list hold nothing |
| `slots` | `None` — see below |

Storage is **one dessim rack per protocol `slot`, each holding one tote**. A slot
*is* a storage position, so the two line up one for one and `StorageSpaceId`
addresses something the document names — `WorldIndex::storage_slots` keeps the
mapping. desmap arrives at the same granularity from the other side whenever a
zone has explicit rack placements (`effective_vertical_tote_capacity` returns 1
there), and cencora is the proof: 10,032 racks on both paths, one per slot entity.

Three things fall out of it. Every rack carries the height of its own shelf, so
dessim's uniform shelf model is not used at all. Bays of a run may differ in how
many slots they hold, which one vertical capacity could not express. And
`zone.slots` stays `None` for good: desmap's `SlotDef` is `{ cell: [col,row], x,
y, … }`, grid coordinates plus the interpolation that stretches millimetres onto
cell centres, and the heights now travel in the racking plan instead.

`zone_width_mm` is worth its own note. It is derived as the span of the zone's
own slot and exchange positions — the very positions `RelayZone::physical_axis_m`
normalises distances over, so the two cannot disagree. On cencora that derives
**12,200 mm**, which is exactly the number the old fixture had to write by hand
because its compiler "cannot know the real margin past the outermost relay
center". Six bays of 2,140 mm steel span 12,840 mm and the outermost slot columns
sit 320 mm in from each end.

Where a slot sits **along** the face is read off the steel around it:
`slots_wide` columns at `slot_pitch`, centred across the bay's footprint. That
reproduces the cencora prototype's surveyed columns exactly (320, 820, 1320,
1820 mm across a 2,140 mm bay at a 500 mm pitch), which is the check that it is
a reading and not an invention. Where a slot sits **in depth** is not read at
all: both converted fixtures are one deep, and the protocol has not said where
depth 0 starts, so every slot of a bay is placed at the bay's own depth centre.
A document with `slots_deep > 1` is the first thing that will need that ruling.

### 3.7 Stations

One `Station` per entity carrying a `station` component.

| Field | Source |
|---|---|
| `position` | the cell under the station's `pose` |
| `presentation_points` | one per cell the station presents at, by cell id. A station with a belt presents at its dropoff, and the lanes into its belt are its `handoff_queues`. Any other presents at each cell carrying a `presentation_point` that names it, once per cell and whatever carrier the point is for, or, when none names it, at the nearest cell carrying one. Each cell's lane is the `queue` no machine owns that ends there, the cells before it; a cell no such lane ends at has none. A `queue` on a station, a presentation point, a port, a relay or a cell is refused, and so is a lane no machine owns over a cell a station presents at |
| `exit_direction` | `station.exit_direction`: `-y` → North, `+y` → South, `+x` → East, `-x` → West |
| `bins` | `station.bins` |
| `station_type` | `station.kind`: `pick` → `Pick`, `put` → `Putaway` |
| `edge` | `None` — a grid-edge hint with no meaning off the grid |

`station.kind` values `induct` and `extract` have no `StationType` and fail the
load rather than being silently filed as picks.

### 3.8 Fleet

`FleetConfig.htms` and `.rtps` are the summed counts of cohorts whose robot class
has role `htm` / `rtp`. VTM cohorts do not go in `FleetConfig` — dessim derives
the VTM fleet from `Zone::vtms_deployed()`, so a VTM cohort's count lands on the
zone its assignment reaches (§3.6) and the fleet size follows.

A document that declares **no** floor cohort at all leaves the map's `fleet`
unset, so dessim's own system-type default applies exactly as it did before.
That is a different statement from a cohort of size zero, which sets the field to
zero and means a world with no floor robots. Several converted fixtures need the
distinction: a source whose `fleet` and `settings.num_htms` are both absent
produces a document with no HTM cohort, and setting the field to zero there would
turn "the old file said nothing" into "there are no HTMs".

### 3.9 Lengths: integer steps in, millimetres out, converted once

v0.6 made every length in the format an integer count of the steps of the frame
it is stated in, and only the primary frame says what a step is worth
(`unit_mm`). dessim measures in millimetres throughout. The two meet in one
place: `worldmap::StepMm`, built from `world.frames.unit_mm()` at load, applied
to every length after the frame chain has carried it into the primary frame.
Both converted fixtures declare `unit_mm: 1`, so their numbers are unchanged; the
Dallas example declares a tenth of a millimetre and converts through the same
factor.

A **height** crosses on that factor and nothing else. A 2-D frame under a 3-D
parent carries a two-element scale (SPEC §4), so no frame in the format stretches
a vertical length, and a shelf's `height` is a count of primary-frame steps above
the floor at the rack's pose. A floor frame's own elevation is not added: dessim
runs one floor and a shelf height is measured from it.

Positions are carried out through `FrameGraph::to_primary`, and a bay's local
geometry is rotated and rounded exactly the way `worldproto`'s own binding search
does it — half-up on whole steps, at the same joints. That is deliberate: the
exchange position dessim computes and the cell the protocol bound must not come
out of two different arithmetics.

## 4. The lattice: how grid positions survive without a grid

> **[2026-08-12, superseded on the integration branch.]** This section
> describes machinery that has since been deleted: the three consumers it
> names were converted to measure millimetres (the A* heuristic already had
> the straight-line arm; the parking registry, station look-back, HTM
> dispatch and stripe ordering now rank on world positions), cells built by
> `worldmap` carry `grid_pos: None`, and `lattice.rs` is gone. Only the
> header extent (`GridBounds`) survives, computed from distinct centres.
> Kept for the record of why it existed.

dessim still wants a `GridPos` per cell: the A\* heuristic is Manhattan on
ordinals, parking ranks candidates by Manhattan distance, and the station
look-back bound is Manhattan over source cells. None of those need *the* legacy
grid — they need *a* lattice.

The adapter derives one from the **travelled floor**: collect the distinct x
centres and the distinct y centres of every cell of a cell_grid, sort each, and
the ordinal of a centre in its list is that axis's coordinate. A bay's rack anchor
reports onto the nearest square and adds none of its own — it carries no arcs.
Where bay pitch differs from lane pitch, adding its coordinate would make a
lane arc crossing that coordinate read as two steps.

`cell_size_mm` is the **most common arc length in the graph** — how far a move
actually goes. On a floor with one pitch that is the pitch; on cencora's three
bands at two pitches there is no single pitch, and the step most moves take is
the honest answer to a question that assumed there was one. (The first cut used
the tightest gap between two rows, which on a banded floor is a seam artefact:
it read cencora's 850 mm floor as 214 mm and would have made every nominal
traversal four times too quick.)

This is the research note's implication 3 made real — the lattice is an
optimisation over a declared geometry, not the definition of space. It needs no
legacy-lattice frame in the document, works on a document that never had one, and
on a document that does have one it agrees with it wherever the geometry is
regular. On `minimal_putaway2` it reproduces desmap's grid exactly: 8 columns,
18 rows, 1000 mm, and every cell on the coordinate desmap gave it.

On cencora it comes out 153 × 93 against desmap's 153 × 96: the relay band's 18
surveyed rows are shelf positions inside a bay rather than floor structure, so
the band is retiled at a lane pitch and 15 rows cover the same steel.

The property that has to hold is that **every arc moves exactly one ordinal on
exactly one axis** — otherwise the A\* heuristic, which charges each ordinal the
shortest arc on its axis, can estimate more than an arc really costs and A\*
stops returning shortest paths. `worldproto_cencora.rs` asserts it over the whole
graph, and it is the reason the lattice ignores the cells nothing travels
through: with the bay cells included, a seam arc of 856 mm was being estimated at
1,700 mm.

Two cells at one lattice square is still possible. The protocol says a lattice
projection is many-to-one by admission; nothing in dessim keys on `GridPos` for
identity, so a collision costs heuristic accuracy and nothing else.

## 5. Behaviour that changes, and why each change is the ruled one

### 5.1 The HTM stands in the lane, not on the rack cell — ADOPTED

desmap's `RelayPosition` puts `htm_access` on the rack cell itself, so an HTM
drives *onto* the steel to make a handoff. That reads as sensible only because a
desmap rack has no depth: one grid column is the steel, the shelf overhang, and
the HTM's standing room at once. The protocol gives the rack a footprint, so the
HTM has to stand somewhere real, and a relay binds the nearest cell of the floor
outward of its face — one lane column out, in the converted fixtures.

Through v0.5 this was a delta of the *conversion* (`access_cell_one_lane_out` in
the converter's ledgers, 570 occurrences at cencora). In v0.6 it is the format's
own semantics: SPEC §5 rules that an access cell "is an ordinary cell of the
cell_grid the HTM travels", so there is no reading left in which a handoff stands
anywhere else.

**How it is visible.** Every handoff trip is one cell longer at each end, and
HTMs waiting for a relay now occupy lane cells instead of rack cells. A lane cell
is through-traffic, so relays on one run contend for the lane with each other and
with passing robots, where before each relay had its own private standing square.
On `minimal_putaway2` the six standing cells are grid column 5 rows 2..7 — a
corridor four columns of open floor can route around. On cencora **all 570
standing cells are through-lane cells** (graph degree above 2), and that is what
§5.7 had to be built for.

### 5.2 A station's lanes are one-way corridors — ADOPTED

desmap stamps `StationArea`, `Presentation` and `QueueCell` kinds and then prunes
the graph: every edge touching a queue or presentation cell is dropped except the
entry, the forward chain and the exit, and survivors carry `dest_only` pointing
at the presentation cell.

A world document states the same thing as entities. A cell carries a
`presentation_point` naming its station, and the cells a robot queues on to
reach that cell are a **lane**: an entity of its own carrying `queue`, whose path
ends at the cell it feeds. That is the one way a document states them; the
loader refuses a `queue` on an entity that is also a station, a presentation
point, a port, a relay or a cell (`QueueNotOnItsOwnEntity`).

The loader keeps the labels and cuts each lane into the graph once the graph is
built (`queue_corridors`, then `restrict_queue_corridors`): the only arcs left
touching a lane's steps and the cell it feeds are the entry into its first step,
the chain through it, and one exit — the cell's curved arc out when it has one,
else, for a station's lane, its neighbour in the station's `exit_direction`. A
station's lane is an approach, gated `dest_only` to the cell it feeds all the way
in. A machine's lane into a belt handover is a through route, and only its last
arc is gated, to the belt's handovers. A presentation cell no lane ends at keeps
the arcs the document authored, ungated.

**How it is visible.** An HTM enters a station's lane only at its entry cell and
only when bound for the cell the lane feeds, and leaves that cell by its one
exit. A presentation cell without a lane is approached and left like ordinary
floor. Two corridors claiming one arc for different robots refuse the load
(`LaneArcsConflict`).

### 5.3 Zone footprints leave the floor — ADOPTED

desmap keeps a graph node under every rack column and aisle column, files them
under the zone, then blocks HTM travel through them edge by edge. The converted
world excludes the whole footprint, so there is no floor under the steel or in
the aisle at all. The adapter mints one rack anchor per bay (§3.3), with no edges. VTM
positions remain coordinates (§3.4).

**How it is visible.** The HTM-traversable graph is smaller by the zone
footprint, and there is no route *through* a rack run — which was already true,
since desmap blocked every such edge; the difference is that it is now true by
absence rather than by patch. Nothing can accidentally re-enable it.

### 5.4 Racking height is a lookup into the level list — ADOPTED

desmap's `zone_height_mm` defaults to 9000 when a fixture does not set one, and
dessim spreads `real_shelf_count` shelves over it. A v0.6 rack states its steel
as one `levels` list, enumerated bottom-up and strictly increasing, each entry a
height above the floor. A slot names an entry of that list by index. So a shelf's
height is a lookup, `zone_height_mm` is the list's last entry, and
`real_shelf_count` is the count of entries that are storage rather than relay
levels.

**How it is visible.** VTM vertical travel changes wherever the rack's own steel
differs from the old zone default. On cencora the eleven surveyed heights
1,580 – 7,577 mm are used directly, which is what desmap did too through its
per-slot rack path — the two agree there. On `minimal_putaway2` the converter
writes ten storage levels 350 mm apart starting at 350, so the shelves stand at
350 … 3,500 mm against desmap's spread over a 9,000 mm default. (v0.5's document
stated a pitch and a count, and this port multiplied them, which put the same ten
shelves at 0 … 3,150 mm. The 350 mm rhythm was invented by the converter in both
rounds; where it now starts and stops is the document's statement, not the
loader's.)

Provenance worth stating plainly: **the number is invented, the rule is not.**
`minimal_putaway2`'s level spacing is a `desmap2proto` parameter with a written
provenance — the old format never modelled rack height in millimetres outside
rack prototypes — and it is recorded in every ledger under `invented_parameters`.
The *rule* ("racking height is the rack's own steel") is the protocol's. A
converted fixture whose spacing matters should get surveyed levels, which is a
conversion-quality question, not a simulator question.

The alternative reading — take `robot_class.height` of the serving VTM, which
would have reproduced desmap exactly — was rejected: SPEC §10 defines
`robot_class.height` as the chassis height checked against the lowest level of a
rack it drives under, and reading it as vertical reach would be misusing a
component whose meaning is ruled.

**Retracted with the round.** An earlier version of this section had to settle
whether `level_heights_mm` were positions or thicknesses, because the shared
reader summed them to compute tunnel clearance and the converter wrote them as
positions. v0.6 removed `level_heights_mm`, `level_pitch_mm`, `levels`-as-a-count
and `tunnel_levels` together: a level entry states a height above the floor,
nothing sums anything, and the robot-height check reads a rack's lowest level.
The disagreement is gone, not resolved.

### 5.5 A cohort is counted once per zone, not once per run — ADOPTED

A `Dual` zone is two runs facing one aisle, and the sector a VTM cohort is
assigned to names both. Counting the cohort per run deployed it twice: cencora
came out with 76 VTMs against the document's 38. A cohort works the zones its
assignment reaches, and it is the same robots in each.

**How it is visible.** It is not, any more — the count now matches the document
and desmap. It is recorded because the first cut had it wrong and a `LeftOnly`
fixture could never have shown it.

### 5.6 Parking uses dessim's own fallback — ADOPTED, see §7.1

No cell is marked `Parking`, and dessim's own open-floor fallback places the
HTMs. v0.6 added a `parking` marker component for exactly this, and
`tools/desmap2proto` does not emit it yet, so the fallback stays until it does.

### 5.7 A path may step aside and come back — ADOPTED

This is the change §5.1 forced, and it is in the planner rather than in the map.

With every standing cell in a travel lane, an HTM sent to a handoff routinely
finds the cell claimed for a fraction of a second by a robot passing through it.
The search handles that: it plans a path that leaves the robot's current cell,
stands somewhere out of the way, returns, and carries on once the cell is free.
The reservation table then refused to commit that path, because
`ReservationTable::reserve_path` treated *any* request on the predecessor's cell
as a malformed plan — the predecessor's open-ended hold is that cell's
reservation, and a second one would duplicate it.

That invariant was right about overlap and wrong about re-entry. It now refuses
only a request that overlaps the hold while it is still live; a request starting
at or after `close_predecessor_at` says the robot left and came back, and is
decided in the ordinary way against every *other* robot's claim on the cell. Two
unit tests in `model/reservation/test/table.rs` pin both halves.

**How it is visible.** cencora at its own parked workload used to abort at
t = 28.165 s with `task graph invariant violation [HtmPickup]: commit conflict
not resolved after retries`; it now runs to the horizon (§8).

Nothing that used to run can move. The change only *admits* plans the table
previously refused — it narrows no admission and alters no committed window — so
a run that completed before completes the same way. On a desmap-compiled map it
never even fires: a relay's standing cell there is the rack cell, a dead-end
square no through traffic uses, so no path needs the manoeuvre. The desmap
cencora run measured against this branch (529 putaways, 42,826 events over 600 s)
is the same run the previous round measured without the change.

The alternative reading — keep the handoff out of the lane — would contradict
SPEC §5, so it would be a spec change rather than a simulator change. It is not
taken.

## 6. What a world document does not carry

`scenario` and `settings` are **parked** by the converter — recorded verbatim in
each fixture's ledger, not converted — and v0.6 rules that this is where they
belong. SPEC §8 settles what was open: workload and simulator settings are
document kinds of their own, dessim owns both, the world layer depends on
neither, and a file that carries settings carries them as an opaque block the
protocol never interprets.

So the loader returns a map, a fleet, and a racking, and the caller supplies
`ScenarioConfig` and `SimConfig`. The smoke runner
(`sim/dessim/examples/worldproto_run.rs`) supplies the values the old fixture's
parked block held, verbatim, selected by `--workload`.

SPEC §8 also rules that a single file must be able to hold map, fleet, workload
and settings together, and names `minimal_putaway2` as the conversion that will
grow into that demonstration. It has not grown yet, and binding a document's own
`demand` entity to `ScenarioConfig` waits on it.

## 7. Stopped on, and what has since closed

### 7.1 A sector's purpose is still not expressable; parking has a marker now

dessim needs parking cells: HTMs start on them, `ParkingRegistry` sends idle
robots to them, and the pathfinder treats them as legal dwell squares. desmap
declares `parking_zones` and stamps `CellKind::Parking`.

The converted document says a `sector` with a `region` and `members: []`. The
`sector` component carries no role, kind or purpose, and SPEC §2 rules that a
name is "not identity, never unique, never referenced", so a loader may not
branch on it. **A sector's purpose is therefore not expressable, and the loader
does not guess.**

Half of this is now answered. v0.6 added a `parking` component — a marker on a
cell, declared for a whole area at once through its tiling's `derives` — which is
the second of the two readings this document asked for and the better one:
parking is a property of floor, not a kind of grouping. `tools/desmap2proto` does
not emit it yet, and until it does, no cell is marked `Parking` and dessim falls
back to its own documented path for maps without designated parking
(`find_open_starts`, "Fallback for designs without designated Parking cells"),
which distributes HTMs over open floor by a seeded shuffle. Every measured run
below uses that fallback.

What remains open is the general question: does `sector` grow a role, or does
every purpose a sector might have end up as a marker on the cells it covers?

### 7.2 `build_rack_side` re-derived from grid columns what the map already states — CLOSED

Not a protocol gap — a dessim one, and the port needed it changed. The function
matched a rack cell to a relay by shared `(zone, grid_x)`, which held only
because desmap's relay cells *are* its rack cells. With the handoff cell moved
into the lane, the columns differ and every rack came back sideless
(`InvalidScenario::RackWithoutSide`).

Fixed by keying on the relay's own `position` first — the rack cell it sits on —
and recording both the rack column and the handoff column in the fallback, which
is the case the heuristic was written for. This changes nothing for a
desmap-compiled map, where the two keys name the same cell. The side itself is
carried from the survey into scenario generation rather than re-derived.

### 7.3 Every handoff happens in traffic — CLOSED, see §5.7

The finding stands and the fix is §5.7. Recorded here because the shape of the
problem is worth keeping: all 570 of cencora's standing cells are through-lane
cells, the failing conflict was a 471 ms window on a cell serving a single relay,
and the run stopped at every fleet size tried (240 s at 60 HTMs, 66 s at 150,
28 s at 340) — it was when, not whether.

### 7.4 Storage no lane serves refuses the load — CLOSED by SPEC §7

`edge_relay_small`, `java_smoke_relay` and `java_relay_30x30` each hold a run
whose shelves face the next run's steel rather than open floor, so no HTM can
stand anywhere to take a tote away. desmap refused those maps too, through a side
lookup that could find no relay column for the rack and said "unreachable by
VTM". This port refuses at load with `StorageWithNoHandoff`, naming the run and
the count, instead of panicking deep in the relay inventory index.

The open question was whether refusing is right, given that the protocol only
warns. v0.6 answers it: SPEC §7 rules that a run none of whose relays binds a
cell warns rather than failing — 3,300 of the 3,609 runs in the converted fixture
set are like that and refusing them would refuse the installed base — and that "a
simulator that cannot operate steel nobody reaches may refuse the document on its
own account". The severities describe the *document*; what is unusable is the
*consumer's* call. dessim refuses, and that is the ruled arrangement rather than
a unilateral one.

### 7.5 A debug assertion in the SKU ledger fires on both paths

`model/inventory/mod.rs`'s `record_pick_consumed` carries
`debug_assert!(reserved >= 0, "sku_reserved went negative")`, and it fires on
`minimal_putaway2` in a debug build: at t = 35.255 s reading the converted world
document, at t = 44.091 s reading the desmap fixture, both on `SkuId(3)`. It is a
pre-existing accounting bug, not something the port introduced — the port only
changes when the run reaches it, because it changes the timing. Release builds
have overflow checks and debug assertions off and never see it, which is why
every measured run below completes.

The map tests keep their run horizon under it deliberately, and say so.

### 7.6 The root workspace manifest had to learn `exclude`

`world-protocol/rust` declares its own `[workspace]` and sits inside the simulator's
workspace directory, so a path dependency on it made cargo see two workspace
roots. The repo-root `Cargo.toml` now carries `exclude = ["world-proto"]`. That
file is generated by `sync-to-simulator.sh`, which is not in this repo, so the
line will be lost the next time it runs — noisily, since the build then fails.
Flagged for whoever owns that script.

## 8. What has actually been run

`sim/dessim/examples/worldproto_run.rs` loads a document, prints the map, and
drives a run; `native-modules/tests/worldproto_map.rs` and `worldproto_cencora.rs`
pin the numbers against the arithmetic of the source geometry and the converter's
ledgers. The cencora document is a build output — regenerate the fixture set with
`cd world-protocol/tools/desmap2proto && cargo run --release --bin desmap2proto` —
and the tests skip when it is absent.

### minimal_putaway2

132 floor cells + 6 bay anchors = 138, 472 arcs, 6 relays, 1 zone,
2 stations, 10 HTMs, 60 storage positions, shelves at 0.350 – 3.500 m. One
simulated hour: 71 orders and 142 picks against the desmap path's 74 and 148.

### cencora_v3 — historical census against desmap's compilation

The table below predates the surveyed-band correction and September 7 removal
of synthetic VTM cells. The current parity gate has 8,089 floor cells and 228
rack anchors (8,317 total), with no synthetic aisle cells.

| | world document | desmap | why they differ |
|---|---:|---:|---|
| cells | 8,257 | 14,688 | clip boxes and zone footprints leave the floor (§5.3); 5,573 `Blank` and 1,026 zone cells have no counterpart |
| — of the floor | 7,801 | 8,089 | the relay band is retiled at a lane pitch, 15 rows where 18 were surveyed (`surveyed_band_retiled`) |
| — rack / docking | 228 / 228 | 684 / 342 | one bay each, where desmap has one grid row each |
| arcs | 29,758 | 35,046 | 29,374 flow arcs plus 192 stitch links, doubled; desmap's are narrowed by four reachability passes and queue pruning (`traversal_policy_not_derived`) |
| relay entities | 1,368 | — | two relay levels × three positions × 228 bays; desmap has no vocabulary for a shelf |
| exchange points | 684 | 684 | **equal** — a bay's shelves that share a cell are one place to stand (§3.5) |
| standing cells | 570 | 684 | relay pitch and cell pitch are incommensurate; fan-in at most 2 |
| zones | 19 | 19 | **equal** |
| VTMs | 38 | 38 | **equal** |
| storage positions | 10,032 | 10,032 | **equal** — one per `slot` entity on both sides |
| shelf heights | 1.580 – 7.577 m | 1.580 – 7.577 m | **equal** — the same survey, read per slot |
| relay-level heights | [0.394, 0.952] m | [0.394, 0.952] m | **equal** |
| aisle travel | 12,200 mm derived | 12,200 mm hand-written | **equal** — the number the old fixture had to state is now computed |
| lattice | 153 × 93 @ 850 mm | 153 × 96 @ 850 mm | the band's 18 surveyed rows become 15 lane rows |

Every difference is a named delta of the conversion ledgers or a behaviour
change §5 rules; none is unexplained.

### cencora_v3 — the run, at its own parked workload

600 simulated seconds, 340 HTMs, the bulk demand pool, same settings on both
sides:

| | world document | desmap |
|---|---:|---:|
| stop reason | sim time limit | sim time limit |
| picks | 0 | 0 |
| putaways | 555 | 529 |
| orders | 0 | 0 |
| events | 44,433 | 42,826 |

The workload is a putaway peak day with a two-hour pick embargo, so no pick
completes on either side inside ten minutes; putaway throughput and event count
are the comparison. The world document does 5% more putaways over the same
window. That number is a net, and this port has not taken it apart: three
changes push on it at once and in different directions — the lane handoff makes
every trip a cell longer at each end (§5.1), the port as run here had dropped
desmap's queue pruning, so HTMs approached a station from any side, and the
relay band's 15 lane rows stand where 18 surveyed rows did (§4). Shelf heights
are not among them: cencora is surveyed and both paths read the same eleven
numbers. Attributing the 5% would mean turning the three off one at a time,
which is a study rather than a port note, and nobody should read a throughput
claim out of this row until somebody has done it.

### The rest of the relay family

`minimal_putaway`, `minimal_putaway1` and `medium_putaway` load and complete
orders. `edge_relay_small`, `java_smoke_relay` and `java_relay_30x30` refuse at
load under §7.4.

## 9. What the next milestone should be

Recommended, in order:

1. **The single-file scenario.** Until a document carries its own workload and
   settings, "load a scenario" still means "load a map and hand-write the rest",
   and the port cannot claim to have replaced `SimSpec`. SPEC §8 has ruled the
   shape (§6); what is left is the carriage and the reader.
2. **Emit the `parking` marker from the converter (§7.1).** The protocol now has
   the component; the conversion is what is missing, and dessim's fallback is a
   seeded shuffle standing in for a stated fact.
3. **Slot depth (§3.6).** The first document with `slots_deep > 1` needs a ruling
   on where depth 0 starts before dessim can place those slots.

Still deliberately not next: desvis, which works unchanged against a map this
loader builds.
