# Walmart floor boundaries — September 8, 2026

The installed change includes the rack floor and both highways, as well as the
station apron. Inventory rack/open-floor approaches to the highways are aligned.
The broader request to align all adjoining floor regions remains unfinished:
**275 compiled links across the six boundary groups below still have lateral
centre offsets.** The apron/highway boundary is one of those groups; the floor
north of the inventory racks also needs work.

These are geometry findings. They do not establish that each affected boundary
is a throughput bottleneck. That requires traffic, reservation-wait and queue
measurements on the routes actually used.

## Scope and evidence

This audit reads the installed
[`walmart_dfw6.json`](../../sim/dessim/tests/fixtures/walmart_dfw6.json), the
[alignment patch](../../scripts/dfw6_floor_alignment.py), and the
[alignment assertions](../../sim/dessim/tests/walmart_floor_alignment.rs).
It examines actual expanded cells and compiled edges, rather than reproducing
contact-matching rules from the source geometry. No map, patcher or test was
changed and no new build or simulation was run for this audit.

The existing `worldprotocol-check` binary expanded the map successfully with
`--expand`; its output was read recursively through the derived entities.
One compiled bidirectional edge counts as one link here. Distances below are
absolute differences between cell centres, converted from the 0.1 mm primary
unit. A north/south boundary is aligned when the two centres have the same X;
a lateral boundary is aligned when they have the same Y. This audit does not
classify intentional EasyLoop curved links as floor-seam defects.

Map SHA-256 at this checkpoint:
`4dfc36adcabebb4590dcd4c99fdb8bf0b21b3385e18b05a9107c2b3da882aa51`.

The installed geometry contains:

- 97 inventory corridor regions: 20 VTM aisles, 74 under-rack service/travel
  regions and three open-floor regions. Each follows the 21 physical bay
  sections, providing 82 rows per column.
- 106 columns across each highway: 424 south-highway cells and 742
  north-highway cells. Column widths range from 698.0 to 1,227.0 mm.
- 104 station-apron columns, with three rows each: 312 cells. Its column
  centres follow the station doors rather than the highway columns.

## Boundaries already aligned

| Boundary | Compiled links | Centre offset along boundary |
| --- | ---: | ---: |
| Inventory service strips ↔ south and north highways | 74 | X = 0.0 mm |
| Inventory travel lanes ↔ south and north highways | 74 | X = 0.0 mm |
| Inventory open floor ↔ south and north highways | 24 | X = 0.0 mm |
| Inventory service/travel/open-floor lateral seams | 4,762 | Y = 0.0 mm |
| Buffer service/travel strips ↔ north highway | 30 | X = 0.0 mm |
| Buffer strip lateral seams | 80 | Y = 0.0 mm |
| EasyLoop doors and box-induction points ↔ apron | 26 | X = 0.0 mm |
| Parking/other approach cells ↔ three band-floor regions | 7 | X = 0.0 mm |

The 4,762 inventory lateral links comprise 492 open-floor/travel links, 170
service/service links, 3,034 service/travel links and 1,066 travel/travel links.
The 172 inventory-to-highway links establish alignment at both ends of all 74
under-rack regions and all three open-floor regions; the open-floor regions
contribute multiple columns.

All 20 inventory VTM aisle centre lines also coincide geometrically with highway
columns. **There are no compiled VTM-aisle-to-highway links**, however. Geometric
alignment does not yet create a cross-aisle VTM floor route. The current VTM
runtime still bypasses floor navigation and its reservations; this audit adds
no new VTM collision claim.

## Boundaries still misaligned

All links in each row below are diagonal: both X and Y differ. The Y separation
is the normal centre-to-centre distance across the boundary, not a physical gap.

| Boundary | Links / diagonal | Absolute X offset | Y separation |
| --- | ---: | ---: | ---: |
| Station apron ↔ south highway | 104 / 104 | 4.4–502.1 mm | 850.0 mm |
| Three band-floor regions ↔ apron | 7 / 7 | 0.9–269.5 mm | 888.7 mm |
| Floor west of buffer ↔ north highway | 67 / 67 | 3.0–448.5 mm | 741.6 mm |
| Floor west of buffer ↔ putaway floor | 67 / 67 | 3.9–425.4 mm | 741.6 mm |
| Buffer service strips ↔ putaway floor | 15 / 15 | 3.3–417.7 mm | 741.6 mm |
| Buffer travel lanes ↔ putaway floor | 15 / 15 | 52.6–407.5 mm | 741.6 mm |

The apron/highway, north-highway/buffer and buffer/putaway region envelopes
abut. The three band-floor regions end 0.4 mm before the apron. The remaining
problem is the different lateral centre grids, rather than missing metres of
floor between their envelopes.

### Region identities

| Region | Declared ID |
| --- | --- |
| station apron | `b1f0a2c4-7d3e-5a91-8c62-4e5f0a1b2c3d` |
| south highway | `007c700a-ab07-5e4b-9d10-72b625e15145` |
| north highway | `d41c643c-272c-5e3c-8e9c-81409549e38f` |
| floor west of the buffer | `6fc75207-c1dc-5067-bdbf-eeda860545da` |
| putaway floor | `82907ea8-1f76-5eaa-805f-fbc2386e2c9c` |
| band floor, columns 40-42 | `88642e55-56bb-58fa-8b86-0e250851f2a6` |
| band floor, columns 53-54 | `41e527b3-5d84-591a-b20d-6df6800db9b5` |
| band floor, columns 73-74 | `ac4222a0-d0f9-53cd-9559-6a3e56e27bf3` |

### Stitch identities

These are the declared owners of the affected compiled links.

| Stitch | Declared ID |
| --- | --- |
| `stitch:apron-shwy` | `c2e1b3d5-8e4f-5ba2-9d73-5f6a1b2c3d4e` |
| `stitch:band floor at column 40 to the apron` | `fdf1d6c7-d9ab-5424-b096-cf4aae9b88df` |
| `stitch:band floor at column 53 to the apron` | `84a6a93a-6039-501e-9410-fc06cb50f0a1` |
| `stitch:band floor at column 73 to the apron` | `de75595d-cad6-5216-9966-6a12280bcbe4` |
| `stitch:bwest-nhwy` | `a3d4c528-5453-59c9-83ee-4e2f71fba8f1` |
| `stitch:bwest-nfloor` | `d4c3a04e-67b8-58b5-aec8-e3310e1dd87c` |

For the 30 buffer-to-putaway owners, the full name is
`stitch:buffer-svc-nfloor:<index>` or `stitch:buffer-trv-nfloor:<index>`:

| Index | Service stitch ID | Travel stitch ID |
| ---: | --- | --- |
| 0 | `91de6c2c-d3d1-597f-9f04-e1f3ee0b2427` | `3df650e7-369c-58f1-bb11-904e80885701` |
| 1 | `d059f58d-208e-5f18-bd03-3bfb6115c6eb` | `5e5b9f66-a84b-583e-972a-76f3406faec0` |
| 2 | `161c61c6-8675-5828-980f-7c8eea3e2778` | `fc9761b7-1af1-54be-a4aa-635df17b1e04` |
| 3 | `3ba2ea38-f23e-520e-8ba8-f40072ae586b` | `2782b4a8-ab9b-5b59-bf15-4891c78e8ee7` |
| 4 | `91fb64b1-045b-56be-8d2f-8aaf597e1998` | `f5857151-f7a5-5929-af01-82015e0ab268` |
| 5 | `79c7d6ba-c0cc-50ef-93dd-b9eacd823158` | `c3c705d7-ffc0-5548-bf4a-d20477ae43b2` |
| 6 | `f1fa3b27-8878-597c-a473-772253a65543` | `eee47ce5-80ce-580c-9f0f-0e3799e138eb` |
| 7 | `f1814753-a89a-5026-8ada-7d3a1f494e3e` | `a7e045d5-7a11-5d32-9501-0d8c2a832302` |
| 8 | `4b01f879-d4cc-5926-a5a3-cb057d8d367b` | `d99a2481-2f1d-5226-a2cb-4c02733f77e0` |
| 9 | `b9e6f3b4-9ece-59cf-9e89-7b5ebc738e8b` | `5df228f6-36fb-592d-b63f-a3390fc8963e` |
| 10 | `b4c83acf-0151-50ae-834e-352a1b610bc6` | `6d29f25d-9437-5cc2-87e9-eb5548db6c84` |
| 11 | `adf28a06-cc98-5e26-997f-6af1facfd310` | `1e97a7ad-8700-57c1-8b21-d7fbab9c83d8` |
| 12 | `b16648b6-02ab-5fb8-add5-e9246d6dd58c` | `e56d92f1-c611-5e47-bfa2-d0d99f2f4912` |
| 13 | `a0976a21-5b09-512c-994b-6c5c67648728` | `1d70d9a0-fb1b-5af9-9ee7-3ad95f9edbe8` |
| 14 | `d48eb6ac-15d2-5026-bf02-b147db16741f` | `477b90fc-0d20-5738-830e-4c0460bf1aa4` |

### Largest-offset examples

Each example gives the actual two derived cell IDs for the worst offset in
that boundary group. The labels are the compiled edge-owner names.

- `stitch:band floor at column 40 to the apron`: **269.5 mm** X offset.
  From `88642e55-56bb-58fa-8b86-0e250851f2a6/cell:2,8` to `b1f0a2c4-7d3e-5a91-8c62-4e5f0a1b2c3d/instance:0/local:column-41/cell:0,0`.
- `stitch:buffer-svc-nfloor:12`: **417.7 mm** X offset.
  From `78cafe4c-7d50-5709-8ee1-42ef9d8f9f6b/cell:0,3` to `82907ea8-1f76-5eaa-805f-fbc2386e2c9c/cell:100,0`.
- `stitch:buffer-trv-nfloor:0`: **407.5 mm** X offset.
  From `98f39c6f-ac94-5616-9ac8-58e2264a3894/cell:0,3` to `82907ea8-1f76-5eaa-805f-fbc2386e2c9c/cell:68,0`.
- `stitch:bwest-nhwy`: **448.5 mm** X offset.
  From `6fc75207-c1dc-5067-bdbf-eeda860545da/cell:34,0` to `d41c643c-272c-5e3c-8e9c-81409549e38f/instance:0/local:column-33/cell:0,6`.
- `stitch:bwest-nfloor`: **425.4 mm** X offset.
  From `6fc75207-c1dc-5067-bdbf-eeda860545da/cell:55,3` to `82907ea8-1f76-5eaa-805f-fbc2386e2c9c/cell:56,0`.
- `stitch:apron-shwy`: **502.1 mm** X offset.
  From `b1f0a2c4-7d3e-5a91-8c62-4e5f0a1b2c3d/instance:0/local:column-20/cell:0,2` to `007c700a-ab07-5e4b-9d10-72b625e15145/instance:0/local:column-19/cell:0,0`.

## What the current tests do not establish

`floor_role` in `walmart_floor_alignment.rs` recognizes inventory under-rack
service strips, inventory travel lanes, inventory open floor and the two
highways. It classifies the apron, the three band-floor regions, buffer strips,
west-buffer floor and putaway floor as `Other`.

Consequently:

- `rack_floor_lateral_seams_and_highway_approaches_are_orthogonal` checks the
  aligned inventory boundaries above. It skips all six remaining boundary
  groups. Its 77-region assertion is 74 under-rack regions plus three open-floor
  regions; it is not a claim about every floor region in the site.
- `easyloop_dropoff_and_collection_have_independent_straight_apron_approaches`
  checks eight inventory ports. It does not check apron-to-highway alignment,
  the 16 order-lane doors or the two box points. The latter 18 doors were checked
  separately in this compiled-graph audit and do align with their apron cells.
- The overlap assertion includes recognized floor roles plus the apron. It
  excludes the band floor, buffer, west-buffer floor and putaway floor.
- The bay-gap assertion establishes row continuity and direction for 97
  corridors; it does not assert alignment of every adjoining floor region.
- The relay assertion checks all 2,132 HTM stands and matching same-Y VTM aisle
  cells. It does not assert a VTM connection from those aisles to the highways.

Passing those tests is useful evidence for their stated subcases, but cannot
close the user's whole-boundary alignment request. A follow-up should inventory
all intended floor adjacencies, assert both their existence and orthogonal
centres, and include those same regions in footprint/overlap checks. Tests must
also preserve lane direction and distinct station approaches after retiling.

## Can shifting, stretching or splitting resolve the remaining seams?

### A union of the current centres is not a feasible common row

The nearest pair in the union of all apron and highway X centres is only
**4.4 mm apart**: X = 24,694.5 and 24,698.9 mm. Even retaining only the 26
mandatory door centres alongside the highway centres yields a nearest pair
**21.9 mm apart**: X = 71,229.2 and 71,251.1 mm.

Two adjacent, centred rectangles each at least approximately 650 mm wide need
at least 650 mm between their centres to avoid overlap. Merely unioning centres
would therefore create tiny filler cells or overlapping cells. Approximately
650 mm is the design constraint for this comparison, not a newly established
robot dimension. The current highway's narrowest 698 mm column meets it.

### Minor whole-EasyLoop transforms are insufficient on their own

A bounded numerical search translated each EasyLoop's six door centres by
−500 to +500 mm, in 1 mm steps. It also tested an X scale from 0.950 to 1.050,
in 0.001 steps, about the mean door X, with the same translation range. The
objective was to minimize the largest remaining distance to a highway centre.
All distances are in global floor X, including mirrored stations.

| EasyLoop | Best translation alone | Remaining worst error | Best X scale | Translation with scale | Remaining worst error |
| --- | ---: | ---: | ---: | ---: | ---: |
| easyloop 1 | -146 mm | 251.4 mm | 0.950 | -89 mm | 144.23 mm |
| easyloop 2 | -84 mm | 295.5 mm | 1.029 | -72 mm | 278.11 mm |
| easyloop 3 | -258 mm | 130.5 mm | 1.025 | -229 mm | 106.42 mm |
| easyloop 4 | +2 mm | 265.7 mm | 0.983 | -8 mm | 240.44 mm |

This bounded search is not a proof against every possible transform. It does
show that a small translation or uniform X stretch does not eliminate the
mismatch. Scaling the physical conveyor and presentation turn geometry simply
to fit a floor partition would also change previously reviewed dimensions.

### Local station approaches have a plausible common-grid option

Matching each of the 26 doors independently to its nearest highway centre
selects **26 distinct columns**. It therefore need not reintroduce a shared
apron approach. The candidate's per-station maximum movement and minimum
spacing between selected centres are:

| Station | Largest individual X move | Minimum spacing of its six selected centres |
| --- | ---: | ---: |
| easyloop 1 | 397.4 mm | 856.3 mm |
| easyloop 2 | 371.4 mm | 749.0 mm |
| easyloop 3 | 360.5 mm | 749.0 mm |
| easyloop 4 | 267.7 mm | 790.0 mm |

The two box points would move −232.6 and −368.5 mm in global X. One candidate
is to carry the highway's 106-column partition through the apron and locally
reconfigure the station approach/queue cells to those columns. The apron
would extend from 92,102.4 to 92,112.1 mm in width: only 9.7 mm of envelope
change. The station cell moves are larger and need physical review: the
conveyor pickup/dropoff endpoints, under-conveyor queue, order-lane direction,
turn curves and waiting-block reachability must still agree.

These distances establish a possible partition, not a safe completed map
change. In particular, adjusting a port cell independently while leaving its
physical conveyor endpoint behind would only move the mismatch into the
station. Preserve the equipment geometry where possible and choose actual
local joins deliberately.

### The north floor can reuse an existing partition

The 106 highway columns already line up with all 30 buffer service/travel
strips. Their first **66** columns end at X = **57,659.3 mm**. The west-buffer
floor currently uses 67 columns of 860.5 mm and ends at **57,653.5 mm**.
Repartitioning that region with the 66-column highway prefix needs only a
**5.8 mm** eastward envelope adjustment.

Retiling the putaway floor with the same 106 columns can align both the
west-buffer floor and the 30 buffer strips at once. It needs the same
**9.7 mm** width extension noted for the apron. This is a narrowly scoped
floor-partition option with no reason to move rack steel. Its other boundaries,
especially putaway station approaches and any parking links, must be checked
so the change does not move the mismatch to the next row.

The three southern band-floor regions also need repartitioning against the
chosen apron columns, while retaining their parking connections and equipment
clearances. Their existing seven links do not become aligned merely because
the apron/highway seam is repaired.

### Alternative: explicit orthogonal transitions in open floor

Where moving an actual station endpoint would be wrong, an explicit lateral
transition can occupy existing open floor before joining the common grid. It
needs real intermediate cells and reservations, widths of at least the agreed
minimum, and ordinary perpendicular links through the intervening space.
Simply replacing a contact stitch with an explicit link between the same two
centres leaves the diagonal unchanged. Adding a long skipping link or moving
the diagonal one row inward does not satisfy the alignment intent either.

The north-side repartition is the simplest independent candidate. The station
side needs a choice between locally moving approach geometry and allocating
orthogonal transition space. Neither option has been applied in this audit.

## Candidate station X adjustments

This is an addressable list for a subsequent geometry review, not an instruction
to shift these cells without updating their connected station geometry. Positive
X means east in the global floor frame. All measurements are millimetres.

| Door | Cell ID | Current X | Candidate highway X | Move X |
| --- | --- | ---: | ---: | ---: |
| box induction 1: presentation cell | `8ed54d3f-9f17-5e55-bca0-02acf402462e` | 47330.4 | 47097.8 | -232.6 |
| box induction 2: presentation cell | `47b9afab-81d6-5c1e-bcf0-cd339bcfbb9e` | 54152.8 | 53784.3 | -368.5 |
| easyloop 1: port:dropoff | `0b108f03-d52d-5505-940f-c91b6439a4e0` | 26447.2 | 26491.5 | +44.3 |
| easyloop 1: port:collect | `9141ab7d-9e68-525a-8e15-3ba7e5b7a9ec` | 27287.2 | 27347.8 | +60.6 |
| easyloop 1: order:0/0 | `c657d350-92b1-5b8b-8f37-5eca8f00c0f9` | 28188.7 | 28293.3 | +104.6 |
| easyloop 1: order:1/0 | `dafebb0d-d8fc-5bd0-ba85-db0a15438202` | 29362.1 | 29238.8 | -123.3 |
| easyloop 1: order:5/0 | `1bfd0bae-4b2d-5def-903f-060f7f526ba8` | 31299.3 | 31129.8 | -169.5 |
| easyloop 1: order:6/0 | `207c8210-2397-5ab9-9292-c54606cf4344` | 32472.7 | 32075.3 | -397.4 |
| easyloop 2: port:dropoff | `0a1b1729-a76a-5d7f-b1ca-77bde4b8c4ea` | 37731.6 | 37646.5 | -85.1 |
| easyloop 2: port:collect | `f1c1b23c-a7ef-590a-ac74-cd8828ed7ec3` | 38571.6 | 38395.5 | -176.1 |
| easyloop 2: order:0/0 | `cd61708c-3157-58e2-80be-58e0008c8140` | 39473.1 | 39844.5 | +371.4 |
| easyloop 2: order:1/0 | `1ce3bec8-a5d2-5b6e-a134-f33f6efb5bc2` | 40646.5 | 40858.0 | +211.5 |
| easyloop 2: order:5/0 | `875a5a1d-3986-5aee-8bad-8ae6a3e6e112` | 42583.7 | 42655.0 | +71.3 |
| easyloop 2: order:6/0 | `96a423ef-e8ea-5260-8476-f4bbe8966920` | 43757.1 | 43505.0 | -252.1 |
| easyloop 3: order:6/0 | `45f0656a-ba9c-5a5d-8f0d-17b008ba8ce7` | 55776.4 | 55547.8 | -228.6 |
| easyloop 3: order:5/0 | `fd852d2a-6fd3-5d50-b579-f3f2cf53a58e` | 56949.8 | 57310.3 | +360.5 |
| easyloop 3: order:1/0 | `a8450e22-8885-5546-aa05-d567aa86db99` | 58887.0 | 58759.3 | -127.7 |
| easyloop 3: order:0/0 | `3e687d6c-dc44-5438-baa0-7d8c481c043f` | 60060.4 | 59772.8 | -287.6 |
| easyloop 3: port:collect | `4d426f85-e176-575f-8f8c-617f0f91db60` | 60961.9 | 60786.3 | -175.6 |
| easyloop 3: port:dropoff | `2520bbfc-4b0e-5da4-ade2-107ebcccb39a` | 61801.9 | 61535.3 | -266.6 |
| easyloop 4: order:6/0 | `421dcbed-0b17-5bc4-8021-89b27be24b4a` | 65203.7 | 65011.3 | -192.4 |
| easyloop 4: order:5/0 | `608602c1-8332-5220-94d4-a6af8eaafa97` | 66377.1 | 66644.8 | +267.7 |
| easyloop 4: order:1/0 | `a93240f6-86f7-5a63-9d99-e0876559d330` | 68314.3 | 68434.1 | +119.8 |
| easyloop 4: order:0/0 | `3132ac9d-7a7e-5e14-ab67-d9e0f9febddb` | 69487.7 | 69224.1 | -263.6 |
| easyloop 4: port:collect | `4f719702-e182-5c81-8f97-0c6ba6c4971f` | 70389.2 | 70237.6 | -151.6 |
| easyloop 4: port:dropoff | `ff877f7a-3adb-59a7-ae2a-16f1d4750c8f` | 71229.2 | 71251.1 | +21.9 |
