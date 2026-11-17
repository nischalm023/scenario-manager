#!/usr/bin/env python3
"""Re-author DFW6's four EasyLoops as declared cells, and take the station floor
out from under them.

CURRENT FLOOR ALIGNMENT (September 8): use `--align-floor --audit` for the current
fixture, or `--align-floor --apply` against its pinned pre-image. This additional
plan aligns apron/highway and rack-bay floor partitions; its byte proof lives in
`dfw6_floor_alignment.py`. The original EasyLoop-only plan below is historical.
For subsequent internal machine layout changes, use `update_easyloop_layout.py`;
it preserves the aligned floor and the machines' boundary stitches.

WHY THIS EXISTS. `sim/dessim/tests/fixtures/walmart_dfw6.json` is hand-managed:
its generator (`gen_dfw6_worldproto.py`) is not in this repository and its inputs
are not either, so the file is edited by byte-proved scripts and never
regenerated (`dfw6-provenance.md`). Its canonical form is
`json.dumps(doc, indent=1, ensure_ascii=True) + "\\n"`, verified at 634,418 bytes
before this change, and every edit below is a byte span cut out of that text and
replaced with canonical JSON at the same indent. Nothing round-trips the whole
tree, so no byte nobody reviewed is rewritten.

WHAT IT CHANGES, and the ruling behind each (Daniel, 2026-09-04):

  1. "the easyloop should not have `station` at all, and it definitely should not
     use tiling" -- the four stamps of `template:easyloop` become, per machine, a
     frame carrying the `go.easyloop` envelope, 46 declared cells, three lanes
     carrying `queue { path: [ids] }`, and explicit stitches. The template and
     its `station:0`/`station:1`/`order_queue:*`/`waiting_block` internals go.
  2. "a3fbe378 [`station floor`] should be removed. I don't want the easyloop to
     overlap a big gridded area" -- the 985-cell band tiling goes, replaced by a
     two-row lane along the floor strip, and by the band's own floor in every
     gap the four machines leave -- "smaller gridded areas in between the
     easyloop", stopping dead at each envelope. Nothing tiles under a machine.
  3. "c7de24f1/instance:0/local:inventory_egress is incorrectly positioned" --
     the two ports return to the drawing's ramp end, one under each belt
     centreline, instead of the 1,705.6 mm that
     `scripts/separate_easyloop_handoff_cells.py` spread them to.

WHAT IT DOES NOT CHANGE. Nothing outside the band: no rack, no aisle, no highway,
no buffer, no putaway or extraction station, no fleet, no workload.

WHERE THE MACHINE'S GEOMETRY COMES FROM. `scripts/easyloop_geometry.py`, which
`scripts/gen_minimal_easyloop.py` imports too, so the small map's EasyLoop and
these four are one machine stated once. What this script decides is only where
each machine stands -- its frame's translation and mirror -- and which apron cell
each of its six doors opens onto.

THIS SCRIPT SUPERSEDES THREE OLDER PATCHERS, on 2026-09-04:
`add_easyloop_to_dfw6.py`, `separate_easyloop_handoff_cells.py` and
`add_flank_parking.py`. They are in git history.

THE PRE-IMAGE, AND WHAT `--audit` MEANS. This plan is a set of byte spans of
ONE document -- the blob recorded in `PRE_IMAGE_BLOB` below -- so it can only be
applied to that document, and `--apply`, `--emit` and the dry run all refuse
anything else by hash. `--audit` reads the recorded blob from git and asserts
two things, both or neither: that this script's own span scanner reproduces that
pre-image at record boundaries, and that applying this plan to it yields the
committed document byte for byte. It prints the hash of each. The committed
document therefore fails "is the pre-image" and passes "is the plan applied to
it", which together say where every byte of it came from.

    python3 scripts/reauthor_dfw6_easyloops.py                 # the plan
    python3 scripts/reauthor_dfw6_easyloops.py --emit /tmp/c.json
    python3 scripts/reauthor_dfw6_easyloops.py --audit         # provenance proof
    git cat-file blob <PRE_IMAGE_BLOB> > sim/dessim/tests/fixtures/walmart_dfw6.json
    python3 scripts/reauthor_dfw6_easyloops.py --apply         # rewrite in place
"""

import argparse
import hashlib
import json
import subprocess
import sys
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))
import easyloop_geometry as geom   # noqa: E402 - the path is set on the line above

DOC = REPO / "sim/dessim/tests/fixtures/walmart_dfw6.json"

# THE PRE-IMAGE. Every edit this script makes is a byte span of ONE document:
# `walmart_dfw6.json` as it stood before this branch touched it, which is the
# blob with git object id 4d9f2e9f040f6e6bfd6e7c97a38600db728b7f33 -- the file
# at commit 09e3b32e7, the last one before the re-authoring. The hash below is
# SHA-256 over those same bytes, recorded so `--audit` can refuse a document
# that is not that one instead of describing whatever it was handed.
#
# Read by OBJECT ID and not through a commit or a path: an object id names the
# bytes themselves, so the reading does not move when the branch is rebased,
# and a blob that has been rewritten cannot answer to it.
PRE_IMAGE_BLOB = "4d9f2e9f040f6e6bfd6e7c97a38600db728b7f33"
PRE_IMAGE_SHA256 = "ab8140ae56e85a9f510b0b0b6105282f03cb6f51b01fc4eed83a716573bb1ffe"
PRE_IMAGE_BYTES = 634418
# The reviewed September 7 map, before Daniel removed the three spare floor blocks.
MAP_REVIEW_SHA256 = "96358e8727de3031403f6dfcf005c0b2b1c4e6bcd7ca11aa567968a922751989"
# Audited map at f07c08f20, before moving presentations beside the inventory chutes.
PRESENTATION_REVIEW_SHA256 = "cede4361555679e2071dbc3d3b297d86db270f476ccae2cf468001dc328d017a"
# Audited map at 6dbdf9320, before aligning the travel columns and tightening turns.
LANE_REVIEW_SHA256 = "b2b0cf8b0502789c94d4a7b3874bc9b09afaae6f7fcbb460c894fe6af61f4d21"

# Daniel: remove 76c42638 (west), 8477de90 (between box-induction points),
# and f8f661ff (east). The middle block expanded from columns 57-61 to 56-62
# during box-induction cleanup, changing its id to 61f4c3f7; remove it in full.
REMOVED_FLOOR_SPANS = [(0, 29), (56, 62), (84, 107)]

# The document's canonical layout, verified in `dfw6-provenance.md`.
CANONICAL = dict(indent=1, ensure_ascii=True)

FLOOR_FRAME = "83ceb0d9-472a-51b5-a471-98640a248bde"
TEMPLATE = "1b3153ad-bac1-56ee-bed4-7862d68ef139"
STATION_FLOOR = "a3fbe378-b049-587c-8439-38f50ec84a06"
FLOOR_STRIP = "0a281c3e-03c4-5cba-8e3f-fd62b288822d"
STATION_APRON = "b1f0a2c4-7d3e-5a91-8c62-4e5f0a1b2c3d"

# The four stamps, in the order their names number them: name, the stamp record
# this machine replaces, the stamp's floor x in steps, whether the DWG block is
# inserted mirrored, and the `external_ids` the stamp carried.
#
# MIRRORING. The site plan inserts the block twice at scale (1, 1) and twice at
# (-1, 1), and the two mirrored inserts are the two easternmost machines
# (`desvis/src/easyloop_prop.rs:47-52`, and commit 855c98c4b: "two of the
# four DWG inserts are x-mirrored ... machines 3 and 4"). The prop's own charter
# calls drawing all four the same way "KNOWN WRONG" and says the fix is to
# record it in the document, which is what the `True`s below do: an AutoCAD
# insert mirrors about its own insert point, so the frame keeps its translation
# and flips `scale[0]`.
MACHINES = [
    ("easyloop 1", "c7de24f1-494e-5efd-8c8c-9e2140af0d6d", 267422, False, {"dfw6_easyloop": 1, "desmap_station_ids": [1]}),
    ("easyloop 2", "3e369efb-5878-5d58-b23a-792157d22e3b", 380266, False, {"dfw6_easyloop": 2, "desmap_station_ids": [2]}),
    ("easyloop 3", "0a870757-41e8-55aa-b793-4141e977757b", 604169, True, {"dfw6_easyloop": 3, "desmap_station_ids": [3]}),
    ("easyloop 4", "ebb7990c-f2f1-5181-b8b6-f069b1077f42", 698442, True, {"dfw6_easyloop": 4, "desmap_station_ids": [4]}),
]

# The two box induction stations, which survive as stations and gain cells:
# label, the station record, the pose-only presentation point it names (which
# becomes a declared cell), and the x that point stands at today.
BOX_INDUCTION = [
    ("box induction 1", "7212b2a4-f507-583d-ae13-d3896463ba87",
     "85402173-eaa8-5d9b-882a-50a8ac488ed3", 470972),
    ("box induction 2", "aacad52c-0690-5730-b7d7-0b1d850cf701",
     "c5cc5f06-521d-56d8-bef9-06c0c5eff54c", 537672),
]

# The contact stitches that stood in for stated doors, and the two that hung the
# floor strip and the apron off the band being deleted.
DEAD_STITCHES = [
    ("8347fd5d-e754-56bd-a473-86219c4c7f80", "stitch:easyloop-floor:0"),
    ("115c1ff8-48b4-5333-806b-97cce566c80a", "stitch:easyloop-floor:1"),
    ("564ff796-a5cb-5cb8-b6a4-a90ee3a1da4a", "stitch:easyloop-floor:2"),
    ("b61e518b-11a6-56d0-ac0f-b01fe0a05b6f", "stitch:easyloop-floor:3"),
    ("c5e00dff-a734-5301-b252-f736ca59983d", "stitch:parking-floor"),
    ("2a208299-d5e0-5982-8cbe-2e88d5816542", "stitch:floor-shwy"),
]

# Where the DWG block's origin sits in the floor frame, per machine: x is the
# stamp's own x plus this, y is this. Both are fixed by things already true --
# `ORIGIN_Y_STEPS` puts the block's ramp end (`V_Y_N`) on the port line the site
# already routes to, floor y 114275, and `ORIGIN_DX_STEPS` puts the block's west
# belt centreline where `desvis/src/easyloop_prop.rs:261` already draws it.
ORIGIN_DX_STEPS = 5450
ORIGIN_Y_STEPS = 128515

BAND = {
    # `station floor`, the tiling being removed.
    "origin": [0, 17000],
    "size": [921024, 101937],
    "pitch": [8528, 9267],
    "rows": 11,
    "cols": 108,
    # What replaces it, in two pieces, both on the SAME lattice at its own
    # phase so nothing that survives moves a step.
    #
    # A two-row lane along the southern floor strip. This preserves the layout
    # while allowing opposing travel without a single-file corridor.
    "lane_rows": 2,
}

# The rows of the band the box induction lanes stand on, which are the rows the
# document's own bare queue steps already name.
BOX_LANE_ROWS = [114307, 105040, 95773]
BOX_PRESENT_ROW = 86505


def mm(v):
    """Millimetres to this document's 0.1 mm steps, rounded half up once."""
    return int(v * 10 + 0.5) if v >= 0 else -int(-v * 10 + 0.5)


def eid(ns, name):
    """A stable identity per authored thing, so a re-run is a no-op."""
    return str(uuid.uuid5(ns, name))


def band_column(x_steps):
    """Which column of the band's lattice a floor x falls in. Half up, always.

    Python's `round` is half-to-even, and two of the eight port columns land on
    a half. Rounding by the machine's parity rather than by the number's is how
    two doors 840 mm apart quietly become one door.
    """
    col = int((x_steps - BAND["pitch"][0] / 2) / BAND["pitch"][0] + 0.5)
    return max(0, min(BAND["cols"] - 1, col))


def band_column_centre(col):
    return int(col * BAND["pitch"][0] + BAND["pitch"][0] / 2)


def apron_cell(x_steps):
    """The apron cell directly north of a floor x. Its lattice, not a search."""
    return {"generator": STATION_APRON, "key": f"cell:{band_column(x_steps)},0"}


# --------------------------------------------------------------------------
# One machine
# --------------------------------------------------------------------------

def to_floor(stamp_x, mirrored, x_mm, y_mm):
    """A point of the DWG block, in floor steps, for the machine at `stamp_x`."""
    scale = -1 if mirrored else 1
    return [stamp_x + ORIGIN_DX_STEPS + scale * mm(x_mm), ORIGIN_Y_STEPS + mm(y_mm)]


def machine_envelope(stamp_x, mirrored):
    """One machine's envelope as `(origin, size)` in FLOOR steps.

    Min and max of the transformed corners, not origin-plus-size in the
    machine's own frame: a rectangle written in a mirrored frame comes out with
    its origin on the east edge, and a region is min-corner anchored (SPEC §5).
    """
    ex, ey, ew, eh = geom.envelope()
    a = to_floor(stamp_x, mirrored, ex, ey)
    b = to_floor(stamp_x, mirrored, ex + ew, ey + eh)
    origin = [min(a[0], b[0]), min(a[1], b[1])]
    return origin, [abs(b[0] - a[0]), abs(b[1] - a[1])]


def envelopes(mirroring=None):
    """Every machine's envelope, as `(name, x0, y0, x1, y1)` in floor steps."""
    out = []
    for name, _stamp, stamp_x, mirrored, _ext in MACHINES:
        if mirroring is not None:
            mirrored = mirroring
        (ox, oy), (w, h) = machine_envelope(stamp_x, mirrored)
        out.append((name, ox, oy, ox + w, oy + h))
    return out


ROLE_COMMENT = {
    "dropoff": "An inventory tote transfers from its HTM onto the conveyor here. The HTM continues under the conveyor.",
    "collect": "An empty HTM receives a returning inventory tote, then leaves the machine.",
    "lane": "A traversable location; its directed links state the permitted movement.",
    "presentation": "An order tote stays on its HTM here. The curved approach pauses for picking and the curved departure completes the U-turn.",
    "waiting": "An order HTM waits here for this block's presentation. It can arrive from either lane and dispatch only into the descending lane.",
    "doorway": "An order-loop boundary cell. Its explicit link states entry or exit.",
}


def machine_entities(ns, name, stamp_x, mirrored, external_ids):
    """Every top-level entity one machine becomes, and its handle-to-id map."""
    n = int(name.split()[-1])
    scale = -1 if mirrored else 1
    machine = eid(ns, f"easyloop:{n}:frame")
    (ox, oy), (w, h) = machine_envelope(stamp_x, mirrored)

    ents = [{
        "id": machine,
        "name": name,
        "components": {
            "frame": {
                "dims": 2,
                "parent": FLOOR_FRAME,
                "units": "mm",
                "transform_to_parent": {
                    "scale": [scale, 1],
                    "translation": [stamp_x + ORIGIN_DX_STEPS, ORIGIN_Y_STEPS],
                },
                "$comment": "DWG block coordinates in 0.1 mm steps. Reflection places the machine; order circulation also reverses locally so physical entries remain on the left.",
            },
            "region": {
                "frame": FLOOR_FRAME,
                "origin": [ox, oy],
                "size": [w, h],
                "$comment": (
                    "In the FLOOR frame, not the machine's own, because that is the statement that "
                    "survives a mirrored insert: a region is min-corner anchored, and a rectangle "
                    "written in a mirrored frame comes out with its origin on the east edge."
                ),
            },
            "go.easyloop": {
                "part": "envelope",
                "loop_transit_secs": geom.LOOP_TRANSIT_SECS,
                "time_per_pick_secs": geom.TIME_PER_PICK_SECS,
                "$comment": (
                    "The machine IS this entity: its members are the cells posed in its frame, "
                    "which is a stated join and not a shared identity prefix. The belt is a timing "
                    "abstraction on the machine rather than on either half of it -- one L-shaped "
                    "twin conveyor carries every tote both pick places work."
                ),
            },
            "external_ids": external_ids,
        },
    }]

    by_handle = {}
    for handle, (x, y), role in geom.check_layout():
        cid = eid(ns, f"easyloop:{n}:cell:{handle}")
        by_handle[handle] = cid
        pose = {"frame": machine, "position": [mm(x), mm(y)]}
        comps = {
            "pose": pose,
            "cell": {"traversable_by": ["htm"], "size": [mm(geom.CELL_SIZE_MM), mm(geom.CELL_SIZE_MM)]},
        }
        if role == "dropoff":
            comps["port"] = {
                "direction": "sink",
                "of": [{"kind": "tote", "designation": "inventory"}],
                "into": machine,
            }
        elif role == "collect":
            comps["port"] = {
                "direction": "source",
                "of": [{"kind": "tote", "designation": "inventory"}],
                "from": machine,
            }
        elif role == "presentation":
            side = int(handle.split(":")[1])
            comps["presentation_point"] = {"for": "order_carrier"}
            comps["pick"] = {}
            # Heading is the tangent at the directly declared presentation cell.
            heading = geom.heading(side, mirrored)
            pose["heading_deg"] = heading
        elif role == "waiting":
            comps["waiting"] = {}
        comps["$comment"] = ROLE_COMMENT[role]
        ents.append({"id": cid, "name": f"{name}: {handle}", "components": comps})

    for handle, path, why in geom.lanes(mirrored):
        ents.append({
            "id": eid(ns, f"easyloop:{n}:{handle}"),
            "name": f"{name}: {handle.split(':', 1)[1]}",
            "components": {"queue": {
                "path": [by_handle[cell] for cell in path],
                "discipline": "no_reverse",
                "$comment": why,
            }},
        })

    for run, pairs, bidirectional, curve in geom.links(mirrored=mirrored):
        ents.append({
            "id": eid(ns, f"easyloop:{n}:stitch:{run}"),
            "name": f"stitch:{name} {run}",
            "components": {"stitch": {
                "a": by_handle[pairs[0][0]],
                "b": by_handle[pairs[-1][1]],
                "rule": "explicit",
                "pairs": [[by_handle[a], by_handle[b]] for a, b in pairs],
                "bidirectional": bidirectional,
                **({"curve": {"frame": machine, "center": [mm(v) for v in curve["center"]],
                              "clockwise": curve["clockwise"],
                              **{k: [mm(v) for v in curve[k]] for k in ('start', 'end') if k in curve}}} if curve else {}),
                "$comment": f"Every arc of the {run}, in the order an HTM takes them. Nothing "
                            f"inside this machine binds by proximity.",
            }},
        })

    at = dict((handle, xy) for handle, xy, _ in geom.cells())
    for handle, direction in geom.doors(mirrored):
        floor = apron_cell(to_floor(stamp_x, mirrored, *at[handle])[0])
        inside = by_handle[handle]
        pair = [floor, inside] if direction == "entry" else [inside, floor]
        ents.append({
            "id": eid(ns, f"easyloop:{n}:stitch:doorway:{handle}"),
            "name": f"stitch:{name} {handle} {direction}",
            "components": {"stitch": {
                "a": inside, "b": STATION_APRON, "rule": "explicit",
                "pairs": [pair], "bidirectional": False,
                "$comment": "Declared one-way boundary crossing. Inventory HTMs cannot leave before collection.",
            }},
        })
    return ents, by_handle


# --------------------------------------------------------------------------
# The band, without `station floor`
# --------------------------------------------------------------------------

def gap_runs(boxes, taken):
    """The band's columns that no machine and no box induction lane stands in,
    as maximal west-to-east runs.

    This is what "smaller gridded areas in between the easyloop" comes to
    (Daniel, 2026-09-04): the band's floor survives everywhere the machines are
    not, and stops dead at each envelope. A column is blocked if any part of it
    overlaps a machine's envelope -- so no tile of these ever stands under a
    machine -- or if a box induction lane or standing cell already claims it,
    since two cells on one patch of ground is the defect this change removes.
    """
    blocked = set(taken)
    for first, last in REMOVED_FLOOR_SPANS:
        blocked.update(range(first, last + 1))
    for col in range(BAND["cols"]):
        lo, hi = col * BAND["pitch"][0], (col + 1) * BAND["pitch"][0]
        if any(lo < x1 and x0 < hi for _n, x0, _y0, x1, _y1 in boxes):
            blocked.add(col)
    runs, run = [], []
    for col in range(BAND["cols"]):
        if col in blocked:
            if run:
                runs.append(run)
            run = []
        else:
            run.append(col)
    if run:
        runs.append(run)
    return runs


def between(boxes, columns, taken):
    """What bounds one run of columns on the west and on the east, for its own
    `$comment`: a machine, a box induction lane, or the edge of the site."""
    def name_at(col):
        lo, hi = col * BAND["pitch"][0], (col + 1) * BAND["pitch"][0]
        for name, x0, _y0, x1, _y1 in boxes:
            if lo < x1 and x0 < hi:
                return name
        return "a box induction lane" if col in taken else None
    west = name_at(columns[0] - 1) if columns[0] > 0 else "the west wall"
    east = name_at(columns[-1] + 1) if columns[-1] + 1 < BAND["cols"] else "the east wall"
    return west or "the west wall", east or "the east wall"


def band_entities(ns, boxes, taken):
    """What replaces `station floor`: a lane along the floor strip, and the
    band's floor in every gap the machines leave."""
    lane_top = BAND["origin"][1] + BAND["lane_rows"] * BAND["pitch"][1]
    lane = eid(ns, "band:parking lane")
    out = [
        {
            "id": lane,
            "name": "floor lane",
            "components": {
                "region": {
                    "frame": FLOOR_FRAME,
                    "origin": list(BAND["origin"]),
                    "size": [BAND["size"][0], BAND["lane_rows"] * BAND["pitch"][1]],
                },
                "tiling": {"pitch": BAND["pitch"], "derives": {"cell": {"traversable_by": ["htm"]}}},
                "$comment": (
                    "Ordinary traversable floor beside the southern strip. This lane keeps "
                    "the deleted band's lattice and phase. Its north edge stops 853 mm short "
                    "of the southernmost machine, so it does not tile beneath a machine."
                ),
            },
        },
        {
            "id": eid(ns, "band:parking lane:south"),
            "name": "stitch:floor strip to its lane",
            "components": {"stitch": {
                "a": FLOOR_STRIP, "b": lane, "rule": "contact", "tolerance": 12750,
                "$comment": "The floor strip connects to its adjacent traversable lane.",
            }},
        },
    ]
    runs = gap_runs(boxes, taken)
    for columns in runs:
        col0 = columns[0]
        west, east = between(boxes, columns, taken)
        cid = eid(ns, f"band:floor:col{col0}")
        out.append({
            "id": cid,
            "name": f"band floor, columns {col0}-{columns[-1]}",
            "components": {
                "region": {
                    "frame": FLOOR_FRAME,
                    "origin": [BAND["origin"][0] + col0 * BAND["pitch"][0], lane_top],
                    "size": [len(columns) * BAND["pitch"][0],
                             BAND["size"][1] - (lane_top - BAND["origin"][1])],
                },
                "tiling": {"pitch": BAND["pitch"], "derives": {"cell": {"traversable_by": ["htm"]}}},
                "$comment": (
                    f"The band's own lattice at its own phase, so no cell that survives moves a "
                    f"step. It runs from {west} to {east} and stops dead at each: the floor "
                    f"between the machines is what the site needs and the floor under them is "
                    f"what it does not, which is the whole of the 2026-09-04 ruling."
                ),
            },
        })
        out.append({
            "id": eid(ns, f"band:floor:col{col0}:south"),
            "name": f"stitch:band floor at column {col0} to the floor lane",
            "components": {"stitch": {
                "a": cid, "b": lane, "rule": "contact", "tolerance": 12750,
                "$comment": "Where this piece of floor meets the lane along the floor strip.",
            }},
        })
        out.append({
            "id": eid(ns, f"band:floor:col{col0}:north"),
            "name": f"stitch:band floor at column {col0} to the apron",
            "components": {"stitch": {
                "a": cid, "b": STATION_APRON, "rule": "contact", "tolerance": 12750,
                "$comment": "And its way north, replacing `stitch:floor-shwy`.",
            }},
        })
    return out, runs


def box_induction_columns(boxes):
    """Each box induction's presentation column, stepped clear of every machine.

    The station stands one band column east of the point it presents on, so the
    pair of columns moves together. Mirroring swings machine 3's body west
    across the column box induction 2 would otherwise use, and this is what
    steps around it -- and says so in the plan's `CONFLICTS` section.
    """
    out = {}
    for label, _station, _point, point_x in BOX_INDUCTION:
        start = band_column(point_x)
        for step in range(0, BAND["cols"]):
            for sign in (1, -1) if step else (1,):
                col = start + sign * step
                if col < 0 or col >= BAND["cols"]:
                    continue
                clear = True
                for c in (col,):
                    lo = band_column_centre(c) - BAND["pitch"][0] // 2
                    hi = lo + BAND["pitch"][0]
                    if any(lo < x1 and x0 < hi for _n, x0, _y0, x1, _y1 in boxes):
                        clear = False
                if clear:
                    out[label] = col
                    break
            if label in out:
                break
        else:
            sys.exit(f"nowhere clear of the machines for {label}'s lane")
    return out


def box_induction_entities(ns, columns):
    """One presentation cell each; boxes and dispatch will be implemented later."""
    out, lanes = [], {}
    for label, station_id, _point_id, _x in BOX_INDUCTION:
        n = label.split()[-1]
        px = band_column_centre(columns[label])
        present = eid(ns, f"boxinduct:{n}:cell:present")
        out.append({
            "id": present, "name": f"{label}: presentation cell",
            "components": {
                "pose": {"frame": FLOOR_FRAME, "position": [px, BOX_LANE_ROWS[0]], "heading_deg": 90.0},
                "cell": {"traversable_by": ["htm"], "size": BAND["pitch"]},
                "presentation_point": {"for": "box", "station": station_id},
                "$comment": "Future box appearance and HTM pickup point. No equipment or approach queue is implied.",
            },
        })
        out.append({
            "id": eid(ns, f"boxinduct:{n}:stitch"), "name": f"stitch:{label} apron access",
            "components": {"stitch": {"a": present, "b": STATION_APRON, "rule": "explicit",
                                         "pairs": [[present, apron_cell(px)]]}},
        })
        lanes[label] = ([], present, present, px, px)
    return out, lanes


# --------------------------------------------------------------------------
# Byte spans
# --------------------------------------------------------------------------

def entity_spans(text):
    """The byte span of every top-level entity record, keyed by its id.

    The document is `json.dumps(indent=1)`, so each record opens at column 2 with
    `\\n  {` and closes with `\\n  }`. Scanning for those is exact for this layout
    and refuses on anything else rather than guessing.
    """
    marker = '\n "entities": [\n'
    start = text.index(marker) + len(marker)
    spans = {}
    i = start
    while True:
        if text[i:i + 2] != "  ":
            break
        if text[i + 2] != "{":
            sys.exit("an entity record must open with a brace; the layout is not canonical")
        depth, j, in_str, esc = 1, i + 3, False, False
        while depth:
            ch = text[j]
            if in_str:
                if esc:
                    esc = False
                elif ch == "\\":
                    esc = True
                elif ch == '"':
                    in_str = False
            elif ch == '"':
                in_str = True
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            j += 1
        record = json.loads(text[i + 2:j])
        if "id" in record:
            spans[record["id"]] = (i + 2, j)
        if text[j:j + 2] == ",\n":
            i = j + 2
            continue
        break
    return spans, start


def render(record, indent_level=2):
    """One record as the document already writes it: indent=1, at this depth."""
    pad = " " * indent_level
    return "\n".join(pad + line for line in json.dumps(record, **CANONICAL).split("\n"))


# --------------------------------------------------------------------------
# The plan
# --------------------------------------------------------------------------

def build_plan(text):
    """Every edit this change makes, as spans, plus what to append."""
    doc = json.loads(text)
    ns = uuid.UUID(doc["doc"]["id"])
    spans, _ = entity_spans(text)
    boxes = envelopes()
    columns = box_induction_columns(boxes)
    taken = {c for label in columns for c in (columns[label],)}

    replace, delete = [], []
    for name, stamp_id, stamp_x, mirrored, external_ids in MACHINES:
        ents, _handles = machine_entities(ns, name, stamp_x, mirrored, external_ids)
        replace.append((stamp_id, ents, f"{name}: one stamp -> {len(ents)} declared entities"))

    band, runs = band_entities(ns, boxes, taken)
    replace.append((STATION_FLOOR, band,
                    f"station floor -> a floor lane and {len(runs)} pieces of floor between the "
                    f"machines ({len(band)} entities)"))

    delete.append((TEMPLATE, "template:easyloop, with its station, queue and waiting internals"))
    for dead, why in DEAD_STITCHES:
        delete.append((dead, why))
    for label, _station, point_id, _x in BOX_INDUCTION:
        delete.append((point_id, f"{label}'s pose-only presentation point, which resolved by "
                                 f"nearest way cell; it becomes a declared cell"))

    appends, lanes = box_induction_entities(ns, columns)

    # Keep box-induction identities and kinds, with a single pickup cell each.
    by_id = {e["id"]: e for e in doc["entities"]}
    for label, station_id, _point, _x in BOX_INDUCTION:
        record = json.loads(json.dumps(by_id[station_id]))
        record["components"].pop("queue", None)
        record["components"]["pose"]["position"] = [lanes[label][3], BOX_LANE_ROWS[0]]
        replace.append((station_id, [record], f"{label}: one presentation cell, no approach queue"))

    # No "has this already been re-authored?" check here any more: `main` and
    # `audit` both hand this function the recorded pre-image and nothing else,
    # pinned by hash, which refuses a re-authored document -- and every other
    # document too -- before a span is looked for.
    edits = []
    for target, ents, why in replace:
        if target not in spans:
            sys.exit(f"cannot find {target} in the document")
        a, b = spans[target]
        edits.append((a, b, ",\n".join(render(e) for e in ents).lstrip(), why))
    for target, why in delete:
        if target not in spans:
            sys.exit(f"cannot find {target} in the document")
        a, b = spans[target]
        edits.append((a, b, None, f"delete {why}"))
    return doc, ns, edits, appends, {"boxes": boxes, "columns": columns, "runs": runs, "lanes": lanes}


def splice(text, edits, appends, required):
    """Apply the edits by byte span, lowest offset last so offsets stay valid."""
    out = text
    for a, b, replacement, _why in sorted(edits, key=lambda e: -e[0]):
        if replacement is None:
            # A record's bytes are its own two-space indent, its braces, and one
            # separator: the `,\n` after it, or -- if it is the array's last --
            # the `,\n` before it. Taking exactly one separator is what keeps the
            # array well formed without re-serialising it.
            start, end = a - 2, b
            if out[b:b + 2] == ",\n":
                end = b + 2
            else:
                start = a - 4
            out = out[:start] + out[end:]
        else:
            out = out[:a] + replacement + out[b:]
    if appends:
        # The entities array is the document's last array, and it closes with a
        # bracket at indent 1 followed by the object's own brace. Anchoring on
        # the tail rather than on a repeated separator is what keeps this from
        # landing inside `required_components`.
        tail = "\n ]\n}\n"
        if not out.endswith(tail):
            sys.exit("the document does not end the way its canonical form does")
        at = len(out) - len(tail)
        out = out[:at] + ",\n" + ",\n".join(render(e) for e in appends) + out[at:]
    if required:
        # `pick` joins `required_components`, and it is the only header field
        # this change touches: the machines' presentation cells carry the
        # happening and nothing else in the document says a pick happens there,
        # so a consumer blind to the name would load four machines with no pick
        # places (SPEC section 9).
        anchor = '\n  "frame",\n  "port",'
        if out.count(anchor) != 1:
            sys.exit("`required_components` is not laid out the way this edit expects")
        out = out.replace(anchor, '\n  "frame",\n  "pick",\n  "port",')
    return out


def prove(before, out, expect_required):
    """The proofs run before a candidate is written and before any apply.

    They are the ones the deleted `scripts/drop_queue_depth.py` ran, in its order: the whole
    document still parses; nothing outside `entities` moved except the one
    header field this change is allowed to touch; every record still carries an
    identity; and the bytes are this document's canonical form with the line
    endings it had.
    """
    after = json.loads(out)
    for key in ("proto_version", "doc", "primary_frame"):
        if after.get(key) != before.get(key):
            sys.exit(f"proof failed: `{key}` changed, and this edit does not touch it")
    was_required, now_required = before["required_components"], after["required_components"]
    added = [c for c in now_required if c not in was_required]
    if added != expect_required or [c for c in was_required if c not in now_required]:
        sys.exit(f"proof failed: `required_components` gained {added}, expected {expect_required}")
    was = {e["id"] for e in before["entities"]}
    got = {e["id"] for e in after["entities"] if "id" in e}
    if len(got) != len(after["entities"]):
        sys.exit("proof failed: an entity record lost its identity")
    if "\r" in out:
        sys.exit("proof failed: a carriage return appeared")
    if json.dumps(after, **CANONICAL) + "\n" != out:
        sys.exit("proof failed: the spliced bytes are not this document's canonical form")
    print(f"  proofs: parses; header untouched but `required_components` + {expect_required}; "
          f"{len(was)} -> {len(got)} identities ({len(was - got)} gone, {len(got - was)} new); "
          f"canonical form; no CR")
    return after


def counts(before, after, runs):
    """What the document holds before and after, in the numbers a gate pins."""
    per_machine = geom.counted()
    lane = BAND["cols"] * BAND["lane_rows"]
    connectors = sum(len(run) for run in runs) * (BAND["rows"] - BAND["lane_rows"])
    machine_cells = len(MACHINES) * per_machine["cells"]
    box_cells = len(BOX_INDUCTION)
    return {
        "machine cells, before (4 x 54)": 216,
        f"machine cells, after (4 x {per_machine['cells']})": machine_cells,
        "of them waiting": len(MACHINES) * per_machine["waiting"],
        "of them pick places": len(MACHINES) * per_machine["presentation"],
        "of them ports": len(MACHINES) * per_machine["ports"],
        "station floor cells, before": BAND["cols"] * BAND["rows"] - 203,
        "floor lane cells, after": lane,
        f"band floor cells, after ({len(runs)} pieces)": connectors,
        "box induction cells, after": box_cells,
        "band cells, before": 1201,
        "band cells, after": lane + connectors + machine_cells + box_cells,
        "declared records, before": len(before["entities"]),
        "declared records, after": len(after["entities"]),
    }


def conflicts(state):
    """Everything this change places, checked against every machine envelope.

    Machine against machine, every declared lane cell against every envelope,
    and every connector column against every envelope -- the last of which is
    what caught the middle connector overlapping machine 2 by 260 mm at the
    column the design named.
    """
    boxes, runs, lanes = state["boxes"], state["runs"], state["lanes"]
    found = []
    for i, (an, ax0, _ay0, ax1, _ay1) in enumerate(boxes):
        for bn, bx0, _by0, bx1, _by1 in boxes[i + 1:]:
            if ax0 < bx1 and bx0 < ax1:
                found.append(f"{an} and {bn} envelopes overlap on x")
    for label, (_ids, _present, _standing, px, sx) in lanes.items():
        for what, x in (("lane", px), ("standing cell", sx)):
            lo, hi = x - BAND["pitch"][0] // 2, x + BAND["pitch"][0] // 2
            for name, x0, _y0, x1, _y1 in boxes:
                if lo < x1 and x0 < hi:
                    found.append(f"{label}'s {what} (x {lo}..{hi}) stands inside {name}'s envelope")
    for columns in runs:
        lo = columns[0] * BAND["pitch"][0]
        hi = (columns[-1] + 1) * BAND["pitch"][0]
        for name, x0, _y0, x1, _y1 in boxes:
            if lo < x1 and x0 < hi:
                found.append(f"band floor at columns {columns[0]}-{columns[-1]} (x {lo}..{hi}) "
                             f"overlaps {name}'s envelope")
        for label, (_ids, _p, _s, px, sx) in lanes.items():
            for what, x in (("lane", px), ("standing cell", sx)):
                if lo < x + BAND["pitch"][0] // 2 and x - BAND["pitch"][0] // 2 < hi:
                    found.append(f"band floor at columns {columns[0]}-{columns[-1]} covers "
                                 f"{label}'s {what}")
    return found


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def read_pre_image():
    """The recorded pre-image's bytes, or an exit.

    A blob this script cannot read is a FAILED audit rather than a missing
    line of output: the audit's whole claim is that the document it patches
    is the one recorded here, and a claim nobody could check is not a claim.
    """
    try:
        blob = subprocess.run(["git", "-C", str(REPO), "cat-file", "blob", PRE_IMAGE_BLOB],
                              capture_output=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", b"") or b""
        sys.exit(f"AUDIT FAILED: cannot read the recorded pre-image, git object {PRE_IMAGE_BLOB} "
                 f"({error}) {detail.decode(errors='replace').strip()}")
    got = sha256(blob)
    if got != PRE_IMAGE_SHA256 or len(blob) != PRE_IMAGE_BYTES:
        sys.exit(f"AUDIT FAILED: git object {PRE_IMAGE_BLOB} is {len(blob)} bytes, sha256 {got}; "
                 f"the recorded pre-image is {PRE_IMAGE_BYTES} bytes, sha256 {PRE_IMAGE_SHA256}")
    return blob.decode()


def audit(text):
    """Two claims about the document at `DOC`, and nothing softer than either.

    ONE: the recorded pre-image is a document this script can read the way it
    patches it -- its canonical form reproduces it byte for byte, and every
    top-level record the span scanner locates re-renders to exactly the bytes
    the span covers. That is what makes a span edit safe: the script cuts at
    record boundaries it has proved are record boundaries.

    TWO: applying this script's plan to that pre-image yields exactly the
    document sitting at `DOC`, byte for byte. That is the whole provenance
    claim -- the committed document is the pre-image plus this plan and
    nothing else -- and it is what a hand edit to either end breaks.

    A document that IS the pre-image fails claim two, and the re-authored
    document at this branch's tip fails claim one. Both are meant to: the
    audit says which of the two the document is, and refuses anything that is
    neither. `text` is the committed document; the pre-image is read from git.
    """
    pre = read_pre_image()
    print("THE RECORDED PRE-IMAGE")
    print(f"  git object                                 {PRE_IMAGE_BLOB}")
    print(f"  sha256                                     {sha256(pre.encode())}")
    print(f"  bytes                                      {len(pre.encode())}")

    doc = json.loads(pre)
    canonical = (json.dumps(doc, **CANONICAL) + "\n").encode() == pre.encode()
    print(f"  canonical form reproduces it byte for byte {canonical}")
    spans, _ = entity_spans(pre)
    located = len(spans) == len(doc["entities"])
    print(f"  records located by the span scanner        {len(spans)} of {len(doc['entities'])}")
    # `render` writes a record the way the document does, two-space indent and
    # all; a span starts at the brace, one indent in, so the pad comes off.
    misread = [i for i, (a, b) in spans.items()
               if json.loads(pre[a:b]).get("id") != i or render(json.loads(pre[a:b]))[2:] != pre[a:b]]
    print(f"  records whose span is not exactly their canonical bytes  {len(misread)}")
    scanned = canonical and located and not misread

    print()
    print(f"THE COMMITTED DOCUMENT  {DOC.relative_to(REPO)}")
    print(f"  sha256                                     {sha256(text.encode())}")
    print(f"  bytes                                      {len(text.encode())}")
    print(f"  is the pre-image                           {text.encode() == pre.encode()}")

    applied = False
    if scanned:
        before, _ns, edits, appends, _state = build_plan(pre)
        out = splice(pre, edits, appends, "pick" not in before["required_components"])
        applied = out.encode() == text.encode()
        print(f"  is the plan applied to the pre-image       {applied}")
        if not applied:
            print(f"  the plan applied to the pre-image gives    {sha256(out.encode())} "
                  f"({len(out.encode())} bytes)")
    else:
        print("  is the plan applied to the pre-image       not asked: the pre-image did not scan")

    print()
    if scanned and applied:
        print("AUDIT PASSED: the pre-image is what this script records, this script reads it at "
              "record boundaries, and the committed document is that pre-image with this plan "
              "applied.")
        return True
    if not scanned:
        print("AUDIT FAILED: the recorded pre-image does not read the way this script patches it. "
              "Every edit here is a byte span, so a scanner that cannot reproduce a record's own "
              "bytes cannot be trusted to cut at its edges.")
    else:
        print("AUDIT FAILED: the committed document is not this plan applied to the recorded "
              "pre-image. Either the document was edited by something other than this script, or "
              "the plan changed and the document was not re-applied -- restore the pre-image and "
              "run --apply rather than editing the document by hand.")
    return False


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--emit", type=Path, help="write a candidate here and change nothing else")
    ap.add_argument("--apply", action="store_true", help="rewrite the fixture in place")
    ap.add_argument("--align-floor", action="store_true",
                    help="apply/audit the September 8 apron and rack-bay alignment plan")
    ap.add_argument("--audit", action="store_true",
                    help="prove the committed document is the recorded pre-image with this plan "
                         "applied, and stop")
    args = ap.parse_args()

    if args.align_floor:
        import dfw6_floor_alignment
        return dfw6_floor_alignment.run(args)

    text = DOC.read_text(newline="")
    if args.audit:
        return 0 if audit(text) else 1

    # Every edit below is a byte span of the RECORDED pre-image, so the file
    # this plan is read off must be that document and nothing else. A canonical
    # edit that keeps a record's id -- to a stamp, to `station floor` -- leaves
    # the spans findable and the plan buildable, and `--apply` would then
    # replace the whole record and discard the edit without a word. Refusing on
    # the hash is what makes "the plan applies to THIS document" checkable
    # rather than assumed.
    got = sha256(text.encode())
    accepted = {PRE_IMAGE_SHA256, MAP_REVIEW_SHA256, PRESENTATION_REVIEW_SHA256, LANE_REVIEW_SHA256, "a94aecf2e03d96dfd91435a6c08c45d5333aae1fa2319a1b2fe38c8b91d3c4da"}
    original = read_pre_image()
    if got not in accepted:
        _before, _ns, plan, appended, _state = build_plan(original)
        required = "pick" not in _before["required_components"]
        expected = splice(original, plan, appended, required)
        if text != expected:
            sys.exit("REFUSED: fixture differs from the pinned input or this script's audited output")
    text = original

    before, _ns, edits, appends, state = build_plan(text)
    need_pick = "pick" not in before["required_components"]
    out = splice(text, edits, appends, need_pick)

    print("GEOMETRY, in the DWG block's own millimetres "
          "(scripts/easyloop_geometry.py, shared with the minimal map)")
    for handle, (x, y), role in geom.cells():
        print(f"  {handle:18s} ({x:9.2f}, {y:9.2f})  {role}")
    print()
    print("CLEARANCES the layout stands on, millimetres")
    for what, value in geom.clearances():
        print(f"  {value:9.2f}  {what}")
    print()
    print("PLACES, in the floor frame")
    for (name, _sid, stamp_x, mirrored, _ext), (_n, x0, y0, x1, y1) in zip(MACHINES, state["boxes"]):
        print(f"  {name}: frame translation [{stamp_x + ORIGIN_DX_STEPS}, {ORIGIN_Y_STEPS}] "
              f"scale [{-1 if mirrored else 1}, 1]{'  MIRRORED' if mirrored else ''}"
              f"   envelope x {x0}..{x1}  y {y0}..{y1}")
    for columns in state["runs"]:
        west, east = between(state["boxes"], columns, {c for label in state["columns"]
                                                       for c in (state["columns"][label],
                                                                 state["columns"][label] + 1)})
        print(f"  band floor: columns {columns[0]}-{columns[-1]} "
              f"({len(columns)} wide), between {west} and {east}")
    for label, (_ids, _p, _s, px, sx) in state["lanes"].items():
        print(f"  {label}: lane column {band_column(px)} (x {px}), standing cell x {sx}")
    print()
    print("CONFLICTS")
    found = conflicts(state)
    for line in found:
        print(f"  {line}")
    if not found:
        print("  none")
    print()
    print("EDITS, by byte span")
    for a, b, replacement, why in sorted(edits):
        kind = "delete" if replacement is None else f"replace with {len(replacement)} bytes"
        print(f"  [{a:7d}, {b:7d})  {kind:28s} {why}")
    print(f"  append {len(appends)} entities (box-induction pickup points and access links)")
    if need_pick:
        print("  `required_components` gains `pick`")
    print()

    after = prove(before, out, ["pick"] if need_pick else [])
    print()
    print("COUNTS")
    for what, value in counts(before, after, state["runs"]).items():
        print(f"  {value:6d}  {what}")
    print()

    if found and (args.apply or args.emit):
        sys.exit("REFUSED: the plan has conflicts, listed above. A conflict is a wrong answer "
                 "rather than a hard one -- fix the layout, not the check.")
    if args.apply:
        DOC.write_text(out, newline="")
        print(f"applied: {DOC} ({len(out.encode())} bytes)")
    elif args.emit:
        args.emit.write_text(out, newline="")
        print(f"candidate: {args.emit} ({len(out.encode())} bytes)")
    else:
        print("dry run: nothing written. --emit <path> writes a candidate; --apply rewrites "
              "the fixture in place.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
