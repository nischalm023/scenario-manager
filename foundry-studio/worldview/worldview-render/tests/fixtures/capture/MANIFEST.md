# Where the golden bytes came from

Every `.bin` beside this folder is what DESVIS UPLOADED before the renderer was
carved out of it, for the scene `src/renderer/pinned_buffers.rs` builds. They
were captured once, this way, and can be captured again the same way.

- **Commit**: `db057e21d` ("worldview: the round-4 review fixes …"), the tip of
  `agent/worldview-p1` before the carve-out commit.
- **Program**: `golden_capture.rs` beside this file, copied into that commit's
  tree as `desvis/src/gpu/renderer/golden_capture.rs`. It builds the scene
  against that desvis's own types and writes the arrays through that desvis's
  own builders.
- **The four-line patch it needs**, all of it reach, none of it arithmetic:
  - `gpu/renderer/mod.rs`: `#[cfg(test)] mod golden_capture;`
  - `gpu/renderer/rack_frames.rs`: the `RackBox → RackFrameInstance` map lifted
    out of `RackFrameGpuResources::new` into `pub(super) fn frame_instances`,
    and the instance struct made `pub(super)` (the crate now does the same, so
    a GPU is not needed to read the steel rows).
  - `gpu/renderer/totes.rs`: `draw_instances` and `DrawInstance` made
    `pub(super)`.
  - `app/mod.rs` and `app/conveyor_motion.rs`: the module and `apply_transits`
    made `pub(crate)`, so the capture can run a travelling container through
    the same code the app does.
  - `traffic.rs`: a `#[cfg(test)] seed_cell_for_capture` that inserts one
    cell's measured traffic, so the wait and speed tints are real numbers
    rather than "nothing measured". The tints it produces are
    `wait = cell / 8` and `speed = cell / 10`, which is what
    `pinned_buffers::Measured` reports.
- **Command**, from a worktree of that commit:
  `cargo test -p desvis --bin desvis golden_capture -- --ignored`
- **What it writes**: `bots_{0,1,2}`, `totes_{0,1,2}` (the expanded shell-and-
  contents rows the GPU receives), `cells_{off,combined,wait,speed}`,
  `arrows`, `rack_frames`, `underlay`.

## What these bytes do NOT cover

The camera uniforms, the pipeline and depth state, the shaders themselves, and
the draw order. Those are held by `camera/projection.rs`'s depth-tier tests,
`tests/shader_validation.rs`, and — for the whole picture, over four real
recordings — `scripts/worldview/parity_b.sh`.
