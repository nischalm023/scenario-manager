# schema/ — the JSON Schema layer

> Purpose of this directory: state, in a form a machine can check, what a
> *Go World* protocol document may look like. `SPEC.md` says what the words mean and
> `rust/` shows what a reader does with them; these files are the third leg —
> the shape, checkable by anyone with a validator and no Rust toolchain. They
> describe **proto_version 0.7.x** and nothing else.

The schemas are draft **2020-12**. They are written against SPEC.md and the
reference reader's serde structs together: where the spec fixes a meaning, the
description says so and cites the section; where only the structs fix a shape,
the structs win, because a document is read by a reader and not by a paragraph.

## The two profiles

A document gets read by two kinds of reader, and they must not be equally
strict. Pick the profile by which one you are.

**`wire.json` — for a document you did not write.** A component this version
does not know is carried and ignored; so is an unknown field inside a component
it does know. That is SPEC §9's forward-compatibility rule, not politeness: a
consumer that rejects what a newer producer added is a consumer that forces
every team to negotiate before shipping anything. What the wire profile does
insist on is the part a consumer acts upon — the four record kinds and their
disjointness, identities and derivation addresses, references, the closed
enums, and the fields a known component cannot work without. **Every v0.7
document the project ships validates against this profile**; that is the
standing bar, and the three documents under `examples/` and `legacy/examples/` meet it. (The copies
under `rust/tests/fixtures/` are the crate's own test inputs — same migration,
kept separate so the test suite never depends on files a team member is
mid-edit on.)

**`authoring.json` — for a document you are writing.** Everything above, plus
the closure: on a component the core vocabulary defines, a field it does not
define is a typo until proven otherwise, and an entity carrying one component
but not the one it needs is unfinished. The closure stops at the core's
components. A component the core does not define passes whatever its name,
plain or dotted — a product's `easyloop`, Map Creator's `zone` or
`mapcreator.station`, an experiment's — with its fields unread, as on the wire:
this profile knows the core's names alone, and whether any other name means
something is a registry's to say, not a schema's (ruled 2026-10-18). Its name
is held to the grammar a registry holds every name to, segments of `[a-z0-9_]`
joined by `.`, so a key that could never name a component (`Radio-Shadow`) is
still refused. `required_components` follows in both profiles: a document may
require any legal component name, and a reader that does not know one refuses
the document (SPEC §9).

A file failing the authoring profile is not an invalid document. It is a
document saying something the v0 vocabulary has no definition for, which is a
question for the spec. Today the fixtures raise five such questions; they are
listed under "What the strict profile currently flags".

## Running it

`validate.py` is the runner. It exists because the schemas identify each other
by URN rather than by URL — nothing is fetched — so a validator has to be handed
the whole set first. The script reads every `*.json` beside it, registers each
under its own `$id`, and validates.

```sh
cd data/seed/application/schema
python3 -m venv .venv && ./.venv/bin/pip install jsonschema   # 4.18+, once; .venv is gitignored

./.venv/bin/python validate.py wire      ../rust/tests/fixtures/walmart-dallas/*.json
./.venv/bin/python validate.py wire      ../rust/tests/fixtures/minimal-putaway2/*.json ../rust/tests/fixtures/cencora-v3/*.json
./.venv/bin/python validate.py authoring ../rust/tests/fixtures/walmart-dallas/world.world.json
```

Exit status is 0 when every file passed. Failures print as `instance path:
message`, following the `oneOf` branch that got furthest into the record — the
other three branches only ever say "this is not the kind of record you wrote".

Both profiles were checked at v0.5 with **python jsonschema 4.26.0** and,
independently, with **ajv 8.20.0** (`new Ajv2020({strict: false})`, every file
added with `addSchema`, then `getSchema('urn:worldproto:0.5:wire')`), and the two
agreed file-for-file and location-for-location. The v0.6 and v0.7 revisions have
been re-run against python jsonschema 4.26.0 only; the ajv half is unrepeated and
is the one claim here that has not been checked since the rename.

## The files

One file per component family, mirroring `rust/src/components/` one-to-one, so
that a component's schema and its struct are found by the same name — which is
also what makes the sync test below a file-to-file comparison rather than a
search. The roots, the record shapes and the shared primitives sit alongside
them.

| File | What it holds |
|---|---|
| `values.json` | The primitives every component is built from: identities, references, derivation addresses and segments, lengths as counts of a frame's steps, the param reference, `$comment`. Mirrors `ids.rs`, `derivation.rs`, `units.rs`. |
| `components/spatial.json` | `frame`, `pose`, `region`, `tiling` (with the exclusions a tiling carves out of itself), `cell`, `charging` — where things are. |
| `components/structure.json` | `rack`, `access`, `storage`, `slot`, `relay`, `access_cell`, `flow`, `stitch` — the built structure and the rules that turn it into a graph. |
| `components/fleet.json` | `robot_class`, `cohort`, `robot`, `carrier_pool`, `carrier` — the population that moves. |
| `components/product.json` | `template`, `stamp`, `station`, `port`, `queue`, `waiting`, `presentation_point`, `sector`, `demand`, `carrier_type`, `lineage` — packaged products, their pieces, and where a product came from. |
| `components/identity.json` | `external_ids`. It has a file of its own because the crate has no module for it — see the sync test. |
| `components.json` | The register: every component name, the lenient reading of a component map, and the strict one. |
| `entity.json` | An entity as a composition of components, plus the derived node and the edge — the expanded form's building blocks. |
| `record.json` | The four record kinds as a `oneOf`, and the `expect` pin. |
| `archetypes.json` | The validation lenses: rack, cell_grid, holding pool, placed station… read off the components present. |
| `document.json` | Document metadata, the top-level keys, and the layer list. |
| `wire.json`, `authoring.json` | The two profile roots. |
| `validate.py` | The runner. |

`$id`s are URNs (`urn:worldproto:0.7:components:spatial`) because a schema id
is an identifier, not an address, and this repository has no published home yet
(SPEC §12.1). If it gets one, they become `https://` URLs in one pass.

## How the pieces compose

- **One `$defs` entry per component** is the atomic unit. `components.json`
  assembles them into a component map; an entity *is* that map plus an
  identity, which is why there is no type field anywhere in these files.
- **Archetypes are lenses, never tags.** `archetypes.json` says "if this entity
  carries a `tiling`, it must carry a `region`" — a rack, a station or a
  holding pool is something you *read off* an entity, never something the data
  declares itself to be.
- **Record kinds are a `oneOf`** whose four branches are disjoint by shape:
  `patch`, `derivation` and `suppress` each exclude the others, exactly as the
  reader's own classification does.
- **Unions stay additive at the component level, and closed below it.** An
  unknown component name is fine on the wire; `flow.rule` is not. The closed
  enums are `flow.rule`, `flow.direction`, `access.face`, a relay level's
  `face`, `stitch.rule`, `station.kind`, `port.direction`, `queue.discipline`,
  `storage.holds`, `carrier_type.kind`, the carrier `designation`,
  `robot_class.locomotion`, a lineage entry's `role`, `demand.kind`, and
  `frame.dims`. Each takes its
  values from a Rust enum's serde spellings or from a SPEC §10 row; the ones
  deliberately left open (`robot_class.role`, `station.role`, `assignment.kind`,
  `placement.kind`, `frame.units`, `doc.kind`) say why in their descriptions.
- **The authoring profile adds a closure, not a second vocabulary.**
  `authoring.json` walks the four places a component map appears — on a
  record, inside a template's internals, on a derived node, and inside a
  tiling's `derives` — and applies the strict reading there. It names those four
  places explicitly. A `$dynamicRef` would say it in one line, but ajv 8 resolves
  a cross-resource dynamic anchor to the resource root instead of to the anchored
  subschema, and a schema layer only one validator reads correctly is not a
  schema layer. The cost is that a fifth place carrying a component map must be
  added to `authoring.json#/$defs/strict_document` by hand — `derives` was the
  fourth, and v0.6 added it that way.

## What the schemas deliberately do not check

- **Anything that needs a second entity.** Whether a reference resolves,
  whether an `expect` pin still matches the geometry, whether a size divides
  the pitch, whether a contact stitch produces any links at all: all of it
  needs the merged, expanded world, which is the reader's job (SPEC §8's
  pipeline) and not a schema's. A document can pass both profiles and fail to
  load.
- **Nested objects stay open in both profiles.** The authoring closure applies
  to each component's own keys. A stray key inside `transform_to_parent` or
  inside a `placement` is not caught, because closing those in the shared
  library would close them for the wire profile too, which §9 forbids.
- **Where a placeholder may stand.** SPEC §6 lets a parameter reference stand
  as any whole JSON value inside a template. The schema accepts one in place of
  a number, a numeric array, a boolean, an entity reference, a cell address or a
  derivation key, and a bare `local_ref` in place of a cell address as well as
  an entity reference. The reference forms arrived on 2026-10-19, so that a
  template can stitch itself to whichever floor its stamp names, and queue over
  a tile its stamp picks or a cell it declares itself. No parameter is accepted
  in place of a closed enum or a whole component object: those are the values
  the schema exists to pin down, and a template that needs a parameterized enum
  is a ruling to make, not a hole to widen. A schema cannot see whether a value
  stands inside a template, so a placeholder passes wherever its value may go,
  and keeping it inside one is the reader's rule: the reference reader refuses
  a parameter left wherever this schema admits one, and a handle left in any
  field it types as an entity reference, a cell address or a derivation key,
  whether it reads the field or carries it unread
  (`proto/rust/src/placeholders.rs`). The placeholders are closed — a
  parameter and a bare handle to their one key, a local address to its handle
  and its key, each with a `$comment` beside — unlike the nested objects above:
  none is data a newer producer may extend, each is the one shape every reader
  rewrites, and every reader refuses a `local_ref` in any company but a `key`.
- **The profile boundary is not a permission boundary.** Both profiles read the
  same document; neither knows which layer it came from.

### What the lenses cannot see

An archetype lens is a statement about a whole entity, and a record is not
always a whole entity. A later layer may patch one component of something an
earlier layer declared — Dallas's `policy.world.json` does exactly that, carrying a
`flow` and nothing else — and a lens read against that fragment would report a
missing `region` that is sitting one layer away. So the authoring profile
applies lenses only to records that introduce an entity and to template
internals, never to patches, overrides or suppressions.

That leaves one real hole: SPEC §2 lets a *declaration* in a later layer
replace components of an entity an earlier layer introduced, and such a
declaration is a fragment too. The lenses would misread it. The honest home for
these checks is the merged world — the reader's check step — and this is filed
as an open question below rather than papered over.

## The sync test (design; the crate is not touched here)

**What it is for.** These schemas and the reader's serde structs are two
statements of the same shape, and nothing today makes them agree. The test's
job is to fail the build the moment they stop agreeing, so that neither can
drift silently into being a lie about the other.

**Where it lives.** `rust/src/components/schema_sync.rs`, a `#[cfg(test)]`
module reading the JSON files from `data/seed/application/schema/` at test time — inside the
crate rather than under `tests/`, because `cfg(test)` does not reach
integration-test builds, so the schemars derives the census needs exist only
for the crate's own test target. It needs no network and no code generation.

**How it enumerates components.** From the schema side, the register is
`components.json#/$defs/core_name` — one list, and every `$ref` in
`#/$defs/set` says which file and which `$def` holds that component. From the
crate side, a table maps each name to its Rust type; the table is checked
against `components::KNOWN_COMPONENTS` for exact equality in both directions,
so adding a component to the crate without adding it to the table fails the
test rather than skipping the component.

**What it checks, per component.**

1. **The register matches.** `core_name`, the `set` property map, the
   `set_strict` property map, and `KNOWN_COMPONENTS` all list the same names.
2. **Fields match.** The schema's `properties` keys equal the struct's serde
   field names, after `#[serde(rename)]` (so `presentation_point.for` is
   compared against `for_`) and excluding `#[serde(flatten)]` catch-alls, which
   are instead required to correspond to an open object in the schema
   (`placement` is the only one today).
3. **Required matches.** A field is `required` in the schema exactly when the
   struct field has no `#[serde(default)]`. That is the mechanical rule the
   schemas were written to, so the test is comparing two spellings of one fact.
4. **Closed enums match.** For every schema `enum`, the values equal the serde
   spellings of the corresponding Rust enum's variants — `FlowRule` against
   `flow.rule`, `Side` against `access.face` and a relay level's `face`,
   `LineageRole` against a lineage entry's `role`, and so on. Enums that exist
   only in SPEC §10 (`station.kind`, `port.direction`, `storage.holds`,
   `carrier_type.kind`, the designation, `queue.discipline`, `demand.kind`) have
   no Rust enum to compare against and are listed in the test as
   spec-sourced, so that giving one a Rust enum later is a change the test
   notices.
5. **Every fixture still validates.** The other direction of the same
   agreement: run both profiles over `rust/tests/fixtures/**` in CI, wire
   required to pass everywhere and authoring reported.

**How a struct's field census is obtained.** Rust has no reflection, so the
test derives it: `#[cfg_attr(test, derive(schemars::JsonSchema))]` on the
component structs plus a `schemars` dev-dependency gives a machine-readable
property and required set per struct, and the test diffs that against the JSON
file. (The alternative — a hand-written table of field names — is cheaper to
start and rots exactly like the thing it is meant to protect.)

**What it reports today:** one field, `cell.legacy_grid_coordinate` (SPEC §4,
§10) — spec-named, with no struct field behind it, because §4 does not say which
integer frame the recorded pair is read in and with two lattice frames the pair
is ambiguous. The test carries it in a list of fields waiting on a ruling and
checks that list in both directions, so the day the struct grows the field is
the day the list has to lose it. The two other gaps this paragraph used to name —
`external_ids` missing from `KNOWN_COMPONENTS`, and `frame.vendor` with no struct
field — were both closed.

## What the strict profile currently flags

Five groups of fields, in every example, none of them typos — each is a field
the v0 vocabulary names as a concept but not as a spelling, or does not name at
all. They are listed here because the strict profile's report is the most
useful census of them anyone has.

| Field | Where | Standing |
|---|---|---|
| `robot_class.max_speed`, `max_climb`, `carry_exclusive` | all three fixtures | SPEC §10 names speeds in steps per second and carrier compatibility; neither has a spelling or a struct field. |
| `station.bins`, `station.exit_direction` | minimal-putaway2, cencora | Carried over from desmap by both conversions and recorded in their CONVERSION.md; SPEC §10's station row is `kind` and `role` only. |
| `presentation_point.station` | minimal-putaway2, cencora | An explicit binding to a station, which is the very thing SPEC §12.11 leaves open (v0 binds by proximity). |
| `demand.bindings`, `demand.routing`, `demand.profile` | Dallas workload | SPEC §10 names "external data binding, routing designations"; no shape is fixed for either. |
| `port.induct_secs`, `port.extract_secs` | Dallas catalog | SPEC §10's port row names "timing"; no spelling. |

The schema does not define these fields, on the rule that a field enters the
schema when SPEC §10 or a serde struct names it. Defining them is a spec
decision, and each is one line of schema once made.

## Open questions

1. **Should the archetype lenses live here at all**, given that a
   later-layer declaration is a legitimate fragment? The alternative is to move
   them into the reader's check step, where the merged world is available, and
   keep the schema to shape alone.
2. **`frame.units`** is an open string. `mm` and `index` are the only values in
   use and the only ones SPEC §4 describes; closing it is a one-line change and
   a ruling.
3. **`placement.kind`** is open for the same reason: SPEC §10 names three
   placement rules but only two have a spelling in the examples
   (`sector`, `distributed_storage`); the third would presumably be
   `named_slots`.
4. **`assignment.kind`** is open because the reader treats anything that is not
   `hard` as free. If `free` is the only other value, the schema should say so.
5. **An explicit stitch with no `pairs`** validates today, because the reader
   also accepts it and silently links nothing — while a *contact* stitch that
   links nothing is a load error. My lean is that both should fail, in the
   schema and in the reader.
6. **`port.of` entries** are unconstrained objects because SPEC §10 describes
   them as carrier-type references plus a designation and the Dallas catalog
   writes a carrier *kind* plus a designation, explaining in its own comment why
   it does not want the document dependency. One of the two should move.
7. **`cell.legacy_grid_coordinate`** is modelled as two integers, which is the
   only sane reading of SPEC §4's sentence, but the spec does not say which
   integer frame the pair is read in. With one grid frame per world it is
   unambiguous; with two it is not.
8. **Three places where the schema is stricter than the reader**, each on a
   record that means nothing as written: `expect` on a declaration or a patch
   (the reader parses it and ignores it), `patch` alongside a `derivation` (the
   reader reads the record as an override and drops the `patch`), and
   `"patch": false` (the reader reads it as a declaration). All three are
   refused here. Worth confirming, or worth tightening the reader.
