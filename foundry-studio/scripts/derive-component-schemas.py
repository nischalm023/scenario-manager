#!/usr/bin/env python3
"""Turn the protocol's JSON Schema layer into one schema per component.

The development seed (`scripts/seed-dev.sh`) publishes what this derives for
the core components, and the Map Creator tests compile the same schemas.

Purpose: the registry publishes one schema per component name, and the server
compiles each through `schema_defs::compile`, which resolves nothing off the
machine — no network, no registry of sibling schema files — and exactly one
document beside the schema: `server/server-defs.json`, the server's own
definitions, folded into every compile it runs and served at
`GET /protocol/defs` for the validators that run elsewhere. The schema layer
under `data/seed/application/schema/` is written the other way round: every file
identifies its neighbours by URN and shares one library of value definitions.
This script bridges the two by inlining, per component, every definition that
component's schema can reach.

So the emitted documents carry `$ref`s of exactly two kinds: local ones into
their own `$defs`, and — on the two reference definitions a pin may extend —
`urn:world-graph-server:defs#/$defs/version_pin`, the one import, which the
server resolves and no other URN in the output survives. `--report` and the
seed's counter both read the output back for stray `urn:worldproto` refs, which
is what says the inlining left nothing behind.

Source files, read here and never written:

    schema/components.json           the register: name -> $ref
    schema/values.json               shared value definitions
    schema/components/spatial.json
    schema/components/structure.json
    schema/components/fleet.json
    schema/components/product.json
    schema/components/identity.json

WHICH READING IT TAKES. `components.json` holds two: `set`, the lenient one
SPEC §9 requires of a consumer, and `set_strict`, the closure an author wants
on a draft they are writing. This script derives from `set` and never from
`set_strict`, because the server is the consumer: a field a newer producer
added is carried, not refused (§9). Nothing here adds `unevaluatedProperties`,
and `--report` names the two places the layer itself closes an object so that
neither is mistaken for something this script did.

WHAT IS NOT IN THE LAYER. A name the register does not carry gets a
hand-written shape from HAND_WRITTEN below, sourced from SPEC §10's row and the
payloads the reference crate emits. `--report` lists which components came from
where, so "layer-derived" and "hand-written" is never a guess.

Usage:
    derive-component-schemas.py [--schema-dir DIR] [--out FILE] [--report] NAME...

The component names are arguments rather than a built-in application vocabulary.
"""

import argparse
import functools
import json
import os
import sys

# The register's own reading of a component map — the lenient one (SPEC §9).
REGISTER = "urn:worldproto:0.7:components#/$defs/set"

# Every value definition whose instances name another entity. A component field
# typed as one of these is a reference-bearing field, which is what the server's
# registry annotates so its reference index has something to extract from
# (DESIGN.md §4). `--report` walks the derived schemas for these and prints
# where they sit, which is how the seed's annotation list is kept honest.
REFERENCE_DEFS = {
    "urn:worldproto:0.7:values#/$defs/entity_ref",
    "urn:worldproto:0.7:values#/$defs/entity_id",
    "urn:worldproto:0.7:values#/$defs/cell_address",
    "urn:worldproto:0.7:values#/$defs/declared_entity_id",
    "urn:worldproto:0.7:values#/$defs/derived_entity_id",
}

# Components the schema layer does not define, with the shape they get instead
# and the reason the layer has none. Only vendor extensions belong here: the
# layer covers the whole core vocabulary, and a core name missing from it would
# be a gap to report rather than to fill in silently.
HAND_WRITTEN = {
    "highway": {
        "title": "highway",
        "description": (
            "A marker the Walmart maps' floor tilings stamp on through-lane cells "
            "beside `cell`. Not a protocol component — the lens crate's pattern "
            "library says exactly that — and hand-written here so a demo can load "
            "Walmart DFW6, whose tilings derive it. It carries no fields."
        ),
        "type": "object",
        "additionalProperties": False,
    },
    "easyloop": {
        "title": "easyloop",
        "description": (
            "GreyOrange's EasyLoop pick station: the loop transit and per-pick "
            "timings that run it. Hand-written rather than layer-derived — "
            "`schema/components.json` registers the core vocabulary only, and "
            "a product's component is not part of the core, whatever its name "
            "(SPEC §2). The two fields are the ones the walmart-dallas "
            "catalog's EasyLoop template declares, and each takes a template "
            "parameter reference as well as a number, because that template "
            "binds both through `$param`."
        ),
        "type": "object",
        "properties": {
            "loop_transit_secs": {
                "description": "Seconds for a carrier to travel the loop once.",
                "$ref": "#/$defs/seconds",
            },
            "time_per_pick_secs": {
                "description": "Seconds an operator spends on one pick.",
                "$ref": "#/$defs/seconds",
            },
        },
        "$defs": {
            "seconds": {
                "title": "seconds",
                "description": (
                    "A duration in seconds. Spelled here rather than taken from "
                    "`values.json`, which has no duration: a time is not a length, "
                    "so it takes its unit from no frame and keeps its `_secs` "
                    "suffix. The parameter alternative is what lets the EasyLoop "
                    "template declare both of these as `$param` and a stamp bind "
                    "them (SPEC §6)."
                ),
                "anyOf": [
                    {"type": "number", "exclusiveMinimum": 0},
                    {"$ref": "urn:worldproto:0.7:values#/$defs/param_ref"},
                ],
            }
        },
    }
}


def load_layer(schema_dir):
    """Every schema file in the layer, indexed by its own `$id`."""
    paths = [
        os.path.join(schema_dir, "values.json"),
        os.path.join(schema_dir, "components.json"),
    ]
    family_dir = os.path.join(schema_dir, "components")
    paths += sorted(
        os.path.join(family_dir, name)
        for name in os.listdir(family_dir)
        if name.endswith(".json")
    )
    by_id, source_of = {}, {}
    for path in paths:
        with open(path, encoding="utf-8") as handle:
            document = json.load(handle)
        schema_id = document.get("$id")
        if not schema_id:
            raise SystemExit(f"{path}: every schema file states its $id")
        by_id[schema_id] = document
        source_of[schema_id] = path
    return by_id, source_of


def split_ref(ref, current_id):
    """A `$ref` as (document id, definition name). Local refs keep the current id."""
    base, _, pointer = ref.partition("#")
    schema_id = base or current_id
    prefix = "/$defs/"
    if not pointer.startswith(prefix) or "/" in pointer[len(prefix) :]:
        raise SystemExit(
            f"{ref}: this script inlines `#/$defs/<name>` pointers only, and the "
            "layer has used a shape it does not know how to follow"
        )
    return schema_id, pointer[len(prefix) :]


def local_name(schema_id, def_name):
    """The name one definition takes in the emitted document's own `$defs`.

    Prefixed by the family — `values_steps`, `structure_level` — so definitions
    from two files can never collide, and so a reader of the emitted document
    can see at a glance which source file a definition came out of.
    """
    family = schema_id.rsplit(":", 1)[-1]
    return f"{family}_{def_name}"


def rewrite(node, current_id, wanted):
    """One subschema with its refs made local, queueing what they reach."""
    if isinstance(node, list):
        return [rewrite(item, current_id, wanted) for item in node]
    if not isinstance(node, dict):
        return node
    out = {}
    for key, value in node.items():
        if key == "$ref" and isinstance(value, str):
            if current_id is None and value.startswith("#"):
                # A hand-written entry pointing at a definition it carries
                # itself: already local, and nothing in the layer to collect.
                out[key] = value
                continue
            schema_id, def_name = split_ref(value, current_id)
            wanted.append((schema_id, def_name))
            out[key] = f"#/$defs/{local_name(schema_id, def_name)}"
        else:
            out[key] = rewrite(value, current_id, wanted)
    return out


def collect(by_id, roots):
    """Every definition reachable from `roots`, rewritten and keyed locally.

    Breadth-first with a seen set, so the layer's cycles — a tiling's `derives`
    is a component map, which contains tilings — terminate instead of inlining
    forever. A cycle is a local `$ref` in the output, which is what a validator
    wants anyway.
    """
    defs, seen, queue = {}, set(), list(roots)
    while queue:
        schema_id, def_name = queue.pop(0)
        if (schema_id, def_name) in seen:
            continue
        seen.add((schema_id, def_name))
        document = by_id.get(schema_id)
        if document is None:
            raise SystemExit(f"{schema_id}: no file in the layer carries this $id")
        body = document.get("$defs", {}).get(def_name)
        if body is None:
            raise SystemExit(f"{schema_id}#/$defs/{def_name}: no such definition")
        defs[local_name(schema_id, def_name)] = rewrite(body, schema_id, queue)
    return defs


# The two definitions that mean "this field points at another entity", and are
# therefore the two the server's own reference grammar extends. Nothing else is
# touched: `derivation_address.generator` refs `declared_entity_id` directly and
# must NOT take a pin, because the two-part spelling of an address has nowhere
# to put one.
PINNABLE = {("urn:worldproto:0.7:values", "entity_ref"),
            ("urn:worldproto:0.7:values", "cell_address")}

# The server's own definitions document, and the definition in it that a
# pinnable reference imports. Loaded from the server source rather than copied here: the
# server embeds the same file and serves it at `GET /protocol/defs`, so the
# grammar has one home, and the URN is read out of that home rather than
# spelled again here — a rename of the document renames the refs with it.
SERVER_DEFS = os.path.join(
    os.environ["FOUNDRY_WPS_ROOT"], "server", "server", "server-defs.json"
)

PIN_DEF = "version_pin"


@functools.cache
def pin_ref():
    """The `$ref` a pinnable definition points at, from the server's document.

    DESIGN.md §4 gives a reference one more form than the protocol does: an
    identity may carry `@<changeset>`, which says "read the target as of that
    version of the scope that owns it", and §5 makes that pin MANDATORY on a
    reference crossing a scope boundary. The schema layer knows nothing about
    it — the pin is the server's addition, and DESIGN.md §13 files both halves
    for the spec conversation (reserve `@` out of key segments; state
    `EntityRef` as the type of a reference-bearing field). Until that lands, a
    schema derived from the layer alone would refuse every cross-scope
    reference this server requires.

    The grammar itself is NOT written here (ruled 2026-08-26). It is defined
    once in `server/server-defs.json`, which the server folds into every schema
    it compiles and serves at `GET /protocol/defs`; the derived schemas import
    it. Stamping a copy of it into each of them was one definition per
    document, all of them obliged to say the same thing.
    """
    with open(SERVER_DEFS, encoding="utf-8") as handle:
        urn = json.load(handle)["$id"]
    return f"{urn}#/$defs/{PIN_DEF}"


def derive(by_id, ref):
    """One component's schema document, whole but for the pin it imports."""
    schema_id, def_name = split_ref(ref, None)
    defs = collect(by_id, [(schema_id, def_name)])
    defs = with_pin_grammar(defs)
    root = dict(defs[local_name(schema_id, def_name)])
    # The root's body sits at the top level so a person reading
    # `GET /registry/<component>` sees the real shape rather than one `$ref`.
    # It stays in `$defs` as well, because a cycle may reach back into it.
    root["$schema"] = "https://json-schema.org/draft/2020-12/schema"
    root["$defs"] = defs
    return root


def with_pin_grammar(defs):
    """The collected definitions, with a reference also accepting its pin.

    Each pinnable definition becomes "what the layer says, OR the pinned
    spelling", so nothing the protocol accepts stops being accepted — the pin
    is added beside the layer's grammar and never in place of it. The pinned
    branch is a `$ref` out to the server's definitions document; this is the
    one place the emitted schemas leave a reference for someone else to
    resolve, and `schema_defs::compile` is who resolves it.
    """
    out = dict(defs)
    for schema_id, def_name in PINNABLE:
        key = local_name(schema_id, def_name)
        if key not in out:
            continue
        body = out[key]
        out[key] = {
            "title": body.get("title"),
            "description": body.get("description"),
            "anyOf": [
                {k: v for k, v in body.items() if k not in ("title", "description")},
                {"$ref": pin_ref()},
            ],
        }
    return out


def derive_hand_written(by_id, component):
    """A component the layer has no entry for, from HAND_WRITTEN.

    The entry may carry `$defs` of its own for shapes the layer has no name
    for; those stay under the names it gave them, and whatever they reach in
    the layer is collected beside them exactly as a derived component's is.
    """
    entry = dict(HAND_WRITTEN[component])
    own_defs = entry.pop("$defs", {})
    wanted = []
    root = rewrite(entry, None, wanted)
    defs = {name: rewrite(body, None, wanted) for name, body in own_defs.items()}
    defs.update(collect(by_id, wanted))
    root["$schema"] = "https://json-schema.org/draft/2020-12/schema"
    root["$defs"] = with_pin_grammar(defs)
    return root


def reference_fields(by_id, ref, path="", seen=None):
    """Where a component's schema puts a reference, as JSON pointers.

    The paths come back in the spelling the registry's ref annotations use — a
    trailing `/*` for each element of an array — so this output is directly
    comparable with the seed's `refs_for`.
    """
    if seen is None:
        seen = set()
    schema_id, def_name = split_ref(ref, None)
    if ref in REFERENCE_DEFS:
        return [path or "/"]
    if (schema_id, def_name) in seen:
        return []
    seen = seen | {(schema_id, def_name)}
    body = by_id[schema_id]["$defs"][def_name]
    return walk_for_refs(by_id, body, schema_id, path, seen)


def walk_for_refs(by_id, node, current_id, path, seen):
    if not isinstance(node, dict):
        return []
    found = []
    if "$ref" in node:
        schema_id, def_name = split_ref(node["$ref"], current_id)
        found += reference_fields(by_id, f"{schema_id}#/$defs/{def_name}", path, seen)
    for name, sub in (node.get("properties") or {}).items():
        found += walk_for_refs(by_id, sub, current_id, f"{path}/{name}", seen)
    items = node.get("items")
    if isinstance(items, dict):
        found += walk_for_refs(by_id, items, current_id, f"{path}/*", seen)
    for branch in ("anyOf", "oneOf", "allOf"):
        for sub in node.get(branch) or []:
            found += walk_for_refs(by_id, sub, current_id, path, seen)
    return found


def closures(node, path="$"):
    """Where the layer itself closes an object, so nothing reads as this script's doing."""
    found = []
    if isinstance(node, dict):
        if node.get("additionalProperties") is False:
            found.append(path)
        if "unevaluatedProperties" in node:
            found.append(f"{path} (unevaluatedProperties!)")
        for key, value in node.items():
            found += closures(value, f"{path}.{key}")
    elif isinstance(node, list):
        for index, value in enumerate(node):
            found += closures(value, f"{path}[{index}]")
    return found


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    default_dir = os.path.normpath(os.path.join(here, "..", "data", "seed", "application", "schema"))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("components", nargs="+", metavar="NAME")
    parser.add_argument("--schema-dir", default=default_dir)
    parser.add_argument("--out", help="write the schemas here; stdout if omitted")
    parser.add_argument(
        "--report",
        action="store_true",
        help="print the source of each schema, its reference fields, and every "
        "closed object, to stderr",
    )
    args = parser.parse_args()

    by_id, source_of = load_layer(args.schema_dir)
    register = by_id[REGISTER.split("#")[0]]["$defs"]["set"]["properties"]

    schemas, derived_from = {}, {}
    for component in args.components:
        if component in register:
            ref = register[component]["$ref"]
            schemas[component] = derive(by_id, ref)
            derived_from[component] = source_of[split_ref(ref, None)[0]]
        elif component in HAND_WRITTEN:
            schemas[component] = derive_hand_written(by_id, component)
            derived_from[component] = "hand-written (see HAND_WRITTEN)"
        else:
            raise SystemExit(
                f"{component}: the register does not define it and HAND_WRITTEN has "
                "no shape for it — add one there, with the reason, or drop the "
                "component from the seed's vocabulary"
            )

    text = json.dumps(schemas, indent=2, sort_keys=True, ensure_ascii=False)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(text + "\n")
    else:
        print(text)

    if args.report:

        def say(line):
            print(line, file=sys.stderr)

        say(f"{len(schemas)} component schemas from {args.schema_dir}")
        for component in sorted(schemas):
            where = derived_from[component]
            where = where if where.startswith("hand") else os.path.basename(where)
            size = len(json.dumps(schemas[component]))
            say(f"  {component:<20} {where:<16} {size:>7} bytes")
        say("")
        say("reference-bearing fields, as the registry's annotations spell them:")
        for component in sorted(schemas):
            if component not in register:
                continue
            paths = sorted(set(reference_fields(by_id, register[component]["$ref"])))
            if paths:
                say(f"  {component:<20} {' '.join(paths)}")
        say("")
        closed = []
        for component in sorted(schemas):
            closed += [f"{component}{p[1:]}" for p in closures(schemas[component])]
        say("objects the LAYER closes (none of them this script's doing):")
        for line in closed:
            say(f"  {line}")


if __name__ == "__main__":
    main()
