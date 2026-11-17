#!/usr/bin/env python3
"""Re-cut cencora_v3's floor so that every hole belongs to the tiling it is a
hole in, and the relay lanes exist only where an HTM has work to do.

`sim/dessim/tests/fixtures/cencora_v3.json` is a hand-managed document of
record: desmap2proto seeded it once and no longer produces it. Two things about
that document are now wrong, and this script fixes both in one pass:

  - Its holes are `exclusion` ENTITIES, which subtracted cells from every
    generator in the document at once. The format retired that component; a
    tiling now states its own carve-outs in `exclusions`, so a hole meant for
    one tiling can no longer take cells out of another that happens to overlap
    it. Step 1 moves each rectangle onto the tilings whose floor it is a hole
    in, by `scripts/migrate_exclusions_to_tiling.py`'s own code, so the two
    scripts cannot drift apart.
  - Its relay lanes are six full-width 645 mm bands with the rack sandwiches
    punched back out of them. Step 2 replaces those six bands with 3-row x
    4-column open-cell patches, one per bay per gap between sandwiches. Floor
    exists where an HTM stands to serve boards, and nowhere else — no strip of
    declared floor running through the steel, no carve-out needed to deny it.

The two blanket bands above and below the lanes keep their identity and carry
their carve-outs as data; nothing here cuts them into pieces.

Six bands going away leaves 192 references pointing at nothing, and step 3
carries each across rather than dropping it: 180 parking overrides are
re-addressed to the patch cell standing at the same millimetre — checked here
against the `expect` pin each one carries, not merely left for the loader — and
the 7 band-to-band seams become 192 patch-to-patch ones, each with a tolerance
sized to its own gap. Anything else still pointing at a deleted band stops the
run with the file untouched.

Run from the repository root. Deterministic: same input, same bytes out —
identifiers are minted as UUIDv5 from a fixed namespace by role path, and the
document is written in the layout it already has (two-space indent, sorted
keys, no trailing newline).

    python3 scripts/recut_cencora_floor.py [--dry-run]
"""

import argparse
import copy
import importlib.util
import json
import math
import re
import sys
import uuid
from pathlib import Path

FIXTURE = Path("sim/dessim/tests/fixtures/cencora_v3.json")
MIGRATION = Path("scripts/migrate_exclusions_to_tiling.py")

# Minted under the namespace the set-aside carve prototype used, against the
# same role paths, so a patch standing in the same gap of the same bay keeps the
# same identity across both attempts at this cut.
NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, "greymatter-foundry/cencora-floor-reauthor")

PITCH_X = 850.0
LANE_PITCH_Y = 645.0
LANE_ROWS = 3
BAY_LEN = 2140.0
GAP_COLS_MAX = 4
# How far apart two pieces of floor may sit and still be one seam. Sized to the
# widest real gap in this document — 205 mm between one bay's lane and the
# next — and used only to FIND the seams; each one then states its own gap.
SEAM_REACH = 425
SEAM_SLACK = 50

# What this document is expected to hold going in, and what this cut is expected
# to make of it. Every one is a fact a later edit could quietly move, and a
# re-cut that runs against a moved floor would put cells where no floor is — so
# each is stated here rather than read back out of whatever the file says.
EXPECT_LANE_BANDS = 6
EXPECT_EXCLUSION_ENTITIES = 103
EXPECT_ZONES = 19
EXPECT_GRID_COLUMNS = 153
EXPECT_PATCHES = 24
EXPECT_STAMPS = 144
EXPECT_LANE_CELLS = 1728
EXPECT_STRANDED_OVERRIDES = 180
EXPECT_STRANDED_STITCHES = 7
EXPECT_SEAMS = 192


def mint(role: str) -> str:
    return str(uuid.uuid5(NAMESPACE, role))


def mm(v: float) -> int:
    """Document coordinates are integer step counts; nothing here may emit a
    float."""
    out = int(round(v))
    assert abs(out - v) < 1e-6, f"non-integral millimetre: {v}"
    return out


def load_migration():
    """`scripts/migrate_exclusions_to_tiling.py` as a module.

    Imported rather than re-implemented: the rule for which tiling a rectangle
    lands on, and how it is localized, has to be the same rule in both places or
    a document migrated by one script and re-cut by the other would disagree
    about where its holes are.
    """
    spec = importlib.util.spec_from_file_location("migrate_exclusions_to_tiling", MIGRATION)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------------------
# The reader's own arithmetic, so this script can check its work
# ---------------------------------------------------------------------------


def centre_of(origin: int, index: int, pitch: int) -> int:
    """`origin + (index + 0.5) · pitch`, rounded half up — SPEC §5's one
    rounding rule, and the reason a 645 mm pitch lands a centre on a half
    step."""
    return math.floor(origin + (index + 0.5) * pitch + 0.5)


def derived_cells(origin, size, pitch, exclusions):
    """The centres a tiling derives, less the ones its own carve-outs remove.

    A carve-out is stated from the region's origin and removes a cell whose
    centre falls STRICTLY inside it, so a centre sitting exactly on an edge
    survives — the reader's `Exclusion::contains_strictly`.
    """
    cols, rows = size[0] // pitch[0], size[1] // pitch[1]
    assert cols * pitch[0] == size[0] and rows * pitch[1] == size[1], (
        f"region {size} is not a whole number of {pitch} tiles"
    )

    def removed(centre):
        for hole in exclusions:
            low = [origin[axis] + hole["origin"][axis] for axis in (0, 1)]
            high = [low[axis] + hole["size"][axis] for axis in (0, 1)]
            if all(low[axis] < centre[axis] < high[axis] for axis in (0, 1)):
                return True
        return False

    out = set()
    for col in range(cols):
        x = centre_of(origin[0], col, pitch[0])
        for row in range(rows):
            centre = (x, centre_of(origin[1], row, pitch[1]))
            if not removed(centre):
                out.add(centre)
    return out


def beyond(actual, wanted, tolerance):
    """The reader's own `expect` test: squared distance against squared
    tolerance, so a pin is a circle and not a box."""
    return sum((actual[axis] - wanted[axis]) ** 2 for axis in (0, 1)) > tolerance**2


# ---------------------------------------------------------------------------
# Reading the floor out of the document
# ---------------------------------------------------------------------------


def find_lane_bands(entities):
    """The six band-per-bay rectangles whose tilings derive the relay lanes.

    Identified by geometry, not by name: a lane band is a tiled region whose
    pitch is one 850 mm column by one 645 mm board spacing, exactly LANE_ROWS
    rows deep — the three board positions inside one bay — and as wide as the
    whole site. Nothing else in this document is tiled at two different pitches.
    The names ("open floor: lane past bay N") and the tiling `$comment` say the
    same thing in prose, and are checked against the geometry rather than
    trusted.
    """
    bands = []
    for entity in entities:
        components = entity.get("components") or {}
        tiling, region = components.get("tiling"), components.get("region")
        if not tiling or not region:
            continue
        if tiling["pitch"] != [mm(PITCH_X), mm(LANE_PITCH_Y)]:
            continue
        if region["size"][1] != mm(LANE_ROWS * LANE_PITCH_Y):
            continue
        bands.append(entity)
    bands.sort(key=lambda e: e["components"]["region"]["origin"][1])
    assert len(bands) == EXPECT_LANE_BANDS, (
        f"expected {EXPECT_LANE_BANDS} lane bands at pitch "
        f"{[mm(PITCH_X), mm(LANE_PITCH_Y)]}, found {len(bands)}"
    )
    for index, band in enumerate(bands):
        assert "lane past bay" in (band.get("name") or ""), (
            f"the band at y={band['components']['region']['origin'][1]} is shaped like a "
            f"relay lane but is named {band.get('name')!r}"
        )
        origin = band["components"]["region"]["origin"][1]
        expected = bands[0]["components"]["region"]["origin"][1] + index * mm(BAY_LEN)
        assert origin == expected, (
            f"lane band {index} starts at y={origin}, not one bay ({mm(BAY_LEN)}) past "
            f"its predecessor at y={expected}"
        )
    return bands


def sandwich_edges(entities):
    """The x-span each rack sandwich occupies, as `(left, right)` pairs.

    A face stamp's pose is its rack column's FAR-X boundary: a bay stamped at
    heading 90 puts its internal pose at x = depth, with its footprint running
    0..depth back from there. So the sandwich — left column, VTM aisle, right
    column — spans `[left pose − one column, right pose]`. The zone footprints
    confirm it: zone 17's footprint is [15300, 17850] and its faces stand at
    16150 and 17850.
    """
    faces = {}
    for entity in entities:
        match = re.match(r"rack run: old zone (\d+) (left|right) face", entity.get("name") or "")
        if match:
            zone, side = int(match.group(1)), match.group(2)
            faces.setdefault(zone, {})[side] = entity["components"]["pose"]["position"][0]
    assert len(faces) == EXPECT_ZONES, f"expected {EXPECT_ZONES} zones, found {len(faces)}"
    for zone, sides in faces.items():
        assert set(sides) == {"left", "right"}, f"old zone {zone} states only {set(sides)}"
    return sorted((sides["left"] - PITCH_X, sides["right"]) for sides in faces.values())


def patch_rectangles(entities, map_width):
    """Every 3-row patch of open floor, as `(x0, width, gap index, slot)`.

    The gaps between sandwiches — and the two outer margins — are the only
    places an HTM has floor to stand on. Each gap is filled with whole 850 mm
    columns, at most GAP_COLS_MAX to a patch, so the four wide cross-aisles take
    two patches side by side and the sixteen ordinary gaps take one.
    """
    spans, cursor = [], 0.0
    for left, right in sandwich_edges(entities):
        spans.append((cursor, left))
        cursor = right
    spans.append((cursor, map_width))

    rectangles = []
    for gap, (x0, x1) in enumerate(spans):
        # Whole columns only — a part-column at a gap's edge gets no floor, the
        # same no-silent-remainder stance the tiling rule itself takes.
        columns = int((x1 - x0) // PITCH_X)
        assert columns * PITCH_X == x1 - x0, (
            f"gap {gap} spans {x1 - x0} mm, which is not a whole number of "
            f"{mm(PITCH_X)} mm columns"
        )
        placed = 0
        while columns > 0:
            take = min(columns, GAP_COLS_MAX)
            rectangles.append((x0 + placed * PITCH_X, take * PITCH_X, gap, placed // GAP_COLS_MAX))
            placed += take
            columns -= take
    return rectangles


# ---------------------------------------------------------------------------
# What points at the bands, and what to do with each kind
# ---------------------------------------------------------------------------


def strands(document, doomed):
    """Everything pointing at `doomed`, sorted into what can be carried across.

    Three kinds reach a band's identity, and only two of them have a home on the
    other side of this cut: a record whose `derivation` names the band as the
    generator of a cell it overrides (re-addressable, because the cell still
    exists under a patch), and a `stitch` naming the band as a seam endpoint
    (re-declarable, because the seam still exists between the patches). Anything
    else is returned unclassified and stops the run.
    """
    overrides, seams, unclassified = [], [], []

    def scan(value, owner, path):
        if isinstance(value, dict):
            if value.get("generator") in doomed:
                unclassified.append(f"{owner}: {path} generates from {value['generator']}")
            for key in ("a", "b"):
                if value.get(key) in doomed:
                    unclassified.append(f"{owner}: {path}.{key} names {value[key]}")
            for name, item in value.items():
                scan(item, owner, f"{path}.{name}")
        elif isinstance(value, list):
            for index, item in enumerate(value):
                scan(item, owner, f"{path}[{index}]")

    for entity in document["entities"]:
        if entity.get("id") in doomed:
            continue
        owner = entity.get("name") or entity.get("id") or "an override record"
        components = entity.get("components") or {}
        if "stitch" in components and (
            components["stitch"].get("a") in doomed or components["stitch"].get("b") in doomed
        ):
            seams.append(entity)
            continue
        derivation = entity.get("derivation")
        if isinstance(derivation, dict) and derivation.get("generator") in doomed:
            if re.fullmatch(r"cell:\d+,\d+", derivation.get("key") or ""):
                overrides.append(entity)
            else:
                unclassified.append(
                    f"{owner}: `derivation` addresses {derivation['key']!r}, which is not "
                    f"a tiled cell this cut knows how to re-address"
                )
            continue
        scan(components, owner, "components")
        identity = entity.get("id")
        if isinstance(identity, str) and identity.split("/", 1)[0] in doomed:
            unclassified.append(f"{owner}: its own `id` is rooted in {identity.split('/', 1)[0]}")
    return overrides, seams, unclassified


def touching_pairs(rects):
    """Every pair of floor rectangles close enough to be one seam, with the gap
    each pair actually spans.

    Two rectangles are a seam when they overlap along one axis and sit within
    SEAM_REACH along the other. Diagonal neighbours — a patch one bay up and one
    gap over — overlap on neither and are not seams.
    """
    out = []
    for i in range(len(rects)):
        for j in range(i + 1, len(rects)):
            ax, ay, aw, ah, aid = rects[i]
            bx, by, bw, bh, bid = rects[j]
            gap_x = max(bx - (ax + aw), ax - (bx + bw))
            gap_y = max(by - (ay + ah), ay - (by + bh))
            if (gap_x <= SEAM_REACH and gap_y < 0) or (gap_y <= SEAM_REACH and gap_x < 0):
                out.append((aid, bid, max(gap_x, gap_y, 0.0)))
    return out


# ---------------------------------------------------------------------------
# The cut
# ---------------------------------------------------------------------------


def recut(document, migration):
    """Rewrite `document` in place. Returns the summary lines to print, or None
    when something in the document stopped the cut."""
    entities = document["entities"]
    lines = []

    # ---- Step 1: every hole onto the tiling it is a hole in ----------------
    carriers = [e for e in entities if "exclusion" in (e.get("components") or {})]
    clips = [e for e in carriers if (e.get("name") or "").startswith("clip")]
    footprints = [e for e in carriers if e not in clips]
    assert len(carriers) == EXPECT_EXCLUSION_ENTITIES, (
        f"expected {EXPECT_EXCLUSION_ENTITIES} exclusion entities, found {len(carriers)}"
    )
    assert len(footprints) == EXPECT_ZONES, (
        f"expected {EXPECT_ZONES} zone footprints among them, found {len(footprints)}"
    )
    # A footprint survives the migration — it still names its zone and still
    # draws its outline — but the prose explaining it rides on the component
    # about to be deleted. The migration copies that prose into every carve-out
    # the rectangle becomes, and for a footprint those land on the six lane
    # bands (which this cut then deletes) and on the upper blanket band. Saying
    # it on the entity too is what keeps it beside the outline it is about,
    # reworded from the present tense of a rule to the past tense of a record:
    # after this cut the footprint takes nothing away.
    for footprint in footprints:
        archaeology = (footprint["components"]["exclusion"].get("$comment") or "").strip()
        footprint["$comment"] = (
            "The zone's outline, kept as the record of where the steel stands and which "
            "old zone id it answers to. It takes no floor away: the blanket bands state "
            "this rectangle among their own carve-outs, and the relay lane beside it is "
            "authored as patches that were never laid under the steel in the first "
            "place. What the rectangle used to mean: " + archaeology
        )

    before_entities = len(entities)
    before_declared = sum(1 for e in entities if "id" in e)
    outcome = migration.migrate(document, str(FIXTURE), [])
    assert outcome is not None, "the document states no `exclusion` component to migrate"
    lines.extend(f"  {line}" for line in outcome[0])
    entities = document["entities"]
    assert "exclusion" not in document.get("required_components", []), (
        "`exclusion` is still required by the document"
    )

    # ---- Step 2: the lane bands become patches ------------------------------
    bands = find_lane_bands(entities)
    frame = bands[0]["components"]["region"]["frame"]
    lane_y0 = bands[0]["components"]["region"]["origin"][1]
    map_width = bands[0]["components"]["region"]["size"][0]
    assert map_width == mm(EXPECT_GRID_COLUMNS * PITCH_X), (
        f"the lane bands span {map_width} mm, not the site's "
        f"{mm(EXPECT_GRID_COLUMNS * PITCH_X)} mm"
    )
    derives = copy.deepcopy(bands[0]["components"]["tiling"]["derives"])
    for band in bands[1:]:
        assert band["components"]["tiling"]["derives"] == derives, (
            f"{band['name']!r} derives cells unlike the first band's, so one patch "
            f"template cannot stand for all six"
        )

    # What the six bands derive today, hole by hole, as the reader would read
    # it. The patches have to land on exactly this set of centres.
    was = set()
    for band in bands:
        region, tiling = band["components"]["region"], band["components"]["tiling"]
        was |= derived_cells(
            region["origin"], region["size"], tiling["pitch"], tiling.get("exclusions", [])
        )
    assert len(was) == EXPECT_LANE_CELLS, (
        f"the six lane bands derive {len(was)} cells, not the {EXPECT_LANE_CELLS} this "
        f"cut is meant to reproduce"
    )

    rectangles = patch_rectangles(entities, map_width)
    assert len(rectangles) == EXPECT_PATCHES, (
        f"expected {EXPECT_PATCHES} patches across the gaps, found {len(rectangles)}"
    )
    widths = sorted({int(width) for _, width, _, _ in rectangles})
    assert widths == [mm(GAP_COLS_MAX * PITCH_X)], (
        f"a patch is meant to be {GAP_COLS_MAX} columns wide; these are {widths} mm"
    )

    template_id = mint("floor/patch-template")
    minted = [
        {
            "id": template_id,
            "name": "template:open-patch",
            "components": {
                "template": {
                    "$comment": (
                        "One bay's worth of HTM standing room in a gap between rack "
                        "sandwiches: 3 rows at the boards' own 645 mm spacing, `width` "
                        "columns of 850 across the gap. The floor exists only where the "
                        "work is — the aisles between these patches carry no cells, so "
                        "nothing has to carve them back out afterwards. Every gap in "
                        "this building takes the same 4 columns; `width` is a parameter "
                        "because the next building's need not."
                    ),
                    "params": {"width": {}},
                    "entities": [
                        {
                            "local": "floor",
                            "components": {
                                # No frame: a frame-less template region composes
                                # with the stamp's pose. Naming a frame would
                                # anchor it to the world and stack every patch at
                                # the origin.
                                "region": {
                                    "origin": [0, 0],
                                    "size": [
                                        {"$param": "width"},
                                        mm(LANE_ROWS * LANE_PITCH_Y),
                                    ],
                                },
                                "tiling": {
                                    "derives": derives,
                                    "pitch": [mm(PITCH_X), mm(LANE_PITCH_Y)],
                                },
                            },
                        }
                    ],
                }
            },
        }
    ]
    # (x0, y0, width, height, id) for every patch: what the re-addressing looks
    # a cell centre up in, and what the seam search runs over.
    patches = []
    for x0, width, gap, slot in rectangles:
        for bay in range(EXPECT_LANE_BANDS):
            identity = mint(f"floor/patch/gap{gap}/slot{slot}/bay{bay}")
            y0 = lane_y0 + bay * BAY_LEN
            patches.append((x0, y0, width, LANE_ROWS * LANE_PITCH_Y, identity))
            minted.append(
                {
                    "id": identity,
                    "name": f"open patch: gap {gap} bay {bay + 1}"
                    + (f" ({slot})" if slot else ""),
                    "components": {
                        "pose": {"frame": frame, "position": [mm(x0), mm(y0)]},
                        "stamp": {
                            "template": template_id,
                            "params": {"width": mm(width)},
                        },
                    },
                }
            )
    assert len(patches) == EXPECT_STAMPS, (
        f"expected {EXPECT_STAMPS} patch stamps, minted {len(patches)}"
    )

    # A stamp at heading 0 translates its template's region and leaves the pitch
    # alone, so a patch derives exactly what a region at its pose would.
    now = set()
    for x0, y0, width, height, _ in patches:
        now |= derived_cells(
            [mm(x0), mm(y0)],
            [mm(width), mm(height)],
            [mm(PITCH_X), mm(LANE_PITCH_Y)],
            [],
        )
    assert now == was, (
        f"the patches land on {len(now)} cell centres and the bands on {len(was)}; "
        f"{len(now - was)} are new floor and {len(was - now)} are floor lost — a patch "
        f"scheme that moves a cell is not this cut"
    )
    lines.append(
        f"  {len(patches)} patch stamps of 1 template reproduce all {len(was)} relay-lane "
        f"cell centres, exactly"
    )

    # ---- Step 3: carry across everything that pointed at the bands ---------
    doomed = {band["id"] for band in bands}
    overrides, old_seams, unclassified = strands(document, doomed)
    if unclassified:
        print(
            f"{FIXTURE}: refused — {len(unclassified)} reference(s) into the lane bands "
            f"have no home on the other side of this cut:",
            file=sys.stderr,
        )
        for detail in unclassified[:8]:
            print(f"    {detail}", file=sys.stderr)
        if len(unclassified) > 8:
            print(f"    ... and {len(unclassified) - 8} more", file=sys.stderr)
        return None
    assert len(overrides) == EXPECT_STRANDED_OVERRIDES, (
        f"expected {EXPECT_STRANDED_OVERRIDES} overrides addressing lane-band cells, "
        f"found {len(overrides)}"
    )
    assert len(old_seams) == EXPECT_STRANDED_STITCHES, (
        f"expected {EXPECT_STRANDED_STITCHES} seams naming a lane band, found "
        f"{len(old_seams)}"
    )

    band_by_id = {band["id"]: band for band in bands}

    def patch_under(x, y):
        """The patch a millimetre position stands in, and the address of the
        cell it stands on. A stamp expands under `instance:{i}` and then under
        each internal's `local:{handle}`, so a plain stamp's cell is reached
        through `instance:0/local:floor` (SPEC §3)."""
        for x0, y0, width, height, identity in patches:
            if x0 <= x <= x0 + width and y0 <= y <= y0 + height:
                return (
                    identity,
                    f"instance:0/local:floor/cell:{int((x - x0) // PITCH_X)},"
                    f"{int((y - y0) // LANE_PITCH_Y)}",
                    (x0, y0),
                )
        return None

    off_pin = []
    for record in overrides:
        derivation = record["derivation"]
        band = band_by_id[derivation["generator"]]
        col, row = (int(v) for v in derivation["key"].split(":")[1].split(","))
        region = band["components"]["region"]
        pitch = band["components"]["tiling"]["pitch"]
        stood_at = (
            centre_of(region["origin"][0], col, pitch[0]),
            centre_of(region["origin"][1], row, pitch[1]),
        )
        found = patch_under(*stood_at)
        assert found, f"no patch stands where {derivation['key']} did, at {stood_at}"
        identity, key, (x0, y0) = found
        derivation["generator"], derivation["key"] = identity, key
        # The address now names a cell of a patch. Derive that cell's centre the
        # way the reader will and hold it against the pin the record carries,
        # rather than leaving the loader to be the only thing that checks.
        local_col, local_row = (int(v) for v in key.split("cell:")[1].split(","))
        landed = (
            centre_of(mm(x0), local_col, mm(PITCH_X)),
            centre_of(mm(y0), local_row, mm(LANE_PITCH_Y)),
        )
        pin = record.get("expect")
        if pin is None:
            off_pin.append((key, landed, "carries no `expect` pin"))
        elif beyond(landed, pin["position"], pin["tolerance"]):
            off_pin.append((key, landed, f"pinned at {pin['position']}"))
    if off_pin:
        print(
            f"{FIXTURE}: refused — {len(off_pin)} re-addressed override(s) do not land "
            f"where the document says they should:",
            file=sys.stderr,
        )
        for key, landed, why in off_pin[:8]:
            print(f"    {key} lands at {landed}, {why}", file=sys.stderr)
        return None
    lines.append(
        f"  {len(overrides)} parking overrides re-addressed to the patch cell at the same "
        f"millimetre, every one inside its own `expect` pin"
    )

    # The seams the patch scheme needs. Each states the gap it actually spans
    # plus slack, rather than one blanket figure: a generous tolerance lets
    # `contact` reach past a cell's straight partner to a diagonal one.
    blankets = [
        entity
        for entity in entities
        if "tiling" in (entity.get("components") or {}) and entity["id"] not in doomed
    ]
    rects = list(patches)
    for blanket in blankets:
        region = blanket["components"]["region"]
        rects.append(
            (
                float(region["origin"][0]),
                float(region["origin"][1]),
                float(region["size"][0]),
                float(region["size"][1]),
                blanket["id"],
            )
        )
    seams = []
    for a, b, gap in touching_pairs(rects):
        seams.append(
            {
                "id": mint(f"floor/stitch/{a}/{b}"),
                "name": "stitch: floor seam",
                "components": {
                    "stitch": {
                        "$comment": (
                            "A flat seam where one piece of floor hands over to the next "
                            "(SPEC §5). The tolerance is this seam's own gap plus slack, "
                            "not one figure for the whole floor: reach further than the "
                            "gap and `contact` can pair a cell with a diagonal neighbour "
                            "instead of the partner straight across."
                        ),
                        "a": a,
                        "b": b,
                        "rule": "contact",
                        "tolerance": mm(gap) + SEAM_SLACK,
                    }
                },
            }
        )
    assert len(seams) == EXPECT_SEAMS, (
        f"expected {EXPECT_SEAMS} seams between the patches and the blanket bands, "
        f"found {len(seams)}"
    )
    lines.append(
        f"  {len(old_seams)} band-to-band seams -> {len(seams)} patch seams, tolerances "
        f"{sorted({s['components']['stitch']['tolerance'] for s in seams})}"
    )

    drop = doomed | {seam["id"] for seam in old_seams}
    document["entities"] = [e for e in entities if e.get("id") not in drop] + minted + seams

    # Nothing may be left naming a band. Checked against the text about to be
    # written, which is the only thing a later reader sees.
    text = render(document)
    still_named = [identity for identity in doomed if identity in text]
    if still_named:
        print(
            f"{FIXTURE}: refused — {len(still_named)} deleted band(s) are still named in "
            f"the document that was about to be written: {', '.join(still_named)}",
            file=sys.stderr,
        )
        return None

    # ---- Step 4: say what this document is ---------------------------------
    document["$comment"] = (
        "Hand-managed document of record (since 2026-08-21). desmap2proto seeded this "
        "file from sim/desmap/tests/fixtures/cencora_v3.json and no longer produces it; "
        "edit it here, by hand or by scripts/recut_cencora_floor.py. Every hole belongs "
        "to the tiling it is a hole in: a region states the floor it has, and its own "
        "`exclusions` state the parts of that floor the building does not — nothing "
        "reaches across to subtract cells from a tiling it does not own. The relay lanes "
        "are not stated as floor at all but as open-cell patches, one per bay in each gap "
        "between rack sandwiches, so no floor is ever declared under the steel or in a "
        "VTM aisle, and the seams between them are declared. Legacy identifiers ride as "
        "external_ids, never as identity."
    )

    lines.append(
        f"  entities: {before_entities} -> {len(document['entities'])} "
        f"(declared {before_declared} -> "
        f"{sum(1 for e in document['entities'] if 'id' in e)})"
    )
    return lines


def render(document):
    """The document as text, in the layout this file already has: two-space
    indent, keys sorted, non-ASCII left as itself, and no trailing newline."""
    return json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--dry-run", action="store_true", help="say what would change, write nothing"
    )
    args = parser.parse_args()

    if not FIXTURE.exists():
        print(f"{FIXTURE}: not here — run this from the repository root", file=sys.stderr)
        return 1
    document = json.loads(FIXTURE.read_text(encoding="utf-8"))
    lines = recut(document, load_migration())
    if lines is None:
        return 1
    for line in lines:
        print(line)
    if args.dry_run:
        print(f"{FIXTURE}: --dry-run, nothing written")
        return 0
    FIXTURE.write_text(render(document), encoding="utf-8")
    print(f"{FIXTURE}: written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
