#!/usr/bin/env python3
"""Validate Go World protocol documents against one of the two profiles.

The schemas identify each other by URN rather than by URL, so nothing is
fetched: this script reads every *.json beside it, registers each under its own
`$id`, and hands the resulting registry to the validator. That is all the
wiring any 2020-12 validator needs — the same three lines in another language
would do.

    python3 validate.py wire ../rust/tests/fixtures/walmart-dallas/*.json
    python3 validate.py authoring ../rust/tests/fixtures/minimal-putaway2/*.json

Exit status is 0 when every file passed. `--quiet` prints one line per file and
nothing else; the default also prints each failure as `instance path: message`.
"""

import sys
import json
import pathlib

try:
    from jsonschema import Draft202012Validator
    from referencing import Registry, Resource
except ImportError:
    sys.exit(
        "needs the jsonschema package (4.18 or newer, for draft 2020-12):\n"
        "    python3 -m venv venv && ./venv/bin/pip install jsonschema"
    )

SCHEMA_DIR = pathlib.Path(__file__).resolve().parent
PROFILES = {
    "wire": "urn:worldproto:0.7:wire",
    "authoring": "urn:worldproto:0.7:authoring",
}


def load_registry():
    """Every schema file in this directory, keyed by the `$id` it declares."""
    resources = []
    for path in sorted(SCHEMA_DIR.rglob("*.json")):
        schema = json.loads(path.read_text())
        if "$id" not in schema:
            continue
        resources.append((schema["$id"], Resource.from_contents(schema)))
    return Registry().with_resources(resources)


def validator_for(profile):
    registry = load_registry()
    uri = PROFILES[profile]
    schema = registry.contents(uri)
    return Draft202012Validator(schema, registry=registry)


def where(error):
    """The instance path, spelled the way a person reads a document."""
    return "/" + "/".join(str(part) for part in error.absolute_path)


def sharpest(error):
    """The most specific complaint behind a broad one.

    A record is a `oneOf` over the four kinds, so a stray field deep inside a
    declaration is reported at the record as "not valid under any of the given
    schemas" with the whole record attached. That is true and useless; the
    branch that got furthest into the instance is the one the author meant, so
    follow it down. The other branches only ever say "this is not the kind of
    record you wrote", which nobody needs to be told.
    """
    while error.context:
        error = max(error.context, key=lambda e: (len(e.absolute_path), e.validator != "required"))
    return error


def main(argv):
    quiet = "--quiet" in argv
    argv = [a for a in argv if a != "--quiet"]
    if len(argv) < 2 or argv[0] not in PROFILES:
        sys.exit(f"usage: validate.py [--quiet] {{{'|'.join(PROFILES)}}} <document.json>...")

    profile, paths = argv[0], argv[1:]
    validator = validator_for(profile)

    failed = 0
    for name in paths:
        path = pathlib.Path(name)
        instance = json.loads(path.read_text())
        errors = sorted(validator.iter_errors(instance), key=lambda e: list(e.absolute_path))
        if errors:
            failed += 1
            print(f"FAIL  {path}  ({len(errors)} error(s), {profile} profile)")
            if not quiet:
                for error in errors:
                    error = sharpest(error)
                    print(f"        {where(error)}: {error.message}")
        else:
            print(f"ok    {path}  ({profile} profile)")

    if failed:
        print(f"\n{failed} of {len(paths)} file(s) failed the {profile} profile")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
