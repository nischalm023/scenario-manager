# Foundry application ownership

Foundry owns the warehouse vocabulary, application reader, Map Creator mapping,
and examples of those features. WPS supplies the generic entity, registry, lens,
generator and adapter mechanisms.

Source integration changes are deliberately small: dependency paths, fixture
locations and the explicit test build wrapper. The application documentation WPS
removed, including its earlier mixed protocol/application specification and
presentations, is in world-proto's history at commit `2e62d4c`, the parent of the
ownership extraction. The current application lens library is
`data/seed/application/lenses/world.lens`; its executable contracts are in
`studio/world-lenses/`.

`research/` holds three forensic digests that came over from world-proto, each
with receipts: what Map Creator actually exports (`mapcreator-format.md`), how
Butler stores and consumes map data (`butler-representation.md`), and how the
retired desmap format worked (`desmap-current.md`). They describe those systems
as they stood in September 2026 and are useful to anyone writing an adapter.

Map Creator's authored exports and schema baseline live under
`data/seed/application-tests/mapcreator/`. They are seed/test inputs, not WPS
snapshots. The compatibility adapter still speaks its existing application
topology. Ownership extraction does not convert it to traversal links or change
document IDs. Those migrations require their own behavior review.

The adapter is in `studio/world-adapters/mapcreator/`. Its executable checks build
the current source into a WASM component, exercise ingest/project through the
generic WPS host, verify exact round trips and idempotence, and retain field
accounting and refusal coverage. Its definitions and lens are under
`data/seed/application/`; moving them does not register them automatically in a
production database.

The five baseline generators are in `studio/world-generators/`; their shared
spatial arithmetic is in `studio/world-spatial/`. Their build wrapper verifies
actual WASM modules against the native reader, including original Dallas and
DFW6 fleet inputs. `scripts/seed-dev.sh` builds and registers them for the
development stack; production bootstrap remains the next integration cut.

The single native reader is the warehouse site model, `sim/site-model/`. It consumes WPS's
generic core, while the simulator and application lens tests consume WPS's
generic lens engine. The old embedded server and lens engine have been removed.
The current reader produces byte-identical canonical expansions for the four
compared Foundry sites. Four preserved native tests and two generator checks
remain explicitly ignored for the existing rotated-cell geometry discrepancy;
this ownership change does not fix it.

WPS dependencies are pinned to commit `cd4923516827d79fbac742179a4193627dafa43b`,
world-proto#4's merge on `master`, which includes the latest PR1 corrections. The WPS
ownership change must be reviewed and merged before this Foundry change lands.
The source-audit tests inspect Cargo's resolved WPS dependency. Generic WPS tests
run without a Foundry checkout.

GitHub Actions and container builds need read access to the private WPS repository.
The draft records that credential integration as a merge blocker; no credential
is stored in source or in an image. Local Cargo builds use the developer's Git
credentials. Production image contexts include the application schema/lens assets
needed at compile time, while site and test inputs remain excluded.

This ownership pair preserves authored inputs; it does not complete the
repository-wide fixture migration. The development stack is seeded by
`scripts/seed-dev.sh`, which registers this application's generators,
vocabularies and lenses in WPS and loads the walmart-dallas demo site. Real WPS
bootstrap/export comparisons, removal of legacy document metadata and adoption
of traversal links are separately tracked work. Preserved inputs must remain
until those comparisons support their retirement.
