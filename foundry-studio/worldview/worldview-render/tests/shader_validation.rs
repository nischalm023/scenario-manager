//! What can be held about this crate's WGSL without a GPU.
//!
//! Two things, and this is the only place either is checked. Every shader in
//! `src/shaders/` parses and validates through naga (the same front end wgpu
//! uses), so `cargo test -p worldview-render` catches shader errors `cargo check` cannot
//! see. And the grid shader's depth bias is read back to confirm it still
//! reaches only the ground surface -- a claim `cell_layer` makes in prose and
//! the Rust side cannot enforce, since the arithmetic lives in the shader.

use naga::valid::{Capabilities, ValidationFlags, Validator};

fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name}: parse error:\n{}", e.emit_to_string(source)));
    Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: validation error: {e:?}"));
}

/// The grid shader's depth bias reaches the GROUND SURFACE and nothing else.
///
/// It is the property `renderer::cell_layer::DEPTH_TIER` claims and the
/// one a refactor is most likely to undo, because applying the bias to the
/// whole instance is the shorter line. Biasing a rack's walls and top pushes
/// the whole box back by a tier -- about 0.8m of world separation at a 200m
/// camera -- and a bot or tote just behind a rack wall then passes its later
/// `LessEqual` test and draws in front of the rack (Codex review, 2026-09-04).
///
/// This reads the source rather than the picture: what the depth buffer ends
/// up holding needs a GPU, and these tests run without one. So it holds the
/// two things the source can say -- the bias is gated, and the gate is the
/// ground plane.
#[test]
fn the_grid_shader_biases_only_the_ground_surface() {
    let src = include_str!("../src/shaders/grid.wgsl");
    assert!(
        src.contains("let on_ground = world_z == 0.0;"),
        "grid.wgsl no longer decides the ground plane by world_z == 0.0"
    );
    let biased: Vec<&str> = src.lines().filter(|l| l.contains("depth_bias") && l.contains("clip_position.z")).collect();
    assert_eq!(biased.len(), 1, "expected exactly one line adding the depth bias to clip z, found {biased:?}");
    assert!(
        biased[0].contains("select(0.0, depth_bias, on_ground)"),
        "grid.wgsl applies the depth bias off the ground plane: {}",
        biased[0].trim()
    );
}

#[test]
fn all_shaders_validate() {
    validate("grid.wgsl", include_str!("../src/shaders/grid.wgsl"));
    validate("totes.wgsl", include_str!("../src/shaders/totes.wgsl"));
    validate("bots.wgsl", include_str!("../src/shaders/bots.wgsl"));
    validate("arrows.wgsl", include_str!("../src/shaders/arrows.wgsl"));
    validate("rack_frames.wgsl", include_str!("../src/shaders/rack_frames.wgsl"));
}
