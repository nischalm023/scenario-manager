#!/usr/bin/env python3
"""Carry a world document off the retired `exclusion` component.

An `exclusion` entity used to subtract cells from every generator in the
document — action at a distance, since what a tiling derived was not a function
of the tiling itself. Exclusion survives as a field a tiling owns: each tiling
states its own carve-outs in `exclusions`, so a hole meant for one tiling can no
longer take cells out of another that happens to overlap it.

This script is the one-time move for documents written before that change. For
every entity carrying an `exclusion`, it localizes that entity's region against
each tiled region it overlaps, writes the rectangle into that tiling's own
`exclusions`, strips the component, and deletes the entity when subtracting was
its only job. It refuses rather than guesses: a frame it would have to convert
across, a non-integer length, or a deleted id something still points at all stop
the migration with the file untouched.

Usage:

    migrate_exclusions_to_tiling.py <file.json> [--context <layer.json>]... [--dry-run]

It prints what it did (or would do) and writes the file in place unless
`--dry-run` is given. The output keeps the house JSON layout of these documents:
two-space indent, one value per line, except an array of number rows — a rack
level's `positions` — which stays on one line.

A `$comment` riding on an `exclusion` component is not dropped: it is copied,
prefixed with the carrier's name, into every carve-out that rectangle becomes,
so the prose a deleted entity carried survives where its geometry went.

`--context` names the other layers of a layered world (catalog, policy,
workload). They are never modified, but the whole world is read from them: the
frame chain, the templates a stamp places, and the tilings a sibling declares
all take part in the checks below, and an `exclusion` carried there, or a
deleted id still referenced there, refuses the migration.

What it refuses, and why each one is a wrong answer rather than a hard one:

* a frame whose chain to the primary does anything but shift by whole steps —
  the retired rule compared a carve-out and a cell centre after projecting both
  into the primary frame, and only a whole-step shift leaves every verdict where
  it was;
* a carve-out overlapping a tiling declared in a sibling layer, or one a stamp
  places from inside a template — this run writes one file, and neither of those
  tilings is in it;
* a carrier that is an override, a patch or a suppression rather than a plain
  declaration — its `region` is not the rectangle that was subtracting cells.
"""

import argparse
import json
import sys


class Refusal(Exception):
    """A document this migration will not touch, and the reason why."""


# ---------------------------------------------------------------------------
# Reading the document
# ---------------------------------------------------------------------------


def components_of(record):
    return record.get("components") or {}


def component_names(record):
    """The component names a record carries, leaving out annotation keys."""
    return [name for name in components_of(record) if not name.startswith("$")]


def rectangle(record, what):
    """One record's region as `(frame, origin, size)`, in that region's frame."""
    region = components_of(record).get("region")
    if not isinstance(region, dict):
        raise Refusal(f"{what} carries no `region`, so it states no rectangle")
    origin = integer_pair(region.get("origin"), f"{what}'s region origin")
    size = integer_pair(region.get("size"), f"{what}'s region size")
    return region.get("frame"), origin, size


def integer_pair(value, what):
    if not isinstance(value, list) or len(value) != 2:
        raise Refusal(f"{what} is not a pair: {value!r}")
    for entry in value:
        if isinstance(entry, bool) or not isinstance(entry, int):
            raise Refusal(
                f"{what} states {entry!r}, and this migration is whole-step "
                f"arithmetic only"
            )
    return [value[0], value[1]]


def label(record):
    name = record.get("name")
    identity = record.get("id", "<no id>")
    return f"entity {identity}" + (f" ({name})" if name else "")


def is_whole_number(value):
    """Whether a JSON value is a whole number. `true` is not one."""
    if isinstance(value, bool):
        return False
    if isinstance(value, int):
        return True
    return isinstance(value, float) and value.is_integer()


def tiled_records(records):
    """The records that tile an area of their own: a region and a tiling."""
    return [
        r
        for r in records
        if "tiling" in components_of(r) and "region" in components_of(r)
    ]


def frames_by_id(documents):
    """Every frame declared across the target and its context layers."""
    out = {}
    for _, document in documents:
        for record in document.get("entities") or []:
            frame = components_of(record).get("frame")
            if isinstance(frame, dict) and "id" in record:
                out[record["id"]] = frame
    return out


def templates_by_id(documents):
    """Every template declared across the world, by the id a stamp names."""
    out = {}
    for _, document in documents:
        for record in document.get("entities") or []:
            template = components_of(record).get("template")
            if isinstance(template, dict) and "id" in record:
                out[record["id"]] = (record, template)
    return out


# ---------------------------------------------------------------------------
# The move
# ---------------------------------------------------------------------------


def insert_after(mapping, existing, key, value):
    """`mapping` with `key: value` placed directly after `existing`."""
    out = {}
    for name, item in mapping.items():
        out[name] = item
        if name == existing:
            out[key] = value
    return out


def overlaps(origin_a, size_a, origin_b, size_b):
    """Whether two rectangles share area — open intervals on both axes.

    Edge-touching is not overlap: a carve-out flush against a region's boundary
    contains no cell centre of it, so writing it into that tiling would add a
    rectangle that removes nothing.
    """
    return all(
        origin_a[axis] < origin_b[axis] + size_b[axis]
        and origin_b[axis] < origin_a[axis] + size_a[axis]
        for axis in (0, 1)
    )


def check_frame_chain(frame_id, frames):
    """Refuse unless this frame reaches the primary by whole-step shifts only.

    Matching frame ids are not enough to carry the old verdicts over. The
    retired rule projected both the carve-out rectangle and the cell centre into
    the primary frame and compared them there, rounding whatever landed between
    steps; a tiling's own `exclusions` are compared where they are written, in
    the region's frame. The two agree exactly while every hop up to the primary
    only shifts by whole steps. A scale of anything but 1 changes the distance
    between a cell centre and a carve-out edge on the way up, and the rounding
    that follows can then land a centre on the other side of that edge — a cell
    the old world removed and the new one keeps, or the reverse.
    """
    if frame_id is None:
        raise Refusal(
            "the exclusions and tilings here state no frame, so there is no chain "
            "to the primary for this migration to check"
        )
    walked = []
    current = frame_id
    while current is not None:
        if current in walked:
            raise Refusal(f"the frame {current} is its own ancestor")
        walked.append(current)
        frame = frames.get(current)
        if frame is None:
            raise Refusal(
                f"the frame {current} carries this world's exclusions and tilings, "
                f"and no document this run was given declares it; name the layer "
                f"that does with --context"
            )
        parent = frame.get("parent")
        transform = frame.get("transform_to_parent")
        if parent is not None and isinstance(transform, dict):
            hop = (
                "its own `transform_to_parent`"
                if current == frame_id
                else f"the `transform_to_parent` of {current}"
            )
            scale = transform.get("scale") or []
            if any(not is_whole_number(entry) or entry != 1 for entry in scale):
                raise Refusal(
                    f"the frame {frame_id} carries this world's exclusions and tilings, "
                    f"and reaches the primary through {hop}, which scales by {scale}; "
                    f"the retired rule tested a carve-out after projecting it, and only "
                    f"a whole-step shift leaves every verdict where it was"
                )
            translation = transform.get("translation") or []
            if any(not is_whole_number(entry) for entry in translation):
                raise Refusal(
                    f"the frame {frame_id} carries this world's exclusions and tilings, "
                    f"and reaches the primary through {hop}, which translates by "
                    f"{translation}; only a whole-step shift leaves every verdict where "
                    f"it was"
                )
        current = parent


def check_sibling_tilings(contexts, carriers, shared_frame):
    """Refuse where a carve-out lands on a tiling this run cannot write to.

    A sibling layer is read, never rewritten, so a carve-out overlapping a
    tiling declared there has nowhere to go: the entity that used to remove
    those cells is about to be deleted from this file, and the tiling that
    should inherit the rectangle is in another one.
    """
    for ctx_path, ctx_document, _ in contexts:
        for target in tiled_records(ctx_document.get("entities") or []):
            frame, region_origin, region_size = rectangle(target, label(target))
            if frame != shared_frame:
                raise Refusal(
                    f"{ctx_path}: {label(target)} tiles in frame {frame} while the "
                    f"exclusions here are in {shared_frame}; this migration converts "
                    f"nothing, so it cannot tell whether the two overlap"
                )
            for carrier in carriers:
                _, origin, size = rectangle(carrier, label(carrier))
                if overlaps(origin, size, region_origin, region_size):
                    raise Refusal(
                        f"{label(carrier)} carves into {ctx_path}: {label(target)}, "
                        f"and this run writes one file — the rectangle has no tiling "
                        f"here to land in. Migrate the two layers together, by hand"
                    )


# How many repeat instances of one stamp this migration will walk. Real repeats
# count in single digits; a number past this is a document to read before it is
# a document to migrate.
MOST_INSTANCES = 10_000


def check_stamped_tilings(documents, carriers, shared_frame):
    """Refuse where a stamp puts a template's own tiling under a carve-out.

    A tiling inside a template is not a record this migration can write to, and
    the retired rule reached the cells a stamp derives from one like any others.
    So every stamp of a template that tiles is walked: at heading 0 the stamped
    rectangle is the stamp's position plus the internal's own origin, once per
    repeat instance. Any other heading re-corners that rectangle against an
    extent this walk does not compute, so it refuses instead of guessing — and a
    template whose tiling no carve-out reaches costs nothing and says nothing.
    """
    templates = templates_by_id(documents)
    for path, document in documents:
        for record in document.get("entities") or []:
            stamp = components_of(record).get("stamp")
            if not isinstance(stamp, dict):
                continue
            held = templates.get(stamp.get("template"))
            if held is None:
                continue
            template_record, template = held
            internals = [
                te
                for te in (template.get("entities") or [])
                if {"region", "tiling"} <= set(te.get("components") or {})
            ]
            if not internals:
                continue

            where = f"{path}: {label(record)} stamps {label(template_record)}"
            pose = components_of(record).get("pose")
            if not isinstance(pose, dict):
                raise Refusal(f"{where}, which tiles, and states no `pose` to place it")
            if pose.get("frame") != shared_frame:
                raise Refusal(
                    f"{where}, which tiles, in frame {pose.get('frame')} while the "
                    f"exclusions here are in {shared_frame}; this migration converts "
                    f"nothing, so it cannot tell whether the two overlap"
                )
            heading = pose.get("heading_deg") or 0
            if not is_whole_number(heading) or heading % 360 != 0:
                raise Refusal(
                    f"{where} at {heading}°, and a turned stamp re-corners its "
                    f"template's tiling against an extent this migration does not "
                    f"compute; migrate this world by hand"
                )
            position = integer_pair(
                list(pose.get("position") or [])[:2], f"{where}: its pose position"
            )
            repeat = stamp.get("repeat") or {}
            count = repeat.get("count", 1)
            if not is_whole_number(count) or isinstance(count, float) or count < 1:
                raise Refusal(
                    f"{where} with `repeat.count` {count!r}, which is not a positive "
                    f"whole number"
                )
            if count > MOST_INSTANCES:
                raise Refusal(
                    f"{where} {count} times, which is more instances than this "
                    f"migration will walk"
                )
            step = integer_pair(repeat.get("step", [0, 0]), f"{where}: its repeat step")

            for index in range(count):
                anchor = [
                    position[0] + index * step[0],
                    position[1] + index * step[1],
                ]
                for internal in internals:
                    what = f"{where}, internal `{internal.get('local')}`"
                    frame, origin, size = rectangle(internal, what)
                    # A template internal naming its own frame is already
                    # absolute; only a frameless one travels with the stamp.
                    if frame is None:
                        origin = [anchor[0] + origin[0], anchor[1] + origin[1]]
                    elif frame != shared_frame:
                        raise Refusal(
                            f"{what} tiles in frame {frame} while the exclusions here "
                            f"are in {shared_frame}; this migration converts nothing"
                        )
                    for carrier in carriers:
                        _, c_origin, c_size = rectangle(carrier, label(carrier))
                        if overlaps(c_origin, c_size, origin, size):
                            raise Refusal(
                                f"{label(carrier)} carves into {what}, whose rectangle "
                                f"lives inside a template this migration cannot write "
                                f"to. Migrate this world by hand"
                            )


def internal_carriers(records):
    """`(record, handle)` for every template internal carrying `exclusion`.

    A template's internals never appear as top-level records, so the carrier
    scan cannot see them — but the old global rule could still act on the cells
    they stamp, and this migration cannot reach inside a template to say
    otherwise. Finding one is a refusal, not a warning.
    """
    return [
        (r, te.get("local"))
        for r in records
        for te in (components_of(r).get("template") or {}).get("entities", [])
        if "exclusion" in (te.get("components") or {})
    ]


def migrate(document, path, contexts):
    """Rewrite `document` in place. Returns the summary lines to print.

    `contexts` is `[(path, document, text)]` for the sibling layers of a
    layered world: searched, never modified.
    """
    records = document.get("entities")
    if not isinstance(records, list):
        raise Refusal(f"{path} has no `entities` array")

    for ctx_path, ctx_document, _ in contexts:
        ctx_records = ctx_document.get("entities")
        if not isinstance(ctx_records, list):
            continue
        strays = [r for r in ctx_records if "exclusion" in components_of(r)]
        if strays:
            raise Refusal(
                f"{ctx_path} carries `exclusion` too ({label(strays[0])}"
                f"{' and more' if len(strays) > 1 else ''}); migrate that "
                f"layer in its own run, with this file as --context"
            )
        for record, handle in internal_carriers(ctx_records):
            raise Refusal(
                f"{ctx_path}: template internal {label(record)} / {handle} "
                f"carries `exclusion`, which this migration cannot reach"
            )

    for record, handle in internal_carriers(records):
        raise Refusal(
            f"template internal {label(record)} / {handle} carries "
            f"`exclusion`, which this migration cannot reach"
        )

    carriers = [r for r in records if "exclusion" in components_of(r)]
    if not carriers:
        return None

    # Only a plain declaration states the rectangle that was subtracting cells.
    # An override or a patch names an entity somebody else declared and carries
    # whichever components it means to change — so reading its `region` as a
    # carve-out would mint removals the old world never made, and deleting the
    # record afterwards would take the override with it.
    for record in carriers:
        wrong = None
        if "derivation" in record:
            wrong = "a `derivation`, which makes it an override"
        elif record.get("suppress") is True:
            wrong = "`suppress: true`"
        elif record.get("patch") is True:
            wrong = "`patch: true`"
        elif "id" not in record:
            wrong = "no `id`"
        if wrong:
            raise Refusal(
                f"{label(record)} carries an `exclusion` and {wrong}; this migration "
                f"moves plain declarations only, and an override, a patch or a "
                f"suppression carrying one needs a ruling of its own"
            )

    tiled = tiled_records(records)

    # One frame, or nothing: comparing two rectangles measured in different
    # frames would mean converting between them, and a migration that converts
    # geometry is a migration that can move a hole.
    frames = {}
    for record in carriers + tiled:
        frame, _, _ = rectangle(record, label(record))
        frames.setdefault(frame, []).append(label(record))
    if len(frames) > 1:
        detail = "; ".join(
            f"{frame}: {', '.join(who[:3])}{'…' if len(who) > 3 else ''}"
            for frame, who in frames.items()
        )
        raise Refusal(
            f"{path} states its exclusions and its tilings in more than one "
            f"frame, and this migration converts nothing — {detail}"
        )
    shared_frame = next(iter(frames))

    documents = [(path, document)] + [(p, d) for p, d, _ in contexts]
    check_frame_chain(shared_frame, frames_by_id(documents))
    check_sibling_tilings(contexts, carriers, shared_frame)
    check_stamped_tilings(documents, carriers, shared_frame)

    lines = []
    placements = {index: [] for index in range(len(tiled))}
    for record in carriers:
        _, origin, size = rectangle(record, label(record))
        # The prose the component carried goes where its geometry goes: into
        # every carve-out this rectangle becomes, prefixed with who said it —
        # after the entity is deleted, the carve-out is the only place left.
        note = (components_of(record).get("exclusion") or {}).get("$comment")
        prefix = record.get("name") or record.get("id")
        for index, target in enumerate(tiled):
            _, region_origin, region_size = rectangle(target, label(target))
            if not overlaps(origin, size, region_origin, region_size):
                continue
            # Unclipped: the localized origin may go negative, which is the
            # honest reading of a carve-out that runs past its tiling's edge.
            carve_out = {}
            if note:
                carve_out["$comment"] = f"{prefix}: {note}"
            carve_out["origin"] = [
                origin[0] - region_origin[0],
                origin[1] - region_origin[1],
            ]
            carve_out["size"] = [size[0], size[1]]
            placements[index].append(carve_out)
    for index, carve_outs in placements.items():
        if not carve_outs:
            continue
        target = tiled[index]
        tiling = components_of(target).get("tiling")
        if not isinstance(tiling, dict) or "pitch" not in tiling:
            raise Refusal(f"{label(target)}'s `tiling` states no `pitch`")
        if "exclusions" in tiling:
            raise Refusal(
                f"{label(target)}'s tiling already states `exclusions`, so this "
                f"document has been migrated once already"
            )
        # Beside the pitch, ahead of `derives`: the same order the reader's own
        # `Tiling` states its fields in.
        components_of(target)["tiling"] = insert_after(
            tiling, "pitch", "exclusions", carve_outs
        )

    deleted = []
    stripped = []
    for record in carriers:
        del record["components"]["exclusion"]
        if component_names(record) == ["region"]:
            deleted.append(record["id"])
        else:
            stripped.append(record["id"])

    document["entities"] = [r for r in records if r.get("id") not in set(deleted)]

    required = document.get("required_components")
    dropped_requirement = False
    if isinstance(required, list) and "exclusion" in required:
        document["required_components"] = [c for c in required if c != "exclusion"]
        dropped_requirement = True

    # Every tiling stays in place, so only a deleted entity can strand a
    # pointer. Checked against the text this migration is about to write —
    # the only thing a later reader will see — and against every sibling
    # layer, which this run does not rewrite and so must not orphan.
    text = render(document)
    for identity in deleted:
        if identity in text:
            raise Refusal(
                f"{identity} was to be deleted — subtracting cells was its only "
                f"job — but something in {path} still names it"
            )
        for ctx_path, _, ctx_text in contexts:
            if identity in ctx_text:
                raise Refusal(
                    f"{identity} was to be deleted — subtracting cells was its "
                    f"only job — but {ctx_path} still names it"
                )

    written = sum(len(c) for c in placements.values())
    reached = sum(1 for c in placements.values() if c)
    lines.append(
        f"{len(carriers)} exclusion entit{'y' if len(carriers) == 1 else 'ies'}, "
        f"{written} carve-out(s) written into {reached} of {len(tiled)} tiling(s)"
    )
    for index, carve_outs in placements.items():
        if not carve_outs:
            continue
        lines.append(f"  {label(tiled[index])}: +{len(carve_outs)}")
    lines.append(
        f"{len(deleted)} entit{'y' if len(deleted) == 1 else 'ies'} deleted "
        f"(region and nothing else left), {len(stripped)} kept without the component"
    )
    if dropped_requirement:
        lines.append("`exclusion` removed from `required_components`")

    return lines, text


# ---------------------------------------------------------------------------
# Writing the document
# ---------------------------------------------------------------------------


def render(value, indent=0):
    """The document as text, in the layout these files are written in."""
    pad = "  " * indent
    inner = "  " * (indent + 1)
    if isinstance(value, dict):
        if not value:
            return "{}"
        entries = [
            f"{inner}{json.dumps(key, ensure_ascii=False)}: {render(item, indent + 1)}"
            for key, item in value.items()
        ]
        return "{\n" + ",\n".join(entries) + "\n" + pad + "}"
    if isinstance(value, list):
        if not value:
            return "[]"
        # A rack level's `positions` and nothing else: rows of numbers read as a
        # table on one line and as noise down forty.
        if all(is_number_row(item) for item in value):
            return (
                "["
                + ", ".join(
                    json.dumps(item, ensure_ascii=False, separators=(", ", ": "))
                    for item in value
                )
                + "]"
            )
        entries = [f"{inner}{render(item, indent + 1)}" for item in value]
        return "[\n" + ",\n".join(entries) + "\n" + pad + "]"
    return json.dumps(value, ensure_ascii=False)


def is_number_row(value):
    return (
        isinstance(value, list)
        and value
        and all(
            isinstance(entry, (int, float)) and not isinstance(entry, bool)
            for entry in value
        )
    )


def main():
    parser = argparse.ArgumentParser(
        description="Move `exclusion` entities into the `exclusions` of the "
        "tilings they carve"
    )
    parser.add_argument("path", help="the world document to migrate")
    parser.add_argument(
        "--context",
        action="append",
        default=[],
        metavar="LAYER",
        help="a sibling layer of the same world, never modified: it supplies "
        "the frames and templates this world is built from, and its own "
        "carriers, tilings and references all take part in the checks "
        "(repeatable)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="say what would change and write nothing",
    )
    args = parser.parse_args()

    with open(args.path, encoding="utf-8") as handle:
        document = json.load(handle)
    contexts = []
    for ctx_path in args.context:
        with open(ctx_path, encoding="utf-8") as handle:
            ctx_text = handle.read()
        contexts.append((ctx_path, json.loads(ctx_text), ctx_text))

    try:
        outcome = migrate(document, args.path, contexts)
    except Refusal as refusal:
        print(f"{args.path}: refused — {refusal}", file=sys.stderr)
        return 1

    if outcome is None:
        print(f"{args.path}: no `exclusion` component here, nothing to migrate")
        return 0

    lines, text = outcome
    for line in lines:
        print(f"{args.path}: {line}")
    if args.dry_run:
        print(f"{args.path}: --dry-run, nothing written")
        return 0
    with open(args.path, "w", encoding="utf-8") as handle:
        handle.write(text + "\n")
    print(f"{args.path}: written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
