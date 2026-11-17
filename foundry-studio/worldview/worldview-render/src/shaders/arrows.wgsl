// Edge arrow shader for warehouse visualization.
// Renders instanced directional arrows between adjacent grid cells.

// ── Camera uniform buffer (shared with grid/bot shaders) ────────────────────

struct CameraUniforms {
    projection: mat4x4<f32>,
    view: mat4x4<f32>,
    viewport_size: vec2<f32>,
    cell_size: f32,
    view_mode: u32,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// ── Vertex / Fragment IO ────────────────────────────────────────────────────

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

// ── Vertex shader ───────────────────────────────────────────────────────────

@vertex
fn vs_main(
    // Per-vertex: arrow triangle vertex in local space [-1, 1]
    @location(0) position: vec2<f32>,
    // Per-instance attributes
    @location(1) from_xy: vec2<f32>,
    @location(2) d_xy: vec2<f32>,
    @location(3) scale: f32,
) -> VertexOutput {
    // Both already in metres, and `from_xy` is already a cell CENTRE — the
    // old `+ 0.5` was "half a cell", which no longer means anything.
    // The glyph sits 28% of the way toward the target (compass-rose style);
    // that fraction is of the real edge length, so it still reads correctly
    // when neighbouring cells are different sizes.
    let center = from_xy + d_xy * 0.28;

    // Build a rotation from the direction vector.
    // Arrow template points in +X; rotate to match edge direction.
    // `d_xy` is a real metre delta, so its length is the centre-to-centre
    // distance rather than 1. Guard the degenerate zero-length case.
    let len = max(length(d_xy), 1e-6);
    let nx = d_xy.x / len;
    let ny = d_xy.y / len;

    // Glyph size scales with the local pitch, so arrows stay legible in a
    // 645mm rack row and a 1227mm aisle alike.
    let arrow_size = scale * 0.16;

    // Rotate and scale the template vertex.
    let scaled = vec2<f32>(position.x * arrow_size, position.y * arrow_size);
    let rotated = vec2<f32>(
        scaled.x * nx - scaled.y * ny,
        scaled.x * ny + scaled.y * nx,
    );

    let world_pos = center + rotated;

    var out: VertexOutput;
    out.clip_position = camera.projection * camera.view * vec4<f32>(world_pos, 0.0, 1.0);
    return out;
}

// ── Fragment shader ─────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Black at 70% alpha (matching egui: rgba_premultiplied(0,0,0,178)).
    return vec4<f32>(0.0, 0.0, 0.0, 0.698);
}
