#!/usr/bin/env python3
"""Record world-proto-server's refusals for wps-client's refusal tests (tests/refusal.rs).

The client reads every refusal by its code and details (spec 04 section 3.8), so its tests hold each mapping on an
answer a real server gave rather than one written by hand. This script asks a running world-proto-server for one
refusal of each kind the client acts on, under scopes of its own named `rec<hex>/...`, and writes every answer,
status and body as received, to refusals.json beside it. Run it against a dev stack's server, which it writes to:

    python3 studio/wps-client/tests/fixtures/record-refusals.py http://127.0.0.1:<wps port> <world-proto revision>

Three codes no request reaches (`registry_kept_moving`, `ended_by_postgres`, and the flagged export's `held_twice`)
and the database's `database` are not recorded; the tests state those from the server's DESIGN.md section 10 instead.
"""

import json
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid

SERVER, REVISION = sys.argv[1].rstrip("/"), sys.argv[2]
OUT = __file__.rsplit("/", 1)[0] + "/refusals.json"
TAG = "rec" + uuid.uuid4().hex[:8]
ANSWERS = []


def call(method, path, body=None, raw=None):
    data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
    headers = {"content-type": "application/json"} if data is not None else {}
    request = urllib.request.Request(SERVER + path, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request) as answer:
            return answer.status, json.loads(answer.read() or b"null")
    except urllib.error.HTTPError as refused:
        text = refused.read()
        try:
            return refused.code, json.loads(text)
        except ValueError:
            return refused.code, text.decode(errors="replace")


def ok(method, path, body=None):
    status, answer = call(method, path, body)
    assert 200 <= status < 300, f"{method} {path}: {status} {answer}"
    return answer


def record(case, route, code, method, path, body=None, raw=None):
    status, answer = call(method, path, body, raw)
    assert isinstance(answer, dict) and answer.get("code") == code, f"{case}: {status} {answer}"
    ANSWERS.append({"case": case, "route": route, "status": status, "body": answer})


def mint():
    return str(uuid.uuid4())


def changesets(scope):
    return f"/scopes/{scope}/-/changesets"


def changeset(scope, base, ops, **beside):
    return {"scope": scope, "base": base, "author": "recorder", "message": "a recorded refusal", "ops": ops, **beside}


def declare(id, components, scope=None):
    op = {"op": "declare", "id": id, "components": components}
    return {**op, "scope": scope} if scope else op


def write(scope, base, ops, **beside):
    return ok("POST", changesets(scope), changeset(scope, base, ops, **beside))


def head(scope):
    return next(row["head"] for row in ok("GET", "/scopes")["scopes"] if row["scope"] == scope)


def sector(*members):
    return {"sector": {"members": list(members)}}


def resolve(scope, reference):
    return f"/scopes/{scope}/-/resolve?ref=" + urllib.parse.quote(reference, safe="")


a, b, p = f"{TAG}/a", f"{TAG}/b", f"{TAG}/p"
for path, boundary in [(TAG, False), (a, True), (f"{a}/map", False), (f"{a}/flow", False), (b, True),
                       (f"{b}/map", False), (p, False)]:
    ok("POST", "/scopes", {"path": path, "boundary": boundary})
S1, S2, N1, GONE = mint(), mint(), mint(), mint()
write(f"{a}/map", 0, [declare(S1, sector()), declare(S2, sector())])
write(f"{a}/flow", 0, [declare(N1, {"flow.node": {"kind": "storage", "holds": "inventory_tote", "sectors": [S1]}})])
write(f"{b}/map", 0, [declare(GONE, sector())])
write(f"{b}/map", 1, [{"op": "delete", "id": GONE}])
tiling = ok("GET", "/registry/generators/tiling")["versions"][-1]

# POST /scopes.
record("a scope created twice", "create_scope", "scope_exists", "POST", "/scopes", {"path": a, "boundary": True})
record("a scope under a missing parent", "create_scope", "parent_missing", "POST", "/scopes",
       {"path": f"{TAG}/none/x", "boundary": False})

# Reads.
record("an export of a scope that does not exist", "export", "unknown_scope", "GET", f"/scopes/{TAG}/none/-/export")
record("an entity read that is not live", "read", "entity_not_live", "GET", f"/scopes/{a}/map/-/entities/{mint()}")
record("the history of an identity the scope never held", "read", "entity_unknown", "GET",
       f"/scopes/{a}/map/-/history/{mint()}")
record("a component no one defined", "read", "component_undefined", "GET", "/registry/norecordedcomponent")
record("an export at a changeset that does not read", "export", "query_unreadable", "GET",
       f"/scopes/{a}/map/-/export?at=x")

# A changeset's shape.
record("an operation outside the initiating scope", "changesets", "outside_the_initiating_scope", "POST",
       changesets(f"{a}/map"), changeset(f"{a}/map", 1, [declare(mint(), sector(), f"{a}/flow")]))
record("a family of the registry stating bases and creations", "changesets", "registry_writes_itself_alone", "POST",
       changesets("registry/lenses"),
       changeset("registry/lenses", head("registry/lenses"), [], bases={a: 0},
                 create=[{"path": f"{a}/x", "boundary": False}]))
record("a scope created twice in one changeset", "changesets", "created_twice", "POST", changesets(a),
       changeset(a, 0, [], create=[{"path": f"{a}/new", "boundary": False}] * 2))
record("a written scope without its base", "changesets", "base_unstated", "POST", changesets(a),
       changeset(a, 0, [declare(mint(), sector(), f"{a}/map")]))
record("a created scope's base other than 0", "changesets", "created_base_not_zero", "POST", changesets(a),
       changeset(a, 0, [], create=[{"path": f"{a}/new", "boundary": False}], bases={f"{a}/new": 3}))
record("the initiating scope's base stated twice", "changesets", "stated_twice", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [declare(mint(), sector())], bases={f"{a}/map": 0}))
twice = {"generator": "tiling", "version": tiling["version"], "digest": tiling["digest"]}
record("a generator expected twice", "changesets", "expected_twice", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [declare(mint(), sector())], generators=[twice, twice]))
again = mint()
record("one entity written twice", "changesets", "duplicate_op", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [declare(again, sector()), declare(again, sector())]))
record("a body that is not JSON", "changesets", "body_syntax", "POST", changesets(f"{a}/map"), raw=b"{ not json")

# A changeset against the world.
record("two scopes written from bases their heads passed", "changesets", "stale_base", "POST", changesets(a),
       changeset(a, 0, [declare(mint(), sector(), f"{a}/map"), declare(mint(), sector(), f"{a}/flow")],
                 bases={f"{a}/map": 0, f"{a}/flow": 0}))
operation = f"{TAG}-op"
write(f"{b}/map", 2, [declare(mint(), sector())], operation=operation)
record("another body under a stored operation identifier", "changesets", "operation_reused", "POST",
       changesets(f"{b}/map"), changeset(f"{b}/map", 3, [declare(mint(), sector())], operation=operation))
record("a stated generator that would not run", "changesets", "generators_moved", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [declare(mint(), sector())],
                 generators=[{"generator": "tiling", "version": tiling["version"], "digest": tiling["digest"]}]))
dallas = "walmart/dallas"
anchored = next(entry for entry in ok("GET", f"/scopes/{dallas}/-/export")["entities"]
                if "tiling" in entry.get("components", {}))
record("generators that run otherwise than stated", "changesets", "generators_moved", "POST", changesets(dallas),
       changeset(dallas, head(dallas),
                 [{"op": "patch", "id": anchored["id"], "components": {"tiling": anchored["components"]["tiling"]}}],
                 generators=[{"generator": "tiling", "version": tiling["version"], "digest": "0" * 64}]))
record("a payload its schema refuses", "changesets", "schema_invalid", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [declare(mint(), {"template.description": {"title": 5}})]))
record("a patch of nothing live", "changesets", "patch_missing_target", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [{"op": "patch", "id": mint(), "components": sector()}]))
record("a delete of nothing live", "changesets", "delete_missing_target", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [{"op": "delete", "id": mint()}]))
record("a replace of nothing live", "changesets", "replace_missing_target", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [{"op": "replace", "id": mint(), "components": sector()}]))
record("a deleted identity declared again", "changesets", "declare_on_deleted", "POST", changesets(f"{b}/map"),
       changeset(f"{b}/map", head(f"{b}/map"), [declare(GONE, sector())]))
record("an identity another scope owns", "changesets", "owned_elsewhere", "POST", changesets(f"{b}/map"),
       changeset(f"{b}/map", head(f"{b}/map"), [declare(S1, sector())]))

# References and the pin rule.
from_b = lambda member: changeset(f"{b}/map", head(f"{b}/map"), [declare(mint(), sector(member))])
from_a = lambda member: changeset(f"{a}/map", 1, [declare(mint(), sector(member))])
record("a reference to an identity no scope declared", "changesets", "undeclared", "POST", changesets(f"{a}/map"),
       from_a(mint()))
record("a deletion its changeset's own reference names", "changesets", "dangling", "POST", changesets(a),
       changeset(a, 0, [{"op": "delete", "id": S2, "scope": f"{a}/map"},
                        declare(mint(), {"flow.node": {"kind": "storage", "holds": "inventory_tote",
                                                       "sectors": [S2]}}, f"{a}/flow")],
                 bases={f"{a}/map": 1, f"{a}/flow": 1}))
record("a deletion still named at head", "changesets", "left_dangling", "POST", changesets(f"{a}/map"),
       changeset(f"{a}/map", 1, [{"op": "delete", "id": S1}]))
record("a pin within one unit", "changesets", "pin_within_unit", "POST", changesets(f"{a}/map"), from_a(f"{S1}@1"))
record("a reference at head across units", "changesets", "head_across_units", "POST", changesets(f"{b}/map"),
       from_b(S1))
record("a branch across units", "changesets", "branch_across_units", "POST", changesets(f"{b}/map"),
       from_b(f"{S1}@main"))
record("a pin past its scope's head", "changesets", "pin_beyond_head", "POST", changesets(f"{b}/map"),
       from_b(f"{S1}@99"))
record("a tag no scope holds", "changesets", "tag_not_held", "POST", changesets(f"{b}/map"), from_b(f"{S1}@tag:v1"))
X, Y = mint(), mint()
frame = lambda parent: {"frame": {"dims": 2, "units": "mm", "parent": f"{parent}@1"}}
record("two sites pinning one another's new changesets", "changesets", "pin_cycle", "POST", changesets(p),
       changeset(p, 0, [declare(X, frame(Y), f"{p}/a"), declare(Y, frame(X), f"{p}/b")],
                 create=[{"path": f"{p}/a", "boundary": True}, {"path": f"{p}/b", "boundary": True}]))

# The resolve route.
record("a referring scope that does not exist", "resolve", "unknown_scope", "GET", resolve(f"{TAG}/none", S1))
record("a reference no scope declared", "resolve", "resolve_undeclared", "GET", resolve(f"{a}/map", mint()))
record("a reference to nothing live at head", "resolve", "resolve_dangling", "GET", resolve(f"{b}/map", GONE))
record("a pin within one unit", "resolve", "resolve_pin_within_unit", "GET", resolve(f"{a}/flow", f"{S1}@1"))
record("a reference at head across units", "resolve", "resolve_moving_across_units", "GET", resolve(f"{b}/map", S1))
record("a tag no scope holds", "resolve", "resolve_tag_not_held", "GET", resolve(f"{b}/map", f"{S1}@tag:v1"))
record("a pin at a changeset the target was not live at", "resolve", "resolve_pinned_dangling", "GET",
       resolve(f"{a}/map", f"{GONE}@2"))
record("a pin past its scope's head", "resolve", "resolve_pin_beyond_head", "GET", resolve(f"{b}/map", f"{S1}@99"))

# The flagged export (DESIGN.md section 9), asked as wps-client asks it: all three flags set and an `at` for each scope
# a reference at head from the requested scopes can reach, a site's unit. `w` pins `v` at two changesets; `big` is a
# unit of more scopes than one export holds, refused before any `at` is read, so it is asked with none.
FLAGS = ["nested=1", "derived=1", "referenced=1"]


def flagged(scope, at):
    return f"/scopes/{scope}/-/export?" + "&".join(FLAGS + [f"at={read}@{changeset}" for read, changeset in at])


u, v, w, big = f"{TAG}/u", f"{TAG}/v", f"{TAG}/w", f"{TAG}/big"
for path, boundary in [(u, True), (f"{u}/map", False), (v, True), (w, True), (big, True)]:
    ok("POST", "/scopes", {"path": path, "boundary": boundary})
V1, V2 = mint(), mint()
write(v, 0, [declare(V1, sector())])
write(v, 1, [declare(V2, sector())])
write(w, 0, [declare(mint(), sector(f"{V1}@1", f"{V2}@2"))])
write(big, 0, [], create=[{"path": f"{big}/s{index:03}", "boundary": False} for index in range(256)])
record("an `at` naming a scope out of reach", "export", "at_out_of_reach", "GET",
       flagged(u, [(u, 0), (f"{u}/map", 0), (v, 2)]))
record("an `at` leaving a scope of the unit out", "export", "at_left_out", "GET", flagged(u, [(u, 0)]))
record("an `at` past a scope's head", "export", "at_beyond_head", "GET", flagged(v, [(v, 9)]))
record("a scope pinned at two changesets", "export", "reached_twice", "GET", flagged(w, [(w, 1)]))
record("a unit of more scopes than an export holds", "export", "closure_too_large", "GET", flagged(big, []))
record("a flag that is neither 1 nor 0", "export", "export_flag_malformed", "GET", f"/scopes/{u}/-/export?nested=2")
record("a parameter the flagged export does not take", "export", "parameter_unknown", "GET",
       f"/scopes/{u}/-/export?" + "&".join(FLAGS + ["flags=all"]))

# Import (DESIGN.md section 9) refuses a flagged export: `v`'s own, uploaded as a document of a scope of its own.
status, export = call("GET", flagged(v, [(v, 2)]))
assert status == 200, f"the flagged export of {v}: {status} {export}"
ok("POST", "/scopes", {"path": f"{TAG}/x", "boundary": True})
record("a flagged export uploaded to import", "import", "document_flagged_export", "POST",
       f"/scopes/{TAG}/x/-/import?base=0", export)

with open(OUT, "w") as out:
    json.dump({"recorded_from": f"world-graph-server at world-proto {REVISION}", "answers": ANSWERS}, out, indent=1,
              ensure_ascii=False)
    out.write("\n")
print(f"{len(ANSWERS)} refusals recorded under {TAG}/ into {OUT}")
