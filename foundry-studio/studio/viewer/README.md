# crucible-viewer

The browser's half of the Crucible: a run's own frames decoded inside the tab
and drawn on a `<canvas>`, compiled to WebAssembly.

It exists so that looking at a run does not need a desktop build of the
simulator. The server sends a browser what it would send desvis — the compiled
floor, one keyframe, the deltas after it — and this turns those bytes into the
same picture, through `worldview-render`, the renderer desvis draws through.
A colleague opens a link instead of a toolchain.

What a shell calls: `push_frame(bytes)` for each frame exactly as it arrived,
`set_cursor(ms)` for where to look, the pointer and wheel events, and `frame()`
once per animation frame. What it answers with: the window of frames it holds,
what it drew, and which backend it got — WebGPU where there is one, WebGL2
otherwise.

It also draws a floor sent with no run — a site's or a template evaluation's,
for Agent Studio's Map panel (spec 03 section 3.6): `set_floor(bytes)` takes the
floor document (`worldview_render::FloorAndRoll`, the compiled floor and the
roll of which cells each entity stands on), and the calls after it speak entity
keys both ways. `pick_entities` and `centre_entities` answer the keys on a cell,
fewest cells first and the cell itself last; `set_highlights`, `set_selection`
(the keys one selection marks, a sector by its members, outlined as one union)
and `set_diff` outline entities over the floor on a 2D canvas the page lays over
the viewer's (`attach_lines`), dashed for the agent, solid for the selection,
and never instead of the cells; `frame_entities`, `pan_by`, `zoom_by` and
`camera`/`set_camera` move the camera, always in one jump, so nothing flies
under `prefers-reduced-motion` or otherwise; `set_palette`, `set_axes` and
`show_centre_cross` say what the lines look like. They work on a recording's
floor too.

Drawing lives in `worldview-render`; the control protocol lives in `crucible`;
the sidebar, the charts and the playback controls live in `studio`. None
of them is here.

## Building

The wasm module is not checked in — it is a build of this crate — and the
script that builds it puts it where the TypeScript package finds it:

```sh
studio/viewer/js/build-wasm.sh release
```

About twenty seconds from cold on a 16-core machine. It needs the
`wasm32-unknown-unknown` target and a `wasm-bindgen` CLI of exactly the version
this crate pins (the script checks, and says the `cargo install` line if they
differ), because wasm-bindgen's generated glue only loads a module its own
version produced. `wasm-opt` is optional: with it the module is about half the
size, without it the script says so and carries on.

`studio/runs/dev.sh start` runs that script for you, and skips it when the
built module is newer than everything it is built from.

## Testing

```sh
cargo test -p crucible-viewer                                # the run state and the floor source, on this machine
cargo check -p crucible-viewer --target wasm32-unknown-unknown
(cd studio/viewer/js && npm test)                    # the floor source in a headless Chrome
```

What the frames say at an instant, which cells an entity stands on and where
an outline goes are plain Rust and are tested without a browser;
`tests/floor_source.rs` runs them over a made L-Shape
(`tests/fixtures/l_shape/world.json`). What is drawn is asserted in
`worldview-render`. That a floor draws in a tab is the package's
`test/floor.test.mjs`, which compiles that fixture with `examples/floor_of.rs`
and questions the wasm viewer in a headless Chrome.
