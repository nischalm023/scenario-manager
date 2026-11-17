# worldview-floor

The compile from a loaded world document to the floor a viewer draws: each
cell's tile and centre, the hops between cells, the rack steel, the ground
underneath, and the roll of which cells each entity of the document became.

A world document states entities and components. It states no floor. Only the
simulator's map compiler turns the one into the other, and this crate turns
that map into the plain scene types `worldview-render` draws. Both viewers come
through here — desvis from the map it has just loaded, and the Crucible server
on behalf of a browser with no simulator in it — so the two cannot put a cell
in two places.

That is also why it is its own crate: it is the one place that depends on both
`dessim` and `worldview-render`, and the renderer compiles to wasm while the
simulator does not.

Nothing a panel says rather than draws lives here — which zone a cell is in, a
relay's diagnostics, a station's tooltip — those stay with the viewer showing
them.

## Building and testing

```sh
cargo test -p worldview-floor
```

One of the tests is the equality that matters: the floor the server compiles
for a browser equals the floor desvis drew, byte for byte, over a checked-in
recording.
