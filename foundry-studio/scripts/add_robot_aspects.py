#!/usr/bin/env python3
"""Give every fixture world's robot entities their aspects, and take the robot
figures out of the specs that stated them.

Why this exists: on 2026-09-22 robot figures left compiled Rust for the world
graph. A robot entity now describes its machine by one component per aspect
(`sim/dessim/tests/fixtures/robots/README.md`), and the simulator reads each
fleet's figures from the entities that feed it (`sim/dessim/src/robots/`). The
fixture worlds are hand-managed documents whose every other byte has to
survive, so the edit is made by this script, which writes nothing it cannot
prove.

What it changes, and nothing else:

  * `walmart_dfw6.json` and its blade-uprights canary. Each robot entity gets
    the components of its robot model file, `lineage`, `body` and every
    `$comment` included: `HTM (RB-K50H)` from `hai_k50`, `VTM (vacuum)` from
    `quicktron_c56_suction`, `VTM (gripper)` from `vtm_g_gripper`.
  * `cencora_v3.json`. `VTM A5` gets `quicktron_a5`'s components but the
    `lineage` it already carries, which must match the model file's.
    `HTM (generic)` gets the M5F's `floor_drive` and `end_effector` only, their
    `$comment`s rewritten to cite the model file, because this world names no
    maker.
  * The eight other test worlds. Each robot entity gets the figures its runs
    have always used, the simulator's former compiled defaults, and an entity
    `$comment` saying so. `scripts/gen_minimal_easyloop.py` writes the same
    for `minimal_easyloop.json`.
  * `cencora_v3_spec.json` and `walmart_dfw6_throughput_spec.json`. Their
    robot-figure settings go, with the `$comment`s that explained them.

The proofs, run before a byte is written:

  (a) Structural. The parsed new world is the parsed old one plus exactly the
      planned components and comments; the parsed new spec is the parsed old
      one less exactly the robot-figure keys and the prose about them.
  (b) Line level. Every line of an old world survives, unchanged and in order:
      the edit only inserts lines.
  (c) Bytes. A document edited as data first dumps back to its own bytes, so
      the dump rewrites no byte the edit does not change; every other world is
      edited by inserting text lines. Files are read and written with
      `newline=""`, so no line ending is translated.

Usage:
  python3 scripts/add_robot_aspects.py --dry-run    # report, write nothing
  python3 scripts/add_robot_aspects.py --apply      # edit the documents
  python3 scripts/add_robot_aspects.py --audit --base 7f46f5631
      # prove every document is `--base` with exactly these edits
"""

from __future__ import annotations

import argparse
import copy
import difflib
import json
import os
import re
import subprocess
import sys

FIXTURES = "sim/dessim/tests/fixtures"
MODELS = f"{FIXTURES}/robots"

PLACEHOLDER = (
    "The simulator's former compiled defaults, which this test world has always run; "
    "not a real robot's figures."
)

GENERIC_HTM_COMMENTS = {
    "floor_drive": (
        "The Quicktron M5F's floor drive, from sim/dessim/tests/fixtures/robots/quicktron_m5f.world.json: "
        "the figures this world's HTM has always run, though the world names no maker. No 180° turn is "
        "stated, so a reversal is two quarter turns."
    ),
    "end_effector": (
        "The Quicktron M5F's end effector, from sim/dessim/tests/fixtures/robots/quicktron_m5f.world.json: "
        "a pick or a drop once the forks are raised."
    ),
}


class Refused(Exception):
    """A document this script cannot prove an edit of."""


def read(path: str) -> str:
    with open(path, encoding="utf-8", newline="") as handle:
        return handle.read()


def model(root: str, name: str) -> dict:
    """The components of the one robot entity in a robot model file, in file order."""
    (entity,) = json.loads(read(os.path.join(root, MODELS, f"{name}.world.json")))["entities"]
    return entity["components"]


def placeholder_htm() -> dict:
    return {
        "floor_drive": {
            "loaded": {"max_speed_mm_s": 4200, "accel_mm_s2": 2000, "quarter_turn_secs": 1.0},
            "empty": {"max_speed_mm_s": 4500, "accel_mm_s2": 2000, "quarter_turn_secs": 1.0},
        },
        "end_effector": {"pick_secs": 2.0, "drop_secs": 2.0},
    }


def placeholder_vtm(locomotion: str) -> dict:
    chassis = {"max_speed_mm_s": 3000, "accel_mm_s2": 2000}
    moves = (
        {"rack_ladder": chassis}
        if locomotion == "ladder"
        else {"floor_drive": {"loaded": dict(chassis), "empty": dict(chassis)}}
    )
    return {
        **moves,
        "lift": {"profile": {"max_speed_mm_s": 3000, "accel_mm_s2": 2000}},
        "end_effector": {
            "pick_secs": 4.0,
            "drop_secs": 2.0,
            "align_before_pick_secs": 1.933,
            "align_before_drop_secs": 1.933,
        },
    }


def robot_entities(document: dict) -> dict:
    """The robot entities the document's cohorts name, by id."""
    by_id = {entity.get("id"): entity for entity in document["entities"]}
    named = {
        entity["components"]["cohort"]["robot_class"]
        for entity in document["entities"]
        if "cohort" in entity.get("components", {})
    }
    if not named <= set(by_id):
        raise Refused("a cohort names a robot entity the document does not declare")
    return {id: by_id[id] for id in sorted(named)}


# --------------------------------------------------------------------------
# What each world gets: robot entity id -> (components, entity `$comment`).
# --------------------------------------------------------------------------

DFW6 = {
    "2eb25be4-a59c-5fad-87cb-59c33527f866": ("HTM (RB-K50H)", "hai_k50"),
    "0b61915b-4ecc-553e-8ec7-785c7b55db3b": ("VTM (vacuum)", "quicktron_c56_suction"),
    "79cd27ad-8387-5120-963c-c1058a071cc2": ("VTM (gripper)", "vtm_g_gripper"),
}


def dfw6_plan(root: str, document: dict) -> dict:
    entities = robot_entities(document)
    if set(entities) != set(DFW6):
        raise Refused(f"the DFW6 robot entities are {sorted(entities)}")
    plan = {}
    for id, (name, file) in DFW6.items():
        if entities[id]["name"] != name:
            raise Refused(f"{id} is named {entities[id]['name']!r}, not {name!r}")
        plan[id] = (dict(model(root, file)), None)
    return plan


def cencora_v3_plan(root: str, document: dict) -> dict:
    by_name = {entity["name"]: id for id, entity in robot_entities(document).items()}
    if set(by_name) != {"VTM A5", "HTM (generic)"}:
        raise Refused(f"the Cencora v3 robot entities are {sorted(by_name)}")
    a5 = dict(model(root, "quicktron_a5"))
    carried = robot_entities(document)[by_name["VTM A5"]]["components"].get("lineage")
    if carried != a5.pop("lineage"):
        raise Refused("the A5 entity's lineage differs from quicktron_a5.world.json's")
    m5f = model(root, "quicktron_m5f")
    htm = {name: {**m5f[name], "$comment": GENERIC_HTM_COMMENTS[name]} for name in ("floor_drive", "end_effector")}
    return {by_name["VTM A5"]: (a5, None), by_name["HTM (generic)"]: (htm, None)}


def placeholder_plan(root: str, document: dict) -> dict:
    plan = {}
    for id, entity in robot_entities(document).items():
        robot_class = entity["components"]["robot_class"]
        role, locomotion = robot_class["role"], robot_class.get("locomotion", "floor")
        if role not in ("htm", "vtm"):
            raise Refused(f"{id} works as {role!r}")
        plan[id] = (placeholder_htm() if role == "htm" else placeholder_vtm(locomotion), PLACEHOLDER)
    return plan


# name -> (plan, the dump that reproduces the document, its closing newline);
# no dump means the document is key-sorted by hand and is edited line by line.
WORLDS = {
    "walmart_dfw6.json": (dfw6_plan, dict(indent=1, ensure_ascii=True), "\n"),
    "walmart_dfw6_blade_uprights_canary.json": (dfw6_plan, dict(indent=1, ensure_ascii=True), ""),
    "cencora_v3.json": (cencora_v3_plan, None, None),
    "minimal_easyloop.json": (placeholder_plan, dict(indent=1, ensure_ascii=True), "\n"),
    **{
        name: (placeholder_plan, None, None)
        for name in [
            "cencora_phase1.json",
            "cencora_phase2.json",
            "minimal_pick_only.json",
            "minimal_putaway.json",
            "minimal_putaway1.json",
            "minimal_putaway1-1k.json",
            "minimal_putaway2.json",
        ]
    },
}


def state(document: dict, plan: dict) -> str:
    """`bare` when no planned component or comment is there yet, `done` when all
    of them are, exactly as planned, and `partial` otherwise."""
    present, exact = 0, 0
    wanted = 0
    for id, (components, comment) in plan.items():
        entity = next(entity for entity in document["entities"] if entity.get("id") == id)
        for name, value in components.items():
            wanted += 1
            present += name in entity["components"]
            exact += entity["components"].get(name) == value
        if comment is not None:
            wanted += 1
            present += "$comment" in entity
            exact += entity.get("$comment") == comment
    if present == 0:
        return "bare"
    return "done" if exact == wanted else "partial"


def as_data(text: str, plan: dict, dump: dict, newline: str) -> str:
    """Edit a document whose parsed form dumps back to its own bytes. New
    components go before the ones already there and an entity `$comment` before
    `components`, so no existing line changes."""
    document = json.loads(text)
    if json.dumps(document, **dump) + newline != text:
        raise Refused("the document does not dump back to its own bytes")
    for entity in document["entities"]:
        if entity.get("id") not in plan:
            continue
        components, comment = plan[entity["id"]]
        entity["components"] = {**components, **entity["components"]}
        if comment is not None:
            rebuilt = {}
            for key, value in entity.items():
                if key == "components":
                    rebuilt["$comment"] = comment
                rebuilt[key] = value
            entity.clear()
            entity.update(rebuilt)
    return json.dumps(document, **dump) + newline


RECORD = 4  # the indent a record of `entities` opens at, in the key-sorted files


def as_lines(text: str, plan: dict) -> str:
    """Edit a key-sorted, two-space document by inserting lines only, each
    component where its name sorts among the entity's own."""
    lines = text.split("\n")
    inserts: dict[int, list[str]] = {}
    for id, (components, comment) in plan.items():
        at_id = [i for i, line in enumerate(lines) if line == f'{" " * (RECORD + 2)}"id": "{id}",']
        if len(at_id) != 1:
            raise Refused(f"{id} is not declared once as a key-sorted record")
        start = max(i for i in range(at_id[0]) if lines[i] == " " * RECORD + "{")
        if lines[start + 1] != " " * (RECORD + 2) + '"components": {':
            raise Refused(f"{id}'s record does not open with its components")
        end = next(i for i in range(start + 2, at_id[0]) if lines[i] == " " * (RECORD + 2) + "},")
        indent = " " * (RECORD + 4)
        existing = [
            (i, re.match(r'\s*"([^"]+)"', lines[i]).group(1))
            for i in range(start + 2, end)
            if lines[i].startswith(indent + '"')
        ]
        names = [name for _, name in existing]
        if names != sorted(names):
            raise Refused(f"{id}'s components are not key-sorted: {names}")
        for name in sorted(components):
            before = next(i for i, existing_name in existing if existing_name > name)
            rendered = json.dumps({name: components[name]}, indent=2, sort_keys=True, ensure_ascii=False)
            block = [indent + line[2:] for line in rendered.split("\n")[1:-1]]
            block[-1] += ","
            inserts.setdefault(before, []).extend(block)
        if comment is not None:
            line = " " * (RECORD + 2) + '"$comment": ' + json.dumps(comment, ensure_ascii=False) + ","
            inserts.setdefault(start + 1, []).insert(0, line)
    out = []
    for i, line in enumerate(lines):
        out.extend(inserts.get(i, []))
        out.append(line)
    return "\n".join(out)


def edit_world(root: str, name: str, old: str) -> tuple[str, int]:
    """The world with its robot entities' aspects, and the lines it gained."""
    plan_for, dump, newline = WORLDS[name]
    plan = plan_for(root, json.loads(old))
    carried = state(json.loads(old), plan)
    if carried != "bare":
        raise Refused(f"its robot entities already carry {'all' if carried == 'done' else 'some'} of the planned aspects")
    new = as_data(old, plan, dump, newline) if dump is not None else as_lines(old, plan)
    expected = json.loads(old)
    for entity in expected["entities"]:
        if entity.get("id") in plan:
            components, comment = plan[entity["id"]]
            entity["components"].update(copy.deepcopy(components))
            if comment is not None:
                entity["$comment"] = comment
    if json.loads(new) != expected:
        raise Refused("the parsed edit is not the old document plus the plan")
    matcher = difflib.SequenceMatcher(a=old.split("\n"), b=new.split("\n"), autojunk=False)
    if {tag for tag, *_ in matcher.get_opcodes()} - {"equal", "insert"}:
        raise Refused("the edit would change or remove a line, not only insert")
    return new, len(new.split("\n")) - len(old.split("\n"))


# --------------------------------------------------------------------------
# Specs
# --------------------------------------------------------------------------

ROBOT_KEYS = {
    "htm": {
        "pickup_secs", "load_time_secs", "deposit_secs", "unload_time_secs", "conveyor_pickup_secs",
        "conveyor_deposit_secs", "loaded_speed", "empty_speed", "loaded_accel", "empty_accel",
        "loaded_rotate", "empty_rotate", "loaded_rotate_180", "empty_rotate_180",
    },
    "vtm": {
        "pick_secs", "drop_secs", "alignment_secs", "pick_alignment_secs", "drop_alignment_secs",
        "chassis_speed", "chassis_accel", "lift_speed", "lift_accel",
    },
}

THROUGHPUT_OLD = (
    "It carries the bot specifications directed on 2026-09-21 (docs/walmart-facts.md section 6b) and is "
    "built on walmart_dfw6_baseline_partial_spec.json. The vtm and htm blocks state the section 6b figures, "
    "each with its row and the compiled default it replaces."
)
THROUGHPUT_NEW = (
    "It runs on the bot specifications directed on 2026-09-21 (docs/walmart-facts.md section 6b), which the "
    "robot entities of walmart_dfw6.json state, and is built on walmart_dfw6_baseline_partial_spec.json."
)

SPECS = {
    "cencora_v3_spec.json": (dict(indent=1, ensure_ascii=True), ""),
    "walmart_dfw6_throughput_spec.json": (dict(indent=2, ensure_ascii=True), "\n"),
}


def edit_spec(name: str, old: str) -> tuple[str, list[str]]:
    """The spec without its robot figures, and what went."""
    dump, newline = SPECS[name]
    spec = json.loads(old)
    if json.dumps(spec, **dump) + newline != old:
        raise Refused("the spec does not dump back to its own bytes")
    removed = []
    for fleet, keys in ROBOT_KEYS.items():
        block = spec["settings"].get(fleet)
        if block is None:
            continue
        gone = [key for key in block if key in keys]
        for key in gone:
            removed.append(f"{fleet}.{key}")
            del block[key]
        # A block's `$comment` explains its robot figures, so it goes with them.
        if gone and set(block) == {"$comment"}:
            removed.append(f"{fleet}.$comment")
            del block["$comment"]
        if not block:
            del spec["settings"][fleet]
    comment = spec["settings"].get("$comment")
    if comment is not None and THROUGHPUT_OLD in comment:
        spec["settings"]["$comment"] = comment.replace(THROUGHPUT_OLD, THROUGHPUT_NEW)
        removed.append("the settings $comment's sentence about the vtm and htm blocks")
    for fleet, keys in ROBOT_KEYS.items():
        if set(spec["settings"].get(fleet, {})) & keys:
            raise Refused(f"settings.{fleet} still states a robot figure")
    return json.dumps(spec, **dump) + newline, removed


# --------------------------------------------------------------------------


def documents() -> list[tuple[str, str]]:
    return [("world", name) for name in WORLDS] + [("spec", name) for name in SPECS]


def audit(root: str, base: str) -> int:
    """Prove every document is `base` with exactly these edits, no others.

    Once the edit is committed, HEAD holds it and there is nothing left to
    compare, so name the commit the edit was made on top of. A document that
    already carries the edit at `base` fails rather than passing unearned.
    """
    failures = 0
    for kind, name in documents():
        path = f"{FIXTURES}/{name}"
        old = subprocess.run(
            ["git", "-C", root, "show", f"{base}:{path}"], capture_output=True, check=True
        ).stdout.decode("utf-8")
        try:
            new = edit_world(root, name, old)[0] if kind == "world" else edit_spec(name, old)[0]
            if new == old:
                raise Refused(f"it already carries the edit at {base}")
            with open(os.path.join(root, path), "rb") as handle:
                proven = handle.read() == new.encode("utf-8")
            note = "PROVEN" if proven else "DIFFERS FROM THE PLAN"
        except Refused as refusal:
            proven, note = False, f"REFUSED: {refusal}"
        failures += not proven
        print(f"{path:<62}  {note}")
    verdict = f"every one is {base} with exactly these edits" if not failures else f"{failures} are not"
    print(f"{len(documents())} documents: {verdict}")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dry-run", action="store_true", help="report, write nothing")
    mode.add_argument("--apply", action="store_true", help="edit the documents")
    mode.add_argument("--audit", action="store_true", help="prove every document is `--base` with these edits")
    parser.add_argument("--base", default="HEAD", help="the commit the edit was made on top of (for --audit)")
    parser.add_argument(
        "--root",
        default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        help="the repository root",
    )
    args = parser.parse_args()
    if args.audit:
        return audit(args.root, args.base)

    refusals = 0
    for kind, name in documents():
        path = os.path.join(args.root, FIXTURES, name)
        old = read(path)
        try:
            if kind == "world":
                plan_for = WORLDS[name][0]
                current = state(json.loads(old), plan_for(args.root, json.loads(old)))
                if current == "done":
                    print(f"{name}: its robot entities already carry their aspects")
                    continue
                new, gained = edit_world(args.root, name, old)
                report = f"{gained} lines inserted"
            else:
                new, removed = edit_spec(name, old)
                if not removed:
                    print(f"{name}: states no robot figure")
                    continue
                report = f"removed {', '.join(removed)}"
        except Refused as refusal:
            refusals += 1
            print(f"{name}: REFUSED: {refusal}")
            continue
        if args.apply:
            with open(path, "w", encoding="utf-8", newline="") as handle:
                handle.write(new)
        print(f"{name}: {report}{'' if args.apply else ' (dry run)'}")
    return 1 if refusals else 0


if __name__ == "__main__":
    sys.exit(main())
