// The package's public surface: what the browser talks to the Crucible server
// with, and what it draws the answers on.
//
// `protocol` is the server's control messages, generated from its own Rust
// types. `viewer` is the wasm build of `studio/viewer` — the same
// renderer desvis draws with, compiled for a tab. `frames` feeds a viewer from
// a run's frames socket under that protocol.
export * from "./frames.js";
export * from "./protocol.js";
export * from "./viewer.js";
