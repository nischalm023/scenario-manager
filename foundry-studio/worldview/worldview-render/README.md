# worldview-render

The wgpu renderer a warehouse floor is drawn with, shared by both viewers:
desvis on the desktop and the browser viewer in a tab.

It exists so that a fix to how a robot, a container or a rack is drawn is made
once. The two viewers used to have their own copies of this code, they had
already drifted, and nothing short of a screenshot could tell.

The crate takes a wgpu device, queue and surface format from whoever owns the
window or the canvas, plus the state of a run at an instant, and draws. It
knows nothing about how a run is read or how a window gets on screen: no egui,
no DOM, no simulator. The camera, the cell the pointer is over, the instance
rows and the shaders are here; the panels around them are not.

Design: `docs/designs/crucible-server-ui.html`, section 04.

## Building and testing

```sh
cargo test -p worldview-render
cargo check -p worldview-render --target wasm32-unknown-unknown
```

The tests validate the shaders with naga and assert the instance rows built
for a given state, so a change in what reaches the GPU is a failing test
rather than a picture somebody has to look at.

## The parity harness

That both viewers draw the same thing is asserted by a test in this crate. That
desvis on this crate draws what master's desvis drew is a separate gate, run
over four fixtures against a pinned master checkout:

```sh
scripts/worldview/parity_b.sh <results-dir>
```

It dumps the arrays each side would upload at a fixed list of instants and
compares them element by element. `docs/designs/worldview-render-dump.md` says
what the dump holds and what the comparison does and does not prove.
