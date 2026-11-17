#!/usr/bin/env python3
"""Cut the Cencora DA50 world documents with the two buffer aisles Dematic
asked for on 2026-10-12: aisle 20 at the east end of the main row, and aisle 21
standing alone in the L portion south of the corridor.

`sim/dessim/tests/fixtures/cencora_v3_da50_baseline.json` is the 19-aisle
document of record and stays untouched: the 19-aisle scenarios run from it.
This script reads it and writes one sibling, `cencora_v3_da50_aisles21.json`,
the baseline plus aisles 20 and 21 and less the floor where aisles 22 to 29
stand, so that a run on the larger map differs from the baseline by those
alone. Dematic asked for 19 and 21 aisles and nothing in between, so no
20-aisle document is written.

Both come from the drawing `9172329_Cencora_NDC2__BTP_Layout_With_All_Aisles`
(Dematic, 2026-10-12), read in notes/cencora-scenario5-2026-10-13/GEOMETRY.md.
The drawing's other eight annex sandwiches, aisles 22 to 29, are not part of
the simulation (Daniel, 2026-10-13): no racks, DA50s, sectors or relays are cut
for them. HTMs cannot drive where they stand either (Vinay, 2026-10-14), so
this cut takes the two regions they occupy out of the floor (Daniel,
2026-10-14).

Aisle 20 is the twentieth sandwich of the main row, one pitch east of the
nineteenth, the same length and height, with two floor cells between it and
the floor's east edge. None of the nineteen moves: the drawing's first nineteen
aisle centres span 119,750 mm and this document's span 120,700, and from aisle
19's east rack edge to the floor edge the drawing has 7,625 mm where this cut
lays 7,650. On this document's grid:

  - both floor bands grow from 130,050 to 134,300 mm (153 to 158 columns);
  - the sandwich at [130050, 17102], 2,550 x 12,840, built from the same parts
    as zones 1 to 19: footprint and inner-aisle regions, the two rack runs
    (six bays each), a sector, a cohort of two DA50s, the two access faces,
    and the 72 relay access cells;
  - the end patch of the 19-aisle map (gap 19, x 126,650) becomes the lane
    shared by zone 1's east face and aisle 20's west face, as every interior
    gap is; a new two-column patch (gap 20, x 132,600) carries aisle 20's east
    face, with seams to its neighbours, to both bands and to the rack faces;
  - the bands carve out the new footprint and the rack-end clips exactly as
    they do for the other nineteen.

Aisle 21 is the west sandwich of the annex's upper row: in the drawing it
stands directly below the main corridor, between the sixteenth and seventeenth
aisles of the row (our zones 4 and 3), with two lane cells to its west and open
floor beyond, and four lane cells to its east. On this document's grid it is a
strip of nine columns cut out of the lower band's L portion:

  - the sandwich at [108800, 40040] (its north end at the L's first row, where
    the drawing's annex racks begin right below the corridor), the same parts
    as the other zones;
  - a two-column patch west of it (x 107,100) and a four-column patch east
    (x 111,350), one per bay, seamed to each other and to the band where floor
    is left beside them: the west lane along its whole length, onto the L
    floor; the east lane only at its ends, onto the corridor to the north and
    the corridor between the annex rows to the south, because the floor east
    of it goes with aisles 22 to 24;
  - the band loses the strip's cells under the steel, the aisle and the
    patches: no rack-end clips, because the drawing has floor right up to
    these racks.

Aisles 22 to 29 are two regions of the lower band with nothing drawn in
them. In the drawing the upper annex row holds aisle 21 and, east of it,
aisles 22 to 24; a corridor separates it from the lower row, aisles 25 to 29,
with another corridor along the south wall. On this document's grid:

  - the region of aisles 22 to 24 is everything east of aisle 21's east lane,
    from the corridor's south edge (the L's first row) down to the region of
    aisles 25 to 29 and out to the floor's east edge. It takes the lanes
    between those three with it, and the stretch of the corridor between the
    rows east of aisle 21, which serves nothing once they are gone;
  - the region of aisles 25 to 29 is everything south of the corridor between
    the rows, across the L: the lower row, its lanes, the strip west of aisle
    25 and the corridor along the south wall. Its north edge is where the
    lower row's steel begins, mapped as aisle 21's racks are (anchored at the
    L's first row, the drawing's lengths scaled by this document's rack
    length over the drawing's) and moved to the nearest row line;
  - what is left of the corridor between the rows runs from the put stations
    to aisle 21's east lane: put station 26 queues and leaves through it, and
    aisle 21's lanes open onto it.

Each entity the cut makes records its role in the cut (`aisle:20/footprint`,
`gap:21w/patch:3`, `aisle:21/rack:left/bay:2/relay-seam`) as its external id
`cencora_aisles_role`. A run reads the committed 21-aisle document first and
gives each entity the id that the entity holding the same role has there, and
the document its own id again; only a role the document does not hold yet gets
a fresh random (v4) id, and a role two entities claim is refused. Ids are
identities, never hashes of a name (Daniel, 2026-09-11), and a rerun on an
unchanged baseline writes the same bytes. The document is written with a
two-space indent, sorted keys, non-ASCII as itself and no trailing newline; the
baseline keeps the key order of cencora_v3.json, which is not sorted, so
compare the two as JSON, not as text.

Run from the repository root:

    python3 scripts/add_cencora_aisles.py [--dry-run]
"""

import argparse
import copy
import json
import sys
import uuid
from pathlib import Path

BASELINE = Path("sim/dessim/tests/fixtures/cencora_v3_da50_baseline.json")
AISLES21 = Path("sim/dessim/tests/fixtures/cencora_v3_da50_aisles21.json")

ROLE = "cencora_aisles_role"  # the external id each entity of the cut records its role under

FRAME = "d29e03f4-f6b3-5572-933e-d3ff1ce9d08e"
UPPER_BAND = "b41b19f6-f81a-57c7-89d6-6b3882d261fe"
LOWER_BAND = "34e878ad-7604-5555-81bc-9f5c03999008"
LEFT_RACK_TEMPLATE = "2e339b4f-5f7d-5240-807a-58aa05ec7732"  # boards on the +y face
RIGHT_RACK_TEMPLATE = "dce713ff-48c5-586e-8836-9b40eae945b4"  # boards on the -y face
PATCH_TEMPLATE = "1d6b47d3-0df1-5ca0-8e7f-55943c7947ae"
DA50_CLASS = "e13e68d7-05dc-5659-8793-8802e6c6b7d3"

PITCH = 850
RACK_Y = 17102
RACK_LEN = 12840
SANDWICH_W = 2550
BAY_STEP = 2140
BAYS = 6
PATCH_LEN = 1935  # 3 rows at 645
CLIP_H = 1700
SEAM_SLACK = 50
BAY_SEAM_TOLERANCE = 255  # the 205 mm between one bay's patch and the next, plus slack

# What the baseline is expected to hold going in. Each is a fact a later edit
# could move, and this cut laid onto a moved floor would put cells where no
# floor is, so they are stated here rather than read back out of the file.
EXPECT_ZONES = 19
EXPECT_BAND_WIDTH = 130050
EXPECT_PATCHES = 144
EXPECT_ACCESS_CELLS = 1368
EXPECT_STITCHES = 420
EAST_GAP_X = 126650  # the 19-aisle map's end patch, four columns wide

# Aisle 21's strip in the L portion, on the lower band's own 850 grid.
AISLE21_X = 108800  # column 128
AISLE21_Y = 40040  # the L's first row (band row 12): the corridor ends here
AISLE21_WEST_COLUMNS = 2
AISLE21_EAST_COLUMNS = 4

# The annex rows in the drawing, as steel y. The upper row (aisle 21, then
# aisles 22 to 24) is anchored here at the L's first row, as aisle 21 is, and
# the drawing's lengths shrink by this document's rack length over the
# drawing's (12,840 over 13,800).
UPPER_ROW_DRAWN = (25600, 39400)  # south to north
LOWER_ROW_NORTH_DRAWN = 22000  # where the steel of aisles 25 to 29 begins
L_WEST_X = 96900  # the L's west edge: the band's first clip takes everything west of it

CLIP_COMMENT = "The only way the old format can say \"not our building\"."


class Ids:
    """The ids of the entities the cut makes, by role: the id the committed
    21-aisle document gives the entity holding a role, or a fresh v4 id for a
    role it does not hold."""

    def __init__(self, committed):
        self.known = {}
        for entity in (committed or {}).get("entities", []):
            role = (comp(entity, "external_ids") or {}).get(ROLE)
            if role is None:
                continue
            if role in self.known:
                raise SystemExit(f"{AISLES21}: two entities claim the role {role}")
            self.known[role] = entity["id"]
        self.taken = {}

    def take(self, role):
        assert role not in self.taken, f"the cut makes {role} twice"
        self.taken[role] = self.known.get(role) or str(uuid.uuid4())
        return self.taken[role]

    def report(self):
        kept = sum(1 for role in self.taken if role in self.known)
        lines = [f"  ids: {kept} kept from the committed document, {len(self.taken) - kept} new"]
        gone = sorted(set(self.known) - set(self.taken))
        if gone:
            lines.append(f"  roles the cut no longer makes, their ids dropped: {', '.join(gone)}")
        return lines


def role_ids(role, **others):
    """An entity's external ids: its role in the cut, and any others it has."""
    return dict(others, **{ROLE: role})


def comp(entity, name):
    return (entity.get("components") or {}).get(name)


def render(document):
    return json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False)


def check(document):
    entities = document["entities"]
    by_id = {e["id"]: e for e in entities if "id" in e}
    problems = []
    sectors = [e for e in entities if comp(e, "sector")]
    if len(sectors) != EXPECT_ZONES:
        problems.append(f"{len(sectors)} sectors, expected {EXPECT_ZONES}")
    for band in (UPPER_BAND, LOWER_BAND):
        width = by_id[band]["components"]["region"]["size"][0]
        if width != EXPECT_BAND_WIDTH:
            problems.append(f"band {band[:8]} is {width} wide, expected {EXPECT_BAND_WIDTH}")
    patches = [e for e in entities if comp(e, "stamp") and comp(e, "stamp")["template"] == PATCH_TEMPLATE]
    if len(patches) != EXPECT_PATCHES:
        problems.append(f"{len(patches)} lane patches, expected {EXPECT_PATCHES}")
    cells = [e for e in entities if comp(e, "access_cell")]
    if len(cells) != EXPECT_ACCESS_CELLS:
        problems.append(f"{len(cells)} relay access cells, expected {EXPECT_ACCESS_CELLS}")
    stitches = [e for e in entities if comp(e, "stitch")]
    if len(stitches) != EXPECT_STITCHES:
        problems.append(f"{len(stitches)} stitches, expected {EXPECT_STITCHES}")
    east = [e for e in patches if comp(e, "pose")["position"][0] == EAST_GAP_X]
    if len(east) != BAYS:
        problems.append(f"{len(east)} patches in the east gap at x={EAST_GAP_X}, expected {BAYS}")
    return problems, by_id


def band_exclusion(origin, size, comment):
    return {"$comment": comment, "origin": list(origin), "size": list(size)}


def floor_seam(ids, role, a, b, tolerance):
    return {
        "components": {
            "external_ids": role_ids(role),
            "stitch": {
                "$comment": (
                    "A flat seam where one piece of floor hands over to the next (SPEC §5). The tolerance is "
                    "this seam's own gap plus slack, not one figure for the whole floor: reach further than the gap "
                    "and `contact` can pair a cell with a diagonal neighbour instead of the partner straight across."
                ),
                "a": a,
                "b": b,
                "rule": "contact",
                "tolerance": tolerance,
            }
        },
        "id": ids.take(role),
        "name": "stitch: floor seam",
    }


def relay_seam(ids, role, rack_id, rack_name, bay, patch_id, column):
    """The three relay cells of one bay of a rack run paired with the patch
    cells straight across: column 0 of the patch east of a right face, the
    last column of the patch west of a left face."""
    pairs = [
        [
            {"generator": patch_id, "key": f"instance:0/local:floor/cell:{column},{row}"},
            {"generator": rack_id, "key": f"instance:{bay}/local:relay-access-{row}"},
        ]
        for row in range(3)
    ]
    return {
        "components": {
            "external_ids": role_ids(role),
            "stitch": {"a": rack_id, "b": patch_id, "pairs": pairs, "rule": "explicit"},
        },
        "id": ids.take(role),
        "name": f"relay access: {rack_name} bay {bay + 1}",
    }


def zone_parts(ids, entities, aisle, x, y, where):
    """Append one sandwich's zone at `(x, y)` (its north-west corner): the
    footprint and inner-aisle regions, two rack runs, sector, cohort of two,
    two access faces and 72 relay access cells. Returns the two rack ids and
    names, left (west) then right (east)."""
    tag = f"aisle:{aisle}"
    external = {"dematic_aisle": aisle}
    footprint_id = ids.take(f"{tag}/footprint")
    inner_id = ids.take(f"{tag}/inner-aisle")
    left_id = ids.take(f"{tag}/rack:left")
    right_id = ids.take(f"{tag}/rack:right")
    sector_id = ids.take(f"{tag}/sector")
    cohort_id = ids.take(f"{tag}/cohort")
    left_name = f"rack run: aisle {aisle} left face"
    right_name = f"rack run: aisle {aisle} right face"

    entities.append(
        {
            "$comment": (
                f"Aisle {aisle} of the Dematic drawing of 2026-10-12, {where}, cut by scripts/add_cencora_aisles.py. "
                "The zone's outline, kept as the record of where the steel stands; it takes no floor away: the "
                "blanket band states this rectangle among its own carve-outs, and the lanes beside it are patches "
                "never laid under the steel."
            ),
            "components": {
                "external_ids": role_ids(f"{tag}/footprint", **external),
                "region": {"frame": FRAME, "origin": [x, y], "size": [SANDWICH_W, RACK_LEN]},
            },
            "id": footprint_id,
            "name": f"aisle {aisle} footprint",
        }
    )
    entities.append(
        {
            "components": {
                "external_ids": role_ids(f"{tag}/inner-aisle"),
                "region": {
                    "$comment": (
                        "The strip the climbing class works from. It declares NO tiling, and that is the point: "
                        "tiling follows locomotion (SPEC §5), and giving this lane cells would put a way in the "
                        "graph that nothing drives."
                    ),
                    "frame": FRAME,
                    "origin": [x + PITCH, y],
                    "size": [PITCH, RACK_LEN],
                }
            },
            "id": inner_id,
            "name": f"inner aisle: aisle {aisle}",
        }
    )
    for rack_id, name, side, template, rack_x in (
        (left_id, left_name, "Left", LEFT_RACK_TEMPLATE, x + PITCH),
        (right_id, right_name, "Right", RIGHT_RACK_TEMPLATE, x + SANDWICH_W),
    ):
        face = "+y" if side == "Left" else "-y"
        entities.append(
            {
                "components": {
                    "external_ids": role_ids(f"{tag}/rack:{side.lower()}", desmap_rack_side=side, **external),
                    "pose": {"frame": FRAME, "heading_deg": 90, "position": [rack_x, y]},
                    "stamp": {
                        "$comment": (
                            f"Aisle {aisle}, {side.lower()} rack column, the same six bays as zones 1 to 19: "
                            f"prototype \"cencora_a5_bay\", 4 x 11 x 1 slots, 6 boards at 3 positions on {face}"
                        ),
                        "params": {},
                        "repeat": {"count": BAYS, "step": [0, BAY_STEP]},
                        "template": template,
                    },
                },
                "id": rack_id,
                "name": name,
            }
        )
    entities.append(
        {
            "components": {
                "external_ids": role_ids(f"{tag}/sector", **external),
                "sector": {
                    "$comment": (
                        "One sector per zone, because vtm_count is a per-zone number: the thing a cohort is "
                        "hard-assigned to has to be the zone. A generator member stands for its whole expansion."
                    ),
                    "members": [left_id, right_id],
                },
            },
            "id": sector_id,
            "name": f"sector: aisle {aisle}",
        }
    )
    entities.append(
        {
            "components": {
                "external_ids": role_ids(f"{tag}/cohort"),
                "cohort": {
                    "$comment": "Two DA50s, as every other aisle: four more for the two aisles Dematic asked for.",
                    "assignment": {"kind": "hard", "to": [sector_id]},
                    "count": 2,
                    "robot_class": DA50_CLASS,
                }
            },
            "id": cohort_id,
            "name": f"cohort: aisle {aisle} DA50s",
        }
    )
    for rack_id, side, face in ((left_id, "left", "-y"), (right_id, "right", "+y")):
        entities.append(
            {
                "components": {
                    "external_ids": role_ids(f"{tag}/access:{side}"),
                    "access": {
                        "$comment": (
                            "The face the storage is worked from, onto the aisle. It derives nothing — the "
                            "climb is rack-internal motion — but it is what makes the zone's vtm_count mean "
                            "something, and it is where slot depth counts from. The other face, the one the boards "
                            "present on, is stated on the steel."
                        ),
                        "along": inner_id,
                        "face": face,
                        "rack": rack_id,
                        "serves": [DA50_CLASS],
                    }
                },
                "id": ids.take(f"{tag}/access:{side}"),
                "name": f"access: aisle {aisle} {side} <-> inner aisle",
            }
        )
    for rack_id in (left_id, right_id):
        for bay in range(BAYS):
            for row in range(2):
                for column in range(3):
                    entities.append(
                        {
                            "components": {
                                "access_cell": {
                                    "cell": {"generator": rack_id, "key": f"instance:{bay}/local:relay-access-{column}"}
                                }
                            },
                            "derivation": {"generator": rack_id, "key": f"instance:{bay}/local:rack/relay:{row},{column}"},
                        }
                    )
    return left_id, right_id, left_name, right_name


def lane_patches(ids, entities, gap, x, y, columns):
    """One patch per bay at `(x, y)`, seamed bay to bay. Returns the patch ids,
    north to south."""
    patch_ids = []
    for bay in range(BAYS):
        role = f"gap:{gap}/patch:{bay}"
        patch_id = ids.take(role)
        patch_ids.append(patch_id)
        entities.append(
            {
                "components": {
                    "external_ids": role_ids(role),
                    "pose": {"frame": FRAME, "position": [x, y + bay * BAY_STEP]},
                    "stamp": {"params": {"width": columns * PITCH}, "template": PATCH_TEMPLATE},
                },
                "id": patch_id,
                "name": f"open patch: gap {gap} bay {bay + 1}",
            }
        )
    for bay, (first, second) in enumerate(zip(patch_ids, patch_ids[1:])):
        role = f"gap:{gap}/patch:{bay}/seam:patch:{bay + 1}"
        entities.append(floor_seam(ids, role, first, second, BAY_SEAM_TOLERANCE))
    return patch_ids


def add_aisle_20(ids, document, by_id):
    """Aisle 20 at the east end of the main row, with a new end patch of two
    columns east of it. Returns the lines to print."""
    entities = document["entities"]
    lines = []
    aisle, x = 20, EXPECT_BAND_WIDTH
    left_id, right_id, left_name, right_name = zone_parts(
        ids, entities, aisle, x, RACK_Y, "one pitch east of the nineteen the baseline carries"
    )
    lines.append(f"  aisle {aisle}: sandwich at [{x}, {RACK_Y}], two rack runs, 72 relay access cells, 2 DA50s")

    # The lane east of the new sandwich, seamed to both bands and carrying
    # the right face's relay cells.
    patch_x = x + SANDWICH_W
    patch_ids = lane_patches(ids, entities, aisle, patch_x, RACK_Y, 2)
    upper_gap = RACK_Y - by_id[UPPER_BAND]["components"]["region"]["size"][1]
    lower_gap = by_id[LOWER_BAND]["components"]["region"]["origin"][1] - (RACK_Y + (BAYS - 1) * BAY_STEP + PATCH_LEN)
    entities.append(
        floor_seam(ids, f"gap:{aisle}/patch:0/seam:upper-band", patch_ids[0], UPPER_BAND, upper_gap + SEAM_SLACK)
    )
    entities.append(
        floor_seam(
            ids, f"gap:{aisle}/patch:{BAYS - 1}/seam:lower-band", patch_ids[-1], LOWER_BAND, lower_gap + SEAM_SLACK
        )
    )
    lines.append(
        f"  gap {aisle}: {BAYS} patches of 2 columns at x={patch_x}, seams to each other "
        f"(tolerance {BAY_SEAM_TOLERANCE}), to the upper band ({upper_gap + SEAM_SLACK}) and the lower band "
        f"({lower_gap + SEAM_SLACK})"
    )

    # Relay seams: the right face onto the new patch's first column, the left
    # face onto the last column of the patch already standing west of it.
    west_patches = sorted(
        (
            e
            for e in entities
            if comp(e, "stamp")
            and comp(e, "stamp")["template"] == PATCH_TEMPLATE
            and comp(e, "pose")["position"][0] + comp(e, "stamp")["params"]["width"] == x
        ),
        key=lambda e: comp(e, "pose")["position"][1],
    )
    assert len(west_patches) == BAYS, f"{len(west_patches)} patches end at x={x}, expected {BAYS}"
    west_last_column = comp(west_patches[0], "stamp")["params"]["width"] // PITCH - 1
    for bay in range(BAYS):
        left_role, right_role = (f"aisle:{aisle}/rack:{side}/bay:{bay}/relay-seam" for side in ("left", "right"))
        entities.append(relay_seam(ids, left_role, left_id, left_name, bay, west_patches[bay]["id"], west_last_column))
        entities.append(relay_seam(ids, right_role, right_id, right_name, bay, patch_ids[bay], 0))
    lines.append(
        f"  relay seams: left face onto column {west_last_column} of the gap at x="
        f"{comp(west_patches[0], 'pose')['position'][0]}, right face onto column 0 of gap {aisle}"
    )

    # The bands: wider, and carved around the new steel as around the others.
    new_width = patch_x + 2 * PITCH
    for band in (UPPER_BAND, LOWER_BAND):
        by_id[band]["components"]["region"]["size"][0] = new_width
    upper = by_id[UPPER_BAND]["components"]["tiling"]["exclusions"]
    lower = by_id[LOWER_BAND]["components"]["tiling"]["exclusions"]
    upper_h = by_id[UPPER_BAND]["components"]["region"]["size"][1]
    lower_y = by_id[LOWER_BAND]["components"]["region"]["origin"][1]
    upper.append(band_exclusion((x, upper_h - CLIP_H), (SANDWICH_W, CLIP_H), f"aisle {aisle} north clip: {CLIP_COMMENT}"))
    lower.append(band_exclusion((x, 0), (SANDWICH_W, CLIP_H), f"aisle {aisle} south clip: {CLIP_COMMENT}"))
    lower.append(
        band_exclusion(
            (x, RACK_Y - lower_y),
            (SANDWICH_W, RACK_LEN),
            f"aisle {aisle} footprint: the floor has no cells under the steel or in the aisle at all.",
        )
    )
    lines.append(f"  bands: {EXPECT_BAND_WIDTH} -> {new_width} wide ({new_width // PITCH} columns), three carve-outs added")
    return lines


def add_aisle_21(ids, document, by_id):
    """Aisle 21 standing alone in the L portion: a strip of the lower band
    becomes two lane patches with the sandwich between them. Returns the lines
    to print."""
    entities = document["entities"]
    lines = []
    aisle, x, y = 21, AISLE21_X, AISLE21_Y
    band = by_id[LOWER_BAND]
    band_x, band_y = band["components"]["region"]["origin"]
    band_w, band_h = band["components"]["region"]["size"]
    west_x = x - AISLE21_WEST_COLUMNS * PITCH
    east_x = x + SANDWICH_W
    strip_x0, strip_x1 = west_x, east_x + AISLE21_EAST_COLUMNS * PITCH
    strip_y0, strip_y1 = y, y + RACK_LEN
    for edge, name in ((strip_x0, "west"), (strip_x1, "east"), (strip_y0, "north")):
        origin = band_x if name in ("west", "east") else band_y
        assert (edge - origin) % PITCH == 0, f"the strip's {name} edge {edge} is not on the band's grid"
    assert strip_x1 <= band_x + band_w and strip_y1 <= band_y + band_h, "the strip leaves the band"

    left_id, right_id, left_name, right_name = zone_parts(
        ids, entities, aisle, x, y, "standing alone in the L portion south of the corridor"
    )
    lines.append(f"  aisle {aisle}: sandwich at [{x}, {y}], two rack runs, 72 relay access cells, 2 DA50s")

    # The lanes: a patch is seamed to the band where floor is left beside it.
    # The first ones touch the corridor to the north and the last ones the
    # corridor between the annex rows to the south; in between, the west lane
    # touches the L floor west of it, and the east lane touches nothing,
    # because the floor east of it goes with aisles 22 to 24 (block_out_annex)
    # and the loader refuses a seam that links no cells.
    # The band keeps the first row whose centre clears the strip; the gap is
    # from the last patch row's end to that row's start.
    first_kept_row = (strip_y1 - band_y - PITCH // 2) // PITCH + 1
    south_gap = (band_y + first_kept_row * PITCH) - (y + (BAYS - 1) * BAY_STEP + PATCH_LEN)
    assert 0 < south_gap < PITCH, f"south gap {south_gap}"
    west_ids = lane_patches(ids, entities, f"{aisle}w", west_x, y, AISLE21_WEST_COLUMNS)
    east_ids = lane_patches(ids, entities, f"{aisle}e", east_x, y, AISLE21_EAST_COLUMNS)
    for side, lane, bays in (("w", west_ids, range(BAYS)), ("e", east_ids, (0, BAYS - 1))):
        for bay in bays:
            gap = south_gap if bay == BAYS - 1 else 0
            role = f"gap:{aisle}{side}/patch:{bay}/seam:lower-band"
            entities.append(floor_seam(ids, role, lane[bay], LOWER_BAND, gap + SEAM_SLACK))
    lines.append(
        f"  gaps {aisle}w and {aisle}e: {BAYS} patches each, {AISLE21_WEST_COLUMNS} columns at x={west_x} and "
        f"{AISLE21_EAST_COLUMNS} at x={east_x}; seamed to the band every west patch and the two end east "
        f"patches (tolerance {SEAM_SLACK}, the last bay {south_gap + SEAM_SLACK})"
    )
    for bay in range(BAYS):
        left_role, right_role = (f"aisle:{aisle}/rack:{side}/bay:{bay}/relay-seam" for side in ("left", "right"))
        entities.append(relay_seam(ids, left_role, left_id, left_name, bay, west_ids[bay], AISLE21_WEST_COLUMNS - 1))
        entities.append(relay_seam(ids, right_role, right_id, right_name, bay, east_ids[bay], 0))
    lines.append(
        f"  relay seams: left face onto column {AISLE21_WEST_COLUMNS - 1} of gap {aisle}w, right face onto "
        f"column 0 of gap {aisle}e"
    )

    band["components"]["tiling"]["exclusions"].append(
        band_exclusion(
            (strip_x0 - band_x, strip_y0 - band_y),
            (strip_x1 - strip_x0, strip_y1 - strip_y0),
            f"aisle {aisle} strip: the lanes are patches and the floor has no cells under the steel or in the "
            "aisle at all; the drawing has floor right up to these racks, so no rack-end clips.",
        )
    )
    lines.append(
        f"  lower band: strip [{strip_x0}, {strip_y0}] to [{strip_x1}, {strip_y1}] carved out "
        f"({(strip_x1 - strip_x0) // PITCH} columns)"
    )
    return lines


def annex_y(drawn):
    """A drawing y in the annex, mapped onto this document's floor as aisle
    21's racks are: the upper row's north end on the L's first row, lengths
    scaled by this document's rack length over the drawing's."""
    scale = RACK_LEN / (UPPER_ROW_DRAWN[1] - UPPER_ROW_DRAWN[0])
    return AISLE21_Y + round((UPPER_ROW_DRAWN[1] - drawn) * scale)


def lower_band_cells(document, band_x, band_y):
    """The centre of every lower-band cell some entity names, and of the cell
    each presentation point exits onto: the floor the stations on this band
    queue, present and leave on."""
    stations = {
        e["id"]: comp(e, "station")["exit_direction"] for e in document["entities"] if comp(e, "station")
    }
    steps = {"+x": (1, 0), "-x": (-1, 0), "+y": (0, 1), "-y": (0, -1)}
    named = []

    def walk(value, owner):
        if isinstance(value, dict):
            key = value.get("key")
            if value.get("generator") == LOWER_BAND and isinstance(key, str) and key.startswith("cell:"):
                column, row = (int(n) for n in key[len("cell:") :].split(","))
                named.append((column, row))
                point = (owner.get("components") or {}).get("presentation_point")
                if point and point.get("station") in stations:
                    dx, dy = steps[stations[point["station"]]]
                    named.append((column + dx, row + dy))
            for child in value.values():
                walk(child, owner)
        elif isinstance(value, list):
            for child in value:
                walk(child, owner)

    for entity in document["entities"]:
        walk(entity, entity)
    half = PITCH // 2
    return [(band_x + c * PITCH + half, band_y + r * PITCH + half, c, r) for c, r in named]


def annex_regions(band_x, band_y, band_w, band_h):
    """The two regions of the annex the simulation does not run, as (name, x0,
    y0, x1, y1, comment) in the floor frame."""
    # The lower row's steel begins on the row line nearest to where the
    # drawing puts it.
    lower_north = band_y + round((annex_y(LOWER_ROW_NORTH_DRAWN) - band_y) / PITCH) * PITCH
    east_lane_end = AISLE21_X + SANDWICH_W + AISLE21_EAST_COLUMNS * PITCH
    floor_x1, floor_y1 = band_x + band_w, band_y + band_h
    return (
        (
            "aisles 22 to 24",
            east_lane_end,
            AISLE21_Y,
            floor_x1,
            lower_north,
            "aisles 22 to 24: the upper annex row east of aisle 21's east lane, out to the floor's east edge, "
            "with the lanes between those three sandwiches and the stretch of the corridor between the annex rows "
            "south of them. Dematic's drawing has three sandwiches of planned buffer-board racks here that the "
            "simulation does not run, so nothing is drawn, and HTMs cannot drive anywhere in it, so the whole region "
            "is taken out of the floor.",
        ),
        (
            "aisles 25 to 29",
            L_WEST_X,
            lower_north,
            floor_x1,
            floor_y1,
            "aisles 25 to 29: everything south of the corridor between the annex rows, across the L: the lower "
            "row's five sandwiches of planned buffer-board racks, the lanes between them, the strip west of aisle 25 "
            "and the corridor along the south wall. The simulation does not run these racks, so nothing is drawn, "
            "and HTMs cannot drive anywhere in it, so the whole region is taken out of the floor.",
        ),
    )


def block_out_annex(document, by_id):
    """Take the two regions of aisles 22 to 29 out of the lower band's floor.
    Returns the lines to print."""
    band = by_id[LOWER_BAND]
    band_x, band_y = band["components"]["region"]["origin"]
    band_w, band_h = band["components"]["region"]["size"]
    exclusions = band["components"]["tiling"]["exclusions"]

    # The L's west edge is where the band's own clip stops taking floor away.
    assert any(
        e["origin"][0] <= 0
        and e["origin"][0] + e["size"][0] == L_WEST_X - band_x
        and e["origin"][1] <= AISLE21_Y - band_y
        and e["origin"][1] + e["size"][1] >= band_h
        for e in exclusions
    ), f"no clip ends the floor west of x={L_WEST_X} below the corridor"

    regions = annex_regions(band_x, band_y, band_w, band_h)
    lines = []
    for name, x0, y0, x1, y1, comment in regions:
        for edge, origin in ((x0, band_x), (y0, band_y), (x1, band_x), (y1, band_y)):
            assert (edge - origin) % PITCH == 0, f"the region of {name} has an edge off the band's grid"
        exclusions.append(band_exclusion((x0 - band_x, y0 - band_y), (x1 - x0, y1 - y0), comment))
        lines.append(
            f"  region of {name}: [{x0}, {y0}] to [{x1}, {y1}] carved out, nothing drawn "
            f"({(x1 - x0) // PITCH} columns by {(y1 - y0) // PITCH} rows)"
        )

    # What is left of the corridor between the rows: the band's rows whose
    # centres fall between aisle 21's south end and the lower region.
    half = PITCH // 2
    lower_north = regions[1][2]
    kept = [r for r in range(band_h // PITCH) if AISLE21_Y + RACK_LEN < band_y + r * PITCH + half < lower_north]
    assert kept, "the corridor between the rows lost every row"
    lines.append(
        f"  the corridor between the rows keeps band rows {kept[0]} to {kept[-1]}, west of x={regions[0][1]}"
    )

    # No station loses the floor its queue, presentation point or exit stands on.
    for x, y, column, row in lower_band_cells(document, band_x, band_y):
        for _, x0, y0, x1, y1, _ in regions:
            assert not (x0 < x < x1 and y0 < y < y1), f"band cell {column},{row}, which a station uses, is carved"
    return lines


def with_doc(document, doc_id, name, comment_prefix):
    document["doc"] = dict(document["doc"], id=doc_id, name=name)
    document["$comment"] = comment_prefix + document["$comment"]
    return document


def cut(baseline, committed):
    """The 21-aisle document and its report lines, or (None, problems).
    `committed` is the 21-aisle document as committed, or None: the ids it
    holds are kept."""
    aisles21 = copy.deepcopy(baseline)
    problems, by_id = check(aisles21)
    if problems:
        return None, problems
    ids = Ids(committed)
    lines = add_aisle_20(ids, aisles21, by_id)
    by_id = {e["id"]: e for e in aisles21["entities"] if "id" in e}
    lines += add_aisle_21(ids, aisles21, by_id)
    lines += block_out_annex(aisles21, by_id)
    lines += ids.report()
    with_doc(
        aisles21,
        committed["doc"]["id"] if committed else str(uuid.uuid4()),
        "cencora_v3 DA50 world with Dematic's aisles 20 and 21 (21 aisles, 42 DA50s)",
        "The 19-aisle DA50 document of record (cencora_v3_da50_baseline.json) plus aisle 20 at the east end of the "
        "row and aisle 21 standing alone in the L portion, with the two regions the annex's other eight aisles "
        "(22 to 29) occupy taken out of the floor and nothing drawn for them, cut by scripts/add_cencora_aisles.py "
        "from Dematic's "
        "drawing of 2026-10-12; edit the baseline and re-run the script, never this file. The baseline's own "
        "comment follows: ",
    )
    return [(AISLES21, aisles21, lines)], None


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dry-run", action="store_true", help="say what would be written, write nothing")
    args = parser.parse_args()
    if not BASELINE.exists():
        print(f"{BASELINE}: not here — run this from the repository root", file=sys.stderr)
        return 1
    baseline = json.loads(BASELINE.read_text(encoding="utf-8"))
    committed = json.loads(AISLES21.read_text(encoding="utf-8")) if AISLES21.exists() else None
    documents, problems = cut(baseline, committed)
    if documents is None:
        print(f"{BASELINE}: refused, the document is not the one this cut expects:", file=sys.stderr)
        for line in problems:
            print(f"    {line}", file=sys.stderr)
        return 1
    for path, document, lines in documents:
        print(f"{path}:")
        for line in lines:
            print(line)
        print(f"  {len(document['entities'])} entities ({len(baseline['entities'])} in the baseline)")
        if args.dry_run:
            print("  --dry-run, nothing written")
            continue
        path.write_text(render(document), encoding="utf-8")
        print("  written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
