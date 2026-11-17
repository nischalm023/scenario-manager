"""September 8 floor alignment plan, invoked by reauthor_dfw6_easyloops.py.

This is a bounded patch of the reviewed 144-HTM document, not a site generator.
Racks, relays, access identities, equipment, fleets and tasks remain byte-identical.
Parent corridor identities survive subdivision so access.along still names one
aisle. All distances are the document's 0.1 mm steps.
"""

import copy
import hashlib
import json
import subprocess
import uuid

import reauthor_dfw6_easyloops as author

PRE_IMAGE_BLOB = "6da68764ff0474ab54e0c5f236fef0755f2a09bf"
PRE_IMAGE_SHA256 = "344b4f14f3a04685593aa476d7d873626511eacbd91715d76da8f9e07e426092"
FLOOR = author.FLOOR_FRAME
APRON = author.STATION_APRON
SOUTH = "007c700a-ab07-5e4b-9d10-72b625e15145"
NORTH = "d41c643c-272c-5e3c-8e9c-81409549e38f"


def identity(name):
    return str(uuid.uuid5(uuid.UUID(APRON), "floor-alignment/" + name))


def local(handle):
    return {"local_ref": handle}


def address(parent, handle, col, row):
    return {"generator": parent, "key": f"instance:0/local:{handle}/cell:{col},{row}"}


def template(name, entities, params=None):
    return {"id": identity(name), "name": name,
            "components": {"template": {"params": params or {}, "entities": entities}}}


def stamp_region(entity, tpl, params):
    components = entity["components"]
    components.pop("tiling")
    components["pose"] = {"frame": FLOOR, "position": components["region"]["origin"][:]}
    components["stamp"] = {"template": tpl["id"], "params": params}


def bay_schedule(entities):
    """Read existing steel geometry; do not reconstruct it from an average pitch."""
    bays = []
    for e in entities.values():
        if not e.get("name", "").startswith("rack 0 group "):
            continue
        c = e["components"]
        rack = entities[c["stamp"]["template"]]["components"]["template"]["entities"][0]["components"]["rack"]
        length = rack["footprint"][0]  # Rack stamp is rotated 90 degrees.
        repeat = c["stamp"]["repeat"]
        for i in range(repeat["count"]):
            bays.append((c["pose"]["position"][1] + i * repeat["step"][1], length))
    bays.sort()
    assert len(bays) == 21 and sum(length == 20121 for _, length in bays) == 2
    assert bays[0][0] == 178441 and sum(bays[-1]) == 743660
    return bays


def bay_template(bays, direction):
    pieces = []
    for i, (y, length) in enumerate(bays):
        rows = 3 if length == 20121 else 4
        c = {"region": {"origin": [0, y - bays[0][0]], "size": [{"$param": "width"}, length]},
             "tiling": {"pitch": [{"$param": "pitch_x"}, length // rows],
                        "derives": {"$param": "derives"}}}
        if direction:
            c["flow"] = {"rule": "one_way", "direction": direction}
        pieces.append({"local": f"bay-{i}", "components": c})
        if i:
            seam = {"a": local(f"bay-{i-1}"), "b": local(f"bay-{i}")}
            if direction:
                prev_rows = 3 if bays[i-1][1] == 20121 else 4
                pair = [{"local_ref": f"bay-{i-1}", "key": f"cell:0,{prev_rows-1}"},
                        {"local_ref": f"bay-{i}", "key": "cell:0,0"}]
                seam.update(rule="explicit", pairs=[pair if direction == "+y" else pair[::-1]], bidirectional=False)
            else:
                # Odd pitches round centres to primary steps. One step covers
                # the resulting half-step shift of a reconstructed tile edge.
                seam.update(rule="contact", tolerance=y - sum(bays[i-1]) + 1)
            pieces.append({"local": f"join-{i-1}-{i}", "components": {"stitch": seam}})
    return template("floor bays " + (direction or "two way"), pieces,
                    {key: {} for key in ["width", "pitch_x", "derives"]})


def columns_template(name, columns, derives):
    """One vertical strip per X interval; every horizontal seam is explicit floor."""
    pieces = []
    for i, (x, width) in enumerate(columns):
        pieces.append({"local": f"column-{i}", "components": {
            "region": {"origin": [x, 0], "size": [width, {"$param": "height"}]},
            "tiling": {"pitch": [width, 8500], "derives": derives}}})
        if i:
            pieces.append({"local": f"join-{i-1}-{i}", "components": {"stitch": {
                "a": local(f"column-{i-1}"), "b": local(f"column-{i}"),
                "rule": "contact", "tolerance": x - sum(columns[i-1]) + 1}}})
    return template(name, pieces, {"height": {}})


def fill_interval(start, end, target=8500):
    """Subdivide an unconstrained span without remainder strips or tiny cells."""
    length = end - start
    if length == 0:
        return []
    count = max(1, round(length / target))
    width, extra = divmod(length, count)
    assert width >= 6950, (start, end, count, width)
    pieces = []
    for i in range(count):
        size = width + (i < extra)
        pieces.append((start, size))
        start += size
    return pieces


def apron_columns(anchors, end):
    """Fit adjoining rectangles with exact mandatory door centre lines.

    Between two door cells, n ordinary cells consume the remaining width.
    Propagate feasible door-width intervals, then choose widths backwards.
    This avoids both nearest-column rounding and untraversably thin fillers.
    """
    low, high, target = 6950, 14000, 8400

    def fit(i, bounds, counts):
        if i == len(anchors) - 1:
            return bounds, counts
        distance = anchors[i+1] - anchors[i]
        candidates = sorted(range(int(distance / low) + 1),
                            key=lambda n: abs(n + 1 - distance / target))
        lo, hi = bounds[-1]
        for n in candidates:
            new_lo = max(low, 2 * (distance - n * high) - hi)
            new_hi = min(high, 2 * (distance - n * low) - lo)
            if new_lo <= new_hi:
                result = fit(i+1, bounds + [(new_lo, new_hi)], counts + [n])
                if result:
                    return result
        return None

    result = fit(0, [(low, high)], [])
    assert result, "door centres cannot be tiled at viable robot-cell widths"
    bounds, counts = result
    widths = [min(max(target, bounds[-1][0]), bounds[-1][1])]
    for i in reversed(range(len(counts))):
        d, n = anchors[i+1] - anchors[i], counts[i]
        lo = max(bounds[i][0], 2 * (d - n * high) - widths[0])
        hi = min(bounds[i][1], 2 * (d - n * low) - widths[0])
        widths.insert(0, min(max(target, lo), hi))
    assert all(w % 2 == 0 for w in widths)
    columns = fill_interval(0, anchors[0] - widths[0] // 2)
    for i, (x, width) in enumerate(zip(anchors, widths)):
        start = x - width // 2
        columns.append((start, width))
        end_of_cell = start + width
        next_start = anchors[i+1] - widths[i+1] // 2 if i+1 < len(anchors) else end
        if i+1 < len(anchors):
            n = counts[i]
            if n:
                step, extra = divmod(next_start - end_of_cell, n)
                assert low <= step <= high
                for j in range(n):
                    size = step + (j < extra)
                    columns.append((end_of_cell, size))
                    end_of_cell += size
            else:
                assert end_of_cell == next_start
        else:
            columns.extend(fill_interval(end_of_cell, next_start))
    assert all(sum(a) == b[0] for a, b in zip(columns, columns[1:]))
    assert all(any(2*x + w == 2*a for x, w in columns) for a in anchors)
    return columns


def plan(pre):
    before = json.loads(pre)
    source = {e["id"]: e for e in before["entities"]}
    entities = copy.deepcopy(source)
    appended = []
    # Shift only the internal boundary of each rack's two floor strips.
    for service_prefix, travel_prefix in [("under-rack service strip ", "under-rack travel lane "),
                                           ("buffer service strip ", "buffer travel lane ")]:
        services = [e for e in entities.values() if e.get("name", "").startswith(service_prefix)]
        for service in services:
            index = service["name"][len(service_prefix):]
            travel = next(e for e in entities.values() if e.get("name") == travel_prefix + index)
            sr, tr = (e["components"]["region"] for e in (service, travel))
            west, east = min(sr["origin"][0], tr["origin"][0]), max(sum((r["origin"][0], r["size"][0])) for r in (sr, tr))
            service_west = sr["origin"][0] < tr["origin"][0]
            sr["origin"][0] = west if service_west else east - 8000
            sr["size"][0] = 8000
            tr["origin"][0] = west + 8000 if service_west else west
            tr["size"][0] = east - west - 8000
            for e in (service, travel):
                e["components"]["tiling"]["pitch"][0] = e["components"]["region"]["size"][0]

    corridors = sorted((e for e in entities.values() if "tiling" in e["components"]
                        and e.get("name", "").startswith(("aisle ", "under-rack ", "open floor "))),
                       key=lambda e: e["components"]["region"]["origin"][0])
    assert len(corridors) == 97
    bays = bay_schedule(source)
    bay_templates = {direction: bay_template(bays, direction) for direction in (None, "+y", "-y")}
    appended.extend(bay_templates.values())
    rack_columns = []
    for e in corridors:
        c = e["components"]
        r, t = c["region"], c["tiling"]
        x, width, pitch = r["origin"][0], r["size"][0], t["pitch"][0]
        rack_columns.extend((x + j*pitch, pitch) for j in range(width // pitch))
        r["size"][1] = sum(bays[-1]) - bays[0][0]
        direction = c.get("flow", {}).get("direction")
        stamp_region(e, bay_templates[direction], {"width": width, "pitch_x": pitch, "derives": t["derives"]})

    highway_template = columns_template("cross-floor rack columns", rack_columns,
                                       entities[SOUTH]["components"]["tiling"]["derives"])
    appended.append(highway_template)
    for parent in (SOUTH, NORTH):
        e = entities[parent]
        r, t = e["components"]["region"], e["components"]["tiling"]
        r["size"][0] = sum(rack_columns[-1])
        assert t["derives"] == source[SOUTH]["components"]["tiling"]["derives"]
        stamp_region(e, highway_template, {"height": r["size"][1]})

    doors = []
    for e in entities.values():
        s = e["components"].get("stitch", {})
        if s.get("rule") != "explicit" or s.get("b") != APRON:
            continue
        p = entities[s["a"]]["components"]["pose"]
        x = p["position"][0]
        if p["frame"] != FLOOR:
            f = entities[p["frame"]]["components"]["frame"]["transform_to_parent"]
            x = f["translation"][0] + f["scale"][0] * x
        doors.append((s, x))
    assert len(doors) == 26
    e = entities[APRON]
    r, t = e["components"]["region"], e["components"]["tiling"]
    columns = apron_columns(sorted(x for _, x in doors), r["size"][0])
    tpl = columns_template("station apron door columns", columns, t["derives"])
    appended.append(tpl)
    stamp_region(e, tpl, {"height": r["size"][1]})
    for stitch, x in doors:
        col = next(i for i, (start, width) in enumerate(columns) if 2*start + width == 2*x)
        for pair in stitch["pairs"]:
            for i, endpoint in enumerate(pair):
                if isinstance(endpoint, dict) and endpoint.get("generator") == APRON:
                    pair[i] = address(APRON, f"column-{col}", 0, 0)

    # Preserve the existing two E/W crossing rows in each named crossing bay.
    # Their old flat-grid row numbers are obsolete after bay subdivision.
    crossings = 0
    for e in entities.values():
        if not e.get("name", "").startswith("E/W crossing, aisle "):
            continue
        bay = int(e["name"].split("bay row ")[1])
        assert bay in (0, 7, 10, 17, 20)
        stitch = e["components"]["stitch"]
        assert len(stitch["pairs"]) == 2
        for row, pair in enumerate(stitch["pairs"]):
            for i, endpoint in enumerate(pair):
                assert endpoint["generator"] in (stitch["a"], stitch["b"])
                pair[i] = address(endpoint["generator"], f"bay-{bay}", 0, row)
        crossings += 1
    assert crossings == 85

    # A broad contact stitch chooses the smaller expansion, which for a wide
    # open floor can be the highway. It then links highway cells outside the
    # floor's width back to its edge column. State the aligned boundary pairs.
    corridor_ids = {e["id"] for e in corridors}
    for e in entities.values():
        s = e["components"].get("stitch", {})
        if s.get("a") not in corridor_ids or s.get("b") not in (SOUTH, NORTH):
            continue
        corridor, highway = entities[s["a"]], entities[s["b"]]
        params = corridor["components"]["stamp"]["params"]
        origin = corridor["components"]["region"]["origin"][0]
        pitch = params["pitch_x"]
        bay, row = (0, 0) if s["b"] == SOUTH else (20, 3)
        highway_row = 3 if s["b"] == SOUTH else 0
        pairs = []
        for col in range(params["width"] // pitch):
            x = origin + col * pitch
            cross_col = rack_columns.index((x, pitch))
            pairs.append([address(corridor["id"], f"bay-{bay}", col, row),
                          address(highway["id"], f"column-{cross_col}", 0, highway_row)])
        s.pop("tolerance", None)
        s.update(rule="explicit", pairs=pairs)

    # Existing open-floor seams skipped a service strip to reach a travel lane.
    # Meet the immediately adjacent HTM floor instead of drawing an arc across
    # a different cell that could be occupied by another robot.
    for e in entities.values():
        if not e.get("name", "").startswith("stitch:floor-lane:"):
            continue
        s = e["components"]["stitch"]
        floor = entities[s["a"]]
        i = corridors.index(floor)
        old_x = entities[s["b"]]["components"]["region"]["origin"][0]
        side = -1 if old_x < floor["components"]["region"]["origin"][0] else 1
        adjacent = corridors[i + side]
        s["b"] = adjacent["id"]
        ar, br = (q["components"]["region"] for q in (floor, adjacent))
        s["tolerance"] = max(ar["origin"][0] - br["origin"][0] - br["size"][0],
                             br["origin"][0] - ar["origin"][0] - ar["size"][0], 0) + 1

    spans, _ = author.entity_spans(pre)
    changed = {k for k in source if source[k] != entities[k]}
    edits = [(*spans[k], author.render(entities[k]).lstrip(), entities[k].get("name", k)) for k in changed]
    out = author.splice(pre, edits, appended, False)
    author.prove(before, out, [])
    after_spans, _ = author.entity_spans(out)
    for k in source.keys() - changed:
        a, b = spans[k]
        c, d = after_spans[k]
        assert pre[a:b] == out[c:d], f"unrelated entity bytes changed: {k}"
    report = {"changed_entities": len(changed), "byte_unchanged_entities": len(source) - len(changed),
              "added_templates": len(appended), "rack_columns": len(rack_columns), "apron_columns": len(columns),
              "bay_rows": 82, "aligned_doors": len(doors), "retained_crossings": crossings,
              "pre_image": PRE_IMAGE_SHA256, "post_image": hashlib.sha256(out.encode()).hexdigest()}
    return out, report


def run(args):
    pre = subprocess.check_output(["git", "-C", str(author.REPO), "cat-file", "blob", PRE_IMAGE_BLOB]).decode()
    assert hashlib.sha256(pre.encode()).hexdigest() == PRE_IMAGE_SHA256
    out, report = plan(pre)
    current = author.DOC.read_text()
    if args.audit:
        assert current == out, "fixture is not the audited alignment plan applied to its pinned pre-image"
    else:
        assert current in (pre, out), "refusing to overwrite unreviewed fixture edits"
        if args.emit:
            args.emit.write_text(out)
        if args.apply:
            author.DOC.write_text(out)
    print(json.dumps(report, indent=2))
    return 0
