//! # crucible-viewer
//!
//! The browser's half of the Crucible: a run's own frames decoded inside the
//! tab and drawn on a `<canvas>` with `worldview-render` — the same renderer
//! desvis draws with — compiled to WebAssembly.
//!
//! **What it is for.** Looking at a run should not need a desktop build of the
//! simulator. The server sends a browser exactly what it sends desvis — the
//! compiled floor, one keyframe, the deltas after it — and this turns those
//! bytes into the same picture, so a colleague opens a link instead of a
//! toolchain.
//!
//! The same viewer draws a floor sent with no run — a site's or a template
//! evaluation's, for Agent Studio's Map panel (spec 03 section 3.6) — and
//! answers in entity keys both ways: what stands under a point, and where
//! the entities a page names stand, outlined over the floor.
//!
//! **What is here.** [`run`] — the frames this browser holds and what they say
//! at the instant the cursor points at; [`roll`] — which cells each entity
//! stands on, read both ways; [`outline`] — the edge of what a page marked;
//! [`screen`] — where the floor lands on the canvas, and the axes and the
//! cross drawn there; [`palette`] — the colours of the ground and the lines.
//! Those five are plain Rust, tested without a browser. `viewer` is
//! the canvas, the GPU and the `#[wasm_bindgen]` surface a TypeScript shell
//! calls, with `canvas` holding the GPU behind one `<canvas>` and `gesture`
//! summing a tab's pointer events into the one frame of input the camera
//! reads; those three exist only on wasm32, because a browser surface does.
//!
//! **What is not here.** Drawing (that is `worldview-render`), the wire's
//! control messages (that is `crucible`'s `protocol`, which reaches the
//! browser as TypeScript), and every choice about what a person sees — the run
//! list, the charts, the playback controls — which belong to the shell.

pub mod outline;
pub mod palette;
pub mod roll;
pub mod run;
pub mod screen;

#[cfg(target_arch = "wasm32")]
mod canvas;
#[cfg(target_arch = "wasm32")]
mod gesture;
#[cfg(target_arch = "wasm32")]
mod viewer;

pub use run::{Run, Took};

#[cfg(target_arch = "wasm32")]
pub use viewer::Viewer;
