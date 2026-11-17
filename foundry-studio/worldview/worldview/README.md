# worldview

The `.worldview` file a simulation run is recorded into, and the code that
writes and reads it.

One file holds everything one attempt at a run produced: the world it ran in,
the scenario it ran, and the run itself as frames stamped with simulated time.
Every way of looking at a run is then one cheap operation over that file —
open it at minute forty, scrub back to minute ten, read a crashed run's
prefix, rebuild the workbook — and none of them decodes from the start.

Two layers, and the crate is split along them. The **container** is frames,
the sections inside a keyframe or a delta, the index block and the trailer; it
carries payload bytes and never reads them. The **vocabulary** is what those
bytes mean: the scenario block, declarations, playback, scalars, heat, tables,
motion, the end block, and the state a reader rebuilds from a keyframe and the
deltas after it.

Nothing here depends on the simulator, so the same code decodes frames in a
browser. Byte-level specification: `docs/designs/worldview-format.md`. The
shape and why: `docs/designs/worldview-streaming.html`.

## Building and testing

```sh
cargo test -p worldview --all-features     # the library, the CLI and the fixtures
cargo check -p worldview --no-default-features --target wasm32-unknown-unknown
```

The wasm check is not optional politeness: the browser viewer compiles this
crate, and the default features pull in a zstd that does not build for wasm32
(a pure-Rust decoder takes its place there).

## Looking at a file

```sh
cargo run --release -p worldview --features cli -- inspect <run.worldview>
cargo run --release -p worldview --features cli -- seek <run.worldview> 300000
cargo run --release -p worldview --features cli -- index <run.worldview> --verify
```

`inspect` prints every frame with its offset, instant, size and sections;
`seek` prints the frames a scrub to that millisecond would read; `index` prints
the index block, and `--verify` says whether it matches the frames ahead of it.
The CLI is behind the `cli` feature so a wasm build never sees clap.
