# Foundry application generators

This ownership cut preserves the original v0.1 rack, stamp, tiling, cohort and carrier-pool
generators, plus their shared identity support.
They use the generic WPS generator SDK for the component contract and Foundry's
`studio/world-spatial` for integer frame steps, headings and tile centers.
The newer traversal-link generators remain a separate integration; these
baseline sources do not adopt that topology or change stamp reference behavior.

Build the actual WASM callers and compare them with this checkout's native reader:

```sh
python3 studio/world-generators/check.py \
  --world-proto /absolute/path/to/world-proto \
  --target-dir /absolute/path/to/build-cache \
  --output /absolute/path/to/new-verification-directory
```

The dependency checkout is explicit. The wrapper renders `Cargo.toml.in` in a
scratch workspace, builds all five modules, runs their focused host contracts,
and checks their generated identities and components against Foundry native
expansion. Authored inputs are in `data/seed/application-tests/generators/`;
these inputs are preserved seed/test data, not WPS snapshots. Built modules go
to the caller's output directory and are not automatically registered or seeded.

The original sources and inputs are world-proto's `server/examples/seed` at
commit `2e62d4c`'s parent, `b9d82a8`. The eight spatial
arithmetic tests remain with `studio/world-spatial`. Two ignored checks record
existing differences tracked as `bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg`:

- `archived_turned_cell_matches_native_expansion` exposes the original v0.1 stamp
  swapping a quarter-turned cell's size while the current native expander leaves
  its size local to the composed heading.
- `quarter_turn_rotates_the_physical_cell_footprint` checks that downstream
  geometry composes that local size and heading into the physical footprint.

Run both explicitly with `--known-geometry-regressions`; their failures remain
separate from the passing ordinary contracts and the other parity scenarios.
This cut preserves baseline behavior and does not fix geometry.

The five input lenses and definitions live under `data/seed/application/lenses/`.
Tests assert that each lens equals the text embedded in the corresponding built
module. Dallas and DFW6 fleet cases compare every generated robot/carrier and
retain the original census counts (15/75 and 278/8,510). The original small fleet
fixture covers identity normalization, optional assignment/placement fields,
empty counts and extra fields; a nested-stamp fixture exercises generation
across passes. Identity support keeps its own native unit test.

Registration belongs to the separate bootstrap cut: it must derive registration
records and hashes from the built modules. This source move preserves no stale
module registration hashes, adds no checked-in module bytes, and does not
publish or seed any module.
