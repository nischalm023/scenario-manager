#!/usr/bin/env python3
"""Emit `sim/dessim/tests/fixtures/minimal_easyloop.json`: the smallest world
that still runs every flow the Walmart-era work added.

WHY THIS MAP EXISTS. `minimal_putaway2.json` is the small map the team iterates
putaway on. Nothing that small exercises an EasyLoop machine, an inventory
induction station, box induction, extraction, the order buffer or order
close-out, so every check on those runs on the 1,517-entity DFW6 document. This
map is the EARLY gate for a development cycle; DFW6 stays the final one.

WHAT IT IS. One EasyLoop (two pick places on one belt), one inventory
induction station, one box induction station, one extraction station, one
inventory run with a two-bay order-tote depot at its east end, one order-buffer
run worked by a gripper, a highway band, and an open floor strip. Nothing here is
Walmart: the bands are plain rectangles on one 770 x 850 mm lattice and the
names say what a thing is for.

THE EASYLOOP IS AUTHORED CELL BY CELL (2026-09-04 rulings). It used to be a
byte-for-byte copy of DFW6's `template:easyloop`, stamped: 54 cells, most of
them a blanket tiling of the machine's whole footprint, which let an HTM drive
anywhere inside the machine on a path the real one has no lane for. It is now
one machine entity -- a FRAME carrying the `go.easyloop` envelope -- with
FORTY-SIX individually declared cells posed in that frame, and nothing else:
no `station` component, no tiling inside the envelope, and every arc into, out
of and inside the machine asserted by an explicit `stitch`. The cells are the
ones an HTM actually stands on: the two ports at the twin belt's ramp end, the
inventory loop under the conveyor between them, the two horseshoe lanes, the
two order presentation cells, and the waiting cells the order half recirculates
through.

THE MACHINE'S TABLE IS NOT HERE. It is `scripts/easyloop_geometry.py`, which
`scripts/reauthor_dfw6_easyloops.py` imports too, so this map's EasyLoop and
DFW6's four are one machine stated once. What this file decides about the
machine is only where it stands: the DWG block's origin in this map's floor
frame, and which apron cell each of its six doors opens onto.

REGENERATE:  python3 scripts/gen_minimal_easyloop.py
"""

import json
import sys
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import easyloop_geometry as geom   # noqa: E402 - the path is set on the line above

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "sim/dessim/tests/fixtures/minimal_easyloop.json"

DOC_ID = str(uuid.uuid5(uuid.NAMESPACE_URL, "greyorange-foundry/minimal_easyloop"))
NS = uuid.UUID(DOC_ID)


def eid(name: str) -> str:
    """A stable identity per authored thing, so a regeneration is a no-op."""
    return str(uuid.uuid5(NS, name))


# --- the robots ------------------------------------------------------------
# Each robot entity carries the figures this map's runs have always used: the
# placeholder robot of the simulator's settings defaults
# (sim/dessim/src/config/robots.rs), which scripts/add_robot_aspects.py first
# wrote here. Units are in the field names.

PLACEHOLDER_ROBOT = (
    "The simulator's former compiled defaults, which this test world has always run; "
    "not a real robot's figures."
)


def placeholder_vtm() -> dict:
    return {
        "rack_ladder": {"max_speed_mm_s": 3000, "accel_mm_s2": 2000},
        "lift": {"profile": {"max_speed_mm_s": 3000, "accel_mm_s2": 2000}},
        "end_effector": {
            "pick_secs": 4.0,
            "drop_secs": 2.0,
            "align_before_pick_secs": 1.933,
            "align_before_drop_secs": 1.933,
        },
    }


# --- the lattice -----------------------------------------------------------
#
# Steps are 0.1 mm. One number in this file is a length in steps; `mm()` is the
# only place millimetres turn into them, so the arithmetic below reads in
# millimetres and the document is written in steps.

def mm(v: float) -> int:
    scaled = round(v * 10, 6)
    assert float(scaled).is_integer(), f"{v} mm is not a whole 0.1 mm step"
    return int(scaled)


# The site lattice every band of this map is tiled at. It is the machine's own
# lane pitch adopted site-wide, so a band tile and a machine cell are the same
# size and nothing stands half on something.
PITCH_X = int(geom.LANE_PITCH_X_MM)   # 770
PITCH_Y = int(geom.LANE_PITCH_Y_MM)   # 850
COLS = 16
WIDTH = COLS * PITCH_X             # 12,320 mm

# Where the machine's DROPOFF port stands on the floor. Everything else about
# the machine comes from `easyloop_geometry`; this is the anchor.
LOOP_X = 1925                      # column 2 of the site lattice
LOOP_Y = 8925                      # the middle of the port row, see Y_PORTS

# Band edges in y, north to south. Every band is COLS wide and tiled at the
# site lattice; the port row is the one the machine's doorway stands in.
Y_FLOOR_STRIP = 0                      # floor strip, 2 rows
Y_INDUCTION = 1700                 # the floor the EasyLoop and box induction stand on, 8 rows
Y_PORTS = 8500                     # 1 row; the machine's doorway cells stand in it
Y_APRON = 9350                     # 2 rows: the machine is approached across these
Y_HIGHWAY = 11050                  # 2 rows, `highway` marker
Y_SERVICE = 12750                  # 4 rows: induction + extraction stations, and the relay lane
Y_STEEL = 16150                    # bays, 1,000 mm deep
Y_AISLE = 17150                    # climbing aisles, untiled
Y_END = 18150

# ---------------------------------------------------------------------------
# The EasyLoop, placed
# ---------------------------------------------------------------------------
#
# The machine's own table -- which cells exist, what each one is, which arcs
# join them and where its envelope is -- lives in `scripts/easyloop_geometry.py`
# and is shared with `scripts/reauthor_dfw6_easyloops.py`, so this map and DFW6
# state ONE machine. What is decided here is only where it stands: the block's
# origin in this map's floor frame, and which apron cell each of its six doors
# opens onto.
#
# The block's origin is not a place a robot goes -- it is the DWG insert point,
# 840 mm west and 1,424 mm north of the dropoff -- so it is derived from the one
# place this map and the drawing agree on, the dropoff cell, which is also what
# `desvis/src/easyloop_prop.rs` anchors the drawn machine on.
BLOCK_ORIGIN_X = LOOP_X - geom.V_WEST_C          # 2,765 mm
BLOCK_ORIGIN_Y = LOOP_Y - geom.V_Y_N             # 10,349 mm


def machine_floor(xy):
    """One of the machine's own points, in this map's floor millimetres."""
    return (BLOCK_ORIGIN_X + xy[0], BLOCK_ORIGIN_Y + xy[1])


def machine_envelope():
    """The machine's envelope as `(x, y, w, h)` in this map's floor millimetres.

    Stated in the FLOOR frame rather than the machine's own, because that is the
    one statement that survives a mirrored insert: DFW6 puts two of its four
    machines in at scale -1 on x, and a rectangle written in a mirrored frame
    comes out with its origin on the east edge.
    """
    ex, ey, ew, eh = geom.envelope()
    x, y = machine_floor((ex, ey))
    return (x, y, ew, eh)


X_INVENTORY = 0
X_DEPOT = 4620                     # 6 stock bays of 770 mm
X_BUFFER = 7700                    # column 10, clear of the depot

# One bay: 770 mm along the run so a run's relays land on the lattice the
# service lane in front of them is tiled at, 1,000 mm deep, five storage
# shelves over one handoff shelf on its north face.
BAY_W = 770
BAY_D = 1000
STORAGE_LEVELS = [800, 1200, 1600, 2000, 2400]
RELAY_HEIGHT = 400
RELAY_REACH = 1000

INVENTORY_BAYS = 6                 # 30 stock positions
DEPOT_BAYS = 2                     # 10 empty-order-tote positions
BUFFER_BAYS = 3                    # 15 closed-order positions
ORDER_TOTES = DEPOT_BAYS * len(STORAGE_LEVELS)

X_STORAGE_END = X_DEPOT + DEPOT_BAYS * BAY_W             # 6,160
X_BUFFER_END = X_BUFFER + BUFFER_BAYS * BAY_W            # 10,010


def col_x(col: int) -> int:
    """The centre of lattice column `col`, in millimetres."""
    return col * PITCH_X + PITCH_X // 2


def row_y(band_top: int, row: int) -> int:
    """The centre of `row` of a band whose north edge is `band_top`."""
    return band_top + row * PITCH_Y + PITCH_Y // 2


# Station columns, chosen where no bay's handoff shelf binds, so a lane never
# gates a handoff cell: the runs' shelves stand on columns 0..7 and 10..12.
COL_INDUCTION = 8                                        # centre 6,545 mm
COL_EXTRACT = 13                                         # centre 10,395 mm
COL_BOX = 13

# Each station's lane. Its id is a random v4, minted once and written here:
# an entity's id is never derived from a name, unlike the `eid` ids the rest
# of this map still carries.
STATION_LANE_IDS = {
    "box induction": "6a99d7c1-a8d7-49ad-8b64-23ab055f62bb",
    "inventory induction": "a811cd80-31ee-401a-bec3-28c889eb822f",
    "extraction": "52f3afd1-b11a-4fa6-a1d1-ae26175677f4",
}

FRAME_SITE = eid("frame:site")
FRAME_FLOOR = eid("frame:floor")
# The machine IS its frame: one entity carrying the frame, the `go.easyloop`
# envelope region, and the belt timing every cell posed in it shares.
FRAME_MACHINE = eid("easyloop 1")


def frame(entity_id, name, comps):
    return {"id": entity_id, "name": name, "components": comps}


def band(entity_id, name, top, rows, comment, derives_extra=None, exclusions=None):
    """One tiled floor band: full width, `rows` rows of the site lattice."""
    derives = {"cell": {"traversable_by": ["htm"]}}
    if derives_extra:
        derives.update(derives_extra)
    tiling = {"pitch": [mm(PITCH_X), mm(PITCH_Y)], "derives": derives}
    if exclusions:
        tiling["exclusions"] = exclusions
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "region": {
                "frame": FRAME_FLOOR,
                "origin": [mm(0), mm(top)],
                "size": [mm(WIDTH), mm(rows * PITCH_Y)],
            },
            "tiling": tiling,
            "$comment": comment,
        },
    }


def stitch(entity_id, name, a, b, comment, tolerance=12750):
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "stitch": {"a": a, "b": b, "rule": "contact", "tolerance": tolerance, "$comment": comment}
        },
    }


def explicit_stitch(entity_id, name, pairs, comment):
    """A stitch that names its arcs outright, one pair per arc.

    `rule: contact` matches by geometry, which is the mechanism a machine
    authored cell by cell must not use: nothing inside this machine binds
    because it happens to be near something (2026-09-04).

    The PAIRS are the whole statement -- an explicit stitch compiles exactly
    them and reads nothing else. `a` and `b` are filled in with the first cell
    and the last cell the pairs touch, which is a reader's signpost and not a
    second claim.
    """
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "stitch": {
                "a": pairs[0][0],
                "b": pairs[-1][1],
                "rule": "explicit",
                "pairs": [[a, b] for a, b in pairs],
                "$comment": comment,
            }
        },
    }


def machine_cell(entity_id, name, xy, role, extra, comment):
    """One declared cell of the machine, posed in the machine's own frame."""
    components = {
        "pose": {"frame": FRAME_MACHINE, "position": [mm(xy[0]), mm(xy[1])]},
        "cell": {"traversable_by": ["htm"], "size": [mm(geom.CELL_SIZE_MM), mm(geom.CELL_SIZE_MM)]},
    }
    components.update(extra)
    components["$comment"] = comment
    return {"id": entity_id, "name": name, "components": components}


def lane(entity_id, name, cells, comment):
    """One declared lane: an ordered walk over cells, named by identity.

    The last cell is the PLACE THE LANE SERVES, not a step short of it (the
    2026-09-02 framing ruling): a lane terminating at the collection port is
    that port's approach, one terminating at a presentation cell is that pick
    place's. There is no `queue.into` -- the terminus says what it feeds.
    """
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "queue": {
                "path": list(cells),
                "discipline": "no_reverse",
                "$comment": comment,
            }
        },
    }


def bay_template(entity_id, name, holds, comment):
    """One bay of a run: five storage shelves over one handoff shelf.

    The handoff shelf stands at `out = 0`, the bay's north face, so a bay
    presents to the service lane in front of it and is worked from the aisle
    behind it -- the sandwich `minimal_putaway2.json` already uses, said as
    geometry rather than as a `sides` enum.
    """
    levels = [
        {
            "height": mm(RELAY_HEIGHT),
            "positions": [[mm(BAY_W / 2), 0]],
            "reach": mm(RELAY_REACH),
        }
    ]
    levels += [{"height": mm(h)} for h in STORAGE_LEVELS]
    components = {
        "pose": {"position": [0, 0], "heading_deg": 0},
        "rack": {
            "slots_wide": 1,
            "slots_deep": 1,
            "slot_pitch": [mm(BAY_W), mm(BAY_D)],
            "footprint": [mm(BAY_W), mm(BAY_D)],
            "levels": levels,
        },
        "storage": {"holds": holds},
    }
    return {
        "id": entity_id,
        "name": name,
        "components": {"template": {"params": {}, "entities": [{"local": "rack", "components": components}], "$comment": comment}},
    }


def run(entity_id, name, template_id, x0, count, comment):
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "pose": {"frame": FRAME_FLOOR, "position": [mm(x0), mm(Y_STEEL)], "heading_deg": 0},
            "stamp": {
                "template": template_id,
                "params": {},
                "repeat": {"count": count, "step": [mm(BAY_W), 0]},
            },
            "$comment": comment,
        },
    }


def station(entity_id, name, kind, pose_xy, exit_direction, comment, extra_station=None):
    st = {"kind": kind, "exit_direction": exit_direction}
    if extra_station:
        st.update(extra_station)
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "pose": {"frame": FRAME_FLOOR, "position": [mm(pose_xy[0]), mm(pose_xy[1])]},
            "station": st,
            "$comment": comment,
        },
    }


def station_lane(entity_id, station_name, band_id, cells):
    """A station's lane: the cells of one floor band a robot queues on, by
    column and row, and last the presentation cell it feeds. A lane names
    cells by identity, so the terminus says which presentation it serves."""
    return {
        "id": entity_id,
        "name": f"{station_name}: lane",
        "components": {
            "queue": {
                "path": [{"generator": band_id, "key": f"cell:{col},{row}"} for col, row in cells],
                "discipline": "no_reverse",
            }
        },
    }


def presentation(entity_id, name, xy, for_what, station_id, comment):
    return {
        "id": entity_id,
        "name": name,
        "components": {
            "pose": {"frame": FRAME_FLOOR, "position": [mm(xy[0]), mm(xy[1])]},
            "presentation_point": {"for": for_what, "station": station_id},
            "$comment": comment,
        },
    }


def main() -> None:
    entities = []

    # --- frames ------------------------------------------------------------
    entities.append(
        frame(
            FRAME_SITE,
            "site",
            {
                "frame": {
                    "dims": 3,
                    "unit_mm": 0.1,
                    "units": "mm",
                    "$comment": "0.1 mm steps, so the drawing's tenth-of-a-millimetre "
                    "measurements land on whole steps.",
                }
            },
        )
    )
    entities.append(
        frame(
            FRAME_FLOOR,
            "floor",
            {
                "frame": {
                    "dims": 2,
                    "parent": FRAME_SITE,
                    "units": "mm",
                    "transform_to_parent": {"scale": [1, 1], "translation": [0, 0, 0]},
                }
            },
        )
    )

    entities.append(
        {
            "id": eid("building"),
            "name": "building",
            "components": {
                "region": {
                    "frame": FRAME_FLOOR,
                    "origin": [mm(-1000), mm(-1000)],
                    "size": [mm(WIDTH + 2000), mm(Y_END + 2000)],
                },
                "building": {},
            },
        }
    )

    # --- floor bands -------------------------------------------------------
    # Preserve the established floor identity even though its old role was removed.
    b_floor_strip = eid("band:parking")
    b_induction = eid("band:induction floor")
    b_ports = eid("band:port row")
    b_apron = eid("band:apron")
    b_highway = eid("band:highway")
    b_service = eid("band:service floor")

    entities.append(
        band(
            b_floor_strip,
            "floor strip",
            Y_FLOOR_STRIP,
            2,
            "Ordinary traversable floor. Idle HTMs use available relay presentation cells.",
        )
    )

    # The machine's envelope, as a carve-out of each band it stands in: no
    # tiling reaches under it (2026-09-04), because the machine's own cells are
    # the only places inside it an HTM may stand, and a band tile under one of
    # them would put two robots on one square metre in two cells that cannot
    # see each other's reservations.
    ex_x, ex_y, ex_w, ex_h = machine_envelope()

    def envelope_hole(band_top, why):
        return [
            {
                # Exclusions test tile centres. Expand by half a floor tile
                # so tiles whose edges overlap the envelope are removed too.
                "origin": [mm(ex_x - PITCH_X / 2), mm(ex_y - band_top - PITCH_Y / 2)],
                "size": [mm(ex_w + PITCH_X), mm(ex_h + PITCH_Y)],
                "$comment": why,
            }
        ]

    entities.append(
        band(
            b_induction,
            "induction floor",
            Y_INDUCTION,
            8,
            "The floor the EasyLoop stands in and the box induction station stands on. "
            "The machine's envelope is carved out of it: inside the envelope the only "
            f"cells are the {geom.counted()['cells']} the machine declares.",
            exclusions=envelope_hole(
                Y_INDUCTION,
                "The EasyLoop's envelope. Nothing tiles under a machine whose lanes are "
                "declared cell by cell.",
            ),
        )
    )

    entities.append(
        band(
            b_ports,
            "port row",
            Y_PORTS,
            1,
            "The machine's doorway row. The four cells the machine presents to the site "
            "-- its two ports, two order entries and two order exits -- stand in it, so the "
            "band is carved away across the machine's width; the columns either side "
            "carry the floor past it.",
            exclusions=envelope_hole(
                Y_PORTS,
                "The same envelope, at the row the machine's doorway cells stand in.",
            ),
        )
    )
    entities.append(band(b_apron, "apron", Y_APRON, 2, "The floor an HTM crosses to reach the machine's ports."))
    entities.append(
        band(
            b_highway,
            "highway",
            Y_HIGHWAY,
            2,
            "The through route between the machines and the racking. `highway` is a marker: "
            "it says what the floor IS and directs no arc.",
            derives_extra={"highway": {}},
        )
    )
    entities.append(
        band(
            b_service,
            "service floor",
            Y_SERVICE,
            4,
            "The inventory induction and extraction stations stand on its north rows; its "
            "south row is the lane every handoff shelf binds to.",
        )
    )

    for a, b, why in [
        (b_floor_strip, b_induction, "floor strip onto the floor the machines stand on"),
        (b_induction, b_ports, "the machine's floor onto its doorway row"),
        (b_ports, b_apron, "the doorway row onto the apron"),
        (b_apron, b_highway, "the apron onto the highway"),
        (b_highway, b_service, "the highway onto the service floor"),
    ]:
        entities.append(stitch(eid(f"stitch:{a}:{b}"), f"stitch: {why}", a, b, f"Bands abut; {why}."))

    # --- the EasyLoop ------------------------------------------------------
    #
    # One machine, one entity: a FRAME, the `go.easyloop` envelope region that
    # says what ground it occupies, and the belt timing both halves share. Every
    # cell posed in that frame is a member of this machine -- which is the whole
    # membership rule, replacing the shared-derivation-root join a stamp used to
    # supply. No `station` component anywhere: station is a lens word for the
    # assemblage, and the happening lives on the presentation cell.
    entities.append(
        {
            "id": FRAME_MACHINE,
            "name": "easyloop 1",
            "components": {
                "frame": {
                    "dims": 2,
                    "parent": FRAME_FLOOR,
                    "units": "mm",
                    "transform_to_parent": {
                        "scale": [1, 1],
                        "translation": [mm(BLOCK_ORIGIN_X), mm(BLOCK_ORIGIN_Y)],
                    },
                    "$comment": "The `Easy Loop Updated` block of `DFW6 Pilot trim down.dwg`, as "
                    "a frame: one step is this document's 0.1 mm, and the block's own "
                    "millimetres times ten are the positions of every cell posed in it. "
                    "The translation puts the block's unload point on the dropoff cell, "
                    "which is what desvis/src/easyloop_prop.rs anchors the drawn "
                    "machine on.",
                },
                "region": {
                    "frame": FRAME_FLOOR,
                    "origin": [mm(ex_x), mm(ex_y)],
                    "size": [mm(ex_w), mm(ex_h)],
                    "$comment": "In the FLOOR frame, not the machine's own, because that is the "
                    "statement that survives a mirrored insert: DFW6 puts two of its four "
                    "machines in at scale -1 on x, and a rectangle written in a mirrored "
                    "frame comes out with its origin on the east edge.",
                },
                "go.easyloop": {
                    "part": "envelope",
                    "loop_transit_secs": geom.LOOP_TRANSIT_SECS,
                    "time_per_pick_secs": geom.TIME_PER_PICK_SECS,
                    "$comment": "The belt as a timing abstraction, on the machine rather than on "
                    "either half of it: one L-shaped twin conveyor carries every tote both "
                    "pick places work. Replaced by conveyor entities when fidelity rises.",
                },
            },
        }
    )

    # The cells. Each is an entity with an identity and a pose in the machine's
    # frame, and carries what it IS: a port, a place an order carrier is
    # presented at, a waiting spot. Nothing here is derived from a lattice.
    role_comment = {
        "dropoff": "Inventory tote transfers onto the conveyor; the empty HTM continues underneath.",
        "collect": "A returning inventory tote transfers onto its collection HTM.",
        "lane": "Travel location with explicitly declared movement links.",
        "presentation": "Directly declared order presentation: dwell within a continuous curved U-turn.",
        "waiting": "Waiting location for one presentation, entered from either lane and dispatched only into the descending lane.",
        "doorway": "Order-loop entry or exit, as the directed boundary link states.",
    }
    cells = geom.check_layout()
    handle_id = {}
    for handle, xy, role in cells:
        entity_id = eid(f"easyloop 1/{handle}")
        handle_id[handle] = entity_id
        extra = {}
        if role == "dropoff":
            extra["port"] = {
                "direction": "sink",
                "of": [{"kind": "tote", "designation": "inventory"}],
                "into": FRAME_MACHINE,
            }
        elif role == "collect":
            extra["port"] = {
                "direction": "source",
                "of": [{"kind": "tote", "designation": "inventory"}],
                "from": FRAME_MACHINE,
            }
        elif role == "presentation":
            side = int(handle.split(":")[1])
            extra["presentation_point"] = {"for": "order_carrier"}
            extra["pick"] = {
                "$comment": "The happening: a SKU leaves an inventory tote and enters the order "
                "carrier standing here. It is what makes this a pick place, and it is why the "
                "machine needs no `station` component.",
            }
            # Tangent heading at the declared presentation position.
            heading = geom.PRESENTATION_HEADING[side]
            if heading is not None:
                extra["pose_heading"] = heading
        elif role == "waiting":
            extra["waiting"] = {}
        entities.append(
            machine_cell(entity_id, f"easyloop 1: {handle}", xy, role, extra, role_comment[role])
        )

    # `pose_heading` is a marker this generator uses, not a component: fold it
    # into the pose it belongs to.
    for entity in entities:
        heading = entity["components"].pop("pose_heading", None)
        if heading is not None:
            entity["components"]["pose"]["heading_deg"] = heading

    # The lanes. Three, and each names the place it serves as its last step.
    for handle, path, why in geom.lanes():
        entities.append(
            lane(
                eid(f"easyloop 1/{handle}"),
                f"easyloop 1: {handle.split(':', 1)[1]}",
                [handle_id[cell] for cell in path],
                why,
            )
        )

    # The arcs. Every one of them, asserted.
    for name, pairs, bidirectional, curve in geom.links():
        entities.append(
            explicit_stitch(
                eid(f"easyloop 1/stitch:{name}"),
                f"easyloop 1: {name}",
                [(handle_id[a], handle_id[b]) for a, b in pairs],
                f"Every arc of the {name}, in the order an HTM takes them.",
            )
        )
        link = entities[-1]["components"]["stitch"]
        link["bidirectional"] = bidirectional
        if curve:
            link["curve"] = {"frame": FRAME_MACHINE, "center": [mm(v) for v in curve["center"]],
                             "clockwise": curve["clockwise"],
                             **{k: [mm(v) for v in curve[k]] for k in ('start', 'end') if k in curve}}

    # Only the declared entry and exit cells connect to the site's apron.
    def apron_cell(block_x):
        floor_x = machine_floor((block_x, 0))[0]
        column = min(range(COLS), key=lambda c: abs(col_x(c) - floor_x))
        return f"{b_apron}/cell:{column},0"

    at = {handle: xy for handle, xy, _ in cells}
    for handle, direction in geom.doors():
        inside, outside = handle_id[handle], apron_cell(at[handle][0])
        pair = (outside, inside) if direction == "entry" else (inside, outside)
        record = explicit_stitch(eid(f"easyloop 1/stitch:doorway:{handle}"),
            f"easyloop 1: {handle} {direction}", [pair], "One-way machine boundary crossing.")
        record["components"]["stitch"]["bidirectional"] = False
        entities.append(record)

    # --- stations ----------------------------------------------------------
    s_box = eid("station:box induction")
    s_put = eid("station:inventory induction")
    s_extract = eid("station:extraction")

    entities.append(
        station(
            s_box,
            "box induction 1",
            "induct",
            (col_x(14), row_y(Y_INDUCTION, 7)),
            "+x",
            "Where an order box is put onto a carrier. It registers, renders and is counted; "
            "no dispatch targets it yet, which is the order-box gap.",
        )
    )
    entities.append(
        station_lane(
            STATION_LANE_IDS["box induction"],
            "box induction 1",
            b_induction,
            [(COL_BOX, 4), (COL_BOX, 5), (COL_BOX, 6), (COL_BOX, 7)],
        )
    )
    entities.append(
        presentation(
            eid("presentation:box induction"),
            "presentation point, box induction 1",
            (col_x(COL_BOX), row_y(Y_INDUCTION, 7)),
            "box",
            s_box,
            "Pose only, and it names its station outright rather than leaving proximity to guess.",
        )
    )

    entities.append(
        station(
            s_put,
            "inventory induction 1",
            "putaway",
            (col_x(9), row_y(Y_SERVICE, 0)),
            "+x",
            "Where stock enters the building: an empty tote is presented, filled, and taken "
            "back to racking. `putaway` is the spelling DFW6 writes.",
            extra_station={"bins": 1},
        )
    )
    entities.append(
        station_lane(
            STATION_LANE_IDS["inventory induction"],
            "inventory induction 1",
            b_service,
            [(COL_INDUCTION, 2), (COL_INDUCTION, 1), (COL_INDUCTION, 0)],
        )
    )
    entities.append(
        presentation(
            eid("presentation:inventory induction"),
            "presentation point, inventory induction 1",
            (col_x(COL_INDUCTION), row_y(Y_SERVICE, 0)),
            "inventory_tote",
            s_put,
            "Pose only, station named outright.",
        )
    )

    entities.append(
        station(
            s_extract,
            "extraction 1",
            "extract",
            (col_x(14), row_y(Y_SERVICE, 0)),
            "+x",
            "Where a closed order leaves the building. Close-out cannot run at all without "
            "one an HTM can reach from every pick station and every buffer relay.",
        )
    )
    entities.append(
        station_lane(
            STATION_LANE_IDS["extraction"],
            "extraction 1",
            b_service,
            [(COL_EXTRACT, 2), (COL_EXTRACT, 1), (COL_EXTRACT, 0)],
        )
    )
    entities.append(
        presentation(
            eid("presentation:extraction"),
            "presentation point, extraction 1",
            (col_x(COL_EXTRACT), row_y(Y_SERVICE, 0)),
            "order_container",
            s_extract,
            "Pose only, station named outright.",
        )
    )

    # --- racking -----------------------------------------------------------
    t_inventory = eid("template:bay:inventory")
    t_order = eid("template:bay:order")
    entities.append(
        bay_template(
            t_inventory,
            "bay: 1 wide, 5 storage shelves, 1 handoff shelf on -y (inventory)",
            "inventory",
            "Stock steel. One bay per lattice column, so a run's handoff shelves land on the "
            "centres of the lane in front of them.",
        )
    )
    entities.append(
        bay_template(
            t_order,
            "bay: 1 wide, 5 storage shelves, 1 handoff shelf on -y (order)",
            "order",
            "The same steel with `holds` replaced, which is the whole difference between "
            "stock racking and order racking.",
        )
    )

    r_inventory = eid("run:inventory")
    r_depot = eid("run:order depot")
    r_buffer = eid("run:order buffer")
    entities.append(
        run(
            r_inventory,
            "inventory run",
            t_inventory,
            X_INVENTORY,
            INVENTORY_BAYS,
            "Stock, and the only steel putaway may deposit into.",
        )
    )
    entities.append(
        run(
            r_depot,
            "order-tote depot run",
            t_order,
            X_DEPOT,
            DEPOT_BAYS,
            "Order bays in an INVENTORY aisle, which is what makes them the depot: where an "
            "unused order tote waits until close-out loads an order into it. A separate stamp "
            "because a run is one stamp of one template and its bays cannot disagree on what "
            "they hold.",
        )
    )
    entities.append(
        run(
            r_buffer,
            "order-buffer run",
            t_order,
            X_BUFFER,
            BUFFER_BAYS,
            "Order bays in an ORDER aisle, which is what makes them the buffer: capacity for a "
            "closed order that has not been picked yet, so nothing is seeded into them.",
        )
    )

    a_storage = eid("aisle:storage")
    a_buffer = eid("aisle:buffer")
    entities.append(
        {
            "id": a_storage,
            "name": "storage aisle",
            "components": {
                "region": {
                    "frame": FRAME_FLOOR,
                    "origin": [mm(X_INVENTORY), mm(Y_AISLE)],
                    "size": [mm(X_STORAGE_END - X_INVENTORY), mm(Y_END - Y_AISLE)],
                },
                "$comment": "The strip the climbing class works from. It declares NO tiling: "
                "tiling follows locomotion, and a lane nothing drives is not floor.",
            },
        }
    )
    entities.append(
        {
            "id": a_buffer,
            "name": "buffer aisle",
            "components": {
                "region": {
                    "frame": FRAME_FLOOR,
                    "origin": [mm(X_BUFFER), mm(Y_AISLE)],
                    "size": [mm(X_BUFFER_END - X_BUFFER), mm(Y_END - Y_AISLE)],
                },
                "$comment": "The buffer's own aisle, so its racking is a zone of its own and a "
                "gripper is the only machine assigned to it.",
            },
        }
    )

    c_vtm = eid("class:vtm vacuum")
    c_gripper = eid("class:vtm gripper")
    c_htm = eid("class:htm")

    for access_name, run_id, aisle_id, served in [
        ("inventory run", r_inventory, a_storage, c_vtm),
        ("order-tote depot run", r_depot, a_storage, c_vtm),
        ("order-buffer run", r_buffer, a_buffer, c_gripper),
    ]:
        entities.append(
            {
                "id": eid(f"access:{access_name}"),
                "name": f"access: {access_name}",
                "components": {
                    "access": {
                        "rack": run_id,
                        "along": aisle_id,
                        "face": "+y",
                        "serves": [served],
                        "$comment": "The face the STORAGE is worked from. The handoff shelves "
                        "present on the other face, which the steel states itself.",
                    }
                },
            }
        )

    sec_storage = eid("sector:storage")
    sec_buffer = eid("sector:buffer")
    entities.append(
        {
            "id": sec_storage,
            "name": "sector: storage aisle",
            "components": {
                "sector": {
                    "members": [r_inventory, r_depot],
                    "$comment": "A generator member stands for its whole expansion, so naming the "
                    "two runs names every bay of them.",
                }
            },
        }
    )
    entities.append(
        {
            "id": sec_buffer,
            "name": "sector: buffer aisle",
            "components": {"sector": {"members": [r_buffer]}},
        }
    )

    entities.append(
        {
            "id": c_htm,
            "name": "HTM",
            "$comment": PLACEHOLDER_ROBOT,
            "components": {
                "floor_drive": {
                    "loaded": {"max_speed_mm_s": 4200, "accel_mm_s2": 2000, "quarter_turn_secs": 1.0},
                    "empty": {"max_speed_mm_s": 4500, "accel_mm_s2": 2000, "quarter_turn_secs": 1.0},
                },
                "end_effector": {"pick_secs": 2.0, "drop_secs": 2.0},
                "robot_class": {
                    "role": "htm",
                    "model": "HTM",
                    "locomotion": "floor",
                    "footprint": [mm(695), mm(460)],
                    "height": mm(300),
                    "max_speed": mm(1500),
                    "carries": ["tote"],
                    "carry_exclusive": True,
                    "configuration": "standard",
                }
            },
        }
    )
    entities.append(
        {
            "id": c_vtm,
            "name": "VTM (vacuum)",
            "$comment": PLACEHOLDER_ROBOT,
            "components": {
                **placeholder_vtm(),
                "robot_class": {
                    "role": "vtm",
                    "model": "VTM",
                    "locomotion": "ladder",
                    "footprint": [mm(695), mm(460)],
                    "height": mm(300),
                    "max_speed": mm(1200),
                    "carries": ["tote"],
                    "carry_exclusive": True,
                    "configuration": "standard",
                }
            },
        }
    )
    entities.append(
        {
            "id": c_gripper,
            "name": "VTM (gripper)",
            "$comment": PLACEHOLDER_ROBOT,
            "components": {
                **placeholder_vtm(),
                "robot_class": {
                    "role": "vtm",
                    "model": "VTM-G",
                    "locomotion": "ladder",
                    "footprint": [mm(695), mm(460)],
                    "height": mm(300),
                    "max_speed": mm(1200),
                    "carries": ["tote"],
                    "carry_exclusive": True,
                    "configuration": "gripper",
                    "$comment": "A gripper works the buffer; a vacuum head works stock. One "
                    "aisle is worked by one kind of machine.",
                }
            },
        }
    )

    entities.append(
        {
            "id": eid("cohort:htm"),
            "name": "cohort: HTMs",
            "components": {
                "cohort": {
                    "count": 8,
                    "robot_class": c_htm,
                    "assignment": {"kind": "free"},
                    "$comment": "EIGHT, and the two above six are what the buffer detour costs. "
                    "At six the run wedges: putaway staging holds four HTMs carrying empty "
                    "totes while a closed order waits for one to fetch it back out of the "
                    "buffer, and neither side yields. Eight clears it; the wedge itself is "
                    "recorded in tests/minimal_easyloop.rs.",
                }
            },
        }
    )
    entities.append(
        {
            "id": eid("cohort:storage vtm"),
            "name": "cohort: storage aisle VTMs",
            "components": {
                "cohort": {
                    "count": 2,
                    "robot_class": c_vtm,
                    "assignment": {"kind": "hard", "to": [sec_storage]},
                }
            },
        }
    )
    entities.append(
        {
            "id": eid("cohort:buffer vtm"),
            "name": "cohort: buffer aisle gripper",
            "components": {
                "cohort": {
                    "count": 1,
                    "robot_class": c_gripper,
                    "assignment": {"kind": "hard", "to": [sec_buffer]},
                }
            },
        }
    )

    tote = eid("carrier_type:tote")
    entities.append(
        {
            "id": tote,
            "name": "tote 650x450",
            "components": {
                "carrier_type": {
                    "kind": "tote",
                    "size": [mm(650), mm(450), mm(300)],
                    "$comment": "One physical tote type; inventory-vs-order is a per-instance "
                    "designation, not a different type.",
                }
            },
        }
    )
    entities.append(
        {
            "id": eid("carrier_pool:order"),
            "name": "pool: order totes",
            "components": {
                "carrier_pool": {
                    "carrier_type": tote,
                    "designation": "order",
                    "count": ORDER_TOTES,
                    "$comment": "One tote per position of the depot, which is where they stand. "
                    "Close-out has nothing to load an order into if this is zero.",
                }
            },
        }
    )

    doc = {
        "$comment": (
            "The minimal map that still runs every Walmart-era flow: one EasyLoop (two pick "
            "places on one belt), inventory induction, box induction, extraction, an "
            "order-tote depot, an order buffer worked by a gripper, a highway and open floor. "
            "It is the EARLY gate for a development cycle -- fast enough to run on every "
            "change -- with walmart_dfw6.json kept for the final gate. Site-agnostic: no "
            "geometry or name here comes from any customer. GENERATED by "
            "scripts/gen_minimal_easyloop.py; regenerate rather than hand-edit. The EasyLoop "
            "is authored cell by cell: one machine entity carrying a frame and the "
            f"`go.easyloop` envelope, {geom.counted()['cells']} declared cells posed in that "
            "frame, an inventory queue, and an explicit stitch per "
            "arc. `pick` on a presentation cell is what makes it a pick place -- the happening "
            "that used to be a `station` component's `kind` -- and this reader knows it by "
            "name, so it is REQUIRED: a consumer blind to it would load a machine with no pick "
            "places at all."
        ),
        "proto_version": "0.7.0",
        "doc": {
            "id": DOC_ID,
            "kind": "world",
            "name": "minimal EasyLoop world (generated by scripts/gen_minimal_easyloop.py)",
        },
        "primary_frame": FRAME_SITE,
        "required_components": sorted(
            {
                "access",
                "carrier_pool",
                "carrier_type",
                "cohort",
                "frame",
                # The machine's two pick places carry `pick` and nothing else
                # says a pick happens there, so a consumer blind to the name
                # would load this map with no pick places at all and simulate a
                # warehouse that fills no orders. That is the mis-simulation
                # `required_components` exists to refuse (SPEC section 9).
                "pick",
                "port",
                "presentation_point",
                "queue",
                "rack",
                "region",
                "robot_class",
                "sector",
                "stamp",
                "station",
                "stitch",
                "storage",
                "template",
                "tiling",
                "waiting",
            }
        ),
        "entities": entities,
    }

    OUT.write_text(json.dumps(doc, indent=1) + "\n")
    print(f"wrote {OUT} ({len(entities)} declared entities)")


if __name__ == "__main__":
    main()
