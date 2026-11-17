# SPEC gaps found while implementing the reference reader

> Against SPEC.md v0.7.

> What this file is for: the reference implementation had to decide something
> the spec does not settle, or found a rule the spec states that its own example
> contradicts. Each entry names the gap, the reading this crate shipped, and —
> where there is one — a recommendation. Every entry is a place two independent
> implementations could disagree and both claim to be right.
>
> **Status.** Items 1, 2, 5, 7, 10, 24, 28 and 32 were adjudicated into SPEC.md
> v0.3 and are marked **RESOLVED** below, as are items 37 and 39 to 41; items 22
> and 43 to 46 were adjudicated into v0.5; item 36 is
> **PARTLY RESOLVED** — its
> embedded sub-case became a rule, its ragged-boundary half did not. Each keeps
> the ruling and the reasoning that produced it. Everything unmarked is still open
> and is the input queue for v0.6, the set `ROADMAP.md` points at by that name.
> v0.6 has since settled items 20, 47 and 53, dissolved the number-spelling half
> of item 26 and half of item 29, and superseded the mechanisms items 7, 44, 45
> and 46 ruled on; each says so in place, and the reasoning is kept even where
> the field it applied to is gone. v0.7 respells every derived identity as its
> address and reshapes relay binding — two-number offsets on the bay, no `face`
> on a relay level, proximity and nothing else in the search, and a warning where
> two shelves at different places share one cell. v0.7 also retires the
> `exclusion` component and gives every tiling its own `exclusions` rectangles:
> that re-owns item 19, narrows item 12, and restates the mechanism items 34,
> 49, 71 and 74 describe without changing what any of them asks for. Each says
> so in place.
> Numbering is stable: resolved items keep their
> number rather than being deleted, because the crate's comments and tests cite
> them.
>
> Ordered roughly by how much a consumer is affected.

## Stitching

**1. RESOLVED (v0.3). The facing direction of a contact stitch was never
defined — and the obvious rule broke the example.** Ruling: SPEC §5 now states
the separation-first rule verbatim as shipped here. SPEC §5 measures a gap "normal to the facing
direction" without saying how that direction is found. Taking it from the
distance between the two cell sets' bounding-box centres picks the wrong axis
twice in `walmart-dallas`, and both times the stitch then produces zero links,
which §5 makes a load error:

| stitch | Δcentre x | Δcentre y | centres pick | separated on |
|---|---|---|---|---|
| approach corridor ↔ station spine | 2250 | 4500 | y → 0 links | x (gap 0) |
| station spine ↔ staging sector | 900 | 6750 | y → 0 links | x (gap 0) |

Both are long-thin-beside-short cases: the spine runs 15300 mm north–south, so
its centre is dragged far away on an axis the two sets *overlap* on. Shipped: an
axis on which the two bounding boxes are separated (edge gap ≥ 0) wins outright;
when both axes separate them, or neither does, the greater centre distance
decides. Reasoning: two sets can only face each other across ground they are not
both standing on.

**2. RESOLVED (v0.3). "Nearest cell within tolerance" or "nearest cell, then
check tolerance"?** Ruling: filter first — among the candidates that clear both
the normal-gap and tangential tests, link the nearest. §5 now says so
explicitly, and the crate matches. The two readings disagreed on exactly one cell
in the example: rack row B's `instance:0/local:rack/relay_cell:0` sits at
x = 18160, and its nearest candidate by gap is an aisle row-1 cell 1310 mm away
tangentially against a 600 + 450 = 1050 mm allowance. Choosing first and checking
after left it unlinked (row B: 23 links); filtering first drops that candidate
and the relay falls through to `cell:17,0` (gap 200, tangential 410), which
qualifies (row B: 24). The suppressed aisle tile `cell:17,1` opens the hole, and
under the ruling the hole costs one link rather than two.

**3. "Half the larger footprint" is unqualified.** Shipped: half the larger of
the two candidate cells' extents, measured on the tangential axis.

**4. Overlap ties leave the matching arbitrary.** With "overlapping footprints
count as gap 0", a rack row A relay cell (y-extent 5300–6200) has normal gap 0
against *both* aisle rows — row 0 overlaps it exactly, row 1 touches at 6200 —
so the choice falls through to the tangential offset (equal, one column) and then
to the lower-identity tie-break. The physically adjacent row wins only by luck. A
signed penetration depth (overlap as a negative distance) would prefer the
overlapping row and remove the coin flip.

*Status: the motivating case no longer reaches the tie-break, and the rest of the
entry stands.* A relay cell standing inside its aisle is the **embedded**
geometry, and item 36's ruling links every cell of the smaller expansion to its
nearest larger-expansion cell by centre distance — so the row A relay cell takes
the adjacent aisle row outright instead of coin-flipping between two rows that
both showed gap 0. What is still open is the same tie in the **separated**
geometry, where two candidates can still show gap 0 on the facing axis and
nothing ranks them.

**5. RESOLVED (v0.3). A region with no `flow` compiled no internal arcs at all,
and the example leaned on that.** Ruling: `flow` is a restriction and never a
permission — a tiled region that declares none is `two_way`. Under v0.2's reading
the station spine, the EasyLoop approach corridor, the staging sector, and the
EasyLoop template's queue and waiting regions produced no arcs among their own
cells, so the "north-south lane every station on this slice hangs off" was a dust
of 17 isolated nodes; the whole example carried 253 arcs, every one of them from
the three regions that do declare a flow. Under the ruling those five regions
gain 6 + 32 + 2 + 14 + 32 = 86 arcs and the example carries 339. This is why the
example documents had to move to `proto_version` 0.3.0: the same bytes mean a
different graph.

**6. Zero-link explicit stitches.** §5's refusal names contact stitches only.
Shipped: an `explicit` stitch with no pairs loads and contributes nothing.

## Expansion

**7. RESOLVED (v0.3), ratified as shipped. Slot indexing under tunnel levels.**
§5 gives `slot:{level},{col},{depth}` zero-based; §10 gives a rack "tunnel
levels". Shipped: storage occupies levels
`tunnel_levels .. levels-1`, so the Dallas tunnel rack (23 levels, 11 tunnel)
emits `slot:11,…` through `slot:22,…` and has no `slot:0,…`. Re-basing the index
at the first storage level gives the same count and different addresses; the
level index would then mean a different height on different racks, which is why
it was not chosen. §5 now says so.

*Superseded (v0.6): there are no tunnel levels.* A rack's `levels` is one
enumerated list of heights and a storage level is any entry carrying no relay
`positions`, so a level index is a position in that list and nothing re-bases
it. What this entry ruled is kept and is now structural rather than a
convention — a level index means the same shelf whatever kind of entry sits
below it (SPEC §5).

**8. Slots carry no pose.** §5 defines where a relay cell sits and says nothing
about where a slot sits. Shipped: `slot` carries `rack`, `level`, `col`, `depth`
and no `pose`, which leaves `rack.slot_pitch_mm` unused and `rack.level_pitch_mm`
used only for tunnel clearance. A consumer that wants to draw a rack cannot.

**9. Relays carry no pose either.** Same reason. Shipped `relay` component:
`rack`, `level`, `index`, `height`, `offset` — the last an `[along, out]` pair in
the bay's own frame, which the rack's pose carries into a frame but which is not
itself a position in one.

**10. RESOLVED (v0.3), ratified as shipped. §10 named no component for an
individual robot or carrier.** The table has
`robot_class`/`cohort` and `carrier_type`/`carrier_pool`, but a cohort's
`robot:{i}` and a pool's `carrier:{i}` have to carry *something*. Shipped: a
`robot` component (`robot_class`, `cohort`, `index`, `assignment`) and a
`carrier` component (`carrier_type`, `pool`, `index`, `designation`,
`placement`), neither with a pose — a pool's `placement` is a rule, not a
position, and resolving it into slots is a consumer's job. Both are now §10 rows.

**11. What a stamp's `instance:{i}` entity carries is unstated.** Shipped: a
`pose` alone — the stamp's pose plus `i · step`, keeping the stamp's heading —
which is what the internals compose through.

**12. Template internals with `region`s must compose too.** §6 says internal
*poses* compose through the instance's pose; the EasyLoop template positions its
queue and waiting block as `region`s with local origins, which are geometry and
must compose the same way. Shipped: the local rectangle's corners rotate by the
instance heading and translate, and `tiling.pitch_mm` swaps axes on a quarter
turn. A v0 region is an axis-aligned rectangle, so a stamped region under a
non-right-angle heading is a load error rather than a silently unrotated
rectangle. §6 should say so.

**NARROWED (v0.7).** A tiling's `exclusions` rectangles are measured from its
region's origin, so translation carries them and heading does not: re-cornering
one onto the region's rotated minimum corner needs the region's extent, which
composing a component at a time never sees. SPEC §6 now refuses a stamp at any
non-zero heading while an internal tiling carries exclusions, which is a
narrower rule than this item's quarter-turn allowance and sits inside it —
quarter turns still stamp regions and pitches as described above, provided no
internal tiling excludes anything.

**13. A template internal that states its own `frame`.** Undefined. Shipped: a
`pose` or `region` carrying a `frame` is treated as already absolute and is not
composed through the instance.

**14. Frames must be declared, never derived.** The frame graph is built from the
merged declarations before expansion runs, so a template cannot ship a frame and
a stamped internal cannot declare one. Nothing in §4 or §6 says this.

**15. A stamp binding a parameter the template does not declare.** §6 makes an
undeclared *reference* and an unbound parameter load errors, and is silent on an
unexpected binding. Shipped as a load error, on the same reasoning §2 gives for
patches: a mistyped name must not silently do nothing.

**16. `alternating_rows` for a `+y` direction.** §5 describes rows only for the
natural along-x case. Shipped: the "row" index is the index on the cross axis, so
a `+y` flow alternates column by column.

**17. Suppressing a generator does not remove what it generated.** §2 says a
suppression "deletes the addressed entity" and nothing more. Shipped literally:
suppressing a tiled region after expansion leaves its cells in place.

*The reading is now demonstrated rather than argued.* One layer declaring a
single tiled way, and a scenario suppressing that way, load together with **0
errors and 0 warnings** and leave the way's three cells standing with the entity
that generated them gone — nothing in §8 makes a derived entity whose generator
left worth mentioning. Two further consequences show up in the artifacts. The
cells lose every arc, because `flow` compiles over a generator's cells and there
is no generator: the same pair of documents keeps 3 cells and goes from **4 flow
arcs to 0**, so the cells survive as an island nothing can enter or leave. And
the two views of that world disagree about what to call it — text-mode output
reads names out of the derived world and prints the generator row as `unnamed`,
while the HTML report falls back to the declaring record, still prints the way's
name, and groups these under "expansions whose generator was itself suppressed".
Both views are defensible, which is the point: nothing says what the surviving
cells belong to.

**18. `expect` on a target with no pose.** §3 defines the pin geometrically.
Shipped: pinning something poseless (a slot, a relay) is a load error rather than
a check that silently passes.

## Records and the pipeline

**19. REVISITED (v0.4, again in v0.7). Excluded coordinates count as
*produced*.** §3 refuses an override or suppression when "no derivation
produces the addressed entity". An exclusion removes a cell its generator *did*
produce. Shipped: such a record loads and is a no-op, which is the literal
reading of "produces".

The second half of this item is now moot. It used to argue that a bake had to
tolerate re-reading its own exclusion tombstones; under cache semantics there are
no tombstones to tolerate. Shipped: the expanded form writes back **only authored
suppressions**. An exclusion is already part of the source — since v0.7 a
rectangle in the tiling's own `exclusions`, before that a region carrying an
`exclusion` component — and re-derivation subtracts it again, so writing the
removals a second time would be a second way of saying what the source already
says, and a way that could drift from it. The removed cells are simply absent
from the `derived` subtree, and the cache check compares that absence against a
fresh derivation like any other node.

**The v0.7 change of owner leaves the ruling exactly where it stood.** A tiling's
`exclusions` remove cells after the tiling has expanded, so the address is still
produced and a record aimed at it still loads and does nothing (SPEC §5 says so
in as many words now). What moved is who may do the removing: the rectangles
belong to the tiling whose cells they take, so the coordinates a record can be
aimed into are decided by one entity rather than by any region anywhere in the
document.

**20. `proto_version` compatibility inside 0.x.** §9 promises additive evolution
"within a major version"; semver's 0.x has no such promise between minors.
Shipped: `0.2.x` accepted, everything else refused.

**RESOLVED (v0.6).** The gate stands and is now written down: SPEC §9 says a
consumer accepts the **current minor** and refuses every other. Shipped
behaviour is therefore `0.7.x` accepted and everything else refused; the
`0.2.x` above is the current minor as of the day this entry was written, not a
second rule. Each minor since has been an argument for the gate — v0.6 turned
`origin_mm` into `origin` and a rack's `boards` into relay levels, and v0.7
turned a relay level's one-number positions into two-number offsets and took
the `exclusion` component away — all of which an ignore-unknown rule would have
swallowed. The last of those does not lean on the gate at all: it changed the
format inside a minor, so a document written against the earlier cut of v0.7
still passes the version check, and only SPEC §9's refusal of the retired name
keeps its floor from loading with the cells its author removed still in it.

**21. `depends_on` ordering.** §8 requires a document's dependencies to be
present, not to precede it in `layers`. Shipped: presence only.

**22. RESOLVED (v0.5). A scenario document's own `entities`.** §8 did not say
whether a scenario may carry records of its own. Shipped: if it does, they act as
the final layer. Ruling (SPEC §8, dated 2026-08-11): a scenario may carry
entities of its own and they act as the final layer, and one document must be
able to hold the whole thing — map, fleet, workload, settings — because that is
how fixtures actually get passed around; multi-file layering stays available for
composition and is never required. Shipped as ruled.

**23. An override and a suppression of one target in one document are
duplicates.** §2's "at most one record per resolved target" resolves both to the
same target. Shipped that way; worth stating, because the natural author intent
("change it, then delete it") is not expressible.

## Baking and the canonical projection

**24. RESOLVED (v0.3), ratified as shipped. Which container each half of a bake
goes in.** §8 lists what a baked
document carries and not where. Folding derived entities into `entities` breaks
two other rules at once: a derived entity written as `id` + `derivation`
classifies as an *override* under §2, and it would then sit beside the actual
override record for the same target, which §2 forbids. Shipped: `entities` holds
source records, `derived` holds derived entities with their provenance,
`edges` holds the arc list.

**25. "All source declarations" when two layers declare one entity.** Keeping
both raw records would put two records on one target inside the single baked
document. Shipped: one merged declaration per entity, and one folded override per
derivation address, both post-layer-order.

**26. RFC 8785 number formatting is approximated.** No canonicalizer crate was
available in the offline registry, so the projection is hand-rolled: object keys
sort by UTF-16 code units, `$comment` is stripped at every depth, and an f64 that
is mathematically an integer inside ±2^53 prints without a fractional part.
Magnitudes outside that range still print in Rust's exponent style rather than
ECMAScript's, so a document carrying 1e30 would canonicalize differently in a
JavaScript implementation. No site geometry reaches that range; a real
`serde_json_canonicalizer` dependency would close it.

*Half of this is dissolved (v0.6).* Every length is now an integer count of a
frame's steps (SPEC §5), so a geometry value has no fractional part to spell
and the `5750` / `5750.0` divergence cannot arise from a world document at all
— which was the half of this entry that touched every artifact. What is left is
the general canonicalizer gap: a non-geometry number outside ±2^53 still prints
in Rust's exponent style rather than ECMAScript's, and `unit_mm` is fractional
by design. The dependency still closes it.

**27. Edges have no stated wire shape.** §5 gives edge *keys* and says edges are
not entities. Shipped: `{ owner, key, from, to, bidirectional }`, with flow arcs
directed (one record each way, so either direction can be suppressed alone) and
stitch links bidirectional.

## Checks

**28. RESOLVED (v0.3). §7's cohort check fired a false positive on the
example.** Ruling: a rack entity in the target's expansion is habitat for a
vtm-role class, because climbing a rack face is rack-internal motion and not
graph traversal — the same distinction `world.json` already draws for aisles.
Under v0.2's wording, "a cohort hard-assigned to a sector none of whose cells
admit its class" warned for the order-buffer VTM cohort, whose sector names the
buffer rack row and whose only cells are the racks' HTM-only relay cells; no VTM
cohort assigned to racks could ever satisfy it. With the ruling the example
produces no warnings at all.

**29. Tunnel clearance has nothing to check against.** §10 says a robot class's
`height_mm` is "checked against tunnel clearance", but no declared reference ties
the corridor region to the rack row above it — in the example they merely share a
footprint. Shipped as a geometric heuristic: a cell whose centre falls inside a
rack's footprint and whose `traversable_by` admits a class taller than
`tunnel_levels × level_pitch_mm` warns. On the example that is 11 × 350 = 3850 mm
against the VTM's 3600 mm, so it stays silent. A declared association would be
better than a heuristic.

*Half resolved (v0.6).* The quantity to check against exists now: there are no
tunnel levels, and SPEC §7 checks a class's height against the **lowest level**
of any rack whose footprint its cell_grid crosses. The association is still
geometric — footprint containment, not a declared reference — so the second
half of this entry stands exactly as written.

**30. "Two tiled regions that touch with no stitch" ships as a warning, not the
error §5 calls it.** "Touch" has no numeric definition, and the example
deliberately *overlaps* two expansions — the rack rows' relay cells stand inside
the aisle strip, which §12.8 raises as an open question. Refusing on the strict
reading would refuse the format's own example. Shipped: an exact shared boundary
on one axis with overlap on the other, reported as a warning. The example
produces none.

**31. Flow over uninhabitable cells is unspecified in detail.** Shipped: warn
when a flowed generator has cells and not one of them lists any
`traversable_by` class.

**32. RESOLVED (v0.3), ratified as shipped. An override that replaces a
generator-input component is a silent no-op, and nothing in the spec said so.** §8 lists the generator inputs (`frame`,
`region`, `tiling`, `flow`, `rack`, `template`, `stamp`, `cohort`,
`carrier_pool`) and puts the merge before expansion for exactly that reason — but
overrides run at step 4, after expansion, so an override replacing one of those
changes the entity and not what it already produced. An author who meant to
change the expansion wanted a patch. Shipped as a warning, since the record is
well formed and might be aimed at a consumer that re-expands. The example raises
none.

## Types and units

**33. A pose's units come from its frame, but the millimetre rule does not cover
poses.** §5 fixes `region`, `tiling` and every `_mm` field as millimetre-valued
and refuses a non-millimetre frame for them; `pose.position` is exempt by
construction, since §5 also says position fields carry no unit suffix. Shipped: a
distinct `Coord` type for frame-relative positions, converted to `Mm` only
through the frame chain, so a pose declared in the nominal grid frame still
resolves.

**34. Everything geometric here is planar.** §4 makes the primary frame 3-D, and
every rule in §5 — a tiling's exclusion rectangles, stitch matching, region
bounds — is written in two dimensions. Shipped: x and y are used and a third
coordinate is carried but ignored. Multi-floor sites will need §5 to say what
happens on z.

**35. Frame rotation is absent (§4 `OPEN:`), so headings pass through frame
changes untouched.** A stamped entity's heading is the stamp's heading composed
with the internal's, and no frame contributes to it. This is consistent with §4
today and will stop being so the day frame rotation lands.

**36. PARTLY RESOLVED (v0.3). "Boundary-facing" admits only the exact extreme
layer.** `facing_pairs` lets a cell participate only when its facing edge
coincides with the whole set's bounding-box facing edge, so an endpoint with a
ragged boundary — cells at several depths along the facing axis — links only its
outermost layer, and a recessed cell that is nonetheless the true nearest partner
for some opposite cell never links.

*The embedded sub-case is resolved.* It stopped being hypothetical the moment a
real desmap fixture was converted: in `minimal-putaway2` the six relay cells of a
rotated rack row stand *inside* the open floor's expansion, so neither axis
separates the bounding boxes, the centre-distance fallback picked the axis
running **along** the relay line, and the boundary filter then admitted its one
extreme cell — 1 link where the seam needs 6. Ruling (SPEC §5, v0.3): contact has
two geometries, chosen by the bounding boxes rather than by anything an author
writes.

- **Embedded** — the two expansions overlap on *both* axes. There is no axis to
  face each other across, so the facing and boundary machinery is skipped
  entirely: every cell of the smaller expansion links to its nearest cell of the
  larger one whose edge-to-edge gap on *each* axis is ≤ `tolerance_mm` (overlap
  counts as 0), nearest by centre distance, ties to the lower identity, filtered
  before choosing.
- **Separated** — otherwise, the separation-first facing rule of items 1 and 2,
  unchanged.

Dallas is unaffected in count: its five embedded stitches (both rack rows into
the aisle, and the three stations beside the spine) keep 24 + 24 + 1 + 1 + 1
links. Their *targets* got more sensible — a row A relay cell now takes the
adjacent aisle row by centre distance instead of coin-flipping between two rows
that both showed gap 0 (item 4's tie).

*The ragged-boundary question in the separated geometry remains open.* Every
current example seam is flat, so nothing exercises it. v0.4 should either define
"boundary-adjacent" per tangential lane — the nearest cell in each lane
participates — or keep the strict reading and say so.

**37. RESOLVED (v0.3). A queue path could not name a hand-declared cell.** §10's
queue row admitted two address forms, `{generator, key}` and the
template-internal `{local_ref, key}`, and both name a *generator* — so a cell
authored directly, which §3 promises is always possible (deriving an entity is a
convenience, not a caste system), could not be queued over at all. Stitch pairs
had no such hole, since §5 already let a pair name a cell by its own identity:
two address vocabularies, differing for no stated reason.

Ruling: a cell's own identity is the third form, and **cell address** is now
defined once in SPEC §3 — the cell's identity in either form, a derivation
address in its two parts, or the local address form inside templates — with
queue paths and stitch pairs both referring to that one
definition instead of each restating part of it. The crate's `CellAddress` carries
all three; resolution and the four queue-path checks were unchanged, because they
already worked on identities rather than on addresses.

**38. Retired top-level flags are refused, not ignored.** Two flags have now come
and gone: `materialized` through v0.3, `baked` briefly in v0.4. The expanded form
retired the idea itself — an inline `derived` subtree *is* the announcement that a
document carries its expansion, so there is nothing left for a flag to say.

Shipped: both names are refused when a document claims `proto_version` 0.4.x.
§9 requires a consumer to ignore "unknown components and unknown fields inside
known components"; a flag at document scope is neither, so the ignore rule does
not reach it. Reading such a flag as absent would treat a document that believes
it carries an expansion as one that carries none, and the version gate cannot
catch it because the document claims the current version. Still worth a sentence
in §9: retired names at document scope are refusals, not unknown fields, and the
distinction is real and currently unwritten.

**39. RESOLVED (v0.4), folded into §8. The expanded form is compared in the
canonical projection, not as JSON.**
A derived cell centre is computed as `5750.0` and written to the canonical
projection as `5750` (§9's number spelling), so an inline expansion that has been
through a file is *equal* to the world it came from and unequal as raw JSON. This
crate reduces both sides to the projection before comparing a cached node against
a freshly derived one, and §8 now says so: "byte-identical" means byte-identical
**in the projection**. A loader that compares parsed values instead refuses every
artifact it just wrote.

**40. RESOLVED (v0.4). What a derived node's `derived` is: array or map?** §8 said
nodes are "keyed by their own segment" while giving the node shape as
`{ key, id?, components, derived?, edges? }` — with `key` *inside* the node, which
reads as an array. Ruling: it is a **map** from segment string to node, and the
node loses its `key` field. Reasoning, which the array reading did not have: a map
makes two nodes claiming one segment unwriteable rather than merely illegal, and
it needs no sorting rule at all, because the canonical projection already orders
object keys. Shipped as ruled.

**41. RESOLVED (v0.4), folded into §8. A partial cache is a stale cache.** §8 said
verification is all or nothing per document without saying whether a document may
cache *some* of its expansion. Now it says: strict. The roots checked are every generator the document
declares that derives anything, plus every root it wrote a subtree for — so a
document that caches one generator and omits another is refused for the missing
one. Reasoning: a partial cache cannot be told apart from a cache that went stale
by losing a subtree, and the whole value of the mechanism is that trusting it is
safe. One consequence worth knowing: dropping `derived` while keeping `edges` is a
refusal; only removing both leaves a document with no cache at all.

**42. The expanded form keeps `expect` pins, and had to be taught to.** A pin is
authored data — it records where the author believed an ordinal address pointed —
so an artifact that folds overrides per target and drops their pins hands the next
reader records that can be silently re-aimed, and §3's own warning fires on the
artifact where it did not fire on the source. Shipped: pins ride through onto the
emitted override and suppression records. §8 should say that the expanded form
preserves every field of the records it restates, not only their components.

**43. RESOLVED (v0.5). `access_side` conflated three statements.** A rack carried
`access_side`, `relay_positions_mm`, `relay_offset_mm` and `relay_cell`, which
between them said *this face is the reachable one*, *a class serves from it*, and
*here is where somebody stands* — three claims in one field, on the wrong entity,
none of them checkable against the stitch that had to agree with all three. Ruling:
the rack is pure steel and the junction is reified as an `access`. A rack
reachable from both sides is one rack with two accesses. Shipped as ruled; the
crate no longer special-cases relay stitches anywhere, because they were ordinary
stitches all along.

**44. RESOLVED (v0.5). Relays had no height.** A relay was a position along a
face and nothing else, so a stack — two boards at one along-face position, which
cencora's real racking has — could not be written. Ruling: `boards:
[{height_mm, positions_mm}]`, relays flat-indexed board-major, and a boards-access
derives one access cell per *distinct* position, so a stack is two shelves and one
place to stand. Shipped as ruled; `level_heights_mm` likewise lets surveyed shelf
spacing beat a nominal pitch, and tunnel clearance sums the array when it exists.

*Superseded (v0.6).* `boards`, `level_heights_mm` and `level_pitch_mm` are all
gone into one list: a rack's `levels`, always enumerated, each entry a height,
and an entry carrying `positions`/`face`/`reach` is a **relay level**. A stack
is two entries at two heights over the same positions, and the relays key
`relay:{level},{i}`. The ruling is kept — a stack must be expressible, and two
stacked shelves are one place to stand — but the second half now falls out of
binding rather than out of a distinct-position rule, since both relays' nearest
cell is the same cell (SPEC §5).

**45. RESOLVED (v0.5). A boards-access bound by the *smaller* set, which is not
what an access means.** SPEC §5 tells a boards-access to use "the existing contact matching", and
that matching is symmetric: whichever set has fewer cells drives, each of its
cells claiming its nearest partner. That is right for a stitch, which joins two
peers. An access is not a peer relation — it binds *its own* cells into a way — and
when the way is the smaller set the direction inverts. Dallas's order-buffer lane
is the case: 8 access cells against 6 lane cells, so the 6 lane cells drive and
`access_cell:0,2` (x 3720) and `access_cell:1,2` (x 6470) bind to nothing. Both
are somebody's second choice — 3720's nearest lane cell was claimed by 3030, and
6470's by 5780 — so two board positions have no floor in front of them and nothing
says so. Ruling, stronger than the recommendation this entry made: a boards-access always
drives from its own access cells, each linking to its nearest `along` cell within
`tolerance_mm`; many access cells per floor cell is the expected shape; and an
access cell with no candidate in range is a **load error** naming it, on the same
doctrine as a zero-link stitch — a board with no floor in front of it is an
authoring error, not a silence. Dallas's buffer now binds all eight.

*Superseded (v0.6), ruling intact.* There is no boards-access left to drive
from, and nothing derives a cell: each **relay** binds its own access cell,
taking the nearest cell within its relay level's `reach`. That is this entry's
ruling with the mechanism removed — the binding is still driven from the steel
outward, many relays per floor cell is still the expected shape rather than an
error, and a relay that finds nothing is still a load error naming it
(SPEC §5).

**46. RESOLVED (v0.5) alongside 45. A zero-link boards-access is a load error.** §5 makes a zero-link *contact stitch* a load error and says a
boards-access "subsum[es] the relay stitches this role was played by before".
Shipped: a boards-access binding nothing fails the load, on the reasoning that a
mechanism inheriting a role inherits its refusal, and that boards nobody can stand
at are steel nobody reaches. Worth one sentence in §5 either way.

*Revised (v0.6).* The refusal lands per relay now, and the total case is a
warning instead. A relay with no cell in reach while its neighbours on the same
run bind is a load error, as here. A run **none** of whose relays binds
anything warns once, because that is the ordinary shape of a converted legacy
site (item 53) and refusing it would refuse the installed base. SPEC §7 carries
the split, including the part that is not the protocol's to decide: a simulator
may refuse steel it cannot operate even where the document is well formed.

**47. What an access cell's `cell` component contains is unstated.** §5 says the
boards-access derives "the access cell in `along`" without saying what it carries.
Shipped: the `cell` defaults of `along`'s tiling, so an access cell admits exactly
what the way admits — it is a piece of that way, not a kind of its own. The
alternative, deriving it from `serves`, would make what a boards-access derives
depend on `serves`, which §5 explicitly forbids.

**RESOLVED (v0.6) by restructure.** Nothing derives an access cell any more. A
relay binds to an ordinary cell of the cell_grid the HTM travels, so an access
cell carries whatever that tiling's `derives` gave it — which is exactly what
this crate shipped, now true by construction instead of by rule. The reasoning
survives the restructure: what the binding produces still cannot depend on
`serves`, because an access derives nothing at all (SPEC §5).

**48. `locomotion` absent reads as `floor`.** §10 lists `floor`/`ladder` without
saying what silence means. Shipped: absent is `floor`, because a floor robot is
the common case and a climbing robot is the one whose way needs no tiling — the
exception should be the thing you have to declare. Every check that asks reads it
through one accessor, so a ruling the other way is one line.

## Converting the installed base

> Items 49 to 53 come from `tools/desmap2proto`, which converts every
> WarehouseDesign v3 fixture and compares each result against desmap's own
> compiled map. They are the constructs the two hand conversions never met, and
> each one is a place the converter stopped rather than invented. Counts are from
> `tools/desmap2proto/ledgers/INDEX.md`.

**49. A one-way lane inside an open floor has no stated composition.** `IDIOMS.md`
says a highway is "a region + tiling + a `flow`", and §5 compiles a `flow` over
one generator's own cells. Neither says how a directional lane sits *inside* a
floor that is otherwise two-way, and the composition it implies is heavy: the
floor must lose its cells under the lane (since v0.7 a rectangle in the floor
tiling's own `exclusions`), the lane must become its own tiled region, and the
two must be stitched back at every seam — four contact
stitches per block, each of which fails the load if it produces no links, and a
1x1 block has four seams and no interior. 119 highway blocks across five fixtures
need this rule and neither hand conversion had a single highway to derive it
from, so the converter records every directional block as unmapped and converts
none of them. (Bidirectional blocks need no rule: desmap's own traversal treats
`Highway` and `Open` identically, so they are the open floor they stand on.)
Recommendation: either write the composition into `IDIOMS.md` as the answer, or
rule on the cheaper alternative — letting a `flow` name a sub-rectangle of its
own region, so a lane needs no lift at all.

**50. No `flow` rule states "both directions along one axis, none across it".**
desmap's `LaneDirection` has `NorthSouthOnly` and `EastWestOnly`; §5's three
rules are `one_way` (directional), `two_way` (unrestricted) and
`alternating_rows`. Seven blocks across four fixtures use them. Recommendation: a
fourth rule (`axis_only`, naming an axis rather than a direction) is a two-line
addition and covers the case exactly; the alternative is the small rule language
§12.4 already leaves open, which this would be the second argument for.

**51. A rack's shelf count and its inventory budget are one number, and the
installed base uses it as both.** `rack.levels` is physical: §5 derives one slot
per level per column per depth from it. The old `vertical_tote_capacity` is
documented in desmap's own field comment as possibly "inflated as a hack for
inventory capacity" independent of `real_shelf_count`, and one fixture proves the
warning — `cencora_phase1_dec16` declares 740 of them against a `zone_height_mm`
of 9,000, so 740 shelves would stand 259 metres tall at any plausible pitch. The
converter carries the number forward faithfully, which derives 710,400 slot
entities from a budget rather than from steel, and records it as unmapped.
Recommendation: rule whether "how many carriers may be held here" is world data
distinct from the shelf grid — a field on `storage` would be the natural home —
or scenario policy that belongs nowhere near `rack`.

**52. A movable rack has no representation, so an RTP zone converts to a hole in
the floor and nothing else.** `IDIOMS.md` maps butler's MSU to **rack** and says
"mobility becomes a component when RTP enters scope"; 320 RTP zones across 19
fixtures are that scope arriving. The old `RtpZone` carries a rectangle and an id
and no vertical model at all, so there is nothing to convert into slots, levels
or boards even if mobility existed. The converter excludes the footprint (desmap
stamps those cells `Storage` and blocks every HTM edge touching one, so they
genuinely leave the floor) and records the storage structure, the carrier and the
robot class as unmapped. Recommendation: rule whether a movable rack is a `rack`
plus a mobility component or a different noun, and what an RTP bot's `carries`
says when the thing it carries is racking.

**53. A run with no way onto it is a load error when authored and the common case
when converted.** Items 45 and 46 made an unbound access cell and a zero-link
boards-access load errors, which is right for a document somebody wrote. Converted
legacy sites are the other shape: relay zones stand shoulder to shoulder, so the
grid column outside a rack column is the *next* zone's rack column, and 3,300 of
the 3,609 rack runs the fixture set converts to have no continuous open lane on
their outward face — 2,142 of 2,176 in `scale_200x200` alone. The converter therefore
declares no boards-access for those runs at all, which loads cleanly and leaves
"which steel can be reached" stated by the *presence* of an access. Two things
follow that the spec does not settle. First, nothing distinguishes a run nobody
declared a way onto from a run that is genuinely unreachable, and §7's check
warns on neither. Second, desmap does emit relay positions on those faces,
through an end-cap rule (a rack cell touching open floor above or below the run)
whose own HTM edge rules then block, so the old format's relay count and the new
format's access-cell count diverge for a reason that is a defect in the source
rather than in either format. Recommendation: a §7 warning on a run carrying
boards that no boards-access serves, phrased as "steel nobody reaches" rather
than as an error, so the converted document says out loud what the fixture set is
full of.

**RESOLVED (v0.6) as recommended.** SPEC §7 warns once per run on a run none of
whose relays binds an access cell, phrased as steel nobody reaches. Both halves
this entry names are answered. The first: a converted site now states its relays
and visibly fails to bind them, rather than omitting an access and leaving
reachability implied by absence — and a run where one relay fails while its
neighbours bind is still refused, so the authoring mistake and the site fact no
longer look alike. The second: §7 states the severity split, so a simulator may
refuse what it cannot operate while the checker keeps calling it a warning. The
end-cap divergence in the source stands as recorded here.

> Items 54 to 69 come from `tools/mapcreator2proto`, which imports five Map
> Creator site dumps — three relay sites, two RTP — and checks each imported
> document three ways: it loads through this crate with no errors, every count is
> predicted from the export before the document is loaded, and every field family
> the dump carries has a written disposition. They are what the installed base
> states that the protocol has no form for. Counts are from
> `tools/mapcreator2proto/ledgers/INDEX.md`, and each is a total across the sites
> that carry the construct.

**54. An explicit stitch's `a` and `b` are required and unread, so an imported
adjacency graph is one stitch entity per edge.** Item 6 found that an `explicit`
stitch with no pairs loads and contributes nothing; this is the same field pair
seen from the other side. `compile_explicit` never looks at `a` or `b` — only
`pairs` — but both are required by the component's schema and both must resolve,
so a single bulk stitch carrying a whole floor's edges has no honest value to put
in them. A Map Creator export declares its cells rather than tiling them, so
every edge is an explicit pair: the importer emits **16,546 stitch entities
across five sites** (5,326 on `apotok-prod` alone), each one an entity, a rule, a
`pairs` list of exactly one pair, and an `a`/`b` that repeat that pair. The
alternative — one stitch with 5,326 pairs and two arbitrary endpoints — states
something false about which two expansions the seam joins. Recommendation: either
make `a` and `b` optional when `rule` is `explicit` (they are documentation
there, and the pairs already name the endpoints), or give the format a bulk
edge-set form for a converted floor whose graph is drawn rather than derived.

**55. A one-way seam between two declared cells cannot be stated at all, and the
answer §5 offers does not reach it.** §5 says a directional seam is expressed by
suppressing one direction's edge, and `DISCUSSION.md` §4.5 records that as the
standing answer with no lean, adding that what forces a decision is "the first
converted site with a one-way seam; on current evidence that comes from Map
Creator". Here it is: **3,888 adjacencies across four sites are passable from one
end only** (1,809 on `apotok-prod`, 1,454 on `owen`, 624 on `walmart-pot`, 1 on
`lakeside`), e.g. `apotok-prod` `[18,1] -> [55,1]`, whose far end answers
`[1,0,0]`. The suppression answer works for flow arcs and not for these: item 27
shipped a flow arc as one record per direction *so that either can be suppressed
alone*, while a stitch link is a single record keyed `link:{i}` with
`bidirectional: true` and no handle for one of its two directions. Between two
declared cells there is no tiling and therefore no flow, so a stitch link is the
only edge available and there is nothing to suppress half of. The importer emits
no link for these pairs rather than a bidirectional one, because a bidirectional
link would grant travel the export withholds — so one in five of `apotok-prod`'s
8,750 adjacencies is simply absent from the imported graph. Recommendation: rule §4.5
now, and note that whichever way it goes, a stitch link needs a per-direction key
before a suppression can reach one.

**56. "These two cells are neighbours and no robot may pass between them" has no
form.** Butler decodes a `neighbours` triple as `[neighbour exists, bot may move,
bot with load may move]`, and the installed base uses `[1,0,0]` at both ends
heavily: **3,326 adjacencies across all five sites** (1,615 on `apotok-prod`,
1,275 on `owen`), e.g. `owen` `[12,2] <-> [17,2]`, both cells under the profile
`HA` and both answering `[1,0,0]`. The protocol has one notion — a link is an arc
— so the importer emits nothing, and the imported floor cannot distinguish two
mapped squares with a rack leg between them from two squares with nothing between
them at all. This matters beyond fidelity: a consumer computing which cells are
*near* each other, or auditing an aisle for a missing seam, sees a hole where the
export saw a wall. Recommendation: rule whether adjacency-without-traversal is a
thing the format should be able to say, or whether it is correctly absent because
the graph is a traversability graph and nothing else.

**57. An edge a robot may cross empty but not loaded has no grade.** The third
slot of a `neighbours` triple is exactly that distinction —
`vda5050_map:get_allowed_movements/1` reads `[1,1,0]` as `[bot]` and `[1,1,1]` as
`[bot, rack]`, and hands the pair to the vendor protocol — and **1,514 edges
across all five sites** carry a grade the two ends do not both answer `1` to
(763 on `lakeside`, 443 on `apotok-prod`, 246 on `warehouse_map`), e.g.
`apotok-prod` `[31,2] <-> [31,1]`. The importer emits these as ordinary two-way
links and records the loss, because the alternative is dropping an edge that is
genuinely traversable. SPEC's own reading of the field is that it "needs a
design, not a port": nothing explains the slot, and the same edge is graded
differently from each end thousands of times. This item is the count that says
how often it is used, not a proposal for what it means.

**58. `cell.traversable_by` is role-granular and the installed base states
permission per robot *model*.** §7 makes `traversable_by` a list of agent classes
and §7's own check matches it against `robot_class.role`, so two configurations of
one role are one entry. `walmart-pot` runs two HAI HTM models, `HTM_HAI_K50_L_E1`
and `HTM_HAI_K50_X_E1`, and its profiles routinely permit one and forbid the
other: **474 cells** are covered by a profile that says yes to one model of a role
and no to another, e.g. cell `[29,1]` under profile `AtPutPPS`. Collapsed to
`["htm"]`, the answer is true of one model and false of the other. The importer
does collapse — a role list is what the check compares — and declares one
`robot_class` per bot type beside it so the type set is at least joinable through
`external_ids.butler_bot_version`. Recommendation: rule whether `traversable_by`
names roles or classes; if classes, it wants entity refs rather than strings, and
§7's check follows.

**59. `traversable_by` is a positive list with no spelling for "unrestricted",
and most of the installed base is unrestricted.** **5,148 cells across all five
sites carry no permission statement at all** — every cell of both RTP sites
(1,867 on `lakeside`, 1,882 on `warehouse_map`, neither of which ships
`location_profiles`), plus 1,234 on `apotok-prod`, e.g. cell `[999,350]`. Butler's
documented convention reads absence as permissive: an empty `barcode_properties`
means every bot, every orientation, every height. A positive list cannot say that,
and the importer will not write every known class in, because that invents a
permission the export never granted — so those cells import with
`traversable_by: []`, which a naive consumer reads as "nothing may stand here",
the exact opposite of what the source means. Recommendation: either a sentinel
(`traversable_by: null` or an `unrestricted` flag) or a ruling that empty means
unrestricted, which would make the *restricted* case the one that has to be
declared and match butler.

**60. A cell's lift-height envelope and ground clearance have nowhere to go, and
they are the best thing in the export.** A `location_profile` states, per bot type
and per heading, which lift heights are legal loaded and unloaded and how much
ground clearance there is — physical capability held separately from policy,
which is §7's own split arriving early. **60 profiles across three relay sites**
carry one (15 on `apotok-prod`, 24 on `owen`, 21 on `walmart-pot`), e.g. profile
`VA`. All that survives the import is the class list; the envelope is dropped
whole. Two encodings would need fixing on the way in, and they are the kind of
thing a schema should refuse rather than accept: `ground_height_clearance` is an
integer of millimetres or the string `"all"` (on `walmart-pot`, 50 strings and 32
integers), and an unconstrained lift range is spelled `[]`, `null`, or an absent
key. Recommendation: this is the component §7 most obviously wants next, and the
installed base has already designed it — adopt the shape and collapse the three
spellings of "no constraint" to one.

**61. A storage position cannot exist without a rack, and the export declares no
rack anywhere.** §5 derives a `slot` only as a rack's projection, and `slot.rack`
is a required reference. Map Creator states storage as a flat tuple — (access
cell, bot orientation, storage direction, depth index, height above the floor)
under an allocation-counter id — with nothing naming the steel it belongs to. It
carries **55,721 such positions across three sites** (15,966 `apotok-prod`,
21,737 `owen`, 18,018 `walmart-pot`), e.g. `{"location":"TLOC_0000001","io_point":{"coordinate":
"[19,151]","bot_orientation":0},"storage_direction":0,"ndeep":0,"height":1285}`.
None of them import. **What the importer deliberately did not do:** it did not
synthesise racks by clustering the rows. Grouping by (access cell, orientation,
direction) and reading the height ladder as levels would give levels and depth
plausibly — `apotok-prod` has 1,175 distinct access cells, 12 distinct heights at
a 400 mm pitch, and 368 relay points, and the regularity is total — but bay
boundaries, column counts and `footprint_mm` would all be invented, and a rack is
the entity every derived slot's identity hangs from. A wrong guess about where one
bay ends would rename every slot beneath it. Recommendation: rule whether a slot
may be declared with no rack (an optional `slot.rack`, or a `storage_position`
component for surveyed storage), noting that the natural key these rows already
carry — (access cell, orientation, direction, `ndeep`, height) — is unique across
all 55,721 rows of all sites, so the identity exists and only the parent is
missing.

**62. A zone is an e-stop domain and there is no component for one.** `IDIOMS.md`
already maps butler's `zone` to "an e-stop/safety domain — a future policy
component on cells (reserve `safety_domain`)"; **92 zones across all five sites**
are that reservation coming due (42 `apotok-prod`, 29 `lakeside`, 15 `owen`, 5
`walmart-pot`, 1 `warehouse_map`), e.g. `apotok-prod`'s `"PPS_116"`. Every cell
carries exactly one, butler pauses and blocks whole zones on physical safety
events (button press, light-curtain breach, station breach), and it computes a
buffer of every cell within 1,700 mm of one for pre-emptive stopping. The importer
carries each cell's zone in `external_ids` as a string and declares no entity for
the domain. **What the importer deliberately did not do:** it did not map a zone
to a `sector`. A sector is a storage-policy partition used as a hard put
constraint — `IDIOMS.md` maps Map Creator's *own* `sector` field to it, and the
importer does — so making a zone one too would put two unrelated gates in one
noun and silently give the put calculation 92 new partitions. Recommendation:
`safety_domain` as a first-class entity with explicit cell membership, and note
what the join key looks like today: `walmart-pot`'s largest zone is
`"Zone 102 Exite Gate "`, typo and trailing space, keying 925 of its 1,424 cells.

**63. A waiting area cannot name its cells unless it is a region.** §10 says a
waiting area's capacity is its cell count and gives it no member list, which works
in the examples because a waiting area there is a tiled region and its cells are
the ones it generates. A surveyed floor has no region to tile: **10 waiting blocks
across three sites state 20 member cells** between them and none of the membership
imports (e.g. `apotok-prod` block 56, which covers one cell). The imported
`waiting` entity is therefore an entity with a name and no extent. Recommendation:
either an explicit member list (the same cell-address vocabulary §3 gives a queue
path) or a ruling that a waiting area must be a region, which would make this an
authoring rule rather than a silent hole.

**64. A queue path is an ordered walk with no way to mark a step, and the
installed base needs both more and less order than that.** Two shapes from one
component. *More:* a `sequenced_queue` block names the cell it synchronises at and
the cell it leaves by, and butler validates that both are members of the block.
There are **2 such blocks on `walmart-pot`**, e.g. block 11 with
`{"sync_point":[999,964],"exit_point":[999,960]}`. A v0 queue has `path` and
`discipline` — and had a `depth` beside them until 2026-09-02, when a queue's
admission became its path and nothing else (`DISCUSSION.md` §5.5) — with no way
to say that step three is the sync point. *Less:* the
RTP dialect's station queue is `queue_barcodes`, a bare list of six to eight cell
barcodes — **30 stations across two sites** (20 `lakeside`, 10 `warehouse_map`),
e.g. `lakeside` pps 1 with 8 of them. **What the importer deliberately did not
do:** it did not import that list as a queue path. The list has an order in the
file and that order carries no meaning — no entry designation, no exit
designation, no relation to `block.json` — and a v0 path is ordered, so importing
it would invent the queue rather than convert it. Recommendation: rule whether a
queue path may carry named steps (`sync`, `exit`), and whether an unordered
holding set at a station is a `waiting` area rather than a queue — which would
give the RTP dialect a home and connect to item 63.

**65. `station.kind` is single-valued and stations are multi-modal.** §10 gives a
station one kind from `pick`, `put`, `induct`, `extract`. Map Creator states a
product type plus a list of `allowed_modes`, and the two do not line up.
Across three sites, **24 stations allow more than one of pick and put, or
neither** (11 of `owen`'s 17, all 10 of `warehouse_map`'s, 3 of `lakeside`'s) —
e.g. `owen` pps 1, type `manual`, modes `["put","pick","audit"]`, and `lakeside`
pps 11, type `manual`, modes `["audit"]`, a mode §10's vocabulary does not
contain at all.
**What the importer deliberately did not do:** it did not pick one. Those stations
import as a placed entity with a `pose` and `external_ids` and *no* `station`
component, with a record comment saying the kind was not derivable — a partial
carry rather than a guess or a drop, since the location is derivable and the kind
is not. Recommendation: rule whether `kind` becomes a set, whether `audit` joins
the vocabulary, and whether the mode list is world data at all or a policy overlay
— a station's modes are the sort of thing that changes per shift.

**66. A station cannot name the queues that serve it, nor the transports allowed
inside it.** Two references the export makes and the format cannot. **40 stations
across three sites name their serving blocks** (`entities.blocks`), e.g.
`apotok-prod` pps 123 served by blocks `["49","50"]` — the queues themselves
import from `block.json`, and nothing in `station` points at them, so the imported
station and its queues are unrelated entities that happen to sit near each other.
And **38 stations carry `associations`**, the one place the whole export reifies a
relationship: a typed connected graph (`{"type":"allowed_transport","entities":
["block-3","presentation_point-1","block-4"],"connections":[{"from":"block-3",
"to":"presentation_point-1"},…]}`) over station-local handles. §2 asks for exactly
that shape with entity refs at the endpoints, and v0 has no component to put it
in. Recommendation: a `station.serves`/`station.queues` ref list is the small half;
the association graph is a component of its own, and the format already has the
identity discipline the export's stringly handles lack.

**67. `presentation_point` carries one field and the export states eight.** §10
gives it `for`, and marks explicit cell binding `OPEN`. Every one of the **50
presentation points across three relay sites** carries, besides `presented_for`:
`presentation_height`, `bot_type`, `pick_direction`, `is_workbench`,
`light_curtain`, `bot_orientation` (on `walmart-pot`), `presented_entity_allowed_types`
and `presenting_agent_type` — e.g. `apotok-prod` pps 123 point 1, height 950,
`bot_type: HTM_QT_M5F`. Butler reads several of them: it derives the point's
ttp/rtp type from `presented_entity_allowed_types` and its sub-type from
`presenting_agent_type` plus `is_workbench`, both with silent fall-through
defaults. The import keeps `for` and the pose and drops the rest.
Recommendation: the height and the presenting class are physical and belong on the
component; the reach direction and the workbench flag are arguably station policy;
`light_curtain` is safety equipment and probably belongs with item 62's domain
rather than here.

**68. Which product generation and which vendor a site is remain undeclarable, and
every consumer still sniffs.** `research/mapcreator-format.md` closes on this and
nothing has changed: **70 stations across all five sites declare a `system` or
`eligible_system`** (`["relay"]` on the three relay sites, `["rtp"]` on the two
older ones), e.g. `apotok-prod` pps 123 declaring `["relay"]`, and no component
carries it. The importer works the generation out the way everyone does — relay
sites ship `location_profiles`, `tote_storage` and `vtm_zone`, RTP sites do not —
and the vendor the way everyone does, by reading `QT` or `HAI` out of a bot-type
string, which it records as an invented value rather than as a fact it read. This
item is evidence, not a proposal: a product-identity component (working name
**lineage**, a chain of `{party, role, designation, options}` entries on cataloged
products) is being ruled at architect level and would cover the vendor half
properly, including the rebrand case (one class, two names). What the evidence adds
is that the *generation* half is a separate axis — relay versus RTP is a fleet and
storage architecture, not a manufacturer — and that a consumer needs it to refuse
a document it cannot simulate rather than half-parse one.

**69. A frame can be nominal or it can be recorded, and the vendor frame is
recorded per cell and drifts.** §4 already rules the legacy grid frame nominal and
lossy by declaration, and the import confirms the ruling is the right one: one
step of `apotok-prod`'s grid index stands for **189 different millimetre
distances** across its own adjacent cells, the commonest covering 622 of 13,382
steps, so the best straight-line fit projects **every one of its 4,190 on-grid
cells** onto a different square than the export records — while `warehouse_map`,
whose grid really is a 1,650 × 1,380 mm lattice, fits **1,882 of 1,882 exactly**.
Nothing is lost, because §4 says a cell's recorded grid coordinate rides as a cell
attribute, and it does. The vendor frame is the different case: `vda_world_coordinate`
is stored *per cell*, and one declared transform cannot reproduce it —
`apotok-prod`'s cells state **125 distinct (Kx, Ky) pairs**, the modal one covering
1,198 of 5,055, so **3,857 recorded vendor positions differ from what the declared
frame projects**; `walmart-pot` states 4 pairs, 5 mm apart, over 100 cells. The
positions were accumulated cell by cell along different measurement paths and
drifted. The importer declares the modal transform and carries every cell's
recorded value in `external_ids`, so a consumer has both and no way to know which
to believe. This is §12.9's as-measured question arriving with numbers: butler
stores robot-measured geometry beside declared geometry unmarked, and here the
export does the same thing to a coordinate frame. Recommendation: rule §12.9, and
note that the answer needs to cover a *measured position in a frame*, not only a
measured extent — an overlay document of per-cell corrections is one shape, a
`measured` flag beside a pose is another.

## References and cell addresses

**70. A reference is checked for pointing at something and not for pointing at
the right kind of thing, except where this crate decided otherwise.** §2 makes a
dangling reference a load error and leaves it to each component's schema to say
which fields hold references — "not guessable from shape" — but says nothing
about what a reference field may require of its target. Three fields are checked
for frame-ness here, because a pose measured in something that is not a frame
cannot be projected at all: `frame.parent`, `pose.frame`, `region.frame`.
`robot_class.native_frame` is checked only for existing, and the comment at that
field in `references.rs` says as much: nothing decides whether a class's native
frame has to be a frame. So a document naming a station there loads under this
implementation and could reasonably be refused by the next one, which is the
disagreement §2 exists to prevent. Recommendation: §10's robot-class row already
calls it a "native vendor frame", so let §2 say that a reference field may name
the kind of entity it points at, and mark this one.

**71. §2's reference vocabulary covers one of the two kinds of pointer the
format has.** A reference, in §2, is an identity inside a component field, and a
dangling one is a load error. A *cell address* is defined separately in §3, in
three spellings — a cell's own identity, a derivation address in two parts, a
template-local address —
and §2 never says whether it is a reference. This crate keeps the two apart on
purpose. `references::of` is the loader's list, the one a dangling-reference
refusal is raised from; `references::cells_addressed` is the other one, left to
the checks that run against the merged world, because those can say which of
three things went wrong — the step was never a cell, its cell was excluded by
the tiling that would have derived it, or the address reaches nothing at all —
where a dangling-reference refusal can only say "does not exist". A test pins
the split by name
(`the_loaders_reference_list_leaves_cell_addresses_to_the_checks_that_can_explain_them`).
An implementation that folds cell addresses into the reference list contradicts
nothing §2 says and loses all three diagnostics. Recommendation: one sentence in
§2 naming both kinds of pointer and saying that a cell address is resolved where
the world is known, so the better diagnostics are the specified behaviour rather
than this implementation's taste.

## Depth and the face it counts from

**72. Depth counts from "the access face", and a run may have no access.** §5
gives a rack `slots_deep` and says depth "counts from the access face (below),
which is what makes double-deep 'front' and 'rear' well-defined". The face is
the **access**'s, and an access is a separate entity somebody has to declare: a
run with none is not an error and not even unusual — it is the ordinary shape of
a converted legacy site, where 3,300 of 3,609 runs have no continuous open lane
on their outward face (item 53). Nothing else in the format names that face. A
rack's relay levels each name one, but a relay level's face is deliberately the
*other* one — §5 says so in as many words, because in a rack | aisle | rack zone
the climbing robot works the inner face and the handoff happens on the outer —
so reading depth from it would count from the wrong side exactly where the
distinction matters.

v0.5 hid this: `access_side` sat on the rack itself, so every rack carried a
face whether or not anybody reached it. Slimming the access to a statement that
derives nothing is what exposed the hole, and it is a hole in the *meaning* of
`slot.depth` rather than in anything this crate computes — the crate emits
`depth` as an index and never resolves it to a side, so a double-deep rack with
no access loads, and its front and rear slots are two indices with no way to say
which is which. The spec does not invent a rule here and this entry does not
either.

Recommendation: rule one of three. Depth counts from the access face and a
double-deep rack **must** have an access, which makes an accessless one a load
error and would refuse the converted fixture set. Or depth counts from the
access face when there is one and is otherwise undefined, which is what ships —
say so, so two implementations agree about the silence. Or `slots_deep > 1`
gains a face of its own on the rack, which is a fourth face in a format that
just finished separating two.

## Filed from the simulator integration

> The two entries below come from the consuming side rather than the reference
> reader: dessim and desmap hit both against real maps, and each is a place a
> second implementation could already have decided differently. Evidence lives
> on the integration branch and is cited in place.

**73. A queue path can name cells its own station makes unusable, and nothing
checks.** §10's `queue.path` is an ordered list of cell addresses, and the only
validation those addresses get is resolution: name a cell that exists and the
queue is well formed. What the cells are FOR is never examined — nothing
requires a queue cell to admit the station's carrier class, and nothing objects
when one is another entity's bound cell (a relay's standing cell, a docking
cell) or stands inside a footprint whose owner plainly did not intend traffic
queued on it.

The failure this reaches was hit in the old format on 2026-08-11: a station
footprint placed flush against racking, whose queue stamped cells across five
rack rows, and the first thing to notice was a panic in scenario generation --
two layers and one vocabulary away from the authoring mistake. The protocol
dissolves part of that case by construction: floor that is never tiled has no
cell, so an address into it fails resolution and refuses the load, which is
already better. The residue is the case where the cells exist legitimately --
tiled floor under a rack overhang, a neighbour's access cell -- and the queue
is legal, loadable, and wrong, with the mistake surfacing wherever the first
consumer trips over it.

Recommendation: a §7 check, warning severity, two clauses. Every `queue.path`
cell must list a `traversable_by` class compatible with what the station's
ports move; and a queue cell that is another entity's `access_cell` is worth
naming out loud, because two machines being sent to the same square by two
different vocabularies is exactly the collision a check exists to catch. Both
clauses are one pass over resolved addresses against components the expansion
already carries.

**74. Lateral clearance is declared, not measured -- the horizontal half of the
question item 29 asked vertically.** §7 (v0.6) checks a robot class's height
against the lowest level of any rack whose footprint its cell_grid crosses:
the vertical case is a MEASUREMENT of declared geometry against a declared
machine. The horizontal case has no counterpart. Whether a class fits BETWEEN
the members holding that rack up is stated only as `traversable_by` -- a claim
with nothing behind it -- and a tiling's `exclusions` cannot express the
members either, because they remove cells by centre and an 80 mm leg contains
no cell centre. The rack itself cannot state the question's inputs: its
footprint is one rectangle, and what stands under a bay -- four 80 mm legs, or
a full-depth blade -- is not in the format at all (item 8 already notes a rack
cannot be drawn; it cannot be driven under for the same reason).

The number that makes this worth a rule rather than a nicety, measured on the
integration branch (dessim, Walmart DFW6 geometry): the choice between legs
and a blade under otherwise identical racking moves the compiled traversal
graph by 29% -- 3,069 under-rack adjacencies against 2,173 -- which is not a
modelling detail but the difference between a warehouse with north-south
relief lanes and one without. Today two implementations converting the same
site drawing MUST invent that answer independently, in opposite directions,
and both claim conformance.

Recommendation: two halves, matching the shape v0.6 gave the vertical case.
First, racks gain member geometry as measured fact -- what interrupts the
floor under a bay and how wide it is -- under the same as-measured provenance
question §12.9 already holds open. Second, the §7 clearance check gains the
horizontal clause: a class whose `footprint` does not pass between a rack's
declared members, on a cell whose `traversable_by` admits it, is the same
contradiction as a class taller than the steel above it, and should warn the
same way.

