"""Writes data/seed/studio/ (the five shell templates and the catalog items they pin, spec 05 section 5) from the template definitions and checked-in desvis robot sources, each id from ids.json so a re-run rewrites the same bytes; `--check` writes nothing and exits 1 where a document differs."""
import json
import pathlib
import sys
import uuid

HERE = pathlib.Path(__file__).parent
IDS_FILE = HERE / "ids.json"
IDS = json.loads(IDS_FILE.read_text()) if IDS_FILE.exists() else {}
USED = set()  # the names this run writes: a retired name leaves the ledger
CHECK = "--check" in sys.argv[1:]
WRITES = {}  # path under data/seed/studio -> the text it should hold


def mint(name):
    """The random v4 id `name` was given when it was first written, or a new one: never derived from the name."""
    if name not in IDS:
        IDS[name] = str(uuid.uuid4())
    USED.add(name)
    return IDS[name]


PROTO = "0.7.0"
# In mm: a floor cell, a station strip's depth, the aisle pitch (two 1,300 mm rack rows, a 1,650 mm VTM aisle, two
# 1,700 mm half-gaps), and where the aisle row starts; the gap length is a rack row of 2,750 mm bays plus an 850 mm
# end each side, in whole cells.
C, S, P, X0, Y0 = 850, 1700, 7650, 6800, 4250
GAP_LENGTH = "(bays * 2750 + 2549) / 850 * 850"
# An outline knob's default is the length its contents need (D120), left to its 850 mm step to round up to whole
# cells, so that the Square's fits the 200 characters a default may hold: along y the rack rows' bays, their ends and
# the 4,250 mm before and 3,400 mm beyond them (`GAP_LENGTH + 7650` once rounded).
RACK_ROWS_Y = "bays * 2750 + 9350"

def doc(path, docname, name, kind, entities, primary_frame=None, depends_on=(), comment=None):
    d = {"proto_version": PROTO}
    if comment:
        d["$comment"] = comment
    d["doc"] = {"id": mint(docname), "name": name, "kind": kind, "depends_on": list(depends_on)}
    if primary_frame:
        d["primary_frame"] = primary_frame
    d["entities"] = entities
    WRITES[path] = json.dumps(d, indent=2, ensure_ascii=False) + "\n"
    return d["doc"]["id"]

def ent(idname, name, components):
    e = {"id": mint(idname)}
    if name:
        e["name"] = name
    e["components"] = components
    return e

# ---------------------------------------------------------------------------------------------------------------
# Catalog items (scopes catalog/...; each a boundary scope pinned at changeset 1 on a freshly seeded server)

TOTE = mint("catalog/tote")
BAY = mint("catalog/relay-bay")
PATCH = mint("catalog/open-patch")
DA50 = mint("catalog/da50")
D30 = mint("catalog/d30")
PIN = "@1"

tote_doc = doc("catalog/carrier-tote.world.json", "doc:catalog/tote", "Tote 650 x 450 (high wall)", "catalog", [
    ent("catalog/tote", "tote 650x450 (high wall)", {
        "carrier_type": {"kind": "tote", "size": [650, 450, 300]}}),
], comment="The inventory tote the relay bay stores, shaped as the GO catalog's tote (world-proto examples/walmart-dallas/fleet.world.json).")

GO_LEVELS = [{"height": 350},
             {"height": 400, "positions": [[340, 1300], [1030, 1300], [1720, 1300], [2410, 1300]], "reach": 1750}] + \
            [{"height": h} for h in range(700, 8051, 350)]
bay_doc = doc("catalog/artifact-relay-bay.world.json", "doc:catalog/relay-bay", "Relay bay, 5 wide, double-deep, 23 levels, 4 relays", "catalog", [
    ent("catalog/relay-bay", "Relay bay, 5 wide, double-deep, 23 levels, 4 relays", {
        "template": {"params": {}, "entities": [{"local": "rack", "components": {
            "pose": {"position": [0, 0], "heading_deg": 0},
            "rack": {"slots_wide": 5, "slots_deep": 2, "slot_pitch": [550, 650], "footprint": [2750, 1300], "levels": GO_LEVELS},
            "storage": {"holds": "inventory", "carrier_type": TOTE + PIN}}}]}}),
], depends_on=[mint("doc:catalog/tote")],
   comment="One bay of relay racking: the GO catalog's 5-wide double-deep rack (world-proto examples/walmart-dallas/catalog.world.json), its relays on the +y face, storing the catalog tote. Its relays bind the floor cells beside that face when the reader loads a site.")

patch_doc = doc("catalog/artifact-open-patch.world.json", "doc:catalog/open-patch", "Open floor patch", "catalog", [
    ent("catalog/open-patch", "Open floor patch, 850 mm cells", {
        "template": {"params": {"size": {}}, "entities": [{"local": "floor", "components": {
            "region": {"origin": [0, 0], "size": {"$param": "size"}},
            "tiling": {"pitch": [850, 850], "derives": {"cell": {"traversable_by": ["htm"]}}}}}]}}),
], comment="A patch of open floor the HTMs drive, tiled at 850 mm, its size a parameter in whole cells.")

def robot_catalog(key, model, source, role, locomotion):
    """Bootstrap from the checked-in desvis source; production reads the WPS export only."""
    source_path = HERE.parents[2] / "sim/dessim/tests/fixtures/robots" / source
    original = json.loads(source_path.read_text())
    components = original["entities"][0]["components"]
    robot_class = {"role": role, "model": model, "configuration": components["end_effector"]["kind"],
                   "locomotion": locomotion, "carries": ["tote"]}
    # The incumbent robot_class reader still consumes geometry there, in the
    # millimetre frame of these seeds. Preserve the source variant caveat too.
    body = components.get("body", {})
    for old, new in [("footprint_mm", "footprint"), ("height_mm", "height"), ("$comment", "$comment")]:
        if old in body:
            robot_class[new] = body[old]
    provenance = f"Bootstrap source: sim/dessim/tests/fixtures/robots/{source}. " + original["$comment"]
    if "$comment" in robot_class:
        provenance += " " + robot_class["$comment"]
    # Catalog document comments are not retained in an assembled site export;
    # the component carries the provenance so the runner's pinned input does.
    robot_class["$comment"] = provenance
    seeded = {"robot_class": robot_class}
    for name in ["lineage", "floor_drive", "rack_ladder", "lift", "end_effector"]:
        if name in components:
            seeded[name] = components[name]
    if "floor_drive" in seeded:
        seeded["floor_drive"]["kind"] = "trapezoidal"
        if role == "htm":
            # The current Cencora scenario states its constant-mode execution cruise
            # separately from the M5F chassis maximum. Preserve both sources.
            scenario_source = "sim/dessim/tests/fixtures/cencora_v3_spec.json"
            settings = json.loads((HERE.parents[2] / scenario_source).read_text())["settings"]["htm"]
            for load in ["loaded", "empty"]:
                seeded["floor_drive"][load]["constant_speed_mm_s"] = settings[f"{load}_constant_speed"] * 1000
            robot_class["$comment"] += f" Constant-mode execution cruise: {scenario_source}, settings.htm."
    return doc(f"catalog/agent-{key}.world.json", f"doc:catalog/{key}", original["doc"]["name"], "catalog", [
        ent(f"catalog/{key}", original["doc"]["name"], seeded),
    ], comment=f"Bootstrap source: sim/dessim/tests/fixtures/robots/{source}. " + original["$comment"])


da50_doc = robot_catalog("da50", "DA50", "quicktron_a5.world.json", "vtm", "ladder")
d30_doc = robot_catalog("d30", "D30", "quicktron_m5f.world.json", "htm", "floor")

CATALOG_ENTRIES = [
    ("catalog/carrier/tote", "catalog/carrier-tote.world.json"),
    ("catalog/artifact/relay-bay", "catalog/artifact-relay-bay.world.json"),
    ("catalog/artifact/open-patch", "catalog/artifact-open-patch.world.json"),
    ("catalog/agent/da50", "catalog/agent-da50.world.json"),
    ("catalog/agent/d30", "catalog/agent-d30.world.json"),
]
CATALOG_DOCS = [mint("doc:catalog/tote"), mint("doc:catalog/relay-bay"), mint("doc:catalog/open-patch"),
                mint("doc:catalog/da50"), mint("doc:catalog/d30")]

# ---------------------------------------------------------------------------------------------------------------
# Shared knob, piece and flow builders

def knob(t, key, label, unit, default, description, group, order, rng=None, options=None, checks=()):
    k = {"key": key, "label": label, "unit": unit}
    if rng:
        k["min"], k["max"], k["step"] = rng
    if options:
        k["options"] = [{"value": v, "label": l} for v, l in options]
    k.update({"default": default, "description": description, "group": group, "order": order})
    if checks:
        k["checks"] = [dict(c) for c in checks]
    return ent(f"{t}/knob/{key}", label, {"template.knob": k})

def check(fails_when, problem, unit="mm", severity="error"):
    return {"severity": severity, "value_unit": unit, "fails_when": fails_when, "problem": problem}

def length(t, key, label, default, description, order, checks=()):
    return knob(t, key, label, "mm", default, description, "Outline", order, rng=(4250, 255000, 850), checks=checks)

def common_knobs(t, htm_sum):
    return [
        knob(t, "aisles", "Aisles", "count", 4, "Aisles of two rack rows each, laid side by side along +x.", "Storage", 1,
             rng=(1, 30, 1)),
        knob(t, "bays", "Bays per rack row", "count", 4, "Bays of relay racking in each rack row, along each aisle.", "Storage", 2,
             rng=(2, 12, 1)),
        knob(t, "vtms_per_aisle", "VTMs per aisle", "count", 2, "VTMs hard-assigned to each aisle.", "Storage", 3,
             options=[(1, "1 VTM"), (2, "2 VTMs")]),
        knob(t, "station_pitch", "Station pitch", "mm", 4250, "Distance from one station to the next along a wall.", "Stations", 10,
             rng=(2550, 12750, 850)),
        knob(t, "htms", "HTMs", "count", 24, "HTMs working the floor.", "Fleet", 20, rng=(1, 600, 1),
             checks=[check(f"{htm_sum} - htms", "Fewer HTMs than stations: {value} stations have no HTM of their own.", unit="count", severity="warning")]),
    ]

def frame_entities(t, binding=None):
    shell = {"frame": {"parent": mint(f"{t}/frame/site"), "dims": 2, "units": "mm",
                       "transform_to_parent": {"translation": [0, 0, 0], "scale": [1, 1]}}}
    if binding:
        shell["template.knob_binding"] = binding
    return [
        ent(f"{t}/frame/site", "site", {"frame": {"dims": 3, "units": "mm", "unit_mm": 1}}),
        ent(f"{t}/frame/shell", "shell", shell),
    ]

def tiling():
    return {"pitch": [C, C], "derives": {"cell": {"traversable_by": ["htm"]}}}

# A piece names the floor it stitches to by the floor's id, and a station piece stands on one wall, because
# world-proto's registered `template` schema (60aa424) refuses a `$param` at a reference or inside a cell address,
# though SPEC §6 lets one stand anywhere; each shell has one floor and each wall a fixed lane.
def aisle_piece(t):
    bay, patch, floor = BAY + PIN, PATCH + PIN, mint(f"{t}/floor/interior")
    L = 12750
    return ent(f"{t}/piece/aisle", "aisle piece", {"template": {"params": {}, "entities": [
        {"local": "gap_left", "name": "aisle's left half-gap", "components": {
            "pose": {"position": [0, 0], "heading_deg": 0},
            "stamp": {"template": patch, "params": {"size": [1700, L]}},
            "template.knob_binding": {"/stamp/params/size/1": GAP_LENGTH}}},
        {"local": "rack_left", "name": "left rack row", "components": {
            "pose": {"position": [3000, 850], "heading_deg": 90},
            "stamp": {"template": bay, "repeat": {"count": 4, "step": [0, 2750]}},
            "template.knob_binding": {"/stamp/repeat/count": "bays"}}},
        {"local": "vtm_aisle", "name": "VTM aisle", "components": {
            "region": {"origin": [3000, 850], "size": [1650, 11000]},
            "template.knob_binding": {"/region/size/1": "bays * 2750"}}},
        {"local": "rack_right", "name": "right rack row", "components": {
            "pose": {"position": [4650, 3600], "heading_deg": 270},
            "stamp": {"template": bay, "repeat": {"count": 4, "step": [0, 2750]}},
            "template.knob_binding": {"/stamp/repeat/count": "bays"}}},
        {"local": "gap_right", "name": "aisle's right half-gap", "components": {
            "pose": {"position": [5950, 0], "heading_deg": 0},
            "stamp": {"template": patch, "params": {"size": [1700, L]}},
            "template.knob_binding": {"/stamp/params/size/1": GAP_LENGTH}}},
        {"local": "access_left", "name": "access: left rack row from the VTM aisle", "components": {
            "access": {"rack": {"local_ref": "rack_left"}, "along": {"local_ref": "vtm_aisle"}, "face": "-y", "serves": [DA50 + PIN]}}},
        {"local": "access_right", "name": "access: right rack row from the VTM aisle", "components": {
            "access": {"rack": {"local_ref": "rack_right"}, "along": {"local_ref": "vtm_aisle"}, "face": "-y", "serves": [DA50 + PIN]}}},
        {"local": "sector", "name": "aisle", "components": {
            "sector": {"members": [{"local_ref": "rack_left"}, {"local_ref": "rack_right"}]}}},
        {"local": "vtms", "name": "aisle VTMs", "components": {
            "cohort": {"robot_class": DA50 + PIN, "count": 2, "assignment": {"kind": "hard", "to": [{"local_ref": "sector"}]}},
            "template.knob_binding": {"/cohort/count": "vtms_per_aisle"},
            "template.target_scope": {"scope": "fleet"}}},
        {"local": "stitch_left", "name": "stitch: left half-gap to the floor", "components": {
            "stitch": {"a": {"local_ref": "gap_left"}, "b": floor, "rule": "contact", "tolerance": 100}}},
        {"local": "stitch_right", "name": "stitch: right half-gap to the floor", "components": {
            "stitch": {"a": {"local_ref": "gap_right"}, "b": floor, "rule": "contact", "tolerance": 100}}},
    ]}})

KIND_LABELS = {"pick": "pick station", "put": "putaway station"}

def station_piece(t, kind, wall):
    label, w = KIND_LABELS[kind], WALLS[wall]
    return ent(f"{t}/piece/{kind}/{wall}", f"{label} piece, {WALL_WORDS[wall]} wall", {"template": {"params": {}, "entities": [
        {"local": "patch", "name": f"{label} floor", "components": {
            "region": {"origin": [0, 0], "size": [2550, 1700]}, "tiling": tiling()}},
        {"local": "station", "name": label, "components": {
            "pose": {"position": [2125, 425], "heading_deg": 0},
            "station": {"kind": kind, "bins": 1, "exit_direction": w["exit"]}}},
        {"local": "lane", "name": f"{label}: lane", "components": {
            "queue": {"discipline": "no_reverse", "path": [{"local_ref": "patch", "key": key} for key in w["lane"]]}}},
        {"local": "present", "name": f"{label} presentation", "components": {
            "pose": {"position": [1275, 425], "heading_deg": 0},
            "presentation_point": {"for": "inventory_tote", "station": {"local_ref": "station"}}}},
        {"local": "stitch", "name": f"stitch: {label} floor to the floor", "components": {
            "stitch": {"a": {"local_ref": "patch"}, "b": mint(f"{t}/floor/interior"), "rule": "contact", "tolerance": 100}}},
    ]}})

# The lane starts on the floor-facing row, continues towards the wall and turns to the presentation cell, where it
# ends: a lane names every cell a robot queues on and, last, the cell it feeds. Starting beside the presentation
# alone has no collinear predecessor inside the patch, so corridor restriction seals it. Keys are the tiling's
# world-oriented addresses after the station patch rotates (spec 05 5.1).
WALLS = {
    "minus_y": dict(heading=0, exit="+y", lane=["cell:0,1", "cell:0,0", "cell:1,0"], pitch=lambda: ["station_pitch", 0]),
    "plus_y": dict(heading=180, exit="-y", lane=["cell:2,0", "cell:2,1", "cell:1,1"], pitch=lambda: ["station_pitch", 0]),
    "minus_x": dict(heading=270, exit="+x", lane=["cell:1,2", "cell:0,2", "cell:0,1"], pitch=lambda: [0, "station_pitch"]),
    "plus_x": dict(heading=90, exit="-x", lane=["cell:0,0", "cell:1,0", "cell:1,1"], pitch=lambda: [0, "station_pitch"]),
}

def station_row(t, kind, label, wall, position, count_expr, extra_bindings=None, default_count=0):
    w = WALLS[wall]
    pitch = [4250 if p == "station_pitch" else 0 for p in w["pitch"]()]
    # A row along x repeats over i, a row along y over j (spec 05 section 3.2: instance (i, j) at [i * pitch[0], j * pitch[1]]).
    axis = 0 if w["pitch"]()[0] == "station_pitch" else 1
    count = [default_count, 1] if axis == 0 else [1, default_count]
    binding = {f"/template.placement/repeat/count/{axis}": count_expr, f"/template.placement/repeat/pitch/{axis}": "station_pitch"}
    binding.update(extra_bindings or {})
    return ent(f"{t}/row/{kind}/{wall}", f"{label} on the {WALL_WORDS[wall]} wall", {
        "pose": {"frame": mint(f"{t}/frame/shell"), "position": position, "heading_deg": w["heading"]},
        "template.placement": {"piece": mint(f"{t}/piece/{kind}/{wall}"),
                               "repeat": {"count": count, "pitch": pitch}, "stands_for": ["station"]},
        "template.knob_binding": binding})

def aisle_row(t):
    return ent(f"{t}/row/aisles", "aisle row", {
        "pose": {"frame": mint(f"{t}/frame/shell"), "position": [X0, Y0], "heading_deg": 0},
        "template.placement": {"piece": mint(f"{t}/piece/aisle"),
                               "repeat": {"count": [4, 1], "pitch": [P, 0]}, "stands_for": ["sector"]},
        "template.knob_binding": {"/template.placement/repeat/count/0": "aisles"}})

def floor_entity(t, idname, name, origin, size, bindings, exclusions=None, presence=None):
    til = tiling()
    if exclusions:
        til["exclusions"] = exclusions
    comps = {"region": {"frame": mint(f"{t}/frame/shell"), "origin": origin, "size": size}, "tiling": til}
    if bindings:
        comps["template.knob_binding"] = bindings
    if presence:
        comps["template.presence"] = {"when": presence}
    return ent(f"{t}/{idname}", name, comps)

def stitch(t, idname, name, a, b, presence=None):
    comps = {"stitch": {"a": mint(f"{t}/{a}"), "b": mint(f"{t}/{b}"), "rule": "contact", "tolerance": 100}}
    if presence:
        comps["template.presence"] = {"when": presence}
    return ent(f"{t}/{idname}", name, comps)

def sectors(t, pick_rows, put_rows):
    return [
        ent(f"{t}/sector/storage", "Inventory storage", {"sector": {"members": [mint(f"{t}/row/aisles")]}}),
        ent(f"{t}/sector/pick", "Pick stations", {"sector": {"members": [mint(f"{t}/row/pick/{w}") for w in pick_rows]}}),
        ent(f"{t}/sector/put", "Put stations", {"sector": {"members": [mint(f"{t}/row/put/{w}") for w in put_rows]}}),
    ]

def flow_entities(t):
    s = lambda name: mint(f"{t}/flow/{name}")
    storage, pick, put = mint(f"{t}/sector/storage"), mint(f"{t}/sector/pick"), mint(f"{t}/sector/put")
    htm = {"cohort": mint(f"{t}/fleet/htms")}
    vtm_from, vtm_to = {"assigned": "from"}, {"assigned": "to"}
    place = lambda node, port: {"node": s(node), "port": port}
    def edge(name, label, frm, to, moves, **more):
        e = {"from": s(frm), "to": s(to), "moves": moves}
        e.update(more)
        return ent(f"{t}/flow/{name}", label, {"flow.material_edge": e})
    def seg(name, label, edge_name, step, frm, to, executor, at_end, retained=None):
        g = {"edge": s(edge_name), "step": step, "from": frm, "to": to, "executor": executor, "at_end": at_end}
        if retained:
            g["retained_for"] = [s(r) for r in retained]
        return ent(f"{t}/flow/{name}", label, {"flow.agent_segment": g})
    return [
        ent(f"{t}/flow/inbound", "Inbound units", {"flow.node": {"kind": "source", "demand": "putaway"}}),
        ent(f"{t}/flow/put", "Put stations", {"flow.node": {
            "kind": "stations", "sectors": [put],
            "process": {"operation": "put", "gives_by": [s("e_inbound_put")], "receives_by": [s("e_storage_put"), s("e_pick_put")]},
            "work": {"presentations_per_hour": 240, "tote_swap_sim_ms": 2000},
            "intake": [{"moves": "inventory_tote:empty", "admission_factor": 1.5}]}}),
        ent(f"{t}/flow/storage", "Inventory storage", {"flow.node": {
            "kind": "storage", "sectors": [storage], "holds": "inventory_tote", "deposit": "ranked"}}),
        ent(f"{t}/flow/pick", "Pick stations", {"flow.node": {
            "kind": "stations", "sectors": [pick],
            "process": {"operation": "pick", "gives_by": [s("e_storage_pick")], "receives_in": "station_bins"},
            "work": {"pick_sim_ms": 7000, "sequencing": "all_totes", "tote_swap_sim_ms": 2000},
            "intake": [{"moves": "inventory_tote:stock", "admission_factor": 4.0}]}}),
        ent(f"{t}/flow/outbound", "Outbound orders", {"flow.node": {"kind": "sink"}}),
        ent(f"{t}/flow/returns", "Return rule", {"flow.return_rule": {"min_backlog": 1, "destination": "nearest_eligible"}}),
        edge("e_inbound_put", "Inbound units to Put stations: units", "inbound", "put", "units",
             induction={"sequence": "sku_demand", "hour_gate": True, "units_per_tote": 1}),
        edge("e_storage_put", "Inventory storage to Put stations: empty tote", "storage", "put", "inventory_tote:empty"),
        edge("e_pick_put", "Pick stations to Put stations: empty tote", "pick", "put", "inventory_tote:empty",
             alternative_to=s("e_storage_put"), bias="fleet_pressure"),
        edge("e_put_storage_stock", "Put stations to Inventory storage: stock tote", "put", "storage", "inventory_tote:stock",
             returns_by=s("returns")),
        edge("e_put_storage_empty", "Put stations to Inventory storage: empty tote once inbound ends", "put", "storage",
             "inventory_tote:empty", when={"phase": "inbound_ended"}, returns_by=s("returns")),
        edge("e_storage_pick", "Inventory storage to Pick stations: stock tote", "storage", "pick", "inventory_tote:stock",
             opens_at_sim_ms=0),
        edge("e_pick_storage", "Pick stations to Inventory storage: tote after the pick", "pick", "storage",
             "inventory_tote:after_pick", returns_by=s("returns")),
        edge("e_pick_outbound", "Pick stations to Outbound orders: order in its bin", "pick", "outbound", "order:in_bin"),
        seg("g_inbound", "units arrive at Put stations", "e_inbound_put", 1, place("inbound", "boundary"), place("put", "presentation"), {"external": True}, "arrives"),
        seg("g_storage_put_vtm", "the aisle VTMs: slot to relay", "e_storage_put", 1, place("storage", "slot"), place("storage", "relay"), vtm_from, "hands_over"),
        seg("g_storage_put_htm", "HTM fleet: relay to Put stations", "e_storage_put", 2, place("storage", "relay"), place("put", "presentation"), htm, "keeps_through_work"),
        seg("g_pick_put_htm", "HTM fleet: Pick stations to Put stations", "e_pick_put", 1, place("pick", "presentation"), place("put", "presentation"), htm, "keeps_through_work"),
        seg("g_put_stock_htm", "HTM fleet: Put stations to relay", "e_put_storage_stock", 1, place("put", "presentation"), place("storage", "relay"), htm, "hands_over"),
        seg("g_put_stock_vtm", "the aisle VTMs: relay to slot", "e_put_storage_stock", 2, place("storage", "relay"), place("storage", "slot"), vtm_to, "sets_down"),
        seg("g_put_empty_htm", "HTM fleet: Put stations to relay, inbound ended", "e_put_storage_empty", 1, place("put", "presentation"), place("storage", "relay"), htm, "hands_over"),
        seg("g_put_empty_vtm", "the aisle VTMs: relay to slot, inbound ended", "e_put_storage_empty", 2, place("storage", "relay"), place("storage", "slot"), vtm_to, "sets_down"),
        seg("g_storage_pick_vtm", "the aisle VTMs: slot to relay for a pick", "e_storage_pick", 1, place("storage", "slot"), place("storage", "relay"), vtm_from, "hands_over"),
        seg("g_storage_pick_htm", "HTM fleet: relay to Pick stations", "e_storage_pick", 2, place("storage", "relay"), place("pick", "presentation"), htm, "keeps_through_work",
            retained=["g_pick_storage_htm", "g_pick_put_htm"]),
        seg("g_pick_storage_htm", "HTM fleet: Pick stations to relay", "e_pick_storage", 1, place("pick", "presentation"), place("storage", "relay"), htm, "hands_over"),
        seg("g_pick_storage_vtm", "the aisle VTMs: relay to slot after a pick", "e_pick_storage", 2, place("storage", "relay"), place("storage", "slot"), vtm_to, "sets_down"),
        seg("g_pick_outbound", "orders leave in their bins", "e_pick_outbound", 1, place("pick", "presentation"), place("outbound", "boundary"), {"external": True}, "leaves"),
    ]

def write_template(t, slug, title, line, knobs, map_entities, comment):
    root_scope = f"template/site/{slug}"
    base = f"template/{slug}"
    m = doc(f"{base}/map.world.json", f"doc:{t}/map", f"{title}: map", "template", map_entities,
            primary_frame=mint(f"{t}/frame/site"), depends_on=[mint("doc:catalog/relay-bay"), mint("doc:catalog/open-patch"), mint("doc:catalog/da50")],
            comment=f"{title}'s map scope ({root_scope}/map): {comment}")
    f = doc(f"{base}/fleet.world.json", f"doc:{t}/fleet", f"{title}: fleet", "template", [
        ent(f"{t}/fleet/htms", "HTM fleet", {
            "cohort": {"robot_class": D30 + PIN, "count": 24, "assignment": {"kind": "free"}},
            "template.knob_binding": {"/cohort/count": "htms"}})],
            depends_on=[mint("doc:catalog/d30")],
            comment=f"{title}'s fleet scope ({root_scope}/fleet): the HTM cohort; each aisle's VTM cohort lands here when a site is made.")
    fl = doc(f"{base}/flow.world.json", f"doc:{t}/flow", f"{title}: flow", "template", flow_entities(t),
             comment=f"{title}'s flow scope ({root_scope}/flow): spec 06 section 6.1's relay flow, putaway and pick, schematic: one storage node over the aisles' sector, the station nodes over the pick and put sectors, the aisle VTMs named as the cohort assigned at each place.")
    r = doc(f"{base}/root.world.json", f"doc:{t}/root", title, "template", knobs + [
        ent(f"{t}/description", title, {"template.description": {"title": title, "text": line,
            "built_from": "GOFoundry-POC's shell library, warehouseLayouts.js:7-19 at 4104823"}})],
            primary_frame=mint(f"{t}/frame/site"), depends_on=CATALOG_DOCS,
            comment=f"{title}'s top scope ({root_scope}): its knobs and its card.")
    return [(f"{root_scope}/map", f"{base}/map.world.json"), (f"{root_scope}/fleet", f"{base}/fleet.world.json"),
            (f"{root_scope}/flow", f"{base}/flow.world.json"), (root_scope, f"{base}/root.world.json")]

# ---------------------------------------------------------------------------------------------------------------
# The five shells

AISLES_X = "The aisle row needs {value} mm more floor than the shell's width gives."
BAYS_Y = "The aisles need {value} mm more depth than the shell gives them."
WALL_WORDS = {"minus_y": "−y", "plus_y": "+y", "minus_x": "−x", "plus_x": "+x"}
# Where each wall of an unmirrored shell frame is drawn, north-up with +y up: a wall knob's hint says so (spec 05
# section 5.3), its description being the hint, since `template.knob` has no wall field (brief I1.07b).
WALL_DRAWN = {"minus_y": "along the bottom", "plus_y": "along the top", "minus_x": "up the left", "plus_x": "up the right"}

def strip_presence(wall):
    return f"1 - min(1, pick_{wall} + putaway_{wall})"

def boxed(t, slug, title, line, wkey, dkey, size, words, margin, comment):
    """The Square and the Rectangle: four walls, each with a pick and a putaway count, and a floor strip along each
    wall that gives way to its stations; `size` is the outline at the knobs' defaults, `words` each outline knob's
    label and description, and `margin` the floor the shell keeps beyond its rack rows along y."""
    W, D = wkey, dkey
    width_d, depth_d = size
    defaults = {("pick", "minus_y"): 4, ("putaway", "plus_y"): 2}
    station_knobs, order = [], 30
    need = {wall: f"pick_{wall} + putaway_{wall}" for wall in WALLS}
    fits = {wall: check(f"({need[wall]}) * station_pitch + 3400 - {W if wall in ('minus_y', 'plus_y') else D}",
                        f"The stations on the {WALL_WORDS[wall]} wall need {{value}} mm more wall than it has.") for wall in WALLS}
    # An outline knob follows what lies along it, its fit checks under it (D120): along x the aisle row and the
    # stations on the −y and +y walls, along y the rack rows, the margin and the stations on the −x and +x walls.
    along_x = (["aisles * 7650 + 10200"], ["minus_y", "plus_y"],
               [check(f"aisles * 7650 + 10200 - {W}", AISLES_X), fits["minus_y"], fits["plus_y"]])
    along_y = ([RACK_ROWS_Y + (f" + {margin}" if margin else "")], ["minus_x", "plus_x"],
               [check(f"{GAP_LENGTH} + 7650 - {D}", BAYS_Y), fits["minus_x"], fits["plus_x"]])
    spans = [tuple(x + y for x, y in zip(along_x, along_y))] if W == D else [along_x, along_y]
    outline = []
    for (key, label, description), order, (terms, walls, checks) in zip(words, [1, 2], spans):
        stations = f"max({', '.join(need[wall] for wall in walls)}) * station_pitch + 3400"
        outline.append(length(t, key, label, f"max({', '.join(terms + [stations])})", description, order, checks=checks))
    for kind in ["pick", "putaway"]:
        for wall in WALLS:
            key = f"{kind}_{wall}"
            checks = []
            if key == "pick_minus_y":
                checks.append(check("1 - min(1, pick_minus_y + pick_plus_y + pick_minus_x + pick_plus_x)",
                                    "Place at least one pick station.", unit="count"))
            if key == "putaway_plus_y":
                checks.append(check("1 - min(1, putaway_minus_y + putaway_plus_y + putaway_minus_x + putaway_plus_x)",
                                    "Place at least one putaway station.", unit="count"))
            label = f"{'Pick' if kind == 'pick' else 'Putaway'} stations, {WALL_WORDS[wall]} wall"
            station_knobs.append(knob(t, key, label, "count", defaults.get((kind, wall), 0),
                                      f"{'Pick' if kind == 'pick' else 'Putaway'} stations along the {WALL_WORDS[wall]} wall, drawn {WALL_DRAWN[wall]}.",
                                      "Stations", order, rng=(0, 12, 1), checks=checks))
            order += 1
    htm_sum = " + ".join(f"{k}_{w}" for k in ["pick", "putaway"] for w in WALLS)
    knobs = outline + common_knobs(t, htm_sum) + station_knobs
    ents = frame_entities(t)
    ents.append(floor_entity(t, "floor/interior", "floor", [1700, 1700], [width_d - 3400, depth_d - 3400], {
        "/region/size/0": f"max(850, {W} - 3400)", "/region/size/1": f"max(850, {D} - 3400)",
        "/tiling/exclusions/0/size/0": "aisles * 7650", "/tiling/exclusions/0/size/1": GAP_LENGTH},
        exclusions=[{"origin": [X0 - 1700, Y0 - 1700], "size": [4 * P, 12750]}]))
    strips = {
        "minus_y": ([0, 0], [width_d, 1700], {"/region/size/0": W}),
        "plus_y": ([0, depth_d - 1700], [width_d, 1700], {"/region/origin/1": f"max(0, {D} - 1700)", "/region/size/0": W}),
        "minus_x": ([0, 1700], [1700, depth_d - 3400], {"/region/size/1": f"max(850, {D} - 3400)"}),
        "plus_x": ([width_d - 1700, 1700], [1700, depth_d - 3400], {"/region/origin/0": f"max(0, {W} - 1700)", "/region/size/1": f"max(850, {D} - 3400)"}),
    }
    for wall, (origin, size, binding) in strips.items():
        ents.append(floor_entity(t, f"floor/strip/{wall}", f"floor strip along the {WALL_WORDS[wall]} wall", origin, size, binding,
                                 presence=strip_presence(wall)))
        ents.append(stitch(t, f"stitch/strip/{wall}", f"stitch: {WALL_WORDS[wall]} strip to the floor", f"floor/strip/{wall}", "floor/interior",
                           presence=strip_presence(wall)))
    for a, b in [("minus_y", "minus_x"), ("minus_y", "plus_x"), ("plus_y", "minus_x"), ("plus_y", "plus_x")]:
        ents.append(stitch(t, f"stitch/corner/{a}/{b}", f"stitch: {WALL_WORDS[a]} strip to the {WALL_WORDS[b]} strip",
                           f"floor/strip/{a}", f"floor/strip/{b}", presence=f"min({strip_presence(a)}, {strip_presence(b)})"))
    ents.append(aisle_piece(t))
    ents += [station_piece(t, kind, wall) for kind in ["pick", "put"] for wall in WALLS]
    ents.append(aisle_row(t))
    starts = {"minus_y": ([1700, 0], {}), "plus_y": ([4250, depth_d], {"/pose/position/1": D}),
              "minus_x": ([0, 4250], {}), "plus_x": ([width_d, 1700], {"/pose/position/0": W})}
    for wall, (start, binding) in starts.items():
        pick_default = defaults.get(("pick", wall), 0)
        ents.append(station_row(t, "pick", "pick stations", wall, start, f"pick_{wall}", binding, pick_default))
        axis = 0 if wall in ("minus_y", "plus_y") else 1
        put_start = list(start)
        put_start[axis] += pick_default * 4250
        put_binding = dict(binding)
        put_binding[f"/pose/position/{axis}"] = f"{start[axis]} + pick_{wall} * station_pitch"
        ents.append(station_row(t, "put", "putaway stations", wall, put_start, f"putaway_{wall}", put_binding,
                                defaults.get(("putaway", wall), 0)))
    ents += sectors(t, list(WALLS), list(WALLS))
    return write_template(t, slug, title, line, knobs, ents, comment)

def rectangle():
    t = "rectangle"
    words = [("width", "Width", "The shell's extent along x. Fits {aisles} aisles and the stations on the −y and +y walls."),
             ("depth", "Depth", "The shell's extent along y; above the width the layout is tall. Fits rack rows of {bays} bays and the stations on the −x and +x walls.")]
    return boxed(t, "rectangle", "Rectangle", "Wide layout, 1.5:1", "width", "depth", (40800, 27200), words, 6800,
                 "a width by depth outline, its aisles from 6,800 mm off the −x wall and 4,250 mm off the −y wall, pick and putaway rows on any wall.")

def square():
    t = "square"
    words = [("side", "Side", "The shell's side, along x and along y. Fits {aisles} aisles and the stations on its walls.")]
    return boxed(t, "square", "Square", "Equal sides, flexible flow", "side", "side", (40800, 40800), words, 0,
                 "a square outline, its aisles from 6,800 mm off the −x wall and 4,250 mm off the −y wall, pick and putaway rows on any wall.")

def mirror_binding(axes):
    b = {}
    if "x" in axes:
        odd = "(orientation - 2 * (orientation / 2))"
        b["/frame/transform_to_parent/scale/0"] = f"1 - 2 * {odd}"
        b["/frame/transform_to_parent/translation/0"] = f"width * {odd}"
    if "y" in axes:
        half = "(orientation / 2)" if "x" in axes else "orientation"
        b["/frame/transform_to_parent/scale/1"] = f"1 - 2 * {half}"
        b["/frame/transform_to_parent/translation/1"] = f"depth * {half}"
    return b

def station_count_knobs(t, put_checks, pick_wall, put_wall):
    """The pick and putaway rows' counts, each described by the wall its row stands on and where it is drawn."""
    return [
        knob(t, "pick_stations", "Pick stations", "count", 4, f"Pick stations {pick_wall}.", "Stations", 30, rng=(1, 12, 1)),
        knob(t, "putaway_stations", "Putaway stations", "count", 2, f"Putaway stations {put_wall}.", "Stations", 31, rng=(1, 12, 1), checks=put_checks),
    ]

def l_shape():
    t = "l-shape"
    # Its outline keeps the 1,700 mm its integer defaults kept past what its checks ask, beyond the last aisle and
    # beyond the rack rows; its putaway row stands on the cut corner, which no default follows.
    knobs = [
        length(t, "width", "Width", "max(aisles * 7650 + 8500 + 1700, pick_stations * station_pitch + 6800, cut_width + 6800)",
               "The L's extent along x, its arm along x included. Fits {aisles} aisles, the pick row and the arm along y.", 1,
               checks=[check("6800 - (width - cut_width)", "The arm along y needs {value} mm more width."),
                       check("aisles * 7650 + 8500 - width", AISLES_X),
                       check("pick_stations * station_pitch + 6800 - width", "The pick row needs {value} mm more of the −y wall.")]),
        length(t, "depth", "Depth", f"cut_depth + {RACK_ROWS_Y}",
               "The L's extent along y, its arm along y included. Fits rack rows of {bays} bays in the arm along x.", 2,
               checks=[check(f"{GAP_LENGTH} + 5950 - (depth - cut_depth)", "The aisles need {value} mm more depth than the arm along x gives them.")]),
        length(t, "cut_width", "Cut corner width", 20400, "How far the cut corner reaches along x.", 3),
        length(t, "cut_depth", "Cut corner depth", 13600, "How far the cut corner reaches along y.", 4,
               checks=[check("6800 - (depth - cut_depth)", "The arm along x needs {value} mm more depth.")]),
        knob(t, "orientation", "Cut corner", "count", 0, "Which corner of the outline the L leaves out, drawn north-up.", "Outline", 5,
             options=[(0, "+x +y"), (1, "−x +y"), (2, "+x −y"), (3, "−x −y")]),
    ] + common_knobs(t, "pick_stations + putaway_stations") + station_count_knobs(t,
        [check("putaway_stations * station_pitch - cut_depth", "The putaway row needs {value} mm more of the −x wall beyond the arm along x.")],
        "on the −y wall, in front of the aisles: drawn along the bottom with the cut corner at +x +y or −x +y, along the top at +x −y or −x −y",
        "on the −x wall, beyond the arm along x: drawn up the left with the cut corner at +x +y or +x −y, up the right at −x +y or −x −y")
    ents = frame_entities(t, mirror_binding("xy"))
    ents.append(floor_entity(t, "floor/interior", "floor", [1700, 1700], [40800 - 1700, 34000 - 1700], {
        "/region/size/0": "max(850, width - 1700)", "/region/size/1": "max(850, depth - 1700)",
        "/tiling/exclusions/0/size/0": "aisles * 7650", "/tiling/exclusions/0/size/1": GAP_LENGTH,
        "/tiling/exclusions/1/origin/0": "max(0, width - cut_width - 1700)", "/tiling/exclusions/1/origin/1": "max(0, depth - cut_depth - 1700)",
        "/tiling/exclusions/1/size/0": "cut_width", "/tiling/exclusions/1/size/1": "cut_depth"},
        exclusions=[{"origin": [X0 - 1700, Y0 - 1700], "size": [4 * P, 12750]},
                    {"origin": [40800 - 20400 - 1700, 34000 - 13600 - 1700], "size": [20400, 13600]}]))
    ents.append(aisle_piece(t))
    ents += [station_piece(t, "pick", "minus_y"), station_piece(t, "put", "minus_x")]
    ents.append(aisle_row(t))
    ents.append(station_row(t, "pick", "pick stations", "minus_y", [X0, 0], "pick_stations", None, 4))
    ents.append(station_row(t, "put", "putaway stations", "minus_x", [0, 34000 - 13600 + 2550], "putaway_stations",
                            {"/pose/position/1": "max(2550, depth - cut_depth + 2550)"}, 2))
    ents += sectors(t, ["minus_y"], ["minus_x"])
    return write_template(t, "l-shape", "L-Shape", "Receiving + shipping on adjacent walls", knobs, ents,
                          "the letter L, its arm along x on the −y side; pick in front of the aisles on the −y wall, putaway on the −x wall beyond the arm along x; the orientation knob mirrors the shell frame to put the cut corner at any corner.")

def u_shape():
    t = "u-shape"
    # Its depth keeps the 1,700 mm its integer default kept beyond the rack rows; its width, the station rows at the
    # arms' open ends at its defaults, kept nothing more.
    knobs = [
        length(t, "width", "Width",
               "max(aisles * 7650 + 8500, court_width + max(13600, 2 * pick_stations * station_pitch, 2 * putaway_stations * station_pitch))",
               "The U's extent along x. Fits {aisles} aisles and the station rows at the arms' open ends.", 1,
               checks=[check("aisles * 7650 + 8500 - width", AISLES_X),
                       check("pick_stations * station_pitch - (width - court_width) / 2", "The pick row needs {value} mm more of the +x arm's open end."),
                       check("putaway_stations * station_pitch - (width - court_width) / 2", "The putaway row needs {value} mm more of the −x arm's open end.")]),
        length(t, "depth", "Depth", f"court_depth + {RACK_ROWS_Y}",
               "The U's extent along y, its arms included. Fits rack rows of {bays} bays in the base.", 2,
               checks=[check(f"{GAP_LENGTH} + 5950 - (depth - court_depth)", "The aisles need {value} mm more depth than the base gives them.")]),
        length(t, "court_width", "Courtyard width", 13600, "The open courtyard's width, centred on the open side.", 3,
               checks=[check("6800 - (width - court_width) / 2", "Each arm needs {value} mm more width.")]),
        length(t, "court_depth", "Courtyard depth", 13600, "How far the courtyard reaches in from the open side.", 4),
        knob(t, "orientation", "Open side", "count", 0, "The side the U opens on, drawn north-up.", "Outline", 5,
             options=[(0, "+y"), (1, "−y")]),
    ] + common_knobs(t, "pick_stations + putaway_stations") + station_count_knobs(t, [],
        "on the +y wall at the +x arm's open end: drawn at the top right with the open side +y, at the bottom right with it −y",
        "on the +y wall at the −x arm's open end: drawn at the top left with the open side +y, at the bottom left with it −y")
    ents = frame_entities(t, mirror_binding("y"))
    ents.append(floor_entity(t, "floor/interior", "floor", [0, 0], [47600, 34000 - 1700], {
        "/region/size/0": "width", "/region/size/1": "max(850, depth - 1700)",
        "/tiling/exclusions/0/size/0": "aisles * 7650", "/tiling/exclusions/0/size/1": GAP_LENGTH,
        "/tiling/exclusions/1/origin/0": "max(0, (width - court_width) / 2)", "/tiling/exclusions/1/origin/1": "max(0, depth - court_depth)",
        "/tiling/exclusions/1/size/0": "court_width", "/tiling/exclusions/1/size/1": "max(850, court_depth - 1700)"},
        exclusions=[{"origin": [X0, Y0], "size": [4 * P, 12750]},
                    {"origin": [(47600 - 13600) // 2, 34000 - 13600], "size": [13600, 13600 - 1700]}]))
    ents.append(aisle_piece(t))
    ents += [station_piece(t, "pick", "plus_y"), station_piece(t, "put", "plus_y")]
    ents.append(aisle_row(t))
    ents.append(station_row(t, "put", "putaway stations", "plus_y", [2550, 34000], "putaway_stations",
                            {"/pose/position/1": "depth"}, 2))
    ents.append(station_row(t, "pick", "pick stations", "plus_y", [(47600 + 13600) // 2 + 2550, 34000], "pick_stations",
                            {"/pose/position/0": "(width + court_width) / 2 + 2550", "/pose/position/1": "depth"}, 4))
    ents += sectors(t, ["plus_y"], ["plus_y"])
    return write_template(t, "u-shape", "U-Shape", "Docks on same side", knobs, ents,
                          "a base along −y and two arms along y around a courtyard centred on the +y wall; the aisles in the base, putaway at the −x arm's open end and pick at the +x arm's; the orientation knob mirrors the shell frame to open it on −y.")

def i_shape():
    t = "i-shape"
    flow_x, flow_y = "(1 - orientation)", "orientation"
    # Each row needs its count of pitches and its ends along the walls it stands on at the orientation: along y with
    # the flow along x, along x with it along y; depth keeps 6,800 mm of floor beyond the rack rows.
    row = {kind: f"{kind}_stations * station_pitch + 3400" for kind in ["pick", "putaway"]}
    row_fits = {(kind, outline): check(f"({row[kind]} - {outline}) * {flow}", f"The {kind} row needs {{value}} mm more wall than it has.")
                for kind in row for outline, flow in [("width", flow_y), ("depth", flow_x)]}
    knobs = [
        length(t, "width", "Width", f"max(aisles * 7650 + 10200, ({row['pick']}) * {flow_y}, ({row['putaway']}) * {flow_y})",
               "The shell's extent along x. Fits {aisles} aisles, and the station rows with the flow along y.", 1,
               checks=[check("aisles * 7650 + 10200 - width", AISLES_X), row_fits[("pick", "width")], row_fits[("putaway", "width")]]),
        length(t, "depth", "Depth", f"max({RACK_ROWS_Y} + 6800, ({row['pick']}) * {flow_x}, ({row['putaway']}) * {flow_x})",
               "The shell's extent along y. Fits rack rows of {bays} bays, and the station rows with the flow along x.", 2,
               checks=[check(f"{GAP_LENGTH} + 7650 - depth", BAYS_Y), row_fits[("pick", "depth")], row_fits[("putaway", "depth")]]),
        knob(t, "orientation", "Flow along", "count", 0, "The axis the flow crosses the shell along: putaway on the wall it enters by, pick on the wall opposite.", "Outline", 3,
             options=[(0, "+x: putaway on −x, pick on +x"), (1, "+y: putaway on −y, pick on +y")]),
    ] + common_knobs(t, "pick_stations + putaway_stations") + station_count_knobs(t, [],
        "on the wall the flow leaves by: +x, drawn up the right, with the flow along x; +y, drawn along the top, with it along y",
        "on the wall the flow enters by: −x, drawn up the left, with the flow along x; −y, drawn along the bottom, with it along y")
    width_d, depth_d = 40800, 27200
    ents = frame_entities(t)
    ents.append(floor_entity(t, "floor/interior", "floor", [1700, 1700], [width_d - 3400, depth_d - 3400], {
        "/region/size/0": "max(850, width - 3400)", "/region/size/1": "max(850, depth - 3400)",
        "/tiling/exclusions/0/size/0": "aisles * 7650", "/tiling/exclusions/0/size/1": GAP_LENGTH},
        exclusions=[{"origin": [X0 - 1700, Y0 - 1700], "size": [4 * P, 12750]}]))
    strips = {
        "minus_y": ([0, 0], [width_d, 1700], {"/region/size/0": "width"}, flow_x),
        "plus_y": ([0, depth_d - 1700], [width_d, 1700], {"/region/origin/1": "max(0, depth - 1700)", "/region/size/0": "width"}, flow_x),
        "minus_x": ([0, 1700], [1700, depth_d - 3400], {"/region/size/1": "max(850, depth - 3400)"}, flow_y),
        "plus_x": ([width_d - 1700, 1700], [1700, depth_d - 3400], {"/region/origin/0": "max(0, width - 1700)", "/region/size/1": "max(850, depth - 3400)"}, flow_y),
    }
    for wall, (origin, size, binding, when) in strips.items():
        ents.append(floor_entity(t, f"floor/strip/{wall}", f"floor strip along the {WALL_WORDS[wall]} wall", origin, size, binding, presence=when))
        ents.append(stitch(t, f"stitch/strip/{wall}", f"stitch: {WALL_WORDS[wall]} strip to the floor", f"floor/strip/{wall}", "floor/interior", presence=when))
    ents.append(aisle_piece(t))
    ents += [station_piece(t, "pick", wall) for wall in ["plus_x", "plus_y"]]
    ents += [station_piece(t, "put", wall) for wall in ["minus_x", "minus_y"]]
    ents.append(aisle_row(t))
    ents.append(station_row(t, "put", "putaway stations", "minus_x", [0, 4250], f"putaway_stations * {flow_x}", None, 2))
    ents.append(station_row(t, "pick", "pick stations", "plus_x", [width_d, 1700], f"pick_stations * {flow_x}", {"/pose/position/0": "width"}, 4))
    ents.append(station_row(t, "put", "putaway stations", "minus_y", [1700, 0], f"putaway_stations * {flow_y}", None, 0))
    ents.append(station_row(t, "pick", "pick stations", "plus_y", [4250, depth_d], f"pick_stations * {flow_y}", {"/pose/position/1": "depth"}, 0))
    ents += sectors(t, ["plus_x", "plus_y"], ["minus_x", "minus_y"])
    return write_template(t, "i-shape", "I-Shape", "Flow-through, inbound → outbound", knobs, ents,
                          "a width by depth outline the flow crosses: putaway on the wall it enters by, pick on the wall opposite, the aisles between; the orientation knob moves both station rows between the x walls and the y walls.")

entries = [{"scope": s, "document": d} for s, d in CATALOG_ENTRIES]
for make in [rectangle, square, l_shape, u_shape, i_shape]:
    entries += [{"scope": s, "document": d} for s, d in make()]
VOCABULARY = ["template.knob", "template.description", "template.placement", "template.knob_binding", "template.presence",
              "template.target_scope", "flow.node", "flow.material_edge", "flow.agent_segment", "flow.return_rule",
              "floor_drive", "rack_ladder", "lift", "end_effector"]
WRITES["manifest.json"] = json.dumps({"v": 1, "vocabulary": VOCABULARY, "entries": entries}, indent=2) + "\n"
WRITES["ids.json"] = json.dumps({name: IDS[name] for name in USED}, indent=1, sort_keys=True) + "\n"

if CHECK:
    stale = [str(p.relative_to(HERE)) for p in HERE.rglob("*.world.json") if str(p.relative_to(HERE)) not in WRITES]
    differs = [path for path, text in WRITES.items() if not (HERE / path).exists() or (HERE / path).read_text() != text]
    for path in differs + stale:
        print(f"{path}: {'not written by this script' if path in stale else 'differs from what this script writes'}")
    sys.exit(1 if differs or stale else 0)
for path, text in WRITES.items():
    (HERE / path).parent.mkdir(parents=True, exist_ok=True)
    (HERE / path).write_text(text)
print(f"wrote {len(entries)} entries")
