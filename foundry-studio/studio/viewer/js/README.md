# crucible-viewer

The TypeScript side of the Crucible viewer: what the browser says to the
server, and what it draws the answers on.

- **`src/protocol.ts` is generated** — do not edit it. The server's own
  bindings test writes it from `studio/runs/src/protocol.rs`:

  ```
  cargo test -p crucible --test ts_bindings
  ```

  That test rewrites the file and then fails if what it wrote differs from
  what is checked in, so a message type that changed without the browser's
  half being regenerated stops the gate rather than a running browser.

- **`src/wasm/` is generated too**, by `./build-wasm.sh`, from
  `studio/viewer` — the viewer itself, in Rust, drawing through the
  renderer desvis draws through. It is gitignored: a checked-in copy would be
  a second and staler version of that crate.

- **`src/viewer.ts`** is the one thing written by hand on top: `openViewer`
  loads the wasm module once per page and opens a `Viewer` on a canvas;
  `openFloorViewer` also hands it the 2D canvas its lines are stroked on, and
  `readPalette` reads the colours those take from the page's tokens, or the
  system colours under forced colours.

- **`src/frames.ts`** is the attach protocol's browser half: `attachFrames`
  opens a websocket to a run's frames (the streaming server's
  `/frames/<run>/<attempt>`, or crucible's `/ws`), sends `attach`, pushes each
  binary message into a `Viewer`, answers `lagged` by attaching again from the
  instant the server named, and reports `attached`, `ended`, errors and the
  socket's close as callbacks. It holds back every keyframe after the first,
  since the viewer takes one per attachment and the deltas carry the state on.

## Building

```
npm install         # once
npm run build       # the wasm, then the TypeScript, into dist/
npm run typecheck   # the TypeScript against the wasm's declarations, built first
npm test            # the floor source in a headless Chrome, after build:wasm
```

`build-wasm.sh` refuses to run when the `wasm-bindgen` command line and the
crate are different versions, because the glue one writes only loads a module
the same one produced. Install the version the crate names:

```
cargo install -f wasm-bindgen-cli --version <the version in studio/viewer/Cargo.toml>
```

## Using it

```ts
import { openViewer } from "crucible-viewer";

const viewer = await openViewer(document.querySelector("canvas"));
socket.addEventListener("message", (event) => {
  if (event.data instanceof ArrayBuffer) viewer.push_frame(new Uint8Array(event.data));
});
requestAnimationFrame(function draw(now) {
  viewer.frame(dt);
  requestAnimationFrame(draw);
});
```

`studio/viewer/test/index.html` is the whole thing in one page —
attach, push every frame, draw, scrub — and is what the headless gate runs.

The same, through `attachFrames`, which is what Studio's run page does:

```ts
import { attachFrames, openViewer } from "crucible-viewer";

const viewer = await openViewer(canvas);
const attachment = attachFrames(run.stream.frames_url, viewer, {
  from: "live",
  onAttached: ({ live_edge_ms }) => badge(`live, as of ${live_edge_ms} ms`),
  onEnded: () => badge("replay"),
  onError: (why) => badge(why),
});
// later: attachment.close();
```

`npm test` also runs `test/frames.test.mjs`, which drives `attachFrames`
against a stand-in socket and viewer in plain Node, no browser needed.

A site's floor, with no run:

```ts
import { openFloorViewer, readPalette } from "crucible-viewer";

const viewer = await openFloorViewer(gpuCanvas, linesCanvas);
viewer.set_palette(readPalette(stage));
viewer.set_floor(new Uint8Array(await (await fetch(`/api/floors/${key}`)).arrayBuffer()));
viewer.set_highlights([{ marks: ["b6a9a763-687c-4067-909b-460e986e2229"] }]);
const keys = viewer.pick_entities(event.offsetX, event.offsetY);
viewer.set_projection("orbit");   // or "orthographic", straight down: ruling D119
viewer.orbit_by(1, 0);            // a step's turn, as an orbit key gives it
```
